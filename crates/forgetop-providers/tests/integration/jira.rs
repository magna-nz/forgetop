//! Jira live tests: work items only (Jira has no PRs/pipelines).

use forgetop_core::provider::*;

use crate::harness;
use crate::jira_raw::JiraRaw;

#[tokio::test]
async fn jira_connectivity_and_list() {
    let jr = skip_if_none!(harness::jira(), "jira");
    assert!(jr.conn.check().await, "jira token authenticates against project {}", jr.project);
    jr.conn.work_items().expect("jira work items").list(&WorkItemQuery::default()).await.expect("list issues");
}

#[tokio::test]
async fn jira_work_item_lifecycle() {
    let jr = skip_if_none!(harness::jira(), "jira");
    let raw = JiraRaw::from_env().expect("jira raw");
    let prefix = harness::run_prefix();

    let me = raw.myself().await;
    let title = format!("{prefix} issue");
    let key = raw.create_issue(&title, &me).await;

    let wi = jr.conn.work_items().expect("jira work items");

    // Find our issue by summary in the mine-only list and capture the adapter's id.
    let id = {
        let wi = &wi;
        let title = title.as_str();
        harness::poll(harness::POLL_LIST, move || async move {
            wi.list(&WorkItemQuery { mine_only: true, include_completed: true, limit: Some(100) })
                .await
                .ok()
                .and_then(|l| l.into_iter().find(|w| w.title == title).map(|w| w.id))
        })
        .await
    }
    .expect("assigned issue appears in the mine-only list");

    assert_eq!(wi.get(&id).await.expect("get").id, id);
    let states = wi.available_states(&id).await.expect("available states (transitions)");
    assert!(!states.is_empty(), "issue reports transitions");
    wi.add_comment(&id, &format!("{prefix} note")).await.expect("comment");

    // Transition to the first available target state.
    wi.set_state(&id, &states[0]).await.expect("transition");

    raw.delete_issue(&key).await;
}
