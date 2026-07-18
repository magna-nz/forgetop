//! GitHub provider: pure mappers (fixture-tested) + a reqwest client.

use std::sync::Arc;

use async_trait::async_trait;
use forgetop_core::domain::*;
use forgetop_core::filter::apply_pull_request_filter;
use forgetop_core::provider::*;
use forgetop_core::{Error, Result};
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde_json::{json, Value};

use crate::json::*;

fn prov<E: std::fmt::Display>(e: E) -> Error {
    Error::Provider(e.to_string())
}

// ---- mappers ----

pub fn map_user(v: &Value) -> User {
    User {
        id: get_i64(v, "id").map(|n| n.to_string()).or_else(|| get_str(v, "login")).unwrap_or_else(|| "unknown".into()),
        display_name: get_str(v, "login").unwrap_or_else(|| "unknown".into()),
        handle: get_str(v, "login"),
        avatar_url: get_str(v, "avatar_url"),
    }
}

pub fn map_pull_request(v: &Value) -> PullRequest {
    let merged = v.get("merged_at").map(|x| x.is_string()).unwrap_or(false);
    let state = get_str(v, "state");
    let draft = get_bool(v, "draft");
    let status = if merged {
        PullRequestStatus::Merged
    } else if state.as_deref() == Some("closed") {
        PullRequestStatus::Closed
    } else if draft {
        PullRequestStatus::Draft
    } else {
        PullRequestStatus::Open
    };

    let mergeable = if draft {
        MergeableState::Blocked
    } else {
        match get_str(v, "mergeable_state").as_deref() {
            Some("clean") | Some("unstable") | Some("has_hooks") => MergeableState::Mergeable,
            Some("dirty") => MergeableState::Conflicting,
            Some("blocked") | Some("behind") | Some("draft") => MergeableState::Blocked,
            _ => MergeableState::Unknown,
        }
    };

    let number = get_i64(v, "number");
    PullRequest {
        id: number.map(|n| n.to_string()).or_else(|| get_i64(v, "id").map(|n| n.to_string())).unwrap_or_else(|| "0".into()),
        number,
        title: get_str(v, "title").unwrap_or_else(|| "(untitled)".into()),
        description: get_str(v, "body"),
        author: get_obj(v, "user").map(map_user).unwrap_or_else(unknown_user),
        status,
        is_draft: draft,
        source_ref: get_obj(v, "head").and_then(|h| get_str(h, "ref")),
        target_ref: get_obj(v, "base").and_then(|b| get_str(b, "ref")),
        reviewers: get_arr(v, "requested_reviewers")
            .iter()
            .map(|r| Reviewer { user: map_user(r), vote: ReviewVote::NoVote, is_required: false })
            .collect(),
        labels: get_arr(v, "labels").iter().filter_map(|l| get_str(l, "name")).collect(),
        checks: CheckStatus::None,
        check_summary: None,
        mergeable,
        changed_files: get_i64(v, "changed_files").unwrap_or(0),
        additions: get_i64(v, "additions").unwrap_or(0),
        deletions: get_i64(v, "deletions").unwrap_or(0),
        created_at: get_date(v, "created_at"),
        updated_at: get_date(v, "updated_at"),
        url: get_str(v, "html_url"),
    }
}

/// Aggregates a GitHub check-runs response into a roll-up status + counts.
pub fn map_checks(v: &Value) -> (CheckStatus, CheckSummary) {
    let mut s = CheckSummary::default();
    for run in get_arr(v, "check_runs") {
        if get_str(run, "status").as_deref() != Some("completed") {
            s.in_progress += 1;
            continue;
        }
        match get_str(run, "conclusion").as_deref() {
            Some("success") => s.successful += 1,
            Some("neutral") | Some("skipped") => s.neutral += 1,
            _ => s.failed += 1,
        }
    }
    let status = if s.total() == 0 {
        CheckStatus::None
    } else if s.failed > 0 {
        CheckStatus::Failed
    } else if s.in_progress > 0 {
        CheckStatus::Pending
    } else {
        CheckStatus::Passed
    };
    (status, s)
}

pub fn is_pull_request(issue: &Value) -> bool {
    issue.get("pull_request").is_some()
}

pub fn map_check_run(v: &Value) -> CheckRun {
    let status = if get_str(v, "status").as_deref() != Some("completed") {
        CheckStatus::Pending
    } else {
        match get_str(v, "conclusion").as_deref() {
            Some("success") => CheckStatus::Passed,
            Some("neutral") | Some("skipped") => CheckStatus::None,
            _ => CheckStatus::Failed,
        }
    };
    CheckRun { name: get_str(v, "name").unwrap_or_else(|| "check".into()), status, url: get_str(v, "html_url") }
}

pub fn map_commit(v: &Value) -> Commit {
    let commit = get_obj(v, "commit");
    let author = commit.and_then(|c| get_obj(c, "author"));
    Commit {
        sha: get_str(v, "sha").unwrap_or_default(),
        message: commit.and_then(|c| get_str(c, "message")).unwrap_or_default().lines().next().unwrap_or_default().to_string(),
        author: author
            .and_then(|a| get_str(a, "name"))
            .or_else(|| get_obj(v, "author").and_then(|a| get_str(a, "login")))
            .unwrap_or_else(|| "unknown".into()),
        date: author.and_then(|a| get_date(a, "date")),
        url: get_str(v, "html_url"),
    }
}

pub fn map_issue(v: &Value) -> WorkItem {
    let state = get_str(v, "state").unwrap_or_else(|| "open".into());
    let reason = get_str(v, "state_reason");
    let category = if state == "closed" {
        if reason.as_deref() == Some("not_planned") {
            WorkItemStateCategory::Canceled
        } else {
            WorkItemStateCategory::Completed
        }
    } else {
        WorkItemStateCategory::Unstarted
    };
    let number = get_i64(v, "number");
    WorkItem {
        id: number.map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        identifier: number.map(|n| format!("#{n}")),
        title: get_str(v, "title").unwrap_or_else(|| "(untitled)".into()),
        description: get_str(v, "body"),
        state,
        state_category: category,
        work_item_type: None,
        assignee: get_obj(v, "assignee").map(map_user),
        created_at: get_date(v, "created_at"),
        updated_at: get_date(v, "updated_at"),
        url: get_str(v, "html_url"),
    }
}

pub fn map_workflow(v: &Value) -> PipelineDefinition {
    PipelineDefinition {
        id: get_i64(v, "id").map(|n| n.to_string()).or_else(|| get_str(v, "path")).unwrap_or_else(|| "0".into()),
        name: get_str(v, "name").unwrap_or_else(|| "(workflow)".into()),
        path: get_str(v, "path"),
        url: get_str(v, "html_url"),
    }
}

fn status_of(v: &Value) -> PipelineRunStatus {
    let status = get_str(v, "status");
    let conclusion = get_str(v, "conclusion");
    match status.as_deref() {
        Some("completed") => match conclusion.as_deref() {
            Some("success") => PipelineRunStatus::Succeeded,
            Some("cancelled") | Some("skipped") => PipelineRunStatus::Canceled,
            _ => PipelineRunStatus::Failed,
        },
        Some("queued") | Some("requested") | Some("waiting") | Some("pending") => PipelineRunStatus::Queued,
        _ => PipelineRunStatus::Running,
    }
}

pub fn map_run(v: &Value) -> PipelineRun {
    let completed = get_str(v, "status").as_deref() == Some("completed");
    PipelineRun {
        id: get_i64(v, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        definition_id: get_i64(v, "workflow_id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        number: get_i64(v, "run_number"),
        name: get_str(v, "name").or_else(|| get_str(v, "display_title")),
        status: status_of(v),
        triggered_by: get_obj(v, "actor").map(map_user),
        branch: get_str(v, "head_branch"),
        commit_sha: get_str(v, "head_sha"),
        started_at: get_date(v, "run_started_at").or_else(|| get_date(v, "created_at")),
        finished_at: if completed { get_date(v, "updated_at") } else { None },
        url: get_str(v, "html_url"),
        stages: vec![],
    }
}

pub fn map_job(v: &Value) -> PipelineJob {
    PipelineJob {
        id: get_i64(v, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        name: get_str(v, "name").unwrap_or_else(|| "(job)".into()),
        status: status_of(v),
        started_at: get_date(v, "started_at"),
        finished_at: get_date(v, "completed_at"),
        steps: get_arr(v, "steps")
            .iter()
            .map(|s| PipelineStep {
                name: get_str(s, "name").unwrap_or_else(|| "step".into()),
                status: status_of(s),
                started_at: get_date(s, "started_at"),
                finished_at: get_date(s, "completed_at"),
            })
            .collect(),
        url: get_str(v, "html_url"),
        problem: None,
    }
}

pub fn map_file_change(v: &Value) -> FileChange {
    let kind = match get_str(v, "status").as_deref() {
        Some("added") => FileChangeKind::Added,
        Some("removed") => FileChangeKind::Deleted,
        Some("renamed") => FileChangeKind::Renamed,
        _ => FileChangeKind::Modified,
    };
    FileChange {
        path: get_str(v, "filename").unwrap_or_else(|| "(unknown)".into()),
        kind,
        additions: get_i64(v, "additions").unwrap_or(0),
        deletions: get_i64(v, "deletions").unwrap_or(0),
        patch: get_str(v, "patch"),
    }
}

fn unknown_user() -> User {
    User { id: "unknown".into(), display_name: "unknown".into(), handle: None, avatar_url: None }
}

/// Map a GitHub comment (issue or review) to our [`Comment`] — both share these fields.
fn map_gh_comment(c: &Value) -> Comment {
    Comment {
        id: get_i64(c, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        author: get_obj(c, "user").map(map_user).unwrap_or_else(unknown_user),
        body: get_str(c, "body").unwrap_or_default(),
        created_at: get_date(c, "created_at"),
    }
}

/// Group GitHub review (diff-line) comments (`GET /pulls/{id}/comments`) into real threads, keyed
/// by the root comment id (replies carry `in_reply_to_id`). The root id is what a reply posts to
/// via `/pulls/{id}/comments/{id}/replies`. Each thread keeps the root's file/line.
fn group_gh_review_threads(raw: &[Value]) -> Vec<CommentThread> {
    use std::collections::HashMap;
    let parent: HashMap<String, String> = raw
        .iter()
        .filter_map(|c| Some((get_i64(c, "id")?.to_string(), get_i64(c, "in_reply_to_id")?.to_string())))
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
    for c in raw {
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
            items.sort_by_key(|c| get_date(c, "created_at"));
            let head = items.first().copied();
            Some(CommentThread {
                id: root,
                file_path: head.and_then(|c| get_str(c, "path")),
                line: head.and_then(|c| get_i64(c, "line").or_else(|| get_i64(c, "original_line"))),
                is_resolved: false,
                comments: items.iter().map(|c| map_gh_comment(c)).collect(),
            })
        })
        .collect()
}

// ---- client ----

/// GitHub notification `reason` → our unified kind.
fn github_reason_kind(reason: &str) -> NotificationKind {
    match reason {
        "review_requested" => NotificationKind::ReviewRequested,
        "mention" | "team_mention" => NotificationKind::Mention,
        "assign" => NotificationKind::Assigned,
        "ci_activity" => NotificationKind::CiFailed,
        "comment" => NotificationKind::Comment,
        "state_change" => NotificationKind::StateChange,
        _ => NotificationKind::Other,
    }
}

/// Map a GitHub notification thread (`GET /notifications`) to a [`Notification`].
pub fn map_notification(v: &Value) -> Notification {
    let subject = get_obj(v, "subject");
    let repo = get_obj(v, "repository");
    let subj_type = subject.and_then(|s| get_str(s, "type")).unwrap_or_default();
    let item_type = match subj_type.as_str() {
        "PullRequest" => NotificationItemType::PullRequest,
        "Issue" => NotificationItemType::WorkItem,
        "CheckSuite" => NotificationItemType::Pipeline,
        _ => NotificationItemType::Other,
    };
    // The PR/issue number is the last path segment of the subject's API URL. Only PRs and
    // issues drill in — a check-suite's trailing id isn't something a source can open.
    let item_id = match item_type {
        NotificationItemType::PullRequest | NotificationItemType::WorkItem => subject
            .and_then(|s| get_str(s, "url"))
            .and_then(|u| u.rsplit('/').next().map(str::to_string))
            .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())),
        _ => None,
    };
    let html = repo.and_then(|r| get_str(r, "html_url"));
    let url = match (&html, &item_id, item_type) {
        (Some(h), Some(n), NotificationItemType::PullRequest) => Some(format!("{h}/pull/{n}")),
        (Some(h), Some(n), NotificationItemType::WorkItem) => Some(format!("{h}/issues/{n}")),
        (Some(h), _, _) => Some(h.clone()),
        _ => None,
    };
    Notification {
        id: get_str(v, "id").unwrap_or_default(),
        kind: github_reason_kind(&get_str(v, "reason").unwrap_or_default()),
        item_type,
        item_id,
        title: subject.and_then(|s| get_str(s, "title")).unwrap_or_default(),
        context: repo.and_then(|r| get_str(r, "full_name")).unwrap_or_default(),
        url,
        unread: get_bool(v, "unread"),
        updated_at: get_date(v, "updated_at"),
    }
}

pub struct GitHubClient {
    http: reqwest::Client,
    base: String,
    owner: String,
    repo: String,
    self_login: tokio::sync::Mutex<Option<String>>,
}

impl GitHubClient {
    fn repo_path(&self, suffix: &str) -> String {
        format!("{}/repos/{}/{}{}", self.base, self.owner, self.repo, suffix)
    }

    async fn get_json(&self, url: &str) -> Result<Value> {
        let resp = self.http.get(url).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("GET {url} -> {}", resp.status())));
        }
        resp.json().await.map_err(prov)
    }

    async fn post_json(&self, url: &str, body: Value) -> Result<()> {
        let resp = self.http.post(url).json(&body).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("POST {url} -> {}", resp.status())));
        }
        Ok(())
    }

    async fn patch_empty(&self, url: &str) -> Result<()> {
        let resp = self.http.patch(url).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PATCH {url} -> {}", resp.status())));
        }
        Ok(())
    }

    async fn put_json(&self, url: &str, body: Value) -> Result<()> {
        let resp = self.http.put(url).json(&body).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PUT {url} -> {}", resp.status())));
        }
        Ok(())
    }

    async fn self_login(&self) -> Result<Option<String>> {
        let mut guard = self.self_login.lock().await;
        if guard.is_none() {
            let v = self.get_json(&format!("{}/user", self.base)).await?;
            *guard = get_str(&v, "login");
        }
        Ok(guard.clone())
    }

    async fn enrich(&self, pr: PullRequest) -> PullRequest {
        let Some(number) = pr.number else { return pr };
        match self.get_json(&self.repo_path(&format!("/pulls/{number}"))).await {
            Ok(detail) => {
                let mut enriched = map_pull_request(&detail);
                if let Some(sha) = get_obj(&detail, "head").and_then(|h| get_str(h, "sha")) {
                    if let Ok(checks) = self.get_json(&self.repo_path(&format!("/commits/{sha}/check-runs"))).await {
                        let (status, summary) = map_checks(&checks);
                        enriched.checks = status;
                        enriched.check_summary = Some(summary);
                    }
                }
                enriched
            }
            Err(_) => pr,
        }
    }
}

macro_rules! source {
    ($name:ident) => {
        pub struct $name(pub Arc<GitHubClient>);
    };
}
source!(GitHubPr);
source!(GitHubWi);
source!(GitHubPipe);
source!(GitHubNotif);

#[async_trait]
impl NotificationSource for GitHubNotif {
    async fn list(&self) -> Result<Vec<Notification>> {
        // Repo-scoped, unread only (the inbox surfaces what still needs you).
        let v = self.0.get_json(&self.0.repo_path("/notifications?per_page=50")).await?;
        Ok(v.as_array().unwrap_or(&vec![]).iter().map(map_notification).collect())
    }
    async fn mark_read(&self, id: &str) -> Result<()> {
        // Marking a thread read is the account-level endpoint.
        self.0.patch_empty(&format!("{}/notifications/threads/{id}", self.0.base)).await
    }
    async fn mark_all_read(&self) -> Result<()> {
        self.0.put_json(&self.0.repo_path("/notifications"), json!({ "read": true })).await
    }
}

#[async_trait]
impl PullRequestSource for GitHubPr {
    async fn list(&self, query: &PullRequestQuery) -> Result<Vec<PullRequest>> {
        let state = if query.include_completed { "all" } else { "open" };
        let url = self.0.repo_path(&format!("/pulls?state={state}&per_page={}", query.limit.unwrap_or(50)));
        let v = self.0.get_json(&url).await?;
        let prs: Vec<PullRequest> = v.as_array().unwrap_or(&vec![]).iter().map(map_pull_request).collect();
        let me = if query.filter == PullRequestFilter::All { None } else { self.0.self_login().await? };
        let filtered = apply_pull_request_filter(prs, query.filter, me.as_deref());
        const CAP: usize = 25;
        let mut out = Vec::with_capacity(filtered.len());
        for (i, pr) in filtered.into_iter().enumerate() {
            out.push(if i < CAP { self.0.enrich(pr).await } else { pr });
        }
        Ok(out)
    }
    async fn get(&self, id: &str) -> Result<PullRequest> {
        let v = self.0.get_json(&self.0.repo_path(&format!("/pulls/{id}"))).await?;
        Ok(map_pull_request(&v))
    }
    async fn threads(&self, id: &str) -> Result<Vec<CommentThread>> {
        // The PR conversation is flat issue comments (one bundled thread; GitHub has no reply API
        // for these), plus the real review (diff-line) threads which *do* support replies.
        let issues = self.0.get_json(&self.0.repo_path(&format!("/issues/{id}/comments?per_page=100"))).await?;
        let comments: Vec<Comment> = issues.as_array().unwrap_or(&vec![]).iter().map(map_gh_comment).collect();
        let mut threads = Vec::new();
        if !comments.is_empty() {
            threads.push(CommentThread { id: format!("pr-{id}"), comments, file_path: None, line: None, is_resolved: false });
        }
        let reviews = self.0.get_json(&self.0.repo_path(&format!("/pulls/{id}/comments?per_page=100"))).await?;
        threads.extend(group_gh_review_threads(reviews.as_array().unwrap_or(&vec![])));
        Ok(threads)
    }
    async fn changes(&self, id: &str) -> Result<Vec<FileChange>> {
        let v = self.0.get_json(&self.0.repo_path(&format!("/pulls/{id}/files?per_page=100"))).await?;
        Ok(v.as_array().unwrap_or(&vec![]).iter().map(map_file_change).collect())
    }
    async fn checks(&self, id: &str) -> Result<Vec<CheckRun>> {
        let detail = self.0.get_json(&self.0.repo_path(&format!("/pulls/{id}"))).await?;
        let Some(sha) = get_obj(&detail, "head").and_then(|h| get_str(h, "sha")) else { return Ok(vec![]) };
        let v = self.0.get_json(&self.0.repo_path(&format!("/commits/{sha}/check-runs"))).await?;
        Ok(get_arr(&v, "check_runs").iter().map(map_check_run).collect())
    }
    async fn commits(&self, id: &str) -> Result<Vec<Commit>> {
        let v = self.0.get_json(&self.0.repo_path(&format!("/pulls/{id}/commits?per_page=100"))).await?;
        Ok(v.as_array().unwrap_or(&vec![]).iter().map(map_commit).collect())
    }
    async fn commit_changes(&self, _id: &str, sha: &str) -> Result<Vec<FileChange>> {
        let v = self.0.get_json(&self.0.repo_path(&format!("/commits/{sha}"))).await?;
        Ok(get_arr(&v, "files").iter().map(map_file_change).collect())
    }
    async fn add_comment(&self, id: &str, body: &str) -> Result<()> {
        self.0.post_json(&self.0.repo_path(&format!("/issues/{id}/comments")), json!({ "body": body })).await
    }
    async fn reply_to_thread(&self, id: &str, thread_id: &str, body: &str) -> Result<()> {
        // The bundled conversation thread ("pr-<id>") is flat issue comments with no reply API, so
        // fall back to a top-level comment; a numeric id is a review thread that supports replies.
        if thread_id.starts_with("pr-") {
            return self.add_comment(id, body).await;
        }
        self.0
            .post_json(&self.0.repo_path(&format!("/pulls/{id}/comments/{thread_id}/replies")), json!({ "body": body }))
            .await
    }
    async fn vote(&self, id: &str, vote: ReviewVote) -> Result<()> {
        let event = match vote {
            ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions => "APPROVE",
            ReviewVote::Rejected => "REQUEST_CHANGES",
            _ => "COMMENT",
        };
        self.0.post_json(&self.0.repo_path(&format!("/pulls/{id}/reviews")), json!({ "event": event })).await
    }
    async fn submit_review(&self, id: &str, event: ReviewVote, comments: &[LineComment]) -> Result<()> {
        let ev = match event {
            ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions => "APPROVE",
            ReviewVote::Rejected => "REQUEST_CHANGES",
            _ => "COMMENT",
        };
        let items: Vec<Value> = comments
            .iter()
            .map(|c| {
                json!({
                    "path": c.path,
                    "line": c.line,
                    "side": match c.side { DiffSide::Old => "LEFT", DiffSide::New => "RIGHT" },
                    "body": c.body,
                })
            })
            .collect();
        self.0
            .post_json(&self.0.repo_path(&format!("/pulls/{id}/reviews")), json!({ "event": ev, "comments": items }))
            .await
    }
    async fn merge(&self, id: &str, options: &MergeOptions) -> Result<()> {
        let method = match options.strategy {
            MergeStrategy::Squash => "squash",
            MergeStrategy::Rebase => "rebase",
            MergeStrategy::Merge => "merge",
        };
        let url = self.0.repo_path(&format!("/pulls/{id}/merge"));
        let resp = self.0.http.put(&url).json(&json!({ "merge_method": method })).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PUT {url} -> {}", resp.status())));
        }
        Ok(())
    }
}

#[async_trait]
impl WorkItemSource for GitHubWi {
    async fn list(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>> {
        let state = if query.include_completed { "all" } else { "open" };
        let mut url = self.0.repo_path(&format!("/issues?state={state}&per_page={}", query.limit.unwrap_or(50)));
        if query.mine_only {
            // The repo issues endpoint rejects `@me` (422) — it needs the actual login.
            if let Some(login) = self.0.self_login().await? {
                url.push_str(&format!("&assignee={login}"));
            }
        }
        let v = self.0.get_json(&url).await?;
        Ok(v.as_array().unwrap_or(&vec![]).iter().filter(|e| !is_pull_request(e)).map(map_issue).collect())
    }
    async fn get(&self, id: &str) -> Result<WorkItem> {
        let v = self.0.get_json(&self.0.repo_path(&format!("/issues/{id}"))).await?;
        Ok(map_issue(&v))
    }
    async fn threads(&self, id: &str) -> Result<Vec<CommentThread>> {
        GitHubPr(self.0.clone()).threads(id).await
    }
    async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        let url = self.0.repo_path(&format!("/issues/{id}"));
        let resp = self.0.http.patch(&url).json(&json!({ "state": state })).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PATCH {url} -> {}", resp.status())));
        }
        Ok(())
    }
    async fn available_states(&self, _id: &str) -> Result<Vec<String>> {
        Ok(vec!["open".into(), "closed".into()])
    }
    async fn add_comment(&self, id: &str, body: &str) -> Result<()> {
        self.0.post_json(&self.0.repo_path(&format!("/issues/{id}/comments")), json!({ "body": body })).await
    }
}

#[async_trait]
impl PipelineSource for GitHubPipe {
    async fn discover(&self) -> Result<Vec<PipelineDefinition>> {
        let v = self.0.get_json(&self.0.repo_path("/actions/workflows?per_page=100")).await?;
        Ok(get_arr(&v, "workflows").iter().map(map_workflow).collect())
    }
    async fn list_runs(&self, query: &PipelineRunQuery) -> Result<Vec<PipelineRun>> {
        let mut url = match &query.definition_id {
            Some(def) => self.0.repo_path(&format!("/actions/workflows/{def}/runs")),
            None => self.0.repo_path("/actions/runs"),
        };
        url.push_str(&format!("?per_page={}", query.limit.unwrap_or(25)));
        if let Some(b) = &query.branch {
            url.push_str(&format!("&branch={b}"));
        }
        let v = self.0.get_json(&url).await?;
        Ok(get_arr(&v, "workflow_runs").iter().map(map_run).collect())
    }
    async fn get_run(&self, run_id: &str) -> Result<PipelineRun> {
        let run_v = self.0.get_json(&self.0.repo_path(&format!("/actions/runs/{run_id}"))).await?;
        let mut run = map_run(&run_v);
        if let Ok(jobs_v) = self.0.get_json(&self.0.repo_path(&format!("/actions/runs/{run_id}/jobs"))).await {
            let jobs: Vec<PipelineJob> = get_arr(&jobs_v, "jobs").iter().map(map_job).collect();
            if !jobs.is_empty() {
                run.stages = vec![PipelineStage { name: "jobs".into(), status: run.status, jobs }];
            }
        }
        Ok(run)
    }
    async fn logs(&self, run_id: &str, job_id: Option<&str>) -> Result<String> {
        let jobs_v = self.0.get_json(&self.0.repo_path(&format!("/actions/runs/{run_id}/jobs"))).await?;
        let lines: Vec<String> = get_arr(&jobs_v, "jobs")
            .iter()
            .filter(|j| job_id.is_none() || get_i64(j, "id").map(|n| n.to_string()).as_deref() == job_id)
            .map(|j| format!("{}: {}/{}", get_str(j, "name").unwrap_or_default(), get_str(j, "status").unwrap_or_default(), get_str(j, "conclusion").unwrap_or_else(|| "-".into())))
            .collect();
        Ok(lines.join("\n"))
    }
    async fn trigger(&self, definition_id: &str, branch: Option<&str>) -> Result<()> {
        let url = self.0.repo_path(&format!("/actions/workflows/{definition_id}/dispatches"));
        self.0.post_json(&url, json!({ "ref": branch.unwrap_or("main") })).await
    }
    fn supports_approvals(&self) -> bool {
        true
    }
    async fn pending_approvals(&self, run_id: &str) -> Result<Vec<PipelineApproval>> {
        // Environments awaiting a required-reviewer decision on this run.
        let url = self.0.repo_path(&format!("/actions/runs/{run_id}/pending_deployments"));
        let v = self.0.get_json(&url).await?;
        Ok(v.as_array().map(|a| a.as_slice()).unwrap_or(&[]).iter().filter_map(map_pending_deployment).collect())
    }
    async fn respond_approval(&self, run_id: &str, approval_id: &str, decision: ApprovalDecision, comment: Option<&str>) -> Result<()> {
        let env_id: i64 = approval_id.parse().map_err(|_| Error::Provider("invalid environment id".into()))?;
        let state = match decision {
            ApprovalDecision::Approve => "approved",
            ApprovalDecision::Reject => "rejected",
        };
        let url = self.0.repo_path(&format!("/actions/runs/{run_id}/pending_deployments"));
        self.0.post_json(&url, json!({ "environment_ids": [env_id], "state": state, "comment": comment.unwrap_or("") })).await
    }
}

/// Maps one entry of the `pending_deployments` array into an approval gate.
fn map_pending_deployment(v: &Value) -> Option<PipelineApproval> {
    let env = get_obj(v, "environment")?;
    let id = get_i64(env, "id")?.to_string();
    let name = get_str(env, "name").unwrap_or_else(|| "environment".into());
    Some(PipelineApproval { id, name, can_respond: get_bool(v, "current_user_can_approve") })
}

pub struct GitHubConnection {
    id: String,
    display_name: String,
    client: Arc<GitHubClient>,
    caps: Capabilities,
}

#[async_trait]
impl ProviderConnection for GitHubConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }
    fn provider_type(&self) -> ProviderType {
        ProviderType::GitHub
    }
    fn display_name(&self) -> &str {
        &self.display_name
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
        Some(Arc::new(GitHubPr(self.client.clone())))
    }
    fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
        Some(Arc::new(GitHubWi(self.client.clone())))
    }
    fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
        Some(Arc::new(GitHubPipe(self.client.clone())))
    }
    fn notifications(&self) -> Option<Arc<dyn NotificationSource>> {
        Some(Arc::new(GitHubNotif(self.client.clone())))
    }
    async fn check(&self) -> bool {
        self.client.get_json(&format!("{}/user", self.client.base)).await.is_ok()
    }
}

pub fn github_capabilities() -> Capabilities {
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
        terminology: Terminology { work_items: "Issues".into(), ..Default::default() },
    }
}

pub struct GitHubFactory;

impl ProviderFactory for GitHubFactory {
    fn provider_type(&self) -> ProviderType {
        ProviderType::GitHub
    }
    fn describe_capabilities(&self) -> Capabilities {
        github_capabilities()
    }
    fn create(&self, connection: &Connection, secret: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
        let owner = connection.organization.clone().ok_or_else(|| Error::Config("GitHub connection requires an Organization (owner)".into()))?;
        let repo = connection.repository.clone().ok_or_else(|| Error::Config("GitHub connection requires a Repository".into()))?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(USER_AGENT, "forgetop".parse().unwrap());
        headers.insert(ACCEPT, "application/vnd.github+json".parse().unwrap());
        if let Some(pat) = secret {
            headers.insert(AUTHORIZATION, format!("Bearer {pat}").parse().map_err(prov)?);
        }
        let http = reqwest::Client::builder().default_headers(headers).build().map_err(prov)?;

        let client = Arc::new(GitHubClient {
            http,
            base: connection.base_url.clone().unwrap_or_else(|| "https://api.github.com".into()),
            owner,
            repo,
            self_login: tokio::sync::Mutex::new(None),
        });
        Ok(Arc::new(GitHubConnection {
            id: connection.id.clone(),
            display_name: connection.display_name.clone(),
            client,
            caps: github_capabilities(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_notification_reason_and_item() {
        let v: Value = serde_json::from_str(
            r#"{ "id": "42", "unread": true, "reason": "review_requested", "updated_at": "2026-07-10T09:00:00Z",
                 "subject": { "title": "Add retries", "type": "PullRequest", "url": "https://api.github.com/repos/acme/pay/pulls/1501" },
                 "repository": { "full_name": "acme/pay", "html_url": "https://github.com/acme/pay" } }"#,
        )
        .unwrap();
        let n = map_notification(&v);
        assert_eq!(n.id, "42");
        assert_eq!(n.kind, NotificationKind::ReviewRequested);
        assert_eq!(n.item_type, NotificationItemType::PullRequest);
        assert_eq!(n.item_id.as_deref(), Some("1501"), "PR number extracted for in-app drill-in");
        assert_eq!(n.url.as_deref(), Some("https://github.com/acme/pay/pull/1501"));
        assert_eq!(n.context, "acme/pay");
        assert!(n.unread);

        // A CI notification on a check-suite maps to a pipeline with no in-app id (browser only).
        let ci: Value = serde_json::from_str(
            r#"{ "id": "9", "unread": true, "reason": "ci_activity",
                 "subject": { "title": "CI failed", "type": "CheckSuite", "url": "https://api.github.com/repos/acme/pay/check-suites/77" },
                 "repository": { "full_name": "acme/pay", "html_url": "https://github.com/acme/pay" } }"#,
        )
        .unwrap();
        let c = map_notification(&ci);
        assert_eq!(c.kind, NotificationKind::CiFailed);
        assert_eq!(c.item_type, NotificationItemType::Pipeline);
        assert_eq!(c.item_id, None);
    }

    #[test]
    fn maps_pending_deployment_to_approval() {
        let v: Value = serde_json::from_str(
            r#"{ "environment": { "id": 161088068, "name": "production" },
                 "current_user_can_approve": true }"#,
        )
        .unwrap();
        let a = map_pending_deployment(&v).expect("gate");
        assert_eq!(a.id, "161088068");
        assert_eq!(a.name, "production");
        assert!(a.can_respond);

        // A gate the user can't act on is still surfaced, flagged not-actionable.
        let other: Value = serde_json::from_str(r#"{ "environment": { "id": 5, "name": "staging" } }"#).unwrap();
        let b = map_pending_deployment(&other).expect("gate");
        assert!(!b.can_respond);
    }

    #[test]
    fn maps_open_pr_with_labels_and_stats() {
        let v: Value = serde_json::from_str(
            r#"{ "number": 42, "title": "Add feature", "state": "open", "draft": false,
                 "user": { "id": 7, "login": "octocat" }, "head": { "ref": "feature/x" }, "base": { "ref": "main" },
                 "mergeable_state": "clean", "changed_files": 4, "additions": 30, "deletions": 9,
                 "labels": [ { "name": "banking" } ], "requested_reviewers": [ { "id": 8, "login": "rev" } ] }"#,
        )
        .unwrap();
        let pr = map_pull_request(&v);
        assert_eq!(pr.id, "42");
        assert_eq!(pr.status, PullRequestStatus::Open);
        assert_eq!(pr.mergeable, MergeableState::Mergeable);
        assert_eq!(pr.changed_files, 4);
        assert_eq!(pr.labels, vec!["banking".to_string()]);
        assert_eq!(pr.reviewers.len(), 1);
    }

    #[test]
    fn merged_and_draft_status() {
        let merged: Value = serde_json::from_str(r#"{ "number": 1, "state": "closed", "merged_at": "2026-06-01T10:00:00Z", "user": { "login": "a" } }"#).unwrap();
        assert_eq!(map_pull_request(&merged).status, PullRequestStatus::Merged);
        let draft: Value = serde_json::from_str(r#"{ "number": 2, "state": "open", "draft": true, "user": { "login": "a" } }"#).unwrap();
        assert_eq!(map_pull_request(&draft).mergeable, MergeableState::Blocked);
    }

    #[test]
    fn aggregates_checks() {
        let v: Value = serde_json::from_str(
            r#"{ "check_runs": [ { "status": "completed", "conclusion": "success" }, { "status": "in_progress" }, { "status": "completed", "conclusion": "failure" } ] }"#,
        )
        .unwrap();
        let (status, summary) = map_checks(&v);
        assert_eq!(status, CheckStatus::Failed);
        assert_eq!(summary.successful, 1);
        assert_eq!(summary.in_progress, 1);
        assert_eq!(summary.failed, 1);
    }

    #[test]
    fn maps_run_and_job_steps() {
        let run: Value = serde_json::from_str(
            r#"{ "id": 123, "workflow_id": 99, "run_number": 7, "name": "CI", "status": "completed", "conclusion": "failure", "head_branch": "main" }"#,
        )
        .unwrap();
        let r = map_run(&run);
        assert_eq!(r.status, PipelineRunStatus::Failed);
        assert_eq!(r.definition_id, "99");

        let job: Value = serde_json::from_str(
            r#"{ "id": 1, "name": "build", "status": "completed", "conclusion": "success", "steps": [ { "name": "checkout", "status": "completed", "conclusion": "success" } ] }"#,
        )
        .unwrap();
        assert_eq!(map_job(&job).steps.len(), 1);
    }

    #[test]
    fn maps_commit() {
        let v: Value = serde_json::from_str(
            r#"{ "sha": "abc123", "html_url": "http://c",
                 "commit": { "message": "Add retry\n\nbody", "author": { "name": "Alice", "date": "2026-06-01T10:00:00Z" } },
                 "author": { "login": "alice" } }"#,
        )
        .unwrap();
        let c = map_commit(&v);
        assert_eq!(c.sha, "abc123");
        assert_eq!(c.message, "Add retry"); // first line only
        assert_eq!(c.author, "Alice");
        assert!(c.date.is_some());
    }

    #[test]
    fn maps_check_run_status() {
        let ok: Value = serde_json::from_str(r#"{ "name": "build", "status": "completed", "conclusion": "success" }"#).unwrap();
        assert_eq!(map_check_run(&ok).status, CheckStatus::Passed);
        let running: Value = serde_json::from_str(r#"{ "name": "test", "status": "in_progress" }"#).unwrap();
        assert_eq!(map_check_run(&running).status, CheckStatus::Pending);
    }

    #[test]
    fn maps_file_change_kinds() {
        let v: Value = serde_json::from_str(r#"{ "filename": "a.rs", "status": "added", "additions": 5, "deletions": 0, "patch": "@@" }"#).unwrap();
        let c = map_file_change(&v);
        assert_eq!(c.kind, FileChangeKind::Added);
        assert_eq!(c.patch.as_deref(), Some("@@"));
    }

    #[test]
    fn groups_review_comments_into_threads() {
        let raw: Vec<Value> = serde_json::from_str(
            r#"[
                { "id": 100, "body": "root", "path": "src/x.rs", "line": 12, "user": { "login": "bob" }, "created_at": "2026-06-01T10:00:00Z" },
                { "id": 101, "body": "reply", "in_reply_to_id": 100, "user": { "login": "you" }, "created_at": "2026-06-01T10:05:00Z" },
                { "id": 200, "body": "other file", "path": "src/y.rs", "original_line": 3, "user": { "login": "amy" }, "created_at": "2026-06-01T11:00:00Z" }
            ]"#,
        )
        .unwrap();
        let threads = group_gh_review_threads(&raw);
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, "100");
        assert_eq!(threads[0].comments.len(), 2, "root + reply grouped");
        assert_eq!(threads[0].file_path.as_deref(), Some("src/x.rs"));
        assert_eq!(threads[0].line, Some(12));
        assert_eq!(threads[1].id, "200");
        assert_eq!(threads[1].line, Some(3), "falls back to original_line");
    }
}
