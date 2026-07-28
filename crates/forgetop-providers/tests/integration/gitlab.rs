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
    assert_eq!(prs.get(&ItemRef::new(&id)).await.expect("get").id, id);
    // GitLab can lag on computing a fresh MR's commit list — poll rather than assume.
    let has_commits = {
        let prs = &prs;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move { prs.commits(&ItemRef::new(id)).await.ok().filter(|c| !c.is_empty()).map(|_| ()) }).await
    };
    assert!(has_commits.is_some(), "the MR reports its commit");

    // The head commit's per-commit diff decodes and reports the pushed fixture file.
    let commits = prs.commits(&ItemRef::new(&id)).await.expect("commits");
    let sha = commits.first().expect("a commit").sha.clone();
    let commit_files = prs.commit_changes(&ItemRef::new(&id), &sha).await.expect("commit changes");
    assert!(
        commit_files.iter().any(|f| f.path.contains(&format!("{prefix}.txt"))),
        "the commit reports the file it added"
    );

    prs.add_comment(&ItemRef::new(&id), &format!("{prefix} comment")).await.expect("comment");
    let threads = prs.threads(&ItemRef::new(&id)).await.expect("threads");
    assert!(threads.iter().any(|t| t.comments.iter().any(|c| c.body.contains(prefix))), "comment shows in threads");

    prs.timeline(&ItemRef::new(&id)).await.expect("timeline decodes");

    // Reply into that discussion; the reply comes back nested in the same thread.
    let thread_id = threads.iter().find(|t| t.comments.iter().any(|c| c.body.contains(prefix))).expect("our thread").id.clone();
    prs.reply_to_thread(&ItemRef::new(&id), &thread_id, &format!("{prefix} reply")).await.expect("reply to discussion");
    let after = prs.threads(&ItemRef::new(&id)).await.expect("threads after reply");
    assert!(
        after.iter().any(|t| t.id == thread_id && t.comments.iter().any(|c| c.body.contains(&format!("{prefix} reply")))),
        "the reply lands in the discussion it targeted"
    );

    // Mergeable flag: the clean fixture MR (off the default branch, no conflicts) settles to
    // Mergeable. GitLab computes `merge_status` asynchronously, so poll get() until it lands.
    let mergeable = {
        let prs = &prs;
        let id = id.as_str();
        harness::poll(harness::POLL_MERGE, move || async move {
            prs.get(&ItemRef::new(id)).await.ok().filter(|p| p.mergeable == MergeableState::Mergeable).map(|_| ())
        })
        .await
    };
    assert!(mergeable.is_some(), "the clean MR computes as mergeable");

    // Merge (retry: GitLab may briefly report the MR as still checking mergeability).
    let merged = harness::poll(harness::POLL_MERGE, || async {
        if prs.merge(&ItemRef::new(&id), &MergeOptions { strategy: MergeStrategy::Merge, delete_source_ref: true }).await.is_ok() {
            prs.get(&ItemRef::new(&id)).await.ok().filter(|p| matches!(p.status, PullRequestStatus::Merged))
        } else {
            None
        }
    })
    .await;
    assert!(merged.is_some(), "the MR reads back as merged");

    // Revert the merge commit onto the target branch (GitLab commits the revert directly).
    prs.revert(&ItemRef::new(&id)).await.expect("revert the merged MR");

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
    assert_eq!(wi.get(&ItemRef::new(&id)).await.expect("get").id, id);

    let states = wi.available_states(&ItemRef::new(&id)).await.expect("available states");
    let closed = states.iter().find(|s| s.eq_ignore_ascii_case("closed")).cloned().unwrap_or_else(|| "closed".into());
    wi.add_comment(&ItemRef::new(&id), &format!("{prefix} note")).await.expect("comment");

    let candidates = wi.assignable_users(&ItemRef::new(&id)).await.expect("assignable users");
    assert!(!candidates.is_empty(), "project reports assignable members");

    let cand = candidates.first().expect("an assignable user");
    wi.set_assignee(&ItemRef::new(&id), Some(&cand.id)).await.expect("assign");
    let assigned = {
        let wi = &wi;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move { wi.get(&ItemRef::new(id)).await.ok().filter(|w| w.assignee.is_some()) }).await
    };
    assert!(assigned.is_some(), "the issue reads back assigned");

    wi.set_assignee(&ItemRef::new(&id), None).await.expect("unassign");
    let unassigned = {
        let wi = &wi;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move { wi.get(&ItemRef::new(id)).await.ok().filter(|w| w.assignee.is_none()) }).await
    };
    assert!(unassigned.is_some(), "the issue reads back unassigned");

    let new_title = format!("{prefix} edited");
    wi.update_fields(&ItemRef::new(&id), Some(&new_title), Some("edited body")).await.expect("edit");
    let edited = {
        let wi = &wi;
        let id = id.as_str();
        let new_title = new_title.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            wi.get(&ItemRef::new(id)).await.ok().filter(|w| {
                w.title == new_title && w.description.as_deref().is_some_and(|d| d.contains("edited body"))
            })
        })
        .await
    };
    assert!(edited.is_some(), "the issue reads back edited");

    wi.set_state(&ItemRef::new(&id), &closed).await.expect("close issue");

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
            pipe.pending_approvals(&ItemRef::new(run_id)).await.ok().and_then(|g| g.into_iter().find(|x| x.can_respond))
        })
        .await
    }
    .expect("the manual job is surfaced as a pending gate");

    // Approve (plays the job); the gate then clears.
    pipe.respond_approval(&ItemRef::new(&run_id), &gate.id, ApprovalDecision::Approve, None).await.expect("play manual job");
    let cleared = {
        let pipe = &pipe;
        let run_id = run_id.as_str();
        let gate_id = gate.id.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            match pipe.pending_approvals(&ItemRef::new(run_id)).await {
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
async fn gitlab_pipeline_cancel_lifecycle() {
    let gl = skip_if_none!(harness::gitlab(), "gitlab");
    let raw = GlRaw::from_env().expect("gitlab raw");
    harness::maybe_sweep(raw.sweep()).await;
    let prefix = harness::run_prefix();

    // Fixture: a throwaway branch carrying a long-running job, so the .gitlab-ci.yml never
    // lands on the default branch (which would trigger CI on every later push/test).
    let default_branch = raw.default_branch().await;
    let branch = format!("{prefix}-cancel");
    raw.create_branch(&branch, &default_branch).await;
    raw.put_file(".gitlab-ci.yml", "job:\n  script:\n    - sleep 120\n", &branch, &format!("{prefix}: add ci")).await;
    let pipeline_id = match raw.create_pipeline(&branch).await {
        Ok(id) => id,
        Err(e) => {
            // GitLab.com can block CI on unvalidated accounts — treat that as a skip.
            eprintln!("SKIP gitlab pipeline cancellation: CI can't run on this account ({e})");
            raw.delete_branch(&branch).await;
            return;
        }
    };
    let id = pipeline_id.to_string();

    let pipe = gl.conn.pipelines().expect("gitlab pipelines");
    // Let GitLab register the new pipeline before requesting cancellation.
    let _ = {
        let pipe = &pipe;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            pipe.get_run(&ItemRef::new(id))
                .await
                .ok()
                .filter(|run| matches!(run.status, PipelineRunStatus::Queued | PipelineRunStatus::Running))
        })
        .await
    };
    pipe.cancel_run(&ItemRef::new(&id)).await.expect("cancel");

    let cancelled = {
        let pipe = &pipe;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            pipe.get_run(&ItemRef::new(id)).await.ok().filter(|run| matches!(run.status, PipelineRunStatus::Canceled))
        })
        .await
    };
    assert!(cancelled.is_some(), "the pipeline reads back as canceled after cancellation");

    raw.delete_pipeline(pipeline_id).await;
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

/// End-to-end: an event on your MR surfaces in the notification inbox, drills back to the MR,
/// and clears on mark-read. GitLab's notification primitive is the to-do, so we create one with
/// the manual `.../todo` endpoint — the single-user harness can't have a *second* person comment
/// on or request review of your MR (and GitLab makes no to-do for your own actions). What this
/// proves is the pipeline the TUI/Dashboard depend on: todo → `notifications().list()` → in-app
/// drill-in id → mark-read.
#[tokio::test]
async fn gitlab_mr_event_surfaces_as_notification() {
    let gl = skip_if_none!(harness::gitlab(), "gitlab");
    let raw = GlRaw::from_env().expect("gitlab raw");
    harness::maybe_sweep(raw.sweep()).await;
    let prefix = harness::run_prefix();

    // Fixture: an MR to hang the to-do on.
    let default = raw.default_branch().await;
    let branch = format!("{prefix}-notif");
    raw.create_branch(&branch, &default).await;
    raw.put_file(&format!("{prefix}.txt"), "forgetop notif fixture\n", &branch, &format!("{prefix}: fixture")).await;
    let iid = raw.open_mr(&branch, &default, &format!("{prefix} MR")).await;

    raw.create_mr_todo(iid).await;

    // The to-do appears as a PR notification pointing back at the MR (todos can lag a moment).
    let notifs = gl.conn.notifications().expect("gitlab notifications");
    let id = iid.to_string();
    let found = {
        let notifs = &notifs;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            notifs
                .list()
                .await
                .ok()
                .and_then(|l| l.into_iter().find(|n| n.item_id.as_deref() == Some(id) && n.item_type == NotificationItemType::PullRequest))
        })
        .await
    };
    let n = found.expect("the MR to-do surfaces as a PR notification");
    assert!(n.unread, "a pending to-do reads as unread");

    // Mark it read through the adapter; it drops out of the inbox.
    notifs.mark_read(&n.id).await.expect("mark the notification read");
    let cleared = {
        let notifs = &notifs;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            match notifs.list().await {
                Ok(l) if !l.iter().any(|x| x.item_id.as_deref() == Some(id)) => Some(()),
                _ => None,
            }
        })
        .await
    };
    assert!(cleared.is_some(), "the notification clears after mark-read");

    raw.delete_branch(&branch).await;
}
