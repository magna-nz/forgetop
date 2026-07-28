//! Linear live tests: work items only (Linear has no PRs/pipelines).

use forgetop_core::provider::*;

use crate::harness;
use crate::ln_raw::LnRaw;

#[tokio::test]
async fn linear_connectivity_and_list() {
    let ln = skip_if_none!(harness::linear(), "linear");
    assert!(ln.conn.check().await, "linear api key authenticates");
    ln.conn.work_items().expect("linear work items").list(&WorkItemQuery::default()).await.expect("list issues");
}

#[tokio::test]
async fn linear_work_item_lifecycle() {
    let ln = skip_if_none!(harness::linear(), "linear");
    let raw = LnRaw::from_env().expect("linear raw");
    let prefix = harness::run_prefix();

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

    wi.set_assignee(&ItemRef::new(&id), None).await.expect("unassign");
    let unassigned = {
        let wi = &wi;
        let id = id.as_str();
        harness::poll(harness::POLL_LIST, move || async move { wi.get(&ItemRef::new(id)).await.ok().filter(|w| w.assignee.is_none()).map(|_| ()) }).await
    };
    assert!(unassigned.is_some(), "issue reports no assignee");

    wi.set_assignee(&ItemRef::new(&id), Some(&viewer_id)).await.expect("assign");
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
}
