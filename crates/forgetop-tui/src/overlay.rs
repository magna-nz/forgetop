//! Modal overlays (confirm / picker / text-input) shown centred over the list.
//! When an overlay is open, the app routes every key to it via [`Overlay::handle`]
//! instead of the table, so there's no ambiguity between typing and navigation.

use forgetop_core::config::StartupMode;
use forgetop_core::domain::ReviewVote;
use forgetop_core::provider::MergeStrategy;

use crate::app::Key;
use crate::palette::{rank, PaletteItem, PaletteKind};

/// A write action to run against the selected item once an overlay is submitted.
#[derive(Debug, Clone)]
pub enum Action {
    PrVote(ReviewVote),
    PrMerge(MergeStrategy),
    PrRevert,
    PrComment(String),
    /// Reply (body) to the thread stashed on the PR view's `reply_target`.
    PrReply(String),
    WiSetState(String),
    WiComment(String),
    /// `repo` is the definition's **connection-relative** repository — a connection spanning
    /// several has no single "own" one to fall back on, so the target must be carried explicitly.
    PipelineTrigger { connection_id: String, repo: Option<String>, definition_id: String, branch: Option<String>, label: String },
    RemoveConnection { id: String, label: String },
    /// Result of a checklist: the ids that ended up ticked, tagged with what they are.
    ApplyToggle { kind: ToggleKind, ids: Vec<String> },
    /// Buffer an inline line comment (body); the target line is held on the PR view.
    AddLineComment(String),
    /// Submit the buffered line comments as a review with this verdict.
    SubmitReview(ReviewVote),
    /// Sort a section by the chosen column index (resolved to a key by the app).
    SetSort { section: usize, index: usize },
    /// Save the current filter/sort/state as a new named view.
    SaveView(String),
    /// Delete the active section's current saved view.
    DeleteView,
    /// A pipeline-approval gate was picked (index into the app's choice list);
    /// opens a confirm before acting.
    PickApproval { index: usize },
    /// Confirmed: respond to the chosen pipeline-approval gate.
    RespondApproval { index: usize },
    /// Open the repository-scope picker for the connection at this index of the section's
    /// repo-addressed connections.
    OpenRepoScope { index: usize },
    /// Jump to an item chosen in the command palette. The app re-resolves the full
    /// PR / work item / pipeline from its lists by `(kind, id)` and opens its view.
    OpenItem { kind: PaletteKind, id: String, connection_id: String },
    /// From the unsubmitted-comments prompt: open the submit-review verdict picker.
    OpenReviewMenu,
    /// From the unsubmitted-comments prompt: leave the PR view, discarding pending comments.
    LeavePrView,
    /// Set the startup preference (what `forgetop` opens on launch).
    SetStartupMode(StartupMode),
}

/// What a [`Overlay::Toggle`] checklist is choosing.
#[derive(Debug, Clone)]
pub enum ToggleKind {
    /// Visible tab sections; item ids are section indices ("0"/"1"/"2").
    Sections,
    /// Pipeline definitions to subscribe a connection to; item ids are definition ids.
    PipelineSubs { connection_id: String },
    /// Work-item states to show; item ids are the state strings themselves.
    WorkItemStates,
    /// PR statuses to show; item ids are the status names ("Open"/"Merged"/…).
    PrStatuses,
    /// Which desktop notifications are enabled; item ids are event keys.
    Notifications,
    /// Which connections feed a section (0 = Pull Requests, 1 = Work Items);
    /// item ids are connection ids.
    SectionBind { section: usize },
    /// Which repositories a connection fetches from; item ids are **connection-relative**
    /// repository paths. Ticking none is a real choice — fetch nothing — so `min_one` is off.
    RepoScope { connection_id: String },
}

/// One row of a [`Overlay::Toggle`] checklist.
pub struct ToggleItem {
    pub id: String,
    pub label: String,
    pub on: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum PickerKind {
    PrMergeStrategy,
    WorkItemState,
    /// The verdict for submitting a batch of pending line comments.
    ReviewSubmit,
    /// Choose the sort column for a section (0=PR, 1=WI, 2=Pipelines).
    SortColumn { section: usize },
    /// Choose a pipeline-approval gate + decision; resolves to the picked index.
    ApprovalGate,
    /// Choose which bound connection's repository scope to edit, when a section has more than
    /// one repo-addressed connection. Resolves to the picked index.
    RepoScopeConnection,
    /// Shown on Esc when line comments are buffered but unsubmitted: submit or leave.
    PendingExit,
    /// Choose what `forgetop` opens on launch (a shared preference).
    StartupMode,
}

#[derive(Debug, Clone, Copy)]
pub enum InputKind {
    PrComment,
    WorkItemComment,
    /// The body of a pending inline line comment.
    PrLineComment,
    /// A reply to the thread stashed on the PR view (`reply_target`).
    PrThreadReply,
    /// The name for a new saved view.
    SaveView,
}

pub enum Overlay {
    Confirm { title: String, message: String, action: Action },
    Picker { title: String, items: Vec<String>, selected: usize, kind: PickerKind },
    Input { title: String, buffer: String, kind: InputKind },
    /// A checklist. `filter` makes it searchable: `Some` means typing narrows the list (and
    /// `selected` indexes the *visible* rows), `None` keeps the plain j/k behaviour. A scope
    /// picker over a few hundred repositories needs the search; a three-row list doesn't.
    Toggle { title: String, kind: ToggleKind, min_one: bool, items: Vec<ToggleItem>, selected: usize, filter: Option<String> },
    /// A scrollable, context-agnostic reference of every keybinding.
    Help { scroll: u16 },
    /// The command palette: fuzzy-jump across every already-fetched item. `results` are
    /// indices into `candidates`, ranked for the current `query`; `selected` indexes into
    /// `results`.
    Palette { query: String, candidates: Vec<PaletteItem>, results: Vec<usize>, selected: usize },
}

/// What the app should do after feeding a key to the overlay.
pub enum Outcome {
    /// Keep the overlay open (state may have changed).
    Keep,
    /// Close the overlay without acting.
    Cancel,
    /// Close the overlay and run this action.
    Submit(Action),
}

impl Overlay {
    pub fn title(&self) -> &str {
        match self {
            Overlay::Confirm { title, .. }
            | Overlay::Picker { title, .. }
            | Overlay::Input { title, .. }
            | Overlay::Toggle { title, .. } => title,
            Overlay::Help { .. } => "Keybindings",
            Overlay::Palette { .. } => "Jump to",
        }
    }

    /// Footer hint shown while this overlay is open.
    pub fn hint(&self) -> Vec<(&'static str, &'static str)> {
        match self {
            Overlay::Confirm { .. } => vec![("y", "confirm"), ("Esc", "cancel")],
            Overlay::Picker { .. } => vec![("↑↓", "choose"), ("↵", "select"), ("Esc", "cancel")],
            Overlay::Input { .. } => vec![("Esc", "cancel"), ("↵", "submit")],
            Overlay::Toggle { filter: Some(_), .. } => {
                vec![("type", "search"), ("↑↓", "move"), ("↵/space", "toggle"), ("Esc", "apply")]
            }
            Overlay::Toggle { .. } => vec![("↑↓", "move"), ("↵/space", "toggle"), ("Esc", "apply")],
            Overlay::Help { .. } => vec![("↑↓", "scroll"), ("Esc", "close")],
            Overlay::Palette { .. } => vec![("↑↓", "move"), ("↵", "open"), ("Esc", "cancel")],
        }
    }

    pub fn handle(&mut self, key: Key) -> Outcome {
        match self {
            Overlay::Confirm { action, .. } => match key {
                Key::Enter | Key::Char('y') | Key::Char('Y') => Outcome::Submit(action.clone()),
                Key::Escape | Key::Char('n') | Key::Char('N') => Outcome::Cancel,
                _ => Outcome::Keep,
            },
            Overlay::Picker { items, selected, kind, .. } => match key {
                Key::Up | Key::Char('k') => {
                    if !items.is_empty() {
                        *selected = (*selected + items.len() - 1) % items.len();
                    }
                    Outcome::Keep
                }
                Key::Down | Key::Char('j') => {
                    if !items.is_empty() {
                        *selected = (*selected + 1) % items.len();
                    }
                    Outcome::Keep
                }
                Key::Enter => Outcome::Submit(resolve_picker(*kind, *selected, items)),
                Key::Escape => Outcome::Cancel,
                _ => Outcome::Keep,
            },
            Overlay::Input { buffer, kind, .. } => match key {
                Key::Char(c) => {
                    buffer.push(c);
                    Outcome::Keep
                }
                Key::Backspace => {
                    buffer.pop();
                    Outcome::Keep
                }
                Key::Enter => Outcome::Submit(resolve_input(*kind, buffer.clone())),
                Key::Escape => Outcome::Cancel,
                _ => Outcome::Keep,
            },
            Overlay::Toggle { items, selected, min_one, kind, filter, .. } => {
                let visible = visible_toggle_indices(items, filter.as_deref());
                match key {
                    // A searchable toggle spends letters on the query, so only the arrows move.
                    Key::Up | Key::Char('k') if filter.is_none() => {
                        if !visible.is_empty() {
                            *selected = (*selected + visible.len() - 1) % visible.len();
                        }
                        Outcome::Keep
                    }
                    Key::Down | Key::Char('j') if filter.is_none() => {
                        if !visible.is_empty() {
                            *selected = (*selected + 1) % visible.len();
                        }
                        Outcome::Keep
                    }
                    Key::Up => {
                        if !visible.is_empty() {
                            *selected = (*selected + visible.len() - 1) % visible.len();
                        }
                        Outcome::Keep
                    }
                    Key::Down => {
                        if !visible.is_empty() {
                            *selected = (*selected + 1) % visible.len();
                        }
                        Outcome::Keep
                    }
                    Key::Char(' ') | Key::Enter => {
                        let on_count = items.iter().filter(|i| i.on).count();
                        if let Some(item) = visible.get(*selected).and_then(|&i| items.get_mut(i)) {
                            if item.on {
                                // Optionally keep at least one ticked (used for visible tabs).
                                if !*min_one || on_count > 1 {
                                    item.on = false;
                                }
                            } else {
                                item.on = true;
                            }
                        }
                        Outcome::Keep
                    }
                    Key::Char(c) if filter.is_some() => {
                        if let Some(q) = filter.as_mut() {
                            q.push(c);
                        }
                        *selected = 0;
                        Outcome::Keep
                    }
                    Key::Backspace if filter.is_some() => {
                        if let Some(q) = filter.as_mut() {
                            q.pop();
                        }
                        *selected = 0;
                        Outcome::Keep
                    }
                    Key::Escape => {
                        let ids = items.iter().filter(|i| i.on).map(|i| i.id.clone()).collect();
                        Outcome::Submit(Action::ApplyToggle { kind: kind.clone(), ids })
                    }
                    _ => Outcome::Keep,
                }
            }
            Overlay::Help { scroll } => match key {
                Key::Up | Key::Char('k') => {
                    *scroll = scroll.saturating_sub(1);
                    Outcome::Keep
                }
                Key::Down | Key::Char('j') => {
                    *scroll = scroll.saturating_add(1);
                    Outcome::Keep
                }
                Key::PageUp => {
                    *scroll = scroll.saturating_sub(10);
                    Outcome::Keep
                }
                Key::PageDown => {
                    *scroll = scroll.saturating_add(10);
                    Outcome::Keep
                }
                Key::Escape | Key::Char('?') | Key::Char('q') => Outcome::Cancel,
                _ => Outcome::Keep,
            },
            Overlay::Palette { query, candidates, results, selected } => match key {
                // Typing edits the query and re-ranks; selection resets to the top match.
                Key::Char(c) => {
                    query.push(c);
                    *results = rank(query, candidates);
                    *selected = 0;
                    Outcome::Keep
                }
                Key::Backspace => {
                    query.pop();
                    *results = rank(query, candidates);
                    *selected = 0;
                    Outcome::Keep
                }
                Key::Down | Key::Ctrl('n') => {
                    if !results.is_empty() {
                        *selected = (*selected + 1) % results.len();
                    }
                    Outcome::Keep
                }
                Key::Up | Key::Ctrl('p') => {
                    if !results.is_empty() {
                        *selected = (*selected + results.len() - 1) % results.len();
                    }
                    Outcome::Keep
                }
                Key::Enter => match results.get(*selected).map(|&i| &candidates[i]) {
                    Some(item) => Outcome::Submit(Action::OpenItem {
                        kind: item.kind,
                        id: item.id.clone(),
                        connection_id: item.connection_id.clone(),
                    }),
                    None => Outcome::Keep, // no matches — swallow Enter
                },
                Key::Escape => Outcome::Cancel,
                _ => Outcome::Keep,
            },
        }
    }
}

fn resolve_picker(kind: PickerKind, selected: usize, items: &[String]) -> Action {
    match kind {
        PickerKind::PrMergeStrategy => {
            let strategy = match selected {
                1 => MergeStrategy::Squash,
                2 => MergeStrategy::Rebase,
                _ => MergeStrategy::Merge,
            };
            Action::PrMerge(strategy)
        }
        PickerKind::WorkItemState => Action::WiSetState(items.get(selected).cloned().unwrap_or_default()),
        PickerKind::ReviewSubmit => {
            let event = match selected {
                1 => ReviewVote::Approved,
                2 => ReviewVote::Rejected,
                _ => ReviewVote::NoVote,
            };
            Action::SubmitReview(event)
        }
        PickerKind::SortColumn { section } => Action::SetSort { section, index: selected },
        PickerKind::ApprovalGate => Action::PickApproval { index: selected },
        PickerKind::RepoScopeConnection => Action::OpenRepoScope { index: selected },
        PickerKind::PendingExit => match selected {
            0 => Action::OpenReviewMenu,
            _ => Action::LeavePrView,
        },
        PickerKind::StartupMode => {
            let mode = match selected {
                1 => StartupMode::TerminalOnly,
                2 => StartupMode::DashboardOnly,
                _ => StartupMode::Both,
            };
            Action::SetStartupMode(mode)
        }
    }
}

fn resolve_input(kind: InputKind, text: String) -> Action {
    match kind {
        InputKind::PrComment => Action::PrComment(text),
        InputKind::WorkItemComment => Action::WiComment(text),
        InputKind::PrLineComment => Action::AddLineComment(text),
        InputKind::PrThreadReply => Action::PrReply(text),
        InputKind::SaveView => Action::SaveView(text),
    }
}

/// The rows a checklist currently shows: everything, or what matches its search query.
/// `Overlay::Toggle::selected` indexes into this, not into `items`.
pub fn visible_toggle_indices(items: &[ToggleItem], filter: Option<&str>) -> Vec<usize> {
    let q = filter.unwrap_or("").trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| q.is_empty() || item.label.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repos(names: &[&str], on: &[&str]) -> Vec<ToggleItem> {
        names
            .iter()
            .map(|n| ToggleItem { id: (*n).into(), label: (*n).into(), on: on.contains(n) })
            .collect()
    }

    #[test]
    fn a_searchable_checklist_narrows_and_toggles_the_visible_row() {
        // A scope picker over hundreds of repositories needs search; letters go to the query, so
        // only the arrows move — j/k would otherwise be untypeable.
        let mut o = Overlay::Toggle {
            title: "Repositories".into(),
            kind: ToggleKind::RepoScope { connection_id: "c".into() },
            min_one: false,
            items: repos(&["acme/pay", "acme/ledger", "other/web"], &["acme/pay"]),
            selected: 0,
            filter: Some(String::new()),
        };
        o.handle(Key::Char('l'));
        o.handle(Key::Char('e'));
        let Overlay::Toggle { items, filter, .. } = &o else { panic!("toggle") };
        assert_eq!(filter.as_deref(), Some("le"));
        assert_eq!(visible_toggle_indices(items, filter.as_deref()), vec![1], "only acme/ledger matches");

        // Space ticks the visible row, not items[selected] — they are different lists.
        o.handle(Key::Char(' '));
        let Overlay::Toggle { items, .. } = &o else { panic!("toggle") };
        assert!(items[1].on, "the matched repository was ticked");
        assert!(items[0].on && !items[2].on, "the others are untouched");
    }

    #[test]
    fn choosing_which_connection_to_scope_resolves_to_that_connection() {
        // A section can have more than one repo-addressed connection bound, and the scope is per
        // connection — so picking the first silently would leave the others unreachable.
        let mut o = Overlay::Picker {
            title: "Repositories · which connection?".into(),
            items: vec!["GitHub".into(), "GitLab".into()],
            selected: 0,
            kind: PickerKind::RepoScopeConnection,
        };
        o.handle(Key::Down);
        match o.handle(Key::Enter) {
            Outcome::Submit(Action::OpenRepoScope { index }) => assert_eq!(index, 1, "the second connection was chosen"),
            _ => panic!("expected the chosen connection to be opened"),
        }
    }

    #[test]
    fn an_emptied_scope_submits_an_empty_list_rather_than_being_prevented() {
        // Choosing no repositories is a real state — fetch nothing — so `min_one` is off and the
        // apply carries an empty set rather than silently keeping the last one ticked.
        let mut o = Overlay::Toggle {
            title: "Repositories".into(),
            kind: ToggleKind::RepoScope { connection_id: "c".into() },
            min_one: false,
            items: repos(&["acme/pay"], &["acme/pay"]),
            selected: 0,
            filter: Some(String::new()),
        };
        o.handle(Key::Char(' '));
        match o.handle(Key::Escape) {
            Outcome::Submit(Action::ApplyToggle { kind: ToggleKind::RepoScope { connection_id }, ids }) => {
                assert_eq!(connection_id, "c");
                assert!(ids.is_empty(), "an emptied scope applies as an empty list");
            }
            _ => panic!("expected the scope to be applied"),
        }
    }

    fn pitem(kind: PaletteKind, id: &str, title: &str) -> PaletteItem {
        PaletteItem {
            kind,
            id: id.into(),
            connection_id: "c".into(),
            title: title.into(),
            subtitle: String::new(),
            tone: crate::palette::Tone::Neutral,
            sort_ts: None,
        }
    }

    fn palette(items: Vec<PaletteItem>) -> Overlay {
        let results = rank("", &items);
        Overlay::Palette { query: String::new(), candidates: items, results, selected: 0 }
    }

    #[test]
    fn palette_typing_filters_and_resets_selection() {
        let mut o = palette(vec![pitem(PaletteKind::Pr, "1", "Migrate billing"), pitem(PaletteKind::Wi, "2", "Fix login")]);
        assert!(matches!(o.handle(Key::Down), Outcome::Keep)); // move off row 0
        assert!(matches!(o.handle(Key::Char('m')), Outcome::Keep)); // "m" → only "Migrate billing"
        let Overlay::Palette { results, selected, .. } = &o else { panic!() };
        assert_eq!(results.len(), 1);
        assert_eq!(*selected, 0, "selection resets to the top match after typing");
    }

    #[test]
    fn palette_enter_submits_open_for_selected() {
        let mut o = palette(vec![pitem(PaletteKind::Pr, "pr1", "alpha")]);
        match o.handle(Key::Enter) {
            Outcome::Submit(Action::OpenItem { kind, id, .. }) => {
                assert_eq!(kind, PaletteKind::Pr);
                assert_eq!(id, "pr1");
            }
            _ => panic!("expected an OpenItem submit"),
        }
    }

    #[test]
    fn palette_enter_with_no_matches_is_swallowed() {
        let mut o = palette(vec![pitem(PaletteKind::Pr, "1", "alpha")]);
        for c in "zzzz".chars() {
            o.handle(Key::Char(c));
        }
        assert!(matches!(o.handle(Key::Enter), Outcome::Keep));
    }

    #[test]
    fn palette_down_wraps_selection() {
        let mut o = palette(vec![pitem(PaletteKind::Pr, "1", "a"), pitem(PaletteKind::Pr, "2", "b")]);
        o.handle(Key::Down);
        o.handle(Key::Down);
        let Overlay::Palette { selected, .. } = &o else { panic!() };
        assert_eq!(*selected, 0, "past the last result wraps to the first");
    }

    #[test]
    fn palette_esc_cancels() {
        let mut o = palette(vec![pitem(PaletteKind::Pr, "1", "a")]);
        assert!(matches!(o.handle(Key::Escape), Outcome::Cancel));
    }

    #[test]
    fn pending_exit_picker_maps_submit_and_leave() {
        assert!(matches!(resolve_picker(PickerKind::PendingExit, 0, &[]), Action::OpenReviewMenu));
        assert!(matches!(resolve_picker(PickerKind::PendingExit, 1, &[]), Action::LeavePrView));
    }

    #[test]
    fn startup_picker_maps_each_mode() {
        use forgetop_core::config::StartupMode::*;
        assert!(matches!(resolve_picker(PickerKind::StartupMode, 0, &[]), Action::SetStartupMode(Both)));
        assert!(matches!(resolve_picker(PickerKind::StartupMode, 1, &[]), Action::SetStartupMode(TerminalOnly)));
        assert!(matches!(resolve_picker(PickerKind::StartupMode, 2, &[]), Action::SetStartupMode(DashboardOnly)));
    }

    #[test]
    fn input_accumulates_text_and_submits_comment() {
        let mut o = Overlay::Input { title: "t".into(), buffer: String::new(), kind: InputKind::PrComment };
        for c in "lgtm".chars() {
            assert!(matches!(o.handle(Key::Char(c)), Outcome::Keep));
        }
        o.handle(Key::Char('!'));
        assert!(matches!(o.handle(Key::Backspace), Outcome::Keep));
        match o.handle(Key::Enter) {
            Outcome::Submit(Action::PrComment(text)) => assert_eq!(text, "lgtm"),
            _ => panic!("expected a PrComment submit"),
        }
    }

    #[test]
    fn picker_moves_and_resolves_merge_strategy() {
        let mut o = Overlay::Picker {
            title: "m".into(),
            items: vec!["Merge commit".into(), "Squash".into(), "Rebase".into()],
            selected: 0,
            kind: PickerKind::PrMergeStrategy,
        };
        o.handle(Key::Down); // -> Squash
        match o.handle(Key::Enter) {
            Outcome::Submit(Action::PrMerge(MergeStrategy::Squash)) => {}
            _ => panic!("expected PrMerge(Squash)"),
        }
    }

    #[test]
    fn work_item_picker_resolves_to_selected_state_name() {
        let mut o = Overlay::Picker {
            title: "s".into(),
            items: vec!["Todo".into(), "In Progress".into(), "Done".into()],
            selected: 0,
            kind: PickerKind::WorkItemState,
        };
        o.handle(Key::Down); // -> In Progress
        match o.handle(Key::Enter) {
            Outcome::Submit(Action::WiSetState(state)) => assert_eq!(state, "In Progress"),
            _ => panic!("expected WiSetState"),
        }
    }

    #[test]
    fn work_item_input_resolves_to_comment() {
        let mut o = Overlay::Input { title: "c".into(), buffer: "needs tests".into(), kind: InputKind::WorkItemComment };
        match o.handle(Key::Enter) {
            Outcome::Submit(Action::WiComment(text)) => assert_eq!(text, "needs tests"),
            _ => panic!("expected WiComment"),
        }
    }

    fn item(id: &str, on: bool) -> ToggleItem {
        ToggleItem { id: id.into(), label: id.into(), on }
    }

    #[test]
    fn toggle_submits_ticked_ids() {
        let mut o = Overlay::Toggle {
            title: "".into(),
            kind: ToggleKind::Sections,
            min_one: true,
            items: vec![item("0", true), item("1", true), item("2", true)],
            selected: 1,
            filter: None,
        };
        o.handle(Key::Char(' ')); // turn item 1 off
        match o.handle(Key::Escape) {
            Outcome::Submit(Action::ApplyToggle { ids, .. }) => assert_eq!(ids, vec!["0".to_string(), "2".to_string()]),
            _ => panic!("expected ApplyToggle"),
        }
    }

    #[test]
    fn toggle_min_one_refuses_to_clear_the_last() {
        let mut o = Overlay::Toggle {
            title: "".into(),
            kind: ToggleKind::Sections,
            min_one: true,
            items: vec![item("0", true)],
            selected: 0,
            filter: None,
        };
        o.handle(Key::Char(' ')); // would clear the only one — ignored
        match o.handle(Key::Escape) {
            Outcome::Submit(Action::ApplyToggle { ids, .. }) => assert_eq!(ids, vec!["0".to_string()]),
            _ => panic!("expected ApplyToggle"),
        }
    }

    #[test]
    fn toggle_without_min_one_allows_empty() {
        let mut o = Overlay::Toggle {
            title: "".into(),
            kind: ToggleKind::PipelineSubs { connection_id: "c".into() },
            min_one: false,
            items: vec![item("ci", true)],
            selected: 0,
            filter: None,
        };
        o.handle(Key::Char(' ')); // clear it — allowed for pipelines
        match o.handle(Key::Escape) {
            Outcome::Submit(Action::ApplyToggle { ids, .. }) => assert!(ids.is_empty()),
            _ => panic!("expected ApplyToggle"),
        }
    }

    #[test]
    fn confirm_yes_submits_and_esc_cancels() {
        let mut yes = Overlay::Confirm { title: "t".into(), message: "m".into(), action: Action::PrVote(ReviewVote::Approved) };
        assert!(matches!(yes.handle(Key::Char('y')), Outcome::Submit(Action::PrVote(ReviewVote::Approved))));

        let mut no = Overlay::Confirm { title: "t".into(), message: "m".into(), action: Action::PrVote(ReviewVote::Approved) };
        assert!(matches!(no.handle(Key::Escape), Outcome::Cancel));
    }
}
