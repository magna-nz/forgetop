// A run's history strip and its in-flight estimates, ported from the TUI
// (crates/forgetop-tui/src/app.rs `pipeline_history`, `estimate_candidates`,
// `pipeline_estimates`, `has_timed_jobs`). Built from the list rows already held — no fetch —
// plus whatever run details the caller has (list rows usually carry no stages).

import type { PipelineRun, PipelineRunStatus, PipeRow } from "../types";
import { pipeDefinitionName, rowShownStatus } from "./grouping";
import { median } from "./runlog";
import { firstFailedNode, msOf, runSecs, type Estimates } from "./timeline";

export type { Estimates };

/** Most runs the history strip shows. */
export const HISTORY_LEN = 10;
/** Recent succeeded runs whose job times feed the per-job estimates. */
export const ESTIMATE_RUNS = 3;

/** One run in a pipeline's history strip. */
export interface HistEntry {
  runId: string;
  number: number | null;
  status: PipelineRunStatus;
  /** Elapsed seconds, when the run has started (running runs count to now). */
  secs: number | null;
  startedAt: string | null;
  /** Index into the `rows` passed in, to open the run. */
  row: number;
}

/** The most recent failure among a run's history. */
export interface LastFailure {
  number: number | null;
  /** When it finished (else started), ISO. */
  at: string | null;
  /** The failed step (or job) when that run's stages are known. */
  step: string | null;
}

/** Recent runs of the same pipeline on the same branch, oldest first, with the open run marked. */
export interface RunHistory {
  /** The pipeline's name, for the panel title. */
  name: string;
  entries: HistEntry[];
  /** The open run's position in `entries`. */
  current: number | null;
  /** Median elapsed time of the succeeded runs among all of this branch's rows, and how many. */
  medianSecs: number | null;
  medianN: number;
  /** How many runs of this pipeline on this branch the list holds (the strip shows ≤ 10). */
  total: number;
  /** The branch had fewer than two runs, so this is the pipeline's history on every branch. */
  allBranches: boolean;
  lastFailure: LastFailure | null;
}

/** The open run: its connection, the run as the drill-in holds it (detail, with stages), its
 *  shown status (defaults to `run.status`) and the list row's branch, for a run that names none. */
export interface CurrentRun {
  connection_id: string;
  run: PipelineRun;
  shown?: PipelineRunStatus;
  branch?: string | null;
}

const samePipeline = (p: PipeRow, cur: CurrentRun) =>
  p.connection_id === cur.connection_id &&
  p.run.definition_id === cur.run.definition_id &&
  (p.run.repository ?? null) === (cur.run.repository ?? null);

/** Rust's `Option` ordering: `None` before any `Some`. */
const cmpOpt = (a: number | null, b: number | null) => (a === b ? 0 : a === null ? -1 : b === null ? 1 : a - b);

/** The run's history strip (the TUI's `App::pipeline_history`). `details` holds run details
 *  already fetched, by run id — read for the last failure's failed step. `null` when the list
 *  holds no run of this pipeline. */
export function pipelineHistory(
  rows: PipeRow[],
  current: CurrentRun,
  shownStatusOf: (row: PipeRow) => PipelineRunStatus = rowShownStatus,
  details: Map<string, PipelineRun> = new Map(),
  now: Date = new Date(),
): RunHistory | null {
  const v = current;
  const branch = v.run.branch ?? v.branch ?? null;
  const distinct = (idx: number[]) => new Set(idx.map((i) => rows[i].run.id)).size;
  const indices = rows.map((_, i) => i);
  let idx = indices.filter((i) => samePipeline(rows[i], v) && (rows[i].run.branch ?? null) === branch);
  let allBranches = false;
  if (distinct(idx) < 2) {
    const all = indices.filter((i) => samePipeline(rows[i], v));
    if (distinct(all) > distinct(idx)) {
      idx = all;
      allBranches = true;
    }
  }
  if (idx.length === 0) return null;
  idx.sort((a, b) => {
    const [x, y] = [rows[a].run, rows[b].run];
    return cmpOpt(msOf(x.started_at), msOf(y.started_at)) || cmpOpt(x.number ?? null, y.number ?? null);
  });
  const seen = new Set<string>();
  idx = idx.filter((i) => {
    const id = rows[i].run.id;
    if (seen.has(id)) return false;
    seen.add(id);
    return true;
  });

  const runOf = (i: number) => (rows[i].run.id === v.run.id ? v.run : rows[i].run);
  const posAt = idx.findIndex((i) => rows[i].run.id === v.run.id);
  const pos = posAt < 0 ? null : posAt;
  const start = Math.min(Math.max(0, idx.length - HISTORY_LEN), pos ?? Infinity);
  const window = idx.slice(start, Math.min(start + HISTORY_LEN, idx.length));
  const entries: HistEntry[] = window.map((i) => {
    const run = runOf(i);
    const status = run.id === v.run.id ? (v.shown ?? v.run.status) : shownStatusOf(rows[i]);
    return { runId: run.id, number: run.number ?? null, status, secs: runSecs(run, now), startedAt: run.started_at ?? null, row: i };
  });
  const succeeded = idx
    .map(runOf)
    .filter((r) => r.status === "Succeeded" && r.id !== v.run.id)
    .map((r) => runSecs(r, now))
    .filter((s): s is number => s !== null);
  const failedRow = [...idx]
    .reverse()
    .map((i) => rows[i])
    .find((p) => p.run.status === "Failed" && p.run.id !== v.run.id);
  let lastFailure: LastFailure | null = null;
  if (failedRow) {
    const run = details.get(failedRow.run.id) ?? failedRow.run;
    const f = firstFailedNode(run);
    let step: string | null = null;
    if (f) {
      const job = run.stages[f[0]]?.jobs[f[1]];
      if (job) step = f[2] !== null && job.steps[f[2]] ? job.steps[f[2]].name : job.name;
    }
    lastFailure = { number: failedRow.run.number ?? null, at: failedRow.run.finished_at ?? failedRow.run.started_at ?? null, step };
  }
  return {
    name: pipeDefinitionName(rows[idx[0]]),
    entries,
    current: pos === null ? null : pos - start,
    medianSecs: median(succeeded),
    medianN: succeeded.length,
    total: idx.length,
    allBranches,
    lastFailure,
  };
}

/** Whether a listed run already carries timed jobs, so its detail needn't be fetched for an
 *  estimate. */
export function hasTimedJobs(run: PipelineRun): boolean {
  return run.stages.some((s) => s.jobs.some((j) => msOf(j.started_at) !== null && msOf(j.finished_at) !== null));
}

/** Recent succeeded runs of the open run's pipeline (any branch), newest first, de-duplicated,
 *  at most `ESTIMATE_RUNS` — the runs whose job times feed the estimates (fetch their details
 *  when `hasTimedJobs` says the row lacks them). */
export function estimateCandidates(rows: PipeRow[], current: CurrentRun): PipeRow[] {
  const picked = rows
    .filter((p) => samePipeline(p, current) && p.run.status === "Succeeded" && p.run.id !== current.run.id)
    .sort((a, b) => cmpOpt(msOf(b.run.started_at), msOf(a.run.started_at)));
  const seen = new Set<string>();
  return picked
    .filter((p) => {
      if (seen.has(p.run.id)) return false;
      seen.add(p.run.id);
      return true;
    })
    .slice(0, ESTIMATE_RUNS);
}

/** How long the open (in-flight) run and its jobs should take (the TUI's
 *  `App::pipeline_estimates`; it computes them only while the run is active). */
export function pipelineEstimates(
  rows: PipeRow[],
  current: CurrentRun,
  history: RunHistory | null,
  details: Map<string, PipelineRun> = new Map(),
  now: Date = new Date(),
): Estimates {
  const e: Estimates = { runMedian: null, runN: 0, jobs: new Map(), jobRuns: 0 };
  if (history && history.medianN > 0) {
    e.runMedian = history.medianSecs;
    e.runN = history.medianN;
  } else {
    const secs = rows
      .filter((p) => samePipeline(p, current) && p.run.status === "Succeeded" && p.run.id !== current.run.id)
      .map((p) => runSecs(p.run, now))
      .filter((s): s is number => s !== null);
    e.runMedian = median(secs);
    e.runN = secs.length;
  }
  const perJob = new Map<string, number[]>();
  for (const row of estimateCandidates(rows, current)) {
    const run = details.get(row.run.id) ?? (hasTimedJobs(row.run) ? row.run : null);
    if (!run) continue;
    e.jobRuns += 1;
    for (const job of run.stages.flatMap((s) => s.jobs)) {
      const [a, b] = [msOf(job.started_at), msOf(job.finished_at)];
      if (a !== null && b !== null) {
        const list = perJob.get(job.name) ?? [];
        list.push(Math.max(0, Math.trunc((b - a) / 1000)));
        perJob.set(job.name, list);
      }
    }
  }
  for (const [name, secs] of perJob) {
    const m = median(secs);
    if (m !== null) e.jobs.set(name, m);
  }
  return e;
}
