//! Application state and the (async) update logic driven by the event loop.

use std::cell::Cell;
use std::cmp::{Ordering, Reverse};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Local, Utc};
use forgetop_core::cache::{CachePut, CacheStore};
use forgetop_core::config::{NotificationPrefs, SavedView, SortPref};
use forgetop_core::domain::*;
use forgetop_core::filter::pull_request_matches;
use forgetop_core::provider::*;
use forgetop_core::service::{ConfigService, ConnectionHealth, ConnectionHealthService, SectionService};
use ratatui::widgets::TableState;
use tokio::sync::mpsc;

use crate::launchpad;
use crate::overlay::{Action, InputKind, Outcome, Overlay, PickerKind, SearchItem, SearchKind, ToggleItem, ToggleKind, WiField};
use crate::palette::{self, CommandContext, GoTo, PaletteItem, PaletteKind, PaletteTarget};
use crate::theme::Theme;
use crate::wizard::{provider_sections, section_label, Wizard, WizardOutcome};

const DASHBOARD_OPEN_FAILURE_CONTEXT: &str = "action";
const DASHBOARD_OPEN_FAILURE_MESSAGE: &str = "failed to open local dashboard in browser";
const FEEDBACK_ISSUE_URL: &str =
    "https://github.com/magna-nz/forgetop/issues/new?template=feedback.yml";
const FEEDBACK_OPEN_FAILURE_CONTEXT: &str = "action";
const FEEDBACK_OPEN_FAILURE_MESSAGE: &str = "failed to open GitHub feedback form in browser";
const DIAG_FAILURE_MESSAGE: &str = "operation failed";
const DIAG_REFRESH: &str = "tui.refresh";
const DIAG_ACTION: &str = "tui.action";
const DIAG_RELOAD_PULL_REQUESTS: &str = "tui.reload.pull_requests";
const DIAG_RELOAD_WORK_ITEMS: &str = "tui.reload.work_items";
const DIAG_RELOAD_PIPELINES: &str = "tui.reload.pipelines";
const DIAG_PR_FEEDS: &str = "tui.pr.feeds";
const DIAG_PR_DETAIL: &str = "tui.pr.detail";
const DIAG_PR_THREADS: &str = "tui.pr.threads";
const DIAG_PR_CHANGES: &str = "tui.pr.changes";
const DIAG_PR_CHECKS: &str = "tui.pr.checks";
const DIAG_PR_COMMITS: &str = "tui.pr.commits";
const DIAG_PR_COMMIT_CHANGES: &str = "tui.pr.commit_changes";
const DIAG_PR_TIMELINE: &str = "tui.pr.timeline";
const DIAG_WI_FEEDS: &str = "tui.work_item.feeds";
const DIAG_WI_THREADS: &str = "tui.work_item.threads";
const DIAG_WI_STATES: &str = "tui.work_item.states";
const DIAG_WI_TIMELINE: &str = "tui.work_item.timeline";
const DIAG_WI_ASSIGNABLE: &str = "tui.work_item.assignable_users";
const DIAG_PIPELINE_FEEDS: &str = "tui.pipeline.feeds";
const DIAG_PIPELINE_DISCOVERY: &str = "tui.pipeline.discovery";
const DIAG_PIPELINE_RUN: &str = "tui.pipeline.run";
const DIAG_PIPELINE_APPROVALS: &str = "tui.pipeline.approvals";
const DIAG_PIPELINE_LOGS: &str = "tui.pipeline.logs";
const DIAG_NOTIFICATION_SCAN_FEEDS: &str = "tui.notification_scan.feeds";
const DIAG_INBOX_FEEDS: &str = "tui.inbox.feeds";
const DIAG_INBOX_PR_DETAIL: &str = "tui.inbox.pr_detail";
const DIAG_INBOX_WI_DETAIL: &str = "tui.inbox.work_item_detail";
const DIAG_INBOX_MARK_READ: &str = "tui.inbox.mark_read";
const DIAG_INBOX_MARK_ALL_READ: &str = "tui.inbox.mark_all_read";

// ---- cache keys ----
//
// One entry per seeded list section. Only the PR list varies by query — the user picks a
// filter and can ask for completed PRs — so its key carries both, or a review-requested list
// would be painted back under the "all" heading on the next launch.
const CACHE_KEY_WORK_ITEMS: &str = "list.work_items";
const CACHE_KEY_PIPELINES: &str = "list.pipelines";
const CACHE_KEY_INBOX: &str = "list.inbox";
const CACHE_KEY_LAUNCHPAD_MINE: &str = "list.launchpad.mine";
const CACHE_KEY_LAUNCHPAD_REVIEW: &str = "list.launchpad.review";

fn prs_cache_key(filter: PullRequestFilter, completed: bool) -> String {
    format!("list.prs.{filter:?}.{completed}")
}

/// A connection's credentials reach every repository it can see, so a bare `conn_id + id` key
/// would let two repositories' PR #7 collide onto the same cache entry and show one repo's
/// diff under the other's title. `item.repo` (via `PullRequest::item_ref`) is the same address
/// every detail call is already made against, so the cache key rides along with it.
fn pr_detail_cache_key(conn_id: &str, item: &ItemRef) -> String {
    format!("detail.pr.{conn_id}.{}.{}", item.repo.as_deref().unwrap_or("-"), item.id)
}

/// Mirrors [`pr_detail_cache_key`]. Jira and Linear work items are project-, not
/// repo-addressed, so `item.repo` is `None` there and `unwrap_or("-")` covers it the same way.
fn wi_detail_cache_key(conn_id: &str, item: &ItemRef) -> String {
    format!("detail.wi.{conn_id}.{}.{}", item.repo.as_deref().unwrap_or("-"), item.id)
}

/// Mirrors [`pr_detail_cache_key`]: a pipeline run is addressed by (connection, repo, id) the
/// same way a PR is, and a connection's credentials reach every repository it can see.
fn pipeline_detail_cache_key(conn_id: &str, run: &ItemRef) -> String {
    format!("detail.pipeline.{conn_id}.{}.{}", run.repo.as_deref().unwrap_or("-"), run.id)
}

/// Reads one cached section into `field` and folds its age into `oldest`.
///
/// A miss — cold cache, disabled store, or an entry whose shape no longer parses — leaves the
/// field untouched, which is the empty list the user would have seen anyway.
fn seed_section<T: serde::de::DeserializeOwned>(
    cache: &CacheStore,
    key: &str,
    field: &mut Vec<T>,
    oldest: &mut Option<DateTime<Utc>>,
) {
    let Some(entry) = cache.get::<Vec<T>>(key) else { return };
    *field = entry.value;
    // The header reports how stale the *most stale* thing on screen is, so the oldest seeded
    // section wins — a fresh inbox must not make a day-old PR list read as current.
    *oldest = Some(oldest.map_or(entry.fetched_at, |prev| prev.min(entry.fetched_at)));
}

/// Rewrites one cached section without a removed connection's rows.
///
/// `seed_section` is deliberately unfiltered, so rows left here outlive the connection: close
/// the app before the post-removal reload lands and the next launch seeds them straight back.
/// Goes through [`CacheStore::rewrite`], not `put`: the entry keeps its `fetched_at`, because
/// dropping rows from a list doesn't make the rest of it any fresher.
fn purge_cached_section<T>(cache: &CacheStore, key: &str, conn_id: &str, connection_of: impl Fn(&T) -> &str)
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    cache.rewrite::<Vec<T>>(key, |rows| rows.into_iter().filter(|row| connection_of(row) != conn_id).collect());
}

/// Clears the unread flag on the cached inbox rows that `matches` picks out.
///
/// `self.inbox` is what this session shows; the cache is what the *next launch* shows. Without
/// this, quitting before the next poll repaints a notification the user already read as unread.
/// Goes through [`CacheStore::rewrite`] for the same reason `purge_cached_section` does: reading
/// a notification doesn't make the list any fresher, and `put` would refuse a write whose
/// timestamp isn't newer anyway.
fn mark_cached_inbox_read(cache: &CacheStore, matches: impl Fn(&NotifRow) -> bool) {
    cache.rewrite::<Vec<NotifRow>>(CACHE_KEY_INBOX, |rows| {
        rows.into_iter()
            .map(|mut row| {
                if matches(&row) {
                    row.notification.unread = false;
                }
                row
            })
            .collect()
    });
}

/// Every cached list a connection contributes rows to. The PR list is keyed by filter and
/// completed-ness, so all six of its combinations are purged, not just the one on screen.
fn purge_cached_rows(cache: &CacheStore, conn_id: &str) {
    for filter in [PullRequestFilter::All, PullRequestFilter::Mine, PullRequestFilter::ReviewRequested] {
        for completed in [false, true] {
            purge_cached_section::<PrRow>(cache, &prs_cache_key(filter, completed), conn_id, |r| &r.connection_id);
        }
    }
    purge_cached_section::<PrRow>(cache, CACHE_KEY_LAUNCHPAD_MINE, conn_id, |r| &r.connection_id);
    purge_cached_section::<PrRow>(cache, CACHE_KEY_LAUNCHPAD_REVIEW, conn_id, |r| &r.connection_id);
    purge_cached_section::<WiRow>(cache, CACHE_KEY_WORK_ITEMS, conn_id, |r| &r.connection_id);
    purge_cached_section::<PipeRow>(cache, CACHE_KEY_PIPELINES, conn_id, |r| &r.connection_id);
    purge_cached_section::<NotifRow>(cache, CACHE_KEY_INBOX, conn_id, |r| &r.connection_id);
}

/// Puts one freshly-fetched section on screen, unless doing so would replace rows with nothing.
///
/// `ok` is that section's "every feed I consulted answered" flag. Empty *and* failed means the
/// outage erased the rows, not that they are gone — so whatever is on screen (often what the
/// cache seeded at launch) stays. Non-empty and failed is partial live data, which is taken:
/// with the failure already surfaced in the status line, some live rows beat stale ones. A
/// section that answered is always taken, including an authoritative empty.
/// Drops every pool row belonging to connections the app no longer shows, so a later local
/// derivation cannot resurrect rows that `prs` / the Launchpad have already had pruned out.
/// Callers prune the visible lists themselves; this keeps the pool they are derived from honest.
fn retain_pool_rows(pool: &mut PrPool, keep: impl Fn(&PrRow) -> bool) {
    pool.open.retain(&keep);
    pool.completed.retain(&keep);
}

fn take_section<T>(field: &mut Vec<T>, incoming: Vec<T>, ok: bool) {
    if ok || !incoming.is_empty() {
        *field = incoming;
    }
}

/// The same swap for the inline `reload_*` methods, which are awaited on the key-handler path.
///
/// They used to `clear()` the list first. Nothing was ever *drawn* mid-handler — the loop paints
/// at the top and `on_key` is awaited to completion — so this was never a visible flicker. What
/// it was is a failure mode: a reload that errored left the cleared list behind, so an outage
/// blanked a perfectly good list and the user lost rows they still had. Building into a local
/// vec and swapping here means a failed reload changes nothing on screen. It also stops the
/// clear from wiping an optimistic edit the same handler made just before it.
///
/// `ok` is derived the only way an inline reload can know it: whether this section pushed a
/// failure onto `errors` while it ran. That reduces to [`take_section`]'s rule — empty *and*
/// failed means the outage erased the rows, so what is on screen stays.
fn take_inline_section<T>(field: &mut Vec<T>, incoming: Vec<T>, errors: &[String], errors_before: usize) {
    take_section(field, incoming, errors.len() == errors_before);
}

/// Whether this provider's work items and pipelines are addressed by *project* rather than by
/// repository, so their rows' `repository` field holds a Team Project name and a repository
/// scope entry ("Project/Repo") is not something it can be compared against.
fn project_addressed_sections(provider: ProviderType) -> bool {
    matches!(provider, ProviderType::AzureDevOps)
}

type FeedbackOpener = fn(&str) -> std::result::Result<(), String>;

fn system_feedback_opener(target: &str) -> std::result::Result<(), String> {
    open::that(target).map_err(|error| error.to_string())
}

/// The section index behind each tab position (0 = Pull Requests, 1 = Work Items, 2 = Pipelines).
fn section_of(index: usize) -> Section {
    match index {
        0 => Section::PullRequests,
        1 => Section::WorkItems,
        _ => Section::Pipelines,
    }
}

fn index_of(section: Section) -> usize {
    match section {
        Section::PullRequests => 0,
        Section::WorkItems => 1,
        Section::Pipelines => 2,
    }
}

/// The three top-level tabs, in order.
pub const TABS: [&str; 3] = ["Pull Requests", "Work Items", "Pipelines"];

/// PR detail sub-tabs, in order.
pub const PR_TABS: [&str; 4] = ["Conversation", "Commits", "Checks", "Diff"];

/// Live services the app talks to. Cheap to clone (all `Arc`).
#[derive(Clone)]
pub struct AppDeps {
    pub sections: Arc<SectionService>,
    pub health: Arc<ConnectionHealthService>,
    pub config: Arc<ConfigService>,
    /// Last-known list data, painted at startup so the first fetch doesn't run against a blank
    /// screen. Disabled (a no-op) for `--demo` and tests.
    pub cache: Arc<CacheStore>,
}

/// A completed background job, applied to the app on the render loop.
pub enum AppEvent {
    /// A full refresh finished fetching.
    Reloaded(Box<Reloaded>),
    /// A PR view's detail (threads/files/checks/commits) finished fetching. Carries the
    /// *fetch*, not a finished [`PrDetail`]: which of the four calls failed is what decides
    /// whether the cached value for that field survives, so it can't be flattened out here.
    PrDetailLoaded { key: String, detail: Box<PrDetailFetch>, fetched_at: DateTime<Utc> },
    /// A work-item view's detail (its comment thread) finished fetching. Mirrors
    /// [`AppEvent::PrDetailLoaded`].
    WiDetailLoaded { key: String, detail: Box<WiDetailFetch>, fetched_at: DateTime<Utc> },
    /// A pipeline drill-in's detail (run/approvals/capabilities) finished fetching. Mirrors
    /// [`AppEvent::PrDetailLoaded`].
    PipelineDetailLoaded { key: String, detail: Box<PipelineDetailFetch>, fetched_at: DateTime<Utc> },
    /// A batch of per-row PR decorations finished fetching. `None` for a row means that call
    /// failed; it is not cached, so the row stays undecorated and may be retried.
    PrDecorationsLoaded { items: Vec<((String, String), Option<PrDecoration>)> },
    /// A pipeline job's log finished fetching (the whole log — providers don't send deltas).
    /// Applied only if the drill-in still shows that job's log pane.
    PipelineLogsLoaded { conn_id: String, run_id: String, job_id: String, text: std::result::Result<String, String> },
}

/// A snapshot of everything the background fetch needs from `self` at spawn time, so it
/// can run without borrowing the app.
struct ReloadParams {
    notifications: NotificationPrefs,
    review_seen: HashSet<(String, String)>,
    pr_review_seen: HashMap<(String, String), (bool, bool)>,
    scan_seeded: bool,
    notifier: Arc<dyn Notifier>,
    /// The open pipeline view's (connection_id, run_id), so the refresh keeps it live.
    /// The open drill-in's `(connection_id, run)` — the run is addressed, so the background
    /// refresh reaches the same repository the view was opened from.
    open_pipeline: Option<(String, ItemRef)>,
    /// Discover repositories during this fetch. Set only while `repo_catalog` is empty, so
    /// discovery still runs once rather than on every poll — it just no longer runs inline.
    seed_catalog: bool,
    /// When this reload was asked for. Taken at spawn time and carried all the way to the
    /// cache write: see [`App::reload_params`].
    requested_at: DateTime<Utc>,
}

/// The PR-notification scan's new seen-sets (notifications are fired during the scan).
struct PrScan {
    review_seen: Option<HashSet<(String, String)>>,
    pr_review_seen: Option<HashMap<(String, String), (bool, bool)>>,
}

/// The result of a full fetch, ready to be folded back into the app.
pub struct Reloaded {
    /// The unfiltered pool every PR view is derived from. Replaces the four separately
    /// filtered lists this used to carry.
    pr_pool: PrPool,
    wis: Vec<WiRow>,
    pipes: Vec<PipeRow>,
    inbox: Vec<NotifRow>,
    health: Vec<ConnectionHealth>,
    scan: Option<PrScan>,
    /// Fresh (run_id, run, approvals) for the open pipeline view, if one is open.
    /// `None` approvals means the gate check failed, as distinct from `Some(vec![])` meaning
    /// the run genuinely has none — the view must not clear real gates on a failed check.
    open_pipeline: Option<(String, PipelineRun, Option<Vec<PipelineApproval>>)>,
    errors: Vec<String>,
    /// Repository discovery, when this fetch was asked to seed it. `None` means "not this
    /// time" and leaves the existing catalog alone.
    catalog: Option<HashMap<String, RepositoryPage>>,
    /// Which sections came back whole, and so may be written to the cache.
    sections_ok: SectionsOk,
    /// When this reload was *asked for*, not when it landed. See [`App::request_reload`].
    requested_at: DateTime<Utc>,
}

/// Per-section "every feed I consulted answered" flags for one reload.
///
/// One global flag can't express this: a Work Items outage says nothing about whether the PR
/// list is whole, and a PR list that lost one connection out of three still arrives non-empty.
#[derive(Clone, Copy)]
struct SectionsOk {
    prs: bool,
    wis: bool,
    pipes: bool,
    inbox: bool,
    lp_mine: bool,
    lp_review: bool,
}

impl SectionsOk {
    /// True only when every section came back whole. What the header's cached-age footer keys
    /// off: while any section is still showing rows the cache seeded, "showing Nm old" is still
    /// the truth.
    fn all(&self) -> bool {
        self.prs && self.wis && self.pipes && self.inbox && self.lp_mine && self.lp_review
    }
}

/// Test-only: the live fetch sets each flag from its own section, so an all-true constructor
/// would be dead code outside the tests that build a `Reloaded` by hand.
#[cfg(test)]
impl SectionsOk {
    fn complete() -> Self {
        Self { prs: true, wis: true, pipes: true, inbox: true, lp_mine: true, lp_review: true }
    }
}

/// One pipeline run, tagged with the connection it came from (for the provider column).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PipeRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub run: PipelineRun,
    /// The pipeline (definition) name — e.g. "CI Build" — resolved from the run's
    /// definition_id, distinct from the run's own name (e.g. a release like "10.1.100").
    pub definition_name: Option<String>,
    /// True when this run has a gate the authenticated user can approve/reject.
    pub awaiting_approval: bool,
}

/// How the Pipelines list groups its runs.
///
/// Grouping is a *view* over the same filtered rows, not a different query: every mode
/// below renders the identical set of runs, only arranged differently. [`PipeGroup::Off`]
/// is the ungrouped list the tab had before grouping existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PipeGroup {
    /// One header per pipeline definition — the default. Answers "is a workflow failing
    /// repeatedly", which is the question the tab is usually open for and the one the flat
    /// list cannot answer without counting rows by eye.
    #[default]
    Pipeline,
    /// One header per trigger: the runs a single push or tag started together.
    Trigger,
    /// One header per branch.
    Branch,
    /// No grouping — one line per run.
    Off,
}

impl PipeGroup {
    /// The persisted spelling. Kept stable: it lands in the user's config file.
    pub fn as_str(self) -> &'static str {
        match self {
            PipeGroup::Pipeline => "pipeline",
            PipeGroup::Trigger => "trigger",
            PipeGroup::Branch => "branch",
            PipeGroup::Off => "off",
        }
    }

    /// Parses a persisted spelling. Anything unrecognised falls back to the default rather
    /// than erroring — a config written by a newer build must not break an older one.
    pub fn parse(s: &str) -> PipeGroup {
        match s {
            "trigger" => PipeGroup::Trigger,
            "branch" => PipeGroup::Branch,
            "off" => PipeGroup::Off,
            _ => PipeGroup::Pipeline,
        }
    }

    /// The `G` cycle: the default first, the ungrouped list last.
    pub fn next(self) -> PipeGroup {
        match self {
            PipeGroup::Pipeline => PipeGroup::Trigger,
            PipeGroup::Trigger => PipeGroup::Branch,
            PipeGroup::Branch => PipeGroup::Off,
            PipeGroup::Off => PipeGroup::Pipeline,
        }
    }
}

/// A group header in the Pipelines list: the roll-up of the runs beneath it.
///
/// Every field is one table cell. A header is column-shaped exactly like a run row, so the two
/// are measured and laid out together and each value lands under the heading describing it.
#[derive(Debug, Clone)]
pub struct PipeHead {
    /// Index into `App::pipes` of the group's most recent run — the one its status and start
    /// time describe, and the one the preview pane shows.
    pub latest: usize,
    /// Stable identity, used as the expand/collapse key. Survives a refresh so a group the
    /// user opened stays open when the rows are re-fetched.
    pub key: String,
    /// What names the group: the pipeline under [`PipeGroup::Pipeline`], the branch otherwise.
    pub subject: String,
    /// The repository every run beneath belongs to.
    pub repo: String,
    /// Short commit, for the trigger grouping whose key includes one. Empty otherwise.
    pub commit: String,
    pub runs: usize,
    pub failed: usize,
    /// The status of the **most recent** run beneath — where the pipeline stands now, not the
    /// worst thing that ever happened to it. The failed count beside it carries the history,
    /// so an older failure is still announced without colouring the present.
    pub status: PipelineRunStatus,
    /// Start of that same most recent run: Status and Started describe one run, not two.
    pub started: Option<DateTime<Utc>>,
    /// True when any run beneath has a gate this user can answer.
    pub approval: bool,
    pub expanded: bool,
    pub provider: ProviderType,
    pub connection: String,
}

/// One rendered line of the Pipelines list.
///
/// Introduced because the list used to be a 1:1 map from selection index to `pipes[i]`;
/// a header is a line that is not a run, so selection, scrolling and Enter all have to
/// address lines rather than rows. Mirrors the drill-in tree's `flatten()`.
#[derive(Debug, Clone)]
pub enum PipeLine {
    Head(PipeHead),
    /// Index into `App::pipes`.
    Run(usize),
}

/// The pipeline (definition) name for a row — "CI Build", not the run's own name.
/// Shared with the renderer so the list and the grouping can never disagree on identity.
pub fn pipe_definition_name(p: &PipeRow) -> String {
    p.definition_name
        .clone()
        .or_else(|| p.run.name.clone())
        .unwrap_or_else(|| p.run.definition_id.clone())
}

/// A run's branch, or a marker when the provider gives none — never an empty cell, which
/// reads as missing data rather than as "this run has no branch".
fn pipe_branch_label(p: &PipeRow) -> String {
    match p.run.branch.as_deref().filter(|b| !b.is_empty()) {
        Some(b) => b.to_string(),
        None => "(no branch)".to_string(),
    }
}

/// What identifies the trigger that started a run: the commit where the provider gives one,
/// otherwise the start time rounded to the minute. Without the fallback, providers that
/// leave `commit_sha` empty would put every run in a group of its own.
fn pipe_trigger_token(p: &PipeRow) -> String {
    if let Some(sha) = p.run.commit_sha.as_deref().filter(|s| !s.is_empty()) {
        return sha.to_string();
    }
    match p.run.started_at {
        Some(t) => t.format("%Y-%m-%dT%H:%M").to_string(),
        None => p.run.id.clone(),
    }
}

/// A pull request tagged with the connection it came from (for aggregation).
///
/// `Clone` so views can be derived from the held pool without consuming it — the pool outlives
/// every list built from it.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PrRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub pr: PullRequest,
}

/// Every pull request the connections will admit to, fetched once per reload, from which all
/// three PR views (All / Mine / Review requested) and both Launchpad PR buckets are derived
/// locally.
///
/// Providers build their list URL from `include_completed` and `limit` alone — the filter is
/// applied in memory afterwards — so a `list` per filter re-fetched identical rows. This used to
/// mean four calls per connection per reload (the list section, the two Launchpad buckets, and
/// the notification scan), and a fifth, blocking, every time `[`/`]` moved between views.
#[derive(Default, Clone)]
pub struct PrPool {
    /// Rows fetched with `include_completed: false`.
    open: Vec<PrRow>,
    /// Rows fetched with `include_completed: true`.
    ///
    /// **Not** a superset of `open`, which is why both are held: Bitbucket's completed state is
    /// literally `state=MERGED` and excludes open pull requests, unlike GitHub's `all`, GitLab's
    /// `all` and Azure's `all`. Deriving the open views from this one would empty them.
    completed: Vec<PrRow>,
    /// connection_id → the signed-in user's handle there. A connection absent from this map, or
    /// mapped to `None`, could not establish an identity, and `pull_request_matches` then passes
    /// all of its rows through rather than hiding them.
    me: HashMap<String, Option<String>>,
    /// connection_id → whether its list endpoint omits the decorated fields (GitHub only), so
    /// only those rows are worth spending a per-row decoration call on.
    needs_decoration: HashMap<String, bool>,
}

impl PrPool {
    fn rows(&self, completed: bool) -> &[PrRow] {
        if completed {
            &self.completed
        } else {
            &self.open
        }
    }
}

/// How many rows a derived PR view keeps, matching the `limit` the per-filter fetches used to
/// pass. The pool itself is fetched uncapped so that capping *after* filtering — which is what
/// the providers did, via `sort_and_cap` — still yields the same rows.
const PR_VIEW_CAP: usize = 50;

/// How many of a derived view's rows get a per-row decoration call. Mirrors the cap GitHub's
/// `list` applied internally when it decorated the rows it was about to return, so the round-trip
/// count per connection is unchanged — it just happens off the render loop now, and against the
/// rows of the view actually on screen rather than the first 25 of an unfiltered list.
const PR_DECORATE_CAP: usize = 25;

/// A work item tagged with the connection it came from (for aggregation).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct WiRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub wi: WorkItem,
}

/// One notification tagged with the connection it came from (for the inbox).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct NotifRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub notification: Notification,
}

/// The four fetched sections behind a full-screen PR view (Conversation/Commits/Checks/Diff),
/// cached as one unit under [`pr_detail_cache_key`] so opening a previously-seen PR paints
/// immediately instead of blocking on four sequential provider round trips.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PrDetail {
    pub threads: Vec<CommentThread>,
    pub files: Vec<FileChange>,
    pub checks: Vec<CheckRun>,
    pub commits: Vec<Commit>,
    /// The activity timeline under the Conversation tab. Defaulted so an entry cached before
    /// the timeline was fetched still reads back (as "none yet") instead of failing to decode.
    #[serde(default)]
    pub timeline: Vec<TimelineEvent>,
}

/// One detail fetch's outcome, before it is resolved against what is already known.
///
/// `None` means that call failed; `Some(vec![])` means the provider really has none. A
/// [`PrDetail`] cannot hold that distinction, and collapsing the two is how a 502 on
/// `changes()` used to blank a PR's file list on screen *and* in the cache.
#[derive(Default)]
pub struct PrDetailFetch {
    pub threads: Option<Vec<CommentThread>>,
    pub files: Option<Vec<FileChange>>,
    pub checks: Option<Vec<CheckRun>>,
    pub commits: Option<Vec<Commit>>,
    pub timeline: Option<Vec<TimelineEvent>>,
}

/// The detail fetched behind a work-item view (its comment thread and activity timeline),
/// cached as one unit under [`wi_detail_cache_key`], mirroring [`PrDetail`].
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct WiDetail {
    pub threads: Vec<CommentThread>,
    /// Defaulted so an entry cached before the timeline was fetched still decodes.
    #[serde(default)]
    pub timeline: Vec<TimelineEvent>,
}

/// `None` means that call failed; `Some(vec![])` means the provider really has none. See
/// [`PrDetailFetch`] for why a [`WiDetail`] can't hold that distinction on its own.
#[derive(Default)]
pub struct WiDetailFetch {
    pub threads: Option<Vec<CommentThread>>,
    pub timeline: Option<Vec<TimelineEvent>>,
}

/// The detail fetched behind a pipeline drill-in, cached as one unit under
/// [`pipeline_detail_cache_key`]. `supports_approvals`/`can_respond_approvals` are provider
/// *capabilities*, not run state — stable and safe to cache alongside the live run data.
///
/// `approvals` is cached on the way out, so the entry stays complete for any other reader, but
/// — see [`PipelineView`] and `open_pipeline_for` — it is deliberately never read back in on
/// open: a cached gate that was already approved would offer the user an action that no longer
/// exists, and [`PipelineView::actionable_approvals`] drives a real write.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineDetail {
    pub run: PipelineRun,
    pub approvals: Vec<PipelineApproval>,
    pub supports_approvals: bool,
    pub can_respond_approvals: bool,
}

/// `None` means that call failed. See [`PrDetailFetch`] for why the distinction from an
/// authoritatively empty answer matters.
#[derive(Default)]
pub struct PipelineDetailFetch {
    pub run: Option<PipelineRun>,
    pub approvals: Option<Vec<PipelineApproval>>,
    pub supports_approvals: Option<bool>,
    pub can_respond_approvals: Option<bool>,
}

/// A section's repository scope, as the header indicator and empty state read it.
#[derive(Clone, Debug)]
pub struct ScopeSummary {
    /// The repo-addressed connections feeding this section.
    pub connections: Vec<String>,
    /// How many repositories are currently fetched from.
    pub selected: usize,
    /// How many the credentials can reach, once discovery has run. `None` until then, so the
    /// indicator never invents a denominator.
    pub available: Option<usize>,
    /// Discovery hit its page ceiling — the total reads "37+", never a cap dressed as a total.
    pub truncated: bool,
    /// Every connection explicitly chose no repositories. Distinct from "nothing to show".
    pub none_selected: bool,
}

impl ScopeSummary {
    /// "Repos · 5 of 37" for the section header. Truncation is marked, never silent.
    pub fn label(&self) -> String {
        match self.available {
            Some(n) => format!("Repos · {} of {}{}", self.selected, n, if self.truncated { "+" } else { "" }),
            None => format!("Repos · {}", self.selected),
        }
    }
}

pub struct App {
    pub theme: Theme,
    pub active: usize,
    pub prs: Vec<PrRow>,
    pub wis: Vec<WiRow>,
    pub pipes: Vec<PipeRow>,
    /// The cross-provider notification inbox (distinct from `notifications`, the desktop-ping prefs).
    pub inbox: Vec<NotifRow>,
    pub inbox_sel: usize,
    /// URL of the embedded web dashboard, if its server started. `B` opens it.
    pub dashboard_url: Option<String>,
    /// Browser boundary for the global feedback shortcut. A function pointer keeps the
    /// production path trivial and lets tests exercise ordinary key dispatch without I/O.
    feedback_opener: FeedbackOpener,
    pub pr_state: TableState,
    pub wi_state: TableState,
    pub pipe_state: TableState,
    pub health: Vec<ConnectionHealth>,
    /// Which sections are shown in the tab bar, indexed by section (0=PR,1=WI,2=Pipelines).
    pub visible: [bool; 3],
    /// Per-section repository-scope summary (0=PR, 1=WI, 2=Pipelines), for the header indicator
    /// and the "no repositories selected" empty state. Recomputed on every reload from config.
    pub repo_scope: [Option<ScopeSummary>; 3],
    /// What discovery found per connection, keyed by connection id. Fetched once at startup and
    /// again whenever the picker opens — the denominator in "5 of 37" has to be a real count.
    pub repo_catalog: HashMap<String, RepositoryPage>,
    /// Connection ids behind an open "which connection?" scope picker, indexed by its selection.
    repo_scope_choices: Vec<String>,
    pub status: String,
    pub loading: bool,
    /// Scroll offset for the current list; body height captured during render.
    pub list_scroll: u16,
    pub content_h: u16,
    /// Max scroll offset for the open PR/WI view, captured during render for clamping.
    pub detail_scroll_max: u16,
    pub pr_filter: PullRequestFilter,
    /// Every pull request the last reload fetched, unfiltered. `prs`, `lp_prs_mine` and
    /// `lp_prs_review` are all derived from it, so moving between views costs no network.
    pub pr_pool: PrPool,
    /// Whether a fetch has ever populated `pr_pool`. An empty pool that was never filled is not
    /// the same as one that genuinely came back empty, and only the latter may clear a list.
    pr_pool_loaded: bool,
    /// Decorated fields fetched per row, keyed by `(connection_id, pull request id)`. Held
    /// beside the pool rather than merged into it so a reload replacing the rows keeps them:
    /// decoration rarely changes, and re-fetching it on every poll is what this avoids.
    pub pr_decorations: HashMap<(String, String), PrDecoration>,
    /// Rows with a decoration call already in flight, so a re-render or a flick between views
    /// doesn't queue the same fetch again.
    pr_decor_inflight: HashSet<(String, String)>,
    /// Rows whose decoration call failed. Held until the next pool replacement, because
    /// re-deriving is what *follows* a finished batch: without this, a row that cannot be
    /// decorated — an expired token, or a connection the feed list no longer resolves —
    /// requeues itself the instant its own failure lands, and spins with no await to slow it.
    pr_decor_failed: HashSet<(String, String)>,
    /// PR statuses shown in the list (session-only). Default is Open + Draft; ticking
    /// Merged/Closed also flips the fetch to include completed PRs.
    pub pr_shown_statuses: HashSet<PullRequestStatus>,
    /// Live per-tab quick-filter text (0=PR, 1=WI, 2=Pipelines). Empty = no filter.
    pub filters: [String; 3],
    /// True while the quick-filter input is capturing keystrokes.
    pub filtering: bool,
    /// Work-item state names hidden from the list (provider-specific strings).
    /// Persisted; anything not listed is shown.
    pub wi_hidden_states: HashSet<String>,
    /// Per-view sort (column key + direction); `None` = provider order. Persisted.
    pub pr_sort: Option<SortPref>,
    pub wi_sort: Option<SortPref>,
    pub pipe_sort: Option<SortPref>,
    /// How the Pipelines list is grouped. Persisted.
    pub pipe_group: PipeGroup,
    /// The group keys currently *expanded*. Stored as the open set rather than the closed
    /// one so a newly-arrived group lands collapsed — the roll-up is the point of the view,
    /// and a group that opens itself on refresh would undo it.
    pub pipe_expanded: HashSet<String>,
    /// Saved views per section (0=PR, 1=WI, 2=Pipelines) and the active index.
    pub views: [Vec<SavedView>; 3],
    pub view_idx: [usize; 3],
    /// Which desktop notifications are enabled. Persisted.
    pub notifications: NotificationPrefs,
    /// Hours a review request may wait before its Command Center age turns yellow (red at 3×).
    pub review_sla_hours: u32,
    /// Where things were in the last frame, for mouse clicks (see [`Hit`]).
    pub hits: Vec<(ratatui::layout::Rect, Hit)>,
    /// Where desktop notifications are sent. Real OS notifier by default; tests
    /// swap in a recorder.
    notifier: Arc<dyn Notifier>,
    /// Last-seen status per pipeline run, to detect transitions into failure.
    pipe_seen: HashMap<String, PipelineRunStatus>,
    /// Whether `pipe_seen` has been seeded (skip notifying on the first load).
    pipe_seeded: bool,
    /// Runs currently awaiting the user's approval, keyed by (connection, run id),
    /// so a pending gate is only notified once.
    approval_seen: HashSet<(String, String)>,
    /// Whether `approval_seen` has been seeded (skip notifying on the first load).
    approval_seeded: bool,
    /// Per-PR (approved, changes-requested) flags for my PRs, keyed by
    /// (connection id, PR id) so ids can't collide across providers.
    pr_review_seen: HashMap<(String, String), (bool, bool)>,
    /// PRs where I'm currently a requested reviewer, keyed by (connection, PR id).
    review_req_seen: HashSet<(String, String)>,
    /// Whether the PR-event scan has been seeded (skip notifying on first load).
    pr_scan_seeded: bool,
    /// Transient one-shot message shown in the footer until the next keypress.
    pub toast: Option<String>,
    /// Text waiting to be edited in `$EDITOR`. The event loop owns the terminal, so a handler
    /// can't suspend it itself: it leaves the request here, and the loop hands the terminal
    /// over, then returns the result through [`App::finish_editor`].
    pub editor_request: Option<EditorRequest>,
    /// Pending-approval gates offered by the current approval picker, indexed by
    /// the picker selection. Rebuilt each time the picker opens.
    approval_choices: Vec<ApprovalChoice>,
    /// Open modal overlay, if any. When set, keys route here instead of the table.
    pub overlay: Option<Overlay>,
    /// Add-connection wizard, if running. Takes priority over the overlay/screens.
    pub wizard: Option<Wizard>,
    /// True while the user has sent setup to the browser and no connection has landed yet.
    /// Cleared by the first reload that finds one — the dashboard and the TUI share a
    /// ConfigService, so that reload arrives as soon as the browser saves.
    pub awaiting_browser_setup: bool,
    /// Current screen — the list, or a full-screen sub-view like the PR diff.
    pub screen: Screen,
    /// Launchpad rows (grouped + sorted), rebuilt each refresh.
    pub lp: Vec<launchpad::Entry>,
    /// Which Launchpad reference lists had more than they show (drives the "more…" row).
    pub lp_overflow: launchpad::Overflow,
    /// Focused column (0 = left, 1 = right) and the selected row within each — the
    /// Launchpad is a two-column layout.
    pub lp_side: usize,
    pub lp_sel: [usize; 2],
    /// PRs feeding the Launchpad — the mine + review-requested union (the section list
    /// uses a single filter, so Launchpad fetches its own).
    lp_prs_mine: Vec<PrRow>,
    lp_prs_review: Vec<PrRow>,
    /// Items dismissed from the Launchpad once acted on (e.g. a PR you've reviewed), so
    /// they drop off immediately without waiting for the provider's feed to catch up.
    lp_dismissed: HashSet<String>,
    /// Items the user explicitly dismissed from Command Center via the `D` key.
    /// Persisted to config so they stay hidden across restarts.
    lp_dismissed_persisted: HashSet<String>,
    /// True when the currently-open item view was opened from the Launchpad, so Esc
    /// returns there (with the same row still selected) instead of to the section list.
    lp_origin: bool,
    /// True when the open item view was opened from the notification inbox, so Esc returns
    /// there. Takes precedence over `lp_origin`.
    from_inbox: bool,
    /// True while a background refresh is in flight (drives the header spinner + loading text).
    pub reloading: bool,
    /// Sender for completed background jobs; set once the event loop is running.
    pub job_tx: Option<mpsc::UnboundedSender<AppEvent>>,
    /// Shared animation frame, advanced by a fast timer. Drives the selected-row title
    /// marquee (see `anim / 2` at the call site) and the running-pipeline spinner. Reset
    /// to 0 when the Launchpad selection moves so each title starts from the beginning.
    pub anim: usize,
    /// When the data on screen was fetched, while it is the cache's rather than the network's:
    /// the oldest section seeded at startup. Cleared as soon as a live reload lands.
    pub data_age: Option<DateTime<Utc>>,
    pub last_refresh: DateTime<Local>,
    pub should_quit: bool,
    /// The preview pane beside the section list, while the list is the screen.
    pub preview: Option<Preview>,
    /// Per section, whether the user switched the preview off (`P`). On by default.
    pub preview_hidden: [bool; 3],
    /// True while the preview is focused: its view is `screen`, drawn beside the list.
    pub preview_focus: bool,
    /// Width of the content area in the last frame, which decides whether the split fits.
    pub content_w: u16,
    /// The log fetch in flight — `(conn, run, job)` key and anim ticks waited — so at most
    /// one goes out at a time and a poll never piles on top of a slow one.
    log_inflight: Option<(String, u16)>,
}

/// Full-screen views layered above the list. The large views are boxed so the
/// common `List` state doesn't bloat every `Screen` value.
/// A selectable row in a Launchpad column: a real entry, or a "more…" link for an overflowing
/// bucket that jumps to the full section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LpSlot {
    Entry(usize),
    More(launchpad::Bucket),
}

/// A detail fetch a view needs to stay fresh, built alongside the view and sent separately so
/// the preview pane can hold it back until the cursor settles.
#[derive(Clone)]
pub enum DetailRequest {
    Pr { conn_id: String, item: ItemRef, key: String },
    Wi { conn_id: String, item: ItemRef, key: String },
    Pipeline { conn_id: String, item: ItemRef, key: String },
}

impl DetailRequest {
    /// The detail cache key, which is also the identity of the previewed item.
    pub fn key(&self) -> &str {
        match self {
            DetailRequest::Pr { key, .. } | DetailRequest::Wi { key, .. } | DetailRequest::Pipeline { key, .. } => key,
        }
    }

    /// The request that keeps an already-built item view fresh — how a focused preview hands
    /// itself back to the pane without being rebuilt (and losing its tab, scroll or drafts).
    fn for_view(screen: &Screen) -> Option<DetailRequest> {
        match screen {
            Screen::PrView(v) => {
                let item = v.pr.item_ref();
                let key = pr_detail_cache_key(&v.connection_id, &item);
                Some(DetailRequest::Pr { conn_id: v.connection_id.clone(), item, key })
            }
            Screen::WiView(v) => {
                let item = v.wi.item_ref();
                let key = wi_detail_cache_key(&v.connection_id, &item);
                Some(DetailRequest::Wi { conn_id: v.connection_id.clone(), item, key })
            }
            Screen::Pipeline(v) => {
                let item = v.run.item_ref();
                let key = pipeline_detail_cache_key(&v.connection_id, &item);
                Some(DetailRequest::Pipeline { conn_id: v.connection_id.clone(), item, key })
            }
            _ => None,
        }
    }
}

/// Anim ticks (150ms each) the cursor has to rest on a row before the preview fetches its
/// detail, so holding ↓ through a list doesn't fire four provider calls per row passed.
pub const PREVIEW_SETTLE_TICKS: u8 = 2;

/// The narrowest content area that gets the split. Below it the list keeps the full width —
/// both halves would be too cramped to read.
pub const PREVIEW_MIN_WIDTH: u16 = 140;

/// The unfocused preview beside a section list: the very view `Enter` would open (a
/// [`Screen::PrView`], [`Screen::WiView`] or [`Screen::Pipeline`]), built from the row and the
/// cache. Focusing it (`p`) makes that view the active screen, drawn in the same place, so every
/// action the full view has works there unchanged.
pub struct Preview {
    /// The section the previewed row belongs to (0 PRs, 1 work items, 2 pipelines).
    pub section: usize,
    pub key: String,
    pub view: Screen,
    /// The fetch that keeps `view` fresh.
    request: DetailRequest,
    /// Whether `request` has gone out since the preview was built (or last marked for resend).
    sent: bool,
    /// Anim ticks since the cursor landed on this row.
    ticks: u8,
}

pub enum Screen {
    /// The default landing: a unified, grouped action inbox across every provider.
    Launchpad,
    List,
    Pipeline(Box<PipelineView>),
    Config(Box<ConfigView>),
    /// Full-screen pull-request view with sub-tabs (Conversation/Commits/Checks/Diff).
    PrView(Box<PrView>),
    /// Full-screen work-item view.
    WiView(Box<WiView>),
    /// The cross-provider notification inbox.
    Inbox,
}

/// State for the full-screen PR view.
pub struct PrView {
    pub label: String,
    pub url: Option<String>,
    /// The connection this PR came from — actions resolve their source through it.
    pub connection_id: String,
    pub pr: PullRequest,
    pub tab: usize,
    pub checks: Vec<CheckRun>,
    pub commits: Vec<Commit>,
    /// Cursor row on the Commits tab.
    pub commit_sel: usize,
    /// The whole-PR changed files, cached so the Diff tab can restore them after
    /// drilling into a single commit's diff.
    pub pr_files: Vec<FileChange>,
    /// Scroll offset for the Conversation / Commits / Checks tabs.
    pub scroll: u16,
    /// Diff-tab state (file list + patch + threads), rendered like the standalone diff.
    pub diff: DiffView,
    /// Line comments buffered locally, submitted together as one review (`s`).
    pub pending: Vec<LineComment>,
    /// Target line for a comment being typed (filled in with the body on submit).
    pub review_draft: Option<DraftComment>,
    /// Thread id a reply is being typed against (`r`); the input body is posted to it on submit.
    pub reply_target: Option<String>,
    /// Reviews, merges, state changes, … shown as Activity under the Conversation tab.
    pub timeline: Vec<TimelineEvent>,
}

/// The file line a pending comment is being written against.
pub struct DraftComment {
    pub path: String,
    pub line: i64,
    pub side: DiffSide,
}

impl PrView {
    /// Restores the whole-PR diff if the view was showing a single commit's changes.
    fn reset_diff_scope(&mut self) {
        self.diff.focus = DiffFocus::FileList;
        if self.diff.commit_label.is_some() {
            self.diff.files = self.pr_files.clone();
            self.diff.selected = 0;
            self.diff.cursor = 0;
            self.diff.commit_label = None;
        }
    }
}

/// A pending `$EDITOR` round trip (see [`App::editor_request`]).
#[derive(Debug, Clone)]
pub struct EditorRequest {
    /// What the editor opens with.
    pub initial: String,
    pub field: WiField,
}

/// State for the full-screen work-item view.
pub struct WiView {
    pub connection_id: String,
    pub wi: WorkItem,
    pub threads: Vec<CommentThread>,
    /// State changes, assignments, … shown as the Activity section.
    pub timeline: Vec<TimelineEvent>,
    pub scroll: u16,
}

/// A configured connection, as shown in the config screen.
pub struct ConnRow {
    pub id: String,
    pub display: String,
    pub provider: ProviderType,
    pub healthy: bool,
    /// Which sections this connection is currently bound to.
    pub bindings: Vec<&'static str>,
}

/// Snapshot state for the config / connections screen. Rebuilt after each mutation.
pub struct ConfigView {
    pub connections: Vec<ConnRow>,
    pub pr_binding: Option<String>,
    pub wi_binding: Option<String>,
    pub pipeline_subs: Vec<String>,
    pub selected: usize,
}

impl ConfigView {
    fn selected_conn(&self) -> Option<&ConnRow> {
        self.connections.get(self.selected)
    }
}

/// One row of the flattened, collapsible pipeline tree.
pub struct FlatNode {
    pub depth: usize,
    pub label: String,
    pub status: PipelineRunStatus,
    /// Collapse key when the node has children; `None` for leaf steps.
    pub key: Option<String>,
    pub expanded: bool,
    /// Elapsed time (only for completed nodes), pre-formatted e.g. `3m12s`.
    pub duration: Option<String>,
    /// Short failure summary for failed jobs (provider-specific).
    pub problem: Option<String>,
    /// Deep link to the job (for `o`); steps inherit their job's link.
    pub url: Option<String>,
    /// The job id whose logs this node maps to (for `L`); `None` for stages.
    pub job_id: Option<String>,
}

/// One approve/reject option offered by the pipeline-approval picker.
struct ApprovalChoice {
    connection_id: String,
    /// The run's **connection-relative** repository, so the decision reaches the right one.
    repo: Option<String>,
    run_id: String,
    approval_id: String,
    decision: ApprovalDecision,
    /// Gate label, for confirm/toast messages.
    label: String,
}

/// Most log lines a pane keeps; a longer log keeps its tail, which is where a live job writes
/// and where a failed one usually says why.
pub const LOG_MAX_LINES: usize = 10_000;
/// Anim ticks between polls of a live job's log (the anim timer runs at ~150ms, so ≈4s).
const LOG_POLL_TICKS: u16 = 27;
/// Anim ticks after which an unanswered log fetch is given up on, so a lost answer can't stall
/// every later poll (≈60s).
const LOG_INFLIGHT_TIMEOUT_TICKS: u16 = 400;
/// Below this width the drill-in shows the log pane alone rather than beside the tree.
pub const LOG_SPLIT_MIN_WIDTH: u16 = 90;
/// Width of the stages/jobs/steps tree beside an open log pane.
pub const LOG_TREE_WIDTH: u16 = 38;

/// A scrollable log view over one job, shown beside the pipeline drill-in's tree.
///
/// The provider hands back the whole log on every call, so a poll simply replaces `lines`;
/// what the user is doing (scroll, follow, search) is carried across the replacement.
pub struct LogView {
    /// Pane title, e.g. `Logs · dotnet test` (live state is appended by the renderer).
    pub title: String,
    /// The job these lines belong to — answers for any other job are dropped.
    pub job_id: String,
    pub lines: Vec<String>,
    /// Top visible line while not following.
    pub scroll: u16,
    /// Pinned to the bottom as lines arrive. On by default for a live job.
    pub follow: bool,
    /// The job was still running at the last check, so the pane is polling.
    pub live: bool,
    /// Visible line count of the last frame, written by the renderer so scrolling can clamp.
    pub viewport: Cell<u16>,
    /// The `/` prompt's text while it is open.
    pub search_input: Option<String>,
    /// The committed search, and the lines that match it.
    pub query: Option<String>,
    pub matches: Vec<usize>,
    pub match_idx: Option<usize>,
    /// A one-line status note, e.g. `first error at line 12`.
    pub note: Option<String>,
    /// The last poll failed; the lines on screen are the last good ones.
    pub fetch_failed: bool,
    /// At least one fetch has answered for this job.
    pub loaded: bool,
    /// A fetch is wanted; [`App::pump_logs`] sends it once nothing else is in flight.
    want_fetch: bool,
    poll_ticks: u16,
}

impl LogView {
    /// A pane that has asked for its first fetch. `live` decides follow's default.
    pub fn new(title: String, job_id: String, live: bool) -> Self {
        Self {
            title,
            job_id,
            lines: vec!["Loading logs…".into()],
            scroll: 0,
            follow: live,
            live,
            viewport: Cell::new(0),
            search_input: None,
            query: None,
            matches: Vec::new(),
            match_idx: None,
            note: None,
            fetch_failed: false,
            loaded: false,
            want_fetch: true,
            poll_ticks: 0,
        }
    }

    /// A finished pane already holding `lines` (tests and fixtures).
    #[cfg(test)]
    pub fn with_lines(title: &str, job_id: &str, lines: Vec<String>) -> Self {
        let mut log = Self::new(title.into(), job_id.into(), false);
        log.lines = lines;
        log.loaded = true;
        log.want_fetch = false;
        log
    }

    /// The last line the top of the viewport can sit on. Before the first frame the viewport is
    /// unknown, so a single line is assumed.
    pub fn max_scroll(&self) -> u16 {
        let vp = self.viewport.get().max(1) as usize;
        self.lines.len().saturating_sub(vp).min(u16::MAX as usize) as u16
    }

    /// Top visible line: the bottom while following, else `scroll` clamped.
    pub fn effective_scroll(&self) -> u16 {
        if self.follow {
            self.max_scroll()
        } else {
            self.scroll.min(self.max_scroll())
        }
    }

    /// Scrolls by `delta` lines. Scrolling up breaks follow; scrolling down never re-arms it
    /// (only `G`/End does).
    fn scroll_by(&mut self, delta: i32) {
        let cur = self.effective_scroll() as i32;
        self.scroll = (cur + delta).clamp(0, self.max_scroll() as i32) as u16;
        if delta < 0 {
            self.follow = false;
        }
    }

    fn scroll_top(&mut self) {
        self.scroll = 0;
        self.follow = false;
    }

    fn scroll_bottom(&mut self) {
        self.scroll = self.max_scroll();
        self.follow = true;
    }

    /// Brings `line` into view with a couple of lines of context above it, and stops following.
    fn jump_to(&mut self, line: usize) {
        self.follow = false;
        self.scroll = line.saturating_sub(2).min(self.max_scroll() as usize) as u16;
    }

    /// Replaces the lines with a fresh fetch, keeping the tail past [`LOG_MAX_LINES`], the view
    /// on the same content, and the current search match where it still exists.
    pub fn set_text(&mut self, text: &str) {
        let lines: Vec<String> =
            if text.trim().is_empty() { vec!["(no logs returned)".into()] } else { text.lines().map(str::to_owned).collect() };
        let (lines, dropped) = cap_tail(lines, LOG_MAX_LINES);
        let prev_match = self.match_idx.and_then(|i| self.matches.get(i).copied());
        self.lines = lines;
        if !self.follow {
            self.scroll = self.scroll.saturating_sub(dropped.min(u16::MAX as usize) as u16);
        }
        self.recompute_matches();
        self.match_idx = match prev_match {
            Some(line) if !self.matches.is_empty() => {
                let target = line.saturating_sub(dropped);
                Some(self.matches.iter().position(|&m| m >= target).unwrap_or(self.matches.len() - 1))
            }
            _ => None,
        };
    }

    fn recompute_matches(&mut self) {
        self.matches = match &self.query {
            Some(q) => find_matches(&self.lines, q),
            None => Vec::new(),
        };
        if self.match_idx.is_some_and(|i| i >= self.matches.len()) {
            self.match_idx = self.matches.len().checked_sub(1);
        }
    }

    /// Commits the `/` prompt and jumps to the first match at or below the top of the view.
    fn commit_search(&mut self) {
        let Some(input) = self.search_input.take() else { return };
        self.query = (!input.is_empty()).then_some(input);
        self.match_idx = None;
        self.recompute_matches();
        if self.matches.is_empty() {
            return;
        }
        let top = self.effective_scroll() as usize;
        let idx = self.matches.iter().position(|&m| m >= top).unwrap_or(0);
        self.match_idx = Some(idx);
        self.jump_to(self.matches[idx]);
    }

    /// `n` / `N`: the next / previous match, wrapping around.
    fn step_match(&mut self, forward: bool) {
        let Some(idx) = next_match_index(self.matches.len(), self.match_idx, forward) else { return };
        self.match_idx = Some(idx);
        self.jump_to(self.matches[idx]);
    }

    /// `E`: scrolls to the first error line and says where it was.
    fn jump_first_error(&mut self) {
        match first_error_line(&self.lines) {
            Some(i) => {
                self.jump_to(i);
                self.note = Some(format!("first error at line {}", i + 1));
            }
            None => {
                self.follow = false;
                self.note = Some("no errors found".into());
            }
        }
    }

    /// One anim tick of the poll schedule; true when a fetch should go out. A live job polls
    /// every [`LOG_POLL_TICKS`]; the tick it is first seen finished asks once more, so the tail
    /// is complete, and after that nothing is polled.
    pub fn poll_step(&mut self, active: bool) -> bool {
        if !self.loaded {
            return false; // the first fetch is still on its way
        }
        if active {
            self.live = true;
            self.poll_ticks = self.poll_ticks.saturating_add(1);
            if self.poll_ticks >= LOG_POLL_TICKS {
                self.poll_ticks = 0;
                return true;
            }
            false
        } else if self.live {
            self.live = false;
            true
        } else {
            false
        }
    }
}

/// Keeps the last `max` lines; returns them and how many were dropped from the front.
pub fn cap_tail(mut lines: Vec<String>, max: usize) -> (Vec<String>, usize) {
    let dropped = lines.len().saturating_sub(max);
    if dropped > 0 {
        lines.drain(..dropped);
    }
    (lines, dropped)
}

/// Whether a log line reports a failure: CI annotations (`##[error]`), test-runner verdicts
/// (`[FAIL]`, `FAILED`), compiler diagnostics (`error:`, `error[E…]`, `ERROR`, `Error:`),
/// `npm ERR!`, and a non-zero exit code. Summary lines that count zero failures (`0 errors`,
/// `Failed: 0`, `exit code 0`) don't qualify.
pub fn is_error_line(line: &str) -> bool {
    if line.contains("##[error]") || line.contains("[FAIL]") || line.contains("npm ERR!") {
        return true;
    }
    if has_token(line, "FAILED", |_| true) || has_token(line, "FAIL", |_| true) || has_token(line, "ERROR", |_| true) {
        return true;
    }
    // Title-case verdicts (`Failed!`, `Tests Failed: 3`) — but not prose like `Failed to retry`.
    if has_token(line, "Failed", |next| matches!(next, Some('!' | ':'))) {
        return true;
    }
    if has_token(line, "error", |next| matches!(next, Some(':' | '['))) || has_token(line, "Error", |next| next == Some(':')) {
        return true;
    }
    nonzero_exit_code(line)
}

/// `word` as a whole token (not inside a longer identifier) whose following character passes
/// `next_ok`, and which isn't a zero count (`0 FAILED`, `ERROR: 0`).
fn has_token(line: &str, word: &str, next_ok: impl Fn(Option<char>) -> bool) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(word).any(|(i, _)| {
        let before = line[..i].chars().next_back();
        let rest = &line[i + word.len()..];
        let after = rest.chars().next();
        !before.is_some_and(is_word)
            && !after.is_some_and(is_word)
            && next_ok(after)
            && !ends_with_zero(&line[..i])
            && !starts_with_zero(rest)
    })
}

/// `text` ends in the number 0 (ignoring trailing spaces), e.g. the `0 ` of `0 FAILED`.
fn ends_with_zero(text: &str) -> bool {
    let t = text.trim_end();
    t.ends_with('0') && !t[..t.len() - 1].ends_with(|c: char| c.is_ascii_digit())
}

/// `text` is a count of 0 after optional `:`/`=`/spaces, e.g. the `: 0` of `ERROR: 0`.
fn starts_with_zero(text: &str) -> bool {
    let t = text.trim_start_matches([':', '=', ' ']);
    t.starts_with('0') && !t[1..].starts_with(|c: char| c.is_ascii_digit())
}

/// `exit code N` / `exited with code N` (any case) with N other than 0.
fn nonzero_exit_code(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    ["exit code", "exited with code"].iter().any(|pat| {
        lower.match_indices(pat).any(|(i, _)| {
            let rest = lower[i + pat.len()..].trim_start_matches([':', ' ']);
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            !digits.is_empty() && !digits.trim_start_matches('0').is_empty()
        })
    })
}

/// Index of the first line [`is_error_line`] matches.
pub fn first_error_line(lines: &[String]) -> Option<usize> {
    lines.iter().position(|l| is_error_line(l))
}

/// Lines containing `query`, compared ASCII-case-insensitively (so byte offsets line up with
/// the original text for highlighting).
pub fn find_matches(lines: &[String], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let q = query.to_ascii_lowercase();
    lines.iter().enumerate().filter(|(_, l)| l.to_ascii_lowercase().contains(&q)).map(|(i, _)| i).collect()
}

/// Byte ranges of every occurrence of `query` in `line`, ASCII-case-insensitively.
pub fn match_ranges(line: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let (hay, q) = (line.to_ascii_lowercase(), query.to_ascii_lowercase());
    hay.match_indices(&q).map(|(i, m)| (i, i + m.len())).collect()
}

/// The match after (or before) `cur`, wrapping around; the first (or last) when none is current.
pub fn next_match_index(len: usize, cur: Option<usize>, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match (cur, forward) {
        (None, true) => 0,
        (None, false) => len - 1,
        (Some(i), true) => (i + 1) % len,
        (Some(i), false) => (i + len - 1) % len,
    })
}

/// The first failed node of a run as `(stage, job, step)`: a failed step when a job has one
/// (its log names the failure most precisely), else a failed job.
fn first_failed_node(run: &PipelineRun) -> Option<(usize, usize, Option<usize>)> {
    for (si, stage) in run.stages.iter().enumerate() {
        for (ji, job) in stage.jobs.iter().enumerate() {
            if let Some(k) = job.steps.iter().position(|s| s.status == PipelineRunStatus::Failed) {
                return Some((si, ji, Some(k)));
            }
            if job.status == PipelineRunStatus::Failed {
                return Some((si, ji, None));
            }
        }
    }
    None
}

/// Formats the elapsed time between two instants (only when both are known).
fn fmt_duration(start: Option<DateTime<Utc>>, finish: Option<DateTime<Utc>>) -> Option<String> {
    let (s, f) = (start?, finish?);
    let secs = (f - s).num_seconds().max(0);
    Some(if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    })
}

/// The elapsed span of a whole stage, or `None` if any job is still unfinished.
fn stage_duration(jobs: &[PipelineJob]) -> Option<String> {
    let start = jobs.iter().filter_map(|j| j.started_at).min();
    let finish = if jobs.iter().all(|j| j.finished_at.is_some()) {
        jobs.iter().filter_map(|j| j.finished_at).max()
    } else {
        None
    };
    fmt_duration(start, finish)
}

/// State for the full-screen pipeline drill-in (stages → jobs → steps).
pub struct PipelineView {
    pub title: String,
    pub run: PipelineRun,
    pub connection_id: String,
    pub provider: ProviderType,
    pub definition_id: String,
    pub branch: Option<String>,
    collapsed: HashSet<String>,
    pub selected: usize,
    /// Open log pane over a selected job, if any.
    pub logs: Option<LogView>,
    /// Whether keys drive the log pane (true) or the tree beside it, while logs are open.
    pub log_focus: bool,
    /// Whether the last frame had room to draw the tree beside the logs. Written by the
    /// renderer; when false the logs fill the pane and always have the keys.
    pub log_split: Cell<bool>,
    /// The user has moved the tree cursor, so no automatic selection may override it.
    user_moved: bool,
    /// The first-load jump to a failed node has been decided (made, or found unnecessary).
    auto_checked: bool,
    /// The first load selected a failed node whose logs should open once this view is the
    /// screen (never while it only sits in the preview).
    pub auto_logs: bool,
    /// Whether this run's provider can surface pending approvals.
    pub supports_approvals: bool,
    /// Whether the app can actually submit an approve/reject here (false = view-only,
    /// e.g. Azure — we show the gate but can't act on it).
    pub can_respond_approvals: bool,
    /// Gates on this run currently awaiting a decision.
    pub approvals: Vec<PipelineApproval>,
    /// True when `run` is a cache-seeded guess, not yet confirmed by a live `get_run`. A run's
    /// status is live in a way a PR's files/checks aren't — a cached "Running" may have failed
    /// an hour ago — so the renderer is expected to mark it as unconfirmed until this clears.
    /// Set on open (see `open_pipeline_for`) and cleared only in `apply_pipeline_detail`, once a
    /// real fetch has actually answered.
    pub stale: bool,
}

impl PipelineView {
    pub fn new(title: String, run: PipelineRun, connection_id: String, provider: ProviderType, definition_id: String, branch: Option<String>) -> Self {
        Self {
            title,
            run,
            connection_id,
            provider,
            definition_id,
            branch,
            collapsed: HashSet::new(),
            selected: 0,
            logs: None,
            log_focus: true,
            log_split: Cell::new(true),
            user_moved: false,
            auto_checked: false,
            auto_logs: false,
            supports_approvals: false,
            can_respond_approvals: false,
            approvals: Vec::new(),
            stale: false,
        }
    }

    /// Patches in a freshly-fetched run, preserving what the user is doing: the open log pane
    /// and the expand/collapse tree state are untouched (this never rebuilds the view), and the
    /// selection is clamped rather than reset, since the new run's flattened node count may be
    /// shorter. `collapsed` is private, so this is how a caller outside the impl block patches
    /// it without reaching in. Called only once a real `get_run` has confirmed the run, which is
    /// also the moment `stale` clears.
    ///
    /// `approvals` is `None` when this fetch's `pending_approvals` call failed, and then the
    /// gates already on screen are left exactly as they are. A gate here drives a real
    /// approve/reject (see [`PipelineView::actionable_approvals`]), so only a call that actually
    /// answered may set them — never a value read back out of the cache, which may name a gate
    /// that was decided an hour ago.
    fn apply_fresh_run(&mut self, run: PipelineRun, approvals: Option<Vec<PipelineApproval>>) {
        self.run = run;
        self.apply_confirmed_approvals(approvals);
        self.stale = false;
        self.clamp_selection();
        self.auto_select_failed();
    }

    /// First load of a failed run: selects its first failed step (or, lacking one, its first
    /// failed job), expanding the parents, and asks for its logs to open. Runs until the first
    /// load has decided — a cache-seeded run with no stages yet waits for the live one — and
    /// never once the user has moved the cursor.
    fn auto_select_failed(&mut self) {
        if self.auto_checked || self.user_moved {
            return;
        }
        if self.run.status == PipelineRunStatus::Failed {
            if let Some((si, ji, step)) = first_failed_node(&self.run) {
                self.collapsed.remove(&format!("s{si}"));
                if step.is_some() {
                    self.collapsed.remove(&format!("s{si}.j{ji}"));
                }
                self.selected = self.node_index(si, ji, step);
                self.auto_logs = true;
                self.auto_checked = true;
                return;
            }
        }
        if !self.stale {
            self.auto_checked = true;
        }
    }

    /// Row of a job (or one of its steps) in [`PipelineView::flatten`]'s order, assuming its
    /// stage (and, for a step, the job) is expanded.
    fn node_index(&self, si: usize, ji: usize, step: Option<usize>) -> usize {
        let mut idx = 0;
        for (s, stage) in self.run.stages.iter().enumerate() {
            idx += 1; // the stage row
            if s < si && self.collapsed.contains(&format!("s{s}")) {
                continue;
            }
            for (j, job) in stage.jobs.iter().enumerate() {
                if s == si && j == ji {
                    return idx + step.map_or(0, |k| k + 1);
                }
                idx += 1;
                if !self.collapsed.contains(&format!("s{s}.j{j}")) {
                    idx += job.steps.len();
                }
            }
        }
        idx
    }

    /// Whether the job the log pane follows is still running — the run's own status when the
    /// job isn't in the tree (yet).
    pub fn log_target_active(&self) -> bool {
        let Some(log) = &self.logs else { return false };
        let job = self.run.stages.iter().flat_map(|s| &s.jobs).find(|j| j.id == log.job_id);
        is_active(job.map_or(self.run.status, |j| j.status))
    }

    /// Opens (or re-targets) the log pane on the selected node's job. Steps share their job's
    /// log, so moving between a job and its steps keeps the pane as it is. Returns false when
    /// the node has no job (a stage).
    fn open_logs_for_selection(&mut self) -> bool {
        let nodes = self.flatten();
        let Some(node) = nodes.get(self.selected) else { return false };
        let Some(job_id) = node.job_id.clone() else { return false };
        if self.logs.as_ref().is_some_and(|l| l.job_id == job_id) {
            return true;
        }
        let job = self.run.stages.iter().flat_map(|s| &s.jobs).find(|j| j.id == job_id);
        let label = job.map_or_else(|| node.label.clone(), |j| j.name.clone());
        let live = is_active(job.map_or(self.run.status, |j| j.status));
        // A search carries over to the next job: it is usually the same question.
        let query = self.logs.as_mut().and_then(|l| l.query.take());
        let mut log = LogView::new(format!("Logs · {label}"), job_id, live);
        log.query = query;
        self.logs = Some(log);
        true
    }

    /// Whether keys go to the log pane: it is open, and either focused or alone on screen.
    pub fn logs_have_keys(&self) -> bool {
        self.logs.is_some() && (self.log_focus || !self.log_split.get())
    }

    /// Applies gates that a `pending_approvals` call confirmed. `Some(vec![])` is authoritative
    /// — the run has no gates now, so any on screen are cleared — while `None` means the call
    /// failed and the view keeps what it had.
    fn apply_confirmed_approvals(&mut self, approvals: Option<Vec<PipelineApproval>>) {
        if let Some(approvals) = approvals {
            self.approvals = approvals;
        }
    }

    /// Pending gates the authenticated user is allowed to act on.
    pub fn actionable_approvals(&self) -> Vec<&PipelineApproval> {
        self.approvals.iter().filter(|a| a.can_respond).collect()
    }

    /// Flattens stages/jobs/steps into visible rows, honouring collapsed nodes.
    pub fn flatten(&self) -> Vec<FlatNode> {
        let mut out = Vec::new();
        for (si, stage) in self.run.stages.iter().enumerate() {
            let key = format!("s{si}");
            let expanded = !self.collapsed.contains(&key);
            out.push(FlatNode {
                depth: 0,
                label: stage.name.clone(),
                status: stage.status,
                key: (!stage.jobs.is_empty()).then(|| key.clone()),
                expanded,
                duration: stage_duration(&stage.jobs),
                problem: None,
                url: None,
                job_id: None,
            });
            if !expanded {
                continue;
            }
            for (ji, job) in stage.jobs.iter().enumerate() {
                let jkey = format!("s{si}.j{ji}");
                let jexpanded = !self.collapsed.contains(&jkey);
                out.push(FlatNode {
                    depth: 1,
                    label: job.name.clone(),
                    status: job.status,
                    key: (!job.steps.is_empty()).then(|| jkey.clone()),
                    expanded: jexpanded,
                    duration: fmt_duration(job.started_at, job.finished_at),
                    problem: job.problem.clone(),
                    url: job.url.clone(),
                    job_id: Some(job.id.clone()),
                });
                if jexpanded {
                    for step in &job.steps {
                        out.push(FlatNode {
                            depth: 2,
                            label: step.name.clone(),
                            status: step.status,
                            key: None,
                            expanded: false,
                            duration: fmt_duration(step.started_at, step.finished_at),
                            problem: None,
                            url: job.url.clone(),
                            job_id: Some(job.id.clone()),
                        });
                    }
                }
            }
        }
        out
    }

    fn move_sel(&mut self, delta: isize) {
        let len = self.flatten().len();
        if len == 0 {
            return;
        }
        let n = len as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
        self.user_moved = true;
    }

    /// Keeps the cursor in range after the tree is refreshed.
    fn clamp_selection(&mut self) {
        let len = self.flatten().len();
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    /// Expands/collapses the node under the cursor (no-op on leaf steps).
    fn toggle_selected(&mut self) {
        if let Some(Some(key)) = self.flatten().get(self.selected).map(|n| n.key.clone()) {
            if !self.collapsed.remove(&key) {
                self.collapsed.insert(key);
            }
            let len = self.flatten().len();
            if self.selected >= len {
                self.selected = len.saturating_sub(1);
            }
        }
    }
}

/// Where key input lands inside the diff tab: the file list, or a line cursor
/// inside the selected file's patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffFocus {
    #[default]
    FileList,
    Patch,
}

/// State for the full-screen PR diff + threads view.
pub struct DiffView {
    pub pr_label: String,
    pub url: Option<String>,
    pub files: Vec<FileChange>,
    pub threads: Vec<CommentThread>,
    pub selected: usize,
    pub scroll: u16,
    /// Whether keys drive the file list or a line cursor in the patch.
    pub focus: DiffFocus,
    /// Cursor line index into the current file's patch (used in `Patch` focus).
    pub cursor: usize,
    /// When set, the diff shows a single commit's changes (label shown in the
    /// file-list title); `None` means the whole-PR diff.
    pub commit_label: Option<String>,
    /// Paths the reviewer has marked "viewed" this session (per open PR, not persisted).
    pub viewed: HashSet<String>,
}

impl DiffView {
    pub fn current(&self) -> Option<&FileChange> {
        self.files.get(self.selected)
    }

    /// The comment thread anchored at the current patch cursor line, if any — the target for `r`.
    pub fn thread_at_cursor(&self) -> Option<&CommentThread> {
        let file = self.current()?;
        let patch = file.patch.as_deref()?;
        let path = file.path.as_str();
        self.threads.iter().find(|t| {
            t.file_path.as_deref() == Some(path)
                && t.line.and_then(|l| crate::diff::patch_line_for_source_line(patch, l)) == Some(self.cursor)
        })
    }

    /// Number of lines in the current file's patch (0 if none).
    fn patch_len(&self) -> usize {
        self.current().and_then(|f| f.patch.as_deref()).map(|p| p.lines().count()).unwrap_or(0)
    }

    fn select_file(&mut self, delta: isize) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
        self.scroll = 0;
        self.cursor = 0;
    }

    /// Enters the patch line cursor for the current file (no-op without a patch).
    fn enter_patch(&mut self) {
        if self.patch_len() > 0 {
            self.focus = DiffFocus::Patch;
            self.cursor = 0;
        }
    }

    /// Returns focus to the file list.
    fn exit_patch(&mut self) {
        self.focus = DiffFocus::FileList;
    }

    /// Moves the patch line cursor, clamped to the patch bounds.
    fn move_cursor(&mut self, delta: isize) {
        let n = self.patch_len();
        if n == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, n as isize - 1) as usize;
    }

    fn scroll_by(&mut self, delta: i32) {
        self.scroll = (self.scroll as i32 + delta).max(0) as u16;
    }

    /// Toggle the "viewed" mark on the current file.
    fn toggle_viewed(&mut self) {
        if let Some(path) = self.current().map(|f| f.path.clone()) {
            if !self.viewed.remove(&path) {
                self.viewed.insert(path);
            }
        }
    }

    pub fn is_viewed(&self, path: &str) -> bool {
        self.viewed.contains(path)
    }

    /// How many of the currently-listed files are marked viewed (for "N/M reviewed").
    pub fn viewed_count(&self) -> usize {
        self.files.iter().filter(|f| self.viewed.contains(&f.path)).count()
    }

    /// Move the patch cursor to the next (`dir > 0`) or previous thread in the current
    /// file, entering the patch cursor. Wraps. No-op if the file has no located threads.
    fn jump_thread(&mut self, dir: isize) {
        let mut targets: Vec<usize> = {
            let Some(file) = self.current() else { return };
            let Some(patch) = file.patch.as_deref() else { return };
            let path = file.path.as_str();
            self.threads
                .iter()
                .filter(|t| t.file_path.as_deref() == Some(path))
                .filter_map(|t| t.line.and_then(|l| crate::diff::patch_line_for_source_line(patch, l)))
                .collect()
        };
        targets.sort_unstable();
        targets.dedup();
        if targets.is_empty() {
            return;
        }
        self.focus = DiffFocus::Patch;
        let cur = self.cursor;
        self.cursor = if dir > 0 {
            targets.iter().find(|&&t| t > cur).copied().unwrap_or(targets[0])
        } else {
            targets.iter().rev().find(|&&t| t < cur).copied().unwrap_or(*targets.last().unwrap())
        };
    }
}

impl App {
    pub fn new(theme_name: &str) -> Self {
        Self {
            theme: Theme::by_name(theme_name),
            active: 0,
            prs: Vec::new(),
            lp_prs_mine: Vec::new(),
            lp_prs_review: Vec::new(),
            wis: Vec::new(),
            pipes: Vec::new(),
            inbox: Vec::new(),
            inbox_sel: 0,
            dashboard_url: None,
            feedback_opener: system_feedback_opener,
            pr_state: TableState::default(),
            wi_state: TableState::default(),
            pipe_state: TableState::default(),
            repo_scope: [None, None, None],
            repo_catalog: HashMap::new(),
            repo_scope_choices: Vec::new(),
            health: Vec::new(),
            visible: [true; 3],
            status: "Loading…".into(),
            loading: true,
            list_scroll: 0,
            content_h: 0,
            detail_scroll_max: 0,
            pr_filter: PullRequestFilter::All,
            pr_pool: PrPool::default(),
            pr_pool_loaded: false,
            pr_decorations: HashMap::new(),
            pr_decor_inflight: HashSet::new(),
            pr_decor_failed: HashSet::new(),
            pr_shown_statuses: [PullRequestStatus::Open, PullRequestStatus::Draft].into_iter().collect(),
            filters: [String::new(), String::new(), String::new()],
            filtering: false,
            wi_hidden_states: HashSet::new(),
            pr_sort: None,
            wi_sort: None,
            pipe_sort: None,
            pipe_group: PipeGroup::default(),
            pipe_expanded: HashSet::new(),
            views: [Vec::new(), Vec::new(), Vec::new()],
            view_idx: [0, 0, 0],
            notifications: NotificationPrefs::default(),
            review_sla_hours: forgetop_core::config::DEFAULT_REVIEW_SLA_HOURS,
            hits: Vec::new(),
            notifier: Arc::new(SystemNotifier),
            pipe_seen: HashMap::new(),
            approval_seen: HashSet::new(),
            approval_seeded: false,
            pipe_seeded: false,
            pr_review_seen: HashMap::new(),
            review_req_seen: HashSet::new(),
            pr_scan_seeded: false,
            toast: None,
            editor_request: None,
            approval_choices: Vec::new(),
            overlay: None,
            wizard: None,
            awaiting_browser_setup: false,
            screen: Screen::Launchpad,
            lp: Vec::new(),
            lp_overflow: launchpad::Overflow::default(),
            lp_side: 0,
            lp_sel: [0, 0],
            lp_dismissed: HashSet::new(),
            lp_dismissed_persisted: HashSet::new(),
            lp_origin: false,
            from_inbox: false,
            reloading: false,
            job_tx: None,
            anim: 0,
            data_age: None,
            last_refresh: Local::now(),
            should_quit: false,
            preview: None,
            preview_hidden: [false; 3],
            preview_focus: false,
            content_w: 0,
            log_inflight: None,
        }
    }

    /// Human label for the current PR filter (shown in the section title / footer).
    pub fn pr_filter_label(&self) -> &'static str {
        match self.pr_filter {
            PullRequestFilter::All => "all",
            PullRequestFilter::Mine => "mine",
            PullRequestFilter::ReviewRequested => "review-requested",
        }
    }

    // ---- quick filter ----

    /// Row indices of the PR list matching its quick filter (all rows if empty),
    /// in the active sort order.
    pub fn filtered_pr_indices(&self) -> Vec<usize> {
        let q = self.filters[0].to_lowercase();
        let mut idx: Vec<usize> = (0..self.prs.len())
            .filter(|&i| self.pr_shown_statuses.contains(&self.prs[i].pr.status) && pr_matches(&self.prs[i].pr, &q))
            .collect();
        if let Some(s) = &self.pr_sort {
            idx.sort_by(|&a, &b| ordered(pr_cmp(&self.prs[a].pr, &self.prs[b].pr, &s.key), s.desc));
        }
        idx
    }

    pub fn filtered_wi_indices(&self) -> Vec<usize> {
        let q = self.filters[1].to_lowercase();
        let mut idx: Vec<usize> = (0..self.wis.len())
            .filter(|&i| !self.wi_hidden_states.contains(&self.wis[i].wi.state) && wi_matches(&self.wis[i].wi, &q))
            .collect();
        if let Some(s) = &self.wi_sort {
            idx.sort_by(|&a, &b| ordered(wi_cmp(&self.wis[a].wi, &self.wis[b].wi, &s.key), s.desc));
        }
        idx
    }

    /// How many distinct states currently in view are hidden (for the title/toast;
    /// ignores stale hidden states left over from another provider).
    pub fn hidden_states_in_view(&self) -> usize {
        self.distinct_wi_states().iter().filter(|s| self.wi_hidden_states.contains(*s)).count()
    }

    /// Distinct work-item states in first-seen order (drives the visibility checklist).
    fn distinct_wi_states(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for w in &self.wis {
            if seen.insert(w.wi.state.clone()) {
                out.push(w.wi.state.clone());
            }
        }
        out
    }

    pub fn filtered_pipe_indices(&self) -> Vec<usize> {
        let q = self.filters[2].to_lowercase();
        let mut idx: Vec<usize> = (0..self.pipes.len()).filter(|&i| pipe_matches(&self.pipes[i], &q)).collect();
        if let Some(s) = &self.pipe_sort {
            idx.sort_by(|&a, &b| ordered(pipe_cmp(&self.pipes[a], &self.pipes[b], &s.key), s.desc));
        }
        idx
    }

    /// The group key a run falls under, for the active mode. Connection-qualified so two
    /// forges with a same-named pipeline never merge, and repo-qualified so `main` in one
    /// repository is not `main` in another.
    fn pipe_group_key(&self, p: &PipeRow) -> String {
        let repo = p.run.repository.clone().unwrap_or_default();
        let branch = p.run.branch.clone().unwrap_or_default();
        // \u{1} cannot occur in a name, so the parts can't collide across the separator.
        match self.pipe_group {
            // Keyed on `definition_id`, never on the displayed name: the name falls back to
            // the run's own name, which is per-run on Azure (a build number), and two distinct
            // workflows are allowed to share a display name on GitHub. Either way the name is
            // the wrong identity — it would split one pipeline apart or merge two together.
            PipeGroup::Pipeline => format!("{}\u{1}{}\u{1}{}", p.connection_id, repo, p.run.definition_id),
            PipeGroup::Trigger => {
                format!("{}\u{1}{}\u{1}{}\u{1}{}", p.connection_id, repo, branch, pipe_trigger_token(p))
            }
            PipeGroup::Branch => format!("{}\u{1}{}\u{1}{}", p.connection_id, repo, branch),
            PipeGroup::Off => String::new(),
        }
    }

    /// What names a group, for the active mode — the value in the subject column.
    ///
    /// A run beneath the header fills that same column with whatever *varies* inside the group
    /// (see [`App::pipe_child_subject`]), so the column reads top-to-bottom as "the group, then
    /// what tells its runs apart", and no row leaves it blank. Blanking it instead put the tree
    /// marker at the far left with the run's first real value forty characters away.
    fn pipe_group_subject(&self, members: &[usize]) -> String {
        let first = &self.pipes[members[0]];
        match self.pipe_group {
            // Never `pipe_definition_name` here: its last fallback is the run's own name,
            // which on Azure is a build number. Titling a group from one arbitrary member
            // would then relabel it whenever a sort changed which member came first. The
            // group is keyed on `definition_id`, so that is the stable name to fall back to,
            // and `min` keeps the choice independent of order.
            PipeGroup::Pipeline | PipeGroup::Off => members
                .iter()
                .filter_map(|&i| self.pipes[i].definition_name.clone())
                .min()
                .unwrap_or_else(|| first.run.definition_id.clone()),
            PipeGroup::Trigger | PipeGroup::Branch => pipe_branch_label(first),
        }
    }

    /// What a run puts in the subject column when it sits under a header: the thing that varies
    /// within the group. Grouped by pipeline that is its branch; grouped by branch or trigger it
    /// is its pipeline.
    pub fn pipe_child_subject(&self, p: &PipeRow) -> String {
        match self.pipe_group {
            PipeGroup::Pipeline => pipe_branch_label(p),
            _ => pipe_definition_name(p),
        }
    }

    /// The heading for the subject column. Once a group is open that column carries both kinds
    /// of value, so the heading says so rather than naming only half of what sits under it.
    pub fn pipe_subject_heading(&self, any_open: bool) -> &'static str {
        match (self.pipe_group, any_open) {
            (PipeGroup::Off, _) | (PipeGroup::Pipeline, false) => "Pipeline",
            (PipeGroup::Pipeline, true) => "Pipeline / Branch",
            (_, false) => "Branch",
            (_, true) => "Branch / Pipeline",
        }
    }

    /// The Pipelines list as rendered lines: group headers plus the runs of expanded groups.
    ///
    /// Groups are ordered by their most recent run, newest first — the same "what changed
    /// last" ordering the flat list has, lifted to the group. Runs *within* a group keep the
    /// order [`App::filtered_pipe_indices`] produced, so an explicit sort still applies; it
    /// just applies inside each group rather than across the whole list.
    pub fn pipe_lines(&self) -> Vec<PipeLine> {
        let idxs = self.filtered_pipe_indices();
        if self.pipe_group == PipeGroup::Off {
            return idxs.into_iter().map(PipeLine::Run).collect();
        }

        let mut order: Vec<String> = Vec::new();
        let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
        for i in idxs {
            let key = self.pipe_group_key(&self.pipes[i]);
            if !buckets.contains_key(&key) {
                order.push(key.clone());
            }
            buckets.entry(key).or_default().push(i);
        }

        let newest = |k: &str| -> Option<DateTime<Utc>> {
            buckets[k].iter().filter_map(|&i| self.pipes[i].run.started_at).max()
        };
        // A sort orders the **groups**, not the runs inside them. Grouped lists land
        // collapsed, so only headers are on screen — a sort applied inside a group would move
        // nothing the user can see. Each group is compared by the run its header displays (its
        // most recent), so the order always matches the cells being sorted on.
        let reps: HashMap<&str, usize> = buckets
            .iter()
            .map(|(k, members)| {
                let rep = members.iter().copied().max_by_key(|&i| self.pipes[i].run.started_at).unwrap_or(members[0]);
                (k.as_str(), rep)
            })
            .collect();
        match &self.pipe_sort {
            Some(sort) => order.sort_by(|a, b| {
                ordered(pipe_cmp(&self.pipes[reps[a.as_str()]], &self.pipes[reps[b.as_str()]], &sort.key), sort.desc)
            }),
            // Newest first. `None` (no start time) is the smallest, so reversing puts it last,
            // and the sort is stable, so ties keep the order the filter already produced.
            None => order.sort_by_key(|k| Reverse(newest(k))),
        }

        let mut out = Vec::new();
        for key in order {
            let members = &buckets[&key];
            let first = &self.pipes[members[0]];
            // Status and Started both come from this one run, so the pair reads as "where
            // this stands now" rather than pairing the worst outcome with the newest time.
            let latest_idx = members.iter().copied().max_by_key(|&i| self.pipes[i].run.started_at).unwrap_or(members[0]);
            let latest = &self.pipes[latest_idx];
            let head = PipeHead {
                latest: latest_idx,
                expanded: self.pipe_expanded.contains(&key),
                subject: self.pipe_group_subject(members),
                repo: first.run.repository.clone().unwrap_or_default(),
                // The trigger key falls back to the start minute when a provider gives no
                // commit, so the cell has to as well, or two separate pushes to one branch
                // render as two identical headers. `~` marks it as a time, not a sha.
                commit: match self.pipe_group {
                    PipeGroup::Trigger => match first.run.commit_sha.as_deref().filter(|c| !c.is_empty()) {
                        Some(sha) => sha.chars().take(7).collect(),
                        // The key falls back the same way — start minute, then run id — so the
                        // cell has to follow it all the way down, or two distinct triggers
                        // render as two identical rows.
                        None => match first.run.started_at {
                            Some(t) => format!("~{}", t.with_timezone(&Local).format("%H:%M")),
                            None => format!("#{}", first.run.id.chars().take(7).collect::<String>()),
                        },
                    },
                    _ => String::new(),
                },
                runs: members.len(),
                failed: members.iter().filter(|&&i| self.pipes[i].run.status == PipelineRunStatus::Failed).count(),
                status: latest.run.status,
                started: latest.run.started_at,
                approval: members.iter().any(|&i| self.pipes[i].awaiting_approval),
                provider: first.provider,
                connection: first.connection.clone(),
                key,
            };
            let expanded = head.expanded;
            out.push(PipeLine::Head(head));
            if expanded {
                out.extend(members.iter().map(|&i| PipeLine::Run(i)));
            }
        }
        out
    }

    /// Drops expand state for groups that no longer exist.
    ///
    /// Without this a group that disappears and comes back — a repository leaving and
    /// re-entering scope, a provider erroring once — returns *expanded*, because its key was
    /// still in the set. Groups are meant to arrive collapsed however they arrive.
    fn prune_pipe_expanded(&mut self) {
        if self.pipe_expanded.is_empty() {
            return;
        }
        let live: HashSet<String> = self.pipes.iter().map(|p| self.pipe_group_key(p)).collect();
        self.pipe_expanded.retain(|k| live.contains(k));
    }

    /// Expands or collapses one group, keeping the cursor on its header.
    fn toggle_pipe_group(&mut self, key: &str) {
        if !self.pipe_expanded.remove(key) {
            self.pipe_expanded.insert(key.to_string());
        }
        self.fix_selection();
        self.ensure_visible();
    }

    /// Expands or collapses every group at once.
    fn set_all_pipe_groups(&mut self, expand: bool) {
        if self.pipe_group == PipeGroup::Off {
            return;
        }
        self.pipe_expanded.clear();
        if expand {
            for line in self.pipe_lines() {
                if let PipeLine::Head(h) = line {
                    self.pipe_expanded.insert(h.key);
                }
            }
        }
        self.fix_selection();
        self.ensure_visible();
    }

    /// Cycles the grouping mode and persists the choice. Resets the cursor to the top: the
    /// line the cursor was on does not exist in the new arrangement.
    async fn cycle_pipe_group(&mut self, deps: &AppDeps) {
        self.pipe_group = self.pipe_group.next();
        self.pipe_expanded.clear();
        self.pipe_state.select(Some(0));
        self.list_scroll = 0;
        self.fix_selection();
        self.toast = Some(match self.pipe_group {
            PipeGroup::Off => "Grouping off".to_string(),
            g => format!("Grouped by {}", g.as_str()),
        });
        let _ = deps.config.set_pipeline_group(Some(self.pipe_group.as_str().to_string())).await;
    }

    fn filtered_len(&self, section: usize) -> usize {
        match section {
            0 => self.filtered_pr_indices().len(),
            1 => self.filtered_wi_indices().len(),
            _ => self.pipe_lines().len(),
        }
    }

    /// The active tab's quick-filter text.
    pub fn active_filter(&self) -> &str {
        &self.filters[self.active]
    }

    /// Opens the quick-filter input for the active tab.
    fn start_filter(&mut self) {
        self.filtering = true;
    }

    /// Re-anchors selection to the first match after the filter changes.
    fn reset_filter_selection(&mut self) {
        self.list_scroll = 0;
        let len = self.active_len();
        self.active_state().select((len > 0).then_some(0));
    }

    /// Esc / `q` on a section list: clear an active quick filter first, otherwise step back to
    /// the Command Center. Neither key quits — leaving the app is Ctrl-C, so a stray keypress
    /// can't tear down a session.
    fn leave_section_list(&mut self) {
        if self.filters[self.active].is_empty() {
            self.screen = Screen::Launchpad;
        } else {
            self.filters[self.active].clear();
            self.reset_filter_selection();
        }
    }

    /// Handles keys while the quick-filter input is open.
    fn on_filter_key(&mut self, key: Key) {
        match key {
            Key::Escape => {
                self.filters[self.active].clear();
                self.filtering = false;
                self.reset_filter_selection();
            }
            Key::Enter => {
                self.filtering = false;
                self.reset_filter_selection();
            }
            Key::Backspace => {
                self.filters[self.active].pop();
                self.reset_filter_selection();
            }
            Key::Char(c) => {
                self.filters[self.active].push(c);
                self.reset_filter_selection();
            }
            _ => {}
        }
    }

    // ---- selection ----

    pub fn active_len(&self) -> usize {
        self.filtered_len(self.active)
    }

    fn active_state(&mut self) -> &mut TableState {
        match self.active {
            0 => &mut self.pr_state,
            1 => &mut self.wi_state,
            _ => &mut self.pipe_state,
        }
    }

    pub fn selected(&self) -> Option<usize> {
        match self.active {
            0 => self.pr_state.selected(),
            1 => self.wi_state.selected(),
            _ => self.pipe_state.selected(),
        }
    }

    fn clamp_selection(&mut self) {
        let len = self.active_len();
        let sel = self.active_state().selected();
        let next = match (len, sel) {
            (0, _) => None,
            (_, None) => Some(0),
            (n, Some(i)) => Some(i.min(n - 1)),
        };
        self.active_state().select(next);
    }

    pub fn move_down(&mut self) {
        let len = self.active_len();
        if len == 0 {
            return;
        }
        let next = self.active_state().selected().map_or(0, |i| (i + 1) % len);
        self.active_state().select(Some(next));
    }

    pub fn move_up(&mut self) {
        let len = self.active_len();
        if len == 0 {
            return;
        }
        let next = self.active_state().selected().map_or(0, |i| (i + len - 1) % len);
        self.active_state().select(Some(next));
    }

    /// Section indices currently shown in the tab bar, in order.
    pub fn visible_indices(&self) -> Vec<usize> {
        (0..TABS.len()).filter(|i| self.visible[*i]).collect()
    }

    fn first_visible(&self) -> usize {
        self.visible_indices().first().copied().unwrap_or(0)
    }

    /// The section a screen belongs to on the tab strip. A full-screen item view answers for
    /// the section its item came from, not `self.active`: a PR opened from the Command Center
    /// (or the inbox) leaves `self.active` on whatever section was last on screen, and Tab out
    /// of that PR must still land on the tab after Pull Requests.
    pub fn screen_section(&self) -> usize {
        match self.screen {
            Screen::PrView(_) => index_of(Section::PullRequests),
            Screen::WiView(_) => index_of(Section::WorkItems),
            Screen::Pipeline(_) => index_of(Section::Pipelines),
            _ => self.active,
        }
    }

    /// The top-level tab position: 0 = Launchpad, then each visible section.
    fn top_pos(&self) -> usize {
        if matches!(self.screen, Screen::Launchpad) {
            0
        } else {
            1 + self.visible_indices().iter().position(|&i| i == self.screen_section()).unwrap_or(0)
        }
    }

    /// Moves to a top-level tab position (0 = Launchpad, 1.. = sections).
    fn go_to_tab(&mut self, pos: usize) {
        if pos == 0 {
            self.screen = Screen::Launchpad;
        } else if let Some(&section) = self.visible_indices().get(pos - 1) {
            self.active = section;
            self.screen = Screen::List;
            self.list_scroll = 0;
            self.clamp_selection();
        }
    }

    /// Cycles across the tab strip [Launchpad, …visible sections] with wraparound.
    pub fn switch_tab(&mut self, delta: isize) {
        let n = (1 + self.visible_indices().len()) as isize;
        let next = (((self.top_pos() as isize + delta) % n) + n) % n;
        self.go_to_tab(next as usize);
    }

    /// Jumps to the Nth tab (0 = Launchpad), for the number keys.
    pub fn set_tab(&mut self, idx: usize) {
        self.go_to_tab(idx);
    }

    /// Applies persisted hidden-section preferences at startup.
    pub fn apply_hidden_sections(&mut self, hidden: &[Section]) {
        self.visible = [true; 3];
        for section in hidden {
            self.visible[index_of(*section)] = false;
        }
        if !self.visible.iter().any(|v| *v) {
            self.visible[0] = true;
        }
        if !self.visible[self.active] {
            self.active = self.first_visible();
        }
    }

    /// Applies persisted hidden work-item-state preferences at startup.
    pub fn apply_hidden_work_item_states(&mut self, hidden: &[String]) {
        self.wi_hidden_states = hidden.iter().cloned().collect();
    }

    /// Applies persisted Command Center dismissals at startup (seeds the in-memory set;
    /// `rebuild_launchpad` applies the actual filtering on the initial reload).
    pub fn apply_dismissed_launchpad_items(&mut self, ids: &[String]) {
        self.lp_dismissed_persisted = ids.iter().cloned().collect();
    }

    /// Applies the persisted Pipelines grouping at startup. `None` keeps the default.
    pub fn apply_pipe_group(&mut self, group: Option<String>) {
        if let Some(g) = group {
            self.pipe_group = PipeGroup::parse(&g);
        }
    }

    /// Applies persisted per-view sort preferences at startup.
    pub fn apply_sorts(&mut self, pr: Option<SortPref>, wi: Option<SortPref>, pipe: Option<SortPref>) {
        self.pr_sort = pr;
        self.wi_sort = wi;
        self.pipe_sort = pipe;
    }

    // ---- saved views ----

    /// Applies persisted saved views at startup, seeding defaults for empty sections.
    pub fn apply_views(&mut self, pr: Vec<SavedView>, wi: Vec<SavedView>, pipe: Vec<SavedView>) {
        self.views = [
            if pr.is_empty() { default_views(0) } else { pr },
            if wi.is_empty() { default_views(1) } else { wi },
            if pipe.is_empty() { default_views(2) } else { pipe },
        ];
    }

    /// The active view for a section, if any.
    pub fn active_view(&self, section: usize) -> Option<&SavedView> {
        self.views[section].get(self.view_idx[section])
    }

    /// Moves to the previous/next saved view on the active section and applies it.
    async fn switch_view(&mut self, delta: isize, deps: &AppDeps) {
        let n = self.views[self.active].len();
        if n <= 1 {
            return;
        }
        let idx = (self.view_idx[self.active] as isize + delta).rem_euclid(n as isize) as usize;
        self.apply_view(self.active, idx, deps).await;
    }

    /// Applies a saved view: sets the quick-filter, sort, PR base filter, and hidden
    /// states from the bundle (reloading PRs only if the server-side filter changed).
    async fn apply_view(&mut self, section: usize, idx: usize, deps: &AppDeps) {
        let Some(view) = self.views[section].get(idx).cloned() else { return };
        self.view_idx[section] = idx;
        self.toast = Some(format!("View: {}", view.name));

        self.filters[section] = view.query.clone();
        let sort = view.sort.clone();
        match section {
            0 => self.pr_sort = sort,
            1 => self.wi_sort = sort,
            _ => self.pipe_sort = sort,
        }
        if section == 1 {
            self.wi_hidden_states = view.hidden_states.iter().cloned().collect();
        }
        if section == 0 {
            let want = parse_pr_filter(view.filter.as_deref());
            if want != self.pr_filter {
                self.pr_filter = want;
                // Derived from the pool already in hand. This used to clear the list and block
                // the render loop on a full fan-out per connection, which is what made `[`/`]`
                // feel slow — for rows the previous fetch had already returned.
                self.refresh_derived_prs(deps);
            }
        }
        self.list_scroll = 0;
        self.fix_selection();
    }

    /// Snapshots the active section's current filter/sort/state into a named view.
    fn current_view_snapshot(&self, name: String) -> SavedView {
        let section = self.active;
        let hidden_states = if section == 1 {
            let mut s: Vec<String> = self.wi_hidden_states.iter().cloned().collect();
            s.sort();
            s
        } else {
            Vec::new()
        };
        SavedView {
            name,
            filter: (section == 0).then(|| pr_filter_key(self.pr_filter).to_string()),
            query: self.filters[section].clone(),
            sort: self.sort_for(section).cloned(),
            hidden_states,
        }
    }

    /// Opens the name prompt to save the current view.
    fn open_save_view(&mut self) {
        self.overlay = Some(Overlay::Input {
            title: "Save current view as".into(),
            buffer: String::new(),
            kind: InputKind::SaveView,
        });
    }

    /// Saves the current filter/sort/state as a new view and switches to it.
    async fn save_view(&mut self, name: String, deps: &AppDeps) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let section = self.active;
        let view = self.current_view_snapshot(name.clone());
        self.views[section].push(view);
        self.view_idx[section] = self.views[section].len() - 1;
        // Local state first, like every other write here — but a persist that failed must not be
        // reported as "Saved": the view is on screen and will be gone on the next launch, and the
        // toast is the only chance the user gets to know that.
        match deps.config.set_views(section_of(section), self.views[section].clone()).await {
            Err(e) => self.toast_error(format!("Couldn't save view: {e}")),
            Ok(()) => self.toast = Some(format!("Saved view: {name}")),
        }
    }

    /// Confirms deleting the active section's current view (never the last one).
    fn open_delete_view(&mut self) {
        let section = self.active;
        if self.views[section].len() <= 1 {
            self.toast = Some("Can't delete the last view".into());
            return;
        }
        let name = self.active_view(section).map(|v| v.name.clone()).unwrap_or_default();
        self.overlay = Some(Overlay::Confirm {
            title: "Delete view".into(),
            message: format!("Delete view '{name}'?"),
            action: Action::DeleteView,
        });
    }

    /// Removes the current view, persists, and applies whatever view is now current.
    async fn delete_view(&mut self, deps: &AppDeps) {
        let section = self.active;
        if self.views[section].len() <= 1 {
            return;
        }
        let idx = self.view_idx[section].min(self.views[section].len() - 1);
        let removed = self.views[section].remove(idx);
        self.view_idx[section] = idx.min(self.views[section].len() - 1);
        let saved = deps.config.set_views(section_of(section), self.views[section].clone()).await;
        let target = self.view_idx[section];
        self.apply_view(section, target, deps).await;
        // Reported after `apply_view`, which toasts a "View: …" of its own — a failed delete has
        // to outrank it, or the only sign the view is coming back is that it comes back.
        match saved {
            Err(e) => self.toast_error(format!("Couldn't delete view: {e}")),
            Ok(()) => self.toast = Some(format!("Deleted view: {}", removed.name)),
        }
    }

    /// The active sort for a section, if any.
    pub fn sort_for(&self, section: usize) -> Option<&SortPref> {
        match section {
            0 => self.pr_sort.as_ref(),
            1 => self.wi_sort.as_ref(),
            _ => self.pipe_sort.as_ref(),
        }
    }

    /// Opens the sort-column picker for the active list.
    fn open_sort_picker(&mut self) {
        let cols = sort_cols(self.active);
        let items: Vec<String> = cols.iter().map(|c| c.label.to_string()).collect();
        let selected = self
            .sort_for(self.active)
            .and_then(|s| cols.iter().position(|c| c.key == s.key))
            .unwrap_or(0);
        self.overlay = Some(Overlay::Picker {
            title: "Sort by".into(),
            items,
            selected,
            kind: PickerKind::SortColumn { section: self.active },
        });
    }

    /// Applies a chosen sort column: same column toggles direction, a new column
    /// starts at its sensible default direction. Persists the choice.
    async fn apply_sort(&mut self, section: usize, index: usize, deps: &AppDeps) {
        let Some(col) = sort_cols(section).get(index) else { return };
        let key = col.key.to_string();
        let label = col.label;
        let desc = match self.sort_for(section) {
            Some(s) if s.key == key => !s.desc, // re-pick the same column → flip
            _ => default_desc(&key),
        };
        let pref = SortPref { key, desc };
        match section {
            0 => self.pr_sort = Some(pref.clone()),
            1 => self.wi_sort = Some(pref.clone()),
            _ => self.pipe_sort = Some(pref.clone()),
        }
        let arrow = if desc { "↓" } else { "↑" };
        self.toast = Some(format!("Sorted by {label} {arrow}"));
        self.list_scroll = 0;
        self.fix_selection();
        let _ = deps.config.set_sort(section_of(section), Some(pref)).await;
    }

    /// Opens the notifications checklist (opt in/out of each event type).
    fn open_notifications_toggle(&mut self) {
        let n = &self.notifications;
        let items = vec![
            ToggleItem { id: "pipeline_failed".into(), label: "Pipeline failed".into(), on: n.pipeline_failed },
            ToggleItem { id: "review_requested".into(), label: "Review requested".into(), on: n.review_requested },
            ToggleItem { id: "pr_approved".into(), label: "Your PR approved".into(), on: n.pr_approved },
            ToggleItem { id: "pr_changes_requested".into(), label: "Your PR: changes requested".into(), on: n.pr_changes_requested },
            ToggleItem { id: "pipeline_approval_needed".into(), label: "Pipeline approval needed".into(), on: n.pipeline_approval_needed },
        ];
        self.overlay =
            Some(Overlay::Toggle { title: "Notifications".into(), kind: ToggleKind::Notifications, min_one: false, items, selected: 0, filter: None });
    }

    /// Applies the notifications checklist: the ticked ids become the enabled set.
    async fn apply_notifications(&mut self, ids: Vec<String>, deps: &AppDeps) {
        let has = |k: &str| ids.iter().any(|i| i == k);
        self.notifications = NotificationPrefs {
            pipeline_failed: has("pipeline_failed"),
            review_requested: has("review_requested"),
            pr_approved: has("pr_approved"),
            pr_changes_requested: has("pr_changes_requested"),
            pipeline_approval_needed: has("pipeline_approval_needed"),
        };
        // Re-seed silently so newly-enabled events don't fire for pre-existing state.
        self.pipe_seeded = false;
        self.approval_seeded = false;
        self.pr_scan_seeded = false;
        let _ = deps.config.set_notifications(self.notifications).await;
        if self.notifications.any() {
            // A real notification so you can confirm they work on this machine.
            self.notifier.notify("forgetop notifications enabled", "You'll be pinged on the events you chose.");
            self.toast = Some("Notifications updated".into());
        } else {
            self.toast = Some("All notifications off".into());
        }
    }

    /// Fetches the review-requested and my-PR sets and notifies on new events.
    /// Applies a completed background job on the render loop.
    pub fn on_event(&mut self, event: AppEvent, deps: &AppDeps) {
        match event {
            AppEvent::Reloaded(r) => {
                self.apply_reloaded(*r, deps);
                self.refresh_preview_row();
                // The scope indicator's denominator depends on the catalog the fetch may have
                // just brought back, so it is recomputed here rather than inside the fetch.
                self.refresh_repo_scope(deps);
            }
            AppEvent::PrDetailLoaded { key, detail, fetched_at } => {
                self.with_preview_screen(&key.clone(), |app| app.apply_pr_detail(deps, key, *detail, fetched_at));
            }
            AppEvent::WiDetailLoaded { key, detail, fetched_at } => {
                self.with_preview_screen(&key.clone(), |app| app.apply_wi_detail(deps, key, *detail, fetched_at));
            }
            AppEvent::PipelineDetailLoaded { key, detail, fetched_at } => {
                self.with_preview_screen(&key.clone(), |app| app.apply_pipeline_detail(deps, key, *detail, fetched_at));
            }
            AppEvent::PrDecorationsLoaded { items } => {
                self.apply_pr_decorations(items, deps);
            }
            AppEvent::PipelineLogsLoaded { conn_id, run_id, job_id, text } => {
                self.apply_pipeline_logs(&conn_id, &run_id, &job_id, text);
            }
        }
        self.drive_logs(deps);
    }


    /// Rebuilds the Launchpad rows from the current feeds (no fetch) and clamps selection.
    fn rebuild_launchpad(&mut self) {
        let built = launchpad::build(&self.lp_prs_review, &self.lp_prs_mine, &self.wis, &self.pipes);
        self.lp = built.entries;
        self.lp_overflow = built.overflow;
        // Drop anything already acted on this session (e.g. a PR you've reviewed), or
        // explicitly dismissed by the user via the `D` key.
        self.lp.retain(|e| {
            let key = launchpad::Entry::key(&e.connection_id, e.item_id());
            !self.lp_dismissed.contains(&key) && !self.lp_dismissed_persisted.contains(&key)
        });
        for side in 0..2 {
            let len = self.lp_slots(side).len();
            if self.lp_sel[side] >= len {
                self.lp_sel[side] = len.saturating_sub(1);
            }
        }
    }

    /// Removes an item from the Launchpad now that you've acted on it, so it disappears
    /// immediately instead of lingering until the provider's feed catches up.
    fn dismiss_from_launchpad(&mut self, connection_id: &str, item_id: &str) {
        self.lp_dismissed.insert(launchpad::Entry::key(connection_id, item_id));
        self.rebuild_launchpad();
    }

    /// Permanently dismisses the selected Command Center item (the `D` key), persisting
    /// the choice to config so it stays hidden across restarts. Only affects the Launchpad
    /// view (`self.lp`); the Pull Requests / Work Items / Pipelines tabs are untouched.
    async fn dismiss_selected_lp_item(&mut self, deps: &AppDeps) {
        let Some(LpSlot::Entry(i)) = self.lp_selected_slot() else { return };
        let Some(entry) = self.lp.get(i) else { return };
        let key = launchpad::Entry::key(&entry.connection_id, entry.item_id());
        self.lp_dismissed_persisted.insert(key);
        self.rebuild_launchpad();

        let ids: Vec<String> = self.lp_dismissed_persisted.iter().cloned().collect();
        if let Err(e) = deps.config.set_dismissed_launchpad_items(ids).await {
            self.toast = Some(format!("Couldn't save: {e}"));
        }
    }

    /// Indices into `self.lp` for a column (0 = left, 1 = right), in display order.
    pub fn lp_column(&self, side: usize) -> Vec<usize> {
        self.lp.iter().enumerate().filter(|(_, e)| e.bucket.column() == side).map(|(i, _)| i).collect()
    }

    /// Whether a capped reference bucket had more than it shows (drives the "more…" slot).
    fn bucket_overflowed(&self, b: launchpad::Bucket) -> bool {
        use launchpad::Bucket::*;
        match b {
            NeedsReview => self.lp_overflow.needs_review,
            YourWork => self.lp_overflow.your_work,
            YourOpenPrs => self.lp_overflow.your_open_prs,
            RecentlyMerged => self.lp_overflow.recently_merged,
            RecentPipelines => self.lp_overflow.recent_pipelines,
            _ => false,
        }
    }

    /// The selectable slots for a column: each entry, plus a "more…" slot after the last entry
    /// of any overflowing bucket.
    pub fn lp_slots(&self, side: usize) -> Vec<LpSlot> {
        let col = self.lp_column(side);
        let mut slots = Vec::with_capacity(col.len());
        for (pos, &i) in col.iter().enumerate() {
            slots.push(LpSlot::Entry(i));
            let bucket = self.lp[i].bucket;
            let last_of_bucket = col.get(pos + 1).map(|&j| self.lp[j].bucket != bucket).unwrap_or(true);
            if last_of_bucket && self.bucket_overflowed(bucket) {
                slots.push(LpSlot::More(bucket));
            }
        }
        slots
    }

    /// The slot the cursor is on, if any.
    fn lp_selected_slot(&self) -> Option<LpSlot> {
        self.lp_slots(self.lp_side).into_iter().nth(self.lp_sel[self.lp_side])
    }

    fn lp_move(&mut self, delta: isize) {
        let len = self.lp_slots(self.lp_side).len();
        if len > 0 {
            let cur = self.lp_sel[self.lp_side] as isize;
            self.lp_sel[self.lp_side] = (cur + delta).clamp(0, len as isize - 1) as usize;
        }
        self.anim = 0; // restart the title scroll on the newly-selected row
    }

    fn lp_switch_side(&mut self, delta: isize) {
        self.lp_side = (self.lp_side as isize + delta).clamp(0, 1) as usize;
        self.anim = 0;
    }

    /// Advances the shared animation frame one step (driven by a fast timer): the
    /// selected-row title marquee and the running-pipeline spinner.
    pub fn tick_anim(&mut self) {
        self.anim = self.anim.wrapping_add(1);
    }

    // ---- preview pane ----

    /// Whether the section list shows the preview pane: on unless switched off for this
    /// section, and only when the terminal is wide enough for both halves.
    pub fn preview_shown(&self) -> bool {
        self.active < 3 && !self.preview_hidden[self.active] && self.content_w >= PREVIEW_MIN_WIDTH
    }

    /// Applies the persisted per-section preview switches at startup.
    pub fn apply_preview_hidden(&mut self, hidden: &[Section]) {
        self.preview_hidden = [false; 3];
        for section in hidden {
            self.preview_hidden[index_of(*section)] = true;
        }
    }

    /// The pipeline run the preview shows for the selected line: the run itself, or a
    /// group's most recent run — the one its header describes.
    fn preview_pipe(&self) -> Option<&PipeRow> {
        if self.active != 2 {
            return None;
        }
        let sel = self.pipe_state.selected()?;
        match self.pipe_lines().get(sel)? {
            PipeLine::Run(i) => self.pipes.get(*i),
            PipeLine::Head(h) => self.pipes.get(h.latest),
        }
    }

    /// The detail key of the item the selected row would preview — cheap enough to compare on
    /// every tick, so the view itself is only built when the selection actually moves.
    fn preview_key(&self) -> Option<String> {
        match self.active {
            0 => self.selected_pr_row().map(|r| pr_detail_cache_key(&r.connection_id, &r.pr.item_ref())),
            1 => self.selected_wi_row().map(|r| wi_detail_cache_key(&r.connection_id, &r.wi.item_ref())),
            _ => self.preview_pipe().map(|p| pipeline_detail_cache_key(&p.connection_id, &p.run.item_ref())),
        }
    }

    fn build_selected_preview(&self, deps: &AppDeps) -> Option<(Screen, DetailRequest)> {
        match self.active {
            0 => {
                let row = self.selected_pr_row()?;
                Some(Self::build_pr_view(deps, 0, pr_label(&row.pr), row.pr.url.clone(), row.connection_id.clone(), row.pr.clone()))
            }
            1 => {
                let row = self.selected_wi_row()?;
                Some(Self::build_wi_view(deps, row.connection_id.clone(), row.wi.clone()))
            }
            _ => {
                let pipe = self.preview_pipe()?;
                Some(Self::build_pipeline_view(
                    deps,
                    pipe.connection_id.clone(),
                    pipe.provider,
                    pipe.run.id.clone(),
                    pipe.run.definition_id.clone(),
                    pipe.run.branch.clone(),
                    pipe_label(pipe),
                    pipe.run.clone(),
                ))
            }
        }
    }

    /// Brings the preview in line with the screen: dropped anywhere but the section list (and
    /// when the pane is off or doesn't fit), rebuilt from the row and cache when the selection
    /// moved. Rebuilding does no I/O; the fetch waits for [`App::tick_preview`].
    fn settle_preview(&mut self, deps: &AppDeps) {
        // Focus lasts only as long as the focused view: Esc, Tab or an action that closes the
        // view all land somewhere else, and the list gets its preview back from there.
        if self.preview_focus && !matches!(self.screen, Screen::PrView(_) | Screen::WiView(_) | Screen::Pipeline(_)) {
            self.preview_focus = false;
        }
        if !matches!(self.screen, Screen::List) || !self.preview_shown() {
            self.preview = None;
            return;
        }
        let Some(key) = self.preview_key() else {
            self.preview = None;
            return;
        };
        if matches!(&self.preview, Some(p) if p.section == self.active && p.key == key) {
            return;
        }
        self.preview = self.build_selected_preview(deps).map(|(view, request)| Preview {
            section: self.active,
            key: request.key().to_owned(),
            view,
            request,
            sent: false,
            ticks: 0,
        });
    }

    /// Driven by the anim timer: keeps the preview in step with the selection and the terminal
    /// width, and sends its detail fetch once the cursor has rested for
    /// [`PREVIEW_SETTLE_TICKS`].
    pub fn tick_preview(&mut self, deps: &AppDeps) {
        self.settle_preview(deps);
        let request = match self.preview.as_mut() {
            Some(p) if !p.sent => {
                p.ticks = p.ticks.saturating_add(1);
                if p.ticks < PREVIEW_SETTLE_TICKS {
                    return;
                }
                p.sent = true;
                p.request.clone()
            }
            _ => return,
        };
        self.send_detail_request(deps, request);
    }

    /// After a refresh, carries the fresh list row into the preview (a PR's status, reviewers
    /// or checks may have moved) and re-asks for a pipeline run's detail — a run's state is
    /// live, and the list row alone has no stages. A different selection is left to
    /// [`App::settle_preview`], which rebuilds.
    fn refresh_preview_row(&mut self) {
        if self.preview.as_ref().map(|p| p.key.clone()) != self.preview_key() {
            return;
        }
        let pr = self.selected_pr_row().map(|r| r.pr.clone());
        let wi = self.selected_wi_row().map(|r| r.wi.clone());
        let Some(p) = self.preview.as_mut() else { return };
        match &mut p.view {
            Screen::PrView(v) => {
                if let Some(pr) = pr {
                    v.pr = pr;
                }
            }
            Screen::WiView(v) => {
                if let Some(wi) = wi {
                    v.wi = wi;
                }
            }
            Screen::Pipeline(_) => {
                p.sent = false;
                p.ticks = PREVIEW_SETTLE_TICKS;
            }
            _ => {}
        }
    }

    fn send_detail_request(&self, deps: &AppDeps, request: DetailRequest) {
        match request {
            DetailRequest::Pr { conn_id, item, key } => self.request_pr_detail(deps, conn_id, item, key),
            DetailRequest::Wi { conn_id, item, key } => self.request_wi_detail(deps, conn_id, item, key),
            DetailRequest::Pipeline { conn_id, item, key } => self.request_pipeline_detail(deps, conn_id, item, key),
        }
    }

    /// Runs `f` with the preview's view standing in as the screen when the preview holds the
    /// item `key` names, so a detail answer patches the pane through the same `apply_*` path
    /// the full view uses. The preview only exists while the list is the screen, so nothing
    /// else can be showing that item.
    fn with_preview_screen(&mut self, key: &str, f: impl FnOnce(&mut Self)) {
        let Some(mut p) = self.preview.take_if(|p| p.key == key) else {
            f(self);
            return;
        };
        std::mem::swap(&mut self.screen, &mut p.view);
        f(self);
        std::mem::swap(&mut self.screen, &mut p.view);
        self.preview = Some(p);
    }

    /// The preview's own keys. From the list, Enter (or `p`) focuses the pane — its view becomes
    /// the screen, drawn in the same place, and the footer turns to that item's keys. With no
    /// preview showing, Enter keeps opening the full-screen view. From a focused pane `p` hands
    /// focus back, keeping the view (tab, scroll, buffered comments) as it was. `P` switches the
    /// pane off or on for the section; pressed while focused, the view stays open full screen.
    /// Whether the Pipelines list cursor is on a group header rather than a run.
    pub fn pipe_head_selected(&self) -> bool {
        self.active == 2
            && self.pipe_state.selected().is_some_and(|sel| matches!(self.pipe_lines().get(sel), Some(PipeLine::Head(_))))
    }

    /// Moves the preview's view into the screen, still drawn beside the list.
    fn focus_preview(&mut self, deps: &AppDeps) {
        let Some(mut p) = self.preview.take() else { return };
        if !p.sent {
            self.send_detail_request(deps, p.request.clone());
        }
        self.screen = std::mem::replace(&mut p.view, Screen::List);
        self.preview_focus = true;
        // Esc from the focused pane returns to this list.
        self.lp_origin = false;
        self.from_inbox = false;
    }

    async fn on_preview_key(&mut self, key: Key, deps: &AppDeps) -> bool {
        let on_list = matches!(self.screen, Screen::List);
        let focused = self.preview_focus && matches!(self.screen, Screen::PrView(_) | Screen::WiView(_) | Screen::Pipeline(_));
        match key {
            // On a pipeline group header Enter keeps expanding / collapsing the group; it is a
            // run row that Enter moves into the pane.
            Key::Enter if on_list && self.preview.is_some() && !self.pipe_head_selected() => {
                self.focus_preview(deps);
                true
            }
            Key::Char('p') if on_list => {
                if self.preview.is_some() {
                    self.focus_preview(deps);
                } else if self.preview_hidden[self.active] {
                    self.toast = Some("The preview is off here. P turns it on.".into());
                } else if self.content_w < PREVIEW_MIN_WIDTH {
                    self.toast = Some(format!("The preview needs a terminal at least {PREVIEW_MIN_WIDTH} columns wide."));
                }
                true
            }
            Key::Char('p') if focused => {
                let view = std::mem::replace(&mut self.screen, Screen::List);
                self.preview_focus = false;
                self.preview = DetailRequest::for_view(&view).map(|request| Preview {
                    section: self.active,
                    key: request.key().to_owned(),
                    view,
                    request,
                    sent: true,
                    ticks: 0,
                });
                true
            }
            Key::Char('P') if on_list || focused => {
                let section = self.active;
                self.preview_hidden[section] = !self.preview_hidden[section];
                self.preview_focus = false;
                let hidden: Vec<Section> = (0..3).filter(|&i| self.preview_hidden[i]).map(section_of).collect();
                let _ = deps.config.set_preview_hidden(hidden).await;
                self.toast = Some(if self.preview_hidden[section] { "Preview off. P turns it back on." } else { "Preview on." }.into());
                true
            }
            _ => false,
        }
    }

    async fn on_launchpad_key(&mut self, key: Key, deps: &AppDeps) {
        match key {
            // No `q` / Esc arm: the Command Center is the root, so "back" has nowhere to go.
            // Neither key quits — leaving the app is Ctrl-C.
            Key::Up | Key::Char('k') => self.lp_move(-1),
            Key::Down | Key::Char('j') => self.lp_move(1),
            // Left/right move between the two columns; Tab (handled globally) leaves for the
            // section tabs.
            Key::Left | Key::Char('h') => self.lp_switch_side(-1),
            Key::Right | Key::Char('l') => self.lp_switch_side(1),
            Key::Char(c @ '1'..='4') => self.set_tab(c as usize - '1' as usize),
            Key::Enter => self.open_launchpad_selected(deps).await,
            Key::Char('D') => self.dismiss_selected_lp_item(deps).await,
            Key::Char('r') => self.request_reload(deps),
            Key::Char('C') => self.open_connections(deps).await,
            Key::Char('t') => {
                let next = Theme::next(self.theme.name);
                self.theme = Theme::by_name(next);
                let _ = deps.config.set_theme(Some(next.to_string())).await;
            }
            _ => {}
        }
    }

    /// Follows a Launchpad "more…" link to the full section, applying the closest filter.
    async fn open_lp_more(&mut self, bucket: launchpad::Bucket, deps: &AppDeps) {
        use launchpad::Bucket::*;
        match bucket {
            NeedsReview => self.goto_pr_section(PullRequestFilter::ReviewRequested, false, deps).await,
            YourOpenPrs => self.goto_pr_section(PullRequestFilter::Mine, false, deps).await,
            RecentlyMerged => self.goto_pr_section(PullRequestFilter::Mine, true, deps).await,
            YourWork => self.goto_section(Section::WorkItems),
            RecentPipelines => self.goto_section(Section::Pipelines),
            _ => {}
        }
    }

    /// Leaves the Launchpad for a section's list.
    fn goto_section(&mut self, section: Section) {
        self.active = index_of(section);
        self.screen = Screen::List;
        self.list_scroll = 0;
        self.clamp_selection();
    }

    /// Jumps to the PR section with a base filter; `show_merged` also ticks Merged so your
    /// recently-merged PRs come back from the provider.
    async fn goto_pr_section(&mut self, filter: PullRequestFilter, show_merged: bool, deps: &AppDeps) {
        self.goto_section(Section::PullRequests);
        self.pr_filter = filter;
        if show_merged {
            self.pr_shown_statuses.insert(PullRequestStatus::Merged);
        }
        self.refresh_derived_prs(deps);
    }

    /// Opens the selected Launchpad row in its full item view, or follows a "more…" slot to the
    /// full section.
    async fn open_launchpad_selected(&mut self, deps: &AppDeps) {
        let entry = match self.lp_selected_slot() {
            Some(LpSlot::Entry(i)) => match self.lp.get(i) {
                Some(e) => e,
                None => return,
            },
            Some(LpSlot::More(bucket)) => {
                self.open_lp_more(bucket, deps).await;
                return;
            }
            None => return,
        };
        self.lp_origin = true; // opened from the Launchpad, so Esc returns there
        self.from_inbox = false;
        let (kind, conn, id) = (entry.kind(), entry.connection_id.clone(), entry.item_id().to_string());
        match kind {
            launchpad::EntryKind::Pr => {
                let found = self
                    .lp_prs_review
                    .iter()
                    .chain(self.lp_prs_mine.iter())
                    .find(|r| r.connection_id == conn && r.pr.id == id)
                    .map(|r| (pr_label(&r.pr), r.pr.url.clone(), r.pr.clone()));
                if let Some((label, url, pr)) = found {
                    self.open_pr_view_for(deps, 0, label, url, conn, pr);
                }
            }
            launchpad::EntryKind::Wi => {
                if let Some(wi) = self.wis.iter().find(|r| r.connection_id == conn && r.wi.id == id).map(|r| r.wi.clone()) {
                    self.open_wi_view_for(deps, conn, wi);
                }
            }
            launchpad::EntryKind::Pipe => {
                let found = self
                    .pipes
                    .iter()
                    .find(|r| r.connection_id == conn && r.run.id == id)
                    .map(|r| (r.provider, r.run.definition_id.clone(), r.run.branch.clone(), pipe_label(r), r.run.clone()));
                if let Some((provider, def, branch, title, fallback)) = found {
                    self.open_pipeline_for(deps, conn, provider, id, def, branch, title, fallback);
                }
            }
        }
    }

    // ---- command palette ----

    /// Open the command palette over the current screen. Empty query → this screen's actions,
    /// then every already-fetched item (most-recent first), then the go-to destinations.
    fn open_palette(&mut self) {
        let candidates = self.palette_candidates();
        let results = palette::rank("", &candidates);
        self.overlay = Some(Overlay::Palette { query: String::new(), candidates, results, selected: 0 });
    }

    /// Everything the palette can search, built from what's already in memory — no provider
    /// calls. Commands are included but only surface in `:` mode (see `palette::rank`).
    fn palette_candidates(&self) -> Vec<PaletteItem> {
        let mut c = palette::action_items(&self.context_actions(), self.screen_name());
        c.extend(palette::build_candidates(&self.prs, &self.wis, &self.pipes));
        c.extend(palette::goto_items(&self.visible));
        c.extend(palette::view_items(&self.views, &self.view_idx, &self.visible));
        c.extend(palette::repo_items(&self.pipes, self.visible[2]));
        c.extend(palette::people_items(&self.prs, &self.wis, &self.visible));
        c.extend(palette::setting_items(&crate::theme::THEMES, self.theme.name));
        c.extend(palette::key_items(&crate::ui::help_sections()));
        c.extend(palette::command_items(&CommandContext {
            themes: &crate::theme::THEMES,
            views: &self.views,
            visible: &self.visible,
            // Exactly when `m` / `u` would open their pickers.
            can_merge: matches!(&self.screen, Screen::PrView(v) if v.pr.status != PullRequestStatus::Merged),
            can_set_state: matches!(self.screen, Screen::WiView(_)),
        }));
        c
    }

    /// The current screen's name, for the subtitle of its palette actions.
    fn screen_name(&self) -> &'static str {
        match self.screen {
            Screen::Launchpad => "Command Center",
            Screen::List => TABS[self.active],
            Screen::PrView(_) => "PR view",
            Screen::WiView(_) => "Work item view",
            Screen::Pipeline(_) => "Pipeline run",
            Screen::Inbox => "Inbox",
            Screen::Config(_) => "Connections",
        }
    }

    /// The actions the palette offers on the current screen, each paired with the key that
    /// performs it. Mirrors the per-screen key handlers' own conditions — an entry is listed
    /// only when its key would do something here — and running one replays that key (see
    /// [`App::replay_key`]), so the palette and the keyboard can't drift. Pure navigation
    /// (j/k, tab switching, back) is left out.
    pub fn context_actions(&self) -> Vec<(&'static str, Key)> {
        let c = Key::Char;
        let mut out: Vec<(&'static str, Key)> = Vec::new();
        match &self.screen {
            // `on_launchpad_key`.
            Screen::Launchpad => {
                if matches!(self.lp_selected_slot(), Some(LpSlot::Entry(_))) {
                    out.push(("Dismiss from Command Center", c('D')));
                }
                out.extend([("Refresh", c('r')), ("Connections", c('C')), ("Cycle theme", c('t'))]);
            }
            // `on_preview_key`, the list arm of `on_key_inner`, and `on_char`.
            Screen::List => {
                let s = self.active;
                if !self.filters[s].is_empty() {
                    out.push(("Clear quick filter", Key::Escape));
                }
                out.push(("Quick filter", c('/')));
                match s {
                    0 => out.push(("Filter by status", c('f'))),
                    1 => out.push(("Choose which states to show", c('f'))),
                    _ => out.extend([
                        ("Trigger a run", c('T')),
                        ("Cycle grouping", c('G')),
                        ("Collapse every group", c('z')),
                        ("Expand every group", c('Z')),
                    ]),
                }
                out.extend([("Sort by column", c('S')), ("Repositories to fetch", c('g'))]);
                if self.views[s].len() > 1 {
                    out.extend([("Previous saved view", c('[')), ("Next saved view", c(']'))]);
                }
                out.push(("Save current view", c('V')));
                if self.views[s].len() > 1 {
                    out.push(("Delete current view", c('X')));
                }
                if self.selected().is_some() {
                    out.push(("Open selected in browser", c('o')));
                }
                if self.preview.is_some() {
                    out.push(("Focus the preview pane", c('p')));
                }
                out.extend([
                    ("Preview pane on / off", c('P')),
                    ("Choose visible tabs", c('v')),
                    ("Refresh", c('r')),
                    ("Cycle theme", c('t')),
                    ("Connections", c('C')),
                ]);
            }
            // The PR arm of `on_key_inner` and `on_pr_view_key`.
            Screen::PrView(v) => {
                if v.pr.status == PullRequestStatus::Merged {
                    out.push(("Revert", c('R')));
                } else {
                    out.extend([("Approve", c('a')), ("Request changes", c('x')), ("Merge…", c('m'))]);
                }
                let on_line = v.tab == 3 && v.diff.focus == DiffFocus::Patch;
                out.push((if on_line { "Comment on this line" } else { "Comment" }, c('c')));
                // `r` replies from the Conversation and Diff tabs; elsewhere it only explains that.
                if matches!(v.tab, 0 | 3) {
                    out.push(("Reply to thread", c('r')));
                }
                if !v.pending.is_empty() {
                    out.push(("Submit review", c('s')));
                }
                if v.tab == 1 && !v.commits.is_empty() {
                    out.push(("Open the commit's diff", Key::Enter));
                }
                if v.tab == 3 {
                    if v.diff.focus == DiffFocus::FileList {
                        out.push(("Line cursor in the patch", Key::Enter));
                    }
                    out.extend([
                        ("Mark file viewed", c('v')),
                        ("Next comment thread", c(']')),
                        ("Previous comment thread", c('[')),
                    ]);
                }
                if v.url.is_some() {
                    out.push(("Open in browser", c('o')));
                }
            }
            // The WI arm of `on_key_inner` and `on_wi_view_key`.
            Screen::WiView(v) => {
                out.extend([
                    ("Update state", c('u')),
                    ("Assign…", c('@')),
                    ("Edit title / description", c('e')),
                    ("Comment", c('c')),
                ]);
                if v.wi.url.is_some() {
                    out.push(("Open in browser", c('o')));
                }
            }
            // `on_pipeline_screen_key`, `on_pipeline_logs_key` and `on_pipeline_key`.
            Screen::Pipeline(v) => {
                let split = v.log_split.get();
                match &v.logs {
                    Some(log) if v.logs_have_keys() => {
                        if split {
                            out.push(("Move the keys to the tree", c('w')));
                        }
                        out.push(("Search the log", c('/')));
                        if log.query.is_some() {
                            out.extend([("Next match", c('n')), ("Previous match", c('N'))]);
                        }
                        out.extend([("Toggle follow", c('f')), ("Jump to the first error", c('E')), ("Close logs", Key::Escape)]);
                    }
                    logs => {
                        if logs.is_some() {
                            if split {
                                out.push(("Move the keys to the logs", c('w')));
                            }
                            out.push(("Close logs", c('L')));
                        } else {
                            out.push(("View the job's logs", c('L')));
                        }
                        out.push(("Trigger a run", c('T')));
                        if is_active(v.run.status) {
                            out.push(("Cancel the run", c('X')));
                        }
                        if v.can_respond_approvals && !v.actionable_approvals().is_empty() {
                            out.push(("Approve / reject a gate", c('A')));
                        }
                        if self.selected_url().is_some() {
                            out.push(("Open the job in browser", c('o')));
                        }
                    }
                }
            }
            // `on_inbox_key`.
            Screen::Inbox => {
                if let Some(row) = self.inbox.get(self.inbox_sel) {
                    if row.notification.url.is_some() {
                        out.push(("Open in browser", c('o')));
                    }
                    out.extend([("Mark read", c('x')), ("Mark all read", c('A'))]);
                }
                out.push(("Refresh", c('r')));
            }
            // `on_config_key`.
            Screen::Config(v) => {
                out.extend([
                    ("Add a connection", c('a')),
                    ("Bind Pull Requests", c('p')),
                    ("Bind Work Items", c('w')),
                    ("Pipeline subscriptions", c('s')),
                ]);
                if v.selected_conn().is_some() {
                    out.push(("Remove connection", c('x')));
                }
            }
        }
        // A view focused from the list's preview pane: `on_preview_key` answers `p` / `P` first.
        if self.preview_focus && matches!(self.screen, Screen::PrView(_) | Screen::WiView(_) | Screen::Pipeline(_)) {
            out.extend([("Back to the list", c('p')), ("Preview pane on / off", c('P'))]);
        }
        out
    }

    /// Keys an entry from help section `section` may run from the palette. The same letter
    /// means different things on different screens (`r` refreshes a list but replies in a PR
    /// view), so a section's keys run only on the screen that section describes. "Global" adds
    /// the keys `on_key_inner` answers everywhere (the log pane keeps `n` / `N` for itself).
    fn runnable_keys(&self, section: &str) -> Vec<Key> {
        let list = |tab: Option<usize>| matches!(self.screen, Screen::List) && tab.is_none_or(|t| t == self.active);
        let applies = match section {
            "Global" => matches!(self.screen, Screen::List | Screen::Launchpad),
            "Saved views" => list(None),
            "Pull Requests (list)" => list(Some(0)),
            "Work Items (list)" => list(Some(1)),
            "Pipelines" => list(Some(2)) || matches!(self.screen, Screen::Pipeline(_)),
            "PR view (after Enter)" => matches!(self.screen, Screen::PrView(_)),
            "Work Item view (after Enter)" => matches!(self.screen, Screen::WiView(_)),
            "Config / connections" => matches!(self.screen, Screen::Config(_)),
            _ => false,
        };
        let mut keys: Vec<Key> =
            if applies { self.context_actions().into_iter().map(|(_, k)| k).collect() } else { Vec::new() };
        if section == "Global" {
            keys.extend([Key::Ctrl('k'), Key::Char('?'), Key::Char('B'), Key::Char('F')]);
            if !matches!(&self.screen, Screen::Pipeline(v) if v.logs_have_keys()) {
                keys.extend([Key::Char('n'), Key::Char('N')]);
            }
            if matches!(self.screen, Screen::List | Screen::Launchpad) {
                keys.push(Key::Char('i'));
            }
        }
        keys
    }

    /// Runs a palette entry other than an item (items go through [`Action::OpenItem`]).
    async fn run_palette_target(&mut self, target: PaletteTarget, deps: &AppDeps) {
        match target {
            PaletteTarget::Item { kind, id, connection_id } => {
                if !self.leave_blocked() {
                    self.open_palette_item(kind, id, connection_id, deps).await;
                }
            }
            PaletteTarget::Key(key) => self.replay_key(key, deps).await,
            PaletteTarget::GoTo(dest) => self.palette_go_to(dest, deps).await,
            PaletteTarget::View { section, idx } => {
                if !self.leave_blocked() {
                    self.goto_section(section_of(section));
                    self.apply_view(section, idx, deps).await;
                }
            }
            PaletteTarget::Filter { section, text } => {
                if !self.leave_blocked() {
                    self.goto_section(section_of(section));
                    self.filters[section] = text;
                    self.reset_filter_selection();
                }
            }
            // Same as the `t` key, with the theme chosen rather than cycled to.
            PaletteTarget::Theme(name) => {
                self.theme = Theme::by_name(&name);
                let _ = deps.config.set_theme(Some(name)).await;
            }
            PaletteTarget::HelpKey { keys, section } => {
                // Only a row naming one key runs it: a row listing several ("/  n  N") describes
                // them together, and its first key alone can mean something else here. Ctrl
                // aliases ("Ctrl-K  Ctrl-P") are the exception — they're the same action.
                let tokens: Vec<&str> = keys.split_whitespace().collect();
                let ctrl = |t: &str| {
                    t.strip_prefix("Ctrl-")
                        .filter(|rest| rest.chars().count() == 1)
                        .and_then(|rest| rest.chars().next())
                        .map(|ch| Key::Ctrl(ch.to_ascii_lowercase()))
                };
                let key = match tokens.as_slice() {
                    [one] if one.chars().count() == 1 => one.chars().next().map(Key::Char),
                    [first, ..] if tokens.iter().all(|t| ctrl(t).is_some()) => ctrl(first),
                    _ => None,
                };
                match key.filter(|k| self.runnable_keys(&section).contains(k)) {
                    Some(key) => self.replay_key(key, deps).await,
                    None => self.toast = Some(format!("Press {keys} in {section}")),
                }
            }
            // `:merge <strategy>` — the `m` picker, with the strategy preselected.
            PaletteTarget::MergePicker { selected } => {
                if matches!(self.screen, Screen::PrView(_)) && !self.active_pr_is_merged() {
                    self.open_pr_merge();
                    if let Some(Overlay::Picker { selected: sel, .. }) = &mut self.overlay {
                        *sel = selected;
                    }
                }
            }
        }
    }

    /// Runs a palette action by replaying its key through the normal key path — the very code
    /// the keyboard reaches, so the two can't drift. The palette overlay has already been taken
    /// (see `on_overlay_key`), so the key reaches the screen rather than the palette. Boxed
    /// because it recurses into `on_key_inner`.
    async fn replay_key(&mut self, key: Key, deps: &AppDeps) {
        Box::pin(self.on_key_inner(key, deps)).await;
    }

    /// A palette destination: the same functions the tab strip and the global keys call.
    async fn palette_go_to(&mut self, dest: GoTo, deps: &AppDeps) {
        let leaves = matches!(dest, GoTo::CommandCenter | GoTo::Section(_) | GoTo::Inbox | GoTo::Connections);
        if leaves && self.leave_blocked() {
            return;
        }
        match dest {
            GoTo::CommandCenter => self.set_tab(0),
            GoTo::Section(i) => self.goto_section(section_of(i)),
            GoTo::Inbox => self.open_inbox(),
            GoTo::Connections => self.open_connections(deps).await,
            GoTo::Help => self.overlay = Some(Overlay::Help { scroll: 0 }),
            GoTo::Dashboard => self.open_dashboard(),
            GoTo::AddConnection => self.open_setup_picker(),
            GoTo::Notifications => self.open_notifications_toggle(),
            GoTo::Refresh => self.request_reload(deps),
        }
    }

    /// Leaving a PR view with unsubmitted line comments asks first — the same prompt Esc and
    /// Tab raise. True when that prompt was opened instead of leaving.
    fn leave_blocked(&mut self) -> bool {
        if matches!(&self.screen, Screen::PrView(v) if !v.pending.is_empty()) {
            self.open_pending_exit_prompt();
            return true;
        }
        false
    }

    /// Open the item chosen in the palette, re-resolving the full struct from the section
    /// lists by `(kind, id)` and reusing the same open path as selecting it on its screen.
    async fn open_palette_item(&mut self, kind: PaletteKind, id: String, conn: String, deps: &AppDeps) {
        // Esc from the opened view should return to wherever the palette was invoked from; from
        // an open view, that's where that view itself came from, so its origin is kept.
        match self.screen {
            Screen::PrView(_) | Screen::WiView(_) | Screen::Pipeline(_) => {}
            _ => {
                self.lp_origin = matches!(self.screen, Screen::Launchpad);
                self.from_inbox = matches!(self.screen, Screen::Inbox);
            }
        }
        // A full view, not the list's preview: the opened item has no list row beside it.
        self.preview_focus = false;
        match kind {
            PaletteKind::Pr => {
                let found = self
                    .prs
                    .iter()
                    .find(|r| r.connection_id == conn && r.pr.id == id)
                    .map(|r| (pr_label(&r.pr), r.pr.url.clone(), r.pr.clone()));
                if let Some((label, url, pr)) = found {
                    self.open_pr_view_for(deps, 0, label, url, conn, pr);
                }
            }
            PaletteKind::Wi => {
                if let Some(wi) = self.wis.iter().find(|r| r.connection_id == conn && r.wi.id == id).map(|r| r.wi.clone()) {
                    self.open_wi_view_for(deps, conn, wi);
                }
            }
            PaletteKind::Pipe => {
                let found = self
                    .pipes
                    .iter()
                    .find(|r| r.connection_id == conn && r.run.id == id)
                    .map(|r| (r.provider, r.run.definition_id.clone(), r.run.branch.clone(), pipe_label(r), r.run.clone()));
                if let Some((provider, def, branch, title, fallback)) = found {
                    self.open_pipeline_for(deps, conn, provider, id, def, branch, title, fallback);
                }
            }
        }
    }

    // ---- notification inbox ----

    /// Show the notification inbox, keeping its selection in range.
    fn open_inbox(&mut self) {
        self.inbox_sel = self.inbox_sel.min(self.inbox.len().saturating_sub(1));
        self.screen = Screen::Inbox;
    }

    /// Open the web dashboard in the browser, if its server is running.
    pub fn open_dashboard(&mut self) {
        self.open_dashboard_at("");
    }

    /// Open the same public GitHub issue form used by the dashboard's feedback entry point.
    fn open_feedback(&mut self) {
        self.open_feedback_with(self.feedback_opener, forgetop_core::diag::log);
    }

    /// Small opener seam so tests can verify the exact external destination and safe failure
    /// logging without launching a browser.
    fn open_feedback_with<E>(
        &mut self,
        opener: impl FnOnce(&str) -> std::result::Result<(), E>,
        log_failure: impl FnOnce(&str, &str),
    ) where
        E: std::fmt::Display,
    {
        self.toast = Some(match opener(FEEDBACK_ISSUE_URL) {
            Ok(()) => "Opening feedback form in your browser…".into(),
            Err(error) => {
                // Keep the destination out of diagnostics even though it currently has no secret.
                log_failure(FEEDBACK_OPEN_FAILURE_CONTEXT, FEEDBACK_OPEN_FAILURE_MESSAGE);
                format!("Couldn't open feedback form: {error}")
            }
        });
    }

    /// Open the dashboard at a specific view (e.g. `#settings` for connection management).
    fn open_dashboard_at(&mut self, hash: &str) {
        self.open_dashboard_at_with(hash, |target| open::that(target), forgetop_core::diag::log);
    }

    /// Dashboard opener seam kept deliberately small: production delegates to the OS while
    /// tests can assert the exact fragment and failure logging without launching a browser.
    fn open_dashboard_at_with<E>(
        &mut self,
        hash: &str,
        opener: impl FnOnce(&str) -> std::result::Result<(), E>,
        log_failure: impl FnOnce(&str, &str),
    ) where
        E: std::fmt::Display,
    {
        let Some(url) = &self.dashboard_url else {
            self.toast = Some("Web dashboard isn't running — start it with `forgetop --dashboard`".into());
            return;
        };
        let target = dashboard_target(url, hash);
        self.toast = Some(match opener(&target) {
            Ok(()) => "Opening dashboard in your browser…".into(),
            Err(error) => {
                // Never log `target`: it contains the dashboard's per-session access token.
                log_failure(DASHBOARD_OPEN_FAILURE_CONTEXT, DASHBOARD_OPEN_FAILURE_MESSAGE);
                format!("Couldn't open dashboard: {error}")
            }
        });
    }

    /// Number of unread notifications — drives the header indicator.
    pub fn unread_count(&self) -> usize {
        self.inbox.iter().filter(|r| r.notification.unread).count()
    }

    fn inbox_move(&mut self, delta: isize) {
        let n = self.inbox.len();
        if n == 0 {
            return;
        }
        self.inbox_sel = (self.inbox_sel as isize + delta).rem_euclid(n as isize) as usize;
    }

    async fn on_inbox_key(&mut self, key: Key, deps: &AppDeps) {
        match key {
            Key::Escape => self.screen = Screen::Launchpad,
            Key::Up | Key::Char('k') => self.inbox_move(-1),
            Key::Down | Key::Char('j') => self.inbox_move(1),
            Key::Enter => self.open_inbox_selected(deps).await,
            Key::Char('o') => {
                if let Some(url) = self.inbox.get(self.inbox_sel).and_then(|r| r.notification.url.clone()) {
                    self.toast = Some(match open::that(&url) {
                        Ok(_) => "Opened in browser".into(),
                        Err(e) => format!("Couldn't open: {e}"),
                    });
                }
            }
            Key::Char('x') => self.mark_selected_inbox_read(deps).await,
            Key::Char('A') => self.mark_all_inbox_read(deps).await,
            Key::Char('r') => self.request_reload(deps),
            _ => {}
        }
    }

    /// Drill into the item a notification points at (fetching it), or open the browser when
    /// there's no in-app target. Opening also marks the notification read.
    async fn open_inbox_selected(&mut self, deps: &AppDeps) {
        let Some(row) = self.inbox.get(self.inbox_sel) else { return };
        let conn = row.connection_id.clone();
        let n = &row.notification;
        let (item_type, item_id, url, notif_id) = (n.item_type, n.item_id.clone(), n.url.clone(), n.id.clone());
        // A notification names the repository its item lives in, which is what lets the inbox
        // open an item on a connection that spans several of them.
        let item_repo = n.repository.clone();

        if let Err(error) = self.mark_inbox_read(&conn, &notif_id, deps).await {
            self.toast = Some(error);
            return;
        }

        match (item_type, item_id) {
            (NotificationItemType::PullRequest, Some(id)) => {
                if let Some(src) = self.pr_source_for(&conn, deps).await {
                    match src.get(&ItemRef::maybe(item_repo.clone(), id.clone())).await {
                        Ok(pr) => {
                            self.from_inbox = true;
                            self.lp_origin = false;
                            let (label, purl) = (pr_label(&pr), pr.url.clone());
                            self.open_pr_view_for(deps, 0, label, purl, conn, pr);
                            return;
                        }
                        Err(_) => log_operation_failure(DIAG_INBOX_PR_DETAIL),
                    }
                }
            }
            (NotificationItemType::WorkItem, Some(id)) => {
                if let Some(src) = self.wi_source_for(&conn, deps).await {
                    match src.get(&ItemRef::maybe(item_repo.clone(), id.clone())).await {
                        Ok(wi) => {
                            self.from_inbox = true;
                            self.lp_origin = false;
                            self.open_wi_view_for(deps, conn, wi);
                            return;
                        }
                        Err(_) => log_operation_failure(DIAG_INBOX_WI_DETAIL),
                    }
                }
            }
            _ => {}
        }
        // No in-app target (or the fetch failed) — fall back to the browser.
        match url {
            Some(u) => {
                self.toast = Some(match open::that(&u) {
                    Ok(_) => "Opened in browser".into(),
                    Err(e) => format!("Couldn't open: {e}"),
                })
            }
            None => self.toast = Some("Nothing to open for this notification".into()),
        }
    }

    async fn mark_selected_inbox_read(&mut self, deps: &AppDeps) {
        if let Some((conn, id)) = self.inbox.get(self.inbox_sel).map(|r| (r.connection_id.clone(), r.notification.id.clone())) {
            self.toast = Some(match self.mark_inbox_read(&conn, &id, deps).await {
                Ok(()) => "Marked read".into(),
                Err(error) => error,
            });
        }
    }

    async fn mark_inbox_read(&mut self, conn: &str, notif_id: &str, deps: &AppDeps) -> std::result::Result<(), String> {
        let feeds = deps.sections.notification_feeds().await.map_err(|error| {
            inbox_action_error(
                "Couldn't load notification connections",
                error,
                DIAG_INBOX_FEEDS,
            )
        })?;
        let feed = feeds
            .iter()
            .find(|feed| feed.connection.connection_id() == conn)
            .ok_or_else(|| {
                inbox_action_error(
                    "Couldn't mark notification read",
                    "notification connection unavailable",
                    DIAG_INBOX_MARK_READ,
                )
            })?;
        feed.source.mark_read(notif_id).await.map_err(|error| {
            inbox_action_error(
                "Couldn't mark notification read",
                error,
                DIAG_INBOX_MARK_READ,
            )
        })?;
        for row in &mut self.inbox {
            if row.connection_id == conn && row.notification.id == notif_id {
                row.notification.unread = false;
            }
        }
        mark_cached_inbox_read(&deps.cache, |row| row.connection_id == conn && row.notification.id == notif_id);
        Ok(())
    }

    async fn mark_all_inbox_read(&mut self, deps: &AppDeps) {
        let feeds = match deps.sections.notification_feeds().await {
            Ok(feeds) => feeds,
            Err(error) => {
                self.toast = Some(inbox_action_error(
                    "Couldn't load notification connections",
                    error,
                    DIAG_INBOX_FEEDS,
                ));
                return;
            }
        };
        if feeds.is_empty() && !self.inbox.is_empty() {
            self.toast = Some(inbox_action_error(
                "Couldn't mark all notifications read",
                "notification connections unavailable",
                DIAG_INBOX_MARK_ALL_READ,
            ));
            return;
        }
        for feed in feeds {
            if let Err(error) = feed.source.mark_all_read().await {
                self.toast = Some(inbox_action_error(
                    "Couldn't mark all notifications read",
                    error,
                    DIAG_INBOX_MARK_ALL_READ,
                ));
                return;
            }
        }
        for row in &mut self.inbox {
            row.notification.unread = false;
        }
        mark_cached_inbox_read(&deps.cache, |_| true);
        self.toast = Some("All marked read".into());
    }

    // ---- data loading ----

    /// Parameters the background fetch needs, snapshotted from `self` at spawn time.
    fn reload_params(&self) -> ReloadParams {
        ReloadParams {
            // Stamped here — before the fetch is spawned — rather than when the answer lands.
            // The store refuses a write that isn't newer than what it holds, which is only a
            // real guard if the timestamp says when the request *started*: two reloads racing
            // must be ordered by when they were sent, or the slow one (which asked first, and
            // therefore knows less) lands last and wins.
            requested_at: Utc::now(),
            // Discovery is a once-per-session cost, so ask for it only until it has landed.
            // It used to run inline; now it rides along with the background fetch.
            seed_catalog: self.repo_catalog.is_empty(),
            notifications: self.notifications,
            review_seen: self.review_req_seen.clone(),
            pr_review_seen: self.pr_review_seen.clone(),
            scan_seeded: self.pr_scan_seeded,
            notifier: self.notifier.clone(),
            open_pipeline: match &self.screen {
                Screen::Pipeline(v) => Some((v.connection_id.clone(), v.run.item_ref())),
                _ => None,
            },
        }
    }

    /// Runs the whole network fetch (no `&mut self`), firing PR-notification pings along
    /// the way. Safe to call from a spawned task — everything it needs is owned.
    async fn fetch_all(deps: AppDeps, p: ReloadParams) -> Reloaded {
        let mut errors = Vec::new();
        // One unfiltered fetch. The list section, both Launchpad PR buckets and the
        // notification scan all read from it — they used to run four separate queries per
        // connection that differed only in a filter applied after the response came back.
        let (pr_pool, prs_ok) = fetch_pr_pool(&deps, &mut errors).await;
        let (wis, wis_ok) = fetch_work_items(&deps, &mut errors).await;
        let (pipes, pipes_ok) = fetch_pipelines(&deps, &mut errors).await;
        let (inbox, inbox_ok) = fetch_notifications(&deps, &mut errors).await;
        let health = deps.health.check_all().await;
        let review = derive_pool_rows(&pr_pool, PullRequestFilter::ReviewRequested, false);
        let mine = derive_pool_rows(&pr_pool, PullRequestFilter::Mine, false);
        let scan = scan_pr_notifications(&deps, &p, &review, &mine).await;
        let open_pipeline = match &p.open_pipeline {
            Some((conn_id, run_ref)) => fetch_open_pipeline(&deps, conn_id, run_ref).await,
            None => None,
        };
        let catalog = if p.seed_catalog { Some(discover_repo_catalog(&deps).await) } else { None };
        // The Launchpad buckets are derived from the same fetch as the list, so they share its
        // outcome: there is no longer a separate call that could fail on its own.
        let sections_ok = SectionsOk {
            prs: prs_ok,
            wis: wis_ok,
            pipes: pipes_ok,
            inbox: inbox_ok,
            lp_mine: prs_ok,
            lp_review: prs_ok,
        };
        Reloaded {
            pr_pool,
            wis,
            pipes,
            inbox,
            health,
            scan,
            open_pipeline,
            errors,
            catalog,
            sections_ok,
            requested_at: p.requested_at,
        }
    }

    /// Folds a completed fetch back into the app state: the lists, the Launchpad, the
    /// pipeline notifications (which compare against the seen-sets), and the status line.
    fn apply_reloaded(&mut self, r: Reloaded, deps: &AppDeps) {
        self.apply_reloaded_with_logger(r, deps, forgetop_core::diag::log);
    }

    fn apply_reloaded_with_logger(&mut self, r: Reloaded, deps: &AppDeps, mut log_failure: impl FnMut(&str, &str)) {
        // One timestamp for every section this reload writes. The store refuses a write that
        // isn't newer than what it holds, so sharing it stops a reload's own sections from
        // racing each other into the cache. It is the reload's *request* time, not now — a
        // response that landed late must not outrank one that was asked for later.
        let fetched_at = r.requested_at;
        let sections_ok = r.sections_ok;
        // The cache is protected from a failed section by the flags below; the screen has to be
        // protected too, or a total outage on the first refresh after launch wipes the rows the
        // cache just seeded and leaves the user staring at the blank screen this all exists to
        // remove. See `take_section` for the rule.
        // The pool stands in for all four PR lists, and inherits `take_section`'s three-way
        // rule: an outage that returned nothing must not replace the rows the cache seeded,
        // but partial live rows are taken. Applied to the pool *and* to each derived list,
        // because a pool kept from an earlier reload still has to repaint what is on screen.
        let pool_incoming = !r.pr_pool.open.is_empty() || !r.pr_pool.completed.is_empty();
        if sections_ok.prs || pool_incoming {
            self.pr_pool = r.pr_pool;
            self.pr_pool_loaded = true;
            self.pr_decor_failed.clear();
            // Rows this reload no longer returns must not keep a stale decoration alive; the
            // map is only meaningful for rows the pool still holds.
            let live: HashSet<(String, String)> = self
                .pr_pool
                .open
                .iter()
                .chain(self.pr_pool.completed.iter())
                .map(|row| (row.connection_id.clone(), row.pr.id.clone()))
                .collect();
            self.pr_decorations.retain(|key, _| live.contains(key));
        }
        let derived_prs = self.derive_pr_rows(self.pr_filter, self.pr_wants_completed());
        let derived_mine = self.derive_pr_rows(PullRequestFilter::Mine, true);
        let derived_review = self.derive_pr_rows(PullRequestFilter::ReviewRequested, false);
        take_section(&mut self.prs, derived_prs, sections_ok.prs);
        take_section(&mut self.lp_prs_mine, derived_mine, sections_ok.lp_mine);
        take_section(&mut self.lp_prs_review, derived_review, sections_ok.lp_review);
        self.request_pr_decorations(deps);
        take_section(&mut self.wis, r.wis, sections_ok.wis);
        take_section(&mut self.pipes, r.pipes, sections_ok.pipes);
        self.prune_pipe_expanded();
        take_section(&mut self.inbox, r.inbox, sections_ok.inbox);
        if self.inbox_sel >= self.inbox.len() {
            self.inbox_sel = self.inbox.len().saturating_sub(1);
        }
        self.health = r.health;
        // Keyed by the filter `self.prs` was just derived under, not the one this fetch was
        // spawned with. They differ whenever the user switches view mid-flight, and the rows
        // now follow the screen rather than the request — caching them under the requesting
        // filter's key would paint one view's rows under another's heading on the next launch.
        let prs_key = prs_cache_key(self.pr_filter, self.pr_wants_completed());
        self.write_through(deps, &prs_key, fetched_at, sections_ok);
        // Only once every section is live is the cached age gone; while any section is still
        // showing carried-over rows, "showing Nm old" is telling the truth and must keep saying so.
        if sections_ok.all() {
            self.data_age = None;
        }
        if let Some(catalog) = r.catalog {
            self.repo_catalog = catalog;
        }
        // A connection landed while we were waiting on the browser — the reason for the
        // waiting card is gone, so retire it rather than leave it sitting over live data.
        // `health` is the same signal the first-run hint keys off, so the two agree.
        if self.awaiting_browser_setup && !self.health.is_empty() {
            self.awaiting_browser_setup = false;
            self.toast = Some("Connection added — you're all set".into());
        }
        if let Some(scan) = r.scan {
            if let Some(seen) = scan.review_seen {
                self.review_req_seen = seen;
            }
            if let Some(seen) = scan.pr_review_seen {
                self.pr_review_seen = seen;
            }
            self.pr_scan_seeded = true;
        }
        // Keep the open pipeline view live, but only if it's still the same run (the user
        // may have navigated away during the fetch).
        if let Some((run_id, run, approvals)) = r.open_pipeline {
            if let Screen::Pipeline(v) = &mut self.screen {
                if v.run.id == run_id {
                    // This run came off the wire, so it confirms a cache-seeded view just as the
                    // detail fetch would. Going through `apply_fresh_run` rather than assigning
                    // the fields keeps `stale` honest: set it here and a view whose own detail
                    // fetch failed would keep flagging an unconfirmed status that the periodic
                    // refresh has since confirmed.
                    v.apply_fresh_run(run, approvals);
                }
            }
        }
        self.rebuild_launchpad();
        self.notify_pipeline_failures();
        self.notify_pending_approvals();
        self.fix_selection();
        self.last_refresh = Local::now();
        self.loading = false;
        self.reloading = false;
        self.status = if r.errors.is_empty() {
            format!("{} PRs · {} work items · {} runs", self.prs.len(), self.wis.len(), self.pipes.len())
        } else {
            log_failure(DIAG_REFRESH, DIAG_FAILURE_MESSAGE);
            r.errors.join("  |  ")
        };
    }

    /// Writes the lists that just landed back to the cache, all under `fetched_at`.
    ///
    /// A section is written **only when its own fetch was complete** — every feed it consulted
    /// answered. A provider outage doesn't fail a section: it drops that connection's rows and
    /// pushes a line into `errors`, so a partial result is indistinguishable from a genuine one
    /// by looking at the rows. Three connections with one down still yields a non-empty list, and
    /// caching it would silently drop the third connection's rows from the next launch.
    ///
    /// So there is deliberately no "…but cache it anyway if it came back with something"
    /// fallback, and no single global flag either — one section's outage says nothing about
    /// whether another is whole. Leaving the previous entry alone is the right answer for a
    /// cache: stale-but-whole beats fresh-but-missing-a-connection, and the user is told the age.
    /// An authoritatively empty section is `ok == true` and still caches normally.
    fn write_through(&self, deps: &AppDeps, prs_key: &str, fetched_at: DateTime<Utc>, ok: SectionsOk) {
        let cache = &deps.cache;
        if ok.prs {
            cache.put(prs_key, &self.prs, fetched_at);
        }
        if ok.wis {
            cache.put(CACHE_KEY_WORK_ITEMS, &self.wis, fetched_at);
        }
        if ok.pipes {
            cache.put(CACHE_KEY_PIPELINES, &self.pipes, fetched_at);
        }
        if ok.inbox {
            cache.put(CACHE_KEY_INBOX, &self.inbox, fetched_at);
        }
        if ok.lp_mine {
            cache.put(CACHE_KEY_LAUNCHPAD_MINE, &self.lp_prs_mine, fetched_at);
        }
        if ok.lp_review {
            cache.put(CACHE_KEY_LAUNCHPAD_REVIEW, &self.lp_prs_review, fetched_at);
        }
    }

    /// Folds a completed PR-detail fetch back in.
    ///
    /// The fetch is resolved into a concrete [`PrDetail`] first, by overlaying what came back on
    /// what is already cached: a call that failed keeps the value we last knew, rather than
    /// blanking that section on screen and in the cache because one endpoint 502'd. The overlay
    /// is against the *cached* entry, never the open view, so this behaves identically whether
    /// or not the PR is still on screen. A fetch where every call failed is a no-op.
    ///
    /// The resolved detail is then written through even if the user has since navigated away —
    /// the data is good, and a later visit should not have to re-fetch it. The view is patched
    /// only if it is still showing the *same* PR: the key is recomputed from the view's own
    /// connection/PR rather than trusted from the event, because the user can press Escape or
    /// open a different PR while this fetch was in flight, and a late answer landing on whatever
    /// happens to be on screen would show one PR's data under another's chrome.
    fn apply_pr_detail(&mut self, deps: &AppDeps, key: String, fetch: PrDetailFetch, fetched_at: DateTime<Utc>) {
        let PrDetailFetch { threads, files, checks, commits, timeline } = fetch;
        if threads.is_none() && files.is_none() && checks.is_none() && commits.is_none() && timeline.is_none() {
            // Nothing was learned, so there is nothing to write and nothing to repaint. Caching
            // four empty lists here is precisely how a total outage used to erase a good entry.
            return;
        }
        let known = match deps.cache.get::<PrDetail>(&key) {
            Some(entry) => entry.value,
            None => PrDetail { threads: Vec::new(), files: Vec::new(), checks: Vec::new(), commits: Vec::new(), timeline: Vec::new() },
        };
        let detail = PrDetail {
            threads: threads.unwrap_or(known.threads),
            files: files.unwrap_or(known.files),
            checks: checks.unwrap_or(known.checks),
            commits: commits.unwrap_or(known.commits),
            timeline: timeline.unwrap_or(known.timeline),
        };
        // A refusal means the store already holds a *newer* answer than this one: repainting
        // from this merge would leave the screen behind the cache — the out-of-order landing the
        // recency guard exists to stop, closed on disk and left open on screen. A disabled store
        // (`--demo`, tests) refuses nothing; it stores nothing, and the view still updates.
        if deps.cache.put(&key, &detail, fetched_at) == CachePut::Stale {
            return;
        }
        let Screen::PrView(v) = &mut self.screen else { return };
        if pr_detail_cache_key(&v.connection_id, &v.pr.item_ref()) != key {
            return;
        }
        if pr_detail_unchanged(v, &detail) {
            return; // a revalidation that found nothing new must not cause a visible repaint
        }
        let PrDetail { threads, files, checks, commits, timeline } = detail;
        v.timeline = timeline;
        v.checks = checks;
        v.commits = commits;
        if v.commit_sel >= v.commits.len() {
            v.commit_sel = v.commits.len().saturating_sub(1);
        }
        v.pr_files = files.clone();
        // A per-commit drill-in (`commit_label` set) is showing that commit's own file list, not
        // the whole-PR one — overwriting `diff.files` here would silently swap the user's current
        // diff out from under them mid-read. It picks up these fresh whole-PR files the next time
        // they back out, via `reset_diff_scope`, which reads from `pr_files`.
        if v.diff.commit_label.is_none() {
            v.diff.files = files;
            if v.diff.selected >= v.diff.files.len() {
                v.diff.selected = v.diff.files.len().saturating_sub(1);
            }
        }
        v.diff.threads = threads;
    }

    /// Paints the last known lists before the first fetch has answered, so launching the app
    /// doesn't mean staring at an empty one for several seconds. Called once at startup, after
    /// the cache is hydrated and before the first reload is requested.
    ///
    /// Deliberately unconditional on age: expiring entries here would hand the blank screen back
    /// to exactly the people this exists for. The age is reported instead, via `data_age`.
    pub fn seed_from_cache(&mut self, deps: &AppDeps) {
        let cache = &deps.cache;
        let mut oldest = None;
        let prs_key = prs_cache_key(self.pr_filter, self.pr_wants_completed());
        seed_section(cache, &prs_key, &mut self.prs, &mut oldest);
        seed_section(cache, CACHE_KEY_WORK_ITEMS, &mut self.wis, &mut oldest);
        seed_section(cache, CACHE_KEY_PIPELINES, &mut self.pipes, &mut oldest);
        seed_section(cache, CACHE_KEY_INBOX, &mut self.inbox, &mut oldest);
        seed_section(cache, CACHE_KEY_LAUNCHPAD_MINE, &mut self.lp_prs_mine, &mut oldest);
        seed_section(cache, CACHE_KEY_LAUNCHPAD_REVIEW, &mut self.lp_prs_review, &mut oldest);
        self.data_age = oldest;
        if self.inbox_sel >= self.inbox.len() {
            self.inbox_sel = self.inbox.len().saturating_sub(1);
        }
        // Seeded rows are only visible once the views derived from them are rebuilt — the
        // Launchpad is the landing screen, so skipping this would seed into nothing.
        self.rebuild_launchpad();
        self.fix_selection();
    }

    /// Kicks off a background refresh (periodic poll / manual `r`) without blocking the
    /// render loop, so the header spinner keeps animating. Single-flight: a refresh
    /// already in progress is left to finish.
    pub fn request_reload(&mut self, deps: &AppDeps) {
        let Some(tx) = self.job_tx.clone() else {
            return;
        };
        if self.reloading {
            return;
        }
        self.reloading = true;
        self.loading = true;
        let (deps, params) = (deps.clone(), self.reload_params());
        tokio::spawn(async move {
            let _ = tx.send(AppEvent::Reloaded(Box::new(App::fetch_all(deps, params).await)));
        });
    }

    /// Kicks off the background fetch for a just-opened PR view's detail, without blocking the
    /// render loop. Not single-flight the way [`request_reload`] is: opening a different PR (or
    /// re-opening this one) while a fetch is outstanding simply spawns another one, and
    /// `apply_pr_detail` drops whichever answer no longer matches what's on screen.
    fn request_pr_detail(&self, deps: &AppDeps, conn_id: String, item: ItemRef, key: String) {
        let Some(tx) = self.job_tx.clone() else {
            return;
        };
        let deps = deps.clone();
        // Stamped before the fetch goes out, not when it answers: the store's "an older write
        // can't overwrite a newer entry" guard only means anything if the timestamp orders the
        // requests. Stamping on completion makes whichever response is slowest also the
        // freshest-looking, which is exactly backwards for two fetches of the same PR.
        let fetched_at = Utc::now();
        tokio::spawn(async move {
            let detail = fetch_pr_detail(&deps, &conn_id, &item).await;
            let _ = tx.send(AppEvent::PrDetailLoaded { key, detail: Box::new(detail), fetched_at });
        });
    }

    /// Mirrors [`App::apply_pr_detail`]: merges the fetch against the *cached* entry (not the
    /// open view), writes it through even if the user has since navigated away, and patches the
    /// view in place — never rebuilding it, so `scroll` survives — only if it's still showing
    /// the same item. An all-failed fetch is a no-op.
    fn apply_wi_detail(&mut self, deps: &AppDeps, key: String, fetch: WiDetailFetch, fetched_at: DateTime<Utc>) {
        let WiDetailFetch { threads, timeline } = fetch;
        if threads.is_none() && timeline.is_none() {
            // Nothing was learned, so there is nothing to write and nothing to repaint.
            return;
        }
        let known = match deps.cache.get::<WiDetail>(&key) {
            Some(entry) => entry.value,
            None => WiDetail { threads: Vec::new(), timeline: Vec::new() },
        };
        let detail = WiDetail { threads: threads.unwrap_or(known.threads), timeline: timeline.unwrap_or(known.timeline) };
        if deps.cache.put(&key, &detail, fetched_at) == CachePut::Stale {
            // See `apply_pr_detail`: a newer entry already won, so the screen must not go back.
            return;
        }
        let Screen::WiView(v) = &mut self.screen else { return };
        if wi_detail_cache_key(&v.connection_id, &v.wi.item_ref()) != key {
            return;
        }
        if wi_detail_unchanged(v, &detail) {
            return; // a revalidation that found nothing new must not cause a visible repaint
        }
        v.threads = detail.threads;
        v.timeline = detail.timeline;
    }

    /// Kicks off the background fetch for a just-opened work-item view's detail, without
    /// blocking the render loop. Mirrors [`App::request_pr_detail`], including stamping
    /// `fetched_at` before the fetch goes out rather than when it answers.
    fn request_wi_detail(&self, deps: &AppDeps, conn_id: String, item: ItemRef, key: String) {
        let Some(tx) = self.job_tx.clone() else {
            return;
        };
        let deps = deps.clone();
        let fetched_at = Utc::now();
        tokio::spawn(async move {
            let detail = fetch_wi_detail(&deps, &conn_id, &item).await;
            let _ = tx.send(AppEvent::WiDetailLoaded { key, detail: Box::new(detail), fetched_at });
        });
    }

    /// Mirrors [`App::apply_pr_detail`], with one addition: a pipeline run's `status` is live in
    /// a way a PR's files/checks aren't (a cached "Running" may have failed an hour ago), so a
    /// cache-seeded view is `stale` (see [`PipelineView::stale`]) until a real `get_run` answers
    /// here — even one that confirms exactly what the cache already guessed still has to clear
    /// the flag, so the unchanged-check is skipped for it. Once confirmed, later revalidations
    /// that find nothing new go back to causing no repaint, same as the PR path.
    fn apply_pipeline_detail(&mut self, deps: &AppDeps, key: String, fetch: PipelineDetailFetch, fetched_at: DateTime<Utc>) {
        let PipelineDetailFetch { run, approvals, supports_approvals, can_respond_approvals } = fetch;
        if run.is_none() && approvals.is_none() && supports_approvals.is_none() && can_respond_approvals.is_none() {
            // Nothing was learned, so there is nothing to write and nothing to repaint.
            return;
        }
        let run_confirmed = run.is_some();
        let known = deps.cache.get::<PipelineDetail>(&key).map(|entry| entry.value);
        // Unlike the PR path's list fields, a `PipelineRun` has no empty value to stand in for
        // "unknown" — if this fetch didn't confirm one and nothing was cached before, there is
        // nothing complete enough to write.
        let Some(resolved_run) = run.or_else(|| known.as_ref().map(|k| k.run.clone())) else {
            return;
        };
        // Two different things from here on: `detail.approvals` is what the *cache* gets — the
        // merged, complete picture a later re-open should read — while `approvals` is what the
        // *view* may get, which is only ever gates this fetch actually confirmed.
        let resolved_approvals = approvals
            .clone()
            .unwrap_or_else(|| known.as_ref().map(|k| k.approvals.clone()).unwrap_or_default());
        let resolved_supports =
            supports_approvals.unwrap_or_else(|| known.as_ref().map(|k| k.supports_approvals).unwrap_or(false));
        let resolved_can_respond = can_respond_approvals
            .unwrap_or_else(|| known.as_ref().map(|k| k.can_respond_approvals).unwrap_or(false));
        let detail = PipelineDetail {
            run: resolved_run,
            approvals: resolved_approvals,
            supports_approvals: resolved_supports,
            can_respond_approvals: resolved_can_respond,
        };
        // See `apply_pr_detail`: a newer entry already won, so the screen must not go back.
        if deps.cache.put(&key, &detail, fetched_at) == CachePut::Stale {
            return;
        }

        let Screen::Pipeline(v) = &mut self.screen else { return };
        if pipeline_detail_cache_key(&v.connection_id, &v.run.item_ref()) != key {
            return;
        }
        // `approvals`, never `detail.approvals`: a gate the cache remembers may already have been
        // decided, and `actionable_approvals` turns whatever is on screen into a real
        // approve/reject against the provider. A failed `pending_approvals` therefore leaves the
        // view's gates alone rather than substituting the cached ones. This has been
        // reintroduced once already: the merged value is for the cache, never for the view.
        if run_confirmed {
            if v.stale || !pipeline_detail_unchanged(v, &detail, approvals.as_deref()) {
                v.supports_approvals = detail.supports_approvals;
                v.can_respond_approvals = detail.can_respond_approvals;
                v.apply_fresh_run(detail.run, approvals);
            }
        } else if !pipeline_detail_unchanged(v, &detail, approvals.as_deref()) {
            // The run itself wasn't reconfirmed this round (only approvals/capabilities
            // answered) — patch those in place, but leave the run and `stale` exactly as they
            // are: only a real `get_run` may confirm the run is actually live.
            v.apply_confirmed_approvals(approvals);
            v.supports_approvals = detail.supports_approvals;
            v.can_respond_approvals = detail.can_respond_approvals;
        }
    }

    /// Kicks off the background fetch for a just-opened pipeline view's detail, without blocking
    /// the render loop. Mirrors [`App::request_pr_detail`].
    fn request_pipeline_detail(&self, deps: &AppDeps, conn_id: String, run: ItemRef, key: String) {
        let Some(tx) = self.job_tx.clone() else {
            return;
        };
        let deps = deps.clone();
        let fetched_at = Utc::now();
        tokio::spawn(async move {
            let detail = fetch_pipeline_detail(&deps, &conn_id, &run).await;
            let _ = tx.send(AppEvent::PipelineDetailLoaded { key, detail: Box::new(detail), fetched_at });
        });
    }

    /// Refetches the pool and repaints every derived PR view. Used after a mutation the
    /// provider has already accepted, where the rows on screen are known to be behind.
    ///
    /// A failed fetch leaves the existing pool alone rather than blanking the lists — the rows
    /// are stale, not wrong, and the error is surfaced separately.
    async fn reload_pr_pool(&mut self, deps: &AppDeps, errors: &mut Vec<String>) {
        let (pool, ok) = fetch_pr_pool(deps, errors).await;
        if ok {
            self.pr_pool = pool;
        }
        self.refresh_derived_prs(deps);
    }

    /// The rows one PR view shows, derived from the pool and decorated from what we hold.
    fn derive_pr_rows(&self, filter: PullRequestFilter, completed: bool) -> Vec<PrRow> {
        let mut rows = derive_pool_rows(&self.pr_pool, filter, completed);
        for row in &mut rows {
            if let Some(d) = self.pr_decorations.get(&(row.connection_id.clone(), row.pr.id.clone())) {
                d.apply_to(&mut row.pr);
            }
        }
        rows
    }

    /// Repoints every PR-derived list at the current pool, and asks for the decoration the
    /// visible view is missing. Cheap and synchronous — call it after anything that changes
    /// which rows a view should show.
    fn refresh_derived_prs(&mut self, deps: &AppDeps) {
        self.prs = self.derive_pr_rows(self.pr_filter, self.pr_wants_completed());
        // Until a fetch has actually landed there is nothing to derive the Launchpad from, and
        // deriving anyway would blank the rows `seed_from_cache` painted — the blank landing
        // screen the seeding exists to prevent. The list above is still cleared, because an
        // empty list under the new heading is honest where carrying the old view's rows is not.
        if self.pr_pool_loaded {
            self.lp_prs_mine = self.derive_pr_rows(PullRequestFilter::Mine, true);
            self.lp_prs_review = self.derive_pr_rows(PullRequestFilter::ReviewRequested, false);
            self.rebuild_launchpad();
        }
        self.request_pr_decorations(deps);
    }

    /// Spawns decoration fetches for the rows on screen that are missing it.
    ///
    /// Only for connections whose list endpoint omits those fields — for everyone but GitHub
    /// the list payload already carried them, and asking again is a call per row that returns
    /// what we hold. Bounded by [`PR_DECORATE_CAP`], the same ceiling GitHub's `list` applied
    /// internally, so the round-trip count per connection is what it always was.
    fn request_pr_decorations(&mut self, deps: &AppDeps) {
        let Some(tx) = self.job_tx.clone() else {
            return;
        };
        let wanted: Vec<(String, ItemRef, (String, String))> = self
            .prs
            .iter()
            .filter(|row| self.pr_pool.needs_decoration.get(&row.connection_id).copied().unwrap_or(false))
            .map(|row| (row.connection_id.clone(), row.pr.item_ref(), (row.connection_id.clone(), row.pr.id.clone())))
            .filter(|(_, _, key)| {
                !self.pr_decorations.contains_key(key)
                    && !self.pr_decor_inflight.contains(key)
                    && !self.pr_decor_failed.contains(key)
            })
            .take(PR_DECORATE_CAP)
            .collect();
        if wanted.is_empty() {
            return;
        }
        for (_, _, key) in &wanted {
            self.pr_decor_inflight.insert(key.clone());
        }
        let deps = deps.clone();
        tokio::spawn(async move {
            // Resolved once for the whole batch: the feed list is the same for every row, and
            // re-resolving it per PR would cost more than the decoration calls themselves.
            let feeds = detail_or_default(deps.sections.pull_request_feeds().await, DIAG_PR_FEEDS);
            let mut out = Vec::with_capacity(wanted.len());
            for (conn_id, item, key) in wanted {
                let decoration = match feeds.iter().find(|f| f.connection.connection_id() == conn_id) {
                    Some(feed) => feed.source.decorate(&item).await.ok(),
                    None => None,
                };
                out.push((key, decoration));
            }
            let _ = tx.send(AppEvent::PrDecorationsLoaded { items: out });
        });
    }

    /// Folds finished decoration fetches into the held map and repaints the derived views.
    ///
    /// A fetch that failed still clears its in-flight mark but stores nothing, so the row keeps
    /// showing undecorated columns and the next refresh may try again — better than caching a
    /// blank decoration, which would read as "this PR changes no files".
    fn apply_pr_decorations(&mut self, items: Vec<((String, String), Option<PrDecoration>)>, deps: &AppDeps) {
        for (key, decoration) in items {
            self.pr_decor_inflight.remove(&key);
            match decoration {
                Some(d) => {
                    self.pr_decorations.insert(key, d);
                }
                // Not cached as a blank decoration — that would read as "this PR changes no
                // files". Marked instead, so the next reload retries it and this one does not.
                None => {
                    self.pr_decor_failed.insert(key);
                }
            }
        }
        self.refresh_derived_prs(deps);
    }

    async fn reload_work_items(&mut self, deps: &AppDeps, errors: &mut Vec<String>) {
        let before = errors.len();
        let mut rows = Vec::new();
        match deps.sections.work_item_feeds().await {
            Ok(feeds) => {
                for feed in feeds {
                    let (provider, name, conn_id) = feed_tag(&feed.connection);
                    match feed.source.list(&wi_query()).await {
                        Ok(list) => rows.extend(list.into_iter().map(|wi| WiRow {
                            connection_id: conn_id.clone(),
                            connection: name.clone(),
                            provider,
                            wi,
                        })),
                        Err(e) => push_reload_error(
                            errors,
                            format!("Work items ({name}): {e}"),
                            DIAG_RELOAD_WORK_ITEMS,
                        ),
                    }
                }
            }
            Err(e) => push_reload_error(
                errors,
                format!("Work items: {e}"),
                DIAG_RELOAD_WORK_ITEMS,
            ),
        }
        take_inline_section(&mut self.wis, rows, errors, before);
    }

    async fn reload_pipelines(&mut self, deps: &AppDeps, errors: &mut Vec<String>) {
        let before = errors.len();
        let mut rows = Vec::new();
        match deps.sections.pipeline_feeds().await {
            Ok(feeds) => {
                for feed in feeds {
                    let provider = feed.connection.provider_type();
                    let name = feed.connection.display_name().to_string();
                    let conn_id = feed.connection.connection_id().to_string();
                    // Map definition_id → pipeline name so rows can show the pipeline
                    // (e.g. "CI Build") separately from the run/release (e.g. "10.1.100").
                    let defs = detail_or_default(feed.source.discover().await, DIAG_PIPELINE_DISCOVERY);
                    let def_names: HashMap<String, String> =
                        defs.iter().map(|d| (d.id.clone(), d.name.clone())).collect();
                    for q in feed_queries(&feed.subscription, &defs) {
                        match feed.source.list_runs(&q).await {
                            Ok(runs) => {
                                let supports = feed.source.supports_approvals();
                                for run in runs {
                                    // Only in-flight runs can be waiting on a gate — bound the
                                    // extra per-run approval calls to those.
                                    let awaiting_approval = supports
                                        && is_active(run.status)
                                        && detail_or_default(
                                            feed.source.pending_approvals(&run.item_ref()).await,
                                            DIAG_PIPELINE_APPROVALS,
                                        )
                                        .iter()
                                        .any(|approval| approval.can_respond);
                                    let definition_name = def_names.get(&run.definition_id).cloned();
                                    rows.push(PipeRow {
                                        connection_id: conn_id.clone(),
                                        connection: name.clone(),
                                        provider,
                                        run,
                                        definition_name,
                                        awaiting_approval,
                                    });
                                }
                            }
                            Err(e) => push_reload_error(
                                errors,
                                format!("Pipelines ({name}): {e}"),
                                DIAG_RELOAD_PIPELINES,
                            ),
                        }
                    }
                }
            }
            Err(e) => push_reload_error(
                errors,
                format!("Pipelines: {e}"),
                DIAG_RELOAD_PIPELINES,
            ),
        }
        take_inline_section(&mut self.pipes, rows, errors, before);
        // Both read `self.pipes`, so they have to run on the swapped-in rows, not the old ones.
        self.notify_pipeline_failures();
        self.notify_pending_approvals();
    }

    /// Fires a desktop notification when a run first starts awaiting the user's
    /// approval. Seeded silently on the first load and de-duped per (connection, run).
    fn notify_pending_approvals(&mut self) {
        if self.notifications.pipeline_approval_needed && self.approval_seeded {
            for row in new_pending_approvals(&self.approval_seen, &self.pipes) {
                self.notifier.notify("Approval needed", &format!("{} · {} is awaiting your approval", row.connection, pipe_label(row)));
            }
        }
        self.approval_seen =
            self.pipes.iter().filter(|r| r.awaiting_approval).map(|r| (r.connection_id.clone(), r.run.id.clone())).collect();
        self.approval_seeded = true;
    }

    /// Fires a desktop notification for any run that has just entered a failed
    /// state since the last refresh. Seeded silently on the first load.
    fn notify_pipeline_failures(&mut self) {
        if self.notifications.pipeline_failed && self.pipe_seeded {
            for row in new_pipeline_failures(&self.pipe_seen, &self.pipes) {
                let branch = row.run.branch.clone().unwrap_or_else(|| "—".into());
                self.notifier.notify("Pipeline failed", &format!("{} · {} on {branch}", row.connection, pipe_label(row)));
            }
        }
        self.pipe_seen = self.pipes.iter().map(|r| (r.run.id.clone(), r.run.status)).collect();
        self.pipe_seeded = true;
    }

    /// Re-selects a valid row per tab after the underlying data (or filter) changed.
    /// Selection is a position within each tab's *filtered* view, so clamp to that.
    fn fix_selection(&mut self) {
        let (pl, wl, ll) = (self.filtered_len(0), self.filtered_len(1), self.filtered_len(2));
        self.pr_state.select((pl > 0).then(|| self.pr_state.selected().unwrap_or(0).min(pl - 1)));
        self.wi_state.select((wl > 0).then(|| self.wi_state.selected().unwrap_or(0).min(wl - 1)));
        self.pipe_state.select((ll > 0).then(|| self.pipe_state.selected().unwrap_or(0).min(ll - 1)));
    }

    // ---- key handling ----

    /// Applies a key. `deps` is used for async refresh / actions / theme persistence.
    pub async fn on_key(&mut self, key: Key, deps: &AppDeps) {
        // A mouse event either acts directly (selecting a row) or stands in for a key (a second
        // click on the selected row is Enter), which then takes the ordinary key path.
        let (key, times) = if key.is_mouse() { self.on_mouse(key).unwrap_or((Key::None, 0)) } else { (key, 1) };
        for _ in 0..times {
            self.on_key_inner(key, deps).await;
        }
        self.settle_preview(deps);
        self.drive_logs(deps);
    }

    /// Resolves a mouse event against the last frame's hit map. Returns the key it stands for
    /// and how many times to press it, or `None` when it was handled here (or means nothing).
    ///
    /// Clicks act only on the main screen: the wizard, an overlay and the quick filter keep
    /// their keyboard-only input, so a stray click can't pick an option or confirm a prompt.
    /// The wheel still scrolls an overlay (it's Up / Down there), but never the wizard or filter.
    fn on_mouse(&mut self, key: Key) -> Option<(Key, usize)> {
        let (Key::Click(x, y) | Key::ScrollUp(x, y) | Key::ScrollDown(x, y)) = key else { return None };
        if self.wizard.is_some() || self.filtering {
            return None;
        }
        let at = ratatui::layout::Position { x, y };
        let hit = self.hits.iter().rev().find(|(r, _)| r.contains(at)).map(|(_, h)| *h);
        match key {
            Key::Click(..) if self.overlay.is_none() => {
                self.toast = None;
                self.on_click(hit?)
            }
            Key::ScrollUp(..) => self.on_wheel(hit, -1),
            Key::ScrollDown(..) => self.on_wheel(hit, 1),
            _ => None,
        }
    }

    /// A click on `hit`. The first click on a row selects it; a click on the row that is
    /// already selected opens it, as Enter would. On a diff line, the second click comments.
    fn on_click(&mut self, hit: Hit) -> Option<(Key, usize)> {
        let enter = Some((Key::Enter, 1));
        match hit {
            Hit::Tab(pos) => {
                // Same guard as Tab: don't walk out from under unsubmitted line comments.
                if matches!(&self.screen, Screen::PrView(v) if !v.pending.is_empty()) {
                    self.open_pending_exit_prompt();
                } else {
                    self.go_to_tab(pos);
                }
            }
            Hit::PrTab(tab) => {
                if let Screen::PrView(v) = &mut self.screen {
                    if v.tab != tab {
                        v.tab = tab;
                        v.scroll = 0;
                        v.reset_diff_scope();
                    }
                }
            }
            Hit::ListRow(i) => {
                if self.selected() == Some(i) {
                    return enter;
                }
                self.active_state().select(Some(i));
                self.ensure_visible();
            }
            Hit::LpColumn(side) => {
                if self.lp_side != side {
                    self.lp_side = side;
                    self.anim = 0;
                }
            }
            Hit::LpRow { side, pos } => {
                if self.lp_side == side && self.lp_sel[side] == pos {
                    return enter;
                }
                self.lp_side = side;
                self.lp_sel[side] = pos;
                self.anim = 0;
            }
            Hit::InboxRow(i) => {
                if self.inbox_sel == i {
                    return enter;
                }
                self.inbox_sel = i;
            }
            Hit::CommitRow(i) => {
                let Screen::PrView(v) = &mut self.screen else { return None };
                if v.commit_sel == i {
                    return enter;
                }
                v.commit_sel = i;
            }
            Hit::DiffFile(i) => {
                let Screen::PrView(v) = &mut self.screen else { return None };
                if v.diff.focus == DiffFocus::FileList && v.diff.selected == i {
                    return enter;
                }
                v.diff.focus = DiffFocus::FileList;
                if v.diff.selected != i {
                    v.diff.selected = i;
                    v.diff.scroll = 0;
                    v.diff.cursor = 0;
                }
            }
            Hit::DiffLine(i) => {
                let Screen::PrView(v) = &mut self.screen else { return None };
                if v.diff.focus == DiffFocus::Patch && v.diff.cursor == i {
                    return Some((Key::Char('c'), 1));
                }
                v.diff.focus = DiffFocus::Patch;
                v.diff.cursor = i;
            }
            Hit::DiffPatch => {}
            Hit::PipeNode(i) => {
                let Screen::Pipeline(v) = &mut self.screen else { return None };
                v.log_focus = false;
                if v.selected == i {
                    return enter;
                }
                v.selected = i;
                v.user_moved = true;
                if v.logs.is_some() {
                    v.open_logs_for_selection();
                }
            }
            Hit::LogPane => {
                if let Screen::Pipeline(v) = &mut self.screen {
                    v.log_focus = true;
                }
            }
        }
        None
    }

    /// One wheel notch (`dir` -1 up, 1 down) over `hit`. The pane under the pointer takes the
    /// wheel: a Command Center column or the pipeline tree / log pane gains focus first. Lists
    /// move their selection one row and stop at the ends (Up / Down wrap, which reads as a jump
    /// under a wheel); scrolling text moves three lines a notch.
    fn on_wheel(&mut self, hit: Option<Hit>, dir: isize) -> Option<(Key, usize)> {
        let key = if dir < 0 { Key::Up } else { Key::Down };
        if self.overlay.is_some() {
            return Some((key, 1));
        }
        let step = |sel: usize, len: usize| (sel as isize + dir).clamp(0, len.saturating_sub(1) as isize) as usize;
        match &mut self.screen {
            Screen::Launchpad => {
                if let Some(Hit::LpColumn(side) | Hit::LpRow { side, .. }) = hit {
                    self.lp_side = side;
                }
                Some((key, 1))
            }
            Screen::List if !self.preview_focus => {
                let len = self.active_len();
                if len > 0 {
                    let next = step(self.selected_index(), len);
                    self.active_state().select(Some(next));
                    self.ensure_visible();
                }
                None
            }
            Screen::Inbox => {
                self.inbox_sel = step(self.inbox_sel, self.inbox.len());
                None
            }
            Screen::Pipeline(v) => {
                if v.logs.is_some() && v.log_split.get() {
                    match hit {
                        Some(Hit::LogPane) => v.log_focus = true,
                        Some(Hit::PipeNode(_)) => v.log_focus = false,
                        _ => {}
                    }
                }
                if v.logs_have_keys() {
                    return Some((key, 3));
                }
                let len = v.flatten().len();
                if len > 0 {
                    v.selected = step(v.selected, len);
                    v.user_moved = true;
                    if v.logs.is_some() {
                        v.open_logs_for_selection();
                    }
                }
                None
            }
            Screen::PrView(v) => {
                // Over the patch while the file list has the keys, the wheel scrolls the patch
                // rather than changing file.
                if v.tab == 3 && v.diff.focus == DiffFocus::FileList && matches!(hit, Some(Hit::DiffPatch | Hit::DiffLine(_))) {
                    v.diff.scroll_by(3 * dir as i32);
                    return None;
                }
                Some((key, if matches!(v.tab, 0 | 2) { 3 } else { 1 }))
            }
            Screen::WiView(_) => Some((key, 3)),
            _ => Some((key, 1)),
        }
    }

    async fn on_key_inner(&mut self, key: Key, deps: &AppDeps) {
        // Ctrl-C hard-quits from any mode.
        if key == Key::Quit {
            self.should_quit = true;
            return;
        }
        // A resize just needs the loop to redraw at the new size — nothing else.
        if key == Key::Redraw {
            return;
        }
        // Any keypress dismisses the previous one-shot toast.
        self.toast = None;

        // The wizard, then any overlay, swallow all input until they resolve.
        if self.wizard.is_some() {
            self.on_wizard_key(key, deps).await;
            return;
        }
        if self.overlay.is_some() {
            self.on_overlay_key(key, deps).await;
            return;
        }
        // The quick-filter input (only ever open on the list) captures every key.
        if self.filtering {
            self.on_filter_key(key);
            return;
        }
        // So does the log pane's `/` prompt; and with the pane holding the keys, its `n` / `N`
        // (next / previous match) win over the global add-connection / notifications keys.
        if let Screen::Pipeline(v) = &self.screen {
            let searching = v.logs.as_ref().is_some_and(|l| l.search_input.is_some());
            if searching || (v.logs_have_keys() && matches!(key, Key::Char('n' | 'N'))) {
                self.on_pipeline_logs_key(key);
                return;
            }
        }
        // Ctrl-K opens the command palette from every screen; Ctrl-P is its alias. Checked
        // after the input-capturing modes above, so it never steals a key from them.
        if matches!(key, Key::Ctrl('k') | Key::Ctrl('p')) {
            self.open_palette();
            return;
        }
        // Help and the notifications chooser are available anywhere.
        if key == Key::Char('?') {
            self.overlay = Some(Overlay::Help { scroll: 0 });
            return;
        }
        if key == Key::Char('N') {
            self.open_notifications_toggle();
            return;
        }
        // `i` opens the notification inbox from the list screens and the Launchpad.
        if key == Key::Char('i') && matches!(self.screen, Screen::List | Screen::Launchpad) {
            self.open_inbox();
            return;
        }
        // `B` opens the web dashboard in the browser (available from anywhere).
        if key == Key::Char('B') {
            self.open_dashboard();
            return;
        }
        // `F` opens the GitHub feedback issue form (available from anywhere). Input-capturing
        // modes above retain priority so an uppercase F can still be typed into them.
        if key == Key::Char('F') {
            self.open_feedback();
            return;
        }
        // `n` adds a connection, from anywhere. The first-run hint has always named this key,
        // and it appears on the Launchpad as well as in empty list sections — and Launchpad
        // returns before `on_char`, so a per-screen arm would only answer on some of them.
        // Overlays and the quick filter are handled above, so the confirm prompt's `n` ("no")
        // and typing `n` into a filter both still win.
        if key == Key::Char('n') {
            self.open_setup_picker();
            return;
        }

        // Tab always walks the tab strip — Command Center, then each visible section — from
        // any screen, an open PR / work item / pipeline run included; Shift-Tab walks it
        // backwards. Both wrap. The input-capturing modes (wizard, overlay, quick filter)
        // return above, so Tab still reaches them.
        if matches!(key, Key::Tab | Key::BackTab) {
            // Don't walk out from under unsubmitted line comments — same prompt as Esc.
            if matches!(&self.screen, Screen::PrView(v) if !v.pending.is_empty()) {
                self.open_pending_exit_prompt();
            } else {
                self.switch_tab(if key == Key::Tab { 1 } else { -1 });
            }
            return;
        }

        if self.on_preview_key(key, deps).await {
            return;
        }

        // Full-screen sub-views handle their own keys.
        match self.screen {
            Screen::Pipeline(_) => {
                self.on_pipeline_screen_key(key);
                return;
            }
            Screen::Config(_) => {
                self.on_config_key(key, deps).await;
                return;
            }
            Screen::PrView(_) => {
                // Enter on the Commits tab drills into that commit's diff (needs async).
                if key == Key::Enter {
                    if let Screen::PrView(v) = &self.screen {
                        if v.tab == 1 {
                            self.open_commit_diff(deps).await;
                            return;
                        }
                    }
                }
                self.on_pr_view_key(key);
                return;
            }
            Screen::WiView(_) => {
                // `u` (update state) pulls the provider's states — needs async.
                if key == Key::Char('u') {
                    self.open_wi_state(deps).await;
                    return;
                }
                // `@` (assign) pulls the provider's assignable users — needs async too.
                if key == Key::Char('@') {
                    self.open_wi_assign(deps).await;
                    return;
                }
                self.on_wi_view_key(key);
                return;
            }
            Screen::Launchpad => {
                self.on_launchpad_key(key, deps).await;
                return;
            }
            Screen::Inbox => {
                self.on_inbox_key(key, deps).await;
                return;
            }
            Screen::List => {}
        }

        match key {
            Key::Escape => self.leave_section_list(),
            // The top nav moves only on Tab (or a number key); the arrows switch nothing here.
            Key::Left | Key::Right => {}
            Key::Up => {
                self.move_up();
                self.ensure_visible();
            }
            Key::Down => {
                self.move_down();
                self.ensure_visible();
            }
            Key::PageDown => self.list_scroll = self.list_scroll.saturating_add(8),
            Key::PageUp => self.list_scroll = self.list_scroll.saturating_sub(8),
            Key::Enter => {
                self.lp_origin = false; // opened from the section list, so Esc returns there
                self.from_inbox = false;
                match self.active {
                    0 => self.open_pr_view(deps, 0),
                    1 => self.open_wi_view(deps),
                    2 => self.enter_pipeline_line(deps),
                    _ => {}
                }
            }
            Key::Char(c) => self.on_char(c, deps).await,
            // Tab and Shift-Tab are answered globally, before any screen sees them.
            Key::Tab | Key::BackTab | Key::Backspace | Key::Ctrl(_) | Key::Quit | Key::Redraw | Key::Home | Key::End | Key::None => {}
            Key::Click(..) | Key::ScrollUp(..) | Key::ScrollDown(..) => {}
        }
    }

    fn selected_index(&self) -> usize {
        self.selected().unwrap_or(0)
    }

    /// Keeps the selected row within the visible list viewport.
    fn ensure_visible(&mut self) {
        let sel = self.selected_index() as u16;
        let h = self.content_h.max(1);
        if sel < self.list_scroll {
            self.list_scroll = sel;
        } else if sel >= self.list_scroll + h {
            self.list_scroll = sel - h + 1;
        }
    }

    // ---- full-screen PR / work-item views ----

    fn open_pr_view(&mut self, deps: &AppDeps, tab: usize) {
        let (label, url, conn_id, pr) = match self.selected_pr_row() {
            Some(row) => (pr_label(&row.pr), row.pr.url.clone(), row.connection_id.clone(), row.pr.clone()),
            None => return,
        };
        self.open_pr_view_for(deps, tab, label, url, conn_id, pr);
    }

    /// Opens the PR view for an explicit PR (used by the Launchpad, where the item
    /// isn't the section list's selected row).
    ///
    /// Synchronous and does no I/O: it paints whatever is cached (or nothing) immediately, so
    /// the view is on screen before the first byte of the detail fetch goes out, then hands the
    /// four detail calls off to [`request_pr_detail`] to run off the render loop. Awaiting them
    /// here — as this used to — froze the whole event loop (no redraw, no spinner, no key input)
    /// for as long as the network took (see commit 8fc2117 for the same fix on the refresh path).
    #[allow(clippy::too_many_arguments)]
    fn open_pr_view_for(&mut self, deps: &AppDeps, tab: usize, label: String, url: Option<String>, conn_id: String, pr: PullRequest) {
        let (view, fetch) = Self::build_pr_view(deps, tab, label, url, conn_id, pr);
        self.screen = view;
        // The view is on screen now; the fetch that keeps it fresh runs in the background and
        // patches it in place via `AppEvent::PrDetailLoaded` (see `apply_pr_detail`).
        self.send_detail_request(deps, fetch);
    }

    /// Builds the PR view from the row plus whatever the cache holds, without any I/O. The
    /// returned request is what keeps it fresh; [`open_pr_view_for`] sends it at once, the
    /// preview pane only once the cursor has settled.
    fn build_pr_view(deps: &AppDeps, tab: usize, label: String, url: Option<String>, conn_id: String, pr: PullRequest) -> (Screen, DetailRequest) {
        // Address the cache and every detail call at the PR's own repository, not just its id:
        // on a connection spanning several, `#7` alone names more than one pull request.
        let item = pr.item_ref();
        let key = pr_detail_cache_key(&conn_id, &item);
        let PrDetail { threads, mut files, checks, commits, timeline } = match deps.cache.get::<PrDetail>(&key) {
            Some(entry) => entry.value,
            None => PrDetail { threads: Vec::new(), files: Vec::new(), checks: Vec::new(), commits: Vec::new(), timeline: Vec::new() },
        };
        // Sort on the way out of the cache too, rather than trusting the order it was written
        // in: an entry from before this sort existed (or a future change that stops sorting
        // before caching) must not silently show the file list out of order.
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let diff = DiffView {
            pr_label: label.clone(),
            url: url.clone(),
            files: files.clone(),
            threads,
            selected: 0,
            scroll: 0,
            focus: DiffFocus::FileList,
            cursor: 0,
            commit_label: None,
            viewed: HashSet::new(),
        };
        let view = Screen::PrView(Box::new(PrView {
            label,
            url,
            connection_id: conn_id.clone(),
            pr,
            tab,
            checks,
            commits,
            commit_sel: 0,
            pr_files: files,
            scroll: 0,
            diff,
            pending: Vec::new(),
            review_draft: None,
            reply_target: None,
            timeline,
        }));
        (view, DetailRequest::Pr { conn_id, item, key })
    }

    /// Buffers a line comment against the cursor line in the diff patch.
    fn open_line_comment(&mut self) {
        // Read the target under an immutable borrow, then mutate.
        let target = {
            let Screen::PrView(v) = &self.screen else { return };
            if v.tab != 3 || v.diff.focus != DiffFocus::Patch {
                self.open_pr_comment();
                return;
            }
            let Some(file) = v.diff.current() else { return };
            let Some(patch) = file.patch.as_deref() else { return };
            crate::diff::comment_target(patch, v.diff.cursor).map(|(line, side)| (file.path.clone(), line, side))
        };
        match target {
            Some((path, line, side)) => {
                let title = format!("Comment on {path}:{line}");
                if let Screen::PrView(v) = &mut self.screen {
                    v.review_draft = Some(DraftComment { path, line, side });
                }
                self.overlay = Some(Overlay::Input { title, buffer: String::new(), kind: InputKind::PrLineComment });
            }
            None => self.toast = Some("Move to a code line to comment (not a hunk header)".into()),
        }
    }

    /// Opens a reply to an existing thread: the one under the diff cursor (Diff tab), or the sole
    /// conversation thread (Conversation tab). Stashes its id on `reply_target` for the submit.
    fn open_thread_reply(&mut self) {
        // Resolve the target id (or an error message) under an immutable borrow, then mutate.
        let target: Result<String, &'static str> = {
            let Screen::PrView(v) = &self.screen else { return };
            match v.tab {
                3 => v
                    .diff
                    .thread_at_cursor()
                    .map(|t| t.id.clone())
                    .ok_or("Move onto a comment thread first (] / [ to jump), then r to reply"),
                0 => {
                    let general: Vec<&CommentThread> = v.diff.threads.iter().filter(|t| t.file_path.is_none()).collect();
                    match general.as_slice() {
                        [t] => Ok(t.id.clone()),
                        [] => Err("No comment thread to reply to — press c to add a comment"),
                        _ => Err("Multiple threads — reply from the Diff tab (] / [ to a thread, then r)"),
                    }
                }
                _ => Err("Switch to the Conversation or Diff tab to reply to a comment"),
            }
        };
        match target {
            Ok(thread_id) => {
                if let Screen::PrView(v) = &mut self.screen {
                    v.reply_target = Some(thread_id);
                }
                self.overlay = Some(Overlay::Input { title: "Reply to thread".into(), buffer: String::new(), kind: InputKind::PrThreadReply });
            }
            Err(msg) => self.toast = Some(msg.into()),
        }
    }

    /// On Esc with unsubmitted line comments, ask whether to submit or leave.
    fn open_pending_exit_prompt(&mut self) {
        let n = match &self.screen {
            Screen::PrView(v) => v.pending.len(),
            _ => return,
        };
        let noun = if n == 1 { "comment" } else { "comments" };
        self.overlay = Some(Overlay::Picker {
            title: format!("{n} unsubmitted {noun}"),
            items: vec!["Submit review…".into(), "Leave without submitting".into()],
            selected: 0,
            kind: PickerKind::PendingExit,
        });
    }

    /// Opens the submit-review verdict picker if there are pending comments.
    fn open_review_submit(&mut self) {
        let has_pending = matches!(&self.screen, Screen::PrView(v) if !v.pending.is_empty());
        if !has_pending {
            self.toast = Some("No pending comments — press c on a diff line to add one".into());
            return;
        }
        self.overlay = Some(Overlay::Picker {
            title: "Submit review".into(),
            items: vec!["Comment".into(), "Approve".into(), "Request changes".into()],
            selected: 0,
            kind: PickerKind::ReviewSubmit,
        });
    }

    /// Buffers a typed line comment against the stashed draft target.
    fn add_line_comment(&mut self, body: String) {
        let msg = if let Screen::PrView(v) = &mut self.screen {
            match v.review_draft.take() {
                Some(d) if !body.trim().is_empty() => {
                    v.pending.push(LineComment { path: d.path, line: d.line, side: d.side, body });
                    Some(format!("Comment buffered — {} pending (s to submit)", v.pending.len()))
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(m) = msg {
            self.toast = Some(m);
        }
    }

    /// Submits the buffered line comments as one review with `event`.
    async fn submit_review(&mut self, event: ReviewVote, deps: &AppDeps) {
        let (item, comments, conn_id) = match &self.screen {
            Screen::PrView(v) => (v.pr.item_ref(), v.pending.clone(), v.connection_id.clone()),
            _ => return,
        };
        let pr_id = item.id.clone();
        if comments.is_empty() {
            return;
        }
        let source = match self.pr_source_for(&conn_id, deps).await {
            Some(s) => s,
            None => {
                self.toast = Some("No pull-request provider is bound".into());
                return;
            }
        };
        match source.submit_review(&item, event, &comments).await {
            Ok(()) => {
                let threads = detail_or_default(source.threads(&item).await, DIAG_PR_THREADS);
                if let Screen::PrView(v) = &mut self.screen {
                    v.pending.clear();
                    v.review_draft = None;
                    v.diff.threads = threads;
                }
                // You've reviewed it — it no longer needs you on the Launchpad.
                self.dismiss_from_launchpad(&conn_id, &pr_id);
                self.toast = Some(format!("Review submitted ({} comment(s))", comments.len()));
            }
            Err(e) => self.toast_error(format!("Submit failed: {e}")),
        }
    }

    /// Loads the selected commit's diff into the diff view and jumps to the Diff tab.
    async fn open_commit_diff(&mut self, deps: &AppDeps) {
        let Screen::PrView(v) = &self.screen else { return };
        let Some(commit) = v.commits.get(v.commit_sel) else { return };
        let (sha, msg) = (commit.sha.clone(), commit.message.clone());
        let item = v.pr.item_ref();
        let conn_id = v.connection_id.clone();

        let source = match self.pr_source_for(&conn_id, deps).await {
            Some(s) => s,
            None => {
                self.toast = Some("No pull-request provider is bound".into());
                return;
            }
        };
        let mut files = detail_or_default(
            source.commit_changes(&item, &sha).await,
            DIAG_PR_COMMIT_CHANGES,
        );
        if files.is_empty() {
            self.toast = Some("No per-commit diff for this provider".into());
            return;
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let short: String = sha.chars().take(7).collect();
        let title: String = msg.chars().take(50).collect();
        let Screen::PrView(v) = &mut self.screen else { return };
        v.diff.files = files;
        v.diff.selected = 0;
        v.diff.cursor = 0;
        v.diff.focus = DiffFocus::FileList;
        v.diff.commit_label = Some(format!("{short} {title}"));
        v.tab = 3;
    }

    fn open_wi_view(&mut self, deps: &AppDeps) {
        let (conn_id, wi) = match self.selected_wi_row() {
            Some(row) => (row.connection_id.clone(), row.wi.clone()),
            None => return,
        };
        self.open_wi_view_for(deps, conn_id, wi);
    }

    /// Opens the work-item view for an explicit item (used by the Launchpad).
    ///
    /// Synchronous and does no I/O: mirrors [`App::open_pr_view_for`] — paints whatever is
    /// cached (or nothing) immediately, so the view is on screen before the first byte of the
    /// detail fetch goes out, then hands the thread fetch off to [`App::request_wi_detail`] to
    /// run off the render loop. Awaiting it here — as this used to — froze the whole event loop
    /// for as long as the network took.
    fn open_wi_view_for(&mut self, deps: &AppDeps, conn_id: String, wi: WorkItem) {
        let (view, fetch) = Self::build_wi_view(deps, conn_id, wi);
        self.screen = view;
        self.send_detail_request(deps, fetch);
    }

    /// Mirrors [`App::build_pr_view`]: the view from the row and the cache, no I/O.
    fn build_wi_view(deps: &AppDeps, conn_id: String, wi: WorkItem) -> (Screen, DetailRequest) {
        let item = wi.item_ref();
        let key = wi_detail_cache_key(&conn_id, &item);
        let WiDetail { threads, timeline } = match deps.cache.get::<WiDetail>(&key) {
            Some(entry) => entry.value,
            None => WiDetail { threads: Vec::new(), timeline: Vec::new() },
        };
        let view = Screen::WiView(Box::new(WiView { connection_id: conn_id.clone(), wi, threads, timeline, scroll: 0 }));
        (view, DetailRequest::Wi { conn_id, item, key })
    }

    /// Opens the repository-scope picker for the active section.
    ///
    /// The scope is per connection, so with more than one bound the picker is opened for the
    /// first repo-addressed one; the rest are reachable by switching connection filter. Discovery
    /// is refreshed here rather than on every 30s reload — this is the moment the candidate list
    /// has to be right.
    async fn open_repo_scope(&mut self, deps: &AppDeps) {
        let Some(summary) = self.repo_scope[self.active].clone() else {
            self.toast = Some("This section's connections aren't repository-scoped".into());
            return;
        };
        // The scope is per connection, so with more than one bound the user has to say which —
        // picking the first silently would leave the others' scopes unreachable from the TUI.
        match summary.connections.as_slice() {
            [] => {}
            [only] => self.open_repo_scope_for(only.clone(), deps).await,
            many => {
                let cfg = deps.config.snapshot();
                let items = many
                    .iter()
                    .map(|id| cfg.find_connection(id).map(|c| c.display_name.clone()).unwrap_or_else(|| id.clone()))
                    .collect();
                self.repo_scope_choices = many.to_vec();
                self.overlay = Some(Overlay::Picker {
                    title: "Repositories · which connection?".into(),
                    items,
                    selected: 0,
                    kind: PickerKind::RepoScopeConnection,
                });
            }
        }
    }

    /// Opens the repository-scope checklist for one connection.
    async fn open_repo_scope_for(&mut self, connection_id: String, deps: &AppDeps) {
        let label = deps
            .config
            .snapshot()
            .find_connection(&connection_id)
            .map(|c| c.display_name.clone())
            .unwrap_or_else(|| connection_id.clone());

        self.toast = Some("Loading repositories…".into());
        if let Ok(page) = deps.sections.discover_repositories(&connection_id).await {
            self.repo_catalog.insert(connection_id.clone(), page);
        }
        let chosen = self.repo_scope_of(deps, &connection_id);
        let discovered = self.repo_catalog.get(&connection_id).cloned().unwrap_or_default();

        // Anything already chosen stays listed even if discovery didn't return it, so a saved
        // scope is never silently dropped by a provider whose discovery is incomplete.
        let mut repos = chosen.clone();
        repos.extend(discovered.repositories.iter().filter(|r| !chosen.contains(r)).cloned());
        if repos.is_empty() {
            self.toast = Some("No repositories found for this connection's credentials".into());
            return;
        }
        let items = repos
            .into_iter()
            .map(|r| ToggleItem { on: chosen.contains(&r), id: r.clone(), label: r })
            .collect();
        self.toast = None;
        self.overlay = Some(Overlay::Toggle {
            title: format!("Repositories · {label}"),
            kind: ToggleKind::RepoScope { connection_id },
            // Ticking none is a real choice — fetch nothing — not a state to be prevented.
            min_one: false,
            items,
            selected: 0,
            filter: Some(String::new()),
        });
    }

    /// The repositories a connection currently fetches from, respecting an explicitly emptied
    /// scope and only falling back to the legacy single repository when none was ever chosen.
    fn repo_scope_of(&self, deps: &AppDeps, connection_id: &str) -> Vec<String> {
        let cfg = deps.config.snapshot();
        match cfg.find_connection(connection_id) {
            Some(c) => c.repo_scope.clone().unwrap_or_else(|| c.repository.clone().into_iter().collect()),
            None => Vec::new(),
        }
    }

    async fn apply_repo_scope(&mut self, connection_id: &str, ids: Vec<String>, deps: &AppDeps) {
        let count = ids.len();
        let scope = ids.clone();
        if let Err(e) = deps.config.set_repo_scope(connection_id, Some(ids)).await {
            self.toast_error(format!("Couldn't set the repository scope: {e}"));
            return;
        }
        self.toast = Some(match count {
            0 => "No repositories selected — nothing will be fetched".to_string(),
            1 => "Fetching from 1 repository".to_string(),
            n => format!("Fetching from {n} repositories"),
        });
        self.narrow_to_repo_scope(connection_id, &scope, deps);
        self.request_reload(deps);
        self.fix_selection();
    }

    /// Drops the rows a connection's just-narrowed repository scope no longer covers.
    ///
    /// Only *narrowing* is knowable: a repository the user just added has contributed no rows
    /// yet, so the reload is what brings them. Matching goes through
    /// [`forgetop_core::repo::matches_scope_entry`], which normalises both sides — a row's
    /// repository and a scope entry are both meant to be connection-relative, and comparing the
    /// two spellings by hand is the mismatch that module exists to prevent.
    fn narrow_to_repo_scope(&mut self, connection_id: &str, scope: &[String], deps: &AppDeps) {
        // A row with no repository can't be placed in or out of the scope, so it stays — that is
        // every row from a provider that isn't repo-addressed at all.
        let covered = |conn: &str, repo: Option<&String>| {
            conn != connection_id
                || repo.is_none_or(|r| scope.iter().any(|entry| forgetop_core::repo::matches_scope_entry(r, entry)))
        };
        self.prs.retain(|r| covered(&r.connection_id, r.pr.repository.as_ref()));
        self.lp_prs_mine.retain(|r| covered(&r.connection_id, r.pr.repository.as_ref()));
        self.lp_prs_review.retain(|r| covered(&r.connection_id, r.pr.repository.as_ref()));
        retain_pool_rows(&mut self.pr_pool, |r| covered(&r.connection_id, r.pr.repository.as_ref()));
        // Azure DevOps addresses work items and pipelines by **Team Project**, so their rows
        // carry a project name where a scope entry holds "Project/Repo" — nothing here could
        // compare the two without re-deriving that addressing, and a wrong guess would empty
        // both sections. Those two are left to the reload, which fans out over the projects.
        self.wis.retain(|r| project_addressed_sections(r.provider) || covered(&r.connection_id, r.wi.repository.as_ref()));
        self.pipes
            .retain(|r| project_addressed_sections(r.provider) || covered(&r.connection_id, r.run.repository.as_ref()));
        self.rebuild_launchpad();
        // Without this the header keeps reporting "Repos · N of M" with the old N.
        self.refresh_repo_scope(deps);
    }

    /// Recomputes the per-section scope summary from config. Cheap (no network) — the discovered
    /// totals come from `repo_catalog`, which is refreshed separately.
    fn refresh_repo_scope(&mut self, deps: &AppDeps) {
        let cfg = deps.config.snapshot();
        let bound = |ids: Vec<String>| -> Option<ScopeSummary> {
            let conns: Vec<_> = ids
                .iter()
                .filter_map(|id| cfg.find_connection(id))
                .filter(|c| forgetop_core::setup::is_repo_addressed(c.provider_type))
                .collect();
            if conns.is_empty() {
                return None;
            }
            let selected = conns
                .iter()
                .map(|c| c.repo_scope.as_ref().map(|s| s.len()).unwrap_or(usize::from(c.repository.is_some())))
                .sum();
            let pages: Vec<_> = conns.iter().filter_map(|c| self.repo_catalog.get(&c.id)).collect();
            let available: usize = pages.iter().map(|p| p.repositories.len()).sum();
            let none_selected = conns.iter().all(|c| c.repo_scope.as_ref().is_some_and(|s| s.is_empty()));
            // Nothing chosen *and* nothing discoverable means there is nothing to scope — the
            // built-in demo connections, or a provider whose discovery didn't answer.
            if selected == 0 && available == 0 && !none_selected {
                return None;
            }
            Some(ScopeSummary {
                connections: conns.iter().map(|c| c.id.clone()).collect(),
                selected,
                available: (!pages.is_empty()).then_some(available),
                truncated: pages.iter().any(|p| p.truncated),
                // An emptied scope is a real state; "never chosen" is not the same thing.
                none_selected,
            })
        };
        let pr_ids = cfg.pull_requests.as_ref().map(|b| b.ids()).unwrap_or_default();
        let wi_ids = cfg.work_items.as_ref().map(|b| b.ids()).unwrap_or_default();
        let pipe_ids =
            cfg.pipelines.as_ref().map(|p| p.subscriptions.iter().map(|s| s.connection_id.clone()).collect()).unwrap_or_default();
        self.repo_scope = [bound(pr_ids), bound(wi_ids), bound(pipe_ids)];
    }

    /// Where Esc lands when closing an item view: back to the Launchpad if it was
    /// opened from there (row still selected), otherwise the section list.
    /// Show an error to the user *and* record it to the log file, so a failed action is
    /// reviewable after the toast fades (write actions, triggers, config changes).
    fn toast_error(&mut self, msg: String) {
        self.toast_error_with_logger(msg, forgetop_core::diag::log);
    }

    fn toast_error_with_logger(&mut self, msg: String, log_failure: impl FnOnce(&str, &str)) {
        log_failure(DIAG_ACTION, DIAG_FAILURE_MESSAGE);
        self.toast = Some(msg);
    }

    fn view_origin(&self) -> Screen {
        if self.from_inbox {
            Screen::Inbox
        } else if self.lp_origin {
            Screen::Launchpad
        } else {
            Screen::List
        }
    }

    fn on_pr_view_key(&mut self, key: Key) {
        // Actions and close are handled before borrowing the view (they need &mut self).
        match key {
            Key::Escape | Key::Char('q') => {
                // In the patch line cursor, Esc steps back to the file list, not out.
                if let Screen::PrView(v) = &mut self.screen {
                    if v.tab == 3 && v.diff.focus == DiffFocus::Patch {
                        v.diff.exit_patch();
                        return;
                    }
                }
                // Leaving the view with buffered-but-unsubmitted comments: ask first.
                if matches!(&self.screen, Screen::PrView(v) if !v.pending.is_empty()) {
                    self.open_pending_exit_prompt();
                    return;
                }
                self.screen = self.view_origin();
                return;
            }
            Key::Char('o') => {
                self.open_selected();
                return;
            }
            // A merged PR offers Revert; an open one offers approve / request-changes / merge.
            Key::Char('a') => {
                if !self.active_pr_is_merged() {
                    self.open_pr_vote(ReviewVote::Approved);
                }
                return;
            }
            Key::Char('x') => {
                if !self.active_pr_is_merged() {
                    self.open_pr_vote(ReviewVote::Rejected);
                }
                return;
            }
            Key::Char('m') => {
                if !self.active_pr_is_merged() {
                    self.open_pr_merge();
                }
                return;
            }
            Key::Char('R') => {
                if self.active_pr_is_merged() {
                    self.open_pr_revert();
                }
                return;
            }
            Key::Char('c') => {
                // On a diff patch line this buffers a line comment; elsewhere it's a
                // plain PR comment.
                self.open_line_comment();
                return;
            }
            Key::Char('r') => {
                // Reply to an existing thread (under the diff cursor, or the sole conversation thread).
                self.open_thread_reply();
                return;
            }
            Key::Char('s') => {
                self.open_review_submit();
                return;
            }
            _ => {}
        }
        let max = self.detail_scroll_max;
        let Screen::PrView(v) = &mut self.screen else { return };
        let n = PR_TABS.len();
        match key {
            // Changing tab resets the scroll (each tab starts at the top), drops any
            // patch line cursor, and restores the whole-PR diff on the Diff tab.
            Key::Left | Key::Char('h') => {
                v.tab = (v.tab + n - 1) % n;
                v.scroll = 0;
                v.reset_diff_scope();
            }
            Key::Right | Key::Char('l') => {
                v.tab = (v.tab + 1) % n;
                v.scroll = 0;
                v.reset_diff_scope();
            }
            // Enter on a file drops into a line cursor within its patch.
            Key::Enter if v.tab == 3 => v.diff.enter_patch(),
            // Diff-tab review ergonomics: mark viewed, jump between threads.
            Key::Char('v') if v.tab == 3 => v.diff.toggle_viewed(),
            Key::Char(']') if v.tab == 3 => v.diff.jump_thread(1),
            Key::Char('[') if v.tab == 3 => v.diff.jump_thread(-1),
            Key::Up | Key::Char('k') => {
                if v.tab == 3 {
                    if v.diff.focus == DiffFocus::Patch {
                        v.diff.move_cursor(-1);
                    } else {
                        v.diff.select_file(-1);
                    }
                } else if v.tab == 1 {
                    v.commit_sel = v.commit_sel.saturating_sub(1);
                } else {
                    v.scroll = v.scroll.saturating_sub(1);
                }
            }
            Key::Down | Key::Char('j') => {
                if v.tab == 3 {
                    if v.diff.focus == DiffFocus::Patch {
                        v.diff.move_cursor(1);
                    } else {
                        v.diff.select_file(1);
                    }
                } else if v.tab == 1 {
                    if !v.commits.is_empty() {
                        v.commit_sel = (v.commit_sel + 1).min(v.commits.len() - 1);
                    }
                } else {
                    v.scroll = (v.scroll + 1).min(max);
                }
            }
            Key::PageDown | Key::Char(' ') => {
                if v.tab == 3 {
                    if v.diff.focus == DiffFocus::Patch {
                        v.diff.move_cursor(10);
                    } else {
                        v.diff.scroll_by(10);
                    }
                } else {
                    v.scroll = (v.scroll + 10).min(max);
                }
            }
            Key::PageUp | Key::Char('b') => {
                if v.tab == 3 {
                    if v.diff.focus == DiffFocus::Patch {
                        v.diff.move_cursor(-10);
                    } else {
                        v.diff.scroll_by(-10);
                    }
                } else {
                    v.scroll = v.scroll.saturating_sub(10);
                }
            }
            _ => {}
        }
    }

    fn on_wi_view_key(&mut self, key: Key) {
        match key {
            Key::Escape | Key::Char('q') => {
                self.screen = self.view_origin();
                return;
            }
            Key::Char('o') => {
                self.open_selected();
                return;
            }
            Key::Char('c') => {
                self.open_wi_comment();
                return;
            }
            Key::Char('e') => {
                self.open_wi_edit();
                return;
            }
            _ => {}
        }
        let max = self.detail_scroll_max;
        let Screen::WiView(v) = &mut self.screen else { return };
        match key {
            Key::Up | Key::Char('k') => v.scroll = v.scroll.saturating_sub(1),
            Key::Down | Key::Char('j') => v.scroll = (v.scroll + 1).min(max),
            Key::PageDown | Key::Char(' ') => v.scroll = (v.scroll + 10).min(max),
            Key::PageUp | Key::Char('b') => v.scroll = v.scroll.saturating_sub(10),
            _ => {}
        }
    }

    /// Normal-mode character commands.
    async fn on_char(&mut self, c: char, deps: &AppDeps) {
        match c {
            'q' => self.leave_section_list(),
            'j' => {
                self.move_down();
                self.ensure_visible();
            }
            'k' => {
                self.move_up();
                self.ensure_visible();
            }
            // 1 = Launchpad, then the sections.
            '1'..='4' => self.set_tab(c as usize - '1' as usize),
            'r' => self.request_reload(deps),
            't' => {
                let next = Theme::next(self.theme.name);
                self.theme = Theme::by_name(next);
                let _ = deps.config.set_theme(Some(next.to_string())).await;
            }
            '/' => self.start_filter(),
            'S' => self.open_sort_picker(),
            'o' => self.open_selected(),
            'v' => self.open_sections_toggle(),
            'C' => self.open_connections(deps).await,
            // Saved views: previous / next on the active section; save / delete.
            '[' => self.switch_view(-1, deps).await,
            ']' => self.switch_view(1, deps).await,
            'V' => self.open_save_view(),
            'X' => self.open_delete_view(),
            // Filter by status (PRs) / by state (Work Items) — 'f' = filter on both tabs.
            'f' if self.active == 0 => self.open_pr_status_toggle(),
            'f' if self.active == 1 => self.open_wi_states_toggle(),
            // Pipeline trigger (Pipelines tab).
            'T' if self.active == 2 => self.open_pipeline_trigger(),
            // `G` cycles how the Pipelines list is **G**rouped. Lowercase `g` is taken by the
            // repo scope picker below, which is a different axis (what is fetched, not how
            // what was fetched is arranged).
            'G' if self.active == 2 => self.cycle_pipe_group(deps).await,
            ' ' if self.active == 2 => self.enter_pipeline_line(deps),
            'z' if self.active == 2 => self.set_all_pipe_groups(false),
            'Z' if self.active == 2 => self.set_all_pipe_groups(true),
            // `g` = which **g**it repositories this section's connections fetch from. Unlike the
            // `f` filter, this gates what is *fetched*, not what is shown from what was fetched.
            'g' => self.open_repo_scope(deps).await,
            // Work-item state/comment (u / c) and PR write actions live inside the
            // opened item's view — press Enter first.
            _ => {}
        }
    }

    // ---- pipeline drill-in + trigger ----

    /// The run under the cursor, or `None` when the cursor is on a group header.
    fn selected_pipe(&self) -> Option<&PipeRow> {
        if self.active != 2 {
            return None;
        }
        let sel = self.pipe_state.selected()?;
        match self.pipe_lines().get(sel)? {
            PipeLine::Run(i) => self.pipes.get(*i),
            PipeLine::Head(_) => None,
        }
    }

    /// The run an *action* key should act on: the selected run, or — when the cursor is on a
    /// group header — that group's most recent run, the one the header's age already refers to.
    ///
    /// Separate from [`App::selected_pipe`], which stays strict: Enter on a header expands it
    /// rather than drilling into something the user did not point at. Without this, `T` and `o`
    /// were dead keys on the default view, where the cursor starts on a header.
    fn pipe_for_action(&self) -> Option<&PipeRow> {
        if self.active != 2 {
            return None;
        }
        let sel = self.pipe_state.selected()?;
        match self.pipe_lines().get(sel)? {
            PipeLine::Run(i) => self.pipes.get(*i),
            PipeLine::Head(h) => {
                let key = h.key.clone();
                self.filtered_pipe_indices()
                    .into_iter()
                    .map(|i| &self.pipes[i])
                    .filter(|p| self.pipe_group_key(p) == key)
                    .max_by_key(|p| p.run.started_at)
            }
        }
    }

    /// Enter on the Pipelines list: a header expands or collapses, a run opens its drill-in.
    fn enter_pipeline_line(&mut self, deps: &AppDeps) {
        // Matches the `Key::Enter` arm, which clears these before dispatching — Space reaches
        // here too, and a stale origin sends Esc back to the Launchpad instead of the list.
        self.lp_origin = false;
        self.from_inbox = false;
        let Some(sel) = self.pipe_state.selected() else { return };
        match self.pipe_lines().get(sel) {
            Some(PipeLine::Head(h)) => {
                let key = h.key.clone();
                self.toggle_pipe_group(&key);
            }
            Some(PipeLine::Run(_)) => self.open_pipeline(deps),
            None => {}
        }
    }

    fn open_pipeline(&mut self, deps: &AppDeps) {
        let Some(pipe) = self.selected_pipe() else { return };
        let (conn_id, provider, run_id, definition_id, branch, title, fallback) = (
            pipe.connection_id.clone(),
            pipe.provider,
            pipe.run.id.clone(),
            pipe.run.definition_id.clone(),
            pipe.run.branch.clone(),
            pipe_label(pipe),
            pipe.run.clone(),
        );
        self.open_pipeline_for(deps, conn_id, provider, run_id, definition_id, branch, title, fallback);
    }

    /// Opens the pipeline drill-in for an explicit run (used by the Launchpad).
    ///
    /// Synchronous and does no I/O: mirrors [`App::open_pr_view_for`] — seeds from the cache,
    /// falling back to the list row's own already-fetched `fallback` run on a miss, so the view
    /// is on screen before the enrich/approvals fetch goes out, then hands that fetch off to
    /// [`App::request_pipeline_detail`].
    ///
    /// Unlike a PR's files/checks, a run's `status` is live: a cached "Running" may have failed
    /// an hour ago, and painting that as current is worse than showing nothing, because the user
    /// acts on it. So the seeded run is marked [`PipelineView::stale`] until
    /// `apply_pipeline_detail` confirms it with a real `get_run`. Approval gates are never
    /// seeded at all — a cached gate that was already actioned would offer a decision that no
    /// longer exists, and `actionable_approvals` drives a real write — they start empty and are
    /// filled in only once the fetch answers.
    #[allow(clippy::too_many_arguments)]
    fn open_pipeline_for(
        &mut self,
        deps: &AppDeps,
        conn_id: String,
        provider: ProviderType,
        run_id: String,
        definition_id: String,
        branch: Option<String>,
        title: String,
        fallback: PipelineRun,
    ) {
        let (view, fetch) = Self::build_pipeline_view(deps, conn_id, provider, run_id, definition_id, branch, title, fallback);
        self.screen = view;
        // The view is on screen now; the fetch that keeps it fresh runs in the background and
        // patches it in place via `AppEvent::PipelineDetailLoaded` (see `apply_pipeline_detail`).
        self.send_detail_request(deps, fetch);
    }

    /// Mirrors [`App::build_pr_view`]: the drill-in seeded from the cache (or the list row's
    /// own run), marked stale until a live fetch confirms it, with no I/O.
    #[allow(clippy::too_many_arguments)]
    fn build_pipeline_view(
        deps: &AppDeps,
        conn_id: String,
        provider: ProviderType,
        run_id: String,
        definition_id: String,
        branch: Option<String>,
        title: String,
        fallback: PipelineRun,
    ) -> (Screen, DetailRequest) {
        let item = ItemRef::maybe(fallback.repository.clone(), run_id);
        let key = pipeline_detail_cache_key(&conn_id, &item);
        let (run, supports_approvals, can_respond_approvals) = match deps.cache.get::<PipelineDetail>(&key) {
            Some(entry) => (entry.value.run, entry.value.supports_approvals, entry.value.can_respond_approvals),
            None => (fallback, false, false),
        };

        let mut view = PipelineView::new(title, run, conn_id.clone(), provider, definition_id, branch);
        view.supports_approvals = supports_approvals;
        view.can_respond_approvals = can_respond_approvals;
        view.stale = true;
        view.auto_select_failed();
        (Screen::Pipeline(Box::new(view)), DetailRequest::Pipeline { conn_id, item, key })
    }

    /// Re-fetches the open pipeline drill-in's run + approvals — used after an approval decision
    /// changes a gate, so the drill-in reflects the decision immediately rather than waiting on
    /// the next detail/reload fetch. Now also writes through to [`pipeline_detail_cache_key`],
    /// so this path and `apply_pipeline_detail` agree on what a later re-open of this run sees.
    async fn refresh_open_pipeline(&mut self, deps: &AppDeps) {
        let Screen::Pipeline(v) = &self.screen else { return };
        let (conn_id, run_ref) = (v.connection_id.clone(), v.run.item_ref());
        // Stamped before the fetch, like every other path: a timestamp taken on completion makes
        // the slowest answer look like the freshest, which is backwards.
        let fetched_at = Utc::now();
        let feeds = detail_or_default(deps.sections.pipeline_feeds().await, DIAG_PIPELINE_FEEDS);
        let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == conn_id) else { return };
        let Some(run) = detail_or_none(feed.source.get_run(&run_ref).await, DIAG_PIPELINE_RUN) else { return };
        let supports_approvals = feed.source.supports_approvals();
        let can_respond_approvals = feed.source.can_respond_to_approvals();
        // A failed gate check stays `None` rather than collapsing to an empty list: clearing the
        // gates would hide one the user still has to act on, and caching that empty would
        // propagate the wrong answer forward as if it had been confirmed.
        let approvals = if supports_approvals {
            detail_or_none(feed.source.pending_approvals(&run_ref).await, DIAG_PIPELINE_APPROVALS)
        } else {
            Some(Vec::new())
        };
        let key = pipeline_detail_cache_key(&conn_id, &run_ref);
        let known = deps.cache.get::<PipelineDetail>(&key).map(|e| e.value);
        let detail = PipelineDetail {
            run: run.clone(),
            approvals: approvals
                .clone()
                .or_else(|| known.map(|k| k.approvals))
                .unwrap_or_default(),
            supports_approvals,
            can_respond_approvals,
        };
        if deps.cache.put(&key, &detail, fetched_at) == CachePut::Stale {
            return; // the store holds something newer — see `apply_pr_detail`
        }
        if let Screen::Pipeline(v) = &mut self.screen {
            v.apply_fresh_run(run, approvals);
        }
    }

    /// Opens the approve/reject picker for the drill-in's actionable gates.
    fn open_approval_picker(&mut self) {
        let Screen::Pipeline(v) = &self.screen else { return };
        if !v.supports_approvals {
            self.toast = Some("Approvals aren't supported on this provider".into());
            return;
        }
        if !v.can_respond_approvals {
            self.toast = Some(format!("Approvals are view-only for {} — approve in the browser", v.provider.as_str()));
            return;
        }
        let actionable = v.actionable_approvals();
        if actionable.is_empty() {
            self.toast = Some("Nothing awaiting your approval on this run".into());
            return;
        }
        // Two rows per gate — an explicit Approve and Reject — so the picker choice
        // already carries the decision; a confirm follows before we act.
        let (conn_id, run_ref) = (v.connection_id.clone(), v.run.item_ref());
        let mut choices = Vec::new();
        let mut items = Vec::new();
        for a in actionable {
            for decision in [ApprovalDecision::Approve, ApprovalDecision::Reject] {
                let verb = match decision {
                    ApprovalDecision::Approve => "Approve",
                    ApprovalDecision::Reject => "Reject",
                };
                items.push(format!("{verb} · {}", a.name));
                choices.push(ApprovalChoice {
                    connection_id: conn_id.clone(),
                    repo: run_ref.repo.clone(),
                    run_id: run_ref.id.clone(),
                    approval_id: a.id.clone(),
                    decision,
                    label: a.name.clone(),
                });
            }
        }
        self.approval_choices = choices;
        self.overlay = Some(Overlay::Picker { title: "Pipeline approval".into(), items, selected: 0, kind: PickerKind::ApprovalGate });
    }

    /// Confirms a picked approval decision before acting.
    fn confirm_approval(&mut self, index: usize) {
        let Some(choice) = self.approval_choices.get(index) else { return };
        let verb = match choice.decision {
            ApprovalDecision::Approve => "Approve",
            ApprovalDecision::Reject => "Reject",
        };
        self.overlay = Some(Overlay::Confirm {
            title: "Pipeline approval".into(),
            message: format!("{verb} deployment to {}?", choice.label),
            action: Action::RespondApproval { index },
        });
    }

    /// Sends the confirmed approve/reject to the provider, then refreshes the run.
    async fn respond_approval(&mut self, index: usize, deps: &AppDeps) {
        let Some(choice) = self.approval_choices.get(index) else { return };
        let (conn_id, run, approval_id, decision, label) = (
            choice.connection_id.clone(),
            ItemRef::maybe(choice.repo.clone(), choice.run_id.clone()),
            choice.approval_id.clone(),
            choice.decision,
            choice.label.clone(),
        );
        let feeds = detail_or_default(deps.sections.pipeline_feeds().await, DIAG_PIPELINE_FEEDS);
        let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == conn_id) else {
            self.toast = Some("Pipeline connection not found".into());
            return;
        };
        match feed.source.respond_approval(&run, &approval_id, decision, None).await {
            Ok(()) => {
                let verb = match decision {
                    ApprovalDecision::Approve => "Approved",
                    ApprovalDecision::Reject => "Rejected",
                };
                self.toast = Some(format!("{verb} {label}"));
                // Drop the decided gate before the refresh, or the picker keeps offering a
                // decision that has already been made until the round trip lands.
                self.drop_decided_approval(&conn_id, &run, &approval_id);
                self.refresh_open_pipeline(deps).await;
            }
            Err(e) => self.toast_error(format!("Approval failed: {e}")),
        }
    }

    /// Removes a just-decided gate from the open run, and clears the list row's badge once the
    /// run has no gate left that the user can answer.
    ///
    /// The run's **status** is deliberately untouched: what a decision does to a run — resume,
    /// fail, queue behind another gate — is the provider's to say, and `refresh_open_pipeline`
    /// is what settles it. Guarded on the run, because the view may have moved on while the
    /// decision was in flight.
    fn drop_decided_approval(&mut self, conn_id: &str, run: &ItemRef, approval_id: &str) {
        let mut still_gated = None;
        if let Screen::Pipeline(v) = &mut self.screen {
            if v.connection_id == conn_id && v.run.id == run.id {
                v.approvals.retain(|a| a.id != approval_id);
                still_gated = Some(v.approvals.iter().any(|a| a.can_respond));
            }
        }
        // No open view means no local knowledge of what is left on the run, so the row keeps its
        // badge until the reload says otherwise: a badge that lingers a second is recoverable, a
        // missing one hides a gate that still wants the user.
        let Some(still_gated) = still_gated else { return };
        if !still_gated {
            for row in self.pipes.iter_mut().filter(|r| r.connection_id == conn_id && r.run.id == run.id) {
                row.awaiting_approval = false;
            }
        }
        // `awaiting_approval` is what puts a run in the Command Center's Approvals bucket.
        self.rebuild_launchpad();
    }

    fn on_pipeline_key(&mut self, key: Key) {
        match key {
            Key::Escape | Key::Char('q') => self.screen = self.view_origin(),
            Key::Char('T') => self.open_pipeline_trigger(),
            Key::Char('A') => self.open_approval_picker(),
            Key::Char('X') => self.open_pipeline_cancel(),
            Key::Char('o') => self.open_selected(),
            other => {
                if let Screen::Pipeline(view) = &mut self.screen {
                    match other {
                        Key::Up | Key::Char('k') => view.move_sel(-1),
                        Key::Down | Key::Char('j') => view.move_sel(1),
                        Key::Enter | Key::Char(' ') | Key::Right | Key::Left => view.toggle_selected(),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Keys on the drill-in. With logs open, `w` moves the keys between the tree and the log
    /// pane; the tree keeps its own keys and re-targets the pane as the cursor crosses jobs.
    fn on_pipeline_screen_key(&mut self, key: Key) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        let logs_open = v.logs.is_some();
        if logs_open && key == Key::Char('w') {
            if v.log_split.get() {
                v.log_focus = !v.log_focus;
            } else {
                self.toast = Some(format!("Widen the terminal to {LOG_SPLIT_MIN_WIDTH}+ columns to see the tree beside the logs"));
            }
            return;
        }
        if v.logs_have_keys() {
            self.on_pipeline_logs_key(key);
            return;
        }
        match key {
            Key::Char('L') if logs_open => v.logs = None,
            Key::Escape if logs_open => v.logs = None,
            Key::Char('L') => self.open_pipeline_logs(),
            _ => {
                self.on_pipeline_key(key);
                // Moving across jobs points the open pane at the new job's log.
                if let Screen::Pipeline(v) = &mut self.screen {
                    if v.logs.is_some() {
                        v.open_logs_for_selection();
                    }
                }
            }
        }
    }

    /// Opens the log pane on the selected job, focused. The fetch goes out in the background
    /// (see [`App::pump_logs`]); the pane shows a placeholder until it answers.
    fn open_pipeline_logs(&mut self) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        if !v.open_logs_for_selection() {
            self.toast = Some("Select a job or step to view its logs".into());
            return;
        }
        v.log_focus = true;
    }

    /// Keys while the log pane holds them: scroll, follow, search, jump to the first error.
    fn on_pipeline_logs_key(&mut self, key: Key) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        let Some(log) = &mut v.logs else { return };
        if let Some(input) = &mut log.search_input {
            match key {
                Key::Escape => log.search_input = None,
                Key::Enter => log.commit_search(),
                Key::Backspace => {
                    input.pop();
                }
                Key::Char(c) => input.push(c),
                _ => {}
            }
            return;
        }
        match key {
            Key::Up | Key::Char('k') => log.scroll_by(-1),
            Key::Down | Key::Char('j') => log.scroll_by(1),
            Key::PageUp | Key::Char('b') => log.scroll_by(-15),
            Key::PageDown | Key::Char(' ') => log.scroll_by(15),
            Key::Home | Key::Char('g') => log.scroll_top(),
            Key::End | Key::Char('G') => log.scroll_bottom(),
            Key::Char('f') => {
                if log.follow {
                    log.follow = false;
                    log.scroll = log.max_scroll();
                } else {
                    log.scroll_bottom();
                }
            }
            Key::Char('E') => log.jump_first_error(),
            Key::Char('/') => log.search_input = Some(String::new()),
            Key::Char('n' | 'N') => {
                if log.query.is_none() {
                    self.toast = Some("No search yet — press / to search the log".into());
                } else if log.matches.is_empty() {
                    self.toast = Some("No matches".into());
                } else {
                    log.step_match(key == Key::Char('n'));
                }
            }
            Key::Escape | Key::Char('q') | Key::Char('L') => {
                v.logs = None;
                v.log_focus = true;
            }
            _ => {}
        }
    }

    /// Keeps the drill-in's log pane fed: opens the pane a failed run's first load asked for,
    /// and sends the fetch the pane wants once nothing else is in flight. Runs after every key,
    /// event and anim tick; does no I/O itself.
    fn drive_logs(&mut self, deps: &AppDeps) {
        if let Screen::Pipeline(v) = &mut self.screen {
            if v.auto_logs {
                v.auto_logs = false;
                if v.logs.is_none() && v.open_logs_for_selection() {
                    v.log_focus = true;
                }
            }
        }
        self.pump_logs(deps);
    }

    /// Driven by the anim timer: advances the open pane's poll schedule (only while the drill-in
    /// is the screen and its job is live), then sends whatever that asked for.
    pub fn tick_logs(&mut self, deps: &AppDeps) {
        if let Some((_, ticks)) = &mut self.log_inflight {
            *ticks = ticks.saturating_add(1);
            if *ticks >= LOG_INFLIGHT_TIMEOUT_TICKS {
                self.log_inflight = None;
            }
        }
        if let Screen::Pipeline(v) = &mut self.screen {
            let active = v.log_target_active();
            if let Some(log) = &mut v.logs {
                if log.poll_step(active) {
                    log.want_fetch = true;
                }
            }
        }
        self.drive_logs(deps);
    }

    /// Sends the log pane's wanted fetch, unless one is already in flight.
    fn pump_logs(&mut self, deps: &AppDeps) {
        if self.log_inflight.is_some() {
            return;
        }
        let Some(tx) = self.job_tx.clone() else { return };
        let Screen::Pipeline(v) = &mut self.screen else { return };
        let (conn_id, run) = (v.connection_id.clone(), v.run.item_ref());
        let Some(log) = v.logs.as_mut().filter(|l| l.want_fetch) else { return };
        log.want_fetch = false;
        let job_id = log.job_id.clone();
        self.log_inflight = Some((log_fetch_key(&conn_id, &run.id, &job_id), 0));
        let deps = deps.clone();
        tokio::spawn(async move {
            let feeds = detail_or_default(deps.sections.pipeline_feeds().await, DIAG_PIPELINE_FEEDS);
            let text = match feeds.iter().find(|f| f.connection.connection_id() == conn_id) {
                Some(feed) => feed.source.logs(&run, Some(&job_id)).await.map_err(|e| {
                    log_operation_failure(DIAG_PIPELINE_LOGS);
                    e.to_string()
                }),
                None => Err("pipeline connection not found".into()),
            };
            let _ = tx.send(AppEvent::PipelineLogsLoaded { conn_id, run_id: run.id, job_id, text });
        });
    }

    /// Folds a log answer into the pane, if the pane still shows that job of that run. A first
    /// fetch that fails closes the pane with a toast, as opening used to; a failed poll keeps the
    /// last good lines and only marks the title, so a flaky connection doesn't spam toasts.
    fn apply_pipeline_logs(&mut self, conn_id: &str, run_id: &str, job_id: &str, text: std::result::Result<String, String>) {
        if self.log_inflight.as_ref().is_some_and(|(k, _)| *k == log_fetch_key(conn_id, run_id, job_id)) {
            self.log_inflight = None;
        }
        let Screen::Pipeline(v) = &mut self.screen else { return };
        if v.connection_id != conn_id || v.run.id != run_id {
            return;
        }
        let Some(log) = v.logs.as_mut().filter(|l| l.job_id == job_id) else { return };
        match text {
            Ok(text) => {
                log.set_text(&text);
                log.loaded = true;
                log.fetch_failed = false;
            }
            Err(e) if !log.loaded => {
                v.logs = None;
                self.toast = Some(format!("Couldn't fetch logs: {e}"));
            }
            Err(_) => log.fetch_failed = true,
        }
    }

    /// The pipeline to trigger — from the drill-in view if open, else the selected list row.
    #[allow(clippy::type_complexity)]
    fn pipeline_target(&self) -> Option<(String, Option<String>, String, Option<String>, String)> {
        if let Screen::Pipeline(v) = &self.screen {
            return Some((v.connection_id.clone(), v.run.repository.clone(), v.definition_id.clone(), v.branch.clone(), v.title.clone()));
        }
        let pipe = self.pipe_for_action()?;
        Some((
            pipe.connection_id.clone(),
            pipe.run.repository.clone(),
            pipe.run.definition_id.clone(),
            pipe.run.branch.clone(),
            pipe_label(pipe),
        ))
    }

    fn open_pipeline_trigger(&mut self) {
        let Some((connection_id, repo, definition_id, branch, label)) = self.pipeline_target() else { return };
        let message = match &branch {
            Some(b) => format!("Trigger {label} on {b}?"),
            None => format!("Trigger {label}?"),
        };
        self.overlay = Some(Overlay::Confirm {
            title: "Trigger".into(),
            message,
            action: Action::PipelineTrigger { connection_id, repo, definition_id, branch, label },
        });
    }

    /// `X` in the drill-in: confirm, then cancel the open run. Only a queued or running run can
    /// be cancelled; a finished one says so instead of asking.
    fn open_pipeline_cancel(&mut self) {
        let Screen::Pipeline(v) = &self.screen else { return };
        if !is_active(v.run.status) {
            self.toast = Some("Only a queued or running run can be cancelled".into());
            return;
        }
        let label = run_label(&v.run);
        self.overlay = Some(Overlay::Confirm {
            title: "Cancel run".into(),
            message: format!("Cancel {label}? Its running jobs stop."),
            action: Action::PipelineCancel { connection_id: v.connection_id.clone(), run: v.run.item_ref(), label },
        });
    }

    /// Cancels a run, then re-reads it. The run's status is left for the provider to report —
    /// a cancel is a request that can take a while to land (or be refused), not a state the
    /// client can assume — so the drill-in and the list are refetched rather than patched.
    async fn cancel_pipeline(&mut self, connection_id: String, run: ItemRef, label: String, deps: &AppDeps) {
        let feeds = match deps.sections.pipeline_feeds().await {
            Ok(f) => f,
            Err(e) => {
                self.toast_error(format!("Cancel failed: {e}"));
                return;
            }
        };
        let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == connection_id) else {
            self.toast = Some("Pipeline connection not found".into());
            return;
        };
        match feed.source.cancel_run(&run).await {
            Ok(()) => {
                self.toast = Some(format!("Cancel requested for {label}"));
                if matches!(&self.screen, Screen::Pipeline(v) if v.connection_id == connection_id && v.run.item_ref() == run) {
                    let key = pipeline_detail_cache_key(&connection_id, &run);
                    self.request_pipeline_detail(deps, connection_id, run, key);
                }
                let mut errors = Vec::new();
                self.reload_pipelines(deps, &mut errors).await;
                self.fix_selection();
                if let Some(e) = errors.first() {
                    self.toast = Some(e.clone());
                }
            }
            Err(e) => self.toast_error(format!("Cancel failed: {e}")),
        }
    }

    async fn execute_pipeline_action(&mut self, action: Action, deps: &AppDeps) {
        let Action::PipelineTrigger { connection_id, repo, definition_id, branch, label } = action else { return };
        let feeds = match deps.sections.pipeline_feeds().await {
            Ok(f) => f,
            Err(e) => {
                self.toast_error(format!("Trigger failed: {e}"));
                return;
            }
        };
        let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == connection_id) else {
            self.toast = Some("Pipeline connection not found".into());
            return;
        };
        match feed.source.trigger(&ItemRef::maybe(repo, definition_id), branch.as_deref()).await {
            Ok(()) => {
                self.toast = Some(format!("Triggered {label}"));
                let mut errors = Vec::new();
                self.reload_pipelines(deps, &mut errors).await;
                self.fix_selection();
                if let Some(e) = errors.first() {
                    self.toast = Some(e.clone());
                }
            }
            Err(e) => self.toast_error(format!("Trigger failed: {e}")),
        }
    }

    async fn on_overlay_key(&mut self, key: Key, deps: &AppDeps) {
        let Some(mut overlay) = self.overlay.take() else { return };
        // Dismissing the palette is a quiet no-op, not a cancelled action.
        let quiet_cancel = matches!(overlay, Overlay::Palette { .. });
        match overlay.handle(key) {
            Outcome::Keep => self.overlay = Some(overlay),
            Outcome::Cancel => {
                if !quiet_cancel {
                    self.toast = Some("Cancelled".into());
                }
            }
            Outcome::Submit(action) => self.execute_action(action, deps).await,
        }
    }

    // ---- open in browser ----

    /// The web URL of whatever is in focus — the open sub-view, else the selected row.
    fn selected_url(&self) -> Option<String> {
        match &self.screen {
            Screen::PrView(v) => return v.url.clone(),
            Screen::WiView(v) => return v.wi.url.clone(),
            Screen::Pipeline(v) => {
                // Prefer the selected job's deep link, falling back to the whole run.
                let nodes = v.flatten();
                return nodes.get(v.selected).and_then(|n| n.url.clone()).or_else(|| v.run.url.clone());
            }
            // The Inbox opens the selected notification's URL directly.
            Screen::Inbox => return self.inbox.get(self.inbox_sel).and_then(|r| r.notification.url.clone()),
            // Launchpad has no single "selected URL" here — Enter opens the item's view,
            // where `o` then works.
            Screen::Launchpad | Screen::List | Screen::Config(_) => {}
        }
        match self.active {
            0 => self.selected_pr().and_then(|p| p.url.clone()),
            1 => self.selected_wi().and_then(|w| w.url.clone()),
            2 => self.pipe_for_action().and_then(|p| p.run.url.clone()),
            _ => None,
        }
    }

    fn open_selected(&mut self) {
        match self.selected_url() {
            Some(url) => {
                self.toast = Some(match open::that(&url) {
                    Ok(()) => format!("Opened {url}"),
                    Err(e) => format!("Couldn't open browser: {e}"),
                });
            }
            None => self.toast = Some("No web URL for this item".into()),
        }
    }

    // ---- add-connection wizard ----

    pub fn start_add_connection(&mut self) {
        self.wizard = Some(Wizard::new());
    }

    async fn on_wizard_key(&mut self, key: Key, deps: &AppDeps) {
        let outcome = match self.wizard.as_mut() {
            Some(w) => w.handle(key),
            None => return,
        };
        match outcome {
            WizardOutcome::Keep => {}
            WizardOutcome::Cancel => {
                self.wizard = None;
                self.toast = Some("Cancelled".into());
            }
            WizardOutcome::Commit => {
                if let Some(w) = self.wizard.take() {
                    self.commit_wizard(w, deps).await;
                }
            }
        }
    }

    async fn commit_wizard(&mut self, wizard: Wizard, deps: &AppDeps) {
        // Offer the notifications chooser once, right after the very first connection.
        let first_run = deps.config.snapshot().connections.is_empty();
        let draft = wizard.draft;
        let Some(provider) = draft.provider else {
            self.toast = Some("No provider chosen".into());
            return;
        };
        let id = Connection::new_id(provider);
        let connection = Connection {
            id: id.clone(),
            provider_type: provider,
            display_name: if draft.display_name.is_empty() { provider.as_str().to_string() } else { draft.display_name },
            base_url: draft.base_url,
            organization: draft.organization,
            project: draft.project,
            repository: draft.repository,
            username: draft.username,
            credential_ref: None,
            // A brand-new connection has no scope yet; it is seeded from discovery below.
            repo_scope: None,
        };

        if let Err(e) = deps.config.add_or_update_connection(connection, draft.pat).await {
            self.toast_error(format!("Add failed: {e}"));
            return;
        }

        // A brand-new account connection with nothing picked starts on its most recently active
        // repositories rather than fetching nothing. Best-effort: if discovery fails the scope
        // stays unset and the connection behaves exactly as a single-repository one did.
        let seeded = forgetop_core::service::seed_default_repo_scope(&deps.config, &deps.sections, &id)
            .await
            .ok()
            .flatten();

        // Bind every section that was ticked. One failing section does not abandon the rest:
        // the connection already exists, so the useful outcome is "bound what it could" plus
        // an honest message about what it could not.
        let mut bound = 0usize;
        let mut failed: Vec<String> = Vec::new();
        for section in &draft.bind_sections {
            let result = match section {
                Section::PullRequests => deps.config.bind_pull_requests(&id).await,
                Section::WorkItems => deps.config.bind_work_items(&id).await,
                Section::Pipelines => deps.config.set_pipeline_auto_discover(&id, true).await,
            };
            match result {
                Ok(()) => bound += 1,
                Err(e) => failed.push(format!("{}: {e}", section_label(*section))),
            }
        }

        if !failed.is_empty() {
            self.toast_error(format!("Added, but binding failed — {}", failed.join("; ")));
            self.request_reload(deps);
            self.rebuild_config_view(deps).await;
            return;
        }

        let sections = match bound {
            0 => String::new(),
            1 => " · 1 section".to_string(),
            n => format!(" · {n} sections"),
        };
        self.toast = Some(match &seeded {
            Some(scope) => format!("Added {} connection · {} repositories{sections}", provider.as_str(), scope.len()),
            None => format!("Added {} connection{sections}", provider.as_str()),
        });
        self.request_reload(deps);
        self.rebuild_config_view(deps).await;

        // First-run: let them choose which notifications to enable.
        if first_run {
            self.open_notifications_toggle();
            self.toast = Some("Choose which notifications you want".into());
        }
    }

    // ---- visible tabs ----

    fn open_sections_toggle(&mut self) {
        let items = (0..TABS.len())
            .map(|i| ToggleItem { id: i.to_string(), label: TABS[i].to_string(), on: self.visible[i] })
            .collect();
        self.overlay =
            Some(Overlay::Toggle { title: "Visible tabs".into(), kind: ToggleKind::Sections, min_one: true, items, selected: 0, filter: None });
    }

    // ---- work-item state visibility ----

    /// Opens a checklist of the distinct states currently present, ticked = shown.
    /// True when the shown-status set includes a completed status, so the fetch must
    /// ask the provider for closed/merged PRs (not just open ones).
    fn pr_wants_completed(&self) -> bool {
        self.pr_shown_statuses.iter().any(|s| matches!(s, PullRequestStatus::Merged | PullRequestStatus::Closed))
    }

    /// Opens the "Show statuses" checklist for the PR list (Open/Draft/Merged/Closed).
    fn open_pr_status_toggle(&mut self) {
        let items = PR_STATUS_ORDER
            .iter()
            .map(|&s| ToggleItem { on: self.pr_shown_statuses.contains(&s), id: pr_status_key(s).into(), label: pr_status_key(s).into() })
            .collect();
        self.overlay =
            Some(Overlay::Toggle { title: "Show statuses".into(), kind: ToggleKind::PrStatuses, min_one: true, items, selected: 0, filter: None });
    }

    /// `ids` are the statuses left ticked. Rebuilds the shown set and repaints.
    ///
    /// No refetch: ticking Merged/Closed only changes which of the pool's two
    /// `include_completed` variants the views derive from, and both were fetched together.
    fn apply_pr_statuses(&mut self, shown_ids: Vec<String>, deps: &AppDeps) {
        self.pr_shown_statuses = shown_ids.iter().filter_map(|id| parse_pr_status(id)).collect();
        self.refresh_derived_prs(deps);
        self.fix_selection();
        self.list_scroll = 0;
        self.toast = Some(format!("Showing {}", pr_status_summary(&self.pr_shown_statuses)));
    }

    fn open_wi_states_toggle(&mut self) {
        let states = self.distinct_wi_states();
        if states.is_empty() {
            self.toast = Some("No work-item states to filter yet".into());
            return;
        }
        let items = states
            .into_iter()
            .map(|s| ToggleItem { on: !self.wi_hidden_states.contains(&s), id: s.clone(), label: s })
            .collect();
        self.overlay =
            Some(Overlay::Toggle { title: "Show states".into(), kind: ToggleKind::WorkItemStates, min_one: false, items, selected: 0, filter: None });
    }

    async fn apply_toggle(&mut self, kind: ToggleKind, ids: Vec<String>, deps: &AppDeps) {
        match kind {
            ToggleKind::Sections => {
                let visible = ids.iter().filter_map(|id| id.parse::<usize>().ok()).map(section_of).collect();
                self.apply_visible_sections(visible, deps).await;
            }
            ToggleKind::PipelineSubs { connection_id } => {
                self.apply_pipeline_subs(&connection_id, ids, deps).await;
            }
            ToggleKind::WorkItemStates => {
                self.apply_wi_states(ids, deps).await;
            }
            ToggleKind::PrStatuses => {
                self.apply_pr_statuses(ids, deps);
            }
            ToggleKind::Notifications => {
                self.apply_notifications(ids, deps).await;
            }
            ToggleKind::RepoScope { connection_id } => {
                self.apply_repo_scope(&connection_id, ids, deps).await;
            }
            ToggleKind::SectionBind { section } => {
                self.apply_section_bind(section, ids, deps).await;
            }
        }
    }

    /// `ids` are the states left *ticked* (shown). Everything present but unticked
    /// becomes hidden; states not present now are left untouched.
    async fn apply_wi_states(&mut self, shown_ids: Vec<String>, deps: &AppDeps) {
        let shown: HashSet<String> = shown_ids.into_iter().collect();
        for state in self.distinct_wi_states() {
            if shown.contains(&state) {
                self.wi_hidden_states.remove(&state);
            } else {
                self.wi_hidden_states.insert(state);
            }
        }
        let hidden: Vec<String> = self.wi_hidden_states.iter().cloned().collect();
        if let Err(e) = deps.config.set_hidden_work_item_states(hidden).await {
            self.toast = Some(format!("Couldn't save: {e}"));
        } else {
            let n = self.hidden_states_in_view();
            self.toast = Some(if n == 0 { "Showing all states".into() } else { format!("{n} state(s) hidden") });
        }
        self.fix_selection();
        self.list_scroll = 0;
    }

    /// Discovers a connection's pipeline definitions and opens a subscribe checklist.
    async fn open_pipeline_subs(&mut self, deps: &AppDeps) {
        let Some((id, display)) = self.config_selected_id() else { return };
        let source = match deps.sections.pipeline_source_for(&id).await {
            Ok(Some(s)) => s,
            _ => {
                self.toast = Some("That connection doesn't support pipelines".into());
                return;
            }
        };
        let defs = match source.discover().await {
            Ok(d) => d,
            Err(e) => {
                self.toast_error(format!("Discover failed: {e}"));
                return;
            }
        };
        if defs.is_empty() {
            self.toast = Some("No pipelines found for this connection".into());
            return;
        }

        let cfg = deps.config.snapshot();
        let sub = cfg.pipelines.as_ref().and_then(|p| p.subscriptions.iter().find(|s| s.connection_id == id));
        let auto = sub.map(|s| s.auto_discover_all).unwrap_or(false);
        let subscribed: std::collections::HashSet<String> =
            sub.map(|s| s.definition_ids.iter().cloned().collect()).unwrap_or_default();

        let items = defs
            .iter()
            .map(|d| ToggleItem { id: d.id.clone(), label: d.name.clone(), on: auto || subscribed.contains(&d.id) })
            .collect();

        self.overlay = Some(Overlay::Toggle {
            title: format!("Subscribe · {display}"),
            kind: ToggleKind::PipelineSubs { connection_id: id },
            min_one: false,
            items,
            selected: 0,
            filter: None,
        });
    }

    async fn apply_pipeline_subs(&mut self, connection_id: &str, ids: Vec<String>, deps: &AppDeps) {
        match deps.config.set_pipeline_definitions(connection_id, ids.clone()).await {
            Ok(()) => {
                self.toast = Some(format!("Subscribed to {} pipeline(s)", ids.len()));
                self.request_reload(deps);
                self.rebuild_config_view(deps).await;
            }
            Err(e) => self.toast = Some(format!("{e}")),
        }
    }

    async fn apply_visible_sections(&mut self, visible: Vec<Section>, deps: &AppDeps) {
        let mut vis = [false; 3];
        for section in &visible {
            vis[index_of(*section)] = true;
        }
        if !vis.iter().any(|v| *v) {
            vis[0] = true;
        }
        self.visible = vis;
        if !self.visible[self.active] {
            self.active = self.first_visible();
            self.list_scroll = 0;
            self.clamp_selection();
        }
        let hidden: Vec<Section> = (0..3).filter(|i| !self.visible[*i]).map(section_of).collect();
        let _ = deps.config.set_hidden_sections(hidden).await;
        self.toast = Some("Updated visible tabs".into());
    }

    // ---- config / connections screen ----

    /// First launch, or nothing set up yet: ask where to do it. forgetop is a terminal tool
    /// first, so the choice is made here rather than by silently opening a browser.
    pub fn open_setup_picker(&mut self) {
        self.overlay = Some(Overlay::Picker {
            title: "Add your provider access tokens".into(),
            items: vec![
                "Set up here in the terminal".into(),
                "Set up in the browser dashboard".into(),
            ],
            selected: 0,
            kind: PickerKind::SetupLocation,
        });
    }

    /// The browser half of that choice: open the dashboard at its settings pane and show the
    /// waiting state until a connection lands.
    pub fn start_setup(&mut self) {
        self.open_dashboard_at("#settings");
        if self.dashboard_url.is_some() {
            self.awaiting_browser_setup = true;
            self.toast = Some("Waiting for setup in the browser…".into());
        }
    }

    /// `C`: the terminal connections list, which adds, binds and removes connections on its own.
    /// It deliberately does *not* open the browser — `B` is the only key that leaves the TUI.
    async fn open_connections(&mut self, deps: &AppDeps) {
        self.open_config(deps).await;
    }

    async fn open_config(&mut self, deps: &AppDeps) {
        let view = self.build_config_view(deps);
        self.screen = Screen::Config(Box::new(view));
    }

    fn build_config_view(&self, deps: &AppDeps) -> ConfigView {
        let cfg = deps.config.snapshot();
        let display_of = |id: &str| cfg.find_connection(id).map(|c| c.display_name.clone()).unwrap_or_else(|| id.to_string());

        let names = |ids: Vec<String>| ids.iter().map(|id| display_of(id)).collect::<Vec<_>>().join(", ");
        let pr_binding = cfg.pull_requests.as_ref().map(|b| b.ids()).filter(|ids| !ids.is_empty()).map(names);
        let wi_binding = cfg.work_items.as_ref().map(|b| b.ids()).filter(|ids| !ids.is_empty()).map(names);
        let pipeline_subs = cfg
            .pipelines
            .as_ref()
            .map(|p| p.subscriptions.iter().map(|s| display_of(&s.connection_id)).collect())
            .unwrap_or_default();

        let connections = cfg
            .connections
            .iter()
            .map(|c| {
                let mut bindings = Vec::new();
                if cfg.pull_requests.as_ref().is_some_and(|b| b.ids().contains(&c.id)) {
                    bindings.push("PR");
                }
                if cfg.work_items.as_ref().is_some_and(|b| b.ids().contains(&c.id)) {
                    bindings.push("WI");
                }
                if cfg.pipelines.as_ref().is_some_and(|p| p.subscriptions.iter().any(|s| s.connection_id == c.id)) {
                    bindings.push("Pipe");
                }
                ConnRow {
                    id: c.id.clone(),
                    display: c.display_name.clone(),
                    provider: c.provider_type,
                    healthy: self.health.iter().find(|h| h.connection.id == c.id).map(|h| h.healthy).unwrap_or(false),
                    bindings,
                }
            })
            .collect();

        ConfigView { connections, pr_binding, wi_binding, pipeline_subs, selected: 0 }
    }

    async fn rebuild_config_view(&mut self, deps: &AppDeps) {
        let sel = match &self.screen {
            Screen::Config(v) => v.selected,
            _ => return,
        };
        let mut view = self.build_config_view(deps);
        view.selected = sel.min(view.connections.len().saturating_sub(1));
        self.screen = Screen::Config(Box::new(view));
    }

    async fn on_config_key(&mut self, key: Key, deps: &AppDeps) {
        match key {
            Key::Escape | Key::Char('q') => self.screen = Screen::List,
            Key::Char('a') => self.start_add_connection(),
            Key::Char('p') => self.open_section_bind(Section::PullRequests, deps),
            Key::Char('w') => self.open_section_bind(Section::WorkItems, deps),
            Key::Char('s') => self.open_pipeline_subs(deps).await,
            Key::Char('x') | Key::Char('d') => self.config_remove_selected(),
            Key::Up | Key::Char('k') => self.config_move(-1),
            Key::Down | Key::Char('j') => self.config_move(1),
            _ => {}
        }
    }

    fn config_move(&mut self, delta: isize) {
        if let Screen::Config(v) = &mut self.screen {
            let len = v.connections.len();
            if len == 0 {
                return;
            }
            let n = len as isize;
            v.selected = (((v.selected as isize + delta) % n + n) % n) as usize;
        }
    }

    fn config_selected_id(&self) -> Option<(String, String)> {
        if let Screen::Config(v) = &self.screen {
            return v.selected_conn().map(|c| (c.id.clone(), c.display.clone()));
        }
        None
    }

    /// Opens a checklist of which connections feed a section (multi-bind).
    fn open_section_bind(&mut self, section: Section, deps: &AppDeps) {
        let cfg = deps.config.snapshot();
        let bound: HashSet<String> = match section {
            Section::PullRequests => cfg.pull_requests.as_ref().map(|b| b.ids()).unwrap_or_default(),
            Section::WorkItems => cfg.work_items.as_ref().map(|b| b.ids()).unwrap_or_default(),
            Section::Pipelines => return,
        }
        .into_iter()
        .collect();

        let items = section_bind_items(&cfg.connections, section, &bound);
        if items.is_empty() {
            self.toast = Some(format!("No connections support {}", section_label(section)));
            return;
        }

        let idx = if section == Section::PullRequests { 0 } else { 1 };
        self.overlay = Some(Overlay::Toggle {
            title: format!("Bind · {}", section_label(section)),
            kind: ToggleKind::SectionBind { section: idx },
            min_one: false,
            items,
            selected: 0,
            filter: None,
        });
    }

    async fn apply_section_bind(&mut self, section: usize, ids: Vec<String>, deps: &AppDeps) {
        let bound: HashSet<String> = ids.iter().cloned().collect();
        let result = if section == 0 {
            deps.config.set_pull_request_connections(ids).await
        } else {
            deps.config.set_work_item_connections(ids).await
        };
        match result {
            Ok(()) => {
                self.toast = Some("Bindings updated".into());
                self.drop_unbound_rows(section, &bound, deps);
                self.request_reload(deps);
                self.rebuild_config_view(deps).await;
            }
            Err(e) => self.toast = Some(format!("{e}")),
        }
    }

    /// Drops the rows of a connection the user just *un*bound from a section.
    ///
    /// Only the unbind half of a binding change is knowable here: a newly bound connection has
    /// contributed no rows yet, so there is nothing to add until the reload brings them — which
    /// is why this only ever removes. `section` is 0 for pull requests, 1 for work items, the
    /// same encoding `apply_section_bind` is called with.
    fn drop_unbound_rows(&mut self, section: usize, bound: &HashSet<String>, deps: &AppDeps) {
        if section == 0 {
            self.prs.retain(|r| bound.contains(&r.connection_id));
            self.lp_prs_mine.retain(|r| bound.contains(&r.connection_id));
            self.lp_prs_review.retain(|r| bound.contains(&r.connection_id));
            retain_pool_rows(&mut self.pr_pool, |r| bound.contains(&r.connection_id));
        } else {
            self.wis.retain(|r| bound.contains(&r.connection_id));
        }
        // The views derived from those lists have to be rebuilt, or the rows stay on screen;
        // the header's "Repos · N of M" counts bound connections, so it moves too.
        self.rebuild_launchpad();
        self.refresh_repo_scope(deps);
        self.fix_selection();
    }

    fn config_remove_selected(&mut self) {
        let Some((id, label)) = self.config_selected_id() else { return };
        self.overlay = Some(Overlay::Confirm {
            title: "Remove connection".into(),
            message: format!("Remove '{label}' and its bindings?"),
            action: Action::RemoveConnection { id, label },
        });
    }

    /// Drops everything a just-removed connection contributed, without waiting for the refetch.
    ///
    /// The reload kicked off alongside this replaces these lists wholesale a second or two later,
    /// so this is not about correctness — it is what makes the removal feel instant. The rows are
    /// gone on the very next frame, and the reload then lands on an identical screen instead of
    /// being the moment the user finally sees their own action take effect.
    fn purge_connection(&mut self, id: &str, deps: &AppDeps) {
        self.prs.retain(|r| r.connection_id != id);
        retain_pool_rows(&mut self.pr_pool, |r| r.connection_id != id);
        self.wis.retain(|r| r.connection_id != id);
        self.pipes.retain(|r| r.connection_id != id);
        self.inbox.retain(|r| r.connection_id != id);
        self.lp_prs_mine.retain(|r| r.connection_id != id);
        self.lp_prs_review.retain(|r| r.connection_id != id);
        self.health.retain(|h| h.connection.id != id);
        self.repo_catalog.remove(id);
        purge_cached_rows(&deps.cache, id);

        // The views derived from those lists have to be rebuilt, or the rows stay on screen.
        self.rebuild_launchpad();
        self.refresh_repo_scope(deps);
        self.inbox_sel = self.inbox_sel.min(self.inbox.len().saturating_sub(1));
        self.fix_selection();
    }

    async fn execute_config_action(&mut self, action: Action, deps: &AppDeps) {
        let Action::RemoveConnection { id, label } = action else { return };
        match deps.config.remove_connection(&id).await {
            Ok(()) => {
                self.toast = Some(format!("Removed {label}"));
                // Reflect the removal before the refetch, not after it.
                self.purge_connection(&id, deps);
                self.request_reload(deps);
                self.rebuild_config_view(deps).await;
            }
            Err(e) => self.toast_error(format!("Remove failed: {e}")),
        }
    }

    // ---- PR write actions ----

    fn selected_pr_row(&self) -> Option<&PrRow> {
        if self.active != 0 {
            return None;
        }
        let idxs = self.filtered_pr_indices();
        self.pr_state.selected().and_then(|p| idxs.get(p)).and_then(|&i| self.prs.get(i))
    }

    fn selected_pr(&self) -> Option<&PullRequest> {
        self.selected_pr_row().map(|r| &r.pr)
    }

    /// The PR a view action targets: the open PR view's PR when one is open (e.g. opened
    /// from the Launchpad, where there's no matching list selection), else the list row.
    fn active_pr(&self) -> Option<&PullRequest> {
        match &self.screen {
            Screen::PrView(v) => Some(&v.pr),
            _ => self.selected_pr(),
        }
    }

    /// Resolves the PR source backing a specific connection (for per-row actions).
    async fn pr_source_for(&self, connection_id: &str, deps: &AppDeps) -> Option<Arc<dyn PullRequestSource>> {
        detail_or_none(
            deps.sections.pull_request_feeds().await,
            DIAG_PR_FEEDS,
        )?
            .into_iter()
            .find(|f| f.connection.connection_id() == connection_id)
            .map(|f| f.source)
    }

    fn open_pr_vote(&mut self, vote: ReviewVote) {
        let Some(pr) = self.active_pr() else { return };
        let verb = match vote {
            ReviewVote::Approved => "Approve",
            ReviewVote::Rejected => "Request changes on",
            _ => "Vote on",
        };
        let message = format!("{verb} {}?", pr_label(pr));
        self.overlay = Some(Overlay::Confirm { title: "Review".into(), message, action: Action::PrVote(vote) });
    }

    fn open_pr_merge(&mut self) {
        let Some(pr) = self.active_pr() else { return };
        let title = format!("Merge {} via", pr_label(pr));
        self.overlay = Some(Overlay::Picker {
            title,
            items: vec!["Merge commit".into(), "Squash".into(), "Rebase".into()],
            selected: 0,
            kind: PickerKind::PrMergeStrategy,
        });
    }

    fn open_pr_revert(&mut self) {
        let Some(pr) = self.active_pr() else { return };
        let message = format!("Revert {}? This asks the provider to undo its merge commit.", pr_label(pr));
        self.overlay = Some(Overlay::Confirm { title: "Revert".into(), message, action: Action::PrRevert });
    }

    /// True when the PR in focus is merged (so it offers Revert instead of approve/merge).
    fn active_pr_is_merged(&self) -> bool {
        self.active_pr().map(|pr| pr.status == PullRequestStatus::Merged).unwrap_or(false)
    }

    fn open_pr_comment(&mut self) {
        let Some(pr) = self.active_pr() else { return };
        let title = format!("Comment on {}", pr_label(pr));
        self.overlay = Some(Overlay::Input { title, buffer: String::new(), kind: InputKind::PrComment });
    }

    async fn execute_action(&mut self, action: Action, deps: &AppDeps) {
        match action {
            Action::PrVote(_) | Action::PrMerge(_) | Action::PrRevert | Action::PrComment(_) | Action::PrReply(_) => {
                self.execute_pr_action(action, deps).await
            }
            Action::WiSetState(_)
            | Action::WiComment(_)
            | Action::WiAssign { .. }
            | Action::WiSetTitle(_)
            | Action::WiSetDescription(_) => self.execute_wi_action(action, deps).await,
            Action::WiEdit(field) => self.start_wi_edit(field),
            Action::PipelineTrigger { .. } => self.execute_pipeline_action(action, deps).await,
            Action::PipelineCancel { connection_id, run, label } => self.cancel_pipeline(connection_id, run, label, deps).await,
            Action::RemoveConnection { .. } => self.execute_config_action(action, deps).await,
            Action::ApplyToggle { kind, ids } => self.apply_toggle(kind, ids, deps).await,
            Action::AddLineComment(body) => self.add_line_comment(body),
            Action::SubmitReview(event) => self.submit_review(event, deps).await,
            Action::SetSort { section, index } => self.apply_sort(section, index, deps).await,
            Action::SaveView(name) => self.save_view(name, deps).await,
            Action::DeleteView => self.delete_view(deps).await,
            Action::PickApproval { index } => self.confirm_approval(index),
            Action::RespondApproval { index } => self.respond_approval(index, deps).await,
            Action::OpenRepoScope { index } => {
                if let Some(id) = self.repo_scope_choices.get(index).cloned() {
                    self.open_repo_scope_for(id, deps).await;
                }
            }
            Action::OpenItem { kind, id, connection_id } => {
                self.run_palette_target(PaletteTarget::Item { kind, id, connection_id }, deps).await
            }
            Action::Palette(target) => self.run_palette_target(target, deps).await,
            Action::OpenReviewMenu => self.open_review_submit(),
            Action::LeavePrView => self.screen = self.view_origin(),
            Action::SetupInTerminal => self.start_add_connection(),
            Action::SetupInBrowser => self.start_setup(),
        }
    }

    async fn execute_pr_action(&mut self, action: Action, deps: &AppDeps) {
        // Resolve the PR + its connection from the open view, else the selected row.
        // Address the PR by repository + id: `#7` alone doesn't say which repository's #7 to
        // merge on a connection that spans several.
        let target = match &self.screen {
            Screen::PrView(v) => Some((v.pr.item_ref(), v.connection_id.clone())),
            _ => self.selected_pr_row().map(|r| (r.pr.item_ref(), r.connection_id.clone())),
        };
        let Some((item, conn_id)) = target else {
            self.toast = Some("Nothing selected".into());
            return;
        };
        let id = item.id.clone();
        let source = match self.pr_source_for(&conn_id, deps).await {
            Some(s) => s,
            None => {
                self.toast = Some("No pull-request provider is bound".into());
                return;
            }
        };

        let result = match &action {
            Action::PrVote(vote) => source.vote(&item, *vote).await.map(|_| vote_message(*vote).to_string()),
            Action::PrMerge(strategy) => source
                .merge(&item, &MergeOptions { strategy: *strategy, delete_source_ref: false })
                .await
                .map(|_| format!("Merged ({strategy:?})")),
            Action::PrRevert => source.revert(&item).await.map(|_| "Revert requested".to_string()),
            Action::PrComment(text) => {
                if text.trim().is_empty() {
                    self.toast = Some("Empty comment — nothing sent".into());
                    return;
                }
                source.add_comment(&item, text).await.map(|_| "Comment added".to_string())
            }
            Action::PrReply(text) => {
                if text.trim().is_empty() {
                    self.toast = Some("Empty reply — nothing sent".into());
                    return;
                }
                let thread_id = match &self.screen {
                    Screen::PrView(v) => v.reply_target.clone(),
                    _ => None,
                };
                let Some(thread_id) = thread_id else {
                    self.toast = Some("No thread selected to reply to".into());
                    return;
                };
                source.reply_to_thread(&item, &thread_id, text).await.map(|_| "Reply posted".to_string())
            }
            _ => return,
        };

        match result {
            Ok(msg) => {
                self.toast = Some(msg);
                // Voting on (reviewing) or merging a PR clears it from the Launchpad now.
                if matches!(action, Action::PrVote(_) | Action::PrMerge(_)) {
                    self.dismiss_from_launchpad(&conn_id, &id);
                }
                // Reflect the change in the open PR view: re-fetch the PR (status / reviewers
                // / mergeable) and its threads (a new comment), like the work-item handler.
                if matches!(&self.screen, Screen::PrView(v) if v.pr.id == id) {
                    let fresh = detail_or_none(source.get(&item).await, DIAG_PR_DETAIL);
                    let threads = detail_or_default(source.threads(&item).await, DIAG_PR_THREADS);
                    let timeline = detail_or_none(source.timeline(&item).await, DIAG_PR_TIMELINE);
                    if let Screen::PrView(v) = &mut self.screen {
                        if let Some(pr) = fresh {
                            v.pr = pr;
                        }
                        v.diff.threads = threads;
                        v.reply_target = None;
                        if let Some(timeline) = timeline {
                            v.timeline = timeline;
                        }
                    }
                }
                let mut errors = Vec::new();
                self.reload_pr_pool(deps, &mut errors).await;
                self.fix_selection();
                if let Some(e) = errors.first() {
                    self.toast = Some(e.clone());
                }
            }
            Err(e) => self.toast_error(format!("Failed: {e}")),
        }
    }

    // ---- work-item write actions ----

    fn selected_wi_row(&self) -> Option<&WiRow> {
        if self.active != 1 {
            return None;
        }
        let idxs = self.filtered_wi_indices();
        self.wi_state.selected().and_then(|p| idxs.get(p)).and_then(|&i| self.wis.get(i))
    }

    fn selected_wi(&self) -> Option<&WorkItem> {
        self.selected_wi_row().map(|r| &r.wi)
    }

    /// Reflects an accepted state change on the open work item *and* the list row behind it.
    ///
    /// Both, because an action taken from the drill-in that only patched the view would be undone
    /// on the screen the user presses Esc back to. `state_category` is deliberately left alone:
    /// which bucket a state name falls in (Backlog/Active/Done) is decided by each provider's
    /// mapper, not derivable from the name here — the reload that follows is what settles it, and
    /// is also the resync if the write turns out to have been rejected.
    fn apply_wi_state(&mut self, conn_id: &str, item: &ItemRef, state: &str) {
        if let Screen::WiView(v) = &mut self.screen {
            v.wi.state = state.to_string();
        }
        // Matched on the whole `ItemRef`, not the bare id: across a multi-repository connection
        // the same id names more than one item.
        for row in self.wis.iter_mut().filter(|r| r.connection_id == conn_id && &r.wi.item_ref() == item) {
            row.wi.state = state.to_string();
        }
        // The Command Center's YourWork bucket reads the work-item rows, so it has to be rebuilt.
        self.rebuild_launchpad();
    }

    /// Resolves the work-item source backing a specific connection (per-row actions).
    async fn wi_source_for(&self, connection_id: &str, deps: &AppDeps) -> Option<Arc<dyn WorkItemSource>> {
        detail_or_none(
            deps.sections.work_item_feeds().await,
            DIAG_WI_FEEDS,
        )?
            .into_iter()
            .find(|f| f.connection.connection_id() == connection_id)
            .map(|f| f.source)
    }

    /// State picker for the open work item, pulling the provider's real available
    /// states (falling back to states seen across the loaded items).
    async fn open_wi_state(&mut self, deps: &AppDeps) {
        let (item, current, title, conn_id) = match &self.screen {
            Screen::WiView(v) => (v.wi.item_ref(), v.wi.state.clone(), format!("Set state — {}", wi_label(&v.wi)), v.connection_id.clone()),
            _ => return,
        };

        let mut states = match self.wi_source_for(&conn_id, deps).await {
            Some(src) => detail_or_default(
                src.available_states(&item).await,
                DIAG_WI_STATES,
            ),
            None => Vec::new(),
        };
        if states.is_empty() {
            states = self.distinct_wi_states();
            if states.len() < 2 {
                states = vec!["Todo".into(), "In Progress".into(), "Done".into()];
            }
        }
        // Ensure the current state is present so it can be preselected.
        if !current.is_empty() && !states.iter().any(|s| s == &current) {
            states.insert(0, current.clone());
        }
        let selected = states.iter().position(|s| *s == current).unwrap_or(0);
        self.overlay = Some(Overlay::Picker { title, items: states, selected, kind: PickerKind::WorkItemState });
    }

    fn open_wi_comment(&mut self) {
        let Screen::WiView(v) = &self.screen else { return };
        let title = format!("Comment on {}", wi_label(&v.wi));
        self.overlay = Some(Overlay::Input { title, buffer: String::new(), kind: InputKind::WorkItemComment });
    }

    /// `@` in the work-item view: a searchable picker over the provider's assignable users,
    /// with *Unassigned* first and the current assignee preselected. `@` again assigns you, when
    /// this connection can say who you are.
    async fn open_wi_assign(&mut self, deps: &AppDeps) {
        let (item, conn_id, current, title) = match &self.screen {
            Screen::WiView(v) => (
                v.wi.item_ref(),
                v.connection_id.clone(),
                v.wi.assignee.clone(),
                format!("Assign {}", wi_label(&v.wi)),
            ),
            _ => return,
        };
        let users = match self.wi_source_for(&conn_id, deps).await {
            Some(src) => detail_or_default(src.assignable_users(&item).await, DIAG_WI_ASSIGNABLE),
            None => Vec::new(),
        };
        if users.is_empty() {
            self.toast = Some("This provider offers no one to assign — assign it in the browser (o)".into());
            return;
        }
        let me = self.pr_pool.me.get(&conn_id).cloned().flatten();
        self.overlay = Some(assignee_picker(title, &users, current.as_ref(), me.as_deref()));
    }

    /// `e` in the work-item view: which field to edit.
    fn open_wi_edit(&mut self) {
        let Screen::WiView(v) = &self.screen else { return };
        self.overlay = Some(Overlay::Picker {
            title: format!("Edit {}", wi_label(&v.wi)),
            items: vec!["Title".into(), "Description (in $EDITOR)".into()],
            selected: 0,
            kind: PickerKind::WorkItemEdit,
        });
    }

    /// The field picked from [`App::open_wi_edit`]: the title in the one-line input, prefilled;
    /// the description handed to `$EDITOR` via [`App::editor_request`].
    fn start_wi_edit(&mut self, field: WiField) {
        let Screen::WiView(v) = &self.screen else { return };
        match field {
            WiField::Title => {
                self.overlay = Some(Overlay::Input {
                    title: format!("Title of {}", v.wi.identifier.clone().unwrap_or_else(|| "this item".into())),
                    buffer: v.wi.title.clone(),
                    kind: InputKind::WorkItemTitle,
                });
            }
            WiField::Description => {
                self.editor_request = Some(EditorRequest { initial: v.wi.description.clone().unwrap_or_default(), field });
            }
        }
    }

    /// Takes the text back from `$EDITOR` (see [`App::editor_request`]). Saving it unchanged,
    /// or quitting without saving, sends nothing.
    pub async fn finish_editor(&mut self, request: EditorRequest, result: std::result::Result<String, String>, deps: &AppDeps) {
        let text = match result {
            Ok(text) => text,
            Err(e) => {
                self.toast_error(format!("Editor failed: {e}"));
                return;
            }
        };
        let text = text.trim_end().to_string();
        if text == request.initial.trim_end() {
            self.toast = Some("Description unchanged — nothing sent".into());
            return;
        }
        match request.field {
            WiField::Description => self.execute_wi_action(Action::WiSetDescription(text), deps).await,
            WiField::Title => self.execute_wi_action(Action::WiSetTitle(text), deps).await,
        }
    }

    /// Reflects an accepted edit on the open work item and on the list row behind it, like
    /// [`App::apply_wi_state`] does for a state change.
    fn patch_wi(&mut self, conn_id: &str, item: &ItemRef, edit: impl Fn(&mut WorkItem)) {
        if let Screen::WiView(v) = &mut self.screen {
            if v.connection_id == conn_id && &v.wi.item_ref() == item {
                edit(&mut v.wi);
            }
        }
        for row in self.wis.iter_mut().filter(|r| r.connection_id == conn_id && &r.wi.item_ref() == item) {
            edit(&mut row.wi);
        }
        self.rebuild_launchpad();
    }

    async fn execute_wi_action(&mut self, action: Action, deps: &AppDeps) {
        let target = match &self.screen {
            Screen::WiView(v) => Some((v.wi.item_ref(), v.connection_id.clone())),
            _ => self.selected_wi_row().map(|r| (r.wi.item_ref(), r.connection_id.clone())),
        };
        let Some((item, conn_id)) = target else {
            self.toast = Some("Nothing selected".into());
            return;
        };
        let source = match self.wi_source_for(&conn_id, deps).await {
            Some(s) => s,
            None => {
                self.toast = Some("No work-item provider is bound".into());
                return;
            }
        };

        let result = match &action {
            Action::WiSetState(state) => source.set_state(&item, state).await.map(|_| format!("State → {state}")),
            Action::WiComment(text) => {
                if text.trim().is_empty() {
                    self.toast = Some("Empty comment — nothing sent".into());
                    return;
                }
                source.add_comment(&item, text).await.map(|_| "Comment added".to_string())
            }
            Action::WiAssign { id, label } => source.set_assignee(&item, id.as_deref()).await.map(|_| match id {
                Some(_) => format!("Assigned to {label}"),
                None => "Unassigned".to_string(),
            }),
            Action::WiSetTitle(title) => {
                let title = title.trim();
                if title.is_empty() {
                    self.toast = Some("A title can't be empty — nothing sent".into());
                    return;
                }
                source.update_fields(&item, Some(title), None).await.map(|_| "Title updated".to_string())
            }
            Action::WiSetDescription(text) => {
                source.update_fields(&item, None, Some(text)).await.map(|_| "Description updated".to_string())
            }
            _ => return,
        };

        match result {
            Ok(msg) => {
                self.toast = Some(msg);
                match &action {
                    Action::WiSetState(state) => self.apply_wi_state(&conn_id, &item, state),
                    Action::WiAssign { id, label } => {
                        let assignee = id.as_ref().map(|id| User {
                            id: id.clone(),
                            display_name: label.clone(),
                            handle: None,
                            avatar_url: None,
                        });
                        self.patch_wi(&conn_id, &item, |wi| wi.assignee = assignee.clone());
                    }
                    Action::WiSetTitle(title) => {
                        let title = title.trim().to_string();
                        self.patch_wi(&conn_id, &item, |wi| wi.title = title.clone());
                    }
                    Action::WiSetDescription(text) => self.patch_wi(&conn_id, &item, |wi| wi.description = Some(text.clone())),
                    _ => {}
                }
                if matches!(action, Action::WiComment(_)) {
                    let threads = detail_or_default(source.threads(&item).await, DIAG_WI_THREADS);
                    if let Screen::WiView(v) = &mut self.screen {
                        v.threads = threads;
                    }
                }
                // Every write lands in the item's history, so the Activity section is re-read too.
                // A failed read keeps what is on screen rather than blanking it.
                if let Some(timeline) = detail_or_none(source.timeline(&item).await, DIAG_WI_TIMELINE) {
                    if let Screen::WiView(v) = &mut self.screen {
                        v.timeline = timeline;
                    }
                }
                let mut errors = Vec::new();
                self.reload_work_items(deps, &mut errors).await;
                self.fix_selection();
                if let Some(e) = errors.first() {
                    self.toast = Some(e.clone());
                }
            }
            Err(e) => self.toast_error(format!("Failed: {e}")),
        }
    }
}

/// The assignee picker: *Unassigned* first, then the provider's users; the current assignee is
/// preselected — matched by id, handle or name, because a provider's item mapper and its
/// assignable-users mapper needn't use the same id (GitHub: numeric id vs login) — and `me` (the signed-in user's handle on this connection, when known) is found
/// among them so `@` can assign you.
fn assignee_picker(title: String, users: &[User], current: Option<&User>, me: Option<&str>) -> Overlay {
    let mut items = vec![SearchItem { id: None, label: "Unassigned".into() }];
    items.extend(users.iter().map(|u| SearchItem { id: Some(u.id.clone()), label: u.display_name.clone() }));
    let is = forgetop_core::filter::is_user;
    let same = |u: &User, c: &User| u.id == c.id || c.handle.as_deref().is_some_and(|h| is(u, h)) || is(u, &c.display_name);
    let selected = current.and_then(|c| users.iter().position(|u| same(u, c))).map_or(0, |i| i + 1);
    let me = me.and_then(|me| users.iter().position(|u| forgetop_core::filter::is_user(u, me))).map(|i| i + 1);
    Overlay::Search { title, query: String::new(), items, selected, kind: SearchKind::Assignee { me } }
}

fn wi_label(wi: &WorkItem) -> String {
    let id = wi.identifier.clone().map(|i| format!("{i} ")).unwrap_or_default();
    let title: String = wi.title.chars().take(40).collect();
    format!("{id}— {title}")
}

/// Discovers repositories for every repo-addressed connection, for the scope indicator's
/// denominator. Best-effort: a connection whose discovery fails simply has no denominator.
/// Runs inside the background fetch, never on the render loop.
async fn discover_repo_catalog(deps: &AppDeps) -> HashMap<String, RepositoryPage> {
    let mut catalog = HashMap::new();
    let cfg = deps.config.snapshot();
    for c in cfg.connections.iter().filter(|c| forgetop_core::setup::is_repo_addressed(c.provider_type)) {
        if let Ok(page) = deps.sections.discover_repositories(&c.id).await {
            catalog.insert(c.id.clone(), page);
        }
    }
    catalog
}

/// Builds a dashboard target without retaining a stale fragment from a previous route.
pub(crate) fn dashboard_target(base: &str, hash: &str) -> String {
    let base = base.split_once('#').map_or(base, |(before_hash, _)| before_hash);
    format!("{base}{hash}")
}

fn log_operation_failure(operation: &'static str) {
    forgetop_core::diag::log(operation, DIAG_FAILURE_MESSAGE);
}

fn detail_or_default<T: Default, E>(
    result: std::result::Result<T, E>,
    operation: &'static str,
) -> T {
    detail_or_default_with_logger(result, operation, forgetop_core::diag::log)
}

fn detail_or_default_with_logger<T: Default, E>(
    result: std::result::Result<T, E>,
    operation: &'static str,
    log_failure: impl FnOnce(&str, &str),
) -> T {
    match result {
        Ok(value) => value,
        Err(_) => {
            log_failure(operation, DIAG_FAILURE_MESSAGE);
            T::default()
        }
    }
}

fn detail_or_none<T, E>(result: std::result::Result<T, E>, operation: &'static str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(_) => {
            log_operation_failure(operation);
            None
        }
    }
}

fn inbox_action_error(
    prefix: &'static str,
    error: impl std::fmt::Display,
    operation: &'static str,
) -> String {
    inbox_action_error_with_logger(prefix, error, operation, forgetop_core::diag::log)
}

fn inbox_action_error_with_logger(
    prefix: &'static str,
    error: impl std::fmt::Display,
    operation: &'static str,
    log_failure: impl FnOnce(&str, &str),
) -> String {
    log_failure(operation, DIAG_FAILURE_MESSAGE);
    format!("{prefix}: {error}")
}

fn push_reload_error(
    errors: &mut Vec<String>,
    detail: String,
    operation: &'static str,
) {
    push_reload_error_with_logger(errors, detail, operation, forgetop_core::diag::log);
}

fn push_reload_error_with_logger(
    errors: &mut Vec<String>,
    detail: String,
    operation: &'static str,
    log_failure: impl FnOnce(&str, &str),
) {
    log_failure(operation, DIAG_FAILURE_MESSAGE);
    errors.push(detail);
}

/// Checklist items for binding a section: connections that support it, ticked if bound.
fn section_bind_items(connections: &[Connection], section: Section, bound: &HashSet<String>) -> Vec<ToggleItem> {
    connections
        .iter()
        .filter(|c| provider_sections(c.provider_type).contains(&section))
        .map(|c| ToggleItem { id: c.id.clone(), label: c.display_name.clone(), on: bound.contains(&c.id) })
        .collect()
}

/// Pulls the (provider, display name, id) tag off a feed's connection.
fn feed_tag(conn: &Arc<dyn ProviderConnection>) -> (ProviderType, String, String) {
    (conn.provider_type(), conn.display_name().to_string(), conn.connection_id().to_string())
}

fn pipe_label(pipe: &PipeRow) -> String {
    let name = pipe.run.name.clone().unwrap_or_else(|| pipe.run.definition_id.clone());
    match pipe.run.number {
        Some(n) => format!("{name} #{n}"),
        None => name,
    }
}

/// A run as one short name for a message: its name and number, without saying the number twice
/// when the name already is the number (`#9902`, not `#9902 #9902`).
fn run_label(run: &PipelineRun) -> String {
    let number = run.number.map(|n| format!("#{n}"));
    match (run.name.as_deref().filter(|n| !n.trim().is_empty()), number) {
        (Some(name), Some(num)) if name.contains(&num) => name.to_string(),
        (Some(name), Some(num)) => format!("{name} {num}"),
        (Some(name), None) => name.to_string(),
        (None, Some(num)) => num,
        (None, None) => format!("run {}", run.id),
    }
}

/// Runs that are Failed now but weren't Failed at the previous refresh.
fn new_pipeline_failures<'a>(prev: &HashMap<String, PipelineRunStatus>, pipes: &'a [PipeRow]) -> Vec<&'a PipeRow> {
    pipes
        .iter()
        .filter(|r| {
            matches!(r.run.status, PipelineRunStatus::Failed) && !matches!(prev.get(&r.run.id), Some(PipelineRunStatus::Failed))
        })
        .collect()
}

/// Identifies one job's log fetch, to match an answer to the fetch that is in flight.
fn log_fetch_key(conn_id: &str, run_id: &str, job_id: &str) -> String {
    format!("{conn_id}\u{1f}{run_id}\u{1f}{job_id}")
}

/// Whether a run is still in flight (only these can be waiting on an approval gate).
fn is_active(status: PipelineRunStatus) -> bool {
    matches!(status, PipelineRunStatus::Queued | PipelineRunStatus::Running)
}

/// Runs awaiting the user's approval now that weren't at the previous refresh.
fn new_pending_approvals<'a>(prev: &HashSet<(String, String)>, pipes: &'a [PipeRow]) -> Vec<&'a PipeRow> {
    pipes
        .iter()
        .filter(|r| r.awaiting_approval && !prev.contains(&(r.connection_id.clone(), r.run.id.clone())))
        .collect()
}

/// Sends desktop notifications. Behind a trait so tests can inject a recorder
/// instead of firing real OS notifications.
pub trait Notifier: Send + Sync {
    fn notify(&self, title: &str, body: &str);
}

/// The real notifier — best-effort OS notification (silently ignored if refused).
pub struct SystemNotifier;

impl Notifier for SystemNotifier {
    fn notify(&self, title: &str, body: &str) {
        let _ = notify_rust::Notification::new().summary(title).body(body).appname("forgetop").show();
    }
}

/// (approved, changes-requested) rollup from a PR's reviewer votes.
pub(crate) use forgetop_core::launchpad::pr_vote_flags;

/// Which vote states newly flipped on since last scan: (newly approved, newly changes).
fn pr_review_transitions(prev: Option<(bool, bool)>, pr: &PullRequest) -> (bool, bool) {
    let (a, c) = pr_vote_flags(pr);
    let (pa, pc) = prev.unwrap_or((false, false));
    (a && !pa, c && !pc)
}


fn pr_label(pr: &PullRequest) -> String {
    let num = pr.number.map(|n| format!("#{n} ")).unwrap_or_default();
    let title: String = pr.title.chars().take(40).collect();
    format!("PR {num}— {title}")
}

/// Quick-filter match: every whitespace-separated token in `q` (already lowercased)
/// must appear somewhere in the row's searchable text. Empty query matches everything.
fn pr_matches(pr: &PullRequest, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {} {}",
        pr.title,
        pr.author.display_name,
        pr.number.map(|n| format!("#{n}")).unwrap_or_default(),
        pr.source_ref.clone().unwrap_or_default(),
        pr.labels.join(" "),
    )
    .to_lowercase();
    q.split_whitespace().all(|t| hay.contains(t))
}

fn wi_matches(wi: &WorkItem, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {} {}",
        wi.title,
        wi.identifier.clone().unwrap_or_default(),
        wi.state,
        wi.work_item_type.clone().unwrap_or_default(),
        wi.assignee.as_ref().map(|a| a.display_name.clone()).unwrap_or_default(),
    )
    .to_lowercase();
    q.split_whitespace().all(|t| hay.contains(t))
}

fn pipe_matches(p: &PipeRow, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    // Everything the row can show, so `/forgetop` finds what the Repository column displays
    // and `/integration` finds what the Pipeline column displays.
    let hay = format!(
        "{} {} {} {} {} {} {:?}",
        p.run.name.clone().unwrap_or_else(|| p.run.definition_id.clone()),
        p.definition_name.clone().unwrap_or_default(),
        p.run.repository.clone().unwrap_or_default(),
        p.provider.as_str(),
        p.connection,
        p.run.branch.clone().unwrap_or_default(),
        p.run.status,
    )
    .to_lowercase();
    q.split_whitespace().all(|t| hay.contains(t))
}

// ---- sorting ----

/// One sortable column: a stable `key` (persisted) and a display `label`.
pub struct SortCol {
    pub key: &'static str,
    pub label: &'static str,
}

const PR_SORTS: &[SortCol] = &[
    SortCol { key: "updated", label: "Updated" },
    SortCol { key: "number", label: "Number" },
    SortCol { key: "title", label: "Title" },
    SortCol { key: "author", label: "Author" },
    SortCol { key: "checks", label: "Checks" },
    SortCol { key: "status", label: "Status" },
];
const WI_SORTS: &[SortCol] = &[
    SortCol { key: "updated", label: "Updated" },
    SortCol { key: "state", label: "State" },
    SortCol { key: "title", label: "Title" },
    SortCol { key: "type", label: "Type" },
    SortCol { key: "assignee", label: "Assignee" },
];
const PIPE_SORTS: &[SortCol] = &[
    SortCol { key: "started", label: "Started" },
    SortCol { key: "status", label: "Status" },
    SortCol { key: "pipeline", label: "Pipeline" },
    SortCol { key: "repository", label: "Repository" },
    SortCol { key: "provider", label: "Provider" },
    SortCol { key: "branch", label: "Branch" },
];

/// The sortable columns for a section (0=PR, 1=WI, 2=Pipelines).
pub fn sort_cols(section: usize) -> &'static [SortCol] {
    match section {
        0 => PR_SORTS,
        1 => WI_SORTS,
        _ => PIPE_SORTS,
    }
}

/// Sensible default direction when first picking a column: newest / highest first
/// for time, number and status columns; A→Z for text.
fn default_desc(key: &str) -> bool {
    matches!(key, "updated" | "created" | "number" | "started" | "checks" | "status")
}

fn ci(s: &str) -> String {
    s.to_lowercase()
}

fn check_rank(s: CheckStatus) -> u8 {
    match s {
        CheckStatus::None => 0,
        CheckStatus::Passed => 1,
        CheckStatus::Pending => 2,
        CheckStatus::Failed => 3,
    }
}

fn pr_status_rank(pr: &PullRequest) -> u8 {
    if pr.is_draft {
        return 0;
    }
    match pr.status {
        PullRequestStatus::Draft => 0,
        PullRequestStatus::Open => 1,
        PullRequestStatus::Merged => 2,
        PullRequestStatus::Closed => 3,
    }
}

fn wi_state_rank(c: WorkItemStateCategory) -> u8 {
    match c {
        WorkItemStateCategory::Triage => 0,
        WorkItemStateCategory::Backlog => 1,
        WorkItemStateCategory::Unstarted => 2,
        WorkItemStateCategory::Started => 3,
        WorkItemStateCategory::Completed => 4,
        WorkItemStateCategory::Canceled => 5,
    }
}

fn pipe_status_rank(s: PipelineRunStatus) -> u8 {
    match s {
        PipelineRunStatus::Failed => 0,
        PipelineRunStatus::Canceled => 1,
        PipelineRunStatus::Running => 2,
        PipelineRunStatus::Queued => 3,
        PipelineRunStatus::PartiallySucceeded => 4,
        PipelineRunStatus::Succeeded => 5,
    }
}

fn pr_cmp(a: &PullRequest, b: &PullRequest, key: &str) -> Ordering {
    match key {
        "updated" => a.updated_at.cmp(&b.updated_at),
        "number" => a.number.cmp(&b.number),
        "title" => ci(&a.title).cmp(&ci(&b.title)),
        "author" => ci(&a.author.display_name).cmp(&ci(&b.author.display_name)),
        "checks" => check_rank(a.checks).cmp(&check_rank(b.checks)),
        "status" => pr_status_rank(a).cmp(&pr_status_rank(b)),
        _ => Ordering::Equal,
    }
}

fn wi_cmp(a: &WorkItem, b: &WorkItem, key: &str) -> Ordering {
    match key {
        "updated" => a.updated_at.cmp(&b.updated_at),
        "state" => wi_state_rank(a.state_category).cmp(&wi_state_rank(b.state_category)).then_with(|| ci(&a.state).cmp(&ci(&b.state))),
        "title" => ci(&a.title).cmp(&ci(&b.title)),
        "type" => ci(a.work_item_type.as_deref().unwrap_or("")).cmp(&ci(b.work_item_type.as_deref().unwrap_or(""))),
        "assignee" => {
            let an = a.assignee.as_ref().map(|u| ci(&u.display_name)).unwrap_or_default();
            let bn = b.assignee.as_ref().map(|u| ci(&u.display_name)).unwrap_or_default();
            an.cmp(&bn)
        }
        _ => Ordering::Equal,
    }
}

fn pipe_cmp(a: &PipeRow, b: &PipeRow, key: &str) -> Ordering {
    match key {
        "started" => a.run.started_at.cmp(&b.run.started_at),
        "status" => pipe_status_rank(a.run.status).cmp(&pipe_status_rank(b.run.status)),
        // Sorts by the same string the column shows, via the one helper that defines it.
        "pipeline" => ci(&pipe_definition_name(a)).cmp(&ci(&pipe_definition_name(b))),
        "repository" => ci(a.run.repository.as_deref().unwrap_or("")).cmp(&ci(b.run.repository.as_deref().unwrap_or(""))),
        "provider" => ci(a.provider.as_str()).cmp(&ci(b.provider.as_str())).then_with(|| ci(&a.connection).cmp(&ci(&b.connection))),
        "branch" => ci(a.run.branch.as_deref().unwrap_or("")).cmp(&ci(b.run.branch.as_deref().unwrap_or(""))),
        _ => Ordering::Equal,
    }
}

/// Applies the sort direction to a comparison.
fn ordered(o: Ordering, desc: bool) -> Ordering {
    if desc {
        o.reverse()
    } else {
        o
    }
}

fn vote_message(vote: ReviewVote) -> &'static str {
    match vote {
        ReviewVote::Approved => "Approved",
        ReviewVote::ApprovedWithSuggestions => "Approved with suggestions",
        ReviewVote::Rejected => "Requested changes",
        ReviewVote::WaitingForAuthor => "Waiting for author",
        ReviewVote::NoVote => "Vote reset",
    }
}

/// Semantic key events the loop feeds into [`App::on_key`]. Character keys keep
/// their raw value so the app can treat them as navigation in normal mode or as
/// literal text while an input overlay is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Tab,
    /// Shift-Tab: the tab strip, backwards.
    BackTab,
    Enter,
    Escape,
    Backspace,
    PageUp,
    PageDown,
    Home,
    End,
    Char(char),
    /// A Ctrl-modified letter (lowercased), e.g. Ctrl-P. Ctrl-C is mapped to [`Key::Quit`].
    Ctrl(char),
    /// Hard quit (Ctrl-C), honoured in every mode.
    Quit,
    /// Terminal was resized — no-op, but wakes the loop so it redraws at the new size.
    Redraw,
    /// A left click at (column, row), resolved against the last frame's [`Hit`] map.
    Click(u16, u16),
    /// The mouse wheel, one notch, over (column, row).
    ScrollUp(u16, u16),
    ScrollDown(u16, u16),
    None,
}

impl Key {
    fn is_mouse(self) -> bool {
        matches!(self, Key::Click(..) | Key::ScrollUp(..) | Key::ScrollDown(..))
    }
}

/// What sits under a screen cell in the last frame, so a click can act on it. The renderer
/// records these as it draws ([`crate::ui::render`]); later entries are drawn on top, so the
/// last one containing a point wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// A tab on the top strip: 0 = Command Center, then each visible section.
    Tab(usize),
    /// A PR view sub-tab (Conversation, Commits, Checks, Diff).
    PrTab(usize),
    /// A row of the active section list, by the index its selection uses.
    ListRow(usize),
    /// A Command Center column, and a row in it (a slot position, as `lp_sel` counts).
    LpColumn(usize),
    LpRow { side: usize, pos: usize },
    /// A notification in the inbox.
    InboxRow(usize),
    /// A commit on the PR view's Commits tab.
    CommitRow(usize),
    /// A file in the Diff tab's file list.
    DiffFile(usize),
    /// The Diff tab's patch pane, and a patch line in it.
    DiffPatch,
    DiffLine(usize),
    /// A node of the pipeline tree, and the log pane beside it.
    PipeNode(usize),
    LogPane,
}


// ---- background fetch helpers (no `&mut self`, safe to run in a spawned task) ----

// Each of these returns its rows plus whether *every* feed it consulted answered. A failure
// drops that connection's rows silently — the list still arrives, just short — so the flag is
// the only thing that can tell a partial result from a real one. `write_through` caches a
// section on that flag alone; see its comment for why "has some rows" is not good enough.


async fn fetch_work_items(deps: &AppDeps, errors: &mut Vec<String>) -> (Vec<WiRow>, bool) {
    let mut out = Vec::new();
    let mut ok = true;
    match deps.sections.work_item_feeds().await {
        Ok(feeds) => {
            for feed in feeds {
                let (provider, name, conn_id) = feed_tag(&feed.connection);
                match feed.source.list(&wi_query()).await {
                    Ok(list) => out.extend(list.into_iter().map(|wi| WiRow { connection_id: conn_id.clone(), connection: name.clone(), provider, wi })),
                    Err(e) => {
                        ok = false;
                        errors.push(format!("Work items ({name}): {e}"));
                    }
                }
            }
        }
        Err(e) => {
            ok = false;
            errors.push(format!("Work items: {e}"));
        }
    }
    (out, ok)
}

async fn fetch_notifications(deps: &AppDeps, errors: &mut Vec<String>) -> (Vec<NotifRow>, bool) {
    let mut out = Vec::new();
    let mut ok = true;
    match deps.sections.notification_feeds().await {
        Ok(feeds) => {
            for feed in feeds {
                let (provider, name, conn_id) = feed_tag(&feed.connection);
                match feed.source.list().await {
                    Ok(list) => out.extend(list.into_iter().map(|n| NotifRow {
                        connection_id: conn_id.clone(),
                        connection: name.clone(),
                        provider,
                        notification: n,
                    })),
                    Err(e) => {
                        ok = false;
                        errors.push(format!("Notifications ({name}): {e}"));
                    }
                }
            }
        }
        Err(e) => {
            ok = false;
            errors.push(format!("Notifications: {e}"));
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.notification.updated_at)); // newest first
    (out, ok)
}

async fn fetch_pipelines(deps: &AppDeps, errors: &mut Vec<String>) -> (Vec<PipeRow>, bool) {
    let mut out = Vec::new();
    let mut ok = true;
    match deps.sections.pipeline_feeds().await {
        Ok(feeds) => {
            for feed in feeds {
                let provider = feed.connection.provider_type();
                let name = feed.connection.display_name().to_string();
                let conn_id = feed.connection.connection_id().to_string();
                // Discovery failing isn't cosmetic: it decides which runs are even asked for,
                // so the rows that come back are a subset of what a healthy fetch would return.
                let defs = match detail_or_none(feed.source.discover().await, DIAG_PIPELINE_DISCOVERY) {
                    Some(defs) => defs,
                    None => {
                        ok = false;
                        Vec::new()
                    }
                };
                let def_names: HashMap<String, String> =
                    defs.iter().map(|d| (d.id.clone(), d.name.clone())).collect();
                for q in feed_queries(&feed.subscription, &defs) {
                    match feed.source.list_runs(&q).await {
                        Ok(runs) => {
                            let supports = feed.source.supports_approvals();
                            for run in runs {
                                // A failed gate check is not "no gate": caching the row as clear
                                // would hide a pending approval until the next whole reload, so
                                // it clears the section flag like discovery's failure does.
                                let (awaiting_approval, gate_ok) = if supports && is_active(run.status) {
                                    gate_from_approvals(detail_or_none(
                                        feed.source.pending_approvals(&run.item_ref()).await,
                                        DIAG_PIPELINE_APPROVALS,
                                    ))
                                } else {
                                    (false, true)
                                };
                                if !gate_ok {
                                    ok = false;
                                }
                                let definition_name = def_names.get(&run.definition_id).cloned();
                                out.push(PipeRow { connection_id: conn_id.clone(), connection: name.clone(), provider, run, definition_name, awaiting_approval });
                            }
                        }
                        Err(e) => {
                            ok = false;
                            errors.push(format!("Pipelines ({name}): {e}"));
                        }
                    }
                }
            }
        }
        Err(e) => {
            ok = false;
            errors.push(format!("Pipelines: {e}"));
        }
    }
    (out, ok)
}

/// A row's `awaiting_approval` plus whether the gate check that decided it actually answered.
/// `None` in means the call failed: the row reads as "no gate pending", but the caller must not
/// cache that guess, so the second element is what clears the section's flag.
fn gate_from_approvals(gates: Option<Vec<PipelineApproval>>) -> (bool, bool) {
    match gates {
        Some(gates) => (gates.iter().any(|approval| approval.can_respond), true),
        None => (false, false),
    }
}

/// Re-fetches the open pipeline run + its pending approvals (mirrors
/// [`App::refresh_open_pipeline`]), so the background refresh keeps the view live.
async fn fetch_open_pipeline(
    deps: &AppDeps,
    conn_id: &str,
    run_ref: &ItemRef,
) -> Option<(String, PipelineRun, Option<Vec<PipelineApproval>>)> {
    let feeds = detail_or_default(deps.sections.pipeline_feeds().await, DIAG_PIPELINE_FEEDS);
    let feed = feeds
        .iter()
        .find(|f| f.connection.connection_id() == conn_id)?;
    let run = detail_or_none(feed.source.get_run(run_ref).await, DIAG_PIPELINE_RUN)?;
    // `detail_or_none`, not `detail_or_default`: a failed gate check must stay distinguishable
    // from an answered "no gates", or a 502 silently clears a gate the user still has to act on.
    let approvals = if feed.source.supports_approvals() {
        detail_or_none(feed.source.pending_approvals(run_ref).await, DIAG_PIPELINE_APPROVALS)
    } else {
        Some(Vec::new())
    };
    Some((run_ref.id.clone(), run, approvals))
}

/// Fetches the four detail sections behind a PR view, off the render loop. Mirrors what
/// `open_pr_view_for` used to await inline. Left sequential on purpose, not fanned out
/// concurrently: that would attribute a slow/failing section's error to the wrong request and
/// is a separate change with its own consequences (see the task note that introduced this).
/// Each call yields `None` rather than an empty list when it fails, so the caller can keep what
/// it already knew for that section instead of painting — and caching — a blank one.
async fn fetch_pr_detail(deps: &AppDeps, conn_id: &str, item: &ItemRef) -> PrDetailFetch {
    let feeds = detail_or_default(deps.sections.pull_request_feeds().await, DIAG_PR_FEEDS);
    let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == conn_id) else {
        // No feed answered for this connection, which is a failure to look — not a PR that
        // genuinely has no files, checks or comments.
        return PrDetailFetch::default();
    };
    let source = &feed.source;
    let threads = detail_or_none(source.threads(item).await, DIAG_PR_THREADS);
    let files = detail_or_none(source.changes(item).await, DIAG_PR_CHANGES).map(|mut files| {
        files.sort_by(|a, b| a.path.cmp(&b.path)); // cluster by directory for grouping
        files
    });
    let checks = detail_or_none(source.checks(item).await, DIAG_PR_CHECKS);
    let commits = detail_or_none(source.commits(item).await, DIAG_PR_COMMITS);
    let timeline = detail_or_none(source.timeline(item).await, DIAG_PR_TIMELINE);
    PrDetailFetch { threads, files, checks, commits, timeline }
}

/// Fetches the detail behind a work-item view, off the render loop. Mirrors [`fetch_pr_detail`].
async fn fetch_wi_detail(deps: &AppDeps, conn_id: &str, item: &ItemRef) -> WiDetailFetch {
    let feeds = detail_or_default(deps.sections.work_item_feeds().await, DIAG_WI_FEEDS);
    let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == conn_id) else {
        // No feed answered for this connection, which is a failure to look — not a work item
        // that genuinely has no comments.
        return WiDetailFetch::default();
    };
    let threads = detail_or_none(feed.source.threads(item).await, DIAG_WI_THREADS);
    let timeline = detail_or_none(feed.source.timeline(item).await, DIAG_WI_TIMELINE);
    WiDetailFetch { threads, timeline }
}

/// Fetches the run + capabilities + approvals behind a pipeline drill-in, off the render loop.
/// Mirrors [`fetch_pr_detail`]; left sequential for the same reason. Approvals are fetched
/// regardless of whether `get_run` itself succeeded — they're addressed by the run ref, not the
/// fetched run object, so a failure to enrich the run needn't also blank the approvals answer.
async fn fetch_pipeline_detail(deps: &AppDeps, conn_id: &str, run_ref: &ItemRef) -> PipelineDetailFetch {
    let feeds = detail_or_default(deps.sections.pipeline_feeds().await, DIAG_PIPELINE_FEEDS);
    let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == conn_id) else {
        return PipelineDetailFetch::default();
    };
    let source = &feed.source;
    let run = detail_or_none(source.get_run(run_ref).await, DIAG_PIPELINE_RUN);
    let supports_approvals = source.supports_approvals();
    let can_respond_approvals = source.can_respond_to_approvals();
    let approvals = if supports_approvals {
        detail_or_none(source.pending_approvals(run_ref).await, DIAG_PIPELINE_APPROVALS)
    } else {
        Some(Vec::new())
    };
    PipelineDetailFetch {
        run,
        approvals,
        supports_approvals: Some(supports_approvals),
        can_respond_approvals: Some(can_respond_approvals),
    }
}

/// Cheap "did anything actually change" check for a landed [`PrDetail`] against what the open
/// view already holds, so a revalidation that finds nothing new causes no repaint (no scroll
/// jump, no flicker). None of `CommentThread`, `FileChange`, `CheckRun` or `Commit` derive
/// `PartialEq` in forgetop-core, so this isn't a full structural diff — it compares the fields
/// most likely to actually move:
/// - threads: count, plus total comment count, plus resolved count (misses an edited comment
///   body, or a same-size swap of which threads are resolved)
/// - files: count, plus (path, kind, additions, deletions) per file (misses a patch-text-only
///   change with unchanged add/delete counts — not achievable from a real diff)
/// - checks: count, plus (name, status) per check (misses a check's URL alone changing)
/// - commits: the sequence of shas (an amend or force-push always changes a sha, so this is
///   exact for reordering/rewriting; misses an in-place message edit on a provider that allows it)
fn pr_detail_unchanged(v: &PrView, d: &PrDetail) -> bool {
    threads_match(&v.diff.threads, &d.threads)
        && files_match(&v.pr_files, &d.files)
        && checks_match(&v.checks, &d.checks)
        && commits_match(&v.commits, &d.commits)
        && timeline_match(&v.timeline, &d.timeline)
}

fn threads_match(a: &[CommentThread], b: &[CommentThread]) -> bool {
    let counts = |ts: &[CommentThread]| -> (usize, usize) {
        (ts.iter().map(|t| t.comments.len()).sum(), ts.iter().filter(|t| t.is_resolved).count())
    };
    a.len() == b.len() && counts(a) == counts(b)
}

fn files_match(a: &[FileChange], b: &[FileChange]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.path == y.path && x.kind == y.kind && x.additions == y.additions && x.deletions == y.deletions
        })
}

fn checks_match(a: &[CheckRun], b: &[CheckRun]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.name == y.name && x.status == y.status)
}

fn commits_match(a: &[Commit], b: &[Commit]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.sha == y.sha)
}

fn timeline_match(a: &[TimelineEvent], b: &[TimelineEvent]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.summary == y.summary && x.at == y.at)
}

/// Mirrors [`pr_detail_unchanged`].
fn wi_detail_unchanged(v: &WiView, d: &WiDetail) -> bool {
    threads_match(&v.threads, &d.threads) && timeline_match(&v.timeline, &d.timeline)
}

/// The parts of a [`PipelineRun`] a revalidation is most likely to have actually moved: the
/// run's own status, and each stage's and job's status (not step-level status, timestamps,
/// problem text, or URLs — a job's own status already reflects its steps having changed). Misses
/// a branch/commit/title edited in place without a new run being created, and a retried step
/// that doesn't move its job's status.
fn pipeline_run_matches(a: &PipelineRun, b: &PipelineRun) -> bool {
    a.status == b.status
        && a.stages.len() == b.stages.len()
        && a.stages.iter().zip(&b.stages).all(|(x, y)| {
            x.status == y.status
                && x.jobs.len() == y.jobs.len()
                && x.jobs.iter().zip(&y.jobs).all(|(jx, jy)| jx.status == jy.status)
        })
}

/// Mirrors [`pr_detail_unchanged`]. `PipelineApproval` derives `PartialEq` (unlike the other
/// domain types this file compares against a landed detail), so approvals compare exactly —
/// order and membership both matter for a gate list. `PipelineRun` doesn't, so it goes through
/// [`pipeline_run_matches`], which always includes `status`: a status change that fails to
/// repaint is the one failure the stale-run design (see `open_pipeline_for`) can't afford.
///
/// `confirmed_approvals` is what the fetch actually answered with, not the cache-merged value:
/// gates the cache remembers never reach the view, so their differing from it is not a change
/// worth repainting for.
fn pipeline_detail_unchanged(
    v: &PipelineView,
    d: &PipelineDetail,
    confirmed_approvals: Option<&[PipelineApproval]>,
) -> bool {
    pipeline_run_matches(&v.run, &d.run)
        && confirmed_approvals.is_none_or(|fresh| fresh == v.approvals)
        && v.supports_approvals == d.supports_approvals
        && v.can_respond_approvals == d.can_respond_approvals
}


/// The rows one PR view shows, derived from the pool — no network, no `&self`, so the
/// background fetch can use it too.
///
/// Reproduces what a per-filter `list` returned, step for step: the pool arrives in provider
/// order (fetched uncapped, so `sort_and_cap` sorted it but truncated nothing), this filters it
/// without reordering, and then caps. Order is preserved rather than re-sorted because
/// `pr_sort` defaults to `None`, which means "provider order" — imposing one here would
/// silently change the list for everyone who never chose a sort.
fn derive_pool_rows(pool: &PrPool, filter: PullRequestFilter, completed: bool) -> Vec<PrRow> {
    let mut per_connection: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::new();
    for row in pool.rows(completed) {
        let me = pool.me.get(&row.connection_id).and_then(|m| m.as_deref());
        if !pull_request_matches(&row.pr, filter, me) {
            continue;
        }
        // Capped per connection, not across all of them: each feed used to run its own capped
        // `list`, so one busy connection must not crowd another off the list.
        let kept = per_connection.entry(row.connection_id.as_str()).or_default();
        if *kept >= PR_VIEW_CAP {
            continue;
        }
        *kept += 1;
        out.push(row.clone());
    }
    out
}

/// Fetches the unfiltered pool both `include_completed` variants of every PR view derive from.
///
/// Two `list` calls per connection, where there used to be four — the list section, both
/// Launchpad buckets and the notification scan each ran their own. None of them carries a
/// filter: the rows are identical whichever view asked for them.
///
/// `limit: None` is deliberate. Providers cap **after** filtering (`sort_and_cap`), so a pool
/// capped at 50 here would make a derived "Mine" a subset of the newest 50 overall, rather than
/// the newest 50 of your own — fewer rows than the per-filter fetch returned. Uncapped, the pool
/// holds exactly what the provider filtered over, and the derived view caps it the same way.
/// The per-repository page size is unaffected: providers use `limit.unwrap_or(50)` for that.
///
/// `decorate: false` for the same reason. GitHub decorates the first 25 rows it is about to
/// return, and on an unfiltered pool those are the first 25 of *All* — so a derived "Mine" would
/// lose its +/- and check columns past whatever overlapped. Decoration moves to a per-row pass
/// over the rows a view actually shows; see [`App::request_pr_decorations`].
async fn fetch_pr_pool(deps: &AppDeps, errors: &mut Vec<String>) -> (PrPool, bool) {
    let mut pool = PrPool::default();
    let mut ok = true;
    let feeds = match deps.sections.pull_request_feeds().await {
        Ok(feeds) => feeds,
        Err(e) => {
            push_reload_error(errors, format!("PRs: {e}"), DIAG_RELOAD_PULL_REQUESTS);
            return (pool, false);
        }
    };
    for feed in feeds {
        let (provider, name, conn_id) = feed_tag(&feed.connection);
        // An identity this fetch cannot establish is not fatal: `pull_request_matches` treats
        // `None` as "cannot filter" and passes rows through, which is what `list` already did.
        match feed.source.current_user().await {
            Ok(me) => {
                pool.me.insert(conn_id.clone(), me);
            }
            Err(e) => {
                ok = false;
                pool.me.insert(conn_id.clone(), None);
                push_reload_error(errors, format!("Signed-in user ({name}): {e}"), DIAG_RELOAD_PULL_REQUESTS);
            }
        }
        pool.needs_decoration.insert(conn_id.clone(), feed.source.list_omits_decoration());
        for completed in [false, true] {
            let query = PullRequestQuery {
                filter: PullRequestFilter::All,
                include_completed: completed,
                limit: None,
                decorate: false,
            };
            match feed.source.list(&query).await {
                Ok(list) => {
                    let rows = list.into_iter().map(|pr| PrRow {
                        connection_id: conn_id.clone(),
                        connection: name.clone(),
                        provider,
                        pr,
                    });
                    if completed {
                        pool.completed.extend(rows);
                    } else {
                        pool.open.extend(rows);
                    }
                }
                Err(e) => {
                    ok = false;
                    push_reload_error(errors, format!("PRs ({name}): {e}"), DIAG_RELOAD_PULL_REQUESTS);
                }
            }
        }
    }
    (pool, ok)
}

/// The PR-notification scan, firing pings for newly-seen review requests / vote changes
/// and returning the fresh seen-sets. Mirrors the inline logic but takes a snapshot so
/// it can run off the render loop. `review` is this reload's review-requested list, fetched
/// once in [`App::fetch_all`] and shared with the Launchpad.
async fn scan_pr_notifications(deps: &AppDeps, p: &ReloadParams, review: &[PrRow], mine: &[PrRow]) -> Option<PrScan> {
    let want_review = p.notifications.review_requested;
    let want_votes = p.notifications.pr_approved || p.notifications.pr_changes_requested;
    if !want_review && !want_votes {
        return None;
    }
    let feeds = detail_or_none(
        deps.sections.pull_request_feeds().await,
        DIAG_NOTIFICATION_SCAN_FEEDS,
    )?;
    if feeds.is_empty() {
        return None;
    }
    let seeded = p.scan_seeded;
    let mut review_now: HashSet<(String, String)> = HashSet::new();
    let mut votes_now: HashMap<(String, String), (bool, bool)> = HashMap::new();
    for feed in &feeds {
        let conn = feed.connection.connection_id().to_string();
        if want_review {
            // `review` is the review-requested list `fetch_all` already fetched for the
            // Launchpad. This used to issue that identical query a second time.
            for row in review.iter().filter(|row| row.connection_id == conn) {
                let key = (conn.clone(), row.pr.id.clone());
                if seeded && !p.review_seen.contains(&key) {
                    p.notifier.notify("Review requested", &pr_label(&row.pr));
                }
                review_now.insert(key);
            }
        }
        if want_votes {
            // Derived from the same pool as `review`, on the query this used to run itself.
            for pr in mine.iter().filter(|row| row.connection_id == conn).map(|row| &row.pr) {
                let key = (conn.clone(), pr.id.clone());
                if seeded {
                    let (approved, changes) = pr_review_transitions(p.pr_review_seen.get(&key).copied(), pr);
                    if approved && p.notifications.pr_approved {
                        p.notifier.notify("Your PR was approved", &pr_label(pr));
                    }
                    if changes && p.notifications.pr_changes_requested {
                        p.notifier.notify("Changes requested on your PR", &pr_label(pr));
                    }
                }
                votes_now.insert(key, pr_vote_flags(pr));
            }
        }
    }
    Some(PrScan {
        review_seen: want_review.then_some(review_now),
        pr_review_seen: want_votes.then_some(votes_now),
    })
}

/// PR statuses in display order — drives the "Show statuses" checklist and the header.
const PR_STATUS_ORDER: [PullRequestStatus; 4] =
    [PullRequestStatus::Open, PullRequestStatus::Draft, PullRequestStatus::Merged, PullRequestStatus::Closed];

fn pr_status_key(s: PullRequestStatus) -> &'static str {
    match s {
        PullRequestStatus::Open => "Open",
        PullRequestStatus::Draft => "Draft",
        PullRequestStatus::Merged => "Merged",
        PullRequestStatus::Closed => "Closed",
    }
}

fn parse_pr_status(s: &str) -> Option<PullRequestStatus> {
    PR_STATUS_ORDER.iter().copied().find(|&st| pr_status_key(st) == s)
}

/// Human summary of the shown-status set, in canonical order (e.g. "Open, Merged").
pub fn pr_status_summary(shown: &HashSet<PullRequestStatus>) -> String {
    if shown.len() == PR_STATUS_ORDER.len() {
        return "all statuses".into();
    }
    PR_STATUS_ORDER.iter().filter(|s| shown.contains(s)).map(|&s| pr_status_key(s)).collect::<Vec<_>>().join(", ")
}

fn parse_pr_filter(s: Option<&str>) -> PullRequestFilter {
    match s {
        Some("mine") => PullRequestFilter::Mine,
        Some("review") => PullRequestFilter::ReviewRequested,
        _ => PullRequestFilter::All,
    }
}

/// The persisted key for a PR base filter (inverse of [`parse_pr_filter`]).
fn pr_filter_key(f: PullRequestFilter) -> &'static str {
    match f {
        PullRequestFilter::Mine => "mine",
        PullRequestFilter::ReviewRequested => "review",
        PullRequestFilter::All => "all",
    }
}

/// The built-in views seeded for a section that has none saved.
fn default_views(section: usize) -> Vec<SavedView> {
    let v = |name: &str, filter: Option<&str>| SavedView {
        name: name.into(),
        filter: filter.map(Into::into),
        query: String::new(),
        sort: None,
        hidden_states: Vec::new(),
    };
    match section {
        0 => vec![v("All", Some("all")), v("Mine", Some("mine")), v("Review", Some("review"))],
        _ => vec![v("All", None)],
    }
}

fn wi_query() -> WorkItemQuery {
    // Work Items only ever shows items assigned to the authenticated user
    // (resolved from the token by each provider: @me / currentUser() / isMe).
    WorkItemQuery { mine_only: true, include_completed: false, limit: Some(50) }
}

/// One query per subscribed definition, or a single catch-all when auto-discovering.
///
/// A subscribed definition id is only unique within its repository, so each query is addressed at
/// the repository discovery says the definition belongs to — otherwise a connection spanning
/// several would ask every one of them about a definition only one of them has.
fn feed_queries(sub: &forgetop_core::config::PipelineSubscription, defs: &[PipelineDefinition]) -> Vec<PipelineRunQuery> {
    if sub.auto_discover_all || sub.definition_ids.is_empty() {
        return vec![PipelineRunQuery { definition_id: None, repository: None, branch: None, limit: Some(20) }];
    }
    sub.definition_ids
        .iter()
        .map(|id| PipelineRunQuery {
            repository: defs.iter().find(|d| &d.id == id).and_then(|d| d.repository.clone()),
            definition_id: Some(id.clone()),
            branch: None,
            limit: Some(10),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_deps() -> AppDeps {
        use forgetop_core::config::InMemoryConfigStore;
        use forgetop_core::secret::InMemorySecretStore;
        use forgetop_core::service::ConnectionResolver;

        let registry = Arc::new(ProviderRegistry::new(Vec::new()));
        let secrets = Arc::new(InMemorySecretStore::default());
        let config = Arc::new(ConfigService::new(
            Arc::new(InMemoryConfigStore::default()),
            secrets.clone(),
            registry.clone(),
        ));
        let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry, secrets));
        AppDeps {
            sections: Arc::new(SectionService::new(config.clone(), resolver.clone())),
            health: Arc::new(ConnectionHealthService::new(config.clone(), resolver)),
            config,
            // Never the user's real cache file: a test run must not read or rewrite it.
            cache: Arc::new(CacheStore::disabled()),
        }
    }

    /// A live store that still never touches the disk — `get`/`put` work purely in memory, and
    /// only `load`/`flush` (which these tests don't call) would open the path.
    fn memory_cache() -> Arc<CacheStore> {
        Arc::new(CacheStore::new(std::env::temp_dir().join("forgetop-tui-tests-never-written.json")))
    }

    fn deps_with_cache(cache: Arc<CacheStore>) -> AppDeps {
        AppDeps { cache, ..test_deps() }
    }

    #[test]
    fn feedback_dispatch_uses_the_exact_github_issue_form_destination() {
        let mut app = App::new("slate");
        let mut opened = None;
        app.open_feedback_with(
            |target| {
                opened = Some(target.to_owned());
                Ok::<(), std::io::Error>(())
            },
            |_, _| {},
        );

        assert_eq!(
            opened.as_deref(),
            Some("https://github.com/magna-nz/forgetop/issues/new?template=feedback.yml")
        );
        assert_eq!(
            app.toast.as_deref(),
            Some("Opening feedback form in your browser…")
        );
    }

    #[test]
    fn feedback_open_failure_is_non_blocking_and_logs_only_safe_constants() {
        let mut app = App::new("slate");
        let mut logged = None;
        app.open_feedback_with(
            |_| Err(std::io::Error::other("browser unavailable")),
            |context, message| logged = Some((context.to_owned(), message.to_owned())),
        );

        assert_eq!(
            logged,
            Some((
                FEEDBACK_OPEN_FAILURE_CONTEXT.to_owned(),
                FEEDBACK_OPEN_FAILURE_MESSAGE.to_owned()
            ))
        );
        let (context, message) = logged.unwrap();
        assert!(!context.contains("github.com"));
        assert!(!message.contains("github.com"));
        assert_eq!(
            app.toast.as_deref(),
            Some("Couldn't open feedback form: browser unavailable")
        );
        assert!(!app.should_quit, "a browser failure remains non-blocking");
    }

    #[tokio::test]
    async fn ordinary_uppercase_f_dispatches_to_the_injected_feedback_opener() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.feedback_opener = |_| Ok(());

        app.on_key(Key::Char('F'), &deps).await;

        assert_eq!(
            app.toast.as_deref(),
            Some("Opening feedback form in your browser…")
        );
    }

    #[test]
    fn refresh_and_action_diagnostics_never_receive_private_provider_errors() {
        let private_error = "failed https://github.com/acme/private-repo?t=dashboard-secret";

        let mut app = App::new("slate");
        let mut refresh_logs = Vec::new();
        app.apply_reloaded_with_logger(
            Reloaded {
                pr_pool: PrPool::default(),
                wis: Vec::new(),
                pipes: Vec::new(),
                inbox: Vec::new(),
                health: Vec::new(),
                scan: None,
                open_pipeline: None,
                errors: vec![private_error.into()],
                catalog: None,
                sections_ok: SectionsOk::complete(),
                requested_at: Utc::now(),
            },
            &test_deps(),
            |context, message| refresh_logs.push((context.to_owned(), message.to_owned())),
        );
        assert!(
            app.status.contains(private_error),
            "the transient UI keeps useful detail"
        );
        assert_eq!(
            refresh_logs,
            vec![(DIAG_REFRESH.to_owned(), DIAG_FAILURE_MESSAGE.to_owned())]
        );

        let mut action_logs = Vec::new();
        app.toast_error_with_logger(private_error.into(), |context, message| {
            action_logs.push((context.to_owned(), message.to_owned()))
        });
        assert_eq!(app.toast.as_deref(), Some(private_error));
        assert_eq!(
            action_logs,
            vec![(DIAG_ACTION.to_owned(), DIAG_FAILURE_MESSAGE.to_owned())]
        );

        for (context, message) in refresh_logs.into_iter().chain(action_logs) {
            assert!(!context.contains("private-repo") && !context.contains("dashboard-secret"));
            assert!(!message.contains("private-repo") && !message.contains("dashboard-secret"));
        }
    }

    #[test]
    fn detail_fallback_logs_only_the_safe_operation_constant() {
        let private_error = "failed https://github.com/acme/private-repo?t=dashboard-secret";
        let mut logged = None;
        let rows: Vec<String> = detail_or_default_with_logger(
            Err(private_error),
            DIAG_PR_THREADS,
            |context, message| logged = Some((context.to_owned(), message.to_owned())),
        );

        assert!(rows.is_empty());
        assert_eq!(
            logged,
            Some((DIAG_PR_THREADS.to_owned(), DIAG_FAILURE_MESSAGE.to_owned()))
        );
    }

    #[test]
    fn failed_inbox_action_returns_detail_but_logs_only_safe_constants() {
        let private_error =
            "failed https://github.com/acme/private-repo?t=dashboard-secret";
        let mut logged = None;

        let message = inbox_action_error_with_logger(
            "Couldn't mark notification read",
            private_error,
            DIAG_INBOX_MARK_READ,
            |context, message| logged = Some((context.to_owned(), message.to_owned())),
        );

        assert!(message.contains(private_error), "the transient toast keeps useful detail");
        assert_eq!(
            logged,
            Some((
                DIAG_INBOX_MARK_READ.to_owned(),
                DIAG_FAILURE_MESSAGE.to_owned()
            ))
        );
        let (context, message) = logged.unwrap();
        assert!(!context.contains("private-repo") && !context.contains("dashboard-secret"));
        assert!(!message.contains("private-repo") && !message.contains("dashboard-secret"));
    }

    #[test]
    fn inline_reload_error_preserves_ui_detail_but_logs_only_safe_constants() {
        let private_error =
            "failed https://github.com/acme/private-repo?t=dashboard-secret";
        let mut errors = Vec::new();
        let mut logged = None;

        push_reload_error_with_logger(
            &mut errors,
            private_error.into(),
            DIAG_RELOAD_PULL_REQUESTS,
            |context, message| logged = Some((context.to_owned(), message.to_owned())),
        );

        assert_eq!(errors, vec![private_error]);
        assert_eq!(
            logged,
            Some((
                DIAG_RELOAD_PULL_REQUESTS.to_owned(),
                DIAG_FAILURE_MESSAGE.to_owned()
            ))
        );
    }

    #[tokio::test]
    async fn failed_inbox_mark_keeps_the_row_unread_and_does_not_report_success() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.inbox = vec![NotifRow {
            connection_id: "missing".into(),
            connection: "Missing".into(),
            provider: ProviderType::GitHub,
            notification: Notification {
                repository: None,
                id: "n1".into(),
                kind: NotificationKind::Mention,
                item_type: NotificationItemType::PullRequest,
                item_id: None,
                title: "Private notification".into(),
                context: String::new(),
                url: None,
                unread: true,
                updated_at: None,
            },
        }];

        app.mark_selected_inbox_read(&deps).await;

        assert!(app.inbox[0].notification.unread, "failed persistence keeps local state unread");
        assert!(
            app.toast
                .as_deref()
                .is_some_and(|toast| toast.starts_with("Couldn't mark notification read:")),
            "the failed action shows a detailed error instead of success"
        );
        assert_ne!(app.toast.as_deref(), Some("Marked read"));
    }

    /// Removing a connection used to leave its rows on screen until the background refetch
    /// answered, so the user watched their own action take effect seconds late. Everything the
    /// connection contributed must be gone by the time `purge_connection` returns.
    #[tokio::test]
    async fn removing_a_connection_clears_its_rows_before_the_refetch() {
        let deps = test_deps();
        let mut app = App::new("slate");

        let row = |conn: &str| PrRow {
            connection_id: conn.into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            pr: pr(None),
        };
        let notif = |conn: &str| NotifRow {
            connection_id: conn.into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            notification: Notification {
                id: conn.into(),
                kind: NotificationKind::Mention,
                item_type: NotificationItemType::WorkItem,
                item_id: None,
                repository: None,
                title: conn.into(),
                context: "c".into(),
                url: None,
                unread: true,
                updated_at: None,
            },
        };

        app.prs = vec![row("gone"), row("kept")];
        app.lp_prs_mine = vec![row("gone"), row("kept")];
        app.lp_prs_review = vec![row("gone")];
        app.inbox = vec![notif("gone"), notif("kept")];
        app.inbox_sel = 1;
        app.health = vec![health("gone"), health("kept")];
        app.repo_catalog.insert("gone".into(), RepositoryPage { repositories: vec!["a/b".into()], truncated: false });
        app.repo_catalog.insert("kept".into(), RepositoryPage { repositories: vec!["c/d".into()], truncated: false });

        app.purge_connection("gone", &deps);

        let ids = |rows: &[PrRow]| rows.iter().map(|r| r.connection_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&app.prs), ["kept"], "list rows must go immediately");
        assert_eq!(ids(&app.lp_prs_mine), ["kept"], "Launchpad 'mine' rows must go too");
        assert!(app.lp_prs_review.is_empty(), "Launchpad 'review' rows must go too");
        assert_eq!(app.inbox.len(), 1, "inbox notifications must go too");
        assert_eq!(app.inbox.iter().map(|r| r.connection_id.clone()).collect::<Vec<_>>(), ["kept"]);
        assert_eq!(app.health.iter().map(|h| h.connection.id.clone()).collect::<Vec<_>>(), ["kept"]);
        assert!(!app.repo_catalog.contains_key("gone"), "the repo catalog is keyed by connection");
        assert!(app.repo_catalog.contains_key("kept"), "other connections are untouched");
        // A selection pointing past the shortened list would panic the table widget.
        assert!(app.inbox_sel < app.inbox.len());
    }

    /// The on-disk cache has to be purged as well: `seed_section` is unfiltered, so rows left
    /// behind come back on the next launch if the app closes before the reload lands.
    #[test]
    fn removing_a_connection_purges_its_rows_from_the_cache_too() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheStore::new(dir.path().join("cache.json"));
        let row = |conn: &str| PrRow {
            connection_id: conn.into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            pr: pr(None),
        };
        let fetched_at = Utc::now() - chrono::Duration::hours(3);
        // The list the user is looking at, and one they are not — both are seeded at launch.
        let on_screen = prs_cache_key(PullRequestFilter::All, false);
        let other = prs_cache_key(PullRequestFilter::Mine, true);
        cache.put(&on_screen, &vec![row("gone"), row("kept")], fetched_at);
        cache.put(&other, &vec![row("gone")], fetched_at);

        purge_cached_rows(&cache, "gone");

        let seeded = cache.get::<Vec<PrRow>>(&on_screen).expect("entry survives the purge");
        assert_eq!(seeded.value.len(), 1);
        assert_eq!(seeded.value[0].connection_id, "kept");
        assert_eq!(seeded.fetched_at, fetched_at, "dropping rows must not make the rest look fresher");
        assert!(
            cache.get::<Vec<PrRow>>(&other).expect("entry survives").value.is_empty(),
            "every PR filter/completed combination is purged, not just the visible one"
        );
    }

    /// The inline `reload_*` methods run on the key-handler path, so whatever they leave in the
    /// list is what the user is looking at for the length of the round trip. Clearing first put
    /// an empty screen there; building into a local vec and swapping at the end does not.
    #[test]
    fn an_inline_reload_that_failed_leaves_the_rows_that_are_on_screen() {
        let mut rows = vec![pr_row(pr(None))];
        let mut errors = Vec::new();
        let before = errors.len();
        errors.push("PRs (GitHub): 502".to_string());

        take_inline_section(&mut rows, Vec::new(), &errors, before);

        assert_eq!(rows.len(), 1, "an outage erased the answer, not the pull requests");
    }

    /// The other half of the rule: a reload that actually answered is authoritative, so an empty
    /// answer really does mean the list is empty — otherwise the last row of a section could
    /// never be seen to go.
    #[test]
    fn an_inline_reload_that_answered_replaces_the_rows_even_with_nothing() {
        let mut rows = vec![pr_row(pr(None))];
        let errors: Vec<String> = Vec::new();

        take_inline_section(&mut rows, Vec::new(), &errors, 0);

        assert!(rows.is_empty());
    }

    /// One feed of several failing is partial live data: the failure is already in the status
    /// line, and some fresh rows beat every stale one.
    #[test]
    fn a_partly_failed_inline_reload_still_takes_the_rows_that_came_back() {
        let mut rows = vec![pr_row(pr(Some("https://old")))];
        let errors = vec!["PRs (GitLab): 502".to_string()];

        take_inline_section(&mut rows, vec![pr_row(pr(Some("https://fresh")))], &errors, 0);

        assert_eq!(rows[0].pr.url.as_deref(), Some("https://fresh"));
    }

    /// An error the *caller* was already carrying must not be read as this section having
    /// failed — the `errors` vec is shared across the sections a handler reloads.
    #[test]
    fn an_inline_reload_judges_only_the_failures_it_added_itself() {
        let mut rows = vec![pr_row(pr(None))];
        let errors = vec!["Work items (GitHub): 502".to_string()];

        take_inline_section(&mut rows, Vec::new(), &errors, errors.len());

        assert!(rows.is_empty(), "the work-item failure says nothing about the PR fetch");
    }

    /// A decided gate has to leave the run the moment the provider accepts it, or the picker
    /// keeps offering a decision that has already been made.
    #[test]
    fn deciding_a_gate_drops_it_from_the_open_run_without_guessing_the_runs_status() {
        let mut app = App::new("slate");
        let mut view = PipelineView::new(
            "CI".into(),
            pipeline_run("r1", PipelineRunStatus::Running, vec![]),
            "c".into(),
            ProviderType::GitHub,
            "ci".into(),
            None,
        );
        view.supports_approvals = true;
        view.can_respond_approvals = true;
        view.approvals = vec![approval("g1", true), approval("g2", true)];
        app.screen = Screen::Pipeline(Box::new(view));
        app.pipes = vec![pipe_row("r1", PipelineRunStatus::Running, true)];

        app.drop_decided_approval("c", &ItemRef::new("r1"), "g1");

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert_eq!(v.approvals.iter().map(|a| a.id.clone()).collect::<Vec<_>>(), ["g2"]);
        assert_eq!(
            v.run.status,
            PipelineRunStatus::Running,
            "what an approval does to a run is the provider's to say, not ours to fake"
        );
        assert!(app.pipes[0].awaiting_approval, "a second gate still wants the user");

        app.drop_decided_approval("c", &ItemRef::new("r1"), "g2");

        assert!(!app.pipes[0].awaiting_approval, "the badge goes once nothing is left to answer");
    }

    /// The badge means "something here still wants *you*", so a gate left behind that the user
    /// can only look at must not keep it lit.
    #[test]
    fn a_gate_the_user_cannot_answer_does_not_keep_the_approval_badge_lit() {
        let mut app = App::new("slate");
        let mut view = PipelineView::new(
            "CI".into(),
            pipeline_run("r1", PipelineRunStatus::Running, vec![]),
            "c".into(),
            ProviderType::GitHub,
            "ci".into(),
            None,
        );
        view.approvals = vec![approval("g1", true), approval("g2", false)];
        app.screen = Screen::Pipeline(Box::new(view));
        app.pipes = vec![pipe_row("r1", PipelineRunStatus::Running, true)];

        app.drop_decided_approval("c", &ItemRef::new("r1"), "g1");

        assert!(!app.pipes[0].awaiting_approval);
        assert!(
            !app.lp.iter().any(|e| e.bucket == launchpad::Bucket::ApprovalsWaiting),
            "the Command Center's approvals bucket is rebuilt off that flag"
        );
    }

    /// A run whose view has been navigated away from can't be reasoned about locally, so the
    /// list row keeps its badge: a badge that lingers a second is recoverable, a missing one
    /// hides a gate that still needs the user.
    #[test]
    fn a_decision_made_against_a_run_that_is_no_longer_open_leaves_the_badge_alone() {
        let mut app = App::new("slate");
        app.pipes = vec![pipe_row("r1", PipelineRunStatus::Running, true)];

        app.drop_decided_approval("c", &ItemRef::new("r1"), "g1");

        assert!(app.pipes[0].awaiting_approval);
    }

    /// An action taken from the drill-in has to land on the list row too, or pressing Esc walks
    /// back onto the state the user just changed.
    #[test]
    fn a_work_item_state_change_lands_on_the_open_view_and_the_row_behind_it() {
        let mut app = App::new("slate");
        let item = wi(None);
        app.wis = vec![wi_row(wi(None)), wi_row(WorkItem { id: "other".into(), ..wi(None) })];
        app.screen = Screen::WiView(Box::new(WiView {
            timeline: Vec::new(),
            connection_id: "c".into(),
            wi: item.clone(),
            threads: Vec::new(),
            scroll: 0,
        }));

        app.apply_wi_state("c", &item.item_ref(), "In Progress");

        let Screen::WiView(v) = &app.screen else { panic!("expected WiView") };
        assert_eq!(v.wi.state, "In Progress");
        assert_eq!(app.wis[0].wi.state, "In Progress", "the row behind the view moves with it");
        assert_eq!(
            app.wis[0].wi.state_category,
            WorkItemStateCategory::Backlog,
            "the category is the provider's mapping of the state, not ours to derive"
        );
        assert_eq!(app.wis[1].wi.state, "Todo", "another item on the same connection is untouched");
    }

    /// Unbinding a connection is knowable straight away; binding one is not, because it has
    /// contributed no rows yet.
    #[test]
    fn unbinding_a_connection_drops_its_rows_before_the_refetch() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let row = |conn: &str| PrRow { connection_id: conn.into(), ..pr_row(pr(None)) };
        app.prs = vec![row("gone"), row("kept")];
        app.lp_prs_mine = vec![row("gone"), row("kept")];
        app.lp_prs_review = vec![row("gone")];
        app.wis = vec![WiRow { connection_id: "gone".into(), ..wi_row(wi(None)) }];

        app.drop_unbound_rows(0, &HashSet::from(["kept".to_string()]), &deps);

        let ids = |rows: &[PrRow]| rows.iter().map(|r| r.connection_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&app.prs), ["kept"]);
        assert_eq!(ids(&app.lp_prs_mine), ["kept"], "the Command Center's PR feeds go too");
        assert!(app.lp_prs_review.is_empty());
        assert_eq!(app.wis.len(), 1, "a PR binding says nothing about the work-item section");

        app.drop_unbound_rows(1, &HashSet::new(), &deps);

        assert!(app.wis.is_empty());
    }

    /// Narrowing the scope is knowable; widening it is not, so only rows outside the new set go.
    #[test]
    fn narrowing_the_repository_scope_drops_the_rows_it_no_longer_covers() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let in_repo = |conn: &str, repo: Option<&str>| PrRow {
            connection_id: conn.into(),
            pr: PullRequest { repository: repo.map(Into::into), ..pr(None) },
            ..pr_row(pr(None))
        };
        app.prs = vec![
            in_repo("c", Some("acme/pay")),
            in_repo("c", Some("acme/legacy")),
            // The same repository in the host-qualified spelling: comparing the two by hand is
            // exactly the silent mismatch `repo::matches_scope_entry` exists to prevent.
            in_repo("c", Some("github.com/acme/pay")),
            in_repo("c", None),
            in_repo("other", Some("acme/legacy")),
        ];
        app.lp_prs_mine = vec![in_repo("c", Some("acme/legacy"))];
        // Azure addresses work items by Team Project, so this row's "repository" is a project
        // name that no scope entry could ever match.
        app.wis = vec![WiRow {
            provider: ProviderType::AzureDevOps,
            wi: WorkItem { repository: Some("Payments".into()), ..wi(None) },
            ..wi_row(wi(None))
        }];

        app.narrow_to_repo_scope("c", &["acme/pay".to_string()], &deps);

        let repos = |rows: &[PrRow]| rows.iter().map(|r| r.pr.repository.clone()).collect::<Vec<_>>();
        assert_eq!(
            repos(&app.prs),
            [
                Some("acme/pay".to_string()),
                Some("github.com/acme/pay".to_string()),
                None,
                Some("acme/legacy".to_string())
            ],
            "only this connection's out-of-scope rows go; an unaddressed row can't be placed"
        );
        assert!(app.lp_prs_mine.is_empty(), "the Command Center's PR feeds narrow too");
        assert_eq!(app.wis.len(), 1, "a project-addressed section is left to the reload");
    }

    /// `self.inbox` is this session; the cache is the *next* launch. Without the rewrite,
    /// quitting before the next poll repaints a notification the user already read as unread.
    #[test]
    fn marking_a_notification_read_rewrites_the_cached_inbox_without_redating_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheStore::new(dir.path().join("cache.json"));
        let fetched_at = Utc::now() - chrono::Duration::hours(3);
        cache.put(CACHE_KEY_INBOX, &vec![notif_row("n1"), notif_row("n2")], fetched_at);

        mark_cached_inbox_read(&cache, |row| row.notification.id == "n1");

        let entry = cache.get::<Vec<NotifRow>>(CACHE_KEY_INBOX).expect("entry survives");
        assert!(!entry.value[0].notification.unread, "the one that was read stays read");
        assert!(entry.value[1].notification.unread, "the others are untouched");
        assert_eq!(entry.fetched_at, fetched_at, "reading a notification doesn't make the list fresher");

        mark_cached_inbox_read(&cache, |_| true);

        let entry = cache.get::<Vec<NotifRow>>(CACHE_KEY_INBOX).expect("entry survives");
        assert!(entry.value.iter().all(|r| !r.notification.unread), "mark-all reaches the cache too");
    }

    /// A config store that loads fine and refuses every write, for the persist-failure paths.
    /// Nothing else in the crate can produce one: `InMemoryConfigStore` always succeeds.
    struct RefusingConfigStore;

    #[async_trait::async_trait]
    impl forgetop_core::config::ConfigStore for RefusingConfigStore {
        async fn load(&self) -> forgetop_core::Result<forgetop_core::config::ForgetopConfig> {
            Ok(forgetop_core::config::ForgetopConfig::default())
        }
        async fn save(&self, _config: &forgetop_core::config::ForgetopConfig) -> forgetop_core::Result<()> {
            Err(forgetop_core::Error::Config("config directory is read-only".into()))
        }
    }

    /// `test_deps`, but every config write fails.
    fn deps_that_cannot_persist() -> AppDeps {
        use forgetop_core::secret::InMemorySecretStore;
        use forgetop_core::service::ConnectionResolver;

        let registry = Arc::new(ProviderRegistry::new(Vec::new()));
        let secrets = Arc::new(InMemorySecretStore::default());
        let config = Arc::new(ConfigService::new(Arc::new(RefusingConfigStore), secrets.clone(), registry.clone()));
        let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry, secrets));
        AppDeps {
            sections: Arc::new(SectionService::new(config.clone(), resolver.clone())),
            health: Arc::new(ConnectionHealthService::new(config.clone(), resolver)),
            config,
            cache: Arc::new(CacheStore::disabled()),
        }
    }

    fn saved_view(name: &str) -> SavedView {
        SavedView { name: name.into(), filter: None, query: String::new(), sort: None, hidden_states: Vec::new() }
    }

    /// The view stays on screen — writing local state first is deliberate and matches the rest of
    /// the app — but "Saved view" would be a lie the user only finds out about on the next launch,
    /// when the view they think they saved is gone.
    #[tokio::test]
    async fn a_view_that_could_not_be_persisted_says_so_instead_of_reporting_success() {
        let deps = deps_that_cannot_persist();
        let mut app = App::new("slate");

        app.save_view("Mine only".into(), &deps).await;

        assert_eq!(app.views[0].len(), 1, "the optimistic local write still stands");
        let toast = app.toast.as_deref().expect("a failed save has to say something");
        assert!(toast.contains("Couldn't save view"), "{toast}");
        assert!(!toast.contains("Saved view"), "a refused write must not report success: {toast}");
    }

    /// Same for the other direction, and the toast has to survive `apply_view` — which sets a
    /// "View: …" toast of its own after the persist has already been attempted.
    #[tokio::test]
    async fn a_view_that_could_not_be_deleted_says_so_rather_than_announcing_the_delete() {
        let deps = deps_that_cannot_persist();
        let mut app = App::new("slate");
        app.views[0] = vec![saved_view("All"), saved_view("Mine")];
        app.view_idx[0] = 1;

        app.delete_view(&deps).await;

        let toast = app.toast.as_deref().expect("a failed delete has to say something");
        assert!(toast.contains("Couldn't delete view"), "{toast}");
        assert!(!toast.contains("Deleted view"), "a refused write must not report success: {toast}");
        assert!(!toast.starts_with("View:"), "apply_view's toast must not bury the failure: {toast}");
    }

    fn health(id: &str) -> ConnectionHealth {
        ConnectionHealth {
            connection: forgetop_core::provider::Connection {
                id: id.into(),
                provider_type: ProviderType::GitHub,
                display_name: id.into(),
                base_url: None,
                organization: None,
                project: None,
                repository: None,
                username: None,
                credential_ref: Some(id.into()),
                repo_scope: None,
            },
            healthy: true,
        }
    }

    fn notif_row(id: &str) -> NotifRow {
        NotifRow {
            connection_id: "c".into(),
            connection: "GitHub".into(),
            provider: ProviderType::GitHub,
            notification: Notification {
                repository: None,
                id: id.into(),
                kind: NotificationKind::Mention,
                item_type: NotificationItemType::WorkItem,
                item_id: None,
                title: "t".into(),
                context: "c".into(),
                url: None,
                unread: true,
                updated_at: None,
            },
        }
    }

    /// A reload carrying just the PR list, keyed as the live fetch would have keyed it.
    /// The rows go into the pool, which is what every view is now derived from.
    fn reloaded(prs: Vec<PrRow>) -> Reloaded {
        Reloaded { pr_pool: test_pool(prs), ..reloaded_with_health(Vec::new()) }
    }

    /// A pool holding `rows` under both `include_completed` variants, with every connection
    /// identified as "me" so `Mine` / `ReviewRequested` derive the way the provider would.
    fn test_pool(rows: Vec<PrRow>) -> PrPool {
        let me = rows.iter().map(|r| (r.connection_id.clone(), Some("me".to_string()))).collect();
        let needs_decoration = rows.iter().map(|r| (r.connection_id.clone(), false)).collect();
        PrPool { open: rows.clone(), completed: rows, me, needs_decoration }
    }

    /// An otherwise-empty reload carrying just the connection health, which is the
    /// signal the waiting card and the first-run hint both key off.
    fn reloaded_with_health(health: Vec<ConnectionHealth>) -> Reloaded {
        Reloaded {
            pr_pool: PrPool::default(),
            wis: Vec::new(),
            pipes: Vec::new(),
            inbox: Vec::new(),
            health,
            scan: None,
            open_pipeline: None,
            errors: Vec::new(),
            catalog: None,
            sections_ok: SectionsOk::complete(),
            requested_at: Utc::now(),
        }
    }

    #[test]
    fn discovery_is_requested_only_until_the_catalog_has_landed() {
        let mut app = App::new("slate");
        // Empty catalog: this fetch should bring discovery back with it.
        assert!(app.reload_params().seed_catalog);

        app.repo_catalog.insert("gh-1".into(), RepositoryPage::default());
        // Already seeded: the 30s poll must not re-discover every repository forever.
        assert!(!app.reload_params().seed_catalog);
    }

    #[test]
    fn a_fetch_that_carried_discovery_replaces_the_catalog() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let mut catalog = HashMap::new();
        catalog.insert("gh-1".into(), RepositoryPage::default());

        let mut r = reloaded_with_health(Vec::new());
        r.catalog = Some(catalog);
        app.apply_reloaded(r, &deps);
        assert_eq!(app.repo_catalog.len(), 1, "discovery from the background fetch must land");

        // A later poll carries no catalog; that must leave the existing one alone rather
        // than wiping the scope indicator's denominator.
        app.apply_reloaded(reloaded_with_health(Vec::new()), &deps);
        assert_eq!(app.repo_catalog.len(), 1, "a catalog-less refresh must not clear it");
    }

    #[tokio::test]
    async fn a_refresh_request_returns_immediately_instead_of_fetching_inline() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        app.job_tx = Some(tx);

        app.request_reload(&deps);
        // Control is back with data still outstanding — that is what keeps the frame drawing.
        assert!(app.loading, "the loading flag must be set for the footer indicator");
        assert!(app.reloading, "the refresh must be in flight, not already applied");

        // Single-flight: a second request while one is running must not stack another fetch.
        app.reloading = true;
        app.request_reload(&deps);
        assert!(app.reloading);
    }

    #[tokio::test]
    async fn first_run_asks_where_to_set_up_rather_than_opening_a_browser() {
        let mut app = App::new("slate");
        app.open_setup_picker();
        let Some(Overlay::Picker { title, items, selected, kind }) = &app.overlay else {
            panic!("expected the setup picker to be open");
        };
        assert!(title.contains("access tokens"), "title should say what it wants: {title}");
        assert_eq!(items.len(), 2);
        assert!(items[0].contains("terminal"), "the terminal must come first: {items:?}");
        assert!(items[1].contains("browser"));
        assert_eq!(*selected, 0, "the terminal is the default");
        assert!(matches!(kind, PickerKind::SetupLocation));
        // Nothing has been launched yet — the choice is still the user's.
        assert!(!app.awaiting_browser_setup);
        assert!(app.wizard.is_none());
    }

    #[tokio::test]
    async fn choosing_the_terminal_opens_the_wizard_and_not_the_browser() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.dashboard_url = Some("http://127.0.0.1:1/".into());
        app.open_setup_picker();
        app.on_key(Key::Enter, &deps).await;

        assert!(app.wizard.is_some(), "the terminal choice must start the wizard");
        assert!(!app.awaiting_browser_setup, "nothing should be waiting on a browser");
        assert!(app.overlay.is_none(), "the picker should be gone");
    }

    #[tokio::test]
    async fn choosing_the_browser_waits_instead_of_leaving_an_empty_screen() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.dashboard_url = Some("http://127.0.0.1:1/".into());
        app.open_setup_picker();
        app.on_key(Key::Down, &deps).await;
        app.on_key(Key::Enter, &deps).await;

        assert!(app.awaiting_browser_setup, "the browser choice must show the waiting state");
        assert!(app.wizard.is_none());
    }

    /// `C` used to open the browser as well, from when connection management lived in the
    /// dashboard. The terminal list does the whole job now, and `B` is the only key that leaves.
    #[tokio::test]
    async fn c_opens_the_terminal_connections_list_and_never_the_browser() {
        let deps = test_deps();
        for screen in [Screen::Launchpad, Screen::List] {
            let mut app = App::new("slate");
            app.screen = screen;
            app.dashboard_url = Some("http://127.0.0.1:1/".into());
            app.on_key(Key::Char('C'), &deps).await;

            assert!(matches!(app.screen, Screen::Config(_)), "C must land on the connections list");
            assert!(
                !app.toast.as_deref().is_some_and(|t| t.contains("dashboard")),
                "C must not launch the dashboard: {:?}",
                app.toast
            );
        }
    }

    #[tokio::test]
    async fn waiting_state_clears_once_a_connection_lands() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.awaiting_browser_setup = true;

        // A reload that still finds nothing configured must keep waiting, or the card
        // would vanish the moment the periodic tick fired and leave a blank screen.
        app.apply_reloaded(reloaded_with_health(Vec::new()), &deps);
        assert!(app.awaiting_browser_setup, "an empty reload must not clear the waiting state");

        app.apply_reloaded(reloaded_with_health(vec![health("gh-1")]), &deps);
        assert!(!app.awaiting_browser_setup, "a connection landing must clear it");
        assert!(app.toast.as_deref().unwrap_or_default().contains("all set"));
    }

    #[tokio::test]
    async fn n_reopens_the_setup_picker() {
        let deps = test_deps();
        let mut app = App::new("slate");
        // The first-run hint has always told users to press `n`; before this it did nothing.
        app.on_key(Key::Char('n'), &deps).await;
        assert!(
            matches!(&app.overlay, Some(Overlay::Picker { kind: PickerKind::SetupLocation, .. })),
            "`n` must open the setup picker"
        );
    }

    #[tokio::test]
    async fn input_contexts_and_lowercase_f_take_precedence_over_the_feedback_shortcut() {
        let deps = test_deps();

        let mut lowercase = App::new("slate");
        lowercase.screen = Screen::List;
        lowercase.active = 0;
        lowercase.on_key(Key::Char('f'), &deps).await;
        assert!(
            matches!(
                lowercase.overlay,
                Some(Overlay::Toggle {
                    kind: ToggleKind::PrStatuses,
                    ..
                })
            ),
            "lowercase f keeps the existing status-filter binding"
        );

        let mut input = App::new("slate");
        input.dashboard_url = Some("http://127.0.0.1:8177/?t=session-secret".into());
        input.overlay = Some(Overlay::Input {
            title: "Comment".into(),
            buffer: String::new(),
            kind: InputKind::PrComment,
        });
        input.on_key(Key::Char('F'), &deps).await;
        let Some(Overlay::Input { buffer, .. }) = &input.overlay else {
            panic!("input overlay should remain open");
        };
        assert_eq!(buffer, "F", "the input receives F instead of opening a browser");

        let mut palette = App::new("slate");
        palette.dashboard_url = Some("http://127.0.0.1:8177/?t=session-secret".into());
        palette.overlay = Some(Overlay::Palette {
            query: String::new(),
            candidates: Vec::new(),
            results: Vec::new(),
            selected: 0,
        });
        palette.on_key(Key::Char('F'), &deps).await;
        let Some(Overlay::Palette { query, .. }) = &palette.overlay else {
            panic!("palette should remain open");
        };
        assert_eq!(query, "F", "the palette receives F instead of opening a browser");

        let mut filter = App::new("slate");
        filter.dashboard_url = Some("http://127.0.0.1:8177/?t=session-secret".into());
        filter.filtering = true;
        filter.on_key(Key::Char('F'), &deps).await;
        assert_eq!(filter.active_filter(), "F", "the quick filter receives F");
    }

    fn pr(url: Option<&str>) -> PullRequest {
        PullRequest {
            repository: None,
            id: "1".into(),
            number: Some(1),
            title: "t".into(),
            description: None,
            author: User { id: "a".into(), display_name: "A".into(), handle: None, avatar_url: None },
            status: PullRequestStatus::Open,
            is_draft: false,
            source_ref: None,
            target_ref: None,
            reviewers: vec![],
            labels: vec![],
            checks: CheckStatus::None,
            check_summary: None,
            mergeable: MergeableState::Unknown,
            changed_files: 0,
            additions: 0,
            deletions: 0,
            created_at: None,
            updated_at: None,
            url: url.map(Into::into),
        }
    }

    fn wi(url: Option<&str>) -> WorkItem {
        WorkItem {
            repository: None,
            id: "w".into(),
            identifier: None,
            title: "t".into(),
            description: None,
            state: "Todo".into(),
            state_category: WorkItemStateCategory::Backlog,
            work_item_type: None,
            assignee: None,
            created_at: None,
            updated_at: None,
            url: url.map(Into::into),
        }
    }

    fn pr_row(pr: PullRequest) -> PrRow {
        PrRow { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, pr }
    }

    fn wi_row(wi: WorkItem) -> WiRow {
        WiRow { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, wi }
    }

    /// A PR authored by `handle`, reviewed by `reviewers`, on connection `conn`.
    fn pool_pr(conn: &str, id: &str, author: &str, reviewers: &[&str]) -> PrRow {
        let user = |h: &str| User { id: h.into(), display_name: h.into(), handle: Some(h.into()), avatar_url: None };
        let mut pr = pr(None);
        pr.id = id.into();
        pr.author = user(author);
        pr.reviewers = reviewers
            .iter()
            .map(|h| Reviewer { user: user(h), vote: ReviewVote::NoVote, is_required: false })
            .collect();
        PrRow { connection_id: conn.into(), connection: conn.into(), provider: ProviderType::GitHub, pr }
    }

    fn pool_of(rows: Vec<PrRow>, me: Option<&str>) -> PrPool {
        let ids: HashSet<String> = rows.iter().map(|r| r.connection_id.clone()).collect();
        PrPool {
            open: rows.clone(),
            completed: rows,
            me: ids.iter().map(|c| (c.clone(), me.map(str::to_string))).collect(),
            needs_decoration: ids.into_iter().map(|c| (c, false)).collect(),
        }
    }

    /// A decoration that cannot be fetched must not requeue itself. Re-deriving is what
    /// *follows* a finished batch, so without the failure mark the row asks again the instant
    /// its own failure lands — and the feed-not-found path does that with no I/O to slow it.
    #[test]
    fn a_failed_decoration_is_not_requeued_until_the_next_reload() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        app.pr_pool = pool_of(vec![pool_pr("c", "1", "me", &[])], Some("me"));
        app.pr_pool.needs_decoration.insert("c".into(), true);
        app.pr_pool_loaded = true;

        let key = ("c".to_string(), "1".to_string());
        app.apply_pr_decorations(vec![(key.clone(), None)], &deps);
        assert!(app.pr_decor_failed.contains(&key), "the failure is remembered");
        assert!(!app.pr_decorations.contains_key(&key), "but not cached as a blank decoration");

        // The re-derivation that just ran must not have queued it again.
        assert!(app.pr_decor_inflight.is_empty(), "a failed row does not requeue itself");
    }

    /// Before any fetch has landed there is nothing to derive the Launchpad from, and deriving
    /// anyway would blank exactly the rows `seed_from_cache` painted.
    #[test]
    fn a_view_switch_before_the_first_fetch_keeps_the_seeded_launchpad() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        app.lp_prs_mine = vec![pr_row(pr(None))];
        app.lp_prs_review = vec![pr_row(pr(None))];

        assert!(!app.pr_pool_loaded, "sanity: nothing fetched yet");
        app.pr_filter = PullRequestFilter::Mine;
        app.refresh_derived_prs(&deps);

        assert_eq!(app.lp_prs_mine.len(), 1, "the seeded Launchpad survives a view switch");
        assert_eq!(app.lp_prs_review.len(), 1);
        assert!(app.prs.is_empty(), "the list itself is empty rather than mislabelled");
    }

    /// The cache key must describe the rows actually written. They follow the screen, so a
    /// filter changed while the fetch was in flight moves them to the new filter's key.
    #[test]
    fn the_pr_cache_is_keyed_by_the_filter_the_rows_were_derived_under() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let mut app = App::new("slate");

        // The fetch goes out under All; the user switches to Mine before it lands.
        app.pr_filter = PullRequestFilter::Mine;
        app.apply_reloaded(reloaded(vec![pool_pr("c", "1", "me", &[]), pool_pr("c", "2", "them", &[])]), &deps);

        let mine = cache.get::<Vec<PrRow>>(&prs_cache_key(PullRequestFilter::Mine, false));
        assert_eq!(mine.expect("cached under Mine").value.len(), 1, "only the user's own row");
        assert!(
            cache.get::<Vec<PrRow>>(&prs_cache_key(PullRequestFilter::All, false)).is_none(),
            "and nothing is filed under the filter that merely asked"
        );
    }

    /// Removing a connection prunes the pool too — otherwise the next local derivation puts
    /// its rows straight back on screen.
    #[test]
    fn purging_a_connection_prunes_the_pool_not_just_the_lists() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        app.pr_pool = pool_of(vec![pool_pr("gone", "1", "me", &[]), pool_pr("kept", "2", "me", &[])], Some("me"));
        app.pr_pool_loaded = true;
        app.refresh_derived_prs(&deps);
        assert_eq!(app.prs.len(), 2, "sanity");

        retain_pool_rows(&mut app.pr_pool, |r| r.connection_id != "gone");
        app.refresh_derived_prs(&deps);
        assert_eq!(app.prs.iter().map(|r| r.connection_id.as_str()).collect::<Vec<_>>(), ["kept"]);
    }

    /// The point of the whole change: `[`/`]` reads the pool the last fetch already returned.
    #[test]
    fn switching_views_derives_from_the_pool_without_refetching() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        app.pr_pool = pool_of(
            vec![
                pool_pr("c", "1", "me", &[]),
                pool_pr("c", "2", "them", &["me"]),
                pool_pr("c", "3", "them", &["someone"]),
            ],
            Some("me"),
        );

        app.pr_filter = PullRequestFilter::All;
        app.refresh_derived_prs(&deps);
        assert_eq!(app.prs.len(), 3, "All shows the whole pool");

        app.pr_filter = PullRequestFilter::Mine;
        app.refresh_derived_prs(&deps);
        assert_eq!(app.prs.iter().map(|r| r.pr.id.as_str()).collect::<Vec<_>>(), ["1"]);

        app.pr_filter = PullRequestFilter::ReviewRequested;
        app.refresh_derived_prs(&deps);
        assert_eq!(app.prs.iter().map(|r| r.pr.id.as_str()).collect::<Vec<_>>(), ["2"]);
    }

    /// The cap is applied **after** filtering, as `sort_and_cap` did inside the provider. Capping
    /// the pool first would make "Mine" the newest 50 overall that happen to be yours — fewer
    /// rows than the per-filter fetch used to return.
    #[test]
    fn a_derived_view_caps_after_filtering_not_before() {
        // 60 of someone else's in front of 60 of yours: a pool capped at 50 would yield none.
        let mut rows: Vec<PrRow> = (0..60).map(|i| pool_pr("c", &format!("t{i}"), "them", &[])).collect();
        rows.extend((0..60).map(|i| pool_pr("c", &format!("m{i}"), "me", &[])));
        let pool = pool_of(rows, Some("me"));

        let mine = derive_pool_rows(&pool, PullRequestFilter::Mine, false);
        assert_eq!(mine.len(), PR_VIEW_CAP, "the cap counts filtered rows");
        assert!(mine.iter().all(|r| r.pr.id.starts_with('m')));
    }

    /// One busy connection must not crowd another off the list: each feed ran its own capped
    /// `list`, so the cap is per connection.
    #[test]
    fn the_view_cap_is_per_connection() {
        let mut rows: Vec<PrRow> = (0..60).map(|i| pool_pr("a", &format!("a{i}"), "me", &[])).collect();
        rows.extend((0..10).map(|i| pool_pr("b", &format!("b{i}"), "me", &[])));
        let derived = derive_pool_rows(&pool_of(rows, Some("me")), PullRequestFilter::Mine, false);
        assert_eq!(derived.iter().filter(|r| r.connection_id == "a").count(), PR_VIEW_CAP);
        assert_eq!(derived.iter().filter(|r| r.connection_id == "b").count(), 10, "b keeps all of its own");
    }

    /// An identity the provider could not establish means "cannot filter", not "nothing
    /// matches" — the same thing `list` already did, and the difference between an
    /// over-inclusive list and a silently empty one.
    #[test]
    fn a_connection_with_no_identity_shows_its_rows_rather_than_hiding_them() {
        let pool = pool_of(vec![pool_pr("c", "1", "them", &[]), pool_pr("c", "2", "other", &[])], None);
        assert_eq!(derive_pool_rows(&pool, PullRequestFilter::Mine, false).len(), 2);
    }

    /// Bitbucket's completed state is `MERGED`, which excludes open PRs — so the two variants
    /// are held separately and the open views must not be derived from the completed pool.
    #[test]
    fn the_two_completed_variants_are_kept_apart() {
        let mut pool = pool_of(vec![pool_pr("c", "open", "me", &[])], Some("me"));
        pool.completed = vec![pool_pr("c", "merged", "me", &[])];
        assert_eq!(derive_pool_rows(&pool, PullRequestFilter::All, false)[0].pr.id, "open");
        assert_eq!(derive_pool_rows(&pool, PullRequestFilter::All, true)[0].pr.id, "merged");
    }

    /// Decoration fetched per row is merged into whatever view is derived next, and a failed
    /// call is not cached as a blank one.
    #[test]
    fn decorations_merge_into_derived_rows_and_failures_are_not_cached() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        app.pr_pool = pool_of(vec![pool_pr("c", "1", "me", &[]), pool_pr("c", "2", "me", &[])], Some("me"));

        let decoration = PrDecoration { additions: 42, changed_files: 3, ..Default::default() };
        app.apply_pr_decorations(
            vec![(("c".into(), "1".into()), Some(decoration)), (("c".into(), "2".into()), None)],
            &deps,
        );

        let row = |id: &str| app.prs.iter().find(|r| r.pr.id == id).expect("row present");
        assert_eq!(row("1").pr.additions, 42, "a fetched decoration lands on the row");
        assert_eq!(row("2").pr.additions, 0, "a failed one leaves the row undecorated");
        assert!(!app.pr_decorations.contains_key(&("c".to_string(), "2".to_string())), "and is not cached");
    }

    #[test]
    fn inbox_unread_count_and_move_wraps() {
        let mut app = App::new("slate");
        let n = |id: &str, unread: bool| NotifRow {
            connection_id: "github".into(),
            connection: "GitHub".into(),
            provider: ProviderType::GitHub,
            notification: Notification {
                repository: None,
                id: id.into(),
                kind: NotificationKind::Mention,
                item_type: NotificationItemType::WorkItem,
                item_id: None,
                title: "t".into(),
                context: "c".into(),
                url: None,
                unread,
                updated_at: None,
            },
        };
        app.inbox = vec![n("a", true), n("b", false), n("c", true)];
        assert_eq!(app.unread_count(), 2);
        app.inbox_move(1);
        assert_eq!(app.inbox_sel, 1);
        app.inbox_move(-1);
        assert_eq!(app.inbox_sel, 0);
        app.inbox_move(-1);
        assert_eq!(app.inbox_sel, 2, "moving up past the top wraps to the end");
    }

    #[test]
    fn launchpad_caps_assigned_work_and_adds_a_more_slot() {
        let mut app = App::new("slate");
        app.wis = (0..6)
            .map(|i| {
                let mut w = wi(None);
                w.id = format!("w{i}");
                w.state_category = WorkItemStateCategory::Started;
                wi_row(w)
            })
            .collect();
        app.rebuild_launchpad();
        // Six assigned → five shown, with the overflow flag set.
        let shown = app.lp.iter().filter(|e| e.bucket == launchpad::Bucket::YourWork).count();
        assert_eq!(shown, 5, "capped at five");
        assert!(app.lp_overflow.your_work, "overflow flagged");
        // The right column ends with a selectable "more…" slot for the overflowing bucket.
        let slots = app.lp_slots(1);
        assert_eq!(slots.len(), shown + 1, "five entries + one more… slot");
        assert!(matches!(slots.last(), Some(LpSlot::More(launchpad::Bucket::YourWork))));
    }

    #[test]
    fn palette_candidates_map_rows_to_searchable_items() {
        let mut p = pr(None);
        p.id = "pr1".into();
        p.title = "Migrate billing".into();
        p.source_ref = Some("feat/pay-412".into());
        p.author = User { id: "u".into(), display_name: "Priya".into(), handle: Some("priya".into()), avatar_url: None };
        let mut w = wi(None);
        w.id = "wi1".into();
        w.identifier = Some("PAY-412".into());
        w.title = "Billing migration".into();

        let cands = palette::build_candidates(&[pr_row(p)], &[wi_row(w)], &[]);
        assert_eq!(cands.len(), 2);
        // PR: routes by (kind,id), title is the PR title, subtitle carries author + branch.
        assert!(matches!(&cands[0].target, PaletteTarget::Item { kind: PaletteKind::Pr, id, .. } if id == "pr1"));
        assert_eq!(cands[0].title, "Migrate billing");
        assert!(cands[0].subtitle.contains("priya"), "subtitle has author: {}", cands[0].subtitle);
        assert!(cands[0].subtitle.contains("feat/pay-412"), "subtitle has branch: {}", cands[0].subtitle);
        // WI: identifier is searchable via the subtitle.
        assert!(matches!(&cands[1].target, PaletteTarget::Item { kind: PaletteKind::Wi, id, .. } if id == "wi1"));
        assert!(cands[1].subtitle.contains("PAY-412"), "subtitle has identifier: {}", cands[1].subtitle);
    }

    #[test]
    fn apply_reloaded_swaps_data_and_clears_refresh_flags() {
        let mut app = App::new("slate");
        app.reloading = true;
        app.loading = true;
        app.apply_reloaded(
            Reloaded {
                pr_pool: test_pool(vec![pr_row(pr(None)), pr_row(pr(None))]),
                wis: vec![wi_row(wi(None))],
                pipes: vec![],
                inbox: vec![],
                health: vec![],
                scan: None,
                catalog: None,
                open_pipeline: None,
                errors: vec![],
                sections_ok: SectionsOk::complete(),
                requested_at: Utc::now(),
            },
            &test_deps(),
        );
        assert_eq!(app.prs.len(), 2);
        assert_eq!(app.wis.len(), 1);
        assert!(!app.reloading && !app.loading, "refresh flags cleared");
        assert!(app.status.contains("2 PRs") && app.status.contains("1 work items"), "status summarises");
    }

    /// A provider outage doesn't fail its section — it returns an empty list and an error — so
    /// writing that empty list through would wipe the good rows and open the app blank next time.
    #[test]
    fn an_errored_reload_never_caches_an_empty_list_over_good_rows() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let key = prs_cache_key(PullRequestFilter::All, false);
        let first = Utc::now();
        let mut app = App::new("slate");

        app.prs = vec![pr_row(pr(None))];
        app.write_through(&deps, &key, first, SectionsOk::complete());
        assert_eq!(cache.get::<Vec<PrRow>>(&key).expect("cached").value.len(), 1);

        // Empty *with* an error: the rows already cached must survive it.
        app.prs = Vec::new();
        let failed = SectionsOk { prs: false, ..SectionsOk::complete() };
        app.write_through(&deps, &key, first + chrono::TimeDelta::seconds(30), failed);
        assert_eq!(cache.get::<Vec<PrRow>>(&key).expect("still cached").value.len(), 1, "an outage must not wipe the cache");

        // Empty with no errors is a genuine "you have none", and does replace them.
        app.write_through(&deps, &key, first + chrono::TimeDelta::seconds(60), SectionsOk::complete());
        assert!(cache.get::<Vec<PrRow>>(&key).expect("cached").value.is_empty(), "a clean empty reload is cacheable");
    }

    /// The failure the old whole-list guard let through: three connections, one down. The PR
    /// list still arrives non-empty, so `!prs.is_empty()` passed and a two-connection list was
    /// written over the cached three-connection one, losing the third's rows until it recovered.
    #[test]
    fn a_partial_section_never_overwrites_a_complete_cached_one() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let key = prs_cache_key(PullRequestFilter::All, false);
        let first = Utc::now();
        let mut app = App::new("slate");

        app.prs = vec![pr_row(pr(None)), pr_row(pr(None)), pr_row(pr(None))];
        app.write_through(&deps, &key, first, SectionsOk::complete());

        // One connection 500s: fewer rows, still rows. That must not be cached.
        app.prs = vec![pr_row(pr(None)), pr_row(pr(None))];
        app.wis = vec![wi_row(wi(None))];
        app.write_through(
            &deps,
            &key,
            first + chrono::TimeDelta::seconds(30),
            SectionsOk { prs: false, ..SectionsOk::complete() },
        );

        assert_eq!(
            cache.get::<Vec<PrRow>>(&key).expect("still cached").value.len(),
            3,
            "a short list must leave the complete cached one alone"
        );
        // One section's outage says nothing about the others, which still cache.
        assert_eq!(
            cache.get::<Vec<WiRow>>(CACHE_KEY_WORK_ITEMS).expect("work items cached").value.len(),
            1,
            "a PR outage must not stop a complete work-item list being cached"
        );
    }

    /// Defect found in review: `fetch_launchpad_prs` swallowed every failure, so a Launchpad
    /// outage looked like a clean fetch and cached two empty lists over the landing screen's
    /// rows. The flags it now returns are what keep those rows.
    #[test]
    fn a_failed_launchpad_fetch_never_caches_empties_over_good_rows() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let first = Utc::now();
        let mut app = App::new("slate");
        let key = prs_cache_key(PullRequestFilter::All, false);

        app.lp_prs_mine = vec![pr_row(pr(None))];
        app.lp_prs_review = vec![pr_row(pr(None))];
        app.write_through(&deps, &key, first, SectionsOk::complete());

        // Both Launchpad queries failed: empty lists, and now flags that say so.
        app.lp_prs_mine = Vec::new();
        app.lp_prs_review = Vec::new();
        app.write_through(
            &deps,
            &key,
            first + chrono::TimeDelta::seconds(30),
            SectionsOk { lp_mine: false, lp_review: false, ..SectionsOk::complete() },
        );

        assert_eq!(
            cache.get::<Vec<PrRow>>(CACHE_KEY_LAUNCHPAD_MINE).expect("still cached").value.len(),
            1,
            "a Launchpad outage must not open the landing screen empty next launch"
        );
        assert_eq!(
            cache.get::<Vec<PrRow>>(CACHE_KEY_LAUNCHPAD_REVIEW).expect("still cached").value.len(),
            1
        );
    }

    /// The store refuses a write that isn't newer than what it holds, which only orders two
    /// reloads if the timestamp says when each was *asked for*. Stamped on arrival, the slow
    /// reload — the one that knows least — always looked freshest and always won.
    #[test]
    fn a_reload_caches_under_its_request_time_not_its_arrival_time() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let key = prs_cache_key(PullRequestFilter::All, false);
        let mut app = App::new("slate");

        // Sent first, lands second: its rows must not displace the later request's.
        let mut slow = reloaded(vec![pr_row(pr(None))]);
        slow.requested_at = Utc::now() - chrono::TimeDelta::seconds(30);
        let fast = reloaded(vec![pr_row(pr(None)), pr_row(pr(None))]);
        let fast_at = fast.requested_at;

        app.apply_reloaded(fast, &deps);
        app.apply_reloaded(slow, &deps);

        let cached = cache.get::<Vec<PrRow>>(&key).expect("cached");
        assert_eq!(cached.fetched_at, fast_at, "the later-requested reload owns the entry");
        assert_eq!(cached.value.len(), 2, "a late answer to an older request must not win");
    }

    #[test]
    fn one_reload_caches_every_section_under_a_single_timestamp() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let mut app = App::new("slate");
        app.data_age = Some(Utc::now() - chrono::TimeDelta::hours(2));

        app.apply_reloaded(reloaded(vec![pr_row(pr(None))]), &deps);

        let prs = cache.get::<Vec<PrRow>>(&prs_cache_key(PullRequestFilter::All, false)).expect("PRs cached");
        let wis = cache.get::<Vec<WiRow>>(CACHE_KEY_WORK_ITEMS).expect("work items cached");
        assert_eq!(prs.value.len(), 1);
        assert_eq!(prs.fetched_at, wis.fetched_at, "one reload's sections share a fetched_at");
        assert!(app.data_age.is_none(), "live data stops the header reporting a cached age");
    }

    /// Defect found in review: the *cache* was protected from a failed section by its flag, the
    /// *screen* wasn't. A total outage on the first refresh after launch therefore replaced the
    /// cache-seeded lists with empty live ones and dropped the "showing Nm old" footer with them.
    #[test]
    fn a_wholly_failed_reload_keeps_the_seeded_rows_and_their_age() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let seeded = Utc::now() - chrono::TimeDelta::minutes(20);
        cache.put(&prs_cache_key(PullRequestFilter::All, false), &[pr_row(pr(None))], seeded);
        cache.put(CACHE_KEY_WORK_ITEMS, &[wi_row(wi(None))], seeded);

        let mut app = App::new("slate");
        app.seed_from_cache(&deps);
        assert_eq!(app.prs.len(), 1, "sanity: seeded before any fetch");
        assert_eq!(app.data_age, Some(seeded));
        app.inbox = vec![notif_row("a"), notif_row("b")];
        app.inbox_sel = 1;

        // Every feed is down: each section comes back empty, and says so.
        let mut r = reloaded(Vec::new());
        r.sections_ok =
            SectionsOk { prs: false, wis: false, pipes: false, inbox: false, lp_mine: false, lp_review: false };
        app.apply_reloaded(r, &deps);

        assert_eq!(app.prs.len(), 1, "an outage must not blank what the cache seeded");
        assert_eq!(app.wis.len(), 1);
        assert_eq!(app.inbox.len(), 2, "the inbox is kept too");
        assert_eq!(app.inbox_sel, 1, "and the selection still points into the list that stayed");
        assert_eq!(app.data_age, Some(seeded), "rows are still cached, so the footer keeps saying so");
    }

    /// The three-way rule, on the two cases the outage test doesn't cover: a failed section that
    /// still returned rows is partial *live* data (and the error is in the status line), so it is
    /// taken; a section that answered is always taken, empty included.
    #[test]
    fn a_short_live_section_is_taken_but_only_a_clean_reload_clears_the_age() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        app.prs = vec![pr_row(pr(None)), pr_row(pr(None)), pr_row(pr(None))];
        app.data_age = Some(Utc::now() - chrono::TimeDelta::hours(1));

        // One connection of three is down: fewer rows, still rows.
        let mut partial = reloaded(vec![pr_row(pr(None))]);
        partial.sections_ok = SectionsOk { prs: false, ..SectionsOk::complete() };
        app.apply_reloaded(partial, &deps);
        assert_eq!(app.prs.len(), 1, "partial live rows beat stale ones on screen");
        assert!(app.data_age.is_some(), "but one section short still means the age is real");

        // Everything answers, and answers empty: that is authoritative.
        app.apply_reloaded(reloaded(Vec::new()), &deps);
        assert!(app.prs.is_empty(), "an answered empty section does clear the list");
        assert!(app.data_age.is_none(), "nothing on screen is cached any more");
    }

    #[test]
    fn seeding_paints_cached_rows_and_reports_the_most_stale_section() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let newest = Utc::now();
        let oldest = newest - chrono::TimeDelta::hours(6);
        cache.put(&prs_cache_key(PullRequestFilter::All, false), &[pr_row(pr(None))], newest);
        cache.put(CACHE_KEY_WORK_ITEMS, &[wi_row(wi(None))], oldest);

        let mut app = App::new("slate");
        app.seed_from_cache(&deps);

        assert_eq!(app.prs.len(), 1, "the PR list is painted before any fetch");
        assert_eq!(app.wis.len(), 1);
        assert!(app.pipes.is_empty(), "a miss leaves its section empty");
        assert_eq!(app.data_age, Some(oldest), "the age shown is the most stale section's, not the freshest");
    }

    #[test]
    fn a_disabled_cache_seeds_nothing_and_claims_no_age() {
        let deps = test_deps();
        let mut app = App::new("slate");

        app.seed_from_cache(&deps);

        assert!(app.prs.is_empty() && app.wis.is_empty() && app.inbox.is_empty());
        assert!(app.data_age.is_none(), "nothing was painted, so nothing is stale");
    }

    #[test]
    fn reviewing_a_pr_drops_it_from_the_launchpad() {
        let mut app = App::new("slate");
        let mut p = pr(None);
        p.id = "pr1".into();
        app.lp_prs_review = vec![pr_row(p)];
        app.rebuild_launchpad();
        assert_eq!(app.lp.len(), 1, "the PR shows in the review bucket");

        // Acting on it removes it immediately, and it stays gone across a refetch.
        app.dismiss_from_launchpad("c", "pr1");
        assert!(app.lp.is_empty(), "reviewed PR is dismissed");
        app.rebuild_launchpad();
        assert!(app.lp.is_empty(), "still gone until the provider feed catches up");
    }

    #[tokio::test]
    async fn pressing_shift_d_dismisses_the_selected_item_and_persists_it() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let mut p = pr(None);
        p.id = "pr1".into();
        app.lp_prs_review = vec![pr_row(p)];
        app.rebuild_launchpad();
        assert_eq!(app.lp.len(), 1, "the PR shows in the review bucket before dismissal");
        assert_eq!(app.prs.len(), 0, "the separate Pull Requests tab list is untouched by setup");

        app.on_key(Key::Char('D'), &deps).await;

        assert!(app.lp.is_empty(), "dismissing hides it from Command Center");
        assert!(
            app.prs.is_empty(),
            "dismissal must not reach the Pull Requests tab's own list"
        );

        // The choice is persisted, not just held in memory: a fresh App seeded from
        // config keeps the item hidden even before any dismissal happens again.
        let saved = deps.config.snapshot().ui.dismissed_launchpad_items;
        assert_eq!(saved.len(), 1, "the dismissal is written to config");

        let mut fresh = App::new("slate");
        fresh.apply_dismissed_launchpad_items(&saved);
        let mut p2 = pr(None);
        p2.id = "pr1".into();
        fresh.lp_prs_review = vec![pr_row(p2)];
        fresh.rebuild_launchpad();
        assert!(fresh.lp.is_empty(), "a restarted app keeps the dismissal");
    }

    #[test]
    fn escape_returns_to_the_launchpad_when_opened_from_it() {
        let mut app = App::new("slate");
        let view = || Screen::WiView(Box::new(WiView { connection_id: "c".into(), wi: wi(None), threads: vec![], scroll: 0, timeline: Vec::new() }));

        // Opened from the Launchpad → Esc goes back to the Launchpad (row still selected).
        app.lp_origin = true;
        app.screen = view();
        app.on_wi_view_key(Key::Escape);
        assert!(matches!(app.screen, Screen::Launchpad));

        // Opened from the section list → Esc goes back to the list.
        app.lp_origin = false;
        app.screen = view();
        app.on_wi_view_key(Key::Escape);
        assert!(matches!(app.screen, Screen::List));
    }

    #[test]
    fn pr_status_filter_limits_the_list_and_drives_completed_fetch() {
        let mut app = App::new("slate");
        let mk = |id: &str, status| {
            let mut p = pr(None);
            p.id = id.into();
            p.status = status;
            pr_row(p)
        };
        app.prs = vec![
            mk("1", PullRequestStatus::Open),
            mk("2", PullRequestStatus::Merged),
            mk("3", PullRequestStatus::Draft),
            mk("4", PullRequestStatus::Closed),
        ];
        app.active = 0;

        // Default (Open + Draft) shows only those, and doesn't need completed PRs.
        assert!(!app.pr_wants_completed());
        assert_eq!(app.filtered_pr_indices(), vec![0, 2]);

        // Ticking Merged shows it and flips the fetch to include completed PRs.
        app.pr_shown_statuses = [PullRequestStatus::Open, PullRequestStatus::Merged].into_iter().collect();
        assert!(app.pr_wants_completed());
        assert_eq!(app.filtered_pr_indices(), vec![0, 1]);
    }

    #[test]
    fn pr_status_summary_reads_in_canonical_order() {
        let set: HashSet<_> = [PullRequestStatus::Merged, PullRequestStatus::Open].into_iter().collect();
        assert_eq!(pr_status_summary(&set), "Open, Merged");
        let all: HashSet<_> = PR_STATUS_ORDER.into_iter().collect();
        assert_eq!(pr_status_summary(&all), "all statuses");
    }

    #[test]
    fn sort_orders_rows_and_respects_direction() {
        let mut app = App::new("slate");
        let mut a = pr(None);
        a.number = Some(3);
        a.title = "banana".into();
        let mut b = pr(None);
        b.number = Some(1);
        b.title = "cherry".into();
        let mut c = pr(None);
        c.number = Some(2);
        c.title = "apple".into();
        app.prs = vec![pr_row(a), pr_row(b), pr_row(c)]; // provider order: numbers 3, 1, 2
        app.active = 0;

        // No sort → provider order.
        assert_eq!(app.filtered_pr_indices(), vec![0, 1, 2]);

        // By number, ascending then descending.
        app.pr_sort = Some(SortPref { key: "number".into(), desc: false });
        assert_eq!(app.filtered_pr_indices(), vec![1, 2, 0]); // 1,2,3
        app.pr_sort = Some(SortPref { key: "number".into(), desc: true });
        assert_eq!(app.filtered_pr_indices(), vec![0, 2, 1]); // 3,2,1

        // By title (case-insensitive) ascending: apple, banana, cherry.
        app.pr_sort = Some(SortPref { key: "title".into(), desc: false });
        assert_eq!(app.filtered_pr_indices(), vec![2, 0, 1]);

        // Sort composes with the quick filter (only matching rows, still sorted).
        app.filters[0] = "e".into(); // matches "cherry" and "apple"
        assert_eq!(app.filtered_pr_indices(), vec![2, 1]); // apple(2) then cherry(1)
    }

    #[test]
    fn pipeline_duration_formatting_and_stage_span() {
        use chrono::TimeZone;
        let t = |s: i64| Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + chrono::Duration::seconds(s);

        assert_eq!(fmt_duration(Some(t(0)), Some(t(45))).as_deref(), Some("45s"));
        assert_eq!(fmt_duration(Some(t(0)), Some(t(75))).as_deref(), Some("1m15s"));
        assert_eq!(fmt_duration(Some(t(0)), Some(t(3720))).as_deref(), Some("1h02m"));
        assert_eq!(fmt_duration(Some(t(0)), None), None, "unfinished → no duration");
        assert_eq!(fmt_duration(None, Some(t(10))), None);

        let mk = |start: i64, fin: Option<i64>| PipelineJob {
            id: "j".into(),
            name: "j".into(),
            status: PipelineRunStatus::Succeeded,
            started_at: Some(t(start)),
            finished_at: fin.map(t),
            steps: vec![],
            url: None,
            problem: None,
        };
        // Stage spans earliest start → latest finish; any unfinished job → None.
        assert_eq!(stage_duration(&[mk(0, Some(30)), mk(10, Some(50))]).as_deref(), Some("50s"));
        assert_eq!(stage_duration(&[mk(0, Some(30)), mk(10, None)]), None);
    }

    fn failed_run() -> PipelineRun {
        PipelineRun {
            repository: None,
            id: "r1".into(),
            definition_id: "ci".into(),
            number: Some(1),
            name: Some("CI".into()),
            title: None,
            status: PipelineRunStatus::Failed,
            triggered_by: None,
            branch: Some("main".into()),
            commit_sha: None,
            started_at: None,
            finished_at: None,
            url: Some("http://run".into()),
            stages: vec![PipelineStage {
                name: "test".into(),
                status: PipelineRunStatus::Failed,
                jobs: vec![PipelineJob {
                    id: "j1".into(),
                    name: "unit".into(),
                    status: PipelineRunStatus::Failed,
                    started_at: None,
                    finished_at: None,
                    steps: vec![],
                    url: Some("http://job".into()),
                    problem: Some("boom".into()),
                }],
            }],
        }
    }

    #[test]
    fn pipeline_node_url_and_log_pane() {
        let mut app = App::new("slate");
        app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI".into(), failed_run(), "c".into(), ProviderType::GitHub, "ci".into(), Some("main".into()))));

        // Node 0 is the stage (no deep link) → falls back to the run URL.
        assert_eq!(app.selected_url().as_deref(), Some("http://run"));
        // Node 1 is the job → its own deep link.
        if let Screen::Pipeline(v) = &mut app.screen {
            v.selected = 1;
        }
        assert_eq!(app.selected_url().as_deref(), Some("http://job"));

        // The log pane scrolls and closes on Esc.
        if let Screen::Pipeline(v) = &mut app.screen {
            v.logs = Some(LogView::with_lines("Logs", "j1", vec!["x".into(); 50]));
        }
        app.on_pipeline_logs_key(Key::Down);
        app.on_pipeline_logs_key(Key::Down);
        let Screen::Pipeline(v) = &app.screen else { panic!() };
        assert_eq!(v.logs.as_ref().unwrap().scroll, 2);
        app.on_pipeline_logs_key(Key::Escape);
        let Screen::Pipeline(v) = &app.screen else { panic!() };
        assert!(v.logs.is_none(), "Esc closes the log pane");
    }

    /// A gate check that failed must leave the gates alone. Collapsing it to an empty list would
    /// drop a gate the user still has to approve, and the picker would then say there is nothing
    /// awaiting them on a run that is blocked.
    #[test]
    fn a_failed_gate_check_on_the_periodic_reload_keeps_the_gates() {
        let mut app = App::new("slate");
        let mut view = PipelineView::new("CI".into(), failed_run(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.supports_approvals = true;
        view.approvals = vec![PipelineApproval { id: "prod".into(), name: "production".into(), can_respond: true }];
        app.screen = Screen::Pipeline(Box::new(view));

        let mut r = reloaded_with_health(Vec::new());
        r.open_pipeline = Some(("r1".into(), failed_run(), None)); // the gate check failed
        app.apply_reloaded(r, &test_deps());

        let Screen::Pipeline(v) = &app.screen else { panic!("still on the pipeline view") };
        assert_eq!(v.approvals.len(), 1, "an unanswered gate check never clears a known gate");
    }

    /// The periodic reload refreshes an open pipeline through `open_pipeline`, not through the
    /// detail fetch. It is still a live answer, so it has to clear staleness too — otherwise a
    /// view whose own detail fetch failed once keeps flagging an unconfirmed status forever.
    #[test]
    fn the_periodic_reload_confirms_a_cache_seeded_pipeline() {
        let mut app = App::new("slate");
        let mut view = PipelineView::new("CI".into(), failed_run(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.stale = true;
        app.screen = Screen::Pipeline(Box::new(view));

        let mut r = reloaded_with_health(Vec::new());
        r.open_pipeline = Some(("r1".into(), failed_run(), Some(Vec::new())));
        app.apply_reloaded(r, &test_deps());

        let Screen::Pipeline(v) = &app.screen else { panic!("still on the pipeline view") };
        assert!(!v.stale, "a run off the wire confirms the view, whichever path fetched it");
    }

    #[test]
    fn approval_picker_offers_only_actionable_gates_and_confirms() {
        let mut app = App::new("slate");
        let mut view = PipelineView::new("CI".into(), failed_run(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.supports_approvals = true;
        view.can_respond_approvals = true;
        view.approvals = vec![
            PipelineApproval { id: "prod".into(), name: "production".into(), can_respond: true },
            PipelineApproval { id: "stg".into(), name: "staging".into(), can_respond: false },
        ];
        app.screen = Screen::Pipeline(Box::new(view));

        app.open_approval_picker();
        // Only the actionable gate is offered — as an Approve and a Reject row.
        assert_eq!(app.approval_choices.len(), 2);
        assert!(app.approval_choices.iter().all(|c| c.approval_id == "prod"));
        match &app.overlay {
            Some(Overlay::Picker { items, kind: PickerKind::ApprovalGate, .. }) => {
                assert_eq!(items.len(), 2);
                assert!(items[0].starts_with("Approve"));
                assert!(items[1].starts_with("Reject"));
            }
            _ => panic!("expected an approval picker"),
        }

        // Picking a row opens a confirm carrying the terminal RespondApproval action.
        app.confirm_approval(0);
        match &app.overlay {
            Some(Overlay::Confirm { action: Action::RespondApproval { index }, .. }) => assert_eq!(*index, 0),
            _ => panic!("expected a respond-approval confirm"),
        }
    }

    #[test]
    fn approval_picker_is_a_noop_without_actionable_gates() {
        let mut app = App::new("slate");
        let mut view = PipelineView::new("CI".into(), failed_run(), "c".into(), ProviderType::Bitbucket, "ci".into(), None);
        view.supports_approvals = false; // Bitbucket
        app.screen = Screen::Pipeline(Box::new(view));
        app.open_approval_picker();
        assert!(app.overlay.is_none(), "no picker when approvals aren't supported");
        assert!(app.toast.is_some());
    }

    /// A notifier that records what it was asked to send, for asserting the glue.
    #[derive(Clone, Default)]
    struct RecordingNotifier {
        events: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    }
    impl Notifier for RecordingNotifier {
        fn notify(&self, title: &str, body: &str) {
            self.events.lock().unwrap().push((title.to_string(), body.to_string()));
        }
    }
    impl RecordingNotifier {
        fn events(&self) -> Vec<(String, String)> {
            self.events.lock().unwrap().clone()
        }
        fn titles(&self) -> Vec<String> {
            self.events().into_iter().map(|(t, _)| t).collect()
        }
    }

    fn pipe_row(id: &str, status: PipelineRunStatus, awaiting: bool) -> PipeRow {
        PipeRow {
            connection_id: "c".into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            definition_name: None,
            awaiting_approval: awaiting,
            run: PipelineRun {
                repository: None,
                id: id.into(),
                definition_id: "ci".into(),
                number: Some(1),
                name: Some("CI".into()),
                title: None,
                status,
                triggered_by: None,
                branch: Some("main".into()),
                commit_sha: None,
                started_at: None,
                finished_at: None,
                url: None,
                stages: vec![],
            },
        }
    }

    #[test]
    fn fires_pipeline_failed_notification_only_for_new_failures() {
        let rec = RecordingNotifier::default();
        let mut app = App::new("slate");
        app.notifier = Arc::new(rec.clone());
        app.notifications.pipeline_failed = true;
        app.pipe_seeded = true; // past the silent first load
        app.pipe_seen = [("a".to_string(), PipelineRunStatus::Failed), ("c".to_string(), PipelineRunStatus::Running)].into_iter().collect();
        app.pipes = vec![pipe_row("a", PipelineRunStatus::Failed, false), pipe_row("c", PipelineRunStatus::Failed, false)];

        app.notify_pipeline_failures();
        // 'a' was already failing; only 'c' transitioned → one notification, right title/body.
        assert_eq!(rec.titles(), vec!["Pipeline failed"]);
        assert!(rec.events()[0].1.contains("CI"), "body names the run");

        // A second pass with the same state (now all seen) fires nothing.
        let rec2 = RecordingNotifier::default();
        app.notifier = Arc::new(rec2.clone());
        app.notify_pipeline_failures();
        assert!(rec2.titles().is_empty(), "already-seen failures don't re-notify");
    }

    #[test]
    fn pipeline_notification_respects_pref_and_first_load_seeding() {
        // Pref off → nothing, even for a brand-new failure.
        let off = RecordingNotifier::default();
        let mut app = App::new("slate");
        app.notifier = Arc::new(off.clone());
        app.notifications.pipeline_failed = false;
        app.pipe_seeded = true;
        app.pipes = vec![pipe_row("a", PipelineRunStatus::Failed, false)];
        app.notify_pipeline_failures();
        assert!(off.titles().is_empty(), "pref off suppresses the notification");

        // First load (not seeded) seeds silently, no notification.
        let first = RecordingNotifier::default();
        let mut app = App::new("slate");
        app.notifier = Arc::new(first.clone());
        app.notifications.pipeline_failed = true;
        app.pipe_seeded = false;
        app.pipes = vec![pipe_row("a", PipelineRunStatus::Failed, false)];
        app.notify_pipeline_failures();
        assert!(first.titles().is_empty(), "first load seeds silently");
        assert!(app.pipe_seeded, "and is now seeded");
    }

    #[test]
    fn fires_approval_needed_notification_once() {
        let rec = RecordingNotifier::default();
        let mut app = App::new("slate");
        app.notifier = Arc::new(rec.clone());
        app.notifications.pipeline_approval_needed = true;
        app.approval_seeded = true;
        app.pipes = vec![pipe_row("a", PipelineRunStatus::Running, true)];

        app.notify_pending_approvals();
        assert_eq!(rec.titles(), vec!["Approval needed"]);

        // Same gate already seen → no repeat.
        let rec2 = RecordingNotifier::default();
        app.notifier = Arc::new(rec2.clone());
        app.notify_pending_approvals();
        assert!(rec2.titles().is_empty(), "the same pending gate isn't re-notified");
    }

    #[test]
    fn notifies_only_on_new_pipeline_failures() {
        let row = |id: &str, status: PipelineRunStatus| PipeRow {
            connection_id: "c".into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            definition_name: None,
            awaiting_approval: false,
            run: PipelineRun {
                repository: None,
                id: id.into(),
                definition_id: "ci".into(),
                number: Some(1),
                name: Some("CI".into()),
                title: None,
                status,
                triggered_by: None,
                branch: Some("main".into()),
                commit_sha: None,
                started_at: None,
                finished_at: None,
                url: None,
                stages: vec![],
            },
        };
        let pipes =
            vec![row("a", PipelineRunStatus::Failed), row("b", PipelineRunStatus::Succeeded), row("c", PipelineRunStatus::Failed)];

        // With no prior state, both current failures are new.
        let ids = |v: Vec<&PipeRow>| v.iter().map(|r| r.run.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(new_pipeline_failures(&HashMap::new(), &pipes)), vec!["a", "c"]);

        // 'a' was already failing (skip); 'c' just transitioned Running→Failed (notify).
        let mut prev = HashMap::new();
        prev.insert("a".to_string(), PipelineRunStatus::Failed);
        prev.insert("c".to_string(), PipelineRunStatus::Running);
        assert_eq!(ids(new_pipeline_failures(&prev, &pipes)), vec!["c"]);
    }

    #[test]
    fn detects_only_newly_pending_approvals() {
        let row = |id: &str, awaiting: bool| PipeRow {
            connection_id: "c".into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            definition_name: None,
            awaiting_approval: awaiting,
            run: PipelineRun {
                repository: None,
                id: id.into(),
                definition_id: "ci".into(),
                number: None,
                name: None,
                title: None,
                status: PipelineRunStatus::Running,
                triggered_by: None,
                branch: None,
                commit_sha: None,
                started_at: None,
                finished_at: None,
                url: None,
                stages: vec![],
            },
        };
        let pipes = vec![row("a", true), row("b", false), row("c", true)];
        let ids = |v: Vec<&PipeRow>| v.iter().map(|r| r.run.id.clone()).collect::<Vec<_>>();

        // Nothing seen before → both awaiting runs are new.
        assert_eq!(ids(new_pending_approvals(&HashSet::new(), &pipes)), vec!["a", "c"]);

        // 'a' already known → only 'c' fires; 'b' (not awaiting) is never included.
        let mut prev = HashSet::new();
        prev.insert(("c".to_string(), "a".to_string()));
        assert_eq!(ids(new_pending_approvals(&prev, &pipes)), vec!["c"]);
    }

    #[test]
    fn pr_review_event_detection() {
        let mk = |votes: &[ReviewVote]| {
            let mut p = pr(None);
            p.reviewers = votes
                .iter()
                .map(|v| Reviewer {
                    user: User { id: "u".into(), display_name: "U".into(), handle: None, avatar_url: None },
                    vote: *v,
                    is_required: false,
                })
                .collect();
            p
        };

        assert_eq!(pr_vote_flags(&mk(&[])), (false, false));
        assert_eq!(pr_vote_flags(&mk(&[ReviewVote::Approved])), (true, false));
        assert_eq!(pr_vote_flags(&mk(&[ReviewVote::Rejected])), (false, true));

        let approved = mk(&[ReviewVote::Approved]);
        assert_eq!(pr_review_transitions(None, &approved), (true, false), "first-seen approval fires");
        assert_eq!(pr_review_transitions(Some((true, false)), &approved), (false, false), "already-approved doesn't re-fire");
        assert_eq!(pr_review_transitions(Some((false, false)), &mk(&[ReviewVote::Rejected])), (false, true));
    }

    #[test]
    fn section_bind_offers_only_capable_connections() {
        let mk = |id: &str, p: ProviderType| Connection {
            id: id.into(),
            provider_type: p,
            display_name: id.into(),
            base_url: None,
            organization: None,
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
            repo_scope: None,
        };
        let conns = vec![mk("gh", ProviderType::GitHub), mk("lin", ProviderType::Linear), mk("bb", ProviderType::Bitbucket)];
        let bound: HashSet<String> = ["gh".to_string()].into_iter().collect();

        // Work Items: GitHub + Linear support it; Bitbucket doesn't.
        let wi = section_bind_items(&conns, Section::WorkItems, &bound);
        assert_eq!(wi.iter().map(|i| i.id.clone()).collect::<Vec<_>>(), vec!["gh", "lin"]);
        assert!(wi.iter().find(|i| i.id == "gh").unwrap().on, "already-bound is ticked");

        // Pull Requests: GitHub + Bitbucket; Linear doesn't.
        let pr = section_bind_items(&conns, Section::PullRequests, &bound);
        assert_eq!(pr.iter().map(|i| i.id.clone()).collect::<Vec<_>>(), vec!["gh", "bb"]);
    }

    #[test]
    fn notifications_toggle_reflects_current_prefs() {
        let mut app = App::new("slate");
        app.notifications = NotificationPrefs {
            pipeline_failed: true,
            review_requested: false,
            pr_approved: true,
            pr_changes_requested: false,
            pipeline_approval_needed: false,
        };
        app.open_notifications_toggle();
        let Some(Overlay::Toggle { kind: ToggleKind::Notifications, items, .. }) = &app.overlay else {
            panic!("expected the notifications toggle");
        };
        assert_eq!(items.len(), 5);
        let on: Vec<&str> = items.iter().filter(|i| i.on).map(|i| i.id.as_str()).collect();
        assert_eq!(on, vec!["pipeline_failed", "pr_approved"]);
    }

    #[test]
    fn saved_views_defaults_and_seeding() {
        assert_eq!(default_views(0).iter().map(|v| v.name.clone()).collect::<Vec<_>>(), vec!["All", "Mine", "Review"]);
        assert_eq!(default_views(1).len(), 1);
        assert_eq!(parse_pr_filter(Some("mine")), PullRequestFilter::Mine);
        assert_eq!(parse_pr_filter(Some("review")), PullRequestFilter::ReviewRequested);
        assert_eq!(parse_pr_filter(None), PullRequestFilter::All);

        // Empty sections get the defaults; a saved set is kept verbatim.
        let mut app = App::new("slate");
        app.apply_views(vec![], vec![], vec![]);
        assert_eq!(app.views[0].len(), 3);
        assert_eq!(app.views[1].len(), 1);

        let custom = vec![SavedView { name: "Stale".into(), filter: None, query: "old".into(), sort: None, hidden_states: vec![] }];
        app.apply_views(custom, vec![], vec![]);
        assert_eq!(app.views[0].len(), 1);
        assert_eq!(app.views[0][0].name, "Stale");
    }

    #[test]
    fn snapshot_captures_current_state_per_section() {
        let mut app = App::new("slate");
        app.apply_views(vec![], vec![], vec![]);

        // Pull Requests: records the base filter + quick-filter, no hidden states.
        app.active = 0;
        app.pr_filter = PullRequestFilter::Mine;
        app.filters[0] = "wip".into();
        let v = app.current_view_snapshot("My PRs".into());
        assert_eq!(v.name, "My PRs");
        assert_eq!(v.filter.as_deref(), Some("mine"));
        assert_eq!(v.query, "wip");
        assert!(v.hidden_states.is_empty());

        // Work Items: records hidden states (sorted, stable) and no PR filter.
        app.active = 1;
        app.wi_hidden_states.insert("Done".into());
        app.wi_hidden_states.insert("Backlog".into());
        let w = app.current_view_snapshot("Active".into());
        assert_eq!(w.filter, None);
        assert_eq!(w.hidden_states, vec!["Backlog".to_string(), "Done".to_string()]);
    }

    #[test]
    fn quick_filter_narrows_and_maps_selection() {
        let mut app = App::new("slate");
        let mut a = pr(None);
        a.title = "Fix login bug".into();
        let mut b = pr(None);
        b.title = "Update deploy pipeline".into();
        b.author.display_name = "Dana".into();
        let mut c = pr(None);
        c.title = "Refactor login flow".into();
        app.prs = vec![pr_row(a), pr_row(b), pr_row(c)];
        app.active = 0;

        // No filter → all rows.
        assert_eq!(app.filtered_pr_indices(), vec![0, 1, 2]);

        // Filter by a title token (case-insensitive).
        app.filters[0] = "LOGIN".into();
        assert_eq!(app.filtered_pr_indices(), vec![0, 2]);

        // Selection is a position into the filtered view: position 1 → original index 2.
        app.pr_state.select(Some(1));
        assert_eq!(app.selected_pr().map(|p| p.title.as_str()), Some("Refactor login flow"));

        // Multi-token AND across fields (title + author).
        app.filters[0] = "deploy dana".into();
        assert_eq!(app.filtered_pr_indices(), vec![1]);

        // No match → empty view, and active_len reflects it.
        app.filters[0] = "zzz".into();
        assert!(app.filtered_pr_indices().is_empty());
        assert_eq!(app.active_len(), 0);
    }

    #[test]
    fn quick_filter_input_edits_and_clears() {
        let mut app = App::new("slate");
        let mut a = pr(None);
        a.title = "alpha".into();
        let mut b = pr(None);
        b.title = "beta".into();
        app.prs = vec![pr_row(a), pr_row(b)];
        app.active = 0;

        app.start_filter();
        assert!(app.filtering);
        for ch in "beta".chars() {
            app.on_filter_key(Key::Char(ch));
        }
        assert_eq!(app.filtered_pr_indices(), vec![1]);
        // Selection re-anchors to the first match as the query changes.
        assert_eq!(app.pr_state.selected(), Some(0));

        // Backspace widens the match.
        app.on_filter_key(Key::Backspace);
        assert_eq!(app.active_filter(), "bet");

        // Enter applies and closes the input, keeping the filter.
        app.on_filter_key(Key::Enter);
        assert!(!app.filtering);
        assert_eq!(app.active_filter(), "bet");

        // Esc while typing clears the filter and closes the input.
        app.start_filter();
        app.on_filter_key(Key::Escape);
        assert!(!app.filtering);
        assert!(app.active_filter().is_empty());
        assert_eq!(app.filtered_pr_indices(), vec![0, 1]);
    }

    #[test]
    fn wi_state_visibility_hides_and_composes_with_quick_filter() {
        let mut app = App::new("slate");
        let mut a = wi(None);
        a.state = "Todo".into();
        a.title = "one".into();
        let mut b = wi(None);
        b.state = "In Progress".into();
        b.title = "two".into();
        let mut c = wi(None);
        c.state = "Done".into();
        c.title = "three".into();
        app.wis = vec![wi_row(a), wi_row(b), wi_row(c)];
        app.active = 1;

        // Nothing hidden → all rows show, distinct states in first-seen order.
        assert_eq!(app.filtered_wi_indices(), vec![0, 1, 2]);
        assert_eq!(app.distinct_wi_states(), vec!["Todo", "In Progress", "Done"]);

        // Hiding a state drops its rows.
        app.wi_hidden_states.insert("Done".into());
        assert_eq!(app.filtered_wi_indices(), vec![0, 1]);

        // State-visibility composes with the `/` quick filter (AND).
        app.filters[1] = "two".into();
        assert_eq!(app.filtered_wi_indices(), vec![1]);

        // A hidden state that isn't currently present is simply inert.
        app.wi_hidden_states.insert("Archived".into());
        app.filters[1].clear();
        assert_eq!(app.filtered_wi_indices(), vec![0, 1]);
    }

    fn diff(files: Vec<FileChange>) -> DiffView {
        DiffView {
            pr_label: "PR".into(),
            url: None,
            files,
            threads: vec![],
            selected: 0,
            scroll: 0,
            focus: DiffFocus::FileList,
            cursor: 0,
            commit_label: None,
            viewed: HashSet::new(),
        }
    }

    fn changed(path: &str, patch: Option<&str>) -> FileChange {
        FileChange {
            path: path.into(),
            kind: FileChangeKind::Modified,
            additions: 1,
            deletions: 1,
            patch: patch.map(Into::into),
        }
    }

    #[test]
    fn diff_line_cursor_navigates_and_clamps() {
        // 4 patch lines (indices 0..3).
        let mut d = diff(vec![changed("a.rs", Some("@@ -1,2 +1,3 @@\n ctx\n+added\n-removed"))]);

        d.enter_patch();
        assert_eq!(d.focus, DiffFocus::Patch);
        assert_eq!(d.cursor, 0);

        d.move_cursor(1);
        assert_eq!(d.cursor, 1);
        d.move_cursor(100); // clamps to last line
        assert_eq!(d.cursor, 3);
        d.move_cursor(-100); // clamps to first line
        assert_eq!(d.cursor, 0);

        d.exit_patch();
        assert_eq!(d.focus, DiffFocus::FileList);

        // Switching files resets the cursor.
        d.cursor = 2;
        d.select_file(0);
        assert_eq!(d.cursor, 0);
    }

    #[test]
    fn diff_enter_patch_is_noop_without_a_patch() {
        let mut d = diff(vec![changed("bin", None)]);
        d.enter_patch();
        assert_eq!(d.focus, DiffFocus::FileList, "no patch → stay in the file list");
    }

    #[test]
    fn diff_toggle_viewed_tracks_progress() {
        let mut d = diff(vec![changed("a.rs", None), changed("b.rs", None)]);
        assert_eq!(d.viewed_count(), 0);
        d.toggle_viewed(); // marks a.rs (selected == 0)
        assert!(d.is_viewed("a.rs"));
        assert_eq!(d.viewed_count(), 1);
        d.select_file(1);
        d.toggle_viewed(); // marks b.rs
        assert_eq!(d.viewed_count(), 2);
        d.toggle_viewed(); // unmarks b.rs
        assert!(!d.is_viewed("b.rs"));
        assert_eq!(d.viewed_count(), 1);
    }

    #[test]
    fn diff_jump_thread_moves_cursor_to_thread_lines() {
        // new-side lines: 20 (ctx, idx 1), 21 (added, idx 2).
        let mut d = diff(vec![changed("a.rs", Some("@@ -10,3 +20,4 @@\n ctx\n+added\n-removed"))]);
        d.threads = vec![
            CommentThread { id: "t1".into(), comments: vec![], file_path: Some("a.rs".into()), line: Some(21), is_resolved: false },
        ];
        d.jump_thread(1); // next thread from cursor 0 → the one at patch line 2
        assert_eq!(d.focus, DiffFocus::Patch);
        assert_eq!(d.cursor, 2);
        d.jump_thread(1); // only one thread → wraps back to it
        assert_eq!(d.cursor, 2);
    }

    #[test]
    fn diff_jump_thread_is_noop_without_located_threads() {
        let mut d = diff(vec![changed("a.rs", Some("@@ -1,1 +1,1 @@\n ctx"))]);
        d.jump_thread(1); // no threads
        assert_eq!(d.focus, DiffFocus::FileList);
        assert_eq!(d.cursor, 0);
    }

    #[test]
    fn diff_thread_at_cursor_matches_only_on_the_anchored_line() {
        // new-side line 21 (added) sits at patch index 2.
        let mut d = diff(vec![changed("a.rs", Some("@@ -10,3 +20,4 @@\n ctx\n+added\n-removed"))]);
        d.threads = vec![CommentThread { id: "t7".into(), comments: vec![], file_path: Some("a.rs".into()), line: Some(21), is_resolved: false }];
        d.cursor = 0;
        assert!(d.thread_at_cursor().is_none(), "cursor not on the thread's line");
        d.cursor = 2;
        assert_eq!(d.thread_at_cursor().map(|t| t.id.as_str()), Some("t7"), "cursor on the anchored line finds it");
    }

    #[test]
    fn commit_diff_scope_restores_whole_pr() {
        let mut d = diff(vec![changed("b.rs", Some("@@ -1 +1 @@\n-p\n+q"))]);
        d.selected = 0;
        d.scroll = 5;
        d.focus = DiffFocus::Patch;
        d.cursor = 2;
        d.commit_label = Some("abc1234 msg".into());
        let mut v = PrView {
            timeline: Vec::new(),
            label: "PR".into(),
            connection_id: "c".into(),
            url: None,
            pr: pr(None),
            tab: 3,
            checks: vec![],
            commits: vec![],
            commit_sel: 0,
            pr_files: vec![changed("a.rs", Some("@@ -1 +1 @@\n-x\n+y"))],
            scroll: 0,
            diff: d,
            pending: vec![],
            review_draft: None,
            reply_target: None,
        };

        v.reset_diff_scope();
        assert_eq!(v.diff.commit_label, None, "scope cleared");
        assert_eq!(v.diff.files.len(), 1);
        assert_eq!(v.diff.files[0].path, "a.rs", "whole-PR files restored");
        assert_eq!(v.diff.selected, 0);
        assert_eq!(v.diff.focus, DiffFocus::FileList);
    }

    #[test]
    fn pr_detail_cache_key_distinguishes_the_same_id_in_different_repos() {
        // A connection's credentials reach every repository it can see, so on a connection
        // spanning several, `#7` alone names more than one pull request — the key must too.
        let a = pr_detail_cache_key("c", &ItemRef::in_repo("acme/pay", "7"));
        let b = pr_detail_cache_key("c", &ItemRef::in_repo("acme/other", "7"));
        assert_ne!(a, b, "same connection and id, different repo, must not collide");
    }

    fn commit(sha: &str) -> Commit {
        Commit { sha: sha.into(), message: "m".into(), author: "a".into(), date: None, url: None }
    }

    /// A detail fetch in which all four calls answered — what the provider returns on a good day.
    fn all_answered(d: PrDetail) -> Box<PrDetailFetch> {
        Box::new(PrDetailFetch {
            timeline: None,
            threads: Some(d.threads),
            files: Some(d.files),
            checks: Some(d.checks),
            commits: Some(d.commits),
        })
    }

    #[test]
    fn pr_detail_for_a_different_pr_than_the_open_one_is_not_applied() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(PrView {
            timeline: Vec::new(),
            label: "PR".into(),
            connection_id: "c".into(),
            url: None,
            pr: pr(None), // id "1"
            tab: 0,
            checks: vec![],
            commits: vec![],
            commit_sel: 0,
            pr_files: vec![],
            scroll: 0,
            diff: diff(vec![]),
            pending: vec![],
            review_draft: None,
            reply_target: None,
        }));

        // A detail landing for a different PR (id "2") on the same connection.
        let other_key = pr_detail_cache_key("c", &ItemRef::new("2"));
        let detail = PrDetail {
            timeline: Vec::new(),
            threads: vec![],
            files: vec![changed("x.rs", None)],
            checks: vec![CheckRun { name: "ci".into(), status: CheckStatus::Failed, url: None }],
            commits: vec![commit("deadbeef")],
        };
        app.on_event(
            AppEvent::PrDetailLoaded { key: other_key.clone(), detail: all_answered(detail), fetched_at: Utc::now() },
            &deps,
        );

        let Screen::PrView(v) = &app.screen else { panic!("expected PrView") };
        assert!(v.checks.is_empty(), "detail for a different PR must not land on this view");
        assert!(v.pr_files.is_empty());
        // It is still good data — just not for what's on screen — so it must be write-through cached.
        assert!(deps.cache.get::<PrDetail>(&other_key).is_some(), "still written through to the cache");
    }

    #[test]
    fn fresh_pr_detail_preserves_user_state_while_patching_the_data() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        let p = pr(None); // id "1", repository None
        let key = pr_detail_cache_key("c", &p.item_ref());
        let mut d = diff(vec![changed("a.rs", Some("@@ -1 +1 @@\n-x\n+y"))]);
        d.scroll = 4;
        d.cursor = 1;
        d.focus = DiffFocus::Patch;
        app.screen = Screen::PrView(Box::new(PrView {
            timeline: Vec::new(),
            label: "PR".into(),
            connection_id: "c".into(),
            url: None,
            pr: p,
            tab: 3,
            checks: vec![],
            commits: vec![],
            commit_sel: 0,
            pr_files: vec![changed("a.rs", None)],
            scroll: 7,
            diff: d,
            pending: vec![LineComment { path: "a.rs".into(), line: 3, side: DiffSide::New, body: "wip".into() }],
            review_draft: Some(DraftComment { path: "a.rs".into(), line: 3, side: DiffSide::New }),
            reply_target: Some("t1".into()),
        }));

        let fresh = PrDetail {
            timeline: Vec::new(),
            threads: vec![],
            files: vec![changed("a.rs", Some("@@ -1 +1 @@\n-x\n+y")), changed("b.rs", None)],
            checks: vec![CheckRun { name: "ci".into(), status: CheckStatus::Passed, url: None }],
            commits: vec![commit("abc123")],
        };
        app.on_event(AppEvent::PrDetailLoaded { key, detail: all_answered(fresh), fetched_at: Utc::now() }, &deps);

        let Screen::PrView(v) = &app.screen else { panic!("expected PrView") };
        assert_eq!(v.scroll, 7, "tab scroll preserved");
        assert_eq!(v.diff.scroll, 4, "diff scroll preserved");
        assert_eq!(v.diff.cursor, 1, "diff cursor preserved");
        assert_eq!(v.diff.focus, DiffFocus::Patch, "diff focus preserved");
        assert_eq!(v.pending.len(), 1, "buffered line comment preserved");
        assert!(v.review_draft.is_some(), "in-progress draft preserved");
        assert_eq!(v.reply_target.as_deref(), Some("t1"), "reply target preserved");
        assert_eq!(v.checks.len(), 1, "fresh checks applied");
        assert_eq!(v.commits.len(), 1, "fresh commits applied");
        assert_eq!(v.diff.files.len(), 2, "fresh files applied");
        assert_eq!(v.pr_files.len(), 2, "fresh files applied to pr_files too");
    }

    #[test]
    fn fresh_pr_detail_clamps_selection_when_the_new_lists_are_shorter() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        let p = pr(None);
        let key = pr_detail_cache_key("c", &p.item_ref());
        let mut d = diff(vec![changed("a.rs", None), changed("b.rs", None), changed("c.rs", None)]);
        d.selected = 2;
        app.screen = Screen::PrView(Box::new(PrView {
            timeline: Vec::new(),
            label: "PR".into(),
            connection_id: "c".into(),
            url: None,
            pr: p,
            tab: 3,
            checks: vec![],
            commits: vec![commit("a"), commit("b")],
            commit_sel: 1,
            pr_files: vec![changed("a.rs", None), changed("b.rs", None), changed("c.rs", None)],
            scroll: 0,
            diff: d,
            pending: vec![],
            review_draft: None,
            reply_target: None,
        }));

        let fresh = PrDetail {
            timeline: Vec::new(),
            threads: vec![],
            files: vec![changed("a.rs", None)], // shrank from 3 to 1
            checks: vec![],
            commits: vec![commit("a")], // shrank from 2 to 1
        };
        app.on_event(AppEvent::PrDetailLoaded { key, detail: all_answered(fresh), fetched_at: Utc::now() }, &deps);

        let Screen::PrView(v) = &app.screen else { panic!("expected PrView") };
        assert_eq!(v.commit_sel, 0, "clamped into the new, shorter commit list");
        assert_eq!(v.diff.selected, 0, "clamped into the new, shorter file list");
    }

    fn thread(id: &str) -> CommentThread {
        CommentThread { id: id.into(), comments: vec![], file_path: Some("a.rs".into()), line: Some(21), is_resolved: false }
    }

    fn check(name: &str, status: CheckStatus) -> CheckRun {
        CheckRun { name: name.into(), status, url: None }
    }

    /// A PR view painted from a detail, the way `open_pr_view_for` paints one from the cache.
    fn pr_view_showing(p: PullRequest, d: &PrDetail) -> Screen {
        let mut dv = diff(d.files.clone());
        dv.threads = d.threads.clone();
        Screen::PrView(Box::new(PrView {
            timeline: Vec::new(),
            label: "PR".into(),
            connection_id: "c".into(),
            url: None,
            pr: p,
            tab: 0,
            checks: d.checks.clone(),
            commits: d.commits.clone(),
            commit_sel: 0,
            pr_files: d.files.clone(),
            scroll: 0,
            diff: dv,
            pending: vec![],
            review_draft: None,
            reply_target: None,
        }))
    }

    /// A detail as it was last known: two files, a thread, a check and a commit.
    fn known_detail() -> PrDetail {
        PrDetail {
            timeline: Vec::new(),
            threads: vec![thread("t1")],
            files: vec![changed("a.rs", None), changed("b.rs", None)],
            checks: vec![check("ci", CheckStatus::Passed)],
            commits: vec![commit("abc123")],
        }
    }

    /// The review's defect 1: `changes()` and `threads()` 502 while the other two answer. The
    /// empty vectors that used to stand in for those failures blanked the open view mid-read and
    /// overwrote the cached entry, so re-opening the PR showed nothing either.
    #[test]
    fn a_partly_failed_detail_fetch_keeps_what_was_already_known() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let p = pr(None);
        let key = pr_detail_cache_key("c", &p.item_ref());
        let seeded = Utc::now() - chrono::TimeDelta::seconds(30);
        cache.put(&key, &known_detail(), seeded);

        let mut app = App::new("slate");
        app.screen = pr_view_showing(p, &known_detail());

        // threads() and changes() failed; checks() and commits() answered.
        app.on_event(
            AppEvent::PrDetailLoaded {
                key: key.clone(),
                detail: Box::new(PrDetailFetch {
                    timeline: None,
                    threads: None,
                    files: None,
                    checks: Some(vec![check("ci", CheckStatus::Failed)]),
                    commits: Some(vec![commit("abc123"), commit("def456")]),
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::PrView(v) = &app.screen else { panic!("expected PrView") };
        assert_eq!(v.pr_files.len(), 2, "a failed changes() must not blank the file list");
        assert_eq!(v.diff.files.len(), 2);
        assert_eq!(v.diff.threads.len(), 1, "a failed threads() must not blank the conversation");
        assert_eq!(v.checks[0].status, CheckStatus::Failed, "the calls that answered are applied");
        assert_eq!(v.commits.len(), 2);

        let cached = cache.get::<PrDetail>(&key).expect("still cached");
        assert_eq!(cached.value.files.len(), 2, "the cached files survive the failed call");
        assert_eq!(cached.value.threads.len(), 1);
        assert_eq!(cached.value.checks[0].status, CheckStatus::Failed, "the fresh fields are cached");
        assert_eq!(cached.value.commits.len(), 2);
    }

    /// A fetch that achieved nothing must be a no-op — not four empty lists written over a good
    /// entry. Nothing is learned, so there is nothing to write and nothing to repaint.
    #[test]
    fn a_wholly_failed_detail_fetch_writes_nothing_and_touches_no_view() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let p = pr(None);
        let key = pr_detail_cache_key("c", &p.item_ref());
        let seeded = Utc::now() - chrono::TimeDelta::seconds(30);
        cache.put(&key, &known_detail(), seeded);

        let mut app = App::new("slate");
        app.screen = pr_view_showing(p, &known_detail());

        app.on_event(
            AppEvent::PrDetailLoaded {
                key: key.clone(),
                detail: Box::new(PrDetailFetch::default()),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::PrView(v) = &app.screen else { panic!("expected PrView") };
        assert_eq!(v.pr_files.len(), 2, "the view is left exactly as it was");
        assert_eq!(v.diff.threads.len(), 1);
        assert_eq!(v.checks.len(), 1);
        assert_eq!(v.commits.len(), 1);

        let cached = cache.get::<PrDetail>(&key).expect("still cached");
        assert_eq!(cached.fetched_at, seeded, "a total failure must not even restamp the entry");
        assert_eq!(cached.value.files.len(), 2);
    }

    /// The other half of the distinction: an *answered* empty list is authoritative — the PR
    /// really has no checks now — and must clear both the view and the cache.
    #[test]
    fn an_answered_empty_detail_section_does_clear_the_previous_one() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let p = pr(None);
        let key = pr_detail_cache_key("c", &p.item_ref());
        cache.put(&key, &known_detail(), Utc::now() - chrono::TimeDelta::seconds(30));

        let mut app = App::new("slate");
        app.screen = pr_view_showing(p, &known_detail());

        app.on_event(
            AppEvent::PrDetailLoaded {
                key: key.clone(),
                detail: Box::new(PrDetailFetch { checks: Some(Vec::new()), ..PrDetailFetch::default() }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::PrView(v) = &app.screen else { panic!("expected PrView") };
        assert!(v.checks.is_empty(), "the provider says there are none, so there are none");
        assert_eq!(v.pr_files.len(), 2, "the sections that weren't fetched are untouched");

        let cached = cache.get::<PrDetail>(&key).expect("cached");
        assert!(cached.value.checks.is_empty());
        assert_eq!(cached.value.files.len(), 2);
    }

    /// The store refuses a write it already has something newer than. Carrying on and painting
    /// the older merge anyway left the screen behind the cache — the same out-of-order landing
    /// the recency guard exists to stop, just on screen instead of on disk.
    #[test]
    fn a_put_the_store_refused_as_stale_never_repaints_the_view() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let p = pr(None);
        let key = pr_detail_cache_key("c", &p.item_ref());
        let newer = Utc::now();
        cache.put(&key, &known_detail(), newer);

        let mut app = App::new("slate");
        app.screen = pr_view_showing(p, &known_detail());

        // Asked for before the entry the store holds, so it lands knowing less.
        app.on_event(
            AppEvent::PrDetailLoaded {
                key: key.clone(),
                detail: Box::new(PrDetailFetch {
                    checks: Some(vec![check("ci", CheckStatus::Failed)]),
                    ..PrDetailFetch::default()
                }),
                fetched_at: newer - chrono::TimeDelta::seconds(30),
            },
            &deps,
        );

        let Screen::PrView(v) = &app.screen else { panic!("expected PrView") };
        assert_eq!(v.checks[0].status, CheckStatus::Passed, "the refused write must not reach the screen either");
        let cached = cache.get::<PrDetail>(&key).expect("cached");
        assert_eq!(cached.fetched_at, newer, "and the store keeps what it had");
    }

    /// A disabled store (`--demo`, and every test) stores nothing, but that is not a refusal:
    /// early-returning on it would mean the view never updated at all.
    #[test]
    fn a_disabled_store_still_repaints_the_view() {
        let deps = test_deps(); // cache is `CacheStore::disabled()`
        let p = pr(None);
        let key = pr_detail_cache_key("c", &p.item_ref());

        let mut app = App::new("slate");
        app.screen = pr_view_showing(p, &known_detail());

        app.on_event(
            AppEvent::PrDetailLoaded {
                key,
                detail: Box::new(PrDetailFetch {
                    checks: Some(vec![check("ci", CheckStatus::Failed)]),
                    ..PrDetailFetch::default()
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::PrView(v) = &app.screen else { panic!("expected PrView") };
        assert_eq!(v.checks[0].status, CheckStatus::Failed, "demo and tests must still paint what landed");
    }

    /// The merge reads the cache, not the screen, so a detail that lands after the user has
    /// navigated away caches the same value it would have cached with the view still open.
    #[test]
    fn a_detail_landing_with_no_view_open_still_merges_against_the_cache() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let p = pr(None);
        let key = pr_detail_cache_key("c", &p.item_ref());
        cache.put(&key, &known_detail(), Utc::now() - chrono::TimeDelta::seconds(30));

        let mut app = App::new("slate"); // no PR view open

        app.on_event(
            AppEvent::PrDetailLoaded {
                key: key.clone(),
                detail: Box::new(PrDetailFetch {
                    commits: Some(vec![commit("zzz999")]),
                    ..PrDetailFetch::default()
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let cached = cache.get::<PrDetail>(&key).expect("cached");
        assert_eq!(cached.value.commits[0].sha, "zzz999", "the answered call is stored");
        assert_eq!(cached.value.files.len(), 2, "the unanswered ones keep their cached values");
        assert_eq!(cached.value.threads.len(), 1);
    }

    // ---- work-item detail caching ----

    #[test]
    fn wi_detail_cache_key_distinguishes_the_same_id_in_different_repos() {
        let a = wi_detail_cache_key("c", &ItemRef::in_repo("acme/pay", "7"));
        let b = wi_detail_cache_key("c", &ItemRef::in_repo("acme/other", "7"));
        assert_ne!(a, b, "same connection and id, different repo, must not collide");
    }

    #[test]
    fn wi_detail_for_a_different_item_than_the_open_one_is_not_applied() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        app.screen = Screen::WiView(Box::new(WiView {
            timeline: Vec::new(),
            connection_id: "c".into(),
            wi: wi(None), // id "w"
            threads: vec![],
            scroll: 0,
        }));

        let other_key = wi_detail_cache_key("c", &ItemRef::new("other"));
        app.on_event(
            AppEvent::WiDetailLoaded {
                key: other_key.clone(),
                detail: Box::new(WiDetailFetch { threads: Some(vec![thread("t1")]), timeline: None }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::WiView(v) = &app.screen else { panic!("expected WiView") };
        assert!(v.threads.is_empty(), "detail for a different item must not land on this view");
        // It is still good data — just not for what's on screen — so it must be write-through cached.
        assert!(deps.cache.get::<WiDetail>(&other_key).is_some(), "still written through to the cache");
    }

    #[test]
    fn a_wholly_failed_wi_detail_fetch_writes_nothing_and_touches_no_view() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let w = wi(None);
        let key = wi_detail_cache_key("c", &w.item_ref());
        let seeded = Utc::now() - chrono::TimeDelta::seconds(30);
        cache.put(&key, &WiDetail { threads: vec![thread("t1")], timeline: Vec::new() }, seeded);

        let mut app = App::new("slate");
        app.screen =
            Screen::WiView(Box::new(WiView { connection_id: "c".into(), wi: w, threads: vec![thread("t1")], scroll: 5, timeline: Vec::new() }));

        app.on_event(
            AppEvent::WiDetailLoaded { key: key.clone(), detail: Box::new(WiDetailFetch::default()), fetched_at: Utc::now() },
            &deps,
        );

        let Screen::WiView(v) = &app.screen else { panic!("expected WiView") };
        assert_eq!(v.threads.len(), 1, "the view is left exactly as it was");
        assert_eq!(v.scroll, 5, "scroll untouched");

        let cached = cache.get::<WiDetail>(&key).expect("still cached");
        assert_eq!(cached.fetched_at, seeded, "a total failure must not even restamp the entry");
    }

    #[test]
    fn fresh_wi_detail_preserves_scroll_while_patching_threads() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        let w = wi(None);
        let key = wi_detail_cache_key("c", &w.item_ref());
        app.screen = Screen::WiView(Box::new(WiView { connection_id: "c".into(), wi: w, threads: vec![], scroll: 9, timeline: Vec::new() }));

        app.on_event(
            AppEvent::WiDetailLoaded {
                key,
                detail: Box::new(WiDetailFetch { threads: Some(vec![thread("t1"), thread("t2")]), timeline: None }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::WiView(v) = &app.screen else { panic!("expected WiView") };
        assert_eq!(v.scroll, 9, "scroll preserved — the view is patched in place, not rebuilt");
        assert_eq!(v.threads.len(), 2, "fresh threads applied");
    }

    // ---- pipeline detail caching ----

    fn pipeline_run(id: &str, status: PipelineRunStatus, stages: Vec<PipelineStage>) -> PipelineRun {
        PipelineRun {
            repository: None,
            id: id.into(),
            definition_id: "ci".into(),
            number: Some(1),
            name: Some("CI".into()),
            title: None,
            status,
            triggered_by: None,
            branch: Some("main".into()),
            commit_sha: None,
            started_at: None,
            finished_at: None,
            url: None,
            stages,
        }
    }

    fn pipeline_stage(name: &str, status: PipelineRunStatus, jobs: Vec<PipelineJob>) -> PipelineStage {
        PipelineStage { name: name.into(), status, jobs }
    }

    fn pipeline_job(id: &str, status: PipelineRunStatus) -> PipelineJob {
        PipelineJob {
            id: id.into(),
            name: id.into(),
            status,
            started_at: None,
            finished_at: None,
            steps: vec![],
            url: None,
            problem: None,
        }
    }

    fn approval(id: &str, can_respond: bool) -> PipelineApproval {
        PipelineApproval { id: id.into(), name: format!("gate-{id}"), can_respond }
    }

    #[test]
    fn pipeline_detail_cache_key_distinguishes_the_same_id_in_different_repos() {
        let a = pipeline_detail_cache_key("c", &ItemRef::in_repo("acme/pay", "7"));
        let b = pipeline_detail_cache_key("c", &ItemRef::in_repo("acme/other", "7"));
        assert_ne!(a, b, "same connection and id, different repo, must not collide");
    }

    #[test]
    fn pipeline_detail_for_a_different_run_than_the_open_one_is_not_applied() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        let run = pipeline_run("1", PipelineRunStatus::Running, vec![]);
        app.screen =
            Screen::Pipeline(Box::new(PipelineView::new("CI".into(), run, "c".into(), ProviderType::GitHub, "ci".into(), None)));

        let other = pipeline_run("2", PipelineRunStatus::Succeeded, vec![]);
        let other_key = pipeline_detail_cache_key("c", &ItemRef::new("2"));
        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key: other_key.clone(),
                detail: Box::new(PipelineDetailFetch {
                    run: Some(other),
                    approvals: Some(vec![]),
                    supports_approvals: Some(true),
                    can_respond_approvals: Some(true),
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert_eq!(v.run.status, PipelineRunStatus::Running, "detail for a different run must not land on this view");
        assert!(deps.cache.get::<PipelineDetail>(&other_key).is_some(), "still written through to the cache");
    }

    /// A revalidation where `get_run` answers but `pending_approvals` 502s: the empty approvals
    /// list that would have stood in for that failure must not blank a real gate — the same
    /// defect the PR path's `changes()`/`threads()` review caught, extended here.
    #[test]
    fn a_partly_failed_pipeline_detail_fetch_keeps_what_was_already_known() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let seeded_run = pipeline_run("1", PipelineRunStatus::Running, vec![]);
        let key = pipeline_detail_cache_key("c", &seeded_run.item_ref());
        let known = PipelineDetail {
            run: seeded_run.clone(),
            approvals: vec![approval("g1", true)],
            supports_approvals: true,
            can_respond_approvals: true,
        };
        cache.put(&key, &known, Utc::now() - chrono::TimeDelta::seconds(30));

        let mut app = App::new("slate");
        let mut view =
            PipelineView::new("CI".into(), seeded_run.clone(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.supports_approvals = true;
        view.can_respond_approvals = true;
        view.approvals = vec![approval("g1", true)];
        view.stale = false; // already confirmed by an earlier fetch
        app.screen = Screen::Pipeline(Box::new(view));

        // get_run answers with a status change; pending_approvals and the capability calls fail.
        let fresh_run = pipeline_run("1", PipelineRunStatus::Failed, vec![]);
        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key: key.clone(),
                detail: Box::new(PipelineDetailFetch {
                    run: Some(fresh_run),
                    approvals: None,
                    supports_approvals: None,
                    can_respond_approvals: None,
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert_eq!(v.run.status, PipelineRunStatus::Failed, "the call that answered is applied");
        assert_eq!(v.approvals.len(), 1, "a failed approvals call must not blank what was known");
        assert!(v.supports_approvals, "capabilities survive the failed call");

        let cached = cache.get::<PipelineDetail>(&key).expect("still cached");
        assert_eq!(cached.value.run.status, PipelineRunStatus::Failed);
        assert_eq!(cached.value.approvals.len(), 1, "the cached approvals survive the failed call");
    }

    /// The hazard `open_pipeline_for` already guards and this path kept re-opening: the cached
    /// entry's gates standing in for a failed `pending_approvals`. A gate on screen drives a real
    /// approve/reject, so one that may already have been decided must never get there — while the
    /// cached entry itself stays complete for a later re-open.
    #[test]
    fn a_failed_approvals_call_never_paints_a_cached_gate_into_the_view() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let run = pipeline_run("1", PipelineRunStatus::Running, vec![]);
        let key = pipeline_detail_cache_key("c", &run.item_ref());
        cache.put(
            &key,
            &PipelineDetail {
                run: run.clone(),
                approvals: vec![approval("g1", true)],
                supports_approvals: true,
                can_respond_approvals: true,
            },
            Utc::now() - chrono::TimeDelta::seconds(30),
        );

        // Opened from the cache: the run seeds, the gates deliberately do not.
        let mut app = App::new("slate");
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.stale = true;
        app.screen = Screen::Pipeline(Box::new(view));

        // get_run answers, pending_approvals 502s.
        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key: key.clone(),
                detail: Box::new(PipelineDetailFetch {
                    run: Some(pipeline_run("1", PipelineRunStatus::Failed, vec![])),
                    approvals: None,
                    supports_approvals: Some(true),
                    can_respond_approvals: Some(true),
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert!(v.approvals.is_empty(), "a cached gate must never reach an open view");
        assert!(v.actionable_approvals().is_empty(), "so nothing can be approved off it");
        assert_eq!(v.run.status, PipelineRunStatus::Failed, "the call that answered still applies");
        assert!(!v.stale, "a confirmed run still clears staleness");

        let cached = cache.get::<PipelineDetail>(&key).expect("cached");
        assert_eq!(cached.value.approvals.len(), 1, "the cached entry stays complete");
        assert_eq!(cached.value.run.status, PipelineRunStatus::Failed);
    }

    /// The other half: an *answered* empty list is authoritative — the gate was decided — and
    /// must clear it from the view rather than being mistaken for a failure.
    #[test]
    fn an_answered_empty_approvals_list_clears_the_views_gates() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let run = pipeline_run("1", PipelineRunStatus::Running, vec![]);
        let key = pipeline_detail_cache_key("c", &run.item_ref());

        let mut app = App::new("slate");
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.supports_approvals = true;
        view.can_respond_approvals = true;
        view.approvals = vec![approval("g1", true)];
        view.stale = false;
        app.screen = Screen::Pipeline(Box::new(view));

        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key: key.clone(),
                detail: Box::new(PipelineDetailFetch {
                    run: Some(run),
                    approvals: Some(Vec::new()),
                    supports_approvals: Some(true),
                    can_respond_approvals: Some(true),
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert!(v.approvals.is_empty(), "the provider says there are no gates, so there are none");
        assert!(cache.get::<PipelineDetail>(&key).expect("cached").value.approvals.is_empty());
    }

    /// Capabilities/approvals answered but `get_run` didn't: the same rule applies on the branch
    /// that patches without confirming the run.
    #[test]
    fn an_unconfirmed_run_also_refuses_the_cached_gates() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let run = pipeline_run("1", PipelineRunStatus::Running, vec![]);
        let key = pipeline_detail_cache_key("c", &run.item_ref());
        cache.put(
            &key,
            &PipelineDetail {
                run: run.clone(),
                approvals: vec![approval("g1", true)],
                supports_approvals: true,
                can_respond_approvals: false,
            },
            Utc::now() - chrono::TimeDelta::seconds(30),
        );

        let mut app = App::new("slate");
        let mut view = PipelineView::new("CI".into(), run, "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.stale = true;
        app.screen = Screen::Pipeline(Box::new(view));

        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key,
                detail: Box::new(PipelineDetailFetch {
                    run: None,
                    approvals: None,
                    supports_approvals: Some(true),
                    can_respond_approvals: Some(true),
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert!(v.approvals.is_empty(), "still no cached gate on screen");
        assert!(v.can_respond_approvals, "the capability that answered is applied");
        assert!(v.stale, "only a real get_run may clear staleness");
    }

    /// Defect 4: the per-run gate check used to fall back to an empty list, so a failure cached
    /// the row as "no approval pending" when one may have been waiting.
    #[test]
    fn a_failed_gate_check_clears_the_pipelines_flag_instead_of_reading_as_no_gate() {
        assert_eq!(gate_from_approvals(Some(vec![approval("g1", true)])), (true, true));
        assert_eq!(gate_from_approvals(Some(vec![approval("g1", false)])), (false, true));
        assert_eq!(gate_from_approvals(Some(Vec::new())), (false, true));
        assert_eq!(gate_from_approvals(None), (false, false), "a failed check must not be cached as clear");
    }

    #[test]
    fn a_wholly_failed_pipeline_detail_fetch_writes_nothing_and_touches_no_view() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let run = pipeline_run("1", PipelineRunStatus::Running, vec![]);
        let key = pipeline_detail_cache_key("c", &run.item_ref());
        let known =
            PipelineDetail { run: run.clone(), approvals: vec![], supports_approvals: false, can_respond_approvals: false };
        let seeded = Utc::now() - chrono::TimeDelta::seconds(30);
        cache.put(&key, &known, seeded);

        let mut app = App::new("slate");
        let mut view = PipelineView::new("CI".into(), run, "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.stale = false;
        app.screen = Screen::Pipeline(Box::new(view));

        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key: key.clone(),
                detail: Box::new(PipelineDetailFetch::default()),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert_eq!(v.run.status, PipelineRunStatus::Running, "the view is left exactly as it was");
        assert!(!v.stale, "an all-failed fetch must not touch stale either");

        let cached = cache.get::<PipelineDetail>(&key).expect("still cached");
        assert_eq!(cached.fetched_at, seeded, "a total failure must not even restamp the entry");
    }

    #[test]
    fn open_pipeline_for_seeds_the_run_from_cache_but_never_seeds_approvals() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let cached_run = pipeline_run("1", PipelineRunStatus::Succeeded, vec![]);
        let key = pipeline_detail_cache_key("c", &cached_run.item_ref());
        cache.put(
            &key,
            &PipelineDetail {
                run: cached_run,
                // Already-actioned in this scenario — must never be painted into a fresh open.
                approvals: vec![approval("g1", true)],
                supports_approvals: true,
                can_respond_approvals: true,
            },
            Utc::now(),
        );

        let mut app = App::new("slate");
        let fallback = pipeline_run("1", PipelineRunStatus::Running, vec![]); // unused on a cache hit
        app.open_pipeline_for(&deps, "c".into(), ProviderType::GitHub, "1".into(), "ci".into(), None, "CI".into(), fallback);

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert_eq!(v.run.status, PipelineRunStatus::Succeeded, "seeded from the cache, not the fallback");
        assert!(v.approvals.is_empty(), "a cached gate must never be painted into a newly opened view");
        assert!(v.stale, "a cache-seeded run is unconfirmed until the refetch lands");
        assert!(v.supports_approvals && v.can_respond_approvals, "capabilities do seed from the cache");
    }

    #[test]
    fn pipeline_stale_flips_to_false_once_a_confirmed_run_lands() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        let run = pipeline_run("1", PipelineRunStatus::Running, vec![]);
        let key = pipeline_detail_cache_key("c", &run.item_ref());
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.stale = true;
        app.screen = Screen::Pipeline(Box::new(view));

        // The fetch confirms exactly the status the cache already guessed — still has to clear
        // `stale`, since nothing had actually confirmed it live until now.
        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key,
                detail: Box::new(PipelineDetailFetch {
                    run: Some(run),
                    approvals: Some(vec![]),
                    supports_approvals: Some(false),
                    can_respond_approvals: Some(false),
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert!(!v.stale, "a real get_run answering clears staleness even if nothing else changed");
    }

    #[test]
    fn fresh_pipeline_detail_preserves_logs_and_collapsed_while_patching() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        let run = pipeline_run(
            "1",
            PipelineRunStatus::Running,
            vec![pipeline_stage("build", PipelineRunStatus::Running, vec![pipeline_job("j1", PipelineRunStatus::Running)])],
        );
        let key = pipeline_detail_cache_key("c", &run.item_ref());
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.logs = Some(LogView::with_lines("Logs · j1", "j1", vec!["hello".into()]));
        view.toggle_selected(); // collapses the "build" stage row (selected starts at 0)
        assert!(view.collapsed.contains("s0"), "sanity: the stage collapsed");
        app.screen = Screen::Pipeline(Box::new(view));

        let fresh = pipeline_run(
            "1",
            PipelineRunStatus::Succeeded,
            vec![pipeline_stage("build", PipelineRunStatus::Succeeded, vec![pipeline_job("j1", PipelineRunStatus::Succeeded)])],
        );
        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key,
                detail: Box::new(PipelineDetailFetch {
                    run: Some(fresh),
                    approvals: Some(vec![]),
                    supports_approvals: Some(false),
                    can_respond_approvals: Some(false),
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert!(v.logs.is_some(), "an open log pane must not be torn down by a background refresh");
        assert!(v.collapsed.contains("s0"), "the expand/collapse tree state survives the patch");
        assert_eq!(v.run.status, PipelineRunStatus::Succeeded, "fresh run applied");
    }

    #[test]
    fn fresh_pipeline_detail_clamps_selection_when_the_new_run_is_shorter() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        let run = pipeline_run(
            "1",
            PipelineRunStatus::Running,
            vec![
                pipeline_stage("build", PipelineRunStatus::Running, vec![pipeline_job("j1", PipelineRunStatus::Running)]),
                pipeline_stage("deploy", PipelineRunStatus::Queued, vec![pipeline_job("j2", PipelineRunStatus::Queued)]),
            ],
        );
        let key = pipeline_detail_cache_key("c", &run.item_ref());
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.selected = 3; // the "deploy" stage row, in the four-row flattened tree
        app.screen = Screen::Pipeline(Box::new(view));

        // Shrinks to one stage with no jobs — a much shorter flattened tree.
        let fresh = pipeline_run("1", PipelineRunStatus::Succeeded, vec![pipeline_stage("build", PipelineRunStatus::Succeeded, vec![])]);
        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key,
                detail: Box::new(PipelineDetailFetch {
                    run: Some(fresh),
                    approvals: Some(vec![]),
                    supports_approvals: Some(false),
                    can_respond_approvals: Some(false),
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert_eq!(v.selected, 0, "clamped into the new, shorter flattened tree");
    }

    #[test]
    fn merged_pr_offers_revert_not_merge() {
        use crate::overlay::{Action, Overlay};
        let view = |status: PullRequestStatus| {
            let mut p = pr(None);
            p.status = status;
            Screen::PrView(Box::new(PrView {
                timeline: Vec::new(),
                label: "PR".into(),
                connection_id: "c".into(),
                url: None,
                pr: p,
                tab: 0,
                checks: vec![],
                commits: vec![],
                commit_sel: 0,
                pr_files: vec![],
                scroll: 0,
                diff: diff(vec![]),
                pending: vec![],
                review_draft: None,
                reply_target: None,
            }))
        };

        // Open PR: `m` opens the merge picker, `R` does nothing.
        let mut app = App::new("slate");
        app.screen = view(PullRequestStatus::Open);
        assert!(!app.active_pr_is_merged());
        app.on_pr_view_key(Key::Char('m'));
        assert!(matches!(app.overlay, Some(Overlay::Picker { .. })), "an open PR can merge");
        app.overlay = None;
        app.on_pr_view_key(Key::Char('R'));
        assert!(app.overlay.is_none(), "an open PR has no revert");

        // Merged PR: `m` is inert, `R` opens the revert confirm.
        let mut app = App::new("slate");
        app.screen = view(PullRequestStatus::Merged);
        assert!(app.active_pr_is_merged());
        app.on_pr_view_key(Key::Char('m'));
        assert!(app.overlay.is_none(), "a merged PR can't be merged again");
        app.on_pr_view_key(Key::Char('R'));
        assert!(
            matches!(app.overlay, Some(Overlay::Confirm { action: Action::PrRevert, .. })),
            "a merged PR reverts",
        );
    }

    #[test]
    fn goto_section_leaves_the_launchpad_for_a_section() {
        // The primitive behind a Launchpad "more…" jump (YourWork → Work Items, RecentPipelines →
        // Pipelines). The PR-view jumps additionally reload, so they're exercised in integration.
        let mut app = App::new("slate");
        app.screen = Screen::Launchpad;
        app.goto_section(Section::Pipelines);
        assert!(matches!(app.screen, Screen::List), "now on a section list");
        assert_eq!(app.active, index_of(Section::Pipelines), "the Pipelines section is active");
    }

    #[test]
    fn add_line_comment_buffers_against_draft() {
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(PrView {
            timeline: Vec::new(),
            label: "PR".into(),
            connection_id: "c".into(),
            url: None,
            pr: pr(None),
            tab: 3,
            checks: vec![],
            commits: vec![],
            commit_sel: 0,
            pr_files: vec![],
            scroll: 0,
            diff: diff(vec![changed("a.rs", Some("@@ -1 +1 @@\n-x\n+y"))]),
            pending: vec![],
            review_draft: Some(DraftComment { path: "a.rs".into(), line: 5, side: DiffSide::New }),
            reply_target: None,
        }));

        app.add_line_comment("looks off".into());
        let Screen::PrView(v) = &app.screen else { panic!("expected PrView") };
        assert_eq!(v.pending.len(), 1);
        assert_eq!(v.pending[0].path, "a.rs");
        assert_eq!(v.pending[0].line, 5);
        assert_eq!(v.pending[0].body, "looks off");
        assert!(v.review_draft.is_none(), "draft consumed after buffering");

        // A blank body (or no draft) doesn't buffer anything.
        app.add_line_comment("   ".into());
        let Screen::PrView(v) = &app.screen else { panic!() };
        assert_eq!(v.pending.len(), 1, "empty body ignored");
    }

    #[test]
    fn pr_view_vote_dialog_targets_the_open_pr_without_a_list_selection() {
        // Opened from the Launchpad: there's no matching PR-list selection.
        let mut app = App::new("slate");
        app.prs.clear();
        app.pr_state.select(None);
        let mut p = pr(None);
        p.title = "Wire up retries".into();
        app.screen = Screen::PrView(Box::new(PrView {
            timeline: Vec::new(),
            label: "PR".into(),
            connection_id: "c".into(),
            url: None,
            pr: p,
            tab: 0,
            checks: vec![],
            commits: vec![],
            commit_sel: 0,
            pr_files: vec![],
            scroll: 0,
            diff: diff(vec![]),
            pending: vec![],
            review_draft: None,
            reply_target: None,
        }));

        app.open_pr_vote(ReviewVote::Approved);
        match &app.overlay {
            Some(crate::overlay::Overlay::Confirm { message, action, .. }) => {
                assert!(message.contains("Wire up retries"), "dialog names the open PR");
                assert!(matches!(action, Action::PrVote(ReviewVote::Approved)));
            }
            _ => panic!("expected the approve confirm dialog to open"),
        }
    }

    fn pr_view_with_pending(pending: Vec<LineComment>) -> Screen {
        Screen::PrView(Box::new(PrView {
            timeline: Vec::new(),
            label: "PR".into(),
            connection_id: "c".into(),
            url: None,
            pr: pr(None),
            tab: 0,
            checks: vec![],
            commits: vec![],
            commit_sel: 0,
            pr_files: vec![],
            scroll: 0,
            diff: diff(vec![]),
            pending,
            review_draft: None,
            reply_target: None,
        }))
    }

    #[test]
    fn esc_with_pending_comments_prompts_before_leaving() {
        let mut app = App::new("slate");
        app.screen = pr_view_with_pending(vec![LineComment {
            path: "a.rs".into(),
            line: 1,
            side: DiffSide::New,
            body: "nit".into(),
        }]);

        app.on_pr_view_key(Key::Escape);

        assert!(matches!(app.screen, Screen::PrView(_)), "does not leave while comments are unsubmitted");
        match &app.overlay {
            Some(Overlay::Picker { kind, .. }) => assert!(matches!(kind, PickerKind::PendingExit)),
            _ => panic!("expected the unsubmitted-comments prompt"),
        }
    }

    #[test]
    fn q_with_pending_comments_prompts_instead_of_leaving() {
        let mut app = App::new("slate");
        app.screen = pr_view_with_pending(vec![LineComment { path: "a.rs".into(), line: 1, side: DiffSide::New, body: "nit".into() }]);

        app.on_pr_view_key(Key::Char('q'));

        assert!(!app.should_quit, "q never quits");
        assert!(matches!(app.screen, Screen::PrView(_)));
        assert!(matches!(&app.overlay, Some(Overlay::Picker { kind, .. }) if matches!(kind, PickerKind::PendingExit)));
    }

    /// `q` is a second spelling of Esc, not a quit: it closes the view it is pressed in.
    #[test]
    fn q_without_pending_comments_leaves_the_pr_view() {
        let mut app = App::new("slate");
        app.screen = pr_view_with_pending(vec![]);
        app.on_pr_view_key(Key::Char('q'));
        assert!(!app.should_quit, "q never quits");
        assert!(!matches!(app.screen, Screen::PrView(_)), "it closes the view instead");
    }

    #[test]
    fn esc_without_pending_leaves_the_pr_view() {
        let mut app = App::new("slate");
        app.screen = pr_view_with_pending(vec![]);

        app.on_pr_view_key(Key::Escape);

        assert!(!matches!(app.screen, Screen::PrView(_)), "leaves the PR view when nothing is buffered");
        assert!(app.overlay.is_none());
    }

    #[test]
    fn selected_url_reads_active_tab_and_subview() {
        let mut app = App::new("slate");
        app.prs.push(pr_row(pr(Some("http://pr"))));
        app.pr_state.select(Some(0));
        app.active = 0;
        assert_eq!(app.selected_url().as_deref(), Some("http://pr"));

        app.wis.push(wi_row(wi(Some("http://wi"))));
        app.wi_state.select(Some(0));
        app.active = 1;
        assert_eq!(app.selected_url().as_deref(), Some("http://wi"));

        // An open sub-view takes precedence over the active tab.
        app.screen = Screen::PrView(Box::new(PrView {
            timeline: Vec::new(),
            label: "x".into(),
            connection_id: "c".into(),
            url: Some("http://prview".into()),
            pr: pr(Some("http://pr")),
            tab: 0,
            checks: vec![],
            commits: vec![],
            commit_sel: 0,
            pr_files: vec![],
            scroll: 0,
            diff: DiffView {
                pr_label: "x".into(),
                url: None,
                files: vec![],
                threads: vec![],
                selected: 0,
                scroll: 0,
                focus: DiffFocus::FileList,
                cursor: 0,
                commit_label: None,
                viewed: HashSet::new(),
            },
            pending: vec![],
            review_draft: None,
            reply_target: None,
        }));
        assert_eq!(app.selected_url().as_deref(), Some("http://prview"));
    }

    #[test]
    fn selected_url_is_none_when_item_has_no_url() {
        let mut app = App::new("slate");
        app.prs.push(pr_row(pr(None)));
        app.pr_state.select(Some(0));
        assert_eq!(app.selected_url(), None);
    }

    #[test]
    fn hiding_a_section_skips_it_in_tabs_and_navigation() {
        let mut app = App::new("slate");
        app.apply_hidden_sections(&[Section::WorkItems]);
        assert_eq!(app.visible_indices(), vec![0, 2]);

        // Tab strip is [Launchpad, Pull Requests, Pipelines] (Work Items hidden).
        // Start on the Launchpad (the default screen).
        app.switch_tab(1); // Launchpad -> PR
        assert!(matches!(app.screen, Screen::List));
        assert_eq!(app.active, 0);
        app.switch_tab(1); // PR -> Pipelines, skipping the hidden Work Items
        assert_eq!(app.active, 2);
        app.switch_tab(1); // wraps back to the Launchpad
        assert!(matches!(app.screen, Screen::Launchpad));

        app.set_tab(1); // tab 1 = Pull Requests
        assert!(matches!(app.screen, Screen::List));
        assert_eq!(app.active, 0);
        app.set_tab(2); // tab 2 = Pipelines
        assert_eq!(app.active, 2);
    }

    /// Neither Esc nor `q` may tear the session down: both are "back / close" on every screen,
    /// and Ctrl-C is the only key that leaves the app.
    #[tokio::test]
    async fn neither_escape_nor_q_quits_from_any_screen() {
        let deps = test_deps();

        // Command Center — the root. Both keys are inert; there is nowhere further back.
        let mut app = App::new("slate");
        app.screen = Screen::Launchpad;
        for key in [Key::Escape, Key::Char('q')] {
            app.on_key(key, &deps).await;
            assert!(!app.should_quit, "{key:?} doesn't quit from the Command Center");
            assert!(matches!(app.screen, Screen::Launchpad));
        }

        // Section list, work item view, pipeline drill-in, config — each steps back.
        for key in [Key::Escape, Key::Char('q')] {
            let mut app = App::new("slate");
            app.active = index_of(Section::Pipelines);
            app.screen = Screen::List;
            app.on_key(key, &deps).await;
            assert!(!app.should_quit, "{key:?} doesn't quit from a section list");
            assert!(matches!(app.screen, Screen::Launchpad));

            let mut app = App::new("slate");
            app.screen = Screen::WiView(Box::new(WiView { connection_id: "c".into(), wi: wi(None), threads: vec![], scroll: 0, timeline: Vec::new() }));
            app.on_key(key, &deps).await;
            assert!(!app.should_quit, "{key:?} doesn't quit from a work item view");
            assert!(matches!(app.screen, Screen::List));

            let mut app = App::new("slate");
            app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI".into(), failed_run(), "c".into(), ProviderType::GitHub, "ci".into(), Some("main".into()))));
            app.on_key(key, &deps).await;
            assert!(!app.should_quit, "{key:?} doesn't quit from a pipeline drill-in");
            assert!(matches!(app.screen, Screen::List));
        }

        // Ctrl-C is what is left.
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.on_key(Key::Quit, &deps).await;
        assert!(app.should_quit, "Ctrl-C still quits");
    }

    /// Esc on a section list is "back", never "quit" — a stray Esc used to tear the whole TUI
    /// down from the list, while every other screen treats it as a step backwards.
    #[tokio::test]
    async fn escape_on_a_section_list_steps_back_instead_of_quitting() {
        let deps = test_deps();

        let mut app = App::new("slate");
        app.active = index_of(Section::Pipelines);
        app.screen = Screen::List;
        app.on_key(Key::Escape, &deps).await;
        assert!(!app.should_quit, "Esc doesn't quit from a section list");
        assert!(matches!(app.screen, Screen::Launchpad), "Esc steps back to the Command Center");

        // With a quick filter still applied, Esc clears that first and stays on the list.
        let mut app = App::new("slate");
        app.active = index_of(Section::Pipelines);
        app.screen = Screen::List;
        app.filters[app.active] = "ci".into();
        app.on_key(Key::Escape, &deps).await;
        assert!(app.filters[app.active].is_empty(), "Esc clears the filter first");
        assert!(matches!(app.screen, Screen::List), "and stays put while doing it");
        assert!(!app.should_quit);

        // The Command Center is the end of the line: Esc there does nothing at all.
        app.on_key(Key::Escape, &deps).await;
        assert!(matches!(app.screen, Screen::Launchpad));
        assert!(!app.should_quit);
    }

    /// Tab is the one key that always walks the tab strip. Opening an item used to trap it:
    /// the full-screen views answered their own keys and dropped Tab on the floor.
    #[tokio::test]
    async fn tab_walks_the_strip_from_inside_an_open_item_view() {
        let deps = test_deps();

        // A PR opened from the Command Center leaves `active` on whatever section was last
        // shown — Tab still has to leave from Pull Requests, the section this item belongs to.
        let mut app = App::new("slate");
        app.active = index_of(Section::Pipelines);
        app.screen = pr_view_showing(pr(None), &known_detail());
        app.on_key(Key::Tab, &deps).await;
        assert!(matches!(app.screen, Screen::List), "Tab leaves the PR view for a section list");
        assert_eq!(app.active, index_of(Section::WorkItems), "the tab after Pull Requests");

        let mut app = App::new("slate");
        app.screen = Screen::WiView(Box::new(WiView { connection_id: "c".into(), wi: wi(None), threads: vec![], scroll: 0, timeline: Vec::new() }));
        app.on_key(Key::Tab, &deps).await;
        assert!(matches!(app.screen, Screen::List));
        assert_eq!(app.active, index_of(Section::Pipelines), "the tab after Work Items");

        // Pipelines is the last tab, so Tab wraps round to the Command Center.
        let mut app = App::new("slate");
        app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI".into(), failed_run(), "c".into(), ProviderType::GitHub, "ci".into(), Some("main".into()))));
        app.on_key(Key::Tab, &deps).await;
        assert!(matches!(app.screen, Screen::Launchpad), "wraps back to the Command Center");

        // The inbox isn't on the strip; Tab enters it from the section that is active behind it.
        let mut app = App::new("slate");
        app.active = index_of(Section::PullRequests);
        app.screen = Screen::Inbox;
        app.on_key(Key::Tab, &deps).await;
        assert!(matches!(app.screen, Screen::List));
        assert_eq!(app.active, index_of(Section::WorkItems));
    }

    /// Leaving by Tab must respect the same guard Esc does: buffered line comments are only
    /// in memory, so walking off the view would drop them silently.
    #[tokio::test]
    async fn tab_out_of_a_pr_view_with_unsubmitted_comments_prompts_first() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.screen = pr_view_showing(pr(None), &known_detail());
        if let Screen::PrView(v) = &mut app.screen {
            v.pending.push(LineComment { path: "a.rs".into(), line: 1, side: DiffSide::New, body: "wait".into() });
        }

        app.on_key(Key::Tab, &deps).await;

        assert!(matches!(app.screen, Screen::PrView(_)), "the view stays put until the prompt is answered");
        assert!(
            matches!(&app.overlay, Some(Overlay::Picker { kind: PickerKind::PendingExit, .. })),
            "same prompt Esc raises"
        );
    }

    #[test]
    fn hiding_the_active_section_falls_back_to_first_visible() {
        let mut app = App::new("slate");
        app.active = 1; // Work Items
        app.apply_hidden_sections(&[Section::WorkItems]);
        assert!(!app.visible[1]);
        assert_eq!(app.active, 0);
    }

    // ---- Pipelines list grouping ----

    /// A run with everything the grouping keys read: repo, branch, commit, start time.
    fn grouped_row(def: &str, repo: &str, branch: &str, sha: &str, at: &str, status: PipelineRunStatus) -> PipeRow {
        PipeRow {
            connection_id: "c".into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            definition_name: Some(def.into()),
            awaiting_approval: false,
            run: PipelineRun {
                repository: Some(repo.into()),
                id: format!("{def}-{sha}"),
                definition_id: def.into(),
                number: Some(1),
                name: None,
                title: None,
                status,
                triggered_by: None,
                branch: Some(branch.into()),
                commit_sha: (!sha.is_empty()).then(|| sha.to_string()),
                started_at: (!at.is_empty()).then(|| at.parse::<DateTime<Utc>>().unwrap()),
                finished_at: None,
                url: None,
                stages: vec![],
            },
        }
    }

    /// The screenshot's shape in miniature: one push fans out to several workflows, and the
    /// same workflow runs again on a later push.
    fn fanned_out() -> Vec<PipeRow> {
        use PipelineRunStatus::{Failed, Succeeded};
        vec![
            grouped_row("CI", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", Succeeded),
            grouped_row("Integration", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", Failed),
            grouped_row("Release", "nz/app", "v1.0", "bbb", "2026-09-24T09:00:00Z", Succeeded),
            grouped_row("CI", "nz/app", "v1.0", "bbb", "2026-09-24T09:00:00Z", Succeeded),
        ]
    }

    /// (subject, commit, runs, failed) — the header cells the grouping decides.
    fn head_cells(app: &App) -> Vec<(String, String, usize, usize)> {
        app.pipe_lines()
            .into_iter()
            .filter_map(|l| match l {
                PipeLine::Head(h) => Some((h.subject, h.commit, h.runs, h.failed)),
                PipeLine::Run(_) => None,
            })
            .collect()
    }

    fn heads(app: &App) -> Vec<(String, usize, usize)> {
        app.pipe_lines()
            .into_iter()
            .filter_map(|l| match l {
                PipeLine::Head(h) => Some((h.subject, h.runs, h.failed)),
                PipeLine::Run(_) => None,
            })
            .collect()
    }

    /// The default view. Groups are ordered by their most recent run, newest first — the
    /// flat list's "what changed last" ordering, lifted to the group.
    #[test]
    fn pipe_lines_group_by_pipeline_ordered_by_most_recent_run() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipes = fanned_out();
        assert_eq!(app.pipe_group, PipeGroup::Pipeline, "grouped by pipeline out of the box");

        // CI last ran 10:00, Integration 10:00 (ties keep list order), Release 09:00.
        assert_eq!(
            heads(&app),
            vec![("CI".to_string(), 2, 0), ("Integration".to_string(), 1, 1), ("Release".to_string(), 1, 0)]
        );
    }

    /// Nothing is expanded until asked: the roll-up is the view, so four runs render as
    /// three lines, not seven.
    #[test]
    fn groups_land_collapsed_and_expand_one_at_a_time() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipes = fanned_out();
        assert_eq!(app.pipe_lines().len(), 3, "three headers, no runs");

        let key = match &app.pipe_lines()[0] {
            PipeLine::Head(h) => h.key.clone(),
            PipeLine::Run(_) => panic!("first line is a header"),
        };
        app.toggle_pipe_group(&key);
        // CI's header plus its two runs, then the other two headers.
        assert_eq!(app.pipe_lines().len(), 5);
        assert!(matches!(app.pipe_lines()[1], PipeLine::Run(_)), "the group's runs follow its header");

        app.toggle_pipe_group(&key);
        assert_eq!(app.pipe_lines().len(), 3, "collapsing puts them away again");
    }

    /// A header reports where the pipeline stands *now* — its latest run — with the failures
    /// beneath it carried by the count instead. Status and Started describe the same run.
    #[test]
    fn group_header_shows_the_latest_runs_status_not_the_worst() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipes = vec![
            grouped_row("CI", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", PipelineRunStatus::Succeeded),
            grouped_row("CI", "nz/app", "main", "bbb", "2026-09-24T09:00:00Z", PipelineRunStatus::Failed),
        ];
        let PipeLine::Head(h) = app.pipe_lines().remove(0) else { panic!("header") };
        assert_eq!(h.status, PipelineRunStatus::Succeeded, "the latest run passed, so the header reads passed");
        assert_eq!((h.runs, h.failed), (2, 1), "the older failure is still announced, as a count");
        assert_eq!(
            h.started,
            Some("2026-09-24T10:00:00Z".parse::<DateTime<Utc>>().unwrap()),
            "Started is that same latest run, not a different one"
        );
    }

    /// Grouping by trigger puts the runs one push started together under one header.
    #[test]
    fn trigger_grouping_collects_one_push_fan_out() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipe_group = PipeGroup::Trigger;
        app.pipes = fanned_out();
        let h = head_cells(&app);
        assert_eq!(h.len(), 2, "two pushes");
        assert_eq!(h[0].2, 2, "the newest push started two workflows");
        assert_eq!(h[0].0, "main", "the subject column names the branch");
        assert_eq!(h[0].1, "aaa", "and the commit column the commit");
    }

    /// Providers that leave `commit_sha` empty must still group: the start minute stands in,
    /// or every run would land in a group of its own.
    #[test]
    fn trigger_grouping_falls_back_to_the_start_minute_without_a_commit() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipe_group = PipeGroup::Trigger;
        app.pipes = vec![
            grouped_row("CI", "nz/app", "main", "", "2026-09-24T10:00:30Z", PipelineRunStatus::Succeeded),
            grouped_row("Integration", "nz/app", "main", "", "2026-09-24T10:00:45Z", PipelineRunStatus::Succeeded),
            grouped_row("CI", "nz/app", "main", "", "2026-09-24T10:02:00Z", PipelineRunStatus::Succeeded),
        ];
        let h = heads(&app);
        assert_eq!(h.len(), 2, "same minute groups together, a later minute does not");
        assert_eq!(h[0].1, 1, "10:02 is its own trigger and is newest");
        assert_eq!(h[1].1, 2, "the two runs from 10:00 share a header");
    }

    /// The same branch name in two repositories is two different branches.
    #[test]
    fn branch_grouping_does_not_merge_across_repositories() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipe_group = PipeGroup::Branch;
        app.pipes = vec![
            grouped_row("CI", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", PipelineRunStatus::Succeeded),
            grouped_row("CI", "nz/other", "main", "bbb", "2026-09-24T09:00:00Z", PipelineRunStatus::Succeeded),
        ];
        assert_eq!(heads(&app).len(), 2, "two repositories, two groups");
    }

    /// Turning grouping off restores exactly the list the tab had before grouping existed.
    #[test]
    fn grouping_off_renders_one_line_per_run() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipe_group = PipeGroup::Off;
        app.pipes = fanned_out();
        let lines = app.pipe_lines();
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| matches!(l, PipeLine::Run(_))), "no headers");
    }

    /// Enter on a header must not open a drill-in — there is no single run to open.
    #[test]
    fn the_cursor_on_a_header_selects_no_run() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipes = fanned_out();
        app.pipe_state.select(Some(0));
        assert!(app.selected_pipe().is_none(), "a header is not a run");

        let key = match &app.pipe_lines()[0] {
            PipeLine::Head(h) => h.key.clone(),
            PipeLine::Run(_) => panic!("header"),
        };
        app.toggle_pipe_group(&key);
        app.pipe_state.select(Some(1));
        assert!(app.selected_pipe().is_some(), "the first child is a run");
    }

    /// Collapsing every group while the cursor sits deep in the list must not leave the
    /// cursor pointing past the end — the bug the display-line model exists to prevent.
    #[test]
    fn collapsing_everything_pulls_the_cursor_back_into_range() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipes = fanned_out();
        app.set_all_pipe_groups(true);
        let expanded = app.pipe_lines().len();
        assert_eq!(expanded, 7, "three headers + four runs");
        app.pipe_state.select(Some(expanded - 1));

        app.set_all_pipe_groups(false);
        assert_eq!(app.pipe_lines().len(), 3);
        assert_eq!(app.pipe_state.selected(), Some(2), "cursor clamped to the last header");
    }

    /// `G` cycles the mode, persists it, and does not leave the cursor on a line that the
    /// new arrangement no longer has.
    #[tokio::test]
    async fn g_cycles_the_grouping_and_persists_it() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.active = 2;
        app.pipes = fanned_out();
        app.set_all_pipe_groups(true);
        app.pipe_state.select(Some(6));

        app.on_key(Key::Char('G'), &deps).await;
        assert_eq!(app.pipe_group, PipeGroup::Trigger);
        assert_eq!(app.pipe_state.selected(), Some(0), "cursor returns to the top");
        assert_eq!(deps.config.snapshot().ui.pipeline_group.as_deref(), Some("trigger"), "persisted");

        app.on_key(Key::Char('G'), &deps).await;
        app.on_key(Key::Char('G'), &deps).await;
        assert_eq!(app.pipe_group, PipeGroup::Off, "pipeline → trigger → branch → off");
        app.on_key(Key::Char('G'), &deps).await;
        assert_eq!(app.pipe_group, PipeGroup::Pipeline, "and back round");
    }

    /// A config written by a newer build must not break an older one.
    #[test]
    fn an_unknown_persisted_grouping_falls_back_to_the_default() {
        let mut app = App::new("slate");
        app.apply_pipe_group(Some("by-phase-of-moon".into()));
        assert_eq!(app.pipe_group, PipeGroup::Pipeline);
        app.apply_pipe_group(Some("branch".into()));
        assert_eq!(app.pipe_group, PipeGroup::Branch);
        app.apply_pipe_group(None);
        assert_eq!(app.pipe_group, PipeGroup::Branch, "None keeps what is already set");
    }

    /// Identity is the definition id, not the name shown in the column. Azure gives every run
    /// its own `name` (a build number) and leaves `definition_name` empty when discovery
    /// fails — keying on the display string would put each run in a group of its own.
    #[test]
    fn pipeline_grouping_keys_on_the_definition_not_its_display_name() {
        let mut app = App::new("slate");
        app.active = 2;
        let mut rows: Vec<PipeRow> = (1..=3)
            .map(|n| {
                let mut r = grouped_row("build", "nz/app", "main", &format!("sha{n}"), "2026-09-24T10:00:00Z", PipelineRunStatus::Succeeded);
                r.definition_name = None;
                r.run.name = Some(format!("20260924.{n}"));
                r
            })
            .collect();
        app.pipes = std::mem::take(&mut rows);
        assert_eq!(heads(&app).len(), 1, "three runs of one definition are one group");
        assert_eq!(heads(&app)[0].1, 3);
    }

    /// The converse: two different workflows are allowed to share a display name, and must
    /// not be merged because of it.
    #[test]
    fn two_definitions_sharing_a_name_stay_apart() {
        let mut app = App::new("slate");
        app.active = 2;
        let mut a = grouped_row("CI", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", PipelineRunStatus::Succeeded);
        let mut b = grouped_row("CI", "nz/app", "main", "bbb", "2026-09-24T09:00:00Z", PipelineRunStatus::Succeeded);
        a.run.definition_id = "111".into();
        b.run.definition_id = "222".into();
        app.pipes = vec![a, b];
        assert_eq!(heads(&app).len(), 2, "same name, different workflows");
    }

    /// A provider that supplies no commit still has to produce headers you can tell apart.
    #[test]
    fn trigger_headers_stay_distinct_without_a_commit() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipe_group = PipeGroup::Trigger;
        app.pipes = vec![
            grouped_row("CI", "nz/app", "main", "", "2026-09-24T10:00:00Z", PipelineRunStatus::Succeeded),
            grouped_row("CI", "nz/app", "main", "", "2026-09-24T12:00:00Z", PipelineRunStatus::Succeeded),
        ];
        // Same branch, no commit: the commit cell falls back to the start time, or the two
        // headers would be identical text.
        let cells: Vec<(String, String)> = head_cells(&app).into_iter().map(|(s, c, _, _)| (s, c)).collect();
        assert_eq!(cells.len(), 2, "two triggers");
        assert_eq!(cells[0].0, cells[1].0, "same branch, so the same subject");
        assert_ne!(cells[0].1, cells[1].1, "told apart by the commit cell: {cells:?}");
        assert!(cells[0].1.starts_with('~'), "marked as a time, not a sha: {:?}", cells[0].1);
    }

    /// `T` and `o` were dead keys on the default view, where the cursor starts on a header.
    /// They now act on the group's most recent run — the one the header's age refers to.
    #[test]
    fn action_keys_on_a_header_act_on_the_groups_newest_run() {
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.active = 2;
        let mut old = grouped_row("CI", "nz/app", "main", "old", "2026-09-24T09:00:00Z", PipelineRunStatus::Succeeded);
        let mut new = grouped_row("CI", "nz/app", "main", "new", "2026-09-24T11:00:00Z", PipelineRunStatus::Succeeded);
        old.run.url = Some("http://old".into());
        new.run.url = Some("http://new".into());
        app.pipes = vec![old, new];
        app.pipe_state.select(Some(0));

        assert!(app.selected_pipe().is_none(), "Enter still treats a header as a header");
        assert_eq!(app.pipe_for_action().map(|p| p.run.id.clone()), Some("CI-new".into()), "newest run");
        assert_eq!(app.selected_url().as_deref(), Some("http://new"), "`o` opens it instead of complaining");
        assert!(app.pipeline_target().is_some(), "`T` has something to trigger");
    }

    /// Space reaches the same handler as Enter, so it has to clear the same origin — or Esc
    /// out of the run it opened lands on the Launchpad instead of the list.
    #[test]
    fn space_clears_the_launchpad_origin_exactly_as_enter_does() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.active = 2;
        app.pipes = fanned_out();
        app.set_all_pipe_groups(true);
        app.pipe_state.select(Some(1)); // a run, under the first header
        app.lp_origin = true;

        app.enter_pipeline_line(&deps);
        assert!(!app.lp_origin, "Space must not leave a stale Launchpad origin behind");
    }

    /// A group that disappears and comes back arrives collapsed, like any other new group.
    #[test]
    fn a_group_that_returns_after_a_refresh_is_collapsed_again() {
        let mut app = App::new("slate");
        app.active = 2;
        app.pipes = fanned_out();
        app.set_all_pipe_groups(true);
        assert!(app.pipe_lines().len() > 3, "expanded");

        // The section comes back empty (a provider error, or the repo left scope), then returns.
        app.pipes = Vec::new();
        app.prune_pipe_expanded();
        app.pipes = fanned_out();
        assert!(app.pipe_expanded.is_empty(), "stale keys dropped");
        assert_eq!(app.pipe_lines().len(), 3, "the returning groups are closed");
    }

    /// Sorting by repository has to move something. Within a pipeline group every run shares
    /// one repository, so the sort applies to the groups instead.
    #[test]
    fn sorting_by_repository_orders_the_groups_alphabetically() {
        use forgetop_core::config::SortPref;
        use PipelineRunStatus::Succeeded as S;
        let mut app = App::new("slate");
        app.active = 2;
        // Deliberately not in alphabetical order, and not in time order either.
        app.pipes = vec![
            grouped_row("CI", "nz/zebra", "main", "aaa", "2026-09-24T12:00:00Z", S),
            grouped_row("CI", "nz/apple", "main", "bbb", "2026-09-24T11:00:00Z", S),
            grouped_row("Release", "nz/mango", "main", "ccc", "2026-09-24T10:00:00Z", S),
        ];

        // Default: newest first.
        assert_eq!(
            heads(&app).into_iter().map(|(s, _, _)| s).collect::<Vec<_>>(),
            vec!["CI", "CI", "Release"],
            "ungrouped-by-repo default is by recency"
        );
        let repos = |app: &App| {
            app.pipe_lines()
                .into_iter()
                .filter_map(|l| match l {
                    PipeLine::Head(h) => Some(h.repo),
                    PipeLine::Run(_) => None,
                })
                .collect::<Vec<_>>()
        };

        app.pipe_sort = Some(SortPref { key: "repository".into(), desc: false });
        assert_eq!(repos(&app), vec!["nz/apple", "nz/mango", "nz/zebra"], "A→Z");

        app.pipe_sort = Some(SortPref { key: "repository".into(), desc: true });
        assert_eq!(repos(&app), vec!["nz/zebra", "nz/mango", "nz/apple"], "and Z→A");
    }

    /// A key that varies inside a group orders the groups by the run each header displays —
    /// its most recent — so the order always matches the cells being sorted on.
    #[test]
    fn a_sort_orders_the_groups_by_what_their_headers_show() {
        use forgetop_core::config::SortPref;
        use PipelineRunStatus::Succeeded as S;
        let mut app = App::new("slate");
        app.active = 2;
        app.pipes = vec![
            grouped_row("CI", "nz/app", "zulu", "aaa", "2026-09-24T10:00:00Z", S),
            grouped_row("CI", "nz/app", "alpha", "bbb", "2026-09-24T09:00:00Z", S),
            grouped_row("Release", "nz/app", "main", "ccc", "2026-09-24T12:00:00Z", S),
        ];
        app.pipe_sort = Some(SortPref { key: "branch".into(), desc: false });
        app.set_all_pipe_groups(true);

        let subjects: Vec<String> = app
            .pipe_lines()
            .into_iter()
            .map(|l| match l {
                PipeLine::Head(h) => h.subject,
                PipeLine::Run(i) => app.pipe_child_subject(&app.pipes[i]),
            })
            .collect();
        // Release's header run is on `main`, CI's is on `zulu`: main sorts first, so Release
        // leads despite CI being nothing to do with recency here.
        assert_eq!(subjects, vec!["Release", "main", "CI", "alpha", "zulu"]);
    }

    // ---- preview pane ----

    /// A Pull Requests list, wide enough for the preview, with `ids` as its rows and the first
    /// one selected.
    fn preview_app(ids: &[&str]) -> App {
        let mut app = App::new("slate");
        app.prs = ids
            .iter()
            .map(|id| {
                let mut p = pr(None);
                p.id = (*id).into();
                p.title = format!("PR {id}");
                pr_row(p)
            })
            .collect();
        app.active = 0;
        app.screen = Screen::List;
        app.pr_state.select(Some(0));
        app.content_w = 150;
        app
    }

    fn preview_pr_id(app: &App) -> Option<String> {
        match app.preview.as_ref().map(|p| &p.view) {
            Some(Screen::PrView(v)) => Some(v.pr.id.clone()),
            _ => None,
        }
    }

    #[tokio::test]
    async fn preview_is_built_for_the_selected_row_only_when_it_fits_and_is_on() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.settle_preview(&deps);
        assert_eq!(preview_pr_id(&app).as_deref(), Some("1"), "wide list previews its selected row");
        assert!(!app.preview.as_ref().unwrap().sent, "building does no I/O");

        app.content_w = PREVIEW_MIN_WIDTH - 1;
        app.settle_preview(&deps);
        assert!(app.preview.is_none(), "a narrow terminal keeps the full-width list");

        app.content_w = 150;
        app.preview_hidden[0] = true;
        app.settle_preview(&deps);
        assert!(app.preview.is_none(), "switched off for the section");
    }

    #[tokio::test]
    async fn preview_fetches_only_once_the_cursor_has_settled() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        for _ in 1..PREVIEW_SETTLE_TICKS {
            app.tick_preview(&deps);
            assert!(!app.preview.as_ref().unwrap().sent, "still settling");
        }
        app.tick_preview(&deps);
        assert!(app.preview.as_ref().unwrap().sent, "sent after the settle ticks");
    }

    #[tokio::test]
    async fn moving_the_cursor_rebuilds_the_preview_for_the_new_row() {
        let deps = test_deps();
        let mut app = preview_app(&["1", "2"]);
        app.settle_preview(&deps);
        app.on_key(Key::Down, &deps).await;
        assert_eq!(preview_pr_id(&app).as_deref(), Some("2"));
        assert!(!app.preview.as_ref().unwrap().sent, "the new row waits for its own settle");
    }

    #[tokio::test]
    async fn p_focuses_the_preview_and_p_hands_it_back_unchanged() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.settle_preview(&deps);

        app.on_key(Key::Char('p'), &deps).await;
        assert!(app.preview_focus);
        assert!(app.preview.is_none(), "the view moved into the screen, not copied");
        let Screen::PrView(v) = &mut app.screen else { panic!("focused preview is the PR view") };
        assert_eq!(v.pr.id, "1");
        v.tab = 2; // the user switched to Checks while focused

        app.on_key(Key::Char('p'), &deps).await;
        assert!(!app.preview_focus);
        assert!(matches!(app.screen, Screen::List));
        match app.preview.as_ref().map(|p| &p.view) {
            Some(Screen::PrView(v)) => assert_eq!(v.tab, 2, "handed back as it was, not rebuilt"),
            _ => panic!("the view returns to the pane"),
        }
    }

    #[tokio::test]
    async fn enter_focuses_the_preview_when_it_is_showing_and_opens_full_screen_when_not() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.settle_preview(&deps);
        app.on_key(Key::Enter, &deps).await;
        assert!(app.preview_focus, "Enter moves context into the pane");
        assert!(matches!(app.screen, Screen::PrView(_)));

        let mut app = preview_app(&["1"]);
        app.preview_hidden[0] = true;
        app.on_key(Key::Enter, &deps).await;
        assert!(!app.preview_focus, "no pane, so Enter opens the full view as before");
        assert!(matches!(app.screen, Screen::PrView(_)));
    }

    #[tokio::test]
    async fn enter_on_a_pipeline_group_expands_it_and_enter_on_a_run_focuses_the_pane() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.pipes = vec![pipe_row("r1", PipelineRunStatus::Failed, false), pipe_row("r2", PipelineRunStatus::Running, false)];
        app.active = 2;
        app.screen = Screen::List;
        app.pipe_state.select(Some(0));
        app.content_w = 150;
        app.settle_preview(&deps);

        app.on_key(Key::Enter, &deps).await;
        assert!(matches!(app.pipe_lines().first(), Some(PipeLine::Head(h)) if h.expanded), "Enter expands the group");
        assert!(matches!(app.screen, Screen::List));
        assert!(!app.preview_focus);

        app.on_key(Key::Down, &deps).await; // onto the group's first run
        app.on_key(Key::Enter, &deps).await;
        assert!(app.preview_focus, "Enter on a run moves into the pane");
        assert!(matches!(app.screen, Screen::Pipeline(_)));
    }

    #[tokio::test]
    async fn only_tab_moves_the_top_nav_from_a_section_list() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        for key in [Key::Left, Key::Right, Key::Char('h'), Key::Char('l')] {
            app.on_key(key, &deps).await;
            assert_eq!(app.active, 0, "{key:?} must not switch sections");
            assert!(matches!(app.screen, Screen::List));
        }
        app.on_key(Key::Tab, &deps).await;
        assert_eq!(app.active, 1, "Tab moves to the next section");
    }

    #[tokio::test]
    async fn shift_tab_walks_the_top_nav_backwards_and_wraps() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        let start = app.top_pos();
        let n = 1 + app.visible_indices().len();
        app.on_key(Key::Tab, &deps).await;
        app.on_key(Key::BackTab, &deps).await;
        assert_eq!(app.top_pos(), start, "Shift-Tab undoes Tab");
        app.on_key(Key::BackTab, &deps).await;
        assert_eq!(app.top_pos(), (start + n - 1) % n, "Shift-Tab steps back, wrapping past the first tab");
        for _ in 0..n {
            app.on_key(Key::BackTab, &deps).await;
        }
        assert_eq!(app.top_pos(), (start + n - 1) % n, "a full backwards lap returns to the same tab");
    }

    #[tokio::test]
    async fn a_focused_preview_offers_the_full_views_approve_prompt() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.settle_preview(&deps);
        app.on_key(Key::Char('p'), &deps).await;
        app.on_key(Key::Char('a'), &deps).await;
        match &app.overlay {
            Some(Overlay::Confirm { action, .. }) => assert!(matches!(action, Action::PrVote(ReviewVote::Approved))),
            _ => panic!("expected the approve confirm dialog"),
        }
    }

    #[tokio::test]
    async fn esc_from_a_focused_preview_returns_to_the_list_with_a_preview() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.settle_preview(&deps);
        app.on_key(Key::Char('p'), &deps).await;
        app.on_key(Key::Escape, &deps).await;
        assert!(matches!(app.screen, Screen::List));
        assert!(!app.preview_focus);
        assert_eq!(preview_pr_id(&app).as_deref(), Some("1"));
    }

    #[tokio::test]
    async fn tab_out_of_a_focused_preview_drops_focus() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.settle_preview(&deps);
        app.on_key(Key::Char('p'), &deps).await;
        app.on_key(Key::Tab, &deps).await;
        assert!(!app.preview_focus, "focus never outlives the focused view");
    }

    #[tokio::test]
    async fn capital_p_switches_the_preview_off_for_the_section_and_saves_it() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.settle_preview(&deps);
        app.on_key(Key::Char('P'), &deps).await;
        assert!(app.preview_hidden[0]);
        assert!(app.preview.is_none());
        assert_eq!(deps.config.snapshot().ui.preview_hidden, vec![Section::PullRequests]);

        app.on_key(Key::Char('P'), &deps).await;
        assert!(!app.preview_hidden[0]);
        assert!(deps.config.snapshot().ui.preview_hidden.is_empty());
        assert!(app.preview.is_some(), "back on, and rebuilt straight away");
    }

    #[tokio::test]
    async fn a_detail_answer_for_the_previewed_item_patches_the_pane() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.settle_preview(&deps);
        let key = app.preview.as_ref().unwrap().key.clone();
        let check = CheckRun { name: "build".into(), status: CheckStatus::Passed, url: None };
        let detail = PrDetail { threads: vec![], files: vec![], checks: vec![check], commits: vec![], timeline: Vec::new() };
        app.on_event(AppEvent::PrDetailLoaded { key, detail: all_answered(detail), fetched_at: Utc::now() }, &deps);
        match app.preview.as_ref().map(|p| &p.view) {
            Some(Screen::PrView(v)) => assert_eq!(v.checks.len(), 1, "the answer landed in the preview"),
            _ => panic!("preview kept"),
        }
        assert!(matches!(app.screen, Screen::List), "the list stays the screen");
    }

    #[tokio::test]
    async fn a_pipeline_group_row_previews_its_most_recent_run() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let mut old = pipe_row("r1", PipelineRunStatus::Failed, false);
        old.run.started_at = Some(Utc::now() - chrono::Duration::hours(2));
        let mut new = pipe_row("r2", PipelineRunStatus::Running, false);
        new.run.started_at = Some(Utc::now());
        app.pipes = vec![old, new];
        app.active = 2;
        app.screen = Screen::List;
        app.pipe_state.select(Some(0));
        app.content_w = 150;
        assert!(matches!(app.pipe_lines().first(), Some(PipeLine::Head(_))), "grouped: row 0 is the header");
        app.settle_preview(&deps);
        match app.preview.as_ref().map(|p| &p.view) {
            Some(Screen::Pipeline(v)) => assert_eq!(v.run.id, "r2"),
            _ => panic!("expected a pipeline preview"),
        }
    }

    #[tokio::test]
    async fn the_list_and_its_preview_render_side_by_side() {
        use ratatui::{backend::TestBackend, Terminal};
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.prs[0].pr.title = "Rotate the signing keys".into();
        let mut terminal = Terminal::new(TestBackend::new(160, 24)).unwrap();
        terminal.draw(|f| crate::ui::render(f, &mut app)).unwrap();
        app.settle_preview(&deps);
        terminal.draw(|f| crate::ui::render(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let row = |y: u16| (0..160).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        let screen: Vec<String> = (0..24).map(row).collect();
        let title_row = screen.iter().find(|r| r.contains("Pull Requests ·")).expect("list title");
        assert!(title_row.contains("PR #1"), "the preview's header sits beside the list's: {title_row}");
    }

    // ---- live logs ----

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|l| l.to_string()).collect()
    }

    fn log_app(log: LogView, status: PipelineRunStatus) -> App {
        let mut app = App::new("slate");
        let run = pipeline_run("1", status, vec![pipeline_stage("build", status, vec![pipeline_job("j1", status)])]);
        let mut view = PipelineView::new("CI".into(), run, "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.logs = Some(log);
        app.screen = Screen::Pipeline(Box::new(view));
        app
    }

    fn open_log(app: &App) -> &LogView {
        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        v.logs.as_ref().expect("log pane open")
    }

    #[test]
    fn error_line_matcher_catches_failures_and_skips_zero_counts() {
        for line in [
            "##[error]Process completed with exit code 1.",
            "  [FAIL] MyTests.Adds",
            "Build FAILED.",
            "error: could not compile `app`",
            "error[E0308]: mismatched types",
            "[ERROR] Failed to execute goal",
            "Error: Cannot find module 'x'",
            "Process completed with exit code 2.",
            "container exited with code 137",
            "npm ERR! code ELIFECYCLE",
            "Failed!  - Failed: 1, Passed: 97, Skipped: 0",
            "Tests Failed: 3",
            "FAIL src/cart.test.ts",
        ] {
            assert!(is_error_line(line), "should match: {line}");
        }
        for line in [
            "Build succeeded. 0 errors, 0 warnings",
            "Failed: 0, Passed: 12, Skipped: 0",
            "Process completed with exit code 0.",
            "exited with code 0",
            "0 FAILED",
            "ERROR: 0",
            "handle_error(x)",
            "TERRORS",
            "no errors here",
            "Passed!  - Failed: 0, Passed: 212, Skipped: 0",
            "Retrying: Failed to reach host, attempt 2",
            "PASS src/cart.test.ts",
        ] {
            assert!(!is_error_line(line), "should not match: {line}");
        }
    }

    #[test]
    fn first_error_line_finds_the_first_match_or_none() {
        assert_eq!(first_error_line(&lines(&["ok", "0 errors", "error: boom", "FAILED"])), Some(2));
        assert_eq!(first_error_line(&lines(&["ok", "fine"])), None);
        assert_eq!(first_error_line(&[]), None);
    }

    #[test]
    fn search_steps_through_matches_with_wraparound() {
        assert_eq!(next_match_index(0, None, true), None);
        assert_eq!(next_match_index(3, None, true), Some(0));
        assert_eq!(next_match_index(3, None, false), Some(2));
        assert_eq!(next_match_index(3, Some(2), true), Some(0), "wraps forward");
        assert_eq!(next_match_index(3, Some(0), false), Some(2), "wraps backward");

        let mut log = LogView::with_lines("Logs", "j1", lines(&["a", "Failed one", "b", "FAILED two", "c", "failed three"]));
        log.search_input = Some("failed".into());
        log.commit_search();
        assert_eq!(log.matches, vec![1, 3, 5], "case-insensitive");
        assert_eq!(log.match_idx, Some(0));
        log.step_match(true);
        log.step_match(true);
        assert_eq!(log.match_idx, Some(2));
        log.step_match(true);
        assert_eq!(log.match_idx, Some(0), "n wraps to the first match");
        log.step_match(false);
        assert_eq!(log.match_idx, Some(2), "N wraps to the last match");
        assert_eq!(match_ranges("a FaIlEd b failed", "failed"), vec![(2, 8), (11, 17)]);
    }

    #[test]
    fn new_lines_keep_the_current_match_and_the_line_cap_keeps_the_tail() {
        let (kept, dropped) = cap_tail((0..12).map(|i| i.to_string()).collect(), 10);
        assert_eq!(dropped, 2);
        assert_eq!(kept.first().map(String::as_str), Some("2"), "the head is dropped");
        assert_eq!(kept.last().map(String::as_str), Some("11"), "the tail is kept");

        let mut log = LogView::with_lines("Logs", "j1", lines(&["x hit", "y", "z hit"]));
        log.query = Some("hit".into());
        log.recompute_matches();
        log.step_match(true);
        log.step_match(true);
        assert_eq!(log.match_idx, Some(1));
        log.set_text("x hit\ny\nz hit\nw hit");
        assert_eq!(log.matches, vec![0, 2, 3]);
        assert_eq!(log.match_idx, Some(1), "still on `z hit` after new lines arrive");

        let big: String = (0..LOG_MAX_LINES + 5).map(|i| format!("line {i}\n")).collect();
        log.set_text(&big);
        assert_eq!(log.lines.len(), LOG_MAX_LINES);
        assert_eq!(log.lines.last().map(String::as_str), Some(format!("line {}", LOG_MAX_LINES + 4).as_str()));
    }

    #[test]
    fn scrolling_up_or_jumping_turns_follow_off_and_end_turns_it_on() {
        let many: Vec<String> = (0..100).map(|i| if i == 40 { "error: boom".into() } else { format!("l{i}") }).collect();
        let mut app = log_app(LogView::with_lines("Logs", "j1", many), PipelineRunStatus::Running);
        let follow = |app: &App| open_log(app).follow;
        if let Screen::Pipeline(v) = &mut app.screen {
            let log = v.logs.as_mut().unwrap();
            log.viewport.set(10);
            log.follow = true;
        }
        assert_eq!(open_log(&app).effective_scroll(), 90, "following pins the view to the bottom");
        app.on_pipeline_logs_key(Key::Up);
        assert!(!follow(&app), "scrolling up stops following");
        assert_eq!(open_log(&app).effective_scroll(), 89);
        app.on_pipeline_logs_key(Key::Char('G'));
        assert!(follow(&app), "G follows again");
        app.on_pipeline_logs_key(Key::Char('E'));
        assert!(!follow(&app), "E stops following");
        assert_eq!(open_log(&app).effective_scroll(), 38, "the error line with two lines of context above");
        assert_eq!(open_log(&app).note.as_deref(), Some("first error at line 41"));
        app.on_pipeline_logs_key(Key::End);
        app.on_pipeline_logs_key(Key::Char('/'));
        for c in "l7".chars() {
            app.on_pipeline_logs_key(Key::Char(c));
        }
        app.on_pipeline_logs_key(Key::Enter);
        assert!(!follow(&app), "a search jump stops following");
        app.on_pipeline_logs_key(Key::Char('f'));
        assert!(follow(&app), "f toggles follow back on");
        app.on_pipeline_logs_key(Key::Char('n'));
        assert!(!follow(&app), "n stops following");
        app.on_pipeline_logs_key(Key::Home);
        assert_eq!(open_log(&app).effective_scroll(), 0);

        // Nothing to scroll must not panic.
        let mut log = LogView::with_lines("Logs", "j1", vec![]);
        log.scroll_by(-5);
        log.scroll_by(5);
        log.jump_first_error();
        log.step_match(true);
        assert_eq!(log.effective_scroll(), 0);
        assert_eq!(log.note.as_deref(), Some("no errors found"));
    }

    #[test]
    fn polling_is_requested_only_while_the_job_is_active() {
        let mut live = LogView::with_lines("Logs", "j1", vec![]);
        live.live = true;
        let polls = (0..LOG_POLL_TICKS * 2).filter(|_| live.poll_step(true)).count();
        assert_eq!(polls, 2, "one poll per interval while live");
        assert!(live.poll_step(false), "one final fetch once the job finishes, for the full tail");
        assert!(!(0..LOG_POLL_TICKS * 3).any(|_| live.poll_step(false)), "then never again");

        let mut done = LogView::with_lines("Logs", "j1", vec![]);
        assert!(!(0..LOG_POLL_TICKS * 3).any(|_| done.poll_step(false)), "a finished job is never polled");

        let mut loading = LogView::new("Logs".into(), "j1".into(), true);
        assert!(!(0..LOG_POLL_TICKS * 3).any(|_| loading.poll_step(true)), "no poll before the first fetch lands");

        // The drill-in's job status drives it.
        let app = log_app(LogView::with_lines("Logs", "j1", vec![]), PipelineRunStatus::Running);
        let Screen::Pipeline(v) = &app.screen else { panic!() };
        assert!(v.log_target_active());
        let app = log_app(LogView::with_lines("Logs", "j1", vec![]), PipelineRunStatus::Failed);
        let Screen::Pipeline(v) = &app.screen else { panic!() };
        assert!(!v.log_target_active());
    }

    #[test]
    fn a_log_answer_for_another_job_or_run_is_dropped() {
        let mut app = log_app(LogView::new("Logs".into(), "j1".into(), true), PipelineRunStatus::Running);
        app.apply_pipeline_logs("c", "1", "j2", Ok("other job".into()));
        app.apply_pipeline_logs("c", "2", "j1", Ok("other run".into()));
        app.apply_pipeline_logs("other", "1", "j1", Ok("other connection".into()));
        assert!(!open_log(&app).loaded, "no stale answer lands");
        app.apply_pipeline_logs("c", "1", "j1", Ok("mine".into()));
        assert_eq!(open_log(&app).lines, lines(&["mine"]));

        // A failed poll keeps the last good lines and says so quietly.
        app.apply_pipeline_logs("c", "1", "j1", Err("timeout".into()));
        assert_eq!(open_log(&app).lines, lines(&["mine"]));
        assert!(open_log(&app).fetch_failed);
        assert!(app.toast.is_none(), "a failed poll doesn't toast");

        // A failed first fetch closes the pane with a toast, as opening always did.
        let mut app = log_app(LogView::new("Logs".into(), "j1".into(), true), PipelineRunStatus::Running);
        app.apply_pipeline_logs("c", "1", "j1", Err("nope".into()));
        let Screen::Pipeline(v) = &app.screen else { panic!() };
        assert!(v.logs.is_none());
        assert!(app.toast.as_deref().is_some_and(|t| t.contains("nope")));
    }

    #[test]
    fn tree_moves_retarget_the_log_pane_only_across_jobs() {
        let mut job1 = pipeline_job("j1", PipelineRunStatus::Succeeded);
        job1.steps = vec![PipelineStep { name: "s".into(), status: PipelineRunStatus::Succeeded, started_at: None, finished_at: None }];
        let job2 = pipeline_job("j2", PipelineRunStatus::Running);
        let run = pipeline_run("1", PipelineRunStatus::Running, vec![pipeline_stage("build", PipelineRunStatus::Running, vec![job1, job2])]);
        let mut app = App::new("slate");
        app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI".into(), run, "c".into(), ProviderType::GitHub, "ci".into(), None)));
        app.on_pipeline_screen_key(Key::Down); // → job j1
        app.on_pipeline_screen_key(Key::Char('L'));
        assert_eq!(open_log(&app).job_id, "j1");
        assert!(!open_log(&app).follow, "a finished job doesn't follow");
        app.on_pipeline_screen_key(Key::Char('w')); // keys to the tree
        if let Screen::Pipeline(v) = &mut app.screen {
            v.logs.as_mut().unwrap().loaded = true;
        }
        app.on_pipeline_screen_key(Key::Down); // → j1's step: same job, pane untouched
        assert!(open_log(&app).loaded, "a step of the same job doesn't refetch");
        app.on_pipeline_screen_key(Key::Down); // → job j2
        assert_eq!(open_log(&app).job_id, "j2");
        assert!(!open_log(&app).loaded && open_log(&app).follow, "a new, live job loads and follows");
        app.on_pipeline_screen_key(Key::Escape);
        let Screen::Pipeline(v) = &app.screen else { panic!("Esc from the tree closes the logs, not the view") };
        assert!(v.logs.is_none());
    }

    #[test]
    fn a_failed_run_selects_its_failed_step_on_first_load_only() {
        let mut job = pipeline_job("j2", PipelineRunStatus::Failed);
        job.steps = vec![
            PipelineStep { name: "restore".into(), status: PipelineRunStatus::Succeeded, started_at: None, finished_at: None },
            PipelineStep { name: "test".into(), status: PipelineRunStatus::Failed, started_at: None, finished_at: None },
        ];
        let run = pipeline_run(
            "1",
            PipelineRunStatus::Failed,
            vec![
                pipeline_stage("build", PipelineRunStatus::Succeeded, vec![pipeline_job("j1", PipelineRunStatus::Succeeded)]),
                pipeline_stage("test", PipelineRunStatus::Failed, vec![job]),
            ],
        );
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.collapsed.insert("s1".into());
        view.collapsed.insert("s1.j0".into());
        view.stale = true;
        view.auto_select_failed();
        let nodes = view.flatten();
        assert_eq!(nodes[view.selected].label, "test", "the failed step, parents expanded");
        assert_eq!(nodes[view.selected].depth, 2);
        assert!(view.auto_logs, "its logs are asked to open");

        // Later loads leave the user's cursor alone.
        view.move_sel(-1);
        let moved = view.selected;
        view.apply_fresh_run(run.clone(), None);
        assert_eq!(view.selected, moved);

        // A run confirmed running at first load isn't jumped when it fails later.
        let mut running = run.clone();
        running.status = PipelineRunStatus::Running;
        let mut view = PipelineView::new("CI".into(), running.clone(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.stale = true;
        view.auto_select_failed(); // the cache-seeded open
        view.apply_fresh_run(running, None); // the first live load
        view.apply_fresh_run(run, None); // a later refresh finds it failed
        assert_eq!(view.selected, 0);
        assert!(!view.auto_logs);
    }

    #[tokio::test]
    async fn opening_a_failed_run_opens_the_failed_jobs_logs() {
        let deps = deps_with_cache(memory_cache());
        let mut app = App::new("slate");
        let fallback = failed_run();
        app.open_pipeline_for(&deps, "c".into(), ProviderType::GitHub, "r1".into(), "ci".into(), None, "CI".into(), fallback);
        app.drive_logs(&deps);
        let Screen::Pipeline(v) = &app.screen else { panic!() };
        assert_eq!(v.flatten()[v.selected].label, "unit");
        assert_eq!(v.logs.as_ref().map(|l| l.job_id.as_str()), Some("j1"));
        assert!(v.log_focus);
    }


    // ---- command palette (Ctrl-K) ----

    async fn type_keys(app: &mut App, text: &str, deps: &AppDeps) {
        for ch in text.chars() {
            app.on_key(Key::Char(ch), deps).await;
        }
    }

    /// Open the palette, type `query` and run the top result.
    async fn palette_run(app: &mut App, query: &str, deps: &AppDeps) {
        app.on_key(Key::Ctrl('k'), deps).await;
        type_keys(app, query, deps).await;
        app.on_key(Key::Enter, deps).await;
    }

    fn open_pr_screen(status: PullRequestStatus) -> Screen {
        let mut p = pr(Some("https://example.test/pr/1"));
        p.status = status;
        let mut screen = pr_view_showing(p, &known_detail());
        if let Screen::PrView(v) = &mut screen {
            v.url = v.pr.url.clone();
        }
        screen
    }

    fn user(name: &str, handle: &str) -> User {
        User { id: handle.into(), display_name: name.into(), handle: Some(handle.into()), avatar_url: None }
    }

    #[tokio::test]
    async fn ctrl_k_and_ctrl_p_open_the_palette_from_every_screen() {
        let deps = test_deps();
        type MakeScreen = Box<dyn Fn(&App) -> Screen>;
        let screens: Vec<(&str, MakeScreen)> = vec![
            ("Launchpad", Box::new(|_| Screen::Launchpad)),
            ("List", Box::new(|_| Screen::List)),
            ("PrView", Box::new(|_| open_pr_screen(PullRequestStatus::Open))),
            ("WiView", Box::new(|_| Screen::WiView(Box::new(WiView { connection_id: "c".into(), wi: wi(None), threads: vec![], scroll: 0, timeline: Vec::new() })))),
            (
                "Pipeline",
                Box::new(|_| {
                    Screen::Pipeline(Box::new(PipelineView::new("CI".into(), failed_run(), "c".into(), ProviderType::GitHub, "ci".into(), None)))
                }),
            ),
            ("Inbox", Box::new(|_| Screen::Inbox)),
            ("Config", Box::new(|app: &App| Screen::Config(Box::new(app.build_config_view(&test_deps()))))),
        ];
        for (name, make) in &screens {
            for key in [Key::Ctrl('k'), Key::Ctrl('p')] {
                let mut app = App::new("slate");
                app.screen = make(&app);
                app.on_key(key, &deps).await;
                assert!(matches!(app.overlay, Some(Overlay::Palette { .. })), "{key:?} opens the palette on {name}");
                // Ctrl-K closes it again, quietly.
                app.on_key(Key::Ctrl('k'), &deps).await;
                assert!(app.overlay.is_none(), "Ctrl-K closes the palette on {name}");
                assert!(app.toast.is_none(), "closing the palette is not a cancelled action");
            }
        }
    }

    #[tokio::test]
    async fn the_palette_does_not_steal_ctrl_k_from_the_quick_filter() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.filtering = true;
        app.on_key(Key::Ctrl('k'), &deps).await;
        assert!(app.overlay.is_none(), "the quick filter keeps its keys");
    }

    #[tokio::test]
    async fn running_approve_from_the_palette_equals_pressing_a() {
        let deps = test_deps();
        let mut keyed = App::new("slate");
        keyed.screen = open_pr_screen(PullRequestStatus::Open);
        keyed.on_key(Key::Char('a'), &deps).await;

        let mut palette = App::new("slate");
        palette.screen = open_pr_screen(PullRequestStatus::Open);
        palette.on_key(Key::Ctrl('k'), &deps).await;
        type_keys(&mut palette, "approve", &deps).await;
        let Some(Overlay::Palette { candidates, results, selected, .. }) = &palette.overlay else { panic!("palette open") };
        let top = &candidates[results[*selected]];
        assert_eq!((top.title.as_str(), &top.target), ("Approve", &PaletteTarget::Key(Key::Char('a'))), "the context action ranks first");
        palette.on_key(Key::Enter, &deps).await;

        match (&keyed.overlay, &palette.overlay) {
            (
                Some(Overlay::Confirm { title: t1, message: m1, action: Action::PrVote(ReviewVote::Approved) }),
                Some(Overlay::Confirm { title: t2, message: m2, action: Action::PrVote(ReviewVote::Approved) }),
            ) => assert_eq!((t1, m1), (t2, m2), "same confirm either way"),
            _ => panic!("both paths should end on the approve confirm"),
        }
    }

    #[test]
    fn context_actions_follow_the_pr_views_own_conditions() {
        let mut app = App::new("slate");
        app.screen = open_pr_screen(PullRequestStatus::Open);
        let keys: Vec<Key> = app.context_actions().into_iter().map(|(_, k)| k).collect();
        for k in ['a', 'x', 'm', 'c', 'r', 'o'] {
            assert!(keys.contains(&Key::Char(k)), "open PR offers {k}");
        }
        assert!(!keys.contains(&Key::Char('R')), "no revert on an open PR");
        assert!(!keys.contains(&Key::Char('s')), "no submit without pending comments");
        assert!(!keys.contains(&Key::Char('v')), "diff-only keys only on the Diff tab");

        app.screen = open_pr_screen(PullRequestStatus::Merged);
        let keys: Vec<Key> = app.context_actions().into_iter().map(|(_, k)| k).collect();
        assert!(keys.contains(&Key::Char('R')));
        assert!(!keys.contains(&Key::Char('a')) && !keys.contains(&Key::Char('m')), "a merged PR can't be approved or merged");
    }

    #[tokio::test]
    async fn the_palette_sets_and_persists_a_theme() {
        let deps = test_deps();
        let mut app = App::new("slate");
        palette_run(&mut app, "theme: matrix", &deps).await;
        assert_eq!(app.theme.name, "matrix");
        assert_eq!(deps.config.snapshot().ui.theme.as_deref(), Some("matrix"), "persisted like `t`");

        palette_run(&mut app, ":theme light", &deps).await;
        assert_eq!(app.theme.name, "light", "the :theme command does the same");
    }

    #[tokio::test]
    async fn a_view_entry_switches_section_and_applies_the_view() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let view = |name: &str, query: &str| SavedView { name: name.into(), filter: None, query: query.into(), sort: None, hidden_states: vec![] };
        app.views[1] = vec![view("Everything", ""), view("Blocked on review", "blocked")];
        app.screen = Screen::Launchpad;
        palette_run(&mut app, "blocked on review", &deps).await;
        assert!(matches!(app.screen, Screen::List));
        assert_eq!(app.active, 1);
        assert_eq!(app.view_idx[1], 1);
        assert_eq!(app.filters[1], "blocked", "the view's quick filter is applied");
    }

    #[tokio::test]
    async fn a_person_entry_filters_the_section_they_appear_in() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let mut p = pr(None);
        p.author = user("Ada Lovelace", "ada");
        let mut w = wi(None);
        w.assignee = Some(user("Grace Hopper", "grace"));
        app.prs = vec![pr_row(p)];
        app.wis = vec![wi_row(w)];

        palette_run(&mut app, "@grace", &deps).await;
        assert!(matches!(app.screen, Screen::List));
        assert_eq!(app.active, 1, "an assignee with no PRs lands on Work Items");
        assert_eq!(app.filters[1], "Grace Hopper");
        assert_eq!(app.filtered_wi_indices(), vec![0], "the filter matches their items");

        palette_run(&mut app, "@ada", &deps).await;
        assert_eq!(app.active, 0, "a PR author lands on Pull Requests");
        assert_eq!(app.filters[0], "Ada Lovelace");
        assert_eq!(app.filtered_pr_indices(), vec![0]);
    }

    #[tokio::test]
    async fn a_help_key_runs_when_the_screen_answers_it_and_explains_otherwise() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.screen = Screen::List;
        // `u` only works in a work-item view, so from a list the palette says where to press it.
        palette_run(&mut app, "?update state", &deps).await;
        assert!(app.overlay.is_none());
        assert_eq!(app.toast.as_deref(), Some("Press u in Work Item view (after Enter)"));

        app.screen = Screen::Launchpad;
        palette_run(&mut app, "?choose which tabs", &deps).await;
        assert!(app.toast.as_deref().is_some_and(|t| t.starts_with("Press v")), "v isn't a Launchpad key");
        app.screen = Screen::List;
        palette_run(&mut app, "?choose which tabs", &deps).await;
        assert!(matches!(app.overlay, Some(Overlay::Toggle { kind: ToggleKind::Sections, .. })), "v runs on a list");
    }

    #[tokio::test]
    async fn an_item_opened_from_the_inbox_palette_returns_to_the_inbox() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let mut p = pr(None);
        p.title = "Rotate the signing keys".into();
        app.prs.push(pr_row(p));
        app.screen = Screen::Inbox;
        app.preview_focus = true;
        palette_run(&mut app, "#rotate", &deps).await;
        assert!(matches!(app.screen, Screen::PrView(_)));
        assert!(!app.preview_focus, "a palette-opened item is a full view, not the preview");
        assert!(matches!(app.view_origin(), Screen::Inbox), "Esc goes back to where the palette was opened");
    }

    #[tokio::test]
    async fn a_help_key_runs_only_on_the_screen_its_section_describes() {
        let deps = test_deps();
        let mut app = App::new("slate");
        // Global `r` refreshes a list; in a PR view `r` replies. The Refresh row must not reply.
        app.screen = open_pr_screen(PullRequestStatus::Open);
        palette_run(&mut app, "?cycle theme", &deps).await;
        assert!(app.overlay.is_none(), "Refresh from a PR view must not open the reply input");
        assert!(app.toast.as_deref().is_some_and(|t| t.starts_with("Press r")));

        // The palette's own row runs its first key, reopening the palette.
        app.screen = Screen::List;
        palette_run(&mut app, "?command palette", &deps).await;
        assert!(matches!(app.overlay, Some(Overlay::Palette { .. })));
    }

    #[tokio::test]
    async fn merge_command_preselects_the_strategy() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.screen = open_pr_screen(PullRequestStatus::Open);
        palette_run(&mut app, ":merge rebase", &deps).await;
        match &app.overlay {
            Some(Overlay::Picker { kind: PickerKind::PrMergeStrategy, selected, .. }) => assert_eq!(*selected, 2),
            _ => panic!("expected the merge picker"),
        }
    }

    #[tokio::test]
    async fn go_to_help_opens_the_help_panel() {
        let deps = test_deps();
        let mut app = App::new("slate");
        palette_run(&mut app, "help", &deps).await;
        assert!(matches!(app.overlay, Some(Overlay::Help { .. })));
    }

    #[test]
    fn help_lists_the_command_palette_on_ctrl_k() {
        let sections = crate::ui::help_sections();
        let global = &sections.iter().find(|(name, _)| *name == "Global").expect("global section").1;
        let (keys, desc) = global.iter().find(|(k, _)| k.contains("Ctrl-K")).expect("Ctrl-K listed");
        assert!(keys.contains("Ctrl-P"), "Ctrl-P stays listed as the alias");
        assert!(desc.starts_with("Command palette"));
    }

    fn wi_view_of(w: WorkItem) -> Screen {
        Screen::WiView(Box::new(WiView { connection_id: "c".into(), wi: w, threads: vec![], timeline: vec![], scroll: 0 }))
    }

    fn person(id: &str, name: &str, handle: &str) -> User {
        User { id: id.into(), display_name: name.into(), handle: Some(handle.into()), avatar_url: None }
    }

    #[test]
    fn the_assignee_picker_leads_with_unassigned_and_preselects_the_current_assignee() {
        let users = vec![person("u1", "Priya Nair", "priya"), person("me", "Sam Rivera", "sam")];
        // The item's assignee carries a different id than the picker's row (GitHub: numeric id
        // on the item, login in assignable_users) — it is still found, by handle.
        let current = User { id: "4242".into(), ..person("x", "sam", "sam") };
        let Overlay::Search { items, selected, kind: SearchKind::Assignee { me }, .. } =
            assignee_picker("Assign".into(), &users, Some(&current), Some("SAM"))
        else {
            panic!("expected the search picker")
        };
        assert_eq!(items[0].label, "Unassigned");
        assert_eq!(items[0].id, None, "the first row clears the assignee");
        assert_eq!(items[selected].label, "Sam Rivera", "the current assignee is highlighted");
        assert_eq!(me, Some(2), "the signed-in user is found by handle, case-insensitively");
        let Overlay::Search { selected, kind: SearchKind::Assignee { me }, .. } = assignee_picker("Assign".into(), &users, None, None)
        else {
            panic!("expected the search picker")
        };
        assert_eq!(selected, 0, "an unassigned item starts on Unassigned");
        assert_eq!(me, None, "no identity, no @ shortcut");
    }

    #[test]
    fn an_accepted_edit_lands_on_the_open_view_and_the_row_behind_it() {
        let mut app = App::new("slate");
        let item = wi(None);
        app.wis = vec![wi_row(wi(None)), wi_row(WorkItem { id: "other".into(), ..wi(None) })];
        app.screen = wi_view_of(item.clone());
        let priya = person("u1", "Priya Nair", "priya");
        app.patch_wi("c", &item.item_ref(), |w| {
            w.assignee = Some(priya.clone());
            w.title = "Renamed".into();
        });
        let Screen::WiView(v) = &app.screen else { panic!("expected WiView") };
        assert_eq!(v.wi.assignee.as_ref().map(|a| a.display_name.as_str()), Some("Priya Nair"));
        assert_eq!(v.wi.title, "Renamed");
        assert_eq!(app.wis[0].wi.title, "Renamed", "the list row moves with the view");
        assert_eq!(app.wis[1].wi.title, "t", "another item is untouched");
    }

    #[tokio::test]
    async fn e_edits_the_title_in_a_prefilled_input_and_the_description_in_the_editor() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.screen = wi_view_of(WorkItem { title: "Old title".into(), description: Some("Body".into()), ..wi(None) });

        app.on_key(Key::Char('e'), &deps).await;
        assert!(matches!(app.overlay, Some(Overlay::Picker { kind: PickerKind::WorkItemEdit, .. })), "e asks which field");
        app.on_key(Key::Enter, &deps).await; // Title is first
        match &app.overlay {
            Some(Overlay::Input { buffer, kind: InputKind::WorkItemTitle, .. }) => assert_eq!(buffer, "Old title"),
            _ => panic!("the title opens prefilled"),
        }

        app.overlay = None;
        app.on_key(Key::Char('e'), &deps).await;
        app.on_key(Key::Down, &deps).await;
        app.on_key(Key::Enter, &deps).await;
        assert!(app.overlay.is_none(), "the description goes to $EDITOR, not an overlay");
        let request = app.editor_request.take().expect("an editor round trip is requested");
        assert_eq!(request.initial, "Body");
        assert_eq!(request.field, WiField::Description);

        // Saving it unchanged (an editor's trailing newline aside) sends nothing.
        app.finish_editor(request, Ok("Body\n".into()), &deps).await;
        assert_eq!(app.toast.as_deref(), Some("Description unchanged — nothing sent"));
    }

    #[tokio::test]
    async fn x_cancels_only_a_run_that_is_still_going_and_asks_first() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let open = |status| {
            Screen::Pipeline(Box::new(PipelineView::new(
                "CI #1".into(),
                pipeline_run("r1", status, vec![]),
                "c".into(),
                ProviderType::GitHub,
                "ci".into(),
                None,
            )))
        };
        app.screen = open(PipelineRunStatus::Succeeded);
        app.on_key(Key::Char('X'), &deps).await;
        assert!(app.overlay.is_none(), "a finished run has nothing to cancel");
        assert_eq!(app.toast.as_deref(), Some("Only a queued or running run can be cancelled"));

        app.screen = open(PipelineRunStatus::Running);
        app.on_key(Key::Char('X'), &deps).await;
        match &app.overlay {
            Some(Overlay::Confirm { message, action: Action::PipelineCancel { connection_id, run, .. }, .. }) => {
                assert_eq!(connection_id, "c");
                assert_eq!(run.id, "r1");
                assert_eq!(message, "Cancel CI #1? Its running jobs stop.");
            }
            _ => panic!("a running run asks before cancelling"),
        }
    }

    #[test]
    fn a_work_item_detail_fetch_carries_the_timeline_and_old_cache_entries_still_decode() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let mut app = App::new("slate");
        let w = wi(None);
        let key = wi_detail_cache_key("c", &w.item_ref());
        app.screen = wi_view_of(w);
        let ev = TimelineEvent { actor: None, kind: TimelineEventKind::StateChanged, summary: "changed status to Done".into(), at: None };
        app.on_event(
            AppEvent::WiDetailLoaded {
                key: key.clone(),
                detail: Box::new(WiDetailFetch { threads: None, timeline: Some(vec![ev]) }),
                fetched_at: Utc::now(),
            },
            &deps,
        );
        let Screen::WiView(v) = &app.screen else { panic!("expected WiView") };
        assert_eq!(v.timeline.len(), 1, "a timeline alone is worth repainting for");

        // An entry written before the timeline existed has no `timeline` key at all.
        let old: WiDetail = serde_json::from_str(r#"{"threads":[]}"#).expect("an old entry still decodes");
        assert!(old.timeline.is_empty());
        let old: PrDetail = serde_json::from_str(r#"{"threads":[],"files":[],"checks":[],"commits":[]}"#).expect("an old PR entry decodes");
        assert!(old.timeline.is_empty());
    }

    // ---- mouse ----

    /// Draws one frame into a test terminal, filling `app.hits`, and returns what it drew.
    fn draw(app: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        t.draw(|f| crate::ui::render(f, app)).unwrap();
        t.backend().buffer().clone()
    }

    /// Where `target` was drawn in the last frame (its top-left cell).
    fn spot(app: &App, target: Hit) -> (u16, u16) {
        let r = app.hits.iter().find(|(_, h)| *h == target).unwrap_or_else(|| panic!("{target:?} not on screen")).0;
        (r.x, r.y)
    }

    fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    async fn click(app: &mut App, deps: &AppDeps, target: Hit) {
        let (x, y) = spot(app, target);
        app.on_key(Key::Click(x, y), deps).await;
    }

    /// A list without the preview, so it has the full width and the keys.
    fn mouse_list(ids: &[&str]) -> App {
        let mut app = preview_app(ids);
        app.preview_hidden[0] = true;
        app
    }

    #[tokio::test]
    async fn clicking_a_row_selects_it_and_clicking_it_again_opens_it() {
        let deps = test_deps();
        let mut app = mouse_list(&["1", "2", "3"]);
        let buf = draw(&mut app, 150, 30);
        assert!(row_text(&buf, spot(&app, Hit::ListRow(1)).1).contains("PR 2"), "the row's hit sits on its own text");

        click(&mut app, &deps, Hit::ListRow(1)).await;
        assert_eq!(app.selected(), Some(1));
        assert!(matches!(app.screen, Screen::List), "the first click only selects");

        draw(&mut app, 150, 30);
        click(&mut app, &deps, Hit::ListRow(1)).await;
        assert!(matches!(&app.screen, Screen::PrView(v) if v.pr.id == "2"), "a second click opens it, like Enter");
    }

    #[tokio::test]
    async fn clicking_a_tab_switches_to_it() {
        let deps = test_deps();
        let mut app = mouse_list(&["1"]);
        let buf = draw(&mut app, 150, 30);
        let (x, y) = spot(&app, Hit::Tab(0));
        assert!(row_text(&buf, y).chars().skip(x as usize).collect::<String>().starts_with(" Command Center"), "tab 0's hit is on its title");
        let (x, y) = spot(&app, Hit::Tab(2));
        assert!(row_text(&buf, y).chars().skip(x as usize).collect::<String>().starts_with(" Work Items"), "tab 2 is Work Items");

        click(&mut app, &deps, Hit::Tab(2)).await;
        assert!(matches!(app.screen, Screen::List) && app.active == 1);
        draw(&mut app, 150, 30);
        click(&mut app, &deps, Hit::Tab(0)).await;
        assert!(matches!(app.screen, Screen::Launchpad));
    }

    #[tokio::test]
    async fn the_wheel_moves_a_list_one_row_and_stops_at_the_ends() {
        let deps = test_deps();
        let mut app = mouse_list(&["1", "2"]);
        draw(&mut app, 150, 30);
        let (x, y) = spot(&app, Hit::ListRow(0));
        app.on_key(Key::ScrollUp(x, y), &deps).await;
        assert_eq!(app.selected(), Some(0), "no wrap to the bottom");
        app.on_key(Key::ScrollDown(x, y), &deps).await;
        app.on_key(Key::ScrollDown(x, y), &deps).await;
        assert_eq!(app.selected(), Some(1), "no wrap to the top");
    }

    #[tokio::test]
    async fn clicks_are_ignored_under_an_overlay() {
        let deps = test_deps();
        let mut app = mouse_list(&["1", "2"]);
        draw(&mut app, 150, 30);
        app.overlay = Some(Overlay::Help { scroll: 0 });
        click(&mut app, &deps, Hit::ListRow(1)).await;
        assert_eq!(app.selected(), Some(0));
        assert!(app.overlay.is_some());
    }

    #[tokio::test]
    async fn clicking_a_diff_line_puts_the_cursor_there_and_a_second_click_comments() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let detail = PrDetail {
            timeline: Vec::new(),
            threads: vec![],
            files: vec![changed("a.rs", Some("@@ -1,2 +1,3 @@\n ctx\n+added line\n-removed"))],
            checks: vec![],
            commits: vec![],
        };
        app.screen = pr_view_showing(pr(None), &detail);
        draw(&mut app, 150, 30);
        click(&mut app, &deps, Hit::PrTab(3)).await;
        assert!(matches!(&app.screen, Screen::PrView(v) if v.tab == 3), "the Diff sub-tab is clickable");

        let buf = draw(&mut app, 150, 30);
        assert!(row_text(&buf, spot(&app, Hit::DiffLine(2)).1).contains("+added line"));
        click(&mut app, &deps, Hit::DiffLine(2)).await;
        let Screen::PrView(v) = &app.screen else { panic!() };
        assert_eq!((v.diff.focus, v.diff.cursor), (DiffFocus::Patch, 2));

        draw(&mut app, 150, 30);
        click(&mut app, &deps, Hit::DiffLine(2)).await;
        assert!(matches!(&app.overlay, Some(Overlay::Input { kind: InputKind::PrLineComment, .. })), "second click opens the line comment");
    }

    #[test]
    fn an_unfocused_preview_takes_no_clicks() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.settle_preview(&deps);
        draw(&mut app, 150, 30);
        assert!(app.preview.is_some(), "the split is showing");
        assert!(app.hits.iter().any(|(_, h)| matches!(h, Hit::ListRow(_))));
        assert!(!app.hits.iter().any(|(_, h)| matches!(h, Hit::PrTab(_))), "the preview's sub-tabs are only a picture");
    }
}
