//! Bitbucket Cloud provider (Pull Requests + Pipelines): pure mappers
//! (fixture-tested) + a reqwest client. Basic auth (`username:app_password`).

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use forgetop_core::domain::*;
use forgetop_core::filter::apply_pull_request_filter;
use forgetop_core::provider::*;
use forgetop_core::{Error, Result};
use reqwest::header::AUTHORIZATION;
use serde_json::{json, Value};

use crate::json::*;
use crate::scope::{self, fan_out, sort_and_cap};

fn prov<E: std::fmt::Display>(e: E) -> Error {
    Error::Provider(e.to_string())
}

fn unknown_user() -> User {
    User { id: "unknown".into(), display_name: "unknown".into(), handle: None, avatar_url: None }
}

fn enc_uuid(s: &str) -> String {
    s.replace('{', "%7B").replace('}', "%7D")
}

// ---- mappers ----

pub fn map_user(v: &Value) -> User {
    User {
        id: get_str(v, "uuid").or_else(|| get_str(v, "account_id")).unwrap_or_else(|| "unknown".into()),
        display_name: get_str(v, "display_name").or_else(|| get_str(v, "nickname")).unwrap_or_else(|| "unknown".into()),
        handle: get_str(v, "nickname"),
        avatar_url: get_obj(v, "links").and_then(|l| get_obj(l, "avatar")).and_then(|a| get_str(a, "href")),
    }
}

fn branch_name(v: &Value, key: &str) -> Option<String> {
    get_obj(v, key).and_then(|s| get_obj(s, "branch")).and_then(|b| get_str(b, "name"))
}

fn html_url(v: &Value) -> Option<String> {
    get_obj(v, "links").and_then(|l| get_obj(l, "html")).and_then(|h| get_str(h, "href"))
}

pub fn map_pull_request(v: &Value, repo: Option<&str>) -> PullRequest {
    let state = get_str(v, "state");
    let draft = get_bool(v, "draft");
    let status = match state.as_deref() {
        Some("MERGED") => PullRequestStatus::Merged,
        Some("DECLINED") | Some("SUPERSEDED") => PullRequestStatus::Closed,
        _ if draft => PullRequestStatus::Draft,
        _ => PullRequestStatus::Open,
    };
    let number = get_i64(v, "id");
    PullRequest {
        // `full_name` is already `workspace/repo` — the connection-relative spelling.
        repository: get_obj(v, "destination")
            .and_then(|d| get_obj(d, "repository"))
            .and_then(|r| get_str(r, "full_name"))
            .or_else(|| repo.map(str::to_string)),
        id: number.map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        number,
        title: get_str(v, "title").unwrap_or_else(|| "(untitled)".into()),
        description: get_obj(v, "rendered")
            .and_then(|r| get_obj(r, "description"))
            .and_then(|d| get_str(d, "raw"))
            .or_else(|| get_str(v, "description")),
        author: get_obj(v, "author").map(map_user).unwrap_or_else(unknown_user),
        status,
        is_draft: draft,
        source_ref: branch_name(v, "source"),
        target_ref: branch_name(v, "destination"),
        reviewers: get_arr(v, "participants")
            .iter()
            .filter(|p| get_str(p, "role").as_deref() == Some("REVIEWER"))
            .map(|p| Reviewer {
                user: get_obj(p, "user").map(map_user).unwrap_or_else(unknown_user),
                vote: if get_bool(p, "approved") { ReviewVote::Approved } else { ReviewVote::NoVote },
                is_required: false,
            })
            .collect(),
        labels: vec![],
        checks: CheckStatus::None,
        check_summary: None,
        mergeable: MergeableState::Unknown,
        changed_files: 0,
        additions: 0,
        deletions: 0,
        created_at: get_date(v, "created_on"),
        updated_at: get_date(v, "updated_on"),
        url: html_url(v),
    }
}

pub fn map_diffstat(v: &Value) -> FileChange {
    let kind = match get_str(v, "status").as_deref() {
        Some("added") => FileChangeKind::Added,
        Some("removed") => FileChangeKind::Deleted,
        Some("renamed") => FileChangeKind::Renamed,
        _ => FileChangeKind::Modified,
    };
    let path = get_obj(v, "new")
        .and_then(|n| get_str(n, "path"))
        .or_else(|| get_obj(v, "old").and_then(|o| get_str(o, "path")))
        .unwrap_or_else(|| "(unknown)".into());
    FileChange {
        path,
        kind,
        additions: get_i64(v, "lines_added").unwrap_or(0),
        deletions: get_i64(v, "lines_removed").unwrap_or(0),
        patch: None,
    }
}

pub fn map_bb_commit(v: &Value) -> Commit {
    let author = get_obj(v, "author");
    Commit {
        // Keep the full hash — the commit-diff API needs a resolvable sha; the UI truncates for display.
        sha: get_str(v, "hash").unwrap_or_default(),
        message: get_str(v, "message").unwrap_or_default().lines().next().unwrap_or_default().to_string(),
        author: author
            .and_then(|a| get_obj(a, "user").and_then(|u| get_str(u, "display_name")).or_else(|| get_str(a, "raw")))
            .unwrap_or_else(|| "unknown".into()),
        date: get_date(v, "date"),
        url: html_url(v),
    }
}

pub fn bb_check_status(state: Option<&str>) -> CheckStatus {
    match state {
        Some("SUCCESSFUL") => CheckStatus::Passed,
        Some("FAILED") | Some("ERROR") => CheckStatus::Failed,
        Some("STOPPED") => CheckStatus::None,
        _ => CheckStatus::Pending,
    }
}

pub fn map_bb_status(v: &Value) -> CheckRun {
    CheckRun {
        name: get_str(v, "name").or_else(|| get_str(v, "key")).unwrap_or_else(|| "status".into()),
        status: bb_check_status(get_str(v, "state").as_deref()),
        url: get_str(v, "url"),
    }
}

/// A Bitbucket PR activity entry (`/pullrequests/{id}/activity`) → a timeline event. The feed
/// mixes approvals, change-requests, merges/declines and comments; comments are dropped (they're
/// in the threads), the rest become events.
fn map_bb_activity(v: &Value) -> Option<TimelineEvent> {
    if let Some(a) = get_obj(v, "approval") {
        return Some(TimelineEvent { actor: get_obj(a, "user").map(map_user), kind: TimelineEventKind::Approved, summary: "approved this".into(), at: get_date(a, "date") });
    }
    if let Some(c) = get_obj(v, "changes_requested") {
        return Some(TimelineEvent { actor: get_obj(c, "user").map(map_user), kind: TimelineEventKind::ChangesRequested, summary: "requested changes".into(), at: get_date(c, "date") });
    }
    if let Some(u) = get_obj(v, "update") {
        let (kind, summary) = match get_str(u, "state").as_deref() {
            Some("MERGED") => (TimelineEventKind::Merged, "merged this"),
            Some("DECLINED") => (TimelineEventKind::Closed, "declined this"),
            _ => return None,
        };
        return Some(TimelineEvent { actor: get_obj(u, "author").map(map_user), kind, summary: summary.into(), at: get_date(u, "date") });
    }
    None
}

fn map_pr_comment(v: &Value) -> Comment {
    Comment {
        id: get_i64(v, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        author: get_obj(v, "user").map(map_user).unwrap_or_else(unknown_user),
        body: get_obj(v, "content").and_then(|c| get_str(c, "raw")).unwrap_or_default(),
        created_at: get_date(v, "created_on"),
    }
}

/// Group flat Bitbucket PR comments into threads keyed by their root comment id: replies carry a
/// `parent.id`, so we walk each comment up to its root and bucket them together. The root id is
/// what a reply posts against. Inline comments keep their file/line from the root's `inline`.
fn group_bb_threads(raw: &[Value]) -> Vec<CommentThread> {
    use std::collections::HashMap;
    let visible: Vec<&Value> = raw.iter().filter(|c| !get_bool(c, "deleted")).collect();
    let parent: HashMap<String, String> = visible
        .iter()
        .filter_map(|c| {
            let id = get_i64(c, "id")?.to_string();
            let pid = get_obj(c, "parent").and_then(|p| get_i64(p, "id"))?.to_string();
            Some((id, pid))
        })
        .collect();
    let root_of = |mut id: String| -> String {
        for _ in 0..100 {
            match parent.get(&id) {
                Some(p) => id = p.clone(),
                None => break,
            }
        }
        id
    };
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&Value>> = HashMap::new();
    for c in &visible {
        let Some(id) = get_i64(c, "id").map(|n| n.to_string()) else { continue };
        let root = root_of(id);
        if !groups.contains_key(&root) {
            order.push(root.clone());
        }
        groups.entry(root).or_default().push(c);
    }
    order
        .into_iter()
        .filter_map(|root| {
            let mut items = groups.remove(&root)?;
            items.sort_by_key(|c| get_date(c, "created_on"));
            let inline = items.first().and_then(|c| get_obj(c, "inline"));
            let file_path = inline.and_then(|i| get_str(i, "path"));
            let line = inline.and_then(|i| get_i64(i, "to").or_else(|| get_i64(i, "from")));
            Some(CommentThread {
                id: root,
                comments: items.iter().map(|c| map_pr_comment(c)).collect(),
                file_path,
                line,
                is_resolved: false,
            })
        })
        .collect()
}

/// Bitbucket state is `{ name, result: { name } }` on the pipeline/step.
pub fn bb_status(state: Option<&Value>) -> PipelineRunStatus {
    let Some(state) = state else { return PipelineRunStatus::Queued };
    match get_str(state, "name").as_deref() {
        Some("COMPLETED") => match get_obj(state, "result").and_then(|r| get_str(r, "name")).as_deref() {
            Some("SUCCESSFUL") => PipelineRunStatus::Succeeded,
            Some("STOPPED") => PipelineRunStatus::Canceled,
            _ => PipelineRunStatus::Failed,
        },
        Some("IN_PROGRESS") => PipelineRunStatus::Running,
        _ => PipelineRunStatus::Queued,
    }
}

/// `repo` is the connection-relative `workspace/slug`, which is also exactly the path segment
/// Bitbucket's web UI wants for a run's deep link.
pub fn map_pipeline(v: &Value, repo: &str) -> PipelineRun {
    let number = get_i64(v, "build_number");
    PipelineRun {
        repository: Some(repo.to_string()),
        id: get_str(v, "uuid").unwrap_or_else(|| number.map(|n| n.to_string()).unwrap_or_else(|| "0".into())),
        definition_id: "pipelines".into(),
        number,
        name: get_obj(v, "target").and_then(|t| get_str(t, "ref_name")),
        title: get_obj(v, "target").and_then(|t| get_obj(t, "commit")).and_then(|c| get_str(c, "message")).map(|m| m.lines().next().unwrap_or("").to_string()),
        status: bb_status(get_obj(v, "state")),
        triggered_by: get_obj(v, "creator").map(map_user),
        branch: get_obj(v, "target").and_then(|t| get_str(t, "ref_name")),
        commit_sha: get_obj(v, "target").and_then(|t| get_obj(t, "commit")).and_then(|c| get_str(c, "hash")),
        started_at: get_date(v, "created_on"),
        finished_at: get_date(v, "completed_on"),
        url: number.map(|n| format!("https://bitbucket.org/{repo}/pipelines/results/{n}")),
        stages: vec![],
    }
}

pub fn map_step(v: &Value) -> PipelineJob {
    PipelineJob {
        id: get_str(v, "uuid").unwrap_or_else(|| "0".into()),
        name: get_str(v, "name").unwrap_or_else(|| "(step)".into()),
        status: bb_status(get_obj(v, "state")),
        started_at: get_date(v, "started_on"),
        finished_at: get_date(v, "completed_on"),
        steps: vec![],
        url: None,
        problem: None,
    }
}

// ---- client ----

pub struct BitbucketClient {
    http: reqwest::Client,
    base: String,
    /// The workspace this connection belongs to. A Bitbucket connection stays **one** workspace:
    /// it is what the connect form collects, what discovery is addressed by, and what the
    /// connection *means*. (`/repositories?role=member` would span workspaces — deliberately not
    /// used.)
    workspace: String,
    /// The repositories this connection fetches from, **connection-relative** (`workspace/slug`).
    scope: Vec<String>,
    self_name: tokio::sync::Mutex<Option<String>>,
}

impl BitbucketClient {
    /// `repo` is connection-relative (`workspace/slug`) — exactly Bitbucket's path shape.
    fn repo_path(&self, repo: &str, suffix: &str) -> String {
        format!("{}/repositories/{repo}{suffix}", self.base)
    }

    fn resolve(&self, item: &ItemRef) -> Result<String> {
        scope::resolve_repo(item, &self.scope)
    }

    /// Every repository in this connection's workspace, most-recently-updated first.
    async fn discover_repositories(&self) -> Result<RepositoryPage> {
        const PER_PAGE: usize = 100;
        const MAX_PAGES: usize = 5;
        let mut repositories = Vec::new();
        let mut truncated = false;
        for page in 1..=MAX_PAGES {
            let url = format!("{}/repositories/{}?pagelen={PER_PAGE}&page={page}&sort=-updated_on", self.base, self.workspace);
            let v = self.get_json(&url).await?;
            let rows = get_arr(&v, "values").to_vec();
            repositories.extend(rows.iter().filter_map(|r| get_str(r, "full_name")));
            if rows.len() < PER_PAGE {
                return Ok(RepositoryPage { repositories, truncated: false });
            }
            truncated = page == MAX_PAGES;
        }
        Ok(RepositoryPage { repositories, truncated })
    }

    async fn get_json(&self, url: &str) -> Result<Value> {
        let resp = self.http.get(url).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("GET {url} -> {}", resp.status())));
        }
        resp.json().await.map_err(prov)
    }

    async fn post_ok(&self, url: &str, body: Value) -> Result<()> {
        let resp = self.http.post(url).json(&body).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("POST {url} -> {}", resp.status())));
        }
        Ok(())
    }

    async fn self_name(&self) -> Result<Option<String>> {
        let mut guard = self.self_name.lock().await;
        if guard.is_none() {
            let v = self.get_json(&format!("{}/user", self.base)).await?;
            *guard = get_str(&v, "display_name").or_else(|| get_str(&v, "nickname"));
        }
        Ok(guard.clone())
    }
}

macro_rules! source {
    ($name:ident) => {
        pub struct $name(pub Arc<BitbucketClient>);
    };
}
source!(BitbucketPr);
source!(BitbucketPipe);

#[async_trait]
impl PullRequestSource for BitbucketPr {
    async fn list(&self, query: &PullRequestQuery) -> Result<Vec<PullRequest>> {
        let scope = &self.0.scope;
        if scope.is_empty() {
            return Ok(Vec::new());
        }
        let state = if query.include_completed { "MERGED" } else { "OPEN" };
        let pagelen = query.limit.unwrap_or(50);
        let rows = fan_out(scope, "bitbucket.pull_requests.list", |repo| async move {
            let url = self.0.repo_path(&repo, &format!("/pullrequests?state={state}&pagelen={pagelen}"));
            let v = self.0.get_json(&url).await?;
            Ok(get_arr(&v, "values").iter().map(|pr| map_pull_request(pr, Some(&repo))).collect())
        })
        .await;
        let me = if query.filter == PullRequestFilter::All { None } else { self.0.self_name().await? };
        let filtered = apply_pull_request_filter(rows, query.filter, me.as_deref());
        Ok(sort_and_cap(filtered, scope.len(), query.limit, |pr| pr.updated_at))
    }
    async fn get(&self, item: &ItemRef) -> Result<PullRequest> {
        let repo = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.repo_path(&repo, &format!("/pullrequests/{}", item.id))).await?;
        Ok(map_pull_request(&v, Some(&repo)))
    }
    async fn threads(&self, item: &ItemRef) -> Result<Vec<CommentThread>> {
        let repo = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.repo_path(&repo, &format!("/pullrequests/{}/comments?pagelen=100", item.id))).await?;
        Ok(group_bb_threads(get_arr(&v, "values")))
    }
    async fn timeline(&self, item: &ItemRef) -> Result<Vec<TimelineEvent>> {
        let repo = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.repo_path(&repo, &format!("/pullrequests/{}/activity?pagelen=50", item.id))).await?;
        let mut out: Vec<TimelineEvent> = get_arr(&v, "values").iter().filter_map(map_bb_activity).collect();
        out.sort_by_key(|e| e.at);
        Ok(out)
    }
    async fn changes(&self, item: &ItemRef) -> Result<Vec<FileChange>> {
        let repo = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.repo_path(&repo, &format!("/pullrequests/{}/diffstat?pagelen=100", item.id))).await?;
        Ok(get_arr(&v, "values").iter().map(map_diffstat).collect())
    }
    async fn commit_changes(&self, item: &ItemRef, sha: &str) -> Result<Vec<FileChange>> {
        let repo = self.0.resolve(item)?;
        // A commit's diffstat (vs its first parent), same shape as the whole-PR file list.
        let v = self.0.get_json(&self.0.repo_path(&repo, &format!("/diffstat/{sha}?pagelen=100"))).await?;
        Ok(get_arr(&v, "values").iter().map(map_diffstat).collect())
    }
    async fn commits(&self, item: &ItemRef) -> Result<Vec<Commit>> {
        let repo = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.repo_path(&repo, &format!("/pullrequests/{}/commits?pagelen=100", item.id))).await?;
        Ok(get_arr(&v, "values").iter().map(map_bb_commit).collect())
    }
    async fn checks(&self, item: &ItemRef) -> Result<Vec<CheckRun>> {
        let repo = self.0.resolve(item)?;
        let pr = self.0.get_json(&self.0.repo_path(&repo, &format!("/pullrequests/{}", item.id))).await?;
        let Some(hash) = get_obj(&pr, "source").and_then(|s| get_obj(s, "commit")).and_then(|c| get_str(c, "hash")) else {
            return Ok(vec![]);
        };
        let v = self.0.get_json(&self.0.repo_path(&repo, &format!("/commit/{hash}/statuses?pagelen=100"))).await?;
        Ok(get_arr(&v, "values").iter().map(map_bb_status).collect())
    }
    async fn add_comment(&self, item: &ItemRef, body: &str) -> Result<()> {
        let repo = self.0.resolve(item)?;
        self.0
            .post_ok(&self.0.repo_path(&repo, &format!("/pullrequests/{}/comments", item.id)), json!({ "content": { "raw": body } }))
            .await
    }
    async fn reply_to_thread(&self, item: &ItemRef, thread_id: &str, body: &str) -> Result<()> {
        let repo = self.0.resolve(item)?;
        // Reply nests under the thread's root comment via `parent.id` (Bitbucket wants an integer).
        let parent_id: i64 = thread_id
            .parse()
            .map_err(|_| Error::Provider(format!("invalid Bitbucket comment id '{thread_id}'")))?;
        self.0
            .post_ok(
                &self.0.repo_path(&repo, &format!("/pullrequests/{}/comments", item.id)),
                json!({ "content": { "raw": body }, "parent": { "id": parent_id } }),
            )
            .await
    }
    async fn vote(&self, item: &ItemRef, vote: ReviewVote) -> Result<()> {
        let repo = self.0.resolve(item)?;
        let id = &item.id;
        match vote {
            ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions => {
                self.0.post_ok(&self.0.repo_path(&repo, &format!("/pullrequests/{id}/approve")), json!({})).await
            }
            ReviewVote::Rejected => {
                self.0.post_ok(&self.0.repo_path(&repo, &format!("/pullrequests/{id}/request-changes")), json!({})).await
            }
            _ => Ok(()),
        }
    }
    async fn merge(&self, item: &ItemRef, options: &MergeOptions) -> Result<()> {
        let repo = self.0.resolve(item)?;
        let strategy = match options.strategy {
            MergeStrategy::Squash => "squash",
            MergeStrategy::Rebase => "fast_forward",
            MergeStrategy::Merge => "merge_commit",
        };
        self.0
            .post_ok(&self.0.repo_path(&repo, &format!("/pullrequests/{}/merge", item.id)), json!({ "merge_strategy": strategy }))
            .await
    }
}

#[async_trait]
impl PipelineSource for BitbucketPipe {
    async fn discover(&self) -> Result<Vec<PipelineDefinition>> {
        // Bitbucket has no named pipeline definitions — model each repository's CI as one.
        Ok(self
            .0
            .scope
            .iter()
            .map(|repo| PipelineDefinition {
                repository: Some(repo.clone()),
                id: "pipelines".into(),
                name: "Bitbucket Pipelines".into(),
                path: None,
                url: None,
            })
            .collect())
    }
    async fn list_runs(&self, query: &PipelineRunQuery) -> Result<Vec<PipelineRun>> {
        let scope: Vec<String> = match &query.repository {
            Some(repo) => vec![repo.clone()],
            None => self.0.scope.clone(),
        };
        if scope.is_empty() {
            return Ok(Vec::new());
        }
        let pagelen = query.limit.unwrap_or(25);
        let rows = fan_out(&scope, "bitbucket.pipelines.list_runs", |repo| async move {
            let url = self.0.repo_path(&repo, &format!("/pipelines?sort=-created_on&pagelen={pagelen}"));
            let v = self.0.get_json(&url).await?;
            Ok(get_arr(&v, "values").iter().map(|p| map_pipeline(p, &repo)).collect())
        })
        .await;
        Ok(sort_and_cap(rows, scope.len(), query.limit, |r| r.started_at))
    }
    async fn get_run(&self, run: &ItemRef) -> Result<PipelineRun> {
        let repo = self.0.resolve(run)?;
        let uuid = enc_uuid(&run.id);
        let run_v = self.0.get_json(&self.0.repo_path(&repo, &format!("/pipelines/{uuid}"))).await?;
        let mut mapped = map_pipeline(&run_v, &repo);
        if let Ok(steps_v) = self.0.get_json(&self.0.repo_path(&repo, &format!("/pipelines/{uuid}/steps"))).await {
            let jobs: Vec<PipelineJob> = get_arr(&steps_v, "values").iter().map(map_step).collect();
            if !jobs.is_empty() {
                mapped.stages = vec![PipelineStage { name: "Steps".into(), status: mapped.status, jobs }];
            }
        }
        Ok(mapped)
    }
    async fn logs(&self, run: &ItemRef, _job_id: Option<&str>) -> Result<String> {
        let repo = self.0.resolve(run)?;
        let steps_v = self.0.get_json(&self.0.repo_path(&repo, &format!("/pipelines/{}/steps", enc_uuid(&run.id)))).await?;
        let lines: Vec<String> = get_arr(&steps_v, "values")
            .iter()
            .map(|s| format!("{}: {}", get_str(s, "name").unwrap_or_default(), get_obj(s, "state").and_then(|st| get_str(st, "name")).unwrap_or_default()))
            .collect();
        Ok(lines.join("\n"))
    }
    async fn trigger(&self, definition: &ItemRef, branch: Option<&str>) -> Result<()> {
        let repo = self.0.resolve(definition)?;
        let body = json!({ "target": { "type": "pipeline_ref_target", "ref_type": "branch", "ref_name": branch.unwrap_or("main") } });
        self.0.post_ok(&self.0.repo_path(&repo, "/pipelines"), body).await
    }
    async fn cancel_run(&self, run: &ItemRef) -> Result<()> {
        let repo = self.0.resolve(run)?;
        self.0.post_ok(&self.0.repo_path(&repo, &format!("/pipelines/{}/stopPipeline", enc_uuid(&run.id))), json!({})).await
    }
}

pub struct BitbucketConnection {
    id: String,
    display_name: String,
    client: Arc<BitbucketClient>,
    caps: Capabilities,
}

#[async_trait]
impl ProviderConnection for BitbucketConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }
    fn provider_type(&self) -> ProviderType {
        ProviderType::Bitbucket
    }
    fn display_name(&self) -> &str {
        &self.display_name
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
        Some(Arc::new(BitbucketPr(self.client.clone())))
    }
    fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
        None
    }
    fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
        Some(Arc::new(BitbucketPipe(self.client.clone())))
    }
    async fn discover_repositories(&self) -> Result<RepositoryPage> {
        self.client.discover_repositories().await
    }
    async fn check(&self) -> bool {
        self.client.get_json(&format!("{}/user", self.client.base)).await.is_ok()
    }
}

pub fn bitbucket_capabilities() -> Capabilities {
    Capabilities {
        supports_pull_requests: true,
        supports_pipelines: true,
        vote_style: VoteStyle::BinaryApprove,
        supports_merge: true,
        supports_inline_comments: true,
        supports_pipeline_trigger: true,
        supports_pipeline_discovery: true,
        ..Default::default()
    }
}

pub struct BitbucketFactory;

impl ProviderFactory for BitbucketFactory {
    fn provider_type(&self) -> ProviderType {
        ProviderType::Bitbucket
    }
    fn describe_capabilities(&self) -> Capabilities {
        bitbucket_capabilities()
    }
    fn create(&self, connection: &Connection, secret: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
        let workspace = connection
            .organization
            .clone()
            .ok_or_else(|| Error::Config("Bitbucket connection requires a Workspace".into()))?;
        // The workspace stays required — discovery is addressed by it, so a connection without
        // one could never populate the scope picker. The repository itself is not.
        let scope = connection.resolve_repo_scope(|| connection.repository.as_ref().map(|r| format!("{workspace}/{r}")));
        let username = connection.username.clone().ok_or_else(|| Error::Config("Bitbucket connection requires a Username".into()))?;

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(app_password) = secret {
            let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{username}:{app_password}"));
            headers.insert(AUTHORIZATION, format!("Basic {encoded}").parse().map_err(prov)?);
        }
        let http = reqwest::Client::builder().default_headers(headers).build().map_err(prov)?;

        let client = Arc::new(BitbucketClient {
            http,
            base: connection.base_url.clone().unwrap_or_else(|| "https://api.bitbucket.org/2.0".into()),
            workspace,
            scope,
            self_name: tokio::sync::Mutex::new(None),
        });
        Ok(Arc::new(BitbucketConnection {
            id: connection.id.clone(),
            display_name: connection.display_name.clone(),
            client,
            caps: bitbucket_capabilities(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_pull_request_with_reviewers() {
        let v: Value = serde_json::from_str(
            r#"{ "id": 7, "title": "Add cache", "state": "OPEN", "draft": false,
                 "author": { "uuid": "{a}", "display_name": "Dana" },
                 "source": { "branch": { "name": "feat" } }, "destination": { "branch": { "name": "main" } },
                 "participants": [ { "role": "REVIEWER", "approved": true, "user": { "display_name": "Rev" } },
                                   { "role": "PARTICIPANT", "approved": false, "user": { "display_name": "X" } } ],
                 "links": { "html": { "href": "https://bitbucket.org/w/r/pull-requests/7" } } }"#,
        )
        .unwrap();
        let pr = map_pull_request(&v, None);
        assert_eq!(pr.number, Some(7));
        assert_eq!(pr.status, PullRequestStatus::Open);
        assert_eq!(pr.source_ref.as_deref(), Some("feat"));
        assert_eq!(pr.reviewers.len(), 1); // only REVIEWER role
        assert_eq!(pr.reviewers[0].vote, ReviewVote::Approved);
        assert_eq!(pr.url.as_deref(), Some("https://bitbucket.org/w/r/pull-requests/7"));
    }

    #[test]
    fn merged_and_declined_status() {
        let merged: Value = serde_json::from_str(r#"{ "id": 1, "state": "MERGED" }"#).unwrap();
        assert_eq!(map_pull_request(&merged, None).status, PullRequestStatus::Merged);
        let declined: Value = serde_json::from_str(r#"{ "id": 2, "state": "DECLINED" }"#).unwrap();
        assert_eq!(map_pull_request(&declined, None).status, PullRequestStatus::Closed);
    }

    #[test]
    fn maps_pipeline_status_from_state_and_result() {
        let ok: Value = serde_json::from_str(
            r#"{ "uuid": "{p1}", "build_number": 42, "state": { "name": "COMPLETED", "result": { "name": "SUCCESSFUL" } },
                 "target": { "ref_name": "main" } }"#,
        )
        .unwrap();
        let run = map_pipeline(&ok, "acme/app");
        assert_eq!(run.status, PipelineRunStatus::Succeeded);
        assert_eq!(run.number, Some(42));
        assert_eq!(run.branch.as_deref(), Some("main"));
        assert_eq!(run.url.as_deref(), Some("https://bitbucket.org/acme/app/pipelines/results/42"));

        let failed: Value = serde_json::from_str(r#"{ "state": { "name": "COMPLETED", "result": { "name": "FAILED" } } }"#).unwrap();
        assert_eq!(map_pipeline(&failed, "a/b").status, PipelineRunStatus::Failed);
        let running: Value = serde_json::from_str(r#"{ "state": { "name": "IN_PROGRESS" } }"#).unwrap();
        assert_eq!(map_pipeline(&running, "a/b").status, PipelineRunStatus::Running);
    }

    #[test]
    fn maps_commit_and_status() {
        let commit: Value = serde_json::from_str(
            r#"{ "hash": "abcdef1234567", "message": "Add cache\nmore", "author": { "user": { "display_name": "Dana" } },
                 "date": "2026-06-01T10:00:00+00:00", "links": { "html": { "href": "u" } } }"#,
        )
        .unwrap();
        let c = map_bb_commit(&commit);
        assert_eq!(c.sha, "abcdef1234567"); // full hash (UI truncates for display)
        assert_eq!(c.message, "Add cache");
        assert_eq!(c.author, "Dana");

        let status: Value = serde_json::from_str(r#"{ "key": "build", "name": "Build", "state": "SUCCESSFUL", "url": "u" }"#).unwrap();
        let s = map_bb_status(&status);
        assert_eq!(s.name, "Build");
        assert_eq!(s.status, CheckStatus::Passed);
    }

    #[test]
    fn maps_diffstat() {
        let v: Value = serde_json::from_str(
            r#"{ "status": "added", "lines_added": 12, "lines_removed": 0, "new": { "path": "src/x.rs" } }"#,
        )
        .unwrap();
        let c = map_diffstat(&v);
        assert_eq!(c.kind, FileChangeKind::Added);
        assert_eq!(c.path, "src/x.rs");
        assert_eq!(c.additions, 12);
    }

    #[test]
    fn groups_comments_into_threads_by_root() {
        let raw: Vec<Value> = serde_json::from_str(
            r#"[
                { "id": 10, "content": { "raw": "root" }, "created_on": "2026-06-01T10:00:00Z", "inline": { "path": "src/x.rs", "to": 12 } },
                { "id": 11, "content": { "raw": "reply" }, "created_on": "2026-06-01T10:05:00Z", "parent": { "id": 10 } },
                { "id": 12, "content": { "raw": "gone" }, "deleted": true },
                { "id": 20, "content": { "raw": "general" }, "created_on": "2026-06-01T11:00:00Z" }
            ]"#,
        )
        .unwrap();
        let threads = group_bb_threads(&raw);
        assert_eq!(threads.len(), 2, "one inline root+reply thread, one general");
        let inline = &threads[0];
        assert_eq!(inline.id, "10");
        assert_eq!(inline.comments.len(), 2, "root + its reply, deleted excluded");
        assert_eq!(inline.file_path.as_deref(), Some("src/x.rs"));
        assert_eq!(inline.line, Some(12));
        assert_eq!(threads[1].id, "20");
        assert!(threads[1].file_path.is_none());
    }

    #[test]
    fn maps_activity_to_timeline() {
        let approval: Value = serde_json::from_str(r#"{ "approval": { "user": { "display_name": "Priya" }, "date": "2026-06-01T10:00:00Z" } }"#).unwrap();
        let e = map_bb_activity(&approval).unwrap();
        assert_eq!(e.kind, TimelineEventKind::Approved);
        assert_eq!(e.actor.unwrap().display_name, "Priya");

        let merged: Value = serde_json::from_str(r#"{ "update": { "state": "MERGED", "author": { "display_name": "Sam" }, "date": "2026-06-01T11:00:00Z" } }"#).unwrap();
        assert_eq!(map_bb_activity(&merged).unwrap().kind, TimelineEventKind::Merged);

        // Comments and plain (OPEN) updates are dropped.
        let comment: Value = serde_json::from_str(r#"{ "comment": { "content": { "raw": "hi" } } }"#).unwrap();
        assert!(map_bb_activity(&comment).is_none());
        let opened: Value = serde_json::from_str(r#"{ "update": { "state": "OPEN" } }"#).unwrap();
        assert!(map_bb_activity(&opened).is_none());
    }
}
