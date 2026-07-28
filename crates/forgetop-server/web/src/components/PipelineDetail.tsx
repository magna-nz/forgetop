import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { AnimatePresence } from "framer-motion";
import { useQueryClient } from "@tanstack/react-query";
import { apiGetText, apiPost, pipelineDetailKey, usePipelineDetail } from "../api";
import { pipeMeta, relativeTime } from "../format";
import type { PipeRef, PipelineApproval, PipelineJob, PipelineStage } from "../types";
import { Avatar, Chip, SlideOver, StatusBadge } from "./ui";

const cap = (s: string): string => (s.length ? s[0].toUpperCase() + s.slice(1) : s);

// ---- opener context ----

const PipeOpenerCtx = createContext<(ref: PipeRef) => void>(() => {});
export const usePipelineOpener = () => useContext(PipeOpenerCtx);

export function PipelineDetailProvider({ children }: { children: ReactNode }) {
  const [ref, setRef] = useState<PipeRef | null>(null);
  const open = useCallback((r: PipeRef) => setRef(r), []);
  return (
    <PipeOpenerCtx.Provider value={open}>
      {children}
      <AnimatePresence>{ref && <PipelineDetailPanel pipeRef={ref} onClose={() => setRef(null)} />}</AnimatePresence>
    </PipeOpenerCtx.Provider>
  );
}

// ---- panel ----

function PipelineDetailPanel({ pipeRef, onClose }: { pipeRef: PipeRef; onClose: () => void }) {
  const { data, isLoading, error } = usePipelineDetail(pipeRef);
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [selectedStage, setSelectedStage] = useState<string | null>(null);

  useEffect(() => {
    setNote(null);
    setSelectedStage(null);
  }, [pipeRef.conn, pipeRef.runId]);

  const refresh = () => {
    qc.invalidateQueries({ queryKey: pipelineDetailKey(pipeRef) });
    qc.invalidateQueries({ queryKey: ["pipelines"] });
    qc.invalidateQueries({ queryKey: ["launchpad"] });
  };

  const act = async (label: string, fn: () => Promise<void>) => {
    setBusy(true);
    setNote(null);
    try {
      await fn();
      setNote(`${label} ✓`);
      refresh();
    } catch (e) {
      setNote(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const run = data?.run;
  const meta = run ? pipeMeta(run.status) : null;
  const label = run ? run.name ?? (run.number != null ? `Run #${run.number}` : run.definition_id) : null;

  const cancel = () =>
    act("Cancel requested", () => apiPost("/api/pipeline/cancel", { conn: pipeRef.conn, repo: pipeRef.repo, run_id: pipeRef.runId }));

  const header = (
    <>
      {meta && (
        <div className="flex items-center gap-1.5 shrink-0">
          <StatusBadge label={cap(meta.label)} color={meta.color} />
          {meta.running && <span className="spin text-xs" style={{ color: meta.color }} aria-hidden="true">◐</span>}
        </div>
      )}
      <div className="flex-1 min-w-0">
        <div className="font-medium truncate" style={{ color: "var(--fg)" }}>
          {label ?? (isLoading ? "Loading…" : "Pipeline run")}
        </div>
        {run && (
          <div className="flex items-center gap-2 mt-0.5 text-xs" style={{ color: "var(--dim)" }}>
            {run.branch && <Chip title="branch">⑂ {run.branch}</Chip>}
            {run.commit_sha && <span className="mono">{run.commit_sha.slice(0, 7)}</span>}
            <span className="whitespace-nowrap">{relativeTime(run.finished_at ?? run.started_at)}</span>
          </div>
        )}
      </div>
    </>
  );

  return (
    <SlideOver onClose={onClose} header={header}>
      {error && <div className="p-6 text-sm" style={{ color: "var(--red)" }}>Couldn't load this pipeline run.</div>}

      {run && data && (
        <div className="p-5 flex flex-col gap-4 max-w-3xl">
          {(run.triggered_by || run.url) && (
            <div className="flex items-center justify-between text-xs" style={{ color: "var(--dim)" }}>
              {run.triggered_by ? (
                <div className="flex items-center gap-1.5">
                  <Avatar name={run.triggered_by.display_name} size={18} />
                  <span>Started by {run.triggered_by.display_name}</span>
                </div>
              ) : <span />}
              {run.url && (
                <a
                  href={run.url}
                  target="_blank"
                  rel="noreferrer"
                  className="rounded-md px-2 py-1 text-xs"
                  style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}
                >
                  Open ↗
                </a>
              )}
            </div>
          )}

          {(data.approvals.length > 0 || run.status === "Running" || run.status === "Queued") && (
            <div className="flex flex-wrap items-center gap-2 rounded-lg px-3 py-2.5" style={{ background: "var(--card)", border: "1px solid var(--border)" }}>
              {data.approvals.map((g) => (
                <Gate key={g.id} gate={g} />
              ))}
              {(run.status === "Running" || run.status === "Queued") && <ActBtn label="■ Cancel" color="var(--red)" disabled={busy} onClick={cancel} />}
            </div>
          )}

          <div className="flex flex-col gap-4">
            {run.stages.length === 0 && <div className="text-sm" style={{ color: "var(--dim)" }}>No stages to show yet.</div>}

            {/* Providers that expose real ordered stages (Azure timeline, GitLab stages) get the
                connected "plan" flow. GitHub Actions / Bitbucket return one flat group of jobs/steps
                (no runtime DAG without parsing the pipeline YAML), so they just show the job list. */}
            {run.stages.length > 1 && (
              <>
                <StageFlow stages={run.stages} selected={selectedStage ?? run.stages[0].name} onSelect={setSelectedStage} />
                <div>
                  <SectionLabel>Plan Steps</SectionLabel>
                  <div className="rounded-lg overflow-hidden" style={{ border: "1px solid var(--border)" }}>
                    {(run.stages.find((stage) => stage.name === (selectedStage ?? run.stages[0].name)) ?? run.stages[0]).jobs.map((job) => (
                      <Job key={job.id} job={job} pipeRef={pipeRef} />
                    ))}
                  </div>
                </div>
              </>
            )}

            {run.stages.length === 1 ? (
              <div>
                <SectionLabel>{cap(run.stages[0].name)}</SectionLabel>
                <div className="rounded-lg overflow-hidden" style={{ border: "1px solid var(--border)" }}>
                  {run.stages[0].jobs.map((job) => (
                    <Job key={job.id} job={job} pipeRef={pipeRef} />
                  ))}
                </div>
              </div>
            ) : null}
          </div>

          {note && (
            <div className="text-xs" style={{ color: note.endsWith("✓") ? "var(--green)" : "var(--red)" }}>
              {note}
            </div>
          )}
        </div>
      )}
    </SlideOver>
  );
}

// ---- stage flow ("the plan") ----

function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <div className="text-[11px] font-semibold uppercase tracking-wider mb-2" style={{ color: "var(--dim)" }}>
      {children}
    </div>
  );
}

/** The pipeline "plan": ordered stages as connected, status-tinted nodes. Shown for providers that
 *  expose real stages (Azure / GitLab). Sequence + live state at a glance; scrolls if it overflows. */
function StageFlow({ stages, selected, onSelect }: { stages: PipelineStage[]; selected: string; onSelect: (stage: string) => void }) {
  return (
    <div className="rounded-lg p-3" style={{ background: "var(--card)", border: "1px solid var(--border)" }}>
      <SectionLabel>Plan</SectionLabel>
      <div className="flex items-center overflow-x-auto pb-1">
        {stages.map((stage, i) => {
          const m = pipeMeta(stage.status);
          return (
            <div key={stage.name} className="flex items-center shrink-0">
              <button
                type="button"
                onClick={() => onSelect(stage.name)}
                className="flex items-center gap-1.5 rounded-md px-2.5 py-1.5 whitespace-nowrap"
                style={{
                  color: m.color,
                  background: `color-mix(in srgb, ${m.color} 12%, transparent)`,
                  border: `${selected === stage.name ? "2px" : "1px"} solid color-mix(in srgb, ${m.color} ${selected === stage.name ? "75%" : "30%"}, transparent)`,
                  boxShadow: selected === stage.name ? `0 0 0 2px color-mix(in srgb, ${m.color} 20%, transparent)` : undefined,
                }}
                title={`${stage.name} — ${m.label}`}
              >
                <span className={m.running ? "spin" : undefined} aria-hidden="true">{m.icon}</span>
                <span className="text-sm font-medium">{stage.name}</span>
              </button>
              {i < stages.length - 1 && (
                <div
                  className="shrink-0 h-0.5 w-6 mx-1 rounded-full"
                  style={{ background: stage.status === "Succeeded" ? "var(--green)" : "var(--border)" }}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ---- job (expandable, with lazy logs) ----

function Job({ job, pipeRef }: { job: PipelineJob; pipeRef: PipeRef }) {
  const [open, setOpen] = useState(false);
  const [logs, setLogs] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [logErr, setLogErr] = useState<string | null>(null);
  const meta = pipeMeta(job.status);

  useEffect(() => {
    if (open && logs === null && !loading && !logErr) {
      setLoading(true);
      apiGetText(`/api/pipeline/logs?conn=${encodeURIComponent(pipeRef.conn)}&run_id=${encodeURIComponent(pipeRef.runId)}&job=${encodeURIComponent(job.id)}${pipeRef.repo ? `&repo=${encodeURIComponent(pipeRef.repo)}` : ""}`)
        .then(setLogs)
        .catch((e) => setLogErr(e instanceof Error ? e.message : String(e)))
        .finally(() => setLoading(false));
    }
  }, [open, logs, loading, logErr, pipeRef.conn, pipeRef.runId, job.id]);

  return (
    <div style={{ borderTop: "1px solid var(--border)" }}>
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-2 px-3 py-2 text-sm text-left"
        style={{ color: "var(--fg)" }}
        onMouseEnter={(e) => (e.currentTarget.style.background = "var(--card-hover)")}
        onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
      >
        <span className="text-xs" style={{ color: "var(--dim)" }}>{open ? "▾" : "▸"}</span>
        <span style={{ color: meta.color }}>{meta.icon}</span>
        <span className="flex-1 truncate">{job.name}</span>
        {job.problem && <span className="text-xs truncate" style={{ color: "var(--red)" }}>{job.problem}</span>}
      </button>
      {open && (
        <div className="px-3 pb-3">
          {job.steps.length > 0 && (
            <div className="flex flex-col gap-0.5 mb-2 text-xs">
              {job.steps.map((s) => (
                <div key={s.name} className="flex items-center gap-2">
                  <span style={{ color: pipeMeta(s.status).color }}>{pipeMeta(s.status).icon}</span>
                  <span style={{ color: "var(--dim)" }}>{s.name}</span>
                </div>
              ))}
            </div>
          )}
          {loading && <div className="text-xs" style={{ color: "var(--dim)" }}>Loading logs…</div>}
          {logErr && <div className="text-xs" style={{ color: "var(--red)" }}>Couldn't load logs.</div>}
          {logs != null && (
            <pre className="mono text-xs rounded p-3 overflow-x-auto whitespace-pre" style={{ background: "var(--panel)", border: "1px solid var(--border)", color: "var(--fg)" }}>
              {logs}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

// ---- approval gate ----

function Gate({ gate }: { gate: PipelineApproval }) {
  // Approvals aren't available across all providers, so a pending gate is surfaced as info
  // (matching the list's "Approval needed" badge) rather than an approve/reject action.
  return (
    <span className="text-xs" style={{ color: "var(--yellow)" }}>⏳ {gate.name} — approval needed</span>
  );
}

function ActBtn({ label, color, onClick, disabled }: { label: string; color: string; onClick: () => void; disabled?: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="rounded px-2 py-0.5 text-xs font-medium"
      style={{ color, border: `1px solid ${color}`, background: "transparent", opacity: disabled ? 0.5 : 1, cursor: disabled ? "not-allowed" : "pointer" }}
    >
      {label}
    </button>
  );
}
