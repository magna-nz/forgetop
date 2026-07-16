//! Connection management for the dashboard settings page. Everything goes through the shared
//! `ConfigService`, so a connection added here is the same one the TUI sees. Tokens are written
//! straight to the OS keychain (via `add_or_update_connection`) and are **never** returned by any
//! of these endpoints, written to the config file, or logged.

use forgetop_core::domain::{ProviderType, Section};
use forgetop_core::provider::Connection;
use forgetop_core::secret::SecretStore;
use forgetop_core::service::{ConfigService, ConnectionHealthService};
use forgetop_core::setup::{self, FieldSpec};
use serde::{Deserialize, Serialize};

fn section_key(s: Section) -> &'static str {
    match s {
        Section::PullRequests => "pull_requests",
        Section::WorkItems => "work_items",
        Section::Pipelines => "pipelines",
    }
}

// ---- provider schema ----

#[derive(Serialize)]
pub struct ProviderInfo {
    pub provider: ProviderType,
    pub label: String,
    pub fields: Vec<FieldSpec>,
    pub sections: Vec<&'static str>,
}

/// The set-up-able providers and the fields each needs (shared with the TUI wizard).
pub fn providers() -> Vec<ProviderInfo> {
    setup::selectable_providers()
        .into_iter()
        .map(|p| ProviderInfo {
            provider: p,
            label: p.as_str().to_string(),
            fields: setup::connection_fields(p),
            sections: setup::provider_sections(p).into_iter().map(section_key).collect(),
        })
        .collect()
}

// ---- listing ----

/// A configured connection as the settings page sees it — never includes the token, only whether
/// one is set.
#[derive(Serialize)]
pub struct ConnectionRow {
    pub id: String,
    pub provider: ProviderType,
    pub display_name: String,
    pub base_url: Option<String>,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub repository: Option<String>,
    pub username: Option<String>,
    pub has_token: bool,
    pub sections: Vec<&'static str>,
}

pub fn list(config: &ConfigService, secrets: &dyn SecretStore) -> Vec<ConnectionRow> {
    let cfg = config.snapshot();
    cfg.connections
        .iter()
        .map(|c| {
            let cred = c.credential_ref.as_deref().unwrap_or(&c.id);
            let has_token = matches!(secrets.get(cred), Ok(Some(_)));
            let mut sections = Vec::new();
            if cfg.pull_requests.as_ref().is_some_and(|b| b.ids().iter().any(|x| x == &c.id)) {
                sections.push("pull_requests");
            }
            if cfg.work_items.as_ref().is_some_and(|b| b.ids().iter().any(|x| x == &c.id)) {
                sections.push("work_items");
            }
            if cfg.pipelines.as_ref().is_some_and(|p| p.subscriptions.iter().any(|s| s.connection_id == c.id)) {
                sections.push("pipelines");
            }
            ConnectionRow {
                id: c.id.clone(),
                provider: c.provider_type,
                display_name: c.display_name.clone(),
                base_url: c.base_url.clone(),
                organization: c.organization.clone(),
                project: c.project.clone(),
                repository: c.repository.clone(),
                username: c.username.clone(),
                has_token,
                sections,
            }
        })
        .collect()
}

// ---- save / delete / test ----

#[derive(Deserialize)]
pub struct SaveConnectionReq {
    /// `None` creates a new connection; `Some` edits an existing one.
    #[serde(default)]
    pub id: Option<String>,
    pub provider: ProviderType,
    pub display_name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    /// The secret. `None`/empty keeps the existing token (on edit).
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub bind_pull_requests: bool,
    #[serde(default)]
    pub bind_work_items: bool,
    #[serde(default)]
    pub bind_pipelines: bool,
}

fn clean(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Adds or updates a connection (token → keychain) and reconciles its section bindings. Returns
/// the connection id.
pub async fn save(config: &ConfigService, req: SaveConnectionReq) -> Result<String, String> {
    let editing = req.id.clone();
    let id = editing.clone().unwrap_or_else(|| Connection::new_id(req.provider));

    // Preserve the existing keychain reference when editing so we don't orphan the token.
    let credential_ref = editing.as_ref().and_then(|eid| {
        config.snapshot().connections.iter().find(|c| &c.id == eid).and_then(|c| c.credential_ref.clone())
    });

    let display_name = match clean(Some(req.display_name)) {
        Some(n) => n,
        None => req.provider.as_str().to_string(),
    };
    let connection = Connection {
        id: id.clone(),
        provider_type: req.provider,
        display_name,
        base_url: clean(req.base_url),
        organization: clean(req.organization),
        project: clean(req.project),
        repository: clean(req.repository),
        username: clean(req.username),
        credential_ref,
    };

    let token = clean(req.token);
    config.add_or_update_connection(connection, token).await.map_err(|e| e.to_string())?;

    // Reconcile bindings for the sections this provider actually supports.
    let supported = setup::provider_sections(req.provider);
    for (section, want) in [
        (Section::PullRequests, req.bind_pull_requests),
        (Section::WorkItems, req.bind_work_items),
        (Section::Pipelines, req.bind_pipelines),
    ] {
        if !supported.contains(&section) {
            continue;
        }
        let result = match (section, want) {
            (Section::PullRequests, true) => config.bind_pull_requests(&id).await,
            (Section::PullRequests, false) => config.unbind_pull_requests(&id).await,
            (Section::WorkItems, true) => config.bind_work_items(&id).await,
            (Section::WorkItems, false) => config.unbind_work_items(&id).await,
            (Section::Pipelines, true) => config.set_pipeline_auto_discover(&id, true).await,
            (Section::Pipelines, false) => config.unbind_pipelines(&id).await,
        };
        result.map_err(|e| e.to_string())?;
    }

    Ok(id)
}

pub async fn remove(config: &ConfigService, id: &str) -> Result<(), String> {
    config.remove_connection(id).await.map_err(|e| e.to_string())
}

/// Checks whether a saved connection authenticates (network round-trip).
pub async fn test(health: &ConnectionHealthService, id: &str) -> Option<bool> {
    health.check_all().await.into_iter().find(|h| h.connection.id == id).map(|h| h.healthy)
}
