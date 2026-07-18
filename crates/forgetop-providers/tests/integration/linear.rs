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

    assert_eq!(wi.get(&id).await.expect("get").id, id);
    let states = wi.available_states(&id).await.expect("available states");
    assert!(!states.is_empty(), "team reports workflow states");
    wi.add_comment(&id, &format!("{prefix} note")).await.expect("comment");
    wi.timeline(&id).await.expect("timeline decodes");

    let current = wi.get(&id).await.expect("get").state;
    if let Some(next) = states.iter().find(|s| !s.eq_ignore_ascii_case(&current)) {
        wi.set_state(&id, next).await.expect("set state");
        let after = wi.get(&id).await.expect("get after");
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
