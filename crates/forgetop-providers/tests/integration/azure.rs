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
    raw.push_file(&branch, &base_sha, &format!("{prefix}.txt"), "forgetop integration fixture\n", &format!("{prefix}: fixture")).await;
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

/// Full tear-up/down of an Azure approval gate: create an environment + Approval
/// check + a YAML pipeline that deploys to it, queue it, approve via the adapter,
/// then delete everything. Needs agent capacity to *run* — but the approval pauses
/// before the agent, so a public project (free hosted agents) suffices.
#[tokio::test]
async fn azure_pipeline_approval_full_lifecycle() {
    let az = skip_if_none!(harness::azure(), "azure");
    let raw = AzRaw::from_env().expect("azure raw");
    let prefix = harness::run_prefix();

    // Fixtures.
    let (default, sha) = raw.default_branch().await;
    let approver = raw.me_id().await;
    let repo_id = raw.repo_id().await;
    let repo_name = harness::env("FORGETOP_IT_AZURE_REPO").unwrap_or_else(|| az.project.clone());
    let env_name = format!("{prefix}-env");
    let yaml_path = format!("{prefix}.yml");

    let yaml = format!(
        "stages:\n- stage: gate\n  jobs:\n  - deployment: approve\n    pool:\n      vmImage: ubuntu-latest\n    environment: {env_name}\n    strategy:\n      runOnce:\n        deploy:\n          steps:\n          - script: echo approved\n"
    );
    raw.push_file(&default, &sha, &yaml_path, &yaml, &format!("{prefix}: gated pipeline")).await;
    let env_id = raw.create_environment(&env_name).await;
    let check_id = raw.add_approval_check(env_id, &env_name, &approver).await;
    let def_id = raw.create_pipeline_def(prefix, &format!("/{yaml_path}"), &repo_id, &repo_name).await;
    let run_id = raw.queue_pipeline(&def_id).await;

    let pipe = az.conn.pipelines().expect("azure pipelines");
    let gate = {
        let pipe = &pipe;
        let run_id = run_id.as_str();
        harness::poll(150, move || async move {
            pipe.pending_approvals(run_id).await.ok().and_then(|g| g.into_iter().find(|x| x.can_respond))
        })
        .await
    };

    // Act + assert only if we reached the gate; always tear down afterwards.
    let result = if let Some(gate) = &gate {
        let approved = pipe.respond_approval(&run_id, &gate.id, ApprovalDecision::Approve, Some("integration approve")).await;
        approved.map(|_| ())
    } else {
        Err(forgetop_core::Error::Provider("run never reached the approval gate".into()))
    };

    // Teardown (best-effort, in reverse order).
    raw.delete_build(&run_id).await;
    raw.delete_pipeline_def(&def_id).await;
    raw.delete_check(check_id).await;
    raw.delete_environment_by_id(env_id).await;
    raw.delete_file(&yaml_path, &default, &format!("{prefix}: remove pipeline")).await;

    result.expect("reach + approve the gate");
    assert!(gate.is_some());
}
