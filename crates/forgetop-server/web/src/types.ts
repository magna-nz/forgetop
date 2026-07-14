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

export interface PipelineRun {
  id: string;
  definition_id: string;
  number?: number | null;
  name?: string | null;
  status: PipelineRunStatus;
  triggered_by?: User | null;
  branch?: string | null;
  commit_sha?: string | null;
  started_at?: string | null;
  finished_at?: string | null;
  url?: string | null;
  stages: unknown[];
}

export interface Notification {
  id: string;
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
}

export interface WiRow {
  connection_id: string;
  connection: string;
  provider: ProviderType;
  work_item: WorkItem;
}

export interface PipeRow {
  connection_id: string;
  connection: string;
  provider: ProviderType;
  run: PipelineRun;
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

export type SectionId = "launchpad" | "prs" | "work-items" | "pipelines" | "notifications";
