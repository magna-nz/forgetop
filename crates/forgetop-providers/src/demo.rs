//! Demo provider — canned, deterministic data so `--demo` works with no credentials.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use forgetop_core::domain::*;
use forgetop_core::filter::apply_pull_request_filter;
use forgetop_core::provider::*;
use forgetop_core::Result;

fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 30, 9, 0, 0).unwrap()
}

fn user(id: &str, name: &str, handle: &str) -> User {
    User { id: id.into(), display_name: name.into(), handle: Some(handle.into()), avatar_url: None }
}

fn alice() -> User {
    user("u1", "Alice Ng", "alice")
}
fn bob() -> User {
    user("u2", "Bob Reyes", "bob")
}
fn carol() -> User {
    user("u3", "Carol Diaz", "carol")
}

fn pull_requests() -> Vec<PullRequest> {
    let now = base();
    vec![
        PullRequest {
            id: "101".into(),
            number: Some(101),
            title: "Add retry policy to HTTP client".into(),
            description: Some("Wraps outbound calls in a retry with jitter.\n\nReduces 5xx-related flakiness in the banking sync job.".into()),
            author: alice(),
            status: PullRequestStatus::Open,
            is_draft: false,
            source_ref: Some("feature/retry".into()),
            target_ref: Some("main".into()),
            reviewers: vec![
                Reviewer { user: bob(), vote: ReviewVote::Approved, is_required: false },
                Reviewer { user: carol(), vote: ReviewVote::WaitingForAuthor, is_required: true },
            ],
            labels: vec!["banking".into(), "enhancement".into()],
            checks: CheckStatus::Passed,
            check_summary: Some(CheckSummary { successful: 14, neutral: 1, ..Default::default() }),
            mergeable: MergeableState::Mergeable,
            changed_files: 2,
            additions: 24,
            deletions: 2,
            created_at: Some(now - chrono::Duration::hours(30)),
            updated_at: Some(now - chrono::Duration::hours(2)),
            url: Some("https://example.test/pr/101".into()),
        },
        PullRequest {
            id: "102".into(),
            number: Some(102),
            title: "Fix flaky pipeline cache key".into(),
            description: Some("WIP — still narrowing down the cache key collision.".into()),
            author: bob(),
            status: PullRequestStatus::Draft,
            is_draft: true,
            source_ref: Some("fix/cache".into()),
            target_ref: Some("main".into()),
            reviewers: vec![Reviewer { user: alice(), vote: ReviewVote::NoVote, is_required: false }],
            labels: vec!["wip".into(), "ci".into()],
            checks: CheckStatus::Pending,
            check_summary: Some(CheckSummary { successful: 2, in_progress: 12, neutral: 1, ..Default::default() }),
            mergeable: MergeableState::Blocked,
            changed_files: 3,
            additions: 12,
            deletions: 5,
            created_at: Some(now - chrono::Duration::hours(6)),
            updated_at: None,
            url: Some("https://example.test/pr/102".into()),
        },
        PullRequest {
            id: "100".into(),
            number: Some(100),
            title: "Bump dependencies".into(),
            description: None,
            author: carol(),
            status: PullRequestStatus::Merged,
            is_draft: false,
            source_ref: Some("chore/bump".into()),
            target_ref: Some("main".into()),
            reviewers: vec![],
            labels: vec!["chore".into()],
            checks: CheckStatus::Passed,
            check_summary: None,
            mergeable: MergeableState::Unknown,
            changed_files: 8,
            additions: 120,
            deletions: 60,
            created_at: Some(now - chrono::Duration::days(3)),
            updated_at: Some(now - chrono::Duration::days(2)),
            url: Some("https://example.test/pr/100".into()),
        },
    ]
}

fn work_items() -> Vec<WorkItem> {
    let now = base();
    vec![
        WorkItem {
            id: "w1".into(),
            identifier: Some("FOR-12".into()),
            title: "Design the provider abstraction".into(),
            description: Some("Capability-scoped source traits.".into()),
            state: "In Progress".into(),
            state_category: WorkItemStateCategory::Started,
            work_item_type: Some("Story".into()),
            assignee: Some(alice()),
            created_at: Some(now - chrono::Duration::days(5)),
            updated_at: Some(now - chrono::Duration::hours(3)),
            url: Some("https://example.test/wi/12".into()),
        },
        WorkItem {
            id: "w2".into(),
            identifier: Some("FOR-13".into()),
            title: "Pipeline auto-discovery".into(),
            description: None,
            state: "Todo".into(),
            state_category: WorkItemStateCategory::Unstarted,
            work_item_type: Some("Task".into()),
            assignee: Some(bob()),
            created_at: Some(now - chrono::Duration::days(4)),
            updated_at: None,
            url: Some("https://example.test/wi/13".into()),
        },
        WorkItem {
            id: "w3".into(),
            identifier: Some("FOR-9".into()),
            title: "Spike: ratatui".into(),
            description: None,
            state: "Done".into(),
            state_category: WorkItemStateCategory::Completed,
            work_item_type: Some("Spike".into()),
            assignee: Some(carol()),
            created_at: Some(now - chrono::Duration::days(9)),
            updated_at: Some(now - chrono::Duration::days(6)),
            url: Some("https://example.test/wi/9".into()),
        },
        WorkItem {
            id: "w4".into(),
            identifier: Some("FOR-20".into()),
            title: "Add GitLab merge-request support".into(),
            description: None,
            state: "In Progress".into(),
            state_category: WorkItemStateCategory::Started,
            work_item_type: Some("Story".into()),
            assignee: Some(bob()),
            created_at: Some(now - chrono::Duration::days(3)),
            updated_at: Some(now - chrono::Duration::hours(6)),
            url: Some("https://example.test/wi/20".into()),
        },
        WorkItem {
            id: "w5".into(),
            identifier: Some("FOR-21".into()),
            title: "Bitbucket pipelines pagination".into(),
            description: None,
            state: "In Review".into(),
            state_category: WorkItemStateCategory::Started,
            work_item_type: Some("Task".into()),
            assignee: Some(carol()),
            created_at: Some(now - chrono::Duration::days(2)),
            updated_at: Some(now - chrono::Duration::hours(2)),
            url: Some("https://example.test/wi/21".into()),
        },
        WorkItem {
            id: "w6".into(),
            identifier: Some("FOR-22".into()),
            title: "Keychain error handling on Linux".into(),
            description: None,
            state: "Blocked".into(),
            state_category: WorkItemStateCategory::Started,
            work_item_type: Some("Bug".into()),
            assignee: Some(alice()),
            created_at: Some(now - chrono::Duration::days(7)),
            updated_at: Some(now - chrono::Duration::days(1)),
            url: Some("https://example.test/wi/22".into()),
        },
        WorkItem {
            id: "w7".into(),
            identifier: Some("FOR-23".into()),
            title: "High-contrast theme".into(),
            description: None,
            state: "Backlog".into(),
            state_category: WorkItemStateCategory::Backlog,
            work_item_type: Some("Story".into()),
            assignee: None,
            created_at: Some(now - chrono::Duration::days(12)),
            updated_at: None,
            url: Some("https://example.test/wi/23".into()),
        },
        WorkItem {
            id: "w8".into(),
            identifier: Some("FOR-24".into()),
            title: "Investigate flaky pipeline cache test".into(),
            description: None,
            state: "Triage".into(),
            state_category: WorkItemStateCategory::Triage,
            work_item_type: Some("Bug".into()),
            assignee: Some(bob()),
            created_at: Some(now - chrono::Duration::hours(20)),
            updated_at: Some(now - chrono::Duration::hours(20)),
            url: Some("https://example.test/wi/24".into()),
        },
        WorkItem {
            id: "w9".into(),
            identifier: Some("FOR-25".into()),
            title: "Docs: token scopes table".into(),
            description: None,
            state: "Todo".into(),
            state_category: WorkItemStateCategory::Unstarted,
            work_item_type: Some("Task".into()),
            assignee: Some(carol()),
            created_at: Some(now - chrono::Duration::days(1)),
            updated_at: None,
            url: Some("https://example.test/wi/25".into()),
        },
        WorkItem {
            id: "w10".into(),
            identifier: Some("FOR-26".into()),
            title: "Cross-provider aggregation for PRs".into(),
            description: None,
            state: "Backlog".into(),
            state_category: WorkItemStateCategory::Backlog,
            work_item_type: Some("Epic".into()),
            assignee: Some(alice()),
            created_at: Some(now - chrono::Duration::days(15)),
            updated_at: Some(now - chrono::Duration::days(2)),
            url: Some("https://example.test/wi/26".into()),
        },
        WorkItem {
            id: "w11".into(),
            identifier: Some("FOR-27".into()),
            title: "Jira epic linking".into(),
            description: None,
            state: "In Review".into(),
            state_category: WorkItemStateCategory::Started,
            work_item_type: Some("Story".into()),
            assignee: Some(bob()),
            created_at: Some(now - chrono::Duration::days(4)),
            updated_at: Some(now - chrono::Duration::hours(9)),
            url: Some("https://example.test/wi/27".into()),
        },
    ]
}

fn pipeline_defs() -> Vec<PipelineDefinition> {
    vec![
        PipelineDefinition { id: "ci".into(), name: "CI".into(), path: Some(".github/workflows/ci.yml".into()), url: None },
        PipelineDefinition { id: "release".into(), name: "Release".into(), path: Some(".github/workflows/release.yml".into()), url: None },
    ]
}

fn step(name: &str, status: PipelineRunStatus) -> PipelineStep {
    PipelineStep { name: name.into(), status }
}

fn pipeline_runs() -> Vec<PipelineRun> {
    let now = base();
    vec![
        PipelineRun {
            id: "r501".into(),
            definition_id: "ci".into(),
            number: Some(501),
            name: Some("CI".into()),
            status: PipelineRunStatus::Running,
            triggered_by: Some(alice()),
            branch: Some("feature/retry".into()),
            commit_sha: Some("a1b2c3d".into()),
            started_at: Some(now - chrono::Duration::minutes(4)),
            finished_at: None,
            url: None,
            stages: vec![
                PipelineStage {
                    name: "build".into(),
                    status: PipelineRunStatus::Succeeded,
                    jobs: vec![PipelineJob {
                        id: "j1".into(),
                        name: "compile".into(),
                        status: PipelineRunStatus::Succeeded,
                        started_at: None,
                        finished_at: None,
                        steps: vec![],
                    }],
                },
                PipelineStage {
                    name: "test".into(),
                    status: PipelineRunStatus::Running,
                    jobs: vec![PipelineJob {
                        id: "j2".into(),
                        name: "unit".into(),
                        status: PipelineRunStatus::Running,
                        started_at: None,
                        finished_at: None,
                        steps: vec![step("restore", PipelineRunStatus::Succeeded), step("dotnet test", PipelineRunStatus::Running)],
                    }],
                },
            ],
        },
        PipelineRun {
            id: "r500".into(),
            definition_id: "ci".into(),
            number: Some(500),
            name: Some("CI".into()),
            status: PipelineRunStatus::Failed,
            triggered_by: Some(bob()),
            branch: Some("main".into()),
            commit_sha: Some("9f8e7d6".into()),
            started_at: Some(now - chrono::Duration::hours(1)),
            finished_at: Some(now - chrono::Duration::minutes(52)),
            url: None,
            stages: vec![
                PipelineStage {
                    name: "build".into(),
                    status: PipelineRunStatus::Succeeded,
                    jobs: vec![PipelineJob {
                        id: "j10".into(),
                        name: "compile".into(),
                        status: PipelineRunStatus::Succeeded,
                        started_at: None,
                        finished_at: None,
                        steps: vec![],
                    }],
                },
                PipelineStage {
                    name: "test".into(),
                    status: PipelineRunStatus::Failed,
                    jobs: vec![
                        PipelineJob {
                            id: "j11".into(),
                            name: "unit".into(),
                            status: PipelineRunStatus::Succeeded,
                            started_at: None,
                            finished_at: None,
                            steps: vec![step("restore", PipelineRunStatus::Succeeded), step("run", PipelineRunStatus::Succeeded)],
                        },
                        PipelineJob {
                            id: "j12".into(),
                            name: "integration".into(),
                            status: PipelineRunStatus::Failed,
                            started_at: None,
                            finished_at: None,
                            steps: vec![
                                step("spin up containers", PipelineRunStatus::Succeeded),
                                step("run suite", PipelineRunStatus::Failed),
                            ],
                        },
                    ],
                },
            ],
        },
        PipelineRun {
            id: "r207".into(),
            definition_id: "release".into(),
            number: Some(207),
            name: Some("Release".into()),
            status: PipelineRunStatus::Succeeded,
            triggered_by: Some(carol()),
            branch: Some("main".into()),
            commit_sha: Some("1234abc".into()),
            started_at: Some(now - chrono::Duration::days(2)),
            finished_at: Some(now - chrono::Duration::days(2) + chrono::Duration::minutes(8)),
            url: None,
            stages: vec![PipelineStage {
                name: "publish".into(),
                status: PipelineRunStatus::Succeeded,
                jobs: vec![PipelineJob {
                    id: "j20".into(),
                    name: "deploy".into(),
                    status: PipelineRunStatus::Succeeded,
                    started_at: None,
                    finished_at: None,
                    steps: vec![step("pack", PipelineRunStatus::Succeeded), step("push", PipelineRunStatus::Succeeded)],
                }],
            }],
        },
    ]
}

struct DemoPr;
#[async_trait]
impl PullRequestSource for DemoPr {
    async fn list(&self, query: &PullRequestQuery) -> Result<Vec<PullRequest>> {
        let prs: Vec<_> = pull_requests()
            .into_iter()
            .filter(|p| query.include_completed || matches!(p.status, PullRequestStatus::Open | PullRequestStatus::Draft))
            .collect();
        Ok(apply_pull_request_filter(prs, query.filter, Some("alice")))
    }
    async fn get(&self, id: &str) -> Result<PullRequest> {
        pull_requests().into_iter().find(|p| p.id == id).ok_or_else(|| forgetop_core::Error::NotFound(id.into()))
    }
    async fn threads(&self, _id: &str) -> Result<Vec<CommentThread>> {
        Ok(vec![CommentThread {
            id: "t1".into(),
            comments: vec![Comment {
                id: "c1".into(),
                author: bob(),
                body: "Looks good — one nit on the jitter.".into(),
                created_at: Some(base() - chrono::Duration::hours(2)),
            }],
            file_path: None,
            line: None,
            is_resolved: false,
        }])
    }
    async fn changes(&self, _id: &str) -> Result<Vec<FileChange>> {
        Ok(vec![
            FileChange {
                path: "src/http/retry.rs".into(),
                kind: FileChangeKind::Added,
                additions: 18,
                deletions: 0,
                patch: Some(
                    "@@ -0,0 +1,18 @@\n\
                     +use std::time::Duration;\n\
                     +\n\
                     +/// Retry policy with jittered exponential backoff.\n\
                     +pub struct RetryPolicy {\n\
                     +    pub max_attempts: u32,\n\
                     +    pub base: Duration,\n\
                     +}\n\
                     +\n\
                     +impl RetryPolicy {\n\
                     +    pub fn new(max_attempts: u32) -> Self {\n\
                     +        Self { max_attempts, base: Duration::from_millis(100) }\n\
                     +    }\n\
                     +\n\
                     +    pub fn backoff(&self, attempt: u32) -> Duration {\n\
                     +        let exp = self.base * 2u32.pow(attempt);\n\
                     +        exp + jitter(exp)\n\
                     +    }\n\
                     +}\n"
                        .into(),
                ),
            },
            FileChange {
                path: "src/http/client.rs".into(),
                kind: FileChangeKind::Modified,
                additions: 8,
                deletions: 1,
                patch: Some(
                    "@@ -12,7 +12,9 @@ impl HttpClient {\n\
                     \x20    pub async fn send(&self, req: Request) -> Result<Response> {\n\
                     -        self.inner.execute(req).await\n\
                     +        let policy = RetryPolicy::new(3);\n\
                     +        self.send_with_retry(req, &policy).await\n\
                     \x20    }\n\
                     \x20\n\
                     \x20    fn base_url(&self) -> &str {\n\
                     @@ -40,6 +42,12 @@ impl HttpClient {\n\
                     \x20        &self.base\n\
                     +    }\n\
                     +\n\
                     +    async fn send_with_retry(&self, req: Request, policy: &RetryPolicy) -> Result<Response> {\n\
                     +        // retry loop with jittered backoff\n\
                     +        self.inner.execute(req).await\n\
                     \x20    }\n"
                        .into(),
                ),
            },
        ])
    }
    async fn checks(&self, _id: &str) -> Result<Vec<CheckRun>> {
        Ok(vec![
            CheckRun { name: "build".into(), status: CheckStatus::Passed, url: None },
            CheckRun { name: "unit-tests".into(), status: CheckStatus::Passed, url: None },
            CheckRun { name: "clippy".into(), status: CheckStatus::Passed, url: None },
            CheckRun { name: "integration".into(), status: CheckStatus::Failed, url: None },
            CheckRun { name: "deploy-preview".into(), status: CheckStatus::Pending, url: None },
        ])
    }
    async fn commits(&self, _id: &str) -> Result<Vec<Commit>> {
        Ok(vec![
            Commit { sha: "a1b2c3d".into(), message: "Add RetryPolicy with jittered backoff".into(), author: "alice".into(), date: Some(base()), url: None },
            Commit { sha: "e4f5a6b".into(), message: "Wire retry into the HTTP client".into(), author: "alice".into(), date: Some(base() - chrono::Duration::hours(3)), url: None },
            Commit { sha: "9c8d7e6".into(), message: "Address review: cap max attempts".into(), author: "bob".into(), date: Some(base() - chrono::Duration::hours(1)), url: None },
        ])
    }
    async fn add_comment(&self, _id: &str, _body: &str) -> Result<()> {
        Ok(())
    }
    async fn vote(&self, _id: &str, _vote: ReviewVote) -> Result<()> {
        Ok(())
    }
    async fn merge(&self, _id: &str, _options: &MergeOptions) -> Result<()> {
        Ok(())
    }
}

struct DemoWi;
#[async_trait]
impl WorkItemSource for DemoWi {
    async fn list(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>> {
        Ok(work_items()
            .into_iter()
            .filter(|w| {
                query.include_completed
                    || !matches!(w.state_category, WorkItemStateCategory::Completed | WorkItemStateCategory::Canceled)
            })
            .collect())
    }
    async fn get(&self, id: &str) -> Result<WorkItem> {
        work_items().into_iter().find(|w| w.id == id).ok_or_else(|| forgetop_core::Error::NotFound(id.into()))
    }
    async fn threads(&self, _id: &str) -> Result<Vec<CommentThread>> {
        Ok(vec![])
    }
    async fn set_state(&self, _id: &str, _state: &str) -> Result<()> {
        Ok(())
    }
    async fn add_comment(&self, _id: &str, _body: &str) -> Result<()> {
        Ok(())
    }
}

struct DemoPipe;
#[async_trait]
impl PipelineSource for DemoPipe {
    async fn discover(&self) -> Result<Vec<PipelineDefinition>> {
        Ok(pipeline_defs())
    }
    async fn list_runs(&self, query: &PipelineRunQuery) -> Result<Vec<PipelineRun>> {
        Ok(pipeline_runs()
            .into_iter()
            .filter(|r| query.definition_id.as_ref().is_none_or(|d| &r.definition_id == d))
            .collect())
    }
    async fn get_run(&self, run_id: &str) -> Result<PipelineRun> {
        pipeline_runs().into_iter().find(|r| r.id == run_id).ok_or_else(|| forgetop_core::Error::NotFound(run_id.into()))
    }
    async fn logs(&self, run_id: &str, job_id: Option<&str>) -> Result<String> {
        Ok(format!("[demo] logs for run {run_id}{}\nAll steps completed.", job_id.map(|j| format!(" job {j}")).unwrap_or_default()))
    }
    async fn trigger(&self, _definition_id: &str, _branch: Option<&str>) -> Result<()> {
        Ok(())
    }
}

pub struct DemoConnection {
    id: String,
    display_name: String,
    caps: Capabilities,
}

#[async_trait]
impl ProviderConnection for DemoConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }
    fn provider_type(&self) -> ProviderType {
        ProviderType::Demo
    }
    fn display_name(&self) -> &str {
        &self.display_name
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
        Some(Arc::new(DemoPr))
    }
    fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
        Some(Arc::new(DemoWi))
    }
    fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
        Some(Arc::new(DemoPipe))
    }
    async fn check(&self) -> bool {
        true
    }
}

pub fn demo_capabilities() -> Capabilities {
    Capabilities {
        supports_pull_requests: true,
        supports_work_items: true,
        supports_pipelines: true,
        supports_merge: true,
        supports_inline_comments: true,
        supports_pipeline_trigger: true,
        supports_pipeline_discovery: true,
        ..Default::default()
    }
}

pub struct DemoFactory;

impl ProviderFactory for DemoFactory {
    fn provider_type(&self) -> ProviderType {
        ProviderType::Demo
    }
    fn describe_capabilities(&self) -> Capabilities {
        demo_capabilities()
    }
    fn create(&self, connection: &Connection, _secret: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
        Ok(Arc::new(DemoConnection {
            id: connection.id.clone(),
            display_name: connection.display_name.clone(),
            caps: demo_capabilities(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> DemoConnection {
        DemoConnection { id: "demo".into(), display_name: "Demo".into(), caps: demo_capabilities() }
    }

    #[tokio::test]
    async fn lists_open_prs_and_filters_mine() {
        let src = conn().pull_requests().unwrap();
        let all = src.list(&PullRequestQuery::default()).await.unwrap();
        assert!(all.iter().all(|p| matches!(p.status, PullRequestStatus::Open | PullRequestStatus::Draft)));
        let mine = src.list(&PullRequestQuery { filter: PullRequestFilter::Mine, ..Default::default() }).await.unwrap();
        assert!(mine.iter().all(|p| p.author.handle.as_deref() == Some("alice")));
    }

    #[tokio::test]
    async fn run_has_stages_jobs_steps() {
        let src = conn().pipelines().unwrap();
        let run = src.get_run("r500").await.unwrap();
        let test = run.stages.iter().find(|s| s.name == "test").unwrap();
        let integ = test.jobs.iter().find(|j| j.name == "integration").unwrap();
        assert!(integ.steps.iter().any(|s| matches!(s.status, PipelineRunStatus::Failed)));
    }

    #[tokio::test]
    async fn health_is_true() {
        assert!(conn().check().await);
    }
}
