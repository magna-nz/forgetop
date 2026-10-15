/**
 * Public-demo domain model.  This deliberately belongs to the demo rather than
 * mirroring the application DTOs: the demo is a self-contained taste test.
 */
export type DemoProvider = "GitHub" | "GitLab" | "Bitbucket" | "Linear" | "Jira";
export type PullRequestStatus = "open" | "draft" | "merged" | "closed";
export type ReviewVote = "approved" | "changes_requested" | "pending";
export type PipelineStatus = "queued" | "running" | "passed" | "failed" | "cancelled";
export type WorkItemCategory = "backlog" | "todo" | "in_progress" | "done" | "blocked";
export type NotificationTarget = "pull_request" | "work_item" | "pipeline";

export interface DemoPerson {
  id: string;
  name: string;
  handle: string;
  initials: string;
}

export interface DemoConnection {
  id: string;
  provider: DemoProvider;
  name: string;
  healthy: boolean;
}

export interface DemoComment {
  id: string;
  author: DemoPerson;
  body: string;
  createdAt: string;
  replyTo?: string;
}

export interface DemoReview {
  reviewer: DemoPerson;
  vote: ReviewVote;
  summary?: string;
  at?: string;
}

export interface DemoTimelineEvent {
  id: string;
  kind: "comment" | "review" | "merge" | "revert" | "state" | "assignment" | "pipeline";
  actor: DemoPerson;
  message: string;
  at: string;
}

export interface DemoFileChange {
  path: string;
  additions: number;
  deletions: number;
  patch: string;
}

export interface DemoPullRequest {
  id: string;
  provider: DemoProvider;
  repository: string;
  number: number;
  title: string;
  description: string;
  author: DemoPerson;
  status: PullRequestStatus;
  sourceBranch: string;
  targetBranch: string;
  labels: string[];
  checks: "passing" | "failing" | "pending";
  mergeable: boolean;
  additions: number;
  deletions: number;
  changedFiles: number;
  updatedAt: string;
  reviewers: DemoReview[];
  comments: DemoComment[];
  timeline: DemoTimelineEvent[];
  files: DemoFileChange[];
}

export interface DemoWorkItem {
  id: string;
  provider: DemoProvider;
  project: string;
  identifier: string;
  title: string;
  description: string;
  type: "Bug" | "Task" | "Story" | "Spike";
  category: WorkItemCategory;
  state: string;
  assignee: DemoPerson | null;
  labels: string[];
  updatedAt: string;
  comments: DemoComment[];
  timeline: DemoTimelineEvent[];
}

export interface DemoPipelineJob {
  name: string;
  status: PipelineStatus;
  duration: string;
}

export interface DemoPipeline {
  id: string;
  provider: DemoProvider;
  project: string;
  name: string;
  runNumber: number;
  branch: string;
  commit: string;
  status: PipelineStatus;
  triggeredBy: DemoPerson;
  startedAt: string;
  jobs: DemoPipelineJob[];
  logs: string[];
}

export interface DemoNotification {
  id: string;
  provider: DemoProvider;
  target: NotificationTarget;
  targetId: string;
  title: string;
  context: string;
  updatedAt: string;
  unread: boolean;
}

export interface DemoLaunchpad {
  reviewRequested: string[];
  needsAttention: string[];
  assignedWork: string[];
  pipelineAlerts: string[];
}

export interface DemoState {
  connections: DemoConnection[];
  currentUser: DemoPerson;
  people: DemoPerson[];
  pullRequests: DemoPullRequest[];
  workItems: DemoWorkItem[];
  pipelines: DemoPipeline[];
  notifications: DemoNotification[];
  launchpad: DemoLaunchpad;
  lastAction: string | null;
}

export type DemoAction =
  | { type: "pr.comment"; prId: string; body: string; replyTo?: string }
  | { type: "pr.review"; prId: string; vote: Exclude<ReviewVote, "pending">; summary?: string }
  | { type: "pr.merge"; prId: string }
  | { type: "pr.revert"; prId: string }
  | { type: "work-item.comment"; workItemId: string; body: string; replyTo?: string }
  | { type: "work-item.assign"; workItemId: string; assigneeId: string | null }
  | { type: "work-item.edit"; workItemId: string; title: string; description: string }
  | { type: "work-item.state"; workItemId: string; category: WorkItemCategory; state: string }
  | { type: "pipeline.cancel"; pipelineId: string }
  | { type: "notification.read"; notificationId: string }
  | { type: "notification.read-all" }
  | { type: "reset" };
