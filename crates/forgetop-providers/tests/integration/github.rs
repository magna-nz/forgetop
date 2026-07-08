//! GitHub live read-path tests (Wave 1): prove auth, base URLs, pagination
//! envelopes, and JSON decoding against the real API. Fixture creation + write
//! paths (PR merge, issue state, environment approvals) arrive in Wave 2.

use forgetop_core::provider::*;

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
