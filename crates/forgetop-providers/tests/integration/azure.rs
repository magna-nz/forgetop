//! Azure DevOps live tests: read paths + create/read-act/teardown for PRs and work
//! items, plus the approval gate (against a pre-created gated pipeline — see
//! INTEGRATION.md; that one test skips unless FORGETOP_IT_AZURE_PIPELINE_ID is set).

use forgetop_core::domain::*;
use forgetop_core::provider::*;

use crate::az_raw::AzRaw;
use crate::harness;

#[tokio::test]
async fn azure_connectivity_and_lists() {
    let az = skip_if_none!(harness::azure(), "azure");
    assert!(az.conn.check().await, "PAT authenticates against {}/{}", az.org, az.project);
    az.conn.pull_requests().expect("azure PRs").list(&PullRequestQuery::default()).await.expect("list PRs");
    az.conn.work_items().expect("azure work items").list(&WorkItemQuery::default()).await.expect("list work items");
    let pipe = az.conn.pipelines().expect("azure pipelines");
    assert!(pipe.supports_approvals(), "azure supports approvals");
    pipe.list_runs(&PipelineRunQuery::default()).await.expect("list pipeline runs");
}

#[tokio::test]
async fn azure_pull_request_lifecycle() {
    let az = skip_if_none!(harness::azure(), "azure");
    let raw = AzRaw::from_env().expect("azure raw");
    let prefix = harness::run_prefix();

    let (default, base_sha) = raw.default_branch().await;
    let branch = format!("{prefix}-pr");
    raw.create_branch_with_file(&branch, &base_sha, &format!("{prefix}.txt"), "forgetop integration fixture\n", &format!("{prefix}: fixture")).await;
    let id = raw.open_pr(&branch, &default, &format!("{prefix} PR")).await.to_string();

    let prs = az.conn.pull_requests().expect("azure PRs");
    let list = prs.list(&PullRequestQuery::default()).await.expect("list");
    assert!(list.iter().any(|p| p.id == id), "created PR appears in the list");
    assert_eq!(prs.get(&id).await.expect("get").id, id);
    assert!(!prs.commits(&id).await.expect("commits").is_empty());

    prs.add_comment(&id, &format!("{prefix} comment")).await.expect("comment");
    let threads = prs.threads(&id).await.expect("threads");
    assert!(threads.iter().any(|t| t.comments.iter().any(|c| c.body.contains(prefix))), "comment shows in threads");

    // Merge via the adapter (retry: Azure needs a moment to compute mergeability).
    let merged = harness::poll(40, || async {
        if prs.merge(&id, &MergeOptions { strategy: MergeStrategy::Squash, delete_source_ref: true }).await.is_ok() {
            prs.get(&id).await.ok().filter(|p| matches!(p.status, PullRequestStatus::Merged))
        } else {
            None
        }
    })
    .await;

    // Teardown regardless of whether the merge landed.
    if merged.is_none() {
        raw.abandon_pr(id.parse().unwrap()).await;
    }
    raw.delete_branch(&branch).await;
    assert!(merged.is_some(), "the PR reads back as merged");
}

#[tokio::test]
async fn azure_work_item_lifecycle() {
    let az = skip_if_none!(harness::azure(), "azure");
    let raw = AzRaw::from_env().expect("azure raw");
    let prefix = harness::run_prefix();

    let me = raw.me_unique().await;
    let wid = raw.create_work_item(&format!("{prefix} task"), &me).await;
    let id = wid.to_string();

    let wi = az.conn.work_items().expect("azure work items");
    // List broadly (not mine-only) and find ours by id — robust across identity quirks.
    let list = wi.list(&WorkItemQuery { mine_only: false, include_completed: true, limit: Some(100) }).await.expect("list");
    assert!(list.iter().any(|w| w.id == id), "created work item appears in the list");

    let got = wi.get(&id).await.expect("get");
    let states = wi.available_states(&id).await.expect("available states");
    assert!(!states.is_empty(), "work item type reports states");
    wi.add_comment(&id, &format!("{prefix} note")).await.expect("comment");

    // Move it to a different state and confirm it sticks.
    if let Some(next) = states.iter().find(|s| !s.eq_ignore_ascii_case(&got.state)) {
        wi.set_state(&id, next).await.expect("set state");
        let after = wi.get(&id).await.expect("get after");
        assert!(after.state.eq_ignore_ascii_case(next), "state changed to {next}, got {}", after.state);
    }

    raw.delete_work_item(wid).await;
}

#[tokio::test]
async fn azure_pipeline_approval_lifecycle() {
    let az = skip_if_none!(harness::azure(), "azure");
    let pipeline_id = match harness::env("FORGETOP_IT_AZURE_PIPELINE_ID") {
        Some(p) => p,
        None => {
            eprintln!("SKIP azure approvals: set FORGETOP_IT_AZURE_PIPELINE_ID to a gated pipeline (see INTEGRATION.md)");
            return;
        }
    };
    let raw = AzRaw::from_env().expect("azure raw");
    let run_id = raw.queue_pipeline(&pipeline_id).await;

    let pipe = az.conn.pipelines().expect("azure pipelines");
    let gate = {
        let pipe = &pipe;
        let run_id = run_id.as_str();
        harness::poll(120, move || async move {
            pipe.pending_approvals(run_id).await.ok().and_then(|g| g.into_iter().find(|x| x.can_respond))
        })
        .await
    }
    .expect("the queued run reached its approval gate");

    pipe.respond_approval(&run_id, &gate.id, ApprovalDecision::Approve, Some("integration approve")).await.expect("approve");
    let cleared = {
        let pipe = &pipe;
        let run_id = run_id.as_str();
        let gate_id = gate.id.as_str();
        harness::poll(60, move || async move {
            match pipe.pending_approvals(run_id).await {
                Ok(g) if !g.iter().any(|x| x.id == gate_id) => Some(()),
                _ => None,
            }
        })
        .await
    };
    assert!(cleared.is_some(), "the gate cleared after approval");

    raw.delete_build(&run_id).await;
}
