//! Azure DevOps provider: pure mappers (fixture-tested) + a reqwest client.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use forgetop_core::domain::*;
use forgetop_core::filter::apply_pull_request_filter;
use forgetop_core::provider::*;
use forgetop_core::{Error, Result};
use reqwest::header::AUTHORIZATION;
use serde_json::{json, Value};
use similar::{ChangeTag, TextDiff};

use crate::json::*;

const API: &str = "api-version=7.1";

fn prov<E: std::fmt::Display>(e: E) -> Error {
    Error::Provider(e.to_string())
}

fn unknown_user() -> User {
    User { id: "unknown".into(), display_name: "unknown".into(), handle: None, avatar_url: None }
}

fn strip_ref(name: Option<String>) -> Option<String> {
    name.map(|n| n.strip_prefix("refs/heads/").map(str::to_string).unwrap_or(n))
}

// ---- mappers ----

pub fn map_user(v: &Value) -> User {
    User {
        id: get_str(v, "id").or_else(|| get_str(v, "uniqueName")).unwrap_or_else(|| "unknown".into()),
        display_name: get_str(v, "displayName").or_else(|| get_str(v, "uniqueName")).unwrap_or_else(|| "unknown".into()),
        handle: get_str(v, "uniqueName"),
        avatar_url: get_str(v, "imageUrl"),
    }
}

pub fn map_vote(vote: i64) -> ReviewVote {
    match vote {
        10 => ReviewVote::Approved,
        5 => ReviewVote::ApprovedWithSuggestions,
        -5 => ReviewVote::WaitingForAuthor,
        -10 => ReviewVote::Rejected,
        _ => ReviewVote::NoVote,
    }
}

pub fn to_vote(vote: ReviewVote) -> i64 {
    match vote {
        ReviewVote::Approved => 10,
        ReviewVote::ApprovedWithSuggestions => 5,
        ReviewVote::WaitingForAuthor => -5,
        ReviewVote::Rejected => -10,
        ReviewVote::NoVote => 0,
    }
}

pub fn map_pull_request(v: &Value) -> PullRequest {
    let is_draft = get_bool(v, "isDraft");
    let status = match get_str(v, "status").as_deref() {
        Some("completed") => PullRequestStatus::Merged,
        Some("abandoned") => PullRequestStatus::Closed,
        _ if is_draft => PullRequestStatus::Draft,
        _ => PullRequestStatus::Open,
    };
    let mergeable = if is_draft {
        MergeableState::Blocked
    } else {
        match get_str(v, "mergeStatus").as_deref() {
            Some("succeeded") => MergeableState::Mergeable,
            Some("conflicts") | Some("failure") => MergeableState::Conflicting,
            Some("rejectedByPolicy") => MergeableState::Blocked,
            _ => MergeableState::Unknown,
        }
    };
    let id = get_i64(v, "pullRequestId");
    PullRequest {
        id: id.map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        number: id,
        title: get_str(v, "title").unwrap_or_else(|| "(untitled)".into()),
        description: get_str(v, "description"),
        author: get_obj(v, "createdBy").map(map_user).unwrap_or_else(unknown_user),
        status,
        is_draft,
        source_ref: strip_ref(get_str(v, "sourceRefName")),
        target_ref: strip_ref(get_str(v, "targetRefName")),
        reviewers: get_arr(v, "reviewers")
            .iter()
            .map(|r| Reviewer { user: map_user(r), vote: map_vote(get_i64(r, "vote").unwrap_or(0)), is_required: get_bool(r, "isRequired") })
            .collect(),
        labels: get_arr(v, "labels").iter().filter_map(|l| get_str(l, "name")).collect(),
        checks: CheckStatus::None,
        check_summary: None,
        mergeable,
        changed_files: 0,
        additions: 0,
        deletions: 0,
        created_at: get_date(v, "creationDate"),
        updated_at: None,
        url: get_str(v, "url"),
    }
}

pub fn map_state(state: &str) -> WorkItemStateCategory {
    match state.to_ascii_lowercase().as_str() {
        "new" | "proposed" | "to do" => WorkItemStateCategory::Unstarted,
        "active" | "committed" | "in progress" | "doing" | "resolved" => WorkItemStateCategory::Started,
        "closed" | "done" | "completed" => WorkItemStateCategory::Completed,
        "removed" => WorkItemStateCategory::Canceled,
        _ => WorkItemStateCategory::Backlog,
    }
}

fn field_str(fields: &Value, name: &str) -> Option<String> {
    fields.get(name).and_then(|x| x.as_str()).map(|s| s.to_string())
}

pub fn map_work_item(v: &Value) -> WorkItem {
    let fields = get_obj(v, "fields").unwrap_or(v);
    let state = field_str(fields, "System.State").unwrap_or_else(|| "New".into());
    let id = get_i64(v, "id");
    WorkItem {
        id: id.map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        identifier: id.map(|n| n.to_string()),
        title: field_str(fields, "System.Title").unwrap_or_else(|| "(untitled)".into()),
        description: field_str(fields, "System.Description"),
        state_category: map_state(&state),
        state,
        work_item_type: field_str(fields, "System.WorkItemType"),
        assignee: get_obj(fields, "System.AssignedTo").map(map_user),
        created_at: get_date(fields, "System.CreatedDate"),
        updated_at: get_date(fields, "System.ChangedDate"),
        url: get_str(v, "url"),
    }
}

pub fn map_definition(v: &Value) -> PipelineDefinition {
    PipelineDefinition {
        id: get_i64(v, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        name: get_str(v, "name").unwrap_or_else(|| "(pipeline)".into()),
        path: get_str(v, "path"),
        url: get_obj(v, "_links").and_then(|l| get_obj(l, "web")).and_then(|w| get_str(w, "href")),
    }
}

pub fn map_build(v: &Value) -> PipelineRun {
    let status = match get_str(v, "status").as_deref() {
        Some("completed") => match get_str(v, "result").as_deref() {
            Some("succeeded") => PipelineRunStatus::Succeeded,
            Some("partiallySucceeded") => PipelineRunStatus::PartiallySucceeded,
            Some("canceled") => PipelineRunStatus::Canceled,
            _ => PipelineRunStatus::Failed,
        },
        Some("inProgress") | Some("cancelling") => PipelineRunStatus::Running,
        _ => PipelineRunStatus::Queued,
    };
    PipelineRun {
        id: get_i64(v, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        definition_id: get_obj(v, "definition").and_then(|d| get_i64(d, "id")).map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        number: None,
        name: get_str(v, "buildNumber").or_else(|| get_obj(v, "definition").and_then(|d| get_str(d, "name"))),
        status,
        triggered_by: get_obj(v, "requestedFor").map(map_user),
        branch: strip_ref(get_str(v, "sourceBranch")),
        commit_sha: get_str(v, "sourceVersion"),
        started_at: get_date(v, "startTime").or_else(|| get_date(v, "queueTime")),
        finished_at: get_date(v, "finishTime"),
        url: get_obj(v, "_links").and_then(|l| get_obj(l, "web")).and_then(|w| get_str(w, "href")),
        stages: vec![],
    }
}

pub fn map_change_entry(v: &Value) -> FileChange {
    let kind = match get_str(v, "changeType").as_deref() {
        Some("add") => FileChangeKind::Added,
        Some("delete") => FileChangeKind::Deleted,
        Some("rename") | Some("sourceRename") => FileChangeKind::Renamed,
        _ => FileChangeKind::Modified,
    };
    FileChange {
        path: get_obj(v, "item").and_then(|i| get_str(i, "path")).unwrap_or_else(|| "(unknown)".into()),
        kind,
        additions: 0,
        deletions: 0,
        patch: None,
    }
}

pub fn map_az_commit(v: &Value) -> Commit {
    let author = get_obj(v, "author");
    Commit {
        // Keep the full commitId — Azure's commits/{id} API needs it; the UI truncates for display.
        sha: get_str(v, "commitId").unwrap_or_default(),
        message: get_str(v, "comment").unwrap_or_default().lines().next().unwrap_or_default().to_string(),
        author: author.and_then(|a| get_str(a, "name")).unwrap_or_else(|| "unknown".into()),
        date: author.and_then(|a| get_date(a, "date")),
        url: get_str(v, "url"),
    }
}

pub fn az_check_status(state: Option<&str>) -> CheckStatus {
    match state {
        Some("succeeded") => CheckStatus::Passed,
        Some("failed") | Some("error") => CheckStatus::Failed,
        Some("pending") => CheckStatus::Pending,
        _ => CheckStatus::None,
    }
}

pub fn map_az_status(v: &Value) -> CheckRun {
    CheckRun {
        name: get_obj(v, "context")
            .and_then(|c| get_str(c, "name"))
            .or_else(|| get_str(v, "description"))
            .unwrap_or_else(|| "status".into()),
        status: az_check_status(get_str(v, "state").as_deref()),
        url: get_str(v, "targetUrl"),
    }
}

fn record_status(v: &Value) -> PipelineRunStatus {
    if get_str(v, "state").as_deref() == Some("completed") {
        match get_str(v, "result").as_deref() {
            Some("succeeded") => PipelineRunStatus::Succeeded,
            Some("canceled") => PipelineRunStatus::Canceled,
            _ => PipelineRunStatus::Failed,
        }
    } else if get_str(v, "state").as_deref() == Some("inProgress") {
        PipelineRunStatus::Running
    } else {
        PipelineRunStatus::Queued
    }
}

/// A short error/warning summary from a timeline record, if any.
fn az_problem(v: &Value) -> Option<String> {
    let plural = |n: i64, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    let e = get_i64(v, "errorCount").unwrap_or(0);
    let w = get_i64(v, "warningCount").unwrap_or(0);
    match (e, w) {
        (0, 0) => None,
        (e, 0) => Some(plural(e, "error")),
        (0, w) => Some(plural(w, "warning")),
        (e, w) => Some(format!("{}, {}", plural(e, "error"), plural(w, "warning"))),
    }
}

/// Build a `+`/`-`/space line diff (ADO doesn't return patch text).
pub fn unified_diff(old: &str, new: &str) -> (String, i64, i64) {
    let diff = TextDiff::from_lines(old, new);
    let mut patch = String::new();
    let (mut adds, mut dels) = (0i64, 0i64);
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => {
                dels += 1;
                '-'
            }
            ChangeTag::Insert => {
                adds += 1;
                '+'
            }
            ChangeTag::Equal => ' ',
        };
        patch.push(sign);
        patch.push_str(change.value());
    }
    (patch.trim_end().to_string(), adds, dels)
}

// ---- client ----

pub struct AzureClient {
    http: reqwest::Client,
    base: String,
    project: String,
    repository: String,
    self_id: tokio::sync::Mutex<Option<String>>,
}

impl AzureClient {
    async fn get_json(&self, url: &str) -> Result<Value> {
        let resp = self.http.get(url).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("GET {url} -> {}", resp.status())));
        }
        resp.json().await.map_err(prov)
    }

    async fn post_json_read(&self, url: &str, body: Value) -> Result<Value> {
        let resp = self.http.post(url).json(&body).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("POST {url} -> {}", resp.status())));
        }
        resp.json().await.map_err(prov)
    }

    fn pr_base(&self, id: &str) -> String {
        format!("{}/{}/_apis/git/repositories/{}/pullRequests/{}", self.base, self.project, self.repository, id)
    }

    async fn self_id(&self) -> Result<Option<String>> {
        let mut guard = self.self_id.lock().await;
        if guard.is_none() {
            let v = self.get_json(&format!("{}/_apis/connectionData?api-version=7.1-preview", self.base)).await?;
            *guard = get_obj(&v, "authenticatedUser").and_then(|u| get_str(u, "id"));
        }
        Ok(guard.clone())
    }

    async fn item_content(&self, path: &str, commit: &str) -> Option<String> {
        let url = format!(
            "{}/{}/_apis/git/repositories/{}/items?path={}&versionDescriptor.versionType=commit&versionDescriptor.version={}&includeContent=true&{API}",
            self.base, self.project, self.repository, urlencoding(path), commit
        );
        let v = self.get_json(&url).await.ok()?;
        get_str(&v, "content")
    }

    async fn read_stages(&self, run_id: &str) -> Vec<PipelineStage> {
        let url = format!("{}/{}/_apis/build/builds/{run_id}/timeline?{API}", self.base, self.project);
        let Ok(v) = self.get_json(&url).await else { return vec![] };
        let records: Vec<&Value> = get_arr(&v, "records").iter().collect();
        records
            .iter()
            .filter(|r| get_str(r, "type").as_deref() == Some("Stage"))
            .map(|stage| {
                let stage_id = get_str(stage, "id");
                let jobs = records
                    .iter()
                    .filter(|r| get_str(r, "type").as_deref() == Some("Job") && get_str(r, "parentId") == stage_id)
                    .map(|job| {
                        let job_id = get_str(job, "id");
                        let steps = records
                            .iter()
                            .filter(|r| get_str(r, "type").as_deref() == Some("Task") && get_str(r, "parentId") == job_id)
                            .map(|t| PipelineStep {
                                name: get_str(t, "name").unwrap_or_else(|| "step".into()),
                                status: record_status(t),
                                started_at: get_date(t, "startTime"),
                                finished_at: get_date(t, "finishTime"),
                            })
                            .collect();
                        PipelineJob {
                            id: job_id.unwrap_or_else(|| "0".into()),
                            name: get_str(job, "name").unwrap_or_else(|| "job".into()),
                            status: record_status(job),
                            started_at: get_date(job, "startTime"),
                            finished_at: get_date(job, "finishTime"),
                            steps,
                            url: None,
                            problem: az_problem(job),
                        }
                    })
                    .collect();
                PipelineStage { name: get_str(stage, "name").unwrap_or_else(|| "stage".into()), status: record_status(stage), jobs }
            })
            .collect()
    }

    /// Pending approval gates on a build run, read from its timeline.
    async fn approval_gates(&self, run_id: &str) -> Result<Vec<PipelineApproval>> {
        let url = format!("{}/{}/_apis/build/builds/{run_id}/timeline?{API}", self.base, self.project);
        let v = self.get_json(&url).await?;
        Ok(approval_gates_from_timeline(get_arr(&v, "records")))
    }
}

/// Extracts pending approval checkpoints from a build timeline. A record of type
/// `Checkpoint.Approval` carries the approval id as its `id`; its enclosing Stage
/// gives the gate a human label.
fn approval_gates_from_timeline(records: &[Value]) -> Vec<PipelineApproval> {
    use std::collections::HashMap;
    let by_id: HashMap<String, &Value> = records.iter().filter_map(|r| get_str(r, "id").map(|id| (id, r))).collect();
    records
        .iter()
        .filter(|r| get_str(r, "type").as_deref() == Some("Checkpoint.Approval"))
        .filter(|r| get_str(r, "state").as_deref() != Some("completed"))
        .filter_map(|r| {
            let id = get_str(r, "id")?;
            let name = enclosing_stage_name(&by_id, r).unwrap_or_else(|| "approval".into());
            Some(PipelineApproval { id, name, can_respond: true })
        })
        .collect()
}

/// Walks `parentId` up from a record to the enclosing Stage record's name.
fn enclosing_stage_name(by_id: &std::collections::HashMap<String, &Value>, rec: &Value) -> Option<String> {
    let mut cur = rec;
    for _ in 0..6 {
        if get_str(cur, "type").as_deref() == Some("Stage") {
            return get_str(cur, "name");
        }
        cur = by_id.get(&get_str(cur, "parentId")?)?;
    }
    None
}

fn urlencoding(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_') { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}

pub struct AzurePr(pub Arc<AzureClient>);
pub struct AzureWi(pub Arc<AzureClient>);
pub struct AzurePipe(pub Arc<AzureClient>);

#[async_trait]
impl PullRequestSource for AzurePr {
    async fn list(&self, query: &PullRequestQuery) -> Result<Vec<PullRequest>> {
        let status = if query.include_completed { "all" } else { "active" };
        let url = format!(
            "{}/{}/_apis/git/repositories/{}/pullrequests?searchCriteria.status={status}&$top={}&{API}",
            self.0.base, self.0.project, self.0.repository, query.limit.unwrap_or(50)
        );
        let v = self.0.get_json(&url).await?;
        let prs: Vec<PullRequest> = get_arr(&v, "value").iter().map(map_pull_request).collect();
        let me = if query.filter == PullRequestFilter::All { None } else { self.0.self_id().await? };
        Ok(apply_pull_request_filter(prs, query.filter, me.as_deref()))
    }
    async fn get(&self, id: &str) -> Result<PullRequest> {
        Ok(map_pull_request(&self.0.get_json(&format!("{}?{API}", self.0.pr_base(id))).await?))
    }
    async fn timeline(&self, id: &str) -> Result<Vec<TimelineEvent>> {
        // Azure has no single timeline endpoint; derive events from the reviewers' votes and the
        // PR's completion status (vote timestamps aren't exposed, so those events carry no time).
        let pr = self.0.get_json(&format!("{}?{API}", self.0.pr_base(id))).await?;
        let mut out = Vec::new();
        for r in get_arr(&pr, "reviewers") {
            let (kind, summary) = match get_i64(r, "vote").unwrap_or(0) {
                10 | 5 => (TimelineEventKind::Approved, "approved this"),
                -10 => (TimelineEventKind::ChangesRequested, "requested changes"),
                -5 => (TimelineEventKind::Reviewed, "is waiting for the author"),
                _ => continue,
            };
            out.push(TimelineEvent { actor: Some(map_user(r)), kind, summary: summary.into(), at: None });
        }
        match get_str(&pr, "status").as_deref() {
            Some("completed") => out.push(TimelineEvent {
                actor: get_obj(&pr, "closedBy").map(map_user),
                kind: TimelineEventKind::Merged,
                summary: "completed this pull request".into(),
                at: get_date(&pr, "closedDate"),
            }),
            Some("abandoned") => out.push(TimelineEvent {
                actor: get_obj(&pr, "closedBy").map(map_user),
                kind: TimelineEventKind::Closed,
                summary: "abandoned this pull request".into(),
                at: get_date(&pr, "closedDate"),
            }),
            _ => {}
        }
        out.sort_by_key(|e| e.at);
        Ok(out)
    }
    async fn threads(&self, id: &str) -> Result<Vec<CommentThread>> {
        let v = self.0.get_json(&format!("{}/threads?{API}", self.0.pr_base(id))).await?;
        Ok(get_arr(&v, "value")
            .iter()
            .map(|t| CommentThread {
                id: get_i64(t, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
                file_path: get_obj(t, "threadContext").and_then(|c| get_str(c, "filePath")),
                line: get_obj(t, "threadContext").and_then(|c| get_obj(c, "rightFileStart")).and_then(|s| get_i64(s, "line")),
                is_resolved: matches!(get_str(t, "status").as_deref(), Some("closed") | Some("fixed")),
                comments: get_arr(t, "comments")
                    .iter()
                    .map(|c| Comment {
                        id: get_i64(c, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
                        author: get_obj(c, "author").map(map_user).unwrap_or_else(unknown_user),
                        body: get_str(c, "content").unwrap_or_default(),
                        created_at: get_date(c, "publishedDate"),
                    })
                    .collect(),
            })
            .collect())
    }
    async fn changes(&self, id: &str) -> Result<Vec<FileChange>> {
        let iters = self.0.get_json(&format!("{}/iterations?{API}", self.0.pr_base(id))).await?;
        let iterations = get_arr(&iters, "value");
        let Some(last) = iterations.last() else { return Ok(vec![]) };
        let new_commit = get_obj(last, "sourceRefCommit").and_then(|c| get_str(c, "commitId"));
        let base_commit = get_obj(last, "commonRefCommit")
            .and_then(|c| get_str(c, "commitId"))
            .or_else(|| get_obj(last, "targetRefCommit").and_then(|c| get_str(c, "commitId")));
        let iter_id = get_i64(last, "id").unwrap_or(1);

        let changes = self.0.get_json(&format!("{}/iterations/{iter_id}/changes?{API}", self.0.pr_base(id))).await?;
        let mut out = Vec::new();
        for raw in get_arr(&changes, "changeEntries") {
            if get_obj(raw, "item").map(|i| get_bool(i, "isFolder")).unwrap_or(false) {
                continue;
            }
            let mut change = map_change_entry(raw);
            let old = if matches!(change.kind, FileChangeKind::Added) {
                Some(String::new())
            } else if let Some(c) = &base_commit {
                self.0.item_content(&change.path, c).await
            } else {
                None
            };
            let new = if matches!(change.kind, FileChangeKind::Deleted) {
                Some(String::new())
            } else if let Some(c) = &new_commit {
                self.0.item_content(&change.path, c).await
            } else {
                None
            };
            if let (Some(o), Some(n)) = (old, new) {
                let (patch, adds, dels) = unified_diff(&o, &n);
                change.patch = Some(patch);
                change.additions = adds;
                change.deletions = dels;
            }
            out.push(change);
        }
        Ok(out)
    }
    async fn commits(&self, id: &str) -> Result<Vec<Commit>> {
        let v = self.0.get_json(&format!("{}/commits?{API}", self.0.pr_base(id))).await?;
        Ok(get_arr(&v, "value").iter().map(map_az_commit).collect())
    }
    async fn commit_changes(&self, _id: &str, sha: &str) -> Result<Vec<FileChange>> {
        // Diff the commit against its first parent, computing each file's patch from item content
        // (same approach as the whole-PR `changes`). A root commit diffs against an empty tree.
        let repo = format!("{}/{}/_apis/git/repositories/{}", self.0.base, self.0.project, self.0.repository);
        let commit = self.0.get_json(&format!("{repo}/commits/{sha}?{API}")).await?;
        let parent = get_arr(&commit, "parents").first().and_then(|p| p.as_str().map(String::from));
        let changes = self.0.get_json(&format!("{repo}/commits/{sha}/changes?{API}")).await?;
        let mut out = Vec::new();
        for raw in get_arr(&changes, "changes") {
            if get_obj(raw, "item").map(|i| get_bool(i, "isFolder")).unwrap_or(false) {
                continue;
            }
            let mut change = map_change_entry(raw);
            let old = if matches!(change.kind, FileChangeKind::Added) {
                Some(String::new())
            } else if let Some(p) = &parent {
                self.0.item_content(&change.path, p).await
            } else {
                Some(String::new())
            };
            let new = if matches!(change.kind, FileChangeKind::Deleted) {
                Some(String::new())
            } else {
                self.0.item_content(&change.path, sha).await
            };
            if let (Some(o), Some(n)) = (old, new) {
                let (patch, adds, dels) = unified_diff(&o, &n);
                change.patch = Some(patch);
                change.additions = adds;
                change.deletions = dels;
            }
            out.push(change);
        }
        Ok(out)
    }
    async fn checks(&self, id: &str) -> Result<Vec<CheckRun>> {
        let v = self.0.get_json(&format!("{}/statuses?{API}", self.0.pr_base(id))).await?;
        Ok(get_arr(&v, "value").iter().map(map_az_status).collect())
    }
    async fn add_comment(&self, id: &str, body: &str) -> Result<()> {
        self.0
            .post_json_read(&format!("{}/threads?{API}", self.0.pr_base(id)), json!({ "comments": [ { "content": body, "commentType": 1 } ], "status": 1 }))
            .await
            .map(|_| ())
    }
    async fn reply_to_thread(&self, id: &str, thread_id: &str, body: &str) -> Result<()> {
        // Append a reply comment to an existing thread; parentCommentId 1 is the thread's root.
        self.0
            .post_json_read(
                &format!("{}/threads/{thread_id}/comments?{API}", self.0.pr_base(id)),
                json!({ "content": body, "parentCommentId": 1, "commentType": 1 }),
            )
            .await
            .map(|_| ())
    }
    async fn vote(&self, id: &str, vote: ReviewVote) -> Result<()> {
        let self_id = self.0.self_id().await?.ok_or_else(|| Error::Provider("could not resolve authenticated user".into()))?;
        let url = format!("{}/reviewers/{self_id}?{API}", self.0.pr_base(id));
        let resp = self.0.http.put(&url).json(&json!({ "vote": to_vote(vote) })).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PUT {url} -> {}", resp.status())));
        }
        Ok(())
    }
    async fn merge(&self, id: &str, options: &MergeOptions) -> Result<()> {
        let pr = self.0.get_json(&format!("{}?{API}", self.0.pr_base(id))).await?;
        let source = get_obj(&pr, "lastMergeSourceCommit").and_then(|c| get_str(c, "commitId"));
        let strategy = match options.strategy {
            MergeStrategy::Squash => "squash",
            MergeStrategy::Rebase => "rebase",
            MergeStrategy::Merge => "noFastForward",
        };
        let url = format!("{}?{API}", self.0.pr_base(id));
        let body = json!({ "status": "completed", "lastMergeSourceCommit": { "commitId": source }, "completionOptions": { "mergeStrategy": strategy, "deleteSourceBranch": options.delete_source_ref } });
        let resp = self.0.http.patch(&url).json(&body).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PATCH {url} -> {}", resp.status())));
        }
        Ok(())
    }
    async fn revert(&self, id: &str) -> Result<()> {
        // Azure creates the revert on a new branch off the PR's target; the user opens a PR from it.
        let pr = self.0.get_json(&format!("{}?{API}", self.0.pr_base(id))).await?;
        let onto = get_str(&pr, "targetRefName")
            .ok_or_else(|| Error::Provider(format!("pull request '{id}' has no target branch")))?;
        let pr_id: i64 = id.parse().map_err(|_| Error::Provider(format!("pull request id '{id}' is not numeric")))?;
        let url = format!("{}/{}/_apis/git/repositories/{}/reverts?{API}", self.0.base, self.0.project, self.0.repository);
        let body = json!({
            "generatedRefName": format!("refs/heads/revert-pr-{id}"),
            "ontoRefName": onto,
            "source": { "pullRequestId": pr_id },
        });
        let resp = self.0.http.post(&url).json(&body).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("POST {url} -> {}", resp.status())));
        }
        Ok(())
    }
}

#[async_trait]
impl WorkItemSource for AzureWi {
    async fn list(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>> {
        let mut conditions = vec!["[System.TeamProject] = @project".to_string()];
        if query.mine_only {
            conditions.push("[System.AssignedTo] = @me".into());
        }
        if !query.include_completed {
            conditions.push("[System.State] NOT IN ('Closed', 'Done', 'Removed')".into());
        }
        let wiql = format!("SELECT [System.Id] FROM WorkItems WHERE {} ORDER BY [System.ChangedDate] DESC", conditions.join(" AND "));
        let url = format!("{}/{}/_apis/wit/wiql?$top={}&{API}", self.0.base, self.0.project, query.limit.unwrap_or(50));
        let ids_v = self.0.post_json_read(&url, json!({ "query": wiql })).await?;
        let ids: Vec<String> = get_arr(&ids_v, "workItems").iter().filter_map(|w| get_i64(w, "id")).map(|n| n.to_string()).collect();
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let v = self.0.get_json(&format!("{}/_apis/wit/workitems?ids={}&{API}", self.0.base, ids.join(","))).await?;
        Ok(get_arr(&v, "value").iter().map(map_work_item).collect())
    }
    async fn get(&self, id: &str) -> Result<WorkItem> {
        Ok(map_work_item(&self.0.get_json(&format!("{}/_apis/wit/workitems/{id}?{API}", self.0.base)).await?))
    }
    async fn threads(&self, _id: &str) -> Result<Vec<CommentThread>> {
        Ok(vec![])
    }
    async fn timeline(&self, id: &str) -> Result<Vec<TimelineEvent>> {
        // Work-item revisions: surface each System.State change.
        let v = self.0.get_json(&format!("{}/_apis/wit/workItems/{id}/updates?{API}", self.0.base)).await?;
        let mut out = Vec::new();
        for u in get_arr(&v, "value") {
            let state = get_obj(u, "fields").and_then(|f| get_obj(f, "System.State")).and_then(|s| get_str(s, "newValue"));
            if let Some(state) = state {
                out.push(TimelineEvent {
                    actor: get_obj(u, "revisedBy").map(map_user),
                    kind: TimelineEventKind::StateChanged,
                    summary: format!("changed status to {state}"),
                    at: get_date(u, "revisedDate"),
                });
            }
        }
        out.sort_by_key(|e| e.at);
        Ok(out)
    }
    async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        let url = format!("{}/_apis/wit/workitems/{id}?{API}", self.0.base);
        let patch = json!([ { "op": "add", "path": "/fields/System.State", "value": state } ]);
        let resp = self
            .0
            .http
            .patch(&url)
            .header("Content-Type", "application/json-patch+json")
            .body(patch.to_string())
            .send()
            .await
            .map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PATCH {url} -> {}", resp.status())));
        }
        Ok(())
    }
    async fn add_comment(&self, id: &str, body: &str) -> Result<()> {
        let url = format!("{}/{}/_apis/wit/workItems/{id}/comments?api-version=7.1-preview.3", self.0.base, self.0.project);
        self.0.post_json_read(&url, json!({ "text": body })).await.map(|_| ())
    }
    async fn available_states(&self, id: &str) -> Result<Vec<String>> {
        // The states come from the item's work-item type workflow.
        let item = self.0.get_json(&format!("{}/_apis/wit/workitems/{id}?{API}", self.0.base)).await?;
        let Some(t) = get_obj(&item, "fields").and_then(|f| get_str(f, "System.WorkItemType")) else {
            return Ok(Vec::new());
        };
        let url = format!("{}/{}/_apis/wit/workItemTypes/{}/states?{API}", self.0.base, self.0.project, urlencoding(&t));
        let v = self.0.get_json(&url).await?;
        Ok(get_arr(&v, "value").iter().filter_map(|s| get_str(s, "name")).collect())
    }
}

#[async_trait]
impl PipelineSource for AzurePipe {
    async fn discover(&self) -> Result<Vec<PipelineDefinition>> {
        let v = self.0.get_json(&format!("{}/{}/_apis/build/definitions?{API}", self.0.base, self.0.project)).await?;
        Ok(get_arr(&v, "value").iter().map(map_definition).collect())
    }
    async fn list_runs(&self, query: &PipelineRunQuery) -> Result<Vec<PipelineRun>> {
        let mut url = format!("{}/{}/_apis/build/builds?$top={}&{API}", self.0.base, self.0.project, query.limit.unwrap_or(25));
        if let Some(def) = &query.definition_id {
            url.push_str(&format!("&definitions={def}"));
        }
        let v = self.0.get_json(&url).await?;
        Ok(get_arr(&v, "value").iter().map(map_build).collect())
    }
    async fn get_run(&self, run_id: &str) -> Result<PipelineRun> {
        let build = self.0.get_json(&format!("{}/{}/_apis/build/builds/{run_id}?{API}", self.0.base, self.0.project)).await?;
        let mut run = map_build(&build);
        run.stages = self.0.read_stages(run_id).await;
        Ok(run)
    }
    async fn logs(&self, run_id: &str, job_id: Option<&str>) -> Result<String> {
        let url = format!("{}/{}/_apis/build/builds/{run_id}/timeline?{API}", self.0.base, self.0.project);
        let v = self.0.get_json(&url).await?;
        let lines: Vec<String> = get_arr(&v, "records")
            .iter()
            .filter(|r| job_id.is_none() || get_str(r, "id").as_deref() == job_id)
            .map(|r| format!("[{}] {}: {}/{}", get_str(r, "type").unwrap_or_default(), get_str(r, "name").unwrap_or_default(), get_str(r, "state").unwrap_or_default(), get_str(r, "result").unwrap_or_else(|| "-".into())))
            .collect();
        Ok(lines.join("\n"))
    }
    async fn trigger(&self, definition_id: &str, branch: Option<&str>) -> Result<()> {
        let def: i64 = definition_id.parse().map_err(prov)?;
        let body = match branch {
            Some(b) => json!({ "definition": { "id": def }, "sourceBranch": format!("refs/heads/{b}") }),
            None => json!({ "definition": { "id": def } }),
        };
        self.0.post_json_read(&format!("{}/{}/_apis/build/builds?{API}", self.0.base, self.0.project), body).await.map(|_| ())
    }
    fn supports_approvals(&self) -> bool {
        true
    }
    // Azure can surface a pending environment approval (via the run timeline) but the
    // check isn't exposed as an actionable `pipelines/approvals` resource, so we can't
    // submit the decision — approve/reject is view-only, done in the Azure UI.
    fn can_respond_to_approvals(&self) -> bool {
        false
    }
    async fn pending_approvals(&self, run_id: &str) -> Result<Vec<PipelineApproval>> {
        self.0.approval_gates(run_id).await
    }
    async fn respond_approval(&self, _run_id: &str, approval_id: &str, decision: ApprovalDecision, comment: Option<&str>) -> Result<()> {
        let status = match decision {
            ApprovalDecision::Approve => "approved",
            ApprovalDecision::Reject => "rejected",
        };
        let url = format!("{}/{}/_apis/pipelines/approvals?{API}", self.0.base, self.0.project);
        let body = json!([{ "approvalId": approval_id, "status": status, "comment": comment.unwrap_or("") }]);
        let resp = self.0.http.patch(&url).json(&body).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PATCH {url} -> {}", resp.status())));
        }
        Ok(())
    }
}

pub struct AzureConnection {
    id: String,
    display_name: String,
    client: Arc<AzureClient>,
    caps: Capabilities,
}

#[async_trait]
impl ProviderConnection for AzureConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }
    fn provider_type(&self) -> ProviderType {
        ProviderType::AzureDevOps
    }
    fn display_name(&self) -> &str {
        &self.display_name
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
        Some(Arc::new(AzurePr(self.client.clone())))
    }
    fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
        Some(Arc::new(AzureWi(self.client.clone())))
    }
    fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
        Some(Arc::new(AzurePipe(self.client.clone())))
    }
    async fn check(&self) -> bool {
        self.client.get_json(&format!("{}/_apis/connectionData?api-version=7.1-preview", self.client.base)).await.is_ok()
    }
}

pub fn azure_capabilities() -> Capabilities {
    Capabilities {
        supports_pull_requests: true,
        supports_work_items: true,
        supports_pipelines: true,
        vote_style: VoteStyle::NumericVotes,
        supports_merge: true,
        supports_inline_comments: true,
        supports_pipeline_trigger: true,
        supports_pipeline_discovery: true,
        ..Default::default()
    }
}

pub struct AzureDevOpsFactory;

impl ProviderFactory for AzureDevOpsFactory {
    fn provider_type(&self) -> ProviderType {
        ProviderType::AzureDevOps
    }
    fn describe_capabilities(&self) -> Capabilities {
        azure_capabilities()
    }
    fn create(&self, connection: &Connection, secret: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
        let org = connection.organization.clone().ok_or_else(|| Error::Config("Azure DevOps connection requires an Organization".into()))?;
        let project = connection.project.clone().ok_or_else(|| Error::Config("Azure DevOps connection requires a Project".into()))?;
        let repo = connection.repository.clone().unwrap_or_else(|| project.clone());
        let base = connection.base_url.clone().unwrap_or_else(|| format!("https://dev.azure.com/{org}"));
        let base = base.trim_end_matches('/').to_string();

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(pat) = secret {
            let token = base64::engine::general_purpose::STANDARD.encode(format!(":{pat}"));
            headers.insert(AUTHORIZATION, format!("Basic {token}").parse().map_err(prov)?);
        }
        let http = reqwest::Client::builder().default_headers(headers).build().map_err(prov)?;

        let client = Arc::new(AzureClient { http, base, project, repository: repo, self_id: tokio::sync::Mutex::new(None) });
        Ok(Arc::new(AzureConnection { id: connection.id.clone(), display_name: connection.display_name.clone(), client, caps: azure_capabilities() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_gates_from_timeline_picks_pending_with_stage_name() {
        // Stage → Checkpoint → Checkpoint.Approval (one pending, one already completed).
        let records: Value = serde_json::from_str(
            r#"[
                { "id": "stage1", "type": "Stage", "name": "Deploy prod", "parentId": null },
                { "id": "chk1", "type": "Checkpoint", "parentId": "stage1" },
                { "id": "appr-uuid-1", "type": "Checkpoint.Approval", "state": "inProgress", "parentId": "chk1" },
                { "id": "stage2", "type": "Stage", "name": "Deploy staging", "parentId": null },
                { "id": "chk2", "type": "Checkpoint", "parentId": "stage2" },
                { "id": "appr-uuid-2", "type": "Checkpoint.Approval", "state": "completed", "parentId": "chk2" }
            ]"#,
        )
        .unwrap();
        let gates = approval_gates_from_timeline(records.as_array().unwrap());
        assert_eq!(gates.len(), 1, "only the pending gate");
        assert_eq!(gates[0].id, "appr-uuid-1");
        assert_eq!(gates[0].name, "Deploy prod");
        assert!(gates[0].can_respond);
    }

    #[test]
    fn maps_votes_both_ways() {
        assert_eq!(map_vote(10), ReviewVote::Approved);
        assert_eq!(map_vote(-10), ReviewVote::Rejected);
        assert_eq!(to_vote(ReviewVote::Approved), 10);
    }

    #[test]
    fn maps_pr_status_labels_mergeable() {
        let v: Value = serde_json::from_str(
            r#"{ "pullRequestId": 10, "title": "t", "status": "active", "isDraft": false, "mergeStatus": "succeeded",
                 "createdBy": { "id": "g1", "displayName": "Dan" }, "sourceRefName": "refs/heads/feature/x", "targetRefName": "refs/heads/main",
                 "labels": [ { "name": "infra" } ], "reviewers": [ { "id": "r1", "displayName": "Rev", "vote": 10 } ] }"#,
        )
        .unwrap();
        let pr = map_pull_request(&v);
        assert_eq!(pr.status, PullRequestStatus::Open);
        assert_eq!(pr.mergeable, MergeableState::Mergeable);
        assert_eq!(pr.source_ref.as_deref(), Some("feature/x"));
        assert_eq!(pr.labels, vec!["infra".to_string()]);
        assert_eq!(pr.reviewers[0].vote, ReviewVote::Approved);
    }

    #[test]
    fn maps_work_item_fields() {
        let v: Value = serde_json::from_str(
            r#"{ "id": 55, "fields": { "System.Title": "WI", "System.State": "Active", "System.WorkItemType": "Bug", "System.AssignedTo": { "id": "u", "displayName": "Dan" } } }"#,
        )
        .unwrap();
        let wi = map_work_item(&v);
        assert_eq!(wi.state, "Active");
        assert_eq!(wi.state_category, WorkItemStateCategory::Started);
        assert_eq!(wi.assignee.unwrap().display_name, "Dan");
    }

    #[test]
    fn maps_build_and_change() {
        let v: Value = serde_json::from_str(
            r#"{ "id": 900, "buildNumber": "20260601.1", "status": "completed", "result": "succeeded", "sourceBranch": "refs/heads/main", "definition": { "id": 3, "name": "CI" } }"#,
        )
        .unwrap();
        let run = map_build(&v);
        assert_eq!(run.definition_id, "3");
        assert_eq!(run.status, PipelineRunStatus::Succeeded);
        assert_eq!(run.branch.as_deref(), Some("main"));

        let ch: Value = serde_json::from_str(r#"{ "changeType": "edit", "item": { "path": "/src/a.rs" } }"#).unwrap();
        assert_eq!(map_change_entry(&ch).kind, FileChangeKind::Modified);
    }

    #[test]
    fn unified_diff_marks_changes() {
        let (patch, adds, dels) = unified_diff("one\ntwo\nthree\n", "one\nTWO\nthree\nfour\n");
        assert_eq!(adds, 2);
        assert_eq!(dels, 1);
        assert!(patch.contains("+TWO"));
        assert!(patch.contains("-two"));
    }

    #[test]
    fn maps_commit_and_status() {
        let commit: Value = serde_json::from_str(
            r#"{ "commitId": "0123456789abcdef", "comment": "Add retry\nbody", "author": { "name": "Dana", "date": "2026-06-01T10:00:00Z" } }"#,
        )
        .unwrap();
        let c = map_az_commit(&commit);
        assert_eq!(c.sha, "0123456789abcdef"); // full commitId (UI truncates for display)
        assert_eq!(c.message, "Add retry");
        assert_eq!(c.author, "Dana");

        let status: Value = serde_json::from_str(r#"{ "state": "failed", "context": { "name": "Build validation" }, "targetUrl": "t" }"#).unwrap();
        let s = map_az_status(&status);
        assert_eq!(s.name, "Build validation");
        assert_eq!(s.status, CheckStatus::Failed);
    }
}
