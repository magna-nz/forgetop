//! Modal overlays (confirm / picker / text-input) shown centred over the list.
//! When an overlay is open, the app routes every key to it via [`Overlay::handle`]
//! instead of the table, so there's no ambiguity between typing and navigation.

use forgetop_core::domain::ReviewVote;
use forgetop_core::provider::MergeStrategy;

use crate::app::Key;

/// A write action to run against the selected item once an overlay is submitted.
#[derive(Debug, Clone)]
pub enum Action {
    PrVote(ReviewVote),
    PrMerge(MergeStrategy),
    PrComment(String),
    WiSetState(String),
    WiComment(String),
    PipelineTrigger { connection_id: String, definition_id: String, branch: Option<String>, label: String },
    RemoveConnection { id: String, label: String },
    /// Result of a checklist: the ids that ended up ticked, tagged with what they are.
    ApplyToggle { kind: ToggleKind, ids: Vec<String> },
    /// Buffer an inline line comment (body); the target line is held on the PR view.
    AddLineComment(String),
    /// Submit the buffered line comments as a review with this verdict.
    SubmitReview(ReviewVote),
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
}

#[derive(Debug, Clone, Copy)]
pub enum InputKind {
    PrComment,
    WorkItemComment,
    /// The body of a pending inline line comment.
    PrLineComment,
}

pub enum Overlay {
    Confirm { title: String, message: String, action: Action },
    Picker { title: String, items: Vec<String>, selected: usize, kind: PickerKind },
    Input { title: String, buffer: String, kind: InputKind },
    Toggle { title: String, kind: ToggleKind, min_one: bool, items: Vec<ToggleItem>, selected: usize },
    /// A scrollable, context-agnostic reference of every keybinding.
    Help { scroll: u16 },
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
        }
    }

    /// Footer hint shown while this overlay is open.
    pub fn hint(&self) -> Vec<(&'static str, &'static str)> {
        match self {
            Overlay::Confirm { .. } => vec![("y", "confirm"), ("Esc", "cancel")],
            Overlay::Picker { .. } => vec![("↑↓", "choose"), ("↵", "select"), ("Esc", "cancel")],
            Overlay::Input { .. } => vec![("Esc", "cancel"), ("↵", "submit")],
            Overlay::Toggle { .. } => vec![("↑↓", "move"), ("↵/space", "toggle"), ("Esc", "apply")],
            Overlay::Help { .. } => vec![("↑↓", "scroll"), ("Esc", "close")],
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
            Overlay::Toggle { items, selected, min_one, kind, .. } => match key {
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
                Key::Char(' ') | Key::Enter => {
                    let on_count = items.iter().filter(|i| i.on).count();
                    if let Some(item) = items.get_mut(*selected) {
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
                Key::Escape => {
                    let ids = items.iter().filter(|i| i.on).map(|i| i.id.clone()).collect();
                    Outcome::Submit(Action::ApplyToggle { kind: kind.clone(), ids })
                }
                _ => Outcome::Keep,
            },
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
    }
}

fn resolve_input(kind: InputKind, text: String) -> Action {
    match kind {
        InputKind::PrComment => Action::PrComment(text),
        InputKind::WorkItemComment => Action::WiComment(text),
        InputKind::PrLineComment => Action::AddLineComment(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
