//! Connection setup schema — the fields each provider needs, and which sections it can feed.
//!
//! Shared by the TUI's add-connection wizard and the web dashboard's settings page so the two
//! ask for exactly the same things. Pure metadata: no I/O, no secrets.

use serde::Serialize;

use crate::domain::{ProviderType, Section};

/// A field in a connection form. Maps onto a [`crate::provider::Connection`] field, except
/// [`FieldKey::Pat`], which is the secret stored in the keychain (never in the config).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKey {
    DisplayName,
    BaseUrl,
    Organization,
    Project,
    Repository,
    Username,
    Pat,
}

/// One input in a provider's connection form.
#[derive(Debug, Clone, Serialize)]
pub struct FieldSpec {
    pub key: FieldKey,
    pub label: String,
    pub help: String,
    pub required: bool,
    /// True for the token/password field — masked in the UI, write-only to the keychain.
    pub secret: bool,
    /// A suggested default (e.g. the provider name for the display name).
    pub default: Option<String>,
}

impl FieldSpec {
    fn text(key: FieldKey, label: &str, help: &str, required: bool) -> Self {
        FieldSpec { key, label: label.into(), help: help.into(), required, secret: false, default: None }
    }
    fn secret(label: &str, help: &str) -> Self {
        FieldSpec { key: FieldKey::Pat, label: label.into(), help: help.into(), required: true, secret: true, default: None }
    }
}

/// The ordered fields a connection to `provider` needs — display name first, secret last.
pub fn connection_fields(provider: ProviderType) -> Vec<FieldSpec> {
    let mut out = vec![FieldSpec {
        key: FieldKey::DisplayName,
        label: "Display name".into(),
        help: "A label shown in the tab bar and connections list".into(),
        required: true,
        secret: false,
        default: Some(provider.as_str().to_string()),
    }];
    match provider {
        ProviderType::Demo => {}
        ProviderType::GitHub => {
            out.push(FieldSpec::text(FieldKey::Repository, "Repository (owner/repo)", "e.g. octocat/hello-world — the owner and repo from the URL", false));
            out.push(FieldSpec::secret("Personal access token", "github.com → Settings → Developer settings → Personal access tokens · scope: repo"));
        }
        ProviderType::AzureDevOps => {
            out.push(FieldSpec::text(FieldKey::Organization, "Organization", "The {org} in dev.azure.com/{org}", true));
            out.push(FieldSpec::text(FieldKey::Project, "Project", "The team project name (leave blank to use the org default)", false));
            out.push(FieldSpec::text(FieldKey::Repository, "Repository", "The Git repo name (defaults to the project name)", false));
            out.push(FieldSpec::secret("Personal access token", "dev.azure.com → User settings → Personal access tokens · Code, Work Items, Build"));
        }
        ProviderType::Linear => {
            out.push(FieldSpec::secret("API key", "linear.app → Settings → Security & access → API → Personal API keys"));
        }
        ProviderType::GitLab => {
            out.push(FieldSpec::text(FieldKey::Repository, "Project (group/project)", "e.g. mygroup/myapp — the full path from the URL", true));
            out.push(FieldSpec::secret("Personal access token", "gitlab.com → Preferences → Access Tokens · scope: api"));
        }
        ProviderType::Jira => {
            out.push(FieldSpec::text(FieldKey::BaseUrl, "Site URL", "Your Atlassian site, e.g. https://your-company.atlassian.net", true));
            out.push(FieldSpec::text(FieldKey::Project, "Project key", "The prefix on issue keys, e.g. ENG in ENG-123", true));
            out.push(FieldSpec::text(FieldKey::Username, "Email", "The email of your Atlassian account", true));
            out.push(FieldSpec::secret("API token", "id.atlassian.com → Security → Create and manage API tokens"));
        }
        ProviderType::Bitbucket => {
            out.push(FieldSpec::text(FieldKey::Organization, "Workspace", "The {workspace} in bitbucket.org/{workspace}", true));
            out.push(FieldSpec::text(FieldKey::Repository, "Repository (slug)", "The repo slug from the URL, e.g. my-app", true));
            out.push(FieldSpec::text(FieldKey::Username, "Username", "Your Bitbucket username (not your email)", true));
            out.push(FieldSpec::secret("App password", "Personal settings → App passwords · Pull requests + Pipelines (read & write)"));
        }
    }
    out
}

/// The sections `provider` can populate (drives which bindings the form offers).
pub fn provider_sections(provider: ProviderType) -> Vec<Section> {
    match provider {
        ProviderType::Linear | ProviderType::Jira => vec![Section::WorkItems],
        ProviderType::Bitbucket => vec![Section::PullRequests, Section::Pipelines],
        _ => vec![Section::PullRequests, Section::WorkItems, Section::Pipelines],
    }
}

/// The provider types a user can set up (excludes `Demo`, which is only for `--demo`).
pub fn selectable_providers() -> Vec<ProviderType> {
    vec![
        ProviderType::GitHub,
        ProviderType::GitLab,
        ProviderType::AzureDevOps,
        ProviderType::Bitbucket,
        ProviderType::Linear,
        ProviderType::Jira,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_needs_a_token_and_feeds_three_sections() {
        let fields = connection_fields(ProviderType::GitHub);
        assert_eq!(fields[0].key, FieldKey::DisplayName);
        assert!(fields.iter().any(|f| f.key == FieldKey::Pat && f.secret));
        assert_eq!(provider_sections(ProviderType::GitHub).len(), 3);
    }

    #[test]
    fn linear_is_work_items_only_and_just_a_key() {
        assert_eq!(provider_sections(ProviderType::Linear), vec![Section::WorkItems]);
        let fields = connection_fields(ProviderType::Linear);
        // display name + the API key, nothing else.
        assert_eq!(fields.len(), 2);
        assert!(fields[1].secret);
    }
}
