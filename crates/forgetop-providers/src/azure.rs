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

use crate::html;
use crate::json::*;
use crate::scope::{self, fan_out, sort_and_cap};

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

pub fn map_pull_request(v: &Value, repo: Option<&str>) -> PullRequest {
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
        // `repository.project.name` + `repository.name` is the connection-relative `project/repo`.
        repository: get_obj(v, "repository")
            .and_then(|r| Some(format!("{}/{}", get_obj(r, "project").and_then(|p| get_str(p, "name"))?, get_str(r, "name")?)))
            .or_else(|| repo.map(str::to_string)),
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

/// `project` is the Team Project the item belongs to — Azure work items are project-addressed,
/// not repo-addressed, so a bare project name is the correct address here.
pub fn map_work_item(v: &Value, project: Option<&str>) -> WorkItem {
    let fields = get_obj(v, "fields").unwrap_or(v);
    let state = field_str(fields, "System.State").unwrap_or_else(|| "New".into());
    let id = get_i64(v, "id");
    WorkItem {
        repository: field_str(fields, "System.TeamProject").or_else(|| project.map(str::to_string)),
        id: id.map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        identifier: id.map(|n| n.to_string()),
        title: field_str(fields, "System.Title").unwrap_or_else(|| "(untitled)".into()),
        // `System.Description` is an HTML field. Neither frontend renders HTML, so it's flattened
        // to text here rather than leaking `<div>`/`<br>` markup into the sidebar (issue #162).
        description: field_str(fields, "System.Description").map(|d| html::to_text(&d)).filter(|d| !d.is_empty()),
        state_category: map_state(&state),
        state,
        work_item_type: field_str(fields, "System.WorkItemType"),
        assignee: get_obj(fields, "System.AssignedTo").map(map_user),
        created_at: get_date(fields, "System.CreatedDate"),
        updated_at: get_date(fields, "System.ChangedDate"),
        url: get_str(v, "url"),
    }
}

/// Azure pipelines are project-addressed too, so `project` is a bare Team Project name.
pub fn map_definition(v: &Value, project: Option<&str>) -> PipelineDefinition {
    PipelineDefinition {
        repository: get_obj(v, "project").and_then(|p| get_str(p, "name")).or_else(|| project.map(str::to_string)),
        id: get_i64(v, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        name: get_str(v, "name").unwrap_or_else(|| "(pipeline)".into()),
        path: get_str(v, "path"),
        url: get_obj(v, "_links").and_then(|l| get_obj(l, "web")).and_then(|w| get_str(w, "href")),
    }
}

pub fn map_build(v: &Value, project: Option<&str>) -> PipelineRun {
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
        repository: get_obj(v, "project").and_then(|p| get_str(p, "name")).or_else(|| project.map(str::to_string)),
        id: get_i64(v, "id").map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        definition_id: get_obj(v, "definition").and_then(|d| get_i64(d, "id")).map(|n| n.to_string()).unwrap_or_else(|| "0".into()),
        number: None,
        name: get_str(v, "buildNumber").or_else(|| get_obj(v, "definition").and_then(|d| get_str(d, "name"))),
        title: None,
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

/// The organisation-wide `_apis/git/repositories` response → **connection-relative**
/// `project/repo` paths. Azure has no single field for this, so it is assembled from the
/// repository's own `project.name` and `name` — never parsed out of a URL.
pub fn repositories_from_page(v: &Value) -> Vec<String> {
    get_arr(v, "value")
        .iter()
        .filter_map(|r| Some(format!("{}/{}", get_obj(r, "project").and_then(|p| get_str(p, "name"))?, get_str(r, "name")?)))
        .collect()
}

pub struct AzureClient {
    http: reqwest::Client,
    base: String,
    /// The repositories this connection fetches from, **connection-relative** (`project/repo`).
    /// An Azure PAT reaches every repository in the organization, so this is a user-chosen scope.
    scope: Vec<String>,
    self_id: tokio::sync::Mutex<Option<String>>,
}

/// Splits a connection-relative Azure scope entry into its two address components. Azure project
/// and repository names cannot themselves contain `/`, so the first separator is the boundary.
fn split_project_repo(entry: &str) -> (String, String) {
    match entry.split_once('/') {
        Some((project, repo)) => (project.to_string(), repo.to_string()),
        // A bare name addresses a Team Project whose repo is named after it (Azure's own default).
        None => (entry.to_string(), entry.to_string()),
    }
}

/// The Team Project half of a scope entry. Work items and pipelines are addressed by this.
fn project_part(entry: &str) -> String {
    split_project_repo(entry).0
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

    /// `repo` is the connection-relative `project/repo` scope entry.
    fn pr_base(&self, repo: &str, id: &str) -> String {
        let (project, repository) = split_project_repo(repo);
        format!("{}/{project}/_apis/git/repositories/{repository}/pullRequests/{id}", self.base)
    }

    fn git_base(&self, repo: &str) -> String {
        let (project, repository) = split_project_repo(repo);
        format!("{}/{project}/_apis/git/repositories/{repository}", self.base)
    }

    fn resolve(&self, item: &ItemRef) -> Result<String> {
        scope::resolve_repo(item, &self.scope)
    }

    /// The distinct Team Projects the scope covers, in first-seen order.
    ///
    /// Work items and pipelines are addressed **per project**, while the scope is per repository —
    /// so fanning them out over the scope directly would query a project once per repository it
    /// contains and return every item twice. Deduplicating here is what stops that.
    fn projects(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for entry in &self.scope {
            let project = project_part(entry);
            if !out.contains(&project) {
                out.push(project);
            }
        }
        out
    }

    /// The Team Project to address a project-scoped call at.
    fn resolve_project(&self, item: &ItemRef) -> Result<String> {
        if let Some(repo) = &item.repo {
            return Ok(project_part(repo));
        }
        let projects = self.projects();
        match projects.as_slice() {
            [only] => Ok(only.clone()),
            [] => Err(Error::Config(
                "this connection has no repositories selected — choose some in the repository scope picker".into(),
            )),
            _ => Err(Error::Config(format!(
                "this connection spans {} projects, so '{}' needs the project it belongs to",
                projects.len(),
                item.id
            ))),
        }
    }

    /// Every Git repository in the organization, as connection-relative `project/repo`.
    ///
    /// The project segment is deliberately omitted from the path: `{org}/_apis/git/repositories`
    /// lists organization-wide, which is what makes one connection cover the whole account.
    /// Azure returns the full set in a single response, so there is nothing to paginate and
    /// `truncated` is always false.
    async fn discover_repositories(&self) -> Result<RepositoryPage> {
        let v = self.get_json(&format!("{}/_apis/git/repositories?{API}", self.base)).await?;
        Ok(RepositoryPage { repositories: repositories_from_page(&v), truncated: false })
    }

    async fn self_id(&self) -> Result<Option<String>> {
        let mut guard = self.self_id.lock().await;
        if guard.is_none() {
            let v = self.get_json(&format!("{}/_apis/connectionData?api-version=7.1-preview", self.base)).await?;
            *guard = get_obj(&v, "authenticatedUser").and_then(|u| get_str(u, "id"));
        }
        Ok(guard.clone())
    }

    async fn item_content(&self, repo: &str, path: &str, commit: &str) -> Option<String> {
        let url = format!(
            "{}/items?path={}&versionDescriptor.versionType=commit&versionDescriptor.version={}&includeContent=true&{API}",
            self.git_base(repo),
            urlencoding(path),
            commit
        );
        let v = self.get_json(&url).await.ok()?;
        get_str(&v, "content")
    }

    async fn read_stages(&self, project: &str, run_id: &str) -> Vec<PipelineStage> {
        let url = format!("{}/{project}/_apis/build/builds/{run_id}/timeline?{API}", self.base);
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
    async fn approval_gates(&self, project: &str, run_id: &str) -> Result<Vec<PipelineApproval>> {
        let url = format!("{}/{project}/_apis/build/builds/{run_id}/timeline?{API}", self.base);
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

impl AzureWi {
    async fn patch_work_item(&self, id: &str, patch: Value) -> Result<()> {
        let url = format!("{}/_apis/wit/workitems/{id}?{API}", self.0.base);
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
}

#[async_trait]
impl PullRequestSource for AzurePr {
    async fn list(&self, query: &PullRequestQuery) -> Result<Vec<PullRequest>> {
        let scope = &self.0.scope;
        if scope.is_empty() {
            return Ok(Vec::new());
        }
        let status = if query.include_completed { "all" } else { "active" };
        let top = query.limit.unwrap_or(50);
        let rows = fan_out(scope, "azure.pull_requests.list", |repo| async move {
            let (project, repository) = split_project_repo(&repo);
            let url = format!(
                "{}/{project}/_apis/git/repositories/{repository}/pullrequests?searchCriteria.status={status}&$top={top}&{API}",
                self.0.base
            );
            let v = self.0.get_json(&url).await?;
            Ok(get_arr(&v, "value").iter().map(|pr| map_pull_request(pr, Some(&repo))).collect())
        })
        .await;
        let me = if query.filter == PullRequestFilter::All { None } else { self.0.self_id().await? };
        let filtered = apply_pull_request_filter(rows, query.filter, me.as_deref());
        // Azure's PR payload carries no `updated_at`, so creation date is the best recency key.
        Ok(sort_and_cap(filtered, scope.len(), query.limit, |pr| pr.created_at))
    }
    async fn get(&self, item: &ItemRef) -> Result<PullRequest> {
        let repo = self.0.resolve(item)?;
        Ok(map_pull_request(&self.0.get_json(&format!("{}?{API}", self.0.pr_base(&repo, &item.id))).await?, Some(&repo)))
    }
    async fn timeline(&self, item: &ItemRef) -> Result<Vec<TimelineEvent>> {
        let repo = self.0.resolve(item)?;
        // Azure has no single timeline endpoint; derive events from the reviewers' votes and the
        // PR's completion status (vote timestamps aren't exposed, so those events carry no time).
        let pr = self.0.get_json(&format!("{}?{API}", self.0.pr_base(&repo, &item.id))).await?;
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
    async fn threads(&self, item: &ItemRef) -> Result<Vec<CommentThread>> {
        let repo = self.0.resolve(item)?;
        let v = self.0.get_json(&format!("{}/threads?{API}", self.0.pr_base(&repo, &item.id))).await?;
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
    async fn changes(&self, item: &ItemRef) -> Result<Vec<FileChange>> {
        let repo = self.0.resolve(item)?;
        let pr_base = self.0.pr_base(&repo, &item.id);
        let iters = self.0.get_json(&format!("{pr_base}/iterations?{API}")).await?;
        let iterations = get_arr(&iters, "value");
        let Some(last) = iterations.last() else { return Ok(vec![]) };
        let new_commit = get_obj(last, "sourceRefCommit").and_then(|c| get_str(c, "commitId"));
        let base_commit = get_obj(last, "commonRefCommit")
            .and_then(|c| get_str(c, "commitId"))
            .or_else(|| get_obj(last, "targetRefCommit").and_then(|c| get_str(c, "commitId")));
        let iter_id = get_i64(last, "id").unwrap_or(1);

        let changes = self.0.get_json(&format!("{pr_base}/iterations/{iter_id}/changes?{API}")).await?;
        let mut out = Vec::new();
        for raw in get_arr(&changes, "changeEntries") {
            if get_obj(raw, "item").map(|i| get_bool(i, "isFolder")).unwrap_or(false) {
                continue;
            }
            let mut change = map_change_entry(raw);
            let old = if matches!(change.kind, FileChangeKind::Added) {
                Some(String::new())
            } else if let Some(c) = &base_commit {
                self.0.item_content(&repo, &change.path, c).await
            } else {
                None
            };
            let new = if matches!(change.kind, FileChangeKind::Deleted) {
                Some(String::new())
            } else if let Some(c) = &new_commit {
                self.0.item_content(&repo, &change.path, c).await
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
    async fn commits(&self, item: &ItemRef) -> Result<Vec<Commit>> {
        let repo = self.0.resolve(item)?;
        let v = self.0.get_json(&format!("{}/commits?{API}", self.0.pr_base(&repo, &item.id))).await?;
        Ok(get_arr(&v, "value").iter().map(map_az_commit).collect())
    }
    async fn commit_changes(&self, item: &ItemRef, sha: &str) -> Result<Vec<FileChange>> {
        let repo = self.0.resolve(item)?;
        // Diff the commit against its first parent, computing each file's patch from item content
        // (same approach as the whole-PR `changes`). A root commit diffs against an empty tree.
        let git = self.0.git_base(&repo);
        let commit = self.0.get_json(&format!("{git}/commits/{sha}?{API}")).await?;
        let parent = get_arr(&commit, "parents").first().and_then(|p| p.as_str().map(String::from));
        let changes = self.0.get_json(&format!("{git}/commits/{sha}/changes?{API}")).await?;
        let mut out = Vec::new();
        for raw in get_arr(&changes, "changes") {
            if get_obj(raw, "item").map(|i| get_bool(i, "isFolder")).unwrap_or(false) {
                continue;
            }
            let mut change = map_change_entry(raw);
            let old = if matches!(change.kind, FileChangeKind::Added) {
                Some(String::new())
            } else if let Some(p) = &parent {
                self.0.item_content(&repo, &change.path, p).await
            } else {
                Some(String::new())
            };
            let new = if matches!(change.kind, FileChangeKind::Deleted) {
                Some(String::new())
            } else {
                self.0.item_content(&repo, &change.path, sha).await
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
    async fn checks(&self, item: &ItemRef) -> Result<Vec<CheckRun>> {
        let repo = self.0.resolve(item)?;
        let v = self.0.get_json(&format!("{}/statuses?{API}", self.0.pr_base(&repo, &item.id))).await?;
        Ok(get_arr(&v, "value").iter().map(map_az_status).collect())
    }
    async fn add_comment(&self, item: &ItemRef, body: &str) -> Result<()> {
        let repo = self.0.resolve(item)?;
        self.0
            .post_json_read(
                &format!("{}/threads?{API}", self.0.pr_base(&repo, &item.id)),
                json!({ "comments": [ { "content": body, "commentType": 1 } ], "status": 1 }),
            )
            .await
            .map(|_| ())
    }
    async fn reply_to_thread(&self, item: &ItemRef, thread_id: &str, body: &str) -> Result<()> {
        let repo = self.0.resolve(item)?;
        // Append a reply comment to an existing thread; parentCommentId 1 is the thread's root.
        self.0
            .post_json_read(
                &format!("{}/threads/{thread_id}/comments?{API}", self.0.pr_base(&repo, &item.id)),
                json!({ "content": body, "parentCommentId": 1, "commentType": 1 }),
            )
            .await
            .map(|_| ())
    }
    async fn vote(&self, item: &ItemRef, vote: ReviewVote) -> Result<()> {
        let repo = self.0.resolve(item)?;
        let self_id = self.0.self_id().await?.ok_or_else(|| Error::Provider("could not resolve authenticated user".into()))?;
        let url = format!("{}/reviewers/{self_id}?{API}", self.0.pr_base(&repo, &item.id));
        let resp = self.0.http.put(&url).json(&json!({ "vote": to_vote(vote) })).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PUT {url} -> {}", resp.status())));
        }
        Ok(())
    }
    async fn merge(&self, item: &ItemRef, options: &MergeOptions) -> Result<()> {
        let repo = self.0.resolve(item)?;
        let pr = self.0.get_json(&format!("{}?{API}", self.0.pr_base(&repo, &item.id))).await?;
        let source = get_obj(&pr, "lastMergeSourceCommit").and_then(|c| get_str(c, "commitId"));
        let strategy = match options.strategy {
            MergeStrategy::Squash => "squash",
            MergeStrategy::Rebase => "rebase",
            MergeStrategy::Merge => "noFastForward",
        };
        let url = format!("{}?{API}", self.0.pr_base(&repo, &item.id));
        let body = json!({ "status": "completed", "lastMergeSourceCommit": { "commitId": source }, "completionOptions": { "mergeStrategy": strategy, "deleteSourceBranch": options.delete_source_ref } });
        let resp = self.0.http.patch(&url).json(&body).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PATCH {url} -> {}", resp.status())));
        }
        Ok(())
    }
    async fn revert(&self, item: &ItemRef) -> Result<()> {
        let repo = self.0.resolve(item)?;
        let id = &item.id;
        // Azure creates the revert on a new branch off the PR's target; the user opens a PR from it.
        let pr = self.0.get_json(&format!("{}?{API}", self.0.pr_base(&repo, id))).await?;
        let onto = get_str(&pr, "targetRefName")
            .ok_or_else(|| Error::Provider(format!("pull request '{id}' has no target branch")))?;
        let pr_id: i64 = id.parse().map_err(|_| Error::Provider(format!("pull request id '{id}' is not numeric")))?;
        let url = format!("{}/reverts?{API}", self.0.git_base(&repo));
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
        // Work items are project-addressed, so this fans out over the scope's *distinct projects*
        // — not its repositories. Two repositories in one project must not query it twice.
        let projects = self.0.projects();
        if projects.is_empty() {
            return Ok(Vec::new());
        }
        let mut conditions = vec!["[System.TeamProject] = @project".to_string()];
        if query.mine_only {
            conditions.push("[System.AssignedTo] = @me".into());
        }
        if !query.include_completed {
            conditions.push("[System.State] NOT IN ('Closed', 'Done', 'Removed')".into());
        }
        let wiql = format!("SELECT [System.Id] FROM WorkItems WHERE {} ORDER BY [System.ChangedDate] DESC", conditions.join(" AND "));
        let top = query.limit.unwrap_or(50);
        let rows = fan_out(&projects, "azure.work_items.list", |project| {
            let wiql = wiql.clone();
            async move {
                let url = format!("{}/{project}/_apis/wit/wiql?$top={top}&{API}", self.0.base);
                let ids_v = self.0.post_json_read(&url, json!({ "query": wiql })).await?;
                let ids: Vec<String> = get_arr(&ids_v, "workItems").iter().filter_map(|w| get_i64(w, "id")).map(|n| n.to_string()).collect();
                if ids.is_empty() {
                    return Ok(vec![]);
                }
                let v = self.0.get_json(&format!("{}/_apis/wit/workitems?ids={}&{API}", self.0.base, ids.join(","))).await?;
                Ok(get_arr(&v, "value").iter().map(|w| map_work_item(w, Some(&project))).collect())
            }
        })
        .await;
        Ok(sort_and_cap(rows, projects.len(), query.limit, |wi| wi.updated_at))
    }
    async fn get(&self, item: &ItemRef) -> Result<WorkItem> {
        // Work items are addressable organization-wide by id, so no project segment is needed.
        let v = self.0.get_json(&format!("{}/_apis/wit/workitems/{}?{API}", self.0.base, item.id)).await?;
        Ok(map_work_item(&v, item.repo.as_deref()))
    }
    async fn threads(&self, _item: &ItemRef) -> Result<Vec<CommentThread>> {
        Ok(vec![])
    }
    async fn timeline(&self, item: &ItemRef) -> Result<Vec<TimelineEvent>> {
        // Work-item revisions: surface each System.State change.
        let v = self.0.get_json(&format!("{}/_apis/wit/workItems/{}/updates?{API}", self.0.base, item.id)).await?;
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
    async fn set_state(&self, item: &ItemRef, state: &str) -> Result<()> {
        self.patch_work_item(&item.id, json!([ { "op": "add", "path": "/fields/System.State", "value": state } ])).await
    }
    async fn add_comment(&self, item: &ItemRef, body: &str) -> Result<()> {
        let project = self.0.resolve_project(item)?;
        let url = format!("{}/{project}/_apis/wit/workItems/{}/comments?api-version=7.1-preview.3", self.0.base, item.id);
        self.0.post_json_read(&url, json!({ "text": body })).await.map(|_| ())
    }
    async fn available_states(&self, item: &ItemRef) -> Result<Vec<String>> {
        let project = self.0.resolve_project(item)?;
        // The states come from the item's work-item type workflow.
        let wi = self.0.get_json(&format!("{}/_apis/wit/workitems/{}?{API}", self.0.base, item.id)).await?;
        let Some(t) = get_obj(&wi, "fields").and_then(|f| get_str(f, "System.WorkItemType")) else {
            return Ok(Vec::new());
        };
        let url = format!("{}/{project}/_apis/wit/workItemTypes/{}/states?{API}", self.0.base, urlencoding(&t));
        let v = self.0.get_json(&url).await?;
        Ok(get_arr(&v, "value").iter().filter_map(|s| get_str(s, "name")).collect())
    }
    async fn assignable_users(&self, item: &ItemRef) -> Result<Vec<User>> {
        let project = self.0.resolve_project(item)?;
        let teams = self.0.get_json(&format!("{}/_apis/projects/{project}/teams?{API}", self.0.base)).await?;
        let Some(team_id) = get_arr(&teams, "value").first().and_then(|t| get_str(t, "id")) else {
            return Ok(Vec::new());
        };
        let members = self
            .0
            .get_json(&format!("{}/_apis/projects/{project}/teams/{team_id}/members?{API}", self.0.base))
            .await?;
        Ok(get_arr(&members, "value")
            .iter()
            .filter_map(|m| {
                let identity = get_obj(m, "identity")?;
                let unique_name = get_str(identity, "uniqueName")?;
                Some(User {
                    id: unique_name.clone(),
                    display_name: get_str(identity, "displayName").unwrap_or_else(|| unique_name.clone()),
                    handle: Some(unique_name),
                    avatar_url: None,
                })
            })
            .collect())
    }
    async fn set_assignee(&self, item: &ItemRef, assignee_id: Option<&str>) -> Result<()> {
        let patch = match assignee_id {
            Some(unique_name) => json!([ { "op": "add", "path": "/fields/System.AssignedTo", "value": unique_name } ]),
            None => json!([ { "op": "remove", "path": "/fields/System.AssignedTo" } ]),
        };
        self.patch_work_item(&item.id, patch).await
    }
    async fn update_fields(&self, item: &ItemRef, title: Option<&str>, description: Option<&str>) -> Result<()> {
        let mut patch = Vec::new();
        if let Some(title) = title {
            patch.push(json!({ "op": "add", "path": "/fields/System.Title", "value": title }));
        }
        if let Some(description) = description {
            // The editor round-trips the flattened text from `map_work_item`, so the write side
            // puts it back into the HTML the field expects — otherwise every line break is lost.
            let value = html::to_html(description);
            patch.push(json!({ "op": "add", "path": "/fields/System.Description", "value": value }));
        }
        if patch.is_empty() {
            return Ok(());
        }
        self.patch_work_item(&item.id, json!(patch)).await
    }
}

#[async_trait]
impl PipelineSource for AzurePipe {
    async fn discover(&self) -> Result<Vec<PipelineDefinition>> {
        // Build definitions belong to a *project*, not a repository. Fanning out over the scope's
        // repositories would return every definition once per repository in its project; the
        // distinct-project list is what keeps a two-repo project from duplicating them.
        let projects = self.0.projects();
        Ok(fan_out(&projects, "azure.pipelines.discover", |project| async move {
            let v = self.0.get_json(&format!("{}/{project}/_apis/build/definitions?{API}", self.0.base)).await?;
            Ok(get_arr(&v, "value").iter().map(|d| map_definition(d, Some(&project))).collect())
        })
        .await)
    }
    async fn list_runs(&self, query: &PipelineRunQuery) -> Result<Vec<PipelineRun>> {
        // Same reasoning as `discover`: per project, deduplicated, never per repository.
        let projects: Vec<String> = match &query.repository {
            Some(repo) => vec![project_part(repo)],
            None => self.0.projects(),
        };
        if projects.is_empty() {
            return Ok(Vec::new());
        }
        let top = query.limit.unwrap_or(25);
        let rows = fan_out(&projects, "azure.pipelines.list_runs", |project| async move {
            let mut url = format!("{}/{project}/_apis/build/builds?$top={top}&{API}", self.0.base);
            if let Some(def) = &query.definition_id {
                url.push_str(&format!("&definitions={def}"));
            }
            let v = self.0.get_json(&url).await?;
            Ok(get_arr(&v, "value").iter().map(|b| map_build(b, Some(&project))).collect())
        })
        .await;
        Ok(sort_and_cap(rows, projects.len(), query.limit, |r| r.started_at))
    }
    async fn get_run(&self, run: &ItemRef) -> Result<PipelineRun> {
        let project = self.0.resolve_project(run)?;
        let build = self.0.get_json(&format!("{}/{project}/_apis/build/builds/{}?{API}", self.0.base, run.id)).await?;
        let mut mapped = map_build(&build, Some(&project));
        mapped.stages = self.0.read_stages(&project, &run.id).await;
        Ok(mapped)
    }
    async fn cancel_run(&self, run: &ItemRef) -> Result<()> {
        let project = self.0.resolve_project(run)?;
        let url = format!("{}/{project}/_apis/build/builds/{}?{API}", self.0.base, run.id);
        let resp = self.0.http.patch(&url).json(&json!({ "status": "cancelling" })).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("PATCH {url} -> {}", resp.status())));
        }
        Ok(())
    }
    async fn logs(&self, run: &ItemRef, job_id: Option<&str>) -> Result<String> {
        let project = self.0.resolve_project(run)?;
        let url = format!("{}/{project}/_apis/build/builds/{}/timeline?{API}", self.0.base, run.id);
        let v = self.0.get_json(&url).await?;
        let lines: Vec<String> = get_arr(&v, "records")
            .iter()
            .filter(|r| job_id.is_none() || get_str(r, "id").as_deref() == job_id)
            .map(|r| format!("[{}] {}: {}/{}", get_str(r, "type").unwrap_or_default(), get_str(r, "name").unwrap_or_default(), get_str(r, "state").unwrap_or_default(), get_str(r, "result").unwrap_or_else(|| "-".into())))
            .collect();
        Ok(lines.join("\n"))
    }
    async fn trigger(&self, definition: &ItemRef, branch: Option<&str>) -> Result<()> {
        let project = self.0.resolve_project(definition)?;
        let def: i64 = definition.id.parse().map_err(prov)?;
        let body = match branch {
            Some(b) => json!({ "definition": { "id": def }, "sourceBranch": format!("refs/heads/{b}") }),
            None => json!({ "definition": { "id": def } }),
        };
        self.0.post_json_read(&format!("{}/{project}/_apis/build/builds?{API}", self.0.base), body).await.map(|_| ())
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
    async fn pending_approvals(&self, run: &ItemRef) -> Result<Vec<PipelineApproval>> {
        let project = self.0.resolve_project(run)?;
        self.0.approval_gates(&project, &run.id).await
    }
    async fn respond_approval(&self, run: &ItemRef, approval_id: &str, decision: ApprovalDecision, comment: Option<&str>) -> Result<()> {
        let project = self.0.resolve_project(run)?;
        let status = match decision {
            ApprovalDecision::Approve => "approved",
            ApprovalDecision::Reject => "rejected",
        };
        let url = format!("{}/{project}/_apis/pipelines/approvals?{API}", self.0.base);
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
    async fn discover_repositories(&self) -> Result<RepositoryPage> {
        self.client.discover_repositories().await
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
        // The organization stays required — discovery is addressed by it. Project and repository
        // are not: an organization-scoped PAT reaches every repository in the org, and the scope
        // picker fills the rest in.
        let org = connection.organization.clone().ok_or_else(|| Error::Config("Azure DevOps connection requires an Organization".into()))?;
        let scope = connection.resolve_repo_scope(|| {
            let project = connection.project.clone()?;
            let repo = connection.repository.clone().unwrap_or_else(|| project.clone());
            Some(format!("{project}/{repo}"))
        });
        let base = connection.base_url.clone().unwrap_or_else(|| format!("https://dev.azure.com/{org}"));
        let base = base.trim_end_matches('/').to_string();

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(pat) = secret {
            let token = base64::engine::general_purpose::STANDARD.encode(format!(":{pat}"));
            headers.insert(AUTHORIZATION, format!("Basic {token}").parse().map_err(prov)?);
        }
        let http = reqwest::Client::builder().default_headers(headers).build().map_err(prov)?;

        let client = Arc::new(AzureClient { http, base, scope, self_id: tokio::sync::Mutex::new(None) });
        Ok(Arc::new(AzureConnection { id: connection.id.clone(), display_name: connection.display_name.clone(), client, caps: azure_capabilities() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_assembles_project_and_repo_rather_than_parsing_a_url() {
        let page: Value = serde_json::from_str(
            r#"{ "count": 2, "value": [
                   { "id": "r1", "name": "pay-api", "project": { "id": "p1", "name": "Payments" },
                     "remoteUrl": "https://dev.azure.com/contoso/Payments/_git/pay-api" },
                   { "id": "r2", "name": "ledger", "project": { "id": "p2", "name": "Ledger" } } ] }"#,
        )
        .unwrap();
        let repos = repositories_from_page(&page);
        assert_eq!(repos, vec!["Payments/pay-api".to_string(), "Ledger/ledger".to_string()]);
        // `remoteUrl` would be the host-qualified spelling; the address is built from the fields.
        assert!(repos.iter().all(|r| !r.contains("dev.azure.com")));
        // And each entry is exactly the project/repo shape `split_project_repo` expects.
        assert!(repos.iter().all(|r| r.split('/').count() == 2));
        // A repository without a project can't be addressed, so it's dropped rather than guessed.
        assert!(repositories_from_page(&serde_json::json!({ "value": [ { "name": "orphan" } ] })).is_empty());
    }

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

    fn client_with_scope(scope: &[&str]) -> AzureClient {
        AzureClient {
            http: reqwest::Client::new(),
            base: "https://dev.azure.com/contoso".into(),
            scope: scope.iter().map(|s| s.to_string()).collect(),
            self_id: tokio::sync::Mutex::new(None),
        }
    }

    #[test]
    fn splits_a_scope_entry_into_project_and_repo() {
        assert_eq!(split_project_repo("Payments/pay-api"), ("Payments".to_string(), "pay-api".to_string()));
        // A bare name is the Azure default: a Team Project whose repo shares its name.
        assert_eq!(split_project_repo("Payments"), ("Payments".to_string(), "Payments".to_string()));
        assert_eq!(project_part("Payments/pay-api"), "Payments");
    }

    #[test]
    fn work_items_and_pipelines_fan_out_over_distinct_projects_not_repositories() {
        // Two repositories in one Team Project. Work items and pipelines are project-addressed,
        // so querying per repository would hit Payments twice and return every item and every
        // pipeline definition twice. The distinct-project list is what prevents that.
        let client = client_with_scope(&["Payments/pay-api", "Payments/pay-web", "Ledger/ledger"]);
        assert_eq!(client.projects(), vec!["Payments".to_string(), "Ledger".to_string()]);
    }

    #[test]
    fn a_project_scoped_call_resolves_the_project_from_the_item() {
        let client = client_with_scope(&["Payments/pay-api", "Ledger/ledger"]);
        // An item that knows its address resolves even across projects…
        assert_eq!(client.resolve_project(&ItemRef::in_repo("Ledger/ledger", "42")).unwrap(), "Ledger");
        // …and a bare project name (what Azure work items carry) resolves too.
        assert_eq!(client.resolve_project(&ItemRef::in_repo("Ledger", "42")).unwrap(), "Ledger");
        // An unaddressed item across two projects is refused rather than guessed at.
        assert!(client.resolve_project(&ItemRef::new("42")).is_err());
        // One project, however many repositories — no ambiguity, so it still resolves.
        let single = client_with_scope(&["Payments/pay-api", "Payments/pay-web"]);
        assert_eq!(single.resolve_project(&ItemRef::new("42")).unwrap(), "Payments");
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
        let pr = map_pull_request(&v, None);
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
        let wi = map_work_item(&v, None);
        assert_eq!(wi.state, "Active");
        assert_eq!(wi.state_category, WorkItemStateCategory::Started);
        assert_eq!(wi.assignee.unwrap().display_name, "Dan");
    }

    #[test]
    fn flattens_the_html_description_azure_returns() {
        // `System.Description` is HTML; the sidebar shows text, so the markup is stripped here.
        let v: Value = serde_json::from_str(
            r#"{ "id": 56, "fields": { "System.Title": "WI", "System.State": "New",
                 "System.Description": "<div>Token expires&nbsp;early.</div><div><br></div><ul><li>Sign in</li></ul>" } }"#,
        )
        .unwrap();
        let wi = map_work_item(&v, None);
        assert_eq!(wi.description.as_deref(), Some("Token expires early.\n\n- Sign in"));

        // An empty HTML shell is no description at all, not an empty pane.
        let blank: Value = serde_json::from_str(r#"{ "id": 57, "fields": { "System.Description": "<div><br></div>" } }"#).unwrap();
        assert_eq!(map_work_item(&blank, None).description, None);
    }

    #[test]
    fn maps_build_and_change() {
        let v: Value = serde_json::from_str(
            r#"{ "id": 900, "buildNumber": "20260601.1", "status": "completed", "result": "succeeded", "sourceBranch": "refs/heads/main", "definition": { "id": 3, "name": "CI" } }"#,
        )
        .unwrap();
        let run = map_build(&v, None);
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
