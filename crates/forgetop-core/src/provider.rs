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
    /// The repositories this connection fetches from, **connection-relative** (`acme/pay` — see
    /// [`crate::repo`]).
    ///
    /// Three states, all distinct and all meaningful:
    /// * `None` — never established. Fall back to the legacy single [`Self::repository`].
    /// * `Some([])` — the user chose no repositories. Fetch nothing; do **not** refill from the
    ///   legacy field, or an intentionally-emptied scope silently comes back on the next load.
    /// * `Some([…])` — fetch these.
    ///
    /// Fallbacks must therefore key on the scope being *absent*, never on it being *empty*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_scope: Option<Vec<String>>,
}

impl Connection {
    pub fn new_id(provider: ProviderType) -> String {
        format!("{}-{}", provider.as_str().to_lowercase(), uuid::Uuid::new_v4().simple())
    }

    /// The repositories to fetch from, resolved at factory time so nothing on disk is rewritten:
    /// an explicit scope wins whenever it is `Some`, and only its absence falls back to the
    /// legacy single repository (via `legacy`, which each provider spells its own way). An
    /// existing single-repository connection therefore yields a one-element scope and behaves
    /// exactly as before.
    pub fn resolve_repo_scope(&self, legacy: impl FnOnce() -> Option<String>) -> Vec<String> {
        match &self.repo_scope {
            Some(scope) => scope.clone(),
            None => legacy().into_iter().collect(),
        }
    }
}

/// Addresses one item inside a connection: which repository it lives in, plus the
/// provider-native id.
///
/// `repo` is always **connection-relative** (`acme/pay`) — never host-qualified. It is `None`
/// only when the caller genuinely has no repository to give; a provider whose scope holds a
/// single repository resolves that case, and one with a wider scope rejects it rather than
/// guessing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub id: String,
}

impl ItemRef {
    /// An unaddressed reference — only resolvable against a single-repository scope.
    pub fn new(id: impl Into<String>) -> Self {
        ItemRef { repo: None, id: id.into() }
    }

    /// A fully addressed reference. `repo` must be connection-relative.
    pub fn in_repo(repo: impl Into<String>, id: impl Into<String>) -> Self {
        ItemRef { repo: Some(repo.into()), id: id.into() }
    }

    /// Builds a reference from an optional connection-relative repository.
    pub fn maybe(repo: Option<String>, id: impl Into<String>) -> Self {
        ItemRef { repo, id: id.into() }
    }
}

/// The repositories a connection's credentials can reach, for the scope picker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryPage {
    /// Connection-relative repository paths, most-recently-active first.
    pub repositories: Vec<String>,
    /// True when the provider had more than we fetched, so the picker can say "5 of 500+"
    /// rather than presenting a cap as a total. Truncation must never be silent.
    pub truncated: bool,
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

#[derive(Debug, Clone)]
pub struct PullRequestQuery {
    pub filter: PullRequestFilter,
    pub include_completed: bool,
    pub limit: Option<u32>,
    /// Whether to spend extra per-PR calls filling the fields a provider's *list* endpoint omits
    /// (GitHub leaves out `mergeable`, `changed_files`, `additions`, `deletions` entirely).
    ///
    /// Defaults to **true**, so no caller loses decoration by accident. Callers that render
    /// decorated fields lazily — the dashboard fetches them per visible row — turn it off, and
    /// providers that honour it must still bound the work to the rows they finally return, so
    /// the cost does not multiply with the size of the repository scope.
    pub decorate: bool,
}

impl Default for PullRequestQuery {
    fn default() -> Self {
        Self { filter: PullRequestFilter::default(), include_completed: false, limit: None, decorate: true }
    }
}

/// The fields a provider's list endpoint omits, fetched on demand for one pull request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrDecoration {
    pub mergeable: MergeableState,
    pub changed_files: i64,
    pub additions: i64,
    pub deletions: i64,
    pub checks: CheckStatus,
    pub check_summary: Option<CheckSummary>,
}

impl PrDecoration {
    /// Projects the decorated fields out of a fully-fetched pull request.
    pub fn from_pull_request(pr: &PullRequest) -> Self {
        Self {
            mergeable: pr.mergeable,
            changed_files: pr.changed_files,
            additions: pr.additions,
            deletions: pr.deletions,
            checks: pr.checks,
            check_summary: pr.check_summary.clone(),
        }
    }
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
    /// Restricts the query to one **connection-relative** repository. `None` fans out over the
    /// connection's whole repository scope.
    pub repository: Option<String>,
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

/// Pull requests from one connection. Every method that names a single pull request takes an
/// [`ItemRef`], not a bare id: on a connection spanning several repositories the id alone does
/// not say *which* `#7` is meant, and guessing silently opens, merges or comments on the wrong
/// one. The `repo` on the ref is connection-relative.
#[async_trait]
pub trait PullRequestSource: Send + Sync {
    async fn list(&self, query: &PullRequestQuery) -> Result<Vec<PullRequest>>;
    async fn get(&self, item: &ItemRef) -> Result<PullRequest>;
    async fn threads(&self, item: &ItemRef) -> Result<Vec<CommentThread>>;
    /// The event timeline (reviews/approvals, merges, state changes, …), oldest → newest.
    /// Defaults to empty for providers without a timeline/activity API.
    async fn timeline(&self, _item: &ItemRef) -> Result<Vec<TimelineEvent>> {
        Ok(Vec::new())
    }
    async fn changes(&self, item: &ItemRef) -> Result<Vec<FileChange>>;
    /// Individual named CI checks. Defaults to empty for providers that don't expose them.
    async fn checks(&self, _item: &ItemRef) -> Result<Vec<CheckRun>> {
        Ok(Vec::new())
    }
    /// Commits on the pull request. Defaults to empty.
    async fn commits(&self, _item: &ItemRef) -> Result<Vec<Commit>> {
        Ok(Vec::new())
    }
    /// The fields the list endpoint omits, for one pull request. Defaults to projecting them out
    /// of a full [`get`](Self::get) — providers with a cheaper route override it.
    async fn decorate(&self, item: &ItemRef) -> Result<PrDecoration> {
        Ok(PrDecoration::from_pull_request(&self.get(item).await?))
    }
    /// Files changed by a single commit on the pull request (with inline patches
    /// where the provider supplies them). Defaults to unsupported (empty).
    async fn commit_changes(&self, _item: &ItemRef, _sha: &str) -> Result<Vec<FileChange>> {
        Ok(Vec::new())
    }
    async fn add_comment(&self, item: &ItemRef, body: &str) -> Result<()>;
    /// Replies to an existing comment thread (returned by [`threads`](Self::threads)), so you can
    /// answer someone else's comment in-thread rather than starting a new top-level one. Defaults
    /// to posting a plain top-level comment — providers whose threads are really flat (e.g. a
    /// GitHub PR *conversation*, which has no reply API) keep that behaviour; providers with real
    /// threads (Azure, GitLab discussions, Bitbucket, GitHub *review* threads) override it.
    async fn reply_to_thread(&self, item: &ItemRef, _thread_id: &str, body: &str) -> Result<()> {
        self.add_comment(item, body).await
    }
    async fn vote(&self, item: &ItemRef, vote: ReviewVote) -> Result<()>;
    async fn merge(&self, item: &ItemRef, options: &MergeOptions) -> Result<()>;
    /// Reverts a merged pull request (undoes its merge commit on the target branch). Defaults to
    /// unsupported — only providers with a revert API (GitLab, Azure DevOps) override this.
    async fn revert(&self, _item: &ItemRef) -> Result<()> {
        Err(Error::Provider("this provider has no revert API — revert it from the provider's web UI".into()))
    }
    /// Submits a review with inline line comments. `event` maps to approve /
    /// request-changes / plain comment. Defaults to unsupported.
    async fn submit_review(&self, _item: &ItemRef, _event: ReviewVote, _comments: &[LineComment]) -> Result<()> {
        Err(Error::Provider("this provider doesn't support line-comment reviews".into()))
    }
}

#[async_trait]
pub trait WorkItemSource: Send + Sync {
    async fn list(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>>;
    async fn get(&self, item: &ItemRef) -> Result<WorkItem>;
    async fn threads(&self, item: &ItemRef) -> Result<Vec<CommentThread>>;
    /// The event history (status changes, assignments, …), oldest → newest. Defaults to empty.
    async fn timeline(&self, _item: &ItemRef) -> Result<Vec<TimelineEvent>> {
        Ok(Vec::new())
    }
    async fn set_state(&self, item: &ItemRef, state: &str) -> Result<()>;
    async fn add_comment(&self, item: &ItemRef, body: &str) -> Result<()>;
    /// The states this item can move to (provider-accurate). Defaults to empty,
    /// letting the caller fall back to the states seen across the loaded items.
    async fn available_states(&self, _item: &ItemRef) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    /// Users this item can be assigned to (candidates for the assignee picker). The `id` on each
    /// returned `User` MUST be the exact token this provider's `set_assignee` accepts. Empty by
    /// default (no picker shown).
    async fn assignable_users(&self, _item: &ItemRef) -> Result<Vec<User>> {
        Ok(Vec::new())
    }
    /// Assign the item to a user (by an id from `assignable_users`), or `None` to unassign.
    /// Unsupported by default.
    async fn set_assignee(&self, _item: &ItemRef, _assignee_id: Option<&str>) -> Result<()> {
        Err(Error::Provider("set_assignee not supported by this provider".into()))
    }
    /// Edit the item's title and/or description (`None` leaves that field unchanged).
    /// Unsupported by default.
    async fn update_fields(&self, _item: &ItemRef, _title: Option<&str>, _description: Option<&str>) -> Result<()> {
        Err(Error::Provider("update_fields not supported by this provider".into()))
    }
}

#[async_trait]
pub trait PipelineSource: Send + Sync {
    async fn discover(&self) -> Result<Vec<PipelineDefinition>>;
    async fn list_runs(&self, query: &PipelineRunQuery) -> Result<Vec<PipelineRun>>;
    async fn get_run(&self, run: &ItemRef) -> Result<PipelineRun>;
    async fn logs(&self, run: &ItemRef, job_id: Option<&str>) -> Result<String>;
    /// Triggers a pipeline. `definition` addresses the definition — its `repo` says which
    /// repository's pipeline to start, which a connection spanning several cannot infer.
    async fn trigger(&self, definition: &ItemRef, branch: Option<&str>) -> Result<()>;
    /// Cancel a running or queued run. Unsupported by default.
    async fn cancel_run(&self, _run: &ItemRef) -> Result<()> {
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
    async fn pending_approvals(&self, _run: &ItemRef) -> Result<Vec<PipelineApproval>> {
        Ok(Vec::new())
    }
    /// Approve or reject a waiting gate on a run. Unsupported by default.
    async fn respond_approval(
        &self,
        _run: &ItemRef,
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
    /// The repositories these credentials can reach, most-recently-active first — the candidate
    /// list for the connection's repository scope picker.
    ///
    /// Defaults to empty, which is the right answer for providers that aren't repo-addressed
    /// (Jira is project-addressed, Linear team-addressed). Only the scope picker calls this, so
    /// a provider whose discovery is wrong shows an empty picker; it cannot break fetching for a
    /// connection that already has a scope.
    async fn discover_repositories(&self) -> Result<RepositoryPage> {
        Ok(RepositoryPage::default())
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
            repo_scope: None,
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
    fn scope_falls_back_only_when_absent_never_when_empty() {
        let mut c = conn(ProviderType::GitHub);

        // Never established → the legacy single repository still applies, so an existing
        // connection keeps behaving exactly as it did.
        c.repository = Some("acme/pay".into());
        assert_eq!(c.resolve_repo_scope(|| c.repository.clone()), vec!["acme/pay".to_string()]);

        // Chosen, and empty. This is the state a bare Vec cannot express: fetch nothing. If the
        // fallback keyed on emptiness instead of absence, the legacy repository would silently
        // refill it on the next load.
        c.repo_scope = Some(vec![]);
        assert!(c.resolve_repo_scope(|| c.repository.clone()).is_empty());

        // Chosen explicitly → exactly those, legacy ignored.
        c.repo_scope = Some(vec!["acme/pay".into(), "acme/ledger".into()]);
        assert_eq!(c.resolve_repo_scope(|| c.repository.clone()).len(), 2);
    }

    #[test]
    fn item_ref_carries_the_connection_relative_repo() {
        let r = ItemRef::in_repo("acme/pay", "7");
        assert_eq!(r.repo.as_deref(), Some("acme/pay"));
        assert_eq!(ItemRef::new("7").repo, None);
        assert_eq!(ItemRef::maybe(None, "7"), ItemRef::new("7"));
    }

    #[test]
    fn pull_request_query_decorates_by_default() {
        // Nothing loses decoration by accident — a caller must opt out on purpose.
        assert!(PullRequestQuery::default().decorate);
    }

    #[test]
    fn new_id_is_prefixed() {
        assert!(Connection::new_id(ProviderType::GitHub).starts_with("github-"));
    }
}
