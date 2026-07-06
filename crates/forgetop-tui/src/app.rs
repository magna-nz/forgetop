//! Application state and the (async) update logic driven by the event loop.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Local};
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
    pub show_detail: bool,
    pub pr_filter: PullRequestFilter,
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
    Diff(Box<DiffView>),
    Pipeline(Box<PipelineView>),
    Config(Box<ConfigView>),
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
}

impl PipelineView {
    pub fn new(title: String, run: PipelineRun, connection_id: String, definition_id: String, branch: Option<String>) -> Self {
        Self { title, run, connection_id, definition_id, branch, collapsed: HashSet::new(), selected: 0 }
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
                });
                if jexpanded {
                    for step in &job.steps {
                        out.push(FlatNode { depth: 2, label: step.name.clone(), status: step.status, key: None, expanded: false });
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

/// State for the full-screen PR diff + threads view.
pub struct DiffView {
    pub pr_label: String,
    pub url: Option<String>,
    pub files: Vec<FileChange>,
    pub threads: Vec<CommentThread>,
    pub selected: usize,
    pub scroll: u16,
}

impl DiffView {
    pub fn current(&self) -> Option<&FileChange> {
        self.files.get(self.selected)
    }

    fn select_file(&mut self, delta: isize) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
        self.scroll = 0;
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
            show_detail: false,
            pr_filter: PullRequestFilter::All,
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

    // ---- selection ----

    pub fn active_len(&self) -> usize {
        match self.active {
            0 => self.prs.len(),
            1 => self.wis.len(),
            _ => self.pipes.len(),
        }
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
        self.show_detail = false;
        self.clamp_selection();
    }

    /// Jumps to the Nth *visible* tab (0-based), for the number keys.
    pub fn set_tab(&mut self, idx: usize) {
        if let Some(&section) = self.visible_indices().get(idx) {
            self.active = section;
            self.show_detail = false;
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

    /// Re-selects a valid row per tab after the underlying data changed.
    fn fix_selection(&mut self) {
        self.clamp_selection();
        self.pr_state.select((!self.prs.is_empty()).then_some(self.pr_state.selected().unwrap_or(0)));
        self.wi_state.select((!self.wis.is_empty()).then_some(self.wi_state.selected().unwrap_or(0)));
        self.pipe_state.select((!self.pipes.is_empty()).then_some(self.pipe_state.selected().unwrap_or(0)));
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
        self.pr_state.select((!self.prs.is_empty()).then_some(0));
        self.show_detail = false;
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

        // Full-screen sub-views handle their own keys.
        match self.screen {
            Screen::Diff(_) => {
                self.on_diff_key(key);
                return;
            }
            Screen::Pipeline(_) => {
                self.on_pipeline_key(key);
                return;
            }
            Screen::Config(_) => {
                self.on_config_key(key, deps).await;
                return;
            }
            Screen::List => {}
        }

        match key {
            Key::Escape => {
                if self.show_detail {
                    self.show_detail = false;
                } else {
                    self.should_quit = true;
                }
            }
            Key::Left => self.switch_tab(-1),
            Key::Right => self.switch_tab(1),
            Key::Tab => self.switch_tab(1),
            Key::Up => self.move_up(),
            Key::Down => self.move_down(),
            Key::Enter => {
                if self.active == 2 {
                    self.open_pipeline(deps).await;
                } else if self.selected().is_some() {
                    self.show_detail = !self.show_detail;
                }
            }
            Key::Char(c) => self.on_char(c, deps).await,
            Key::Backspace | Key::PageUp | Key::PageDown | Key::Quit | Key::None => {}
        }
    }

    /// Normal-mode character commands.
    async fn on_char(&mut self, c: char, deps: &AppDeps) {
        match c {
            'q' => self.should_quit = true,
            'j' => self.move_down(),
            'k' => self.move_up(),
            'h' => self.switch_tab(-1),
            'l' => self.switch_tab(1),
            '1'..='3' => self.set_tab(c as usize - '1' as usize),
            'r' => self.reload_all(deps).await,
            't' => {
                let next = Theme::next(self.theme.name);
                self.theme = Theme::by_name(next);
                let _ = deps.config.set_theme(Some(next.to_string())).await;
            }
            'o' => self.open_selected(),
            'n' => self.start_add_connection(),
            'v' => self.open_sections_toggle(),
            'C' => self.open_config(deps).await,
            'f' if self.active == 0 => self.cycle_pr_filter(deps).await,
            // PR write actions (Pull Requests tab only; each no-ops off-tab).
            'a' => self.open_pr_vote(ReviewVote::Approved),
            'x' => self.open_pr_vote(ReviewVote::Rejected),
            'm' => self.open_pr_merge(),
            'd' => self.open_diff(deps).await,
            // Work-item actions (Work Items tab only).
            's' => self.open_wi_state(),
            // Pipeline trigger (Pipelines tab).
            'T' if self.active == 2 => self.open_pipeline_trigger(),
            // Comment is offered on both PR and work-item tabs.
            'c' => match self.active {
                0 => self.open_pr_comment(),
                1 => self.open_wi_comment(),
                _ => {}
            },
            _ => {}
        }
    }

    /// Key handling while the full-screen diff view is open.
    fn on_diff_key(&mut self, key: Key) {
        let Screen::Diff(diff) = &mut self.screen else { return };
        match key {
            Key::Escape => self.screen = Screen::List,
            Key::Char('q') => self.should_quit = true,
            Key::Char('o') => self.open_selected(),
            Key::Up | Key::Char('k') => diff.select_file(-1),
            Key::Down | Key::Char('j') => diff.select_file(1),
            Key::PageDown | Key::Char(' ') => diff.scroll_by(10),
            Key::PageUp | Key::Char('b') => diff.scroll_by(-10),
            _ => {}
        }
    }

    async fn open_diff(&mut self, deps: &AppDeps) {
        let Some(pr) = self.selected_pr() else { return };
        let id = pr.id.clone();
        let label = pr_label(pr);
        let url = pr.url.clone();
        let source = match deps.sections.pull_request_source().await {
            Ok(Some(s)) => s,
            _ => {
                self.toast = Some("No pull-request provider is bound".into());
                return;
            }
        };
        let files = match source.changes(&id).await {
            Ok(f) => f,
            Err(e) => {
                self.toast = Some(format!("Diff failed: {e}"));
                return;
            }
        };
        let threads = source.threads(&id).await.unwrap_or_default();
        self.screen = Screen::Diff(Box::new(DiffView { pr_label: label, url, files, threads, selected: 0, scroll: 0 }));
    }

    // ---- pipeline drill-in + trigger ----

    fn selected_pipe(&self) -> Option<&PipeRow> {
        if self.active != 2 {
            return None;
        }
        self.pipe_state.selected().and_then(|i| self.pipes.get(i))
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
            Screen::Diff(v) => return v.url.clone(),
            Screen::Pipeline(v) => return v.run.url.clone(),
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

    async fn apply_toggle(&mut self, kind: ToggleKind, ids: Vec<String>, deps: &AppDeps) {
        match kind {
            ToggleKind::Sections => {
                let visible = ids.iter().filter_map(|id| id.parse::<usize>().ok()).map(section_of).collect();
                self.apply_visible_sections(visible, deps).await;
            }
            ToggleKind::PipelineSubs { connection_id } => {
                self.apply_pipeline_subs(&connection_id, ids, deps).await;
            }
        }
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
            self.show_detail = false;
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

        let pr_binding = cfg.pull_requests.as_ref().map(|b| display_of(&b.connection_id));
        let wi_binding = cfg.work_items.as_ref().map(|b| display_of(&b.connection_id));
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
                if cfg.pull_requests.as_ref().is_some_and(|b| b.connection_id == c.id) {
                    bindings.push("PR");
                }
                if cfg.work_items.as_ref().is_some_and(|b| b.connection_id == c.id) {
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
        self.pr_state.selected().and_then(|i| self.prs.get(i))
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
        self.wi_state.selected().and_then(|i| self.wis.get(i))
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
    None,
}

fn pr_query(filter: PullRequestFilter) -> PullRequestQuery {
    PullRequestQuery { filter, include_completed: false, limit: Some(50) }
}

fn wi_query() -> WorkItemQuery {
    WorkItemQuery { mine_only: false, include_completed: false, limit: Some(50) }
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
        app.screen = Screen::Diff(Box::new(DiffView {
            pr_label: "x".into(),
            url: Some("http://diff".into()),
            files: vec![],
            threads: vec![],
            selected: 0,
            scroll: 0,
        }));
        assert_eq!(app.selected_url().as_deref(), Some("http://diff"));
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
