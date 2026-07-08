//! GitHub live read-path tests (Wave 1): prove auth, base URLs, pagination
//! envelopes, and JSON decoding against the real API. Fixture creation + write
//! paths (PR merge, issue state, environment approvals) arrive in Wave 2.

use forgetop_core::domain::*;
use forgetop_core::provider::*;

use crate::gh_raw::GhRaw;
use crate::harness;

#[tokio::test]
async fn github_connectivity_check_passes() {
    let gh = skip_if_none!(harness::github(), "github");
    assert!(gh.conn.check().await, "the token should authenticate against {}/{}", gh.owner, gh.repo);
    eprintln!("github: connected to {}/{}", gh.owner, gh.repo);
}

#[tokio::test]
async fn github_lists_pull_requests() {
    let gh = skip_if_none!(harness::github(), "github");
    let prs = gh.conn.pull_requests().expect("github advertises pull requests");
    // Decoding the list envelope is the assertion; an empty repo returns [].
    let list = prs.list(&PullRequestQuery::default()).await.expect("list pull requests");
    eprintln!("github: {} open PR(s)", list.len());
    // If any exist, get + threads must decode too.
    if let Some(pr) = list.first() {
        let got = prs.get(&pr.id).await.expect("get a listed PR");
        assert_eq!(got.id, pr.id);
        prs.threads(&pr.id).await.expect("decode PR threads");
    }
}

#[tokio::test]
async fn github_lists_work_items() {
    let gh = skip_if_none!(harness::github(), "github");
    let wi = gh.conn.work_items().expect("github advertises work items");
    let list = wi.list(&WorkItemQuery::default()).await.expect("list work items");
    eprintln!("github: {} work item(s)", list.len());
    if let Some(item) = list.first() {
        let got = wi.get(&item.id).await.expect("get a listed work item");
        assert_eq!(got.id, item.id);
    }
}

#[tokio::test]
async fn github_lists_pipeline_runs_and_supports_approvals() {
    let gh = skip_if_none!(harness::github(), "github");
    let pipe = gh.conn.pipelines().expect("github advertises pipelines");
    assert!(pipe.supports_approvals(), "github should support approvals");
    let runs = pipe.list_runs(&PipelineRunQuery::default()).await.expect("list pipeline runs");
    eprintln!("github: {} pipeline run(s)", runs.len());
    if let Some(run) = runs.first() {
        let got = pipe.get_run(&run.id).await.expect("get a listed run");
        assert_eq!(got.id, run.id);
        // A finished run just returns [] — this only checks the call decodes.
        pipe.pending_approvals(&run.id).await.expect("decode pending approvals");
    }
}

// ---- Wave 2: create → read/act → teardown (writes) ----

#[tokio::test]
async fn github_pull_request_lifecycle() {
    let gh = skip_if_none!(harness::github(), "github");
    let raw = GhRaw::from_env().expect("github raw client");
    harness::maybe_sweep(raw.sweep()).await;
    let prefix = harness::run_prefix();

    // Fixture: a branch with one file, opened as a PR against the default branch.
    let default = raw.default_branch().await;
    let base_sha = raw.branch_sha(&default).await;
    let branch = format!("{prefix}-pr");
    raw.create_branch(&branch, &base_sha).await;
    raw.put_file(&format!("{prefix}.txt"), "forgetop integration fixture\n", &branch, &format!("{prefix}: add fixture file")).await;
    let id = raw.open_pr(&branch, &default, &format!("{prefix} PR")).await.to_string();

    let prs = gh.conn.pull_requests().expect("github PRs");

    // Read paths through the adapter under test.
    let list = prs.list(&PullRequestQuery::default()).await.expect("list PRs");
    assert!(list.iter().any(|p| p.id == id), "the created PR appears in the list");
    let got = prs.get(&id).await.expect("get PR");
    assert_eq!(got.id, id);
    assert!(!prs.commits(&id).await.expect("commits").is_empty(), "PR has commits");
    prs.checks(&id).await.expect("checks decode");

    // Comment write → shows up in threads.
    prs.add_comment(&id, &format!("{prefix} comment")).await.expect("add comment");
    let threads = prs.threads(&id).await.expect("threads");
    assert!(
        threads.iter().any(|t| t.comments.iter().any(|c| c.body.contains(prefix))),
        "the posted comment appears in the PR threads"
    );

    // Merge write (the actual adapter action) → PR reads back as merged.
    prs.merge(&id, &MergeOptions { strategy: MergeStrategy::Squash, delete_source_ref: true }).await.expect("merge PR");
    let after = harness::poll(30, || async {
        prs.get(&id).await.ok().filter(|p| matches!(p.status, PullRequestStatus::Merged))
    })
    .await;
    assert!(after.is_some(), "the PR reads back as merged");

    // Teardown (squash+delete usually removes the branch already).
    raw.delete_ref(&branch).await;
}

#[tokio::test]
async fn github_work_item_lifecycle() {
    let gh = skip_if_none!(harness::github(), "github");
    let raw = GhRaw::from_env().expect("github raw client");
    let prefix = harness::run_prefix();

    // Fixture: an issue assigned to me, so the mine-only list finds it.
    let (_id, login) = raw.me().await;
    let number = raw.create_issue(&format!("{prefix} issue"), &login).await;
    let id = number.to_string();

    let wi = gh.conn.work_items().expect("github work items");

    let list = wi.list(&WorkItemQuery { mine_only: true, ..Default::default() }).await.expect("list work items");
    assert!(list.iter().any(|w| w.id == id), "the assigned issue appears in the mine-only list");
    assert_eq!(wi.get(&id).await.expect("get").id, id);

    let states = wi.available_states(&id).await.expect("available states");
    assert!(states.iter().any(|s| s.eq_ignore_ascii_case("closed")), "closed is an available state, got {states:?}");

    wi.add_comment(&id, &format!("{prefix} note")).await.expect("comment");
    wi.set_state(&id, "closed").await.expect("close issue");
    let after = wi.get(&id).await.expect("get after close");
    assert!(after.state.eq_ignore_ascii_case("closed"), "issue reads back closed, got {}", after.state);

    // Teardown: it's already closed (GitHub issues can't be hard-deleted via REST).
}

/// Full approval-gate round trip. NOTE: needs a **public** container repo — required
/// reviewers on environments are free on public repos, paid on private ones.
#[tokio::test]
async fn github_pipeline_approval_gate_lifecycle() {
    let gh = skip_if_none!(harness::github(), "github");
    let raw = GhRaw::from_env().expect("github raw client");
    harness::maybe_sweep(raw.sweep()).await;
    let prefix = harness::run_prefix();

    // Fixture: an environment requiring me as reviewer + a workflow_dispatch job
    // gated on it (the job waits for approval before any runner starts).
    let (uid, _login) = raw.me().await;
    let default = raw.default_branch().await;
    let env_name = prefix.to_string();
    let wf_file = format!("{prefix}.yml");
    let wf_path = format!(".github/workflows/{wf_file}");
    raw.put_environment(&env_name, uid).await;
    let yaml = format!(
        "name: {prefix}\non:\n  workflow_dispatch:\njobs:\n  gate:\n    runs-on: ubuntu-latest\n    environment: {env_name}\n    steps:\n      - run: echo approved\n"
    );
    raw.put_file(&wf_path, &yaml, &default, &format!("{prefix}: add gated workflow")).await;
    raw.dispatch(&wf_file, &default).await;

    // Wait for the dispatched run to reach the "waiting" (approval) state.
    let run_id = {
        let raw = &raw;
        let wf = wf_file.as_str();
        harness::poll(120, move || async move {
            raw.workflow_runs(wf).await.into_iter().find(|(_, s)| s == "waiting").map(|(id, _)| id)
        })
        .await
    }
    .expect("a dispatched run reached the approval gate ('waiting')");

    let pipe = gh.conn.pipelines().expect("github pipelines");

    // The adapter surfaces our gate as actionable.
    let gates = pipe.pending_approvals(&run_id).await.expect("pending approvals");
    let gate = gates
        .iter()
        .find(|g| g.name == env_name)
        .unwrap_or_else(|| panic!("expected a pending gate named {env_name}, got {gates:?}"));
    assert!(gate.can_respond, "the gate should be actionable by me");

    // Approve through the adapter, then the gate clears.
    pipe.respond_approval(&run_id, &gate.id, ApprovalDecision::Approve, Some("integration approve")).await.expect("approve");
    let cleared = {
        let pipe = &pipe;
        let run_id = run_id.as_str();
        let env_name = env_name.as_str();
        harness::poll(60, move || async move {
            match pipe.pending_approvals(run_id).await {
                Ok(g) if !g.iter().any(|x| x.name == env_name) => Some(()),
                _ => None,
            }
        })
        .await
    };
    assert!(cleared.is_some(), "the gate cleared after approval");

    // Teardown.
    raw.delete_run(&run_id).await;
    raw.delete_environment(&env_name).await;
    raw.delete_file(&wf_path, &default, &format!("{prefix}: remove workflow")).await;
}
