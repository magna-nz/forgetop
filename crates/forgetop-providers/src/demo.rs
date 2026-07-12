//! Demo provider — canned, deterministic data so `--demo` works with no credentials.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use forgetop_core::domain::*;
use forgetop_core::filter::apply_pull_request_filter;
use forgetop_core::provider::*;
use forgetop_core::Result;

/// A little simulated network latency so `--demo` visibly shows the loading / refresh
/// spinners (the canned data is otherwise instant). Skipped under `cargo test` so the
/// suite stays fast.
async fn demo_latency() {
    if !cfg!(test) {
        tokio::time::sleep(std::time::Duration::from_millis(70)).await;
    }
}

fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 30, 9, 0, 0).unwrap()
}

fn user(id: &str, name: &str, handle: &str) -> User {
    User { id: id.into(), display_name: name.into(), handle: Some(handle.into()), avatar_url: None }
}

/// The current user ("you") — a backend engineer at Northwind. `is_user` matches on
/// handle, so PR filters pass "you".
fn me() -> User {
    user("me", "Sam Rivera", "you")
}
// Teammates (function names kept short; these are the people around you at Northwind).
fn alice() -> User {
    user("u1", "Priya Nair", "priya")
}
fn bob() -> User {
    user("u2", "Marcus Lee", "marcus")
}
fn carol() -> User {
    user("u3", "Elena Sokolova", "elena")
}
fn dev() -> User {
    user("u4", "Tom Becker", "tom")
}

fn rev(u: User, vote: ReviewVote) -> Reviewer {
    Reviewer { user: u, vote, is_required: true }
}

/// Compact PR/MR builder for the demo data.
#[allow(clippy::too_many_arguments)]
fn pr(
    n: i64,
    title: &str,
    author: User,
    status: PullRequestStatus,
    checks: CheckStatus,
    mergeable: MergeableState,
    reviewers: Vec<Reviewer>,
    add: i64,
    del: i64,
    updated_h: i64,
    branch: &str,
    labels: &[&str],
) -> PullRequest {
    let now = base();
    PullRequest {
        id: n.to_string(),
        number: Some(n),
        title: title.into(),
        description: Some(format!(
            "{title}.\n\nReworks the `{branch}` path and adds test coverage. Main things to \
             review: error handling and the retry / edge-case paths. Background and \
             acceptance criteria are on the linked ticket."
        )),
        author,
        is_draft: matches!(status, PullRequestStatus::Draft),
        status,
        source_ref: Some(branch.into()),
        target_ref: Some("main".into()),
        reviewers,
        labels: labels.iter().map(|s| s.to_string()).collect(),
        checks,
        check_summary: None,
        mergeable,
        changed_files: 0,
        additions: add,
        deletions: del,
        created_at: Some(now - chrono::Duration::days(3)),
        updated_at: Some(now - chrono::Duration::hours(updated_h)),
        url: Some(format!("https://example.test/pr/{n}")),
    }
}

/// Compact work-item builder. `mine` assigns it to you; otherwise unassigned.
fn wi(id: &str, title: &str, state: &str, cat: WorkItemStateCategory, ty: &str, mine: bool, updated_h: i64) -> WorkItem {
    let now = base();
    WorkItem {
        id: id.into(),
        identifier: Some(id.into()),
        title: title.into(),
        description: None,
        state: state.into(),
        state_category: cat,
        work_item_type: Some(ty.into()),
        assignee: mine.then(me),
        created_at: Some(now - chrono::Duration::days(4)),
        updated_at: Some(now - chrono::Duration::hours(updated_h)),
        url: Some(format!("https://example.test/issue/{id}")),
    }
}

use CheckStatus as CS;
use MergeableState as MS;
use PullRequestStatus as PS;
use ReviewVote as RV;

/// GitHub — the payments product repos (`northwind/payments`).
fn github_prs() -> Vec<PullRequest> {
    vec![
        // Ready to merge: yours, approved, green.
        pr(1487, "Add idempotency keys to the payments API", me(), PS::Open, CS::Passed, MS::Mergeable, vec![rev(alice(), RV::Approved), rev(bob(), RV::Approved)], 132, 18, 3, "feat/idempotency-keys", &["payments", "api"]),
        // Needs fixing: yours, CI red.
        pr(1492, "Bump Next.js to 14.2.5", me(), PS::Open, CS::Failed, MS::Blocked, vec![rev(bob(), RV::NoVote)], 40, 12, 5, "chore/next-14-2-5", &["frontend", "dependencies"]),
        // Needs your review: a teammate's PR, you're a reviewer.
        pr(1501, "Refactor the webhook retry queue", bob(), PS::Open, CS::Passed, MS::Mergeable, vec![rev(me(), RV::NoVote), rev(alice(), RV::Approved)], 210, 64, 4, "refactor/webhook-retry", &["reliability"]),
        pr(1495, "Tighten CORS on the admin API", carol(), PS::Open, CS::Passed, MS::Mergeable, vec![rev(me(), RV::NoVote)], 18, 6, 9, "security/admin-cors", &["security"]),
        // Draft (yours).
        pr(1476, "Checkout redesign", me(), PS::Draft, CS::Pending, MS::Blocked, vec![], 88, 20, 26, "feat/checkout-redesign", &["frontend", "wip"]),
        // Recently merged (yours).
        pr(1450, "Cache the customer risk score", me(), PS::Merged, CS::Passed, MS::Unknown, vec![rev(alice(), RV::Approved)], 63, 18, 20, "perf/risk-score-cache", &["performance"]),
    ]
}

/// GitHub Issues on the product repos.
fn github_wis() -> Vec<WorkItem> {
    use WorkItemStateCategory as C;
    vec![
        wi("#842", "Investigate elevated p99 on POST /charge", "In Progress", C::Started, "Bug", true, 3),
        wi("#851", "Add retry-budget metrics to the sync worker", "Todo", C::Unstarted, "Task", true, 26),
        // Unassigned (not yours) — the mine-only filter drops it.
        wi("#860", "Flaky test: webhook_delivery_spec", "Backlog", C::Backlog, "Bug", false, 30),
    ]
}

/// GitLab — the platform / infra group (`northwind-infra`). Merge Requests.
fn gitlab_prs() -> Vec<PullRequest> {
    vec![
        // Yours, open, waiting on review (no action → your open PRs).
        pr(312, "Terraform: add a Postgres read replica", me(), PS::Open, CS::Passed, MS::Mergeable, vec![rev(alice(), RV::NoVote)], 96, 4, 6, "infra/read-replica", &["terraform"]),
        // A teammate's, you're the reviewer → needs your review.
        pr(318, "Rotate the KMS signing keys", alice(), PS::Open, CS::Passed, MS::Mergeable, vec![rev(me(), RV::NoVote)], 22, 8, 10, "security/kms-rotation", &["security"]),
        // Yours, merged recently.
        pr(305, "Bump the base image to alpine 3.20", me(), PS::Merged, CS::Passed, MS::Unknown, vec![rev(bob(), RV::Approved)], 6, 6, 40, "chore/alpine-3-20", &["docker"]),
    ]
}

fn gitlab_wis() -> Vec<WorkItem> {
    use WorkItemStateCategory as C;
    vec![wi("#77", "Right-size the staging cluster", "In Progress", C::Started, "Task", true, 5)]
}

/// Bitbucket — the data team's dbt/ingestion repo (`northwind-data`).
fn bitbucket_prs() -> Vec<PullRequest> {
    vec![
        // Yours, changes requested → needs fixing.
        pr(64, "dbt: add revenue recognition model", me(), PS::Open, CS::Passed, MS::Blocked, vec![rev(dev(), RV::Rejected)], 180, 12, 7, "feat/rev-rec", &["dbt"]),
        // A teammate's, you're the reviewer → needs your review.
        pr(61, "Fix the nightly ingestion retry", dev(), PS::Open, CS::Passed, MS::Mergeable, vec![rev(me(), RV::NoVote)], 34, 10, 12, "fix/ingestion-retry", &["airflow"]),
    ]
}

/// Linear — the Engineering team's tickets, all assigned to you.
fn linear_wis() -> Vec<WorkItem> {
    use WorkItemStateCategory as C;
    vec![
        wi("ENG-231", "Design the ledger reconciliation job", "In Progress", C::Started, "Story", true, 4),
        wi("ENG-245", "Spike: event sourcing for the payments ledger", "Todo", C::Unstarted, "Spike", true, 28),
        wi("ENG-250", "Add SLO dashboards for the charge API", "Backlog", C::Backlog, "Task", true, 50),
        wi("ENG-198", "Migrate feature flags to OpenFeature", "Blocked", C::Started, "Story", true, 18),
    ]
}

/// Jira — company ops / security tickets, all assigned to you.
fn jira_wis() -> Vec<WorkItem> {
    use WorkItemStateCategory as C;
    vec![
        wi("OPS-1423", "SOC2: collect access-review evidence for Q3", "In Progress", C::Started, "Task", true, 2),
        wi("SEC-88", "INC-4821 postmortem action items", "To Do", C::Unstarted, "Bug", true, 22),
        wi("OPS-1440", "Upgrade Vault to 1.16", "Backlog", C::Backlog, "Task", true, 60),
    ]
}

fn prs_for(conn: &str) -> Vec<PullRequest> {
    match conn {
        "gitlab" => gitlab_prs(),
        "bitbucket" => bitbucket_prs(),
        _ => github_prs(),
    }
}

fn wis_for(conn: &str) -> Vec<WorkItem> {
    match conn {
        "gitlab" => gitlab_wis(),
        "linear" => linear_wis(),
        "jira" => jira_wis(),
        _ => github_wis(),
    }
}

fn pipeline_defs() -> Vec<PipelineDefinition> {
    vec![
        PipelineDefinition { id: "ci".into(), name: "CI Build".into(), path: Some(".github/workflows/ci.yml".into()), url: None },
        PipelineDefinition { id: "release".into(), name: "CD (Release)".into(), path: Some(".github/workflows/release.yml".into()), url: None },
    ]
}

fn step(name: &str, status: PipelineRunStatus, secs: i64) -> PipelineStep {
    let now = base();
    PipelineStep {
        name: name.into(),
        status,
        started_at: Some(now - chrono::Duration::seconds(secs)),
        finished_at: Some(now),
    }
}

fn job(id: &str, name: &str, status: PipelineRunStatus, secs: i64, steps: Vec<PipelineStep>, problem: Option<&str>) -> PipelineJob {
    let now = base();
    PipelineJob {
        id: id.into(),
        name: name.into(),
        status,
        started_at: Some(now - chrono::Duration::seconds(secs)),
        finished_at: if matches!(status, PipelineRunStatus::Running) { None } else { Some(now) },
        steps,
        url: Some(format!("https://example.test/job/{id}")),
        problem: problem.map(Into::into),
    }
}

/// Compact (stage-less) run builder for the secondary CI providers.
#[allow(clippy::too_many_arguments)]
fn run(id: &str, def: &str, num: i64, name: &str, status: PipelineRunStatus, branch: &str, who: User, updated_h: i64) -> PipelineRun {
    let now = base();
    let started = now - chrono::Duration::hours(updated_h);
    PipelineRun {
        id: id.into(),
        definition_id: def.into(),
        number: Some(num),
        name: Some(name.into()),
        status,
        triggered_by: Some(who),
        branch: Some(branch.into()),
        commit_sha: Some("abc1234".into()),
        started_at: Some(started),
        finished_at: matches!(status, PipelineRunStatus::Running | PipelineRunStatus::Queued).then(|| started + chrono::Duration::minutes(6)),
        url: None,
        stages: vec![],
    }
}

fn gitlab_pipeline_defs() -> Vec<PipelineDefinition> {
    vec![PipelineDefinition { id: "gl-pipeline".into(), name: "pipeline".into(), path: Some(".gitlab-ci.yml".into()), url: None }]
}
fn gitlab_runs() -> Vec<PipelineRun> {
    vec![
        run("gl-9902", "gl-pipeline", 9902, "#9902", PipelineRunStatus::Running, "infra/read-replica", me(), 1),
        run("gl-9901", "gl-pipeline", 9901, "#9901", PipelineRunStatus::Succeeded, "main", alice(), 6),
    ]
}
fn bitbucket_pipeline_defs() -> Vec<PipelineDefinition> {
    vec![PipelineDefinition { id: "bb-default".into(), name: "default".into(), path: Some("bitbucket-pipelines.yml".into()), url: None }]
}
fn bitbucket_runs() -> Vec<PipelineRun> {
    vec![
        run("bb-441", "bb-default", 441, "#441", PipelineRunStatus::Failed, "feat/rev-rec", me(), 2),
        run("bb-440", "bb-default", 440, "#440", PipelineRunStatus::Succeeded, "main", dev(), 10),
    ]
}
fn pipeline_defs_for(conn: &str) -> Vec<PipelineDefinition> {
    match conn {
        "gitlab" => gitlab_pipeline_defs(),
        "bitbucket" => bitbucket_pipeline_defs(),
        _ => pipeline_defs(),
    }
}
fn pipeline_runs_for(conn: &str) -> Vec<PipelineRun> {
    match conn {
        "gitlab" => gitlab_runs(),
        "bitbucket" => bitbucket_runs(),
        _ => pipeline_runs(),
    }
}

fn pipeline_runs() -> Vec<PipelineRun> {
    let now = base();
    vec![
        PipelineRun {
            id: "r501".into(),
            definition_id: "ci".into(),
            number: Some(501),
            name: Some("10.1.100".into()),
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
                    jobs: vec![job("j1", "compile", PipelineRunStatus::Succeeded, 95, vec![], None)],
                },
                PipelineStage {
                    name: "test".into(),
                    status: PipelineRunStatus::Running,
                    jobs: vec![job(
                        "j2",
                        "unit",
                        PipelineRunStatus::Running,
                        140,
                        vec![step("restore", PipelineRunStatus::Succeeded, 12), step("dotnet test", PipelineRunStatus::Running, 128)],
                        None,
                    )],
                },
            ],
        },
        PipelineRun {
            id: "r500".into(),
            definition_id: "ci".into(),
            number: Some(500),
            name: Some("10.1.99".into()),
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
                    jobs: vec![job("j10", "compile", PipelineRunStatus::Succeeded, 88, vec![], None)],
                },
                PipelineStage {
                    name: "test".into(),
                    status: PipelineRunStatus::Failed,
                    jobs: vec![
                        job(
                            "j11",
                            "unit",
                            PipelineRunStatus::Succeeded,
                            64,
                            vec![step("restore", PipelineRunStatus::Succeeded, 11), step("run", PipelineRunStatus::Succeeded, 53)],
                            None,
                        ),
                        job(
                            "j12",
                            "integration",
                            PipelineRunStatus::Failed,
                            240,
                            vec![
                                step("spin up containers", PipelineRunStatus::Succeeded, 30),
                                step("run suite", PipelineRunStatus::Failed, 210),
                            ],
                            Some("run suite failed (exit 1)"),
                        ),
                    ],
                },
            ],
        },
        PipelineRun {
            id: "r207".into(),
            definition_id: "release".into(),
            number: Some(207),
            name: Some("10.1.98".into()),
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
                jobs: vec![job(
                    "j20",
                    "deploy",
                    PipelineRunStatus::Succeeded,
                    150,
                    vec![step("pack", PipelineRunStatus::Succeeded, 40), step("push", PipelineRunStatus::Succeeded, 110)],
                    None,
                )],
            }],
        },
        PipelineRun {
            id: "r502".into(),
            definition_id: "ci".into(),
            number: Some(502),
            name: Some("10.1.101".into()),
            status: PipelineRunStatus::Queued,
            triggered_by: Some(alice()),
            branch: Some("main".into()),
            commit_sha: Some("cafe123".into()),
            started_at: None,
            finished_at: None,
            url: None,
            stages: vec![],
        },
    ]
}

/// Session-global store of review comments submitted during this `--demo` run, keyed by PR
/// id. It lets the demo emulate a real provider: a comment you submit persists and comes
/// back from `threads()` (even after reopening the PR), instead of vanishing.
fn submitted_threads() -> &'static Mutex<HashMap<String, Vec<CommentThread>>> {
    static STORE: OnceLock<Mutex<HashMap<String, Vec<CommentThread>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

struct DemoPr {
    conn: String,
}
#[async_trait]
impl PullRequestSource for DemoPr {
    async fn list(&self, query: &PullRequestQuery) -> Result<Vec<PullRequest>> {
        demo_latency().await;
        let prs: Vec<_> = prs_for(&self.conn)
            .into_iter()
            .filter(|p| query.include_completed || matches!(p.status, PullRequestStatus::Open | PullRequestStatus::Draft))
            .collect();
        Ok(apply_pull_request_filter(prs, query.filter, Some("you")))
    }
    async fn get(&self, id: &str) -> Result<PullRequest> {
        prs_for(&self.conn).into_iter().find(|p| p.id == id).ok_or_else(|| forgetop_core::Error::NotFound(id.into()))
    }
    async fn threads(&self, id: &str) -> Result<Vec<CommentThread>> {
        let mut threads = vec![
            // Anchored to a diff line so it renders inline and `]`/`[` can jump to it.
            CommentThread {
                id: "t1".into(),
                comments: vec![Comment {
                    id: "c1".into(),
                    author: bob(),
                    body: "One nit on the jitter — cap the backoff so it can't grow unbounded.".into(),
                    created_at: Some(base() - chrono::Duration::hours(2)),
                }],
                file_path: Some("src/http/retry.rs".into()),
                line: Some(15),
                is_resolved: false,
            },
            // A resolved thread on the other file.
            CommentThread {
                id: "t2".into(),
                comments: vec![Comment {
                    id: "c2".into(),
                    author: carol(),
                    body: "Good call reusing the policy here.".into(),
                    created_at: Some(base() - chrono::Duration::hours(3)),
                }],
                file_path: Some("src/http/client.rs".into()),
                line: Some(13),
                is_resolved: true,
            },
        ];
        // Include anything submitted this session so it persists like a real provider.
        if let Some(extra) = submitted_threads().lock().unwrap().get(id) {
            threads.extend(extra.iter().cloned());
        }
        Ok(threads)
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
    async fn commit_changes(&self, _id: &str, sha: &str) -> Result<Vec<FileChange>> {
        // Canned per-commit diff so drilling into each commit shows distinct changes.
        let file = match sha {
            "a1b2c3d" => FileChange {
                path: "src/http/retry.rs".into(),
                kind: FileChangeKind::Added,
                additions: 6,
                deletions: 0,
                patch: Some(
                    "@@ -0,0 +1,6 @@\n\
                     +/// Retry policy with jittered exponential backoff.\n\
                     +pub struct RetryPolicy {\n\
                     +    pub max_attempts: u32,\n\
                     +    pub base: Duration,\n\
                     +}\n\
                     +\n"
                        .into(),
                ),
            },
            "e4f5a6b" => FileChange {
                path: "src/http/client.rs".into(),
                kind: FileChangeKind::Modified,
                additions: 2,
                deletions: 1,
                patch: Some(
                    "@@ -12,3 +12,4 @@ impl HttpClient {\n\
                     \x20    pub async fn send(&self, req: Request) -> Result<Response> {\n\
                     -        self.inner.execute(req).await\n\
                     +        let policy = RetryPolicy::new(3);\n\
                     +        self.send_with_retry(req, &policy).await\n\
                     \x20    }\n"
                        .into(),
                ),
            },
            _ => FileChange {
                path: "src/http/retry.rs".into(),
                kind: FileChangeKind::Modified,
                additions: 1,
                deletions: 1,
                patch: Some(
                    "@@ -2,3 +2,3 @@ pub struct RetryPolicy {\n\
                     \x20pub struct RetryPolicy {\n\
                     -    pub max_attempts: u32, // unbounded\n\
                     +    pub max_attempts: u32, // capped at 3\n\
                     \x20    pub base: Duration,\n"
                        .into(),
                ),
            },
        };
        Ok(vec![file])
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
    async fn submit_review(&self, id: &str, _event: ReviewVote, comments: &[LineComment]) -> Result<()> {
        // Persist each line comment as an open thread by "you", so it comes back from
        // threads() and the diff shows it exactly as a real provider would.
        let mut store = submitted_threads().lock().unwrap();
        let entry = store.entry(id.to_string()).or_default();
        for c in comments {
            let n = entry.len();
            entry.push(CommentThread {
                id: format!("submitted-{n}"),
                comments: vec![Comment { id: format!("mine-{n}"), author: me(), body: c.body.clone(), created_at: Some(base()) }],
                file_path: Some(c.path.clone()),
                line: Some(c.line),
                is_resolved: false,
            });
        }
        Ok(())
    }
}

struct DemoWi {
    conn: String,
}
#[async_trait]
impl WorkItemSource for DemoWi {
    async fn list(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>> {
        demo_latency().await;
        // The demo's "me" is Alice (u1); mine_only keeps only her items.
        Ok(wis_for(&self.conn)
            .into_iter()
            .filter(|w| {
                query.include_completed
                    || !matches!(w.state_category, WorkItemStateCategory::Completed | WorkItemStateCategory::Canceled)
            })
            .filter(|w| !query.mine_only || w.assignee.as_ref().map(|u| u.id == "me").unwrap_or(false))
            .collect())
    }
    async fn get(&self, id: &str) -> Result<WorkItem> {
        wis_for(&self.conn).into_iter().find(|w| w.id == id).ok_or_else(|| forgetop_core::Error::NotFound(id.into()))
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
    async fn available_states(&self, _id: &str) -> Result<Vec<String>> {
        Ok(["Backlog", "Todo", "In Progress", "In Review", "Blocked", "Done"].iter().map(|s| s.to_string()).collect())
    }
}

struct DemoPipe {
    conn: String,
}
#[async_trait]
impl PipelineSource for DemoPipe {
    async fn discover(&self) -> Result<Vec<PipelineDefinition>> {
        Ok(pipeline_defs_for(&self.conn))
    }
    async fn list_runs(&self, query: &PipelineRunQuery) -> Result<Vec<PipelineRun>> {
        demo_latency().await;
        Ok(pipeline_runs_for(&self.conn)
            .into_iter()
            .filter(|r| query.definition_id.as_ref().is_none_or(|d| &r.definition_id == d))
            .collect())
    }
    async fn get_run(&self, run_id: &str) -> Result<PipelineRun> {
        pipeline_runs_for(&self.conn).into_iter().find(|r| r.id == run_id).ok_or_else(|| forgetop_core::Error::NotFound(run_id.into()))
    }
    async fn logs(&self, run_id: &str, job_id: Option<&str>) -> Result<String> {
        let job = job_id.unwrap_or("job");
        let mut out = format!("=== logs for run {run_id} · {job} ===\n");
        for i in 1..=24 {
            out.push_str(&format!("[00:{i:02}] step output line {i}\n"));
        }
        if job == "j12" {
            out.push_str("ERROR: integration suite failed: 2 tests failed\n");
            out.push_str("  - test_checkout_flow\n  - test_refund\n");
            out.push_str("Process exited with code 1\n");
        } else {
            out.push_str("Done. All steps completed successfully.\n");
        }
        Ok(out)
    }
    async fn trigger(&self, _definition_id: &str, _branch: Option<&str>) -> Result<()> {
        Ok(())
    }
    fn supports_approvals(&self) -> bool {
        true
    }
    async fn pending_approvals(&self, run_id: &str) -> Result<Vec<PipelineApproval>> {
        // The running CI run (#501) waits on a production deployment gate you can act on.
        Ok(if run_id == "r501" {
            vec![PipelineApproval { id: "production".into(), name: "production".into(), can_respond: true }]
        } else {
            Vec::new()
        })
    }
    async fn respond_approval(&self, _run_id: &str, _approval_id: &str, _decision: ApprovalDecision, _comment: Option<&str>) -> Result<()> {
        Ok(())
    }
}

pub struct DemoConnection {
    id: String,
    display_name: String,
    provider: ProviderType,
    caps: Capabilities,
}

#[async_trait]
impl ProviderConnection for DemoConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }
    fn provider_type(&self) -> ProviderType {
        self.provider
    }
    fn display_name(&self) -> &str {
        &self.display_name
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
        self.caps.supports_pull_requests.then(|| Arc::new(DemoPr { conn: self.id.clone() }) as Arc<dyn PullRequestSource>)
    }
    fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
        self.caps.supports_work_items.then(|| Arc::new(DemoWi { conn: self.id.clone() }) as Arc<dyn WorkItemSource>)
    }
    fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
        self.caps.supports_pipelines.then(|| Arc::new(DemoPipe { conn: self.id.clone() }) as Arc<dyn PipelineSource>)
    }
    async fn check(&self) -> bool {
        true
    }
}

/// Capabilities for a demo connection — mirrors the real provider so the UI gates
/// sections and labels (MRs vs PRs, Issues) exactly as it would live.
pub fn demo_capabilities(provider: ProviderType) -> Capabilities {
    match provider {
        ProviderType::GitLab => crate::gitlab::gitlab_capabilities(),
        ProviderType::Bitbucket => crate::bitbucket::bitbucket_capabilities(),
        ProviderType::Linear => crate::linear::linear_capabilities(),
        ProviderType::Jira => crate::jira::jira_capabilities(),
        _ => crate::github::github_capabilities(),
    }
}

pub struct DemoFactory {
    provider: ProviderType,
}

impl ProviderFactory for DemoFactory {
    fn provider_type(&self) -> ProviderType {
        self.provider
    }
    fn describe_capabilities(&self) -> Capabilities {
        demo_capabilities(self.provider)
    }
    fn create(&self, connection: &Connection, _secret: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
        Ok(Arc::new(DemoConnection {
            id: connection.id.clone(),
            display_name: connection.display_name.clone(),
            provider: self.provider,
            caps: demo_capabilities(self.provider),
        }))
    }
}

/// One demo factory per real provider type, so `--demo` connections report their real
/// provider (and the Provider column reads correctly) while serving canned data.
pub fn demo_factories() -> Vec<Arc<dyn ProviderFactory>> {
    [ProviderType::GitHub, ProviderType::GitLab, ProviderType::Linear, ProviderType::Bitbucket, ProviderType::Jira]
        .into_iter()
        .map(|p| Arc::new(DemoFactory { provider: p }) as Arc<dyn ProviderFactory>)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> DemoConnection {
        DemoConnection {
            id: "github".into(),
            display_name: "GitHub".into(),
            provider: ProviderType::GitHub,
            caps: demo_capabilities(ProviderType::GitHub),
        }
    }

    #[tokio::test]
    async fn submitted_review_comments_persist_in_threads() {
        let src = DemoPr { conn: "github".into() };
        // Unique ids so the session-global store can't collide with other tests.
        let (pr, other) = ("persist-pr-a", "persist-pr-b");
        let before = src.threads(pr).await.unwrap().len();

        src.submit_review(
            pr,
            ReviewVote::NoVote,
            &[LineComment { path: "src/http/retry.rs".into(), line: 7, side: DiffSide::New, body: "please add a test".into() }],
        )
        .await
        .unwrap();

        let after = src.threads(pr).await.unwrap();
        assert_eq!(after.len(), before + 1, "the submitted comment persists as a new thread");
        assert!(
            after.iter().any(|t| t.comments.iter().any(|c| c.body == "please add a test")),
            "the submitted body comes back from threads()"
        );
        // A different PR is unaffected by what was submitted to this one.
        assert_eq!(src.threads(other).await.unwrap().len(), before);
    }

    #[tokio::test]
    async fn lists_open_prs_and_filters_mine() {
        let src = conn().pull_requests().unwrap();
        let all = src.list(&PullRequestQuery::default()).await.unwrap();
        assert!(all.iter().all(|p| matches!(p.status, PullRequestStatus::Open | PullRequestStatus::Draft)));
        let mine = src.list(&PullRequestQuery { filter: PullRequestFilter::Mine, ..Default::default() }).await.unwrap();
        assert!(mine.iter().all(|p| p.author.handle.as_deref() == Some("you")));
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

    fn demo_conn(id: &str, p: ProviderType) -> DemoConnection {
        DemoConnection { id: id.into(), display_name: p.as_str().into(), provider: p, caps: demo_capabilities(p) }
    }

    #[test]
    fn connections_report_their_real_provider_and_gate_sections() {
        // Each demo connection reports its real provider type (for the Provider column)…
        let gh = demo_conn("github", ProviderType::GitHub);
        assert_eq!(gh.provider_type(), ProviderType::GitHub);
        assert!(gh.pull_requests().is_some() && gh.work_items().is_some() && gh.pipelines().is_some());

        // …and only offers what that provider really supports.
        let linear = demo_conn("linear", ProviderType::Linear);
        assert!(linear.work_items().is_some() && linear.pull_requests().is_none() && linear.pipelines().is_none());

        let bb = demo_conn("bitbucket", ProviderType::Bitbucket);
        assert!(bb.pull_requests().is_some() && bb.pipelines().is_some() && bb.work_items().is_none());

        // Five factories, one per real provider — none report "Demo".
        let providers: Vec<ProviderType> = demo_factories().iter().map(|f| f.provider_type()).collect();
        assert_eq!(providers.len(), 5);
        assert!(!providers.contains(&ProviderType::Demo));
    }

    #[tokio::test]
    async fn work_items_expose_available_states() {
        let src = conn().work_items().unwrap();
        let states = src.available_states("w1").await.unwrap();
        assert!(states.contains(&"In Progress".to_string()) && states.contains(&"Done".to_string()));
        assert!(states.len() >= 4, "a meaningful set of states to pick from");
    }

    #[tokio::test]
    async fn work_items_mine_only_filters_to_the_current_user() {
        let src = conn().work_items().unwrap();
        let all = src.list(&WorkItemQuery { mine_only: false, include_completed: false, limit: None }).await.unwrap();
        let mine = src.list(&WorkItemQuery { mine_only: true, include_completed: false, limit: None }).await.unwrap();
        assert!(!mine.is_empty() && mine.len() < all.len(), "mine-only narrows the list");
        assert!(
            mine.iter().all(|w| w.assignee.as_ref().map(|u| u.id == "me").unwrap_or(false)),
            "only Alice's items remain"
        );
    }
}
