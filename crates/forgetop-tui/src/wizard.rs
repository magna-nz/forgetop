//! Add-connection wizard: a small state machine that drives a *sequence* of the
//! same modal prompt kinds used elsewhere (pick / text / secret). Steps after the
//! provider pick are enqueued dynamically, since each provider needs different fields.

use std::collections::VecDeque;

use forgetop_core::domain::{ProviderType, Section};

use crate::app::Key;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Provider,
    DisplayName,
    BaseUrl,
    Organization,
    Project,
    Repository,
    Username,
    Pat,
    Bind,
}

pub enum PromptKind {
    Text { buffer: String, secret: bool },
    Pick { items: Vec<String>, selected: usize },
    /// A checklist: space ticks a row, Enter moves on. Used for section binding, where a
    /// connection usually populates more than one section and picking exactly one was the
    /// wrong shape.
    Multi { items: Vec<String>, on: Vec<bool>, selected: usize },
}

pub struct Prompt {
    pub field: Field,
    pub label: String,
    /// One-line guidance shown under the field (where to find it, expected format).
    pub help: String,
    pub required: bool,
    pub kind: PromptKind,
}

impl Prompt {
    fn text(field: Field, label: &str, help: &str, required: bool, prefill: &str) -> Self {
        Prompt { field, label: label.into(), help: help.into(), required, kind: PromptKind::Text { buffer: prefill.into(), secret: false } }
    }
    fn secret(field: Field, label: &str, help: &str) -> Self {
        Prompt { field, label: label.into(), help: help.into(), required: true, kind: PromptKind::Text { buffer: String::new(), secret: true } }
    }
    fn pick(field: Field, label: &str, help: &str, items: Vec<String>, selected: usize) -> Self {
        Prompt { field, label: label.into(), help: help.into(), required: true, kind: PromptKind::Pick { items, selected } }
    }
    /// Every row starts ticked: a connection is normally wanted for everything it supports,
    /// and unticking is the exception.
    fn multi(field: Field, label: &str, help: &str, items: Vec<String>) -> Self {
        let on = vec![true; items.len()];
        Prompt { field, label: label.into(), help: help.into(), required: false, kind: PromptKind::Multi { items, on, selected: 0 } }
    }
}

/// The connection being assembled. Empty text fields become `None`.
#[derive(Default)]
pub struct Draft {
    pub provider: Option<ProviderType>,
    pub display_name: String,
    pub base_url: Option<String>,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub repository: Option<String>,
    pub username: Option<String>,
    pub pat: Option<String>,
    pub bind_sections: Vec<Section>,
}

pub struct Wizard {
    pub queue: VecDeque<Prompt>,
    pub draft: Draft,
    pub done: usize,
}

pub enum WizardOutcome {
    Keep,
    Cancel,
    Commit,
}

impl Default for Wizard {
    fn default() -> Self {
        Self::new()
    }
}

/// Human label for a provider in the picker. `as_str` is the serialised form, which
/// renders Azure DevOps without its space.
fn provider_label(p: ProviderType) -> &'static str {
    match p {
        ProviderType::AzureDevOps => "Azure DevOps",
        other => other.as_str(),
    }
}

impl Wizard {
    pub fn new() -> Self {
        // One source of truth with the dashboard. The hand-written list this replaced had
        // drifted: it offered `Demo` as a real choice and ordered the rest differently, so
        // the two setup paths disagreed about what you could connect to.
        let providers = forgetop_core::setup::selectable_providers().into_iter().map(provider_label).map(String::from).collect();
        let mut queue = VecDeque::new();
        queue.push_back(Prompt::pick(Field::Provider, "Provider", "Which platform this connection talks to", providers, 0));
        Wizard { queue, draft: Draft::default(), done: 0 }
    }

    pub fn current(&self) -> Option<&Prompt> {
        self.queue.front()
    }

    /// "Step N of M" — M grows once the provider-specific steps are enqueued.
    pub fn step_label(&self) -> String {
        format!("Step {} of {}", self.done + 1, self.done + self.queue.len())
    }

    pub fn handle(&mut self, key: Key) -> WizardOutcome {
        let Some(prompt) = self.queue.front_mut() else { return WizardOutcome::Commit };
        match &mut prompt.kind {
            PromptKind::Text { buffer, .. } => match key {
                Key::Char(c) => {
                    buffer.push(c);
                    WizardOutcome::Keep
                }
                Key::Backspace => {
                    buffer.pop();
                    WizardOutcome::Keep
                }
                Key::Enter => self.advance(),
                Key::Escape => WizardOutcome::Cancel,
                _ => WizardOutcome::Keep,
            },
            PromptKind::Multi { items, on, selected } => match key {
                Key::Up | Key::Char('k') => {
                    if !items.is_empty() {
                        *selected = (*selected + items.len() - 1) % items.len();
                    }
                    WizardOutcome::Keep
                }
                Key::Down | Key::Char('j') => {
                    if !items.is_empty() {
                        *selected = (*selected + 1) % items.len();
                    }
                    WizardOutcome::Keep
                }
                Key::Char(' ') => {
                    if let Some(flag) = on.get_mut(*selected) {
                        *flag = !*flag;
                    }
                    WizardOutcome::Keep
                }
                Key::Enter => self.advance(),
                Key::Escape => WizardOutcome::Cancel,
                _ => WizardOutcome::Keep,
            },
            PromptKind::Pick { items, selected } => match key {
                Key::Up | Key::Char('k') => {
                    if !items.is_empty() {
                        *selected = (*selected + items.len() - 1) % items.len();
                    }
                    WizardOutcome::Keep
                }
                Key::Down | Key::Char('j') => {
                    if !items.is_empty() {
                        *selected = (*selected + 1) % items.len();
                    }
                    WizardOutcome::Keep
                }
                Key::Enter => self.advance(),
                Key::Escape => WizardOutcome::Cancel,
                _ => WizardOutcome::Keep,
            },
        }
    }

    fn advance(&mut self) -> WizardOutcome {
        // Required text fields must be non-empty to move on.
        if let Some(Prompt { required: true, kind: PromptKind::Text { buffer, .. }, .. }) = self.queue.front() {
            if buffer.trim().is_empty() {
                return WizardOutcome::Keep;
            }
        }
        let prompt = self.queue.pop_front().expect("advance with a current prompt");
        self.store(&prompt);
        if prompt.field == Field::Provider {
            self.enqueue_provider_steps();
        }
        self.done += 1;
        if self.queue.is_empty() {
            WizardOutcome::Commit
        } else {
            WizardOutcome::Keep
        }
    }

    fn store(&mut self, prompt: &Prompt) {
        match (&prompt.field, &prompt.kind) {
            (Field::Provider, PromptKind::Pick { selected, .. }) => {
                // Index straight back into the same list the prompt was built from, so the
                // labels and the resulting ProviderType cannot drift apart again.
                let providers = forgetop_core::setup::selectable_providers();
                self.draft.provider = providers.get(*selected).copied().or_else(|| providers.first().copied());
            }
            (Field::DisplayName, PromptKind::Text { buffer, .. }) => self.draft.display_name = buffer.trim().to_string(),
            (Field::BaseUrl, PromptKind::Text { buffer, .. }) => self.draft.base_url = non_empty(buffer),
            (Field::Username, PromptKind::Text { buffer, .. }) => self.draft.username = non_empty(buffer),
            (Field::Organization, PromptKind::Text { buffer, .. }) => self.draft.organization = non_empty(buffer),
            (Field::Project, PromptKind::Text { buffer, .. }) => self.draft.project = non_empty(buffer),
            (Field::Repository, PromptKind::Text { buffer, .. }) => self.draft.repository = non_empty(buffer),
            (Field::Pat, PromptKind::Text { buffer, .. }) => self.draft.pat = non_empty(buffer),
            (Field::Bind, PromptKind::Multi { items, on, .. }) => {
                self.draft.bind_sections = items
                    .iter()
                    .zip(on.iter())
                    .filter(|(_, ticked)| **ticked)
                    .filter_map(|(label, _)| section_from_label(label))
                    .collect();
            }
            _ => {}
        }
    }

    fn enqueue_provider_steps(&mut self) {
        // Fields come from the shared schema in forgetop-core, so the wizard and the web
        // dashboard's settings form always ask for the same things.
        let provider = self.draft.provider.unwrap_or(ProviderType::Demo);
        for spec in forgetop_core::setup::connection_fields(provider) {
            let field = field_from_key(spec.key);
            if spec.secret {
                self.queue.push_back(Prompt::secret(field, &spec.label, &spec.help));
            } else {
                self.queue.push_back(Prompt::text(field, &spec.label, &spec.help, spec.required, spec.default.as_deref().unwrap_or("")));
            }
        }
        let items: Vec<String> = provider_sections(provider).into_iter().map(|s| section_label(s).to_string()).collect();
        self.queue.push_back(Prompt::multi(
            Field::Bind,
            "Sections to populate",
            "Space toggles a section · nothing ticked skips binding for now",
            items,
        ));
    }
}

fn field_from_key(key: forgetop_core::setup::FieldKey) -> Field {
    use forgetop_core::setup::FieldKey as K;
    match key {
        K::DisplayName => Field::DisplayName,
        K::BaseUrl => Field::BaseUrl,
        K::Organization => Field::Organization,
        K::Project => Field::Project,
        K::Repository => Field::Repository,
        K::Username => Field::Username,
        K::Pat => Field::Pat,
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

pub use forgetop_core::setup::provider_sections;

pub fn section_label(section: Section) -> &'static str {
    match section {
        Section::PullRequests => "Pull Requests",
        Section::WorkItems => "Work Items",
        Section::Pipelines => "Pipelines",
    }
}

fn section_from_label(label: &str) -> Option<Section> {
    match label {
        "Pull Requests" => Some(Section::PullRequests),
        "Work Items" => Some(Section::WorkItems),
        "Pipelines" => Some(Section::Pipelines),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typ(w: &mut Wizard, s: &str) {
        for c in s.chars() {
            w.handle(Key::Char(c));
        }
    }

    #[test]
    fn builds_a_github_connection_and_binds() {
        let mut w = Wizard::new();
        // Provider pick defaults to GitHub (index 1) — accept.
        assert!(matches!(w.handle(Key::Enter), WizardOutcome::Keep));
        // Display name pre-filled "GitHub" — accept.
        assert!(matches!(w.handle(Key::Enter), WizardOutcome::Keep));
        // Repository (optional).
        typ(&mut w, "octo/repo");
        w.handle(Key::Enter);
        // PAT (required) — empty Enter should NOT advance.
        assert!(matches!(w.handle(Key::Enter), WizardOutcome::Keep));
        typ(&mut w, "ghp_xyz");
        w.handle(Key::Enter);
        // The bind step is a checklist with everything GitHub supports already ticked,
        // so Enter commits the lot without any further input.
        assert!(matches!(w.handle(Key::Enter), WizardOutcome::Commit));

        let d = &w.draft;
        assert_eq!(d.provider, Some(ProviderType::GitHub));
        assert_eq!(d.display_name, "GitHub");
        assert_eq!(d.repository.as_deref(), Some("octo/repo"));
        assert_eq!(d.pat.as_deref(), Some("ghp_xyz"));
        assert_eq!(d.bind_sections, provider_sections(ProviderType::GitHub));
    }

    #[test]
    fn linear_only_asks_for_key_and_binds_work_items() {
        let mut w = Wizard::new();
        // The list is `setup::selectable_providers()`: GitHub, GitLab, Azure DevOps,
        // Bitbucket, Linear, Jira — default GitHub (0), so Linear (4) is four Downs.
        let linear = forgetop_core::setup::selectable_providers()
            .iter()
            .position(|p| *p == ProviderType::Linear)
            .expect("Linear is selectable");
        for _ in 0..linear {
            w.handle(Key::Down);
        }
        w.handle(Key::Enter);
        assert_eq!(w.draft.provider, Some(ProviderType::Linear));
        // Display name.
        w.handle(Key::Enter);
        // API key.
        typ(&mut w, "lin_key");
        w.handle(Key::Enter);
        // Bind: Linear only supports Work Items, so that is the only row, ticked.
        assert!(matches!(w.handle(Key::Enter), WizardOutcome::Commit));
        assert_eq!(w.draft.bind_sections, vec![Section::WorkItems]);
    }

    #[test]
    fn space_unticks_a_section_and_the_rest_still_bind() {
        let mut w = Wizard::new();
        w.handle(Key::Enter); // GitHub
        w.handle(Key::Enter); // display name (prefilled)
        w.handle(Key::Enter); // repository (optional, blank)
        typ(&mut w, "ghp_xyz");
        w.handle(Key::Enter);

        // On the checklist: untick the first row, leave the others.
        let all = provider_sections(ProviderType::GitHub);
        assert!(all.len() > 1, "this test needs a provider supporting several sections");
        w.handle(Key::Char(' '));
        assert!(matches!(w.handle(Key::Enter), WizardOutcome::Commit));

        assert_eq!(w.draft.bind_sections, all[1..].to_vec(), "only the unticked row is dropped");
    }

    #[test]
    fn unticking_everything_binds_nothing_and_still_commits() {
        let mut w = Wizard::new();
        w.handle(Key::Enter);
        w.handle(Key::Enter);
        w.handle(Key::Enter);
        typ(&mut w, "ghp_xyz");
        w.handle(Key::Enter);

        // "Don't bind now" used to be a row; it is now simply an empty checklist.
        for _ in 0..provider_sections(ProviderType::GitHub).len() {
            w.handle(Key::Char(' '));
            w.handle(Key::Down);
        }
        assert!(matches!(w.handle(Key::Enter), WizardOutcome::Commit));
        assert!(w.draft.bind_sections.is_empty(), "nothing ticked binds nothing");
    }
}
