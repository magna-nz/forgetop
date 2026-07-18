//! GitLab live tests: read paths + create/read-act/teardown for MRs, issues, and
//! the manual-job approval gate.

use forgetop_core::domain::*;
use forgetop_core::provider::*;

use crate::gl_raw::GlRaw;
use crate::harness;

#[tokio::test]
async fn gitlab_connectivity_and_lists() {
    let gl = skip_if_none!(harness::gitlab(), "gitlab");
    assert!(gl.conn.check().await, "token authenticates against project {}", gl.project);
    let prs = gl.conn.pull_requests().expect("gitlab MRs");
    prs.list(&PullRequestQuery::default()).await.expect("list MRs");
    let wi = gl.conn.work_items().expect("gitlab issues");
    wi.list(&WorkItemQuery::default()).await.expect("list issues");
    let pipe = gl.conn.pipelines().expect("gitlab pipelines");
    assert!(pipe.supports_approvals(), "gitlab supports approvals");
    pipe.list_runs(&PipelineRunQuery::default()).await.expect("list pipelines");
}

#[tokio::test]
async fn gitlab_merge_request_lifecycle() {
    let gl = skip_if_none!(harness::gitlab(), "gitlab");
    let raw = GlRaw::from_env().expect("gitlab raw");
    harness::maybe_sweep(raw.sweep()).await;
    let prefix = harness::run_prefix();

    let default = raw.default_branch().await;
    let branch = format!("{prefix}-mr");
    raw.create_branch(&branch, &default).await;
    raw.put_file(&format!("{prefix}.txt"), "forgetop integration fixture\n", &branch, &format!("{prefix}: fixture")).await;
    let id = raw.open_mr(&branch, &default, &format!("{prefix} MR")).await.to_string();

    let prs = gl.conn.pull_requests().expect("gitlab MRs");
    let list = prs.list(&PullRequestQuery::default()).await.expect("list");
    assert!(list.iter().any(|p| p.id == id), "created MR appears in the list");
    assert_eq!(prs.get(&id).await.expect("get").id, id);
    // GitLab can lag on computing a fresh MR's commit list — poll rather than assume.
    let has_commits = {
        let prs = &prs;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move { prs.commits(id).await.ok().filter(|c| !c.is_empty()).map(|_| ()) }).await
    };
    assert!(has_commits.is_some(), "the MR reports its commit");

    prs.add_comment(&id, &format!("{prefix} comment")).await.expect("comment");
    let threads = prs.threads(&id).await.expect("threads");
    assert!(threads.iter().any(|t| t.comments.iter().any(|c| c.body.contains(prefix))), "comment shows in threads");

    // Merge (retry: GitLab may briefly report the MR as still checking mergeability).
    let merged = harness::poll(harness::POLL_MERGE, || async {
        if prs.merge(&id, &MergeOptions { strategy: MergeStrategy::Merge, delete_source_ref: true }).await.is_ok() {
            prs.get(&id).await.ok().filter(|p| matches!(p.status, PullRequestStatus::Merged))
        } else {
            None
        }
    })
    .await;
    assert!(merged.is_some(), "the MR reads back as merged");

    // Revert the merge commit onto the target branch (GitLab commits the revert directly).
    prs.revert(&id).await.expect("revert the merged MR");

    raw.delete_branch(&branch).await;
}

#[tokio::test]
async fn gitlab_issue_lifecycle() {
    let gl = skip_if_none!(harness::gitlab(), "gitlab");
    let raw = GlRaw::from_env().expect("gitlab raw");
    let prefix = harness::run_prefix();

    let (uid, _name) = raw.me().await;
    let iid = raw.create_issue(&format!("{prefix} issue"), uid).await;
    let id = iid.to_string();

    let wi = gl.conn.work_items().expect("gitlab issues");
    let list = wi.list(&WorkItemQuery { mine_only: true, ..Default::default() }).await.expect("list");
    assert!(list.iter().any(|w| w.id == id), "assigned issue appears in mine-only list");
    assert_eq!(wi.get(&id).await.expect("get").id, id);

    let states = wi.available_states(&id).await.expect("available states");
    let closed = states.iter().find(|s| s.eq_ignore_ascii_case("closed")).cloned().unwrap_or_else(|| "closed".into());
    wi.add_comment(&id, &format!("{prefix} note")).await.expect("comment");
    wi.set_state(&id, &closed).await.expect("close issue");

    raw.delete_issue(iid).await;
}

#[tokio::test]
async fn gitlab_manual_job_approval_lifecycle() {
    let gl = skip_if_none!(harness::gitlab(), "gitlab");
    let raw = GlRaw::from_env().expect("gitlab raw");
    harness::maybe_sweep(raw.sweep()).await;
    let prefix = harness::run_prefix();

    // Fixture: a branch whose CI has a single `when: manual` job — the actionable gate.
    let default = raw.default_branch().await;
    let branch = format!("{prefix}-ci");
    raw.create_branch(&branch, &default).await;
    let yaml = "gate:\n  stage: deploy\n  when: manual\n  script:\n    - echo approved\n";
    raw.put_file(".gitlab-ci.yml", yaml, &branch, &format!("{prefix}: manual gate")).await;
    let run_id = match raw.create_pipeline(&branch).await {
        Ok(id) => id.to_string(),
        Err(e) => {
            // GitLab.com blocks CI on unvalidated accounts — treat as a skip.
            eprintln!("SKIP gitlab manual-job approval: CI can't run on this account ({e})");
            raw.delete_branch(&branch).await;
            return;
        }
    };

    let pipe = gl.conn.pipelines().expect("gitlab pipelines");

    // The manual job surfaces as an actionable gate.
    let gate = {
        let pipe = &pipe;
        let run_id = run_id.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            pipe.pending_approvals(run_id).await.ok().and_then(|g| g.into_iter().find(|x| x.can_respond))
        })
        .await
    }
    .expect("the manual job is surfaced as a pending gate");

    // Approve (plays the job); the gate then clears.
    pipe.respond_approval(&run_id, &gate.id, ApprovalDecision::Approve, None).await.expect("play manual job");
    let cleared = {
        let pipe = &pipe;
        let run_id = run_id.as_str();
        let gate_id = gate.id.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            match pipe.pending_approvals(run_id).await {
                Ok(g) if !g.iter().any(|x| x.id == gate_id) => Some(()),
                _ => None,
            }
        })
        .await
    };
    assert!(cleared.is_some(), "the manual gate cleared after playing");

    raw.delete_pipeline(run_id.parse().unwrap()).await;
    raw.delete_branch(&branch).await;
}

#[tokio::test]
async fn gitlab_lists_notifications() {
    let gl = skip_if_none!(harness::gitlab(), "gitlab");
    let notifs = gl.conn.notifications().expect("gitlab advertises notifications");
    // Decoding the todos envelope is the assertion; no pending todos returns [].
    let list = notifs.list().await.expect("list todos");
    eprintln!("gitlab: {} todo(s)", list.len());
    if let Some(n) = list.first() {
        assert!(!n.id.is_empty(), "a todo carries an id for mark-as-done");
    }
}
