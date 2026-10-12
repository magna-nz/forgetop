// The run pane's state helpers: what a write does to the cached run detail before the provider
// answers (the TUI's `mark_rerun` / `mark_canceled`), and small pane-wide utilities.
import { useEffect, useState } from "react";
import { Icon } from "../icons";
import { gateStage, isActive, rowGates, shownStatus } from "../../pipeline/grouping";
import type { ApprovalDecision, PipelineApproval, PipelineCapabilities, PipelineDetail, PipelineRun, PipelineRunStatus } from "../../types";

/** What a detail from an older server (no `capabilities`) supports: nothing optional. */
export const NO_CAPS: PipelineCapabilities = {
  supports_rerun: false,
  supports_rerun_failed: false,
  rerun_starts_new_run: false,
  rerun_failed_starts_new_run: false,
  supports_artifacts: false,
  supports_approvals: false,
  can_respond_approvals: false,
};

/** A clock that ticks every second while `ticking` (an in-flight run's elapsed times). */
export function useNow(ticking: boolean): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    if (!ticking) return;
    setNow(new Date());
    const t = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(t);
  }, [ticking]);
  return now;
}

/** The pull request a run was built for, as a link derived from the run's own URL. */
export function prLink(run: PipelineRun): string | null {
  if (run.pull_request == null || !run.url) return null;
  const n = run.pull_request;
  let m = /^(.*?)\/actions\/runs\//.exec(run.url);
  if (m) return `${m[1]}/pull/${n}`;
  m = /^(.*?)\/-\/pipelines\//.exec(run.url);
  if (m) return `${m[1]}/-/merge_requests/${n}`;
  m = /^(.*?)\/(?:addon\/pipelines\/home#!\/results|pipelines\/results)\//.exec(run.url);
  if (m) return `${m[1]}/pull-requests/${n}`;
  return null;
}

const resetStatus = <T extends { status: PipelineRunStatus; started_at?: string | null; finished_at?: string | null }>(x: T): T => ({
  ...x,
  status: "Queued",
  started_at: null,
  finished_at: null,
});

/** A re-run as the provider will report it: queued again as the next attempt — every job, or
 *  only the failed and canceled ones. */
export function markRerun(d: PipelineDetail, failedOnly: boolean): PipelineDetail {
  const again = (s: PipelineRunStatus) => !failedOnly || s === "Failed" || s === "Canceled";
  const stages = d.run.stages.map((st) => {
    let touched = false;
    const jobs = st.jobs.map((j) => {
      if (!again(j.status)) return j;
      touched = true;
      return { ...resetStatus(j), problem: null, steps: j.steps.map(resetStatus) };
    });
    return touched || !failedOnly ? { ...st, status: "Queued" as const, jobs } : st;
  });
  const attempt = d.run.attempt != null ? d.run.attempt + 1 : d.run.attempt;
  return { ...d, run: { ...d.run, status: "Queued", attempt, finished_at: null, stages } };
}

/** A canceled run: everything still in flight stops, and its gates will never open. */
export function markCanceled(d: PipelineDetail): PipelineDetail {
  const stop = <T extends { status: PipelineRunStatus }>(x: T): T => (isActive(x.status) ? { ...x, status: "Canceled" } : x);
  return {
    ...d,
    approvals: [],
    run: {
      ...stop(d.run),
      finished_at: d.run.finished_at ?? new Date().toISOString(),
      stages: d.run.stages.map((s) => ({ ...stop(s), jobs: s.jobs.map((j) => ({ ...stop(j), steps: j.steps.map(stop) })) })),
    },
  };
}

/** An answered gate goes away; an approved one lets its stage queue and the run go on. */
export function markResponded(d: PipelineDetail, approval: PipelineApproval, decision: ApprovalDecision): PipelineDetail {
  const approvals = d.approvals.filter((a) => a.id !== approval.id);
  if (decision !== "Approve") return { ...d, approvals };
  const shown = shownStatus(d.run, d.approvals.some((a) => a.blocks_run));
  const gate = gateStage(d.run, shown, rowGates(d));
  const stages = d.run.stages.map((s, i) => (i === gate && (s.status === "Waiting" || s.status === "Queued") ? { ...s, status: "Queued" as const } : s));
  return { ...d, approvals, run: { ...d.run, status: d.run.status === "Waiting" ? "Running" : d.run.status, stages } };
}

/** What the last write said: done (or optimistically assumed done), still under way, or failed. */
export interface Note {
  text: string;
  tone: "ok" | "progress" | "error";
}

export const noteColor = (n: Note) => (n.tone === "ok" ? "var(--green)" : n.tone === "progress" ? "var(--fg3)" : "var(--red)");

/** The note as one 12px line with its glyph. */
export function NoteLine({ note, className }: { note: Note; className?: string }) {
  return (
    <span role="status" className={`inline-flex items-center gap-1.5 text-[12px] min-w-0 ${className ?? ""}`} style={{ color: noteColor(note) }}>
      <Icon name={note.tone === "ok" ? "check" : note.tone === "progress" ? "running" : "x-circle"} size={13} spin={note.tone === "progress"} />
      <span className="truncate">{note.text}</span>
    </span>
  );
}
