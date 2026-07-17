// Presentation helpers. Colours/icons mirror the TUI (crates/forgetop-tui/src/{theme,ui}.rs)
// so the two frontends read identically: green = done/approved, blue = in-flight,
// red = failed/closed, yellow = pending, grey = draft/neutral, magenta = merged.

import type {
  CheckStatus,
  NotificationKind,
  PipelineRunStatus,
  ProviderType,
  PullRequest,
  ReviewVote,
  WorkItemStateCategory,
} from "./types";

export interface Meta {
  label: string;
  icon: string;
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
  if (pr.is_draft) return { label: "draft", icon: "◌", color: V("dim") };
  switch (pr.status) {
    case "Open":
      return { label: "open", icon: "●", color: V("green") };
    case "Merged":
      return { label: "merged", icon: "✦", color: V("magenta") };
    case "Closed":
      return { label: "closed", icon: "✗", color: V("red") };
    case "Draft":
      return { label: "draft", icon: "◌", color: V("dim") };
  }
}

export function checkMeta(status: CheckStatus): Meta {
  switch (status) {
    case "Passed":
      return { label: "checks passing", icon: "✓", color: V("green") };
    case "Failed":
      return { label: "checks failing", icon: "✗", color: V("red") };
    case "Pending":
      return { label: "checks running", icon: "◐", color: V("yellow") };
    case "None":
      return { label: "no checks", icon: "·", color: V("dim") };
  }
}

export function voteMeta(vote: ReviewVote): Meta {
  switch (vote) {
    case "Approved":
    case "ApprovedWithSuggestions":
      return { label: "approved", icon: "✓", color: V("green") };
    case "Rejected":
      return { label: "changes requested", icon: "✗", color: V("red") };
    case "WaitingForAuthor":
      return { label: "waiting", icon: "…", color: V("yellow") };
    case "NoVote":
      return { label: "no vote", icon: "·", color: V("dim") };
  }
}

const PIPE_ICON: Record<PipelineRunStatus, string> = {
  Succeeded: "✓",
  Running: "◐",
  Queued: "◔",
  Failed: "✗",
  PartiallySucceeded: "▲",
  Canceled: "⊘",
};

export function pipeMeta(status: PipelineRunStatus): Meta & { running: boolean } {
  const icon = PIPE_ICON[status];
  switch (status) {
    case "Succeeded":
      return { label: "succeeded", icon, color: V("green"), running: false };
    case "Running":
      return { label: "running", icon, color: V("blue"), running: true };
    case "Failed":
      return { label: "failed", icon, color: V("red"), running: false };
    case "PartiallySucceeded":
      return { label: "partial", icon, color: V("yellow"), running: false };
    case "Queued":
      return { label: "queued", icon, color: V("dim"), running: false };
    case "Canceled":
      return { label: "canceled", icon, color: V("dim"), running: false };
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
    default:
      return V("dim");
  }
}

export function notificationMeta(kind: NotificationKind): { icon: string; label: string; color: string } {
  switch (kind) {
    case "ReviewRequested":
      return { icon: "◈", label: "Review requested", color: V("accent") };
    case "Mention":
      return { icon: "@", label: "Mention", color: V("magenta") };
    case "Assigned":
      return { icon: "◎", label: "Assigned", color: V("cyan") };
    case "CiFailed":
      return { icon: "✗", label: "CI failed", color: V("red") };
    case "Comment":
      return { icon: "❝", label: "Comment", color: V("yellow") };
    case "StateChange":
      return { icon: "↻", label: "State change", color: V("green") };
    case "Other":
      return { icon: "•", label: "Update", color: V("dim") };
  }
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
