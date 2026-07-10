//! Provider-neutral domain model (mirrors the .NET `Forgetop.Core.Domain`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The three top-level sections, each independently bound to a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Section {
    #[default]
    PullRequests,
    WorkItems,
    Pipelines,
}

/// The platforms forgetop can talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderType {
    Demo,
    GitHub,
    AzureDevOps,
    Linear,
    GitLab,
    Bitbucket,
    Jira,
}

impl ProviderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderType::Demo => "Demo",
            ProviderType::GitHub => "GitHub",
            ProviderType::AzureDevOps => "AzureDevOps",
            ProviderType::Linear => "Linear",
            ProviderType::GitLab => "GitLab",
            ProviderType::Bitbucket => "Bitbucket",
            ProviderType::Jira => "Jira",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PullRequestStatus {
    Open,
    Draft,
    Merged,
    Closed,
}

/// Provider-neutral reviewer vote (ADO numeric votes / GitHub review states map here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewVote {
    Rejected,
    WaitingForAuthor,
    NoVote,
    ApprovedWithSuggestions,
    Approved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkItemStateCategory {
    Triage,
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineRunStatus {
    Queued,
    Running,
    Succeeded,
    PartiallySucceeded,
    Failed,
    Canceled,
}

/// Roll-up CI/check state for a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    None,
    Pending,
    Passed,
    Failed,
}

/// Whether a pull request can be merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeableState {
    Unknown,
    Mergeable,
    Blocked,
    Conflicting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

// ---- entities ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub display_name: String,
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub full_name: Option<String>,
    pub default_branch: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: User,
    pub body: String,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentThread {
    pub id: String,
    pub comments: Vec<Comment>,
    pub file_path: Option<String>,
    pub line: Option<i64>,
    pub is_resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reviewer {
    pub user: User,
    pub vote: ReviewVote,
    pub is_required: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckSummary {
    pub successful: u32,
    pub in_progress: u32,
    pub failed: u32,
    pub neutral: u32,
}

impl CheckSummary {
    pub fn total(&self) -> u32 {
        self.successful + self.in_progress + self.failed + self.neutral
    }
}

/// A single named CI check / status on a pull request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRun {
    pub name: String,
    pub status: CheckStatus,
    pub url: Option<String>,
}

/// A commit on a pull request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub date: Option<DateTime<Utc>>,
    pub url: Option<String>,
}

/// Which side of a diff a line belongs to: the old (removed) or new (added) file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiffSide {
    Old,
    New,
}

/// A pending review comment targeting a specific line of a file in a pull request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineComment {
    pub path: String,
    pub line: i64,
    pub side: DiffSide,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: FileChangeKind,
    pub additions: i64,
    pub deletions: i64,
    /// Unified-diff patch text when the provider supplies it.
    pub patch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub id: String,
    pub number: Option<i64>,
    pub title: String,
    pub description: Option<String>,
    pub author: User,
    pub status: PullRequestStatus,
    pub is_draft: bool,
    pub source_ref: Option<String>,
    pub target_ref: Option<String>,
    pub reviewers: Vec<Reviewer>,
    pub labels: Vec<String>,
    pub checks: CheckStatus,
    pub check_summary: Option<CheckSummary>,
    pub mergeable: MergeableState,
    pub changed_files: i64,
    pub additions: i64,
    pub deletions: i64,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: String,
    pub identifier: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub state_category: WorkItemStateCategory,
    pub work_item_type: Option<String>,
    pub assignee: Option<User>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineDefinition {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    pub name: String,
    pub status: PipelineRunStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineJob {
    pub id: String,
    pub name: String,
    pub status: PipelineRunStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub steps: Vec<PipelineStep>,
    /// Deep link to this job in the provider's web UI.
    pub url: Option<String>,
    /// A short problem summary for failed jobs (provider-specific: GitLab's
    /// failure reason, Azure's error/warning counts, etc.).
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStage {
    pub name: String,
    pub status: PipelineRunStatus,
    pub jobs: Vec<PipelineJob>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRun {
    pub id: String,
    pub definition_id: String,
    pub number: Option<i64>,
    pub name: Option<String>,
    pub status: PipelineRunStatus,
    pub triggered_by: Option<User>,
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub url: Option<String>,
    pub stages: Vec<PipelineStage>,
}

/// A gate on a pipeline run that is waiting for a manual decision (a deployment
/// environment reviewer, an approval check, or a manual job).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineApproval {
    /// Provider-native identifier used to act on this gate (environment id,
    /// approval id, or manual job id, depending on the provider).
    pub id: String,
    /// Human label for the gate — usually the environment or stage name.
    pub name: String,
    /// Whether the authenticated user is allowed to respond to this gate.
    pub can_respond: bool,
}

/// A decision on a pending [`PipelineApproval`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    Approve,
    Reject,
}
