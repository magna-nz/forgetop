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
    /// In flight but parked: held on an approval gate, so nothing is running and nothing will
    /// until someone acts. A stage behind the gate reads this too.
    Waiting,
    Succeeded,
    PartiallySucceeded,
    Failed,
    Canceled,
    /// Never ran — skipped by its condition, or because something before it failed. Not a
    /// failure in its own right.
    Skipped,
}

impl PipelineRunStatus {
    /// Still in flight: queued, running, or waiting on a gate. Only these can be cancelled or
    /// held by an approval.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Waiting)
    }

    /// An active status, held: [`Waiting`](Self::Waiting). Callers with a whole run should use
    /// [`PipelineRun::shown_status`], which also checks nothing is still executing.
    pub fn with_pending_approval(self, pending: bool) -> Self {
        if pending && self.is_active() {
            Self::Waiting
        } else {
            self
        }
    }
}

/// Roll-up CI/check state for a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CheckStatus {
    #[default]
    None,
    Pending,
    Passed,
    Failed,
}

/// Whether a pull request can be merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MergeableState {
    #[default]
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
    /// Whether the forge can mark this thread resolved at all. `false` for threads a forge only
    /// bundles (GitHub's flat conversation comments, GitLab's individual notes, work-item
    /// comments), so a frontend offers "resolve" only where it would work.
    #[serde(default = "resolvable_by_default")]
    pub is_resolvable: bool,
}

/// Serde default for [`CommentThread::is_resolvable`]: a thread cached before the field
/// existed is assumed resolvable, which is the common case.
fn resolvable_by_default() -> bool {
    true
}

/// What happened in a timeline event — drives its icon/colour in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimelineEventKind {
    Approved,
    ChangesRequested,
    Reviewed,
    Commented,
    Merged,
    Closed,
    Reopened,
    StateChanged,
    Assigned,
    Labeled,
    Committed,
    Other,
}

/// A single event on a pull request or work item (a review, a merge, a state change, an
/// assignment, …), assembled from whatever the provider's timeline / activity / history API
/// exposes. Ordered oldest → newest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub actor: Option<User>,
    pub kind: TimelineEventKind,
    /// Human-readable one-liner, e.g. "approved", "changed status to In Progress", "merged".
    pub summary: String,
    pub at: Option<DateTime<Utc>>,
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
    /// The repository this pull request lives in, **connection-relative** (`acme/pay`) — see
    /// [`crate::repo`]. `None` for providers that aren't repo-addressed (Jira, Linear).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
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
    /// Where this item lives, **connection-relative** — a repository (`acme/pay`) for the
    /// repo-addressed forges, or a bare project name for Azure DevOps, whose work items are
    /// project-addressed rather than repo-addressed. `None` for Jira and Linear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
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
    /// The repository this definition belongs to, **connection-relative**. `None` for providers
    /// that aren't repo-addressed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
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
    /// The repository this run belongs to, **connection-relative**. `None` for providers that
    /// aren't repo-addressed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    pub definition_id: String,
    pub number: Option<i64>,
    pub name: Option<String>,
    /// A human-readable title for the run — the triggering PR title or commit-message subject
    /// where the provider exposes it (GitHub `display_title`). `None` when unavailable.
    pub title: Option<String>,
    pub status: PipelineRunStatus,
    pub triggered_by: Option<User>,
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub url: Option<String>,
    pub stages: Vec<PipelineStage>,
    /// What started the run, in the provider's own words (`push`, `pull_request`,
    /// `schedule`, Azure `individualCI`, GitLab `merge_request_event`, …). `None` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// Which attempt of this run this is (GitHub `run_attempt`); `None` where the provider
    /// doesn't number re-runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    /// The pull/merge request this run was built for, by number, when the provider says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<i64>,
}

/// How serious a [`PipelineAnnotation`] is. Ordered most severe first, so sorting by it puts
/// failures on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AnnotationLevel {
    Failure,
    Warning,
    Notice,
}

/// A problem a run reported, usually anchored to a file and line: a GitHub check-run
/// annotation, a GitLab failed test case, an Azure timeline issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineAnnotation {
    pub level: AnnotationLevel,
    pub message: String,
    /// Short heading where the provider gives one (a test name, an annotation title).
    pub title: Option<String>,
    /// Repository-relative file path, when the problem is anchored to one.
    pub path: Option<String>,
    pub line: Option<u32>,
    /// The job that reported it, matching a [`PipelineJob::id`], when known.
    pub job_id: Option<String>,
}

/// A file a run published (build output, report, installer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineArtifact {
    pub id: String,
    pub name: String,
    pub size_bytes: Option<u64>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Where a person opens or downloads it in a browser.
    pub url: Option<String>,
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
    /// Whether the run is held until this gate is answered. An environment approval or an Azure
    /// check is; an optional GitLab manual job (`allow_failure: true`) isn't — the pipeline runs
    /// on without it, so it mustn't make the run read as [`PipelineRunStatus::Waiting`].
    #[serde(default = "holds_by_default")]
    pub blocks_run: bool,
}

fn holds_by_default() -> bool {
    true
}

/// Addressing helpers: each item knows the repository it came from, so a call site never has to
/// reconstruct one. Use these instead of building an [`crate::provider::ItemRef`] by hand.
impl PullRequest {
    pub fn item_ref(&self) -> crate::provider::ItemRef {
        crate::provider::ItemRef::maybe(self.repository.clone(), self.id.clone())
    }
}

impl WorkItem {
    pub fn item_ref(&self) -> crate::provider::ItemRef {
        crate::provider::ItemRef::maybe(self.repository.clone(), self.id.clone())
    }
}

impl PipelineRun {
    pub fn item_ref(&self) -> crate::provider::ItemRef {
        crate::provider::ItemRef::maybe(self.repository.clone(), self.id.clone())
    }

    /// Addresses this run's *definition* (for a re-run/trigger), not the run itself.
    pub fn definition_ref(&self) -> crate::provider::ItemRef {
        crate::provider::ItemRef::maybe(self.repository.clone(), self.definition_id.clone())
    }

    /// Whether any stage, job or step of this run is executing right now. A list row usually
    /// carries no stages, so this is only ever evidence *for* running, never against.
    pub fn has_running_work(&self) -> bool {
        let running = PipelineRunStatus::Running;
        self.stages.iter().any(|s| {
            s.status == running || s.jobs.iter().any(|j| j.status == running || j.steps.iter().any(|t| t.status == running))
        })
    }

    /// The status to show for this run. It reads [`Waiting`](PipelineRunStatus::Waiting) when it
    /// is in flight, held on a gate — a pending approval that `blocks_run`, or a stage the
    /// provider itself reports as waiting — and nothing in it is still executing: a gate further
    /// on doesn't make a run that's still building "waiting". Every surface asks here, so the
    /// list, the run view, the Command Center and the dashboard can't disagree.
    pub fn shown_status(&self, blocking_gate: bool) -> PipelineRunStatus {
        let held = blocking_gate || self.stages.iter().any(|s| s.status == PipelineRunStatus::Waiting);
        self.status.with_pending_approval(held && !self.has_running_work())
    }
}

impl PipelineDefinition {
    pub fn item_ref(&self) -> crate::provider::ItemRef {
        crate::provider::ItemRef::maybe(self.repository.clone(), self.id.clone())
    }
}

/// A decision on a pending [`PipelineApproval`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    Approve,
    Reject,
}

/// Why a notification fired — each provider's native reason/action maps onto one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationKind {
    /// You were asked to review a PR/MR.
    ReviewRequested,
    /// You were @-mentioned (comment, description, team mention).
    Mention,
    /// Something was assigned to you.
    Assigned,
    /// CI failed on something of yours.
    CiFailed,
    /// A new comment/reply on something you follow.
    Comment,
    /// The state of something you follow changed (merged, closed, status).
    StateChange,
    /// Anything else the provider notified about.
    Other,
}

/// What kind of item a notification points at — drives its icon and which source opens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationItemType {
    PullRequest,
    WorkItem,
    Pipeline,
    Other,
}

/// One item in the cross-provider notification inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// Provider-native notification id — used to mark it read.
    pub id: String,
    pub kind: NotificationKind,
    pub item_type: NotificationItemType,
    /// The underlying PR / work-item / pipeline id, for opening it in-app. `None` when the
    /// notification has no resolvable item (then we fall back to the web URL).
    pub item_id: Option<String>,
    /// The repository the item lives in, **connection-relative** — needed to address `item_id`
    /// on a connection that spans several repositories. `None` when the provider doesn't say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Subject line — the PR/issue title.
    pub title: String,
    /// Where it lives: `org/repo`, or the project / team name.
    pub context: String,
    /// Web URL, for the browser fallback.
    pub url: Option<String>,
    pub unread: bool,
    pub updated_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod pipeline_status_tests {
    use super::PipelineRunStatus::{self, *};

    #[test]
    fn waiting_is_in_flight_and_skipped_is_not() {
        for s in [Queued, Running, Waiting] {
            assert!(s.is_active(), "{s:?}");
        }
        for s in [Succeeded, PartiallySucceeded, Failed, Canceled, Skipped] {
            assert!(!s.is_active(), "{s:?}");
        }
    }

    #[test]
    fn a_pending_gate_holds_only_an_active_run() {
        assert_eq!(Running.with_pending_approval(true), Waiting);
        assert_eq!(Queued.with_pending_approval(true), Waiting);
        assert_eq!(Running.with_pending_approval(false), Running);
        // A finished run's leftover gate doesn't rewrite its outcome.
        assert_eq!(Succeeded.with_pending_approval(true), Succeeded);
        assert_eq!(Failed.with_pending_approval(true), Failed);
    }

    fn run_with(status: PipelineRunStatus, stages: Vec<super::PipelineStage>) -> super::PipelineRun {
        super::PipelineRun {
            event: None, attempt: None, pull_request: None, repository: None, id: "1".into(), definition_id: "d".into(),
            number: None, name: None, title: None, status, triggered_by: None, branch: None, commit_sha: None,
            started_at: None, finished_at: None, url: None, stages,
        }
    }

    fn stage(status: PipelineRunStatus) -> super::PipelineStage {
        super::PipelineStage { name: "s".into(), status, jobs: vec![] }
    }

    #[test]
    fn a_run_reads_waiting_only_when_a_gate_holds_it_and_nothing_runs() {
        // A list row: no stages to say otherwise, so a blocking gate parks it.
        assert_eq!(run_with(Running, vec![]).shown_status(true), Waiting);
        assert_eq!(run_with(Running, vec![]).shown_status(false), Running);
        // A stage still executing keeps the run Running whatever is gated further on.
        assert_eq!(run_with(Running, vec![stage(Succeeded), stage(Running), stage(Queued)]).shown_status(true), Running);
        // The provider's own Waiting stage holds the run even before the approvals call answers.
        assert_eq!(run_with(Running, vec![stage(Succeeded), stage(Waiting)]).shown_status(false), Waiting);
        // A finished run is never rewritten.
        assert_eq!(run_with(Canceled, vec![stage(Waiting)]).shown_status(true), Canceled);
    }

    #[test]
    fn an_approval_from_an_older_cache_still_holds_its_run() {
        let a: super::PipelineApproval = serde_json::from_str(r#"{ "id": "g", "name": "Prod", "can_respond": true }"#).unwrap();
        assert!(a.blocks_run);
    }

    #[test]
    fn new_states_round_trip_by_name() {
        for s in [Waiting, Skipped] {
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(serde_json::from_str::<PipelineRunStatus>(&json).unwrap(), s);
        }
        assert_eq!(serde_json::to_string(&Waiting).unwrap(), "\"Waiting\"");
    }
}
