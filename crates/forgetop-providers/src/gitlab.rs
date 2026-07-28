//! GitLab provider: pure mappers (fixture-tested) + a reqwest client.
//!
//! GitLab covers all three sections from one API (v4): Merge Requests (pull
//! requests), Issues (work items), and CI pipelines. The project is addressed by
//! its URL-encoded `group/project` path.

use std::sync::Arc;

use async_trait::async_trait;
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

/// URL-encodes a `group/project` path for the `/projects/:id` API.
fn encode_project(path: &str) -> String {
    path.replace('/', "%2F")
}

// ---- mappers ----

pub fn map_user(v: &Value) -> User {
    User {
        id: get_i64(v, "id").map(|n| n.to_string()).or_else(|| get_str(v, "username")).unwrap_or_else(|| "unknown".into()),
        display_name: get_str(v, "name").or_else(|| get_str(v, "username")).unwrap_or_else(|| "unknown".into()),
        handle: get_str(v, "username"),
        avatar_url: get_str(v, "avatar_url"),
    }
}

fn parse_leading_i64(s: &str) -> i64 {
    s.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
}

pub fn map_merge_request(v: &Value, repo: Option<&str>) -> PullRequest {
    let state = get_str(v, "state");
    let draft = get_bool(v, "draft") || get_bool(v, "work_in_progress");
    let status = match state.as_deref() {
        Some("merged") => PullRequestStatus::Merged,
        Some("closed") | Some("locked") => PullRequestStatus::Closed,
        _ if draft => PullRequestStatus::Draft,
        _ => PullRequestStatus::Open,
    };
    let mergeable = if draft {
        MergeableState::Blocked
    } else {
        match get_str(v, "merge_status").as_deref() {
            Some("can_be_merged") => MergeableState::Mergeable,
            Some("cannot_be_merged") => MergeableState::Conflicting,
            _ => MergeableState::Unknown,
        }
    };
    let number = get_i64(v, "iid");
    PullRequest {
        // `references.full` is `group/project!42` — its path half is already connection-relative.
        repository: get_obj(v, "references")
            .and_then(|r| get_str(r, "full"))
            .and_then(|full| full.split_once('!').map(|(path, _)| path.to_string()))
            .filter(|path| !path.is_empty())
            .or_else(|| repo.map(str::to_string)),
        id: number.map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        number,
        title: get_str(v, "title").unwrap_or_else(|| "(untitled)".into()),
        description: get_str(v, "description"),
        author: get_obj(v, "author").map(map_user).unwrap_or_else(unknown_user),
        status,
        is_draft: draft,
        source_ref: get_str(v, "source_branch"),
        target_ref: get_str(v, "target_branch"),
        reviewers: get_arr(v, "reviewers")
            .iter()
            .map(|r| Reviewer { user: map_user(r), vote: ReviewVote::NoVote, is_required: false })
            .collect(),
        labels: get_arr(v, "labels").iter().filter_map(|l| l.as_str().map(String::from)).collect(),
        checks: CheckStatus::None,
        check_summary: None,
        mergeable,
        changed_files: get_str(v, "changes_count").map(|s| parse_leading_i64(&s)).unwrap_or(0),
        additions: 0,
        deletions: 0,
        created_at: get_date(v, "created_at"),
        updated_at: get_date(v, "updated_at"),
        url: get_str(v, "web_url"),
    }
}

pub fn map_issue(v: &Value, repo: Option<&str>) -> WorkItem {
    let state = get_str(v, "state").unwrap_or_else(|| "opened".into());
    let category = if state == "closed" { WorkItemStateCategory::Completed } else { WorkItemStateCategory::Unstarted };
    let number = get_i64(v, "iid");
    WorkItem {
        repository: get_obj(v, "references")
            .and_then(|r| get_str(r, "full"))
            .and_then(|full| full.split_once('#').map(|(path, _)| path.to_string()))
            .filter(|path| !path.is_empty())
            .or_else(|| repo.map(str::to_string)),
        id: number.map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        identifier: number.map(|n| format!("#{n}")),
        title: get_str(v, "title").unwrap_or_else(|| "(untitled)".into()),
        description: get_str(v, "description"),
        state,
        state_category: category,
        work_item_type: None,
        assignee: get_obj(v, "assignee").map(map_user).or_else(|| get_arr(v, "assignees").first().map(map_user)),
        created_at: get_date(v, "created_at"),
        updated_at: get_date(v, "updated_at"),
        url: get_str(v, "web_url"),
    }
}

pub fn gl_pipeline_status(status: Option<&str>) -> PipelineRunStatus {
    match status {
        Some("success") => PipelineRunStatus::Succeeded,
        Some("failed") => PipelineRunStatus::Failed,
        Some("running") => PipelineRunStatus::Running,
        Some("canceled") | Some("skipped") => PipelineRunStatus::Canceled,
        // A `manual` pipeline/job is blocked awaiting a manual action — surface it as
        // pending (Queued) so it reads as in-flight and its gate can be actioned.
        Some("manual") => PipelineRunStatus::Queued,
        _ => PipelineRunStatus::Queued,
    }
}

pub fn map_pipeline(v: &Value, repo: Option<&str>) -> PipelineRun {
    PipelineRun {
        repository: repo.map(str::to_string),
        id: get_i64(v, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        definition_id: "pipelines".into(),
        number: get_i64(v, "iid").or_else(|| get_i64(v, "id")),
        name: get_str(v, "name").or_else(|| get_str(v, "ref")),
        title: None,
        status: gl_pipeline_status(get_str(v, "status").as_deref()),
        triggered_by: get_obj(v, "user").map(map_user),
        branch: get_str(v, "ref"),
        commit_sha: get_str(v, "sha"),
        started_at: get_date(v, "started_at").or_else(|| get_date(v, "created_at")),
        finished_at: get_date(v, "finished_at"),
        url: get_str(v, "web_url"),
        stages: vec![],
    }
}

pub fn map_gl_job(v: &Value) -> PipelineJob {
    PipelineJob {
        id: get_i64(v, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        name: get_str(v, "name").unwrap_or_else(|| "(job)".into()),
        status: gl_pipeline_status(get_str(v, "status").as_deref()),
        started_at: get_date(v, "started_at"),
        finished_at: get_date(v, "finished_at"),
        steps: vec![],
        url: get_str(v, "web_url"),
        problem: get_str(v, "failure_reason").filter(|s| !s.is_empty()).map(|s| s.replace('_', " ")),
    }
}

/// Groups jobs into stages, preserving first-seen stage order.
pub fn stages_from_jobs(jobs: &[Value]) -> Vec<PipelineStage> {
    let mut stages: Vec<PipelineStage> = Vec::new();
    for j in jobs {
        let stage_name = get_str(j, "stage").unwrap_or_else(|| "jobs".into());
        let job = map_gl_job(j);
        match stages.iter_mut().find(|s| s.name == stage_name) {
            Some(stage) => stage.jobs.push(job),
            None => stages.push(PipelineStage { name: stage_name, status: job.status, jobs: vec![job] }),
        }
    }
    // Roll a stage up to the worst of its jobs.
    for stage in &mut stages {
        stage.status = worst_status(&stage.jobs);
    }
    stages
}

fn worst_status(jobs: &[PipelineJob]) -> PipelineRunStatus {
    use PipelineRunStatus::*;
    if jobs.iter().any(|j| j.status == Failed) {
        Failed
    } else if jobs.iter().any(|j| j.status == Running) {
        Running
    } else if jobs.iter().any(|j| j.status == Queued) {
        Queued
    } else if jobs.iter().all(|j| j.status == Succeeded) && !jobs.is_empty() {
        Succeeded
    } else {
        Canceled
    }
}

fn count_diff(diff: &str) -> (i64, i64) {
    let (mut adds, mut dels) = (0, 0);
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            adds += 1;
        } else if line.starts_with('-') {
            dels += 1;
        }
    }
    (adds, dels)
}

pub fn map_gl_commit(v: &Value) -> Commit {
    Commit {
        sha: get_str(v, "short_id").or_else(|| get_str(v, "id")).unwrap_or_default(),
        message: get_str(v, "title").or_else(|| get_str(v, "message")).unwrap_or_default(),
        author: get_str(v, "author_name").unwrap_or_else(|| "unknown".into()),
        date: get_date(v, "created_at"),
        url: get_str(v, "web_url"),
    }
}

pub fn gl_check_status(status: Option<&str>) -> CheckStatus {
    match status {
        Some("success") => CheckStatus::Passed,
        Some("failed") => CheckStatus::Failed,
        Some("canceled") | Some("skipped") => CheckStatus::None,
        _ => CheckStatus::Pending,
    }
}

pub fn map_gl_status(v: &Value) -> CheckRun {
    CheckRun {
        name: get_str(v, "name").unwrap_or_else(|| "status".into()),
        status: gl_check_status(get_str(v, "status").as_deref()),
        url: get_str(v, "target_url"),
    }
}

pub fn map_change(v: &Value) -> FileChange {
    let kind = if get_bool(v, "new_file") {
        FileChangeKind::Added
    } else if get_bool(v, "deleted_file") {
        FileChangeKind::Deleted
    } else if get_bool(v, "renamed_file") {
        FileChangeKind::Renamed
    } else {
        FileChangeKind::Modified
    };
    let diff = get_str(v, "diff").unwrap_or_default();
    let (additions, deletions) = count_diff(&diff);
    FileChange {
        path: get_str(v, "new_path").or_else(|| get_str(v, "old_path")).unwrap_or_else(|| "(unknown)".into()),
        kind,
        additions,
        deletions,
        patch: if diff.is_empty() { None } else { Some(diff) },
    }
}

/// A GitLab resource_state_event (`/…/resource_state_events`) → a timeline event.
fn map_gl_state_event(v: &Value) -> Option<TimelineEvent> {
    let (kind, summary) = match get_str(v, "state").as_deref() {
        Some("merged") => (TimelineEventKind::Merged, "merged this"),
        Some("closed") => (TimelineEventKind::Closed, "closed this"),
        Some("reopened") => (TimelineEventKind::Reopened, "reopened this"),
        _ => return None,
    };
    Some(TimelineEvent { actor: get_obj(v, "user").map(map_user), kind, summary: summary.into(), at: get_date(v, "created_at") })
}

fn map_note(v: &Value) -> Comment {
    Comment {
        id: get_i64(v, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        author: get_obj(v, "author").map(map_user).unwrap_or_else(unknown_user),
        body: get_str(v, "body").unwrap_or_default(),
        created_at: get_date(v, "created_at"),
    }
}

fn notes_to_thread(prefix: &str, id: &str, notes: &[Value]) -> Vec<CommentThread> {
    let comments: Vec<Comment> = notes.iter().filter(|n| !get_bool(n, "system")).map(map_note).collect();
    if comments.is_empty() {
        vec![]
    } else {
        vec![CommentThread { id: format!("{prefix}-{id}"), comments, file_path: None, line: None, is_resolved: false }]
    }
}

/// Map GitLab discussions (`GET .../discussions`) to real threads, keyed by discussion id so a
/// reply can post back into the same discussion. Diff-anchored discussions keep their file/line.
fn discussions_to_thread(discussions: &[Value]) -> Vec<CommentThread> {
    discussions
        .iter()
        .filter_map(|d| {
            let visible: Vec<&Value> = get_arr(d, "notes").iter().filter(|n| !get_bool(n, "system")).collect();
            let comments: Vec<Comment> = visible.iter().map(|n| map_note(n)).collect();
            if comments.is_empty() {
                return None;
            }
            let pos = visible.first().and_then(|n| get_obj(n, "position"));
            let file_path = pos.and_then(|p| get_str(p, "new_path").or_else(|| get_str(p, "old_path")));
            let line = pos.and_then(|p| get_i64(p, "new_line").or_else(|| get_i64(p, "old_line")));
            Some(CommentThread {
                id: get_str(d, "id").unwrap_or_default(),
                comments,
                file_path,
                line,
                is_resolved: visible.iter().any(|n| get_bool(n, "resolved")),
            })
        })
        .collect()
}

// ---- client ----

/// GitLab todo `action_name` → our unified kind.
fn gitlab_action_kind(action: &str) -> NotificationKind {
    match action {
        "assigned" => NotificationKind::Assigned,
        "review_requested" | "approval_required" => NotificationKind::ReviewRequested,
        "mentioned" | "directly_addressed" => NotificationKind::Mention,
        "build_failed" => NotificationKind::CiFailed,
        _ => NotificationKind::Other,
    }
}

/// Map a GitLab todo (`GET /todos`) to a [`Notification`]. `iid` is safe to drill into
/// because the source scopes todos to this project.
pub fn map_todo(v: &Value) -> Notification {
    let target = get_obj(v, "target");
    let item_type = match get_str(v, "target_type").as_deref() {
        Some("MergeRequest") => NotificationItemType::PullRequest,
        Some("Issue") => NotificationItemType::WorkItem,
        _ => NotificationItemType::Other,
    };
    let item_id = match item_type {
        NotificationItemType::PullRequest | NotificationItemType::WorkItem => {
            target.and_then(|t| get_i64(t, "iid")).map(|i| i.to_string())
        }
        _ => None,
    };
    Notification {
        id: get_i64(v, "id").map(|i| i.to_string()).unwrap_or_default(),
        kind: gitlab_action_kind(&get_str(v, "action_name").unwrap_or_default()),
        item_type,
        item_id,
        title: target.and_then(|t| get_str(t, "title")).unwrap_or_default(),
        context: get_obj(v, "project").and_then(|p| get_str(p, "path_with_namespace")).unwrap_or_default(),
        // `path_with_namespace` is already the connection-relative `group/project` spelling.
        repository: get_obj(v, "project").and_then(|p| get_str(p, "path_with_namespace")),
        url: get_str(v, "target_url").or_else(|| target.and_then(|t| get_str(t, "web_url"))),
        unread: get_str(v, "state").as_deref() == Some("pending"),
        updated_at: get_date(v, "updated_at").or_else(|| get_date(v, "created_at")),
    }
}

pub struct GitLabClient {
    http: reqwest::Client,
    base: String,
    /// The projects this connection fetches from, **connection-relative** (`group/project`,
    /// un-encoded). A GitLab token reaches every project the account is a member of, so this is
    /// a user-chosen scope, not a permission boundary.
    scope: Vec<String>,
    self_username: tokio::sync::Mutex<Option<String>>,
}

impl GitLabClient {
    /// `project` is the connection-relative `group/project` path; GitLab wants it URL-encoded.
    fn project_path(&self, project: &str, suffix: &str) -> String {
        format!("{}/projects/{}{}", self.base, encode_project(project), suffix)
    }

    fn resolve(&self, item: &ItemRef) -> Result<String> {
        scope::resolve_repo(item, &self.scope)
    }

    /// Every project the token is a member of, most-recently-active first.
    async fn discover_repositories(&self) -> Result<RepositoryPage> {
        const PER_PAGE: usize = 100;
        const MAX_PAGES: usize = 5;
        let mut repositories = Vec::new();
        let mut truncated = false;
        for page in 1..=MAX_PAGES {
            let url = format!(
                "{}/projects?membership=true&order_by=last_activity_at&sort=desc&per_page={PER_PAGE}&page={page}",
                self.base
            );
            let v = self.get_json(&url).await?;
            let rows = v.as_array().cloned().unwrap_or_default();
            repositories.extend(rows.iter().filter_map(|r| get_str(r, "path_with_namespace")));
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

    async fn send(&self, req: reqwest::RequestBuilder, what: &str) -> Result<()> {
        let resp = req.send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("{what} -> {}", resp.status())));
        }
        Ok(())
    }

    async fn post_json(&self, url: &str, body: Value) -> Result<()> {
        self.send(self.http.post(url).json(&body), &format!("POST {url}")).await
    }

    async fn self_username(&self) -> Result<Option<String>> {
        let mut guard = self.self_username.lock().await;
        if guard.is_none() {
            let v = self.get_json(&format!("{}/user", self.base)).await?;
            *guard = get_str(&v, "username");
        }
        Ok(guard.clone())
    }

}

macro_rules! source {
    ($name:ident) => {
        pub struct $name(pub Arc<GitLabClient>);
    };
}
source!(GitLabPr);
source!(GitLabWi);
source!(GitLabPipe);
source!(GitLabNotif);

#[async_trait]
impl NotificationSource for GitLabNotif {
    async fn list(&self) -> Result<Vec<Notification>> {
        // Account-level, like the feed itself: each todo carries the project it belongs to, so an
        // iid is still safe to open in-app without narrowing the query to one project.
        let url = format!("{}/todos?state=pending&per_page=50", self.0.base);
        let v = self.0.get_json(&url).await?;
        Ok(v.as_array().unwrap_or(&vec![]).iter().map(map_todo).collect())
    }
    async fn mark_read(&self, id: &str) -> Result<()> {
        self.0.post_json(&format!("{}/todos/{id}/mark_as_done", self.0.base), json!({})).await
    }
    async fn mark_all_read(&self) -> Result<()> {
        self.0.post_json(&format!("{}/todos/mark_as_done", self.0.base), json!({})).await
    }
}

#[async_trait]
impl PullRequestSource for GitLabPr {
    async fn list(&self, query: &PullRequestQuery) -> Result<Vec<PullRequest>> {
        // An empty scope is a real state — the user chose no projects. Fetch nothing.
        let scope = &self.0.scope;
        if scope.is_empty() {
            return Ok(Vec::new());
        }
        let state = if query.include_completed { "all" } else { "opened" };
        let per_page = query.limit.unwrap_or(50);
        let rows = fan_out(scope, "gitlab.pull_requests.list", |project| async move {
            let url = self.0.project_path(&project, &format!("/merge_requests?state={state}&per_page={per_page}"));
            let v = self.0.get_json(&url).await?;
            Ok(v.as_array().unwrap_or(&vec![]).iter().map(|mr| map_merge_request(mr, Some(&project))).collect())
        })
        .await;
        let me = if query.filter == PullRequestFilter::All { None } else { self.0.self_username().await? };
        let filtered = apply_pull_request_filter(rows, query.filter, me.as_deref());
        Ok(sort_and_cap(filtered, scope.len(), query.limit, |pr| pr.updated_at))
    }
    async fn get(&self, item: &ItemRef) -> Result<PullRequest> {
        let project = self.0.resolve(item)?;
        let id = &item.id;
        let v = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{id}"))).await?;
        let mut pr = map_merge_request(&v, Some(&project));
        // The `reviewers` field carries no vote; fold in `approved_by` so approvers show a tick.
        if let Ok(approvals) = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{id}/approvals"))).await {
            for a in get_arr(&approvals, "approved_by") {
                let Some(user) = get_obj(a, "user").map(map_user) else { continue };
                match pr.reviewers.iter_mut().find(|r| r.user.id == user.id) {
                    Some(r) => r.vote = ReviewVote::Approved,
                    None => pr.reviewers.push(Reviewer { user, vote: ReviewVote::Approved, is_required: false }),
                }
            }
        }
        Ok(pr)
    }
    async fn threads(&self, item: &ItemRef) -> Result<Vec<CommentThread>> {
        let project = self.0.resolve(item)?;
        // Discussions (not flat notes) so each thread carries its discussion id for replies.
        let v = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{}/discussions?per_page=100", item.id))).await?;
        Ok(discussions_to_thread(v.as_array().unwrap_or(&vec![])))
    }
    async fn timeline(&self, item: &ItemRef) -> Result<Vec<TimelineEvent>> {
        let project = self.0.resolve(item)?;
        let id = &item.id;
        // State changes (merge/close/reopen) + who approved (approvals API carries no timestamp).
        let states = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{id}/resource_state_events?per_page=100"))).await?;
        let mut out: Vec<TimelineEvent> = states.as_array().unwrap_or(&vec![]).iter().filter_map(map_gl_state_event).collect();
        if let Ok(approvals) = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{id}/approvals"))).await {
            for a in get_arr(&approvals, "approved_by") {
                if let Some(user) = get_obj(a, "user").map(map_user) {
                    out.push(TimelineEvent { actor: Some(user), kind: TimelineEventKind::Approved, summary: "approved this".into(), at: None });
                }
            }
        }
        out.sort_by_key(|e| e.at);
        Ok(out)
    }
    async fn changes(&self, item: &ItemRef) -> Result<Vec<FileChange>> {
        let project = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{}/changes", item.id))).await?;
        Ok(get_arr(&v, "changes").iter().map(map_change).collect())
    }
    async fn commits(&self, item: &ItemRef) -> Result<Vec<Commit>> {
        let project = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{}/commits?per_page=100", item.id))).await?;
        Ok(v.as_array().unwrap_or(&vec![]).iter().map(map_gl_commit).collect())
    }
    async fn commit_changes(&self, item: &ItemRef, sha: &str) -> Result<Vec<FileChange>> {
        let project = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.project_path(&project, &format!("/repository/commits/{sha}/diff?per_page=100"))).await?;
        Ok(v.as_array().unwrap_or(&vec![]).iter().map(map_change).collect())
    }
    async fn checks(&self, item: &ItemRef) -> Result<Vec<CheckRun>> {
        let project = self.0.resolve(item)?;
        let mr = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{}", item.id))).await?;
        let Some(sha) = get_str(&mr, "sha") else { return Ok(vec![]) };
        let v = self.0.get_json(&self.0.project_path(&project, &format!("/repository/commits/{sha}/statuses?per_page=100"))).await?;
        Ok(v.as_array().unwrap_or(&vec![]).iter().map(map_gl_status).collect())
    }
    async fn add_comment(&self, item: &ItemRef, body: &str) -> Result<()> {
        let project = self.0.resolve(item)?;
        self.0.post_json(&self.0.project_path(&project, &format!("/merge_requests/{}/notes", item.id)), json!({ "body": body })).await
    }
    async fn reply_to_thread(&self, item: &ItemRef, thread_id: &str, body: &str) -> Result<()> {
        let project = self.0.resolve(item)?;
        // Post a note into the existing discussion so it threads under the original comment.
        self.0
            .post_json(
                &self.0.project_path(&project, &format!("/merge_requests/{}/discussions/{thread_id}/notes", item.id)),
                json!({ "body": body }),
            )
            .await
    }
    async fn vote(&self, item: &ItemRef, vote: ReviewVote) -> Result<()> {
        let project = self.0.resolve(item)?;
        let id = &item.id;
        match vote {
            ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions => {
                self.0.post_json(&self.0.project_path(&project, &format!("/merge_requests/{id}/approve")), json!({})).await
            }
            ReviewVote::Rejected => {
                self.0.post_json(&self.0.project_path(&project, &format!("/merge_requests/{id}/unapprove")), json!({})).await
            }
            _ => Ok(()),
        }
    }
    async fn merge(&self, item: &ItemRef, options: &MergeOptions) -> Result<()> {
        let project = self.0.resolve(item)?;
        let id = &item.id;
        // GitLab requires the head SHA to confirm exactly what's being merged
        // ("SHA must be provided when merging"), so fetch the MR's current head first.
        let mr = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{id}"))).await?;
        let sha = get_str(&mr, "sha")
            .or_else(|| get_obj(&mr, "diff_refs").and_then(|d| get_str(d, "head_sha")))
            .ok_or_else(|| Error::Provider(format!("merge request '{id}' has no head SHA")))?;
        let body = json!({
            "squash": matches!(options.strategy, MergeStrategy::Squash),
            "should_remove_source_branch": options.delete_source_ref,
            "sha": sha,
        });
        let url = self.0.project_path(&project, &format!("/merge_requests/{id}/merge"));
        self.0.send(self.0.http.put(&url).json(&body), &format!("PUT {url}")).await
    }
    async fn revert(&self, item: &ItemRef) -> Result<()> {
        let project = self.0.resolve(item)?;
        let id = &item.id;
        // Revert the MR's merge commit onto its target branch (GitLab commits the revert directly).
        let mr = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{id}"))).await?;
        let sha = get_str(&mr, "merge_commit_sha")
            .ok_or_else(|| Error::Provider(format!("merge request '{id}' has no merge commit to revert")))?;
        let branch = get_str(&mr, "target_branch")
            .ok_or_else(|| Error::Provider(format!("merge request '{id}' has no target branch")))?;
        let url = self.0.project_path(&project, &format!("/repository/commits/{sha}/revert"));
        self.0.send(self.0.http.post(&url).json(&json!({ "branch": branch })), &format!("POST {url}")).await
    }
    async fn submit_review(&self, item: &ItemRef, event: ReviewVote, comments: &[LineComment]) -> Result<()> {
        let project = self.0.resolve(item)?;
        let id = &item.id;
        // GitLab positions a diff note against the MR's base/head/start commits.
        let mr = self.0.get_json(&self.0.project_path(&project, &format!("/merge_requests/{id}"))).await?;
        let refs = get_obj(&mr, "diff_refs");
        let base = refs.and_then(|r| get_str(r, "base_sha"));
        let head = refs.and_then(|r| get_str(r, "head_sha"));
        let start = refs.and_then(|r| get_str(r, "start_sha"));
        for c in comments {
            let mut pos = json!({
                "position_type": "text",
                "base_sha": base,
                "head_sha": head,
                "start_sha": start,
                "new_path": c.path,
                "old_path": c.path,
            });
            match c.side {
                DiffSide::New => pos["new_line"] = json!(c.line),
                DiffSide::Old => pos["old_line"] = json!(c.line),
            }
            self.0
                .post_json(
                    &self.0.project_path(&project, &format!("/merge_requests/{id}/discussions")),
                    json!({ "body": c.body, "position": pos }),
                )
                .await?;
        }
        match event {
            ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions => {
                self.0.post_json(&self.0.project_path(&project, &format!("/merge_requests/{id}/approve")), json!({})).await?;
            }
            ReviewVote::Rejected => {
                self.0
                    .post_json(&self.0.project_path(&project, &format!("/merge_requests/{id}/notes")), json!({ "body": "Requested changes" }))
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }
}

#[async_trait]
impl WorkItemSource for GitLabWi {
    async fn list(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>> {
        let scope = &self.0.scope;
        if scope.is_empty() {
            return Ok(Vec::new());
        }
        let state = if query.include_completed { "all" } else { "opened" };
        let per_page = query.limit.unwrap_or(50);
        let mine = query.mine_only;
        let rows = fan_out(scope, "gitlab.work_items.list", |project| async move {
            let mut url = self.0.project_path(&project, &format!("/issues?state={state}&per_page={per_page}"));
            if mine {
                url.push_str("&scope=assigned_to_me");
            }
            let v = self.0.get_json(&url).await?;
            Ok(v.as_array().unwrap_or(&vec![]).iter().map(|i| map_issue(i, Some(&project))).collect())
        })
        .await;
        Ok(sort_and_cap(rows, scope.len(), query.limit, |wi| wi.updated_at))
    }
    async fn get(&self, item: &ItemRef) -> Result<WorkItem> {
        let project = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.project_path(&project, &format!("/issues/{}", item.id))).await?;
        Ok(map_issue(&v, Some(&project)))
    }
    async fn threads(&self, item: &ItemRef) -> Result<Vec<CommentThread>> {
        let project = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.project_path(&project, &format!("/issues/{}/notes?per_page=100", item.id))).await?;
        Ok(notes_to_thread("issue", &item.id, v.as_array().unwrap_or(&vec![])))
    }
    async fn timeline(&self, item: &ItemRef) -> Result<Vec<TimelineEvent>> {
        let project = self.0.resolve(item)?;
        let states = self.0.get_json(&self.0.project_path(&project, &format!("/issues/{}/resource_state_events?per_page=100", item.id))).await?;
        let mut out: Vec<TimelineEvent> = states.as_array().unwrap_or(&vec![]).iter().filter_map(map_gl_state_event).collect();
        out.sort_by_key(|e| e.at);
        Ok(out)
    }
    async fn set_state(&self, item: &ItemRef, state: &str) -> Result<()> {
        let project = self.0.resolve(item)?;
        let event = if state.eq_ignore_ascii_case("closed") || state.eq_ignore_ascii_case("close") { "close" } else { "reopen" };
        let url = self.0.project_path(&project, &format!("/issues/{}?state_event={event}", item.id));
        self.0.send(self.0.http.put(&url), &format!("PUT {url}")).await
    }
    async fn available_states(&self, _item: &ItemRef) -> Result<Vec<String>> {
        Ok(vec!["opened".into(), "closed".into()])
    }
    async fn assignable_users(&self, item: &ItemRef) -> Result<Vec<User>> {
        let project = self.0.resolve(item)?;
        let v = self.0.get_json(&self.0.project_path(&project, "/members/all?per_page=100")).await?;
        Ok(v.as_array().unwrap_or(&vec![]).iter().map(map_user).collect())
    }
    async fn set_assignee(&self, item: &ItemRef, assignee_id: Option<&str>) -> Result<()> {
        let project = self.0.resolve(item)?;
        let assignee_ids = match assignee_id {
            Some(assignee_id) => vec![assignee_id.parse::<i64>().map_err(prov)?],
            None => vec![],
        };
        let url = self.0.project_path(&project, &format!("/issues/{}", item.id));
        self.0.send(self.0.http.put(&url).json(&json!({ "assignee_ids": assignee_ids })), &format!("PUT {url}")).await
    }
    async fn update_fields(&self, item: &ItemRef, title: Option<&str>, description: Option<&str>) -> Result<()> {
        let mut body = serde_json::Map::new();
        if let Some(title) = title {
            body.insert("title".into(), json!(title));
        }
        if let Some(description) = description {
            body.insert("description".into(), json!(description));
        }
        if body.is_empty() {
            return Ok(());
        }
        let project = self.0.resolve(item)?;
        let url = self.0.project_path(&project, &format!("/issues/{}", item.id));
        self.0.send(self.0.http.put(&url).json(&Value::Object(body)), &format!("PUT {url}")).await
    }
    async fn add_comment(&self, item: &ItemRef, body: &str) -> Result<()> {
        let project = self.0.resolve(item)?;
        self.0.post_json(&self.0.project_path(&project, &format!("/issues/{}/notes", item.id)), json!({ "body": body })).await
    }
}

#[async_trait]
impl PipelineSource for GitLabPipe {
    async fn discover(&self) -> Result<Vec<PipelineDefinition>> {
        // GitLab has no named pipeline definitions — model each project's CI as one.
        Ok(self
            .0
            .scope
            .iter()
            .map(|project| PipelineDefinition {
                repository: Some(project.clone()),
                id: "pipelines".into(),
                name: "GitLab CI".into(),
                path: None,
                url: None,
            })
            .collect())
    }
    async fn list_runs(&self, query: &PipelineRunQuery) -> Result<Vec<PipelineRun>> {
        let scope: Vec<String> = match &query.repository {
            Some(project) => vec![project.clone()],
            None => self.0.scope.clone(),
        };
        if scope.is_empty() {
            return Ok(Vec::new());
        }
        let per_page = query.limit.unwrap_or(25);
        let rows = fan_out(&scope, "gitlab.pipelines.list_runs", |project| async move {
            let mut url = self.0.project_path(&project, &format!("/pipelines?per_page={per_page}"));
            if let Some(b) = &query.branch {
                url.push_str(&format!("&ref={b}"));
            }
            let v = self.0.get_json(&url).await?;
            Ok(v.as_array().unwrap_or(&vec![]).iter().map(|p| map_pipeline(p, Some(&project))).collect())
        })
        .await;
        Ok(sort_and_cap(rows, scope.len(), query.limit, |r| r.started_at))
    }
    async fn get_run(&self, run: &ItemRef) -> Result<PipelineRun> {
        let project = self.0.resolve(run)?;
        let id = &run.id;
        let run_v = self.0.get_json(&self.0.project_path(&project, &format!("/pipelines/{id}"))).await?;
        let mut mapped = map_pipeline(&run_v, Some(&project));
        if let Ok(jobs_v) = self.0.get_json(&self.0.project_path(&project, &format!("/pipelines/{id}/jobs?per_page=100"))).await {
            mapped.stages = stages_from_jobs(jobs_v.as_array().unwrap_or(&vec![]));
        }
        Ok(mapped)
    }
    async fn logs(&self, run: &ItemRef, _job_id: Option<&str>) -> Result<String> {
        let project = self.0.resolve(run)?;
        let jobs_v = self.0.get_json(&self.0.project_path(&project, &format!("/pipelines/{}/jobs?per_page=100", run.id))).await?;
        let lines: Vec<String> = jobs_v
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|j| format!("{} [{}]: {}", get_str(j, "stage").unwrap_or_default(), get_str(j, "name").unwrap_or_default(), get_str(j, "status").unwrap_or_default()))
            .collect();
        Ok(lines.join("\n"))
    }
    async fn trigger(&self, definition: &ItemRef, branch: Option<&str>) -> Result<()> {
        let project = self.0.resolve(definition)?;
        let url = self.0.project_path(&project, "/pipeline");
        self.0.post_json(&url, json!({ "ref": branch.unwrap_or("main") })).await
    }
    async fn cancel_run(&self, run: &ItemRef) -> Result<()> {
        let project = self.0.resolve(run)?;
        self.0.post_json(&self.0.project_path(&project, &format!("/pipelines/{}/cancel", run.id)), json!({})).await
    }
    fn supports_approvals(&self) -> bool {
        true
    }
    async fn pending_approvals(&self, run: &ItemRef) -> Result<Vec<PipelineApproval>> {
        let project = self.0.resolve(run)?;
        // Unplayed `manual` jobs on the pipeline are the actionable gates.
        let jobs_v = self.0.get_json(&self.0.project_path(&project, &format!("/pipelines/{}/jobs?per_page=100", run.id))).await?;
        Ok(jobs_v
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter(|j| get_str(j, "status").as_deref() == Some("manual"))
            .filter_map(|j| {
                let id = get_i64(j, "id")?.to_string();
                Some(PipelineApproval { id, name: get_str(j, "name").unwrap_or_else(|| "(job)".into()), can_respond: true })
            })
            .collect())
    }
    async fn respond_approval(&self, run: &ItemRef, approval_id: &str, decision: ApprovalDecision, _comment: Option<&str>) -> Result<()> {
        let project = self.0.resolve(run)?;
        // A manual job is approved by playing it, rejected by cancelling it.
        let action = match decision {
            ApprovalDecision::Approve => "play",
            ApprovalDecision::Reject => "cancel",
        };
        self.0.post_json(&self.0.project_path(&project, &format!("/jobs/{approval_id}/{action}")), json!({})).await
    }
}

pub struct GitLabConnection {
    id: String,
    display_name: String,
    client: Arc<GitLabClient>,
    caps: Capabilities,
}

#[async_trait]
impl ProviderConnection for GitLabConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }
    fn provider_type(&self) -> ProviderType {
        ProviderType::GitLab
    }
    fn display_name(&self) -> &str {
        &self.display_name
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
        Some(Arc::new(GitLabPr(self.client.clone())))
    }
    fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
        Some(Arc::new(GitLabWi(self.client.clone())))
    }
    fn notifications(&self) -> Option<Arc<dyn NotificationSource>> {
        Some(Arc::new(GitLabNotif(self.client.clone())))
    }
    fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
        Some(Arc::new(GitLabPipe(self.client.clone())))
    }
    async fn discover_repositories(&self) -> Result<RepositoryPage> {
        self.client.discover_repositories().await
    }
    async fn check(&self) -> bool {
        self.client.get_json(&format!("{}/user", self.client.base)).await.is_ok()
    }
}

pub fn gitlab_capabilities() -> Capabilities {
    Capabilities {
        supports_pull_requests: true,
        supports_work_items: true,
        supports_pipelines: true,
        vote_style: VoteStyle::BinaryApprove,
        supports_merge: true,
        supports_inline_comments: true,
        supports_pipeline_trigger: true,
        supports_pipeline_discovery: true,
        supports_notifications: true,
        terminology: Terminology {
            pull_requests: "Merge Requests".into(),
            work_items: "Issues".into(),
            pipelines: "Pipelines".into(),
        },
    }
}

pub struct GitLabFactory;

impl ProviderFactory for GitLabFactory {
    fn provider_type(&self) -> ProviderType {
        ProviderType::GitLab
    }
    fn describe_capabilities(&self) -> Capabilities {
        gitlab_capabilities()
    }
    fn create(&self, connection: &Connection, secret: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
        // A GitLab token reaches every project the account is a member of, so no project is
        // required up front — the scope picker fills it in after connecting.
        let scope = connection.resolve_repo_scope(|| connection.repository.clone());

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(pat) = secret {
            headers.insert(AUTHORIZATION, format!("Bearer {pat}").parse().map_err(prov)?);
        }
        let http = reqwest::Client::builder().default_headers(headers).build().map_err(prov)?;

        let client = Arc::new(GitLabClient {
            http,
            base: connection.base_url.clone().unwrap_or_else(|| "https://gitlab.com/api/v4".into()),
            scope,
            self_username: tokio::sync::Mutex::new(None),
        });
        Ok(Arc::new(GitLabConnection {
            id: connection.id.clone(),
            display_name: connection.display_name.clone(),
            client,
            caps: gitlab_capabilities(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_todo_action_and_target() {
        let v: Value = serde_json::from_str(
            r#"{ "id": 130, "action_name": "review_requested", "state": "pending", "created_at": "2026-07-10T08:00:00Z",
                 "target_type": "MergeRequest", "target": { "iid": 42, "title": "Rotate keys" },
                 "project": { "path_with_namespace": "platform/infra" },
                 "target_url": "https://gitlab.com/platform/infra/-/merge_requests/42" }"#,
        )
        .unwrap();
        let n = map_todo(&v);
        assert_eq!(n.id, "130");
        assert_eq!(n.kind, NotificationKind::ReviewRequested);
        assert_eq!(n.item_type, NotificationItemType::PullRequest);
        assert_eq!(n.item_id.as_deref(), Some("42"), "MR iid for in-app drill-in");
        assert_eq!(n.context, "platform/infra");
        assert!(n.unread);

        let build: Value = serde_json::from_str(
            r#"{ "id": 5, "action_name": "build_failed", "state": "pending", "target_type": "Issue",
                 "target": { "iid": 7, "title": "Flaky test" }, "project": { "path_with_namespace": "p/x" } }"#,
        )
        .unwrap();
        assert_eq!(map_todo(&build).kind, NotificationKind::CiFailed);
    }

    #[test]
    fn manual_status_surfaces_as_pending_not_canceled() {
        // A manual pipeline/job is a waiting gate — must read as in-flight so the
        // approval detection kicks in, not discarded as Canceled.
        assert_eq!(gl_pipeline_status(Some("manual")), PipelineRunStatus::Queued);
        assert_eq!(gl_pipeline_status(Some("canceled")), PipelineRunStatus::Canceled);
        assert_eq!(gl_pipeline_status(Some("running")), PipelineRunStatus::Running);
    }

    #[test]
    fn maps_merge_request() {
        let v: Value = serde_json::from_str(
            r#"{ "iid": 12, "title": "Add retry", "state": "opened", "draft": false,
                 "author": { "id": 3, "username": "dana", "name": "Dana" },
                 "source_branch": "feat", "target_branch": "main", "merge_status": "can_be_merged",
                 "changes_count": "4", "labels": ["backend"], "reviewers": [ { "username": "rev", "name": "Rev" } ],
                 "web_url": "https://gitlab.com/mr/12" }"#,
        )
        .unwrap();
        let pr = map_merge_request(&v, None);
        assert_eq!(pr.number, Some(12));
        assert_eq!(pr.status, PullRequestStatus::Open);
        assert_eq!(pr.mergeable, MergeableState::Mergeable);
        assert_eq!(pr.changed_files, 4);
        assert_eq!(pr.labels, vec!["backend".to_string()]);
        assert_eq!(pr.reviewers.len(), 1);
        assert_eq!(pr.author.display_name, "Dana");
    }

    #[test]
    fn merged_and_draft_status() {
        let merged: Value = serde_json::from_str(r#"{ "iid": 1, "state": "merged" }"#).unwrap();
        assert_eq!(map_merge_request(&merged, None).status, PullRequestStatus::Merged);
        let draft: Value = serde_json::from_str(r#"{ "iid": 2, "state": "opened", "work_in_progress": true }"#).unwrap();
        let d = map_merge_request(&draft, None);
        assert_eq!(d.status, PullRequestStatus::Draft);
        assert_eq!(d.mergeable, MergeableState::Blocked);
    }

    #[test]
    fn maps_issue_states() {
        let open: Value = serde_json::from_str(r#"{ "iid": 5, "title": "Bug", "state": "opened", "assignees": [ { "username": "x", "name": "X" } ] }"#).unwrap();
        let wi = map_issue(&open, None);
        assert_eq!(wi.identifier.as_deref(), Some("#5"));
        assert_eq!(wi.state_category, WorkItemStateCategory::Unstarted);
        assert_eq!(wi.assignee.unwrap().display_name, "X");
        let closed: Value = serde_json::from_str(r#"{ "iid": 6, "title": "Done", "state": "closed" }"#).unwrap();
        assert_eq!(map_issue(&closed, None).state_category, WorkItemStateCategory::Completed);
    }

    #[test]
    fn maps_pipeline_and_groups_jobs_into_stages() {
        let p: Value = serde_json::from_str(r#"{ "id": 100, "iid": 7, "status": "running", "ref": "main", "sha": "abc" }"#).unwrap();
        let run = map_pipeline(&p, None);
        assert_eq!(run.status, PipelineRunStatus::Running);
        assert_eq!(run.branch.as_deref(), Some("main"));

        let jobs: Vec<Value> = serde_json::from_str(
            r#"[ { "id": 1, "name": "build", "stage": "build", "status": "success" },
                 { "id": 2, "name": "unit", "stage": "test", "status": "failed" },
                 { "id": 3, "name": "lint", "stage": "test", "status": "success" } ]"#,
        )
        .unwrap();
        let stages = stages_from_jobs(&jobs);
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].name, "build");
        assert_eq!(stages[1].name, "test");
        assert_eq!(stages[1].jobs.len(), 2);
        assert_eq!(stages[1].status, PipelineRunStatus::Failed); // worst-of the test jobs
    }

    #[test]
    fn maps_commit_and_status() {
        let commit: Value =
            serde_json::from_str(r#"{ "short_id": "a1b2c3d", "title": "Fix bug", "author_name": "Dana", "created_at": "2026-06-01T10:00:00Z", "web_url": "u" }"#).unwrap();
        let c = map_gl_commit(&commit);
        assert_eq!(c.sha, "a1b2c3d");
        assert_eq!(c.message, "Fix bug");
        assert_eq!(c.author, "Dana");

        let status: Value = serde_json::from_str(r#"{ "name": "pipeline", "status": "failed", "target_url": "t" }"#).unwrap();
        let s = map_gl_status(&status);
        assert_eq!(s.name, "pipeline");
        assert_eq!(s.status, CheckStatus::Failed);
    }

    #[test]
    fn maps_change_with_diff_counts() {
        let v: Value = serde_json::from_str(
            r#"{ "new_path": "src/a.rs", "old_path": "src/a.rs", "new_file": true,
                 "diff": "@@ -0,0 +1,2 @@\n+one\n+two\n-old\n" }"#,
        )
        .unwrap();
        let c = map_change(&v);
        assert_eq!(c.kind, FileChangeKind::Added);
        assert_eq!(c.additions, 2);
        assert_eq!(c.deletions, 1);
    }

    #[test]
    fn maps_discussions_to_threads() {
        let discussions: Vec<Value> = serde_json::from_str(
            r#"[
                { "id": "abc123", "notes": [
                    { "id": 1, "body": "on the diff", "author": { "name": "Bob" }, "created_at": "2026-06-01T10:00:00Z",
                      "resolved": true, "position": { "new_path": "src/x.rs", "new_line": 15 } },
                    { "id": 2, "body": "reply", "author": { "name": "You" }, "created_at": "2026-06-01T10:05:00Z" }
                ] },
                { "id": "sys", "notes": [ { "id": 3, "body": "changed the milestone", "system": true } ] },
                { "id": "def456", "notes": [ { "id": 4, "body": "general note", "author": { "name": "Amy" }, "created_at": "2026-06-01T11:00:00Z" } ] }
            ]"#,
        )
        .unwrap();
        let threads = discussions_to_thread(&discussions);
        assert_eq!(threads.len(), 2, "system-only discussion dropped");
        assert_eq!(threads[0].id, "abc123");
        assert_eq!(threads[0].comments.len(), 2);
        assert_eq!(threads[0].file_path.as_deref(), Some("src/x.rs"));
        assert_eq!(threads[0].line, Some(15));
        assert!(threads[0].is_resolved);
        assert_eq!(threads[1].id, "def456");
        assert!(threads[1].file_path.is_none());
    }

    #[test]
    fn maps_state_events_to_timeline() {
        let merged: Value = serde_json::from_str(r#"{ "state": "merged", "user": { "name": "Sam" }, "created_at": "2026-06-01T10:00:00Z" }"#).unwrap();
        let e = map_gl_state_event(&merged).unwrap();
        assert_eq!(e.kind, TimelineEventKind::Merged);
        assert_eq!(e.actor.unwrap().display_name, "Sam");
        let reopened: Value = serde_json::from_str(r#"{ "state": "reopened", "user": { "name": "Amy" } }"#).unwrap();
        assert_eq!(map_gl_state_event(&reopened).unwrap().kind, TimelineEventKind::Reopened);
        // "opened" isn't surfaced.
        let opened: Value = serde_json::from_str(r#"{ "state": "opened", "user": { "name": "Amy" } }"#).unwrap();
        assert!(map_gl_state_event(&opened).is_none());
    }
}
