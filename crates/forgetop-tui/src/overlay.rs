//! Modal overlays (confirm / picker / text-input) shown centred over the list.
//! When an overlay is open, the app routes every key to it via [`Overlay::handle`]
//! instead of the table, so there's no ambiguity between typing and navigation.

use forgetop_core::domain::ReviewVote;
use forgetop_core::provider::{ItemRef, MergeStrategy};

use crate::app::Key;
use crate::palette::{complete, mode_title, rank, PaletteItem, PaletteKind, PaletteTarget};

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
    /// Assign the open work item to the user with this id (`None` = unassign). `label` is the
    /// name to show for it, so the change can be reflected before the provider answers.
    WiAssign { id: Option<String>, label: String },
    /// From the edit picker: start editing this field of the open work item.
    WiEdit(WiField),
    WiSetTitle(String),
    /// The description as it came back from `$EDITOR`.
    WiSetDescription(String),
    /// Confirmed: cancel this pipeline run. `run` carries its repository — see `PipelineTrigger`.
    PipelineCancel { connection_id: String, run: ItemRef, label: String },
    /// `repo` is the definition's **connection-relative** repository — a connection spanning
    /// several has no single "own" one to fall back on, so the target must be carried explicitly.
    PipelineTrigger { connection_id: String, repo: Option<String>, definition_id: String, branch: Option<String>, label: String },
    /// Confirmed: re-run a finished run — every job, or with `failed_only` just the failed ones.
    /// `repo` is the run's connection-relative repository, as for [`Action::PipelineTrigger`].
    /// `new_run` is set when the provider starts a separate run rather than re-queueing this one.
    PipelineRerun { connection_id: String, repo: Option<String>, run_id: String, failed_only: bool, new_run: bool, label: String },
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
    /// Run any other palette entry (an action's key, a destination, a view, a filter, a
    /// theme, a keybinding, a command). Interpreted by the app.
    Palette(PaletteTarget),
    /// From the unsubmitted-comments prompt: open the submit-review verdict picker.
    OpenReviewMenu,
    /// From the unsubmitted-comments prompt: leave the PR view, discarding pending comments.
    LeavePrView,
    /// First run: add the first connection here, in the terminal wizard.
    SetupInTerminal,
    /// First run: hand setup to the browser dashboard and wait for it to land.
    SetupInBrowser,
}

/// A work-item field the edit picker (`e`) can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WiField {
    Title,
    Description,
}

/// One row of a [`Overlay::Search`] picker: the id it submits (`None` for a "nobody" row such
/// as *Unassigned*) and what it shows.
#[derive(Debug, Clone)]
pub struct SearchItem {
    pub id: Option<String>,
    pub label: String,
}

/// What a [`Overlay::Search`] picker is choosing.
#[derive(Debug, Clone)]
pub enum SearchKind {
    /// The open work item's assignee. `me` indexes the signed-in user's row, when this
    /// connection can say who that is, so `@` assigns you without typing.
    Assignee { me: Option<usize> },
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
    /// Which field of the open work item to edit (Title / Description).
    WorkItemEdit,
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
    /// First run: set up the first connection in the terminal, or in the browser.
    SetupLocation,
}

#[derive(Debug, Clone, Copy)]
pub enum InputKind {
    PrComment,
    WorkItemComment,
    /// The open work item's new title (prefilled with the current one).
    WorkItemTitle,
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
    /// A searchable single-choice list: typing narrows it, Enter picks the highlighted row.
    /// `selected` indexes the *visible* rows (see [`visible_search_indices`]).
    Search { title: String, query: String, items: Vec<SearchItem>, selected: usize, kind: SearchKind },
    /// A scrollable, context-agnostic reference of every keybinding.
    Help { scroll: u16 },
    /// The command palette: "search everything" — the screen's actions, every already-fetched
    /// item, destinations, views, settings and keys. `results` are indices into `candidates`,
    /// ranked (and grouped) for the current `query`; `selected` indexes into `results`.
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
            | Overlay::Toggle { title, .. }
            | Overlay::Search { title, .. } => title,
            Overlay::Help { .. } => "Keybindings",
            Overlay::Palette { query, .. } => mode_title(query),
        }
    }

    /// Footer hint shown while this overlay is open.
    pub fn hint(&self) -> Vec<(&'static str, &'static str)> {
        match self {
            Overlay::Confirm { .. } => vec![("y", "confirm"), ("Esc", "cancel")],
            Overlay::Picker { .. } => vec![("↑↓", "choose"), ("↵", "select"), ("Esc", "cancel")],
            Overlay::Input { .. } => vec![("Esc", "cancel"), ("↵", "submit")],
            Overlay::Toggle { filter: Some(_), .. } => {
                vec![("type", "search"), ("↑↓", "move"), ("space", "toggle"), ("↵", "apply")]
            }
            Overlay::Toggle { .. } => vec![("↑↓", "move"), ("space", "toggle"), ("↵", "apply")],
            Overlay::Search { kind: SearchKind::Assignee { me }, .. } => {
                let mut keys = vec![("type", "search"), ("↑↓", "choose"), ("↵", "assign")];
                if me.is_some() {
                    keys.push(("@", "assign me"));
                }
                keys.push(("Esc", "cancel"));
                keys
            }
            Overlay::Help { .. } => vec![("↑↓", "scroll"), ("Esc", "close")],
            Overlay::Palette { .. } => {
                vec![("↑↓", "move"), ("↵", "run/open"), ("Tab", "complete"), ("^K", "close"), ("Esc", "cancel")]
            }
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
                    Key::Char(' ') => {
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
                    // Enter applies, matching every other overlay where Enter is the commit.
                    // Esc keeps applying too: ticks mutate in place as you go, so there is
                    // nothing to discard, and existing muscle memory still works.
                    Key::Enter | Key::Escape => {
                        let ids = items.iter().filter(|i| i.on).map(|i| i.id.clone()).collect();
                        Outcome::Submit(Action::ApplyToggle { kind: kind.clone(), ids })
                    }
                    _ => Outcome::Keep,
                }
            }
            Overlay::Search { query, items, selected, kind, .. } => {
                let visible = visible_search_indices(items, query);
                match key {
                    // `@` never appears in a display name, so it is free to mean "me".
                    Key::Char('@') => match kind {
                        SearchKind::Assignee { me: Some(i) } => match items.get(*i) {
                            Some(item) => Outcome::Submit(resolve_search(kind, item)),
                            None => Outcome::Keep,
                        },
                        SearchKind::Assignee { me: None } => Outcome::Keep,
                    },
                    Key::Char(c) => {
                        query.push(c);
                        *selected = 0;
                        Outcome::Keep
                    }
                    Key::Backspace => {
                        query.pop();
                        *selected = 0;
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
                    Key::Enter => match visible.get(*selected).and_then(|&i| items.get(i)) {
                        Some(item) => Outcome::Submit(resolve_search(kind, item)),
                        None => Outcome::Keep, // nothing matches — swallow Enter
                    },
                    Key::Escape => Outcome::Cancel,
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
                // Tab completes the query to the selected entry's text (keeping the mode prefix).
                Key::Tab => {
                    if let Some(item) = results.get(*selected).map(|&i| &candidates[i]) {
                        *query = complete(query, item);
                        *results = rank(query, candidates);
                        *selected = 0;
                    }
                    Outcome::Keep
                }
                Key::Enter => match results.get(*selected).map(|&i| &candidates[i].target) {
                    Some(PaletteTarget::Item { kind, id, connection_id }) => {
                        Outcome::Submit(Action::OpenItem { kind: *kind, id: id.clone(), connection_id: connection_id.clone() })
                    }
                    Some(target) => Outcome::Submit(Action::Palette(target.clone())),
                    None => Outcome::Keep, // no matches — swallow Enter
                },
                // Ctrl-K toggles: the key that opened the palette closes it again.
                Key::Escape | Key::Ctrl('k') => Outcome::Cancel,
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
        PickerKind::WorkItemEdit => match selected {
            1 => Action::WiEdit(WiField::Description),
            _ => Action::WiEdit(WiField::Title),
        },
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
        // The terminal is the default and sits first: forgetop is a TUI that happens to
        // serve a dashboard, and the first screen should say so.
        PickerKind::SetupLocation => match selected {
            1 => Action::SetupInBrowser,
            _ => Action::SetupInTerminal,
        },
    }
}

fn resolve_input(kind: InputKind, text: String) -> Action {
    match kind {
        InputKind::PrComment => Action::PrComment(text),
        InputKind::WorkItemComment => Action::WiComment(text),
        InputKind::WorkItemTitle => Action::WiSetTitle(text),
        InputKind::PrLineComment => Action::AddLineComment(text),
        InputKind::PrThreadReply => Action::PrReply(text),
        InputKind::SaveView => Action::SaveView(text),
    }
}

fn resolve_search(kind: &SearchKind, item: &SearchItem) -> Action {
    match kind {
        SearchKind::Assignee { .. } => Action::WiAssign { id: item.id.clone(), label: item.label.clone() },
    }
}

/// The rows a search picker currently shows: everything, or what matches its query.
/// `Overlay::Search::selected` indexes into this, not into `items`.
pub fn visible_search_indices(items: &[SearchItem], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| q.is_empty() || item.label.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
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

    fn assignees() -> Overlay {
        let item = |id: Option<&str>, label: &str| SearchItem { id: id.map(str::to_string), label: label.into() };
        Overlay::Search {
            title: "Assign".into(),
            query: String::new(),
            items: vec![item(None, "Unassigned"), item(Some("u1"), "Priya Nair"), item(Some("u9"), "Priyanka Shah"), item(Some("me"), "Sam Rivera")],
            selected: 0,
            kind: SearchKind::Assignee { me: Some(3) },
        }
    }

    #[test]
    fn the_assignee_picker_narrows_as_you_type_and_assigns_the_highlighted_row() {
        let mut o = assignees();
        for c in "pri".chars() {
            assert!(matches!(o.handle(Key::Char(c)), Outcome::Keep));
        }
        assert!(matches!(o.handle(Key::Down), Outcome::Keep));
        match o.handle(Key::Enter) {
            Outcome::Submit(Action::WiAssign { id, label }) => {
                assert_eq!(id.as_deref(), Some("u9"));
                assert_eq!(label, "Priyanka Shah");
            }
            _ => panic!("Enter should assign the second match"),
        }
    }

    #[test]
    fn at_assigns_you_and_the_unassigned_row_clears_the_assignee() {
        match assignees().handle(Key::Char('@')) {
            Outcome::Submit(Action::WiAssign { id, .. }) => assert_eq!(id.as_deref(), Some("me")),
            _ => panic!("@ should assign the signed-in user"),
        }
        match assignees().handle(Key::Enter) {
            Outcome::Submit(Action::WiAssign { id, label }) => {
                assert_eq!(id, None);
                assert_eq!(label, "Unassigned");
            }
            _ => panic!("the first row unassigns"),
        }
        // With no known identity, `@` does nothing rather than being typed into the query.
        let mut o = assignees();
        if let Overlay::Search { kind, .. } = &mut o {
            *kind = SearchKind::Assignee { me: None };
        }
        assert!(matches!(o.handle(Key::Char('@')), Outcome::Keep));
        assert!(matches!(&o, Overlay::Search { query, .. } if query.is_empty()));
        // A query that matches nothing swallows Enter instead of assigning someone unseen.
        for c in "zzz".chars() {
            o.handle(Key::Char(c));
        }
        assert!(matches!(o.handle(Key::Enter), Outcome::Keep));
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
        let target = PaletteTarget::Item { kind, id: id.into(), connection_id: "c".into() };
        PaletteItem::new(crate::palette::Group::Item, target, title)
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
    fn palette_ctrl_k_closes_and_ctrl_p_still_moves() {
        let mut o = palette(vec![pitem(PaletteKind::Pr, "1", "a"), pitem(PaletteKind::Pr, "2", "b")]);
        assert!(matches!(o.handle(Key::Ctrl('n')), Outcome::Keep));
        assert!(matches!(o.handle(Key::Ctrl('p')), Outcome::Keep), "Ctrl-P moves inside the palette");
        let Overlay::Palette { selected, .. } = &o else { panic!() };
        assert_eq!(*selected, 0);
        assert!(matches!(o.handle(Key::Ctrl('k')), Outcome::Cancel), "Ctrl-K closes");
    }

    #[test]
    fn palette_tab_completes_the_query_to_the_selected_entry() {
        use crate::palette::{Group, GoTo};
        let cmd = |t: &str| PaletteItem::new(Group::Command, PaletteTarget::GoTo(GoTo::Help), t);
        let mut o = palette(vec![cmd(":theme matrix"), cmd(":go help")]);
        for c in ":thm".chars() {
            o.handle(Key::Char(c));
        }
        assert!(matches!(o.handle(Key::Tab), Outcome::Keep));
        let Overlay::Palette { query, results, candidates, selected } = &o else { panic!() };
        assert_eq!(query, ":theme matrix");
        assert_eq!(candidates[results[*selected]].title, ":theme matrix", "the completed entry stays selected");
    }

    #[test]
    fn palette_enter_emits_the_selected_targets_action() {
        use crate::palette::{GoTo, Group};
        let targets = [
            (Group::Action, PaletteTarget::Key(Key::Char('a'))),
            (Group::GoTo, PaletteTarget::GoTo(GoTo::Inbox)),
            (Group::View, PaletteTarget::View { section: 1, idx: 2 }),
            (Group::Person, PaletteTarget::Filter { section: 0, text: "Ada".into() }),
            (Group::Repo, PaletteTarget::Filter { section: 2, text: "acme/pay".into() }),
            (Group::Setting, PaletteTarget::Theme("matrix".into())),
            (Group::Key, PaletteTarget::HelpKey { keys: "u".into(), section: "Work Item view".into() }),
            (Group::Command, PaletteTarget::MergePicker { selected: 1 }),
        ];
        for (group, target) in targets {
            // Typing reaches every group (an empty query shows only actions / items / go to);
            // commands only show in `:` mode.
            let mut o = palette(vec![PaletteItem::new(group, target.clone(), "entry")]);
            let query = if group == Group::Command { ":entry" } else { "entry" };
            for c in query.chars() {
                o.handle(Key::Char(c));
            }
            match o.handle(Key::Enter) {
                Outcome::Submit(Action::Palette(got)) => assert_eq!(got, target),
                _ => panic!("expected Action::Palette for {target:?}"),
            }
        }
        // Items keep their own open action.
        let mut o = palette(vec![pitem(PaletteKind::Wi, "w1", "a work item")]);
        assert!(matches!(o.handle(Key::Enter), Outcome::Submit(Action::OpenItem { kind: PaletteKind::Wi, .. })));
    }

    #[test]
    fn pending_exit_picker_maps_submit_and_leave() {
        assert!(matches!(resolve_picker(PickerKind::PendingExit, 0, &[]), Action::OpenReviewMenu));
        assert!(matches!(resolve_picker(PickerKind::PendingExit, 1, &[]), Action::LeavePrView));
    }

    #[test]
    fn setup_location_picker_defaults_to_the_terminal() {
        let items = vec!["Set up here in the terminal".to_string(), "Set up in the browser".to_string()];
        // Index 0 is the terminal, and anything unexpected falls back to it rather than
        // launching a browser the user did not ask for.
        assert!(matches!(resolve_picker(PickerKind::SetupLocation, 0, &items), Action::SetupInTerminal));
        assert!(matches!(resolve_picker(PickerKind::SetupLocation, 1, &items), Action::SetupInBrowser));
        assert!(matches!(resolve_picker(PickerKind::SetupLocation, 99, &items), Action::SetupInTerminal));
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
    fn enter_applies_and_does_not_tick_the_row() {
        let mut o = Overlay::Toggle {
            title: "".into(),
            kind: ToggleKind::Sections,
            min_one: false,
            items: vec![item("0", true), item("1", false)],
            selected: 1,
            filter: None,
        };
        // Enter used to toggle the selected row; it must now commit what is already ticked,
        // leaving item 1 off rather than turning it on as a side effect of applying.
        match o.handle(Key::Enter) {
            Outcome::Submit(Action::ApplyToggle { ids, .. }) => assert_eq!(ids, vec!["0".to_string()]),
            _ => panic!("Enter must apply"),
        }
    }

    #[test]
    fn space_still_ticks_and_esc_still_applies() {
        let mut o = Overlay::Toggle {
            title: "".into(),
            kind: ToggleKind::Sections,
            min_one: false,
            items: vec![item("0", true), item("1", false)],
            selected: 1,
            filter: None,
        };
        assert!(matches!(o.handle(Key::Char(' ')), Outcome::Keep), "space ticks without applying");
        // Esc keeps applying: ticks mutate in place, so there is nothing to discard.
        match o.handle(Key::Escape) {
            Outcome::Submit(Action::ApplyToggle { ids, .. }) => {
                assert_eq!(ids, vec!["0".to_string(), "1".to_string()])
            }
            _ => panic!("Esc must still apply"),
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
