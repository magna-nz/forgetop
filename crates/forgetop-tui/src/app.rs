//! Application state and the (async) update logic driven by the event loop.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Local, Utc};
use forgetop_core::config::{NotificationPrefs, SavedView, SortPref};
use forgetop_core::domain::*;
use forgetop_core::provider::*;
use forgetop_core::service::{ConfigService, ConnectionHealth, ConnectionHealthService, SectionService};
use ratatui::widgets::TableState;
use tokio::sync::mpsc;

use crate::launchpad;
use crate::overlay::{Action, InputKind, Outcome, Overlay, PickerKind, ToggleItem, ToggleKind};
use crate::palette::{self, PaletteKind};
use crate::theme::Theme;
use crate::wizard::{provider_sections, section_label, Wizard, WizardOutcome};

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
}

/// A completed background job, applied to the app on the render loop.
pub enum AppEvent {
    /// A full refresh finished fetching.
    Reloaded(Box<Reloaded>),
}

/// A snapshot of everything the background fetch needs from `self` at spawn time, so it
/// can run without borrowing the app.
struct ReloadParams {
    pr_filter: PullRequestFilter,
    pr_completed: bool,
    notifications: NotificationPrefs,
    review_seen: HashSet<(String, String)>,
    pr_review_seen: HashMap<(String, String), (bool, bool)>,
    scan_seeded: bool,
    notifier: Arc<dyn Notifier>,
    /// The open pipeline view's (connection_id, run_id), so the refresh keeps it live.
    open_pipeline: Option<(String, String)>,
}

/// The PR-notification scan's new seen-sets (notifications are fired during the scan).
struct PrScan {
    review_seen: Option<HashSet<(String, String)>>,
    pr_review_seen: Option<HashMap<(String, String), (bool, bool)>>,
}

/// The result of a full fetch, ready to be folded back into the app.
pub struct Reloaded {
    prs: Vec<PrRow>,
    wis: Vec<WiRow>,
    pipes: Vec<PipeRow>,
    inbox: Vec<NotifRow>,
    lp_prs_mine: Vec<PrRow>,
    lp_prs_review: Vec<PrRow>,
    health: Vec<ConnectionHealth>,
    scan: Option<PrScan>,
    /// Fresh (run_id, run, approvals) for the open pipeline view, if one is open.
    open_pipeline: Option<(String, PipelineRun, Vec<PipelineApproval>)>,
    errors: Vec<String>,
}

/// One pipeline run, tagged with the connection it came from (for the provider column).
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

/// A pull request tagged with the connection it came from (for aggregation).
pub struct PrRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub pr: PullRequest,
}

/// A work item tagged with the connection it came from (for aggregation).
pub struct WiRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub wi: WorkItem,
}

/// One notification tagged with the connection it came from (for the inbox).
pub struct NotifRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub notification: Notification,
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
    pub pr_state: TableState,
    pub wi_state: TableState,
    pub pipe_state: TableState,
    pub health: Vec<ConnectionHealth>,
    /// Which sections are shown in the tab bar, indexed by section (0=PR,1=WI,2=Pipelines).
    pub visible: [bool; 3],
    pub status: String,
    pub loading: bool,
    /// Scroll offset for the current list; body height captured during render.
    pub list_scroll: u16,
    pub content_h: u16,
    /// Max scroll offset for the open PR/WI view, captured during render for clamping.
    pub detail_scroll_max: u16,
    pub pr_filter: PullRequestFilter,
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
    /// Saved views per section (0=PR, 1=WI, 2=Pipelines) and the active index.
    pub views: [Vec<SavedView>; 3],
    pub view_idx: [usize; 3],
    /// Which desktop notifications are enabled. Persisted.
    pub notifications: NotificationPrefs,
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
    /// Pending-approval gates offered by the current approval picker, indexed by
    /// the picker selection. Rebuilt each time the picker opens.
    approval_choices: Vec<ApprovalChoice>,
    /// Open modal overlay, if any. When set, keys route here instead of the table.
    pub overlay: Option<Overlay>,
    /// Add-connection wizard, if running. Takes priority over the overlay/screens.
    pub wizard: Option<Wizard>,
    /// Current screen — the list, or a full-screen sub-view like the PR diff.
    pub screen: Screen,
    /// Launchpad rows (grouped + sorted), rebuilt each refresh.
    pub lp: Vec<launchpad::Entry>,
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
    pub last_refresh: DateTime<Local>,
    pub should_quit: bool,
}

/// Full-screen views layered above the list. The large views are boxed so the
/// common `List` state doesn't bloat every `Screen` value.
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

/// State for the full-screen work-item view.
pub struct WiView {
    pub connection_id: String,
    pub wi: WorkItem,
    pub threads: Vec<CommentThread>,
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
    run_id: String,
    approval_id: String,
    decision: ApprovalDecision,
    /// Gate label, for confirm/toast messages.
    label: String,
}

/// A scrollable log view over one job, shown within the pipeline drill-in.
pub struct LogView {
    pub title: String,
    pub lines: Vec<String>,
    pub scroll: u16,
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
    /// Whether this run's provider can surface pending approvals.
    pub supports_approvals: bool,
    /// Whether the app can actually submit an approve/reject here (false = view-only,
    /// e.g. Azure — we show the gate but can't act on it).
    pub can_respond_approvals: bool,
    /// Gates on this run currently awaiting a decision.
    pub approvals: Vec<PipelineApproval>,
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
            supports_approvals: false,
            can_respond_approvals: false,
            approvals: Vec::new(),
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
            wis: Vec::new(),
            pipes: Vec::new(),
            inbox: Vec::new(),
            inbox_sel: 0,
            pr_state: TableState::default(),
            wi_state: TableState::default(),
            pipe_state: TableState::default(),
            health: Vec::new(),
            visible: [true; 3],
            status: "Loading…".into(),
            loading: true,
            list_scroll: 0,
            content_h: 0,
            detail_scroll_max: 0,
            pr_filter: PullRequestFilter::All,
            pr_shown_statuses: [PullRequestStatus::Open, PullRequestStatus::Draft].into_iter().collect(),
            filters: [String::new(), String::new(), String::new()],
            filtering: false,
            wi_hidden_states: HashSet::new(),
            pr_sort: None,
            wi_sort: None,
            pipe_sort: None,
            views: [Vec::new(), Vec::new(), Vec::new()],
            view_idx: [0, 0, 0],
            notifications: NotificationPrefs::default(),
            notifier: Arc::new(SystemNotifier),
            pipe_seen: HashMap::new(),
            approval_seen: HashSet::new(),
            approval_seeded: false,
            pipe_seeded: false,
            pr_review_seen: HashMap::new(),
            review_req_seen: HashSet::new(),
            pr_scan_seeded: false,
            toast: None,
            approval_choices: Vec::new(),
            overlay: None,
            wizard: None,
            screen: Screen::Launchpad,
            lp: Vec::new(),
            lp_side: 0,
            lp_sel: [0, 0],
            lp_prs_mine: Vec::new(),
            lp_prs_review: Vec::new(),
            lp_dismissed: HashSet::new(),
            lp_origin: false,
            from_inbox: false,
            reloading: false,
            job_tx: None,
            anim: 0,
            last_refresh: Local::now(),
            should_quit: false,
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

    fn filtered_len(&self, section: usize) -> usize {
        match section {
            0 => self.filtered_pr_indices().len(),
            1 => self.filtered_wi_indices().len(),
            _ => self.filtered_pipe_indices().len(),
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

    /// The top-level tab position: 0 = Launchpad, then each visible section.
    fn top_pos(&self) -> usize {
        if matches!(self.screen, Screen::Launchpad) {
            0
        } else {
            1 + self.visible_indices().iter().position(|&i| i == self.active).unwrap_or(0)
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
                let mut errors = Vec::new();
                self.reload_pull_requests(deps, &mut errors).await;
                if let Some(e) = errors.first() {
                    self.toast = Some(e.clone());
                }
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
        let _ = deps.config.set_views(section_of(section), self.views[section].clone()).await;
        self.toast = Some(format!("Saved view: {name}"));
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
        let _ = deps.config.set_views(section_of(section), self.views[section].clone()).await;
        let target = self.view_idx[section];
        self.apply_view(section, target, deps).await;
        self.toast = Some(format!("Deleted view: {}", removed.name));
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
            Some(Overlay::Toggle { title: "Notifications".into(), kind: ToggleKind::Notifications, min_one: false, items, selected: 0 });
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
    pub fn on_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Reloaded(r) => self.apply_reloaded(*r),
        }
    }


    /// Rebuilds the Launchpad rows from the current feeds (no fetch) and clamps selection.
    fn rebuild_launchpad(&mut self) {
        self.lp = launchpad::build(&self.lp_prs_review, &self.lp_prs_mine, &self.wis, &self.pipes);
        // Drop anything already acted on this session (e.g. a PR you've reviewed).
        self.lp.retain(|e| !self.lp_dismissed.contains(&launchpad::Entry::key(&e.connection_id, e.item_id())));
        for side in 0..2 {
            let len = self.lp_column(side).len();
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

    /// Indices into `self.lp` for a column (0 = left, 1 = right), in display order.
    pub fn lp_column(&self, side: usize) -> Vec<usize> {
        self.lp.iter().enumerate().filter(|(_, e)| e.bucket.column() == side).map(|(i, _)| i).collect()
    }

    /// The `self.lp` index of the currently-focused Launchpad row, if any.
    fn lp_selected(&self) -> Option<usize> {
        self.lp_column(self.lp_side).get(self.lp_sel[self.lp_side]).copied()
    }

    fn lp_move(&mut self, delta: isize) {
        let len = self.lp_column(self.lp_side).len();
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

    async fn on_launchpad_key(&mut self, key: Key, deps: &AppDeps) {
        match key {
            Key::Char('q') => self.should_quit = true,
            Key::Up | Key::Char('k') => self.lp_move(-1),
            Key::Down | Key::Char('j') => self.lp_move(1),
            // Left/right move between the two columns; Tab leaves for the section tabs.
            Key::Left | Key::Char('h') => self.lp_switch_side(-1),
            Key::Right | Key::Char('l') => self.lp_switch_side(1),
            Key::Tab => self.switch_tab(1),
            Key::Char(c @ '1'..='4') => self.set_tab(c as usize - '1' as usize),
            Key::Enter => self.open_launchpad_selected(deps).await,
            Key::Char('r') => self.request_reload(deps),
            Key::Char('n') => self.start_add_connection(),
            Key::Char('C') => self.open_config(deps).await,
            Key::Char('t') => {
                let next = Theme::next(self.theme.name);
                self.theme = Theme::by_name(next);
                let _ = deps.config.set_theme(Some(next.to_string())).await;
            }
            _ => {}
        }
    }

    /// Opens the selected Launchpad row in its full item view.
    async fn open_launchpad_selected(&mut self, deps: &AppDeps) {
        let Some(entry) = self.lp_selected().and_then(|i| self.lp.get(i)) else { return };
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
                    self.open_pr_view_for(deps, 0, id, label, url, conn, pr).await;
                }
            }
            launchpad::EntryKind::Wi => {
                if let Some(wi) = self.wis.iter().find(|r| r.connection_id == conn && r.wi.id == id).map(|r| r.wi.clone()) {
                    self.open_wi_view_for(deps, id, conn, wi).await;
                }
            }
            launchpad::EntryKind::Pipe => {
                let found = self
                    .pipes
                    .iter()
                    .find(|r| r.connection_id == conn && r.run.id == id)
                    .map(|r| (r.provider, r.run.definition_id.clone(), r.run.branch.clone(), pipe_label(r), r.run.clone()));
                if let Some((provider, def, branch, title, fallback)) = found {
                    self.open_pipeline_for(deps, conn, provider, id, def, branch, title, fallback).await;
                }
            }
        }
    }

    // ---- command palette ----

    /// Open the command palette over the current screen, seeded with every already-fetched
    /// PR / work item / pipeline. Empty query → all, most-recent first.
    fn open_palette(&mut self) {
        let candidates = palette::build_candidates(&self.prs, &self.wis, &self.pipes);
        let results = palette::rank("", &candidates);
        self.overlay = Some(Overlay::Palette { query: String::new(), candidates, results, selected: 0 });
    }

    /// Open the item chosen in the palette, re-resolving the full struct from the section
    /// lists by `(kind, id)` and reusing the same open path as selecting it on its screen.
    async fn open_palette_item(&mut self, kind: PaletteKind, id: String, conn: String, deps: &AppDeps) {
        // Esc from the opened view should return to wherever the palette was invoked from.
        self.lp_origin = matches!(self.screen, Screen::Launchpad);
        self.from_inbox = false;
        match kind {
            PaletteKind::Pr => {
                let found = self
                    .prs
                    .iter()
                    .find(|r| r.connection_id == conn && r.pr.id == id)
                    .map(|r| (pr_label(&r.pr), r.pr.url.clone(), r.pr.clone()));
                if let Some((label, url, pr)) = found {
                    self.open_pr_view_for(deps, 0, id, label, url, conn, pr).await;
                }
            }
            PaletteKind::Wi => {
                if let Some(wi) = self.wis.iter().find(|r| r.connection_id == conn && r.wi.id == id).map(|r| r.wi.clone()) {
                    self.open_wi_view_for(deps, id, conn, wi).await;
                }
            }
            PaletteKind::Pipe => {
                let found = self
                    .pipes
                    .iter()
                    .find(|r| r.connection_id == conn && r.run.id == id)
                    .map(|r| (r.provider, r.run.definition_id.clone(), r.run.branch.clone(), pipe_label(r), r.run.clone()));
                if let Some((provider, def, branch, title, fallback)) = found {
                    self.open_pipeline_for(deps, conn, provider, id, def, branch, title, fallback).await;
                }
            }
        }
    }

    // ---- notification inbox ----

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

        self.mark_inbox_read(&conn, &notif_id, deps).await;

        match (item_type, item_id) {
            (NotificationItemType::PullRequest, Some(id)) => {
                if let Some(src) = self.pr_source_for(&conn, deps).await {
                    if let Ok(pr) = src.get(&id).await {
                        self.from_inbox = true;
                        self.lp_origin = false;
                        let (label, purl) = (pr_label(&pr), pr.url.clone());
                        self.open_pr_view_for(deps, 0, id, label, purl, conn, pr).await;
                        return;
                    }
                }
            }
            (NotificationItemType::WorkItem, Some(id)) => {
                if let Some(src) = self.wi_source_for(&conn, deps).await {
                    if let Ok(wi) = src.get(&id).await {
                        self.from_inbox = true;
                        self.lp_origin = false;
                        self.open_wi_view_for(deps, id, conn, wi).await;
                        return;
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
            self.mark_inbox_read(&conn, &id, deps).await;
            self.toast = Some("Marked read".into());
        }
    }

    async fn mark_inbox_read(&mut self, conn: &str, notif_id: &str, deps: &AppDeps) {
        for row in &mut self.inbox {
            if row.connection_id == conn && row.notification.id == notif_id {
                row.notification.unread = false;
            }
        }
        if let Ok(feeds) = deps.sections.notification_feeds().await {
            if let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == conn) {
                let _ = feed.source.mark_read(notif_id).await;
            }
        }
    }

    async fn mark_all_inbox_read(&mut self, deps: &AppDeps) {
        for row in &mut self.inbox {
            row.notification.unread = false;
        }
        if let Ok(feeds) = deps.sections.notification_feeds().await {
            for feed in feeds {
                let _ = feed.source.mark_all_read().await;
            }
        }
        self.toast = Some("All marked read".into());
    }

    // ---- data loading ----

    /// A full refresh, run inline (blocking). Used for the initial load and after write
    /// actions, where the caller wants the fresh data before continuing. The periodic
    /// poll and manual `r` refresh go through [`request_reload`] instead, which runs the
    /// same fetch off the render loop so the UI stays live (see the header spinner).
    pub async fn reload_all(&mut self, deps: &AppDeps) {
        self.loading = true;
        self.status = "Refreshing…".into();
        let fetched = self.fetch_bundle(deps).await;
        self.apply_reloaded(fetched);
    }

    /// Parameters the background fetch needs, snapshotted from `self` at spawn time.
    fn reload_params(&self) -> ReloadParams {
        ReloadParams {
            pr_filter: self.pr_filter,
            pr_completed: self.pr_wants_completed(),
            notifications: self.notifications,
            review_seen: self.review_req_seen.clone(),
            pr_review_seen: self.pr_review_seen.clone(),
            scan_seeded: self.pr_scan_seeded,
            notifier: self.notifier.clone(),
            open_pipeline: match &self.screen {
                Screen::Pipeline(v) => Some((v.connection_id.clone(), v.run.id.clone())),
                _ => None,
            },
        }
    }

    /// Runs the whole network fetch (no `&mut self`), firing PR-notification pings along
    /// the way. Safe to call from a spawned task — everything it needs is owned.
    async fn fetch_all(deps: AppDeps, p: ReloadParams) -> Reloaded {
        let mut errors = Vec::new();
        let prs = fetch_pull_requests(&deps, p.pr_filter, p.pr_completed, &mut errors).await;
        let wis = fetch_work_items(&deps, &mut errors).await;
        let pipes = fetch_pipelines(&deps, &mut errors).await;
        let inbox = fetch_notifications(&deps, &mut errors).await;
        let (lp_prs_mine, lp_prs_review) = fetch_launchpad_prs(&deps).await;
        let health = deps.health.check_all().await;
        let scan = scan_pr_notifications(&deps, &p).await;
        let open_pipeline = match &p.open_pipeline {
            Some((conn_id, run_id)) => fetch_open_pipeline(&deps, conn_id, run_id).await,
            None => None,
        };
        Reloaded { prs, wis, pipes, inbox, lp_prs_mine, lp_prs_review, health, scan, open_pipeline, errors }
    }

    /// The inline (blocking) variant used by [`reload_all`].
    async fn fetch_bundle(&self, deps: &AppDeps) -> Reloaded {
        Self::fetch_all(deps.clone(), self.reload_params()).await
    }

    /// Folds a completed fetch back into the app state: the lists, the Launchpad, the
    /// pipeline notifications (which compare against the seen-sets), and the status line.
    fn apply_reloaded(&mut self, r: Reloaded) {
        self.prs = r.prs;
        self.wis = r.wis;
        self.pipes = r.pipes;
        self.inbox = r.inbox;
        if self.inbox_sel >= self.inbox.len() {
            self.inbox_sel = self.inbox.len().saturating_sub(1);
        }
        self.lp_prs_mine = r.lp_prs_mine;
        self.lp_prs_review = r.lp_prs_review;
        self.health = r.health;
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
                    v.run = run;
                    v.approvals = approvals;
                    v.clamp_selection();
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
            for e in &r.errors {
                forgetop_core::diag::log("fetch", e);
            }
            r.errors.join("  |  ")
        };
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

    async fn reload_pull_requests(&mut self, deps: &AppDeps, errors: &mut Vec<String>) {
        self.prs.clear();
        match deps.sections.pull_request_feeds().await {
            Ok(feeds) => {
                let query = PullRequestQuery {
                    filter: self.pr_filter,
                    include_completed: self.pr_wants_completed(),
                    limit: Some(50),
                };
                for feed in feeds {
                    let (provider, name, conn_id) = feed_tag(&feed.connection);
                    match feed.source.list(&query).await {
                        Ok(list) => self.prs.extend(list.into_iter().map(|pr| PrRow {
                            connection_id: conn_id.clone(),
                            connection: name.clone(),
                            provider,
                            pr,
                        })),
                        Err(e) => errors.push(format!("PRs ({name}): {e}")),
                    }
                }
            }
            Err(e) => errors.push(format!("PRs: {e}")),
        }
    }

    async fn reload_work_items(&mut self, deps: &AppDeps, errors: &mut Vec<String>) {
        self.wis.clear();
        match deps.sections.work_item_feeds().await {
            Ok(feeds) => {
                for feed in feeds {
                    let (provider, name, conn_id) = feed_tag(&feed.connection);
                    match feed.source.list(&wi_query()).await {
                        Ok(list) => self.wis.extend(list.into_iter().map(|wi| WiRow {
                            connection_id: conn_id.clone(),
                            connection: name.clone(),
                            provider,
                            wi,
                        })),
                        Err(e) => errors.push(format!("Work items ({name}): {e}")),
                    }
                }
            }
            Err(e) => errors.push(format!("Work items: {e}")),
        }
    }

    async fn reload_pipelines(&mut self, deps: &AppDeps, errors: &mut Vec<String>) {
        self.pipes.clear();
        match deps.sections.pipeline_feeds().await {
            Ok(feeds) => {
                for feed in feeds {
                    let provider = feed.connection.provider_type();
                    let name = feed.connection.display_name().to_string();
                    let conn_id = feed.connection.connection_id().to_string();
                    // Map definition_id → pipeline name so rows can show the pipeline
                    // (e.g. "CI Build") separately from the run/release (e.g. "10.1.100").
                    let def_names: HashMap<String, String> =
                        feed.source.discover().await.unwrap_or_default().into_iter().map(|d| (d.id, d.name)).collect();
                    for q in feed_queries(&feed.subscription) {
                        match feed.source.list_runs(&q).await {
                            Ok(runs) => {
                                let supports = feed.source.supports_approvals();
                                for run in runs {
                                    // Only in-flight runs can be waiting on a gate — bound the
                                    // extra per-run approval calls to those.
                                    let awaiting_approval = supports
                                        && is_active(run.status)
                                        && feed
                                            .source
                                            .pending_approvals(&run.id)
                                            .await
                                            .map(|a| a.iter().any(|x| x.can_respond))
                                            .unwrap_or(false);
                                    let definition_name = def_names.get(&run.definition_id).cloned();
                                    self.pipes.push(PipeRow {
                                        connection_id: conn_id.clone(),
                                        connection: name.clone(),
                                        provider,
                                        run,
                                        definition_name,
                                        awaiting_approval,
                                    });
                                }
                            }
                            Err(e) => errors.push(format!("Pipelines ({name}): {e}")),
                        }
                    }
                }
            }
            Err(e) => errors.push(format!("Pipelines: {e}")),
        }
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
        // Help and the notifications chooser are available anywhere.
        if key == Key::Char('?') {
            self.overlay = Some(Overlay::Help { scroll: 0 });
            return;
        }
        if key == Key::Char('N') {
            self.open_notifications_toggle();
            return;
        }
        // Ctrl-P opens the command palette from the list screens and the Launchpad.
        if key == Key::Ctrl('p') && matches!(self.screen, Screen::List | Screen::Launchpad) {
            self.open_palette();
            return;
        }
        // `i` opens the notification inbox from the list screens and the Launchpad.
        if key == Key::Char('i') && matches!(self.screen, Screen::List | Screen::Launchpad) {
            self.inbox_sel = self.inbox_sel.min(self.inbox.len().saturating_sub(1));
            self.screen = Screen::Inbox;
            return;
        }

        // Full-screen sub-views handle their own keys.
        match self.screen {
            Screen::Pipeline(_) => {
                // An open log pane captures scroll/close; `L` fetches logs (async).
                let logs_open = matches!(&self.screen, Screen::Pipeline(v) if v.logs.is_some());
                if logs_open {
                    self.on_pipeline_logs_key(key);
                } else if key == Key::Char('L') {
                    self.open_pipeline_logs(deps).await;
                } else {
                    self.on_pipeline_key(key);
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
            // Esc clears an active quick filter first, then quits.
            Key::Escape => {
                if self.filters[self.active].is_empty() {
                    self.should_quit = true;
                } else {
                    self.filters[self.active].clear();
                    self.reset_filter_selection();
                }
            }
            Key::Left => self.switch_tab(-1),
            Key::Right => self.switch_tab(1),
            Key::Tab => self.switch_tab(1),
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
                    0 => self.open_pr_view(deps, 0).await,
                    1 => self.open_wi_view(deps).await,
                    2 => self.open_pipeline(deps).await,
                    _ => {}
                }
            }
            Key::Char(c) => self.on_char(c, deps).await,
            Key::Backspace | Key::Ctrl(_) | Key::Quit | Key::Redraw | Key::None => {}
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

    async fn open_pr_view(&mut self, deps: &AppDeps, tab: usize) {
        let (id, label, url, conn_id, pr) = match self.selected_pr_row() {
            Some(row) => (row.pr.id.clone(), pr_label(&row.pr), row.pr.url.clone(), row.connection_id.clone(), row.pr.clone()),
            None => return,
        };
        self.open_pr_view_for(deps, tab, id, label, url, conn_id, pr).await;
    }

    /// Opens the PR view for an explicit PR (used by the Launchpad, where the item
    /// isn't the section list's selected row).
    #[allow(clippy::too_many_arguments)]
    async fn open_pr_view_for(&mut self, deps: &AppDeps, tab: usize, id: String, label: String, url: Option<String>, conn_id: String, pr: PullRequest) {
        let source = match self.pr_source_for(&conn_id, deps).await {
            Some(s) => s,
            None => {
                self.toast = Some("No pull-request provider is bound".into());
                return;
            }
        };
        let threads = source.threads(&id).await.unwrap_or_default();
        let mut files = source.changes(&id).await.unwrap_or_default();
        files.sort_by(|a, b| a.path.cmp(&b.path)); // cluster by directory for grouping
        let checks = source.checks(&id).await.unwrap_or_default();
        let commits = source.commits(&id).await.unwrap_or_default();
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
        self.screen = Screen::PrView(Box::new(PrView {
            label,
            url,
            connection_id: conn_id,
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
        }));
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
        let (pr_id, comments, conn_id) = match &self.screen {
            Screen::PrView(v) => (v.pr.id.clone(), v.pending.clone(), v.connection_id.clone()),
            _ => return,
        };
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
        match source.submit_review(&pr_id, event, &comments).await {
            Ok(()) => {
                let threads = source.threads(&pr_id).await.unwrap_or_default();
                if let Screen::PrView(v) = &mut self.screen {
                    v.pending.clear();
                    v.review_draft = None;
                    v.diff.threads = threads;
                }
                // You've reviewed it — it no longer needs you on the Launchpad.
                self.dismiss_from_launchpad(&conn_id, &pr_id);
                self.toast = Some(format!("Review submitted ({} comment(s))", comments.len()));
            }
            Err(e) => self.toast = Some(format!("Submit failed: {e}")),
        }
    }

    /// Loads the selected commit's diff into the diff view and jumps to the Diff tab.
    async fn open_commit_diff(&mut self, deps: &AppDeps) {
        let Screen::PrView(v) = &self.screen else { return };
        let Some(commit) = v.commits.get(v.commit_sel) else { return };
        let (sha, msg) = (commit.sha.clone(), commit.message.clone());
        let pr_id = v.pr.id.clone();
        let conn_id = v.connection_id.clone();

        let source = match self.pr_source_for(&conn_id, deps).await {
            Some(s) => s,
            None => {
                self.toast = Some("No pull-request provider is bound".into());
                return;
            }
        };
        let mut files = source.commit_changes(&pr_id, &sha).await.unwrap_or_default();
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

    async fn open_wi_view(&mut self, deps: &AppDeps) {
        let (id, conn_id, wi) = match self.selected_wi_row() {
            Some(row) => (row.wi.id.clone(), row.connection_id.clone(), row.wi.clone()),
            None => return,
        };
        self.open_wi_view_for(deps, id, conn_id, wi).await;
    }

    /// Opens the work-item view for an explicit item (used by the Launchpad).
    async fn open_wi_view_for(&mut self, deps: &AppDeps, id: String, conn_id: String, wi: WorkItem) {
        let threads = match self.wi_source_for(&conn_id, deps).await {
            Some(src) => src.threads(&id).await.unwrap_or_default(),
            None => Vec::new(),
        };
        self.screen = Screen::WiView(Box::new(WiView { connection_id: conn_id, wi, threads, scroll: 0 }));
    }

    /// Where Esc lands when closing an item view: back to the Launchpad if it was
    /// opened from there (row still selected), otherwise the section list.
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
            Key::Escape => {
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
            Key::Char('q') => {
                // Don't quit out from under unsubmitted line comments — same prompt as Esc.
                if matches!(&self.screen, Screen::PrView(v) if !v.pending.is_empty()) {
                    self.open_pending_exit_prompt();
                } else {
                    self.should_quit = true;
                }
                return;
            }
            Key::Char('o') => {
                self.open_selected();
                return;
            }
            Key::Char('a') => {
                self.open_pr_vote(ReviewVote::Approved);
                return;
            }
            Key::Char('x') => {
                self.open_pr_vote(ReviewVote::Rejected);
                return;
            }
            Key::Char('m') => {
                self.open_pr_merge();
                return;
            }
            Key::Char('c') => {
                // On a diff patch line this buffers a line comment; elsewhere it's a
                // plain PR comment.
                self.open_line_comment();
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
            Key::Escape => {
                self.screen = self.view_origin();
                return;
            }
            Key::Char('q') => {
                self.should_quit = true;
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
            'q' => self.should_quit = true,
            'j' => {
                self.move_down();
                self.ensure_visible();
            }
            'k' => {
                self.move_up();
                self.ensure_visible();
            }
            'h' => self.switch_tab(-1),
            'l' => self.switch_tab(1),
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
            'n' => self.start_add_connection(),
            'v' => self.open_sections_toggle(),
            'C' => self.open_config(deps).await,
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
            // Work-item state/comment (u / c) and PR write actions live inside the
            // opened item's view — press Enter first.
            _ => {}
        }
    }

    // ---- pipeline drill-in + trigger ----

    fn selected_pipe(&self) -> Option<&PipeRow> {
        if self.active != 2 {
            return None;
        }
        let idxs = self.filtered_pipe_indices();
        self.pipe_state.selected().and_then(|p| idxs.get(p)).and_then(|&i| self.pipes.get(i))
    }

    async fn open_pipeline(&mut self, deps: &AppDeps) {
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
        self.open_pipeline_for(deps, conn_id, provider, run_id, definition_id, branch, title, fallback).await;
    }

    /// Opens the pipeline drill-in for an explicit run (used by the Launchpad).
    #[allow(clippy::too_many_arguments)]
    async fn open_pipeline_for(
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
        // Enrich with full stages/jobs/steps via get_run (list_runs may be shallow),
        // plus any pending approval gates.
        let feeds = deps.sections.pipeline_feeds().await.unwrap_or_default();
        let (run, supports_approvals, can_respond, approvals) = match feeds.iter().find(|f| f.connection.connection_id() == conn_id) {
            Some(feed) => {
                let run = feed.source.get_run(&run_id).await.unwrap_or(fallback);
                let supports = feed.source.supports_approvals();
                let can_respond = feed.source.can_respond_to_approvals();
                let approvals = if supports { feed.source.pending_approvals(&run_id).await.unwrap_or_default() } else { Vec::new() };
                (run, supports, can_respond, approvals)
            }
            None => (fallback, false, false, Vec::new()),
        };

        let mut view = PipelineView::new(title, run, conn_id, provider, definition_id, branch);
        view.supports_approvals = supports_approvals;
        view.can_respond_approvals = can_respond;
        view.approvals = approvals;
        self.screen = Screen::Pipeline(Box::new(view));
    }

    /// Re-fetches the open pipeline drill-in's run + approvals (called on the 30s tick).
    async fn refresh_open_pipeline(&mut self, deps: &AppDeps) {
        let Screen::Pipeline(v) = &self.screen else { return };
        let (conn_id, run_id) = (v.connection_id.clone(), v.run.id.clone());
        let feeds = deps.sections.pipeline_feeds().await.unwrap_or_default();
        let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == conn_id) else { return };
        let Ok(run) = feed.source.get_run(&run_id).await else { return };
        let approvals = if feed.source.supports_approvals() {
            feed.source.pending_approvals(&run_id).await.unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Screen::Pipeline(v) = &mut self.screen {
            v.run = run;
            v.approvals = approvals;
            v.clamp_selection();
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
        let (conn_id, run_id) = (v.connection_id.clone(), v.run.id.clone());
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
                    run_id: run_id.clone(),
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
        let (conn_id, run_id, approval_id, decision, label) =
            (choice.connection_id.clone(), choice.run_id.clone(), choice.approval_id.clone(), choice.decision, choice.label.clone());
        let feeds = deps.sections.pipeline_feeds().await.unwrap_or_default();
        let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == conn_id) else {
            self.toast = Some("Pipeline connection not found".into());
            return;
        };
        match feed.source.respond_approval(&run_id, &approval_id, decision, None).await {
            Ok(()) => {
                let verb = match decision {
                    ApprovalDecision::Approve => "Approved",
                    ApprovalDecision::Reject => "Rejected",
                };
                self.toast = Some(format!("{verb} {label}"));
                self.refresh_open_pipeline(deps).await;
            }
            Err(e) => self.toast = Some(format!("Approval failed: {e}")),
        }
    }

    fn on_pipeline_key(&mut self, key: Key) {
        match key {
            Key::Escape => self.screen = self.view_origin(),
            Key::Char('q') => self.should_quit = true,
            Key::Char('T') => self.open_pipeline_trigger(),
            Key::Char('A') => self.open_approval_picker(),
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

    /// Fetches the selected job's logs and opens the scrollable log pane.
    async fn open_pipeline_logs(&mut self, deps: &AppDeps) {
        let (conn_id, run_id, job_id, title) = {
            let Screen::Pipeline(v) = &self.screen else { return };
            let nodes = v.flatten();
            let node = nodes.get(v.selected);
            let Some(job_id) = node.and_then(|n| n.job_id.clone()) else {
                self.toast = Some("Select a job or step to view its logs".into());
                return;
            };
            let label = node.map(|n| n.label.clone()).unwrap_or_default();
            (v.connection_id.clone(), v.run.id.clone(), job_id, format!("Logs · {label}"))
        };

        self.toast = Some("Fetching logs…".into());
        let feeds = deps.sections.pipeline_feeds().await.unwrap_or_default();
        let text = match feeds.iter().find(|f| f.connection.connection_id() == conn_id) {
            Some(feed) => match feed.source.logs(&run_id, Some(&job_id)).await {
                Ok(t) => t,
                Err(e) => format!("Couldn't fetch logs: {e}"),
            },
            None => "Pipeline connection not found".into(),
        };
        let lines: Vec<String> =
            if text.trim().is_empty() { vec!["(no logs returned)".into()] } else { text.lines().map(|l| l.to_string()).collect() };
        if let Screen::Pipeline(v) = &mut self.screen {
            v.logs = Some(LogView { title, lines, scroll: 0 });
        }
        self.toast = None;
    }

    /// Scroll / close keys while the log pane is open.
    fn on_pipeline_logs_key(&mut self, key: Key) {
        if let Screen::Pipeline(v) = &mut self.screen {
            if let Some(log) = &mut v.logs {
                match key {
                    Key::Up | Key::Char('k') => log.scroll = log.scroll.saturating_sub(1),
                    Key::Down | Key::Char('j') => log.scroll = log.scroll.saturating_add(1),
                    Key::PageUp | Key::Char('b') => log.scroll = log.scroll.saturating_sub(15),
                    Key::PageDown | Key::Char(' ') => log.scroll = log.scroll.saturating_add(15),
                    Key::Escape | Key::Char('L') => v.logs = None,
                    _ => {}
                }
            }
        }
    }

    /// The pipeline to trigger — from the drill-in view if open, else the selected list row.
    fn pipeline_target(&self) -> Option<(String, String, Option<String>, String)> {
        if let Screen::Pipeline(v) = &self.screen {
            return Some((v.connection_id.clone(), v.definition_id.clone(), v.branch.clone(), v.title.clone()));
        }
        let pipe = self.selected_pipe()?;
        Some((pipe.connection_id.clone(), pipe.run.definition_id.clone(), pipe.run.branch.clone(), pipe_label(pipe)))
    }

    fn open_pipeline_trigger(&mut self) {
        let Some((connection_id, definition_id, branch, label)) = self.pipeline_target() else { return };
        let message = match &branch {
            Some(b) => format!("Trigger {label} on {b}?"),
            None => format!("Trigger {label}?"),
        };
        self.overlay = Some(Overlay::Confirm {
            title: "Trigger".into(),
            message,
            action: Action::PipelineTrigger { connection_id, definition_id, branch, label },
        });
    }

    async fn execute_pipeline_action(&mut self, action: Action, deps: &AppDeps) {
        let Action::PipelineTrigger { connection_id, definition_id, branch, label } = action else { return };
        let feeds = match deps.sections.pipeline_feeds().await {
            Ok(f) => f,
            Err(e) => {
                self.toast = Some(format!("Trigger failed: {e}"));
                return;
            }
        };
        let Some(feed) = feeds.iter().find(|f| f.connection.connection_id() == connection_id) else {
            self.toast = Some("Pipeline connection not found".into());
            return;
        };
        match feed.source.trigger(&definition_id, branch.as_deref()).await {
            Ok(()) => {
                self.toast = Some(format!("Triggered {label}"));
                let mut errors = Vec::new();
                self.reload_pipelines(deps, &mut errors).await;
                self.fix_selection();
                if let Some(e) = errors.first() {
                    self.toast = Some(e.clone());
                }
            }
            Err(e) => self.toast = Some(format!("Trigger failed: {e}")),
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
            2 => self.selected_pipe().and_then(|p| p.run.url.clone()),
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
        };

        if let Err(e) = deps.config.add_or_update_connection(connection, draft.pat).await {
            self.toast = Some(format!("Add failed: {e}"));
            return;
        }

        if let Some(section) = draft.bind_section {
            let result = match section {
                Section::PullRequests => deps.config.bind_pull_requests(&id).await,
                Section::WorkItems => deps.config.bind_work_items(&id).await,
                Section::Pipelines => deps.config.set_pipeline_auto_discover(&id, true).await,
            };
            if let Err(e) = result {
                self.toast = Some(format!("Added, but binding failed: {e}"));
                self.reload_all(deps).await;
                return;
            }
        }

        self.toast = Some(format!("Added {} connection", provider.as_str()));
        self.reload_all(deps).await;
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
            Some(Overlay::Toggle { title: "Visible tabs".into(), kind: ToggleKind::Sections, min_one: true, items, selected: 0 });
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
            Some(Overlay::Toggle { title: "Show statuses".into(), kind: ToggleKind::PrStatuses, min_one: true, items, selected: 0 });
    }

    /// `ids` are the statuses left ticked. Rebuilds the shown set, then refetches so a
    /// newly-ticked Merged/Closed pulls completed PRs from the provider.
    async fn apply_pr_statuses(&mut self, shown_ids: Vec<String>, deps: &AppDeps) {
        self.pr_shown_statuses = shown_ids.iter().filter_map(|id| parse_pr_status(id)).collect();
        let mut errors = Vec::new();
        self.reload_pull_requests(deps, &mut errors).await;
        self.fix_selection();
        self.list_scroll = 0;
        self.toast = Some(if let Some(e) = errors.first() { e.clone() } else { format!("Showing {}", pr_status_summary(&self.pr_shown_statuses)) });
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
            Some(Overlay::Toggle { title: "Show states".into(), kind: ToggleKind::WorkItemStates, min_one: false, items, selected: 0 });
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
                self.apply_pr_statuses(ids, deps).await;
            }
            ToggleKind::Notifications => {
                self.apply_notifications(ids, deps).await;
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
                self.toast = Some(format!("Discover failed: {e}"));
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
        });
    }

    async fn apply_pipeline_subs(&mut self, connection_id: &str, ids: Vec<String>, deps: &AppDeps) {
        match deps.config.set_pipeline_definitions(connection_id, ids.clone()).await {
            Ok(()) => {
                self.toast = Some(format!("Subscribed to {} pipeline(s)", ids.len()));
                self.reload_all(deps).await;
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
            Key::Escape => self.screen = Screen::List,
            Key::Char('q') => self.should_quit = true,
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
        });
    }

    async fn apply_section_bind(&mut self, section: usize, ids: Vec<String>, deps: &AppDeps) {
        let result = if section == 0 {
            deps.config.set_pull_request_connections(ids).await
        } else {
            deps.config.set_work_item_connections(ids).await
        };
        match result {
            Ok(()) => {
                self.toast = Some("Bindings updated".into());
                self.reload_all(deps).await;
                self.rebuild_config_view(deps).await;
            }
            Err(e) => self.toast = Some(format!("{e}")),
        }
    }

    fn config_remove_selected(&mut self) {
        let Some((id, label)) = self.config_selected_id() else { return };
        self.overlay = Some(Overlay::Confirm {
            title: "Remove connection".into(),
            message: format!("Remove '{label}' and its bindings?"),
            action: Action::RemoveConnection { id, label },
        });
    }

    async fn execute_config_action(&mut self, action: Action, deps: &AppDeps) {
        let Action::RemoveConnection { id, label } = action else { return };
        match deps.config.remove_connection(&id).await {
            Ok(()) => {
                self.toast = Some(format!("Removed {label}"));
                self.reload_all(deps).await;
                self.rebuild_config_view(deps).await;
            }
            Err(e) => self.toast = Some(format!("Remove failed: {e}")),
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
        deps.sections
            .pull_request_feeds()
            .await
            .ok()?
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

    fn open_pr_comment(&mut self) {
        let Some(pr) = self.active_pr() else { return };
        let title = format!("Comment on {}", pr_label(pr));
        self.overlay = Some(Overlay::Input { title, buffer: String::new(), kind: InputKind::PrComment });
    }

    async fn execute_action(&mut self, action: Action, deps: &AppDeps) {
        match action {
            Action::PrVote(_) | Action::PrMerge(_) | Action::PrComment(_) => self.execute_pr_action(action, deps).await,
            Action::WiSetState(_) | Action::WiComment(_) => self.execute_wi_action(action, deps).await,
            Action::PipelineTrigger { .. } => self.execute_pipeline_action(action, deps).await,
            Action::RemoveConnection { .. } => self.execute_config_action(action, deps).await,
            Action::ApplyToggle { kind, ids } => self.apply_toggle(kind, ids, deps).await,
            Action::AddLineComment(body) => self.add_line_comment(body),
            Action::SubmitReview(event) => self.submit_review(event, deps).await,
            Action::SetSort { section, index } => self.apply_sort(section, index, deps).await,
            Action::SaveView(name) => self.save_view(name, deps).await,
            Action::DeleteView => self.delete_view(deps).await,
            Action::PickApproval { index } => self.confirm_approval(index),
            Action::RespondApproval { index } => self.respond_approval(index, deps).await,
            Action::OpenItem { kind, id, connection_id } => self.open_palette_item(kind, id, connection_id, deps).await,
            Action::OpenReviewMenu => self.open_review_submit(),
            Action::LeavePrView => self.screen = self.view_origin(),
        }
    }

    async fn execute_pr_action(&mut self, action: Action, deps: &AppDeps) {
        // Resolve the PR + its connection from the open view, else the selected row.
        let target = match &self.screen {
            Screen::PrView(v) => Some((v.pr.id.clone(), v.connection_id.clone())),
            _ => self.selected_pr_row().map(|r| (r.pr.id.clone(), r.connection_id.clone())),
        };
        let Some((id, conn_id)) = target else {
            self.toast = Some("Nothing selected".into());
            return;
        };
        let source = match self.pr_source_for(&conn_id, deps).await {
            Some(s) => s,
            None => {
                self.toast = Some("No pull-request provider is bound".into());
                return;
            }
        };

        let result = match &action {
            Action::PrVote(vote) => source.vote(&id, *vote).await.map(|_| vote_message(*vote).to_string()),
            Action::PrMerge(strategy) => source
                .merge(&id, &MergeOptions { strategy: *strategy, delete_source_ref: false })
                .await
                .map(|_| format!("Merged ({strategy:?})")),
            Action::PrComment(text) => {
                if text.trim().is_empty() {
                    self.toast = Some("Empty comment — nothing sent".into());
                    return;
                }
                source.add_comment(&id, text).await.map(|_| "Comment added".to_string())
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
                    let fresh = source.get(&id).await.ok();
                    let threads = source.threads(&id).await.unwrap_or_default();
                    if let Screen::PrView(v) = &mut self.screen {
                        if let Some(pr) = fresh {
                            v.pr = pr;
                        }
                        v.diff.threads = threads;
                    }
                }
                let mut errors = Vec::new();
                self.reload_pull_requests(deps, &mut errors).await;
                self.fix_selection();
                if let Some(e) = errors.first() {
                    self.toast = Some(e.clone());
                }
            }
            Err(e) => self.toast = Some(format!("Failed: {e}")),
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
        deps.sections
            .work_item_feeds()
            .await
            .ok()?
            .into_iter()
            .find(|f| f.connection.connection_id() == connection_id)
            .map(|f| f.source)
    }

    /// State picker for the open work item, pulling the provider's real available
    /// states (falling back to states seen across the loaded items).
    async fn open_wi_state(&mut self, deps: &AppDeps) {
        let (id, current, title, conn_id) = match &self.screen {
            Screen::WiView(v) => (v.wi.id.clone(), v.wi.state.clone(), format!("Set state — {}", wi_label(&v.wi)), v.connection_id.clone()),
            _ => return,
        };

        let mut states = match self.wi_source_for(&conn_id, deps).await {
            Some(src) => src.available_states(&id).await.unwrap_or_default(),
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

    async fn execute_wi_action(&mut self, action: Action, deps: &AppDeps) {
        let target = match &self.screen {
            Screen::WiView(v) => Some((v.wi.id.clone(), v.connection_id.clone())),
            _ => self.selected_wi_row().map(|r| (r.wi.id.clone(), r.connection_id.clone())),
        };
        let Some((id, conn_id)) = target else {
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
            Action::WiSetState(state) => source.set_state(&id, state).await.map(|_| format!("State → {state}")),
            Action::WiComment(text) => {
                if text.trim().is_empty() {
                    self.toast = Some("Empty comment — nothing sent".into());
                    return;
                }
                source.add_comment(&id, text).await.map(|_| "Comment added".to_string())
            }
            _ => return,
        };

        match result {
            Ok(msg) => {
                self.toast = Some(msg);
                // Reflect the change in the open view.
                if let Action::WiSetState(state) = &action {
                    if let Screen::WiView(v) = &mut self.screen {
                        v.wi.state = state.clone();
                    }
                }
                if matches!(action, Action::WiComment(_)) {
                    let threads = source.threads(&id).await.unwrap_or_default();
                    if let Screen::WiView(v) = &mut self.screen {
                        v.threads = threads;
                    }
                }
                let mut errors = Vec::new();
                self.reload_work_items(deps, &mut errors).await;
                self.fix_selection();
                if let Some(e) = errors.first() {
                    self.toast = Some(e.clone());
                }
            }
            Err(e) => self.toast = Some(format!("Failed: {e}")),
        }
    }
}

fn wi_label(wi: &WorkItem) -> String {
    let id = wi.identifier.clone().map(|i| format!("{i} ")).unwrap_or_default();
    let title: String = wi.title.chars().take(40).collect();
    format!("{id}— {title}")
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

/// Runs that are Failed now but weren't Failed at the previous refresh.
fn new_pipeline_failures<'a>(prev: &HashMap<String, PipelineRunStatus>, pipes: &'a [PipeRow]) -> Vec<&'a PipeRow> {
    pipes
        .iter()
        .filter(|r| {
            matches!(r.run.status, PipelineRunStatus::Failed) && !matches!(prev.get(&r.run.id), Some(PipelineRunStatus::Failed))
        })
        .collect()
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
pub(crate) fn pr_vote_flags(pr: &PullRequest) -> (bool, bool) {
    let approved = pr.reviewers.iter().any(|r| matches!(r.vote, ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions));
    let changes = pr.reviewers.iter().any(|r| matches!(r.vote, ReviewVote::Rejected));
    (approved, changes)
}

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
    let hay = format!(
        "{} {} {} {} {:?}",
        p.run.name.clone().unwrap_or_else(|| p.run.definition_id.clone()),
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
        "pipeline" => {
            // Sort by the pipeline (definition) name shown in the column, falling back to
            // the run name / id when it's unknown.
            let name = |r: &PipeRow| r.definition_name.clone().or_else(|| r.run.name.clone()).unwrap_or_else(|| r.run.definition_id.clone());
            ci(&name(a)).cmp(&ci(&name(b)))
        }
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
    Enter,
    Escape,
    Backspace,
    PageUp,
    PageDown,
    Char(char),
    /// A Ctrl-modified letter (lowercased), e.g. Ctrl-P. Ctrl-C is mapped to [`Key::Quit`].
    Ctrl(char),
    /// Hard quit (Ctrl-C), honoured in every mode.
    Quit,
    /// Terminal was resized — no-op, but wakes the loop so it redraws at the new size.
    Redraw,
    None,
}

fn pr_query(filter: PullRequestFilter) -> PullRequestQuery {
    PullRequestQuery { filter, include_completed: false, limit: Some(50) }
}

// ---- background fetch helpers (no `&mut self`, safe to run in a spawned task) ----

async fn fetch_pull_requests(deps: &AppDeps, filter: PullRequestFilter, completed: bool, errors: &mut Vec<String>) -> Vec<PrRow> {
    let mut out = Vec::new();
    match deps.sections.pull_request_feeds().await {
        Ok(feeds) => {
            let query = PullRequestQuery { filter, include_completed: completed, limit: Some(50) };
            for feed in feeds {
                let (provider, name, conn_id) = feed_tag(&feed.connection);
                match feed.source.list(&query).await {
                    Ok(list) => out.extend(list.into_iter().map(|pr| PrRow { connection_id: conn_id.clone(), connection: name.clone(), provider, pr })),
                    Err(e) => errors.push(format!("PRs ({name}): {e}")),
                }
            }
        }
        Err(e) => errors.push(format!("PRs: {e}")),
    }
    out
}

async fn fetch_work_items(deps: &AppDeps, errors: &mut Vec<String>) -> Vec<WiRow> {
    let mut out = Vec::new();
    match deps.sections.work_item_feeds().await {
        Ok(feeds) => {
            for feed in feeds {
                let (provider, name, conn_id) = feed_tag(&feed.connection);
                match feed.source.list(&wi_query()).await {
                    Ok(list) => out.extend(list.into_iter().map(|wi| WiRow { connection_id: conn_id.clone(), connection: name.clone(), provider, wi })),
                    Err(e) => errors.push(format!("Work items ({name}): {e}")),
                }
            }
        }
        Err(e) => errors.push(format!("Work items: {e}")),
    }
    out
}

async fn fetch_notifications(deps: &AppDeps, errors: &mut Vec<String>) -> Vec<NotifRow> {
    let mut out = Vec::new();
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
                    Err(e) => errors.push(format!("Notifications ({name}): {e}")),
                }
            }
        }
        Err(e) => errors.push(format!("Notifications: {e}")),
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.notification.updated_at)); // newest first
    out
}

async fn fetch_pipelines(deps: &AppDeps, errors: &mut Vec<String>) -> Vec<PipeRow> {
    let mut out = Vec::new();
    match deps.sections.pipeline_feeds().await {
        Ok(feeds) => {
            for feed in feeds {
                let provider = feed.connection.provider_type();
                let name = feed.connection.display_name().to_string();
                let conn_id = feed.connection.connection_id().to_string();
                let def_names: HashMap<String, String> =
                    feed.source.discover().await.unwrap_or_default().into_iter().map(|d| (d.id, d.name)).collect();
                for q in feed_queries(&feed.subscription) {
                    match feed.source.list_runs(&q).await {
                        Ok(runs) => {
                            let supports = feed.source.supports_approvals();
                            for run in runs {
                                let awaiting_approval = supports
                                    && is_active(run.status)
                                    && feed.source.pending_approvals(&run.id).await.map(|a| a.iter().any(|x| x.can_respond)).unwrap_or(false);
                                let definition_name = def_names.get(&run.definition_id).cloned();
                                out.push(PipeRow { connection_id: conn_id.clone(), connection: name.clone(), provider, run, definition_name, awaiting_approval });
                            }
                        }
                        Err(e) => errors.push(format!("Pipelines ({name}): {e}")),
                    }
                }
            }
        }
        Err(e) => errors.push(format!("Pipelines: {e}")),
    }
    out
}

/// Re-fetches the open pipeline run + its pending approvals (mirrors
/// [`App::refresh_open_pipeline`]), so the background refresh keeps the view live.
async fn fetch_open_pipeline(deps: &AppDeps, conn_id: &str, run_id: &str) -> Option<(String, PipelineRun, Vec<PipelineApproval>)> {
    let feeds = deps.sections.pipeline_feeds().await.unwrap_or_default();
    let feed = feeds.iter().find(|f| f.connection.connection_id() == conn_id)?;
    let run = feed.source.get_run(run_id).await.ok()?;
    let approvals = if feed.source.supports_approvals() {
        feed.source.pending_approvals(run_id).await.unwrap_or_default()
    } else {
        Vec::new()
    };
    Some((run_id.to_string(), run, approvals))
}

async fn fetch_launchpad_prs(deps: &AppDeps) -> (Vec<PrRow>, Vec<PrRow>) {
    let (mut mine_out, mut review_out) = (Vec::new(), Vec::new());
    if let Ok(feeds) = deps.sections.pull_request_feeds().await {
        for feed in feeds {
            let (provider, name, conn_id) = feed_tag(&feed.connection);
            let mine_q = PullRequestQuery { filter: PullRequestFilter::Mine, include_completed: true, limit: Some(50) };
            if let Ok(list) = feed.source.list(&mine_q).await {
                mine_out.extend(list.into_iter().map(|pr| PrRow { connection_id: conn_id.clone(), connection: name.clone(), provider, pr }));
            }
            if let Ok(list) = feed.source.list(&pr_query(PullRequestFilter::ReviewRequested)).await {
                review_out.extend(list.into_iter().map(|pr| PrRow { connection_id: conn_id.clone(), connection: name.clone(), provider, pr }));
            }
        }
    }
    (mine_out, review_out)
}

/// The PR-notification scan, firing pings for newly-seen review requests / vote changes
/// and returning the fresh seen-sets. Mirrors the inline logic but takes a snapshot so
/// it can run off the render loop.
async fn scan_pr_notifications(deps: &AppDeps, p: &ReloadParams) -> Option<PrScan> {
    let want_review = p.notifications.review_requested;
    let want_votes = p.notifications.pr_approved || p.notifications.pr_changes_requested;
    if !want_review && !want_votes {
        return None;
    }
    let feeds = deps.sections.pull_request_feeds().await.ok()?;
    if feeds.is_empty() {
        return None;
    }
    let seeded = p.scan_seeded;
    let mut review_now: HashSet<(String, String)> = HashSet::new();
    let mut votes_now: HashMap<(String, String), (bool, bool)> = HashMap::new();
    for feed in &feeds {
        let conn = feed.connection.connection_id().to_string();
        if want_review {
            if let Ok(review) = feed.source.list(&pr_query(PullRequestFilter::ReviewRequested)).await {
                for pr in &review {
                    let key = (conn.clone(), pr.id.clone());
                    if seeded && !p.review_seen.contains(&key) {
                        p.notifier.notify("Review requested", &pr_label(pr));
                    }
                    review_now.insert(key);
                }
            }
        }
        if want_votes {
            if let Ok(mine) = feed.source.list(&pr_query(PullRequestFilter::Mine)).await {
                for pr in &mine {
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
fn feed_queries(sub: &forgetop_core::config::PipelineSubscription) -> Vec<PipelineRunQuery> {
    if sub.auto_discover_all || sub.definition_ids.is_empty() {
        vec![PipelineRunQuery { definition_id: None, branch: None, limit: Some(20) }]
    } else {
        sub.definition_ids
            .iter()
            .map(|id| PipelineRunQuery { definition_id: Some(id.clone()), branch: None, limit: Some(10) })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(url: Option<&str>) -> PullRequest {
        PullRequest {
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

    #[test]
    fn inbox_unread_count_and_move_wraps() {
        let mut app = App::new("slate");
        let n = |id: &str, unread: bool| NotifRow {
            connection_id: "github".into(),
            connection: "GitHub".into(),
            provider: ProviderType::GitHub,
            notification: Notification {
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
        assert_eq!(cands[0].kind, PaletteKind::Pr);
        assert_eq!(cands[0].id, "pr1");
        assert_eq!(cands[0].title, "Migrate billing");
        assert!(cands[0].subtitle.contains("priya"), "subtitle has author: {}", cands[0].subtitle);
        assert!(cands[0].subtitle.contains("feat/pay-412"), "subtitle has branch: {}", cands[0].subtitle);
        // WI: identifier is searchable via the subtitle.
        assert_eq!(cands[1].kind, PaletteKind::Wi);
        assert_eq!(cands[1].id, "wi1");
        assert!(cands[1].subtitle.contains("PAY-412"), "subtitle has identifier: {}", cands[1].subtitle);
    }

    #[test]
    fn apply_reloaded_swaps_data_and_clears_refresh_flags() {
        let mut app = App::new("slate");
        app.reloading = true;
        app.loading = true;
        app.apply_reloaded(Reloaded {
            prs: vec![pr_row(pr(None)), pr_row(pr(None))],
            wis: vec![wi_row(wi(None))],
            pipes: vec![],
            inbox: vec![],
            lp_prs_mine: vec![],
            lp_prs_review: vec![],
            health: vec![],
            scan: None,
            open_pipeline: None,
            errors: vec![],
        });
        assert_eq!(app.prs.len(), 2);
        assert_eq!(app.wis.len(), 1);
        assert!(!app.reloading && !app.loading, "refresh flags cleared");
        assert!(app.status.contains("2 PRs") && app.status.contains("1 work items"), "status summarises");
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

    #[test]
    fn escape_returns_to_the_launchpad_when_opened_from_it() {
        let mut app = App::new("slate");
        let view = || Screen::WiView(Box::new(WiView { connection_id: "c".into(), wi: wi(None), threads: vec![], scroll: 0 }));

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
            id: "r1".into(),
            definition_id: "ci".into(),
            number: Some(1),
            name: Some("CI".into()),
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
            v.logs = Some(LogView { title: "Logs".into(), lines: vec!["x".into(); 50], scroll: 0 });
        }
        app.on_pipeline_logs_key(Key::Down);
        app.on_pipeline_logs_key(Key::Down);
        let Screen::Pipeline(v) = &app.screen else { panic!() };
        assert_eq!(v.logs.as_ref().unwrap().scroll, 2);
        app.on_pipeline_logs_key(Key::Escape);
        let Screen::Pipeline(v) = &app.screen else { panic!() };
        assert!(v.logs.is_none(), "Esc closes the log pane");
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
                id: id.into(),
                definition_id: "ci".into(),
                number: Some(1),
                name: Some("CI".into()),
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
                id: id.into(),
                definition_id: "ci".into(),
                number: Some(1),
                name: Some("CI".into()),
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
                id: id.into(),
                definition_id: "ci".into(),
                number: None,
                name: None,
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
    fn commit_diff_scope_restores_whole_pr() {
        let mut d = diff(vec![changed("b.rs", Some("@@ -1 +1 @@\n-p\n+q"))]);
        d.selected = 0;
        d.scroll = 5;
        d.focus = DiffFocus::Patch;
        d.cursor = 2;
        d.commit_label = Some("abc1234 msg".into());
        let mut v = PrView {
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
        };

        v.reset_diff_scope();
        assert_eq!(v.diff.commit_label, None, "scope cleared");
        assert_eq!(v.diff.files.len(), 1);
        assert_eq!(v.diff.files[0].path, "a.rs", "whole-PR files restored");
        assert_eq!(v.diff.selected, 0);
        assert_eq!(v.diff.focus, DiffFocus::FileList);
    }

    #[test]
    fn add_line_comment_buffers_against_draft() {
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(PrView {
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
    fn quit_with_pending_comments_prompts_instead_of_quitting() {
        let mut app = App::new("slate");
        app.screen = pr_view_with_pending(vec![LineComment { path: "a.rs".into(), line: 1, side: DiffSide::New, body: "nit".into() }]);

        app.on_pr_view_key(Key::Char('q'));

        assert!(!app.should_quit, "q doesn't quit out from under unsubmitted comments");
        assert!(matches!(app.screen, Screen::PrView(_)));
        assert!(matches!(&app.overlay, Some(Overlay::Picker { kind, .. }) if matches!(kind, PickerKind::PendingExit)));
    }

    #[test]
    fn quit_without_pending_comments_quits() {
        let mut app = App::new("slate");
        app.screen = pr_view_with_pending(vec![]);
        app.on_pr_view_key(Key::Char('q'));
        assert!(app.should_quit);
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

    #[test]
    fn hiding_the_active_section_falls_back_to_first_visible() {
        let mut app = App::new("slate");
        app.active = 1; // Work Items
        app.apply_hidden_sections(&[Section::WorkItems]);
        assert!(!app.visible[1]);
        assert_eq!(app.active, 0);
    }
}
