//! Application state and the (async) update logic driven by the event loop.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Local, Utc};
use forgetop_core::config::SortPref;
use forgetop_core::domain::*;
use forgetop_core::provider::*;
use forgetop_core::service::{ConfigService, ConnectionHealth, ConnectionHealthService, SectionService};
use ratatui::widgets::TableState;

use crate::overlay::{Action, InputKind, Outcome, Overlay, PickerKind, ToggleItem, ToggleKind};
use crate::theme::Theme;
use crate::wizard::{section_label, Wizard, WizardOutcome};

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

/// One pipeline run, tagged with the connection it came from (for the provider column).
pub struct PipeRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub run: PipelineRun,
}

pub struct App {
    pub theme: Theme,
    pub active: usize,
    pub prs: Vec<PullRequest>,
    pub wis: Vec<WorkItem>,
    pub pipes: Vec<PipeRow>,
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
    /// Transient one-shot message shown in the footer until the next keypress.
    pub toast: Option<String>,
    /// Open modal overlay, if any. When set, keys route here instead of the table.
    pub overlay: Option<Overlay>,
    /// Add-connection wizard, if running. Takes priority over the overlay/screens.
    pub wizard: Option<Wizard>,
    /// Current screen — the list, or a full-screen sub-view like the PR diff.
    pub screen: Screen,
    pub last_refresh: DateTime<Local>,
    pub should_quit: bool,
}

/// Full-screen views layered above the list. The large views are boxed so the
/// common `List` state doesn't bloat every `Screen` value.
pub enum Screen {
    List,
    Pipeline(Box<PipelineView>),
    Config(Box<ConfigView>),
    /// Full-screen pull-request view with sub-tabs (Conversation/Commits/Checks/Diff).
    PrView(Box<PrView>),
    /// Full-screen work-item view.
    WiView(Box<WiView>),
}

/// State for the full-screen PR view.
pub struct PrView {
    pub label: String,
    pub url: Option<String>,
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
    pub definition_id: String,
    pub branch: Option<String>,
    collapsed: HashSet<String>,
    pub selected: usize,
    /// Open log pane over a selected job, if any.
    pub logs: Option<LogView>,
}

impl PipelineView {
    pub fn new(title: String, run: PipelineRun, connection_id: String, definition_id: String, branch: Option<String>) -> Self {
        Self { title, run, connection_id, definition_id, branch, collapsed: HashSet::new(), selected: 0, logs: None }
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
}

impl App {
    pub fn new(theme_name: &str) -> Self {
        Self {
            theme: Theme::by_name(theme_name),
            active: 0,
            prs: Vec::new(),
            wis: Vec::new(),
            pipes: Vec::new(),
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
            filters: [String::new(), String::new(), String::new()],
            filtering: false,
            wi_hidden_states: HashSet::new(),
            pr_sort: None,
            wi_sort: None,
            pipe_sort: None,
            toast: None,
            overlay: None,
            wizard: None,
            screen: Screen::List,
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
        let mut idx: Vec<usize> = (0..self.prs.len()).filter(|&i| pr_matches(&self.prs[i], &q)).collect();
        if let Some(s) = &self.pr_sort {
            idx.sort_by(|&a, &b| ordered(pr_cmp(&self.prs[a], &self.prs[b], &s.key), s.desc));
        }
        idx
    }

    pub fn filtered_wi_indices(&self) -> Vec<usize> {
        let q = self.filters[1].to_lowercase();
        let mut idx: Vec<usize> = (0..self.wis.len())
            .filter(|&i| !self.wi_hidden_states.contains(&self.wis[i].state) && wi_matches(&self.wis[i], &q))
            .collect();
        if let Some(s) = &self.wi_sort {
            idx.sort_by(|&a, &b| ordered(wi_cmp(&self.wis[a], &self.wis[b], &s.key), s.desc));
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
            if seen.insert(w.state.clone()) {
                out.push(w.state.clone());
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

    pub fn switch_tab(&mut self, delta: isize) {
        let vis = self.visible_indices();
        if vis.is_empty() {
            return;
        }
        let pos = vis.iter().position(|&i| i == self.active).unwrap_or(0) as isize;
        let n = vis.len() as isize;
        self.active = vis[(((pos + delta) % n + n) % n) as usize];
        self.list_scroll = 0;
        self.clamp_selection();
    }

    /// Jumps to the Nth *visible* tab (0-based), for the number keys.
    pub fn set_tab(&mut self, idx: usize) {
        if let Some(&section) = self.visible_indices().get(idx) {
            self.active = section;
            self.list_scroll = 0;
            self.clamp_selection();
        }
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

    // ---- data loading ----

    pub async fn reload_all(&mut self, deps: &AppDeps) {
        self.loading = true;
        self.status = "Refreshing…".into();

        let mut errors: Vec<String> = Vec::new();
        self.reload_pull_requests(deps, &mut errors).await;
        self.reload_work_items(deps, &mut errors).await;
        self.reload_pipelines(deps, &mut errors).await;
        self.health = deps.health.check_all().await;

        self.fix_selection();
        self.last_refresh = Local::now();
        self.loading = false;
        self.status = if errors.is_empty() {
            format!("{} PRs · {} work items · {} runs", self.prs.len(), self.wis.len(), self.pipes.len())
        } else {
            errors.join("  |  ")
        };
    }

    async fn reload_pull_requests(&mut self, deps: &AppDeps, errors: &mut Vec<String>) {
        match deps.sections.pull_request_source().await {
            Ok(Some(src)) => match src.list(&pr_query(self.pr_filter)).await {
                Ok(list) => self.prs = list,
                Err(e) => errors.push(format!("PRs: {e}")),
            },
            Ok(None) => self.prs.clear(),
            Err(e) => errors.push(format!("PRs: {e}")),
        }
    }

    async fn reload_work_items(&mut self, deps: &AppDeps, errors: &mut Vec<String>) {
        match deps.sections.work_item_source().await {
            Ok(Some(src)) => match src.list(&wi_query()).await {
                Ok(list) => self.wis = list,
                Err(e) => errors.push(format!("Work items: {e}")),
            },
            Ok(None) => self.wis.clear(),
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
                    for q in feed_queries(&feed.subscription) {
                        match feed.source.list_runs(&q).await {
                            Ok(runs) => {
                                for run in runs {
                                    self.pipes.push(PipeRow { connection_id: conn_id.clone(), connection: name.clone(), provider, run });
                                }
                            }
                            Err(e) => errors.push(format!("Pipelines ({name}): {e}")),
                        }
                    }
                }
            }
            Err(e) => errors.push(format!("Pipelines: {e}")),
        }
    }

    /// Re-selects a valid row per tab after the underlying data (or filter) changed.
    /// Selection is a position within each tab's *filtered* view, so clamp to that.
    fn fix_selection(&mut self) {
        let (pl, wl, ll) = (self.filtered_len(0), self.filtered_len(1), self.filtered_len(2));
        self.pr_state.select((pl > 0).then(|| self.pr_state.selected().unwrap_or(0).min(pl - 1)));
        self.wi_state.select((wl > 0).then(|| self.wi_state.selected().unwrap_or(0).min(wl - 1)));
        self.pipe_state.select((ll > 0).then(|| self.pipe_state.selected().unwrap_or(0).min(ll - 1)));
    }

    /// Cycles the PR filter, reloads just the PR list, and toasts the new filter.
    async fn cycle_pr_filter(&mut self, deps: &AppDeps) {
        self.pr_filter = match self.pr_filter {
            PullRequestFilter::All => PullRequestFilter::Mine,
            PullRequestFilter::Mine => PullRequestFilter::ReviewRequested,
            PullRequestFilter::ReviewRequested => PullRequestFilter::All,
        };
        let mut errors = Vec::new();
        self.reload_pull_requests(deps, &mut errors).await;
        self.pr_state.select((!self.filtered_pr_indices().is_empty()).then_some(0));
        self.list_scroll = 0;
        self.toast = Some(match errors.first() {
            Some(e) => e.clone(),
            None => format!("Filter: {} ({} PRs)", self.pr_filter_label(), self.prs.len()),
        });
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
        // Help is available from the list and every sub-view.
        if key == Key::Char('?') {
            self.overlay = Some(Overlay::Help { scroll: 0 });
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
                self.on_wi_view_key(key);
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
            Key::Enter => match self.active {
                0 => self.open_pr_view(deps, 0).await,
                1 => self.open_wi_view(deps).await,
                2 => self.open_pipeline(deps).await,
                _ => {}
            },
            Key::Char(c) => self.on_char(c, deps).await,
            Key::Backspace | Key::Quit | Key::Redraw | Key::None => {}
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
        let Some(pr) = self.selected_pr() else { return };
        let id = pr.id.clone();
        let label = pr_label(pr);
        let url = pr.url.clone();
        let pr = pr.clone();
        let source = match deps.sections.pull_request_source().await {
            Ok(Some(s)) => s,
            _ => {
                self.toast = Some("No pull-request provider is bound".into());
                return;
            }
        };
        let threads = source.threads(&id).await.unwrap_or_default();
        let files = source.changes(&id).await.unwrap_or_default();
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
        };
        self.screen = Screen::PrView(Box::new(PrView {
            label,
            url,
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
        let (pr_id, comments) = match &self.screen {
            Screen::PrView(v) => (v.pr.id.clone(), v.pending.clone()),
            _ => return,
        };
        if comments.is_empty() {
            return;
        }
        let source = match deps.sections.pull_request_source().await {
            Ok(Some(s)) => s,
            _ => {
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

        let source = match deps.sections.pull_request_source().await {
            Ok(Some(s)) => s,
            _ => {
                self.toast = Some("No pull-request provider is bound".into());
                return;
            }
        };
        let files = source.commit_changes(&pr_id, &sha).await.unwrap_or_default();
        if files.is_empty() {
            self.toast = Some("No per-commit diff for this provider".into());
            return;
        }

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
        let Some(wi) = self.selected_wi() else { return };
        let id = wi.id.clone();
        let wi = wi.clone();
        let threads = match deps.sections.work_item_source().await {
            Ok(Some(src)) => src.threads(&id).await.unwrap_or_default(),
            _ => Vec::new(),
        };
        self.screen = Screen::WiView(Box::new(WiView { wi, threads, scroll: 0 }));
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
                self.screen = Screen::List;
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
                self.screen = Screen::List;
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
            Key::Char('s') => {
                self.open_wi_state();
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
            '1'..='3' => self.set_tab(c as usize - '1' as usize),
            'r' => self.reload_all(deps).await,
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
            'f' if self.active == 0 => self.cycle_pr_filter(deps).await,
            'f' if self.active == 1 => self.open_wi_states_toggle(),
            // Work-item actions (Work Items tab only).
            's' => self.open_wi_state(),
            'c' if self.active == 1 => self.open_wi_comment(),
            // Pipeline trigger (Pipelines tab).
            'T' if self.active == 2 => self.open_pipeline_trigger(),
            // PR write actions (approve / reject / merge / comment / diff) live inside
            // the PR view — press Enter to open a PR first.
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
        let conn_id = pipe.connection_id.clone();
        let run_id = pipe.run.id.clone();
        let definition_id = pipe.run.definition_id.clone();
        let branch = pipe.run.branch.clone();
        let title = pipe_label(pipe);
        let fallback = pipe.run.clone();

        // Enrich with full stages/jobs/steps via get_run (list_runs may be shallow).
        let feeds = deps.sections.pipeline_feeds().await.unwrap_or_default();
        let run = match feeds.iter().find(|f| f.connection.connection_id() == conn_id) {
            Some(feed) => feed.source.get_run(&run_id).await.unwrap_or(fallback),
            None => fallback,
        };

        self.screen = Screen::Pipeline(Box::new(PipelineView::new(title, run, conn_id, definition_id, branch)));
    }

    fn on_pipeline_key(&mut self, key: Key) {
        match key {
            Key::Escape => self.screen = Screen::List,
            Key::Char('q') => self.should_quit = true,
            Key::Char('T') => self.open_pipeline_trigger(),
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
        match overlay.handle(key) {
            Outcome::Keep => self.overlay = Some(overlay),
            Outcome::Cancel => self.toast = Some("Cancelled".into()),
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
            Screen::List | Screen::Config(_) => {}
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
            Key::Char('p') => self.config_bind(Section::PullRequests, deps).await,
            Key::Char('w') => self.config_bind(Section::WorkItems, deps).await,
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

    async fn config_bind(&mut self, section: Section, deps: &AppDeps) {
        let Some((id, _)) = self.config_selected_id() else { return };
        let result = match section {
            Section::PullRequests => deps.config.bind_pull_requests(&id).await,
            Section::WorkItems => deps.config.bind_work_items(&id).await,
            Section::Pipelines => deps.config.set_pipeline_auto_discover(&id, true).await,
        };
        match result {
            Ok(()) => {
                self.toast = Some(format!("Bound to {}", section_label(section)));
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

    fn selected_pr(&self) -> Option<&PullRequest> {
        if self.active != 0 {
            return None;
        }
        let idxs = self.filtered_pr_indices();
        self.pr_state.selected().and_then(|p| idxs.get(p)).and_then(|&i| self.prs.get(i))
    }

    fn open_pr_vote(&mut self, vote: ReviewVote) {
        let Some(pr) = self.selected_pr() else { return };
        let verb = match vote {
            ReviewVote::Approved => "Approve",
            ReviewVote::Rejected => "Request changes on",
            _ => "Vote on",
        };
        let message = format!("{verb} {}?", pr_label(pr));
        self.overlay = Some(Overlay::Confirm { title: "Review".into(), message, action: Action::PrVote(vote) });
    }

    fn open_pr_merge(&mut self) {
        let Some(pr) = self.selected_pr() else { return };
        let title = format!("Merge {} via", pr_label(pr));
        self.overlay = Some(Overlay::Picker {
            title,
            items: vec!["Merge commit".into(), "Squash".into(), "Rebase".into()],
            selected: 0,
            kind: PickerKind::PrMergeStrategy,
        });
    }

    fn open_pr_comment(&mut self) {
        let Some(pr) = self.selected_pr() else { return };
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
        }
    }

    async fn execute_pr_action(&mut self, action: Action, deps: &AppDeps) {
        let Some(id) = self.selected_pr().map(|p| p.id.clone()) else {
            self.toast = Some("Nothing selected".into());
            return;
        };
        let source = match deps.sections.pull_request_source().await {
            Ok(Some(s)) => s,
            _ => {
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

    fn selected_wi(&self) -> Option<&WorkItem> {
        if self.active != 1 {
            return None;
        }
        let idxs = self.filtered_wi_indices();
        self.wi_state.selected().and_then(|p| idxs.get(p)).and_then(|&i| self.wis.get(i))
    }

    fn open_wi_state(&mut self) {
        let Some(wi) = self.selected_wi() else { return };
        let current = wi.state.clone();
        let title = format!("Set state — {}", wi_label(wi));
        // Offer the real state names seen across the current items (provider-accurate);
        // fall back to a generic set if we can't infer at least two.
        let mut states: Vec<String> = self.wis.iter().map(|w| w.state.clone()).collect();
        states.sort();
        states.dedup();
        if states.len() < 2 {
            states = vec!["Todo".into(), "In Progress".into(), "Done".into()];
        }
        let selected = states.iter().position(|s| *s == current).unwrap_or(0);
        self.overlay = Some(Overlay::Picker { title, items: states, selected, kind: PickerKind::WorkItemState });
    }

    fn open_wi_comment(&mut self) {
        let Some(wi) = self.selected_wi() else { return };
        let title = format!("Comment on {}", wi_label(wi));
        self.overlay = Some(Overlay::Input { title, buffer: String::new(), kind: InputKind::WorkItemComment });
    }

    async fn execute_wi_action(&mut self, action: Action, deps: &AppDeps) {
        let Some(id) = self.selected_wi().map(|w| w.id.clone()) else {
            self.toast = Some("Nothing selected".into());
            return;
        };
        let source = match deps.sections.work_item_source().await {
            Ok(Some(s)) => s,
            _ => {
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

fn pipe_label(pipe: &PipeRow) -> String {
    let name = pipe.run.name.clone().unwrap_or_else(|| pipe.run.definition_id.clone());
    match pipe.run.number {
        Some(n) => format!("{name} #{n}"),
        None => name,
    }
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
            let an = a.run.name.clone().unwrap_or_else(|| a.run.definition_id.clone());
            let bn = b.run.name.clone().unwrap_or_else(|| b.run.definition_id.clone());
            ci(&an).cmp(&ci(&bn))
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
    /// Hard quit (Ctrl-C), honoured in every mode.
    Quit,
    /// Terminal was resized — no-op, but wakes the loop so it redraws at the new size.
    Redraw,
    None,
}

fn pr_query(filter: PullRequestFilter) -> PullRequestQuery {
    PullRequestQuery { filter, include_completed: false, limit: Some(50) }
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
        app.prs = vec![a, b, c]; // provider order: numbers 3, 1, 2
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
        app.screen = Screen::Pipeline(Box::new(PipelineView::new("CI".into(), failed_run(), "c".into(), "ci".into(), Some("main".into()))));

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
    fn quick_filter_narrows_and_maps_selection() {
        let mut app = App::new("slate");
        let mut a = pr(None);
        a.title = "Fix login bug".into();
        let mut b = pr(None);
        b.title = "Update deploy pipeline".into();
        b.author.display_name = "Dana".into();
        let mut c = pr(None);
        c.title = "Refactor login flow".into();
        app.prs = vec![a, b, c];
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
        app.prs = vec![a, b];
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
        app.wis = vec![a, b, c];
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
    fn commit_diff_scope_restores_whole_pr() {
        let mut d = diff(vec![changed("b.rs", Some("@@ -1 +1 @@\n-p\n+q"))]);
        d.selected = 0;
        d.scroll = 5;
        d.focus = DiffFocus::Patch;
        d.cursor = 2;
        d.commit_label = Some("abc1234 msg".into());
        let mut v = PrView {
            label: "PR".into(),
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
    fn selected_url_reads_active_tab_and_subview() {
        let mut app = App::new("slate");
        app.prs.push(pr(Some("http://pr")));
        app.pr_state.select(Some(0));
        app.active = 0;
        assert_eq!(app.selected_url().as_deref(), Some("http://pr"));

        app.wis.push(wi(Some("http://wi")));
        app.wi_state.select(Some(0));
        app.active = 1;
        assert_eq!(app.selected_url().as_deref(), Some("http://wi"));

        // An open sub-view takes precedence over the active tab.
        app.screen = Screen::PrView(Box::new(PrView {
            label: "x".into(),
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
            },
            pending: vec![],
            review_draft: None,
        }));
        assert_eq!(app.selected_url().as_deref(), Some("http://prview"));
    }

    #[test]
    fn selected_url_is_none_when_item_has_no_url() {
        let mut app = App::new("slate");
        app.prs.push(pr(None));
        app.pr_state.select(Some(0));
        assert_eq!(app.selected_url(), None);
    }

    #[test]
    fn hiding_a_section_skips_it_in_tabs_and_navigation() {
        let mut app = App::new("slate");
        app.apply_hidden_sections(&[Section::WorkItems]);
        assert_eq!(app.visible_indices(), vec![0, 2]);

        app.active = 0;
        app.switch_tab(1); // PR -> Pipelines, skipping the hidden Work Items
        assert_eq!(app.active, 2);
        app.switch_tab(1); // wraps back to PR
        assert_eq!(app.active, 0);

        app.set_tab(1); // 2nd visible tab is Pipelines
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
