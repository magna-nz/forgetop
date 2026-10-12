// The run drill-in's tree and timeline, ported from the TUI: `PipelineView::flatten` and its
// helpers (crates/forgetop-tui/src/app.rs) and the timeline's axis, estimates and notes
// (crates/forgetop-tui/src/ui.rs `TimeAxis`, `render_timeline`, `job_ghosts`,
// `critical_path_hint`, `slowest_note`, `tree_title`). Pure functions; the dashboard draws them.
//
// Times: a `FlatNode` carries the DTO's ISO strings; every other time here is epoch
// milliseconds, and every duration whole seconds (chrono's `num_seconds`, truncated).

import { axisLabel, fmtSecs } from "../format";
import type { PipelineJob, PipelineRun, PipelineRunStatus, PipelineStage, PipelineStep } from "../types";
import { eqIgnoreAsciiCase, gateStage, isActive } from "./grouping";

export { gateStage };

/** Everything the tree is built from — the TUI's `PipelineView` fields that `flatten` reads. */
export interface TreeInput {
  run: PipelineRun;
  /** The user's fold choices by node key (`s0`, `s0.j1`, `s0.j1.pass`, `s0.j1.post`, `s2.skip`);
   *  a key not present takes its default. */
  folds: Record<string, boolean>;
  /** Names of the pending gates that block the run (`rowGates`). */
  gates: string[];
  /** The run's shown status (`shownStatus`). */
  shown: PipelineRunStatus;
  canRespondApprovals: boolean;
  /** The provider's name as a person writes it (`Azure DevOps`), for the "open in …" hint. */
  providerLabel: string;
  now: Date;
}

export type Tone = "neutral" | "warn" | "bad";

/** One row of the flattened, collapsible pipeline tree. Mirrors app.rs `FlatNode`. */
export interface FlatNode {
  depth: number;
  label: string;
  status: PipelineRunStatus;
  /** Collapse key when the node has children; `null` for leaves. */
  key: string | null;
  expanded: boolean;
  /** Elapsed time, only for completed nodes (`3m12s`). */
  duration: string | null;
  /** Short failure summary for failed jobs. */
  problem: string | null;
  url: string | null;
  /** The job whose logs this node maps to; `null` for stages. */
  jobId: string | null;
  startedAt: string | null;
  finishedAt: string | null;
  stage: number;
  job: number | null;
  step: number | null;
  /** A folded run of steps or of skipped stages, not a single node. */
  group: boolean;
  /** A folded stage's roll-up (`3 jobs`, or the problem its jobs report). */
  summary: { text: string; tone: Tone } | null;
  /** Text for a row with no span: `after Deploy Prod`, `Build failed`. */
  note: string | null;
  /** An explanatory line (under a gated stage), not a node of the run. */
  info: boolean;
}

function bare(depth: number, label: string, status: PipelineRunStatus, stage: number): FlatNode {
  return {
    depth,
    label,
    status,
    key: null,
    expanded: false,
    duration: null,
    problem: null,
    url: null,
    jobId: null,
    startedAt: null,
    finishedAt: null,
    stage,
    job: null,
    step: null,
    group: false,
    summary: null,
    note: null,
    info: false,
  };
}

// ---- time helpers ----

export const msOf = (iso?: string | null): number | null => {
  if (!iso) return null;
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : t;
};

const secsBetween = (from: number, to: number) => Math.max(0, Math.trunc((to - from) / 1000));

function minIso(xs: (string | null | undefined)[]): string | null {
  let best: string | null = null;
  let bestT = Infinity;
  for (const x of xs) {
    const t = msOf(x);
    if (t !== null && t < bestT) [best, bestT] = [x!, t];
  }
  return best;
}

function maxIso(xs: (string | null | undefined)[]): string | null {
  let best: string | null = null;
  let bestT = -Infinity;
  for (const x of xs) {
    const t = msOf(x);
    if (t !== null && t > bestT) [best, bestT] = [x!, t];
  }
  return best;
}

/** Formats the elapsed time between two instants, only when both are known (app.rs
 *  `fmt_duration`). */
export function fmtDuration(start?: string | null, finish?: string | null): string | null {
  const [s, f] = [msOf(start), msOf(finish)];
  return s === null || f === null ? null : fmtSecs(secsBetween(s, f));
}

/** A stage's span: earliest job start to latest finish, the finish only once all jobs are done. */
function jobsSpan(jobs: PipelineJob[]): { start: string | null; finish: string | null } {
  const start = minIso(jobs.map((j) => j.started_at));
  const finish = jobs.every((j) => msOf(j.finished_at) !== null) ? maxIso(jobs.map((j) => j.finished_at)) : null;
  return { start, finish };
}

/** When a run last did anything: the latest finish among its jobs and steps. */
export function lastActivity(run: PipelineRun): number | null {
  let best: number | null = null;
  for (const s of run.stages)
    for (const j of s.jobs)
      for (const t of [j.finished_at, ...j.steps.map((x) => x.finished_at)]) {
        const v = msOf(t);
        if (v !== null && (best === null || v > best)) best = v;
      }
  return best;
}

/** How long a parked run has waited: `now` less its last activity. */
export function waitingSecs(run: PipelineRun, now: Date): number | null {
  const last = lastActivity(run);
  return last === null ? null : secsBetween(last, now.getTime());
}

/** A run's elapsed seconds: start to finish, or to `now` while it is still going. A queued run
 *  has used no time yet. */
export function runSecs(run: PipelineRun, now: Date): number | null {
  if (run.status === "Queued") return null;
  const start = msOf(run.started_at);
  if (start === null) return null;
  const fin = msOf(run.finished_at);
  const end = fin !== null ? fin : isActive(run.status) ? now.getTime() : null;
  return end === null ? null : secsBetween(start, end);
}

/** A parked run's `(worked, waited)` seconds: run start to its last activity, then that to now. */
export function parkedSplit(run: PipelineRun, now: Date): { worked: number; waited: number } | null {
  const start = msOf(run.started_at);
  const last = lastActivity(run);
  if (start === null || last === null) return null;
  return { worked: secsBetween(start, last), waited: secsBetween(last, now.getTime()) };
}

// ---- steps and stages ----

/** Trailing steps that only tidy up after a job: `Post …` hooks and `Complete job`. */
export function isCleanupStep(name: string): boolean {
  const n = name.trim();
  return n.startsWith("Post ") || eqIgnoreAsciiCase(n, "Complete job");
}

/** How a job's steps fold, as `[passed, cleanup]`: steps `..passed` fold into "N steps passed"
 *  (a failed run's leading passed steps, keeping the one before the failure in view), steps
 *  `cleanup..` into "N post & cleanup steps". `passed === 0` / `cleanup === steps.length`: no fold. */
export function stepFolds(job: PipelineJob, runFailed: boolean): [number, number] {
  const n = job.steps.length;
  let cleanup = n;
  while (cleanup > 0 && isCleanupStep(job.steps[cleanup - 1].name)) cleanup -= 1;
  if (n - cleanup < 2) cleanup = n;
  let passed = 0;
  if (runFailed) {
    const f = job.steps.findIndex((s) => s.status === "Failed");
    if (f >= 0) {
      let lead = 0;
      while (lead < n && job.steps[lead].status === "Succeeded") lead += 1;
      const k = Math.min(lead, Math.max(0, f - 1), cleanup);
      if (k >= 2) passed = k;
    }
  }
  return [passed, cleanup];
}

/** The span of some steps: earliest start to latest finish, the finish only once all are done. */
export function stepsSpan(steps: PipelineStep[]): { start: string | null; finish: string | null } {
  const start = minIso(steps.map((s) => s.started_at));
  const finish = steps.every((s) => msOf(s.finished_at) !== null) ? maxIso(steps.map((s) => s.finished_at)) : null;
  return { start, finish };
}

/** A folded group's status: the first step that didn't pass, else passed. */
export function stepsStatus(steps: PipelineStep[]): PipelineRunStatus {
  return steps.find((s) => s.status !== "Succeeded")?.status ?? "Succeeded";
}

/** What a folded stage says after its label: the problem its jobs report when it didn't pass
 *  cleanly, else how many jobs it ran. */
export function stageSummary(stage: PipelineStage, status: PipelineRunStatus): { text: string; tone: Tone } | null {
  const problem = (want: PipelineRunStatus) => stage.jobs.filter((j) => j.status === want).find((j) => j.problem != null)?.problem ?? null;
  if (status === "Waiting") return { text: "needs approval", tone: "warn" };
  if (status === "PartiallySucceeded") {
    const p = problem("PartiallySucceeded");
    if (p !== null) return { text: p, tone: "warn" };
  }
  if (status === "Failed") {
    const p = problem("Failed");
    if (p !== null) return { text: p, tone: "bad" };
  }
  const n = stage.jobs.length;
  return n > 0 ? { text: `${n} ${n === 1 ? "job" : "jobs"}`, tone: "neutral" } : null;
}

const isOpen = (input: TreeInput, key: string, dflt: boolean) => input.folds[key] ?? dflt;

const finishedClean = (s: PipelineRunStatus) => s === "Succeeded" || s === "PartiallySucceeded" || s === "Skipped";

/** The tree's gated stage (`PipelineView::gate_stage`). */
export function viewGateStage(input: Pick<TreeInput, "run" | "shown" | "gates">): number | null {
  return gateStage(input.run, input.shown, input.gates);
}

/** Flattens stages → jobs → steps into visible rows, honouring folds (`PipelineView::flatten`).
 *  A single stage drops its row; a single stage with a single job drops the job row too. Finished
 *  stages and jobs start folded; two or more consecutive skipped stages fold into one row. */
export function flatten(input: TreeInput): FlatNode[] {
  const out: FlatNode[] = [];
  const stages = input.run.stages;
  const singleStage = stages.length === 1;
  const failed = input.run.status === "Failed";
  const gate = viewGateStage(input);
  const live = isActive(input.run.status);
  const shown = (si: number): PipelineRunStatus => {
    const st = stages[si].status;
    if (gate === si) return "Waiting";
    // A gate on a run that has since finished never opened: the stage didn't run.
    if (st === "Waiting" && !live) return "Canceled";
    return st;
  };
  let si = 0;
  while (si < stages.length) {
    if (!singleStage && shown(si) === "Skipped") {
      let n = 0;
      while (si + n < stages.length && shown(si + n) === "Skipped") n += 1;
      if (n >= 2) {
        pushSkippedRun(input, out, si, n);
        si += n;
        continue;
      }
    }
    flattenStage(input, out, si, shown(si), shown, singleStage, failed);
    si += 1;
  }
  return out;
}

/** `n` consecutive skipped stages from `first` as one foldable row, naming the failure that
 *  stopped them; opened, it lists the stages. */
export function pushSkippedRun(input: TreeInput, out: FlatNode[], first: number, n: number): void {
  const stages = input.run.stages;
  const key = `s${first}.skip`;
  const open = isOpen(input, key, false);
  const cause = [...stages.slice(0, first)].reverse().find((s) => s.status === "Failed");
  out.push({
    ...bare(0, `${n} stages skipped`, "Skipped", first),
    key,
    expanded: open,
    note: cause ? `${cause.name} failed` : null,
    group: true,
  });
  if (open) {
    for (let k = first; k < first + n; k++) out.push(bare(1, stages[k].name, "Skipped", k));
  }
}

/** One stage's rows: its own (unless it is the run's only stage), then — when open — its jobs and
 *  their steps, or for a gated stage with no jobs yet, the line saying why. */
export function flattenStage(
  input: TreeInput,
  out: FlatNode[],
  si: number,
  status: PipelineRunStatus,
  shown: (si: number) => PipelineRunStatus,
  singleStage: boolean,
  failed: boolean,
): void {
  const stage = input.run.stages[si];
  const key = `s${si}`;
  const expanded = singleStage || isOpen(input, key, !finishedClean(status));
  if (!singleStage) {
    const { start, finish } = jobsSpan(stage.jobs);
    const gateInfo = status === "Waiting" && stage.jobs.length === 0;
    let note: string | null = null;
    if (status === "Queued" && stage.jobs.length === 0) {
      for (let k = si - 1; k >= 0; k--) {
        if (isActive(shown(k))) {
          note = `after ${input.run.stages[k].name}`;
          break;
        }
      }
    }
    out.push({
      ...bare(0, stage.name, status, si),
      key: stage.jobs.length > 0 || gateInfo ? key : null,
      expanded,
      duration: fmtDuration(start, finish),
      startedAt: start,
      finishedAt: finish,
      summary: expanded ? null : stageSummary(stage, status),
      note,
    });
    if (!expanded) return;
    if (gateInfo) {
      out.push({ ...bare(1, "approval required · its jobs start once approved", status, si), info: true });
      if (!input.canRespondApprovals) {
        out.push({ ...bare(1, `o  open in ${input.providerLabel} to approve`, status, si), info: true });
      }
      return;
    }
  }
  const jobDepth = singleStage ? 0 : 1;
  const singleJob = singleStage && stage.jobs.length === 1 && stage.jobs[0].steps.length > 0;
  stage.jobs.forEach((job, ji) => {
    const jkey = `s${si}.j${ji}`;
    const jexpanded = singleJob || isOpen(input, jkey, !finishedClean(job.status));
    if (!singleJob) {
      out.push({
        ...bare(jobDepth, job.name, job.status, si),
        key: job.steps.length > 0 ? jkey : null,
        expanded: jexpanded,
        duration: fmtDuration(job.started_at, job.finished_at),
        problem: job.problem ?? null,
        url: job.url ?? null,
        jobId: job.id,
        startedAt: job.started_at ?? null,
        finishedAt: job.finished_at ?? null,
        job: ji,
      });
    }
    if (jexpanded) flattenSteps(input, out, si, ji, job, singleJob ? jobDepth : jobDepth + 1, failed);
  });
}

/** A job's step rows, with its passed and cleanup runs folded into group rows. */
export function flattenSteps(input: TreeInput, out: FlatNode[], si: number, ji: number, job: PipelineJob, depth: number, failed: boolean): void {
  const [passed, cleanup] = stepFolds(job, failed);
  const stepNode = (k: number, step: PipelineStep, d: number): FlatNode => ({
    ...bare(d, step.name, step.status, si),
    duration: fmtDuration(step.started_at, step.finished_at),
    url: job.url ?? null,
    jobId: job.id,
    startedAt: step.started_at ?? null,
    finishedAt: step.finished_at ?? null,
    job: ji,
    step: k,
  });
  const group = (from: number, to: number, label: string, suffix: string) => {
    const key = `s${si}.j${ji}.${suffix}`;
    const open = isOpen(input, key, false);
    const steps = job.steps.slice(from, to);
    const { start, finish } = stepsSpan(steps);
    out.push({
      ...bare(depth, label, stepsStatus(steps), si),
      key,
      expanded: open,
      duration: fmtDuration(start, finish),
      url: job.url ?? null,
      jobId: job.id,
      startedAt: start,
      finishedAt: finish,
      job: ji,
      group: true,
    });
    if (open) for (let k = from; k < to; k++) out.push(stepNode(k, job.steps[k], depth + 1));
  };
  if (passed > 0) group(0, passed, `${passed} steps passed`, "pass");
  for (let k = passed; k < cleanup; k++) out.push(stepNode(k, job.steps[k], depth));
  if (cleanup < job.steps.length) group(cleanup, job.steps.length, `${job.steps.length - cleanup} post & cleanup steps`, "post");
}

// ---- selection ----

/** The first failed node of a run as `[stage, job, step]`: a failed step when a job has one, else
 *  a failed job. */
export function firstFailedNode(run: PipelineRun): [number, number, number | null] | null {
  for (let si = 0; si < run.stages.length; si++) {
    const stage = run.stages[si];
    for (let ji = 0; ji < stage.jobs.length; ji++) {
      const job = stage.jobs[ji];
      const k = job.steps.findIndex((s) => s.status === "Failed");
      if (k >= 0) return [si, ji, k];
      if (job.status === "Failed") return [si, ji, null];
    }
  }
  return null;
}

/** The run's first failed node as the failure line names it and its log opens on. */
export function failedTarget(run: PipelineRun): { jobId: string; name: string; step: number | null } | null {
  const f = firstFailedNode(run);
  if (!f) return null;
  const [si, ji, step] = f;
  const job = run.stages[si]?.jobs[ji];
  if (!job) return null;
  const s = step !== null ? job.steps[step] : undefined;
  return { jobId: job.id, name: s ? s.name : job.name, step };
}

/** The row of an in-flight run that wants watching: the stage a parked run waits at, or a running
 *  run's first running job. */
export function attentionRow(input: TreeInput, nodes: FlatNode[] = flatten(input)): number | null {
  let i = -1;
  if (input.shown === "Waiting") {
    const si = viewGateStage(input);
    if (si === null) return null;
    i = nodes.findIndex((n) => n.stage === si && n.job === null && !n.info && !n.group);
  } else if (input.shown === "Running") {
    let at: [number, number] | null = null;
    for (let si = 0; si < input.run.stages.length && !at; si++) {
      const ji = input.run.stages[si].jobs.findIndex((j) => j.status === "Running");
      if (ji >= 0) at = [si, ji];
    }
    if (!at) return null;
    const [si, ji] = at;
    i = nodes.findIndex((n) => n.stage === si && n.job === ji && n.step === null && !n.group);
  }
  return i < 0 ? null : i;
}

/** Row of a job (or one of its steps) in `flatten`'s order. Its stage and job must already be
 *  open; a step folded into a group has that group opened — returned in `folds`. */
export function nodeIndex(input: TreeInput, si: number, ji: number, step: number | null): { index: number; folds: Record<string, boolean> } {
  const find = (nodes: FlatNode[]) => nodes.findIndex((n) => !n.group && n.stage === si && n.job === ji && n.step === step);
  const at = find(flatten(input));
  if (at >= 0) return { index: at, folds: input.folds };
  const folds = { ...input.folds, [`s${si}.j${ji}.pass`]: true, [`s${si}.j${ji}.post`]: true };
  const nodes = flatten({ ...input, folds });
  let i = find(nodes);
  if (i < 0) i = nodes.findIndex((n) => n.stage === si && n.job === ji);
  return { index: Math.max(0, i), folds };
}

/** First load: what to select (`PipelineView::auto_select`, less its once-only bookkeeping). A
 *  failed run opens and selects its first failed step (or job) and asks for its logs; a parked
 *  run selects its gated stage; a running one its first running job. */
export function autoSelect(input: TreeInput): { selected: number | null; folds: Record<string, boolean>; openLogs: boolean } {
  if (input.run.status === "Failed") {
    const f = firstFailedNode(input.run);
    if (f) {
      const [si, ji, step] = f;
      const folds = { ...input.folds, [`s${si}`]: true };
      if (step !== null) folds[`s${si}.j${ji}`] = true;
      const r = nodeIndex({ ...input, folds }, si, ji, step);
      return { selected: r.index, folds: r.folds, openLogs: true };
    }
  }
  return { selected: attentionRow(input), folds: input.folds, openLogs: false };
}

/** The tree's title: the job for a single-job run, `Jobs` for one stage of several jobs, else the
 *  full stages · jobs · steps. */
export function treeTitle(run: PipelineRun): string {
  if (run.stages.length === 1) {
    const stage = run.stages[0];
    if (stage.jobs.length === 1 && stage.jobs[0].steps.length > 0) return stage.jobs[0].name;
    if (eqIgnoreAsciiCase(stage.name, "jobs") || stage.name === "") return "Jobs";
    return `Jobs · ${stage.name}`;
  }
  return "Stages · jobs · steps";
}

// ---- the time axis ----

/** Where times land on the timeline's bars, as fractional columns from the axis start. Linear,
 *  or — for a run parked on a gate longer than it worked — squeezed: the work gets the first
 *  `WORK_SHARE` of the width and the wait a fixed segment that ends on now. */
export class TimeAxis {
  static readonly WORK_SHARE = 0.65;

  private constructor(
    /** Seconds from the axis start to its end. */
    readonly total: number,
    readonly width: number,
    /** When squeezed: the seconds of work, and the columns they get. */
    readonly squeeze: { worked: number; workW: number } | null,
  ) {}

  static linear(total: number, width: number): TimeAxis {
    return new TimeAxis(total, width, null);
  }

  /** Squeezed when the idle gap after `worked` is more than half the span, linear otherwise. */
  static parked(total: number, worked: number, width: number): TimeAxis {
    const workW = Math.round(width * TimeAxis.WORK_SHARE);
    const ok = total - worked > total / 2 && worked > 0 && workW >= 1 && width > workW + 2;
    return new TimeAxis(total, width, ok ? { worked, workW } : null);
  }

  /** The fractional column of `secs` after the axis start. */
  pos(secs: number): number {
    const sq = this.squeeze;
    if (!sq) return (secs / this.total) * this.width;
    if (secs <= sq.worked) return (secs / sq.worked) * sq.workW;
    const waitW = this.width - 1 - sq.workW;
    return sq.workW + ((secs - sq.worked) / Math.max(this.total - sq.worked, 1)) * waitW;
  }

  col(secs: number): number {
    return Math.max(0, Math.round(this.pos(secs)));
  }

  /** The seconds and columns the round-number ticks are spread over: the work, when squeezed. */
  ticked(): [number, number] {
    return this.squeeze ? [this.squeeze.worked, this.squeeze.workW] : [this.total, this.width];
  }
}

const TICK_STEPS = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200, 86400];

/** The TUI's tick budget for a ticked width of `tickW` columns: one per 7, between 2 and 5. */
export function maxTicksFor(tickW: number): number {
  return Math.min(5, Math.max(2, Math.floor(tickW / 7)));
}

/** Round-number ticks over the axis's ticked span (`render_timeline`'s chooser): the first step
 *  giving fewer than `maxTicks` intervals, then every multiple of it from 0. `pos` is the tick's
 *  fractional column within the ticked width. (The TUI's per-cell collision rules are left to the
 *  renderer.) */
export function axisTicks(axis: TimeAxis, maxTicks: number): { step: number; ticks: { secs: number; label: string; pos: number }[] } {
  const [tickTotal, tickW] = axis.ticked();
  const step = TICK_STEPS.find((s) => Math.floor(tickTotal / s) < maxTicks) ?? 86400;
  const ticks: { secs: number; label: string; pos: number }[] = [];
  for (let t = 0; t <= tickTotal; t += step) ticks.push({ secs: t, label: axisLabel(t), pos: (t / tickTotal) * tickW });
  return { step, ticks };
}

/** The span the timeline draws (`render_timeline`'s preamble): from the earliest start — while a
 *  rerun is in flight, no earlier than this attempt — to the latest finish, now while active, and
 *  every estimate's end. `last`/`waitSecs` are set for a parked run (where the wait began).
 *  `null` when the run carries no times. */
export function timelineSpan(
  run: PipelineRun,
  shown: PipelineRunStatus,
  ghosts: Map<string, Ghost>,
  now: Date,
): { t0: number; end: number; total: number; last: number | null; waitSecs: number | null } | null {
  const active = isActive(run.status);
  const jobs = run.stages.flatMap((s) => s.jobs);
  const floor = active ? msOf(run.started_at) : null;
  const starts = [run.started_at, ...jobs.map((j) => j.started_at), ...jobs.flatMap((j) => j.steps.map((s) => s.started_at))]
    .map(msOf)
    .filter((t): t is number => t !== null && (floor === null || t >= floor));
  if (starts.length === 0) return null;
  const t0 = Math.min(...starts);
  const finishes = [run.finished_at, ...jobs.map((j) => j.finished_at), ...jobs.flatMap((j) => j.steps.map((s) => s.finished_at))]
    .map(msOf)
    .filter((t): t is number => t !== null);
  let end = finishes.length ? Math.max(...finishes) : t0;
  if (active) end = Math.max(end, now.getTime());
  for (const g of ghosts.values()) end = Math.max(end, g.end);
  const total = Math.max((end - t0) / 1000, 1);
  const la = shown === "Waiting" ? lastActivity(run) : null;
  const last = la !== null && la >= t0 ? la : null;
  return { t0, end, total, last, waitSecs: last !== null ? secsBetween(last, now.getTime()) : null };
}

/** The axis for a span `width` columns wide: squeezed around a parked run's wait, else linear. */
export function timelineAxis(span: { t0: number; total: number; last: number | null }, width: number): TimeAxis {
  return span.last !== null ? TimeAxis.parked(span.total, (span.last - span.t0) / 1000, width) : TimeAxis.linear(span.total, width);
}

// ---- estimates ----

/** How long an in-flight run and its jobs should take, from medians of recent runs (app.rs
 *  `Estimates`). Built by `pipelineEstimates` in ./history. */
export interface Estimates {
  runMedian: number | null;
  runN: number;
  /** Median seconds per job name. */
  jobs: Map<string, number>;
  jobRuns: number;
}

/** An estimate drawn after a job's bar: `start..end` (epoch ms), and the seconds left for a
 *  running job (`null` for a queued one). */
export interface Ghost {
  start: number;
  end: number;
  left: number | null;
}

/** The `jobGhosts` key for a job. */
export const ghostKey = (si: number, ji: number) => `${si}.${ji}`;

/** A matrix leg's base name: `build (x86_64-apple-darwin)` → `build`. */
export function matrixBase(name: string): string | null {
  const at = name.lastIndexOf(" (");
  if (at < 0) return null;
  return name.slice(at + 2).endsWith(")") ? name.slice(0, at).trim() : null;
}

/** A job name's first word (`build-global-artifacts` → `build`). */
export function firstWord(name: string): string {
  return name.split(/[-_ :/]/)[0];
}

/** Estimated finishes for an in-flight run's jobs, keyed by `ghostKey(stage, job)`. A running job
 *  ends its median after it started. Queued jobs in one stage run side by side once the stage is
 *  ready; successive stages follow each other. In a single stage, a queued job sharing its first
 *  word with a running matrix starts after that matrix, any other once the running jobs are done. */
export function jobGhosts({
  run,
  shown,
  estimates,
  now,
}: {
  run: PipelineRun;
  shown: PipelineRunStatus;
  estimates: Pick<Estimates, "jobs">;
  now: Date;
}): Map<string, Ghost> {
  const out = new Map<string, Ghost>();
  // Nothing is running behind a gate, so nothing can be estimated past it.
  if (!(shown === "Queued" || shown === "Running") || estimates.jobs.size === 0) return out;
  const nowMs = now.getTime();
  let afterRunning = nowMs;
  // When each running matrix (by base name) is estimated done, in first-seen order.
  const matrices = new Map<string, number>();
  run.stages.forEach((stage, si) =>
    stage.jobs.forEach((job, ji) => {
      const median = estimates.jobs.get(job.name);
      const start = msOf(job.started_at);
      if (median === undefined || start === null || job.status !== "Running") return;
      const end = Math.max(start + median * 1000, nowMs);
      afterRunning = Math.max(afterRunning, end);
      const base = matrixBase(job.name);
      if (base !== null) matrices.set(base, Math.max(matrices.get(base) ?? end, end));
      if (end > nowMs) out.set(ghostKey(si, ji), { start: nowMs, end, left: Math.trunc((end - nowMs) / 1000) });
    }),
  );
  const singleStage = run.stages.length === 1;
  let ready = afterRunning;
  run.stages.forEach((stage, si) => {
    let stageDone = ready;
    stage.jobs.forEach((job, ji) => {
      const median = estimates.jobs.get(job.name);
      if (median === undefined || job.status !== "Queued") return;
      let start = ready;
      if (singleStage) {
        start = afterRunning;
        for (const [base, done] of matrices) {
          if (firstWord(base) === firstWord(job.name)) {
            start = done;
            break;
          }
        }
      }
      const end = start + median * 1000;
      stageDone = Math.max(stageDone, end);
      out.set(ghostKey(si, ji), { start, end, left: null });
    });
    ready = stageDone;
  });
  return out;
}

/** The running job expected to finish last — the run's long pole — and, when a queued job's
 *  estimate starts as it ends, that job (`critical_path_hint`). */
export function criticalPath(run: PipelineRun, ghosts: Map<string, Ghost>): { job: PipelineJob; waiting?: PipelineJob; leftSecs: number } | null {
  let best: { si: number; ji: number; g: Ghost } | null = null;
  run.stages.forEach((stage, si) =>
    stage.jobs.forEach((_, ji) => {
      const g = ghosts.get(ghostKey(si, ji));
      if (g && g.left !== null && (!best || g.end >= best.g.end)) best = { si, ji, g };
    }),
  );
  const b = best as { si: number; ji: number; g: Ghost } | null;
  if (!b) return null;
  const job = run.stages[b.si]?.jobs[b.ji];
  if (!job) return null;
  let waiting: PipelineJob | undefined;
  for (let s = 0; s < run.stages.length && !waiting; s++) {
    run.stages[s].jobs.forEach((j, ji) => {
      if (waiting || j.status !== "Queued") return;
      const g = ghosts.get(ghostKey(s, ji));
      if (g && g.left === null && g.start === b.g.end) waiting = j;
    });
  }
  return { job, waiting, leftSecs: Math.max(0, b.g.left!) };
}

/** The critical-path hint as the TUI words it: `windows is the critical path: host waits ~1m28
 *  on it`, or `windows is the long pole · ~1m28 left`. */
export function criticalPathText(cp: { job: PipelineJob; waiting?: PipelineJob; leftSecs: number }): string {
  const left = axisLabel(cp.leftSecs);
  return cp.waiting
    ? `${cp.job.name} is the critical path: ${cp.waiting.name} waits ~${left} on it`
    : `${cp.job.name} is the long pole · ~${left} left`;
}

/** The three longest steps (or jobs, where no job reports steps) and their share of the run, in
 *  percent (`slowest_note`; the TUI shows it only for a finished run). */
export function slowestLeaves(run: PipelineRun, now: Date): { leaves: { name: string; secs: number }[]; sharePct: number } | null {
  const jobs = run.stages.flatMap((s) => s.jobs);
  const span = (s?: string | null, f?: string | null) => {
    const [a, b] = [msOf(s), msOf(f)];
    return a === null || b === null ? null : secsBetween(a, b);
  };
  const raw: { name: string; secs: number | null }[] = jobs.some((j) => j.steps.length > 0)
    ? jobs.flatMap((j) => j.steps).map((s) => ({ name: s.name, secs: span(s.started_at, s.finished_at) }))
    : jobs.map((j) => ({ name: j.name, secs: span(j.started_at, j.finished_at) }));
  const leaves = raw
    .filter((l): l is { name: string; secs: number } => l.secs !== null)
    .sort((a, b) => b.secs - a.secs)
    .slice(0, 3)
    .filter((l) => l.secs > 0);
  const total = runSecs(run, now);
  if (total === null || total <= 0 || leaves.length === 0) return null;
  const sum = leaves.reduce((acc, l) => acc + l.secs, 0);
  return { leaves, sharePct: Math.min(100, Math.floor((sum * 100) / total)) };
}
