// Presentation helpers. Colours/icons mirror the TUI (crates/forgetop-tui/src/{theme,ui}.rs)
// so the two frontends read identically: green = done/approved, blue = in-flight,
// red = failed/closed, yellow = pending, grey = draft/neutral, magenta = merged.

import type {
  CheckRun,
  CheckStatus,
  CheckSummary,
  NotificationKind,
  PipelineRun,
  PipelineRunStatus,
  PipeRow,
  ProviderType,
  PullRequest,
  Reviewer,
  ReviewVote,
  TimelineEventKind,
  WorkItemStateCategory,
} from "./types";
import type { IconName } from "./components/icons";

export type GlyphName = IconName;

export interface Meta {
  label: string;
  icon: string;
  /** The status drawn as a stroke icon (see components/icons.tsx); the dashboard's rendering. */
  glyph: GlyphName;
  color: string;
}

const V = (k: string) => `var(--${k})`;

export function providerMeta(p: ProviderType): { label: string; color: string } {
  switch (p) {
    case "GitHub":
      return { label: "GitHub", color: "#dcdcdc" };
    case "GitLab":
      return { label: "GitLab", color: "#fc6d26" };
    case "Bitbucket":
      return { label: "Bitbucket", color: "#2684ff" };
    case "Linear":
      return { label: "Linear", color: "#8b8fd6" };
    case "Jira":
      return { label: "Jira", color: "#4a90e2" };
    case "AzureDevOps":
      return { label: "Azure DevOps", color: "#3aa0f0" };
    default:
      return { label: "Demo", color: V("dim") };
  }
}

export function prStatusMeta(pr: PullRequest): Meta {
  if (pr.is_draft) return { label: "draft", glyph: "pr-draft", icon: "◌", color: V("dim") };
  switch (pr.status) {
    case "Open":
      return { label: "open", glyph: "pr-open", icon: "●", color: V("green") };
    case "Merged":
      return { label: "merged", glyph: "pr-merged", icon: "✦", color: V("magenta") };
    case "Closed":
      return { label: "closed", glyph: "pr-closed", icon: "✗", color: V("red") };
    case "Draft":
      return { label: "draft", glyph: "pr-draft", icon: "◌", color: V("dim") };
  }
}

export function checkMeta(status: CheckStatus): Meta {
  switch (status) {
    case "Passed":
      return { label: "checks passing", glyph: "check-circle", icon: "✓", color: V("green") };
    case "Failed":
      return { label: "checks failing", glyph: "x-circle", icon: "✗", color: V("red") };
    case "Pending":
      return { label: "checks running", glyph: "running", icon: "◐", color: V("blue") };
    case "None":
      return { label: "no checks", glyph: "circle", icon: "·", color: V("dim") };
  }
}

export function voteMeta(vote: ReviewVote): Meta {
  switch (vote) {
    case "Approved":
    case "ApprovedWithSuggestions":
      return { label: "approved", glyph: "check-circle", icon: "✓", color: V("green") };
    case "Rejected":
      return { label: "changes requested", glyph: "warning", icon: "✗", color: V("yellow") };
    case "WaitingForAuthor":
      return { label: "waiting", glyph: "clock", icon: "…", color: V("yellow") };
    case "NoVote":
      return { label: "no vote", glyph: "circle", icon: "·", color: V("dim") };
  }
}

const PIPE_ICON: Record<PipelineRunStatus, string> = {
  Succeeded: "✓",
  Running: "◐",
  Queued: "◔",
  Failed: "✗",
  PartiallySucceeded: "▲",
  Canceled: "⊘",
  Waiting: "⏸",
  Skipped: "⊘",
};

export function pipeMeta(status: PipelineRunStatus): Meta & { running: boolean } {
  const icon = PIPE_ICON[status];
  switch (status) {
    case "Succeeded":
      return { label: "succeeded", glyph: "check-circle", icon, color: V("green"), running: false };
    case "Running":
      return { label: "running", glyph: "running", icon, color: V("blue"), running: true };
    case "Failed":
      return { label: "failed", glyph: "x-circle", icon, color: V("red"), running: false };
    case "PartiallySucceeded":
      return { label: "partial", glyph: "warning", icon, color: V("yellow"), running: false };
    case "Queued":
      return { label: "queued", glyph: "queued", icon, color: V("dim"), running: false };
    case "Canceled":
      return { label: "canceled", glyph: "blocked", icon, color: V("dim"), running: false };
    case "Waiting":
      return { label: "waiting", glyph: "clock", icon, color: V("yellow"), running: false };
    case "Skipped":
      return { label: "skipped", glyph: "skipped", icon, color: V("dim"), running: false };
  }
}

/** What a pipeline run was *building*, not what ran it — mirrors `launchpad::pipe_title` in
 *  forgetop-core. Prefers the run's own title (GitHub's `display_title`, Bitbucket's commit
 *  subject), falling back to the workflow for providers that expose none (GitLab, Azure DevOps).
 *  A blank title counts as absent. Keep in step with the Rust side: the two frontends must not
 *  label the same run differently. */
export function pipeTitle(run: PipelineRun, definitionName?: string | null): string {
  return run.title?.trim() ? run.title : pipeWorkflow(run, definitionName);
}

/** The pipeline a run belongs to — mirrors `launchpad::pipe_workflow` in forgetop-core. */
export function pipeWorkflow(run: PipelineRun, definitionName?: string | null): string {
  return definitionName || run.name || run.definition_id;
}

/** How many distinct pipelines the runs belong to — the sidebar badge counts pipelines, not runs
 *  (one busy pipeline fetches many). Keyed on connection + repository + `definition_id`, never the
 *  display name, exactly as the TUI's `App::pipeline_count` keys it. */
export function pipelineCount(rows: PipeRow[]): number {
  return new Set(rows.map((r) => `${r.connection_id}\u0001${r.run.repository ?? ""}\u0001${r.run.definition_id}`)).size;
}

/** What stands between a pull request and its merge. Mirrors `launchpad::PrBlocker`. */
export type PrBlocker = "conflicting" | "changes_requested" | "checks_failing";

/** Where a pull request stands. Mirrors `launchpad::PrState`. */
export type PrState =
  | { kind: "merged" }
  | { kind: "closed" }
  | { kind: "draft" }
  | { kind: "blocked"; blocker: PrBlocker }
  | { kind: "checks_running" }
  | { kind: "ready_to_merge" }
  | { kind: "nothing_blocking" };

/** Mirrors `launchpad::pr_blocker`: most-fundamental first, only the winner reported. A conflict
 *  blocks regardless of review and CI; a review asking for changes outranks red checks, since
 *  addressing it re-runs them. Drafts are never blocked. */
export function prBlocker(pr: PullRequest): PrBlocker | null {
  if (pr.is_draft) return null;
  if (pr.mergeable === "Conflicting") return "conflicting";
  if (pr.reviewers.some((r: Reviewer) => r.vote === "Rejected")) return "changes_requested";
  if (pr.checks === "Failed") return "checks_failing";
  return null;
}

/** Mirrors `launchpad::pr_state`. Lifecycle is tested first: a merged or closed pull request keeps
 *  the votes and merge state it had while open, so asking what blocks it gives a stale answer.
 *  Keep in step with the Rust side — the TUI and this panel must not disagree about a PR. */
export function prState(pr: PullRequest): PrState {
  if (pr.status === "Merged") return { kind: "merged" };
  if (pr.status === "Closed") return { kind: "closed" };
  if (pr.status === "Draft" || pr.is_draft) return { kind: "draft" };

  const blocker = prBlocker(pr);
  if (blocker) return { kind: "blocked", blocker };

  const running = pr.check_summary ? pr.check_summary.in_progress > 0 : pr.checks === "Pending";
  if (running) return { kind: "checks_running" };

  const approved = pr.reviewers.some((r: Reviewer) => r.vote === "Approved" || r.vote === "ApprovedWithSuggestions");
  return approved && pr.mergeable === "Mergeable" ? { kind: "ready_to_merge" } : { kind: "nothing_blocking" };
}

/** Rolls a list of check runs up into the counts `PullRequest.check_summary` carries, using the
 *  same buckets the providers do. The detail endpoint returns the runs but doesn't always carry
 *  the summary, and the state line needs counts to say "1 of 8 failed" rather than just "failing".
 *  This completes the input; it does not change the judgement. */
export function checkSummaryOf(checks: CheckRun[]): CheckSummary {
  const s: CheckSummary = { successful: 0, in_progress: 0, failed: 0, neutral: 0 };
  for (const c of checks) {
    if (c.status === "Passed") s.successful += 1;
    else if (c.status === "Pending") s.in_progress += 1;
    else if (c.status === "None") s.neutral += 1;
    else s.failed += 1;
  }
  return s;
}

const summaryTotal = (s: CheckSummary): number => s.successful + s.in_progress + s.failed + s.neutral;

/** "5 of 8 checks passed", or null when there's no CI to speak of. */
function checksClause(pr: PullRequest): string | null {
  const s = pr.check_summary;
  if (s && summaryTotal(s) > 0) return `${s.successful} of ${summaryTotal(s)} checks passed`;
  switch (pr.checks) {
    case "None":
      return null;
    case "Passed":
      return "checks passed";
    case "Failed":
      return "checks failing";
    case "Pending":
      return "checks running";
  }
}

/** What is stopping this pull request, as a pill: the blocker when there is one, otherwise the
 *  check roll-up. Mirrors the TUI's `pr_signal`, which fills its list's State column and its
 *  Command Center rows — a conflicted or changes-requested PR must not read as healthy just
 *  because CI is green. `spin` marks in-flight checks. */
export function prSignalMeta(pr: PullRequest): Meta & { spin: boolean } {
  const st = prState(pr);
  if (st.kind === "blocked" && st.blocker === "conflicting") {
    return { label: "conflicts", glyph: "warning", icon: "⚠", color: V("yellow"), spin: false };
  }
  if (st.kind === "blocked" && st.blocker === "changes_requested") {
    return { label: "changes requested", glyph: "warning", icon: "⚠", color: V("yellow"), spin: false };
  }
  if (st.kind === "checks_running") {
    return { ...checkMeta("Pending"), spin: true };
  }
  return { ...checkMeta(pr.checks), spin: false };
}

/** The one-line verdict: where this pull request stands, and why. Same sentences as the TUI's
 *  `pr_state_line`, from the same `prState`. */
export function prStateLine(pr: PullRequest): { icon: string; glyph: GlyphName; text: string; color: string } {
  // The raw ref, as the TUI uses — the two must read identically.
  const target = pr.target_ref || "the target branch";
  const by = (r?: Reviewer) => (r ? ` by ${r.user.display_name}` : "");
  const st = prState(pr);
  switch (st.kind) {
    case "merged": {
      const age = relativeTime(pr.updated_at);
      return { icon: "✦", glyph: "pr-merged", text: `Merged into ${target}${age ? ` ${age} ago` : ""}`, color: V("magenta") };
    }
    case "closed":
      return { icon: "✗", glyph: "x-circle", text: "Closed without merging", color: V("red") };
    case "draft":
      return { icon: "◌", glyph: "pr-draft", text: "Draft — not open for review yet", color: V("dim") };
    case "blocked":
      switch (st.blocker) {
        case "conflicting":
          return { icon: "⚠", glyph: "warning", text: `Blocked — conflicts with ${target}`, color: V("yellow") };
        case "changes_requested": {
          const who = pr.reviewers.find((r: Reviewer) => r.vote === "Rejected");
          return { icon: "⚠", glyph: "warning", text: `Blocked — changes requested${by(who)}`, color: V("yellow") };
        }
        case "checks_failing": {
          // The count of failures, never total minus passed: with checks still in flight that
          // subtraction reports work-in-progress as failed.
          const s = pr.check_summary;
          const detail = s && s.failed > 0 ? `${s.failed} of ${summaryTotal(s)} checks failed` : "checks failing";
          return { icon: "✗", glyph: "x-circle", text: `Blocked — ${detail}`, color: V("red") };
        }
      }
    // eslint-disable-next-line no-fallthrough
    case "checks_running": {
      const s = pr.check_summary;
      const detail = s ? `${summaryTotal(s) - s.in_progress} of ${summaryTotal(s)} done` : "still running";
      return { icon: "◐", glyph: "running", text: `Checks running — ${detail}`, color: V("blue") };
    }
    case "ready_to_merge": {
      const who = pr.reviewers.find((r: Reviewer) => r.vote === "Approved" || r.vote === "ApprovedWithSuggestions");
      const parts = [checksClause(pr), `approved${by(who)}`].filter(Boolean);
      return { icon: "✓", glyph: "check-circle", text: `Ready to merge — ${parts.join(", ")}`, color: V("green") };
    }
    case "nothing_blocking": {
      const parts = [checksClause(pr), pr.reviewers.length === 0 ? "no reviews yet" : null].filter(Boolean);
      return { icon: "✓", glyph: "check-circle", text: `Nothing blocking${parts.length ? ` — ${parts.join(", ")}` : ""}`, color: V("green") };
    }
  }
}

// Work-item colour: "blocked" always reds out; otherwise the category drives it.
export function wiStateColor(state: string, cat: WorkItemStateCategory): string {
  if (state.toLowerCase() === "blocked") return V("red");
  switch (cat) {
    case "Completed":
      return V("green");
    case "Started":
      return V("blue");
    // Past the doing and waiting on someone else: between the blue of doing and the green of done.
    case "Review":
      return V("cyan");
    default:
      return V("dim");
  }
}

/** The work-item state as a glyph: blocked always slashes out; otherwise the category. */
export function wiStateGlyph(state: string, cat: WorkItemStateCategory): GlyphName {
  if (state.toLowerCase() === "blocked") return "blocked";
  switch (cat) {
    case "Completed":
      return "check-circle";
    case "Started":
      return "half";
    case "Review":
      return "review";
    default:
      return "circle";
  }
}

export function notificationMeta(kind: NotificationKind): { icon: string; glyph: GlyphName; label: string; color: string } {
  switch (kind) {
    case "ReviewRequested":
      return { icon: "◈", glyph: "review", label: "Review requested", color: V("blue") };
    case "Mention":
      return { icon: "@", glyph: "at", label: "Mention", color: V("magenta") };
    case "Assigned":
      return { icon: "◎", glyph: "user", label: "Assigned", color: V("cyan") };
    case "CiFailed":
      return { icon: "✗", glyph: "x-circle", label: "CI failed", color: V("red") };
    case "Comment":
      return { icon: "❝", glyph: "comment", label: "Comment", color: V("yellow") };
    case "StateChange":
      return { icon: "↻", glyph: "refresh", label: "State change", color: V("green") };
    case "NewItem":
      return { icon: "+", glyph: "plus", label: "New", color: V("cyan") };
    case "Due":
      return { icon: "◷", glyph: "clock", label: "Due", color: V("yellow") };
    case "Reaction":
      return { icon: "☺", glyph: "smile", label: "Reaction", color: V("dim") };
    case "Other":
      return { icon: "•", glyph: "dot", label: "Update", color: V("dim") };
  }
}

export function timelineMeta(kind: TimelineEventKind): { icon: string; glyph: GlyphName; color: string } {
  switch (kind) {
    case "Approved":
      return { icon: "✓", glyph: "check-circle", color: V("green") };
    case "ChangesRequested":
      return { icon: "✗", glyph: "x-circle", color: V("red") };
    case "Reviewed":
      return { icon: "◎", glyph: "review", color: V("blue") };
    case "Commented":
      return { icon: "❝", glyph: "comment", color: V("yellow") };
    case "Merged":
      return { icon: "✦", glyph: "pr-merged", color: V("magenta") };
    case "Closed":
      return { icon: "⊘", glyph: "blocked", color: V("red") };
    case "Reopened":
      return { icon: "↻", glyph: "refresh", color: V("green") };
    case "StateChanged":
      return { icon: "→", glyph: "arrow-right", color: V("blue") };
    case "Assigned":
      return { icon: "◎", glyph: "user", color: V("cyan") };
    case "Labeled":
      return { icon: "▪", glyph: "dot", color: V("dim") };
    case "Committed":
      return { icon: "●", glyph: "dot", color: V("dim") };
    default:
      return { icon: "•", glyph: "dot", color: V("dim") };
  }
}

/** The last segment of a connection-relative repository (`acme/pay` → `pay`), for display. */
export function repoShort(repo?: string | null): string {
  if (!repo) return "";
  const i = repo.lastIndexOf("/");
  return i >= 0 ? repo.slice(i + 1) : repo;
}

/** `relativeTime` squeezed for a narrow column: "12h ago" → "12h", "just now" → "now". */
export function shortAge(iso?: string | null): string {
  const t = relativeTime(iso);
  if (!t) return "";
  if (t === "just now") return "now";
  return t.replace(/ ago$/, "");
}

/** Whether two item references name the same thing: same connection, repository and id. */
export function sameRef(a: { conn: string; repo?: string | null; id?: string; runId?: string } | null | undefined, b: { conn: string; repo?: string | null; id?: string; runId?: string }): boolean {
  if (!a) return false;
  return a.conn === b.conn && (a.repo ?? "") === (b.repo ?? "") && (a.id ?? a.runId) === (b.id ?? b.runId);
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

/** Epoch millis for sorting; missing/invalid dates sort last (as 0). */
export function toTime(iso?: string | null): number {
  if (!iso) return 0;
  const t = new Date(iso).getTime();
  return Number.isNaN(t) ? 0 : t;
}

export function relativeTime(iso?: string | null): string {
  if (!iso) return "";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const secs = Math.round((Date.now() - then) / 1000);
  if (secs < 0) return "just now";
  if (secs < 45) return "just now";
  const mins = Math.round(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.round(days / 30);
  if (months < 12) return `${months}mo ago`;
  return `${Math.round(months / 12)}y ago`;
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/** Elapsed seconds as `45s`, `3m12s`, `1h02m` — the drill-in's one duration format. Mirrors the
 *  TUI's `fmt_secs` (crates/forgetop-tui/src/ui.rs). */
export function fmtSecs(secs: number): string {
  const s = Math.max(0, Math.trunc(secs));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m${pad2(s % 60)}s`;
  return `${Math.floor(s / 3600)}h${pad2(Math.floor((s % 3600) / 60))}m`;
}

/** A time-axis tick: `0s`, `30s`, `1m`, `1m30`, `2h`, `1h05`. Mirrors the TUI's `axis_label`. */
export function axisLabel(secs: number): string {
  const s = Math.trunc(secs);
  if (s < 60) return `${s}s`;
  if (s < 3600) {
    const m = Math.floor(s / 60);
    const r = s % 60;
    return r === 0 ? `${m}m` : `${m}m${pad2(r)}`;
  }
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return m === 0 ? `${h}h` : `${h}h${pad2(m)}`;
}
