import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
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
import { fmtSecs, pipeMeta, pipeTitle, providerMeta, relativeTime } from "../format";
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
import { ArtifactsCard, ExternalIconLink, ProblemsCard, statusWord } from "./pipeline/parts";
import { HistoryStrip, historyDelta } from "./pipeline/RunHistory";
import { nodeId, RunTimeline, StageStrip } from "./pipeline/RunTimeline";
import { markCanceled, markRerun, markResponded, NO_CAPS, NoteLine, prLink, useNow, type Note } from "./pipeline/runState";
import { Icon } from "./icons";
import { Avatar, Button, Chip, claimPane, PaneHeader, SlideOver, StatusGlyph, usePaneRegistration } from "./ui";

/** At most `max` characters, the last an ellipsis when cut (the TUI's `truncate`). */
const truncateChars = (s: string, max: number) => {
  const cs = Array.from(s);
  return cs.length > max ? `${cs.slice(0, max - 1).join("")}…` : s;
};

const Sep = () => <span style={{ color: "var(--fg3)" }}>·</span>;

// ---- opener context ----

/** What the pane does once the run has loaded, beyond opening: `showLog` lands on the failing
 *  job's log (a row's "Show failing log"). Applied again on every open, so it works on a run that
 *  is already open and whose log the user has since left. */
export interface PipeOpenOptions {
  showLog?: boolean;
}
const PipeOpenerCtx = createContext<(ref: PipeRef, opts?: PipeOpenOptions) => void>(() => {});
export const usePipelineOpener = () => useContext(PipeOpenerCtx);
/** The run open in the pane, so a list can mark its row as selected. */
const PipeCurrentCtx = createContext<PipeRef | null>(null);
export const useCurrentPipeline = () => useContext(PipeCurrentCtx);

export function PipelineDetailProvider({ children }: { children: ReactNode }) {
  const [ref, setRef] = useState<PipeRef | null>(null);
  // Bumped on each open that asks for the log, so the panel applies it once per request.
  const [showLogReq, setShowLogReq] = useState(0);
  const close = useCallback(() => setRef(null), []);
  usePaneRegistration("pipeline", close);
  // One pane at a time: opening a run closes a PR or work-item pane (after its guard, if any).
  const open = useCallback((r: PipeRef, opts?: PipeOpenOptions) => {
    if (!claimPane("pipeline")) return;
    setRef(r);
    if (opts?.showLog) setShowLogReq((n) => n + 1);
  }, []);
  return (
    <PipeOpenerCtx.Provider value={open}>
      <PipeCurrentCtx.Provider value={ref}>
        {children}
        {/* Keyed by run: another run gets a fresh panel, so no frame shows the last run's state. */}
        {ref && <PipelineDetailPanel key={`${ref.conn}|${ref.repo ?? ""}|${ref.runId}`} pipeRef={ref} showLogReq={showLogReq} onClose={close} />}
      </PipeCurrentCtx.Provider>
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

function PipelineDetailPanel({ pipeRef, showLogReq, onClose }: { pipeRef: PipeRef; showLogReq: number; onClose: () => void }) {
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
  const [note, setNote] = useState<Note | null>(null);
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

  // Opened with "Show failing log": land on the failing job's log at its first error, once per
  // request and once the run is here. Nothing to show on a run that has not failed.
  const appliedShowLog = useRef(0);
  useEffect(() => {
    if (!run || showLogReq === 0 || appliedShowLog.current === showLogReq) return;
    appliedShowLog.current = showLogReq;
    const t = failedTarget(run);
    if (t) {
      setLogTarget({ jobId: t.jobId, step: t.step, jump: true });
      setJumpToken((n) => n + 1);
    }
  }, [run, showLogReq]);

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
      setNote({ text: label, tone: "ok" });
      await opts.optimistic();
    } else {
      setNote(opts.progress ? { text: opts.progress, tone: "progress" } : null);
    }
    try {
      const said = await fn();
      setNote({ text: said ?? label, tone: "ok" });
    } catch (e) {
      setNote({ text: e instanceof Error ? e.message : String(e), tone: "error" });
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
          return "Started a new run";
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
  const title = run ? pipeTitle(run, listRow?.definition_name) : null;
  const elapsed = run ? runSecs(run, now) : null;
  const pr = run ? prLink(run) : null;
  const parked = run && shown === "Waiting" ? parkedSplit(run, now) : null;
  let eta: ReactNode = null;
  if (run && estimates?.runMedian != null && (shown === "Running" || shown === "Queued")) {
    const left = estimates.runMedian - (elapsed ?? 0);
    eta =
      left > 0 ? (
        <span>
          <span className="num" style={{ color: "var(--yellow)" }}>
            done in ~{fmtSecs(left)}
          </span>{" "}
          <span style={{ color: "var(--fg3)" }}>(median of last {estimates.runN})</span>
        </span>
      ) : (
        <span className="num" style={{ color: "var(--yellow)" }}>
          past the usual {fmtSecs(estimates.runMedian)}
        </span>
      );
  }
  const delta = history ? historyDelta(history) : null;

  const who = run?.triggered_by?.display_name;
  const when = run?.finished_at ? `finished ${relativeTime(run.finished_at)}` : run?.started_at ? `started ${relativeTime(run.started_at)}` : null;

  // The second line: state · workflow · who and what triggered it · commit · time · median.
  const facts: ReactNode[] = [];
  if (run && meta && shown) {
    facts.push(
      <span key="state" className="font-medium" style={{ color: meta.color }}>
        {statusWord(shown)}
      </span>,
    );
    if (name && name !== title) facts.push(<span key="name">{name}</span>);
    if (who || run.event || run.branch)
      facts.push(
        <span key="who" className="inline-flex items-center gap-1.5 min-w-0">
          {who && <Avatar name={who} size={18} />}
          {who && <span>{who}</span>}
          {run.event && <span>{run.branch ? `${run.event} to` : run.event}</span>}
          {run.branch && <Chip title="branch">{run.branch}</Chip>}
        </span>,
      );
    if (run.commit_sha)
      facts.push(
        <span key="sha" className="mono" title={run.commit_sha}>
          {run.commit_sha.slice(0, 7)}
        </span>,
      );
    if (elapsed !== null)
      facts.push(
        <span key="elapsed" className="inline-flex items-center gap-1">
          {active && <span>running</span>}
          <span className="num" style={{ color: "var(--fg)" }} title="Elapsed">
            {fmtSecs(elapsed)}
          </span>
        </span>,
      );
    if (history?.medianSecs != null)
      facts.push(
        <span key="median" className="num">
          median {fmtSecs(history.medianSecs)}
          {delta && <span style={{ color: delta.color }}> ({delta.text})</span>}
        </span>,
      );
    if (when) facts.push(<span key="when">{when}</span>);
    if (run.attempt != null && run.attempt > 1) facts.push(<span key="attempt">attempt {run.attempt}</span>);
    if (run.pull_request != null)
      facts.push(
        pr ? (
          <a key="pr" href={pr} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1" style={{ color: "var(--accent-fg)" }}>
            PR <span className="mono">#{run.pull_request}</span>
            <Icon name="external" size={12} />
          </a>
        ) : (
          <span key="pr">
            PR <span className="mono">#{run.pull_request}</span>
          </span>
        ),
      );
    if (eta) facts.push(<span key="eta">{eta}</span>);
    if (parked)
      facts.push(
        <span key="parked" className="num" style={{ color: "var(--yellow)" }}>
          ran {fmtSecs(parked.worked)}, then waiting {fmtSecs(parked.waited)}
        </span>,
      );
  }

  const header = (
    <PaneHeader
      glyph={meta?.glyph}
      color={meta?.color}
      title={title ?? (isLoading ? "Loading…" : "Pipeline run")}
      ident={run?.number != null ? `#${run.number}` : undefined}
      actions={run?.url ? <ExternalIconLink href={run.url} label={`Open in ${providerLabel}`} /> : undefined}
    >
      {facts.length > 0 &&
        facts.flatMap((f, i) => (i === 0 ? [f] : [<Sep key={`sep${i}`} />, f]))}
    </PaneHeader>
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
  const artifactsLabel = active ? "Artifacts · none yet" : artifactsQ.data ? `Artifacts · ${artifactsQ.data.length}` : "Artifacts";

  const showHistory = !!history && history.entries.length >= 2;
  const footer =
    run && data && shown ? (
      <>
        {showHistory && history && (
          <HistoryStrip
            history={history}
            onOpen={(i) => {
              const r = rows[i];
              if (r) openRun({ conn: r.connection_id, repo: r.run.repository, runId: r.run.id });
            }}
          />
        )}
        {note && <NoteLine note={note} className={showHistory ? "ml-3" : undefined} />}
        <span className="ml-auto flex items-center gap-1.5 shrink-0">
          <Button
            quiet
            aria-pressed={artifactsOpen && !active}
            disabled={!caps.supports_artifacts || active}
            title={gate(caps.supports_artifacts, active ? "Artifacts appear once the run finishes" : artifactsOpen ? "Hide artifacts" : "Show artifacts")}
            onClick={() => setArtifactsOpen((o) => !o)}
          >
            {artifactsLabel}
          </Button>
          {active ? (
            <Button danger disabled={busy} onClick={cancel}>
              Cancel
            </Button>
          ) : (
            <>
              <Button
                quiet
                disabled={busy || !caps.supports_rerun}
                title={gate(caps.supports_rerun, caps.rerun_starts_new_run ? "Starts a new run" : "Runs it again as the next attempt")}
                onClick={() => rerun(false)}
              >
                Re-run
              </Button>
              <Button
                quiet
                disabled={busy || !caps.supports_rerun_failed || !hasFailedJobs}
                title={gate(caps.supports_rerun_failed, hasFailedJobs ? "Runs the failed and canceled jobs again" : "No job failed")}
                onClick={() => rerun(true)}
              >
                Re-run failed jobs
              </Button>
            </>
          )}
        </span>
      </>
    ) : undefined;

  return (
    <SlideOver onClose={onClose} header={header} footer={footer} width="xl" label="Pipeline run">
      {error && (
        <div className="px-5 py-6 text-[13px]" style={{ color: "var(--red)" }}>
          Couldn't load this pipeline run.
        </div>
      )}
      {!data && isLoading && (
        <div className="px-5 py-6 text-[13px]" style={{ color: "var(--fg3)" }}>
          Loading…
        </div>
      )}

      {run && data && shown && (
        <div className="px-5 pt-4 pb-5 flex flex-col gap-4">
          {run.status === "Failed" && (failed || failDetail) && (
            <div
              className="flex items-center gap-2 px-3 py-2.5 text-[12.5px] flex-wrap"
              style={{
                borderRadius: 6,
                border: "1px solid color-mix(in srgb, var(--red) 35%, transparent)",
                background: "color-mix(in srgb, var(--red) 6%, transparent)",
              }}
              role="status"
            >
              <StatusGlyph glyph="x-circle" color="var(--red)" />
              <span className="font-medium" style={{ color: "var(--fg)" }}>
                {failed?.name ?? "Run"} failed
              </span>
              {failDetail?.test && (
                <>
                  <Sep />
                  <span className="mono" style={{ color: "var(--fg)" }}>
                    {failDetail.test}
                  </span>
                </>
              )}
              {failDetail?.location && (
                <>
                  <Sep />
                  <span className="mono" style={{ color: "var(--red)" }}>
                    {failDetail.location}
                  </span>
                </>
              )}
              {failMessage && (
                <>
                  <Sep />
                  <span className="mono" style={{ color: "var(--fg2)" }}>
                    {failMessage}
                  </span>
                </>
              )}
              {failed && (
                <Button quiet sm className="ml-auto" style={{ color: "var(--red)" }} onClick={jumpToError}>
                  Jump to error
                </Button>
              )}
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
            <div className="text-[12px]" style={{ color: "var(--fg3)" }}>
              Approvals not supported on {providerLabel}
            </div>
          )}

          <StageStrip run={run} nodes={nodes} />

          {artifactsOpen && !active && caps.supports_artifacts && (
            <ArtifactsCard artifacts={artifactsQ.data} loading={artifactsQ.isLoading} error={!!artifactsQ.error} />
          )}

          {!active && annotations.length > 0 && <ProblemsCard annotations={annotations} onOpenJob={openJobLog} />}

          <div className="-mx-2">
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
          </div>

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
      )}
    </SlideOver>
  );
}
