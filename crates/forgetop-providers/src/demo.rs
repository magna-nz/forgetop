//! Demo provider — canned, deterministic data so `--demo` works with no credentials.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
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

/// "Now" for the demo, so all the canned timestamps read as fresh relative to today
/// (ages in hours/days, and the "recently merged" window actually catches recent merges)
/// instead of drifting stale against a hard-coded date.
fn base() -> DateTime<Utc> {
    Utc::now()
}

fn user(id: &str, name: &str, handle: &str) -> User {
    User { id: id.into(), display_name: name.into(), handle: Some(handle.into()), avatar_url: None }
}

/// The current user ("you") — a backend engineer at Northwind. `is_user` matches on
/// handle, so PR filters pass "you".
fn me() -> User {
    user("me", DEMO_ME_NAME, DEMO_ME)
}

/// The handle [`me`] is reachable by, named once so `list` and `current_user` cannot drift apart
/// — a mismatch between them would make the demo's locally-derived "Mine" view silently empty.
const DEMO_ME: &str = "you";
const DEMO_ME_NAME: &str = "Sam Rivera";
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

/// A believable body for each demo PR (keyed by number), so the Conversation tab reads
/// like a real review. Unknown numbers fall back to a short synthesized note.
fn pr_description(n: i64, title: &str, branch: &str) -> String {
    let bespoke = match n {
        1487 => "Adds client-supplied `Idempotency-Key` support to `POST /charge` and `POST /refund` so a retried request can't double-charge.\n\n- Keys are stored in Redis with a 24h TTL; a replay returns the original response.\n- A conflicting body on the same key returns 422.\n\nCloses PAY-1187.",
        1492 => "Routine bump to pull in the security fixes in the Next.js 14.2.x line.\n\n- Regenerated the lockfile; no app code changes.\n- CI is red on the visual-regression suite — checking whether it's a real diff or just stale snapshots.",
        1501 => "Reworks webhook delivery to use a jittered exponential backoff instead of a fixed 30s interval, and moves retries onto a dedicated queue so one slow endpoint can't starve first-delivery.\n\n- New `RetryPolicy` with a capped backoff.\n- Dead-letters after 12 attempts.\n\nReview focus: the backoff maths and the dead-letter cutoff.",
        1495 => "Restricts the admin API CORS allow-list to the internal dashboard origins (it was effectively `*`).\n\n- Explicit origin allow-list read from config.\n- Credentialed cross-origin requests are rejected otherwise.\n\nCloses SEC-73.",
        1476 => "**Draft — please don't review yet.** First cut of the new single-page checkout.\n\n- New card + wallet layout.\n- Address validation is still stubbed and tests are missing.\n\nOpening early for directional feedback on the component split.",
        1450 => "Caches the computed customer risk score for 5 minutes to take it off the hot charge path.\n\n- Read-through cache keyed by customer id.\n- Invalidated on any KYC or limit change.\n\nCut charge-path p99 by ~40% in staging.",
        312 => "Adds a managed Postgres read replica in `us-east-1` and routes read-heavy reporting queries to it.\n\n- New replica instance + parameter group.\n- Read endpoint wired into the analytics config.\n\nTerraform plan output is on the ticket.",
        318 => "Scheduled rotation of the JWT signing keys in KMS.\n\n- Adds the new key version and keeps the previous one active for verification during the overlap window.\n- Retire the old key after 7 days (runbook step included).",
        305 => "Bumps the service base image to Alpine 3.20 for the latest CVE patches.\n\n- No application changes; rebuilt and smoke-tested.\n- Image is ~6MB smaller.",
        64 => "Adds a dbt model for ASC 606 revenue recognition off the invoices + subscriptions sources.\n\n- New `rev_rec_monthly` model with tests.\n- Blocked: waiting on finance to confirm the deferral schedule (see the review thread).",
        61 => "Fixes the nightly ingestion DAG silently swallowing a failed task, which left downstream tables stale.\n\n- Retries with backoff, then fails loudly and pages on exhaustion.\n- Backfills the two missed partitions.",
        _ => "",
    };
    if bespoke.is_empty() {
        format!("{title}.\n\nReworks the `{branch}` path and adds test coverage. Please review the error handling and edge-case paths; background is on the linked ticket.")
    } else {
        bespoke.to_string()
    }
}

/// A believable body for each demo work item (keyed by identifier).
fn wi_description(id: &str, title: &str) -> String {
    let bespoke = match id {
        "#842" => "p99 on `POST /charge` has crept from ~180ms to ~600ms over the last week.\n\n- Prime suspect is the new risk-score lookup on the hot path.\n- Next: trace a slow request end-to-end and confirm whether the cache is actually being hit.",
        "#851" => "We have no visibility into how much of its retry budget the sync worker burns before giving up.\n\n- Emit `retries_used` / `retry_budget` counters.\n- Add a panel and alert when a worker consistently exhausts its budget.",
        "#860" => "`webhook_delivery_spec` fails roughly 1 in 10 CI runs, almost always on the ordering assertion.\n\n- Looks like a timing assumption on async delivery.\n- Fix the ordering expectation or quarantine the test until it's stable.",
        "#77" => "Staging is running prod-sized node pools and costing more than it should.\n\n- Move to smaller instances and enable scale-to-zero overnight.\n- Confirm nothing relies on the current headroom first.",
        "ENG-231" => "Design a daily job that reconciles the payments ledger against the processor settlement report and flags mismatches.\n\n- Define the matching keys and an acceptable tolerance.\n- Decide where discrepancies surface (dashboard vs alert).\n\nDeliverable: a short design doc before we implement.",
        "ENG-245" => "Timeboxed 3-day spike to evaluate event sourcing for the payments ledger.\n\n- Prototype append-only events plus a projection.\n- Assess replay cost and operational complexity.\n\nOutcome is a recommendation, not production code.",
        "ENG-250" => "Stand up SLO dashboards for the charge API.\n\n- Availability and latency SLOs with error budgets.\n- Wire up burn-rate alerts.\n\nUse the existing Grafana / Prometheus stack.",
        "ENG-198" => "Migrate our bespoke feature-flag client to the OpenFeature SDK.\n\n- Wrap the current provider behind the OpenFeature API.\n- Migrate call sites incrementally.\n\nBlocked on the platform team publishing the shared provider.",
        "OPS-1423" => "Collect Q3 access-review evidence for the SOC2 audit.\n\n- Export access lists for the production systems.\n- Get sign-off from each system owner.\n\nDue before the auditor's window closes.",
        "SEC-88" => "Track the action items from the INC-4821 postmortem (the webhook outage).\n\n- Add alerting on delivery lag.\n- Cap the retry backoff.\n- Document the manual drain runbook.",
        "OPS-1440" => "Upgrade the Vault cluster from 1.14 to 1.16.\n\n- Review the breaking changes and the storage migration.\n- Roll nodes one at a time, verifying unseal + auth after each.\n\nSchedule inside a maintenance window.",
        _ => "",
    };
    if bespoke.is_empty() {
        format!("{title}.\n\nSee the linked ticket for background and acceptance criteria.")
    } else {
        bespoke.to_string()
    }
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
        repository: None,
        id: n.to_string(),
        number: Some(n),
        title: title.into(),
        description: Some(pr_description(n, title, branch)),
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

/// Sets when a demo PR was opened, so the review requests span the Command Center's age colours
/// (grey inside the 24h review SLA, yellow past it, red past three times it).
fn opened(mut pr: PullRequest, hours_ago: i64) -> PullRequest {
    pr.created_at = Some(base() - chrono::Duration::hours(hours_ago));
    pr
}

/// Compact work-item builder. `mine` assigns it to you; otherwise unassigned.
fn wi(id: &str, title: &str, state: &str, cat: WorkItemStateCategory, ty: &str, mine: bool, updated_h: i64) -> WorkItem {
    let now = base();
    WorkItem {
        repository: None,
        id: id.into(),
        identifier: Some(id.into()),
        title: title.into(),
        description: Some(wi_description(id, title)),
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
        opened(pr(1501, "Refactor the webhook retry queue", bob(), PS::Open, CS::Passed, MS::Mergeable, vec![rev(alice(), RV::Approved), rev(carol(), RV::Rejected), rev(me(), RV::NoVote)], 210, 64, 4, "refactor/webhook-retry", &["reliability"]), 18),
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
        opened(pr(318, "Rotate the KMS signing keys", alice(), PS::Open, CS::Passed, MS::Mergeable, vec![rev(me(), RV::NoVote)], 22, 8, 10, "security/kms-rotation", &["security"]), 30),
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
        opened(pr(61, "Fix the nightly ingestion retry", dev(), PS::Open, CS::Passed, MS::Mergeable, vec![rev(me(), RV::NoVote)], 34, 10, 12, "fix/ingestion-retry", &["airflow"]), 26),
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

/// The repository a demo connection's items live in, **connection-relative**.
///
/// The demo provider is deliberately exempt from the repository scope — it stays single-repo —
/// but it still stamps a real repository on everything it returns, so any deep-link or
/// hard-refresh path carries an address rather than a `None`. It never *validates* one, so a
/// fixture connection can't fail addressing. (Linear and Jira aren't repo-addressed at all.)
fn demo_repository(conn: &str) -> Option<String> {
    match conn {
        "gitlab" => Some("northwind/platform".into()),
        "bitbucket" => Some("northwind/mobile".into()),
        "linear" | "jira" => None,
        _ => Some("northwind/payments".into()),
    }
}

fn prs_for(conn: &str) -> Vec<PullRequest> {
    let repo = demo_repository(conn);
    let prs = match conn {
        "gitlab" => gitlab_prs(),
        "bitbucket" => bitbucket_prs(),
        _ => github_prs(),
    };
    prs.into_iter().map(|mut pr| { pr.repository = repo.clone(); pr }).collect()
}

fn wis_for(conn: &str) -> Vec<WorkItem> {
    let repo = demo_repository(conn);
    let wis = match conn {
        "gitlab" => gitlab_wis(),
        "linear" => linear_wis(),
        "jira" => jira_wis(),
        _ => github_wis(),
    };
    wis.into_iter().map(|mut w| { w.repository = repo.clone(); w }).collect()
}

fn pipeline_defs() -> Vec<PipelineDefinition> {
    vec![
        PipelineDefinition { repository: None, id: "ci".into(), name: "CI Build".into(), path: Some(".github/workflows/ci.yml".into()), url: None },
        PipelineDefinition { repository: None, id: "release".into(), name: "CD (Release)".into(), path: Some(".github/workflows/release.yml".into()), url: None },
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
fn run(id: &str, def: &str, num: i64, name: &str, title: &str, status: PipelineRunStatus, branch: &str, who: User, updated_h: i64) -> PipelineRun {
    let now = base();
    let started = now - chrono::Duration::hours(updated_h);
    PipelineRun {
        event: None,
        attempt: None,
        pull_request: None,
        repository: None,
        id: id.into(),
        definition_id: def.into(),
        number: Some(num),
        name: Some(name.into()),
        title: Some(title.into()),
        status,
        triggered_by: Some(who),
        branch: Some(branch.into()),
        commit_sha: Some("abc1234".into()),
        started_at: Some(started),
        finished_at: matches!(status, PipelineRunStatus::Running | PipelineRunStatus::Queued).then(|| started + chrono::Duration::minutes(6)),
        url: Some("https://ci.example.com/runs/demo".into()),
        stages: vec![],
    }
}

fn gitlab_pipeline_defs() -> Vec<PipelineDefinition> {
    vec![PipelineDefinition { repository: None, id: "gl-pipeline".into(), name: "Integration Suite".into(), path: Some(".gitlab-ci.yml".into()), url: None }]
}
fn gitlab_runs() -> Vec<PipelineRun> {
    vec![
        run("gl-9902", "gl-pipeline", 9902, "#9902", "Add a Postgres read replica", PipelineRunStatus::Running, "infra/read-replica", me(), 1),
        run("gl-9901", "gl-pipeline", 9901, "#9901", "Cache the customer risk score", PipelineRunStatus::Succeeded, "main", alice(), 6),
    ]
}
fn bitbucket_pipeline_defs() -> Vec<PipelineDefinition> {
    vec![PipelineDefinition { repository: None, id: "bb-default".into(), name: "Deploy to Staging".into(), path: Some("bitbucket-pipelines.yml".into()), url: None }]
}
fn bitbucket_runs() -> Vec<PipelineRun> {
    vec![
        run("bb-441", "bb-default", 441, "#441", "dbt: add revenue recognition model", PipelineRunStatus::Failed, "feat/rev-rec", me(), 2),
        run("bb-440", "bb-default", 440, "#440", "Tighten CORS on the admin API", PipelineRunStatus::Succeeded, "main", dev(), 10),
    ]
}
fn pipeline_defs_for(conn: &str) -> Vec<PipelineDefinition> {
    let repo = demo_repository(conn);
    let defs = match conn {
        "gitlab" => gitlab_pipeline_defs(),
        "bitbucket" => bitbucket_pipeline_defs(),
        _ => pipeline_defs(),
    };
    defs.into_iter().map(|mut d| { d.repository = repo.clone(); d }).collect()
}
fn pipeline_runs_for(conn: &str) -> Vec<PipelineRun> {
    let repo = demo_repository(conn);
    let runs = match conn {
        "gitlab" => gitlab_runs(),
        "bitbucket" => bitbucket_runs(),
        _ => pipeline_runs(),
    };
    runs.into_iter().map(|mut r| { r.repository = repo.clone(); r }).collect()
}

fn pipeline_runs() -> Vec<PipelineRun> {
    let now = base();
    vec![
        PipelineRun {
            event: None,
            attempt: None,
            pull_request: None,
            repository: None,
            id: "r501".into(),
            definition_id: "ci".into(),
            number: Some(501),
            name: Some("10.1.100".into()),
            title: Some("Rework the webhook retry queue".into()),
            status: PipelineRunStatus::Running,
            triggered_by: Some(alice()),
            branch: Some("feature/retry".into()),
            commit_sha: Some("a1b2c3d".into()),
            started_at: Some(now - chrono::Duration::minutes(4)),
            finished_at: None,
            url: Some("https://ci.example.com/runs/demo".into()),
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
            event: None,
            attempt: None,
            pull_request: None,
            repository: None,
            id: "r500".into(),
            definition_id: "ci".into(),
            number: Some(500),
            name: Some("10.1.99".into()),
            title: Some("Bump Next.js to 14.2.5".into()),
            status: PipelineRunStatus::Failed,
            triggered_by: Some(bob()),
            branch: Some("main".into()),
            commit_sha: Some("9f8e7d6".into()),
            started_at: Some(now - chrono::Duration::hours(1)),
            finished_at: Some(now - chrono::Duration::minutes(52)),
            url: Some("https://ci.example.com/runs/demo".into()),
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
            event: None,
            attempt: None,
            pull_request: None,
            repository: None,
            id: "r207".into(),
            definition_id: "release".into(),
            number: Some(207),
            name: Some("10.1.98".into()),
            title: Some("Release 10.1.98".into()),
            status: PipelineRunStatus::Succeeded,
            triggered_by: Some(carol()),
            branch: Some("main".into()),
            commit_sha: Some("1234abc".into()),
            started_at: Some(now - chrono::Duration::days(2)),
            finished_at: Some(now - chrono::Duration::days(2) + chrono::Duration::minutes(8)),
            url: Some("https://ci.example.com/runs/demo".into()),
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
            event: None,
            attempt: None,
            pull_request: None,
            repository: None,
            id: "r502".into(),
            definition_id: "ci".into(),
            number: Some(502),
            name: Some("10.1.101".into()),
            title: Some("Rotate the KMS signing keys".into()),
            status: PipelineRunStatus::Queued,
            triggered_by: Some(alice()),
            branch: Some("main".into()),
            commit_sha: Some("cafe123".into()),
            started_at: None,
            finished_at: None,
            url: Some("https://ci.example.com/runs/demo".into()),
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

/// Work-item edits made this run (reassign / title / description), keyed by item id, so they
/// persist and read back like a real provider. `assignee: None` = unchanged, `Some(None)` =
/// unassigned, `Some(Some(u))` = assigned.
#[derive(Default, Clone)]
struct WiOverride {
    assignee: Option<Option<User>>,
    title: Option<String>,
    description: Option<String>,
    state: Option<String>,
}

/// Work-item writes made this run, keyed by item id, appended to the item's history so the
/// activity timeline shows what you just did — like a real provider's history API would.
fn wi_events() -> &'static Mutex<HashMap<String, Vec<TimelineEvent>>> {
    static STORE: OnceLock<Mutex<HashMap<String, Vec<TimelineEvent>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The category a demo state name falls in — what a real provider's mapper decides — so a
/// state change moves the item between buckets (a "Done" item leaves the open list).
fn demo_state_category(state: &str) -> WorkItemStateCategory {
    use WorkItemStateCategory as C;
    match state.to_ascii_lowercase().as_str() {
        "done" | "closed" | "resolved" => C::Completed,
        "canceled" | "cancelled" | "won't do" => C::Canceled,
        "backlog" => C::Backlog,
        "todo" | "to do" | "new" => C::Unstarted,
        "triage" => C::Triage,
        _ => C::Started,
    }
}

fn record_wi_event(id: &str, kind: TimelineEventKind, summary: String) {
    let event = TimelineEvent { actor: Some(me()), kind, summary, at: Some(base()) };
    wi_events().lock().unwrap().entry(id.to_string()).or_default().push(event);
}

/// A believable history for a demo work item, derived from the item itself so it agrees with
/// the fields on screen: someone filed it, it moved to its current state if it has left the
/// backlog, and it was assigned to whoever holds it. Oldest → newest, like every provider.
fn wi_history(wi: &WorkItem) -> Vec<TimelineEvent> {
    use TimelineEventKind as K;
    let created = wi.created_at.unwrap_or_else(base);
    let updated = wi.updated_at.unwrap_or_else(base).max(created);
    let mid = created + (updated - created) / 2;
    let mut events = vec![TimelineEvent { actor: Some(bob()), kind: K::Other, summary: "created this".into(), at: Some(created) }];
    if !matches!(wi.state_category, WorkItemStateCategory::Triage | WorkItemStateCategory::Backlog | WorkItemStateCategory::Unstarted) {
        let actor = wi.assignee.clone().unwrap_or_else(alice);
        events.push(TimelineEvent { actor: Some(actor), kind: K::StateChanged, summary: format!("changed status to {}", wi.state), at: Some(mid) });
    }
    if let Some(a) = &wi.assignee {
        events.push(TimelineEvent {
            actor: Some(alice()),
            kind: K::Assigned,
            summary: format!("assigned this to {}", a.display_name),
            at: Some(updated),
        });
    }
    events
}

fn wi_overrides() -> &'static Mutex<HashMap<String, WiOverride>> {
    static STORE: OnceLock<Mutex<HashMap<String, WiOverride>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The people you can assign demo work to (mirrors a provider's assignable-users call).
fn demo_assignable() -> Vec<User> {
    vec![me(), alice(), bob(), carol(), dev()]
}

/// Fold any edits made this run onto a freshly-built work item.
fn apply_wi_override(mut wi: WorkItem) -> WorkItem {
    if let Some(ov) = wi_overrides().lock().unwrap().get(&wi.id) {
        if let Some(a) = &ov.assignee {
            wi.assignee = a.clone();
        }
        if let Some(t) = &ov.title {
            wi.title = t.clone();
        }
        if let Some(d) = &ov.description {
            wi.description = Some(d.clone());
        }
        if let Some(st) = &ov.state {
            wi.state = st.clone();
            wi.state_category = demo_state_category(st);
        }
    }
    wi
}

/// Replies posted this run, keyed by `"{pr_id}:{thread_id}"`, so `reply_to_thread` persists
/// like a real provider: the reply comes back appended to its thread on the next `threads()`.
fn thread_replies() -> &'static Mutex<HashMap<String, Vec<Comment>>> {
    static STORE: OnceLock<Mutex<HashMap<String, Vec<Comment>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// PR ids merged during this `--demo` run. `merge()` records them and `list()`/`get()`
/// then report those PRs as freshly merged — so a PR you merge drops out of "open" and
/// shows up under "Recently merged", exactly like a real provider.
fn merged_prs() -> &'static Mutex<HashSet<String>> {
    static STORE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Apply any session merges to a PR: freshly merged, no longer a draft, updated just now.
fn apply_session_merge(mut pr: PullRequest) -> PullRequest {
    if merged_prs().lock().unwrap().contains(&pr.id) {
        pr.status = PullRequestStatus::Merged;
        pr.is_draft = false;
        pr.updated_at = Some(base());
    }
    pr
}

/// PR ids you've requested changes on this run. `vote(Rejected)` records them; approving clears
/// them — so a re-fetch reflects your review exactly like a real provider would.
fn changes_requested_prs() -> &'static Mutex<HashSet<String>> {
    static STORE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Reflect a session "request changes": your reviewer entry reads as Rejected (added if you
/// weren't already a reviewer) until you approve — exactly what re-fetching the PR would show.
fn apply_session_review(mut pr: PullRequest) -> PullRequest {
    if changes_requested_prs().lock().unwrap().contains(&pr.id) {
        match pr.reviewers.iter_mut().find(|r| r.user.id == me().id) {
            Some(r) => r.vote = ReviewVote::Rejected,
            None => pr.reviewers.push(rev(me(), ReviewVote::Rejected)),
        }
        pr.updated_at = Some(base());
    }
    pr
}

/// All session mutations a real provider would surface on a re-fetch (merge + your review).
fn apply_session_state(pr: PullRequest) -> PullRequest {
    apply_session_review(apply_session_merge(pr))
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
            .map(apply_session_state)
            .filter(|p| query.include_completed || matches!(p.status, PullRequestStatus::Open | PullRequestStatus::Draft))
            .collect();
        Ok(apply_pull_request_filter(prs, query.filter, Some(DEMO_ME)))
    }
    async fn current_user(&self) -> Result<Option<String>> {
        Ok(Some(DEMO_ME.to_string()))
    }
    async fn get(&self, item: &ItemRef) -> Result<PullRequest> {
        let id: &str = &item.id;
        prs_for(&self.conn)
            .into_iter()
            .find(|p| p.id == id)
            .map(apply_session_state)
            .ok_or_else(|| forgetop_core::Error::NotFound(id.into()))
    }
    async fn threads(&self, item: &ItemRef) -> Result<Vec<CommentThread>> {
        let id: &str = &item.id;
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
            // A general (conversation) comment — no file — so it also shows in the timeline.
            CommentThread {
                id: "t3".into(),
                comments: vec![Comment {
                    id: "c3".into(),
                    author: dev(),
                    body: "Thanks for the quick turnaround on this.".into(),
                    created_at: Some(base() - chrono::Duration::hours(2)),
                }],
                file_path: None,
                line: None,
                is_resolved: false,
            },
        ];
        // Include anything submitted this session so it persists like a real provider.
        if let Some(extra) = submitted_threads().lock().unwrap().get(id) {
            threads.extend(extra.iter().cloned());
        }
        // Append any replies posted this session into their target thread.
        let replies = thread_replies().lock().unwrap();
        for t in &mut threads {
            if let Some(rs) = replies.get(&format!("{id}:{}", t.id)) {
                t.comments.extend(rs.iter().cloned());
            }
        }
        Ok(threads)
    }
    async fn timeline(&self, item: &ItemRef) -> Result<Vec<TimelineEvent>> {
        let id: &str = &item.id;
        use TimelineEventKind as K;
        let mut events = Vec::new();
        // Derive review events from the PR's actual reviewers, so the timeline matches the
        // Reviewers panel exactly (comment events are added from the threads by the server).
        if let Some(pr) = prs_for(&self.conn).into_iter().find(|p| p.id == id) {
            events.push(TimelineEvent {
                actor: Some(pr.author.clone()),
                kind: K::Other,
                summary: "opened this pull request".into(),
                at: pr.created_at,
            });
            for (i, r) in pr.reviewers.iter().enumerate() {
                let at = Some(base() - chrono::Duration::hours(6 - i as i64));
                match r.vote {
                    ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions => {
                        events.push(TimelineEvent { actor: Some(r.user.clone()), kind: K::Approved, summary: "approved these changes".into(), at });
                    }
                    ReviewVote::Rejected => {
                        events.push(TimelineEvent { actor: Some(r.user.clone()), kind: K::ChangesRequested, summary: "requested changes".into(), at });
                    }
                    _ => {}
                }
            }
        }
        // Actions taken this session, like re-fetching after acting.
        if changes_requested_prs().lock().unwrap().contains(id) {
            events.push(TimelineEvent { actor: Some(me()), kind: K::ChangesRequested, summary: "requested changes".into(), at: Some(base()) });
        }
        if merged_prs().lock().unwrap().contains(id) {
            events.push(TimelineEvent { actor: Some(me()), kind: K::Merged, summary: "merged this pull request".into(), at: Some(base()) });
        }
        Ok(events)
    }
    async fn changes(&self, _item: &ItemRef) -> Result<Vec<FileChange>> {
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
    async fn checks(&self, item: &ItemRef) -> Result<Vec<CheckRun>> {
        let id: &str = &item.id;
        // Canned URLs so the dashboard Checks tab is clickable in --demo, like a real provider.
        let url = |name: &str| Some(format!("https://example.test/pr/{id}/checks/{name}"));
        Ok(vec![
            CheckRun { name: "build".into(), status: CheckStatus::Passed, url: url("build") },
            CheckRun { name: "unit-tests".into(), status: CheckStatus::Passed, url: url("unit-tests") },
            CheckRun { name: "clippy".into(), status: CheckStatus::Passed, url: url("clippy") },
            CheckRun { name: "integration".into(), status: CheckStatus::Failed, url: url("integration") },
            CheckRun { name: "deploy-preview".into(), status: CheckStatus::Pending, url: url("deploy-preview") },
        ])
    }
    async fn commits(&self, _item: &ItemRef) -> Result<Vec<Commit>> {
        Ok(vec![
            Commit { sha: "a1b2c3d".into(), message: "Add RetryPolicy with jittered backoff".into(), author: "alice".into(), date: Some(base()), url: None },
            Commit { sha: "e4f5a6b".into(), message: "Wire retry into the HTTP client".into(), author: "alice".into(), date: Some(base() - chrono::Duration::hours(3)), url: None },
            Commit { sha: "9c8d7e6".into(), message: "Address review: cap max attempts".into(), author: "bob".into(), date: Some(base() - chrono::Duration::hours(1)), url: None },
        ])
    }
    async fn commit_changes(&self, _item: &ItemRef, sha: &str) -> Result<Vec<FileChange>> {
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
    async fn add_comment(&self, item: &ItemRef, body: &str) -> Result<()> {
        let id: &str = &item.id;
        // Persist as a general (non-line) thread so it comes back from threads() and shows
        // on the Conversation tab after the view refreshes.
        let mut store = submitted_threads().lock().unwrap();
        let entry = store.entry(id.to_string()).or_default();
        let n = entry.len();
        entry.push(CommentThread {
            id: format!("submitted-{n}"),
            comments: vec![Comment { id: format!("mine-{n}"), author: me(), body: body.into(), created_at: Some(base()) }],
            file_path: None,
            line: None,
            is_resolved: false,
        });
        Ok(())
    }
    async fn reply_to_thread(&self, item: &ItemRef, thread_id: &str, body: &str) -> Result<()> {
        let id: &str = &item.id;
        // Append the reply to its thread so it comes back inline on the next refresh.
        let mut store = thread_replies().lock().unwrap();
        let entry = store.entry(format!("{id}:{thread_id}")).or_default();
        let n = entry.len();
        entry.push(Comment { id: format!("reply-{thread_id}-{n}"), author: me(), body: body.into(), created_at: Some(base()) });
        Ok(())
    }
    async fn vote(&self, item: &ItemRef, vote: ReviewVote) -> Result<()> {
        let id: &str = &item.id;
        // Record your review so list()/get() reflect it on the next fetch, like a real provider:
        // requesting changes marks the PR, approving clears it.
        let mut cr = changes_requested_prs().lock().unwrap();
        match vote {
            ReviewVote::Rejected => {
                cr.insert(id.to_string());
            }
            ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions => {
                cr.remove(id);
            }
            _ => {}
        }
        Ok(())
    }
    async fn merge(&self, item: &ItemRef, _options: &MergeOptions) -> Result<()> {
        let id: &str = &item.id;
        // Record the merge so list()/get() report this PR as freshly merged (→ Recently merged).
        merged_prs().lock().unwrap().insert(id.to_string());
        Ok(())
    }
    async fn revert(&self, _item: &ItemRef) -> Result<()> {
        // Demo revert is a no-op success so the button is present and clickable without a live forge.
        Ok(())
    }
    async fn submit_review(&self, item: &ItemRef, _event: ReviewVote, comments: &[LineComment]) -> Result<()> {
        let id: &str = &item.id;
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
            .map(apply_wi_override)
            .filter(|w| {
                query.include_completed
                    || !matches!(w.state_category, WorkItemStateCategory::Completed | WorkItemStateCategory::Canceled)
            })
            .filter(|w| !query.mine_only || w.assignee.as_ref().map(|u| u.id == "me").unwrap_or(false))
            .collect())
    }
    async fn get(&self, item: &ItemRef) -> Result<WorkItem> {
        let id: &str = &item.id;
        wis_for(&self.conn)
            .into_iter()
            .find(|w| w.id == id)
            .map(apply_wi_override)
            .ok_or_else(|| forgetop_core::Error::NotFound(id.into()))
    }
    async fn threads(&self, item: &ItemRef) -> Result<Vec<CommentThread>> {
        let id: &str = &item.id;
        // Comments submitted this session persist and come back, like a real provider.
        Ok(submitted_threads().lock().unwrap().get(id).cloned().unwrap_or_default())
    }
    async fn timeline(&self, item: &ItemRef) -> Result<Vec<TimelineEvent>> {
        let id: &str = &item.id;
        // The history of the item as first built (before this run's edits), then what was done
        // to it this run — so an assignment made here reads as a new event, not a rewritten one.
        let mut events = wis_for(&self.conn).iter().find(|w| w.id == id).map(wi_history).unwrap_or_default();
        events.extend(wi_events().lock().unwrap().get(id).cloned().unwrap_or_default());
        Ok(events)
    }
    async fn set_state(&self, item: &ItemRef, state: &str) -> Result<()> {
        let id: &str = &item.id;
        wi_overrides().lock().unwrap().entry(id.to_string()).or_default().state = Some(state.to_string());
        record_wi_event(id, TimelineEventKind::StateChanged, format!("changed status to {state}"));
        Ok(())
    }
    async fn add_comment(&self, item: &ItemRef, body: &str) -> Result<()> {
        let id: &str = &item.id;
        let mut store = submitted_threads().lock().unwrap();
        let entry = store.entry(id.to_string()).or_default();
        let n = entry.len();
        entry.push(CommentThread {
            id: format!("submitted-{n}"),
            comments: vec![Comment { id: format!("mine-{n}"), author: me(), body: body.into(), created_at: Some(base()) }],
            file_path: None,
            line: None,
            is_resolved: false,
        });
        Ok(())
    }
    async fn available_states(&self, _item: &ItemRef) -> Result<Vec<String>> {
        Ok(["Backlog", "Todo", "In Progress", "In Review", "Blocked", "Done"].iter().map(|s| s.to_string()).collect())
    }
    async fn assignable_users(&self, _item: &ItemRef) -> Result<Vec<User>> {
        Ok(demo_assignable())
    }
    async fn set_assignee(&self, item: &ItemRef, assignee_id: Option<&str>) -> Result<()> {
        let id: &str = &item.id;
        let user = assignee_id.and_then(|aid| demo_assignable().into_iter().find(|u| u.id == aid));
        let summary = match &user {
            Some(u) => format!("assigned this to {}", u.display_name),
            None => "unassigned this".into(),
        };
        wi_overrides().lock().unwrap().entry(id.to_string()).or_default().assignee = Some(user);
        record_wi_event(id, TimelineEventKind::Assigned, summary);
        Ok(())
    }
    async fn update_fields(&self, item: &ItemRef, title: Option<&str>, description: Option<&str>) -> Result<()> {
        let id: &str = &item.id;
        let mut store = wi_overrides().lock().unwrap();
        let ov = store.entry(id.to_string()).or_default();
        if let Some(t) = title {
            ov.title = Some(t.to_string());
        }
        if let Some(d) = description {
            ov.description = Some(d.to_string());
        }
        drop(store);
        let what = match (title.is_some(), description.is_some()) {
            (true, true) => "edited the title and description",
            (true, false) => "edited the title",
            _ => "edited the description",
        };
        record_wi_event(id, TimelineEventKind::Other, what.into());
        Ok(())
    }
}

/// Believable per-job demo logs, keyed by job id (`j1`/`j10` compile, `j2` the live-running
/// `dotnet test`, `j11` a finished `dotnet test`, `j12` the failing integration suite, `j20`
/// a deploy) so a "live logs" feature (follow mode, jump-to-first-error, search) has something
/// real to demonstrate. Fixed timestamps keep everything deterministic except [`unit_log_growing`].
fn job_log(job_id: &str) -> String {
    match job_id {
        "j1" | "j10" => compile_log(),
        "j2" => unit_log_growing(),
        "j11" => unit_log_succeeded(),
        "j12" => integration_log_failed(),
        "j20" => deploy_log(),
        other => generic_log(other),
    }
}

fn compile_log() -> String {
    concat!(
        "09:36:40  Restoring NuGet packages...\n",
        "09:36:44  Restored /src/Northwind.sln (in 3.6s)\n",
        "09:36:45  Northwind.Domain -> bin/Debug/net8.0/Northwind.Domain.dll\n",
        "09:36:52  Northwind.Infrastructure -> bin/Debug/net8.0/Northwind.Infrastructure.dll\n",
        "09:37:01  Northwind.Api -> bin/Debug/net8.0/Northwind.Api.dll\n",
        "09:37:01  Build succeeded.\n",
        "09:37:01      0 Warning(s)\n",
        "09:37:01      0 Error(s)\n",
        "09:37:01  Time Elapsed 00:00:21.44\n",
    )
    .to_string()
}

fn unit_log_succeeded() -> String {
    concat!(
        "09:41:40  Restoring NuGet packages...\n",
        "09:41:41  Restore complete (0.8s)\n",
        "09:41:41  Northwind.Orders.Tests -> bin/Debug/net8.0/Northwind.Orders.Tests.dll\n",
        "09:41:42  Starting test execution, please wait...\n",
        "09:41:42  A total of 1 test files matched the specified pattern.\n",
        "09:41:43  [xUnit.net 00:00:00.91]   Discovering: Northwind.Orders.Tests\n",
        "09:41:43  [xUnit.net 00:00:00.98]   Discovered:  Northwind.Orders.Tests\n",
        "09:41:43  [xUnit.net 00:00:00.99]   Starting:    Northwind.Orders.Tests\n",
        "09:41:45    Passed OrderServiceTests.CreatesOrder_WithValidItems [12 ms]\n",
        "09:41:45    Passed OrderServiceTests.RejectsOrder_WithNoItems [3 ms]\n",
        "09:41:46    Passed OrderServiceTests.AppliesDiscountCode [9 ms]\n",
        "09:41:47    Passed OrderServiceTests.RecalculatesTotals_OnLineItemChange [14 ms]\n",
        "09:41:48  [xUnit.net 00:00:05.60]   Finished:    Northwind.Orders.Tests\n",
        "09:41:48  Passed!  - Failed: 0, Passed: 64, Skipped: 0, Total: 64, Duration: 6 s - Northwind.Orders.Tests.dll\n",
    )
    .to_string()
}

/// The failing job (j12, run r500) — a realistic xUnit failure a matcher can find: a `[FAIL]`
/// line with `Assert.InRange` detail and a stack line, a `Failed!` summary, and a final
/// `##[error]` line. Also includes lines that look error-ish but aren't (a clean `Passed!`
/// summary and a "0 errors" build line) to exercise the false-positive side of the matcher.
fn integration_log_failed() -> String {
    concat!(
        "09:41:10  Restoring NuGet packages...\n",
        "09:41:12  Restore complete (1.8s)\n",
        "09:41:12  Building Northwind.Integration.Tests -> bin/Debug/net8.0/Northwind.Integration.Tests.dll\n",
        "09:41:18  Build succeeded. 0 errors, 2 warnings\n",
        "09:41:19  Starting test execution, please wait...\n",
        "09:41:19  A total of 2 test files matched the specified pattern.\n",
        "09:41:20  [xUnit.net 00:00:00.87]   Discovering: Northwind.Cart.Tests\n",
        "09:41:20  [xUnit.net 00:00:00.95]   Discovered:  Northwind.Cart.Tests\n",
        "09:41:20  [xUnit.net 00:00:00.96]   Starting:    Northwind.Cart.Tests\n",
        "09:41:22    Passed CartTests.test_add_item [41 ms]\n",
        "09:41:22    Passed CartTests.test_remove_item [18 ms]\n",
        "09:41:24    Passed InventoryTests.test_reserve_stock [55 ms]\n",
        "09:41:24    Passed InventoryTests.test_release_stock [22 ms]\n",
        "09:41:28  [xUnit.net 00:00:08.10]   Finished:    Northwind.Cart.Tests\n",
        "09:41:28  Passed!  - Failed: 0, Passed: 212, Skipped: 0, Total: 212, Duration: 8 s - Northwind.Cart.Tests.dll\n",
        "09:41:29  [xUnit.net 00:00:09.00]   Starting:    Northwind.Integration.Tests\n",
        "09:41:31    Passed RefundTests.test_partial_refund [19 ms]\n",
        "09:41:31    Passed CheckoutFlowTests.test_apply_discount [30 ms]\n",
        "09:41:33  [FAIL] CheckoutFlowTests.test_checkout_flow [812 ms]\n",
        "09:41:33    Assert.InRange() Failure: Value not in range\n",
        "09:41:33    Expected: value in range (200, 299]\n",
        "09:41:33    Actual: 503\n",
        "09:41:33       at Northwind.Integration.Tests.CheckoutFlowTests.test_checkout_flow() in /src/tests/Northwind.Integration.Tests/CheckoutFlowTests.cs:line 47\n",
        "09:41:33    ----- Inner Stack Trace -----\n",
        "09:41:33       at System.Net.Http.HttpClient.SendAsync(HttpRequestMessage request)\n",
        "09:41:35    Passed RefundTests.test_refund [19 ms]\n",
        "09:41:36  [xUnit.net 00:00:16.40]   Finished:    Northwind.Integration.Tests\n",
        "09:41:36  Failed!  - Failed: 1, Passed: 97, Skipped: 0, Total: 98, Duration: 16 s - Northwind.Integration.Tests.dll\n",
        "09:41:36  ##[error]Process completed with exit code 1.\n",
    )
    .to_string()
}

fn deploy_log() -> String {
    concat!(
        "09:50:01  Initializing the backend...\n",
        "09:50:02  Initializing provider plugins...\n",
        "09:50:04  Terraform has been successfully initialized!\n",
        "09:50:05  terraform plan -out=tfplan\n",
        "09:50:09  Plan: 3 to add, 1 to change, 0 to destroy.\n",
        "09:50:10  terraform apply tfplan\n",
        "09:50:22  aws_ecs_service.api: Modifying...\n",
        "09:50:40  aws_ecs_service.api: Modifications complete after 18s\n",
        "09:50:41  Apply complete! Resources: 3 added, 1 changed, 0 destroyed.\n",
        "09:50:42  kubectl rollout status deployment/api -n production\n",
        "09:50:55  deployment \"api\" successfully rolled out\n",
    )
    .to_string()
}

/// A plausible-but-generic log for a job id/name we don't have a scripted log for.
fn generic_log(job_id: &str) -> String {
    let mut out = format!("09:30:00  Starting job {job_id}...\n");
    for i in 1..=18 {
        out.push_str(&format!("09:30:{i:02}  step output line {i}\n"));
    }
    out.push_str("09:30:19  Done. All steps completed successfully.\n");
    out
}

/// Process-wide start instant for the demo's one "live" job (r501/j2's `dotnet test` step,
/// which stays Running for the life of the `--demo` session) — [`unit_log_growing`] measures
/// elapsed time against this so the log starts short and grows the longer `--demo` runs.
fn demo_start() -> std::time::Instant {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

/// Scripted lines revealed one at a time as the growing `dotnet test` log (job j2, run r501)
/// plays out — roughly one every 2 seconds of elapsed demo time (see [`growth_line_count`]).
fn dotnet_test_script() -> &'static [&'static str] {
    &[
        "09:38:02  Restoring NuGet packages...",
        "09:38:03  Restore complete (0.9s)",
        "09:38:03  Northwind.Orders.Tests -> bin/Debug/net8.0/Northwind.Orders.Tests.dll",
        "09:38:04  Starting test execution, please wait...",
        "09:38:04  A total of 3 test files matched the specified pattern.",
        "09:38:05  [xUnit.net 00:00:01.02]   Discovering: Northwind.Orders.Tests",
        "09:38:05  [xUnit.net 00:00:01.10]   Discovered:  Northwind.Orders.Tests",
        "09:38:05  [xUnit.net 00:00:01.11]   Starting:    Northwind.Orders.Tests",
        "09:38:07    Passed OrderServiceTests.CreatesOrder_WithValidItems [12 ms]",
        "09:38:07    Passed OrderServiceTests.RejectsOrder_WithNoItems [3 ms]",
        "09:38:09    Passed OrderServiceTests.AppliesDiscountCode [9 ms]",
        "09:38:09  [xUnit.net 00:00:05.40]   Finished:    Northwind.Orders.Tests",
        "09:38:10  [xUnit.net 00:00:05.90]   Starting:    Northwind.Payments.Tests",
        "09:38:12    Passed PaymentGatewayTests.ChargesCard_WithValidToken [22 ms]",
        "09:38:12    Passed PaymentGatewayTests.RefundsFullAmount [15 ms]",
        "09:38:14    Passed PaymentGatewayTests.HandlesGatewayTimeout [201 ms]",
        "09:38:14  [xUnit.net 00:00:09.70]   Finished:    Northwind.Payments.Tests",
        "09:38:15  [xUnit.net 00:00:10.10]   Starting:    Northwind.Webhooks.Tests",
        "09:38:17    Passed WebhookRetryQueueTests.Enqueues_OnTransientFailure [8 ms]",
        "09:38:17    Passed WebhookRetryQueueTests.DropsAfter_MaxAttempts [4 ms]",
        "09:38:19    Passed WebhookRetryQueueTests.BackoffIsExponential [61 ms]",
        "09:38:19    Passed WebhookRetryQueueTests.PersistsDeadLetterOnFinalFailure [11 ms]",
    ]
}

/// Pure elapsed-seconds -> revealed-line-count for the growing log, so growth is testable
/// without sleeping: about one new line every 2 seconds, capped so a long-lived `--demo`
/// session can't grow the log unboundedly.
fn growth_line_count(elapsed_secs: u64) -> usize {
    const CAP: usize = 300;
    ((1 + elapsed_secs / 2) as usize).min(CAP)
}

/// Builds the growing log for `elapsed_secs` of elapsed time — the scripted lines first, then
/// (once the script is exhausted) periodic "still running" progress lines, so the log keeps
/// growing for as long as the job stays Running.
fn unit_log_growing_at(elapsed_secs: u64) -> String {
    let script = dotnet_test_script();
    let n = growth_line_count(elapsed_secs);
    let mut out = String::new();
    for line in script.iter().take(n) {
        out.push_str(line);
        out.push('\n');
    }
    if n > script.len() {
        let mut completed: u32 = 22; // tests completed by the end of the scripted portion
        for k in 0..(n - script.len()) {
            completed += 1;
            let sec = 20 + (k % 40);
            out.push_str(&format!("09:39:{sec:02}  Still running… {completed} tests completed\n"));
        }
    }
    out
}

/// The growing log for the running job (j2, run r501) — grows with real elapsed time since
/// `--demo` started (see [`demo_start`]), so a "follow mode" feature has something to follow.
fn unit_log_growing() -> String {
    unit_log_growing_at(demo_start().elapsed().as_secs())
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
            .map(apply_pipe_cancel)
            .collect())
    }
    async fn get_run(&self, run: &ItemRef) -> Result<PipelineRun> {
        let run_id: &str = &run.id;
        pipeline_runs_for(&self.conn)
            .into_iter()
            .find(|r| r.id == run_id)
            .map(apply_pipe_cancel)
            .ok_or_else(|| forgetop_core::Error::NotFound(run_id.into()))
    }
    async fn logs(&self, run: &ItemRef, job_id: Option<&str>) -> Result<String> {
        let run_id: &str = &run.id;
        if let Some(job) = job_id {
            let mut out = format!("=== logs for run {run_id} · {job} ===\n");
            out.push_str(&job_log(job));
            return Ok(out);
        }
        // No job specified: a sensible whole-run log — concatenate each job's log in order.
        let job_ids: &[&str] = match run_id {
            "r501" => &["j1", "j2"],
            "r500" => &["j10", "j11", "j12"],
            "r207" => &["j20"],
            _ => &[],
        };
        if job_ids.is_empty() {
            let mut out = format!("=== logs for run {run_id} ===\n");
            out.push_str(&job_log(run_id));
            return Ok(out);
        }
        let mut out = String::new();
        for job in job_ids {
            out.push_str(&format!("=== job {job} ===\n"));
            out.push_str(&job_log(job));
            out.push('\n');
        }
        Ok(out)
    }
    async fn trigger(&self, _definition: &ItemRef, _branch: Option<&str>) -> Result<()> {
        Ok(())
    }
    async fn cancel_run(&self, run: &ItemRef) -> Result<()> {
        let run_id: &str = &run.id;
        canceled_runs().lock().unwrap().insert(run_id.to_string());
        Ok(())
    }
    fn supports_approvals(&self) -> bool {
        true
    }
    async fn pending_approvals(&self, run: &ItemRef) -> Result<Vec<PipelineApproval>> {
        let run_id: &str = &run.id;
        // A cancelled run no longer has a live gate.
        if canceled_runs().lock().unwrap().contains(run_id) {
            return Ok(Vec::new());
        }
        // The running CI run (#501) waits on a production deployment gate you can act on.
        Ok(if run_id == "r501" {
            vec![PipelineApproval { id: "production".into(), name: "production".into(), can_respond: true }]
        } else {
            Vec::new()
        })
    }
    async fn respond_approval(&self, _run: &ItemRef, _approval_id: &str, _decision: ApprovalDecision, _comment: Option<&str>) -> Result<()> {
        Ok(())
    }
}

/// Pipeline run ids cancelled this `--demo` session, so cancel_run persists like a real provider:
/// the run reads back as Canceled through list_runs()/get_run().
fn canceled_runs() -> &'static Mutex<HashSet<String>> {
    static STORE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Fold a session cancel onto a freshly-built run.
fn apply_pipe_cancel(mut run: PipelineRun) -> PipelineRun {
    if canceled_runs().lock().unwrap().contains(&run.id) {
        run.status = PipelineRunStatus::Canceled;
    }
    run
}

/// Notification ids marked read this `--demo` session, so mark_read persists like real.
fn read_notifications() -> &'static Mutex<HashSet<String>> {
    static STORE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashSet::new()))
}

#[allow(clippy::too_many_arguments)]
fn notif(
    id: &str,
    kind: NotificationKind,
    item: NotificationItemType,
    item_id: Option<&str>,
    title: &str,
    context: &str,
    url: &str,
    updated_h: i64,
) -> Notification {
    Notification {
        repository: None,
        id: id.into(),
        kind,
        item_type: item,
        item_id: item_id.map(Into::into),
        title: title.into(),
        context: context.into(),
        url: Some(url.into()),
        unread: true,
        updated_at: Some(base() - chrono::Duration::hours(updated_h)),
    }
}

/// Canned notifications per demo connection, each pointing at one of that connection's real
/// demo items (by id) so pressing it drills into the actual PR / work item.
fn demo_notifications_for(conn: &str) -> Vec<Notification> {
    use NotificationItemType as IT;
    use NotificationKind as K;
    match conn {
        "github" => vec![
            notif("gh-1", K::ReviewRequested, IT::PullRequest, Some("1501"), "Refactor the webhook retry queue", "northwind/payments", "https://example.test/pr/1501", 1),
            notif("gh-2", K::CiFailed, IT::PullRequest, Some("1492"), "Bump Next.js to 14.2.5", "northwind/web", "https://example.test/pr/1492", 3),
            notif("gh-3", K::Mention, IT::WorkItem, Some("#842"), "Investigate elevated p99 on POST /charge", "northwind/payments", "https://example.test/issue/842", 5),
            notif("gh-4", K::Comment, IT::PullRequest, Some("1487"), "Add idempotency keys to the payments API", "northwind/payments", "https://example.test/pr/1487", 26),
        ],
        "gitlab" => vec![
            notif("gl-1", K::ReviewRequested, IT::PullRequest, Some("318"), "Rotate the KMS signing keys", "platform/infra", "https://example.test/mr/318", 2),
            notif("gl-2", K::Assigned, IT::WorkItem, Some("#77"), "Right-size the staging cluster", "platform/infra", "https://example.test/issue/77", 6),
        ],
        "linear" => vec![
            notif("ln-1", K::Assigned, IT::WorkItem, Some("ENG-231"), "Design the ledger reconciliation job", "Engineering", "https://example.test/issue/ENG-231", 4),
            notif("ln-2", K::StateChange, IT::WorkItem, Some("ENG-198"), "Migrate feature flags to OpenFeature", "Engineering", "https://example.test/issue/ENG-198", 18),
        ],
        _ => vec![],
    }
}

struct DemoNotifications {
    conn: String,
}
#[async_trait]
impl NotificationSource for DemoNotifications {
    async fn list(&self) -> Result<Vec<Notification>> {
        demo_latency().await;
        let read = read_notifications().lock().unwrap();
        let mut ns = demo_notifications_for(&self.conn);
        for n in ns.iter_mut() {
            if read.contains(&n.id) {
                n.unread = false;
            }
        }
        ns.sort_by_key(|n| std::cmp::Reverse(n.updated_at)); // newest first
        Ok(ns)
    }
    async fn mark_read(&self, id: &str) -> Result<()> {
        read_notifications().lock().unwrap().insert(id.to_string());
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
    fn notifications(&self) -> Option<Arc<dyn NotificationSource>> {
        self.caps.supports_notifications.then(|| Arc::new(DemoNotifications { conn: self.id.clone() }) as Arc<dyn NotificationSource>)
    }
    async fn check(&self) -> bool {
        true
    }
}

/// Capabilities for a demo connection — mirrors the real provider so the UI gates
/// sections and labels (MRs vs PRs, Issues) exactly as it would live.
pub fn demo_capabilities(provider: ProviderType) -> Capabilities {
    let mut caps = match provider {
        ProviderType::GitLab => crate::gitlab::gitlab_capabilities(),
        ProviderType::Bitbucket => crate::bitbucket::bitbucket_capabilities(),
        ProviderType::Linear => crate::linear::linear_capabilities(),
        ProviderType::Jira => crate::jira::jira_capabilities(),
        _ => crate::github::github_capabilities(),
    };
    // The demo shows the inbox for the providers that have a real notification feed.
    caps.supports_notifications = matches!(provider, ProviderType::GitHub | ProviderType::GitLab | ProviderType::Linear);
    caps
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
        let (pr, other) = (ItemRef::new("persist-pr-a"), ItemRef::new("persist-pr-b"));
        let before = src.threads(&pr).await.unwrap().len();

        src.submit_review(
            &pr,
            ReviewVote::NoVote,
            &[LineComment { path: "src/http/retry.rs".into(), line: 7, side: DiffSide::New, body: "please add a test".into() }],
        )
        .await
        .unwrap();

        let after = src.threads(&pr).await.unwrap();
        assert_eq!(after.len(), before + 1, "the submitted comment persists as a new thread");
        assert!(
            after.iter().any(|t| t.comments.iter().any(|c| c.body == "please add a test")),
            "the submitted body comes back from threads()"
        );
        // A different PR is unaffected by what was submitted to this one.
        assert_eq!(src.threads(&other).await.unwrap().len(), before);
    }

    #[tokio::test]
    async fn pr_comment_persists_as_a_conversation_thread() {
        let src = DemoPr { conn: "github".into() };
        let id = ItemRef::new("persist-prcomment-a"); // unique id → no cross-test pollution
        src.add_comment(&id, "ship it").await.unwrap();
        let threads = src.threads(&id).await.unwrap();
        assert!(
            threads.iter().any(|t| t.file_path.is_none() && t.comments.iter().any(|c| c.body == "ship it")),
            "a PR comment comes back as a general (non-line) thread"
        );
    }

    #[tokio::test]
    async fn wi_comment_persists_in_threads() {
        let src = DemoWi { conn: "github".into() };
        let id = ItemRef::new("persist-wicomment-a");
        let before = src.threads(&id).await.unwrap().len();
        src.add_comment(&id, "on it").await.unwrap();
        let after = src.threads(&id).await.unwrap();
        assert_eq!(after.len(), before + 1);
        assert!(after.iter().any(|t| t.comments.iter().any(|c| c.body == "on it")));
    }

    #[tokio::test]
    async fn demo_notifications_list_mark_and_targets_resolve() {
        // Capability gating: only GitHub/GitLab/Linear have a feed.
        assert!(demo_capabilities(ProviderType::GitHub).supports_notifications);
        assert!(demo_capabilities(ProviderType::Linear).supports_notifications);
        assert!(!demo_capabilities(ProviderType::Jira).supports_notifications);

        let src = DemoNotifications { conn: "github".into() };
        let ns = src.list().await.unwrap();
        assert!(!ns.is_empty());
        assert!(ns.windows(2).all(|w| w[0].updated_at >= w[1].updated_at), "newest first");
        assert!(ns.iter().any(|n| n.unread), "some are unread");

        // The review-request points at a real demo PR the source can open (in-app drill-in).
        let review = ns.iter().find(|n| n.kind == NotificationKind::ReviewRequested).unwrap();
        assert_eq!(review.item_type, NotificationItemType::PullRequest);
        let pr_id = review.item_id.clone().expect("has an item to open");
        let pr = DemoPr { conn: "github".into() }.get(&ItemRef::new(&pr_id)).await.unwrap();
        assert_eq!(pr.id, pr_id, "item_id resolves to a real demo PR");

        // Marking read persists for the session.
        src.mark_read(&review.id).await.unwrap();
        let after = src.list().await.unwrap();
        assert!(!after.iter().find(|n| n.id == review.id).unwrap().unread);
    }

    #[tokio::test]
    async fn merging_marks_the_pr_as_freshly_merged() {
        let src = conn().pull_requests().unwrap();
        // A fabricated id (not a real demo PR) so the session-global store can't pollute
        // other tests' open-PR counts.
        let id = "demo-merge-test-42";
        src.merge(&ItemRef::new(id), &MergeOptions { strategy: MergeStrategy::Merge, delete_source_ref: false }).await.unwrap();

        // A PR with that id now reports as merged and no longer open/draft.
        let mut p = pr(0, "x", me(), PullRequestStatus::Open, CheckStatus::Passed, MergeableState::Mergeable, vec![], 1, 1, 1, "b", &[]);
        p.id = id.to_string();
        let merged = apply_session_merge(p);
        assert_eq!(merged.status, PullRequestStatus::Merged);
        assert!(!merged.is_draft);
    }

    #[tokio::test]
    async fn requesting_changes_reflects_your_review_on_refetch() {
        // Models the #1501 ("Refactor the webhook retry queue") scenario where you're a NoVote
        // reviewer; uses a fabricated id so the session-global store can't pollute other tests.
        let src = conn().pull_requests().unwrap();
        let id = "demo-review-test-99";
        let base_pr = || {
            let mut p = pr(0, "x", me(), PullRequestStatus::Open, CheckStatus::Passed, MergeableState::Mergeable, vec![rev(me(), ReviewVote::NoVote)], 1, 1, 1, "b", &[]);
            p.id = id.to_string();
            p
        };

        // Request changes → a re-fetch (apply_session_state) shows your review as Rejected.
        src.vote(&ItemRef::new(id), ReviewVote::Rejected).await.unwrap();
        let reviewed = apply_session_state(base_pr());
        assert!(
            reviewed.reviewers.iter().any(|r| r.user.id == "me" && r.vote == ReviewVote::Rejected),
            "your review reads as changes-requested after voting"
        );

        // Approving clears it again.
        src.vote(&ItemRef::new(id), ReviewVote::Approved).await.unwrap();
        let cleared = apply_session_state(base_pr());
        assert!(cleared.reviewers.iter().all(|r| r.vote != ReviewVote::Rejected), "approving clears the changes-requested review");
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
        let run = src.get_run(&ItemRef::new("r500")).await.unwrap();
        let test = run.stages.iter().find(|s| s.name == "test").unwrap();
        let integ = test.jobs.iter().find(|j| j.name == "integration").unwrap();
        assert!(integ.steps.iter().any(|s| matches!(s.status, PipelineRunStatus::Failed)));
    }

    #[tokio::test]
    async fn failed_job_log_has_an_error_marker() {
        let src = conn().pipelines().unwrap();
        let log = src.logs(&ItemRef::new("r500"), Some("j12")).await.unwrap();
        assert!(log.contains("##[error]"), "failing job's log should carry an error marker: {log}");
        assert!(log.contains("test_checkout_flow"), "keeps the gist of the original failure");
        assert!(log.contains("Failed!  - Failed: 1, Passed: 97"), "a realistic failure summary line");
    }

    #[tokio::test]
    async fn succeeded_job_logs_have_no_error_marker() {
        let src = conn().pipelines().unwrap();
        for (run_id, job_id) in [("r500", "j10"), ("r500", "j11"), ("r207", "j20")] {
            let log = src.logs(&ItemRef::new(run_id), Some(job_id)).await.unwrap();
            assert!(!log.contains("##[error]"), "{job_id}'s log should not carry an error marker: {log}");
        }
    }

    #[test]
    fn growing_log_is_non_empty_and_grows_with_elapsed_time() {
        let at_start = unit_log_growing_at(0);
        assert!(!at_start.is_empty(), "the running job's log should never be empty");
        let later = unit_log_growing_at(60);
        assert!(later.len() >= at_start.len(), "the log should grow (or hold steady) as time passes");
        assert!(later.len() > at_start.len(), "60s in, the log should have grown past the first tick");
    }

    #[test]
    fn growing_log_keeps_growing_past_the_scripted_lines() {
        // Long after the scripted lines run out, it should still be producing fresh content
        // (the "still running" tail), not stalling.
        let mid = unit_log_growing_at(120);
        let far = unit_log_growing_at(600);
        assert!(far.len() > mid.len());
        assert!(far.contains("Still running…"));
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
        let states = src.available_states(&ItemRef::new("w1")).await.unwrap();
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
