//! Capability-scoped provider traits, connection descriptor, capabilities, queries,
//! and the provider registry (mirrors `Forgetop.Core.Providers`).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::*;
use crate::error::{Error, Result};

// ---- connection descriptor ----

/// A configured connection: identity, optional scope, and a reference to the PAT in
/// the secret store (never the secret itself).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub provider_type: ProviderType,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Identity for Basic-auth providers (e.g. Jira email, Bitbucket username).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

impl Connection {
    pub fn new_id(provider: ProviderType) -> String {
        format!("{}-{}", provider.as_str().to_lowercase(), uuid::Uuid::new_v4().simple())
    }
}

// ---- capabilities ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoteStyle {
    BinaryApprove,
    NumericVotes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Terminology {
    pub pull_requests: String,
    pub work_items: String,
    pub pipelines: String,
}

impl Default for Terminology {
    fn default() -> Self {
        Self {
            pull_requests: "Pull Requests".into(),
            work_items: "Work Items".into(),
            pipelines: "Pipelines".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub supports_pull_requests: bool,
    pub supports_work_items: bool,
    pub supports_pipelines: bool,
    pub vote_style: VoteStyle,
    pub supports_merge: bool,
    pub supports_inline_comments: bool,
    pub supports_pipeline_trigger: bool,
    pub supports_pipeline_discovery: bool,
    /// Whether this provider exposes a personal notification feed (GitHub / GitLab / Linear).
    pub supports_notifications: bool,
    pub terminology: Terminology,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            supports_pull_requests: false,
            supports_work_items: false,
            supports_pipelines: false,
            vote_style: VoteStyle::BinaryApprove,
            supports_merge: false,
            supports_inline_comments: false,
            supports_pipeline_trigger: false,
            supports_pipeline_discovery: false,
            supports_notifications: false,
            terminology: Terminology::default(),
        }
    }
}

impl Capabilities {
    pub fn supports(&self, section: Section) -> bool {
        match section {
            Section::PullRequests => self.supports_pull_requests,
            Section::WorkItems => self.supports_work_items,
            Section::Pipelines => self.supports_pipelines,
        }
    }
}

// ---- queries ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PullRequestFilter {
    #[default]
    All,
    Mine,
    ReviewRequested,
}

#[derive(Debug, Clone, Default)]
pub struct PullRequestQuery {
    pub filter: PullRequestFilter,
    pub include_completed: bool,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct WorkItemQuery {
    pub mine_only: bool,
    pub include_completed: bool,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct PipelineRunQuery {
    pub definition_id: Option<String>,
    pub branch: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStrategy {
    #[default]
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Default)]
pub struct MergeOptions {
    pub strategy: MergeStrategy,
    pub delete_source_ref: bool,
}

// ---- capability-scoped sources ----

#[async_trait]
pub trait PullRequestSource: Send + Sync {
    async fn list(&self, query: &PullRequestQuery) -> Result<Vec<PullRequest>>;
    async fn get(&self, id: &str) -> Result<PullRequest>;
    async fn threads(&self, pull_request_id: &str) -> Result<Vec<CommentThread>>;
    /// The event timeline (reviews/approvals, merges, state changes, …), oldest → newest.
    /// Defaults to empty for providers without a timeline/activity API.
    async fn timeline(&self, _pull_request_id: &str) -> Result<Vec<TimelineEvent>> {
        Ok(Vec::new())
    }
    async fn changes(&self, pull_request_id: &str) -> Result<Vec<FileChange>>;
    /// Individual named CI checks. Defaults to empty for providers that don't expose them.
    async fn checks(&self, _pull_request_id: &str) -> Result<Vec<CheckRun>> {
        Ok(Vec::new())
    }
    /// Commits on the pull request. Defaults to empty.
    async fn commits(&self, _pull_request_id: &str) -> Result<Vec<Commit>> {
        Ok(Vec::new())
    }
    /// Files changed by a single commit on the pull request (with inline patches
    /// where the provider supplies them). Defaults to unsupported (empty).
    async fn commit_changes(&self, _pull_request_id: &str, _sha: &str) -> Result<Vec<FileChange>> {
        Ok(Vec::new())
    }
    async fn add_comment(&self, pull_request_id: &str, body: &str) -> Result<()>;
    /// Replies to an existing comment thread (returned by [`threads`](Self::threads)), so you can
    /// answer someone else's comment in-thread rather than starting a new top-level one. Defaults
    /// to posting a plain top-level comment — providers whose threads are really flat (e.g. a
    /// GitHub PR *conversation*, which has no reply API) keep that behaviour; providers with real
    /// threads (Azure, GitLab discussions, Bitbucket, GitHub *review* threads) override it.
    async fn reply_to_thread(&self, pull_request_id: &str, _thread_id: &str, body: &str) -> Result<()> {
        self.add_comment(pull_request_id, body).await
    }
    async fn vote(&self, pull_request_id: &str, vote: ReviewVote) -> Result<()>;
    async fn merge(&self, pull_request_id: &str, options: &MergeOptions) -> Result<()>;
    /// Reverts a merged pull request (undoes its merge commit on the target branch). Defaults to
    /// unsupported — only providers with a revert API (GitLab, Azure DevOps) override this.
    async fn revert(&self, _pull_request_id: &str) -> Result<()> {
        Err(Error::Provider("this provider has no revert API — revert it from the provider's web UI".into()))
    }
    /// Submits a review with inline line comments. `event` maps to approve /
    /// request-changes / plain comment. Defaults to unsupported.
    async fn submit_review(&self, _pull_request_id: &str, _event: ReviewVote, _comments: &[LineComment]) -> Result<()> {
        Err(Error::Provider("this provider doesn't support line-comment reviews".into()))
    }
}

#[async_trait]
pub trait WorkItemSource: Send + Sync {
    async fn list(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>>;
    async fn get(&self, id: &str) -> Result<WorkItem>;
    async fn threads(&self, work_item_id: &str) -> Result<Vec<CommentThread>>;
    /// The event history (status changes, assignments, …), oldest → newest. Defaults to empty.
    async fn timeline(&self, _work_item_id: &str) -> Result<Vec<TimelineEvent>> {
        Ok(Vec::new())
    }
    async fn set_state(&self, work_item_id: &str, state: &str) -> Result<()>;
    async fn add_comment(&self, work_item_id: &str, body: &str) -> Result<()>;
    /// The states this item can move to (provider-accurate). Defaults to empty,
    /// letting the caller fall back to the states seen across the loaded items.
    async fn available_states(&self, _work_item_id: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    /// Users this item can be assigned to (candidates for the assignee picker). The `id` on each
    /// returned `User` MUST be the exact token this provider's `set_assignee` accepts. Empty by
    /// default (no picker shown).
    async fn assignable_users(&self, _work_item_id: &str) -> Result<Vec<User>> {
        Ok(Vec::new())
    }
    /// Assign the item to a user (by an id from `assignable_users`), or `None` to unassign.
    /// Unsupported by default.
    async fn set_assignee(&self, _work_item_id: &str, _assignee_id: Option<&str>) -> Result<()> {
        Err(Error::Provider("set_assignee not supported by this provider".into()))
    }
    /// Edit the item's title and/or description (`None` leaves that field unchanged).
    /// Unsupported by default.
    async fn update_fields(&self, _work_item_id: &str, _title: Option<&str>, _description: Option<&str>) -> Result<()> {
        Err(Error::Provider("update_fields not supported by this provider".into()))
    }
}

#[async_trait]
pub trait PipelineSource: Send + Sync {
    async fn discover(&self) -> Result<Vec<PipelineDefinition>>;
    async fn list_runs(&self, query: &PipelineRunQuery) -> Result<Vec<PipelineRun>>;
    async fn get_run(&self, run_id: &str) -> Result<PipelineRun>;
    async fn logs(&self, run_id: &str, job_id: Option<&str>) -> Result<String>;
    async fn trigger(&self, definition_id: &str, branch: Option<&str>) -> Result<()>;
    /// Cancel a running or queued run. Unsupported by default.
    async fn cancel_run(&self, _run_id: &str) -> Result<()> {
        Err(Error::Provider("cancel_run not supported by this provider".into()))
    }
    /// Whether this provider can surface and act on pending run approvals/gates.
    /// `false` by default — the UI shows the section as unsupported.
    fn supports_approvals(&self) -> bool {
        false
    }
    /// Whether the app can actually submit an approve/reject decision. Providers may
    /// be able to *surface* a pending gate (`supports_approvals`) without being able
    /// to act on it — e.g. Azure DevOps, where the pending environment check isn't
    /// exposed as an actionable approval resource. Defaults to `supports_approvals()`.
    fn can_respond_to_approvals(&self) -> bool {
        self.supports_approvals()
    }
    /// Gates on a run that are waiting for a decision. Empty by default (unsupported).
    async fn pending_approvals(&self, _run_id: &str) -> Result<Vec<PipelineApproval>> {
        Ok(Vec::new())
    }
    /// Approve or reject a waiting gate on a run. Unsupported by default.
    async fn respond_approval(
        &self,
        _run_id: &str,
        _approval_id: &str,
        _decision: ApprovalDecision,
        _comment: Option<&str>,
    ) -> Result<()> {
        Err(Error::Provider("this provider doesn't support pipeline approvals".into()))
    }
}

/// The provider's personal notification feed (GitHub notifications, GitLab todos, Linear
/// notifications). Only some providers have one — see [`Capabilities::supports_notifications`].
#[async_trait]
pub trait NotificationSource: Send + Sync {
    /// The current user's notifications, newest first.
    async fn list(&self) -> Result<Vec<Notification>>;
    /// Mark a single notification read by its id.
    async fn mark_read(&self, id: &str) -> Result<()>;
    /// Mark every notification read. Default: mark each one; providers with a bulk endpoint
    /// (GitHub `PUT /notifications`, GitLab `/todos/mark_as_done`) should override.
    async fn mark_all_read(&self) -> Result<()> {
        for n in self.list().await? {
            let _ = self.mark_read(&n.id).await;
        }
        Ok(())
    }
}

/// A live, authenticated connection exposing only the sources it supports.
#[async_trait]
pub trait ProviderConnection: Send + Sync {
    fn connection_id(&self) -> &str;
    fn provider_type(&self) -> ProviderType;
    fn display_name(&self) -> &str;
    fn capabilities(&self) -> &Capabilities;
    fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>>;
    fn work_items(&self) -> Option<Arc<dyn WorkItemSource>>;
    fn pipelines(&self) -> Option<Arc<dyn PipelineSource>>;
    /// The notification feed, when the provider has one. Defaults to `None` (unsupported).
    fn notifications(&self) -> Option<Arc<dyn NotificationSource>> {
        None
    }
    /// Cheap reachability/auth check for the connections health bar.
    async fn check(&self) -> bool;
}

/// Builds a live [`ProviderConnection`] for one provider type.
pub trait ProviderFactory: Send + Sync {
    fn provider_type(&self) -> ProviderType;
    fn describe_capabilities(&self) -> Capabilities;
    fn create(&self, connection: &Connection, secret: Option<String>) -> Result<Arc<dyn ProviderConnection>>;
}

/// Resolves provider factories and creates connections by provider type.
pub struct ProviderRegistry {
    factories: HashMap<ProviderType, Arc<dyn ProviderFactory>>,
}

impl ProviderRegistry {
    pub fn new(factories: Vec<Arc<dyn ProviderFactory>>) -> Self {
        let factories = factories.into_iter().map(|f| (f.provider_type(), f)).collect();
        Self { factories }
    }

    pub fn available(&self) -> Vec<ProviderType> {
        self.factories.keys().copied().collect()
    }

    pub fn supports(&self, provider: ProviderType) -> bool {
        self.factories.contains_key(&provider)
    }

    pub fn describe(&self, provider: ProviderType) -> Option<Capabilities> {
        self.factories.get(&provider).map(|f| f.describe_capabilities())
    }

    pub fn create(&self, connection: &Connection, secret: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
        let factory = self
            .factories
            .get(&connection.provider_type)
            .ok_or_else(|| Error::Config(format!("no provider registered for '{}'", connection.provider_type.as_str())))?;
        factory.create(connection, secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct FakeConn {
        caps: Capabilities,
    }

    #[async_trait]
    impl ProviderConnection for FakeConn {
        fn connection_id(&self) -> &str {
            "fake"
        }
        fn provider_type(&self) -> ProviderType {
            ProviderType::GitHub
        }
        fn display_name(&self) -> &str {
            "Fake"
        }
        fn capabilities(&self) -> &Capabilities {
            &self.caps
        }
        fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
            None
        }
        fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
            None
        }
        fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
            None
        }
        async fn check(&self) -> bool {
            true
        }
    }

    struct FakeFactory {
        provider: ProviderType,
        caps: Capabilities,
    }

    impl ProviderFactory for FakeFactory {
        fn provider_type(&self) -> ProviderType {
            self.provider
        }
        fn describe_capabilities(&self) -> Capabilities {
            self.caps.clone()
        }
        fn create(&self, _connection: &Connection, _secret: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
            Ok(Arc::new(FakeConn { caps: self.caps.clone() }))
        }
    }

    fn conn(provider: ProviderType) -> Connection {
        Connection {
            id: "c".into(),
            provider_type: provider,
            display_name: "c".into(),
            base_url: None,
            organization: None,
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
        }
    }

    #[test]
    fn capabilities_supports_reflects_flags() {
        let caps = Capabilities { supports_pull_requests: true, ..Default::default() };
        assert!(caps.supports(Section::PullRequests));
        assert!(!caps.supports(Section::WorkItems));
    }

    #[test]
    fn registry_dispatches_and_rejects_unknown() {
        let caps = Capabilities { supports_pull_requests: true, ..Default::default() };
        let registry = ProviderRegistry::new(vec![Arc::new(FakeFactory { provider: ProviderType::GitHub, caps })]);

        assert!(registry.supports(ProviderType::GitHub));
        assert!(!registry.supports(ProviderType::Linear));

        let live = registry.create(&conn(ProviderType::GitHub), Some("pat".into())).unwrap();
        assert_eq!(live.provider_type(), ProviderType::GitHub);
        assert!(registry.create(&conn(ProviderType::Linear), None).is_err());
    }

    #[test]
    fn new_id_is_prefixed() {
        assert!(Connection::new_id(ProviderType::GitHub).starts_with("github-"));
    }
}
