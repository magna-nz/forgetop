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

/// A step in a GitHub Actions-style job: name, offset from the job's start (seconds), duration
/// (seconds), and status. The exact same list drives both the run's `PipelineStep`s and the
/// `##[group]`-sectioned log for the job (see [`gh_stage`]/[`gh_step_log`]), so a step can never
/// appear in the tree without a matching log section.
type GhStep = (&'static str, i64, i64, PipelineRunStatus);

/// The branch and PR most of the demo's `CI` runs build — forgetop's own recent history
/// (`a571ef7`/`1a4c3ae`, see `git log`), so the pane reads like a real self-hosted run.
const CI_BRANCH: &str = "claude/m4-action-palette-shortcut-f096cc";
const CI_PR: i64 = 195;

/// A passing `CI` run's steps — mirrors this repo's own `ci.yml`: checkout, Rust toolchain,
/// cache, Node, the dashboard build, then `cargo build`/`test`/`clippy`, followed by GitHub's
/// automatic post/cleanup steps. Totals ~101s.
fn gh_steps_ok() -> Vec<GhStep> {
    use PipelineRunStatus::Succeeded as Ok_;
    vec![
        ("Set up job", 0, 2, Ok_),
        ("Run actions/checkout@v4", 2, 1, Ok_),
        ("Run dtolnay/rust-toolchain@stable", 3, 6, Ok_),
        ("Run Swatinem/rust-cache@v2", 9, 9, Ok_),
        ("Run actions/setup-node@v4", 18, 6, Ok_),
        ("Install dashboard deps", 24, 3, Ok_),
        ("Dashboard tests", 27, 8, Ok_),
        ("Build dashboard SPA", 35, 7, Ok_),
        ("Build", 42, 26, Ok_),
        ("Test", 68, 19, Ok_),
        ("Clippy", 87, 11, Ok_),
        ("Post Run actions/setup-node@v4", 98, 0, Ok_),
        ("Post Run Swatinem/rust-cache@v2", 98, 1, Ok_),
        ("Post Run actions/checkout@v4", 99, 0, Ok_),
        ("Complete job", 99, 0, Ok_),
    ]
}

/// The same job, but `Test` fails on a (fictionalised) failure in this repo's own `cache` tests
/// and `Clippy` never runs. Totals ~88s.
fn gh_steps_failed() -> Vec<GhStep> {
    use PipelineRunStatus::{Canceled, Failed, Succeeded as Ok_};
    vec![
        ("Set up job", 0, 2, Ok_),
        ("Run actions/checkout@v4", 2, 1, Ok_),
        ("Run dtolnay/rust-toolchain@stable", 3, 6, Ok_),
        ("Run Swatinem/rust-cache@v2", 9, 9, Ok_),
        ("Run actions/setup-node@v4", 18, 5, Ok_),
        ("Install dashboard deps", 23, 3, Ok_),
        ("Dashboard tests", 26, 8, Ok_),
        ("Build dashboard SPA", 34, 7, Ok_),
        ("Build", 41, 27, Ok_),
        ("Test", 68, 19, Failed),
        ("Clippy", 87, 0, Canceled),
        ("Post Run actions/setup-node@v4", 87, 0, Ok_),
        ("Post Run Swatinem/rust-cache@v2", 87, 1, Ok_),
        ("Post Run actions/checkout@v4", 88, 0, Ok_),
        ("Complete job", 88, 0, Ok_),
    ]
}

/// Scales a step list's durations to a different run total (history runs vary in length),
/// keeping the same names/statuses and recomputing cumulative offsets.
fn scale_gh_steps(steps: &[GhStep], total_secs: i64, base_total: i64) -> Vec<GhStep> {
    if total_secs == base_total {
        return steps.to_vec();
    }
    let factor = total_secs as f64 / base_total as f64;
    let mut out = Vec::with_capacity(steps.len());
    let mut cursor = 0i64;
    for (name, _off, dur, status) in steps {
        let d = if *dur == 0 { 0 } else { ((*dur as f64) * factor).round().max(1.0) as i64 };
        out.push((*name, cursor, d, *status));
        cursor += d;
    }
    out
}

/// Builds the single "jobs" stage (GitHub's synthetic stage for a run with only one job) from a
/// step list, anchored to `run_started`.
fn gh_stage(run_started: DateTime<Utc>, job_id: &str, steps: &[GhStep]) -> Vec<PipelineStage> {
    let total = steps.iter().map(|(_, off, dur, _)| off + dur).max().unwrap_or(0);
    let pl_steps: Vec<PipelineStep> = steps
        .iter()
        .map(|(name, off, dur, status)| {
            let started = run_started + chrono::Duration::seconds(*off);
            PipelineStep {
                name: (*name).into(),
                status: *status,
                started_at: Some(started),
                finished_at: Some(started + chrono::Duration::seconds(*dur)),
            }
        })
        .collect();
    let failed = pl_steps.iter().any(|s| matches!(s.status, PipelineRunStatus::Failed));
    let job_status = if failed { PipelineRunStatus::Failed } else { PipelineRunStatus::Succeeded };
    vec![PipelineStage {
        name: "jobs".into(),
        status: job_status,
        jobs: vec![PipelineJob {
            id: job_id.into(),
            name: "build · test · clippy".into(),
            status: job_status,
            started_at: Some(run_started),
            finished_at: Some(run_started + chrono::Duration::seconds(total)),
            steps: pl_steps,
            url: Some(format!("https://example.test/job/{job_id}")),
            problem: failed.then(|| "Test failed (exit code 101)".to_string()),
        }],
    }]
}

/// The step list behind each GitHub-style `CI` run's single job, keyed by run id — the same
/// source both [`pipeline_runs`] and [`job_log`] read from, so a step's name in the tree always
/// has a matching `##[group]` section in the log.
fn ci_run_steps(run_id: &str) -> Option<Vec<GhStep>> {
    const BASE: i64 = 101;
    match run_id {
        "r493" => Some(scale_gh_steps(&gh_steps_ok(), 98, BASE)),
        "r494" => Some(scale_gh_steps(&gh_steps_ok(), 95, BASE)),
        "r495" => Some(scale_gh_steps(&gh_steps_ok(), 101, BASE)),
        "r496" => Some(gh_steps_failed()),
        "r497" => Some(scale_gh_steps(&gh_steps_ok(), 99, BASE)),
        "r498" => Some(scale_gh_steps(&gh_steps_ok(), 97, BASE)),
        "r499" => Some(gh_steps_ok()),
        "r500" => Some(gh_steps_failed()),
        _ => None,
    }
}

fn ci_run_stage(run_id: &str, run_started: DateTime<Utc>) -> Vec<PipelineStage> {
    ci_run_steps(run_id).map(|steps| gh_stage(run_started, &format!("job-{run_id}"), &steps)).unwrap_or_default()
}

/// Builds one GitHub-style `CI` run from its step list (looked up by `id` via [`ci_run_steps`]).
#[allow(clippy::too_many_arguments)]
fn ci_run(id: &str, number: i64, title: &str, branch: &str, pull_request: Option<i64>, event: &str, attempt: u32, commit: &str, who: User, started: DateTime<Utc>) -> PipelineRun {
    let stages = ci_run_stage(id, started);
    let job = stages.first().and_then(|s| s.jobs.first());
    let status = job.map(|j| j.status).unwrap_or(PipelineRunStatus::Succeeded);
    let finished = job.and_then(|j| j.finished_at);
    PipelineRun {
        event: Some(event.into()),
        attempt: Some(attempt),
        pull_request,
        repository: None,
        id: id.into(),
        definition_id: "ci".into(),
        number: Some(number),
        name: Some("CI Build".into()),
        title: Some(title.into()),
        status,
        triggered_by: Some(who),
        branch: Some(branch.into()),
        commit_sha: Some(commit.into()),
        started_at: Some(started),
        finished_at: finished,
        url: Some("https://ci.example.com/runs/demo".into()),
        stages,
    }
}

/// One job in the release's `build-local-artifacts` matrix, or one of the jobs queued behind it.
struct ReleaseJob {
    id: &'static str,
    name: &'static str,
    offset: i64,
    /// Duration in seconds; `None` for a job that hasn't started (queued).
    dur: Option<i64>,
    status: PipelineRunStatus,
}

fn release_jobs_to_pipeline(run_started: DateTime<Utc>, legs: &[ReleaseJob]) -> Vec<PipelineJob> {
    legs.iter()
        .map(|j| {
            let queued = matches!(j.status, PipelineRunStatus::Queued);
            let started_at = (!queued).then(|| run_started + chrono::Duration::seconds(j.offset));
            let finished_at = if matches!(j.status, PipelineRunStatus::Running | PipelineRunStatus::Queued) {
                None
            } else {
                j.dur.map(|d| run_started + chrono::Duration::seconds(j.offset + d))
            };
            PipelineJob {
                id: j.id.into(),
                name: j.name.into(),
                status: j.status,
                started_at,
                finished_at,
                steps: vec![],
                url: Some(format!("https://example.test/job/{}", j.id)),
                problem: None,
            }
        })
        .collect()
}

/// Compact (stage-less) run builder for the secondary CI providers.
#[allow(clippy::too_many_arguments)]
fn run(id: &str, def: &str, num: i64, name: &str, title: &str, status: PipelineRunStatus, branch: &str, who: User, updated_h: i64) -> PipelineRun {
    let now = base();
    let started = now - chrono::Duration::hours(updated_h);
    PipelineRun {
        event: Some("push".into()),
        attempt: Some(1),
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
    // Triggered by the open MR on this branch (northwind-infra !312).
    let mut r9902 = run("gl-9902", "gl-pipeline", 9902, "#9902", "Add a Postgres read replica", PipelineRunStatus::Running, "infra/read-replica", me(), 1);
    r9902.pull_request = Some(312);
    vec![r9902, run("gl-9901", "gl-pipeline", 9901, "#9901", "Cache the customer risk score", PipelineRunStatus::Succeeded, "main", alice(), 6)]
}
fn bitbucket_pipeline_defs() -> Vec<PipelineDefinition> {
    vec![PipelineDefinition { repository: None, id: "bb-default".into(), name: "Deploy to Staging".into(), path: Some("bitbucket-pipelines.yml".into()), url: None }]
}
fn bitbucket_runs() -> Vec<PipelineRun> {
    // Triggered by PR #64 ("dbt: add revenue recognition model").
    let mut r441 = run("bb-441", "bb-default", 441, "#441", "dbt: add revenue recognition model", PipelineRunStatus::Failed, "feat/rev-rec", me(), 2);
    r441.event = Some("pull_request".into());
    r441.pull_request = Some(64);
    vec![r441, run("bb-440", "bb-default", 440, "#440", "Tighten CORS on the admin API", PipelineRunStatus::Succeeded, "main", dev(), 10)]
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

    // `CI` — GitHub-style single-job runs, seven of them on the same PR branch (a mix of
    // succeeded and one failed, varying durations) so a history strip/median has something to
    // show, plus one dedicated failed run (a different branch/PR) with the full annotations /
    // rerun / logs demonstration, and one queued nightly run.
    let ci_runs = vec![
        ci_run("r493", 436, "Add per-repo cache eviction metrics", CI_BRANCH, Some(CI_PR), "push", 1, "5c1a9e2", carol(), now - chrono::Duration::days(4) - chrono::Duration::seconds(98)),
        ci_run("r494", 437, "Fix flaky rewrite_edits_in_place ordering", CI_BRANCH, Some(CI_PR), "push", 1, "7d4f0b1", bob(), now - chrono::Duration::days(3) - chrono::Duration::seconds(95)),
        ci_run("r495", 439, "Tighten the action-palette shortcut help text", CI_BRANCH, Some(CI_PR), "push", 1, "b2c8a91", alice(), now - chrono::Duration::days(1) - chrono::Duration::seconds(101)),
        ci_run("r496", 440, "Leave every footer with Ctrl-K search anywhere", CI_BRANCH, Some(CI_PR), "push", 1, "e91f3d6", me(), now - chrono::Duration::hours(9) - chrono::Duration::seconds(88)),
        ci_run("r497", 441, "Leave visible-tabs (v) to help and the palette", CI_BRANCH, Some(CI_PR), "push", 2, "14944b9", me(), now - chrono::Duration::hours(3) - chrono::Duration::seconds(99)),
        ci_run("r498", 442, "Merge pull request #195 from magna-nz/claude/m4-action-palette-shortcut-f096cc", CI_BRANCH, Some(CI_PR), "push", 1, "a571ef7", me(), now - chrono::Duration::minutes(41) - chrono::Duration::seconds(97)),
        ci_run("r499", 443, "feat(tui): lead every footer with a yellow Ctrl-K search anywhere", CI_BRANCH, Some(CI_PR), "push", 1, "1a4c3ae", me(), now - chrono::Duration::minutes(12) - chrono::Duration::seconds(101)),
        ci_run(
            "r500",
            438,
            "Cache rewrite: edit entries in place instead of refusing same-timestamp writes",
            "claude/cache-rewrite-7ab2",
            Some(198),
            "pull_request",
            1,
            "9c0e1d2",
            me(),
            now - chrono::Duration::days(2) - chrono::Duration::seconds(88),
        ),
        PipelineRun {
            event: Some("schedule".into()),
            attempt: Some(1),
            pull_request: None,
            repository: None,
            id: "r502".into(),
            definition_id: "ci".into(),
            number: Some(444),
            name: Some("CI Build".into()),
            title: Some("Nightly CI".into()),
            status: PipelineRunStatus::Queued,
            triggered_by: Some(alice()),
            branch: Some("main".into()),
            commit_sha: Some("cafe123".into()),
            started_at: None,
            finished_at: None,
            url: Some("https://ci.example.com/runs/demo".into()),
            stages: vec![],
        },
    ];

    // `CD (Release)` — one running release (a `build-local-artifacts` matrix across four
    // targets, two legs still in flight, four jobs queued behind it) plus three previous runs
    // of the same definition with the same job names, for per-job duration estimates: two
    // succeeded (one of them — #56 — the artifacts demonstration) and one failed three weeks
    // back (Windows).
    let r501_started = now - chrono::Duration::seconds(252);
    let r501_elapsed = 252i64;
    let r501 = PipelineRun {
        event: Some("push".into()),
        attempt: Some(1),
        pull_request: None,
        repository: None,
        id: "r501".into(),
        definition_id: "release".into(),
        number: Some(57),
        name: Some("v1.2.1".into()),
        title: Some("release: 1.2.1".into()),
        status: PipelineRunStatus::Running,
        triggered_by: Some(me()),
        branch: Some("v1.2.1".into()),
        commit_sha: Some("f62f3c5".into()),
        started_at: Some(r501_started),
        finished_at: None,
        url: Some("https://ci.example.com/runs/demo".into()),
        stages: vec![PipelineStage {
            name: "jobs".into(),
            status: PipelineRunStatus::Running,
            jobs: release_jobs_to_pipeline(
                r501_started,
                &[
                    ReleaseJob { id: "mj-plan", name: "plan", offset: 0, dur: Some(18), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-aarch64", name: "build-local-artifacts (aarch64-apple-darwin)", offset: 20, dur: Some(182), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-x86mac", name: "build-local-artifacts (x86_64-apple-darwin)", offset: 20, dur: Some(200), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-linux", name: "build-local-artifacts (x86_64-unknown-linux-gnu)", offset: 21, dur: Some(r501_elapsed - 21), status: PipelineRunStatus::Running },
                    ReleaseJob { id: "mj-win", name: "build-local-artifacts (x86_64-pc-windows-msvc)", offset: 22, dur: Some(r501_elapsed - 22), status: PipelineRunStatus::Running },
                    ReleaseJob { id: "mj-global", name: "build-global-artifacts", offset: 342, dur: None, status: PipelineRunStatus::Queued },
                    ReleaseJob { id: "mj-host", name: "host", offset: 362, dur: None, status: PipelineRunStatus::Queued },
                    ReleaseJob { id: "mj-homebrew", name: "publish-homebrew-formula", offset: 384, dur: None, status: PipelineRunStatus::Queued },
                    ReleaseJob { id: "mj-announce", name: "announce", offset: 398, dur: None, status: PipelineRunStatus::Queued },
                ],
            ),
        }],
    };

    let r207_started = now - chrono::Duration::days(12);
    let r207 = PipelineRun {
        event: Some("push".into()),
        attempt: Some(1),
        pull_request: None,
        repository: None,
        id: "r207".into(),
        definition_id: "release".into(),
        number: Some(56),
        name: Some("v1.2.0".into()),
        title: Some("release: 1.2.0".into()),
        status: PipelineRunStatus::Succeeded,
        triggered_by: Some(carol()),
        branch: Some("v1.2.0".into()),
        commit_sha: Some("3be90a1".into()),
        started_at: Some(r207_started),
        finished_at: Some(r207_started + chrono::Duration::seconds(411)),
        url: Some("https://ci.example.com/runs/demo".into()),
        stages: vec![PipelineStage {
            name: "jobs".into(),
            status: PipelineRunStatus::Succeeded,
            jobs: release_jobs_to_pipeline(
                r207_started,
                &[
                    ReleaseJob { id: "mj-plan", name: "plan", offset: 0, dur: Some(18), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-aarch64", name: "build-local-artifacts (aarch64-apple-darwin)", offset: 20, dur: Some(178), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-x86mac", name: "build-local-artifacts (x86_64-apple-darwin)", offset: 20, dur: Some(196), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-linux", name: "build-local-artifacts (x86_64-unknown-linux-gnu)", offset: 20, dur: Some(213), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-win", name: "build-local-artifacts (x86_64-pc-windows-msvc)", offset: 20, dur: Some(318), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-global", name: "build-global-artifacts", offset: 340, dur: Some(21), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-host", name: "host", offset: 362, dur: Some(22), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-homebrew", name: "publish-homebrew-formula", offset: 385, dur: Some(14), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-announce", name: "announce", offset: 400, dur: Some(11), status: PipelineRunStatus::Succeeded },
                ],
            ),
        }],
    };

    let r206_started = now - chrono::Duration::days(20);
    let r206 = PipelineRun {
        event: Some("manual".into()),
        attempt: Some(1),
        pull_request: None,
        repository: None,
        id: "r206".into(),
        definition_id: "release".into(),
        number: Some(55),
        name: Some("v1.1.9".into()),
        title: Some("release: 1.1.9".into()),
        status: PipelineRunStatus::Succeeded,
        triggered_by: Some(me()),
        branch: Some("v1.1.9".into()),
        commit_sha: Some("6e2a410".into()),
        started_at: Some(r206_started),
        finished_at: Some(r206_started + chrono::Duration::seconds(408)),
        url: Some("https://ci.example.com/runs/demo".into()),
        stages: vec![PipelineStage {
            name: "jobs".into(),
            status: PipelineRunStatus::Succeeded,
            jobs: release_jobs_to_pipeline(
                r206_started,
                &[
                    ReleaseJob { id: "mj-plan", name: "plan", offset: 0, dur: Some(17), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-aarch64", name: "build-local-artifacts (aarch64-apple-darwin)", offset: 19, dur: Some(185), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-x86mac", name: "build-local-artifacts (x86_64-apple-darwin)", offset: 19, dur: Some(198), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-linux", name: "build-local-artifacts (x86_64-unknown-linux-gnu)", offset: 19, dur: Some(208), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-win", name: "build-local-artifacts (x86_64-pc-windows-msvc)", offset: 19, dur: Some(315), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-global", name: "build-global-artifacts", offset: 337, dur: Some(20), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-host", name: "host", offset: 358, dur: Some(23), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-homebrew", name: "publish-homebrew-formula", offset: 382, dur: Some(15), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-announce", name: "announce", offset: 398, dur: Some(10), status: PipelineRunStatus::Succeeded },
                ],
            ),
        }],
    };

    let r205_started = now - chrono::Duration::weeks(3);
    let r205 = PipelineRun {
        event: Some("push".into()),
        attempt: Some(1),
        pull_request: None,
        repository: None,
        id: "r205".into(),
        definition_id: "release".into(),
        number: Some(54),
        name: Some("v1.1.8".into()),
        title: Some("release: 1.1.8".into()),
        status: PipelineRunStatus::Failed,
        triggered_by: Some(bob()),
        branch: Some("v1.1.8".into()),
        commit_sha: Some("2ab7f04".into()),
        started_at: Some(r205_started),
        finished_at: Some(r205_started + chrono::Duration::seconds(235)),
        url: Some("https://ci.example.com/runs/demo".into()),
        stages: vec![PipelineStage {
            name: "jobs".into(),
            status: PipelineRunStatus::Failed,
            jobs: release_jobs_to_pipeline(
                r205_started,
                &[
                    ReleaseJob { id: "mj-plan", name: "plan", offset: 0, dur: Some(18), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-aarch64", name: "build-local-artifacts (aarch64-apple-darwin)", offset: 20, dur: Some(175), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-x86mac", name: "build-local-artifacts (x86_64-apple-darwin)", offset: 20, dur: Some(190), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-linux", name: "build-local-artifacts (x86_64-unknown-linux-gnu)", offset: 20, dur: Some(205), status: PipelineRunStatus::Succeeded },
                    ReleaseJob { id: "mj-win", name: "build-local-artifacts (x86_64-pc-windows-msvc)", offset: 20, dur: Some(40), status: PipelineRunStatus::Failed },
                ],
            ),
        }],
    };

    ci_runs.into_iter().chain([r501, r207, r206, r205]).collect()
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

/// Believable per-job demo logs. `run_id` disambiguates job ids reused across several release
/// runs (`mj-plan`, `mj-win`, …); `job_id` alone picks the `CI` GitHub-style jobs (`job-r499`,
/// …), which are unique per run. Fixed timestamps keep everything deterministic except
/// [`unit_log_growing`], the one job (`r501`/`mj-linux`) that stays Running for the life of the
/// `--demo` session.
fn job_log(run_id: &str, job_id: &str) -> String {
    if job_id.starts_with("job-") {
        if let Some(steps) = ci_run_steps(run_id) {
            return gh_step_log(&steps);
        }
    }
    if run_id == "r501" && job_id == "mj-linux" {
        return unit_log_growing();
    }
    match job_id {
        "mj-plan" => release_plan_log(),
        "mj-aarch64" => matrix_leg_log("aarch64-apple-darwin"),
        "mj-x86mac" => matrix_leg_log("x86_64-apple-darwin"),
        "mj-linux" => matrix_leg_log("x86_64-unknown-linux-gnu"),
        "mj-win" if run_id == "r205" => matrix_leg_log_failed("x86_64-pc-windows-msvc"),
        "mj-win" if run_id == "r501" => matrix_leg_log_running("x86_64-pc-windows-msvc"),
        "mj-win" => matrix_leg_log("x86_64-pc-windows-msvc"),
        "mj-global" => release_downstream_log("build-global-artifacts"),
        "mj-host" => release_downstream_log("host"),
        "mj-homebrew" => release_downstream_log("publish-homebrew-formula"),
        "mj-announce" => release_downstream_log("announce"),
        other => generic_log(other),
    }
}

/// Plausible output for one step of a GitHub-style `CI` job, keyed by the step's name so it
/// reads as if it actually ran that command.
fn gh_step_body(name: &str, status: PipelineRunStatus) -> String {
    match name {
        "Set up job" => "Current runner version: '2.319.1'\nOperating System: Ubuntu 22.04.4 LTS\nRunner Image: 'ubuntu-22.04:20240701.1.0'\n".into(),
        "Run actions/checkout@v4" => "Syncing repository: magna-nz/forgetop\nGetting Git version info\nCopying '/usr/bin/git'\nSetting up auth\n".into(),
        "Run dtolnay/rust-toolchain@stable" => "info: syncing channel updates for 'stable-x86_64-unknown-linux-gnu'\ninfo: latest update on stable, rust version 1.82.0\ninfo: downloading component 'clippy'\n".into(),
        "Run Swatinem/rust-cache@v2" => "Cache restored successfully\nCache Size: ~412 MB\n".into(),
        "Run actions/setup-node@v4" => "Found in cache @ /opt/hostedtoolcache/node/20.15.1/x64\nEnvironment details\n  node: v20.15.1\n  npm: 10.7.0\n".into(),
        "Install dashboard deps" => "added 412 packages in 2.4s\n".into(),
        "Dashboard tests" => " Test Files  14 passed (14)\n      Tests  86 passed (86)\n   Start at  09:14:02\n   Duration  7.91s\n".into(),
        "Build dashboard SPA" => "vite v5.3.1 building for production...\n✓ 812 modules transformed.\ndist/assets/index-4f2a9c1e.js  318.42 kB │ gzip: 98.11 kB\n✓ built in 6.44s\n".into(),
        "Build" => "   Compiling forgetop-core v1.2.1 (/home/runner/work/forgetop/forgetop/crates/forgetop-core)\n   Compiling forgetop-providers v1.2.1 (/home/runner/work/forgetop/forgetop/crates/forgetop-providers)\n   Compiling forgetop-server v1.2.1 (/home/runner/work/forgetop/forgetop/crates/forgetop-server)\n   Compiling forgetop-tui v1.2.1 (/home/runner/work/forgetop/forgetop/crates/forgetop-tui)\n   Compiling forgetop v1.2.1 (/home/runner/work/forgetop/forgetop/crates/forgetop-cli)\n    Finished `release` profile [optimized] target(s) in 25.87s\n".into(),
        "Test" if matches!(status, PipelineRunStatus::Failed) => rust_test_failure_block(),
        "Test" => {
            "running 184 tests\ntest cache::tests::put_refuses_same_timestamp ... ok\ntest cache::tests::rewrite_edits_in_place ... ok\ntest repo::tests::to_connection_relative_strips_host ... ok\n\ntest result: ok. 184 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 18.62s\n"
                .into()
        }
        "Clippy" if matches!(status, PipelineRunStatus::Canceled) => "The job was canceled before this step ran.\n".into(),
        "Clippy" => "    Checking forgetop-cli v1.2.1\n    Finished checking in 10.91s\nwarning: `forgetop` (bin) generated 0 warnings\n".into(),
        n if n.starts_with("Post Run") => "Post job cleanup.\n".into(),
        "Complete job" => "Cleaning up orphan processes\n".into(),
        _ => "ok\n".into(),
    }
}

/// The `Test` step's failure block for the failed `CI` runs — a real (fictionalised) failure in
/// this repo's own `cache` tests, with the panic location, assertion, and summary a matcher can
/// find, plus the trailing `##[error]` line GitHub Actions appends on a non-zero exit.
fn rust_test_failure_block() -> String {
    concat!(
        "running 184 tests\n",
        "test cache::tests::put_refuses_same_timestamp ... ok\n",
        "test cache::tests::rewrite_edits_in_place ... FAILED\n",
        "test repo::tests::to_connection_relative_strips_host ... ok\n",
        "\n",
        "failures:\n",
        "\n",
        "---- cache::tests::rewrite_edits_in_place stdout ----\n",
        "\n",
        "thread 'cache::tests::rewrite_edits_in_place' panicked at crates/forgetop-core/src/cache.rs:212:9:\n",
        "assertion `left == right` failed\n",
        "  left: 3\n",
        " right: 4\n",
        "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n",
        "\n",
        "failures:\n",
        "    cache::tests::rewrite_edits_in_place\n",
        "\n",
        "test result: FAILED. 183 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 19.03s\n",
        "\n",
        "##[error]Process completed with exit code 101.\n",
    )
    .to_string()
}

/// Wraps a GitHub-style job's steps as `##[group]<name>` … `##[endgroup]` sections, in order —
/// the same shape GitHub Actions' raw log takes, so a client-side parser has something real to
/// split on.
fn gh_step_log(steps: &[GhStep]) -> String {
    let mut out = String::new();
    for (name, _off, _dur, status) in steps {
        out.push_str(&format!("##[group]{name}\n"));
        out.push_str(&gh_step_body(name, *status));
        out.push_str("##[endgroup]\n");
    }
    out
}

fn release_plan_log() -> String {
    "Resolving the 1.2.1 release plan...\nChangelog: CHANGELOG.md (12 entries since 1.2.0)\nTargets: aarch64-apple-darwin, x86_64-apple-darwin, x86_64-unknown-linux-gnu, x86_64-pc-windows-msvc\n".into()
}

fn matrix_leg_log(target: &str) -> String {
    format!(
        "Downloading crates ...\n   Compiling forgetop-core v1.2.1\n   Compiling forgetop-providers v1.2.1\n   Compiling forgetop-cli v1.2.1\n    Finished `release` profile [optimized] target(s) in 2m41s\nStripping symbols from target/{target}/release/forgetop\nPackaging forgetop-{target}.tar.xz\nUploading forgetop-{target}.tar.xz\n"
    )
}

fn matrix_leg_log_running(target: &str) -> String {
    format!("Downloading crates ...\n   Compiling forgetop-core v1.2.1\n   Compiling forgetop-providers v1.2.1\n   Compiling forgetop-cli v1.2.1\nStill building target/{target}/release/forgetop ...\n")
}

fn matrix_leg_log_failed(target: &str) -> String {
    format!(
        "Downloading crates ...\n   Compiling forgetop-core v1.2.1\n   Compiling forgetop-providers v1.2.1\nerror[E0433]: failed to resolve: use of undeclared crate `winapi`\n  --> crates/forgetop-tui/src/platform/windows.rs:9:5\nerror: could not compile `forgetop-tui` (lib) due to 1 previous error\n##[error]Process completed with exit code 101. ({target})\n"
    )
}

fn release_downstream_log(name: &str) -> String {
    format!("Waiting on the build-local-artifacts matrix...\nRunning {name}...\nDone.\n")
}

/// A plausible-but-generic log for a job id we don't have a scripted log for.
fn generic_log(job_id: &str) -> String {
    let mut out = format!("09:30:00  Starting job {job_id}...\n");
    for i in 1..=18 {
        out.push_str(&format!("09:30:{i:02}  step output line {i}\n"));
    }
    out.push_str("09:30:19  Done. All steps completed successfully.\n");
    out
}

/// Process-wide start instant for the demo's one "live" job (`r501`/`mj-linux`, the matrix leg
/// that stays Running for the life of the `--demo` session) — [`unit_log_growing`] measures
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

/// The growing log for the still-running matrix leg (`r501`/`mj-linux`) — grows with real
/// elapsed time since `--demo` started (see [`demo_start`]), so a "follow mode" feature has
/// something to follow.
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
            .map(apply_pipe_rerun)
            .collect())
    }
    async fn get_run(&self, run: &ItemRef) -> Result<PipelineRun> {
        let run_id: &str = &run.id;
        pipeline_runs_for(&self.conn)
            .into_iter()
            .find(|r| r.id == run_id)
            .map(apply_pipe_cancel)
            .map(apply_pipe_rerun)
            .ok_or_else(|| forgetop_core::Error::NotFound(run_id.into()))
    }
    async fn logs(&self, run: &ItemRef, job_id: Option<&str>) -> Result<String> {
        let run_id: &str = &run.id;
        if let Some(job) = job_id {
            let mut out = format!("=== logs for run {run_id} · {job} ===\n");
            out.push_str(&job_log(run_id, job));
            return Ok(out);
        }
        // No job specified: a sensible whole-run log — concatenate each job's log in order.
        if ci_run_steps(run_id).is_some() {
            let job = format!("job-{run_id}");
            let mut out = format!("=== job {job} ===\n");
            out.push_str(&job_log(run_id, &job));
            return Ok(out);
        }
        let job_ids: &[&str] = match run_id {
            "r501" | "r207" | "r206" | "r205" => &["mj-plan", "mj-aarch64", "mj-x86mac", "mj-linux", "mj-win", "mj-global", "mj-host", "mj-homebrew", "mj-announce"],
            _ => &[],
        };
        if job_ids.is_empty() {
            let mut out = format!("=== logs for run {run_id} ===\n");
            out.push_str(&job_log(run_id, run_id));
            return Ok(out);
        }
        let mut out = String::new();
        for job in job_ids {
            out.push_str(&format!("=== job {job} ===\n"));
            out.push_str(&job_log(run_id, job));
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
    fn supports_rerun(&self) -> bool {
        true
    }
    fn supports_rerun_failed(&self) -> bool {
        true
    }
    async fn rerun_run(&self, run: &ItemRef, failed_only: bool) -> Result<Option<String>> {
        // Like GitHub: the same run is re-queued as its next attempt.
        let run_id: &str = &run.id;
        rerun_state().lock().unwrap().insert(run_id.to_string(), RerunState { failed_only, at: Utc::now() });
        Ok(None)
    }
    fn supports_artifacts(&self) -> bool {
        true
    }
    async fn artifacts(&self, run: &ItemRef) -> Result<Vec<PipelineArtifact>> {
        let run_id: &str = &run.id;
        // Only the finished release run (#56) published artifacts — everything else reports none
        // rather than making up a fake publish step for a `CI` run.
        if run_id != "r207" {
            return Ok(Vec::new());
        }
        let expires_at = Some(base() + chrono::Duration::days(83));
        let art = |name: &str, size_bytes: u64| PipelineArtifact {
            id: format!("art-{name}"),
            name: name.into(),
            size_bytes: Some(size_bytes),
            expires_at,
            url: Some(format!("https://example.test/releases/v1.2.0/{name}")),
        };
        Ok(vec![
            art("forgetop-aarch64-apple-darwin.tar.xz", 7_900_000),
            art("forgetop-x86_64-apple-darwin.tar.xz", 8_300_000),
            art("forgetop-x86_64-unknown-linux-gnu.tar.xz", 8_600_000),
            art("forgetop-x86_64-pc-windows-msvc.zip", 8_100_000),
            art("forgetop-installer.sh", 18_000),
            art("forgetop-installer.ps1", 21_000),
            art("sha256.sum", 1_000),
        ])
    }
    async fn annotations(&self, run: &ItemRef) -> Result<Vec<PipelineAnnotation>> {
        let run_id: &str = &run.id;
        // Only the headline failed run (#438) carries provider annotations — a Failure anchored
        // to the real panic site, a Warning on an unrelated file, and a Notice.
        if run_id != "r500" {
            return Ok(Vec::new());
        }
        let job_id = Some("job-r500".to_string());
        Ok(vec![
            PipelineAnnotation {
                level: AnnotationLevel::Failure,
                message: "assertion `left == right` failed".into(),
                title: Some("cache::tests::rewrite_edits_in_place".into()),
                path: Some("crates/forgetop-core/src/cache.rs".into()),
                line: Some(212),
                job_id: job_id.clone(),
            },
            PipelineAnnotation {
                level: AnnotationLevel::Warning,
                message: "'status' is possibly 'undefined'".into(),
                title: None,
                path: Some("crates/forgetop-server/web/src/components/Pipelines.tsx".into()),
                line: Some(88),
                job_id: job_id.clone(),
            },
            PipelineAnnotation {
                level: AnnotationLevel::Notice,
                message: "Node.js 16 actions are deprecated. Please update the following actions to use Node.js 20: actions/cache@v3".into(),
                title: None,
                path: None,
                line: None,
                job_id,
            },
        ])
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
        // The running release (#57) waits on a production deployment gate you can act on.
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

/// A session `rerun_run` request, keyed by run id: whether it asked for the whole run or just
/// the jobs that failed.
#[derive(Clone, Copy)]
struct RerunState {
    failed_only: bool,
    /// When the rerun was asked for — the new attempt's start.
    at: DateTime<Utc>,
}

fn rerun_state() -> &'static Mutex<HashMap<String, RerunState>> {
    static STORE: OnceLock<Mutex<HashMap<String, RerunState>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Fold a session rerun onto a freshly-built run, like a real provider re-queuing it: the
/// attempt bumps, the finish time clears, and the run (and, with `failed_only`, just its failed
/// jobs — a job that never failed is left exactly as it was) goes back to Queued.
fn apply_pipe_rerun(mut run: PipelineRun) -> PipelineRun {
    let Some(state) = rerun_state().lock().unwrap().get(&run.id).copied() else {
        return run;
    };
    run.attempt = Some(run.attempt.unwrap_or(1) + 1);
    run.started_at = Some(state.at);
    run.finished_at = None;
    run.status = PipelineRunStatus::Queued;
    for stage in run.stages.iter_mut() {
        for job in stage.jobs.iter_mut() {
            if state.failed_only && !matches!(job.status, PipelineRunStatus::Failed) {
                continue;
            }
            job.status = PipelineRunStatus::Queued;
            job.finished_at = None;
            for s in job.steps.iter_mut() {
                s.status = PipelineRunStatus::Queued;
                s.finished_at = None;
            }
        }
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
        let stage = run.stages.iter().find(|s| s.name == "jobs").unwrap();
        let job = stage.jobs.first().expect("the GitHub-style run has a single job");
        assert!(job.steps.iter().any(|s| s.name == "Test" && matches!(s.status, PipelineRunStatus::Failed)));
        assert!(job.steps.iter().all(|s| s.started_at.is_some() && s.finished_at.is_some()), "every step is timestamped");
    }

    #[tokio::test]
    async fn failed_job_log_has_an_error_marker() {
        let src = conn().pipelines().unwrap();
        let log = src.logs(&ItemRef::new("r500"), Some("job-r500")).await.unwrap();
        assert!(log.contains("##[error]"), "failing job's log should carry an error marker: {log}");
        assert!(log.contains("rewrite_edits_in_place"), "keeps the gist of the original failure");
        assert!(log.contains("test result: FAILED. 183 passed; 1 failed"), "a realistic failure summary line");
        assert!(log.contains("crates/forgetop-core/src/cache.rs:212:9"), "the panic location is in the log");
    }

    #[tokio::test]
    async fn succeeded_job_logs_have_no_error_marker() {
        let src = conn().pipelines().unwrap();
        for (run_id, job_id) in [("r499", "job-r499"), ("r207", "mj-plan"), ("r207", "mj-aarch64")] {
            let log = src.logs(&ItemRef::new(run_id), Some(job_id)).await.unwrap();
            assert!(!log.contains("##[error]"), "{job_id}'s log should not carry an error marker: {log}");
        }
    }

    #[test]
    fn every_gh_style_step_has_a_matching_log_group() {
        for run_id in ["r493", "r494", "r495", "r496", "r497", "r498", "r499", "r500"] {
            let steps = ci_run_steps(run_id).unwrap();
            let log = gh_step_log(&steps);
            for (name, _, _, _) in &steps {
                assert!(log.contains(&format!("##[group]{name}\n")), "{run_id}: missing a log group for step {name:?}");
            }
        }
    }

    #[tokio::test]
    async fn annotations_are_only_on_the_headline_failed_run() {
        let src = conn().pipelines().unwrap();
        let anns = src.annotations(&ItemRef::new("r500")).await.unwrap();
        assert_eq!(anns.len(), 3);
        assert!(anns.iter().any(|a| a.level == AnnotationLevel::Failure
            && a.path.as_deref() == Some("crates/forgetop-core/src/cache.rs")
            && a.line == Some(212)));
        assert!(anns.iter().any(|a| a.level == AnnotationLevel::Warning));
        assert!(anns.iter().any(|a| a.level == AnnotationLevel::Notice));
        assert!(anns.iter().all(|a| a.job_id.as_deref() == Some("job-r500")));

        assert!(src.annotations(&ItemRef::new("r499")).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn artifacts_are_only_on_the_finished_release_run() {
        let src = conn().pipelines().unwrap();
        assert!(src.supports_artifacts());
        let arts = src.artifacts(&ItemRef::new("r207")).await.unwrap();
        assert_eq!(arts.len(), 7);
        assert!(arts.iter().all(|a| a.size_bytes.is_some() && a.expires_at.is_some()));
        assert!(arts.iter().all(|a| a.url.as_deref().is_some_and(|u| u.starts_with("https://"))));

        assert!(src.artifacts(&ItemRef::new("r501")).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rerun_bumps_the_attempt_and_requeues_the_whole_run() {
        let src = DemoPipe { conn: "github".into() };
        assert!(src.supports_rerun() && src.supports_rerun_failed());
        // A fabricated id so the session-global rerun store can't pollute other tests reading
        // the real demo runs (r500 in particular).
        let id = "demo-rerun-test-whole";
        src.rerun_run(&ItemRef::new(id), false).await.unwrap();

        let mut run = ci_run("r500", 1, "t", "b", None, "push", 1, "abc", me(), base());
        run.id = id.to_string();
        let before_attempt = run.attempt;
        let after = apply_pipe_rerun(run);

        assert_eq!(after.attempt, before_attempt.map(|a| a + 1));
        assert_eq!(after.status, PipelineRunStatus::Queued);
        assert!(after.finished_at.is_none());
        assert!(after.stages.iter().flat_map(|s| &s.jobs).all(|j| matches!(j.status, PipelineRunStatus::Queued)), "a full rerun requeues every job");
        assert!(after.stages.iter().flat_map(|s| &s.jobs).flat_map(|j| &j.steps).all(|s| matches!(s.status, PipelineRunStatus::Queued)));
    }

    #[tokio::test]
    async fn rerun_failed_only_leaves_jobs_that_never_failed_alone() {
        let src = DemoPipe { conn: "github".into() };
        let id = "demo-rerun-test-failed-only";
        src.rerun_run(&ItemRef::new(id), true).await.unwrap();

        let now = base();
        let job = |id: &str, status: PipelineRunStatus| PipelineJob {
            id: id.into(),
            name: id.into(),
            status,
            started_at: Some(now),
            finished_at: Some(now),
            steps: vec![],
            url: None,
            problem: None,
        };
        let run = PipelineRun {
            event: None,
            attempt: Some(1),
            pull_request: None,
            repository: None,
            id: id.into(),
            definition_id: "release".into(),
            number: Some(1),
            name: None,
            title: None,
            status: PipelineRunStatus::Failed,
            triggered_by: None,
            branch: None,
            commit_sha: None,
            started_at: Some(now),
            finished_at: Some(now),
            url: None,
            stages: vec![PipelineStage { name: "jobs".into(), status: PipelineRunStatus::Failed, jobs: vec![job("a", PipelineRunStatus::Succeeded), job("b", PipelineRunStatus::Failed)] }],
        };

        let after = apply_pipe_rerun(run);
        assert!(matches!(after.stages[0].jobs[0].status, PipelineRunStatus::Succeeded), "the job that didn't fail is untouched");
        assert!(matches!(after.stages[0].jobs[1].status, PipelineRunStatus::Queued), "the failed job is requeued");
        assert_eq!(after.attempt, Some(2));
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
