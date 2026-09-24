//! Command palette (`Ctrl-K`, alias `Ctrl-P`) — "search everything" over what the app
//! already holds: the actions valid on the current screen, every fetched PR / work item /
//! pipeline, the tab strip and other destinations, saved views, repositories, people,
//! settings, the keybinding reference, and (behind `:`) a small command language.
//!
//! This module is the **pure core**: flat [`PaletteItem`] lists built from app state, and
//! [`rank`], which filters by prefix, fuzzy-ranks and groups them for a query. No UI, no
//! I/O, no provider calls — everything operates on data already in memory. What running an
//! entry *does* is described by its [`PaletteTarget`]; the app interprets it (and runs an
//! action by replaying its key, so the palette can never drift from the keyboard).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use forgetop_core::config::SavedView;
use forgetop_core::domain::{PipelineRunStatus, PullRequestStatus, User, WorkItemStateCategory};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

use crate::app::{Key, PipeRow, PrRow, WiRow};

/// Which kind of item a palette result routes to — decides the "open" path on `Enter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKind {
    Pr,
    Wi,
    Pipe,
}

/// The group a palette entry belongs to. Results are shown grouped, under a header per
/// group; the declaration order is the tiebreak when two groups' best hits score the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// Actions valid on the current screen, each run by replaying its key.
    Action,
    /// Already-fetched pull requests, work items and pipeline runs.
    Item,
    /// Tabs and other places / global commands.
    GoTo,
    /// Saved views, per section.
    View,
    /// Repositories seen in the loaded rows.
    Repo,
    /// PR authors and work-item assignees.
    Person,
    /// Theme and other preferences.
    Setting,
    /// The keybinding reference (the `?` help table).
    Key,
    /// `:`-prefixed commands — only shown in `:` mode.
    Command,
}

impl Group {
    /// The group header, and the panel title in a prefix mode.
    pub fn label(self) -> &'static str {
        match self {
            Group::Action => "Actions",
            Group::Item => "Items",
            Group::GoTo => "Go to",
            Group::View => "Views",
            Group::Repo => "Repos",
            Group::Person => "People",
            Group::Setting => "Settings",
            Group::Key => "Keys",
            Group::Command => "Commands",
        }
    }
}

/// A destination or global command, reached by calling the app's own function for it (not
/// by replaying a key: global keys like `n` are shadowed on some screens, e.g. the log pane).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoTo {
    CommandCenter,
    /// A section list (0 = Pull Requests, 1 = Work Items, 2 = Pipelines).
    Section(usize),
    Inbox,
    Connections,
    Help,
    Dashboard,
    AddConnection,
    Notifications,
    Refresh,
}

/// What running a palette entry does. Interpreted by the app.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteTarget {
    /// Open an already-fetched item, re-resolved by `(kind, id)` from the app's lists.
    Item {
        kind: PaletteKind,
        id: String,
        connection_id: String,
    },
    /// Replay this key through the normal key path.
    Key(Key),
    GoTo(GoTo),
    /// Switch to a section and apply its saved view at `idx`.
    View {
        section: usize,
        idx: usize,
    },
    /// Switch to a section and set its quick filter to `text`.
    Filter {
        section: usize,
        text: String,
    },
    /// Switch to (and persist) this theme.
    Theme(String),
    /// A keybinding from the help table: run it when it's a key the screen answers,
    /// otherwise say where to press it.
    HelpKey {
        keys: String,
        section: String,
    },
    /// Open the merge-strategy picker with this row (0 merge, 1 squash, 2 rebase) preselected.
    MergePicker {
        selected: usize,
    },
}

/// A row's status in the shared green/blue/red/grey model (the UI maps this to a colour).
/// Mirrors `pr_status` / `wi_state_color` / `Theme::pipeline_color` — keep in step with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Done / healthy — green.
    Good,
    /// Actively in flight — blue.
    Active,
    /// Partial — yellow.
    Warn,
    /// Failed / blocked / worth a look — red.
    Bad,
    /// Shipped — a merged PR — magenta.
    Merged,
    /// Waiting / neutral — grey.
    Neutral,
}

/// One searchable row in the palette. Holds only what's needed to display a result and to
/// run it — never the heavy domain struct itself. The app re-resolves the full
/// `PullRequest` / `WorkItem` / `PipelineRun` from its own lists by `(kind, id)`.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteItem {
    pub group: Group,
    pub target: PaletteTarget,
    /// Primary match text (an item's title, an action's label, …).
    pub title: String,
    /// Secondary match text: author / repo / identifier / branch / connection, the screen an
    /// action belongs to, a keybinding's help section — so results are disambiguable and
    /// searchable by more than the title.
    pub subtitle: String,
    /// Status in the shared colour model, for the leading dot on item rows.
    pub tone: Option<Tone>,
    /// The key that does the same thing, shown as a badge on the right.
    pub key_hint: Option<String>,
    /// Recency key (updated/finished time) — the tiebreak when scores are equal and the
    /// order of items for an empty query.
    pub sort_ts: Option<DateTime<Utc>>,
}

impl PaletteItem {
    /// A bare entry; fill in the rest with the `with_*` builders.
    pub fn new(group: Group, target: PaletteTarget, title: impl Into<String>) -> Self {
        PaletteItem {
            group,
            target,
            title: title.into(),
            subtitle: String::new(),
            tone: None,
            key_hint: None,
            sort_ts: None,
        }
    }

    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key_hint = Some(key.into());
        self
    }

    /// Short type tag shown at the head of the row.
    pub fn tag(&self) -> &'static str {
        match (&self.target, self.group) {
            (
                PaletteTarget::Item {
                    kind: PaletteKind::Pr,
                    ..
                },
                _,
            ) => "PR",
            (
                PaletteTarget::Item {
                    kind: PaletteKind::Wi,
                    ..
                },
                _,
            ) => "WI",
            (
                PaletteTarget::Item {
                    kind: PaletteKind::Pipe,
                    ..
                },
                _,
            ) => "CI",
            (_, Group::Action) => "do",
            (_, Group::Item) => "",
            (_, Group::GoTo) => "go",
            (_, Group::View) => "view",
            (_, Group::Repo) => "repo",
            (_, Group::Person) => "@",
            (_, Group::Setting) => "set",
            (_, Group::Key) => "key",
            (_, Group::Command) => ":",
        }
    }
}

/// Human label for a key, for the right-hand badge.
pub fn key_label(key: Key) -> String {
    match key {
        Key::Char(' ') => "Space".into(),
        Key::Char(c) => c.to_string(),
        Key::Ctrl(c) => format!("Ctrl-{}", c.to_ascii_uppercase()),
        Key::Enter => "↵".into(),
        Key::Escape => "Esc".into(),
        Key::Tab => "Tab".into(),
        Key::BackTab => "Shift-Tab".into(),
        Key::Up => "↑".into(),
        Key::Down => "↓".into(),
        Key::Left => "←".into(),
        Key::Right => "→".into(),
        Key::Backspace => "⌫".into(),
        Key::PageUp => "PgUp".into(),
        Key::PageDown => "PgDn".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::Quit => "Ctrl-C".into(),
        Key::Redraw | Key::Click(..) | Key::ScrollUp(..) | Key::ScrollDown(..) | Key::None => String::new(),
    }
}

/// Section names, indexed like the app's sections (0 = PRs, 1 = WIs, 2 = Pipelines).
const SECTIONS: [&str; 3] = ["Pull Requests", "Work Items", "Pipelines"];

/// A user's most human-recognisable handle, for a subtitle.
fn who(user: &User) -> &str {
    user.handle.as_deref().unwrap_or(&user.display_name)
}

/// Join the non-empty parts of a subtitle with " · ".
fn subtitle(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// PR tone — mirrors `pr_status`: draft grey, open/merged green, closed red.
fn pr_tone(row: &PrRow) -> Tone {
    if row.pr.is_draft {
        return Tone::Neutral;
    }
    match row.pr.status {
        PullRequestStatus::Open => Tone::Good,
        PullRequestStatus::Merged => Tone::Merged,
        PullRequestStatus::Closed => Tone::Bad,
        PullRequestStatus::Draft => Tone::Neutral,
    }
}

fn item(kind: PaletteKind, id: &str, connection_id: &str, title: String) -> PaletteItem {
    let target = PaletteTarget::Item {
        kind,
        id: id.to_string(),
        connection_id: connection_id.to_string(),
    };
    PaletteItem::new(Group::Item, target, title)
}

pub fn pr_candidate(row: &PrRow) -> PaletteItem {
    let branch = row.pr.source_ref.as_deref().unwrap_or("");
    // The number makes `#` mode ("# ids") find a PR by it.
    let number = row.pr.number.map(|n| format!("#{n}")).unwrap_or_default();
    PaletteItem {
        subtitle: subtitle(&[&number, who(&row.pr.author), branch, &row.connection]),
        tone: Some(pr_tone(row)),
        sort_ts: row.pr.updated_at,
        ..item(
            PaletteKind::Pr,
            &row.pr.id,
            &row.connection_id,
            row.pr.title.clone(),
        )
    }
}

/// WI tone — mirrors `wi_state_color`: blocked red, completed green, started blue, else grey.
fn wi_tone(row: &WiRow) -> Tone {
    if row.wi.state.eq_ignore_ascii_case("blocked") {
        return Tone::Bad;
    }
    match row.wi.state_category {
        WorkItemStateCategory::Completed => Tone::Good,
        WorkItemStateCategory::Started => Tone::Active,
        _ => Tone::Neutral,
    }
}

pub fn wi_candidate(row: &WiRow) -> PaletteItem {
    let ident = row.wi.identifier.as_deref().unwrap_or("");
    let ty = row.wi.work_item_type.as_deref().unwrap_or("");
    PaletteItem {
        subtitle: subtitle(&[ident, ty, &row.connection]),
        tone: Some(wi_tone(row)),
        sort_ts: row.wi.updated_at,
        ..item(
            PaletteKind::Wi,
            &row.wi.id,
            &row.connection_id,
            row.wi.title.clone(),
        )
    }
}

pub fn pipe_candidate(row: &PipeRow) -> PaletteItem {
    // Title is the pipeline (definition) name — matching how pipelines read elsewhere —
    // falling back to the run's own name.
    let title = row
        .definition_name
        .clone()
        .or_else(|| row.run.name.clone())
        .unwrap_or_else(|| "pipeline".to_string());
    let run_name = row.run.name.as_deref().unwrap_or("");
    let branch = row.run.branch.as_deref().unwrap_or("");
    // Mirrors Theme::pipeline_color: succeeded green, running blue, failed red,
    // partial yellow, queued/canceled grey.
    let tone = match row.run.status {
        PipelineRunStatus::Succeeded => Tone::Good,
        PipelineRunStatus::Running => Tone::Active,
        PipelineRunStatus::Failed => Tone::Bad,
        PipelineRunStatus::PartiallySucceeded => Tone::Warn,
        PipelineRunStatus::Queued | PipelineRunStatus::Canceled => Tone::Neutral,
    };
    PaletteItem {
        subtitle: subtitle(&[run_name, branch, &row.connection]),
        tone: Some(tone),
        sort_ts: row.run.finished_at.or(row.run.started_at),
        ..item(PaletteKind::Pipe, &row.run.id, &row.connection_id, title)
    }
}

/// Build the item candidates from the app's three row lists, in a stable order
/// (PRs, then work items, then pipelines). Ordering only matters for the empty-query and
/// equal-score cases, both of which are then resolved by recency in [`rank`].
pub fn build_candidates(prs: &[PrRow], wis: &[WiRow], pipes: &[PipeRow]) -> Vec<PaletteItem> {
    prs.iter()
        .map(pr_candidate)
        .chain(wis.iter().map(wi_candidate))
        .chain(pipes.iter().map(pipe_candidate))
        .collect()
}

/// The current screen's actions, each paired with the key that performs it. `context`
/// names the screen, for the subtitle.
pub fn action_items(actions: &[(&str, Key)], context: &str) -> Vec<PaletteItem> {
    actions
        .iter()
        .map(|&(label, key)| {
            PaletteItem::new(Group::Action, PaletteTarget::Key(key), label)
                .with_subtitle(context)
                .with_key(key_label(key))
        })
        .collect()
}

/// Destinations: the tab strip (visible sections only), then the global places.
pub fn goto_items(visible: &[bool; 3]) -> Vec<PaletteItem> {
    let mut out = vec![PaletteItem::new(
        Group::GoTo,
        PaletteTarget::GoTo(GoTo::CommandCenter),
        "Command Center",
    )];
    for (i, name) in SECTIONS.iter().enumerate().filter(|(i, _)| visible[*i]) {
        out.push(PaletteItem::new(
            Group::GoTo,
            PaletteTarget::GoTo(GoTo::Section(i)),
            *name,
        ));
    }
    let places = [
        (GoTo::Inbox, "Inbox", "i"),
        (GoTo::Connections, "Connections", "C"),
        (GoTo::Help, "Help", "?"),
        (GoTo::Dashboard, "Web dashboard", "B"),
        (GoTo::AddConnection, "Add connection", "n"),
    ];
    for (dest, label, key) in places {
        out.push(PaletteItem::new(Group::GoTo, PaletteTarget::GoTo(dest), label).with_key(key));
    }
    out
}

/// Every saved view of every visible section; `active` marks each section's current one.
pub fn view_items(
    views: &[Vec<SavedView>; 3],
    active: &[usize; 3],
    visible: &[bool; 3],
) -> Vec<PaletteItem> {
    let mut out = Vec::new();
    for section in (0..3).filter(|&s| visible[s]) {
        for (idx, view) in views[section].iter().enumerate() {
            let current = if active[section] == idx {
                "current"
            } else {
                ""
            };
            out.push(
                PaletteItem::new(
                    Group::View,
                    PaletteTarget::View { section, idx },
                    view.name.clone(),
                )
                .with_subtitle(subtitle(&[SECTIONS[section], current])),
            );
        }
    }
    out
}

/// Distinct repositories in the loaded pipeline runs; Enter filters Pipelines by it.
///
/// Only pipelines: the Pipelines quick filter matches on the repository, while the PR and
/// work-item quick filters don't — a repository filter there would just empty the list.
pub fn repo_items(pipes: &[PipeRow], pipelines_visible: bool) -> Vec<PaletteItem> {
    if !pipelines_visible {
        return Vec::new();
    }
    let mut counts: Vec<(String, usize)> = Vec::new();
    for repo in pipes.iter().filter_map(|p| p.run.repository.as_deref()) {
        match counts.iter_mut().find(|(r, _)| r == repo) {
            Some((_, n)) => *n += 1,
            None => counts.push((repo.to_string(), 1)),
        }
    }
    counts
        .into_iter()
        .map(|(repo, n)| {
            let target = PaletteTarget::Filter {
                section: 2,
                text: repo.clone(),
            };
            let runs = format!("{n} run{}", if n == 1 { "" } else { "s" });
            PaletteItem::new(Group::Repo, target, repo)
                .with_subtitle(subtitle(&["Pipelines", &runs]))
        })
        .collect()
}

/// Distinct PR authors and work-item assignees. Enter filters Pull Requests by the person
/// when they author any loaded PR, else Work Items (both quick filters match the display
/// name). A person whose only section is hidden is left out.
pub fn people_items(prs: &[PrRow], wis: &[WiRow], visible: &[bool; 3]) -> Vec<PaletteItem> {
    struct Person {
        name: String,
        handle: Option<String>,
        prs: usize,
        wis: usize,
    }
    let mut order: Vec<String> = Vec::new();
    let mut people: HashMap<String, Person> = HashMap::new();
    let mut add = |user: &User, is_pr: bool| {
        let key = user.display_name.to_lowercase();
        if key.trim().is_empty() {
            return;
        }
        let p = people.entry(key.clone()).or_insert_with(|| {
            order.push(key);
            Person {
                name: user.display_name.clone(),
                handle: user.handle.clone(),
                prs: 0,
                wis: 0,
            }
        });
        if p.handle.is_none() {
            p.handle = user.handle.clone();
        }
        if is_pr {
            p.prs += 1;
        } else {
            p.wis += 1;
        }
    };
    for row in prs {
        add(&row.pr.author, true);
    }
    for user in wis.iter().filter_map(|r| r.wi.assignee.as_ref()) {
        add(user, false);
    }
    order
        .iter()
        .filter_map(|key| people.get(key))
        .filter_map(|p| {
            let section = if p.prs > 0 && visible[0] {
                0
            } else if p.wis > 0 && visible[1] {
                1
            } else {
                return None;
            };
            let handle = p
                .handle
                .as_deref()
                .map(|h| format!("@{h}"))
                .unwrap_or_default();
            let prs = if p.prs > 0 {
                format!("{} PR{}", p.prs, if p.prs == 1 { "" } else { "s" })
            } else {
                String::new()
            };
            let wis = if p.wis > 0 {
                format!("{} WI{}", p.wis, if p.wis == 1 { "" } else { "s" })
            } else {
                String::new()
            };
            let target = PaletteTarget::Filter {
                section,
                text: p.name.clone(),
            };
            Some(
                PaletteItem::new(Group::Person, target, p.name.clone())
                    .with_subtitle(subtitle(&[&handle, &prs, &wis])),
            )
        })
        .collect()
}

/// Settings: one entry per theme (the current one marked), then the other preferences.
pub fn setting_items(themes: &[&str], current_theme: &str) -> Vec<PaletteItem> {
    let mut out: Vec<PaletteItem> = themes
        .iter()
        .map(|&name| {
            let item = PaletteItem::new(
                Group::Setting,
                PaletteTarget::Theme(name.to_string()),
                format!("Theme: {name}"),
            );
            if name == current_theme {
                item.with_subtitle("current")
            } else {
                item
            }
        })
        .collect();
    let others = [
        (GoTo::Notifications, "Notification settings", "N"),
        (GoTo::Refresh, "Refresh now", "r"),
    ];
    for (dest, label, key) in others {
        out.push(PaletteItem::new(Group::Setting, PaletteTarget::GoTo(dest), label).with_key(key));
    }
    out
}

/// The keybinding reference: one entry per `(keys, description)` row of each help section.
pub fn key_items(sections: &[(&str, Vec<(&str, &str)>)]) -> Vec<PaletteItem> {
    sections
        .iter()
        .flat_map(|(section, rows)| {
            rows.iter().map(move |&(keys, desc)| {
                let target = PaletteTarget::HelpKey {
                    keys: keys.to_string(),
                    section: section.to_string(),
                };
                PaletteItem::new(Group::Key, target, desc)
                    .with_subtitle(*section)
                    .with_key(keys)
            })
        })
        .collect()
}

/// What the `:` command language can currently offer beyond the always-available commands.
pub struct CommandContext<'a> {
    pub themes: &'a [&'a str],
    pub views: &'a [Vec<SavedView>; 3],
    pub visible: &'a [bool; 3],
    /// A PR view is open on a PR that isn't merged (`m` would offer the merge picker).
    pub can_merge: bool,
    /// A work-item view is open (`u` would offer the state picker).
    pub can_set_state: bool,
}

/// The concrete `:` commands for the current state: `:theme <name>`, `:view <name>`,
/// `:go <screen>`, and — only where their key would work — `:merge <strategy>` / `:state`.
pub fn command_items(ctx: &CommandContext) -> Vec<PaletteItem> {
    let cmd = |target: PaletteTarget, text: String| PaletteItem::new(Group::Command, target, text);
    let mut out = Vec::new();
    for &name in ctx.themes {
        out.push(cmd(
            PaletteTarget::Theme(name.into()),
            format!(":theme {name}"),
        ));
    }
    for section in (0..3).filter(|&s| ctx.visible[s]) {
        for (idx, view) in ctx.views[section].iter().enumerate() {
            out.push(
                cmd(
                    PaletteTarget::View { section, idx },
                    format!(":view {}", view.name),
                )
                .with_subtitle(SECTIONS[section]),
            );
        }
    }
    let mut dests = vec![("command-center", GoTo::CommandCenter)];
    let slugs = ["pull-requests", "work-items", "pipelines"];
    for (i, slug) in slugs.iter().enumerate().filter(|(i, _)| ctx.visible[*i]) {
        dests.push((slug, GoTo::Section(i)));
    }
    dests.extend([
        ("inbox", GoTo::Inbox),
        ("connections", GoTo::Connections),
        ("help", GoTo::Help),
    ]);
    for (slug, dest) in dests {
        out.push(cmd(PaletteTarget::GoTo(dest), format!(":go {slug}")));
    }
    if ctx.can_merge {
        // Rows of the `m` picker: 0 merge commit, 1 squash, 2 rebase.
        for (strategy, selected) in [("squash", 1), ("rebase", 2), ("merge", 0)] {
            out.push(
                cmd(
                    PaletteTarget::MergePicker { selected },
                    format!(":merge {strategy}"),
                )
                .with_key("m"),
            );
        }
    }
    if ctx.can_set_state {
        out.push(cmd(PaletteTarget::Key(Key::Char('u')), ":state".into()).with_key("u"));
    }
    out
}

/// Split a query into its mode prefix (if any) and the text to match.
///
/// `>` actions, `:` commands, `@` people, `#` items, `?` keys. No prefix searches every
/// group except commands.
pub fn parse_query(query: &str) -> (Option<Group>, &str) {
    let q = query.trim_start();
    let mut chars = q.chars();
    let group = match chars.next() {
        Some('>') => Group::Action,
        Some(':') => Group::Command,
        Some('@') => Group::Person,
        Some('#') => Group::Item,
        Some('?') => Group::Key,
        _ => return (None, q.trim()),
    };
    (Some(group), chars.as_str().trim())
}

/// The panel title for a query: "Search everything", or the prefix mode's group.
pub fn mode_title(query: &str) -> &'static str {
    match parse_query(query).0 {
        Some(group) => group.label(),
        None => "Search everything",
    }
}

/// What `Tab` completes the query to for the selected entry: its text, keeping the current
/// mode prefix (a command's text already carries its `:`).
pub fn complete(query: &str, item: &PaletteItem) -> String {
    if item.group == Group::Command {
        return item.title.clone();
    }
    let prefix: String = query
        .trim_start()
        .chars()
        .next()
        .filter(|c| ">@#?".contains(*c))
        .map(String::from)
        .unwrap_or_default();
    format!("{prefix}{}", item.title)
}

/// Score added to an action's match so the current screen's actions float above equally
/// good matches elsewhere, without burying a clearly better one.
const ACTION_BOOST: i64 = 20;

/// Newest-first comparison on `sort_ts`; items without a timestamp sort last.
fn by_recency_desc(a: &PaletteItem, b: &PaletteItem) -> std::cmp::Ordering {
    b.sort_ts.cmp(&a.sort_ts)
}

/// Filter and order `candidates` for `query`, returning indices into `candidates`.
///
/// - A mode prefix (see [`parse_query`]) keeps one group; without one, every group but
///   commands is searched.
/// - Empty text, no prefix → the current screen's actions, then items most-recent first,
///   then the go-to destinations. Empty text in a prefix mode → that whole group.
/// - Otherwise → only candidates whose title *or* subtitle fuzzy-matches, grouped by
///   group, groups ordered by their best hit; within a group by score, then recency.
///   Actions get a small boost. Case-insensitive.
///
/// Results always come out contiguous per group, so the UI can put a header above each.
pub fn rank(query: &str, candidates: &[PaletteItem]) -> Vec<usize> {
    let (mode, text) = parse_query(query);
    let in_scope = |c: &PaletteItem| match mode {
        Some(group) => c.group == group,
        None => c.group != Group::Command,
    };

    if text.is_empty() {
        let shown = |c: &PaletteItem| match mode {
            Some(_) => in_scope(c),
            None => matches!(c.group, Group::Action | Group::Item | Group::GoTo),
        };
        let mut idx: Vec<usize> = (0..candidates.len())
            .filter(|&i| shown(&candidates[i]))
            .collect();
        // Stable: within a group, input order is kept except items, which go by recency.
        idx.sort_by(|&a, &b| {
            let (ca, cb) = (&candidates[a], &candidates[b]);
            ca.group
                .cmp(&cb.group)
                .then_with(|| by_recency_desc(ca, cb))
        });
        return idx;
    }

    // ignore_case (not the default smart-case) so an uppercase query still matches — a
    // palette should filter predictably regardless of how the query is typed.
    let matcher = SkimMatcherV2::default().ignore_case();
    let scored: Vec<(usize, i64)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| in_scope(c))
        .filter_map(|(i, c)| {
            let title = matcher.fuzzy_match(&c.title, text);
            let sub = matcher.fuzzy_match(&c.subtitle, text);
            let boost = if c.group == Group::Action {
                ACTION_BOOST
            } else {
                0
            };
            title.max(sub).map(|score| (i, score + boost))
        })
        .collect();

    let mut best: HashMap<Group, i64> = HashMap::new();
    for &(i, score) in &scored {
        let b = best.entry(candidates[i].group).or_insert(score);
        *b = (*b).max(score);
    }

    let mut scored = scored;
    scored.sort_by(|&(ai, asc), &(bi, bsc)| {
        let (ca, cb) = (&candidates[ai], &candidates[bi]);
        best[&cb.group]
            .cmp(&best[&ca.group])
            .then_with(|| ca.group.cmp(&cb.group))
            .then_with(|| bsc.cmp(&asc))
            .then_with(|| by_recency_desc(ca, cb))
    });
    scored.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> Option<DateTime<Utc>> {
        Some(Utc.timestamp_opt(secs, 0).unwrap())
    }

    fn item(kind: PaletteKind, title: &str, subtitle: &str, ts_secs: i64) -> PaletteItem {
        let target = PaletteTarget::Item {
            kind,
            id: format!("{title}-id"),
            connection_id: "conn".into(),
        };
        PaletteItem {
            subtitle: subtitle.into(),
            tone: Some(Tone::Neutral),
            sort_ts: ts(ts_secs),
            ..PaletteItem::new(Group::Item, target, title)
        }
    }

    fn entry(group: Group, title: &str) -> PaletteItem {
        PaletteItem::new(group, PaletteTarget::GoTo(GoTo::Help), title)
    }

    fn action(title: &str, key: char) -> PaletteItem {
        PaletteItem::new(Group::Action, PaletteTarget::Key(Key::Char(key)), title)
    }

    fn titles(order: &[usize], candidates: &[PaletteItem]) -> Vec<String> {
        order.iter().map(|&i| candidates[i].title.clone()).collect()
    }

    fn groups(order: &[usize], candidates: &[PaletteItem]) -> Vec<Group> {
        order.iter().map(|&i| candidates[i].group).collect()
    }

    /// One of each group, all sharing the word "deploy" so any text query hits them all.
    fn everything() -> Vec<PaletteItem> {
        vec![
            entry(Group::Command, ":go deploy"),
            entry(Group::Key, "deploy key"),
            entry(Group::Setting, "deploy setting"),
            entry(Group::Person, "deploy person"),
            entry(Group::Repo, "deploy/repo"),
            entry(Group::View, "deploy view"),
            entry(Group::GoTo, "deploy goto"),
            item(PaletteKind::Pr, "deploy item", "", 100),
            action("deploy action", 'd'),
        ]
    }

    #[test]
    fn empty_query_returns_all_most_recent_first() {
        let c = vec![
            item(PaletteKind::Pr, "old", "", 100),
            item(PaletteKind::Wi, "new", "", 300),
            item(PaletteKind::Pipe, "mid", "", 200),
        ];
        assert_eq!(titles(&rank("", &c), &c), vec!["new", "mid", "old"]);
        // Whitespace-only is treated as empty.
        assert_eq!(rank("   ", &c).len(), 3);
    }

    #[test]
    fn items_without_a_timestamp_sort_last() {
        let mut c = vec![
            item(PaletteKind::Pr, "dated", "", 100),
            item(PaletteKind::Pr, "undated", "", 0),
        ];
        c[1].sort_ts = None;
        assert_eq!(titles(&rank("", &c), &c), vec!["dated", "undated"]);
    }

    #[test]
    fn filters_out_non_matches() {
        let c = vec![
            item(PaletteKind::Pr, "Migrate billing", "", 100),
            item(PaletteKind::Pr, "Fix login redirect", "", 100),
        ];
        assert_eq!(titles(&rank("migrate", &c), &c), vec!["Migrate billing"]);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let c = vec![item(PaletteKind::Pr, "Migrate Billing", "", 100)];
        assert_eq!(rank("MIGRATE", &c).len(), 1);
        assert_eq!(rank("migrate", &c).len(), 1);
    }

    #[test]
    fn matches_on_subtitle_when_title_does_not() {
        let c = vec![
            item(PaletteKind::Wi, "Untitled task", "PAY-412 · Bug", 100),
            item(PaletteKind::Wi, "Other", "ENG-9 · Story", 100),
        ];
        assert_eq!(titles(&rank("pay-412", &c), &c), vec!["Untitled task"]);
    }

    #[test]
    fn stronger_match_ranks_higher() {
        // A contiguous substring match should beat a scattered subsequence match.
        let c = vec![
            item(PaletteKind::Pr, "b-a-r-b-a-z", "", 100), // scattered "bar"
            item(PaletteKind::Pr, "bar service", "", 100), // contiguous "bar"
        ];
        assert_eq!(titles(&rank("bar", &c), &c)[0], "bar service");
    }

    #[test]
    fn equal_scores_break_ties_by_recency() {
        // Identical text → identical scores → newer one first.
        let c = vec![
            item(PaletteKind::Pr, "deploy pipeline", "acme", 100),
            item(PaletteKind::Pr, "deploy pipeline", "acme", 500),
        ];
        let order = rank("deploy", &c);
        assert_eq!(c[order[0]].sort_ts, ts(500));
        assert_eq!(c[order[1]].sort_ts, ts(100));
    }

    #[test]
    fn each_prefix_keeps_only_its_group() {
        let c = everything();
        for (prefix, group) in [
            ('>', Group::Action),
            (':', Group::Command),
            ('@', Group::Person),
            ('#', Group::Item),
            ('?', Group::Key),
        ] {
            let with_text = rank(&format!("{prefix}deploy"), &c);
            assert_eq!(groups(&with_text, &c), vec![group], "{prefix}deploy");
            let bare = rank(&prefix.to_string(), &c);
            assert_eq!(
                groups(&bare, &c),
                vec![group],
                "bare {prefix} lists its whole group"
            );
        }
    }

    #[test]
    fn commands_are_hidden_without_the_colon() {
        let c = everything();
        let all = rank("deploy", &c);
        assert_eq!(all.len(), c.len() - 1, "every group but commands matches");
        assert!(!groups(&all, &c).contains(&Group::Command));
        assert!(!groups(&rank("", &c), &c).contains(&Group::Command));
        assert!(groups(&rank(":deploy", &c), &c).contains(&Group::Command));
    }

    #[test]
    fn empty_query_orders_actions_then_items_then_go_to() {
        let c = vec![
            entry(Group::GoTo, "Help"),
            item(PaletteKind::Pr, "older", "", 100),
            entry(Group::View, "a view"),
            action("Refresh", 'r'),
            item(PaletteKind::Wi, "newer", "", 200),
            action("Sort", 'S'),
        ];
        assert_eq!(
            titles(&rank("", &c), &c),
            vec!["Refresh", "Sort", "newer", "older", "Help"],
            "actions (in context order), items by recency, then go to — nothing else"
        );
    }

    #[test]
    fn results_are_grouped_with_groups_ordered_by_their_best_hit() {
        let c = vec![
            item(PaletteKind::Pr, "b-i-l-l-i-n-g", "", 100), // weak item hit
            entry(Group::View, "billing"),                   // the best hit overall
            item(PaletteKind::Pr, "bxixlxlxixnxg", "", 200), // another weak item hit
            entry(Group::View, "bill of lading"),            // weak view hit
        ];
        let order = rank("billing", &c);
        // Views lead (they hold the best hit) and each group is contiguous, even though the
        // candidates interleave and a weak view may score below an item.
        assert_eq!(
            groups(&order, &c),
            vec![Group::View, Group::View, Group::Item, Group::Item]
        );
        assert_eq!(titles(&order, &c)[0], "billing");
    }

    #[test]
    fn context_actions_are_boosted_over_equal_matches() {
        let c = vec![entry(Group::GoTo, "Merge"), action("Merge", 'm')];
        let order = rank("merge", &c);
        assert_eq!(groups(&order, &c), vec![Group::Action, Group::GoTo]);
    }

    #[test]
    fn complete_keeps_the_mode_prefix() {
        let a = action("Approve", 'a');
        assert_eq!(complete(">app", &a), ">Approve");
        assert_eq!(complete("app", &a), "Approve");
        let cmd = entry(Group::Command, ":theme matrix");
        assert_eq!(complete(":th", &cmd), ":theme matrix");
    }

    #[test]
    fn mode_titles_follow_the_prefix() {
        assert_eq!(mode_title(""), "Search everything");
        assert_eq!(mode_title("merge"), "Search everything");
        assert_eq!(mode_title(">"), "Actions");
        assert_eq!(mode_title(":go"), "Commands");
        assert_eq!(mode_title("@ada"), "People");
        assert_eq!(mode_title("#12"), "Items");
        assert_eq!(mode_title("?"), "Keys");
    }

    #[test]
    fn key_items_come_from_the_help_table() {
        let sections = vec![(
            "Global",
            vec![("Ctrl-K", "Command palette"), ("?", "This help")],
        )];
        let items = key_items(&sections);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Command palette");
        assert_eq!(items[0].subtitle, "Global");
        assert_eq!(items[0].key_hint.as_deref(), Some("Ctrl-K"));
        assert_eq!(
            items[1].target,
            PaletteTarget::HelpKey {
                keys: "?".into(),
                section: "Global".into()
            }
        );
    }

    #[test]
    fn go_to_respects_visible_sections_and_lists_help() {
        let items = goto_items(&[true, false, true]);
        let t: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert!(t.contains(&"Pull Requests") && t.contains(&"Pipelines"));
        assert!(!t.contains(&"Work Items"), "hidden section left out");
        assert!(t.contains(&"Help") && t.contains(&"Command Center"));
    }

    #[test]
    fn merge_and_state_commands_only_where_their_key_works() {
        let views: [Vec<SavedView>; 3] = Default::default();
        let mut ctx = CommandContext {
            themes: &["slate"],
            views: &views,
            visible: &[true; 3],
            can_merge: false,
            can_set_state: false,
        };
        let has = |items: &[PaletteItem], t: &str| items.iter().any(|i| i.title == t);
        let none = command_items(&ctx);
        assert!(has(&none, ":theme slate") && has(&none, ":go pipelines"));
        assert!(!has(&none, ":merge squash") && !has(&none, ":state"));
        ctx.can_merge = true;
        ctx.can_set_state = true;
        let all = command_items(&ctx);
        let squash = all.iter().find(|i| i.title == ":merge squash").unwrap();
        assert_eq!(squash.target, PaletteTarget::MergePicker { selected: 1 });
        assert!(has(&all, ":state"));
    }
}
