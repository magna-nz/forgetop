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
            .map(|s| PipelineStep { name: get_str(s, "name").unwrap_or_else(|| "step".into()), status: status_of(s) })
            .collect(),
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

// ---- client ----

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
        let v = self.0.get_json(&self.0.repo_path(&format!("/issues/{id}/comments?per_page=100"))).await?;
        let comments: Vec<Comment> = v
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|c| Comment {
                id: get_i64(c, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
                author: get_obj(c, "user").map(map_user).unwrap_or_else(unknown_user),
                body: get_str(c, "body").unwrap_or_default(),
                created_at: get_date(c, "created_at"),
            })
            .collect();
        Ok(if comments.is_empty() {
            vec![]
        } else {
            vec![CommentThread { id: format!("pr-{id}"), comments, file_path: None, line: None, is_resolved: false }]
        })
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
    async fn add_comment(&self, id: &str, body: &str) -> Result<()> {
        self.0.post_json(&self.0.repo_path(&format!("/issues/{id}/comments")), json!({ "body": body })).await
    }
    async fn vote(&self, id: &str, vote: ReviewVote) -> Result<()> {
        let event = match vote {
            ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions => "APPROVE",
            ReviewVote::Rejected => "REQUEST_CHANGES",
            _ => "COMMENT",
        };
        self.0.post_json(&self.0.repo_path(&format!("/pulls/{id}/reviews")), json!({ "event": event })).await
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
            url.push_str("&assignee=@me");
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
    fn maps_file_change_kinds() {
        let v: Value = serde_json::from_str(r#"{ "filename": "a.rs", "status": "added", "additions": 5, "deletions": 0, "patch": "@@" }"#).unwrap();
        let c = map_file_change(&v);
        assert_eq!(c.kind, FileChangeKind::Added);
        assert_eq!(c.patch.as_deref(), Some("@@"));
    }
}
