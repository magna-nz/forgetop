import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { AnimatePresence } from "framer-motion";
import { useQueries, useQueryClient } from "@tanstack/react-query";
import {
  apiGet,
  apiPost,
  pipelineAnnotationsKey,
  pipelineDetailKey,
  pipelineRunLogsKey,
  repoParam,
  rerunPipeline,
  respondPipelineApproval,
  useConnections,
  usePipelineAnnotations,
  usePipelineArtifacts,
  usePipelineDetail,
  usePipelineLogs,
  usePipelines,
} from "../api";
import { unsupportedMessage } from "../capabilities";
import { fmtSecs, pipeMeta, providerMeta, relativeTime } from "../format";
import { isActive, rowGates, rowShownStatus, shownStatus } from "../pipeline/grouping";
import { estimateCandidates, hasTimedJobs, pipelineEstimates, pipelineHistory, type CurrentRun } from "../pipeline/history";
import { failureSummary, logLines, shortLocation } from "../pipeline/runlog";
import {
  autoSelect,
  failedTarget,
  flatten,
  jobGhosts,
  parkedSplit,
  runSecs,
  waitingSecs,
  type FlatNode,
  type TreeInput,
} from "../pipeline/timeline";
import type {
  ApprovalDecision,
  PipeRef,
  PipelineApproval,
  PipelineDetail,
  PipelineRun,
  PipelineRunStatus,
  PipeRow,
} from "../types";
import { ApprovalBanner } from "./pipeline/ApprovalBanner";
import { LogPanel } from "./pipeline/LogPanel";
import { ArtifactsCard, PaneButton, PaneLink, ProblemsCard, statusWord } from "./pipeline/parts";
import { HistoryStrip } from "./pipeline/RunHistory";
import { nodeId, RunTimeline } from "./pipeline/RunTimeline";
import { markCanceled, markRerun, markResponded, NO_CAPS, noteColor, prLink, useNow } from "./pipeline/runState";
import { Avatar, Chip, SlideOver, StatusBadge } from "./ui";

/** At most `max` characters, the last an ellipsis when cut (the TUI's `truncate`). */
const truncateChars = (s: string, max: number) => {
  const cs = Array.from(s);
  return cs.length > max ? `${cs.slice(0, max - 1).join("")}…` : s;
};

// ---- opener context ----

const PipeOpenerCtx = createContext<(ref: PipeRef) => void>(() => {});
export const usePipelineOpener = () => useContext(PipeOpenerCtx);

export function PipelineDetailProvider({ children }: { children: ReactNode }) {
  const [ref, setRef] = useState<PipeRef | null>(null);
  const open = useCallback((r: PipeRef) => setRef(r), []);
  return (
    <PipeOpenerCtx.Provider value={open}>
      {children}
      {/* Keyed by run: another run gets a fresh panel, so no frame shows the last run's state. */}
      <AnimatePresence>
        {ref && <PipelineDetailPanel key={`${ref.conn}|${ref.repo ?? ""}|${ref.runId}`} pipeRef={ref} onClose={() => setRef(null)} />}
      </AnimatePresence>
    </PipeOpenerCtx.Provider>
  );
}

interface LogTarget {
  jobId: string;
  step: number | null;
  /** Opened by "Jump to error": land on the first error. */
  jump: boolean;
}

// ---- panel ----

function PipelineDetailPanel({ pipeRef, onClose }: { pipeRef: PipeRef; onClose: () => void }) {
  const { data, isLoading, error } = usePipelineDetail(pipeRef);
  const connections = useConnections();
  const pipes = usePipelines();
  const openRun = usePipelineOpener();
  const qc = useQueryClient();

  const rows = useMemo(() => pipes.data ?? [], [pipes.data]);
  const provider =
    connections.data?.find((c) => c.id === pipeRef.conn)?.provider ?? rows.find((r) => r.connection_id === pipeRef.conn)?.provider;
  const providerLabel = provider ? providerMeta(provider).label : "the provider";
  const unsupported = provider ? unsupportedMessage(provider) : "This provider currently does not support this feature";

  const run = data?.run;
  const approvals = useMemo(() => data?.approvals ?? [], [data]);
  const caps = data?.capabilities ?? NO_CAPS;
  const active = run ? isActive(run.status) : false;
  const now = useNow(active);
  const shown: PipelineRunStatus | null = run ? shownStatus(run, approvals.some((a) => a.blocks_run)) : null;
  const gates = rowGates({ approvals });

  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [folds, setFolds] = useState<Record<string, boolean>>({});
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [logTarget, setLogTarget] = useState<LogTarget | null>(null);
  const [jumpToken, setJumpToken] = useState(0);
  const [artifactsOpen, setArtifactsOpen] = useState(false);
  const initFor = useRef<string | null>(null);
  const prevStatus = useRef<PipelineRunStatus | null>(null);
  const runKey = `${pipeRef.conn}\u0001${pipeRef.repo ?? ""}\u0001${pipeRef.runId}`;

  const treeInput = (f: Record<string, boolean>): TreeInput | null =>
    run && shown
      ? { run, folds: f, gates, shown, canRespondApprovals: caps.can_respond_approvals, providerLabel, now }
      : null;

  // First load of a run: open and select what wants looking at (a failed run's failure, with its
  // log; a parked run's gate; a running run's first running job). Once per run.
  useEffect(() => {
    const input = treeInput({});
    if (!input || !run || initFor.current === runKey) return;
    initFor.current = runKey;
    const a = autoSelect(input);
    setFolds(a.folds);
    const nodes = flatten({ ...input, folds: a.folds });
    setSelectedId(a.selected !== null && nodes[a.selected] ? nodeId(nodes[a.selected]) : null);
    if (a.openLogs) {
      const t = failedTarget(run);
      if (t) setLogTarget({ jobId: t.jobId, step: t.step, jump: false });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run, runKey]);

  // A run's problems are fetched once it finishes and re-read whenever its status moves on. Its
  // logs are re-read once it finishes: one fetched while it ran may be missing the tail.
  useEffect(() => {
    if (!run) return;
    if (prevStatus.current && prevStatus.current !== run.status) {
      qc.invalidateQueries({ queryKey: pipelineAnnotationsKey(pipeRef) });
      if (isActive(prevStatus.current) && !active) qc.invalidateQueries({ queryKey: pipelineRunLogsKey(pipeRef) });
    }
    prevStatus.current = run.status;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run?.status]);

  const annotationsQ = usePipelineAnnotations(pipeRef, !!run && !active);
  const annotations = !active ? (annotationsQ.data ?? []) : [];
  const artifactsQ = usePipelineArtifacts(pipeRef, artifactsOpen && !active);
  const failed = run?.status === "Failed" ? failedTarget(run) : null;
  const failedLog = usePipelineLogs(pipeRef, failed?.jobId ?? null, false);
  const logSummary = useMemo(() => (failedLog.data !== undefined ? failureSummary(logLines(failedLog.data).lines) : null), [failedLog.data]);

  // ---- history and estimates (from the list rows and run details already held) ----

  const listRow = rows.find((r) => r.connection_id === pipeRef.conn && r.run.id === pipeRef.runId);
  const current: CurrentRun | null = run && shown ? { connection_id: pipeRef.conn, run, shown, branch: listRow?.run.branch } : null;
  const details = new Map<string, PipelineRun>();
  for (const [, d] of qc.getQueriesData<PipelineDetail>({ queryKey: ["pipeline-detail", pipeRef.conn] })) if (d?.run) details.set(d.run.id, d.run);
  const candidates = current && active && shown !== "Waiting" ? estimateCandidates(rows, current).filter((r) => !hasTimedJobs(r.run)) : [];
  const estimateDetails = useQueries({
    queries: candidates.map((r) => {
      const ref = { conn: r.connection_id, repo: r.run.repository, runId: r.run.id };
      return {
        queryKey: pipelineDetailKey(ref),
        queryFn: () =>
          apiGet<PipelineDetail>(`/api/pipeline/detail?conn=${encodeURIComponent(ref.conn)}&run_id=${encodeURIComponent(ref.runId)}${repoParam(ref.repo)}`),
        staleTime: Infinity,
        refetchInterval: false as const,
      };
    }),
  });
  for (const q of estimateDetails) if (q.data?.run) details.set(q.data.run.id, q.data.run);
  const history = current ? pipelineHistory(rows, current, rowShownStatus, details, now) : null;
  const estimates = current && active ? pipelineEstimates(rows, current, history, details, now) : null;
  const ghosts = run && shown && estimates ? jobGhosts({ run, shown, estimates, now }) : new Map();

  const input = treeInput(folds);
  const nodes = input ? flatten(input) : [];

  // ---- writes ----

  const refresh = () => {
    qc.invalidateQueries({ queryKey: pipelineDetailKey(pipeRef) });
    qc.invalidateQueries({ queryKey: ["pipelines"] });
    qc.invalidateQueries({ queryKey: ["launchpad"] });
  };

  /**
   * Runs a write. With `optimistic`, the cached detail is patched and the note says it's done
   * before the request goes out; without it (the provider decides, e.g. a re-run that starts a
   * new run) the note says it's under way. Refreshed either way — after a failure that refetch
   * is the rollback. `fn` may answer with its own note.
   */
  const act = async (
    label: string,
    fn: () => Promise<string | void>,
    opts: { optimistic?: () => Promise<void>; progress?: string } = {},
  ) => {
    setBusy(true);
    if (opts.optimistic) {
      setNote(`${label} ✓`);
      await opts.optimistic();
    } else {
      setNote(opts.progress ?? null);
    }
    try {
      const said = await fn();
      setNote(said ?? `${label} ✓`);
    } catch (e) {
      setNote(e instanceof Error ? e.message : String(e));
    } finally {
      refresh();
      setBusy(false);
    }
  };

  /** Patches the cached detail, and the run's row in the list so the two agree at once. Any
   *  fetch in flight is canceled first — landing after the patch, it would undo it. */
  const patchDetail = async (fn: (d: PipelineDetail) => PipelineDetail) => {
    await Promise.all([qc.cancelQueries({ queryKey: pipelineDetailKey(pipeRef) }), qc.cancelQueries({ queryKey: ["pipelines"] })]);
    const next = qc.setQueryData<PipelineDetail>(pipelineDetailKey(pipeRef), (d) => (d ? fn(d) : d));
    if (!next) return;
    qc.setQueryData<PipeRow[]>(["pipelines"], (rs) =>
      rs?.map((r) =>
        r.connection_id === pipeRef.conn && r.run.id === pipeRef.runId && (!pipeRef.repo || r.run.repository === pipeRef.repo)
          ? {
              ...r,
              approvals: next.approvals,
              run: { ...r.run, status: next.run.status, attempt: next.run.attempt, finished_at: next.run.finished_at },
            }
          : r,
      ),
    );
  };

  const rerun = (failedOnly: boolean) => {
    const startsNew = failedOnly ? caps.rerun_failed_starts_new_run : caps.rerun_starts_new_run;
    setLogTarget(null);
    act(
      failedOnly ? "Re-running failed jobs" : "Re-run started",
      async () => {
        const r = await rerunPipeline(pipeRef, failedOnly);
        if (startsNew || (r.new_run_id && r.new_run_id !== pipeRef.runId)) {
          qc.invalidateQueries({ queryKey: ["pipelines"] });
          return "Started a new run ✓";
        }
      },
      startsNew
        ? { progress: "Starting a new run…" }
        : {
            // This run becomes its next attempt — knowable here, so shown at once. (A provider
            // that starts a new run instead decides its id, so that waits for the answer.)
            // Its problems and logs belong to the attempt that ended: a job id can outlive it.
            optimistic: async () => {
              qc.removeQueries({ queryKey: pipelineAnnotationsKey(pipeRef) });
              qc.removeQueries({ queryKey: pipelineRunLogsKey(pipeRef) });
              await patchDetail((d) => markRerun(d, failedOnly));
            },
          },
    );
  };

  const cancel = () =>
    act(
      "Cancel requested",
      () => apiPost("/api/pipeline/cancel", { conn: pipeRef.conn, repo: pipeRef.repo, run_id: pipeRef.runId }).then(() => undefined),
      { optimistic: () => patchDetail(markCanceled) },
    );

  const respond = (a: PipelineApproval, decision: ApprovalDecision, comment?: string) => {
    act(
      decision === "Approve" ? `Approved ${a.name}` : `Rejected ${a.name}`,
      () => respondPipelineApproval(pipeRef, a.id, decision, comment).then(() => undefined),
      { optimistic: () => patchDetail((d) => markResponded(d, a, decision)) },
    );
  };

  // ---- tree and log callbacks ----

  const onToggle = useCallback((key: string, open: boolean) => setFolds((f) => ({ ...f, [key]: open })), []);
  const onSelect = useCallback((n: FlatNode) => {
    setSelectedId(nodeId(n));
    if (n.jobId) setLogTarget({ jobId: n.jobId, step: n.step, jump: false });
  }, []);
  const openJobLog = useCallback((jobId: string) => {
    setLogTarget({ jobId, step: null, jump: true });
    setJumpToken((t) => t + 1);
  }, []);
  const closeLog = useCallback(() => setLogTarget(null), []);
  const jumpToError = () => {
    if (!failed) return;
    setLogTarget({ jobId: failed.jobId, step: failed.step, jump: true });
    setJumpToken((t) => t + 1);
  };

  const allJobs = run ? run.stages.flatMap((s) => s.jobs) : [];
  const logJob = logTarget ? allJobs.find((j) => j.id === logTarget.jobId) : undefined;
  const logLive = logTarget && run ? isActive(logJob?.status ?? run.status) : false;

  // ---- header ----

  const meta = shown ? pipeMeta(shown) : null;
  const name = listRow?.definition_name ?? run?.name ?? run?.definition_id ?? null;
  const elapsed = run ? runSecs(run, now) : null;
  const pr = run ? prLink(run) : null;
  const parked = run && shown === "Waiting" ? parkedSplit(run, now) : null;
  let eta: ReactNode = null;
  if (run && estimates?.runMedian != null && (shown === "Running" || shown === "Queued")) {
    const left = estimates.runMedian - (elapsed ?? 0);
    eta =
      left > 0 ? (
        <>
          <span style={{ color: "var(--yellow)" }}>done in ~{fmtSecs(left)}</span>
          <span>(median of last {estimates.runN})</span>
        </>
      ) : (
        <span style={{ color: "var(--yellow)" }}>past the usual {fmtSecs(estimates.runMedian)}</span>
      );
  }

  const who = run?.triggered_by?.display_name;
  const when = run?.finished_at ? `finished ${relativeTime(run.finished_at)}` : run?.started_at ? `started ${relativeTime(run.started_at)}` : null;
  const facts = [who, when, run?.attempt != null ? `attempt ${run.attempt}` : null].filter(Boolean).join(" · ");

  const header = (
    <>
      {meta && shown && (
        <div className="flex items-center gap-1.5 shrink-0 mt-0.5">
          {meta.running && (
            <span className="spin text-xs" style={{ color: meta.color }} aria-hidden="true">
              ◐
            </span>
          )}
          <StatusBadge label={meta.running ? statusWord(shown) : `${meta.icon} ${statusWord(shown)}`} color={meta.color} />
        </div>
      )}
      <div className="flex-1 min-w-0 flex flex-col gap-1">
        <div className="flex items-baseline gap-2.5 flex-wrap">
          <span className="font-semibold" style={{ color: "var(--fg)" }}>
            {name ?? (isLoading ? "Loading…" : "Pipeline run")}
            {run?.number != null && (
              <span className="mono font-medium ml-1.5" style={{ color: "var(--dim)" }}>
                #{run.number}
              </span>
            )}
          </span>
          {elapsed !== null && (
            <span className="font-semibold" style={{ color: "var(--fg)" }} title="Elapsed">
              {fmtSecs(elapsed)}
            </span>
          )}
          {run?.event && (
            <span className="text-xs" style={{ color: "var(--dim)" }}>
              {run.event} →
            </span>
          )}
          {run?.branch && (
            <Chip title="branch">
              <span style={{ color: "var(--cyan)" }}>⑂ {run.branch}</span>
            </Chip>
          )}
        </div>
        {run && (run.commit_sha || run.title) && (
          <div className="flex items-baseline gap-2.5 text-[13px] min-w-0">
            {run.commit_sha && (
              <span className="mono text-xs shrink-0" style={{ color: "var(--magenta)" }}>
                {run.commit_sha.slice(0, 7)}
              </span>
            )}
            {run.title && (
              <span className="truncate" style={{ color: "var(--fg)" }}>
                {run.title}
              </span>
            )}
          </div>
        )}
        {run && (
          <div className="flex items-center gap-1.5 text-xs flex-wrap" style={{ color: "var(--dim)" }}>
            {who && <Avatar name={who} size={16} />}
            {facts && <span>{facts}</span>}
            {run.pull_request != null &&
              (pr ? (
                <a href={pr} target="_blank" rel="noreferrer" style={{ color: "var(--accent)" }}>
                  {facts && "· "}PR #{run.pull_request} ↗
                </a>
              ) : (
                <span>
                  {facts && "· "}PR #{run.pull_request}
                </span>
              ))}
            {(eta || parked) && <span>·</span>}
            {eta}
            {parked && (
              <span style={{ color: "var(--yellow)" }}>
                ran {fmtSecs(parked.worked)}, then waiting {fmtSecs(parked.waited)}
              </span>
            )}
          </div>
        )}
      </div>
    </>
  );

  // ---- failure line and approval banner ----

  // The TUI's `failure_line`: the test and where it broke (file name only), or with no test, where
  // it broke and the first line of what it said.
  const failAnn = annotations.find((a) => a.level === "Failure");
  const failDetail = failAnn
    ? {
        test: failAnn.title ?? null,
        location: failAnn.path ? (failAnn.line != null ? `${shortLocation(failAnn.path)}:${failAnn.line}` : shortLocation(failAnn.path)) : null,
        message: failAnn.message.split("\n")[0] ?? null,
      }
    : logSummary
      ? { test: logSummary.test, location: logSummary.location ? shortLocation(logSummary.location) : null, message: logSummary.message }
      : null;
  const failMessage = !failDetail?.test && failDetail?.message ? truncateChars(failDetail.message, 60) : null;

  const wait = run ? waitingSecs(run, now) : null;

  const gate = (supported: boolean, also?: string | null) => (supported ? (also ?? undefined) : unsupported);
  const hasFailedJobs = allJobs.some((j) => j.status === "Failed" || j.status === "Canceled");
  const artifactsLabel = active
    ? "⬇ Artifacts · none yet"
    : artifactsQ.data
      ? `⬇ Artifacts · ${artifactsQ.data.length}`
      : "⬇ Artifacts";

  return (
    <SlideOver onClose={onClose} header={header} width="5xl">
      {error && (
        <div className="p-6 text-sm" style={{ color: "var(--red)" }}>
          Couldn't load this pipeline run.
        </div>
      )}
      {!data && isLoading && (
        <div className="p-6 text-sm" style={{ color: "var(--dim)" }}>
          Loading…
        </div>
      )}

      {run && data && shown && (
        <>
          {run.status === "Failed" && (failed || failDetail) && (
            <div
              className="flex items-center gap-2 px-5 py-2 text-[13px] flex-wrap"
              style={{ background: "color-mix(in srgb, var(--red) 8%, transparent)", borderBottom: "1px solid color-mix(in srgb, var(--red) 25%, transparent)" }}
              role="status"
            >
              <span className="font-bold" style={{ color: "var(--red)" }}>
                ✗ {failed?.name ?? "Run"} failed
              </span>
              {failDetail?.test && (
                <>
                  <span style={{ color: "var(--red)" }}>·</span>
                  <span className="mono text-xs" style={{ color: "var(--fg)" }}>
                    {failDetail.test}
                  </span>
                </>
              )}
              {failDetail?.location && (
                <>
                  <span style={{ color: "var(--red)" }}>·</span>
                  <span className="mono text-xs" style={{ color: "var(--red)" }}>
                    {failDetail.location}
                  </span>
                </>
              )}
              {failMessage && (
                <>
                  <span style={{ color: "var(--red)" }}>·</span>
                  <span className="mono text-xs" style={{ color: "var(--fg)" }}>
                    {failMessage}
                  </span>
                </>
              )}
              <span className="flex-1" />
              {failed && <PaneButton label="Jump to error ↓" color="var(--red)" onClick={jumpToError} />}
            </div>
          )}

          {approvals.length > 0 && (
            <ApprovalBanner
              key={runKey}
              approvals={approvals}
              caps={caps}
              runUrl={run.url ?? null}
              providerLabel={providerLabel}
              waitSecs={wait}
              busy={busy}
              onRespond={respond}
            />
          )}
          {approvals.length === 0 && !caps.supports_approvals && shown === "Waiting" && (
            <div className="px-5 py-1.5 text-xs" style={{ color: "var(--dim)", borderBottom: "1px solid var(--border)" }}>
              Approvals not supported on {providerLabel}
            </div>
          )}

          <div className="px-5 py-4 flex flex-col gap-3.5">
            <div className="flex items-center gap-2 flex-wrap">
              {active ? (
                <PaneButton label="■ Cancel" color="var(--red)" disabled={busy} onClick={cancel} />
              ) : (
                <>
                  <PaneButton
                    label="↻ Re-run"
                    color="var(--accent)"
                    disabled={busy || !caps.supports_rerun}
                    title={gate(caps.supports_rerun, caps.rerun_starts_new_run ? "Starts a new run" : "Runs it again as the next attempt")}
                    onClick={() => rerun(false)}
                  />
                  <PaneButton
                    label="↻ Re-run failed jobs"
                    color="var(--yellow)"
                    disabled={busy || !caps.supports_rerun_failed || !hasFailedJobs}
                    title={gate(caps.supports_rerun_failed, hasFailedJobs ? "Runs the failed and canceled jobs again" : "No job failed")}
                    onClick={() => rerun(true)}
                  />
                </>
              )}
              <PaneButton
                label={artifactsLabel}
                neutral
                pressed={artifactsOpen && !active}
                disabled={!caps.supports_artifacts || active}
                title={gate(caps.supports_artifacts, active ? "Artifacts appear once the run finishes" : artifactsOpen ? "Hide artifacts" : "Show artifacts")}
                onClick={() => setArtifactsOpen((o) => !o)}
              />
              <span className="flex-1" />
              {run.url && <PaneLink href={run.url}>Open ↗</PaneLink>}
            </div>
            {note && (
              <div className="text-xs -mt-1.5" style={{ color: noteColor(note) }} role="status">
                {note}
              </div>
            )}

            {artifactsOpen && !active && caps.supports_artifacts && (
              <ArtifactsCard artifacts={artifactsQ.data} loading={artifactsQ.isLoading} error={!!artifactsQ.error} />
            )}

            {!active && annotations.length > 0 && <ProblemsCard annotations={annotations} onOpenJob={openJobLog} />}

            {history && history.entries.length >= 2 && (
              <HistoryStrip
                history={history}
                onOpen={(i) => {
                  const r = rows[i];
                  if (r) openRun({ conn: r.connection_id, repo: r.run.repository, runId: r.run.id });
                }}
              />
            )}

            <RunTimeline
              run={run}
              nodes={nodes}
              shown={shown}
              ghosts={ghosts}
              estimates={estimates}
              now={now}
              selectedId={selectedId}
              onToggle={onToggle}
              onSelect={onSelect}
              providerLabel={providerLabel}
            />

            {logTarget && (
              <LogPanel
                key={`${logTarget.jobId}|${logTarget.step ?? ""}`}
                pipeRef={pipeRef}
                jobId={logTarget.jobId}
                job={logJob}
                step={logTarget.step}
                live={logLive}
                jumpToken={logTarget.jump ? jumpToken : 0}
                onClose={closeLog}
              />
            )}
          </div>
        </>
      )}
    </SlideOver>
  );
}
