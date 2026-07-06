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
}

pub struct Prompt {
    pub field: Field,
    pub label: String,
    pub required: bool,
    pub kind: PromptKind,
}

impl Prompt {
    fn text(field: Field, label: &str, required: bool, prefill: &str) -> Self {
        Prompt { field, label: label.into(), required, kind: PromptKind::Text { buffer: prefill.into(), secret: false } }
    }
    fn secret(field: Field, label: &str) -> Self {
        Prompt { field, label: label.into(), required: true, kind: PromptKind::Text { buffer: String::new(), secret: true } }
    }
    fn pick(field: Field, label: &str, items: Vec<String>, selected: usize) -> Self {
        Prompt { field, label: label.into(), required: true, kind: PromptKind::Pick { items, selected } }
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
    pub bind_section: Option<Section>,
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

impl Wizard {
    pub fn new() -> Self {
        let providers = vec![
            "Demo".into(),
            "GitHub".into(),
            "Azure DevOps".into(),
            "Linear".into(),
            "GitLab".into(),
            "Jira".into(),
            "Bitbucket".into(),
        ];
        let mut queue = VecDeque::new();
        queue.push_back(Prompt::pick(Field::Provider, "Provider", providers, 1));
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
                self.draft.provider = Some(match selected {
                    0 => ProviderType::Demo,
                    1 => ProviderType::GitHub,
                    2 => ProviderType::AzureDevOps,
                    3 => ProviderType::Linear,
                    4 => ProviderType::GitLab,
                    5 => ProviderType::Jira,
                    _ => ProviderType::Bitbucket,
                });
            }
            (Field::DisplayName, PromptKind::Text { buffer, .. }) => self.draft.display_name = buffer.trim().to_string(),
            (Field::BaseUrl, PromptKind::Text { buffer, .. }) => self.draft.base_url = non_empty(buffer),
            (Field::Username, PromptKind::Text { buffer, .. }) => self.draft.username = non_empty(buffer),
            (Field::Organization, PromptKind::Text { buffer, .. }) => self.draft.organization = non_empty(buffer),
            (Field::Project, PromptKind::Text { buffer, .. }) => self.draft.project = non_empty(buffer),
            (Field::Repository, PromptKind::Text { buffer, .. }) => self.draft.repository = non_empty(buffer),
            (Field::Pat, PromptKind::Text { buffer, .. }) => self.draft.pat = non_empty(buffer),
            (Field::Bind, PromptKind::Pick { items, selected }) => {
                self.draft.bind_section = items.get(*selected).and_then(|l| section_from_label(l));
            }
            _ => {}
        }
    }

    fn enqueue_provider_steps(&mut self) {
        let provider = self.draft.provider.unwrap_or(ProviderType::Demo);
        self.queue.push_back(Prompt::text(Field::DisplayName, "Display name", true, provider.as_str()));
        match provider {
            ProviderType::Demo => {}
            ProviderType::GitHub => {
                self.queue.push_back(Prompt::text(Field::Repository, "Repository (owner/repo)", false, ""));
                self.queue.push_back(Prompt::secret(Field::Pat, "Personal access token"));
            }
            ProviderType::AzureDevOps => {
                self.queue.push_back(Prompt::text(Field::Organization, "Organization", true, ""));
                self.queue.push_back(Prompt::text(Field::Project, "Project", false, ""));
                self.queue.push_back(Prompt::text(Field::Repository, "Repository", false, ""));
                self.queue.push_back(Prompt::secret(Field::Pat, "Personal access token"));
            }
            ProviderType::Linear => {
                self.queue.push_back(Prompt::secret(Field::Pat, "API key"));
            }
            ProviderType::GitLab => {
                self.queue.push_back(Prompt::text(Field::Repository, "Project (group/project)", true, ""));
                self.queue.push_back(Prompt::secret(Field::Pat, "Personal access token"));
            }
            ProviderType::Jira => {
                self.queue.push_back(Prompt::text(Field::BaseUrl, "Site URL (https://your-site.atlassian.net)", true, ""));
                self.queue.push_back(Prompt::text(Field::Project, "Project key (e.g. ENG)", true, ""));
                self.queue.push_back(Prompt::text(Field::Username, "Email", true, ""));
                self.queue.push_back(Prompt::secret(Field::Pat, "API token"));
            }
            ProviderType::Bitbucket => {
                self.queue.push_back(Prompt::text(Field::Organization, "Workspace", true, ""));
                self.queue.push_back(Prompt::text(Field::Repository, "Repository (slug)", true, ""));
                self.queue.push_back(Prompt::text(Field::Username, "Username", true, ""));
                self.queue.push_back(Prompt::secret(Field::Pat, "App password"));
            }
        }
        let mut items: Vec<String> = provider_sections(provider).into_iter().map(|s| section_label(s).to_string()).collect();
        items.push("Don't bind now".into());
        self.queue.push_back(Prompt::pick(Field::Bind, "Bind to section", items, 0));
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

pub fn provider_sections(provider: ProviderType) -> Vec<Section> {
    match provider {
        ProviderType::Linear | ProviderType::Jira => vec![Section::WorkItems],
        ProviderType::Bitbucket => vec![Section::PullRequests, Section::Pipelines],
        _ => vec![Section::PullRequests, Section::WorkItems, Section::Pipelines],
    }
}

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
        // Bind pick defaults to first section (Pull Requests) — Enter commits.
        assert!(matches!(w.handle(Key::Enter), WizardOutcome::Commit));

        let d = &w.draft;
        assert_eq!(d.provider, Some(ProviderType::GitHub));
        assert_eq!(d.display_name, "GitHub");
        assert_eq!(d.repository.as_deref(), Some("octo/repo"));
        assert_eq!(d.pat.as_deref(), Some("ghp_xyz"));
        assert_eq!(d.bind_section, Some(Section::PullRequests));
    }

    #[test]
    fn linear_only_asks_for_key_and_binds_work_items() {
        let mut w = Wizard::new();
        // Move provider selection to Linear (index 3): default is GitHub (1) -> Down twice.
        w.handle(Key::Down); // 1 -> 2 (Azure DevOps)
        w.handle(Key::Down); // 2 -> 3 (Linear)
        w.handle(Key::Enter);
        assert_eq!(w.draft.provider, Some(ProviderType::Linear));
        // Display name.
        w.handle(Key::Enter);
        // API key.
        typ(&mut w, "lin_key");
        w.handle(Key::Enter);
        // Bind: only Work Items + "Don't bind now" offered.
        assert!(matches!(w.handle(Key::Enter), WizardOutcome::Commit));
        assert_eq!(w.draft.bind_section, Some(Section::WorkItems));
    }
}
