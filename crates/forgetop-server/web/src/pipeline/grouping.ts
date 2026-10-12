// The Pipelines list's grouping and the run statuses it reads, ported from the TUI
// (crates/forgetop-tui/src/app.rs `PipeGroup`, `pipe_group_key`, `pipe_group_subject`,
// `App::pipe_lines`, ui.rs `pipe_live`) and forgetop-core (domain.rs `has_running_work`,
// `shown_status`). Grouping is a *view* over the rows the caller already filtered and sorted:
// every mode shows the identical set of runs, only arranged differently.

import type { PipelineRun, PipelineRunStatus, PipeRow, ProviderType } from "../types";

/** How the Pipelines list groups its runs. `pipeline` is the default; `off` is the flat list. */
export type GroupMode = "pipeline" | "trigger" | "branch" | "off";

/** Parses a persisted spelling. Anything unrecognised falls back to the default. */
export function parseGroupMode(s: string | null | undefined): GroupMode {
  switch (s) {
    case "trigger":
      return "trigger";
    case "branch":
      return "branch";
    case "off":
      return "off";
    default:
      return "pipeline";
  }
}

/** The `G` cycle: the default first, the ungrouped list last. */
export function nextGroupMode(m: GroupMode): GroupMode {
  switch (m) {
    case "pipeline":
      return "trigger";
    case "trigger":
      return "branch";
    case "branch":
      return "off";
    case "off":
      return "pipeline";
  }
}

// ---- statuses (forgetop-core domain.rs) ----

/** `PipelineRunStatus::is_active`: queued, running, or held on a gate. */
export function isActive(s: PipelineRunStatus): boolean {
  return s === "Queued" || s === "Running" || s === "Waiting";
}

/** `PipelineRunStatus::with_pending_approval`: an active status, held, reads Waiting. */
export function withPendingApproval(s: PipelineRunStatus, pending: boolean): PipelineRunStatus {
  return pending && isActive(s) ? "Waiting" : s;
}

/** Whether any stage, job or step of this run is executing right now. A list row usually carries
 *  no stages, so this is only ever evidence *for* running, never against. */
export function hasRunningWork(run: PipelineRun): boolean {
  return run.stages.some(
    (s) => s.status === "Running" || s.jobs.some((j) => j.status === "Running" || j.steps.some((t) => t.status === "Running")),
  );
}

/** The status to show for a run: Waiting when it is in flight, held on a gate (a pending approval
 *  that blocks it, or a stage the provider reports as waiting), and nothing in it is executing. */
export function shownStatus(run: PipelineRun, blockingGate: boolean): PipelineRunStatus {
  const held = blockingGate || run.stages.some((s) => s.status === "Waiting");
  return withPendingApproval(run.status, held && !hasRunningWork(run));
}

/** The pending gates that hold a row's run, by name (the TUI's `PipeRow.gates`). */
export function rowGates(row: Pick<PipeRow, "approvals">): string[] {
  return row.approvals.filter((a) => a.blocks_run).map((a) => a.name);
}

/** `PipeRow::shown_status`: the row's run, held by any gate that blocks it. */
export function rowShownStatus(row: Pick<PipeRow, "run" | "approvals">): PipelineRunStatus {
  return shownStatus(row.run, row.approvals.some((a) => a.blocks_run));
}

/** True when the row has a gate this user can answer (the TUI's `awaiting_approval`). */
export function rowAwaitingApproval(row: Pick<PipeRow, "approvals">): boolean {
  return row.approvals.some((a) => a.can_respond);
}

const asciiLower = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());

/** Rust's `str::eq_ignore_ascii_case`. */
export function eqIgnoreAsciiCase(a: string, b: string): boolean {
  return a.length === b.length && asciiLower(a) === asciiLower(b);
}

/** The stage a parked run is held at: one the provider reports as Waiting, else — when the run
 *  reads Waiting only through its gates (`shown`) — the queued stage a gate is named after, or the
 *  first queued one. `null` when the run isn't waiting. Mirrors app.rs `gate_stage`. */
export function gateStage(run: PipelineRun, shown: PipelineRunStatus, gates: string[]): number | null {
  if (shown !== "Waiting") return null;
  const waiting = run.stages.findIndex((s) => s.status === "Waiting");
  if (waiting >= 0) return waiting;
  const named = run.stages.findIndex((s) => s.status === "Queued" && gates.some((g) => eqIgnoreAsciiCase(g, s.name)));
  if (named >= 0) return named;
  const queued = run.stages.findIndex((s) => s.status === "Queued");
  return queued >= 0 ? queued : null;
}

/** The list's stage strip: one status per stage, the gated stage reading Waiting (ui.rs
 *  `pipe_live`). Empty for a run that carries no stages (most list rows). */
export function stageStrip(run: PipelineRun, shown: PipelineRunStatus = run.status, gates: string[] = []): PipelineRunStatus[] {
  const gate = gateStage(run, shown, gates);
  return run.stages.map((s, i) => (gate === i ? "Waiting" : s.status));
}

/** What the dashboard list's strip draws: one square per stage, or — where the run has a single
 *  stage (GitHub's and Bitbucket's synthetic `jobs`) — one per job of it, since a lone stage
 *  square says no more than the status badge. A gated single stage reads Waiting on its first
 *  queued job, unless a job already says Waiting or Running. Empty for a run with no stages. */
export function listStrip(
  run: PipelineRun,
  shown: PipelineRunStatus = run.status,
  gates: string[] = [],
): { unit: "stages" | "jobs"; statuses: PipelineRunStatus[] } {
  const stages = stageStrip(run, shown, gates);
  const only = run.stages.length === 1 ? run.stages[0] : null;
  if (!only || only.jobs.length === 0) return { unit: "stages", statuses: stages };
  const statuses = only.jobs.map((j) => j.status);
  if (stages[0] === "Waiting" && !statuses.some((s) => s === "Waiting" || s === "Running")) {
    const q = statuses.indexOf("Queued");
    if (q >= 0) statuses[q] = "Waiting";
  }
  return { unit: "jobs", statuses };
}

// ---- names ----

/** The pipeline (definition) name for a row — "CI Build", not the run's own name. */
export function pipeDefinitionName(p: Pick<PipeRow, "definition_name" | "run">): string {
  return p.definition_name ?? p.run.name ?? p.run.definition_id;
}

/** A run's branch, or `(no branch)` when the provider gives none. */
export function pipeBranchLabel(p: Pick<PipeRow, "run">): string {
  const b = p.run.branch;
  return b ? b : "(no branch)";
}

const msOf = (iso?: string | null): number | null => {
  if (!iso) return null;
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : t;
};

/** What identifies the trigger that started a run: the commit, else the start time to the minute
 *  (UTC, `%Y-%m-%dT%H:%M`), else the run id. */
export function pipeTriggerToken(p: Pick<PipeRow, "run">): string {
  const sha = p.run.commit_sha;
  if (sha) return sha;
  const t = msOf(p.run.started_at);
  return t !== null ? new Date(t).toISOString().slice(0, 16) : p.run.id;
}

/** The group key a run falls under, for `mode`. Connection- and repo-qualified; keyed on
 *  `definition_id`, never the displayed name. Empty for `off`. */
export function pipeGroupKey(p: Pick<PipeRow, "connection_id" | "run">, mode: GroupMode): string {
  const repo = p.run.repository ?? "";
  const branch = p.run.branch ?? "";
  switch (mode) {
    case "pipeline":
      return `${p.connection_id}\u0001${repo}\u0001${p.run.definition_id}`;
    case "trigger":
      return `${p.connection_id}\u0001${repo}\u0001${branch}\u0001${pipeTriggerToken(p)}`;
    case "branch":
      return `${p.connection_id}\u0001${repo}\u0001${branch}`;
    case "off":
      return "";
  }
}

/** What names a group: the smallest definition name among its runs (else the definition id)
 *  grouped by pipeline, the branch otherwise. */
export function pipeGroupSubject(members: PipeRow[], mode: GroupMode): string {
  const first = members[0];
  if (mode === "pipeline" || mode === "off") {
    const named = members.map((m) => m.definition_name).filter((n): n is string => n != null);
    if (named.length === 0) return first.run.definition_id;
    return named.reduce((a, b) => (b < a ? b : a));
  }
  return pipeBranchLabel(first);
}

/** What a run under a header puts in the subject column: the thing that varies within the group
 *  — its branch grouped by pipeline, its pipeline otherwise. */
export function pipeChildSubject(p: PipeRow, mode: GroupMode): string {
  return mode === "pipeline" ? pipeBranchLabel(p) : pipeDefinitionName(p);
}

// ---- grouping ----

/** A group header in the Pipelines list: the roll-up of the runs beneath it. */
export interface PipeGroup {
  /** Stable identity, the expand/collapse key. */
  key: string;
  subject: string;
  /** The repository every run beneath belongs to. */
  repo: string;
  /** Short commit for the trigger grouping (`~HH:MM` local, or `#<id>` without one); else empty. */
  commit: string;
  /** In the caller's order. */
  rows: PipeRow[];
  /** The most recent run (by `started_at`) — the one status and started describe. */
  latest: PipeRow;
  runs: number;
  failed: number;
  /** The shown status of the most recent run — where the pipeline stands now, not the worst. */
  status: PipelineRunStatus;
  started?: string | null;
  /** Any run beneath has a gate this user can answer. */
  approval: boolean;
  provider: ProviderType;
  connection: string;
}

/** The newest member by `started_at` — Rust's `max_by_key`, so the *last* of equal maxima, and a
 *  run with no start the smallest. */
function newestRow(members: PipeRow[]): PipeRow {
  let best = members[0];
  let bestT = msOf(best.run.started_at) ?? -Infinity;
  for (const m of members) {
    const t = msOf(m.run.started_at) ?? -Infinity;
    if (t >= bestT) {
      best = m;
      bestT = t;
    }
  }
  return best;
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/** The Pipelines list grouped for `mode` (the TUI's `App::pipe_lines`, header part). Groups are in
 *  first-seen order, then sorted (stable, so ties keep the order the caller produced): with `cmp`
 *  (the list's active sort) by comparing each group's most recent run — grouped lists land
 *  collapsed, so a sort that only moved runs inside groups would move nothing on screen — else
 *  newest first by their most recent run (a group whose runs have no start sorts last). Rows
 *  within a group keep the caller's order. `off` has no groups and returns `[]` — the caller lists
 *  `rows` flat. */
export function groupPipelines(rows: PipeRow[], mode: GroupMode, cmp?: (a: PipeRow, b: PipeRow) => number): PipeGroup[] {
  if (mode === "off") return [];
  const order: string[] = [];
  const buckets = new Map<string, PipeRow[]>();
  for (const r of rows) {
    const key = pipeGroupKey(r, mode);
    const b = buckets.get(key);
    if (b) b.push(r);
    else {
      order.push(key);
      buckets.set(key, [r]);
    }
  }
  const newest = (k: string) => {
    const ts = buckets.get(k)!.map((r) => msOf(r.run.started_at)).filter((t): t is number => t !== null);
    return ts.length ? Math.max(...ts) : -Infinity;
  };
  const latestOf = new Map(order.map((k) => [k, newestRow(buckets.get(k)!)]));
  const sorted = cmp
    ? [...order].sort((a, b) => cmp(latestOf.get(a)!, latestOf.get(b)!))
    : [...order].sort((a, b) => {
        const [x, y] = [newest(a), newest(b)];
        return x === y ? 0 : y > x ? 1 : -1;
      });
  return sorted.map((key) => {
    const members = buckets.get(key)!;
    const first = members[0];
    const latest = latestOf.get(key)!;
    let commit = "";
    if (mode === "trigger") {
      const sha = first.run.commit_sha;
      const t = msOf(first.run.started_at);
      if (sha) commit = Array.from(sha).slice(0, 7).join("");
      else if (t !== null) {
        const d = new Date(t);
        commit = `~${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
      } else commit = `#${Array.from(first.run.id).slice(0, 7).join("")}`;
    }
    return {
      key,
      subject: pipeGroupSubject(members, mode),
      repo: first.run.repository ?? "",
      commit,
      rows: members,
      latest,
      runs: members.length,
      failed: members.filter((m) => m.run.status === "Failed").length,
      status: rowShownStatus(latest),
      started: latest.run.started_at,
      approval: members.some(rowAwaitingApproval),
      provider: first.provider,
      connection: first.connection,
    };
  });
}
