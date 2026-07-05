//! Application state and the (async) update logic driven by the event loop.

use std::sync::Arc;

use chrono::{DateTime, Local};
use forgetop_core::domain::*;
use forgetop_core::provider::*;
use forgetop_core::service::{ConfigService, ConnectionHealth, ConnectionHealthService, SectionService};
use ratatui::widgets::TableState;

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
            last_refresh: Local::now(),
            should_quit: false,
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

        match deps.sections.pull_request_source().await {
            Ok(Some(src)) => match src.list(&pr_query()).await {
                Ok(list) => self.prs = list,
                Err(e) => errors.push(format!("PRs: {e}")),
            },
            Ok(None) => self.prs.clear(),
            Err(e) => errors.push(format!("PRs: {e}")),
        }

        match deps.sections.work_item_source().await {
            Ok(Some(src)) => match src.list(&wi_query()).await {
                Ok(list) => self.wis = list,
                Err(e) => errors.push(format!("Work items: {e}")),
            },
            Ok(None) => self.wis.clear(),
            Err(e) => errors.push(format!("Work items: {e}")),
        }

        self.pipes.clear();
        match deps.sections.pipeline_feeds().await {
            Ok(feeds) => {
                for feed in feeds {
                    let provider = feed.connection.provider_type();
                    let name = feed.connection.display_name().to_string();
                    let queries = feed_queries(&feed.subscription);
                    for q in queries {
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

        self.health = deps.health.check_all().await;

        self.clamp_selection();
        self.wi_state.select((!self.wis.is_empty()).then_some(self.wi_state.selected().unwrap_or(0)));
        self.pipe_state.select((!self.pipes.is_empty()).then_some(self.pipe_state.selected().unwrap_or(0)));
        self.pr_state.select((!self.prs.is_empty()).then_some(self.pr_state.selected().unwrap_or(0)));

        self.last_refresh = Local::now();
        self.loading = false;
        self.status = if errors.is_empty() {
            format!("{} PRs · {} work items · {} runs", self.prs.len(), self.wis.len(), self.pipes.len())
        } else {
            errors.join("  |  ")
        };
    }

    // ---- key handling ----

    /// Returns after applying the key. `deps` is used for async refresh / theme persistence.
    pub async fn on_key(&mut self, key: Key, deps: &AppDeps) {
        match key {
            Key::Quit => self.should_quit = true,
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
            Key::Num(n) => self.set_tab(n),
            Key::Up => self.move_up(),
            Key::Down => self.move_down(),
            Key::Enter => {
                if self.selected().is_some() {
                    self.show_detail = !self.show_detail;
                }
            }
            Key::Refresh => self.reload_all(deps).await,
            Key::Theme => {
                let next = Theme::next(self.theme.name);
                self.theme = Theme::by_name(next);
                let _ = deps.config.set_theme(Some(next.to_string())).await;
            }
            Key::None => {}
        }
    }
}

/// Semantic key events the loop feeds into [`App::on_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Tab,
    Enter,
    Escape,
    Refresh,
    Theme,
    Quit,
    Num(usize),
    None,
}

fn pr_query() -> PullRequestQuery {
    PullRequestQuery { filter: PullRequestFilter::All, include_completed: false, limit: Some(50) }
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
