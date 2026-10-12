//! The lists the dashboard shares with the terminal UI through the on-disk cache
//! (`forgetop_core::cache::CacheStore`, one `cache.json` per install).
//!
//! Both frontends read and write the same entries under the same keys, so whichever ran last
//! leaves the next one a warm start: a list request answers from here at once, and the live
//! fetch that follows it writes back. Last writer wins — the file is rewritten atomically and
//! every entry carries when it was fetched, so the worst a race can do is leave slightly older
//! rows for the next cold start to paint.
//!
//! The entries hold the TUI's row shapes (its `PrRow` has `pr`, the dashboard's has
//! `pull_request`; its `PipeRow` records gates by name, the dashboard's carries approvals), so
//! this module is the one place that translates. The keys and shapes mirror
//! `crates/forgetop-tui/src/app.rs`; change them together.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use forgetop_core::cache::CacheStore;
use forgetop_core::domain::{PipelineApproval, PipelineRun, ProviderType, PullRequest, WorkItem};
use forgetop_core::filter::pull_request_matches;
use forgetop_core::launchpad::{PipeInput, PrInput, WiInput};
use forgetop_core::provider::PullRequestFilter;
use serde::{Deserialize, Serialize};

use crate::dto::{self, NotifRow, PipeRow, PrRow, PrView, WiRow};

pub const KEY_WORK_ITEMS: &str = "list.work_items";
pub const KEY_PIPELINES: &str = "list.pipelines";
pub const KEY_INBOX: &str = "list.inbox";
pub const KEY_PR_POOL: &str = "list.prs.pool";
pub const KEY_LAUNCHPAD_MINE: &str = "list.launchpad.mine";
pub const KEY_LAUNCHPAD_REVIEW: &str = "list.launchpad.review";

/// The TUI's key for one PR view's last rows (`list.prs.Mine.false`).
pub fn prs_key(filter: PullRequestFilter, completed: bool) -> String {
    format!("list.prs.{filter:?}.{completed}")
}

/// How many rows a PR view derived from the pool keeps — the TUI's `PR_VIEW_CAP`, which is the
/// `limit` every per-view fetch passes.
const PR_VIEW_CAP: usize = 50;

/// A pull request as the TUI caches it.
#[derive(Serialize, Deserialize)]
pub struct TuiPrRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub pr: PullRequest,
}

/// A work item as the TUI caches it.
#[derive(Serialize, Deserialize)]
pub struct TuiWiRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub wi: WorkItem,
}

/// A pipeline run as the TUI caches it: the run exactly as the provider reported it (the
/// Waiting status is derived at display time from `gates`), the gates holding it by name, and
/// whether one of them is this user's to answer.
#[derive(Serialize, Deserialize)]
pub struct TuiPipeRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub run: PipelineRun,
    pub definition_name: Option<String>,
    pub awaiting_approval: bool,
    #[serde(default)]
    pub triggered_by_me: bool,
    #[serde(default)]
    pub gates: Vec<String>,
}

/// The TUI's unfiltered pull-request pool, from which it derives every PR view. Read only: the
/// dashboard never has the whole account's worth of rows to write it, so it writes the per-view
/// keys instead. Unknown fields are ignored, defaults cover a pool written by an older build.
#[derive(Deserialize)]
struct TuiPrPool {
    #[serde(default)]
    open: Vec<TuiPrRow>,
    #[serde(default)]
    completed: Vec<TuiPrRow>,
    /// connection_id → the signed-in user's handle there; absent means "could not tell", and
    /// `pull_request_matches` then passes every row through rather than hiding them.
    #[serde(default)]
    me: HashMap<String, Option<String>>,
    #[serde(default)]
    needs_decoration: HashMap<String, bool>,
}

/// A value read from the cache, with when it was fetched so the response can say so.
pub struct Cached<T> {
    pub value: T,
    pub fetched_at: DateTime<Utc>,
}

// ---- reading ----

fn pr_row(r: TuiPrRow, needs_decoration: &HashMap<String, bool>) -> PrRow {
    // The pool records which connections list without decoration; a per-view entry does not,
    // and GitHub is the one provider whose list endpoint omits those fields.
    let needs = needs_decoration
        .get(&r.connection_id)
        .copied()
        .unwrap_or(matches!(r.provider, ProviderType::GitHub));
    PrRow { connection_id: r.connection_id, connection: r.connection, provider: r.provider, pull_request: r.pr, needs_decoration: needs }
}

fn pr_input(r: TuiPrRow) -> PrInput {
    PrInput { connection_id: r.connection_id, connection: r.connection, provider: r.provider, pr: r.pr }
}

/// The TUI's own derivation of a view from the pool: the completed or open rows, the filter
/// applied with that connection's identity, capped like a fetched view.
fn derive_from_pool(pool: TuiPrPool, filter: PullRequestFilter, completed: bool) -> (Vec<TuiPrRow>, HashMap<String, bool>) {
    let TuiPrPool { open, completed: done, me, needs_decoration } = pool;
    let rows = if completed { done } else { open };
    let rows = rows
        .into_iter()
        .filter(|r| pull_request_matches(&r.pr, filter, me.get(&r.connection_id).and_then(|m| m.as_deref())))
        .take(PR_VIEW_CAP)
        .collect();
    (rows, needs_decoration)
}

/// One PR view's rows, as last known. The view's own entry wins (it is what the TUI paints at
/// launch); otherwise the view is derived from the pool.
pub fn pull_requests(cache: &CacheStore, view: PrView) -> Option<Cached<Vec<PrRow>>> {
    let (filter, completed) = view.parts();
    if let Some(entry) = cache.get::<Vec<TuiPrRow>>(&prs_key(filter, completed)) {
        let none = HashMap::new();
        let value = entry.value.into_iter().filter(|r| view.keep(&r.pr)).map(|r| pr_row(r, &none)).collect();
        return Some(Cached { value, fetched_at: entry.fetched_at });
    }
    let entry = cache.get::<TuiPrPool>(KEY_PR_POOL)?;
    let (rows, needs_decoration) = derive_from_pool(entry.value, filter, completed);
    let value = rows.into_iter().filter(|r| view.keep(&r.pr)).map(|r| pr_row(r, &needs_decoration)).collect();
    Some(Cached { value, fetched_at: entry.fetched_at })
}

pub fn work_items(cache: &CacheStore) -> Option<Cached<Vec<WiRow>>> {
    let entry = cache.get::<Vec<TuiWiRow>>(KEY_WORK_ITEMS)?;
    let value = entry
        .value
        .into_iter()
        .map(|r| WiRow { connection_id: r.connection_id, connection: r.connection, provider: r.provider, work_item: r.wi })
        .collect();
    Some(Cached { value, fetched_at: entry.fetched_at })
}

/// A cached run as the dashboard lists it: the status the gates imply, and each gate as an
/// approval the user can answer only if the TUI recorded that one of them was theirs.
fn pipe_row(r: TuiPipeRow) -> PipeRow {
    let TuiPipeRow { connection_id, connection, provider, mut run, definition_name, awaiting_approval, gates, .. } = r;
    run.status = run.shown_status(!gates.is_empty());
    let approvals = gates
        .into_iter()
        .map(|name| PipelineApproval { id: name.clone(), name, can_respond: awaiting_approval, blocks_run: true })
        .collect();
    PipeRow { connection_id, connection, provider, run, definition_name, approvals }
}

/// A cached run as a Command Center input — the same derivation the live `pipe_inputs` makes.
pub fn pipe_input(r: TuiPipeRow) -> PipeInput {
    let TuiPipeRow { connection_id, connection, provider, mut run, definition_name, awaiting_approval, triggered_by_me, gates } = r;
    run.status = run.shown_status(!gates.is_empty());
    PipeInput { connection_id, connection, provider, run, definition_name, awaiting_approval, triggered_by_me }
}

pub fn pipelines(cache: &CacheStore) -> Option<Cached<Vec<PipeRow>>> {
    let entry = cache.get::<Vec<TuiPipeRow>>(KEY_PIPELINES)?;
    Some(Cached { value: entry.value.into_iter().map(pipe_row).collect(), fetched_at: entry.fetched_at })
}

pub fn notifications(cache: &CacheStore) -> Option<Cached<Vec<NotifRow>>> {
    // Same shape on both sides.
    let entry = cache.get::<Vec<NotifRow>>(KEY_INBOX)?;
    Some(Cached { value: entry.value, fetched_at: entry.fetched_at })
}

/// The Command Center from the cached lists. Needs both PR slices (the TUI's launchpad keys, or
/// the pool to derive them from); work items and pipelines fall back to empty, and the response
/// is dated by its oldest part so the page knows how old what it shows is.
pub fn launchpad(cache: &CacheStore) -> Option<Cached<dto::LaunchpadResponse>> {
    let slice = |key: &str, filter: PullRequestFilter, completed: bool| -> Option<(Vec<PrInput>, DateTime<Utc>)> {
        if let Some(entry) = cache.get::<Vec<TuiPrRow>>(key) {
            return Some((entry.value.into_iter().map(pr_input).collect(), entry.fetched_at));
        }
        let entry = cache.get::<TuiPrPool>(KEY_PR_POOL)?;
        let (rows, _) = derive_from_pool(entry.value, filter, completed);
        Some((rows.into_iter().map(pr_input).collect(), entry.fetched_at))
    };
    let (review, review_at) = slice(KEY_LAUNCHPAD_REVIEW, PullRequestFilter::ReviewRequested, false)?;
    let (mine, mine_at) = slice(KEY_LAUNCHPAD_MINE, PullRequestFilter::Mine, true)?;
    let mut fetched_at = review_at.min(mine_at);
    let wis = match cache.get::<Vec<TuiWiRow>>(KEY_WORK_ITEMS) {
        Some(entry) => {
            fetched_at = fetched_at.min(entry.fetched_at);
            entry
                .value
                .into_iter()
                .map(|r| WiInput { connection_id: r.connection_id, connection: r.connection, provider: r.provider, wi: r.wi })
                .collect()
        }
        None => Vec::new(),
    };
    let pipes = match cache.get::<Vec<TuiPipeRow>>(KEY_PIPELINES) {
        Some(entry) => {
            fetched_at = fetched_at.min(entry.fetched_at);
            entry.value.into_iter().map(pipe_input).collect()
        }
        None => Vec::new(),
    };
    Some(Cached { value: dto::build_launchpad(&review, &mine, &wis, &pipes), fetched_at })
}

// ---- writing ----

pub fn store_pull_requests(cache: &CacheStore, view: PrView, rows: &[PrRow], at: DateTime<Utc>) {
    let (filter, completed) = view.parts();
    let tui: Vec<TuiPrRow> = rows
        .iter()
        .map(|r| TuiPrRow { connection_id: r.connection_id.clone(), connection: r.connection.clone(), provider: r.provider, pr: r.pull_request.clone() })
        .collect();
    cache.put(&prs_key(filter, completed), &tui, at);
}

pub fn store_launchpad_prs(cache: &CacheStore, key: &str, inputs: &[PrInput], at: DateTime<Utc>) {
    let tui: Vec<TuiPrRow> = inputs
        .iter()
        .map(|i| TuiPrRow { connection_id: i.connection_id.clone(), connection: i.connection.clone(), provider: i.provider, pr: i.pr.clone() })
        .collect();
    cache.put(key, &tui, at);
}

pub fn store_work_items(cache: &CacheStore, inputs: &[WiInput], at: DateTime<Utc>) {
    let tui: Vec<TuiWiRow> = inputs
        .iter()
        .map(|i| TuiWiRow { connection_id: i.connection_id.clone(), connection: i.connection.clone(), provider: i.provider, wi: i.wi.clone() })
        .collect();
    cache.put(KEY_WORK_ITEMS, &tui, at);
}

pub fn store_pipelines(cache: &CacheStore, rows: &[TuiPipeRow], at: DateTime<Utc>) {
    cache.put(KEY_PIPELINES, &rows, at);
}

pub fn store_notifications(cache: &CacheStore, rows: &[NotifRow], at: DateTime<Utc>) {
    cache.put(KEY_INBOX, &rows, at);
}

// ---- flushing ----

/// Writes the cache to disk shortly after something changed it, once per burst: every list the
/// dashboard polls lands within a second or two of the others, and one flush covers them all.
pub struct Flusher {
    cache: Arc<CacheStore>,
    pending: AtomicBool,
}

impl Flusher {
    pub fn new(cache: Arc<CacheStore>) -> Self {
        Self { cache, pending: AtomicBool::new(false) }
    }

    pub fn schedule(self: &Arc<Self>) {
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            me.pending.store(false, Ordering::Release);
            me.cache.flush().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(name: &str) -> forgetop_core::domain::User {
        forgetop_core::domain::User { id: name.into(), display_name: name.into(), handle: Some(name.into()), avatar_url: None }
    }

    fn pr(id: &str, author: &str, reviewer: Option<&str>) -> TuiPrRow {
        let reviewers = reviewer
            .map(|r| vec![forgetop_core::domain::Reviewer { user: user(r), vote: forgetop_core::domain::ReviewVote::NoVote, is_required: false }])
            .unwrap_or_default();
        TuiPrRow {
            connection_id: "gh".into(),
            connection: "GitHub".into(),
            provider: ProviderType::GitHub,
            pr: PullRequest {
                id: id.into(),
                repository: Some("acme/pay".into()),
                number: None,
                title: format!("PR {id}"),
                description: None,
                author: user(author),
                status: forgetop_core::domain::PullRequestStatus::Open,
                is_draft: false,
                source_ref: Some("x".into()),
                target_ref: Some("main".into()),
                reviewers,
                labels: Vec::new(),
                checks: forgetop_core::domain::CheckStatus::Passed,
                check_summary: None,
                mergeable: forgetop_core::domain::MergeableState::Unknown,
                changed_files: 0,
                additions: 0,
                deletions: 0,
                created_at: None,
                updated_at: None,
                url: None,
            },
        }
    }

    #[tokio::test]
    async fn views_derive_from_the_pool_with_the_tui_identity() {
        // A disabled store never stores, so build one on a scratch path.
        let path = std::env::temp_dir().join(format!("forgetop-cached-{}.json", uuid::Uuid::new_v4()));
        let cache = CacheStore::new(&path);
        let pool = json!({
            "open": [pr("1", "sam", Some("priya")), pr("2", "priya", Some("sam")), pr("3", "tom", None)],
            "completed": Vec::<TuiPrRow>::new(),
            "me": { "gh": "sam" },
            "needs_decoration": { "gh": true }
        });
        cache.put(KEY_PR_POOL, &pool, Utc::now());

        let all = pull_requests(&cache, PrView::All).expect("pool answers All");
        assert_eq!(all.value.len(), 3);
        assert!(all.value.iter().all(|r| r.needs_decoration), "the pool says GitHub lists undecorated");
        let yours = pull_requests(&cache, PrView::Yours).expect("pool answers Yours");
        assert_eq!(yours.value.iter().map(|r| r.pull_request.id.as_str()).collect::<Vec<_>>(), ["1"]);
        let review = pull_requests(&cache, PrView::ReviewRequested).expect("pool answers Review requested");
        assert_eq!(review.value.iter().map(|r| r.pull_request.id.as_str()).collect::<Vec<_>>(), ["2"]);
        assert!(pull_requests(&cache, PrView::MergedByYou).expect("pool answers Merged").value.is_empty());

        // A view's own entry, as the TUI leaves one, wins over the pool.
        cache.put(&prs_key(PullRequestFilter::All, false), &json!([pr("9", "sam", None)]), Utc::now() + chrono::Duration::seconds(1));
        let all = pull_requests(&cache, PrView::All).expect("view entry");
        assert_eq!(all.value.iter().map(|r| r.pull_request.id.as_str()).collect::<Vec<_>>(), ["9"]);
        let _ = std::fs::remove_file(path);
    }
}
