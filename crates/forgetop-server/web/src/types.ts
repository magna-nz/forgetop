// Mirrors the Rust DTOs in crates/forgetop-server/src/dto.rs and the domain enums in
// crates/forgetop-core/src/domain.rs. serde serializes enums as their variant name.

export type ProviderType = "Demo" | "GitHub" | "AzureDevOps" | "Linear" | "GitLab" | "Bitbucket" | "Jira";
export type PullRequestStatus = "Open" | "Draft" | "Merged" | "Closed";
export type ReviewVote = "Rejected" | "WaitingForAuthor" | "NoVote" | "ApprovedWithSuggestions" | "Approved";
export type WorkItemStateCategory =
  | "Triage"
  | "Backlog"
  | "Unstarted"
  | "Started"
  | "Completed"
  | "Canceled";
export type PipelineRunStatus = "Queued" | "Running" | "Succeeded" | "PartiallySucceeded" | "Failed" | "Canceled";
export type CheckStatus = "None" | "Pending" | "Passed" | "Failed";
export type MergeableState = "Unknown" | "Mergeable" | "Blocked" | "Conflicting";
export type NotificationKind =
  | "ReviewRequested"
  | "Mention"
  | "Assigned"
  | "CiFailed"
  | "Comment"
  | "StateChange"
  | "Other";
export type NotificationItemType = "PullRequest" | "WorkItem" | "Pipeline" | "Other";

export interface User {
  id: string;
  display_name: string;
  handle?: string | null;
  avatar_url?: string | null;
}

export interface Reviewer {
  user: User;
  vote: ReviewVote;
  is_required: boolean;
}

export interface CheckSummary {
  successful: number;
  in_progress: number;
  failed: number;
  neutral: number;
}

export interface PullRequest {
  id: string;
  /** The repository this lives in, **connection-relative** (`acme/pay`). Undefined for
   *  providers that aren't repo-addressed (Jira, Linear). Mirrors the Rust domain type. */
  repository?: string | null;
  number?: number | null;
  title: string;
  description?: string | null;
  author: User;
  status: PullRequestStatus;
  is_draft: boolean;
  source_ref?: string | null;
  target_ref?: string | null;
  reviewers: Reviewer[];
  labels: string[];
  checks: CheckStatus;
  check_summary?: CheckSummary | null;
  mergeable: MergeableState;
  changed_files: number;
  additions: number;
  deletions: number;
  created_at?: string | null;
  updated_at?: string | null;
  url?: string | null;
}

export interface WorkItem {
  id: string;
  /** The repository this lives in, **connection-relative** (`acme/pay`). Undefined for
   *  providers that aren't repo-addressed (Jira, Linear). Mirrors the Rust domain type. */
  repository?: string | null;
  identifier?: string | null;
  title: string;
  description?: string | null;
  state: string;
  state_category: WorkItemStateCategory;
  work_item_type?: string | null;
  assignee?: User | null;
  created_at?: string | null;
  updated_at?: string | null;
  url?: string | null;
}

export interface PipelineStep {
  name: string;
  status: PipelineRunStatus;
  started_at?: string | null;
  finished_at?: string | null;
}

export interface PipelineJob {
  id: string;
  name: string;
  status: PipelineRunStatus;
  started_at?: string | null;
  finished_at?: string | null;
  steps: PipelineStep[];
  url?: string | null;
  problem?: string | null;
}

export interface PipelineStage {
  name: string;
  status: PipelineRunStatus;
  jobs: PipelineJob[];
}

export interface PipelineRun {
  id: string;
  /** The repository this lives in, **connection-relative** (`acme/pay`). Undefined for
   *  providers that aren't repo-addressed (Jira, Linear). Mirrors the Rust domain type. */
  repository?: string | null;
  definition_id: string;
  number?: number | null;
  name?: string | null;
  title?: string | null;
  status: PipelineRunStatus;
  triggered_by?: User | null;
  branch?: string | null;
  commit_sha?: string | null;
  started_at?: string | null;
  finished_at?: string | null;
  url?: string | null;
  stages: PipelineStage[];
}

export interface Notification {
  id: string;
  /** The repository this lives in, **connection-relative** (`acme/pay`). Undefined for
   *  providers that aren't repo-addressed (Jira, Linear). Mirrors the Rust domain type. */
  repository?: string | null;
  kind: NotificationKind;
  item_type: NotificationItemType;
  item_id?: string | null;
  title: string;
  context: string;
  url?: string | null;
  unread: boolean;
  updated_at?: string | null;
}

export interface PrRow {
  connection_id: string;
  connection: string;
  provider: ProviderType;
  pull_request: PullRequest;
  /** True when this row's decorated fields are genuinely missing and worth a per-row fetch. Only
   *  GitHub says yes — the other providers fill them straight from their list payload, so asking
   *  them would be one call per row that returns what we already have. */
  needs_decoration?: boolean;
}

export interface WiRow {
  connection_id: string;
  connection: string;
  provider: ProviderType;
  work_item: WorkItem;
}

export interface PipelineApproval {
  id: string;
  name: string;
  can_respond: boolean;
}

export interface PipeRow {
  connection_id: string;
  connection: string;
  provider: ProviderType;
  run: PipelineRun;
  definition_name?: string | null;
  approvals: PipelineApproval[];
}

export interface NotifRow {
  connection_id: string;
  connection: string;
  provider: ProviderType;
  notification: Notification;
}

export interface HealthRow {
  connection_id: string;
  display_name: string;
  provider: ProviderType;
  healthy: boolean;
}

export type FileChangeKind = "Added" | "Modified" | "Deleted" | "Renamed";

export interface Comment {
  id: string;
  author: User;
  body: string;
  created_at?: string | null;
}

export interface CommentThread {
  id: string;
  comments: Comment[];
  file_path?: string | null;
  line?: number | null;
  is_resolved: boolean;
}

export interface FileChange {
  path: string;
  kind: FileChangeKind;
  additions: number;
  deletions: number;
  patch?: string | null;
}

export interface CheckRun {
  name: string;
  status: CheckStatus;
  url?: string | null;
}

export interface Commit {
  sha: string;
  message: string;
  author: string;
  date?: string | null;
  url?: string | null;
}

export type TimelineEventKind =
  | "Approved"
  | "ChangesRequested"
  | "Reviewed"
  | "Commented"
  | "Merged"
  | "Closed"
  | "Reopened"
  | "StateChanged"
  | "Assigned"
  | "Labeled"
  | "Committed"
  | "Other";

export interface TimelineEvent {
  actor: User | null;
  kind: TimelineEventKind;
  summary: string;
  at?: string | null;
}

export interface PrDetail {
  pull_request: PullRequest;
  threads: CommentThread[];
  timeline: TimelineEvent[];
  changes: FileChange[];
  checks: CheckRun[];
  commits: Commit[];
}

/** A pending line comment (matches the Rust LineComment shape). */
export interface LineComment {
  path: string;
  line: number;
  side: "Old" | "New";
  body: string;
}

/** Identifies a PR for the detail view. */
export interface PrRef {
  conn: string;
  id: string;
  /** The item's **connection-relative** repository. A connection now spans an account, so the
   *  id alone doesn't say which repository's item this is. Optional: a single-repository
   *  connection resolves without it. */
  repo?: string | null;
}

/** Identifies a work item for the detail view. */
export interface WiRef {
  conn: string;
  id: string;
  /** The item's **connection-relative** repository. A connection now spans an account, so the
   *  id alone doesn't say which repository's item this is. Optional: a single-repository
   *  connection resolves without it. */
  repo?: string | null;
}

/** Identifies a pipeline run for the detail view. */
export interface PipeRef {
  conn: string;
  runId: string;
  /** The item's **connection-relative** repository. A connection now spans an account, so the
   *  id alone doesn't say which repository's item this is. Optional: a single-repository
   *  connection resolves without it. */
  repo?: string | null;
}

export interface WiDetail {
  work_item: WorkItem;
  threads: CommentThread[];
  timeline: TimelineEvent[];
}

export interface PipelineDetail {
  run: PipelineRun;
  approvals: PipelineApproval[];
}

interface LaunchpadBase {
  bucket: string;
  bucket_title: string;
  column: number;
  muted: boolean;
  connection_id: string;
  connection: string;
  provider: ProviderType;
}

export type LaunchpadRow = LaunchpadBase &
  (
    | { kind: "pr"; pull_request: PullRequest }
    | { kind: "wi"; work_item: WorkItem }
    | { kind: "pipe"; run: PipelineRun; definition_name?: string | null }
  );

/** Per-bucket overflow flags: true when the reference list had more than it shows. */
export interface LaunchpadMore {
  needs_review: boolean;
  your_work: boolean;
  your_open_prs: boolean;
  recently_merged: boolean;
  recent_pipelines: boolean;
}

export interface LaunchpadResponse {
  rows: LaunchpadRow[];
  more: LaunchpadMore;
}

export type FieldKey = "display_name" | "base_url" | "organization" | "project" | "repository" | "username" | "pat";

export interface FieldSpec {
  key: FieldKey;
  label: string;
  help: string;
  required: boolean;
  secret: boolean;
  default?: string | null;
}

export interface ProviderInfo {
  provider: ProviderType;
  label: string;
  fields: FieldSpec[];
  sections: string[];
}

export interface ConnectionRow {
  id: string;
  provider: ProviderType;
  display_name: string;
  base_url?: string | null;
  organization?: string | null;
  project?: string | null;
  repository?: string | null;
  username?: string | null;
  /** The chosen repository scope, exactly as stored: `null` = never chosen (the legacy single
   *  repository still applies), `[]` = the user chose none, `[…]` = fetch these. */
  repo_scope?: string[] | null;
  has_token: boolean;
  sections: string[];
}

export type StartupMode = "terminal_only" | "dashboard_only" | "both";
export interface Preferences {
  startup_mode: StartupMode;
}

export type SectionId = "launchpad" | "prs" | "work-items" | "pipelines" | "notifications" | "settings";

/** The fields the PR *list* endpoint omits — GitHub leaves `mergeable`, `changed_files`,
 *  `additions` and `deletions` out entirely — fetched per visible row from
 *  `/api/pr/decoration` rather than for every row of every repository in the scope. */
export interface PrDecoration {
  mergeable: MergeableState;
  changed_files: number;
  additions: number;
  deletions: number;
  checks: CheckStatus;
  check_summary?: CheckSummary | null;
}

/** The repositories a connection's credentials can reach — the scope picker's candidates. */
export interface RepositoryPage {
  repositories: string[];
  /** True when the provider had more than we fetched, so the picker says "5 of 500+" rather
   *  than presenting a cap as a total. Truncation must never be silent. */
  truncated: boolean;
}
