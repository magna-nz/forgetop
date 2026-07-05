//! Application state and the (async) update logic driven by the event loop.

use std::sync::Arc;

use chrono::{DateTime, Local};
use forgetop_core::domain::*;
use forgetop_core::provider::*;
use forgetop_core::service::{ConfigService, ConnectionHealth, ConnectionHealthService, SectionService};
use ratatui::widgets::TableState;

use crate::overlay::{Action, InputKind, Outcome, Overlay, PickerKind};
use crate::theme::Theme;

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
    pub status: String,
    pub loading: bool,
    pub show_detail: bool,
    pub pr_filter: PullRequestFilter,
    /// Transient one-shot message shown in the footer until the next keypress.
    pub toast: Option<String>,
    /// Open modal overlay, if any. When set, keys route here instead of the table.
    pub overlay: Option<Overlay>,
    pub last_refresh: DateTime<Local>,
    pub should_quit: bool,
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
            status: "Loading…".into(),
            loading: true,
            show_detail: false,
            pr_filter: PullRequestFilter::All,
            toast: None,
            overlay: None,
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

    pub fn switch_tab(&mut self, delta: isize) {
        let n = TABS.len() as isize;
        self.active = (((self.active as isize + delta) % n + n) % n) as usize;
        self.show_detail = false;
        self.clamp_selection();
    }

    pub fn set_tab(&mut self, idx: usize) {
        if idx < TABS.len() {
            self.active = idx;
            self.show_detail = false;
            self.clamp_selection();
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
                    for q in feed_queries(&feed.subscription) {
                        match feed.source.list_runs(&q).await {
                            Ok(runs) => {
                                for run in runs {
                                    self.pipes.push(PipeRow { connection: name.clone(), provider, run });
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

        // An open overlay swallows all input until it resolves.
        if self.overlay.is_some() {
            self.on_overlay_key(key, deps).await;
            return;
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
                if self.selected().is_some() {
                    self.show_detail = !self.show_detail;
                }
            }
            Key::Char(c) => self.on_char(c, deps).await,
            Key::Backspace | Key::Quit | Key::None => {}
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
            'f' if self.active == 0 => self.cycle_pr_filter(deps).await,
            // PR write actions (Pull Requests tab only).
            'a' => self.open_pr_vote(ReviewVote::Approved),
            'x' => self.open_pr_vote(ReviewVote::Rejected),
            'm' => self.open_pr_merge(),
            'c' => self.open_pr_comment(),
            _ => {}
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
