//! Linear live tests: work items only (Linear has no PRs/pipelines).

use forgetop_core::domain::{NotificationKind, NotificationTarget, WorkItemParent, WorkItemStateCategory};
use forgetop_core::provider::*;

use crate::harness;
use crate::ln_raw::LnRaw;

#[tokio::test]
async fn linear_connectivity_and_list() {
    let ln = skip_if_none!(harness::linear(), "linear");
    assert!(ln.conn.check().await, "linear api key authenticates");
    ln.conn.work_items().expect("linear work items").list(&WorkItemQuery::default()).await.expect("list issues");
}

/// A real issue under a real project milestone comes back naming it as its parent, and one in
/// the project with no milestone names the project — from the list and from a single read. This
/// is also the only place the extended issue selection (`project`, `projectMilestone`) is
/// answered with real objects rather than nulls.
#[tokio::test]
async fn linear_issue_names_its_milestone_or_project_as_its_parent() {
    let ln = skip_if_none!(harness::linear(), "linear");
    let raw = LnRaw::from_env().expect("linear raw");
    let prefix = harness::run_prefix();

    let swept = raw.sweep_stale_projects().await;
    if swept > 0 {
        eprintln!("linear: deleted {swept} leaked fixture project(s)");
    }

    let me = raw.viewer_id().await;
    let team = raw.team_id().await;
    let project_name = format!("{prefix} project");
    let (project, project_url) = raw.create_project(&team, &project_name).await;
    let milestone = raw.create_milestone(&project, "Beta").await;
    let in_milestone = raw.create_issue_in_project(&team, &format!("{prefix} milestone issue"), &me, &project, Some(&milestone)).await;
    let in_project = raw.create_issue_in_project(&team, &format!("{prefix} project issue"), &me, &project, None).await;

    let wi = ln.conn.work_items().expect("linear work items");
    let listed = {
        let (wi, a, b) = (&wi, in_milestone.as_str(), in_project.as_str());
        harness::poll(harness::POLL_LIST, move || async move {
            let list = wi.list(&WorkItemQuery { mine_only: true, include_completed: true, limit: Some(100) }).await.ok()?;
            let find = |id: &str| list.iter().find(|w| w.id == id).cloned();
            Some((find(a)?, find(b)?))
        })
        .await
    };
    let got = wi.get(&ItemRef::new(&in_milestone)).await;

    // Torn down before anything is asserted, so a failure leaks no fixture into a workspace
    // with a hard limit on active issues.
    raw.try_archive_issue(&in_milestone).await;
    raw.try_archive_issue(&in_project).await;
    raw.delete_project(&project).await;

    let under_milestone =
        WorkItemParent { kind: "Milestone".into(), id: milestone, title: format!("{project_name} › Beta"), url: Some(project_url.clone()) };
    let under_project = WorkItemParent { kind: "Project".into(), id: project, title: project_name, url: Some(project_url) };
    let (a, b) = listed.expect("both issues appear in the mine-only list");
    assert_eq!(a.parent, Some(under_milestone.clone()), "the milestone, titled with its project, from the list");
    assert_eq!(b.parent, Some(under_project), "the project, for an issue with no milestone");
    assert_eq!(got.expect("get").parent, Some(under_milestone), "from a single read");
}

/// In review is told from in progress by the team's real workflow order, not by state names:
/// an issue in the team's first started state is in progress, one in a later started state is
/// in review — from the list and from a single read.
#[tokio::test]
async fn linear_tells_in_review_from_in_progress_by_workflow_order() {
    let ln = skip_if_none!(harness::linear(), "linear");
    let raw = LnRaw::from_env().expect("linear raw");
    let prefix = harness::run_prefix();

    let me = raw.viewer_id().await;
    let team = raw.team_id().await;
    let started = raw.started_states(&team).await;
    let [first, .., last] = started.as_slice() else {
        eprintln!("SKIP linear in-review: the team has fewer than two started states");
        return;
    };
    let doing = raw.create_issue_in_state(&team, &format!("{prefix} doing"), &me, first).await;
    let reviewing = raw.create_issue_in_state(&team, &format!("{prefix} reviewing"), &me, last).await;

    let wi = ln.conn.work_items().expect("linear work items");
    let listed = {
        let (wi, a, b) = (&wi, doing.as_str(), reviewing.as_str());
        harness::poll(harness::POLL_LIST, move || async move {
            let list = wi.list(&WorkItemQuery { mine_only: true, include_completed: true, limit: Some(100) }).await.ok()?;
            let category = |id: &str| list.iter().find(|w| w.id == id).map(|w| w.state_category);
            Some((category(a)?, category(b)?))
        })
        .await
    };
    let got = wi.get(&ItemRef::new(&reviewing)).await.map(|w| w.state_category);

    // Torn down before anything is asserted, so a failure leaks no fixture.
    raw.try_archive_issue(&doing).await;
    raw.try_archive_issue(&reviewing).await;

    assert_eq!(
        listed.expect("both issues appear in the mine-only list"),
        (WorkItemStateCategory::Started, WorkItemStateCategory::Review),
        "the first started state is in progress, a later one in review"
    );
    assert_eq!(got.expect("get"), WorkItemStateCategory::Review, "from a single read");
}

#[tokio::test]
async fn linear_work_item_lifecycle() {
    let ln = skip_if_none!(harness::linear(), "linear");
    let raw = LnRaw::from_env().expect("linear raw");
    let prefix = harness::run_prefix();

    // Every run, not just the nightly sweep: leaked issues otherwise pile up until the
    // workspace refuses to create any more.
    let swept = raw.sweep_stale().await;
    if swept > 0 {
        eprintln!("linear: archived {swept} leaked fixture issue(s)");
    }

    let me = raw.viewer_id().await;
    let team = raw.team_id().await;
    let id = raw.create_issue(&team, &format!("{prefix} issue"), &me).await;

    let wi = ln.conn.work_items().expect("linear work items");

    // The new issue is assigned to me → shows up in the mine-only list (eventually).
    let found = {
        let wi = &wi;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            wi.list(&WorkItemQuery { mine_only: true, include_completed: true, limit: Some(100) }).await.ok().filter(|l| l.iter().any(|w| w.id == id)).map(|_| ())
        })
        .await
    };
    assert!(found.is_some(), "assigned issue appears in the mine-only list");

    assert_eq!(wi.get(&ItemRef::new(&id)).await.expect("get").id, id);
    let states = wi.available_states(&ItemRef::new(&id)).await.expect("available states");
    assert!(!states.is_empty(), "team reports workflow states");
    wi.add_comment(&ItemRef::new(&id), &format!("{prefix} note")).await.expect("comment");
    wi.timeline(&ItemRef::new(&id)).await.expect("timeline decodes");

    let candidates = wi.assignable_users(&ItemRef::new(&id)).await.expect("assignable users");
    assert!(!candidates.is_empty(), "workspace reports users");
    let viewer_id = candidates
        .iter()
        .find(|candidate| candidate.id == me)
        .map(|candidate| candidate.id.clone())
        .expect("authenticated viewer appears in assignable users");

    let item = ItemRef::new(&id);
    harness::retry_write(harness::POLL_LIST, || wi.set_assignee(&item, None)).await.expect("unassign");
    let unassigned = {
        let wi = &wi;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move { wi.get(&ItemRef::new(id)).await.ok().filter(|w| w.assignee.is_none()).map(|_| ()) }).await
    };
    assert!(unassigned.is_some(), "issue reports no assignee");

    // Linear intermittently answers this write with `cannot delegate to Linear`; every other
    // step in this test already tolerates a transient API, so this one does too.
    harness::retry_write(harness::POLL_LIST, || wi.set_assignee(&item, Some(&viewer_id))).await.expect("assign");
    let assigned = {
        let wi = &wi;
        let id = id.as_str();
        let viewer_id = viewer_id.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            wi.get(&ItemRef::new(id))
                .await
                .ok()
                .filter(|w| w.assignee.as_ref().is_some_and(|assignee| assignee.id == viewer_id))
                .map(|_| ())
        })
        .await
    };
    assert!(assigned.is_some(), "issue reports the authenticated viewer as assignee");

    let new_title = format!("{prefix} edited");
    wi.update_fields(&ItemRef::new(&id), Some(&new_title), Some("edited body")).await.expect("edit");
    let edited = {
        let wi = &wi;
        let id = id.as_str();
        let new_title = new_title.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            wi.get(&ItemRef::new(id))
                .await
                .ok()
                .filter(|w| w.title == new_title && w.description.as_deref().is_some_and(|d| d.contains("edited body")))
                .map(|_| ())
        })
        .await
    };
    assert!(edited.is_some(), "issue reports edited fields");

    let current = wi.get(&ItemRef::new(&id)).await.expect("get").state;
    if let Some(next) = states.iter().find(|s| !s.eq_ignore_ascii_case(&current)) {
        wi.set_state(&ItemRef::new(&id), next).await.expect("set state");
        let after = wi.get(&ItemRef::new(&id)).await.expect("get after");
        assert!(after.state.eq_ignore_ascii_case(next), "state moved to {next}, got {}", after.state);
    }

    raw.archive_issue(&id).await;
}

#[tokio::test]
async fn linear_lists_notifications() {
    let ln = skip_if_none!(harness::linear(), "linear");
    let notifs = ln.conn.notifications().expect("linear advertises notifications");
    // Decoding the notifications envelope is the assertion; an empty inbox returns [].
    let list = notifs.list().await.expect("list notifications");
    eprintln!("linear: {} notification(s)", list.len());
    if let Some(n) = list.first() {
        assert!(!n.id.is_empty(), "a notification carries an id for mark-read");
    }
    // The query's `actor`, `comment` and `issue.state` fields decoded (a wrong field name would
    // have failed the whole list above). Print what came back — types only, no content.
    let mut seen: std::collections::BTreeMap<(String, String), usize> = Default::default();
    for n in &list {
        assert!(!n.reason.is_empty(), "every notification keeps Linear's raw type ({})", n.id);
        *seen.entry((n.reason.clone(), format!("{:?}", n.kind))).or_default() += 1;
        if n.kind == NotificationKind::Reaction {
            assert_eq!(n.target, NotificationTarget::Overview, "a reaction opens the issue");
        }
        if n.kind == NotificationKind::StateChange {
            assert_eq!(n.target, NotificationTarget::Activity);
            if let Some(state) = &n.excerpt {
                assert!(!state.contains('\n'), "a state excerpt is the state's name alone");
            }
        }
        if let NotificationTarget::Comment { id } = &n.target {
            assert!(!id.is_empty());
            assert!(matches!(n.kind, NotificationKind::Mention | NotificationKind::Comment));
        }
    }
    for ((reason, kind), count) in &seen {
        eprintln!("linear: {count} x {reason} -> {kind}");
    }
    let with_actor = list.iter().filter(|n| n.actor.is_some()).count();
    let with_comment = list.iter().filter(|n| matches!(n.target, NotificationTarget::Comment { .. })).count();
    let with_excerpt = list.iter().filter(|n| n.excerpt.is_some()).count();
    eprintln!("linear: {with_actor} with an actor, {with_comment} targeting a comment, {with_excerpt} with an excerpt");
}
