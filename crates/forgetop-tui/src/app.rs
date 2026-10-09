//! Application state and the (async) update logic driven by the event loop.

use std::cell::Cell;
use std::cmp::{Ordering, Reverse};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Local, Utc};
use forgetop_core::cache::{CachePut, CacheStore};
use forgetop_core::config::{NotificationPrefs, SavedView, SortPref};
use forgetop_core::domain::*;
use forgetop_core::filter::{pull_request_matches, run_triggered_by};
use forgetop_core::provider::*;
use forgetop_core::runlog::{self, FailureSummary, LogSection};
use forgetop_core::service::{ConfigService, ConnectionHealth, ConnectionHealthService, SectionService};
use ratatui::widgets::TableState;
use tokio::sync::mpsc;

use crate::launchpad;
use crate::overlay::{Action, InputKind, Outcome, Overlay, PickerKind, SearchItem, SearchKind, ToggleItem, ToggleKind, WiField};
use crate::palette::{self, CommandContext, GoTo, PaletteItem, PaletteKind, PaletteTarget};
use crate::theme::Theme;
use crate::wizard::{provider_sections, section_label, Wizard, WizardOutcome};

/// Reading a job log for errors lives in core, so the web dashboard reads it the same way.
pub use forgetop_core::runlog::{first_error_line, is_error_line};

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
const DIAG_WI_DETAIL: &str = "tui.work_item.detail";
const DIAG_WI_THREADS: &str = "tui.work_item.threads";
const DIAG_WI_STATES: &str = "tui.work_item.states";
const DIAG_WI_TIMELINE: &str = "tui.work_item.timeline";
const DIAG_WI_ASSIGNABLE: &str = "tui.work_item.assignable_users";
const DIAG_PIPELINE_FEEDS: &str = "tui.pipeline.feeds";
const DIAG_PIPELINE_DISCOVERY: &str = "tui.pipeline.discovery";
const DIAG_PIPELINE_RUN: &str = "tui.pipeline.run";
const DIAG_PIPELINE_APPROVALS: &str = "tui.pipeline.approvals";
const DIAG_PIPELINE_CURRENT_USER: &str = "tui.pipeline.current_user";
const DIAG_PIPELINE_LOGS: &str = "tui.pipeline.logs";
const DIAG_PIPELINE_ANNOTATIONS: &str = "tui.pipeline.annotations";
const DIAG_PIPELINE_ARTIFACTS: &str = "tui.pipeline.artifacts";
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
/// The unfiltered PR pool, with each connection's signed-in user. Every PR view derives from it,
/// so a view that was never on screen when the cache was written still has rows at launch —
/// the per-view `prs_cache_key` entries only ever held the view that happened to be showing.
const CACHE_KEY_PR_POOL: &str = "list.prs.pool";

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

/// Identity of a notification for dismissal. It carries `updated_at` so a thread with new
/// activity since it was dismissed is a different key — and shows up again.
fn inbox_dismiss_key(row: &NotifRow) -> String {
    let at = row.notification.updated_at.map(|t| t.timestamp()).unwrap_or_default();
    format!("{}:{}:{at}", row.connection_id, row.notification.id)
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
    cache.rewrite::<PrPool>(CACHE_KEY_PR_POOL, |mut pool| {
        retain_pool_rows(&mut pool, |r| r.connection_id != conn_id);
        pool.me.remove(conn_id);
        pool.needs_decoration.remove(conn_id);
        pool.review_clears_request.remove(conn_id);
        pool
    });
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

/// Puts text on the system clipboard. A function pointer, like [`FeedbackOpener`], so tests can
/// exercise the copy keys without writing escape codes to the terminal.
type ClipboardWriter = fn(&str) -> std::result::Result<(), String>;

/// Copies through the terminal with an OSC 52 escape — no clipboard crate, and it works over
/// SSH and inside tmux (with `set-clipboard on`). Terminals that don't support it ignore it.
fn osc52_clipboard(text: &str) -> std::result::Result<(), String> {
    use std::io::Write;
    let seq = format!("\x1b]52;c;{}\x07", base64_encode(text.as_bytes()));
    let mut out = std::io::stdout();
    out.write_all(seq.as_bytes()).and_then(|()| out.flush()).map_err(|e| e.to_string())
}

/// Standard base64 with padding — just enough for OSC 52.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

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
    /// A refresh's PR pool, handed over as soon as it is fetched and before the rest of that
    /// refresh (work items, pipelines, notifications, health, discovery) has answered. `ok` is
    /// the pool's own "every feed answered" flag. The full [`AppEvent::Reloaded`] that follows
    /// carries the same pool again.
    PrPoolLoaded { pool: Box<PrPool>, ok: bool },
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
    /// A run's artifact list finished fetching, for the drill-in's artifacts overlay. Applied only
    /// if that overlay is still open on that run.
    PipelineArtifactsLoaded { conn_id: String, run_id: String, result: std::result::Result<Vec<PipelineArtifact>, String> },
    /// The provider answered a rerun or cancel sent by [`App::execute_pipeline_run_action`]:
    /// the new run's id when it started one.
    PipelineRunActionDone { token: u64, result: std::result::Result<Option<String>, String> },
    /// The provider answered a PR write sent by [`App::execute_pr_action`] or
    /// [`App::submit_review`], with the PR re-read after an accepted one.
    PrActionDone { token: u64, result: std::result::Result<(), String>, fresh: Option<Box<PullRequest>> },
    /// The provider answered a work-item write sent by [`App::execute_wi_action`], with the item
    /// re-read after an accepted one.
    WiActionDone { token: u64, result: std::result::Result<(), String>, fresh: Option<Box<WorkItem>> },
    /// A finished run's problems, fetched apart from its detail. `None` means the call failed.
    PipelineAnnotationsLoaded { key: String, annotations: Option<Vec<PipelineAnnotation>> },
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
    /// Where the fetch hands its PR pool over early ([`AppEvent::PrPoolLoaded`]). `None` when
    /// the app has no job channel, as in tests that drive `fetch_all` directly.
    pr_pool_tx: Option<mpsc::UnboundedSender<AppEvent>>,
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
    /// Each connection's discovered pipeline definitions, from the discoveries that answered.
    /// Merged into [`App::pipe_catalog`]; a connection missing here keeps its last catalog.
    pipe_catalog: HashMap<String, Vec<PipelineDefinition>>,
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
    /// True when the authenticated user started this run — the Command Center lists only these.
    /// Defaults to `false` for rows cached before it existed; the next reload sets it.
    #[serde(default)]
    pub triggered_by_me: bool,
    /// The pending gates that hold this run, by name, whoever may answer them. `awaiting_approval`
    /// is the narrower "this user can answer one". An optional gate the run carries on past (a
    /// GitLab manual job allowed to fail) isn't listed: it doesn't park the run as Waiting.
    #[serde(default)]
    pub gates: Vec<String>,
}

impl PipeRow {
    /// The status to show: an in-flight run held on any gate reads as Waiting. Derived at display
    /// time so the stored run stays exactly what the provider said (refresh and cache compare it).
    pub fn shown_status(&self) -> PipelineRunStatus {
        self.run.shown_status(!self.gates.is_empty())
    }
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
/// Providers used to build their list URL from `include_completed` and `limit` alone — the filter
/// was applied in memory afterwards — so a `list` per filter re-fetched identical rows. This
/// meant four calls per connection per reload (the list section, the two Launchpad buckets, and
/// the notification scan), and a fifth, blocking, every time `[`/`]` moved between views. A
/// provider that targets its filters instead (all four repository forges now do; the demo does
/// not) contributes the rows its filtered `list` finds beyond the page, merged in by
/// [`fetch_pr_pool`].
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
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
    /// connection_id → whether giving your review takes a pull request out of its
    /// review-requested listing ([`PullRequestSource::review_clears_request`]), so a vote can
    /// take the row off the list before the refetch that would.
    #[serde(default)]
    review_clears_request: HashMap<String, bool>,
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
    /// Problems the run reported. Defaulted so entries cached before these fields existed
    /// still read back.
    #[serde(default)]
    pub annotations: Vec<PipelineAnnotation>,
    #[serde(default)]
    pub supports_rerun: bool,
    #[serde(default)]
    pub supports_rerun_failed: bool,
    #[serde(default)]
    pub supports_artifacts: bool,
    #[serde(default)]
    pub rerun_new_run: bool,
    #[serde(default)]
    pub rerun_failed_new_run: bool,
}

/// `None` means that call failed. See [`PrDetailFetch`] for why the distinction from an
/// authoritatively empty answer matters.
#[derive(Default)]
pub struct PipelineDetailFetch {
    pub run: Option<PipelineRun>,
    pub approvals: Option<Vec<PipelineApproval>>,
    pub supports_approvals: Option<bool>,
    pub can_respond_approvals: Option<bool>,
    pub annotations: Option<Vec<PipelineAnnotation>>,
    pub supports_rerun: Option<bool>,
    pub supports_rerun_failed: Option<bool>,
    pub supports_artifacts: Option<bool>,
    pub rerun_new_run: Option<bool>,
    pub rerun_failed_new_run: Option<bool>,
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

/// How many of a section's discovered pipelines are fetched, across its connections.
#[derive(Debug, Clone, PartialEq)]
pub struct PipeScope {
    /// The pipeline connections, in subscription order — the picker asks which when several.
    pub connections: Vec<String>,
    pub selected: usize,
    pub available: usize,
}

impl PipeScope {
    /// "Pipelines · 6 of 38" for the section header.
    pub fn label(&self) -> String {
        format!("Pipelines · {} of {}", self.selected, self.available)
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
    /// Each pipeline connection's discovered definitions, keyed by connection id — the
    /// denominator in "Pipelines · 6 of 38" and the candidates of the add/remove picker.
    pub pipe_catalog: HashMap<String, Vec<PipelineDefinition>>,
    /// The Pipelines header's "which pipelines are fetched" summary. On that section it stands
    /// in for the repository summary: what is fetched there is chosen pipeline by pipeline.
    pub pipe_scope: Option<PipeScope>,
    pub status: String,
    pub loading: bool,
    /// Whether the refresh in flight has already handed over its PR pool. The PR list stops
    /// saying "Loading…" once it has: an empty view then means no pull requests, not not yet.
    pub prs_landed: bool,
    /// Scroll offset for the current list; body height captured during render.
    pub list_scroll: u16,
    pub content_h: u16,
    /// Max scroll offset for the open PR/WI view, captured during render for clamping.
    pub detail_scroll_max: u16,
    pub pr_filter: PullRequestFilter,
    /// Every pull request the last reload fetched, unfiltered. `prs`, `lp_prs_mine` and
    /// `lp_prs_review` are all derived from it, so moving between views costs no network.
    pub pr_pool: PrPool,
    /// Whether `pr_pool` holds rows worth deriving from — a fetch has populated it, or launch
    /// seeded it from the cached pool. An empty pool that was never filled is not the same as one
    /// that genuinely came back empty, and only the latter may clear a list.
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
    /// List-table columns switched off per section, by header name (`c` picks). Persisted.
    pub hidden_cols: [Vec<String>; 3],
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
    /// Notifications dismissed from the inbox (`d` / `D`), keyed by [`inbox_dismiss_key`].
    /// Persisted to config so they stay hidden across restarts.
    inbox_dismissed: HashSet<String>,
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
    /// Per section, whether the list's automatic preview is off. `P` switches it; until the user
    /// has, [`DEFAULT_PREVIEW_HIDDEN`] decides.
    pub preview_hidden: [bool; 3],
    /// Files marked "viewed" (`v`), per PR detail cache key, each pinned to the fingerprint of
    /// its diff when marked — so closing a PR and coming back finds the marks where they were,
    /// and a file whose diff has changed since has to be looked at again. Kept for the session.
    pub pr_viewed: HashMap<String, HashMap<String, u64>>,
    /// True while the preview is focused: its view is `screen`, drawn beside the list.
    pub preview_focus: bool,
    /// Width of the content area in the last frame, which decides whether the split fits.
    pub content_w: u16,
    /// The log fetch in flight — `(conn, run, job)` key and anim ticks waited — so at most
    /// one goes out at a time and a poll never piles on top of a slow one.
    log_inflight: Option<(String, u16)>,
    /// Run details fetched this session, keyed by [`pipeline_detail_cache_key`]: what per-job
    /// duration estimates read, from recent succeeded runs of the same pipeline. Session-only,
    /// so estimates work with the cache disabled (`--demo`).
    run_details: HashMap<String, PipelineRun>,
    /// Estimate fetches already sent this session, so each run's detail is asked for once.
    estimate_requested: HashSet<String>,
    /// Where the copy keys (`c`, `y`) write. OSC 52 in production.
    clipboard: ClipboardWriter,
    /// Reruns and cancels waiting on the provider, by token.
    run_actions: HashMap<u64, PendingRunAction>,
    next_run_action: u64,
    /// Optimistic run changes held against stale refreshes, by `(connection, run id)`.
    held_runs: HashMap<(String, String), RunHold>,
    /// PR and work-item writes waiting on the provider, by token.
    item_actions: HashMap<u64, PendingItemAction>,
    next_item_action: u64,
    /// Optimistic PR and work-item changes held against stale refreshes, by the item's detail
    /// cache key.
    held_items: HashMap<String, ItemHold>,
    /// A write landed while a refresh was already out: that refresh was asked for before the
    /// write, so another one follows it.
    reload_again: bool,
    /// A run a rerun started, to open once it shows up in the list: `(connection, run id)`.
    follow_new_run: Option<(String, String)>,
    /// Detail keys whose annotations were asked for, and the run status they were asked at —
    /// a finished run's problems don't change until it is rerun.
    annotations_asked: std::cell::RefCell<HashMap<String, PipelineRunStatus>>,
    /// Problems fetched this session, by detail key — what a detail landing after them carries,
    /// whichever of the two events arrives first.
    annotations_by_key: HashMap<String, Vec<PipelineAnnotation>>,
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

/// Which sections' lists preview the selected row without being asked, until the user switches
/// it with `P`. Pull requests and work items are opened to be read, so their lists keep the full
/// width and Enter opens the pane; a pipeline run is glanced at, so its list previews as you move.
pub const DEFAULT_PREVIEW_HIDDEN: [bool; 3] = [true, true, false];

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
    /// Moves `delta` tabs along, wrapping. Changing tab resets the scroll (each tab starts at
    /// the top), drops any patch line cursor, and restores the whole-PR diff on the Diff tab.
    pub fn step_tab(&mut self, delta: isize) {
        let n = PR_TABS.len() as isize;
        self.tab = (self.tab as isize + delta).rem_euclid(n) as usize;
        self.scroll = 0;
        self.reset_diff_scope();
    }

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
    /// The node's span — a step's, job's, stage's or folded group's. Drives the timeline bars.
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Where the node sits in the run: its stage, its job (not for a stage row), and its step
    /// (steps only).
    pub stage: usize,
    pub job: Option<usize>,
    pub step: Option<usize>,
    /// A folded run of steps ("8 steps passed", "4 post & cleanup steps") or of skipped stages
    /// ("4 stages skipped"), not a single node.
    pub group: bool,
    /// A short roll-up drawn after the label when there's room — a folded stage's `3 jobs`, or
    /// the problem its jobs report — and how to colour it.
    pub summary: Option<(String, palette::Tone)>,
    /// Text drawn where the bar would be, for a row with no span of its own: what a queued stage
    /// waits on (`after Deploy Prod`), what made a run of stages skip (`Build failed`).
    pub note: Option<String>,
    /// An explanatory line, not a node of the run: the row under a gated stage that says why
    /// it has no jobs. The label is the whole text.
    pub info: bool,
}

impl FlatNode {
    /// A row with nothing but its place, name and status; the rest is filled in by the caller.
    fn bare(depth: usize, label: String, status: PipelineRunStatus, stage: usize) -> Self {
        FlatNode {
            depth,
            label,
            status,
            key: None,
            expanded: false,
            duration: None,
            problem: None,
            url: None,
            job_id: None,
            started_at: None,
            finished_at: None,
            stage,
            job: None,
            step: None,
            group: false,
            summary: None,
            note: None,
            info: false,
        }
    }
}

/// When a run last did anything: the latest finish among its jobs and steps. For a run parked on
/// a gate, this is where the work stopped and the wait began.
pub fn last_activity(run: &PipelineRun) -> Option<DateTime<Utc>> {
    run.stages
        .iter()
        .flat_map(|s| &s.jobs)
        .flat_map(|j| j.finished_at.into_iter().chain(j.steps.iter().filter_map(|s| s.finished_at)))
        .max()
}

/// How long a parked run has waited: `now` less its last activity. `None` when nothing in it has
/// finished, so there's no telling when the wait began.
pub fn waiting_secs(run: &PipelineRun, now: DateTime<Utc>) -> Option<i64> {
    last_activity(run).map(|last| (now - last).num_seconds().max(0))
}

/// The stage a parked run is held at: one the provider already reports as Waiting, else — when
/// the run reads as Waiting only through its pending gates (`shown`) — the queued stage a gate is
/// named after, or the first queued one. `None` when the run isn't waiting — finished, cancelled,
/// or still running — whatever its stages last said.
pub fn gate_stage(run: &PipelineRun, shown: PipelineRunStatus, gates: &[String]) -> Option<usize> {
    if shown != PipelineRunStatus::Waiting {
        return None;
    }
    if let Some(i) = run.stages.iter().position(|s| s.status == PipelineRunStatus::Waiting) {
        return Some(i);
    }
    let queued = |s: &PipelineStage| s.status == PipelineRunStatus::Queued;
    run.stages
        .iter()
        .position(|s| queued(s) && gates.iter().any(|g| g.eq_ignore_ascii_case(&s.name)))
        .or_else(|| run.stages.iter().position(queued))
}

/// A provider's name as a person writes it — `as_str` is the serialised form (`AzureDevOps`).
pub fn provider_label(p: ProviderType) -> &'static str {
    match p {
        ProviderType::AzureDevOps => "Azure DevOps",
        other => other.as_str(),
    }
}

/// What a folded stage says after its label: the problem its jobs report when it didn't pass
/// cleanly, else how many jobs it ran.
fn stage_summary(stage: &PipelineStage, status: PipelineRunStatus) -> Option<(String, palette::Tone)> {
    use palette::Tone;
    let problem = |want: PipelineRunStatus| stage.jobs.iter().filter(|j| j.status == want).find_map(|j| j.problem.clone());
    match status {
        PipelineRunStatus::Waiting => return Some(("needs approval".into(), Tone::Warn)),
        PipelineRunStatus::PartiallySucceeded => {
            if let Some(p) = problem(PipelineRunStatus::PartiallySucceeded) {
                return Some((p, Tone::Warn));
            }
        }
        PipelineRunStatus::Failed => {
            if let Some(p) = problem(PipelineRunStatus::Failed) {
                return Some((p, Tone::Bad));
            }
        }
        _ => {}
    }
    let n = stage.jobs.len();
    (n > 0).then(|| (format!("{n} {}", if n == 1 { "job" } else { "jobs" }), Tone::Neutral))
}

/// Trailing steps that only tidy up after a job: `Post …` hooks and `Complete job`.
fn is_cleanup_step(name: &str) -> bool {
    let n = name.trim();
    n.starts_with("Post ") || n.eq_ignore_ascii_case("Complete job")
}

/// How a job's steps fold in the tree, as `(passed, cleanup)`: steps `..passed` fold into one
/// "N steps passed" node (on a failed run, the leading passed steps — keeping the one just before
/// the failure in view), and steps `cleanup..` into one "N post & cleanup steps" node. A fold
/// needs at least two steps; `passed == 0` and `cleanup == steps.len()` mean no fold.
pub fn step_folds(job: &PipelineJob, run_failed: bool) -> (usize, usize) {
    let n = job.steps.len();
    let mut cleanup = n;
    while cleanup > 0 && is_cleanup_step(&job.steps[cleanup - 1].name) {
        cleanup -= 1;
    }
    if n - cleanup < 2 {
        cleanup = n;
    }
    let mut passed = 0;
    if run_failed {
        if let Some(f) = job.steps.iter().position(|s| s.status == PipelineRunStatus::Failed) {
            let lead = job.steps.iter().take_while(|s| s.status == PipelineRunStatus::Succeeded).count();
            let k = lead.min(f.saturating_sub(1)).min(cleanup);
            if k >= 2 {
                passed = k;
            }
        }
    }
    (passed, cleanup)
}

/// The span of some steps: earliest start to latest finish, the finish only once all are done.
fn steps_span(steps: &[PipelineStep]) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let start = steps.iter().filter_map(|s| s.started_at).min();
    let finish =
        if steps.iter().all(|s| s.finished_at.is_some()) { steps.iter().filter_map(|s| s.finished_at).max() } else { None };
    (start, finish)
}

/// A folded group's status: the first step that didn't pass, else passed.
fn steps_status(steps: &[PipelineStep]) -> PipelineRunStatus {
    steps.iter().map(|s| s.status).find(|s| *s != PipelineRunStatus::Succeeded).unwrap_or(PipelineRunStatus::Succeeded)
}

/// A run's elapsed seconds: start to finish, or to `now` while it is still going.
pub fn run_secs(run: &PipelineRun, now: DateTime<Utc>) -> Option<i64> {
    // A queued run's start is when it was asked for, not when it began: no time used yet.
    if run.status == PipelineRunStatus::Queued {
        return None;
    }
    let start = run.started_at?;
    let end = match run.finished_at {
        Some(f) => f,
        None if run.status.is_active() => now,
        None => return None,
    };
    Some((end - start).num_seconds().max(0))
}

/// One run in a pipeline's history strip.
#[derive(Debug, Clone)]
pub struct HistEntry {
    pub run_id: String,
    pub number: Option<i64>,
    pub status: PipelineRunStatus,
    /// Elapsed seconds, when the run has started (running runs count to now).
    pub secs: Option<i64>,
    pub started_at: Option<DateTime<Utc>>,
    /// Index into `App::pipes`, to open the run and move the list selection to it.
    pub row: usize,
}

/// The most recent failure among a run's history, for the strip's "last failure" note.
#[derive(Debug, Clone)]
pub struct LastFailure {
    pub number: Option<i64>,
    pub at: Option<DateTime<Utc>>,
    /// The failed step (or job) when that run's stages are known.
    pub step: Option<String>,
}

/// Recent runs of the same pipeline on the same branch, oldest first, with the open run marked.
/// Built from the rows the list already holds — no fetch — and rebuilt whenever they change.
#[derive(Debug, Clone, Default)]
pub struct RunHistory {
    /// The pipeline's name, for the panel title.
    pub name: String,
    pub entries: Vec<HistEntry>,
    /// The open run's position in `entries`.
    pub current: Option<usize>,
    /// Median elapsed time of the succeeded runs among all of this branch's rows, and how many.
    pub median_secs: Option<i64>,
    pub median_n: usize,
    /// How many runs of this pipeline on this branch the list holds (the strip shows ≤ 10).
    pub total: usize,
    /// The branch had fewer than two runs (a tag, say), so this is the pipeline's history on
    /// every branch.
    pub all_branches: bool,
    pub last_failure: Option<LastFailure>,
}

/// Most runs the history strip shows.
pub const HISTORY_LEN: usize = 10;
/// Recent succeeded runs whose job times feed the per-job estimates.
pub const ESTIMATE_RUNS: usize = 3;

/// How long an in-flight run and its jobs should take, from medians of recent runs.
#[derive(Debug, Clone, Default)]
pub struct Estimates {
    /// Median elapsed time of this pipeline's succeeded runs, and how many it is taken over.
    pub run_median: Option<i64>,
    pub run_n: usize,
    /// Median duration per job name, over up to [`ESTIMATE_RUNS`] recent succeeded runs.
    pub jobs: HashMap<String, i64>,
    pub job_runs: usize,
}

/// The run's artifact list, shown over the pane (`a`).
#[derive(Debug, Clone, Default)]
pub struct ArtifactsPanel {
    /// `None` while the fetch is out.
    pub items: Option<Vec<PipelineArtifact>>,
    pub error: Option<String>,
    pub selected: usize,
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
pub const LOG_TREE_WIDTH: u16 = 26;

/// One row of a log pane: a log line, or a step section's fold header — which stands in for the
/// section's own marker line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogRow {
    Line(usize),
    Header(usize),
    /// Consecutive folded sections of passed steps, `first..=last`, drawn as one header.
    Group(usize, usize),
}

/// A jump a log pane owes once its lines arrive, asked for before they had (`E` from the tree,
/// ↵ on a problem).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogJump {
    FirstError,
    /// The first line containing any of these, most specific first.
    Find(Vec<String>),
}

/// A scrollable log view over one job, shown beside the pipeline drill-in's tree.
///
/// The provider hands back the whole log on every call, so a poll simply replaces `lines`;
/// what the user is doing (scroll, follow, search, folds) is carried across the replacement.
///
/// When the log marks its steps (see [`runlog::parse_sections`]) each step becomes a fold
/// header, and a cursor row appears for `z` to act on. A log without markers keeps the plain
/// scrolling behaviour: one row per line, no cursor.
pub struct LogView {
    /// Pane title, e.g. `Logs · dotnet test` (live state is appended by the renderer).
    pub title: String,
    /// The job these lines belong to — answers for any other job are dropped.
    pub job_id: String,
    pub lines: Vec<String>,
    /// Top visible row while not following.
    pub scroll: u16,
    /// Pinned to the bottom as lines arrive. On by default for a live job.
    pub follow: bool,
    /// The job was still running at the last check, so the pane is polling.
    pub live: bool,
    /// Visible row count of the last frame, written by the renderer so scrolling can clamp.
    pub viewport: Cell<u16>,
    /// Columns the text is panned right by (`←`/`→`). A line's time gutter stays put.
    pub hscroll: usize,
    /// Body width of the last frame, written by the renderer so panning can clamp.
    pub viewport_w: Cell<u16>,
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
    /// Per-step sections parsed from `lines`; empty when the log marks none.
    pub sections: Vec<LogSection>,
    /// Sections shown unfolded, by index into `sections`.
    pub open: HashSet<usize>,
    /// What the pane draws, top to bottom. Scrolling counts rows, not lines.
    pub rows: Vec<LogRow>,
    /// The row under the cursor (sectioned logs only) — what `z` folds.
    pub cursor: usize,
    /// The job's step names, to match a step to its section.
    pub steps: Vec<String>,
    /// Which of those steps passed — folded runs of passed steps merge into one header.
    pub step_passed: Vec<bool>,
    /// How many lines of the whole log were dropped before `lines[0]` (see [`LOG_MAX_LINES`]).
    /// Folds and the view are remembered by *absolute* line, so a poll that drops more of the
    /// head doesn't shift them onto other steps.
    pub base: usize,
    /// The step (index into `steps`) to unfold and land on when the lines first arrive.
    pub target_step: Option<usize>,
    /// The first line [`is_error_line`] matches, kept alongside the lines it was read from.
    pub first_error: Option<usize>,
    /// A jump waiting for the lines.
    pending: Option<LogJump>,
    /// The first folding has been decided; later polls keep the user's folds.
    folds_ready: bool,
    /// A fetch is wanted; [`App::pump_logs`] sends it once nothing else is in flight.
    want_fetch: bool,
    poll_ticks: u16,
}

impl LogView {
    /// A pane that has asked for its first fetch. `live` decides follow's default.
    pub fn new(title: String, job_id: String, live: bool) -> Self {
        let mut log = Self {
            title,
            job_id,
            lines: vec!["Loading logs…".into()],
            scroll: 0,
            follow: live,
            live,
            viewport: Cell::new(0),
            hscroll: 0,
            viewport_w: Cell::new(0),
            search_input: None,
            query: None,
            matches: Vec::new(),
            match_idx: None,
            note: None,
            fetch_failed: false,
            loaded: false,
            sections: Vec::new(),
            open: HashSet::new(),
            rows: Vec::new(),
            cursor: 0,
            steps: Vec::new(),
            step_passed: Vec::new(),
            base: 0,
            target_step: None,
            first_error: None,
            pending: None,
            folds_ready: false,
            want_fetch: true,
            poll_ticks: 0,
        };
        log.relayout();
        log
    }

    /// A finished pane already holding `lines` (tests and fixtures).
    #[cfg(test)]
    pub fn with_lines(title: &str, job_id: &str, lines: Vec<String>) -> Self {
        let mut log = Self::new(title.into(), job_id.into(), false);
        log.lines = lines;
        log.loaded = true;
        log.want_fetch = false;
        log.reparse();
        log
    }

    /// Whether the log marks its steps, so it draws fold headers and has a cursor.
    pub fn sectioned(&self) -> bool {
        !self.sections.is_empty()
    }

    /// The last row the top of the viewport can sit on. Before the first frame the viewport is
    /// unknown, so a single row is assumed.
    pub fn max_scroll(&self) -> u16 {
        let vp = self.viewport.get().max(1) as usize;
        self.rows.len().saturating_sub(vp).min(u16::MAX as usize) as u16
    }

    /// Top visible row: the bottom while following, else `scroll` clamped.
    pub fn effective_scroll(&self) -> u16 {
        if self.follow {
            self.max_scroll()
        } else {
            self.scroll.min(self.max_scroll())
        }
    }

    /// The cursor's row: the last one while following.
    pub fn cursor_row(&self) -> usize {
        let last = self.rows.len().saturating_sub(1);
        if self.follow {
            last
        } else {
            self.cursor.min(last)
        }
    }

    /// The first line a row stands for (a header stands for its section's marker line).
    pub fn row_line(&self, row: LogRow) -> usize {
        match row {
            LogRow::Line(l) => l,
            LogRow::Header(s) | LogRow::Group(s, _) => self.sections.get(s).map_or(0, |s| s.start),
        }
    }

    /// Whether section `s` is a step that passed.
    fn section_passed(&self, s: usize) -> bool {
        self.sections.get(s).and_then(|sec| sec.step).and_then(|j| self.step_passed.get(j)).copied().unwrap_or(false)
    }

    /// The row drawing `line`, or the first one after it (a hidden marker line has none).
    fn row_of_line(&self, line: usize) -> usize {
        let at = self.rows.partition_point(|r| self.row_line(*r) < line);
        at.min(self.rows.len().saturating_sub(1))
    }

    fn section_of_line(&self, line: usize) -> Option<usize> {
        self.sections.iter().position(|s| s.contains(line))
    }

    /// The row drawing section `section`'s header — its merged group's, when it is in one.
    fn header_row(&self, section: usize) -> usize {
        self.rows
            .iter()
            .position(|r| match *r {
                LogRow::Header(s) => s == section,
                LogRow::Group(a, b) => (a..=b).contains(&section),
                LogRow::Line(_) => false,
            })
            .unwrap_or(0)
    }

    /// Rebuilds `rows` from the lines, sections and folds.
    fn relayout(&mut self) {
        let mut rows = Vec::with_capacity(self.lines.len());
        if self.sections.is_empty() {
            rows.extend((0..self.lines.len()).map(LogRow::Line));
        } else {
            let (mut line, mut next) = (0, 0);
            while line < self.lines.len() {
                while next < self.sections.len() && self.sections[next].start < line {
                    next += 1;
                }
                // Two or more folded passed steps in a row read as one header.
                let folded_pass = |s: usize| !self.open.contains(&s) && self.section_passed(s);
                let mut last = next;
                while next < self.sections.len()
                    && self.sections[next].start == line
                    && folded_pass(last)
                    && last + 1 < self.sections.len()
                    && folded_pass(last + 1)
                    && self.sections[last + 1].start == self.sections[last].end
                {
                    last += 1;
                }
                if last > next {
                    rows.push(LogRow::Group(next, last));
                    line = self.sections[last].end.max(line + 1);
                    next = last + 1;
                    continue;
                }
                if let Some(sec) = self.sections.get(next).filter(|s| s.start == line) {
                    rows.push(LogRow::Header(next));
                    if self.open.contains(&next) {
                        for l in sec.start + 1..sec.end.min(self.lines.len()) {
                            if !runlog::is_hidden_marker(&self.lines[l]) {
                                rows.push(LogRow::Line(l));
                            }
                        }
                    }
                    line = sec.end.max(line + 1);
                    next += 1;
                } else {
                    if !runlog::is_hidden_marker(&self.lines[line]) {
                        rows.push(LogRow::Line(line));
                    }
                    line += 1;
                }
            }
        }
        self.rows = rows;
        if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len().saturating_sub(1);
        }
    }

    /// Re-reads sections and the first error from `lines`, deciding the first folding once.
    #[cfg(test)]
    fn reparse(&mut self) {
        self.reparse_from(self.base);
    }

    /// Re-reads the lines, where the previous ones started at absolute line `old_base`.
    fn reparse_from(&mut self, old_base: usize) {
        self.first_error = first_error_line(&self.lines);
        // Folds are carried by absolute line: a section keeps its fold however much of the
        // log's head a later poll drops.
        let open_abs: HashSet<usize> =
            self.open.iter().filter_map(|&s| self.sections.get(s)).map(|s| s.start + old_base).collect();
        let last_abs = self.sections.last().map(|s| s.start + old_base);
        self.sections = runlog::parse_step_sections(&self.lines, &self.steps);
        let base = self.base;
        self.open = (0..self.sections.len()).filter(|&s| open_abs.contains(&(self.sections[s].start + base))).collect();
        let mut land = None;
        if self.sectioned() {
            if !self.folds_ready {
                self.folds_ready = true;
                land = self.initial_section();
                self.open = land.into_iter().collect();
            } else if self.live {
                // Steps that started since the last poll are the ones being watched.
                let fresh = (0..self.sections.len()).filter(|&s| last_abs.is_none_or(|l| self.sections[s].start + base > l));
                self.open.extend(fresh.collect::<Vec<_>>());
            }
        }
        self.relayout();
        // Land on the opened step — its first error if it has one — with its header at the top.
        if let Some(s) = land.filter(|_| !self.follow) {
            let header = self.header_row(s);
            let sec = &self.sections[s];
            let err = self.lines[sec.start..sec.end.min(self.lines.len())].iter().position(|l| is_error_line(l));
            self.cursor = err.map_or(header, |i| self.row_of_line(sec.start + i));
            self.scroll = header.min(self.max_scroll() as usize) as u16;
            self.ensure_cursor_visible();
        }
    }

    /// The section to leave unfolded first: the step the pane was opened for, else the one
    /// holding the first error, else — for a live job — the latest.
    fn initial_section(&self) -> Option<usize> {
        self.target_step
            .and_then(|i| runlog::step_section(&self.sections, &self.steps, i))
            .or_else(|| self.first_error.and_then(|l| self.section_of_line(l)))
            .or_else(|| self.live.then(|| self.sections.len().checked_sub(1)).flatten())
    }

    /// Scrolls just enough to bring the cursor row into view.
    fn ensure_cursor_visible(&mut self) {
        let vp = self.viewport.get().max(1) as usize;
        let top = self.effective_scroll() as usize;
        let cursor = self.cursor_row();
        if cursor < top {
            self.scroll = cursor as u16;
        } else if cursor >= top + vp {
            self.scroll = (cursor + 1 - vp).min(self.max_scroll() as usize) as u16;
        } else {
            self.scroll = top as u16;
        }
    }

    /// Moves the cursor (sectioned logs) by `delta` rows. Moving up breaks follow.
    fn move_cursor(&mut self, delta: i32) {
        let Some(last) = self.rows.len().checked_sub(1) else { return };
        let top = self.effective_scroll();
        self.cursor = (self.cursor_row() as i64 + i64::from(delta)).clamp(0, last as i64) as usize;
        if delta < 0 && self.follow {
            self.follow = false;
            self.scroll = top;
        }
        if !self.follow {
            self.ensure_cursor_visible();
        }
    }

    /// Scrolls by `delta` rows — or, in a sectioned log, moves the cursor. Scrolling up breaks
    /// follow; scrolling down never re-arms it (only `G`/End does).
    fn scroll_by(&mut self, delta: i32) {
        if self.sectioned() {
            self.move_cursor(delta);
            return;
        }
        let cur = self.effective_scroll() as i32;
        self.scroll = (cur + delta).clamp(0, self.max_scroll() as i32) as u16;
        if delta < 0 {
            self.follow = false;
        }
    }

    /// Pans by half the pane's width, so a long line reads in a few presses, and stops once
    /// the widest line's end is in view.
    fn pan_by(&mut self, dir: i32) {
        let vw = self.viewport_w.get() as usize;
        let step = (vw / 2).max(8);
        let lead = if self.sectioned() { 3 } else { 0 };
        let widest = self.rows.iter().filter_map(|r| match r {
            LogRow::Line(l) => Some(lead + self.row_text(*l).chars().count()),
            _ => None,
        });
        let max = widest.max().unwrap_or(0).saturating_sub(vw);
        self.hscroll = if dir < 0 { self.hscroll.saturating_sub(step) } else { (self.hscroll + step).min(max) };
    }

    /// The text a log line draws: a sectioned log leaves the ISO timestamp out.
    pub fn row_text(&self, line: usize) -> &str {
        let text = &self.lines[line];
        if self.sectioned() { runlog::strip_timestamp(text) } else { text }
    }

    fn scroll_top(&mut self) {
        self.scroll = 0;
        self.cursor = 0;
        self.follow = false;
    }

    fn scroll_bottom(&mut self) {
        self.scroll = self.max_scroll();
        self.cursor = self.rows.len().saturating_sub(1);
        self.follow = true;
    }

    /// Brings `line` into view with a couple of rows of context above it — unfolding its section
    /// first — puts the cursor on it, and stops following.
    fn jump_to(&mut self, line: usize) {
        self.follow = false;
        if let Some(s) = self.section_of_line(line) {
            if self.open.insert(s) {
                self.relayout();
            }
        }
        let row = self.row_of_line(line);
        self.cursor = row;
        self.scroll = row.saturating_sub(2).min(self.max_scroll() as usize) as u16;
    }

    /// `z`: folds or unfolds the section under the cursor, keeping the cursor on its header.
    fn toggle_fold(&mut self) {
        if !self.sectioned() {
            return;
        }
        let section = match self.rows.get(self.cursor_row()).copied() {
            Some(LogRow::Header(s)) => Some(s),
            Some(LogRow::Group(a, b)) => {
                // A merged group opens every step in it.
                let top = self.effective_scroll();
                self.follow = false;
                self.scroll = top;
                self.open.extend(a..=b);
                self.relayout();
                self.cursor = self.header_row(a);
                self.ensure_cursor_visible();
                return;
            }
            Some(LogRow::Line(l)) => self.section_of_line(l),
            None => None,
        };
        let Some(s) = section else { return };
        let top = self.effective_scroll();
        self.follow = false;
        self.scroll = top;
        if !self.open.remove(&s) {
            self.open.insert(s);
        }
        self.relayout();
        self.cursor = self.header_row(s);
        self.ensure_cursor_visible();
    }

    /// `Z`: unfolds every section, or folds them all when all are already open.
    fn toggle_all_folds(&mut self) {
        if !self.sectioned() {
            return;
        }
        let line = self.rows.get(self.cursor_row()).map(|r| self.row_line(*r));
        let top = self.effective_scroll();
        self.follow = false;
        self.scroll = top;
        if self.open.len() == self.sections.len() {
            self.open.clear();
        } else {
            self.open = (0..self.sections.len()).collect();
        }
        self.relayout();
        if let Some(line) = line {
            self.cursor = match self.section_of_line(line) {
                Some(s) if !self.open.contains(&s) => self.header_row(s),
                _ => self.row_of_line(line),
            };
        }
        self.ensure_cursor_visible();
    }

    /// Unfolds the `index`th step's section and puts its header at the top. Before the lines
    /// arrive this just records the step, for the first folding to open.
    pub fn focus_step(&mut self, index: usize) {
        self.target_step = Some(index);
        if !self.loaded || !self.sectioned() {
            return;
        }
        match runlog::step_section(&self.sections, &self.steps, index) {
            Some(s) => {
                self.open.insert(s);
                self.relayout();
                self.follow = false;
                self.cursor = self.header_row(s);
                self.scroll = self.cursor.min(self.max_scroll() as usize) as u16;
            }
            None => {
                let name = self.steps.get(index).cloned().unwrap_or_default();
                self.note = Some(format!("no section for {name} in this log"));
            }
        }
    }

    /// Queues a jump for when the lines arrive, or makes it now if they already have.
    pub fn request_jump(&mut self, jump: LogJump) {
        self.pending = Some(jump);
        if self.loaded {
            self.apply_pending();
        }
    }

    fn apply_pending(&mut self) {
        let Some(jump) = self.pending.take() else { return };
        match jump {
            LogJump::FirstError => self.jump_first_error(),
            LogJump::Find(needles) => {
                let hit = needles
                    .iter()
                    .filter(|n| !n.is_empty())
                    .find_map(|n| self.lines.iter().position(|l| l.contains(n.as_str())));
                match hit {
                    Some(i) => {
                        self.jump_to(i);
                        self.note = Some(format!("line {}", i + 1));
                    }
                    None => {
                        self.jump_first_error();
                        let what = needles.first().cloned().unwrap_or_default();
                        self.note = Some(format!("{what} isn't in this log — first error instead"));
                    }
                }
            }
        }
    }

    /// Replaces the lines with a fresh fetch, keeping the tail past [`LOG_MAX_LINES`], the view
    /// on the same content, and the current search match where it still exists.
    pub fn set_text(&mut self, text: &str) {
        let lines: Vec<String> =
            if text.trim().is_empty() { vec!["(no logs returned)".into()] } else { text.lines().map(str::to_owned).collect() };
        let (lines, dropped) = cap_tail(lines, LOG_MAX_LINES);
        // Everything below is remembered as an absolute line (`base` + index), then mapped back
        // onto the new lines and rows: the head the cap drops can grow between polls.
        let old_base = self.base;
        let abs = |line: usize| line + old_base;
        let prev_match = self.match_idx.and_then(|i| self.matches.get(i).copied()).map(abs);
        let keep_view = self.loaded && !self.follow;
        let top_line = keep_view.then(|| self.rows.get(self.effective_scroll() as usize).map(|r| abs(self.row_line(*r)))).flatten();
        let cursor_line = (keep_view && self.sectioned())
            .then(|| self.rows.get(self.cursor).map(|r| abs(self.row_line(*r))))
            .flatten();
        self.lines = lines;
        self.base = dropped;
        self.reparse_from(old_base);
        let local = |line: usize| line.saturating_sub(dropped);
        if let Some(line) = top_line {
            self.scroll = self.row_of_line(local(line)).min(self.max_scroll() as usize) as u16;
        }
        if let Some(line) = cursor_line {
            self.cursor = self.row_of_line(local(line));
        }
        self.recompute_matches();
        self.match_idx = match prev_match {
            Some(line) if !self.matches.is_empty() => {
                let target = local(line);
                Some(self.matches.iter().position(|&m| m >= target).unwrap_or(self.matches.len() - 1))
            }
            _ => None,
        };
        self.loaded = true;
        self.apply_pending();
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
        let top = self.rows.get(self.effective_scroll() as usize).map_or(0, |r| self.row_line(*r));
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
pub fn first_failed_node(run: &PipelineRun) -> Option<(usize, usize, Option<usize>)> {
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
    /// Fold state the user chose, by node key (`s0`, `s0.j1`, `s0.j1.pass`, `s2.skip`): `true`
    /// open. A node absent here takes its default — finished stages folded, the stage needing
    /// attention open, passed jobs of a failed run folded, step groups folded — so a refresh that
    /// moves a stage on re-decides only what the user hasn't touched.
    folds: HashMap<String, bool>,
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
    /// Whether this run's provider can re-run a whole run / only its failed jobs, and list its
    /// artifacts. From the detail fetch; `false` until it answers.
    pub supports_rerun: bool,
    pub supports_rerun_failed: bool,
    pub supports_artifacts: bool,
    /// Whether this provider's rerun (all / failed only) starts a separate run.
    pub rerun_new_run: bool,
    pub rerun_failed_new_run: bool,
    /// Problems the run reported (the Problems panel), most severe first.
    pub annotations: Vec<PipelineAnnotation>,
    /// Keys go to the Problems panel (`e`), and which problem is selected.
    pub problem_focus: bool,
    pub problem_sel: usize,
    /// Recent runs of this pipeline on this branch. Rebuilt by the app from its list rows.
    pub history: Option<RunHistory>,
    /// Estimated durations for an in-flight run. Rebuilt by the app.
    pub estimates: Estimates,
    /// The artifacts overlay, while open.
    pub artifacts: Option<ArtifactsPanel>,
    /// What the failed job's log says went wrong, read when that log loads: `(job id, summary)`.
    pub log_failure: Option<(String, FailureSummary)>,
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
            folds: HashMap::new(),
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
            supports_rerun: false,
            supports_rerun_failed: false,
            supports_artifacts: false,
            rerun_new_run: false,
            rerun_failed_new_run: false,
            annotations: Vec::new(),
            problem_focus: false,
            problem_sel: 0,
            history: None,
            estimates: Estimates::default(),
            artifacts: None,
            log_failure: None,
        }
    }

    /// Patches in a freshly-fetched run, preserving what the user is doing: the open log pane
    /// and the expand/collapse tree state are untouched (this never rebuilds the view), and the
    /// selection is clamped rather than reset, since the new run's flattened node count may be
    /// shorter. `folds` is private, so this is how a caller outside the impl block patches
    /// it without reaching in. Called only once a real `get_run` has confirmed the run, which is
    /// also the moment `stale` clears.
    ///
    /// `approvals` is `None` when this fetch's `pending_approvals` call failed, and then the
    /// gates already on screen are left exactly as they are. A gate here drives a real
    /// approve/reject (see [`PipelineView::actionable_approvals`]), so only a call that actually
    /// answered may set them — never a value read back out of the cache, which may name a gate
    /// that was decided an hour ago.
    /// Takes the problems and capabilities a detail carries.
    fn apply_extras(&mut self, detail: &PipelineDetail) {
        // An in-flight run's problems are the last attempt's; they come back once it finishes.
        self.annotations = if detail.run.status.is_active() { Vec::new() } else { detail.annotations.clone() };
        if self.problem_sel >= self.annotations.len() {
            self.problem_sel = self.annotations.len().saturating_sub(1);
        }
        if self.annotations.is_empty() {
            self.problem_focus = false;
        }
        self.supports_rerun = detail.supports_rerun;
        self.supports_rerun_failed = detail.supports_rerun_failed;
        self.supports_artifacts = detail.supports_artifacts;
        self.rerun_new_run = detail.rerun_new_run;
        self.rerun_failed_new_run = detail.rerun_failed_new_run;
    }

    fn apply_fresh_run(&mut self, run: PipelineRun, approvals: Option<Vec<PipelineApproval>>) {
        // Fold defaults follow live status — a stage folds once it passes — so a row index from
        // before the refresh can point at a different node after it. Hold on to the node instead.
        let anchor = self.flatten().get(self.selected).map(|n| (n.stage, n.job, n.step, n.key.clone()));
        self.run = run;
        self.apply_confirmed_approvals(approvals);
        self.stale = false;
        if let Some(anchor) = anchor {
            self.reselect(anchor);
        }
        self.clamp_selection();
        self.auto_select();
    }

    /// Puts the cursor back on the node it was on. When the run moved on and folded that node's
    /// stage (or job) away, the fold is pinned open rather than the cursor jumping — the user was
    /// looking at it, and an open log pane is still showing it.
    fn reselect(&mut self, (stage, job, step, key): (usize, Option<usize>, Option<usize>, Option<String>)) {
        let find = |v: &Self| v.flatten().iter().position(|n| n.stage == stage && n.job == job && n.step == step && n.key == key);
        let mut at = find(self);
        if at.is_none() {
            if let Some(j) = job {
                self.folds.entry(format!("s{stage}")).or_insert(true);
                if step.is_some() {
                    self.folds.entry(format!("s{stage}.j{j}")).or_insert(true);
                }
                at = find(self);
            }
        }
        if let Some(i) = at {
            self.selected = i;
        }
    }

    /// What the run reads as: Waiting while a gate holds it (whoever may answer the gate) and
    /// nothing is still running — see [`PipelineRun::shown_status`]. Derived rather than written
    /// into `run`, which stays what the provider said.
    pub fn shown_status(&self) -> PipelineRunStatus {
        self.run.shown_status(self.approvals.iter().any(|a| a.blocks_run))
    }

    /// The stage this run is parked at, when it is waiting (see [`gate_stage`]).
    pub fn gate_stage(&self) -> Option<usize> {
        let names: Vec<String> = self.approvals.iter().filter(|a| a.blocks_run).map(|a| a.name.clone()).collect();
        gate_stage(&self.run, self.shown_status(), &names)
    }

    /// Whether a foldable node is open: the user's choice when they made one, else `default`.
    fn is_open(&self, key: &str, default: bool) -> bool {
        self.folds.get(key).copied().unwrap_or(default)
    }

    /// First load: puts the cursor on what needs attention. A failed run selects its first failed
    /// step (or, lacking one, its first failed job), expanding the parents, and asks for its logs
    /// to open; a parked run selects the stage it waits at; a running one its first running job.
    /// Runs until the first load has decided — a cache-seeded run with no stages (or gates) yet
    /// waits for the live one — and never once the user has moved the cursor.
    fn auto_select(&mut self) {
        if self.auto_checked || self.user_moved {
            return;
        }
        if self.run.status == PipelineRunStatus::Failed {
            if let Some((si, ji, step)) = first_failed_node(&self.run) {
                self.folds.insert(format!("s{si}"), true);
                if step.is_some() {
                    self.folds.insert(format!("s{si}.j{ji}"), true);
                }
                self.selected = self.node_index(si, ji, step);
                self.auto_logs = true;
                self.auto_checked = true;
                return;
            }
        }
        if let Some(i) = self.attention_row() {
            self.selected = i;
        }
        if !self.stale {
            self.auto_checked = true;
        }
    }

    /// The row of an in-flight run that wants watching: the stage a parked run waits at, or a
    /// running run's first running job. `None` when there is no such row on screen.
    fn attention_row(&self) -> Option<usize> {
        let nodes = self.flatten();
        match self.shown_status() {
            PipelineRunStatus::Waiting => {
                let si = self.gate_stage()?;
                nodes.iter().position(|n| n.stage == si && n.job.is_none() && !n.info && !n.group)
            }
            PipelineRunStatus::Running => {
                let (si, ji) = self.run.stages.iter().enumerate().find_map(|(si, s)| {
                    s.jobs.iter().position(|j| j.status == PipelineRunStatus::Running).map(|ji| (si, ji))
                })?;
                nodes.iter().position(|n| n.stage == si && n.job == Some(ji) && n.step.is_none() && !n.group)
            }
            _ => None,
        }
    }

    /// Row of a job (or one of its steps) in [`PipelineView::flatten`]'s order. Its stage and
    /// job must already be expanded; a step folded into a group has that group opened. A job whose
    /// own row isn't drawn (a single-job run) lands on its first row.
    fn node_index(&mut self, si: usize, ji: usize, step: Option<usize>) -> usize {
        let find = |nodes: &[FlatNode]| {
            nodes.iter().position(|n| !n.group && n.stage == si && n.job == Some(ji) && n.step == step)
        };
        if let Some(i) = find(&self.flatten()) {
            return i;
        }
        self.folds.insert(format!("s{si}.j{ji}.pass"), true);
        self.folds.insert(format!("s{si}.j{ji}.post"), true);
        let nodes = self.flatten();
        find(&nodes)
            .or_else(|| nodes.iter().position(|n| n.stage == si && n.job == Some(ji)))
            .unwrap_or(0)
    }

    /// The node under the cursor.
    pub fn selected_node(&self) -> Option<FlatNode> {
        self.flatten().into_iter().nth(self.selected)
    }

    /// The run's first failed node as `(job id, step name, step index)` — what the failure line
    /// names and `E` opens.
    pub fn failed_target(&self) -> Option<(String, String, Option<usize>)> {
        let (si, ji, step) = first_failed_node(&self.run)?;
        let job = self.run.stages.get(si)?.jobs.get(ji)?;
        let name = step.and_then(|k| job.steps.get(k)).map_or_else(|| job.name.clone(), |s| s.name.clone());
        Some((job.id.clone(), name, step))
    }

    /// Whether the job the log pane follows is still running — the run's own status when the
    /// job isn't in the tree (yet).
    pub fn log_target_active(&self) -> bool {
        let Some(log) = &self.logs else { return false };
        let job = self.run.stages.iter().flat_map(|s| &s.jobs).find(|j| j.id == log.job_id);
        job.map_or(self.run.status, |j| j.status).is_active()
    }

    /// Opens (or re-targets) the log pane on the selected node's job. Steps share their job's
    /// log, so moving between a job and its steps keeps the pane — landing on a step unfolds its
    /// section. Returns false when the node has no job (a stage).
    fn open_logs_for_selection(&mut self) -> bool {
        let nodes = self.flatten();
        let Some(node) = nodes.get(self.selected) else { return false };
        let Some(job_id) = node.job_id.clone() else { return false };
        let step = if node.group { None } else { node.step };
        let reuse = self.logs.as_ref().is_some_and(|l| l.job_id == job_id);
        let log = self.open_logs_for_job(&job_id, step);
        if reuse {
            if let Some(k) = step {
                log.focus_step(k);
            }
        }
        true
    }

    /// The log pane on `job_id`, opening it if it shows another job (or none). A new pane lands
    /// on `step`, or on the job's failed step.
    fn open_logs_for_job(&mut self, job_id: &str, step: Option<usize>) -> &mut LogView {
        if !self.logs.as_ref().is_some_and(|l| l.job_id == job_id) {
            let job = self.run.stages.iter().flat_map(|s| &s.jobs).find(|j| j.id == job_id);
            let label = job.map_or_else(|| job_id.to_string(), |j| j.name.clone());
            let live = job.map_or(self.run.status, |j| j.status).is_active();
            // A search carries over to the next job: it is usually the same question.
            let query = self.logs.as_mut().and_then(|l| l.query.take());
            let mut log = LogView::new(format!("Logs · {label}"), job_id.to_string(), live);
            log.query = query;
            if let Some(job) = job {
                log.steps = job.steps.iter().map(|s| s.name.clone()).collect();
                log.step_passed = job.steps.iter().map(|s| s.status == PipelineRunStatus::Succeeded).collect();
            }
            log.target_step =
                step.or_else(|| job.and_then(|j| j.steps.iter().position(|s| s.status == PipelineRunStatus::Failed)));
            self.logs = Some(log);
        }
        self.logs.as_mut().expect("the pane was just opened")
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

    /// Flattens stages/jobs/steps into visible rows, honouring folded nodes.
    ///
    /// A run with a single stage drops the stage row (GitHub's synthetic `jobs`), and a single
    /// stage with a single job drops the job row too, so the steps sit at the top. Leading passed
    /// steps of a failed run and trailing cleanup steps fold into one row each (see
    /// [`step_folds`]); those start folded and open on ↵.
    ///
    /// What starts open is what needs attention: finished stages fold to one line each, the stage
    /// that is running, failed or waiting on a gate stays open, and jobs follow the same rule: the
    /// ones that finished cleanly fold, so the broken or running one is what you see. Two or more
    /// stages in a row that never ran fold into one `N stages skipped` row. Whatever the user folds
    /// or opens stays that way.
    pub fn flatten(&self) -> Vec<FlatNode> {
        let mut out = Vec::new();
        let stages = &self.run.stages;
        let single_stage = stages.len() == 1;
        let failed = self.run.status == PipelineRunStatus::Failed;
        let gate = self.gate_stage();
        let live = self.run.status.is_active();
        let shown = |si: usize| match stages[si].status {
            _ if gate == Some(si) => PipelineRunStatus::Waiting,
            // A gate on a run that has since finished (cancelled, say) never opened: the stage
            // didn't run. Still in flight, a stage the provider says is gated really is — even
            // while a parallel stage runs and the run as a whole reads Running.
            PipelineRunStatus::Waiting if !live => PipelineRunStatus::Canceled,
            other => other,
        };
        let mut si = 0;
        while si < stages.len() {
            if !single_stage && shown(si) == PipelineRunStatus::Skipped {
                let n = (si..stages.len()).take_while(|&k| shown(k) == PipelineRunStatus::Skipped).count();
                if n >= 2 {
                    self.push_skipped_run(&mut out, si, n);
                    si += n;
                    continue;
                }
            }
            self.flatten_stage(&mut out, si, shown(si), &shown, single_stage, failed);
            si += 1;
        }
        out
    }

    /// `n` consecutive skipped stages from `first` as one foldable row, naming the failure that
    /// stopped them; opened, it lists the stages.
    fn push_skipped_run(&self, out: &mut Vec<FlatNode>, first: usize, n: usize) {
        let stages = &self.run.stages;
        let key = format!("s{first}.skip");
        let open = self.is_open(&key, false);
        let cause = stages[..first].iter().rev().find(|s| s.status == PipelineRunStatus::Failed).map(|s| format!("{} failed", s.name));
        out.push(FlatNode {
            key: Some(key),
            expanded: open,
            note: cause,
            group: true,
            ..FlatNode::bare(0, format!("{n} stages skipped"), PipelineRunStatus::Skipped, first)
        });
        if open {
            for (k, stage) in stages.iter().enumerate().skip(first).take(n) {
                out.push(FlatNode::bare(1, stage.name.clone(), PipelineRunStatus::Skipped, k));
            }
        }
    }

    /// One stage's rows: its own (unless it is the run's only stage), then — when open — its jobs
    /// and their steps, or for a gated stage with no jobs yet, a line saying why.
    fn flatten_stage(
        &self,
        out: &mut Vec<FlatNode>,
        si: usize,
        status: PipelineRunStatus,
        shown: &dyn Fn(usize) -> PipelineRunStatus,
        single_stage: bool,
        failed: bool,
    ) {
        let stage = &self.run.stages[si];
        let key = format!("s{si}");
        let default_open = !matches!(
            status,
            PipelineRunStatus::Succeeded | PipelineRunStatus::PartiallySucceeded | PipelineRunStatus::Skipped
        );
        let expanded = single_stage || self.is_open(&key, default_open);
        if !single_stage {
            let start = stage.jobs.iter().filter_map(|j| j.started_at).min();
            let finish = if stage.jobs.iter().all(|j| j.finished_at.is_some()) {
                stage.jobs.iter().filter_map(|j| j.finished_at).max()
            } else {
                None
            };
            // A gated stage with no jobs yet still opens, onto the line explaining why.
            let gate_info = status == PipelineRunStatus::Waiting && stage.jobs.is_empty();
            // A queued stage with nothing in it yet says what it waits on: the nearest stage
            // before it that hasn't finished.
            let note = (status == PipelineRunStatus::Queued && stage.jobs.is_empty())
                .then(|| (0..si).rev().find(|&k| shown(k).is_active()))
                .flatten()
                .map(|k| format!("after {}", self.run.stages[k].name));
            out.push(FlatNode {
                depth: 0,
                label: stage.name.clone(),
                status,
                key: (!stage.jobs.is_empty() || gate_info).then(|| key.clone()),
                expanded,
                duration: stage_duration(&stage.jobs),
                problem: None,
                url: None,
                job_id: None,
                started_at: start,
                finished_at: finish,
                stage: si,
                job: None,
                step: None,
                group: false,
                summary: if expanded { None } else { stage_summary(stage, status) },
                note,
                info: false,
            });
            if !expanded {
                return;
            }
            if gate_info {
                out.push(FlatNode {
                    info: true,
                    ..FlatNode::bare(1, "approval required · its jobs start once approved".into(), status, si)
                });
                if !self.can_respond_approvals {
                    let hint = format!("o  open in {} to approve", provider_label(self.provider));
                    out.push(FlatNode { info: true, ..FlatNode::bare(1, hint, status, si) });
                }
                return;
            }
        }
        let job_depth = usize::from(!single_stage);
        let single_job = single_stage && stage.jobs.len() == 1 && !stage.jobs[0].steps.is_empty();
        for (ji, job) in stage.jobs.iter().enumerate() {
            let jkey = format!("s{si}.j{ji}");
            // Like stages, a job that finished cleanly folds to its one line; the one running,
            // failed or waiting is what you see.
            let jdefault = !matches!(
                job.status,
                PipelineRunStatus::Succeeded | PipelineRunStatus::PartiallySucceeded | PipelineRunStatus::Skipped
            );
            let jexpanded = single_job || self.is_open(&jkey, jdefault);
            if !single_job {
                out.push(FlatNode {
                    depth: job_depth,
                    label: job.name.clone(),
                    status: job.status,
                    key: (!job.steps.is_empty()).then(|| jkey.clone()),
                    expanded: jexpanded,
                    duration: fmt_duration(job.started_at, job.finished_at),
                    problem: job.problem.clone(),
                    url: job.url.clone(),
                    job_id: Some(job.id.clone()),
                    started_at: job.started_at,
                    finished_at: job.finished_at,
                    stage: si,
                    job: Some(ji),
                    step: None,
                    group: false,
                    summary: None,
                    note: None,
                    info: false,
                });
            }
            if jexpanded {
                let depth = if single_job { job_depth } else { job_depth + 1 };
                self.flatten_steps(out, si, ji, job, depth, failed);
            }
        }
    }

    /// A job's step rows, with its passed and cleanup runs folded into group rows.
    fn flatten_steps(&self, out: &mut Vec<FlatNode>, si: usize, ji: usize, job: &PipelineJob, depth: usize, failed: bool) {
        let (passed, cleanup) = step_folds(job, failed);
        let step_node = |k: usize, step: &PipelineStep, depth: usize| FlatNode {
            depth,
            label: step.name.clone(),
            status: step.status,
            key: None,
            expanded: false,
            duration: fmt_duration(step.started_at, step.finished_at),
            problem: None,
            url: job.url.clone(),
            job_id: Some(job.id.clone()),
            started_at: step.started_at,
            finished_at: step.finished_at,
            stage: si,
            job: Some(ji),
            step: Some(k),
            group: false,
            summary: None,
            note: None,
            info: false,
        };
        let group = |out: &mut Vec<FlatNode>, range: std::ops::Range<usize>, label: String, suffix: &str| {
            let key = format!("s{si}.j{ji}.{suffix}");
            let open = self.is_open(&key, false);
            let steps = &job.steps[range.clone()];
            let (start, finish) = steps_span(steps);
            out.push(FlatNode {
                depth,
                label,
                status: steps_status(steps),
                key: Some(key),
                expanded: open,
                duration: fmt_duration(start, finish),
                problem: None,
                url: job.url.clone(),
                job_id: Some(job.id.clone()),
                started_at: start,
                finished_at: finish,
                stage: si,
                job: Some(ji),
                step: None,
                group: true,
                summary: None,
                note: None,
                info: false,
            });
            if open {
                for k in range {
                    out.push(step_node(k, &job.steps[k], depth + 1));
                }
            }
        };
        if passed > 0 {
            group(out, 0..passed, format!("{passed} steps passed"), "pass");
        }
        for k in passed..cleanup {
            out.push(step_node(k, &job.steps[k], depth));
        }
        if cleanup < job.steps.len() {
            let n = job.steps.len() - cleanup;
            group(out, cleanup..job.steps.len(), format!("{n} post & cleanup steps"), "post");
        }
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
    pub(crate) fn toggle_selected(&mut self) {
        if let Some((Some(key), open)) = self.flatten().get(self.selected).map(|n| (n.key.clone(), n.expanded)) {
            // Recorded as the user's choice, so it outlives a refresh that changes the default.
            self.folds.insert(key, !open);
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
    /// Paths the reviewer has marked "viewed". The view's copy; [`App::pr_viewed`] keeps them
    /// across closing and reopening the PR.
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
            pipe_catalog: HashMap::new(),
            pipe_scope: None,
            health: Vec::new(),
            visible: [true; 3],
            status: "Loading…".into(),
            loading: true,
            prs_landed: false,
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
            hidden_cols: default_hidden_columns(),
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
            inbox_dismissed: HashSet::new(),
            lp_origin: false,
            from_inbox: false,
            reloading: false,
            job_tx: None,
            anim: 0,
            data_age: None,
            last_refresh: Local::now(),
            should_quit: false,
            preview: None,
            preview_hidden: DEFAULT_PREVIEW_HIDDEN,
            pr_viewed: HashMap::new(),
            preview_focus: false,
            content_w: 0,
            log_inflight: None,
            run_details: HashMap::new(),
            estimate_requested: HashSet::new(),
            clipboard: osc52_clipboard,
            run_actions: HashMap::new(),
            next_run_action: 0,
            held_runs: HashMap::new(),
            item_actions: HashMap::new(),
            next_item_action: 0,
            held_items: HashMap::new(),
            reload_again: false,
            follow_new_run: None,
            annotations_asked: std::cell::RefCell::new(HashMap::new()),
            annotations_by_key: HashMap::new(),
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

    /// How many distinct pipelines the fetched runs belong to — the tab's badge. The section
    /// counts pipelines (as its header does), not runs: one busy pipeline fetches many runs.
    /// Keyed as [`PipeGroup::Pipeline`] keys a group, whatever grouping is active.
    pub fn pipeline_count(&self) -> usize {
        self.pipes
            .iter()
            .map(|p| (&p.connection_id, &p.run.repository, &p.run.definition_id))
            .collect::<HashSet<_>>()
            .len()
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

    /// The copy of a row's run that carries its stages: the row's own when the list call brought
    /// them, else a detail already fetched this session for the same run in the same state (a
    /// detail from while it ran says nothing true about it once finished). Never a fetch.
    pub fn pipe_staged_run<'a>(&'a self, p: &'a PipeRow) -> &'a PipelineRun {
        if !p.run.stages.is_empty() {
            return &p.run;
        }
        let key = pipeline_detail_cache_key(&p.connection_id, &p.run.item_ref());
        self.run_details
            .get(&key)
            .filter(|d| d.status == p.run.status || (d.status.is_active() && p.run.status.is_active()))
            .unwrap_or(&p.run)
    }

    /// The heading for the subject column. Once a group is open that column carries both kinds
    /// of value, so the heading says so rather than naming only half of what sits under it.
    pub fn pipe_subject_heading(&self, any_open: bool) -> &'static str {
        match (self.pipe_group, any_open) {
            (PipeGroup::Off, _) | (PipeGroup::Pipeline, false) => "Pipeline",
            (PipeGroup::Pipeline, true) => "Pipeline / Run",
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
                status: latest.shown_status(),
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
        self.anim = 0; // restart the title scroll on the newly-selected row
    }

    pub fn move_up(&mut self) {
        let len = self.active_len();
        if len == 0 {
            return;
        }
        let next = self.active_state().selected().map_or(0, |i| (i + len - 1) % len);
        self.active_state().select(Some(next));
        self.anim = 0; // restart the title scroll on the newly-selected row
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

    /// Applies persisted column choices at startup; `None` (never chosen) keeps the defaults.
    pub fn apply_hidden_columns(&mut self, hidden: Option<[Vec<String>; 3]>) {
        self.hidden_cols = hidden.unwrap_or_else(default_hidden_columns);
    }

    /// Nothing chosen to fetch from: every connection's scope emptied, or repositories
    /// discovered and none picked yet (how a new connection starts).
    pub fn no_repos_chosen(&self, section: usize) -> bool {
        self.repo_scope[section].as_ref().is_some_and(|s| s.none_selected || s.selected == 0)
    }

    /// Whether the `name`d column of `section`'s table is switched on.
    pub fn col_shown(&self, section: usize, name: &str) -> bool {
        !self.hidden_cols[section].iter().any(|h| h == name)
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

    /// Applies persisted inbox dismissals at startup; the rows themselves are filtered as they
    /// arrive, from the cache or a reload.
    pub fn apply_dismissed_notifications(&mut self, keys: &[String]) {
        self.inbox_dismissed = keys.iter().cloned().collect();
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
            if pr.is_empty() { default_views(0) } else { reorder_legacy_pr_views(pr) },
            if wi.is_empty() { default_views(1) } else { wi },
            if pipe.is_empty() { default_views(2) } else { pipe },
        ];
        // Land on the first view. Only its base filter: a persisted section sort outranks the
        // view's (and the default views carry none), so this must not go through `apply_view`.
        self.pr_filter = parse_pr_filter(self.views[0].first().and_then(|v| v.filter.as_deref()));
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
            AppEvent::PrPoolLoaded { pool, ok } => {
                let ok = SectionsOk { prs: ok, lp_mine: ok, lp_review: ok, wis: false, pipes: false, inbox: false };
                self.take_pr_pool(*pool, ok, deps);
                self.prs_landed = true;
                self.rebuild_launchpad();
                self.fix_selection();
                self.refresh_preview_row();
            }
            AppEvent::Reloaded(r) => {
                self.apply_reloaded(*r, deps);
                if std::mem::take(&mut self.reload_again) {
                    self.request_reload(deps);
                }
                self.follow_started_run(deps);
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
                // Every run detail seen this session feeds the job estimates of in-flight runs.
                if let Some(run) = &detail.run {
                    self.run_details.insert(key.clone(), run.clone());
                }
                self.with_preview_screen(&key.clone(), |app| app.apply_pipeline_detail(deps, key, *detail, fetched_at));
            }
            AppEvent::PrDecorationsLoaded { items } => {
                self.apply_pr_decorations(items, deps);
            }
            AppEvent::PipelineLogsLoaded { conn_id, run_id, job_id, text } => {
                self.apply_pipeline_logs(&conn_id, &run_id, &job_id, text);
            }
            AppEvent::PipelineArtifactsLoaded { conn_id, run_id, result } => {
                self.apply_pipeline_artifacts(&conn_id, &run_id, result);
            }
            AppEvent::PipelineRunActionDone { token, result } => self.finish_run_action(token, result, deps),
            AppEvent::PrActionDone { token, result, fresh } => self.finish_pr_action(token, result, fresh.map(|p| *p), deps),
            AppEvent::WiActionDone { token, result, fresh } => self.finish_wi_action(token, result, fresh.map(|w| *w), deps),
            AppEvent::PipelineAnnotationsLoaded { key, annotations } => self.apply_pipeline_annotations(deps, key, annotations),
        }
        self.drive_logs(deps);
        self.refresh_pipeline_context();
        self.request_estimates(deps);
    }


    /// Rebuilds the Launchpad rows from the current feeds (no fetch) and clamps selection.
    fn rebuild_launchpad(&mut self) {
        let built = launchpad::build(&self.lp_prs_review, &self.lp_prs_mine, &self.wis, &self.pipes);
        self.lp = built.entries;
        self.lp_overflow = built.overflow;
        // A run held on a gate reads as Waiting here too. Only the row knows its gates, so the
        // entry's copy of the run takes the row's shown status (the row itself is untouched).
        for e in &mut self.lp {
            if let launchpad::EntryItem::Pipe { run, .. } = &mut e.item {
                if let Some(row) = self.pipes.iter().find(|r| r.connection_id == e.connection_id && r.run.id == run.id) {
                    run.status = row.shown_status();
                }
            }
        }
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

    /// Whether the section list shows the preview pane unasked: on unless off for this section,
    /// and only when the terminal is wide enough for both halves.
    pub fn preview_shown(&self) -> bool {
        self.active < 3 && !self.preview_hidden[self.active] && self.content_w >= PREVIEW_MIN_WIDTH
    }

    /// Whether an item can open in the pane beside its list: a section list on a terminal wide
    /// enough for both, whether or not that list previews unasked.
    fn pane_fits(&self) -> bool {
        self.active < 3 && self.content_w >= PREVIEW_MIN_WIDTH
    }

    /// Applies the persisted per-section preview switches at startup; `None` (never switched)
    /// keeps [`DEFAULT_PREVIEW_HIDDEN`].
    pub fn apply_preview_hidden(&mut self, hidden: Option<&[Section]>) {
        let Some(hidden) = hidden else {
            self.preview_hidden = DEFAULT_PREVIEW_HIDDEN;
            return;
        };
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
                Some(Self::build_pr_view(deps, &self.pr_viewed, 0, pr_label(&row.pr), row.pr.url.clone(), row.connection_id.clone(), row.pr.clone()))
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
        self.refresh_pipeline_context();
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
        self.request_estimates(deps);
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

    /// Whether a detail fetch for the pipeline view `key` names should ask for its problems: only
    /// for a finished run (an in-flight one hasn't reported them yet), and once per status — a
    /// finished run's problems don't change until it runs again. Records the ask.
    fn wants_annotations(&self, key: &str) -> bool {
        let view = [Some(&self.screen), self.preview.as_ref().map(|p| &p.view)].into_iter().flatten().find_map(|s| match s {
            Screen::Pipeline(v) if pipeline_detail_cache_key(&v.connection_id, &v.run.item_ref()) == key => Some(v),
            _ => None,
        });
        let Some(status) = view.map(|v| v.run.status).filter(|s| !s.is_active()) else { return false };
        self.annotations_asked.borrow_mut().insert(key.to_string(), status) != Some(status)
    }

    fn send_detail_request(&self, deps: &AppDeps, request: DetailRequest) {
        match request {
            DetailRequest::Pr { conn_id, item, key } => self.request_pr_detail(deps, conn_id, item, key),
            DetailRequest::Wi { conn_id, item, key } => self.request_wi_detail(deps, conn_id, item, key),
            DetailRequest::Pipeline { conn_id, item, key } => {
                let annotations = self.wants_annotations(&key);
                self.request_pipeline_detail(deps, conn_id, item, key, annotations)
            }
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

    /// Whether the Pipelines list cursor is on a group header rather than a run.
    pub fn pipe_head_selected(&self) -> bool {
        self.active == 2
            && self.pipe_state.selected().is_some_and(|sel| matches!(self.pipe_lines().get(sel), Some(PipeLine::Head(_))))
    }

    /// Moves the preview's view into the screen, still drawn beside the list. A list that
    /// doesn't preview unasked builds the selected row's view here, so Enter opens the pane
    /// either way. Returns whether there was a view to focus.
    fn focus_preview(&mut self, deps: &AppDeps) -> bool {
        if self.preview.is_none() && self.pane_fits() {
            self.preview = self.build_selected_preview(deps).map(|(view, request)| Preview {
                section: self.active,
                key: request.key().to_owned(),
                view,
                request,
                sent: false,
                ticks: 0,
            });
        }
        let Some(mut p) = self.preview.take() else { return false };
        // The view is the screen before its fetch goes out, so the fetch can see what it holds.
        self.screen = std::mem::replace(&mut p.view, Screen::List);
        if !p.sent {
            self.send_detail_request(deps, p.request.clone());
        }
        self.preview_focus = true;
        self.anim = 0; // the list's cut title scrolls from its start
        // Esc from the focused pane returns to this list.
        self.lp_origin = false;
        self.from_inbox = false;
        true
    }

    /// The preview's own keys. From the list, Enter (or `p`) opens the selected item in the pane
    /// — its view becomes the screen, drawn beside the list, and the footer turns to that item's
    /// keys — whether or not the list was previewing it. Only a terminal too narrow for both
    /// halves opens the full-screen view instead. From a focused pane `p` (or Esc) hands focus
    /// back, keeping the view (tab, scroll, buffered comments) as it was for a list that
    /// previews. `P` switches the automatic preview off or on for the section.
    async fn on_preview_key(&mut self, key: Key, deps: &AppDeps) -> bool {
        let on_list = matches!(self.screen, Screen::List);
        let focused = self.preview_focus && matches!(self.screen, Screen::PrView(_) | Screen::WiView(_) | Screen::Pipeline(_));
        match key {
            // On a pipeline group header Enter keeps expanding / collapsing the group; it is a
            // run row that Enter moves into the pane.
            Key::Enter if on_list && !self.pipe_head_selected() => self.focus_preview(deps),
            Key::Char('p') if on_list => {
                if !self.focus_preview(deps) && self.active < 3 && self.content_w < PREVIEW_MIN_WIDTH {
                    self.toast = Some(format!("The pane needs a terminal at least {PREVIEW_MIN_WIDTH} columns wide."));
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
                self.toast = Some(
                    if self.preview_hidden[section] { "Preview off: Enter opens the pane. P turns it back on." } else { "Preview on." }
                        .into(),
                );
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
        // Highlight the view that matches, so the bar doesn't claim a different one.
        if let Some(i) = self.views[0].iter().position(|v| parse_pr_filter(v.filter.as_deref()) == filter) {
            self.view_idx[0] = i;
        }
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
                        ("Choose pipelines to show", c('w')),
                        ("Trigger a run", c('T')),
                        ("Cycle grouping", c('G')),
                        ("Collapse every group", c('z')),
                        ("Expand every group", c('Z')),
                    ]),
                }
                out.extend([("Sort by column", c('S')), ("Choose columns", c('c'))]);
                // On Pipelines `w` is the pipeline picker once repositories are chosen.
                if s != 2 || self.no_repos_chosen(2) {
                    out.push(("Repositories to fetch", c('w')));
                }
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
                if self.selected().is_some() && self.pane_fits() {
                    out.push(("Open in the pane", c('p')));
                }
                out.extend([
                    ("Preview while browsing on / off", c('P')),
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
                        out.extend([("Search the log", c('/')), ("Pan left", Key::Left), ("Pan right", Key::Right)]);
                        if log.query.is_some() {
                            out.extend([("Next match", c('n')), ("Previous match", c('N'))]);
                        }
                        if log.sectioned() {
                            out.extend([("Fold / unfold this step", c('z')), ("Fold / unfold every step", c('Z'))]);
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
                        if v.run.status.is_active() {
                            out.push(("Cancel the run", c('X')));
                        }
                        if v.can_respond_approvals && !v.actionable_approvals().is_empty() {
                            out.push(("Approve / reject a gate", c('A')));
                        }
                        let finished = !v.run.status.is_active();
                        if finished && v.supports_rerun {
                            out.push(("Rerun the run", c('R')));
                        }
                        if finished && v.supports_rerun_failed && v.run.status != PipelineRunStatus::Succeeded {
                            out.push(("Rerun the failed jobs", c('F')));
                        }
                        if v.run.status == PipelineRunStatus::Failed {
                            out.push(("Jump to the first error", c('E')));
                        }
                        if v.supports_artifacts {
                            out.push(("List artifacts", c('a')));
                        }
                        if !v.annotations.is_empty() {
                            out.push(("Problems", c('e')));
                        }
                        if v.run.commit_sha.as_deref().is_some_and(|s| !s.is_empty()) {
                            out.push(("Copy the commit sha", c('c')));
                        }
                        if v.history.as_ref().is_some_and(|h| h.entries.len() > 1) {
                            out.extend([("Older run", Key::Left), ("Newer run", Key::Right)]);
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
                    out.extend([
                        ("Mark read", c('x')),
                        ("Mark all read", c('A')),
                        ("Dismiss", c('d')),
                        ("Dismiss all", c('D')),
                    ]);
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
            out.extend([("Back to the list", c('p')), ("Preview while browsing on / off", c('P'))]);
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
            keys.extend([Key::Ctrl('k'), Key::Char('?'), Key::Char('B')]);
            // `F` is feedback everywhere but a pipeline run, where it reruns the failed jobs.
            if !self.f_reruns() {
                keys.push(Key::Char('F'));
            }
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
            Key::Char('d') => self.dismiss_selected_inbox(deps).await,
            Key::Char('D') => self.confirm_dismiss_all_inbox(),
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

    /// Hides the selected notification from the inbox (the `d` key). Dismissal is local — the
    /// provider's own read state is untouched — and persisted, so it survives a restart.
    async fn dismiss_selected_inbox(&mut self, deps: &AppDeps) {
        let Some(row) = self.inbox.get(self.inbox_sel) else { return };
        self.inbox_dismissed.insert(inbox_dismiss_key(row));
        self.inbox.remove(self.inbox_sel);
        self.inbox_sel = self.inbox_sel.min(self.inbox.len().saturating_sub(1));
        self.toast = Some(match self.persist_inbox_dismissed(deps).await {
            Ok(()) => "Dismissed".into(),
            Err(error) => error,
        });
    }

    /// `D` clears the whole list, so it asks first.
    fn confirm_dismiss_all_inbox(&mut self) {
        if self.inbox.is_empty() {
            return;
        }
        self.overlay = Some(Overlay::Confirm {
            title: "Dismiss all".into(),
            message: format!("Dismiss all {} notifications from the inbox?", self.inbox.len()),
            action: Action::DismissAllInbox,
        });
    }

    async fn dismiss_all_inbox(&mut self, deps: &AppDeps) {
        self.inbox_dismissed.extend(self.inbox.iter().map(inbox_dismiss_key));
        self.inbox.clear();
        self.inbox_sel = 0;
        self.toast = Some(match self.persist_inbox_dismissed(deps).await {
            Ok(()) => "All dismissed".into(),
            Err(error) => error,
        });
    }

    async fn persist_inbox_dismissed(&self, deps: &AppDeps) -> std::result::Result<(), String> {
        let mut keys: Vec<String> = self.inbox_dismissed.iter().cloned().collect();
        keys.sort();
        deps.config.set_dismissed_notifications(keys).await.map_err(|e| format!("Couldn't save: {e}"))
    }

    /// Drops dismissed notifications from `rows`.
    fn without_dismissed(&self, mut rows: Vec<NotifRow>) -> Vec<NotifRow> {
        rows.retain(|row| !self.inbox_dismissed.contains(&inbox_dismiss_key(row)));
        rows
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
            pr_pool_tx: self.job_tx.clone(),
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
        // Everything below runs one after another, and on a wide account (discovery, pipeline
        // runs per definition, a second provider) that is long enough for the PR list to sit
        // on "Loading…" with its rows already in hand.
        if let Some(tx) = &p.pr_pool_tx {
            let _ = tx.send(AppEvent::PrPoolLoaded { pool: Box::new(pr_pool.clone()), ok: prs_ok });
        }
        let (wis, wis_ok) = fetch_work_items(&deps, &mut errors).await;
        let mut pipe_catalog = HashMap::new();
        let (pipes, pipes_ok) = fetch_pipelines(&deps, &mut errors, &mut pipe_catalog).await;
        let (inbox, inbox_ok) = fetch_notifications(&deps, &mut errors).await;
        let health = deps.health.check_all().await;
        let review = derive_pool_rows(&pr_pool, PullRequestFilter::ReviewRequested, false, &HashSet::new());
        let mine = derive_pool_rows(&pr_pool, PullRequestFilter::Mine, false, &HashSet::new());
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
            pipe_catalog,
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
        self.take_pr_pool(r.pr_pool, sections_ok, deps);
        take_section(&mut self.wis, r.wis, sections_ok.wis);
        self.settle_held_edits();
        take_section(&mut self.pipes, r.pipes, sections_ok.pipes);
        if !self.held_runs.is_empty() {
            for i in 0..self.pipes.len() {
                let conn = self.pipes[i].connection_id.clone();
                let run = self.pipes[i].run.clone();
                self.pipes[i].run = self.settle_held(&conn, run);
            }
        }
        self.prune_pipe_expanded();
        if sections_ok.inbox {
            // A dismissal the feed no longer returns can never match again (new activity is a
            // new key), so forget it rather than letting the saved list grow forever. Only on a
            // complete fetch: a connection that failed returned nothing, not "none of these".
            let live: HashSet<String> = r.inbox.iter().map(inbox_dismiss_key).collect();
            self.inbox_dismissed.retain(|key| live.contains(key));
        }
        let inbox = self.without_dismissed(r.inbox);
        take_section(&mut self.inbox, inbox, sections_ok.inbox);
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
        // Merged, not replaced: a connection added while this fetch was out has its page
        // already, and this fetch's catalog was taken before it existed.
        if let Some(catalog) = r.catalog {
            self.repo_catalog.extend(catalog);
        }
        self.pipe_catalog.extend(r.pipe_catalog);
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
            let conn = match &self.screen {
                Screen::Pipeline(v) => v.connection_id.clone(),
                _ => String::new(),
            };
            let run = self.settle_held(&conn, run);
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

    /// Puts a fetched PR pool on screen: replaces the pool (under `take_section`'s rule) and
    /// re-derives every PR list and both Launchpad buckets from it. Only `ok`'s PR flags are read.
    ///
    /// Shared by the full reload and the pool the reload hands over early, so the PR list never
    /// waits on work items, pipelines, notifications or discovery to show rows it already has.
    fn take_pr_pool(&mut self, mut pool: PrPool, ok: SectionsOk, deps: &AppDeps) {
        // A reload that could not say who you are on a connection (a rate limit answering
        // `/user`, say) keeps the identity the last one established — including the one the
        // cache seeded at launch. "Mine" filtered by who you were a minute ago beats "Mine"
        // showing everyone's rows because the answer was missing this time.
        for (conn_id, me) in &self.pr_pool.me {
            if me.is_some() && pool.me.get(conn_id).is_some_and(|m| m.is_none()) {
                pool.me.insert(conn_id.clone(), me.clone());
            }
        }
        let pool_incoming = !pool.open.is_empty() || !pool.completed.is_empty();
        if ok.prs || pool_incoming {
            self.pr_pool = pool;
            self.pr_pool_loaded = true;
            self.settle_held_votes();
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
        take_section(&mut self.prs, derived_prs, ok.prs);
        take_section(&mut self.lp_prs_mine, derived_mine, ok.lp_mine);
        take_section(&mut self.lp_prs_review, derived_review, ok.lp_review);
        self.request_pr_decorations(deps);
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
            cache.put(CACHE_KEY_PR_POOL, &self.decorated_pool(), fetched_at);
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

    /// The pool with every decoration we hold folded into its rows, for the cache. A launch
    /// seeded from it then shows the +/- and state columns while their re-decoration is in
    /// flight, as the per-view entries it replaces did.
    fn decorated_pool(&self) -> PrPool {
        let mut pool = self.pr_pool.clone();
        for row in pool.open.iter_mut().chain(pool.completed.iter_mut()) {
            if let Some(d) = self.pr_decorations.get(&(row.connection_id.clone(), row.pr.id.clone())) {
                d.apply_to(&mut row.pr);
            }
        }
        pool
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
        // Comments posted from here that the provider hasn't listed yet stay on screen. After
        // the write-through: the cache holds only what the provider said.
        let mut detail = detail;
        self.settle_held_comments(&key, &mut detail.threads);
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
        // A mark stands only while the file's diff is the one that was marked: fresh files can
        // clear some, and they restore the marks a view built before any files were known lacks.
        if let Some(marks) = self.pr_viewed.get_mut(&key) {
            marks.retain(|path, mark| v.pr_files.iter().find(|f| &f.path == path).is_none_or(|f| file_fingerprint(f) == *mark));
            v.diff.viewed = still_viewed(Some(marks), &v.pr_files);
        }
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
        // The cached pool first: every view derives from it, so switching views before the first
        // fetch lands shows real rows. The per-view entry is the fallback for a cache written
        // before the pool was, which only ever holds the view that was on screen then.
        if let Some(entry) = cache.get::<PrPool>(CACHE_KEY_PR_POOL) {
            self.pr_pool = entry.value;
            self.pr_pool_loaded = true;
            self.prs = self.derive_pr_rows(self.pr_filter, self.pr_wants_completed());
            oldest = Some(entry.fetched_at);
        } else {
            let prs_key = prs_cache_key(self.pr_filter, self.pr_wants_completed());
            seed_section(cache, &prs_key, &mut self.prs, &mut oldest);
        }
        seed_section(cache, CACHE_KEY_WORK_ITEMS, &mut self.wis, &mut oldest);
        seed_section(cache, CACHE_KEY_PIPELINES, &mut self.pipes, &mut oldest);
        seed_section(cache, CACHE_KEY_INBOX, &mut self.inbox, &mut oldest);
        let dismissed = &self.inbox_dismissed;
        self.inbox.retain(|row| !dismissed.contains(&inbox_dismiss_key(row)));
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
        self.prs_landed = false;
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
        let mut detail = detail;
        self.settle_held_comments(&key, &mut detail.threads);
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
        let PipelineDetailFetch {
            run,
            approvals,
            supports_approvals,
            can_respond_approvals,
            annotations,
            supports_rerun,
            supports_rerun_failed,
            supports_artifacts,
            rerun_new_run,
            rerun_failed_new_run,
        } = fetch;
        if run.is_none()
            && approvals.is_none()
            && supports_approvals.is_none()
            && can_respond_approvals.is_none()
            && annotations.is_none()
            && supports_rerun.is_none()
            && supports_rerun_failed.is_none()
            && supports_artifacts.is_none()
            && rerun_new_run.is_none()
            && rerun_failed_new_run.is_none()
        {
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
        // Unlike gates, problems and capabilities drive no write of their own, so the cache's
        // answer may stand in for a call that failed.
        let flag = |fresh: Option<bool>, cached: fn(&PipelineDetail) -> bool| {
            fresh.unwrap_or_else(|| known.as_ref().is_some_and(cached))
        };
        let detail = PipelineDetail {
            run: resolved_run,
            approvals: resolved_approvals,
            supports_approvals: resolved_supports,
            can_respond_approvals: resolved_can_respond,
            annotations: annotations
                .clone()
                .or_else(|| self.annotations_by_key.get(&key).cloned())
                .unwrap_or_else(|| known.as_ref().map(|k| k.annotations.clone()).unwrap_or_default()),
            supports_rerun: flag(supports_rerun, |k| k.supports_rerun),
            supports_rerun_failed: flag(supports_rerun_failed, |k| k.supports_rerun_failed),
            supports_artifacts: flag(supports_artifacts, |k| k.supports_artifacts),
            rerun_new_run: flag(rerun_new_run, |k| k.rerun_new_run),
            rerun_failed_new_run: flag(rerun_failed_new_run, |k| k.rerun_failed_new_run),
        };
        // See `apply_pr_detail`: a newer entry already won, so the screen must not go back.
        if deps.cache.put(&key, &detail, fetched_at) == CachePut::Stale {
            return;
        }

        let conn = match &self.screen {
            Screen::Pipeline(v) if pipeline_detail_cache_key(&v.connection_id, &v.run.item_ref()) == key => v.connection_id.clone(),
            _ => return,
        };
        let mut detail = detail;
        detail.run = self.settle_held(&conn, detail.run);
        let Screen::Pipeline(v) = &mut self.screen else { return };
        // `approvals`, never `detail.approvals`: a gate the cache remembers may already have been
        // decided, and `actionable_approvals` turns whatever is on screen into a real
        // approve/reject against the provider. A failed `pending_approvals` therefore leaves the
        // view's gates alone rather than substituting the cached ones. This has been
        // reintroduced once already: the merged value is for the cache, never for the view.
        if run_confirmed {
            if v.stale || !pipeline_detail_unchanged(v, &detail, approvals.as_deref()) {
                v.supports_approvals = detail.supports_approvals;
                v.can_respond_approvals = detail.can_respond_approvals;
                v.apply_extras(&detail);
                v.apply_fresh_run(detail.run, approvals);
            }
        } else if !pipeline_detail_unchanged(v, &detail, approvals.as_deref()) {
            v.apply_extras(&detail);
            // The run itself wasn't reconfirmed this round (only approvals/capabilities
            // answered) — patch those in place, but leave the run and `stale` exactly as they
            // are: only a real `get_run` may confirm the run is actually live.
            v.apply_confirmed_approvals(approvals);
            v.supports_approvals = detail.supports_approvals;
            v.can_respond_approvals = detail.can_respond_approvals;
            // A gate arriving or clearing adds or removes the gate stage's rows.
            v.clamp_selection();
        }
    }

    /// Kicks off the background fetch for a just-opened pipeline view's detail, without blocking
    /// the render loop. Mirrors [`App::request_pr_detail`].
    ///
    /// `annotations` asks for the run's problems too — see [`App::wants_annotations`].
    fn request_pipeline_detail(&self, deps: &AppDeps, conn_id: String, run: ItemRef, key: String, annotations: bool) {
        let Some(tx) = self.job_tx.clone() else {
            return;
        };
        let deps = deps.clone();
        let fetched_at = Utc::now();
        if annotations {
            // Its own task and event: a slow annotations call never holds up the run.
            let (deps, tx, conn_id, run, key) = (deps.clone(), tx.clone(), conn_id.clone(), run.clone(), key.clone());
            tokio::spawn(async move {
                let annotations = match pipeline_source(&deps, &conn_id).await {
                    Some(source) => detail_or_none(source.annotations(&run).await, DIAG_PIPELINE_ANNOTATIONS),
                    None => None,
                };
                let _ = tx.send(AppEvent::PipelineAnnotationsLoaded { key, annotations });
            });
        }
        tokio::spawn(async move {
            let detail = fetch_pipeline_detail(&deps, &conn_id, &run).await;
            let _ = tx.send(AppEvent::PipelineDetailLoaded { key, detail: Box::new(detail), fetched_at });
        });
    }

    /// A finished run's problems landed: remembered for the session, written into the cached
    /// detail, and shown if the run is still on screen (or in the preview).
    fn apply_pipeline_annotations(&mut self, deps: &AppDeps, key: String, annotations: Option<Vec<PipelineAnnotation>>) {
        let Some(annotations) = annotations else {
            // Unanswered: may be asked for again.
            self.annotations_asked.borrow_mut().remove(&key);
            return;
        };
        self.annotations_by_key.insert(key.clone(), annotations.clone());
        let cached = annotations.clone();
        deps.cache.rewrite::<PipelineDetail>(&key, move |mut d| {
            d.annotations = cached;
            d
        });
        self.with_preview_screen(&key.clone(), move |app| {
            let Screen::Pipeline(v) = &mut app.screen else { return };
            if pipeline_detail_cache_key(&v.connection_id, &v.run.item_ref()) != key || v.run.status.is_active() {
                return;
            }
            v.annotations = annotations;
            if v.problem_sel >= v.annotations.len() {
                v.problem_sel = v.annotations.len().saturating_sub(1);
            }
            if v.annotations.is_empty() {
                v.problem_focus = false;
            }
        });
    }

    /// The rows one PR view shows, derived from the pool and decorated from what we hold.
    fn derive_pr_rows(&self, filter: PullRequestFilter, completed: bool) -> Vec<PrRow> {
        let mut rows = derive_pool_rows(&self.pr_pool, filter, completed, &self.review_cleared_keys());
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
        // Before the first fetch lands the pool is empty, so every view would derive to nothing.
        // Switching views then is exactly when the cache is the better answer: it holds each
        // view's last rows under its own key, the same ones `seed_from_cache` paints at launch.
        if !self.pr_pool_loaded {
            if let Some(entry) = deps.cache.get::<Vec<PrRow>>(&prs_cache_key(self.pr_filter, self.pr_wants_completed())) {
                self.prs = entry.value;
            }
        }
        // Until a fetch has actually landed there is nothing to derive the Launchpad from, and
        // deriving anyway would blank the rows `seed_from_cache` painted — the blank landing
        // screen the seeding exists to prevent. The list above shows the new view's own cached
        // rows, or nothing: an empty list under the new heading is honest where carrying the
        // old view's rows is not.
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
                    let discovered = feed.source.discover().await;
                    if let Ok(defs) = &discovered {
                        self.pipe_catalog.insert(conn_id.clone(), defs.clone());
                    }
                    let defs = detail_or_default(discovered, DIAG_PIPELINE_DISCOVERY);
                    let def_names: HashMap<String, String> =
                        defs.iter().map(|d| (d.id.clone(), d.name.clone())).collect();
                    let me = detail_or_default(feed.source.current_user().await, DIAG_PIPELINE_CURRENT_USER);
                    for result in feed.list_runs(&defs).await {
                        match result {
                            Ok(runs) => {
                                let supports = feed.source.supports_approvals();
                                for run in runs {
                                    // Only in-flight runs can be waiting on a gate — bound the
                                    // extra per-run approval calls to those.
                                    let pending = if supports && run.status.is_active() {
                                        detail_or_default(
                                            feed.source.pending_approvals(&run.item_ref()).await,
                                            DIAG_PIPELINE_APPROVALS,
                                        )
                                    } else {
                                        Vec::new()
                                    };
                                    let awaiting_approval = pending.iter().any(|approval| approval.can_respond);
                                    let gates = pending.into_iter().filter(|a| a.blocks_run).map(|approval| approval.name).collect();
                                    let definition_name = def_names.get(&run.definition_id).cloned();
                                    let triggered_by_me = run_triggered_by(&run, me.as_deref());
                                    rows.push(PipeRow {
                                        connection_id: conn_id.clone(),
                                        connection: name.clone(),
                                        provider,
                                        run,
                                        definition_name,
                                        awaiting_approval,
                                        triggered_by_me,
                                        gates,
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
        self.refresh_pipeline_context();
        self.request_estimates(deps);
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

    /// One wheel notch (`dir` -1 up, 1 down) over `hit`. Where a screen has two panes that take
    /// keys — the Command Center columns, the pipeline tree and log pane, the diff's file list
    /// and patch — the one under the pointer takes the wheel; elsewhere it goes wherever the
    /// keys do (over an unfocused preview, that is the list beside it). Lists
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
                    self.anim = 0; // restart the title scroll on the newly-selected row
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
                // Over the file list while the patch has the keys, the wheel changes file.
                if v.tab == 3 && v.diff.focus == DiffFocus::Patch && matches!(hit, Some(Hit::DiffFile(_))) {
                    v.diff.exit_patch();
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
        // Inside a pipeline run `F` reruns its failed jobs instead (see `on_pipeline_screen_key`).
        if key == Key::Char('F') && !self.f_reruns() {
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

        // Tab walks the tab strip — Command Center, then each visible section — from any
        // screen, an open PR / work item / pipeline run included; Shift-Tab walks it backwards.
        // Both wrap. A PR open in the pane beside its list is the exception: there Tab walks the
        // PR's own tabs, and Esc closes the pane to hand Tab back to the strip. The
        // input-capturing modes (wizard, overlay, quick filter) return above, so Tab still
        // reaches them.
        if matches!(key, Key::Tab | Key::BackTab) && self.preview_focus {
            if let Screen::PrView(v) = &mut self.screen {
                v.step_tab(if key == Key::Tab { 1 } else { -1 });
                return;
            }
        }
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
                if !self.on_pipeline_deps_key(key, deps) {
                    self.on_pipeline_screen_key(key);
                }
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
        let (view, fetch) = Self::build_pr_view(deps, &self.pr_viewed, tab, label, url, conn_id, pr);
        self.screen = view;
        // The view is on screen now; the fetch that keeps it fresh runs in the background and
        // patches it in place via `AppEvent::PrDetailLoaded` (see `apply_pr_detail`).
        self.send_detail_request(deps, fetch);
    }

    /// Copies the open PR's "viewed" marks into [`App::pr_viewed`], each pinned to the diff it
    /// was marked against — the whole-PR file where there is one, so a mark made while drilled
    /// into a commit still holds once back on the full diff.
    fn record_viewed(&mut self) {
        let Screen::PrView(v) = &self.screen else { return };
        let key = pr_detail_cache_key(&v.connection_id, &v.pr.item_ref());
        let marks: HashMap<String, u64> = v
            .diff
            .viewed
            .iter()
            .filter_map(|path| {
                let file = v.pr_files.iter().chain(&v.diff.files).find(|f| &f.path == path)?;
                Some((path.clone(), file_fingerprint(file)))
            })
            .collect();
        if marks.is_empty() {
            self.pr_viewed.remove(&key);
        } else {
            self.pr_viewed.insert(key, marks);
        }
    }

    /// Builds the PR view from the row plus whatever the cache holds, without any I/O. The
    /// returned request is what keeps it fresh; [`open_pr_view_for`] sends it at once, the
    /// preview pane only once the cursor has settled.
    #[allow(clippy::too_many_arguments)]
    fn build_pr_view(
        deps: &AppDeps,
        pr_viewed: &HashMap<String, HashMap<String, u64>>,
        tab: usize,
        label: String,
        url: Option<String>,
        conn_id: String,
        pr: PullRequest,
    ) -> (Screen, DetailRequest) {
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
            viewed: still_viewed(pr_viewed.get(&key), &files),
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
        // A comment still on its way to the provider has no thread there to reply to yet.
        let target = target.and_then(|id| if is_local(&id) { Err("That comment is still being posted — reply in a moment") } else { Ok(id) });
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

    /// Submits the buffered line comments as one review with `event`. Optimistic, like
    /// [`App::execute_pr_action`]: the comments leave the buffer and show on the diff at once.
    async fn submit_review(&mut self, event: ReviewVote, deps: &AppDeps) {
        let (item, comments, conn_id) = match &self.screen {
            Screen::PrView(v) => (v.pr.item_ref(), v.pending.clone(), v.connection_id.clone()),
            _ => return,
        };
        if comments.is_empty() {
            return;
        }
        self.send_pr_call(conn_id, item, PrCall::Review { event, comments }, deps).await;
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
        self.pipe_scope = self.pipe_scope_of(deps);
    }

    /// "N of M" over the pipeline connections whose discovery has answered. Subscribing to
    /// everything counts every definition; nothing selected counts none (pipelines are opt-in);
    /// an explicit list counts the ids discovery still knows, so a deleted pipeline isn't
    /// counted as fetched.
    fn pipe_scope_of(&self, deps: &AppDeps) -> Option<PipeScope> {
        let cfg = deps.config.snapshot();
        let subs = cfg.pipelines.as_ref().map(|p| p.subscriptions.clone()).unwrap_or_default();
        let (mut selected, mut available) = (0, 0);
        let mut known = false;
        for sub in &subs {
            let Some(defs) = self.pipe_catalog.get(&sub.connection_id) else { continue };
            known = true;
            available += defs.len();
            selected += if sub.auto_discover_all {
                defs.len()
            } else {
                defs.iter().filter(|d| sub.definition_ids.contains(&d.id)).count()
            };
        }
        (known && available > 0).then(|| PipeScope {
            connections: subs.iter().map(|s| s.connection_id.clone()).collect(),
            selected,
            available,
        })
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
        match key {
            Key::Left | Key::Char('h') => v.step_tab(-1),
            Key::Right | Key::Char('l') => v.step_tab(1),
            // Enter on a file drops into a line cursor within its patch.
            Key::Enter if v.tab == 3 => v.diff.enter_patch(),
            // Diff-tab review ergonomics: mark viewed, jump between threads.
            Key::Char('v') if v.tab == 3 => {
                v.diff.toggle_viewed();
                self.record_viewed();
            }
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
            // `c` = which **c**olumns the list table draws.
            'c' => self.open_columns_toggle(),
            'f' if self.active == 0 => self.open_pr_status_toggle(),
            'f' if self.active == 1 => self.open_wi_states_toggle(),
            // Pipeline trigger (Pipelines tab).
            'T' if self.active == 2 => self.open_pipeline_trigger(),
            // `w` = which pipelines you **w**atch. On this section it replaces the repository
            // scope: Azure groups pipelines by project and folder, not by repository, so
            // "which pipelines" is the question the user is actually answering. (`p` is taken by
            // the preview focus.)
            // Until repositories are chosen there are no pipelines to pick from — discovery fans
            // out over the same scope — so `w` opens the repository picker instead.
            'w' if self.active == 2 && !self.no_repos_chosen(2) => self.open_pipeline_subs_from_list(deps).await,
            // `G` cycles how the Pipelines list is **G**rouped.
            'G' if self.active == 2 => self.cycle_pipe_group(deps).await,
            ' ' if self.active == 2 => self.enter_pipeline_line(deps),
            'z' if self.active == 2 => self.set_all_pipe_groups(false),
            'Z' if self.active == 2 => self.set_all_pipe_groups(true),
            // `w` = which repositories this section **w**atches — the same key Pipelines uses for
            // which pipelines it watches. Unlike the `f` filter, this gates what is *fetched*,
            // not what is shown from what was fetched.
            'w' => self.open_repo_scope(deps).await,
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
        let cached = deps.cache.get::<PipelineDetail>(&key).map(|entry| entry.value);
        let run = cached.as_ref().map_or(fallback, |d| d.run.clone());

        let mut view = PipelineView::new(title, run, conn_id.clone(), provider, definition_id, branch);
        if let Some(d) = &cached {
            view.supports_approvals = d.supports_approvals;
            view.can_respond_approvals = d.can_respond_approvals;
            // Gates never seed (see above); problems and capabilities drive no write of their own.
            view.apply_extras(d);
        }
        view.stale = true;
        view.auto_select();
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
        let annotations = match &self.screen {
            Screen::Pipeline(v) => v.annotations.clone(),
            _ => known.as_ref().map(|k| k.annotations.clone()).unwrap_or_default(),
        };
        let detail = PipelineDetail {
            run: run.clone(),
            approvals: approvals
                .clone()
                .or_else(|| known.map(|k| k.approvals))
                .unwrap_or_default(),
            supports_approvals,
            can_respond_approvals,
            annotations,
            supports_rerun: feed.source.supports_rerun(),
            supports_rerun_failed: feed.source.supports_rerun_failed(),
            supports_artifacts: feed.source.supports_artifacts(),
            rerun_new_run: feed.source.rerun_starts_new_run(false),
            rerun_failed_new_run: feed.source.rerun_starts_new_run(true),
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
        let mut left = None;
        if let Screen::Pipeline(v) = &mut self.screen {
            if v.connection_id == conn_id && v.run.id == run.id {
                v.approvals.retain(|a| a.id != approval_id);
                // The gate's rows leave the tree with it.
                v.clamp_selection();
                let holding = v.approvals.iter().filter(|a| a.blocks_run).map(|a| a.name.clone()).collect::<Vec<_>>();
                left = Some((v.approvals.iter().any(|a| a.can_respond), holding));
            }
        }
        // No open view means no local knowledge of what is left on the run, so the row keeps its
        // badge until the reload says otherwise: a badge that lingers a second is recoverable, a
        // missing one hides a gate that still wants the user.
        let Some((still_gated, gates)) = left else { return };
        for row in self.pipes.iter_mut().filter(|r| r.connection_id == conn_id && r.run.id == run.id) {
            if !still_gated {
                row.awaiting_approval = false;
            }
            row.gates = gates.clone();
        }
        // `awaiting_approval` is what puts a run in the Command Center's Approvals bucket.
        self.rebuild_launchpad();
    }

    // ---- run pane: history, estimates, problems, rerun / cancel, artifacts, copy ----

    /// Whether `F` reruns failed jobs here rather than opening the feedback form: only on a
    /// finished run whose provider can rerun just the failed jobs.
    fn f_reruns(&self) -> bool {
        matches!(&self.screen, Screen::Pipeline(v) if v.supports_rerun_failed && !v.run.status.is_active())
    }

    /// Recent runs of the view's pipeline on its branch, from the list rows (no fetch).
    fn pipeline_history(&self, v: &PipelineView) -> Option<RunHistory> {
        let branch = v.run.branch.as_deref().or(v.branch.as_deref());
        let same_pipeline = |p: &PipeRow| {
            p.connection_id == v.connection_id && p.run.definition_id == v.run.definition_id && p.run.repository == v.run.repository
        };
        let distinct = |rows: &[usize]| rows.iter().map(|&i| self.pipes[i].run.id.as_str()).collect::<HashSet<_>>().len();
        let mut rows: Vec<usize> =
            (0..self.pipes.len()).filter(|&i| same_pipeline(&self.pipes[i]) && self.pipes[i].run.branch.as_deref() == branch).collect();
        // A branch with a single run — a tag, typically — says nothing on its own: show the
        // pipeline's runs on every branch instead.
        let mut all_branches = false;
        if distinct(&rows) < 2 {
            let all: Vec<usize> = (0..self.pipes.len()).filter(|&i| same_pipeline(&self.pipes[i])).collect();
            if distinct(&all) > distinct(&rows) {
                rows = all;
                all_branches = true;
            }
        }
        if rows.is_empty() {
            return None;
        }
        rows.sort_by(|&a, &b| {
            let (x, y) = (&self.pipes[a].run, &self.pipes[b].run);
            x.started_at.cmp(&y.started_at).then(x.number.cmp(&y.number))
        });
        // One feed per subscription can list a run twice; the strip shows it once.
        let mut seen = HashSet::new();
        rows.retain(|&i| seen.insert(self.pipes[i].run.id.clone()));

        let now = Utc::now();
        // The open run is the freshest copy of itself — the row may be a refresh behind.
        let run_of = |i: usize| if self.pipes[i].run.id == v.run.id { &v.run } else { &self.pipes[i].run };
        let pos = rows.iter().position(|&i| self.pipes[i].run.id == v.run.id);
        let start = rows.len().saturating_sub(HISTORY_LEN).min(pos.unwrap_or(usize::MAX));
        let window = &rows[start..(start + HISTORY_LEN).min(rows.len())];
        let entries = window
            .iter()
            .map(|&i| {
                let run = run_of(i);
                let status = if run.id == v.run.id { v.shown_status() } else { self.pipes[i].shown_status() };
                HistEntry {
                    run_id: run.id.clone(),
                    number: run.number,
                    status,
                    secs: run_secs(run, now),
                    started_at: run.started_at,
                    row: i,
                }
            })
            .collect();
        // This run is measured against the others, never against itself.
        let succeeded: Vec<i64> = rows
            .iter()
            .map(|&i| run_of(i))
            .filter(|r| r.status == PipelineRunStatus::Succeeded && r.id != v.run.id)
            .filter_map(|r| run_secs(r, now))
            .collect();
        let last_failure = rows.iter().rev().map(|&i| &self.pipes[i]).find(|p| p.run.status == PipelineRunStatus::Failed && p.run.id != v.run.id).map(|p| {
            let key = pipeline_detail_cache_key(&p.connection_id, &p.run.item_ref());
            let run = self.run_details.get(&key).unwrap_or(&p.run);
            let step = first_failed_node(run).and_then(|(si, ji, k)| {
                let job = run.stages.get(si)?.jobs.get(ji)?;
                Some(k.and_then(|k| job.steps.get(k)).map_or_else(|| job.name.clone(), |s| s.name.clone()))
            });
            LastFailure { number: p.run.number, at: p.run.finished_at.or(p.run.started_at), step }
        });
        Some(RunHistory {
            name: pipe_definition_name(&self.pipes[rows[0]]),
            entries,
            current: pos.map(|p| p - start),
            median_secs: runlog::median(&succeeded),
            median_n: succeeded.len(),
            total: rows.len(),
            all_branches,
            last_failure,
        })
    }

    /// Recent succeeded runs of the view's pipeline (any branch) whose job times can feed an
    /// estimate, newest first: `(detail key, row)`.
    fn estimate_candidates(&self, v: &PipelineView) -> Vec<(String, usize)> {
        let mut rows: Vec<usize> = (0..self.pipes.len())
            .filter(|&i| {
                let p = &self.pipes[i];
                p.connection_id == v.connection_id
                    && p.run.definition_id == v.run.definition_id
                    && p.run.repository == v.run.repository
                    && p.run.status == PipelineRunStatus::Succeeded
                    && p.run.id != v.run.id
            })
            .collect();
        rows.sort_by_key(|&i| Reverse(self.pipes[i].run.started_at));
        let mut seen = HashSet::new();
        rows.retain(|&i| seen.insert(self.pipes[i].run.id.clone()));
        rows.into_iter()
            .take(ESTIMATE_RUNS)
            .map(|i| (pipeline_detail_cache_key(&self.pipes[i].connection_id, &self.pipes[i].run.item_ref()), i))
            .collect()
    }

    /// How long the view's in-flight run and its jobs should take.
    fn pipeline_estimates(&self, v: &PipelineView, history: Option<&RunHistory>) -> Estimates {
        let mut e = Estimates::default();
        if let Some(h) = history.filter(|h| h.median_n > 0) {
            e.run_median = h.median_secs;
            e.run_n = h.median_n;
        } else {
            // No succeeded run on this branch yet: the pipeline's runs anywhere.
            let now = Utc::now();
            let secs: Vec<i64> = self
                .pipes
                .iter()
                .filter(|p| {
                    p.connection_id == v.connection_id
                        && p.run.definition_id == v.run.definition_id
                        && p.run.repository == v.run.repository
                        && p.run.status == PipelineRunStatus::Succeeded
                        && p.run.id != v.run.id
                })
                .filter_map(|p| run_secs(&p.run, now))
                .collect();
            e.run_median = runlog::median(&secs);
            e.run_n = secs.len();
        }
        let mut per_job: HashMap<String, Vec<i64>> = HashMap::new();
        for (key, row) in self.estimate_candidates(v) {
            let listed = &self.pipes[row].run;
            let Some(run) = self.run_details.get(&key).or_else(|| has_timed_jobs(listed).then_some(listed)) else { continue };
            e.job_runs += 1;
            for job in run.stages.iter().flat_map(|s| &s.jobs) {
                if let (Some(a), Some(b)) = (job.started_at, job.finished_at) {
                    per_job.entry(job.name.clone()).or_default().push((b - a).num_seconds().max(0));
                }
            }
        }
        e.jobs = per_job.into_iter().filter_map(|(name, secs)| runlog::median(&secs).map(|m| (name, m))).collect();
        e
    }

    /// Rebuilds the history strip and estimates of the pipeline view on screen and of the one in
    /// the preview, from the list rows and the session's run details. No I/O.
    pub(crate) fn refresh_pipeline_context(&mut self) {
        let context = |app: &App, screen: &Screen| match screen {
            Screen::Pipeline(v) => {
                let history = app.pipeline_history(v);
                let estimates =
                    if v.run.status.is_active() { app.pipeline_estimates(v, history.as_ref()) } else { Estimates::default() };
                Some((history, estimates))
            }
            _ => None,
        };
        if let Some((history, estimates)) = context(self, &self.screen) {
            if let Screen::Pipeline(v) = &mut self.screen {
                v.history = history;
                v.estimates = estimates;
            }
        }
        let preview = self.preview.as_ref().and_then(|p| context(self, &p.view));
        if let (Some((history, estimates)), Some(p)) = (preview, self.preview.as_mut()) {
            if let Screen::Pipeline(v) = &mut p.view {
                v.history = history;
                v.estimates = estimates;
            }
        }
    }

    /// Asks, once per session, for the details of the recent succeeded runs an in-flight run's
    /// job estimates are read from — through the ordinary detail fetch, in the background.
    fn request_estimates(&mut self, deps: &AppDeps) {
        if self.job_tx.is_none() {
            return;
        }
        let mut wanted = Vec::new();
        // An unfocused preview waits for its own detail fetch to go out — the cursor has
        // settled — so holding ↓ through the list doesn't fetch estimates for every run passed.
        let preview = self.preview.as_ref().filter(|p| p.sent).map(|p| &p.view);
        for screen in [Some(&self.screen), preview].into_iter().flatten() {
            let Screen::Pipeline(v) = screen else { continue };
            // A run parked on a gate has nothing running to estimate.
            if !v.run.status.is_active() || v.shown_status() == PipelineRunStatus::Waiting {
                continue;
            }
            for (key, row) in self.estimate_candidates(v) {
                let listed = &self.pipes[row];
                if self.run_details.contains_key(&key) || self.estimate_requested.contains(&key) || has_timed_jobs(&listed.run) {
                    continue;
                }
                wanted.push((listed.connection_id.clone(), listed.run.item_ref(), key));
            }
        }
        for (conn_id, item, key) in wanted {
            if self.estimate_requested.insert(key.clone()) {
                // Estimates read job times only; they never ask for problems.
                self.request_pipeline_detail(deps, conn_id, item, key, false);
            }
        }
    }

    /// Keys on the drill-in that need `deps`: `←`/`→` through the history, `a` artifacts.
    /// Returns false for anything else, or while the logs, or the artifacts list, hold the keys.
    fn on_pipeline_deps_key(&mut self, key: Key, deps: &AppDeps) -> bool {
        let Screen::Pipeline(v) = &self.screen else { return false };
        if v.artifacts.is_some() || v.logs_have_keys() {
            return false;
        }
        match key {
            Key::Left => self.step_pipeline_history(-1, deps),
            Key::Right => self.step_pipeline_history(1, deps),
            Key::Char('a') => self.open_artifacts(deps),
            _ => return false,
        }
        true
    }

    /// `←` / `→`: opens the older / newer run from the history strip in place, and moves the
    /// list selection to it, so `p` returns to that run.
    fn step_pipeline_history(&mut self, delta: isize, deps: &AppDeps) {
        let Screen::Pipeline(v) = &self.screen else { return };
        let target = v.history.as_ref().and_then(|h| {
            let at = h.current? as isize + delta;
            usize::try_from(at).ok().and_then(|at| h.entries.get(at)).map(|e| e.row)
        });
        let Some(row) = target.filter(|&r| r < self.pipes.len()) else {
            self.toast = Some(
                if delta < 0 { "No older run of this pipeline on this branch" } else { "This is the newest run on this branch" }.into(),
            );
            return;
        };
        let pipe = &self.pipes[row];
        let (view, request) = Self::build_pipeline_view(
            deps,
            pipe.connection_id.clone(),
            pipe.provider,
            pipe.run.id.clone(),
            pipe.run.definition_id.clone(),
            pipe.run.branch.clone(),
            pipe_label(pipe),
            pipe.run.clone(),
        );
        self.screen = view;
        self.send_detail_request(deps, request);
        self.select_pipe_row(row);
        self.refresh_pipeline_context();
    }

    /// Puts the Pipelines list cursor on the run at `row`, opening its group if it is folded.
    fn select_pipe_row(&mut self, row: usize) {
        if self.active != 2 {
            return;
        }
        let find = |app: &App| app.pipe_lines().iter().position(|l| matches!(l, PipeLine::Run(i) if *i == row));
        let mut pos = find(self);
        if pos.is_none() && self.pipe_group != PipeGroup::Off {
            if let Some(p) = self.pipes.get(row) {
                let key = self.pipe_group_key(p);
                if self.pipe_expanded.insert(key) {
                    pos = find(self);
                }
            }
        }
        if let Some(pos) = pos {
            self.pipe_state.select(Some(pos));
            self.ensure_visible();
        }
    }

    /// `e`: moves the keys into the Problems panel and back.
    fn toggle_problem_focus(&mut self) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        if v.annotations.is_empty() {
            self.toast = Some("No problems reported for this run".into());
            return;
        }
        v.problem_focus = !v.problem_focus;
        v.problem_sel = v.problem_sel.min(v.annotations.len() - 1);
    }

    /// Keys while the Problems panel has focus. False for keys it leaves to the tree.
    fn on_problems_key(&mut self, key: Key) -> bool {
        let Screen::Pipeline(v) = &mut self.screen else { return false };
        let n = v.annotations.len();
        match key {
            Key::Up | Key::Char('k') => v.problem_sel = v.problem_sel.saturating_sub(1),
            Key::Down | Key::Char('j') => {
                if v.problem_sel + 1 < n {
                    v.problem_sel += 1;
                }
            }
            Key::Escape | Key::Char('e') => v.problem_focus = false,
            Key::Enter => self.open_problem(),
            _ => return false,
        }
        true
    }

    /// ↵ on a problem: opens its job's log at the line it names (`path:line`, else its message).
    fn open_problem(&mut self) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        let Some(a) = v.annotations.get(v.problem_sel).cloned() else { return };
        let known = |id: &String| v.run.stages.iter().flat_map(|s| &s.jobs).any(|j| &j.id == id);
        let job_id = a
            .job_id
            .clone()
            .filter(known)
            .or_else(|| v.failed_target().map(|t| t.0))
            .or_else(|| v.run.stages.iter().flat_map(|s| &s.jobs).next().map(|j| j.id.clone()));
        let Some(job_id) = job_id else {
            self.toast = Some("No job to open for this problem".into());
            return;
        };
        let mut needles = Vec::new();
        match (&a.path, a.line) {
            (Some(path), Some(line)) => {
                needles.push(format!("{path}:{line}"));
                needles.push(format!("{}:{line}", runlog::short_location(path)));
            }
            (Some(path), None) => needles.push(path.clone()),
            _ => {}
        }
        needles.push(a.message.lines().next().unwrap_or("").trim().to_string());
        v.problem_focus = false;
        if let Some(i) = v.flatten().iter().position(|n| n.job_id.as_deref() == Some(job_id.as_str())) {
            v.selected = i;
        }
        v.open_logs_for_job(&job_id, None).request_jump(LogJump::Find(needles));
        v.log_focus = true;
    }

    /// `E` from the tree (the failure line's hint): the failed job's log at its first error.
    fn open_first_error(&mut self) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        let Some((si, ji, step)) = first_failed_node(&v.run) else {
            self.toast = Some("Nothing failed in this run".into());
            return;
        };
        let Some(job_id) = v.run.stages.get(si).and_then(|s| s.jobs.get(ji)).map(|j| j.id.clone()) else { return };
        v.selected = v.node_index(si, ji, step);
        v.user_moved = true;
        v.open_logs_for_job(&job_id, step).request_jump(LogJump::FirstError);
        v.log_focus = true;
    }

    /// ↵ on a step: its job's log, scrolled to that step's section.
    fn open_step_logs(&mut self) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        let Some(node) = v.selected_node() else { return };
        let (Some(job_id), Some(step)) = (node.job_id, node.step) else { return };
        let reuse = v.logs.as_ref().is_some_and(|l| l.job_id == job_id);
        let log = v.open_logs_for_job(&job_id, Some(step));
        if reuse {
            log.focus_step(step);
        }
        v.log_focus = true;
    }

    /// `R` / `F`: confirms re-running the open run (all of it, or its failed jobs).
    fn confirm_rerun(&mut self, failed_only: bool) {
        let Screen::Pipeline(v) = &self.screen else { return };
        if v.run.status.is_active() {
            self.toast = Some(format!("{} is still running — C cancels it", v.title));
            return;
        }
        let supported = if failed_only { v.supports_rerun_failed } else { v.supports_rerun };
        if !supported {
            let what = if failed_only { "Rerunning failed jobs" } else { "Rerunning a run" };
            self.toast = Some(format!("{what} isn't supported for {}", v.provider.as_str()));
            return;
        }
        let failed_jobs: Vec<&str> = v
            .run
            .stages
            .iter()
            .flat_map(|s| &s.jobs)
            .filter(|j| matches!(j.status, PipelineRunStatus::Failed | PipelineRunStatus::Canceled))
            .map(|j| j.name.as_str())
            .collect();
        if failed_only && failed_jobs.is_empty() && v.run.status == PipelineRunStatus::Succeeded {
            self.toast = Some("Nothing failed in this run — R reruns all of it".into());
            return;
        }
        let mut message = if failed_only {
            format!("Rerun the failed {} of {}?", if failed_jobs.len() == 1 { "job" } else { "jobs" }, v.title)
        } else {
            format!("Rerun every job of {}?", v.title)
        };
        if failed_only && !failed_jobs.is_empty() {
            message.push('\n');
            for name in failed_jobs.iter().take(5) {
                message.push_str(&format!("\n  ✗ {name}"));
            }
            if failed_jobs.len() > 5 {
                message.push_str(&format!("\n  … and {} more", failed_jobs.len() - 5));
            }
        }
        let new_run = if failed_only { v.rerun_failed_new_run } else { v.rerun_new_run };
        let mut same = Vec::new();
        if new_run {
            // A provider that starts a separate run: there's no attempt to count up.
            same.push(match v.run.branch.as_deref().or(v.branch.as_deref()).filter(|b| !b.is_empty()) {
                Some(b) => format!("Starts a new run on {b}"),
                None => "Starts a new run".to_string(),
            });
        } else {
            if let Some(sha) = v.run.commit_sha.as_deref().filter(|s| !s.is_empty()) {
                same.push(format!("Same commit {}", sha.chars().take(7).collect::<String>()));
            }
            if let Some(n) = v.run.attempt {
                same.push(format!("becomes attempt {}", n + 1));
            }
        }
        if !same.is_empty() {
            message.push_str("\n\n");
            message.push_str(&same.join(" · "));
        }
        let action = Action::PipelineRerun {
            connection_id: v.connection_id.clone(),
            repo: v.run.repository.clone(),
            run_id: v.run.id.clone(),
            failed_only,
            new_run,
            label: v.title.clone(),
        };
        let title = if failed_only { "Rerun failed jobs" } else { "Rerun run" };
        self.overlay = Some(Overlay::Confirm { title: title.into(), message, action });
    }

    /// Runs a confirmed rerun or cancel. Optimistic: the pane and the list show the outcome
    /// straight away, the provider is asked in the background (its answer comes back as
    /// [`AppEvent::PipelineRunActionDone`]), and a refusal puts the run back as it was. A rerun
    /// that starts a *new* run leaves the open one alone and follows the new one once it lists.
    async fn execute_pipeline_run_action(&mut self, action: Action, deps: &AppDeps) {
        let (conn_id, repo, run_id, label, rerun, new_run) = match action {
            Action::PipelineRerun { connection_id, repo, run_id, failed_only, new_run, label } => {
                (connection_id, repo, run_id, label, Some(failed_only), new_run)
            }
            Action::PipelineCancel { connection_id, run, label } => (connection_id, run.repo, run.id, label, None, false),
            _ => return,
        };
        let run = ItemRef::maybe(repo, run_id.clone());
        let now = Utc::now();
        let optimistic = !new_run;
        let mut undo = if optimistic {
            self.change_run(&conn_id, &run_id, |r| match rerun {
                Some(failed_only) => mark_rerun(r, failed_only, now),
                None => mark_canceled(r, now),
            })
        } else {
            RunUndo { conn_id: conn_id.clone(), view: None, extras: None, row: None }
        };
        if optimistic && rerun.is_some() {
            // The old attempt's problems don't describe the new one.
            if let Screen::Pipeline(v) = &mut self.screen {
                if v.connection_id == conn_id && v.run.id == run_id {
                    undo.extras = Some((std::mem::take(&mut v.annotations), v.log_failure.take()));
                    v.problem_focus = false;
                }
            }
        }
        if optimistic {
            // Until the provider catches up, a refresh that still says the old thing mustn't
            // flip the run back.
            let shown = match &self.screen {
                Screen::Pipeline(v) if v.connection_id == conn_id && v.run.id == run_id => Some(v.run.clone()),
                _ => self.pipes.iter().find(|p| p.connection_id == conn_id && p.run.id == run_id).map(|p| p.run.clone()),
            };
            if let Some(shown) = shown {
                self.held_runs.insert((conn_id.clone(), run_id.clone()), RunHold { run: shown, until: now + chrono::Duration::seconds(RUN_HOLD_SECS) });
            }
        }
        let token = self.next_run_action;
        self.next_run_action += 1;
        self.run_actions.insert(token, PendingRunAction { undo, run_id, label, rerun, new_run });
        let Some(tx) = self.job_tx.clone() else {
            // No event loop to answer on (a bare harness): ask inline.
            let result = run_action_call(deps, &conn_id, &run, rerun).await;
            self.finish_run_action(token, result, deps);
            return;
        };
        let deps = deps.clone();
        tokio::spawn(async move {
            let result = run_action_call(&deps, &conn_id, &run, rerun).await;
            let _ = tx.send(AppEvent::PipelineRunActionDone { token, result });
        });
    }

    /// Folds in the provider's answer to a rerun or cancel.
    fn finish_run_action(&mut self, token: u64, result: std::result::Result<Option<String>, String>, deps: &AppDeps) {
        let Some(pending) = self.run_actions.remove(&token) else { return };
        let PendingRunAction { undo, run_id, label, rerun, new_run } = pending;
        let conn_id = undo.conn_id.clone();
        match result {
            Err(e) => {
                self.held_runs.remove(&(conn_id, run_id));
                self.restore_run(undo);
                let what = if rerun.is_some() { "Rerun" } else { "Cancel" };
                self.toast_error(format!("{what} failed: {e}"));
            }
            Ok(started) => {
                let started = started.filter(|id| *id != run_id);
                if new_run || started.is_some() {
                    self.held_runs.remove(&(conn_id.clone(), run_id));
                    self.toast = Some(match &started {
                        Some(id) => format!("Started a new run of {label} ({id}) — it opens here once it lists"),
                        None => format!("Started a new run of {label}"),
                    });
                    if let Some(id) = started {
                        self.follow_new_run = Some((conn_id, id));
                    }
                } else {
                    self.toast = Some(match rerun {
                        Some(true) => format!("Rerunning the failed jobs of {label}"),
                        Some(false) => format!("Rerunning {label}"),
                        None => format!("Cancelled {label}"),
                    });
                    // The provider has the last word on what the run looks like now.
                    if let Screen::Pipeline(v) = &self.screen {
                        if v.connection_id == conn_id && v.run.id == run_id {
                            if let Some(request) = DetailRequest::for_view(&self.screen) {
                                self.send_detail_request(deps, request);
                            }
                        }
                    }
                }
                self.request_reload(deps);
            }
        }
    }

    /// A refreshed copy of a run, unless an optimistic change to it is still being held: then
    /// the held copy, until the provider's own answer shows the change (or the hold runs out).
    fn settle_held(&mut self, conn_id: &str, fresh: PipelineRun) -> PipelineRun {
        let key = (conn_id.to_string(), fresh.id.clone());
        let Some(hold) = self.held_runs.get(&key) else { return fresh };
        let caught_up = match hold.run.status {
            PipelineRunStatus::Canceled => !fresh.status.is_active(),
            _ => fresh.status.is_active(),
        };
        if caught_up || Utc::now() > hold.until {
            self.held_runs.remove(&key);
            return fresh;
        }
        hold.run.clone()
    }

    /// Once the run a rerun started shows up in the list, the pane moves to it.
    fn follow_started_run(&mut self, deps: &AppDeps) {
        let Some((conn_id, id)) = self.follow_new_run.clone() else { return };
        let Some(row) = self.pipes.iter().position(|p| p.connection_id == conn_id && p.run.id == id) else { return };
        self.follow_new_run = None;
        if !matches!(&self.screen, Screen::Pipeline(v) if v.connection_id == conn_id) {
            return;
        }
        let pipe = &self.pipes[row];
        let label = pipe_label(pipe);
        let (view, request) = Self::build_pipeline_view(
            deps,
            pipe.connection_id.clone(),
            pipe.provider,
            pipe.run.id.clone(),
            pipe.run.definition_id.clone(),
            pipe.run.branch.clone(),
            label.clone(),
            pipe.run.clone(),
        );
        self.screen = view;
        self.send_detail_request(deps, request);
        self.select_pipe_row(row);
        self.refresh_pipeline_context();
        self.toast = Some(format!("Now showing {label}, the new run"));
    }

    /// Applies `change` to a run wherever it is shown — the open view and its list rows —
    /// returning what it replaced.
    fn change_run(&mut self, conn_id: &str, run_id: &str, change: impl Fn(&mut PipelineRun)) -> RunUndo {
        let mut undo = RunUndo { conn_id: conn_id.to_string(), view: None, extras: None, row: None };
        if let Screen::Pipeline(v) = &mut self.screen {
            if v.connection_id == conn_id && v.run.id == run_id {
                undo.view = Some(v.run.clone());
                change(&mut v.run);
                v.clamp_selection();
            }
        }
        for row in self.pipes.iter_mut().filter(|r| r.connection_id == conn_id && r.run.id == run_id) {
            undo.row.get_or_insert_with(|| row.run.clone());
            change(&mut row.run);
        }
        self.rebuild_launchpad();
        undo
    }

    /// Puts back what [`App::change_run`] replaced, where it is still the same run.
    fn restore_run(&mut self, undo: RunUndo) {
        if let (Some(prev), Screen::Pipeline(v)) = (undo.view, &mut self.screen) {
            if v.connection_id == undo.conn_id && v.run.id == prev.id {
                v.run = prev;
                if let Some((annotations, log_failure)) = undo.extras {
                    v.annotations = annotations;
                    v.log_failure = log_failure;
                }
                v.clamp_selection();
            }
        }
        // By identity, not position: a reload may have landed while the provider was deciding,
        // moving (or duplicating) the run's rows.
        if let Some(prev) = undo.row {
            for row in self.pipes.iter_mut().filter(|r| r.connection_id == undo.conn_id && r.run.id == prev.id) {
                row.run = prev.clone();
            }
        }
        self.rebuild_launchpad();
    }

    /// `a`: opens the artifacts list over the pane and fetches it in the background.
    fn open_artifacts(&mut self, deps: &AppDeps) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        if !v.supports_artifacts {
            self.toast = Some(format!("Artifacts aren't supported for {}", v.provider.as_str()));
            return;
        }
        v.artifacts = Some(ArtifactsPanel::default());
        let (conn_id, run) = (v.connection_id.clone(), v.run.item_ref());
        let Some(tx) = self.job_tx.clone() else { return };
        let deps = deps.clone();
        tokio::spawn(async move {
            let result = match pipeline_source(&deps, &conn_id).await {
                Some(source) => source.artifacts(&run).await.map_err(|e| {
                    log_operation_failure(DIAG_PIPELINE_ARTIFACTS);
                    e.to_string()
                }),
                None => Err("pipeline connection not found".into()),
            };
            let _ = tx.send(AppEvent::PipelineArtifactsLoaded { conn_id, run_id: run.id, result });
        });
    }

    /// Fills the artifacts list, if it is still open on that run.
    fn apply_pipeline_artifacts(&mut self, conn_id: &str, run_id: &str, result: std::result::Result<Vec<PipelineArtifact>, String>) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        if v.connection_id != conn_id || v.run.id != run_id {
            return;
        }
        let Some(panel) = &mut v.artifacts else { return };
        match result {
            Ok(items) => {
                panel.items = Some(items);
                panel.error = None;
            }
            Err(e) => {
                panel.items = Some(Vec::new());
                panel.error = Some(e);
            }
        }
        panel.selected = 0;
    }

    /// Keys while the artifacts list is open: move, open in the browser, copy the link, close.
    fn on_artifacts_key(&mut self, key: Key) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        let Some(panel) = &mut v.artifacts else { return };
        let n = panel.items.as_ref().map_or(0, Vec::len);
        let url = panel.items.as_ref().and_then(|items| items.get(panel.selected)).map(|a| a.url.clone());
        match key {
            Key::Escape | Key::Char('q' | 'a') => v.artifacts = None,
            Key::Up | Key::Char('k') => panel.selected = panel.selected.saturating_sub(1),
            Key::Down | Key::Char('j') => {
                if panel.selected + 1 < n {
                    panel.selected += 1;
                }
            }
            Key::Enter | Key::Char('o') => match url {
                Some(Some(url)) => {
                    self.toast = Some(match open::that(&url) {
                        Ok(()) => format!("Opened {url}"),
                        Err(e) => format!("Couldn't open browser: {e}"),
                    });
                }
                Some(None) => self.toast = Some("This artifact has no link".into()),
                None => {}
            },
            Key::Char('y') => match url {
                Some(Some(url)) => self.copy_to_clipboard(&url, "artifact link"),
                Some(None) => self.toast = Some("This artifact has no link".into()),
                None => {}
            },
            _ => {}
        }
    }

    /// Copies `text` and says so.
    fn copy_to_clipboard(&mut self, text: &str, what: &str) {
        self.toast = Some(match (self.clipboard)(text) {
            Ok(()) => format!("Copied {what}"),
            Err(e) => format!("Couldn't copy {what}: {e}"),
        });
    }

    /// `c`: copies the open run's full commit sha.
    fn copy_commit_sha(&mut self) {
        let Screen::Pipeline(v) = &self.screen else { return };
        match v.run.commit_sha.clone().filter(|s| !s.is_empty()) {
            Some(sha) => {
                let short: String = sha.chars().take(7).collect();
                self.copy_to_clipboard(&sha, &format!("commit {short}"));
            }
            None => self.toast = Some("This run has no commit sha".into()),
        }
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
                        // ←/→ step through the run history (see `on_pipeline_deps_key`).
                        Key::Enter | Key::Char(' ') => view.toggle_selected(),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Keys on the drill-in. With logs open, `w` moves the keys between the tree and the log
    /// pane; the tree keeps its own keys and re-targets the pane as the cursor crosses jobs. The
    /// artifacts list and the Problems panel take the keys while they are open / focused.
    fn on_pipeline_screen_key(&mut self, key: Key) {
        let Screen::Pipeline(v) = &mut self.screen else { return };
        if v.artifacts.is_some() {
            self.on_artifacts_key(key);
            return;
        }
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
        if v.problem_focus && self.on_problems_key(key) {
            return;
        }
        let Screen::Pipeline(v) = &mut self.screen else { return };
        let on_step = v.selected_node().is_some_and(|n| n.step.is_some() && !n.group);
        match key {
            Key::Char('L') if logs_open => v.logs = None,
            Key::Escape if logs_open => v.logs = None,
            Key::Char('L') => self.open_pipeline_logs(),
            Key::Enter if on_step => self.open_step_logs(),
            Key::Char('e') => self.toggle_problem_focus(),
            Key::Char('E') => self.open_first_error(),
            Key::Char('R') => self.confirm_rerun(false),
            Key::Char('F') => self.confirm_rerun(true),
            Key::Char('c') => self.copy_commit_sha(),
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
            Key::Left | Key::Char('h') => log.pan_by(-1),
            Key::Right | Key::Char('l') => log.pan_by(1),
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
            Key::Char('z') => log.toggle_fold(),
            Key::Char('Z') => log.toggle_all_folds(),
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
                // A failed job's log is where the failure line reads from when the provider
                // reported no annotation.
                let failed = v.run.stages.iter().flat_map(|s| &s.jobs).any(|j| j.id == job_id && j.status == PipelineRunStatus::Failed);
                if failed {
                    if let Some(summary) = runlog::failure_summary(&log.lines) {
                        v.log_failure = Some((job_id.to_string(), summary));
                    }
                }
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
        if !v.run.status.is_active() {
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

        // A brand-new account connection starts with no repositories chosen, but with the ones
        // its credentials reach already discovered, so the header reads "0 of 38" and the list
        // asks for a pick. Best-effort: if discovery fails the scope stays unset and the
        // connection behaves exactly as a single-repository one did.
        let seeded = forgetop_core::service::seed_default_repo_scope(&deps.config, &deps.sections, &id)
            .await
            .ok()
            .flatten();
        if let Some(page) = &seeded {
            self.repo_catalog.insert(id.clone(), page.clone());
        }

        // Bind every section that was ticked. One failing section does not abandon the rest:
        // the connection already exists, so the useful outcome is "bound what it could" plus
        // an honest message about what it could not.
        let mut bound = 0usize;
        let mut failed: Vec<String> = Vec::new();
        for section in &draft.bind_sections {
            let result = match section {
                Section::PullRequests => deps.config.bind_pull_requests(&id).await,
                Section::WorkItems => deps.config.bind_work_items(&id).await,
                // Pipelines are opt-in: the connection is added with nothing selected (`w`).
                Section::Pipelines => deps.config.bind_pipelines(&id).await,
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
            Some(page) => format!(
                "Added {} connection{sections} · {} repositories found — press w to choose",
                provider.as_str(),
                page.repositories.len()
            ),
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

    /// Opens the "Columns" checklist for the active list: ticked = shown. The status and title
    /// columns aren't offered — a row without them has nothing left to read.
    fn open_columns_toggle(&mut self) {
        let section = self.active;
        let items = LIST_COLUMNS[section]
            .iter()
            .map(|&c| ToggleItem { on: self.col_shown(section, c), id: c.into(), label: c.into() })
            .collect();
        self.overlay = Some(Overlay::Toggle {
            title: "Columns".into(),
            kind: ToggleKind::Columns { section },
            min_one: false,
            items,
            selected: 0,
            filter: None,
        });
    }

    /// `ids` are the columns left ticked; every other offered column is hidden.
    async fn apply_columns(&mut self, section: usize, shown: Vec<String>, deps: &AppDeps) {
        self.hidden_cols[section] =
            LIST_COLUMNS[section].iter().filter(|c| !shown.iter().any(|s| s == *c)).map(|c| c.to_string()).collect();
        if let Err(e) = deps.config.set_hidden_columns(self.hidden_cols.clone()).await {
            self.toast = Some(format!("Couldn't save: {e}"));
        }
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
            ToggleKind::Columns { section } => {
                self.apply_columns(section, ids, deps).await;
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

    /// Opens the subscribe checklist for the connection selected on the Config screen.
    async fn open_pipeline_subs(&mut self, deps: &AppDeps) {
        let Some((id, display)) = self.config_selected_id() else { return };
        self.open_pipeline_subs_for(id, display, deps).await;
    }

    /// The Pipelines section's add/remove picker. The subscription is per connection, so with
    /// several the user says which first, as the repository picker does.
    async fn open_pipeline_subs_from_list(&mut self, deps: &AppDeps) {
        let cfg = deps.config.snapshot();
        let ids: Vec<String> =
            cfg.pipelines.as_ref().map(|p| p.subscriptions.iter().map(|s| s.connection_id.clone()).collect()).unwrap_or_default();
        let name = |id: &String| cfg.find_connection(id).map(|c| c.display_name.clone()).unwrap_or_else(|| id.clone());
        match ids.as_slice() {
            [] => self.toast = Some("No connection feeds Pipelines — bind one with C".into()),
            [only] => {
                let display = name(only);
                self.open_pipeline_subs_for(only.clone(), display, deps).await;
            }
            many => {
                let items = many.iter().map(name).collect();
                self.repo_scope_choices = many.to_vec();
                self.overlay = Some(Overlay::Picker {
                    title: "Pipelines · which connection?".into(),
                    items,
                    selected: 0,
                    kind: PickerKind::PipelineSubsConnection,
                });
            }
        }
    }

    /// Discovers a connection's pipeline definitions and opens a subscribe checklist.
    async fn open_pipeline_subs_for(&mut self, id: String, display: String, deps: &AppDeps) {
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
        self.pipe_catalog.insert(id.clone(), defs.clone());
        let mut defs = defs;
        // Grouped by where they live (project + folder, or repository), so a folder's pipelines
        // sit together and the search can narrow to one.
        defs.sort_by_key(|d| (pipeline_location(d), d.name.to_lowercase()));

        let cfg = deps.config.snapshot();
        let sub = cfg.pipelines.as_ref().and_then(|p| p.subscriptions.iter().find(|s| s.connection_id == id));
        // Pipelines are opt-in: only "every pipeline" or an explicit selection is ticked. The
        // selection is saved to the config, so it opens as it was left on the next run.
        let auto = sub.is_some_and(|s| s.auto_discover_all);
        let subscribed: std::collections::HashSet<String> =
            sub.map(|s| s.definition_ids.iter().cloned().collect()).unwrap_or_default();

        let items = defs
            .iter()
            .map(|d| ToggleItem { id: d.id.clone(), label: pipeline_label(d), on: auto || subscribed.contains(&d.id) })
            .collect();

        self.overlay = Some(Overlay::Toggle {
            title: format!("Pipelines · {display}"),
            kind: ToggleKind::PipelineSubs { connection_id: id },
            // Nothing ticked is a real choice: this connection's runs are not fetched.
            min_one: false,
            items,
            selected: 0,
            filter: Some(String::new()),
        });
    }

    async fn apply_pipeline_subs(&mut self, connection_id: &str, ids: Vec<String>, deps: &AppDeps) {
        // Every pipeline ticked is saved as "all", so one created later is picked up too.
        let all = self
            .pipe_catalog
            .get(connection_id)
            .is_some_and(|defs| !defs.is_empty() && defs.iter().all(|d| ids.contains(&d.id)));
        let saved = if all {
            deps.config.set_pipeline_auto_discover(connection_id, true).await
        } else {
            deps.config.set_pipeline_definitions(connection_id, ids.clone()).await
        };
        match saved {
            Ok(()) => {
                self.toast = Some(if all {
                    "Fetching every pipeline".into()
                } else if ids.is_empty() {
                    "No pipelines selected".into()
                } else {
                    format!("Fetching {} pipeline(s)", ids.len())
                });
                // Runs of a pipeline just unticked go now, not on the next reload.
                if !all {
                    self.pipes.retain(|r| r.connection_id != connection_id || ids.contains(&r.run.definition_id));
                }
                self.refresh_repo_scope(deps);
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
        // Its identity goes too, or a connection re-made under the same id could inherit it.
        self.pr_pool.me.remove(id);
        self.pr_pool.review_clears_request.remove(id);
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
            Action::PipelineRerun { .. } | Action::PipelineCancel { .. } => {
                self.execute_pipeline_run_action(action, deps).await
            }
            Action::RemoveConnection { .. } => self.execute_config_action(action, deps).await,
            Action::DismissAllInbox => self.dismiss_all_inbox(deps).await,
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
            Action::OpenPipelineSubs { index } => {
                if let Some(id) = self.repo_scope_choices.get(index).cloned() {
                    let display = deps
                        .config
                        .snapshot()
                        .find_connection(&id)
                        .map(|c| c.display_name.clone())
                        .unwrap_or_else(|| id.clone());
                    self.open_pipeline_subs_for(id, display, deps).await;
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

    /// Runs a confirmed PR write. Optimistic: what the write decides on its own — your vote, your
    /// comment or reply — shows on the open PR and its list rows straight away, the provider is
    /// asked in the background (its answer comes back as [`AppEvent::PrActionDone`]), and a
    /// refusal takes it back. A merge or revert changes nothing up front: whether it happens is
    /// the provider's call, so it is reported once the provider has made it.
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
        let call = match action {
            Action::PrVote(vote) => PrCall::Vote(vote),
            Action::PrMerge(strategy) => PrCall::Merge(strategy),
            Action::PrRevert => PrCall::Revert,
            Action::PrComment(text) => {
                if text.trim().is_empty() {
                    self.toast = Some("Empty comment — nothing sent".into());
                    return;
                }
                PrCall::Comment(text)
            }
            Action::PrReply(text) => {
                if text.trim().is_empty() {
                    self.toast = Some("Empty reply — nothing sent".into());
                    return;
                }
                let thread_id = match &mut self.screen {
                    Screen::PrView(v) => v.reply_target.take(),
                    _ => None,
                };
                let Some(thread_id) = thread_id else {
                    self.toast = Some("No thread selected to reply to".into());
                    return;
                };
                PrCall::Reply { thread_id, body: text }
            }
            _ => return,
        };
        self.send_pr_call(conn_id, item, call, deps).await;
    }

    /// Shows `call`'s outcome where it is the user's to decide, then sends it in the background.
    async fn send_pr_call(&mut self, conn_id: String, item: ItemRef, call: PrCall, deps: &AppDeps) {
        let token = self.next_item_action;
        self.next_item_action += 1;
        let key = pr_detail_cache_key(&conn_id, &item);
        let me = self.me_on(&conn_id);
        let mut pending = PendingItemAction::new(conn_id.clone(), item.clone(), key.clone());
        let (now_msg, done) = match &call {
            PrCall::Vote(vote) => (Some(vote_message(*vote).to_string()), vote_message(*vote).to_string()),
            PrCall::Merge(strategy) => (Some("Merging…".to_string()), format!("Merged ({strategy:?})")),
            PrCall::Revert => (Some("Requesting a revert…".to_string()), "Revert requested".to_string()),
            PrCall::Comment(_) => (Some("Comment added".to_string()), "Comment added".to_string()),
            PrCall::Reply { .. } => (Some("Reply posted".to_string()), "Reply posted".to_string()),
            PrCall::Review { comments, .. } => {
                let msg = format!("Review submitted ({} comment(s))", comments.len());
                (Some(msg.clone()), msg)
            }
        };
        pending.done = done;
        pending.merge = matches!(call, PrCall::Merge(_));
        if matches!(call, PrCall::Review { .. }) {
            pending.failed = "Submit failed";
        }

        // Your verdict: a vote, or the event a review is submitted with.
        let vote = match &call {
            PrCall::Vote(vote) => Some(*vote),
            PrCall::Review { event, .. } if *event != ReviewVote::NoVote => Some(*event),
            _ => None,
        };
        if let (Some(vote), Some(me)) = (vote, me.as_deref()) {
            pending.reviewers = self.shown_pr(&conn_id, &item).map(|pr| pr.reviewers.clone());
            self.patch_pr(&conn_id, &item, |pr| set_vote(pr, me, vote));
            self.hold(&key).vote = Some((token, me.to_string(), vote));
        }
        // Reviewing a PR clears it from the Launchpad now; a merge waits for the provider.
        if matches!(call, PrCall::Vote(_) | PrCall::Review { .. }) {
            pending.dismissed = self.lp_dismissed.insert(launchpad::Entry::key(&conn_id, &item.id));
            self.rebuild_launchpad();
        }
        // On a forge that stops asking for your review once you have given it, the PR leaves
        // the review-requested list now, not when the refetch finds it gone.
        if vote.is_some() && me.is_some() && self.review_clears_request_on(&conn_id) {
            self.hold(&key).review_cleared = Some(token);
            self.refresh_derived_prs(deps);
            self.fix_selection();
            self.refresh_preview_row();
        }

        let author = me_user(me.as_deref());
        let local = |n: usize| format!("{}{n}", local_prefix(token));
        let held: Vec<HeldComment> = match &call {
            PrCall::Comment(body) => vec![HeldComment::Thread(CommentThread {
                id: local(0),
                comments: vec![Comment { id: local(0), author: author.clone(), body: body.clone(), created_at: Some(Utc::now()) }],
                file_path: None,
                line: None,
                is_resolved: false,
            })],
            PrCall::Reply { thread_id, body } => vec![HeldComment::Reply {
                thread_id: thread_id.clone(),
                comment: Comment { id: local(0), author: author.clone(), body: body.clone(), created_at: Some(Utc::now()) },
            }],
            PrCall::Review { comments, .. } => comments
                .iter()
                .enumerate()
                .map(|(n, c)| {
                    HeldComment::Thread(CommentThread {
                        id: local(n),
                        comments: vec![Comment { id: local(n), author: author.clone(), body: c.body.clone(), created_at: Some(Utc::now()) }],
                        file_path: Some(c.path.clone()),
                        line: Some(c.line),
                        is_resolved: false,
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        if let PrCall::Review { .. } = &call {
            for v in self.item_views_mut() {
                if let Screen::PrView(v) = v {
                    if v.connection_id == conn_id && v.pr.item_ref() == item {
                        pending.pending = std::mem::take(&mut v.pending);
                        v.review_draft = None;
                    }
                }
            }
        }
        if !held.is_empty() {
            for v in self.item_views_mut() {
                if let Screen::PrView(v) = v {
                    if v.connection_id == conn_id && v.pr.item_ref() == item {
                        for h in &held {
                            h.show_on(&mut v.diff.threads);
                        }
                    }
                }
            }
            let seen = self.shown_threads(&key);
            let hold = self.hold(&key);
            hold.comments.extend(held.into_iter().map(|h| h.counted(&seen)));
        }
        if let Some(msg) = now_msg {
            self.toast = Some(msg);
        }
        self.item_actions.insert(token, pending);

        let Some(tx) = self.job_tx.clone() else {
            // No event loop to answer on (a bare harness): ask inline.
            let (result, fresh) = pr_action_call(deps, &conn_id, &item, &call).await;
            self.finish_pr_action(token, result, fresh, deps);
            return;
        };
        let deps = deps.clone();
        tokio::spawn(async move {
            let (result, fresh) = pr_action_call(&deps, &conn_id, &item, &call).await;
            let _ = tx.send(AppEvent::PrActionDone { token, result, fresh: fresh.map(Box::new) });
        });
    }

    /// Folds in the provider's answer to a PR write.
    fn finish_pr_action(&mut self, token: u64, result: std::result::Result<(), String>, fresh: Option<PullRequest>, deps: &AppDeps) {
        let Some(pending) = self.item_actions.remove(&token) else { return };
        let PendingItemAction { conn_id, item, key, done, failed, reviewers, pending: comments, dismissed, merge, .. } = pending;
        match result {
            Err(e) => {
                let prefix = local_prefix(token);
                let listed_again = self.held_items.get(&key).is_some_and(|h| h.review_cleared == Some(token));
                if let Some(hold) = self.held_items.get_mut(&key) {
                    hold.forget(token);
                }
                self.prune_holds();
                if let Some(reviewers) = reviewers {
                    self.patch_pr(&conn_id, &item, |pr| pr.reviewers = reviewers.clone());
                }
                if listed_again {
                    // The review was refused, so the forge still wants it: back on the list.
                    self.refresh_derived_prs(deps);
                    self.fix_selection();
                    self.refresh_preview_row();
                }
                for v in self.item_views_mut() {
                    if let Screen::PrView(v) = v {
                        if v.connection_id == conn_id && v.pr.item_ref() == item {
                            drop_local(&mut v.diff.threads, &prefix);
                            // The review's comments go back in the buffer, ahead of any added since.
                            if !comments.is_empty() {
                                let added = std::mem::take(&mut v.pending);
                                v.pending = comments.iter().cloned().chain(added).collect();
                            }
                        }
                    }
                }
                if dismissed {
                    self.lp_dismissed.remove(&launchpad::Entry::key(&conn_id, &item.id));
                    self.rebuild_launchpad();
                }
                self.toast_error(format!("{failed}: {e}"));
            }
            Ok(()) => {
                if merge {
                    self.dismiss_from_launchpad(&conn_id, &item.id);
                }
                self.toast = Some(done);
                // The provider's own copy of the PR (status / reviewers / mergeable), with any
                // vote it hasn't caught up with yet still showing.
                if let Some(mut fresh) = fresh {
                    self.settle_vote(&key, &mut fresh);
                    for v in self.item_views_mut() {
                        if let Screen::PrView(v) = v {
                            if v.connection_id == conn_id && v.pr.item_ref() == item {
                                v.pr = fresh.clone();
                            }
                        }
                    }
                }
                // Threads (the new comment) and the timeline come back with the view's detail.
                self.refresh_item_views(&key, deps);
                self.reload_after_write(deps);
            }
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

    /// Reflects an edit on the open work item (or its preview) *and* the list row behind it.
    ///
    /// Both, because an action taken from the drill-in that only patched the view would be undone
    /// on the screen the user presses Esc back to. A state change leaves `state_category` alone:
    /// which bucket a state name falls in (Backlog/Active/Done) is decided by each provider's
    /// mapper, not derivable from the name here — the provider's copy re-read after the write is
    /// what settles it.
    fn patch_wi(&mut self, conn_id: &str, item: &ItemRef, edit: impl Fn(&mut WorkItem)) {
        for v in self.item_views_mut() {
            if let Screen::WiView(v) = v {
                if v.connection_id == conn_id && &v.wi.item_ref() == item {
                    edit(&mut v.wi);
                }
            }
        }
        for row in self.wis.iter_mut().filter(|r| r.connection_id == conn_id && &r.wi.item_ref() == item) {
            edit(&mut row.wi);
        }
        self.rebuild_launchpad();
    }

    /// Runs a work-item write. Optimistic, like [`App::execute_pr_action`]: the edit or comment
    /// shows on the open item and its list row straight away, the provider is asked in the
    /// background (its answer comes back as [`AppEvent::WiActionDone`]), and a refusal puts the
    /// item back as it was.
    async fn execute_wi_action(&mut self, action: Action, deps: &AppDeps) {
        let target = match &self.screen {
            Screen::WiView(v) => Some((v.wi.item_ref(), v.connection_id.clone())),
            _ => self.selected_wi_row().map(|r| (r.wi.item_ref(), r.connection_id.clone())),
        };
        let Some((item, conn_id)) = target else {
            self.toast = Some("Nothing selected".into());
            return;
        };
        let (call, done) = match action {
            Action::WiSetState(state) => (WiCall::Edit(WiEdit::State(state.clone())), format!("State → {state}")),
            Action::WiComment(text) => {
                if text.trim().is_empty() {
                    self.toast = Some("Empty comment — nothing sent".into());
                    return;
                }
                (WiCall::Comment(text), "Comment added".to_string())
            }
            Action::WiAssign { id, label } => {
                let done = match &id {
                    Some(_) => format!("Assigned to {label}"),
                    None => "Unassigned".to_string(),
                };
                let assignee = id.map(|id| User { id, display_name: label, handle: None, avatar_url: None });
                (WiCall::Edit(WiEdit::Assignee(assignee)), done)
            }
            Action::WiSetTitle(title) => {
                let title = title.trim();
                if title.is_empty() {
                    self.toast = Some("A title can't be empty — nothing sent".into());
                    return;
                }
                (WiCall::Edit(WiEdit::Title(title.to_string())), "Title updated".to_string())
            }
            Action::WiSetDescription(text) => (WiCall::Edit(WiEdit::Description(text)), "Description updated".to_string()),
            _ => return,
        };

        let token = self.next_item_action;
        self.next_item_action += 1;
        let key = wi_detail_cache_key(&conn_id, &item);
        let mut pending = PendingItemAction::new(conn_id.clone(), item.clone(), key.clone());
        match &call {
            WiCall::Edit(edit) => {
                pending.wi = self.shown_wi(&conn_id, &item).cloned();
                let edit = edit.clone();
                self.patch_wi(&conn_id, &item, |wi| edit.apply(wi));
                self.hold(&key).edits.push((token, edit));
            }
            WiCall::Comment(body) => {
                let id = format!("{}0", local_prefix(token));
                let author = me_user(self.me_on(&conn_id).as_deref());
                let held = HeldComment::Thread(CommentThread {
                    id: id.clone(),
                    comments: vec![Comment { id, author, body: body.clone(), created_at: Some(Utc::now()) }],
                    file_path: None,
                    line: None,
                    is_resolved: false,
                });
                for v in self.item_views_mut() {
                    if let Screen::WiView(v) = v {
                        if v.connection_id == conn_id && v.wi.item_ref() == item {
                            held.show_on(&mut v.threads);
                        }
                    }
                }
                let seen = self.shown_threads(&key);
                let held = held.counted(&seen);
                self.hold(&key).comments.push(held);
            }
        }
        self.toast = Some(done.clone());
        pending.done = done;
        self.item_actions.insert(token, pending);

        let Some(tx) = self.job_tx.clone() else {
            // No event loop to answer on (a bare harness): ask inline.
            let (result, fresh) = wi_action_call(deps, &conn_id, &item, &call).await;
            self.finish_wi_action(token, result, fresh, deps);
            return;
        };
        let deps = deps.clone();
        tokio::spawn(async move {
            let (result, fresh) = wi_action_call(&deps, &conn_id, &item, &call).await;
            let _ = tx.send(AppEvent::WiActionDone { token, result, fresh: fresh.map(Box::new) });
        });
    }

    /// Folds in the provider's answer to a work-item write.
    fn finish_wi_action(&mut self, token: u64, result: std::result::Result<(), String>, fresh: Option<WorkItem>, deps: &AppDeps) {
        let Some(pending) = self.item_actions.remove(&token) else { return };
        let PendingItemAction { conn_id, item, key, done: _, failed, wi, .. } = pending;
        match result {
            Err(e) => {
                let undo = self.held_items.get_mut(&key).and_then(|hold| hold.forget(token));
                self.prune_holds();
                // Only the field this edit changed goes back: anything else on the item may
                // have moved on since.
                if let (Some(edit), Some(before)) = (undo, wi) {
                    let back = edit.undo(&before);
                    self.patch_wi(&conn_id, &item, |wi| back.apply(wi));
                }
                let prefix = local_prefix(token);
                for v in self.item_views_mut() {
                    if let Screen::WiView(v) = v {
                        if v.connection_id == conn_id && v.wi.item_ref() == item {
                            drop_local(&mut v.threads, &prefix);
                        }
                    }
                }
                self.toast_error(format!("{failed}: {e}"));
            }
            Ok(()) => {
                // The provider's own copy settles what can't be guessed here — which bucket a
                // state name falls in is each provider mapper's call.
                if let Some(mut fresh) = fresh {
                    self.settle_edits(&key, &mut fresh);
                    for v in self.item_views_mut() {
                        if let Screen::WiView(v) = v {
                            if v.connection_id == conn_id && v.wi.item_ref() == item {
                                v.wi = fresh.clone();
                            }
                        }
                    }
                    for row in self.wis.iter_mut().filter(|r| r.connection_id == conn_id && r.wi.item_ref() == item) {
                        row.wi = fresh.clone();
                    }
                    self.rebuild_launchpad();
                }
                // Every write lands in the item's history, so the Activity section is re-read.
                self.refresh_item_views(&key, deps);
                self.reload_after_write(deps);
            }
        }
    }

    // ---- optimistic PR / work-item writes: shared plumbing ----

    /// The open item view and the preview's, either of which can show the item a write is for.
    fn item_views_mut(&mut self) -> impl Iterator<Item = &mut Screen> {
        std::iter::once(&mut self.screen).chain(self.preview.as_mut().map(|p| &mut p.view))
    }

    /// The signed-in user's handle on `conn_id`, when the last PR fetch established one.
    fn me_on(&self, conn_id: &str) -> Option<String> {
        self.pr_pool.me.get(conn_id).cloned().flatten()
    }

    /// Whether `conn_id`'s forge stops listing a PR as review-requested once you have reviewed it.
    fn review_clears_request_on(&self, conn_id: &str) -> bool {
        self.pr_pool.review_clears_request.get(conn_id).copied().unwrap_or(false)
    }

    /// The detail keys of PRs a live hold keeps off the review-requested list.
    fn review_cleared_keys(&self) -> HashSet<String> {
        let now = Utc::now();
        self.held_items
            .iter()
            .filter(|(_, h)| h.review_cleared.is_some() && now <= h.until)
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// The hold for the item whose detail key is `key`, started now if there was none.
    fn hold(&mut self, key: &str) -> &mut ItemHold {
        let now = Utc::now();
        let until = now + chrono::Duration::seconds(ITEM_HOLD_SECS);
        let hold = self.held_items.entry(key.to_string()).or_insert_with(|| ItemHold::new(until));
        // One that ran out is over — the provider had the last word — so it starts afresh
        // rather than reviving what it used to hold.
        if now > hold.until {
            *hold = ItemHold::new(until);
        }
        hold.until = until;
        hold
    }

    /// The hold for `key`, unless it has run out (then it is dropped).
    fn live_hold(&mut self, key: &str) -> Option<&mut ItemHold> {
        if self.held_items.get(key).is_some_and(|h| Utc::now() > h.until) {
            self.held_items.remove(key);
        }
        self.held_items.get_mut(key)
    }

    /// The PR as shown: the open view's copy, else its list row's.
    fn shown_pr(&self, conn_id: &str, item: &ItemRef) -> Option<&PullRequest> {
        let views = std::iter::once(&self.screen).chain(self.preview.as_ref().map(|p| &p.view));
        views
            .filter_map(|s| match s {
                Screen::PrView(v) if v.connection_id == conn_id && &v.pr.item_ref() == item => Some(&v.pr),
                _ => None,
            })
            .next()
            .or_else(|| self.pr_pool.open.iter().chain(&self.pr_pool.completed).chain(&self.prs).find(|r| r.connection_id == conn_id && &r.pr.item_ref() == item).map(|r| &r.pr))
    }

    /// The work item as shown: the open view's copy, else its list row's.
    fn shown_wi(&self, conn_id: &str, item: &ItemRef) -> Option<&WorkItem> {
        let views = std::iter::once(&self.screen).chain(self.preview.as_ref().map(|p| &p.view));
        views
            .filter_map(|s| match s {
                Screen::WiView(v) if v.connection_id == conn_id && &v.wi.item_ref() == item => Some(&v.wi),
                _ => None,
            })
            .next()
            .or_else(|| self.wis.iter().find(|r| r.connection_id == conn_id && &r.wi.item_ref() == item).map(|r| &r.wi))
    }

    /// The threads on screen for the item whose detail key is `key` (empty when it isn't open).
    fn shown_threads(&self, key: &str) -> Vec<CommentThread> {
        let views = std::iter::once(&self.screen).chain(self.preview.as_ref().map(|p| &p.view));
        views
            .filter_map(|s| match s {
                Screen::PrView(v) if pr_detail_cache_key(&v.connection_id, &v.pr.item_ref()) == key => Some(v.diff.threads.clone()),
                Screen::WiView(v) if wi_detail_cache_key(&v.connection_id, &v.wi.item_ref()) == key => Some(v.threads.clone()),
                _ => None,
            })
            .next()
            .unwrap_or_default()
    }

    /// Applies `edit` to the PR wherever it is shown — the open view, the preview, the pool and
    /// every list derived from it — so the screen the user goes back to agrees with this one.
    fn patch_pr(&mut self, conn_id: &str, item: &ItemRef, edit: impl Fn(&mut PullRequest)) {
        for v in self.item_views_mut() {
            if let Screen::PrView(v) = v {
                if v.connection_id == conn_id && &v.pr.item_ref() == item {
                    edit(&mut v.pr);
                }
            }
        }
        let rows = self
            .pr_pool
            .open
            .iter_mut()
            .chain(self.pr_pool.completed.iter_mut())
            .chain(self.prs.iter_mut())
            .chain(self.lp_prs_mine.iter_mut())
            .chain(self.lp_prs_review.iter_mut());
        for row in rows.filter(|r| r.connection_id == conn_id && &r.pr.item_ref() == item) {
            edit(&mut row.pr);
        }
        self.rebuild_launchpad();
    }

    /// Shows a held vote on a refreshed copy of its PR, until the provider's copy shows it too.
    fn settle_vote(&mut self, key: &str, pr: &mut PullRequest) {
        let Some(hold) = self.live_hold(key) else { return };
        if let Some((_, me, vote)) = hold.vote.clone() {
            if has_vote(pr, &me, vote) {
                hold.vote = None;
            } else {
                set_vote(pr, &me, vote);
            }
        }
        self.prune_holds();
    }

    /// Re-applies held votes to a freshly landed PR pool.
    fn settle_held_votes(&mut self) {
        if self.held_items.is_empty() {
            return;
        }
        let mut pool = std::mem::take(&mut self.pr_pool);
        // Settled per PR, not per row: the open and completed lists can both hold one, and the
        // second must not be read as caught up because the first cleared the hold.
        let mut caught_up = HashSet::new();
        let mut cleared = HashSet::new();
        for row in pool.open.iter_mut().chain(pool.completed.iter_mut()) {
            let key = pr_detail_cache_key(&row.connection_id, &row.pr.item_ref());
            let Some(hold) = self.live_hold(&key) else { continue };
            // A row that no longer names you as a reviewer is the forge having caught up with
            // your review — the vote too, so nothing puts it back on this row later: that would
            // list you again, and the review-requested view with you. The row is left as it came.
            if hold.review_cleared.is_some() {
                let me = pool.me.get(&row.connection_id).and_then(|m| m.as_deref());
                if !pull_request_matches(&row.pr, PullRequestFilter::ReviewRequested, me) {
                    caught_up.insert(key.clone());
                    cleared.insert(key);
                    continue;
                }
            }
            let Some((_, me, vote)) = hold.vote.clone() else { continue };
            if has_vote(&row.pr, &me, vote) {
                caught_up.insert(key);
            } else {
                set_vote(&mut row.pr, &me, vote);
            }
        }
        for key in caught_up {
            if let Some(hold) = self.held_items.get_mut(&key) {
                hold.vote = None;
            }
        }
        for key in cleared {
            if let Some(hold) = self.held_items.get_mut(&key) {
                hold.review_cleared = None;
            }
        }
        self.pr_pool = pool;
        self.prune_holds();
    }

    /// Re-applies held edits to a work item's refreshed copy, until it shows them itself.
    fn settle_edits(&mut self, key: &str, wi: &mut WorkItem) {
        let Some(hold) = self.live_hold(key) else { return };
        hold.edits.retain(|(_, e)| !e.shown_by(wi));
        for (_, e) in &hold.edits {
            e.apply(wi);
        }
        self.prune_holds();
    }

    /// Re-applies held edits to freshly landed work-item rows.
    fn settle_held_edits(&mut self) {
        if self.held_items.is_empty() {
            return;
        }
        let mut wis = std::mem::take(&mut self.wis);
        for row in &mut wis {
            let key = wi_detail_cache_key(&row.connection_id, &row.wi.item_ref());
            self.settle_edits(&key, &mut row.wi);
        }
        self.wis = wis;
    }

    /// Puts held comments the provider hasn't listed yet onto `threads`; forgets those it has.
    fn settle_held_comments(&mut self, key: &str, threads: &mut Vec<CommentThread>) {
        let Some(hold) = self.live_hold(key) else { return };
        hold.comments.retain(|h| !h.listed_in(threads));
        for h in &hold.comments {
            h.show_on(threads);
        }
        self.prune_holds();
    }

    /// Drops holds with nothing left to hold.
    fn prune_holds(&mut self) {
        self.held_items
            .retain(|_, h| h.vote.is_some() || h.review_cleared.is_some() || !h.edits.is_empty() || !h.comments.is_empty());
    }

    /// Re-asks for the detail of every view showing the item whose detail key is `key`.
    fn refresh_item_views(&self, key: &str, deps: &AppDeps) {
        let views = std::iter::once(&self.screen).chain(self.preview.as_ref().map(|p| &p.view));
        for request in views.filter_map(DetailRequest::for_view).filter(|r| r.key() == key) {
            self.send_detail_request(deps, request);
        }
    }

    /// A refresh after a write. One already out was asked for before the write, so it can't
    /// show it: another follows it.
    fn reload_after_write(&mut self, deps: &AppDeps) {
        if self.reloading {
            self.reload_again = true;
        }
        self.request_reload(deps);
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

/// Whether a listed run already carries timed jobs, so its detail needn't be fetched for an
/// estimate.
fn has_timed_jobs(run: &PipelineRun) -> bool {
    run.stages.iter().flat_map(|s| &s.jobs).any(|j| j.started_at.is_some() && j.finished_at.is_some())
}

/// An attempt's problems as the pane shows them: `(annotations, failure read from its log)`.
type AttemptProblems = (Vec<PipelineAnnotation>, Option<(String, FailureSummary)>);

/// What an optimistic rerun or cancel replaced, so a refusal can put it back.
struct RunUndo {
    conn_id: String,
    view: Option<PipelineRun>,
    /// The view's problems and failure line, cleared by a rerun.
    extras: Option<AttemptProblems>,
    /// The run as its list rows showed it, restored onto every row of that run.
    row: Option<PipelineRun>,
}

/// A rerun or cancel in flight: what to undo, and what to say when the provider answers.
struct PendingRunAction {
    undo: RunUndo,
    run_id: String,
    label: String,
    /// `Some(failed_only)` for a rerun, `None` for a cancel.
    rerun: Option<bool>,
    /// The provider starts a new run rather than re-queueing this one.
    new_run: bool,
}

/// An optimistic change held against refreshes that haven't caught up with it yet.
struct RunHold {
    run: PipelineRun,
    until: DateTime<Utc>,
}

/// How long an optimistic rerun or cancel is held against a provider still reporting the old
/// state.
const RUN_HOLD_SECS: i64 = 60;

/// A PR write, as sent to the provider.
enum PrCall {
    Vote(ReviewVote),
    Merge(MergeStrategy),
    Revert,
    Comment(String),
    Reply { thread_id: String, body: String },
    Review { event: ReviewVote, comments: Vec<LineComment> },
}

/// A work-item write, as sent to the provider.
enum WiCall {
    Edit(WiEdit),
    Comment(String),
}

/// A work-item field change, applied on screen before the provider has it.
#[derive(Clone)]
enum WiEdit {
    State(String),
    Assignee(Option<User>),
    Title(String),
    Description(String),
}

impl WiEdit {
    fn apply(&self, wi: &mut WorkItem) {
        match self {
            WiEdit::State(state) => wi.state = state.clone(),
            WiEdit::Assignee(user) => wi.assignee = user.clone(),
            WiEdit::Title(title) => wi.title = title.clone(),
            WiEdit::Description(text) => wi.description = Some(text.clone()),
        }
    }

    /// Whether a copy of the item from the provider already shows this edit. An assignee is
    /// matched by id or name, since a provider's item mapper and its assignable-users mapper
    /// needn't use the same id (see [`assignee_picker`]).
    fn shown_by(&self, wi: &WorkItem) -> bool {
        match self {
            WiEdit::State(state) => &wi.state == state,
            WiEdit::Assignee(None) => wi.assignee.is_none(),
            WiEdit::Assignee(Some(u)) => wi.assignee.as_ref().is_some_and(|a| a.id == u.id || a.display_name == u.display_name),
            WiEdit::Title(title) => &wi.title == title,
            WiEdit::Description(text) => wi.description.as_deref() == Some(text.as_str()),
        }
    }

    /// The edit that puts back what this one replaced on `before`.
    fn undo(&self, before: &WorkItem) -> WiEdit {
        match self {
            WiEdit::State(_) => WiEdit::State(before.state.clone()),
            WiEdit::Assignee(_) => WiEdit::Assignee(before.assignee.clone()),
            WiEdit::Title(_) => WiEdit::Title(before.title.clone()),
            WiEdit::Description(_) => WiEdit::Description(before.description.clone().unwrap_or_default()),
        }
    }
}

/// A comment posted from here, shown before the provider lists it.
#[derive(Clone)]
enum HeldComment {
    /// A new thread: a top-level comment, or a review's line comment.
    Thread(CommentThread),
    /// A reply on an existing thread.
    Reply { thread_id: String, comment: Comment },
    /// Either of the above, with how many comments with the same text were already listed when
    /// it was posted — so it is caught up once one more is, and posting "LGTM" twice holds the
    /// second until the provider has two.
    Counted { held: Box<HeldComment>, seen: usize },
}

impl HeldComment {
    fn comment(&self) -> Option<&Comment> {
        match self {
            HeldComment::Thread(t) => t.comments.first(),
            HeldComment::Reply { comment, .. } => Some(comment),
            HeldComment::Counted { held, .. } => held.comment(),
        }
    }

    /// How many comments in `threads` the provider listed with this one's text — and, when
    /// who you are is known, by you: a teammate's identical "LGTM" isn't yours.
    fn listed(&self, threads: &[CommentThread]) -> usize {
        let Some(held) = self.comment() else { return 0 };
        let body = held.body.trim();
        let me = held.author.handle.as_deref();
        let thread = match self {
            HeldComment::Reply { thread_id, .. } => Some(thread_id.as_str()),
            HeldComment::Counted { held, .. } => match held.as_ref() {
                HeldComment::Reply { thread_id, .. } => Some(thread_id.as_str()),
                _ => None,
            },
            HeldComment::Thread(_) => None,
        };
        threads
            .iter()
            .filter(|t| thread.is_none_or(|id| t.id == id))
            .flat_map(|t| &t.comments)
            .filter(|c| !is_local(&c.id) && c.body.trim() == body)
            .filter(|c| me.is_none_or(|me| forgetop_core::filter::is_user(&c.author, me)))
            .count()
    }

    /// This comment, counting what `threads` already lists like it.
    fn counted(self, threads: &[CommentThread]) -> HeldComment {
        let seen = self.listed(threads);
        HeldComment::Counted { held: Box::new(self), seen }
    }

    /// Whether the provider has listed this comment in `threads`.
    fn listed_in(&self, threads: &[CommentThread]) -> bool {
        match self {
            HeldComment::Counted { seen, .. } => self.listed(threads) > *seen,
            _ => self.listed(threads) > 0,
        }
    }

    /// Puts the comment on `threads`, unless it is already there.
    fn show_on(&self, threads: &mut Vec<CommentThread>) {
        match self {
            HeldComment::Thread(t) => {
                if !threads.iter().any(|x| x.id == t.id) {
                    threads.push(t.clone());
                }
            }
            HeldComment::Reply { thread_id, comment } => {
                if let Some(t) = threads.iter_mut().find(|t| &t.id == thread_id) {
                    if !t.comments.iter().any(|c| c.id == comment.id) {
                        t.comments.push(comment.clone());
                    }
                }
            }
            HeldComment::Counted { held, .. } => held.show_on(threads),
        }
    }

    /// The id of the placeholder this shows.
    fn local_id(&self) -> &str {
        match self {
            HeldComment::Thread(t) => &t.id,
            HeldComment::Reply { comment, .. } => &comment.id,
            HeldComment::Counted { held, .. } => held.local_id(),
        }
    }
}

/// Placeholder ids start with this, then the action's token — so a refusal takes back exactly
/// the comments its own action added.
const LOCAL_ID: &str = "local-";

fn local_prefix(token: u64) -> String {
    format!("{LOCAL_ID}{token}-")
}

fn is_local(id: &str) -> bool {
    id.starts_with(LOCAL_ID)
}

/// Takes the placeholders whose ids start with `prefix` off `threads`.
fn drop_local(threads: &mut Vec<CommentThread>, prefix: &str) {
    threads.retain(|t| !t.id.starts_with(prefix));
    for t in threads.iter_mut() {
        t.comments.retain(|c| !c.id.starts_with(prefix));
    }
}

/// You, as a placeholder comment or reviewer entry shows you before the provider has.
fn me_user(me: Option<&str>) -> User {
    let name = me.unwrap_or("you");
    User { id: name.to_string(), display_name: name.to_string(), handle: me.map(str::to_string), avatar_url: None }
}

/// Records `vote` as your verdict on `pr`, adding you as a reviewer if you weren't one.
fn set_vote(pr: &mut PullRequest, me: &str, vote: ReviewVote) {
    match pr.reviewers.iter_mut().find(|r| forgetop_core::filter::is_user(&r.user, me)) {
        Some(r) => r.vote = vote,
        None => pr.reviewers.push(Reviewer { user: me_user(Some(me)), vote, is_required: false }),
    }
}

fn has_vote(pr: &PullRequest, me: &str, vote: ReviewVote) -> bool {
    pr.reviewers.iter().any(|r| r.vote == vote && forgetop_core::filter::is_user(&r.user, me))
}

/// Optimistic changes to one PR or work item, held against refreshes that haven't caught up.
/// Each part is let go as soon as a copy from the provider shows it, and all of it when the
/// hold runs out — then the provider has the last word.
struct ItemHold {
    /// Your verdict on a PR: `(token, your handle, verdict)`.
    vote: Option<(u64, String, ReviewVote)>,
    /// The vote (by token) that took the PR off the review-requested list, on a forge that
    /// stops asking for your review once you have given it. Held until a refetched row no
    /// longer names you as a reviewer.
    review_cleared: Option<u64>,
    /// Comments posted from here that the provider hasn't listed yet.
    comments: Vec<HeldComment>,
    /// Work-item edits the provider hasn't reflected yet, by token.
    edits: Vec<(u64, WiEdit)>,
    until: DateTime<Utc>,
}

impl ItemHold {
    fn new(until: DateTime<Utc>) -> Self {
        ItemHold { vote: None, review_cleared: None, comments: Vec::new(), edits: Vec::new(), until }
    }

    /// Lets go of what the refused action `token` held, returning its work-item edit.
    fn forget(&mut self, token: u64) -> Option<WiEdit> {
        if self.vote.as_ref().is_some_and(|(t, ..)| *t == token) {
            self.vote = None;
        }
        if self.review_cleared == Some(token) {
            self.review_cleared = None;
        }
        let prefix = local_prefix(token);
        self.comments.retain(|c| !c.local_id().starts_with(&prefix));
        let i = self.edits.iter().position(|(t, _)| *t == token)?;
        Some(self.edits.remove(i).1)
    }
}

/// How long an optimistic PR or work-item change is held against a provider still reporting
/// the old state.
const ITEM_HOLD_SECS: i64 = 60;

/// A PR or work-item write in flight: what to undo, and what to say when the provider answers.
struct PendingItemAction {
    conn_id: String,
    item: ItemRef,
    /// The item's detail cache key, which is also its hold's.
    key: String,
    /// What to say once the provider accepts it.
    done: String,
    /// How a refusal is introduced.
    failed: &'static str,
    /// A merge, which takes the PR off the Launchpad once the provider has done it.
    merge: bool,
    /// The PR's reviewers before an optimistic vote.
    reviewers: Option<Vec<Reviewer>>,
    /// The work item before an optimistic edit.
    wi: Option<WorkItem>,
    /// A review's comments, taken out of the buffer when it was sent.
    pending: Vec<LineComment>,
    /// This action took the PR off the Launchpad.
    dismissed: bool,
}

impl PendingItemAction {
    fn new(conn_id: String, item: ItemRef, key: String) -> Self {
        PendingItemAction {
            conn_id,
            item,
            key,
            done: String::new(),
            failed: "Failed",
            merge: false,
            reviewers: None,
            wi: None,
            pending: Vec::new(),
            dismissed: false,
        }
    }
}

/// The provider call behind a PR write, and the PR re-read once it is accepted.
async fn pr_action_call(
    deps: &AppDeps,
    conn_id: &str,
    item: &ItemRef,
    call: &PrCall,
) -> (std::result::Result<(), String>, Option<PullRequest>) {
    let feeds = detail_or_default(deps.sections.pull_request_feeds().await, DIAG_PR_FEEDS);
    let Some(source) = feeds.into_iter().find(|f| f.connection.connection_id() == conn_id).map(|f| f.source) else {
        return (Err("No pull-request provider is bound".into()), None);
    };
    let result = match call {
        PrCall::Vote(vote) => source.vote(item, *vote).await,
        PrCall::Merge(strategy) => source.merge(item, &MergeOptions { strategy: *strategy, delete_source_ref: false }).await,
        PrCall::Revert => source.revert(item).await,
        PrCall::Comment(body) => source.add_comment(item, body).await,
        PrCall::Reply { thread_id, body } => source.reply_to_thread(item, thread_id, body).await,
        PrCall::Review { event, comments } => source.submit_review(item, *event, comments).await,
    };
    match result {
        Ok(()) => (Ok(()), detail_or_none(source.get(item).await, DIAG_PR_DETAIL)),
        Err(e) => (Err(e.to_string()), None),
    }
}

/// The provider call behind a work-item write, and the item re-read once it is accepted.
async fn wi_action_call(
    deps: &AppDeps,
    conn_id: &str,
    item: &ItemRef,
    call: &WiCall,
) -> (std::result::Result<(), String>, Option<WorkItem>) {
    let feeds = detail_or_default(deps.sections.work_item_feeds().await, DIAG_WI_FEEDS);
    let Some(source) = feeds.into_iter().find(|f| f.connection.connection_id() == conn_id).map(|f| f.source) else {
        return (Err("No work-item provider is bound".into()), None);
    };
    let result = match call {
        WiCall::Edit(WiEdit::State(state)) => source.set_state(item, state).await,
        WiCall::Edit(WiEdit::Assignee(user)) => source.set_assignee(item, user.as_ref().map(|u| u.id.as_str())).await,
        WiCall::Edit(WiEdit::Title(title)) => source.update_fields(item, Some(title), None).await,
        WiCall::Edit(WiEdit::Description(text)) => source.update_fields(item, None, Some(text)).await,
        WiCall::Comment(body) => source.add_comment(item, body).await,
    };
    match result {
        Ok(()) => (Ok(()), detail_or_none(source.get(item).await, DIAG_WI_DETAIL)),
        Err(e) => (Err(e.to_string()), None),
    }
}

/// The provider call behind a rerun (`Some(failed_only)`) or a cancel (`None`).
async fn run_action_call(
    deps: &AppDeps,
    conn_id: &str,
    run: &ItemRef,
    rerun: Option<bool>,
) -> std::result::Result<Option<String>, String> {
    let Some(source) = pipeline_source(deps, conn_id).await else {
        return Err("pipeline connection not found".into());
    };
    match rerun {
        Some(failed_only) => source.rerun_run(run, failed_only).await,
        None => source.cancel_run(run).await.map(|()| None),
    }
    .map_err(|e| e.to_string())
}

/// A rerun as it will look once queued: the run (and the jobs it re-runs) back to Queued, their
/// times cleared, and the attempt counted up. The provider's answer replaces it on refresh.
/// It starts `now`, so it keeps its place at the new end of the history.
fn mark_rerun(run: &mut PipelineRun, failed_only: bool, now: DateTime<Utc>) {
    let redo = |s: PipelineRunStatus| !failed_only || matches!(s, PipelineRunStatus::Failed | PipelineRunStatus::Canceled);
    run.status = PipelineRunStatus::Queued;
    run.started_at = Some(now);
    run.finished_at = None;
    run.attempt = run.attempt.map(|n| n + 1);
    for stage in &mut run.stages {
        for job in stage.jobs.iter_mut().filter(|j| redo(j.status)) {
            job.status = PipelineRunStatus::Queued;
            job.started_at = None;
            job.finished_at = None;
            job.problem = None;
            for step in &mut job.steps {
                step.status = PipelineRunStatus::Queued;
                step.started_at = None;
                step.finished_at = None;
            }
        }
        if stage.jobs.iter().any(|j| j.status == PipelineRunStatus::Queued) {
            stage.status = PipelineRunStatus::Queued;
        }
    }
}

/// A cancel as it will look once it lands: whatever was still running or queued is Canceled.
fn mark_canceled(run: &mut PipelineRun, now: DateTime<Utc>) {
    if run.status.is_active() {
        run.status = PipelineRunStatus::Canceled;
        run.finished_at.get_or_insert(now);
    }
    for stage in &mut run.stages {
        for job in stage.jobs.iter_mut().filter(|j| j.status.is_active()) {
            job.status = PipelineRunStatus::Canceled;
            if job.started_at.is_some() {
                job.finished_at.get_or_insert(now);
            }
            for step in job.steps.iter_mut().filter(|s| s.status.is_active()) {
                step.status = PipelineRunStatus::Canceled;
            }
        }
        if stage.status.is_active() {
            stage.status = PipelineRunStatus::Canceled;
        }
    }
}

/// The pipeline source behind a connection, through the same feeds every pipeline call uses.
async fn pipeline_source(deps: &AppDeps, conn_id: &str) -> Option<Arc<dyn PipelineSource>> {
    let feeds = detail_or_default(deps.sections.pipeline_feeds().await, DIAG_PIPELINE_FEEDS);
    feeds.into_iter().find(|f| f.connection.connection_id() == conn_id).map(|f| f.source)
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
        p.shown_status(),
    )
    .to_lowercase();
    q.split_whitespace().all(|t| hay.contains(t))
}

// ---- sorting ----

/// The columns `c` can switch off, per section, by header name. Status and title always show.
pub const LIST_COLUMNS: [&[&str]; 3] = [
    &["Provider", "Repository", "#", "Author", "State", "±", "Updated"],
    &["Provider", "ID", "Type", "Assignee", "Updated"],
    &["Provider", "Repository", "Branch", "Commit", "Now", "Stages", "Took", "Started"],
];

/// Provider starts off everywhere: the repository already says where a row lives, and the
/// provider is a column of the same word down most accounts. On Pipelines the repository starts
/// off too — the pipeline name says what ran, and on a busy Azure org it is one project repeated
/// down every row.
fn default_hidden_columns() -> [Vec<String>; 3] {
    let provider = || vec!["Provider".to_string()];
    [provider(), provider(), vec!["Provider".to_string(), "Repository".to_string()]]
}

/// The display label of `section`'s active sort, for the list title ("· by Updated").
pub fn sort_label(section: usize, pref: &SortPref) -> Option<&'static str> {
    sort_cols(section).iter().find(|c| c.key == pref.key).map(|c| c.label)
}

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
        // Held on a gate: someone has to act, so it sorts with the failures.
        PipelineRunStatus::Waiting => 1,
        PipelineRunStatus::Canceled | PipelineRunStatus::Skipped => 2,
        PipelineRunStatus::Running => 3,
        PipelineRunStatus::Queued => 4,
        PipelineRunStatus::PartiallySucceeded => 5,
        PipelineRunStatus::Succeeded => 6,
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
        "status" => pipe_status_rank(a.shown_status()).cmp(&pipe_status_rank(b.shown_status())),
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

/// `catalog` collects each connection's discovered definitions, for the header's
/// "Pipelines · N of M" and the add/remove picker — only from discoveries that answered.
async fn fetch_pipelines(
    deps: &AppDeps,
    errors: &mut Vec<String>,
    catalog: &mut HashMap<String, Vec<PipelineDefinition>>,
) -> (Vec<PipeRow>, bool) {
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
                    Some(defs) => {
                        catalog.insert(conn_id.clone(), defs.clone());
                        defs
                    }
                    None => {
                        ok = false;
                        Vec::new()
                    }
                };
                let def_names: HashMap<String, String> =
                    defs.iter().map(|d| (d.id.clone(), d.name.clone())).collect();
                // An identity that can't be established owns no runs — see `run_triggered_by`. A
                // failed lookup would cache every row as someone else's, so it clears the section
                // flag like discovery's failure does.
                let me = match detail_or_none(feed.source.current_user().await, DIAG_PIPELINE_CURRENT_USER) {
                    Some(me) => me,
                    None => {
                        ok = false;
                        None
                    }
                };
                for result in feed.list_runs(&defs).await {
                    match result {
                        Ok(runs) => {
                            let supports = feed.source.supports_approvals();
                            for run in runs {
                                // A failed gate check is not "no gate": caching the row as clear
                                // would hide a pending approval until the next whole reload, so
                                // it clears the section flag like discovery's failure does.
                                let checked = if supports && run.status.is_active() {
                                    Some(detail_or_none(
                                        feed.source.pending_approvals(&run.item_ref()).await,
                                        DIAG_PIPELINE_APPROVALS,
                                    ))
                                } else {
                                    None
                                };
                                let gates: Vec<String> =
                                    checked.iter().flatten().flatten().filter(|a| a.blocks_run).map(|approval| approval.name.clone()).collect();
                                let (awaiting_approval, gate_ok) = checked.map_or((false, true), gate_from_approvals);
                                if !gate_ok {
                                    ok = false;
                                }
                                let definition_name = def_names.get(&run.definition_id).cloned();
                                let triggered_by_me = run_triggered_by(&run, me.as_deref());
                                out.push(PipeRow { connection_id: conn_id.clone(), connection: name.clone(), provider, run, definition_name, awaiting_approval, triggered_by_me, gates });
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
/// Approvals are fetched regardless of whether `get_run` itself succeeded — they're addressed by
/// the run ref, not the fetched run object, so a failure to enrich the run needn't also blank the
/// approvals answer; the two go out together. The run's problems are not fetched here — see
/// [`App::request_pipeline_detail`], which asks for them separately so they never delay the run.
async fn fetch_pipeline_detail(deps: &AppDeps, conn_id: &str, run_ref: &ItemRef) -> PipelineDetailFetch {
    let feeds = detail_or_default(deps.sections.pipeline_feeds().await, DIAG_PIPELINE_FEEDS);
    let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == conn_id) else {
        return PipelineDetailFetch::default();
    };
    let source = &feed.source;
    let supports_approvals = source.supports_approvals();
    let can_respond_approvals = source.can_respond_to_approvals();
    let approvals = async {
        if supports_approvals {
            detail_or_none(source.pending_approvals(run_ref).await, DIAG_PIPELINE_APPROVALS)
        } else {
            Some(Vec::new())
        }
    };
    let (run, approvals) = tokio::join!(source.get_run(run_ref), approvals);
    let run = detail_or_none(run, DIAG_PIPELINE_RUN);
    PipelineDetailFetch {
        run,
        approvals,
        supports_approvals: Some(supports_approvals),
        can_respond_approvals: Some(can_respond_approvals),
        annotations: None,
        supports_rerun: Some(source.supports_rerun()),
        supports_rerun_failed: Some(source.supports_rerun_failed()),
        supports_artifacts: Some(source.supports_artifacts()),
        rerun_new_run: Some(source.rerun_starts_new_run(false)),
        rerun_failed_new_run: Some(source.rerun_starts_new_run(true)),
    }
}

/// What a "viewed" mark is pinned to: the file's diff as it stood when it was marked.
fn file_fingerprint(f: &FileChange) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (f.additions, f.deletions, &f.patch).hash(&mut h);
    h.finish()
}

/// The marked paths still true of `files`: listed, with the same diff they were marked against.
fn still_viewed(marks: Option<&HashMap<String, u64>>, files: &[FileChange]) -> HashSet<String> {
    let Some(marks) = marks else { return HashSet::new() };
    files.iter().filter(|f| marks.get(&f.path) == Some(&file_fingerprint(f))).map(|f| f.path.clone()).collect()
}

/// Cheap "did anything actually change" check for a landed [`PrDetail`] against what the open
/// view already holds, so a revalidation that finds nothing new causes no repaint (no scroll
/// jump, no flicker). None of `CommentThread`, `FileChange`, `CheckRun` or `Commit` derive
/// `PartialEq` in forgetop-core, so this isn't a full structural diff — it compares the fields
/// most likely to actually move:
/// - threads: count, plus total comment count, plus resolved count (misses an edited comment
///   body, or a same-size swap of which threads are resolved)
/// - files: count, plus (path, kind, additions, deletions, patch) per file (a commit can rewrite
///   a line without moving the counts, and the open diff — and its "viewed" marks — must see it)
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
    // A placeholder swapped for the provider's copy of the same comment leaves every count as it
    // was, but the swap still has to land: the placeholder's id means nothing to the provider.
    let placeholders = |ts: &[CommentThread]| -> Vec<String> {
        ts.iter()
            .flat_map(|t| std::iter::once(&t.id).chain(t.comments.iter().map(|c| &c.id)))
            .filter(|id| is_local(id))
            .cloned()
            .collect()
    };
    a.len() == b.len() && counts(a) == counts(b) && placeholders(a) == placeholders(b)
}

fn files_match(a: &[FileChange], b: &[FileChange]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.path == y.path && x.kind == y.kind && x.additions == y.additions && x.deletions == y.deletions && x.patch == y.patch
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
        && v.annotations == d.annotations
        && v.supports_rerun == d.supports_rerun
        && v.supports_rerun_failed == d.supports_rerun_failed
        && v.supports_artifacts == d.supports_artifacts
        && v.rerun_new_run == d.rerun_new_run
        && v.rerun_failed_new_run == d.rerun_failed_new_run
}


/// The rows one PR view shows, derived from the pool — no network, no `&self`, so the
/// background fetch can use it too.
///
/// Reproduces what a per-filter `list` returned, step for step: the pool arrives in provider
/// order (fetched uncapped, so `sort_and_cap` sorted it but truncated nothing), this filters it
/// without reordering, and then caps. Order is preserved rather than re-sorted because
/// `pr_sort` defaults to `None`, which means "provider order" — imposing one here would
/// silently change the list for everyone who never chose a sort.
///
/// `review_cleared` holds the detail keys of PRs the user has just reviewed on a forge that
/// then stops listing them as review-requested ([`ItemHold::review_cleared`]); the
/// `ReviewRequested` view leaves those out ahead of the refetch that would.
fn derive_pool_rows(pool: &PrPool, filter: PullRequestFilter, completed: bool, review_cleared: &HashSet<String>) -> Vec<PrRow> {
    let mut per_connection: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::new();
    for row in pool.rows(completed) {
        let me = pool.me.get(&row.connection_id).and_then(|m| m.as_deref());
        if !pull_request_matches(&row.pr, filter, me) {
            continue;
        }
        if filter == PullRequestFilter::ReviewRequested
            && !review_cleared.is_empty()
            && review_cleared.contains(&pr_detail_cache_key(&row.connection_id, &row.pr.item_ref()))
        {
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

/// Adds to `rows` the `incoming` rows it does not already hold — same connection, repository and
/// id — and returns how many it added. When it added any, `rows` is re-ordered newest-updated
/// first (stably), so a view derived from it does not show the page's rows and then the searched
/// ones as two runs.
fn merge_pool_rows(rows: &mut Vec<PrRow>, incoming: Vec<PrRow>) -> usize {
    let key = |row: &PrRow| (row.connection_id.clone(), row.pr.repository.clone(), row.pr.id.clone());
    let mut held: HashSet<(String, Option<String>, String)> = rows.iter().map(key).collect();
    let mut added = 0;
    for row in incoming {
        if held.insert(key(&row)) {
            rows.push(row);
            added += 1;
        }
    }
    if added > 0 {
        rows.sort_by_key(|row| std::cmp::Reverse(row.pr.updated_at));
    }
    added
}

/// The filtered `list` calls a connection that targets its filters gets on top of the
/// unfiltered pair: the views the pool derives, each on the `include_completed` it derives on
/// (the list section and the review bucket on open rows, the Launchpad's "yours" bucket on
/// completed ones, which for every such provider include the open ones).
const POOL_TARGETED_VIEWS: [(PullRequestFilter, bool); 3] =
    [(PullRequestFilter::Mine, false), (PullRequestFilter::Mine, true), (PullRequestFilter::ReviewRequested, false)];

/// Fetches the pool both `include_completed` variants of every PR view derive from.
///
/// Two unfiltered `list` calls per connection, where there used to be four — the list section,
/// both Launchpad buckets and the notification scan each ran their own. The rows are identical
/// whichever view asked for them, *as long as the provider filters a page it fetched anyway*.
///
/// A provider whose `list` targets the filter itself ([`PullRequestSource::list_targets_filter`])
/// gets three more, for [`POOL_TARGETED_VIEWS`], merged into the pool without duplicates. Those
/// are the rows an unfiltered page cannot hold: on a busy repository the page is the newest 50,
/// and your own pull request leaves it as soon as 50 newer ones exist — "Mine" then went blank
/// although nothing about it had changed. The derived views still filter the merged pool with
/// `pull_request_matches`, so a searched row shows under exactly the views it belongs to.
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
        pool.review_clears_request.insert(conn_id.clone(), feed.source.review_clears_request());
        let targeted: &[(PullRequestFilter, bool)] =
            if feed.source.list_targets_filter() { &POOL_TARGETED_VIEWS } else { &[] };
        let queries = [(PullRequestFilter::All, false), (PullRequestFilter::All, true)].iter().chain(targeted.iter());
        // This connection's rows, merged on their own: a targeted view re-orders only the rows of
        // the connection it was fetched from, never another connection's.
        let (mut open, mut completed_rows) = (Vec::new(), Vec::new());
        for &(filter, completed) in queries {
            let query = PullRequestQuery { filter, include_completed: completed, limit: None, decorate: false };
            match feed.source.list(&query).await {
                Ok(list) => {
                    let rows = list
                        .into_iter()
                        .map(|pr| PrRow { connection_id: conn_id.clone(), connection: name.clone(), provider, pr })
                        .collect();
                    let into = if completed { &mut completed_rows } else { &mut open };
                    if filter == PullRequestFilter::All {
                        into.extend(rows);
                    } else {
                        merge_pool_rows(into, rows);
                    }
                }
                Err(e) => {
                    ok = false;
                    push_reload_error(errors, format!("PRs ({name}): {e}"), DIAG_RELOAD_PULL_REQUESTS);
                }
            }
        }
        pool.open.extend(open);
        pool.completed.extend(completed_rows);
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
/// Saved PR views that still open with the old stock trio (All, Mine, Review) get it in the
/// current order. Saving any view persists the whole list, stock views included, so without
/// this everyone who ever saved one would keep landing on All.
fn reorder_legacy_pr_views(mut views: Vec<SavedView>) -> Vec<SavedView> {
    let stock = |v: &SavedView, name: &str| {
        v.name == name && v.filter.as_deref() == Some(&name.to_lowercase()) && v.query.is_empty() && v.sort.is_none()
    };
    if views.len() >= 3 && stock(&views[0], "All") && stock(&views[1], "Mine") && stock(&views[2], "Review") {
        views[..3].rotate_left(1);
    }
    views
}

fn default_views(section: usize) -> Vec<SavedView> {
    let v = |name: &str, filter: Option<&str>| SavedView {
        name: name.into(),
        filter: filter.map(Into::into),
        query: String::new(),
        sort: None,
        hidden_states: Vec::new(),
    };
    match section {
        // Yours first, and where the tab lands: the two narrow views are what you open it for,
        // and both are derived from the one pool All is, so none costs a fetch of its own.
        0 => vec![v("Mine", Some("mine")), v("Review", Some("review")), v("All", Some("all"))],
        _ => vec![v("All", None)],
    }
}

fn wi_query() -> WorkItemQuery {
    // Work Items only ever shows items assigned to the authenticated user
    // (resolved from the token by each provider: @me / currentUser() / isMe).
    WorkItemQuery { mine_only: true, include_completed: false, limit: Some(50) }
}

/// Where a pipeline lives, for the picker: its project or repository, then — for Azure — the
/// folder it was filed under. The root folder (`\`) and a workflow file path add nothing.
fn pipeline_location(d: &PipelineDefinition) -> String {
    let folder = d.path.as_deref().filter(|p| p.starts_with('\\') && p.len() > 1);
    match (d.repository.as_deref(), folder) {
        (Some(repo), Some(folder)) => format!("{repo} {folder}"),
        (Some(repo), None) => repo.to_string(),
        (None, Some(folder)) => folder.to_string(),
        (None, None) => String::new(),
    }
}

/// A picker row: the pipeline's name, then where it lives.
fn pipeline_label(d: &PipelineDefinition) -> String {
    match pipeline_location(d) {
        loc if loc.is_empty() => d.name.clone(),
        loc => format!("{}  ·  {loc}", d.name),
    }
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
                pipe_catalog: HashMap::new(),
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

        app.patch_wi("c", &item.item_ref(), |wi| WiEdit::State("In Progress".into()).apply(wi));

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
        PrPool { open: rows.clone(), completed: rows, me, needs_decoration, review_clears_request: HashMap::new() }
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
            pipe_catalog: HashMap::new(),
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
            needs_decoration: ids.iter().map(|c| (c.clone(), false)).collect(),
            review_clears_request: ids.into_iter().map(|c| (c, false)).collect(),
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

    /// Switching view before the first fetch lands shows that view's cached rows rather than
    /// an empty list under a spinner — the pool they would be derived from isn't there yet.
    #[test]
    fn a_view_switch_before_the_first_fetch_shows_that_views_cached_rows() {
        let dir = std::env::temp_dir().join(format!("forgetop-view-cache-{}", std::process::id()));
        let cache = Arc::new(CacheStore::new(dir.join("cache.json")));
        cache.put(&prs_cache_key(PullRequestFilter::Mine, false), &vec![pr_row(pr(None))], Utc::now());
        let deps = deps_with_cache(cache);
        let mut app = App::new("slate");

        app.pr_filter = PullRequestFilter::Mine;
        app.refresh_derived_prs(&deps);
        assert_eq!(app.prs.len(), 1, "Mine's cached rows are on screen");

        app.pr_filter = PullRequestFilter::ReviewRequested;
        app.refresh_derived_prs(&deps);
        assert!(app.prs.is_empty(), "a view with nothing cached stays empty, never another view's rows");
        let _ = std::fs::remove_dir_all(dir);
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

    /// Launch derives every PR view from the cached pool. The per-view entries only ever held
    /// the view on screen when they were written, so landing on Mine with only All cached used
    /// to mean "Loading…" until the whole first refresh had answered.
    #[test]
    fn launch_derives_every_pr_view_from_the_cached_pool() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let mut writer = App::new("slate");
        writer.pr_filter = PullRequestFilter::All;
        writer.apply_reloaded(
            reloaded(vec![pool_pr("c", "1", "me", &[]), pool_pr("c", "2", "them", &["me"]), pool_pr("c", "3", "them", &[])]),
            &deps,
        );
        assert!(cache.get::<PrPool>(CACHE_KEY_PR_POOL).is_some(), "a whole reload caches the pool");

        let mut app = App::new("slate");
        app.pr_filter = PullRequestFilter::Mine;
        app.seed_from_cache(&deps);
        assert_eq!(app.prs.iter().map(|r| r.pr.id.as_str()).collect::<Vec<_>>(), ["1"], "Mine, though only All was on screen");

        app.pr_filter = PullRequestFilter::ReviewRequested;
        app.refresh_derived_prs(&deps);
        assert_eq!(app.prs.iter().map(|r| r.pr.id.as_str()).collect::<Vec<_>>(), ["2"], "a view switch derives too");
        assert!(app.data_age.is_some(), "and the seeded rows are reported as cached");
    }

    /// A cache written before the pool was still paints the view it holds.
    #[test]
    fn launch_falls_back_to_the_per_view_cache_without_a_pool() {
        let cache = memory_cache();
        cache.put(&prs_cache_key(PullRequestFilter::Mine, false), &vec![pool_pr("c", "1", "me", &[])], Utc::now());
        let deps = deps_with_cache(cache);
        let mut app = App::new("slate");
        app.pr_filter = PullRequestFilter::Mine;
        app.seed_from_cache(&deps);
        assert_eq!(app.prs.len(), 1);
        assert!(!app.pr_pool_loaded, "nothing to derive other views from");
    }

    /// Removing a connection drops its rows and identity from the cached pool, or the next
    /// launch seeds them straight back.
    #[test]
    fn purging_a_connection_prunes_the_cached_pool() {
        let cache = memory_cache();
        let pool = pool_of(vec![pool_pr("gone", "1", "me", &[]), pool_pr("kept", "2", "me", &[])], Some("me"));
        cache.put(CACHE_KEY_PR_POOL, &pool, Utc::now());
        purge_cached_rows(&cache, "gone");
        let pool = cache.get::<PrPool>(CACHE_KEY_PR_POOL).expect("still cached").value;
        assert!(pool.open.iter().chain(pool.completed.iter()).all(|r| r.connection_id == "kept"));
        assert!(!pool.me.contains_key("gone") && !pool.needs_decoration.contains_key("gone"));
    }

    /// The fetch hands its PR pool over before work items, pipelines, notifications, health
    /// and discovery have answered, and the list takes it without waiting for them.
    #[tokio::test]
    async fn the_pr_pool_lands_before_the_rest_of_the_reload() {
        let deps = test_deps();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut app = App::new("slate");
        app.job_tx = Some(tx);
        let _ = App::fetch_all(deps.clone(), app.reload_params()).await;
        assert!(matches!(rx.try_recv(), Ok(AppEvent::PrPoolLoaded { .. })), "the pool is sent from inside the fetch");

        app.loading = true;
        app.pr_filter = PullRequestFilter::Mine;
        let pool = pool_of(vec![pool_pr("c", "1", "me", &[]), pool_pr("c", "2", "them", &[])], Some("me"));
        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(pool), ok: true }, &deps);
        assert_eq!(app.prs.len(), 1, "Mine is on screen");
        assert!(app.loading && app.prs_landed, "while the rest of the reload is still out");

        app.request_reload(&deps);
        assert!(!app.prs_landed, "the next reload starts over");
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

        let mine = derive_pool_rows(&pool, PullRequestFilter::Mine, false, &HashSet::new());
        assert_eq!(mine.len(), PR_VIEW_CAP, "the cap counts filtered rows");
        assert!(mine.iter().all(|r| r.pr.id.starts_with('m')));
    }

    /// One busy connection must not crowd another off the list: each feed ran its own capped
    /// `list`, so the cap is per connection.
    #[test]
    fn the_view_cap_is_per_connection() {
        let mut rows: Vec<PrRow> = (0..60).map(|i| pool_pr("a", &format!("a{i}"), "me", &[])).collect();
        rows.extend((0..10).map(|i| pool_pr("b", &format!("b{i}"), "me", &[])));
        let derived = derive_pool_rows(&pool_of(rows, Some("me")), PullRequestFilter::Mine, false, &HashSet::new());
        assert_eq!(derived.iter().filter(|r| r.connection_id == "a").count(), PR_VIEW_CAP);
        assert_eq!(derived.iter().filter(|r| r.connection_id == "b").count(), 10, "b keeps all of its own");
    }

    /// An identity the provider could not establish means "cannot filter", not "nothing
    /// matches" — the same thing `list` already did, and the difference between an
    /// over-inclusive list and a silently empty one.
    #[test]
    fn a_connection_with_no_identity_shows_its_rows_rather_than_hiding_them() {
        let pool = pool_of(vec![pool_pr("c", "1", "them", &[]), pool_pr("c", "2", "other", &[])], None);
        assert_eq!(derive_pool_rows(&pool, PullRequestFilter::Mine, false, &HashSet::new()).len(), 2);
    }

    /// Bitbucket's completed state is `MERGED`, which excludes open PRs — so the two variants
    /// are held separately and the open views must not be derived from the completed pool.
    #[test]
    fn the_two_completed_variants_are_kept_apart() {
        let mut pool = pool_of(vec![pool_pr("c", "open", "me", &[])], Some("me"));
        pool.completed = vec![pool_pr("c", "merged", "me", &[])];
        assert_eq!(derive_pool_rows(&pool, PullRequestFilter::All, false, &HashSet::new())[0].pr.id, "open");
        assert_eq!(derive_pool_rows(&pool, PullRequestFilter::All, true, &HashSet::new())[0].pr.id, "merged");
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
                pipe_catalog: HashMap::new(),
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

    #[tokio::test]
    async fn d_dismisses_the_selected_notification_and_it_stays_hidden_across_reloads_and_restarts() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.screen = Screen::Inbox;
        app.inbox = vec![notif_row("a"), notif_row("b"), notif_row("c")];
        app.inbox_sel = 2;

        app.on_key(Key::Char('d'), &deps).await;

        let ids: Vec<&str> = app.inbox.iter().map(|r| r.notification.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"], "the selected row is gone");
        assert_eq!(app.inbox_sel, 1, "the selection stays in range");
        assert_eq!(app.toast.as_deref(), Some("Dismissed"));

        // The next poll returns it again; it must not come back.
        let mut r = reloaded(Vec::new());
        r.inbox = vec![notif_row("a"), notif_row("b"), notif_row("c")];
        app.apply_reloaded(r, &deps);
        assert_eq!(app.inbox.len(), 2, "a reload doesn't resurrect a dismissed notification");

        let saved = deps.config.snapshot().ui.dismissed_notifications;
        assert_eq!(saved.len(), 1, "the dismissal is written to config");
        let mut fresh = App::new("slate");
        fresh.apply_dismissed_notifications(&saved);
        let mut r = reloaded(Vec::new());
        r.inbox = vec![notif_row("a"), notif_row("c")];
        fresh.apply_reloaded(r, &deps);
        let ids: Vec<&str> = fresh.inbox.iter().map(|r| r.notification.id.as_str()).collect();
        assert_eq!(ids, ["a"], "a restarted app keeps the dismissal");
    }

    #[tokio::test]
    async fn shift_d_asks_then_dismisses_the_whole_inbox() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.screen = Screen::Inbox;
        app.inbox = vec![notif_row("a"), notif_row("b")];

        app.on_key(Key::Char('D'), &deps).await;
        assert!(matches!(&app.overlay, Some(Overlay::Confirm { action: Action::DismissAllInbox, .. })));
        assert_eq!(app.inbox.len(), 2, "nothing is dismissed before confirming");

        app.on_key(Key::Char('y'), &deps).await;
        assert!(app.inbox.is_empty());
        assert_eq!(app.unread_count(), 0, "dismissed notifications leave the unread count too");
        assert_eq!(deps.config.snapshot().ui.dismissed_notifications.len(), 2);
    }

    #[test]
    fn a_dismissed_thread_with_new_activity_returns_and_stale_dismissals_are_forgotten() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let mut old = notif_row("a");
        old.notification.updated_at = Some(Utc::now() - chrono::TimeDelta::hours(1));
        app.apply_dismissed_notifications(&[inbox_dismiss_key(&old), inbox_dismiss_key(&notif_row("gone"))]);

        let mut fresh = notif_row("a");
        fresh.notification.updated_at = Some(Utc::now());
        let mut r = reloaded(Vec::new());
        r.inbox = vec![fresh];
        app.apply_reloaded(r, &deps);

        assert_eq!(app.inbox.len(), 1, "new activity on a dismissed thread shows it again");
        assert!(app.inbox_dismissed.is_empty(), "keys the feed no longer returns are dropped");
    }

    #[test]
    fn a_failed_inbox_fetch_keeps_dismissals() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let key = inbox_dismiss_key(&notif_row("a"));
        app.apply_dismissed_notifications(std::slice::from_ref(&key));

        let mut r = reloaded(Vec::new());
        r.sections_ok = SectionsOk { inbox: false, ..SectionsOk::complete() };
        app.apply_reloaded(r, &deps);

        assert!(app.inbox_dismissed.contains(&key), "an outage is not evidence the notification is gone");
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
            event: None,
            attempt: None,
            pull_request: None,
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

        // A single-stage run has no stage row: node 0 is the job → its own deep link.
        assert_eq!(app.selected_url().as_deref(), Some("http://job"));
        // A node with no link of its own (past the end here) falls back to the run URL.
        if let Screen::Pipeline(v) = &mut app.screen {
            v.selected = 9;
        }
        assert_eq!(app.selected_url().as_deref(), Some("http://run"));

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
        view.approvals = vec![PipelineApproval { id: "prod".into(), name: "production".into(), can_respond: true, blocks_run: true }];
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
            PipelineApproval { id: "prod".into(), name: "production".into(), can_respond: true, blocks_run: true },
            PipelineApproval { id: "stg".into(), name: "staging".into(), can_respond: false, blocks_run: true },
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
            triggered_by_me: true,
            gates: Vec::new(),
            run: PipelineRun {
                event: None,
                attempt: None,
                pull_request: None,
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
            triggered_by_me: true,
            gates: Vec::new(),
            run: PipelineRun {
                event: None,
                attempt: None,
                pull_request: None,
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
            triggered_by_me: true,
            gates: Vec::new(),
            run: PipelineRun {
                event: None,
                attempt: None,
                pull_request: None,
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
        assert_eq!(default_views(0).iter().map(|v| v.name.clone()).collect::<Vec<_>>(), vec!["Mine", "Review", "All"]);
        assert_eq!(default_views(1).len(), 1);
        assert_eq!(parse_pr_filter(Some("mine")), PullRequestFilter::Mine);
        assert_eq!(parse_pr_filter(Some("review")), PullRequestFilter::ReviewRequested);
        assert_eq!(parse_pr_filter(None), PullRequestFilter::All);

        // Empty sections get the defaults; a saved set is kept verbatim.
        let mut app = App::new("slate");
        app.apply_views(vec![], vec![], vec![]);
        assert_eq!(app.views[0].len(), 3);
        assert_eq!(app.pr_filter, PullRequestFilter::Mine, "the tab lands on the first view, Mine");

        // A saved list that still opens with the old stock order is moved to the new one;
        // anything the user built after it stays where it was.
        let old: Vec<SavedView> = ["all", "mine", "review"]
            .iter()
            .map(|f| SavedView { name: format!("{}{}", f[..1].to_uppercase(), &f[1..]), filter: Some(f.to_string()), query: String::new(), sort: None, hidden_states: vec![] })
            .chain([saved_view("Hot")])
            .collect();
        app.apply_views(old, vec![], vec![]);
        assert_eq!(app.views[0].iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), vec!["Mine", "Review", "All", "Hot"]);
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
    fn files_detail(files: Vec<FileChange>) -> PrDetail {
        PrDetail { timeline: Vec::new(), threads: vec![], files, checks: vec![], commits: vec![] }
    }

    fn pr_id(id: &str) -> PullRequest {
        PullRequest { id: id.into(), number: id.parse().ok(), ..pr(None) }
    }

    /// Opens `pr` on its Diff tab, selects file `selected`, and presses `v`.
    async fn open_and_mark(app: &mut App, deps: &AppDeps, pr: PullRequest, selected: usize) {
        app.open_pr_view_for(deps, 3, "PR".into(), None, "c".into(), pr);
        if let Screen::PrView(v) = &mut app.screen {
            v.diff.selected = selected;
        }
        app.on_key(Key::Char('v'), deps).await;
    }

    fn viewed(app: &App) -> Vec<String> {
        let Screen::PrView(v) = &app.screen else { panic!("expected PrView") };
        let mut paths: Vec<String> = v.diff.files.iter().filter(|f| v.diff.is_viewed(&f.path)).map(|f| f.path.clone()).collect();
        paths.sort();
        paths
    }

    #[tokio::test]
    async fn viewed_marks_survive_closing_and_reopening_the_pr() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let mut app = App::new("slate");
        let files = vec![changed("a.rs", Some("@@ -1 +1 @@\n-x\n+y")), changed("b.rs", Some("@@ -1 +1 @@\n-p\n+q"))];
        for id in ["1", "2"] {
            cache.put(&pr_detail_cache_key("c", &pr_id(id).item_ref()), &files_detail(files.clone()), Utc::now());
        }

        open_and_mark(&mut app, &deps, pr_id("1"), 1).await;
        assert_eq!(viewed(&app), ["b.rs"]);
        app.screen = Screen::List; // closed

        app.open_pr_view_for(&deps, 3, "PR".into(), None, "c".into(), pr_id("1"));
        assert_eq!(viewed(&app), ["b.rs"], "reopened where it was left");
        app.open_pr_view_for(&deps, 3, "PR".into(), None, "c".into(), pr_id("2"));
        assert!(viewed(&app).is_empty(), "another PR keeps its own marks");

        // Unmarking is remembered too.
        open_and_mark(&mut app, &deps, pr_id("1"), 1).await;
        app.open_pr_view_for(&deps, 3, "PR".into(), None, "c".into(), pr_id("1"));
        assert!(viewed(&app).is_empty());
    }

    #[tokio::test]
    async fn a_file_whose_diff_changed_since_it_was_viewed_is_unmarked() {
        let cache = memory_cache();
        let deps = deps_with_cache(cache.clone());
        let mut app = App::new("slate");
        let p = pr_id("1");
        let key = pr_detail_cache_key("c", &p.item_ref());
        let b = changed("b.rs", Some("@@ -1 +1 @@\n-p\n+q"));
        cache.put(&key, &files_detail(vec![changed("a.rs", Some("@@ -1 +1 @@\n-x\n+y")), b.clone()]), Utc::now());
        open_and_mark(&mut app, &deps, p.clone(), 0).await;
        open_and_mark(&mut app, &deps, p.clone(), 1).await;
        assert_eq!(viewed(&app), ["a.rs", "b.rs"]);

        // A new commit rewrites a.rs; b.rs is untouched.
        let fresh = files_detail(vec![changed("a.rs", Some("@@ -1 +1 @@\n-x\n+z")), b]);
        app.on_event(AppEvent::PrDetailLoaded { key, detail: all_answered(fresh), fetched_at: Utc::now() }, &deps);
        assert_eq!(viewed(&app), ["b.rs"], "the changed file has to be looked at again");

        app.screen = Screen::List;
        app.open_pr_view_for(&deps, 3, "PR".into(), None, "c".into(), p);
        assert_eq!(viewed(&app), ["b.rs"], "and stays unmarked once reopened");
    }

    /// With no cached files (`--demo`, a cold start) the view opens empty; the marks come back
    /// once the fetch lists the files.
    #[tokio::test]
    async fn viewed_marks_return_when_the_files_arrive() {
        let deps = test_deps();
        let mut app = App::new("slate");
        let p = pr_id("1");
        let key = pr_detail_cache_key("c", &p.item_ref());
        let files = vec![changed("a.rs", Some("@@ -1 +1 @@\n-x\n+y")), changed("b.rs", None)];
        let load = |app: &mut App| {
            let detail = all_answered(files_detail(files.clone()));
            app.on_event(AppEvent::PrDetailLoaded { key: key.clone(), detail, fetched_at: Utc::now() }, &deps);
        };

        app.open_pr_view_for(&deps, 3, "PR".into(), None, "c".into(), p.clone());
        load(&mut app);
        app.on_key(Key::Char('v'), &deps).await;
        app.screen = Screen::List;

        app.open_pr_view_for(&deps, 3, "PR".into(), None, "c".into(), p);
        assert!(viewed(&app).is_empty(), "no files known yet");
        load(&mut app);
        assert_eq!(viewed(&app), ["a.rs"]);
    }

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
            event: None,
            attempt: None,
            pull_request: None,
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
        PipelineApproval { id: id.into(), name: format!("gate-{id}"), can_respond, blocks_run: true }
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
                    ..Default::default()
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
            annotations: Vec::new(), supports_rerun: false, supports_rerun_failed: false, supports_artifacts: false, rerun_new_run: false, rerun_failed_new_run: false,
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
                    ..Default::default()
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
                annotations: Vec::new(), supports_rerun: false, supports_rerun_failed: false, supports_artifacts: false, rerun_new_run: false, rerun_failed_new_run: false,
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
                    ..Default::default()
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
                    ..Default::default()
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
                annotations: Vec::new(), supports_rerun: false, supports_rerun_failed: false, supports_artifacts: false, rerun_new_run: false, rerun_failed_new_run: false,
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
                    ..Default::default()
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
            PipelineDetail { run: run.clone(), approvals: vec![], supports_approvals: false, can_respond_approvals: false, annotations: Vec::new(), supports_rerun: false, supports_rerun_failed: false, supports_artifacts: false, rerun_new_run: false, rerun_failed_new_run: false };
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
                annotations: Vec::new(), supports_rerun: false, supports_rerun_failed: false, supports_artifacts: false, rerun_new_run: false, rerun_failed_new_run: false,
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
                    ..Default::default()
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
        // Two stages, so the "build" stage has a row to collapse (a lone stage has none).
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
        view.logs = Some(LogView::with_lines("Logs · j1", "j1", vec!["hello".into()]));
        view.toggle_selected(); // collapses the "build" stage row (selected starts at 0)
        assert_eq!(view.folds.get("s0"), Some(&false), "sanity: the stage collapsed");
        app.screen = Screen::Pipeline(Box::new(view));

        let fresh = pipeline_run(
            "1",
            PipelineRunStatus::Succeeded,
            vec![
                pipeline_stage("build", PipelineRunStatus::Succeeded, vec![pipeline_job("j1", PipelineRunStatus::Succeeded)]),
                pipeline_stage("deploy", PipelineRunStatus::Succeeded, vec![pipeline_job("j2", PipelineRunStatus::Succeeded)]),
            ],
        );
        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key,
                detail: Box::new(PipelineDetailFetch {
                    run: Some(fresh),
                    approvals: Some(vec![]),
                    supports_approvals: Some(false),
                    can_respond_approvals: Some(false),
                    ..Default::default()
                }),
                fetched_at: Utc::now(),
            },
            &deps,
        );

        let Screen::Pipeline(v) = &app.screen else { panic!("expected Pipeline") };
        assert!(v.logs.is_some(), "an open log pane must not be torn down by a background refresh");
        assert_eq!(v.folds.get("s0"), Some(&false), "the expand/collapse tree state survives the patch");
        assert!(!v.flatten()[0].expanded, "and the stage still draws folded");
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
                    ..Default::default()
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
            triggered_by_me: true,
            gates: Vec::new(),
            run: PipelineRun {
                event: None,
                attempt: None,
                pull_request: None,
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

    /// The tab badge counts pipelines, not runs, and doesn't move with the grouping mode.
    #[test]
    fn pipeline_count_counts_pipelines_not_runs() {
        let mut app = App::new("slate");
        app.pipes = fanned_out();
        assert_eq!(app.pipes.len(), 4);
        assert_eq!(app.pipeline_count(), 3, "CI ran twice but is one pipeline");
        app.pipe_group = PipeGroup::Off;
        assert_eq!(app.pipeline_count(), 3);
        // Same definition in another repository is another pipeline.
        app.pipes.push(grouped_row("CI", "nz/other", "main", "ccc", "2026-09-24T08:00:00Z", PipelineRunStatus::Succeeded));
        assert_eq!(app.pipeline_count(), 4);
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
        app.preview_hidden[0] = false; // PRs don't preview by default; these tests are about the preview
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
    async fn enter_focuses_the_preview_when_it_is_showing_and_opens_full_screen_only_when_narrow() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.settle_preview(&deps);
        app.on_key(Key::Enter, &deps).await;
        assert!(app.preview_focus, "Enter moves context into the pane");
        assert!(matches!(app.screen, Screen::PrView(_)));

        let mut app = preview_app(&["1"]);
        app.content_w = PREVIEW_MIN_WIDTH - 1;
        app.on_key(Key::Enter, &deps).await;
        assert!(!app.preview_focus, "no room for the pane, so Enter opens the full view");
        assert!(matches!(app.screen, Screen::PrView(_)));
    }

    #[test]
    fn prs_and_work_items_open_on_enter_while_pipelines_preview_unasked() {
        let mut app = App::new("slate");
        assert_eq!(app.preview_hidden, [true, true, false]);
        app.apply_preview_hidden(Some(&[Section::Pipelines]));
        assert_eq!(app.preview_hidden, [false, false, true], "a saved choice wins over the defaults");
        app.apply_preview_hidden(Some(&[]));
        assert_eq!(app.preview_hidden, [false; 3], "switched back on everywhere stays on");
        app.apply_preview_hidden(None);
        assert_eq!(app.preview_hidden, DEFAULT_PREVIEW_HIDDEN, "never switched: the defaults");
    }

    #[tokio::test]
    async fn with_the_preview_off_enter_opens_the_pane_and_esc_closes_it() {
        let deps = test_deps();
        let mut app = preview_app(&["1", "2"]);
        app.preview_hidden[0] = true;
        app.settle_preview(&deps);
        assert!(app.preview.is_none(), "the list keeps the full width while browsing");

        app.on_key(Key::Down, &deps).await;
        app.on_key(Key::Enter, &deps).await;
        assert!(app.preview_focus, "Enter opens the pane beside the list");
        let Screen::PrView(v) = &app.screen else { panic!("the pane holds the PR view") };
        assert_eq!(v.pr.id, "2", "the row under the cursor");

        app.on_key(Key::Escape, &deps).await;
        app.settle_preview(&deps);
        assert!(matches!(app.screen, Screen::List));
        assert!(!app.preview_focus);
        assert!(app.preview.is_none(), "Esc closes the pane rather than leaving a preview behind");
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
    async fn tab_walks_a_pr_s_own_tabs_in_the_pane_and_the_strip_once_it_is_closed() {
        let deps = test_deps();
        let mut app = preview_app(&["1"]);
        app.on_key(Key::Enter, &deps).await;
        let tab = |app: &App| match &app.screen {
            Screen::PrView(v) => v.tab,
            _ => panic!("the PR stays open in the pane"),
        };
        for want in [1, 2, 3, 0] {
            app.on_key(Key::Tab, &deps).await;
            assert_eq!(tab(&app), want, "Tab steps the PR's tabs and wraps");
        }
        app.on_key(Key::BackTab, &deps).await;
        assert_eq!(tab(&app), 3, "Shift-Tab steps back, wrapping");
        assert_eq!(app.active, 0, "the top nav hasn't moved");
        assert!(app.preview_focus);

        app.on_key(Key::Escape, &deps).await;
        app.on_key(Key::Tab, &deps).await;
        assert_eq!(app.active, 1, "with the pane closed, Tab moves the top nav");
    }

    #[tokio::test]
    async fn tab_out_of_a_focused_pipeline_run_drops_focus() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.pipes = vec![pipe_row("r1", PipelineRunStatus::Failed, false)];
        app.active = 2;
        app.screen = Screen::List;
        app.pipe_group = PipeGroup::Off;
        app.pipe_state.select(Some(0));
        app.content_w = 150;
        app.on_key(Key::Enter, &deps).await;
        assert!(app.preview_focus, "the run opens in the pane");
        // A run has no tabs of its own, so Tab keeps walking the top nav.
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
        assert_eq!(deps.config.snapshot().ui.preview_hidden, Some(vec![Section::PullRequests, Section::WorkItems]));

        app.on_key(Key::Char('P'), &deps).await;
        assert!(!app.preview_hidden[0]);
        assert_eq!(deps.config.snapshot().ui.preview_hidden, Some(vec![Section::WorkItems]), "saved, not left to the defaults");
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

    #[tokio::test]
    async fn left_and_right_pan_the_focused_log_and_stop_at_the_widest_line() {
        let deps = test_deps();
        let long = format!("10:40:36  {}", "x".repeat(90));
        let mut app = log_app(LogView::with_lines("Logs", "j1", vec!["10:40:36  short".into(), long]), PipelineRunStatus::Succeeded);
        let pan = |app: &App| open_log(app).hscroll;
        if let Screen::Pipeline(v) = &mut app.screen {
            v.logs.as_mut().unwrap().viewport_w.set(40);
        }
        app.on_key(Key::Right, &deps).await;
        assert_eq!(pan(&app), 20, "half the pane per press");
        for _ in 0..5 {
            app.on_key(Key::Right, &deps).await;
        }
        assert_eq!(pan(&app), 60, "no further than brings the longest line's end into view");
        app.on_key(Key::Char('h'), &deps).await;
        assert_eq!(pan(&app), 40);
        for _ in 0..5 {
            app.on_key(Key::Left, &deps).await;
        }
        assert_eq!(pan(&app), 0);
        assert!(open_log(&app).effective_scroll() == 0 && matches!(app.screen, Screen::Pipeline(_)), "panning moves nothing else");
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
        let mut view = PipelineView::new("CI".into(), run, "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.folds.insert("s0.j0".into(), true); // a passed job starts folded; open it to reach its step
        app.screen = Screen::Pipeline(Box::new(view));
        // One stage: no stage row, so the cursor starts on job j1.
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
        view.folds.insert("s1".into(), false);
        view.folds.insert("s1.j0".into(), false);
        view.stale = true;
        view.auto_select();
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
        view.auto_select(); // the cache-seeded open
        view.apply_fresh_run(running, None); // the first live load
        view.apply_fresh_run(run, None); // a later refresh finds it failed
        assert_eq!(view.selected, 0);
        assert!(!view.auto_logs);
    }

    /// Build ✓ · Deploy Dev ✓ · Deploy Prod (queued, but held by a gate) · Post-deploy (queued):
    /// what the provider says while a run sits on its Prod approval.
    fn gated_pipeline() -> PipelineRun {
        let done = |id: &str| {
            let mut j = pipeline_job(id, PipelineRunStatus::Succeeded);
            j.steps = vec![PipelineStep { name: "work".into(), status: PipelineRunStatus::Succeeded, started_at: None, finished_at: None }];
            j
        };
        pipeline_run(
            "1",
            PipelineRunStatus::Running,
            vec![
                pipeline_stage("Build", PipelineRunStatus::Succeeded, vec![done("compile"), done("test")]),
                pipeline_stage("Deploy Dev", PipelineRunStatus::Succeeded, vec![done("dev")]),
                pipeline_stage("Deploy Prod", PipelineRunStatus::Queued, vec![]),
                pipeline_stage("Post-deploy", PipelineRunStatus::Queued, vec![]),
            ],
        )
    }

    fn prod_gate() -> Vec<PipelineApproval> {
        vec![PipelineApproval { id: "g".into(), name: "Deploy Prod".into(), can_respond: true, blocks_run: true }]
    }

    #[test]
    fn finished_stages_start_folded_and_the_gate_takes_the_cursor() {
        let mut view = PipelineView::new("CI".into(), gated_pipeline(), "c".into(), ProviderType::AzureDevOps, "ci".into(), None);
        view.supports_approvals = true;
        view.apply_fresh_run(gated_pipeline(), Some(prod_gate()));
        assert_eq!(view.shown_status(), PipelineRunStatus::Waiting, "a pending gate parks the run");
        assert_eq!(view.run.status, PipelineRunStatus::Running, "the stored run stays what the provider said");

        let nodes = view.flatten();
        let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Build",
                "Deploy Dev",
                "Deploy Prod",
                "approval required · its jobs start once approved",
                "o  open in Azure DevOps to approve",
                "Post-deploy"
            ],
            "finished stages fold, the gated stage opens on why it has no jobs"
        );
        assert!(!nodes[0].expanded && nodes[0].summary.as_ref().is_some_and(|(t, _)| t == "2 jobs"));
        assert_eq!(nodes[2].status, PipelineRunStatus::Waiting, "the gate's stage reads as waiting, not queued");
        assert!(nodes[3].info && nodes[4].info);
        assert_eq!(nodes[5].note.as_deref(), Some("after Deploy Prod"), "a queued stage says what it waits on");
        assert_eq!(view.selected, 2, "the cursor opens on the gate");

        // `o` on the gate (or its explanation) opens the run, since neither has a page of its own.
        let mut app = App::new("slate");
        view.run.url = Some("https://dev.azure.com/run/1".into());
        view.selected = 3;
        app.screen = Screen::Pipeline(Box::new(view));
        assert_eq!(app.selected_url().as_deref(), Some("https://dev.azure.com/run/1"));
    }

    #[test]
    fn a_gate_further_on_doesnt_park_a_run_that_is_still_running() {
        // Prod is gated, but a Dev job is still executing (parallel stages, or a GitLab manual job
        // later in the pipeline): the run is running, not waiting.
        let mut run = gated_pipeline();
        run.stages[1].status = PipelineRunStatus::Running;
        run.stages[1].jobs = vec![pipeline_job("dev", PipelineRunStatus::Running)];
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::AzureDevOps, "ci".into(), None);
        view.apply_fresh_run(run, Some(prod_gate()));
        assert_eq!(view.shown_status(), PipelineRunStatus::Running);
        assert_eq!(view.gate_stage(), None, "no gate row while work is still running");
    }

    #[test]
    fn a_cancelled_run_keeps_no_gate_row_even_if_a_stage_still_says_waiting() {
        let mut run = gated_pipeline();
        run.status = PipelineRunStatus::Canceled;
        run.stages[2].status = PipelineRunStatus::Waiting; // what the provider last reported
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::AzureDevOps, "ci".into(), None);
        view.apply_fresh_run(run, Some(vec![]));
        assert_eq!(view.gate_stage(), None);
        assert!(!view.flatten().iter().any(|n| n.info), "no 'approval required' row on a cancelled run");
    }

    #[test]
    fn the_cursor_stays_on_its_job_when_that_stage_finishes_and_folds() {
        // Build is running with the cursor on its `test` job; then Build passes and Deploy starts.
        let mut run = gated_pipeline();
        run.stages[0].status = PipelineRunStatus::Running;
        run.stages[0].jobs[1].status = PipelineRunStatus::Running;
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::AzureDevOps, "ci".into(), None);
        view.apply_fresh_run(run, Some(vec![]));
        let at = view.flatten().iter().position(|n| n.label == "test").expect("Build is open while it runs");
        view.selected = at;
        view.user_moved = true;

        let mut next = gated_pipeline();
        next.stages[1].status = PipelineRunStatus::Running;
        view.apply_fresh_run(next, Some(vec![]));
        let nodes = view.flatten();
        assert_eq!(nodes[view.selected].label, "test", "the cursor follows the node, not the row number");
        assert_eq!(view.folds.get("s0"), Some(&true), "Build is pinned open under the cursor");
    }

    #[test]
    fn sorting_and_filtering_the_list_go_by_the_status_it_shows() {
        let mut gated = pipe_row("1", PipelineRunStatus::Running, false);
        gated.gates = vec!["Deploy Prod".into()];
        let running = pipe_row("2", PipelineRunStatus::Running, false);
        assert_eq!(gated.shown_status(), PipelineRunStatus::Waiting);
        assert!(pipe_matches(&gated, "waiting") && !pipe_matches(&running, "waiting"));
        assert_eq!(pipe_cmp(&gated, &running, "status"), Ordering::Less, "waiting sorts ahead of running");
    }

    #[test]
    fn a_users_fold_survives_a_refresh_that_changes_the_default() {
        let mut view = PipelineView::new("CI".into(), gated_pipeline(), "c".into(), ProviderType::AzureDevOps, "ci".into(), None);
        view.apply_fresh_run(gated_pipeline(), Some(prod_gate()));
        view.selected = 0;
        view.toggle_selected(); // opens the folded Build stage
        assert_eq!(view.flatten()[1].label, "compile", "Build opened");

        // The gate is approved and Prod starts: the defaults move on, the user's choice doesn't.
        let mut next = gated_pipeline();
        next.stages[2].status = PipelineRunStatus::Running;
        next.stages[2].jobs = vec![pipeline_job("prod", PipelineRunStatus::Running)];
        view.apply_fresh_run(next.clone(), Some(vec![]));
        let nodes = view.flatten();
        assert!(nodes[0].expanded && nodes[1].label == "compile", "Build stays open");
        assert!(nodes.iter().any(|n| n.label == "prod"), "the running stage is open by default");

        // And a stage the user folded stays folded once it finishes.
        let at = nodes.iter().position(|n| n.label == "Deploy Prod").unwrap();
        view.selected = at;
        view.toggle_selected();
        next.stages[2].status = PipelineRunStatus::Succeeded;
        view.apply_fresh_run(next, Some(vec![]));
        let prod = view.flatten().into_iter().find(|n| n.label == "Deploy Prod").unwrap();
        assert!(!prod.expanded);
        assert_eq!(view.folds.get("s2"), Some(&false));
    }

    #[test]
    fn a_failed_run_folds_its_passed_jobs_and_its_skipped_stages() {
        let mut ok = pipeline_job("compile", PipelineRunStatus::Succeeded);
        ok.steps = vec![PipelineStep { name: "build".into(), status: PipelineRunStatus::Succeeded, started_at: None, finished_at: None }];
        let mut broken = pipeline_job("unit", PipelineRunStatus::Failed);
        broken.steps = vec![PipelineStep { name: "dotnet test".into(), status: PipelineRunStatus::Failed, started_at: None, finished_at: None }];
        let mut stages = vec![pipeline_stage("Build", PipelineRunStatus::Failed, vec![ok, broken])];
        for name in ["Dev", "Staging", "Prod"] {
            stages.push(pipeline_stage(name, PipelineRunStatus::Skipped, vec![]));
        }
        let mut view = PipelineView::new("CI".into(), pipeline_run("1", PipelineRunStatus::Failed, stages), "c".into(), ProviderType::AzureDevOps, "ci".into(), None);
        let nodes = view.flatten();
        let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, vec!["Build", "compile", "unit", "dotnet test", "3 stages skipped"]);
        assert!(!nodes[1].expanded, "the passed job starts folded");
        assert_eq!(nodes[4].note.as_deref(), Some("Build failed"));
        assert_eq!(nodes[4].status, PipelineRunStatus::Skipped);

        view.selected = 4;
        view.toggle_selected();
        let nodes = view.flatten();
        assert_eq!(nodes.iter().skip(5).map(|n| (n.label.as_str(), n.depth)).collect::<Vec<_>>(), vec![("Dev", 1), ("Staging", 1), ("Prod", 1)]);

        // One skipped stage on its own is just itself.
        view.run.stages.truncate(2);
        let last = view.flatten().pop().unwrap();
        assert_eq!((last.label.as_str(), last.status, last.group), ("Dev", PipelineRunStatus::Skipped, false));
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
        palette_run(&mut app, "?refresh", &deps).await;
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

    // ---- run pane: history, estimates, folding, log sections, and the write keys ----

    /// What the recording provider was asked to do, and how it answers.
    #[derive(Clone, Default)]
    struct PipeCalls {
        log: Arc<std::sync::Mutex<Vec<String>>>,
        refuse: bool,
        rerun: bool,
        artifacts: bool,
        /// A rerun starts this new run instead of re-queueing the old one.
        new_run: Option<String>,
        /// When set, a rerun waits for this before answering — to see the pane mid-flight.
        gate: Option<Arc<tokio::sync::Notify>>,
    }

    impl PipeCalls {
        fn calls(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    struct MockPipeSource(PipeCalls);

    impl MockPipeSource {
        fn record(&self, call: String) -> forgetop_core::Result<()> {
            self.0.log.lock().unwrap().push(call);
            if self.0.refuse {
                Err(forgetop_core::Error::Provider("the provider said no".into()))
            } else {
                Ok(())
            }
        }
    }

    #[async_trait::async_trait]
    impl PipelineSource for MockPipeSource {
        async fn discover(&self) -> forgetop_core::Result<Vec<PipelineDefinition>> {
            Ok(Vec::new())
        }
        async fn list_runs(&self, _query: &PipelineRunQuery) -> forgetop_core::Result<Vec<PipelineRun>> {
            Ok(Vec::new())
        }
        async fn get_run(&self, run: &ItemRef) -> forgetop_core::Result<PipelineRun> {
            Err(forgetop_core::Error::NotFound(run.id.clone()))
        }
        async fn logs(&self, _run: &ItemRef, _job_id: Option<&str>) -> forgetop_core::Result<String> {
            Ok(String::new())
        }
        async fn trigger(&self, _definition: &ItemRef, _branch: Option<&str>) -> forgetop_core::Result<()> {
            Ok(())
        }
        async fn cancel_run(&self, run: &ItemRef) -> forgetop_core::Result<()> {
            self.record(format!("cancel {}", run.id))
        }
        fn supports_rerun(&self) -> bool {
            self.0.rerun
        }
        fn supports_rerun_failed(&self) -> bool {
            self.0.rerun
        }
        fn rerun_starts_new_run(&self, _failed_only: bool) -> bool {
            self.0.new_run.is_some()
        }
        async fn rerun_run(&self, run: &ItemRef, failed_only: bool) -> forgetop_core::Result<Option<String>> {
            if let Some(gate) = &self.0.gate {
                gate.notified().await;
            }
            self.record(format!("rerun {} failed_only={failed_only}", run.id))?;
            Ok(self.0.new_run.clone())
        }
        fn supports_artifacts(&self) -> bool {
            self.0.artifacts
        }
        async fn artifacts(&self, run: &ItemRef) -> forgetop_core::Result<Vec<PipelineArtifact>> {
            self.record(format!("artifacts {}", run.id))?;
            Ok(vec![PipelineArtifact {
                id: "a1".into(),
                name: "forgetop.tar.xz".into(),
                size_bytes: Some(8_000_000),
                expires_at: None,
                url: Some("https://example.test/a1".into()),
            }])
        }
    }

    struct MockPipeConn(PipeCalls, Capabilities);

    #[async_trait::async_trait]
    impl ProviderConnection for MockPipeConn {
        fn connection_id(&self) -> &str {
            "c"
        }
        fn provider_type(&self) -> ProviderType {
            ProviderType::GitHub
        }
        fn display_name(&self) -> &str {
            "GH"
        }
        fn capabilities(&self) -> &Capabilities {
            &self.1
        }
        fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
            None
        }
        fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
            None
        }
        fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
            Some(Arc::new(MockPipeSource(self.0.clone())))
        }
        async fn check(&self) -> bool {
            true
        }
    }

    struct MockPipeFactory(PipeCalls);

    impl ProviderFactory for MockPipeFactory {
        fn provider_type(&self) -> ProviderType {
            ProviderType::GitHub
        }
        fn describe_capabilities(&self) -> Capabilities {
            Capabilities { supports_pipelines: true, ..Capabilities::default() }
        }
        fn create(&self, _connection: &Connection, _secret: Option<String>) -> forgetop_core::Result<Arc<dyn ProviderConnection>> {
            Ok(Arc::new(MockPipeConn(self.0.clone(), self.describe_capabilities())))
        }
    }

    /// Deps whose one pipeline connection, `c`, is the recording provider.
    async fn deps_with_pipes(calls: PipeCalls) -> AppDeps {
        use forgetop_core::config::InMemoryConfigStore;
        use forgetop_core::secret::InMemorySecretStore;
        use forgetop_core::service::ConnectionResolver;

        let registry = Arc::new(ProviderRegistry::new(vec![Arc::new(MockPipeFactory(calls))]));
        let secrets = Arc::new(InMemorySecretStore::default());
        let config = Arc::new(ConfigService::new(Arc::new(InMemoryConfigStore::default()), secrets.clone(), registry.clone()));
        let connection = Connection {
            id: "c".into(),
            provider_type: ProviderType::GitHub,
            display_name: "GH".into(),
            base_url: None,
            organization: None,
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
            repo_scope: None,
        };
        config.add_or_update_connection(connection, None).await.unwrap();
        config.subscribe_pipeline("c", "ci").await.unwrap();
        let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry, secrets));
        AppDeps {
            sections: Arc::new(SectionService::new(config.clone(), resolver.clone())),
            health: Arc::new(ConnectionHealthService::new(config.clone(), resolver)),
            config,
            cache: Arc::new(CacheStore::disabled()),
        }
    }

    fn row_of(run: PipelineRun) -> PipeRow {
        PipeRow {
            connection_id: "c".into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            run,
            definition_name: Some("CI".into()),
            awaiting_approval: false,
            triggered_by_me: true,
            gates: Vec::new(),
        }
    }

    /// The Pipelines tab with `run` open in its pane and listed.
    fn run_pane_app(run: PipelineRun) -> App {
        let mut app = App::new("slate");
        app.clipboard = |_| Ok(());
        app.active = 2;
        app.pipes.push(row_of(run.clone()));
        let title = format!("CI #{}", run.number.unwrap_or(0));
        app.screen = Screen::Pipeline(Box::new(PipelineView::new(title, run, "c".into(), ProviderType::GitHub, "ci".into(), None)));
        app
    }

    fn pane(app: &App) -> &PipelineView {
        let Screen::Pipeline(v) = &app.screen else { panic!("expected the pipeline screen") };
        v
    }

    fn pane_mut(app: &mut App) -> &mut PipelineView {
        let Screen::Pipeline(v) = &mut app.screen else { panic!("expected the pipeline screen") };
        v
    }

    fn secs_ago(s: i64) -> DateTime<Utc> {
        Utc::now() - chrono::Duration::seconds(s)
    }

    #[test]
    fn base64_matches_the_rfc_4648_vectors() {
        for (plain, encoded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64_encode(plain.as_bytes()), encoded, "{plain}");
        }
    }

    static FEEDBACK_OPENED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    #[tokio::test]
    async fn f_reruns_the_failed_jobs_only_where_it_can_and_the_pane_shows_it_queued_before_the_provider_answers() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let calls = PipeCalls { rerun: true, gate: Some(gate.clone()), ..PipeCalls::default() };
        let deps = deps_with_pipes(calls.clone()).await;
        let mut run = failed_run();
        run.attempt = Some(1);
        run.commit_sha = Some("9c0e1d2aa".into());
        let mut app = run_pane_app(run);
        app.feedback_opener = |_| {
            FEEDBACK_OPENED.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        };

        // A provider that can't rerun failed jobs leaves F to the feedback form.
        app.on_key(Key::Char('F'), &deps).await;
        assert!(app.overlay.is_none());
        assert!(FEEDBACK_OPENED.load(std::sync::atomic::Ordering::SeqCst), "F falls through to feedback");

        pane_mut(&mut app).supports_rerun = true;
        pane_mut(&mut app).supports_rerun_failed = true;
        pane_mut(&mut app).annotations = vec![PipelineAnnotation {
            level: AnnotationLevel::Failure,
            message: "old attempt".into(),
            title: None,
            path: None,
            line: None,
            job_id: None,
        }];
        app.on_key(Key::Char('F'), &deps).await;
        match &app.overlay {
            Some(Overlay::Confirm { title, message, action: Action::PipelineRerun { failed_only: true, new_run: false, run_id, .. } }) => {
                assert_eq!(title, "Rerun failed jobs");
                assert_eq!(run_id, "r1");
                assert!(message.contains("✗ unit"), "names the failed job: {message}");
                assert!(message.contains("Same commit 9c0e1d2 · becomes attempt 2"), "{message}");
            }
            _ => panic!("expected the rerun confirm"),
        }

        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        app.on_key(Key::Char('y'), &deps).await;
        // The provider hasn't answered — it is still waiting on the gate — and the pane already
        // shows the rerun.
        assert!(calls.calls().is_empty(), "the provider hasn't been asked yet");
        assert_eq!(pane(&app).run.status, PipelineRunStatus::Queued, "the pane shows it queued");
        assert!(pane(&app).run.started_at.is_some(), "it starts now, keeping its place in the history");
        assert_eq!(pane(&app).run.attempt, Some(2));
        assert_eq!(pane(&app).run.stages[0].jobs[0].status, PipelineRunStatus::Queued, "the failed job re-queued");
        assert_eq!(app.pipes[0].run.status, PipelineRunStatus::Queued, "and so does the list row");
        assert!(pane(&app).annotations.is_empty(), "the old attempt's problems are gone");

        gate.notify_one();
        let event = loop {
            let event = rx.recv().await.expect("the provider's answer");
            if matches!(event, AppEvent::PipelineRunActionDone { .. }) {
                break event;
            }
        };
        app.on_event(event, &deps);
        assert_eq!(calls.calls(), vec!["rerun r1 failed_only=true".to_string()]);
        assert!(app.toast.as_deref().is_some_and(|t| t.starts_with("Rerunning the failed jobs of CI #1")), "{:?}", app.toast);

        // A refresh that still reports the old attempt doesn't flip the pane back.
        let key = pipeline_detail_cache_key("c", &pane(&app).run.item_ref());
        app.on_event(
            AppEvent::PipelineDetailLoaded {
                key,
                detail: Box::new(PipelineDetailFetch { run: Some(failed_run()), ..Default::default() }),
                fetched_at: Utc::now(),
            },
            &deps,
        );
        assert_eq!(pane(&app).run.status, PipelineRunStatus::Queued, "held until the provider catches up");
    }

    #[tokio::test]
    async fn a_rerun_that_starts_a_new_run_leaves_the_old_one_alone_and_follows_the_new_one() {
        let calls = PipeCalls { rerun: true, new_run: Some("r2".into()), ..PipeCalls::default() };
        let deps = deps_with_pipes(calls.clone()).await;
        let mut app = run_pane_app(failed_run());
        pane_mut(&mut app).supports_rerun = true;
        pane_mut(&mut app).rerun_new_run = true;
        app.on_key(Key::Char('R'), &deps).await;
        match &app.overlay {
            Some(Overlay::Confirm { message, action: Action::PipelineRerun { new_run: true, .. }, .. }) => {
                assert!(message.contains("Starts a new run on main"), "{message}");
                assert!(!message.contains("attempt"), "{message}");
            }
            _ => panic!("expected the rerun confirm"),
        }
        app.on_key(Key::Char('y'), &deps).await;
        assert_eq!(calls.calls(), vec!["rerun r1 failed_only=false".to_string()]);
        assert_eq!(pane(&app).run.status, PipelineRunStatus::Failed, "the old run isn't marked re-queued");
        assert!(app.toast.as_deref().is_some_and(|t| t.contains("Started a new run") && t.contains("r2")), "{:?}", app.toast);

        // The next reload lists the new run: the pane moves to it.
        let mut next = failed_run();
        next.id = "r2".into();
        next.number = Some(2);
        next.status = PipelineRunStatus::Queued;
        let mut r = reloaded_with_health(Vec::new());
        r.pipes = vec![row_of(failed_run()), row_of(next)];
        app.on_event(AppEvent::Reloaded(Box::new(r)), &deps);
        assert_eq!(pane(&app).run.id, "r2", "the pane follows the new run");
        assert!(app.toast.as_deref().is_some_and(|t| t.contains("the new run")), "{:?}", app.toast);
    }

    #[tokio::test]
    async fn a_refused_rerun_puts_the_run_back_and_says_why() {
        let calls = PipeCalls { rerun: true, refuse: true, ..PipeCalls::default() };
        let deps = deps_with_pipes(calls.clone()).await;
        let mut app = run_pane_app(failed_run());
        pane_mut(&mut app).supports_rerun = true;
        app.on_key(Key::Char('R'), &deps).await;
        assert!(matches!(&app.overlay, Some(Overlay::Confirm { action: Action::PipelineRerun { failed_only: false, .. }, .. })));
        app.on_key(Key::Char('y'), &deps).await;
        assert_eq!(calls.calls(), vec!["rerun r1 failed_only=false".to_string()]);
        assert_eq!(pane(&app).run.status, PipelineRunStatus::Failed, "rolled back");
        assert_eq!(pane(&app).run.stages[0].jobs[0].status, PipelineRunStatus::Failed);
        assert_eq!(app.pipes[0].run.status, PipelineRunStatus::Failed);
        assert!(app.toast.as_deref().is_some_and(|t| t.contains("Rerun failed") && t.contains("the provider said no")), "{:?}", app.toast);
    }

    #[tokio::test]
    async fn cancel_is_offered_only_on_a_live_run_and_lands_before_the_provider_answers() {
        let calls = PipeCalls::default();
        let deps = deps_with_pipes(calls.clone()).await;
        let mut app = run_pane_app(failed_run());
        app.on_key(Key::Char('X'), &deps).await;
        assert!(app.overlay.is_none());
        assert_eq!(app.toast.as_deref(), Some("Only a queued or running run can be cancelled"));

        let mut running = pipeline_run("r9", PipelineRunStatus::Running, vec![pipeline_stage("build", PipelineRunStatus::Running, vec![pipeline_job("j1", PipelineRunStatus::Running)])]);
        running.number = Some(9);
        let mut app = run_pane_app(running);
        pane_mut(&mut app).supports_rerun = true;
        app.on_key(Key::Char('R'), &deps).await;
        assert!(app.overlay.is_none(), "a live run can't be rerun");
        assert!(app.toast.as_deref().is_some_and(|t| t.contains("still running")));

        app.on_key(Key::Char('X'), &deps).await;
        assert!(matches!(&app.overlay, Some(Overlay::Confirm { action: Action::PipelineCancel { .. }, .. })));
        app.on_key(Key::Char('y'), &deps).await;
        assert_eq!(calls.calls(), vec!["cancel r9".to_string()]);
        assert_eq!(pane(&app).run.status, PipelineRunStatus::Canceled);
        assert_eq!(pane(&app).run.stages[0].jobs[0].status, PipelineRunStatus::Canceled);
        assert_eq!(app.toast.as_deref(), Some("Cancelled CI #9"));
    }

    #[tokio::test]
    async fn artifacts_load_in_the_background_copy_their_link_and_close() {
        let calls = PipeCalls { artifacts: true, ..PipeCalls::default() };
        let deps = deps_with_pipes(calls.clone()).await;
        let mut app = run_pane_app(failed_run());
        app.on_key(Key::Char('a'), &deps).await;
        assert!(pane(&app).artifacts.is_none());
        assert!(app.toast.as_deref().is_some_and(|t| t.contains("Artifacts aren't supported for GitHub")));

        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        pane_mut(&mut app).supports_artifacts = true;
        app.on_key(Key::Char('a'), &deps).await;
        assert!(pane(&app).artifacts.as_ref().is_some_and(|p| p.items.is_none()), "loading");
        let event = rx.recv().await.expect("the artifacts answer");
        assert!(matches!(event, AppEvent::PipelineArtifactsLoaded { .. }));
        app.on_event(event, &deps);
        assert_eq!(calls.calls(), vec!["artifacts r1".to_string()]);
        let panel = pane(&app).artifacts.as_ref().expect("still open");
        assert_eq!(panel.items.as_ref().map(Vec::len), Some(1));

        app.on_key(Key::Char('y'), &deps).await;
        assert_eq!(app.toast.as_deref(), Some("Copied artifact link"));
        app.on_key(Key::Char('j'), &deps).await; // one row: nowhere to go
        assert_eq!(pane(&app).artifacts.as_ref().map(|p| p.selected), Some(0));
        app.on_key(Key::Escape, &deps).await;
        assert!(pane(&app).artifacts.is_none(), "Esc closes the list, not the run");
    }

    static COPIED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

    #[tokio::test]
    async fn c_copies_the_full_commit_sha() {
        let deps = test_deps();
        let mut run = failed_run();
        run.commit_sha = Some("1a4c3aeb2f0d".into());
        let mut app = run_pane_app(run);
        app.clipboard = |text| {
            *COPIED.lock().unwrap() = Some(text.to_string());
            Ok(())
        };
        app.on_key(Key::Char('c'), &deps).await;
        assert_eq!(COPIED.lock().unwrap().as_deref(), Some("1a4c3aeb2f0d"));
        assert_eq!(app.toast.as_deref(), Some("Copied commit 1a4c3ae"));

        let mut app = run_pane_app(failed_run());
        app.on_key(Key::Char('c'), &deps).await;
        assert_eq!(app.toast.as_deref(), Some("This run has no commit sha"));
    }

    /// Runs `n` of CI on main, `i` hours apart, succeeded unless listed in `failed`.
    fn ci_history(n: i64, failed: &[i64], secs: impl Fn(i64) -> i64) -> Vec<PipeRow> {
        (1..=n)
            .map(|i| {
                let status = if failed.contains(&i) { PipelineRunStatus::Failed } else { PipelineRunStatus::Succeeded };
                let mut run = pipeline_run(&format!("r{i}"), status, vec![]);
                run.number = Some(i);
                run.started_at = Some(secs_ago((n - i + 1) * 3600));
                run.finished_at = run.started_at.map(|s| s + chrono::Duration::seconds(secs(i)));
                row_of(run)
            })
            .collect()
    }

    #[test]
    fn history_measures_the_run_against_the_median_of_the_others_and_names_the_last_failure() {
        let mut app = App::new("slate");
        app.pipes = ci_history(5, &[3], |i| [0, 100, 120, 90, 110, 200][i as usize]);
        // Another branch and another pipeline stay out of it.
        let mut elsewhere = ci_history(1, &[], |_| 5).remove(0);
        elsewhere.run.branch = Some("dev".into());
        elsewhere.run.id = "other".into();
        app.pipes.push(elsewhere);
        let run = app.pipes[4].run.clone();
        app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI #5".into(), run, "c".into(), ProviderType::GitHub, "ci".into(), None)));
        app.refresh_pipeline_context();

        let h = pane(&app).history.clone().expect("history");
        assert_eq!(h.entries.iter().map(|e| e.number.unwrap()).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5], "oldest first");
        assert_eq!(h.current, Some(4));
        assert_eq!(h.median_secs, Some(110), "the median of 100, 120, 110 — not the failure, not this run");
        assert_eq!(h.median_n, 3);
        assert_eq!(h.last_failure.as_ref().and_then(|f| f.number), Some(3));

        // A long history keeps the ten around the open run, the open run always among them.
        let mut app = App::new("slate");
        app.pipes = ci_history(14, &[], |_| 60);
        let run = app.pipes[1].run.clone();
        app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI #2".into(), run, "c".into(), ProviderType::GitHub, "ci".into(), None)));
        app.refresh_pipeline_context();
        let h = pane(&app).history.clone().expect("history");
        assert_eq!(h.entries.len(), HISTORY_LEN);
        assert_eq!(h.total, 14);
        assert_eq!(h.current.map(|c| h.entries[c].run_id.clone()).as_deref(), Some("r2"));
    }

    #[tokio::test]
    async fn left_and_right_open_the_older_and_newer_run_and_the_list_selection_follows() {
        let deps = test_deps();
        let mut app = App::new("slate");
        app.active = 2;
        app.pipes = ci_history(3, &[], |_| 60);
        app.pipe_state.select(Some(0)); // the collapsed CI group
        let run = app.pipes[2].run.clone();
        app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI #3".into(), run, "c".into(), ProviderType::GitHub, "ci".into(), None)));
        app.refresh_pipeline_context();

        app.on_key(Key::Left, &deps).await;
        assert_eq!(pane(&app).run.id, "r2", "the older run is open");
        let sel = app.pipe_state.selected().expect("a selection");
        assert!(matches!(app.pipe_lines().get(sel), Some(PipeLine::Run(i)) if app.pipes[*i].run.id == "r2"), "the list points at it");
        assert_eq!(pane(&app).history.as_ref().and_then(|h| h.current), Some(1));

        app.on_key(Key::Left, &deps).await;
        app.on_key(Key::Left, &deps).await;
        assert_eq!(pane(&app).run.id, "r1");
        assert_eq!(app.toast.as_deref(), Some("No older run of this pipeline on this branch"));
        app.on_key(Key::Right, &deps).await;
        app.on_key(Key::Right, &deps).await;
        app.on_key(Key::Right, &deps).await;
        assert_eq!(pane(&app).run.id, "r3");
        assert_eq!(app.toast.as_deref(), Some("This is the newest run on this branch"));
    }

    #[test]
    fn estimates_take_job_medians_from_recent_succeeded_runs_of_the_same_pipeline() {
        let mut app = App::new("slate");
        app.pipes = ci_history(4, &[], |i| 100 + i * 10);
        let mut running = pipeline_run("live", PipelineRunStatus::Running, vec![pipeline_stage("build", PipelineRunStatus::Running, vec![pipeline_job("build", PipelineRunStatus::Running)])]);
        running.started_at = Some(secs_ago(30));
        app.pipes.push(row_of(running.clone()));
        // The three most recent succeeded runs' details, as the background fetch leaves them.
        for (i, secs) in [(4, 60), (3, 80), (2, 100)] {
            let mut job = pipeline_job("build", PipelineRunStatus::Succeeded);
            job.started_at = Some(secs_ago(1000));
            job.finished_at = job.started_at.map(|s| s + chrono::Duration::seconds(secs));
            let mut run = app.pipes[i - 1].run.clone();
            run.stages = vec![pipeline_stage("build", PipelineRunStatus::Succeeded, vec![job])];
            app.run_details.insert(pipeline_detail_cache_key("c", &run.item_ref()), run);
        }
        app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI".into(), running, "c".into(), ProviderType::GitHub, "ci".into(), None)));
        app.refresh_pipeline_context();
        let e = &pane(&app).estimates;
        assert_eq!(e.jobs.get("build"), Some(&80), "median of 60, 80 and 100");
        assert_eq!(e.job_runs, ESTIMATE_RUNS);
        assert_eq!(e.run_median, Some(125), "median of the four succeeded runs on this branch");
        assert_eq!(e.run_n, 4);
    }

    #[test]
    fn the_tree_folds_cleanup_steps_and_a_failed_runs_leading_passes() {
        let names = ["Set up job", "checkout", "toolchain", "Build", "Test", "Clippy", "Post checkout", "Complete job"];
        let status = |i: usize| match i {
            4 => PipelineRunStatus::Failed,
            5 => PipelineRunStatus::Canceled,
            _ => PipelineRunStatus::Succeeded,
        };
        let mut job = pipeline_job("j1", PipelineRunStatus::Failed);
        job.steps = names
            .iter()
            .enumerate()
            .map(|(i, n)| PipelineStep { name: (*n).into(), status: status(i), started_at: None, finished_at: None })
            .collect();
        let run = pipeline_run("1", PipelineRunStatus::Failed, vec![pipeline_stage("jobs", PipelineRunStatus::Failed, vec![job])]);
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        let labels = |v: &PipelineView| v.flatten().iter().map(|n| n.label.clone()).collect::<Vec<_>>();
        assert_eq!(
            labels(&view),
            vec!["3 steps passed", "Build", "Test", "Clippy", "2 post & cleanup steps"],
            "one stage and one job: no stage or job rows; passes before the failure and cleanup fold"
        );
        view.toggle_selected(); // the passed group opens
        assert_eq!(labels(&view)[..4], ["3 steps passed", "Set up job", "checkout", "toolchain"]);
        assert_eq!(view.flatten()[1].depth, 1);
        view.toggle_selected();
        assert_eq!(labels(&view).len(), 5, "and folds again");

        // First load lands on the failed step.
        let mut view = PipelineView::new("CI".into(), run.clone(), "c".into(), ProviderType::GitHub, "ci".into(), None);
        view.stale = true;
        view.auto_select();
        assert_eq!(view.flatten()[view.selected].label, "Test");

        // A passing run folds only its cleanup.
        let mut ok = run;
        ok.status = PipelineRunStatus::Succeeded;
        for s in &mut ok.stages[0].jobs[0].steps {
            s.status = PipelineRunStatus::Succeeded;
        }
        let view = PipelineView::new("CI".into(), ok, "c".into(), ProviderType::GitHub, "ci".into(), None);
        assert_eq!(labels(&view).len(), 7, "six steps and the cleanup group");
    }

    const SECTIONED: &str = "prelude\n\
        ##[group]Set up job\n\
        runner 2.3\n\
        ##[group]Run cargo build\n\
        ##[endgroup]\n\
        Compiling x\n\
        ##[group]Run cargo test\n\
        ##[endgroup]\n\
        running 2 tests\n\
        test a ... FAILED\n\
        needle here\n\
        ##[group]Clippy\n\
        clippy says hi";

    fn sectioned_log() -> LogView {
        let mut log = LogView::new("Logs · j1".into(), "j1".into(), false);
        log.steps = vec!["Set up job".into(), "Build".into(), "Test".into(), "Clippy".into()];
        log.set_text(SECTIONED);
        log
    }

    #[test]
    fn a_sectioned_log_opens_the_failing_step_and_z_folds_what_the_cursor_is_on() {
        let mut log = sectioned_log();
        assert_eq!(log.sections.len(), 4);
        assert_eq!(log.open, [2].into_iter().collect(), "only the step holding the first error is open");
        let rows = log.rows.clone();
        assert_eq!(rows[0], LogRow::Line(0), "the preamble stays a plain line");
        assert_eq!(rows[1], LogRow::Header(0));
        assert!(!rows.contains(&LogRow::Line(4)) && !rows.contains(&LogRow::Line(7)), "endgroup markers are hidden");
        assert_eq!(log.rows[log.cursor], LogRow::Line(9), "the cursor starts on the first error");

        log.toggle_fold(); // z inside "Run cargo test" folds it, cursor to its header
        assert!(log.open.is_empty());
        assert_eq!(log.rows[log.cursor], LogRow::Header(2));
        log.toggle_fold();
        assert!(log.open.contains(&2), "z again unfolds");

        log.toggle_all_folds(); // Z: not all open → open all
        assert_eq!(log.open.len(), 4);
        log.toggle_all_folds(); // Z: all open → fold all
        assert!(log.open.is_empty());
        assert_eq!(log.rows.len(), 5, "preamble + four headers");

        // E and search both unfold what they land in.
        log.jump_first_error();
        assert!(log.open.contains(&2));
        assert_eq!(log.rows[log.cursor], LogRow::Line(9));
        log.toggle_all_folds();
        log.toggle_all_folds();
        assert!(log.open.is_empty());
        log.search_input = Some("clippy says".into());
        log.commit_search();
        assert!(log.open.contains(&3), "the match's section opened");
        assert_eq!(log.rows[log.cursor], LogRow::Line(12));

        // j/k move the cursor rather than the view in a sectioned log.
        log.scroll_top();
        log.scroll_by(1);
        assert_eq!(log.cursor, 1);
    }

    #[test]
    fn a_log_without_sections_behaves_as_it_always_did() {
        let mut log = LogView::with_lines("Logs", "j1", lines(&["a", "b", "error: c"]));
        assert!(!log.sectioned());
        assert_eq!(log.rows, vec![LogRow::Line(0), LogRow::Line(1), LogRow::Line(2)]);
        log.toggle_fold();
        log.toggle_all_folds();
        assert_eq!(log.rows.len(), 3, "z and Z do nothing");
    }

    /// A single-job run whose steps are the sections of [`SECTIONED`].
    fn sectioned_run() -> PipelineRun {
        let mut job = pipeline_job("j1", PipelineRunStatus::Failed);
        job.steps = ["Set up job", "Build", "Test", "Clippy"]
            .iter()
            .map(|n| PipelineStep {
                name: (*n).into(),
                status: if *n == "Test" { PipelineRunStatus::Failed } else { PipelineRunStatus::Succeeded },
                started_at: None,
                finished_at: None,
            })
            .collect();
        pipeline_run("1", PipelineRunStatus::Failed, vec![pipeline_stage("jobs", PipelineRunStatus::Failed, vec![job])])
    }

    #[tokio::test]
    async fn enter_on_a_step_opens_that_steps_section_of_the_job_log() {
        let deps = test_deps();
        let mut app = run_pane_app(sectioned_run());
        pane_mut(&mut app).selected = 1; // "Build"
        app.on_key(Key::Enter, &deps).await;
        let log = pane(&app).logs.as_ref().expect("the job's log opened");
        assert_eq!((log.job_id.as_str(), log.target_step), ("j1", Some(1)));
        assert!(pane(&app).log_focus);
        app.apply_pipeline_logs("c", "1", "j1", Ok(SECTIONED.into()));
        let log = pane(&app).logs.as_ref().unwrap();
        assert_eq!(log.open, [1].into_iter().collect(), "Build's section, by name through `Run cargo build`");
        assert_eq!(log.rows[log.cursor], LogRow::Header(1));
        // The failed job's log also feeds the failure line.
        let (job, summary) = pane(&app).log_failure.clone().expect("a failure read from the log");
        assert_eq!((job.as_str(), summary.test.as_deref()), ("j1", Some("a")));
    }

    #[tokio::test]
    async fn e_from_the_tree_opens_the_failed_log_at_its_first_error() {
        let deps = test_deps();
        let mut app = run_pane_app(sectioned_run());
        app.on_key(Key::Char('E'), &deps).await;
        assert_eq!(pane(&app).flatten()[pane(&app).selected].label, "Test", "the tree points at the failed step");
        app.apply_pipeline_logs("c", "1", "j1", Ok(SECTIONED.into()));
        let log = pane(&app).logs.as_ref().expect("open");
        assert_eq!(log.rows[log.cursor], LogRow::Line(9));
        assert_eq!(log.note.as_deref(), Some("first error at line 10"));
    }

    #[tokio::test]
    async fn enter_on_a_problem_opens_its_job_log_at_the_line_it_names() {
        let deps = test_deps();
        let mut app = run_pane_app(sectioned_run());
        app.on_key(Key::Char('e'), &deps).await;
        assert_eq!(app.toast.as_deref(), Some("No problems reported for this run"));
        pane_mut(&mut app).annotations = vec![PipelineAnnotation {
            level: AnnotationLevel::Failure,
            message: "needle".into(),
            title: None,
            path: Some("src/here.rs".into()),
            line: Some(4),
            job_id: Some("j1".into()),
        }];
        app.on_key(Key::Char('e'), &deps).await;
        assert!(pane(&app).problem_focus);
        app.on_key(Key::Enter, &deps).await;
        assert!(!pane(&app).problem_focus && pane(&app).log_focus);
        app.apply_pipeline_logs("c", "1", "j1", Ok(SECTIONED.into()));
        let log = pane(&app).logs.as_ref().expect("open");
        assert_eq!(log.rows[log.cursor], LogRow::Line(10), "no `src/here.rs:4` in the log, so the message");
        assert_eq!(log.note.as_deref(), Some("line 11"));
    }

    #[test]
    fn a_branch_with_one_run_shows_the_pipelines_history_across_branches() {
        let mut app = App::new("slate");
        app.pipes = ci_history(3, &[], |_| 60);
        let mut tag = pipeline_run("tag", PipelineRunStatus::Succeeded, vec![]);
        tag.branch = Some("v1.2.1".into());
        tag.started_at = Some(secs_ago(10));
        tag.finished_at = Some(secs_ago(5));
        app.pipes.push(row_of(tag.clone()));
        app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI".into(), tag, "c".into(), ProviderType::GitHub, "ci".into(), None)));
        app.refresh_pipeline_context();
        let h = pane(&app).history.clone().expect("history");
        assert!(h.all_branches);
        assert_eq!(h.entries.len(), 4, "every branch's runs");
        assert_eq!(h.current, Some(3));
    }

    #[test]
    fn annotations_are_asked_for_a_finished_run_once_per_status() {
        let mut app = run_pane_app(failed_run());
        let key = pipeline_detail_cache_key("c", &pane(&app).run.item_ref());
        assert!(app.wants_annotations(&key), "a finished run's problems are asked for");
        assert!(!app.wants_annotations(&key), "and not again while its status holds");
        pane_mut(&mut app).run.status = PipelineRunStatus::Running;
        assert!(!app.wants_annotations(&key), "an in-flight run's never are");
        pane_mut(&mut app).run.status = PipelineRunStatus::Succeeded;
        assert!(app.wants_annotations(&key), "a new outcome asks again");
        assert!(!app.wants_annotations("some other run"), "nothing on screen, nothing asked");
    }

    #[test]
    fn folds_and_the_view_survive_the_line_cap_dropping_more_of_the_head() {
        // Three steps, the second big; a live log grows at its tail, so each poll the cap
        // drops more of the head.
        let body = |tail: usize| {
            let mut t = String::new();
            for i in 0..8000 {
                t.push_str(&format!("preamble {i}\n"));
            }
            t.push_str("##[group]Run one\none\n##[group]Run two\n");
            for i in 0..3000 {
                t.push_str(&format!("two {i}\n"));
            }
            t.push_str("##[group]Run three\n");
            for i in 0..tail {
                t.push_str(&format!("three {i}\n"));
            }
            t
        };
        let mut log = LogView::new("Logs".into(), "j1".into(), false);
        log.steps = vec!["one".into(), "two".into(), "three".into()];
        log.set_text(&body(1000));
        assert!(log.base > 0, "the head was capped");
        log.viewport.set(10);
        // Open "two" and put the cursor on one of its lines.
        let two = log.sections.iter().position(|s| s.name == "Run two").unwrap();
        log.open = [two].into_iter().collect();
        log.relayout();
        let target = log.lines.iter().position(|l| l == "two 1500").unwrap();
        log.jump_to(target);
        let top_before = log.rows[log.effective_scroll() as usize];
        let top_text = log.lines[log.row_line(top_before)].clone();

        // The next poll has 500 more lines, so 500 more of the head are dropped.
        let before = log.base;
        log.set_text(&body(1500));
        assert_eq!(log.base, before + 500);
        let two = log.sections.iter().position(|s| s.name == "Run two").unwrap();
        assert_eq!(log.open, [two].into_iter().collect(), "the same step is still the open one");
        assert_eq!(log.lines[log.row_line(log.rows[log.cursor])], "two 1500", "the cursor stayed on its line");
        assert_eq!(log.lines[log.row_line(log.rows[log.effective_scroll() as usize])], top_text, "and the view on its content");
    }

    #[test]
    fn folded_passed_steps_merge_into_one_header_that_z_opens() {
        let mut log = LogView::new("Logs".into(), "j1".into(), false);
        log.steps = vec!["one".into(), "two".into(), "three".into(), "four".into()];
        log.step_passed = vec![true, true, true, false];
        log.set_text("##[group]Run one\na\n##[group]Run two\nb\n##[group]Run three\nc\n##[group]Run four\nerror: d");
        assert_eq!(log.rows[0], LogRow::Group(0, 2), "three passed steps, one header");
        assert_eq!(log.rows[1], LogRow::Header(3));
        log.cursor = 0;
        log.toggle_fold();
        assert!((0..3).all(|s| log.open.contains(&s)), "z opens every step in the group");
        assert_eq!(log.rows[0], LogRow::Header(0));
    }

    fn a_problem(message: &str) -> PipelineAnnotation {
        PipelineAnnotation { level: AnnotationLevel::Failure, message: message.into(), title: None, path: None, line: None, job_id: None }
    }

    #[test]
    fn problems_arrive_apart_from_the_run_and_land_whichever_comes_first() {
        let deps = deps_with_cache(memory_cache());
        // The run first, then its problems.
        let mut app = run_pane_app(failed_run());
        let key = pipeline_detail_cache_key("c", &pane(&app).run.item_ref());
        let detail = || Box::new(PipelineDetailFetch { run: Some(failed_run()), approvals: Some(Vec::new()), ..Default::default() });
        app.on_event(AppEvent::PipelineDetailLoaded { key: key.clone(), detail: detail(), fetched_at: Utc::now() }, &deps);
        assert!(pane(&app).annotations.is_empty());
        app.on_event(AppEvent::PipelineAnnotationsLoaded { key: key.clone(), annotations: Some(vec![a_problem("boom")]) }, &deps);
        assert_eq!(pane(&app).annotations.len(), 1, "shown when they land");
        let cached = deps.cache.get::<PipelineDetail>(&key).expect("cached").value;
        assert_eq!(cached.annotations.len(), 1, "and written into the cached detail");

        // The problems first, then a run refresh: the refresh keeps them.
        let deps = deps_with_cache(memory_cache());
        let mut app = run_pane_app(failed_run());
        app.on_event(AppEvent::PipelineAnnotationsLoaded { key: key.clone(), annotations: Some(vec![a_problem("boom")]) }, &deps);
        app.on_event(AppEvent::PipelineDetailLoaded { key: key.clone(), detail: detail(), fetched_at: Utc::now() }, &deps);
        assert_eq!(pane(&app).annotations.len(), 1, "a detail landing after them doesn't blank them");

        // A failed call may be asked again.
        assert!(app.wants_annotations(&key));
        app.on_event(AppEvent::PipelineAnnotationsLoaded { key: key.clone(), annotations: None }, &deps);
        assert!(app.wants_annotations(&key), "unanswered, so asked again");
    }

    #[tokio::test]
    async fn a_refused_rerun_restores_the_runs_rows_even_after_a_reload_moved_them() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let calls = PipeCalls { rerun: true, refuse: true, gate: Some(gate.clone()), ..PipeCalls::default() };
        let deps = deps_with_pipes(calls.clone()).await;
        let mut app = run_pane_app(failed_run());
        pane_mut(&mut app).supports_rerun = true;
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        app.on_key(Key::Char('R'), &deps).await;
        app.on_key(Key::Char('y'), &deps).await;
        assert_eq!(app.pipes[0].run.status, PipelineRunStatus::Queued);

        // A reload lands mid-flight with the run at another position.
        let mut other = failed_run();
        other.id = "r0".into();
        let mut r = reloaded_with_health(Vec::new());
        r.pipes = vec![row_of(other), row_of(failed_run())];
        app.on_event(AppEvent::Reloaded(Box::new(r)), &deps);
        assert_eq!(app.pipes[1].run.status, PipelineRunStatus::Queued, "held against the stale reload");

        gate.notify_one();
        let event = loop {
            let event = rx.recv().await.expect("an answer");
            if matches!(event, AppEvent::PipelineRunActionDone { .. }) {
                break event;
            }
        };
        app.on_event(event, &deps);
        assert_eq!(app.pipes[1].run.status, PipelineRunStatus::Failed, "restored where it now is");
        assert_eq!(app.pipes[0].run.id, "r0");
        assert_eq!(pane(&app).run.status, PipelineRunStatus::Failed);
    }

    #[test]
    fn estimates_ignore_the_same_pipeline_in_another_repository() {
        let mut app = App::new("slate");
        let mut elsewhere = ci_history(1, &[], |_| 999).remove(0);
        elsewhere.run.repository = Some("acme/other".into());
        let elsewhere_run = elsewhere.run.clone();
        app.pipes.push(elsewhere);
        let mut job = pipeline_job("build", PipelineRunStatus::Succeeded);
        job.started_at = Some(secs_ago(1000));
        job.finished_at = Some(secs_ago(1));
        let mut detailed = elsewhere_run;
        detailed.stages = vec![pipeline_stage("build", PipelineRunStatus::Succeeded, vec![job])];
        app.run_details.insert(pipeline_detail_cache_key("c", &detailed.item_ref()), detailed);
        let running = pipeline_run("live", PipelineRunStatus::Running, vec![pipeline_stage("build", PipelineRunStatus::Running, vec![pipeline_job("build", PipelineRunStatus::Running)])]);
        app.pipes.push(row_of(running.clone()));
        app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI".into(), running, "c".into(), ProviderType::GitHub, "ci".into(), None)));
        app.refresh_pipeline_context();
        let e = &pane(&app).estimates;
        assert_eq!((e.job_runs, e.run_median), (0, None), "another repository's runs don't count");
    }

    fn def_at(id: &str, repo: Option<&str>, path: Option<&str>) -> PipelineDefinition {
        PipelineDefinition { repository: repo.map(Into::into), id: id.into(), name: id.into(), path: path.map(Into::into), url: None }
    }

    /// Azure files pipelines under folders; the picker shows the project and folder so a busy
    /// org's 38 pipelines can be told apart and searched by folder. The root folder and a
    /// GitHub workflow file path add nothing.
    #[test]
    fn picker_rows_say_where_a_pipeline_lives() {
        assert_eq!(pipeline_label(&def_at("MainLine", Some("Tilt"), Some("\\Releases\\Core"))), "MainLine  ·  Tilt \\Releases\\Core");
        assert_eq!(pipeline_label(&def_at("Tests", Some("Tilt"), Some("\\"))), "Tests  ·  Tilt");
        assert_eq!(pipeline_label(&def_at("ci", Some("acme/pay"), Some(".github/workflows/ci.yml"))), "ci  ·  acme/pay");
        assert_eq!(pipeline_label(&def_at("CI Build", None, None)), "CI Build");
    }

    /// The Pipelines header counts pipelines, and the add/remove picker writes the subscription:
    /// ticking every one is saved as "all" so a pipeline created later is fetched too, and an
    /// unticked pipeline's rows leave the list at once.
    #[tokio::test]
    async fn the_pipeline_picker_rewrites_the_subscription_and_the_header_count() {
        let deps = deps_with_pipes(PipeCalls::default()).await;
        let mut app = App::new("slate");
        app.pipe_catalog.insert("c".into(), vec![def_at("ci", None, None), def_at("cd", None, None), def_at("nightly", None, None)]);
        app.refresh_repo_scope(&deps);
        assert_eq!(app.pipe_scope.as_ref().map(PipeScope::label).as_deref(), Some("Pipelines · 1 of 3"), "subscribed to ci only");

        app.apply_pipeline_subs("c", vec!["ci".into(), "cd".into(), "nightly".into()], &deps).await;
        let sub = deps.config.snapshot().pipelines.unwrap().subscriptions[0].clone();
        assert!(sub.auto_discover_all, "every pipeline ticked is saved as all");
        assert_eq!(app.pipe_scope.as_ref().map(PipeScope::label).as_deref(), Some("Pipelines · 3 of 3"));

        let mut nightly = row_of(failed_run());
        nightly.run.definition_id = "nightly".into();
        app.pipes = vec![nightly];
        app.apply_pipeline_subs("c", vec!["ci".into(), "cd".into()], &deps).await;
        let sub = deps.config.snapshot().pipelines.unwrap().subscriptions[0].clone();
        assert!(!sub.auto_discover_all);
        assert_eq!(sub.definition_ids, vec!["ci".to_string(), "cd".to_string()]);
        assert_eq!(app.pipe_scope.as_ref().map(PipeScope::label).as_deref(), Some("Pipelines · 2 of 3"));
        assert!(app.pipes.is_empty(), "the unticked pipeline's runs are gone before the reload lands");

        // Nothing ticked is saved as nothing selected — and reads as such, not as everything.
        app.apply_pipeline_subs("c", vec![], &deps).await;
        let sub = deps.config.snapshot().pipelines.unwrap().subscriptions[0].clone();
        assert!(!sub.auto_discover_all && sub.definition_ids.is_empty());
        assert_eq!(app.pipe_scope.as_ref().map(PipeScope::label).as_deref(), Some("Pipelines · 0 of 3"));
    }


    // ---- optimistic PR and work-item writes ----

    /// What the recording PR / work-item provider was asked to do, and how it answers.
    #[derive(Clone)]
    struct ItemCalls {
        log: Arc<std::sync::Mutex<Vec<String>>>,
        refuse: bool,
        /// When set, every write waits for this before answering — to see the screen mid-flight.
        gate: Option<Arc<tokio::sync::Notify>>,
        /// What `get` answers with after a write.
        pr: PullRequest,
        wi: WorkItem,
    }

    impl ItemCalls {
        fn new() -> Self {
            ItemCalls { log: Arc::default(), refuse: false, gate: None, pr: pr(None), wi: wi(None) }
        }

        fn gated() -> (Self, Arc<tokio::sync::Notify>) {
            let gate = Arc::new(tokio::sync::Notify::new());
            (ItemCalls { gate: Some(gate.clone()), ..ItemCalls::new() }, gate)
        }

        fn calls(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }

        async fn record(&self, call: String) -> forgetop_core::Result<()> {
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            self.log.lock().unwrap().push(call);
            if self.refuse {
                Err(forgetop_core::Error::Provider("the provider said no".into()))
            } else {
                Ok(())
            }
        }
    }

    struct MockItemSource(ItemCalls);

    #[async_trait::async_trait]
    impl PullRequestSource for MockItemSource {
        async fn list(&self, _query: &PullRequestQuery) -> forgetop_core::Result<Vec<PullRequest>> {
            Ok(Vec::new())
        }
        async fn get(&self, _item: &ItemRef) -> forgetop_core::Result<PullRequest> {
            Ok(self.0.pr.clone())
        }
        async fn threads(&self, _item: &ItemRef) -> forgetop_core::Result<Vec<CommentThread>> {
            Ok(Vec::new())
        }
        async fn changes(&self, _item: &ItemRef) -> forgetop_core::Result<Vec<FileChange>> {
            Ok(Vec::new())
        }
        async fn add_comment(&self, item: &ItemRef, body: &str) -> forgetop_core::Result<()> {
            self.0.record(format!("comment {} {body}", item.id)).await
        }
        async fn reply_to_thread(&self, item: &ItemRef, thread_id: &str, body: &str) -> forgetop_core::Result<()> {
            self.0.record(format!("reply {} {thread_id} {body}", item.id)).await
        }
        async fn vote(&self, item: &ItemRef, vote: ReviewVote) -> forgetop_core::Result<()> {
            self.0.record(format!("vote {} {vote:?}", item.id)).await
        }
        async fn merge(&self, item: &ItemRef, _options: &MergeOptions) -> forgetop_core::Result<()> {
            self.0.record(format!("merge {}", item.id)).await
        }
        async fn submit_review(&self, item: &ItemRef, event: ReviewVote, comments: &[LineComment]) -> forgetop_core::Result<()> {
            self.0.record(format!("review {} {event:?} {}", item.id, comments.len())).await
        }
    }

    #[async_trait::async_trait]
    impl WorkItemSource for MockItemSource {
        async fn list(&self, _query: &WorkItemQuery) -> forgetop_core::Result<Vec<WorkItem>> {
            Ok(Vec::new())
        }
        async fn get(&self, _item: &ItemRef) -> forgetop_core::Result<WorkItem> {
            Ok(self.0.wi.clone())
        }
        async fn threads(&self, _item: &ItemRef) -> forgetop_core::Result<Vec<CommentThread>> {
            Ok(Vec::new())
        }
        async fn set_state(&self, item: &ItemRef, state: &str) -> forgetop_core::Result<()> {
            self.0.record(format!("state {} {state}", item.id)).await
        }
        async fn add_comment(&self, item: &ItemRef, body: &str) -> forgetop_core::Result<()> {
            self.0.record(format!("wi-comment {} {body}", item.id)).await
        }
    }

    struct MockItemConn(ItemCalls, Capabilities);

    #[async_trait::async_trait]
    impl ProviderConnection for MockItemConn {
        fn connection_id(&self) -> &str {
            "c"
        }
        fn provider_type(&self) -> ProviderType {
            ProviderType::GitHub
        }
        fn display_name(&self) -> &str {
            "GH"
        }
        fn capabilities(&self) -> &Capabilities {
            &self.1
        }
        fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
            Some(Arc::new(MockItemSource(self.0.clone())))
        }
        fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
            Some(Arc::new(MockItemSource(self.0.clone())))
        }
        fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
            None
        }
        async fn check(&self) -> bool {
            true
        }
    }

    struct MockItemFactory(ItemCalls);

    impl ProviderFactory for MockItemFactory {
        fn provider_type(&self) -> ProviderType {
            ProviderType::GitHub
        }
        fn describe_capabilities(&self) -> Capabilities {
            Capabilities { supports_pull_requests: true, supports_work_items: true, ..Capabilities::default() }
        }
        fn create(&self, _connection: &Connection, _secret: Option<String>) -> forgetop_core::Result<Arc<dyn ProviderConnection>> {
            Ok(Arc::new(MockItemConn(self.0.clone(), self.describe_capabilities())))
        }
    }

    /// A PR source whose `list` answers each filter from its own rows, recording the queries it
    /// was asked — the shape of a provider that targets its filters (GitHub) or does not.
    #[derive(Clone)]
    struct TargetedPrs {
        targets: bool,
        /// What `review_clears_request` answers.
        clears: bool,
        log: Arc<std::sync::Mutex<Vec<(PullRequestFilter, bool)>>>,
    }

    struct TargetedPrSource(TargetedPrs);

    #[async_trait::async_trait]
    impl PullRequestSource for TargetedPrSource {
        async fn list(&self, query: &PullRequestQuery) -> forgetop_core::Result<Vec<PullRequest>> {
            use chrono::TimeZone;
            self.0.log.lock().unwrap().push((query.filter, query.include_completed));
            let at = |day: u32| Some(chrono::Utc.with_ymd_and_hms(2026, 10, day, 0, 0, 0).unwrap());
            let mut by_me_in_page = pool_pr("c", "99", "me", &[]).pr;
            by_me_in_page.updated_at = at(3);
            let mut by_other = pool_pr("c", "100", "sam", &[]).pr;
            by_other.updated_at = at(2);
            // Older than the page holds — only a targeted fetch finds it.
            let mut by_me_beyond_page = pool_pr("c", "7", "me", &[]).pr;
            by_me_beyond_page.updated_at = at(4);
            let mut wants_my_review = pool_pr("c", "5", "sam", &["me"]).pr;
            wants_my_review.updated_at = at(1);
            Ok(match query.filter {
                PullRequestFilter::All => vec![by_me_in_page, by_other],
                PullRequestFilter::Mine => vec![by_me_in_page, by_me_beyond_page],
                PullRequestFilter::ReviewRequested => vec![wants_my_review],
            })
        }
        async fn current_user(&self) -> forgetop_core::Result<Option<String>> {
            Ok(Some("me".into()))
        }
        fn list_targets_filter(&self) -> bool {
            self.0.targets
        }
        fn review_clears_request(&self) -> bool {
            self.0.clears
        }
        async fn get(&self, _item: &ItemRef) -> forgetop_core::Result<PullRequest> {
            Ok(pr(None))
        }
        async fn threads(&self, _item: &ItemRef) -> forgetop_core::Result<Vec<CommentThread>> {
            Ok(Vec::new())
        }
        async fn changes(&self, _item: &ItemRef) -> forgetop_core::Result<Vec<FileChange>> {
            Ok(Vec::new())
        }
        async fn add_comment(&self, _item: &ItemRef, _body: &str) -> forgetop_core::Result<()> {
            Ok(())
        }
        async fn reply_to_thread(&self, _item: &ItemRef, _thread_id: &str, _body: &str) -> forgetop_core::Result<()> {
            Ok(())
        }
        async fn vote(&self, _item: &ItemRef, _vote: ReviewVote) -> forgetop_core::Result<()> {
            Ok(())
        }
        async fn merge(&self, _item: &ItemRef, _options: &MergeOptions) -> forgetop_core::Result<()> {
            Ok(())
        }
        async fn submit_review(&self, _item: &ItemRef, _event: ReviewVote, _comments: &[LineComment]) -> forgetop_core::Result<()> {
            Ok(())
        }
    }

    struct TargetedPrConn(TargetedPrs, Capabilities);

    #[async_trait::async_trait]
    impl ProviderConnection for TargetedPrConn {
        fn connection_id(&self) -> &str {
            "c"
        }
        fn provider_type(&self) -> ProviderType {
            ProviderType::GitHub
        }
        fn display_name(&self) -> &str {
            "GH"
        }
        fn capabilities(&self) -> &Capabilities {
            &self.1
        }
        fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
            Some(Arc::new(TargetedPrSource(self.0.clone())))
        }
        fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
            None
        }
        fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
            None
        }
        async fn check(&self) -> bool {
            true
        }
    }

    struct TargetedPrFactory(TargetedPrs);

    impl ProviderFactory for TargetedPrFactory {
        fn provider_type(&self) -> ProviderType {
            ProviderType::GitHub
        }
        fn describe_capabilities(&self) -> Capabilities {
            Capabilities { supports_pull_requests: true, ..Capabilities::default() }
        }
        fn create(&self, _connection: &Connection, _secret: Option<String>) -> forgetop_core::Result<Arc<dyn ProviderConnection>> {
            Ok(Arc::new(TargetedPrConn(self.0.clone(), self.describe_capabilities())))
        }
    }

    /// Deps whose one PR connection, `c`, is a [`TargetedPrSource`].
    async fn deps_with_targeted_prs(prs: TargetedPrs) -> AppDeps {
        use forgetop_core::config::InMemoryConfigStore;
        use forgetop_core::secret::InMemorySecretStore;
        use forgetop_core::service::ConnectionResolver;

        let registry = Arc::new(ProviderRegistry::new(vec![Arc::new(TargetedPrFactory(prs))]));
        let secrets = Arc::new(InMemorySecretStore::default());
        let config = Arc::new(ConfigService::new(Arc::new(InMemoryConfigStore::default()), secrets.clone(), registry.clone()));
        let connection = Connection {
            id: "c".into(),
            provider_type: ProviderType::GitHub,
            display_name: "GH".into(),
            base_url: None,
            organization: None,
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
            repo_scope: None,
        };
        config.add_or_update_connection(connection, None).await.unwrap();
        config.bind_pull_requests("c").await.unwrap();
        let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry, secrets));
        AppDeps {
            sections: Arc::new(SectionService::new(config.clone(), resolver.clone())),
            health: Arc::new(ConnectionHealthService::new(config.clone(), resolver)),
            config,
            cache: Arc::new(CacheStore::disabled()),
        }
    }

    fn ids(rows: &[PrRow]) -> Vec<&str> {
        rows.iter().map(|r| r.pr.id.as_str()).collect()
    }

    /// The bug this guards: a page is the newest 50 of a repository, and "Mine" derived from it
    /// goes blank once 50 newer pull requests exist. A provider that targets the filter is asked
    /// for those views, and what it finds beyond the page joins the pool — once, and in order.
    #[tokio::test]
    async fn the_pool_fetches_targeted_views_and_merges_what_the_page_lacks() {
        let prs = TargetedPrs { targets: true, clears: true, log: Arc::default() };
        let deps = deps_with_targeted_prs(prs.clone()).await;
        let mut errors = Vec::new();
        let (pool, ok) = fetch_pr_pool(&deps, &mut errors).await;
        assert!(ok && errors.is_empty(), "{errors:?}");
        assert_eq!(
            prs.log.lock().unwrap().clone(),
            [(PullRequestFilter::All, false), (PullRequestFilter::All, true)].into_iter().chain(POOL_TARGETED_VIEWS).collect::<Vec<_>>(),
            "the unfiltered pair, then each targeted view on the variant it is derived on"
        );
        // Every row once — `99` came back from the page and the search — newest-updated first.
        assert_eq!(ids(&pool.open), vec!["7", "99", "100", "5"]);
        assert_eq!(ids(&pool.completed), vec!["7", "99", "100"], "the review view is not fetched on completed rows");
        // And the derived views are what the user sees: "Mine" has the pull request the page lost.
        assert_eq!(ids(&derive_pool_rows(&pool, PullRequestFilter::Mine, false, &HashSet::new())), vec!["7", "99"]);
        assert_eq!(ids(&derive_pool_rows(&pool, PullRequestFilter::ReviewRequested, false, &HashSet::new())), vec!["5"]);
        assert_eq!(ids(&derive_pool_rows(&pool, PullRequestFilter::All, false, &HashSet::new())), vec!["7", "99", "100", "5"]);
        assert_eq!(pool.review_clears_request.get("c"), Some(&true), "the pool remembers that this forge drops a reviewed PR");
    }

    /// A provider that only filters its page is not asked again: the answer would be a subset
    /// of rows the pool already holds, at the price of the same calls over.
    #[tokio::test]
    async fn the_pool_leaves_a_page_filtering_provider_at_two_calls() {
        let prs = TargetedPrs { targets: false, clears: false, log: Arc::default() };
        let deps = deps_with_targeted_prs(prs.clone()).await;
        let (pool, _) = fetch_pr_pool(&deps, &mut Vec::new()).await;
        assert_eq!(prs.log.lock().unwrap().clone(), vec![(PullRequestFilter::All, false), (PullRequestFilter::All, true)]);
        assert_eq!(ids(&pool.open), vec!["99", "100"], "page order, untouched");
        assert_eq!(pool.review_clears_request.get("c"), Some(&false), "and that this one keeps a reviewed PR listed");
    }

    #[test]
    fn merging_pool_rows_skips_what_is_held_and_reorders_only_when_it_added() {
        use chrono::TimeZone;
        let row = |id: &str, repo: Option<&str>, day: u32| {
            let mut r = pool_pr("c", id, "me", &[]);
            r.pr.repository = repo.map(str::to_string);
            r.pr.updated_at = Some(chrono::Utc.with_ymd_and_hms(2026, 10, day, 0, 0, 0).unwrap());
            r
        };
        let mut rows = vec![row("2", Some("acme/pay"), 2), row("1", Some("acme/pay"), 5)];
        // Nothing new: the page's own order (not newest-first here) is left alone.
        assert_eq!(merge_pool_rows(&mut rows, vec![row("2", Some("acme/pay"), 2)]), 0);
        assert_eq!(ids(&rows), vec!["2", "1"]);
        // The same number in another repository is another pull request; a held one is skipped.
        assert_eq!(merge_pool_rows(&mut rows, vec![row("1", Some("acme/pay"), 5), row("2", Some("acme/ledger"), 9)]), 1);
        assert_eq!(ids(&rows), vec!["2", "1", "2"]);
        assert_eq!(rows[0].pr.repository.as_deref(), Some("acme/ledger"), "newest-updated first once something was added");
    }

    /// Deps whose one PR and work-item connection, `c`, is the recording provider.
    async fn deps_with_items(calls: ItemCalls) -> AppDeps {
        use forgetop_core::config::InMemoryConfigStore;
        use forgetop_core::secret::InMemorySecretStore;
        use forgetop_core::service::ConnectionResolver;

        let registry = Arc::new(ProviderRegistry::new(vec![Arc::new(MockItemFactory(calls))]));
        let secrets = Arc::new(InMemorySecretStore::default());
        let config = Arc::new(ConfigService::new(Arc::new(InMemoryConfigStore::default()), secrets.clone(), registry.clone()));
        let connection = Connection {
            id: "c".into(),
            provider_type: ProviderType::GitHub,
            display_name: "GH".into(),
            base_url: None,
            organization: None,
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
            repo_scope: None,
        };
        config.add_or_update_connection(connection, None).await.unwrap();
        config.bind_pull_requests("c").await.unwrap();
        config.bind_work_items("c").await.unwrap();
        let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry, secrets));
        AppDeps {
            sections: Arc::new(SectionService::new(config.clone(), resolver.clone())),
            health: Arc::new(ConnectionHealthService::new(config.clone(), resolver)),
            config,
            cache: Arc::new(CacheStore::disabled()),
        }
    }

    /// PR `1` open full-screen, listed, waiting on your review on the Launchpad; you are `me`.
    fn pr_action_app() -> App {
        let mut app = App::new("slate");
        app.pr_pool = pool_of(vec![pr_row(pr(None))], Some("me"));
        app.prs = vec![pr_row(pr(None))];
        app.lp_prs_review = vec![pr_row(pr(None))];
        app.rebuild_launchpad();
        app.screen = pr_view_with_pending(Vec::new());
        app
    }

    fn pr_pane(app: &App) -> &PrView {
        let Screen::PrView(v) = &app.screen else { panic!("expected the PR view") };
        v
    }

    fn my_vote(pr: &PullRequest) -> Option<ReviewVote> {
        pr.reviewers.iter().find(|r| forgetop_core::filter::is_user(&r.user, "me")).map(|r| r.vote)
    }

    fn on_launchpad(app: &App) -> bool {
        !app.lp_dismissed.contains(&launchpad::Entry::key("c", "1"))
    }

    /// Waits for the provider's answer to a write, skipping the background fetches it starts.
    async fn answer(rx: &mut mpsc::UnboundedReceiver<AppEvent>) -> AppEvent {
        loop {
            let event = rx.recv().await.expect("the provider's answer");
            if matches!(event, AppEvent::PrActionDone { .. } | AppEvent::WiActionDone { .. }) {
                return event;
            }
        }
    }

    fn pr_detail_with(threads: Vec<CommentThread>) -> AppEvent {
        AppEvent::PrDetailLoaded {
            key: pr_detail_cache_key("c", &pr(None).item_ref()),
            detail: Box::new(PrDetailFetch { threads: Some(threads), ..Default::default() }),
            fetched_at: Utc::now(),
        }
    }

    fn comment_by(id: &str, who: &str, body: &str) -> CommentThread {
        CommentThread {
            id: id.into(),
            comments: vec![Comment { id: format!("{id}-c"), author: me_user(Some(who)), body: body.into(), created_at: None }],
            file_path: None,
            line: None,
            is_resolved: false,
        }
    }

    #[tokio::test]
    async fn an_approval_shows_before_the_provider_answers_and_a_stale_refresh_keeps_it() {
        let (calls, gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = pr_action_app();
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);

        app.execute_action(Action::PrVote(ReviewVote::Approved), &deps).await;
        assert!(calls.calls().is_empty(), "the provider hasn't answered yet");
        assert_eq!(my_vote(&pr_pane(&app).pr), Some(ReviewVote::Approved), "the view shows your approval");
        assert_eq!(my_vote(&app.prs[0].pr), Some(ReviewVote::Approved), "and so does the list row");
        assert!(!on_launchpad(&app), "reviewed, so it has left the Launchpad");
        assert_eq!(app.toast.as_deref(), Some("Approved"));

        gate.notify_one();
        let event = answer(&mut rx).await;
        app.on_event(event, &deps);
        assert_eq!(calls.calls(), vec!["vote 1 Approved".to_string()]);
        assert_eq!(my_vote(&pr_pane(&app).pr), Some(ReviewVote::Approved), "a re-read that hasn't caught up doesn't undo it");

        // A refresh that still lists the PR without your vote doesn't flip it back…
        let stale = pool_of(vec![pr_row(pr(None))], Some("me"));
        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(stale), ok: true }, &deps);
        assert_eq!(my_vote(&app.prs[0].pr), Some(ReviewVote::Approved), "held until the provider catches up");

        // …and once one shows it, the provider's copy is taken as it is.
        let mut caught_up = pr(None);
        set_vote(&mut caught_up, "me", ReviewVote::Approved);
        let pool = pool_of(vec![pr_row(caught_up)], Some("me"));
        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(pool), ok: true }, &deps);
        assert!(app.held_items.is_empty(), "nothing left to hold");
    }

    /// `pr(None)` with you named as a reviewer who hasn't voted — what the review-requested
    /// list shows.
    fn pr_awaiting_me() -> PullRequest {
        let mut pr = pr(None);
        pr.reviewers.push(Reviewer { user: me_user(Some("me")), vote: ReviewVote::NoVote, is_required: false });
        pr
    }

    /// A pool of `rows` whose connection `c` drops you from a PR's reviewers once you've
    /// reviewed it, the way GitHub does — or keeps you listed, like GitLab, when `clears` is false.
    fn pool_where_review_clears(rows: Vec<PrRow>, clears: bool) -> PrPool {
        let mut pool = pool_of(rows, Some("me"));
        pool.review_clears_request.insert("c".into(), clears);
        pool
    }

    /// `pr_action_app` on the review-requested list, with PR `1` waiting on you.
    fn review_list_app(clears: bool) -> App {
        let mut app = pr_action_app();
        app.pr_filter = PullRequestFilter::ReviewRequested;
        app.pr_pool = pool_where_review_clears(vec![pr_row(pr_awaiting_me())], clears);
        if let Screen::PrView(v) = &mut app.screen {
            v.pr = pr_awaiting_me();
        }
        app.pr_pool_loaded = true;
        app.prs = app.derive_pr_rows(PullRequestFilter::ReviewRequested, false);
        app.lp_prs_review = app.prs.clone();
        app.pr_state.select(Some(0));
        assert_eq!(app.prs.len(), 1, "listed as waiting on your review");
        app
    }

    #[tokio::test]
    async fn on_a_forge_that_stops_asking_once_you_review_an_approval_leaves_the_review_list_at_once() {
        let (mut calls, gate) = ItemCalls::gated();
        // The provider's re-read after the write shows your approval, as GitHub's `get` does.
        set_vote(&mut calls.pr, "me", ReviewVote::Approved);
        let deps = deps_with_items(calls.clone()).await;
        let mut app = review_list_app(true);
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);

        app.execute_action(Action::PrVote(ReviewVote::Approved), &deps).await;
        assert!(calls.calls().is_empty(), "the provider hasn't answered yet");
        assert!(app.prs.is_empty(), "off the review-requested list before the provider answers");
        assert!(app.lp_prs_review.is_empty(), "and out of the Launchpad's review bucket");
        assert_eq!(app.pr_state.selected(), None, "nothing left to select");
        assert_eq!(my_vote(&pr_pane(&app).pr), Some(ReviewVote::Approved), "the open view still shows your approval");

        // A refetch that still names you as a reviewer hasn't caught up: the row stays off…
        let stale = pool_where_review_clears(vec![pr_row(pr_awaiting_me())], true);
        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(stale), ok: true }, &deps);
        assert!(app.prs.is_empty(), "held off the list until the forge catches up");

        gate.notify_one();
        let event = answer(&mut rx).await;
        app.on_event(event, &deps);
        assert_eq!(calls.calls(), vec!["vote 1 Approved".to_string()]);
        assert!(app.prs.is_empty(), "the provider's acceptance changes nothing on the list");

        // …and one that no longer names you is the forge caught up: the row is left as it
        // came, your vote isn't pushed back onto it, and nothing is held any more.
        let gone = pool_where_review_clears(vec![pr_row(pr(None))], true);
        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(gone), ok: true }, &deps);
        assert!(app.prs.is_empty(), "still off the list");
        assert!(app.pr_pool.open[0].pr.reviewers.is_empty(), "the forge's row is taken as it is");
        assert!(app.held_items.is_empty(), "nothing left to hold");
    }

    #[tokio::test]
    async fn on_a_forge_that_keeps_you_as_a_reviewer_an_approval_leaves_the_review_list_alone() {
        let (calls, _gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = review_list_app(false);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);

        app.execute_action(Action::PrVote(ReviewVote::Approved), &deps).await;
        assert_eq!(app.prs.len(), 1, "still a reviewer, so still listed");
        assert_eq!(my_vote(&app.prs[0].pr), Some(ReviewVote::Approved), "with your tick on the row");
        assert!(app.held_items.values().all(|h| h.review_cleared.is_none()), "nothing to take off the list");
    }

    #[test]
    fn a_cleared_review_hides_the_pr_from_the_review_view_only() {
        let pool = pool_where_review_clears(vec![pr_row(pr_awaiting_me())], true);
        let cleared: HashSet<String> = [pr_detail_cache_key("c", &pr(None).item_ref())].into_iter().collect();
        assert!(derive_pool_rows(&pool, PullRequestFilter::ReviewRequested, false, &cleared).is_empty(), "off the review view");
        assert_eq!(derive_pool_rows(&pool, PullRequestFilter::All, false, &cleared).len(), 1, "still in All");
        assert_eq!(derive_pool_rows(&pool, PullRequestFilter::ReviewRequested, false, &HashSet::new()).len(), 1, "listed with nothing cleared");
        let other: HashSet<String> = [pr_detail_cache_key("c", &ItemRef::new("2"))].into_iter().collect();
        assert_eq!(derive_pool_rows(&pool, PullRequestFilter::ReviewRequested, false, &other).len(), 1, "another PR's clearing is not this one's");
    }

    #[tokio::test]
    async fn a_review_hold_that_has_run_out_no_longer_hides_the_row() {
        let (calls, _gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = review_list_app(true);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        app.execute_action(Action::PrVote(ReviewVote::Approved), &deps).await;
        assert!(app.prs.is_empty());

        // Sixty seconds on, the provider has the last word: the row derives again.
        let key = pr_detail_cache_key("c", &pr(None).item_ref());
        app.held_items.get_mut(&key).unwrap().until = Utc::now() - chrono::Duration::seconds(1);
        assert_eq!(app.derive_pr_rows(PullRequestFilter::ReviewRequested, false).len(), 1, "no longer held off");
        let stale = pool_where_review_clears(vec![pr_row(pr_awaiting_me())], true);
        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(stale), ok: true }, &deps);
        assert_eq!(app.prs.len(), 1, "a refetch after the hold ran out lists it as the forge does");
        assert!(app.held_items.is_empty(), "the run-out hold is dropped");
    }

    #[tokio::test]
    async fn requesting_changes_leaves_the_review_list_at_once_too() {
        let (calls, _gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = review_list_app(true);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        app.execute_action(Action::PrVote(ReviewVote::Rejected), &deps).await;
        assert!(app.prs.is_empty(), "a verdict either way is a review given");
        assert_eq!(my_vote(&pr_pane(&app).pr), Some(ReviewVote::Rejected));
    }

    #[tokio::test]
    async fn a_review_submitted_with_a_verdict_leaves_the_review_list_but_a_comment_only_one_stays() {
        let line = || LineComment { path: "a.rs".into(), line: 3, side: DiffSide::New, body: "nit".into() };
        for (event, listed_after) in [(ReviewVote::NoVote, 1), (ReviewVote::Approved, 0)] {
            let (calls, _gate) = ItemCalls::gated();
            let deps = deps_with_items(calls.clone()).await;
            let mut app = review_list_app(true);
            if let Screen::PrView(v) = &mut app.screen {
                v.pending = vec![line()];
            }
            let (tx, _rx) = mpsc::unbounded_channel();
            app.job_tx = Some(tx);
            app.submit_review(event, &deps).await;
            assert_eq!(app.prs.len(), listed_after, "submitted with {event:?}");
        }
    }

    #[tokio::test]
    async fn a_plain_comment_leaves_the_review_list_alone() {
        let (calls, _gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = review_list_app(true);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        app.execute_action(Action::PrComment("looks fine so far".into()), &deps).await;
        assert_eq!(app.prs.len(), 1, "a comment is not a review");
        assert!(app.held_items.values().all(|h| h.review_cleared.is_none()));
    }

    #[tokio::test]
    async fn without_a_known_identity_an_approval_leaves_the_review_list_alone() {
        let (calls, _gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = review_list_app(true);
        // The connection could not say who you are: every row passes the filter, and there is
        // no reviewer entry of yours to clear.
        app.pr_pool.me.insert("c".into(), None);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        app.execute_action(Action::PrVote(ReviewVote::Approved), &deps).await;
        assert_eq!(app.prs.len(), 1, "nothing to take off the list without knowing who voted");
        assert!(app.held_items.values().all(|h| h.review_cleared.is_none()));
    }

    #[tokio::test]
    async fn a_full_reload_that_still_names_you_keeps_the_row_off_the_review_list() {
        let (calls, _gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = review_list_app(true);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        app.execute_action(Action::PrVote(ReviewVote::Approved), &deps).await;
        assert!(app.prs.is_empty());

        // The reload that was already out when you voted still names you as a reviewer.
        let mut r = reloaded(vec![pr_row(pr_awaiting_me())]);
        r.pr_pool.review_clears_request.insert("c".into(), true);
        app.on_event(AppEvent::Reloaded(Box::new(r)), &deps);
        assert!(app.prs.is_empty(), "the full reload cannot put it back either");
        assert!(app.lp_prs_review.is_empty(), "nor the Launchpad's review bucket");
        assert_eq!(app.derive_pr_rows(PullRequestFilter::All, false).len(), 1, "it is still a PR in All");
    }

    /// `fetch_all` hands the pool over early and then again inside the full reload, so a
    /// caught-up pool that lands before the provider's answer lands twice while your vote is
    /// still held — and must not put you back on the row the forge has dropped you from.
    #[tokio::test]
    async fn a_caught_up_pool_landing_before_the_answer_and_again_after_never_relists_the_pr() {
        let (calls, gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = review_list_app(true);
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        app.execute_action(Action::PrVote(ReviewVote::Approved), &deps).await;

        let gone = || pool_where_review_clears(vec![pr_row(pr(None))], true);
        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(gone()), ok: true }, &deps);
        assert!(app.prs.is_empty(), "the forge has dropped you: still off the list");
        assert!(app.held_items.is_empty(), "nothing is held once the forge's row shows the review was taken");

        let mut r = reloaded(vec![pr_row(pr(None))]);
        r.pr_pool.review_clears_request.insert("c".into(), true);
        app.on_event(AppEvent::Reloaded(Box::new(r)), &deps);
        assert!(app.prs.is_empty(), "the same pool again does not put your vote — and you — back on the row");
        assert!(app.pr_pool.open[0].pr.reviewers.is_empty());

        // The provider's answer (a re-read without the vote, say the reviews call failed)
        // changes nothing on the list either.
        gate.notify_one();
        let event = answer(&mut rx).await;
        app.on_event(event, &deps);
        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(gone()), ok: true }, &deps);
        assert!(app.prs.is_empty());
        assert!(app.held_items.is_empty());
    }

    #[test]
    fn a_hold_that_ran_out_starts_afresh_instead_of_reviving_what_it_held() {
        let mut app = App::new("slate");
        let key = pr_detail_cache_key("c", &pr(None).item_ref());
        app.hold(&key).review_cleared = Some(1);
        app.held_items.get_mut(&key).unwrap().until = Utc::now() - chrono::Duration::seconds(1);
        // Sixty seconds later a comment on the same PR takes a hold again…
        let hold = app.hold(&key);
        assert_eq!(hold.review_cleared, None, "…without bringing back the clearing the provider has had the last word on");
        assert!(hold.until > Utc::now());
    }

    /// A reload whose `/user` call failed (a rate limit, say) must not turn "Mine" into
    /// everyone's pull requests: the identity the previous reload established still filters it.
    #[tokio::test]
    async fn a_reload_that_cannot_say_who_you_are_keeps_the_last_identity() {
        let deps = deps_with_items(ItemCalls::new()).await;
        let mut app = App::new("slate");
        app.pr_filter = PullRequestFilter::Mine;
        let mine = || pool_pr("c", "1", "me", &[]);
        let theirs = || pool_pr("c", "2", "sam", &[]);
        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(pool_of(vec![mine(), theirs()], Some("me"))), ok: true }, &deps);
        assert_eq!(ids(&app.prs), vec!["1"], "Mine is yours");

        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(pool_of(vec![mine(), theirs()], None)), ok: true }, &deps);
        assert_eq!(app.pr_pool.me.get("c"), Some(&Some("me".to_string())), "the identity carries over");
        assert_eq!(ids(&app.prs), vec!["1"], "so Mine is still yours, not everyone's");

        // One that answers is taken as it is, as ever.
        app.on_event(AppEvent::PrPoolLoaded { pool: Box::new(pool_of(vec![mine(), theirs()], Some("sam"))), ok: true }, &deps);
        assert_eq!(ids(&app.prs), vec!["2"]);
    }

    #[tokio::test]
    async fn a_refused_approval_puts_the_pr_back_on_the_review_list() {
        let calls = ItemCalls { refuse: true, ..ItemCalls::new() };
        let deps = deps_with_items(calls.clone()).await;
        let mut app = review_list_app(true);

        app.execute_action(Action::PrVote(ReviewVote::Approved), &deps).await;
        assert_eq!(calls.calls(), vec!["vote 1 Approved".to_string()]);
        assert_eq!(app.prs.len(), 1, "the forge still wants the review, so it is listed again");
        assert_eq!(my_vote(&app.prs[0].pr), Some(ReviewVote::NoVote), "without a vote");
        assert_eq!(app.pr_state.selected(), Some(0), "and selected");
        assert!(app.held_items.is_empty());
    }

    #[tokio::test]
    async fn a_refused_approval_takes_your_vote_back_and_returns_the_pr_to_the_launchpad() {
        let calls = ItemCalls { refuse: true, ..ItemCalls::new() };
        let deps = deps_with_items(calls.clone()).await;
        let mut app = pr_action_app();

        app.execute_action(Action::PrVote(ReviewVote::Approved), &deps).await;
        assert_eq!(calls.calls(), vec!["vote 1 Approved".to_string()]);
        assert_eq!(my_vote(&pr_pane(&app).pr), None, "rolled back on the view");
        assert_eq!(my_vote(&app.prs[0].pr), None, "and on the list row");
        assert!(on_launchpad(&app), "back on the Launchpad");
        assert!(app.held_items.is_empty());
        assert!(app.toast.as_deref().is_some_and(|t| t.contains("Failed") && t.contains("the provider said no")), "{:?}", app.toast);
    }

    #[tokio::test]
    async fn a_comment_shows_before_the_provider_answers_and_survives_a_detail_that_hasnt_caught_up() {
        let (calls, gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = pr_action_app();
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);

        app.execute_action(Action::PrComment("LGTM".into()), &deps).await;
        assert!(calls.calls().is_empty(), "the provider hasn't answered yet");
        let threads = &pr_pane(&app).diff.threads;
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].comments[0].body, "LGTM");
        assert!(forgetop_core::filter::is_user(&threads[0].comments[0].author, "me"), "posted as you");

        gate.notify_one();
        let event = answer(&mut rx).await;
        app.on_event(event, &deps);
        assert_eq!(calls.calls(), vec!["comment 1 LGTM".to_string()]);
        assert_eq!(app.toast.as_deref(), Some("Comment added"));

        // A detail fetch that doesn't list it yet keeps it on screen…
        app.on_event(pr_detail_with(vec![comment_by("t1", "sam", "LGTM")]), &deps);
        let bodies: Vec<_> = pr_pane(&app).diff.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(bodies.len(), 2, "someone else's LGTM was already there; yours is still held: {bodies:?}");

        // …and once the provider lists it, its copy replaces the placeholder.
        app.on_event(pr_detail_with(vec![comment_by("t1", "sam", "LGTM"), comment_by("t2", "me", "LGTM")]), &deps);
        let ids: Vec<_> = pr_pane(&app).diff.threads.iter().map(|t| t.id.clone()).collect();
        assert_eq!(ids, vec!["t1".to_string(), "t2".to_string()]);
        assert!(app.held_items.is_empty());
    }

    #[tokio::test]
    async fn a_refused_comment_is_taken_back() {
        let calls = ItemCalls { refuse: true, ..ItemCalls::new() };
        let deps = deps_with_items(calls.clone()).await;
        let mut app = pr_action_app();

        app.execute_action(Action::PrComment("LGTM".into()), &deps).await;
        assert!(pr_pane(&app).diff.threads.is_empty());
        assert!(app.held_items.is_empty());
        assert!(app.toast.as_deref().is_some_and(|t| t.contains("the provider said no")), "{:?}", app.toast);
    }

    #[tokio::test]
    async fn a_reply_lands_on_its_thread_at_once() {
        let (calls, gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = pr_action_app();
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        if let Screen::PrView(v) = &mut app.screen {
            v.diff.threads = vec![comment_by("t1", "sam", "why?")];
            v.reply_target = Some("t1".into());
        }

        app.execute_action(Action::PrReply("because".into()), &deps).await;
        let thread = &pr_pane(&app).diff.threads[0];
        assert_eq!(thread.comments.iter().map(|c| c.body.as_str()).collect::<Vec<_>>(), vec!["why?", "because"]);
        assert!(pr_pane(&app).reply_target.is_none());

        gate.notify_one();
        let event = answer(&mut rx).await;
        app.on_event(event, &deps);
        assert_eq!(calls.calls(), vec!["reply 1 t1 because".to_string()]);
    }

    #[tokio::test]
    async fn a_review_empties_the_buffer_at_once_and_a_refusal_puts_its_comments_back() {
        let note = LineComment { path: "src/a.rs".into(), line: 3, side: DiffSide::New, body: "nit".into() };

        let (calls, gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = pr_action_app();
        app.screen = pr_view_with_pending(vec![note.clone()]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);
        app.execute_action(Action::SubmitReview(ReviewVote::Approved), &deps).await;
        assert!(pr_pane(&app).pending.is_empty(), "the buffer is sent");
        let threads = &pr_pane(&app).diff.threads;
        assert_eq!((threads.len(), threads[0].file_path.as_deref(), threads[0].line), (1, Some("src/a.rs"), Some(3)));
        assert_eq!(my_vote(&pr_pane(&app).pr), Some(ReviewVote::Approved));
        gate.notify_one();
        let event = answer(&mut rx).await;
        app.on_event(event, &deps);
        assert_eq!(calls.calls(), vec!["review 1 Approved 1".to_string()]);

        let calls = ItemCalls { refuse: true, ..ItemCalls::new() };
        let deps = deps_with_items(calls).await;
        let mut app = pr_action_app();
        app.screen = pr_view_with_pending(vec![note]);
        app.execute_action(Action::SubmitReview(ReviewVote::Approved), &deps).await;
        assert_eq!(pr_pane(&app).pending.len(), 1, "the comments are back in the buffer");
        assert!(pr_pane(&app).diff.threads.is_empty());
        assert_eq!(my_vote(&pr_pane(&app).pr), None);
        assert!(on_launchpad(&app));
        assert!(app.toast.as_deref().is_some_and(|t| t.starts_with("Submit failed")), "{:?}", app.toast);
    }

    #[tokio::test]
    async fn a_merge_is_reported_once_the_provider_has_done_it_without_holding_the_screen() {
        let (calls, gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = pr_action_app();
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);

        app.execute_action(Action::PrMerge(MergeStrategy::Squash), &deps).await;
        assert_eq!(app.toast.as_deref(), Some("Merging…"));
        assert_eq!(pr_pane(&app).pr.status, PullRequestStatus::Open, "whether it merges is the provider's call");
        assert!(on_launchpad(&app));

        gate.notify_one();
        let event = answer(&mut rx).await;
        app.on_event(event, &deps);
        assert_eq!(calls.calls(), vec!["merge 1".to_string()]);
        assert_eq!(app.toast.as_deref(), Some("Merged (Squash)"));
        assert!(!on_launchpad(&app));
    }

    fn wi_action_app() -> App {
        let mut app = App::new("slate");
        app.active = 1;
        app.wis = vec![wi_row(wi(None))];
        app.screen = Screen::WiView(Box::new(WiView {
            timeline: Vec::new(),
            connection_id: "c".into(),
            wi: wi(None),
            threads: Vec::new(),
            scroll: 0,
        }));
        app
    }

    fn wi_pane(app: &App) -> &WiView {
        let Screen::WiView(v) = &app.screen else { panic!("expected the work-item view") };
        v
    }

    #[tokio::test]
    async fn a_state_change_shows_before_the_provider_answers_and_a_stale_refresh_keeps_it() {
        let (mut calls, gate) = ItemCalls::gated();
        calls.wi = WorkItem { state: "Done".into(), state_category: WorkItemStateCategory::Completed, ..wi(None) };
        let deps = deps_with_items(calls.clone()).await;
        let mut app = wi_action_app();
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);

        app.execute_action(Action::WiSetState("Done".into()), &deps).await;
        assert!(calls.calls().is_empty(), "the provider hasn't answered yet");
        assert_eq!(wi_pane(&app).wi.state, "Done");
        assert_eq!(app.wis[0].wi.state, "Done", "and on the row behind it");

        // A refresh asked for before the write still says Todo: it doesn't flip the row back.
        let mut r = reloaded_with_health(Vec::new());
        r.wis = vec![wi_row(wi(None))];
        app.on_event(AppEvent::Reloaded(Box::new(r)), &deps);
        assert_eq!(app.wis[0].wi.state, "Done", "held until the provider catches up");

        gate.notify_one();
        let event = answer(&mut rx).await;
        app.on_event(event, &deps);
        assert_eq!(calls.calls(), vec!["state w Done".to_string()]);
        assert_eq!(wi_pane(&app).wi.state_category, WorkItemStateCategory::Completed, "the provider's bucket for it");
        assert_eq!(app.wis[0].wi.state_category, WorkItemStateCategory::Completed);
        assert!(app.held_items.is_empty(), "its copy showed the change");
    }

    #[tokio::test]
    async fn a_refused_state_change_or_comment_puts_the_item_back() {
        let calls = ItemCalls { refuse: true, ..ItemCalls::new() };
        let deps = deps_with_items(calls.clone()).await;
        let mut app = wi_action_app();

        app.execute_action(Action::WiSetState("Done".into()), &deps).await;
        assert_eq!(wi_pane(&app).wi.state, "Todo");
        assert_eq!(app.wis[0].wi.state, "Todo");
        assert!(app.toast.as_deref().is_some_and(|t| t.contains("the provider said no")), "{:?}", app.toast);

        app.execute_action(Action::WiComment("on it".into()), &deps).await;
        assert!(wi_pane(&app).threads.is_empty());
        assert!(app.held_items.is_empty());
        assert_eq!(calls.calls(), vec!["state w Done".to_string(), "wi-comment w on it".to_string()]);
    }

    #[tokio::test]
    async fn a_work_item_comment_shows_before_the_provider_answers() {
        let (calls, gate) = ItemCalls::gated();
        let deps = deps_with_items(calls.clone()).await;
        let mut app = wi_action_app();
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.job_tx = Some(tx);

        app.execute_action(Action::WiComment("on it".into()), &deps).await;
        assert_eq!(wi_pane(&app).threads.len(), 1);
        assert_eq!(wi_pane(&app).threads[0].comments[0].body, "on it");
        gate.notify_one();
        let event = answer(&mut rx).await;
        app.on_event(event, &deps);
        assert_eq!(calls.calls(), vec!["wi-comment w on it".to_string()]);
        assert_eq!(wi_pane(&app).threads.len(), 1, "still shown while the provider catches up");
    }
}
