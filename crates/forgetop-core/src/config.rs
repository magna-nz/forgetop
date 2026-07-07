//! Persisted configuration: connections + per-section bindings, and the config store.
//! Note: config NEVER contains secrets — only a `credential_ref` key into the secret store.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::Section;
use crate::error::Result;
use crate::provider::Connection;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestBinding {
    pub connection_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkItemBinding {
    pub connection_id: String,
}

/// One connection feeding the Pipelines section, plus the pipelines subscribed from it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineSubscription {
    pub connection_id: String,
    #[serde(default)]
    pub definition_ids: Vec<String>,
    #[serde(default)]
    pub auto_discover_all: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineBinding {
    #[serde(default)]
    pub subscriptions: Vec<PipelineSubscription>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default)]
    pub active_section: Section,
    /// Sections the user has hidden from the tab bar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_sections: Vec<Section>,
    /// Work-item state names the user has hidden from the Work Items list
    /// (provider-specific strings, e.g. "Done"). Anything not listed is shown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_work_item_states: Vec<String>,
    /// Per-view sort column + direction. `None` = provider order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_sort: Option<SortPref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_sort: Option<SortPref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline_sort: Option<SortPref>,
    /// Which desktop notifications are enabled. Defaults to all on.
    #[serde(default)]
    pub notifications: NotificationPrefs,
}

fn yes() -> bool {
    true
}

/// Per-event desktop-notification opt-ins. Every event defaults to on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPrefs {
    #[serde(default = "yes")]
    pub pipeline_failed: bool,
    #[serde(default = "yes")]
    pub review_requested: bool,
    #[serde(default = "yes")]
    pub pr_approved: bool,
    #[serde(default = "yes")]
    pub pr_changes_requested: bool,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self { pipeline_failed: true, review_requested: true, pr_approved: true, pr_changes_requested: true }
    }
}

impl NotificationPrefs {
    /// True if any event is enabled.
    pub fn any(&self) -> bool {
        self.pipeline_failed || self.review_requested || self.pr_approved || self.pr_changes_requested
    }
}

/// A saved sort: which column (by key) and whether descending.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortPref {
    pub key: String,
    pub desc: bool,
}

/// Root persisted configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForgetopConfig {
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_requests: Option<PullRequestBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_items: Option<WorkItemBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipelines: Option<PipelineBinding>,
    #[serde(default)]
    pub ui: UiState,
}

impl ForgetopConfig {
    pub fn find_connection(&self, id: &str) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }
}

/// Loads and saves the root configuration.
#[async_trait]
pub trait ConfigStore: Send + Sync {
    async fn load(&self) -> Result<ForgetopConfig>;
    async fn save(&self, config: &ForgetopConfig) -> Result<()>;
}

/// Resolves the on-disk config path: `$XDG_CONFIG_HOME/forgetop/config.json` (or the
/// platform config dir).
pub fn default_config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("forgetop").join("config.json")
}

/// JSON-file config store with atomic (temp + rename) writes.
pub struct JsonConfigStore {
    path: PathBuf,
}

impl JsonConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn at_default_path() -> Self {
        Self::new(default_config_path())
    }
}

#[async_trait]
impl ConfigStore for JsonConfigStore {
    async fn load(&self) -> Result<ForgetopConfig> {
        if !self.path.exists() {
            return Ok(ForgetopConfig::default());
        }
        let bytes = tokio::fs::read(&self.path).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn save(&self, config: &ForgetopConfig) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let json = serde_json::to_vec_pretty(config)?;
        // Write to a temp file then rename so a crash mid-write can't corrupt config.
        let tmp = with_extension(&self.path, "tmp");
        tokio::fs::write(&tmp, &json).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }
}

fn with_extension(path: &Path, ext: &str) -> PathBuf {
    let mut p = path.to_path_buf();
    p.set_extension(ext);
    p
}

/// Non-persistent config store (used for `--demo` and tests).
pub struct InMemoryConfigStore {
    config: std::sync::Mutex<ForgetopConfig>,
}

impl InMemoryConfigStore {
    pub fn new(seed: ForgetopConfig) -> Self {
        Self { config: std::sync::Mutex::new(seed) }
    }
}

impl Default for InMemoryConfigStore {
    fn default() -> Self {
        Self::new(ForgetopConfig::default())
    }
}

#[async_trait]
impl ConfigStore for InMemoryConfigStore {
    async fn load(&self) -> Result<ForgetopConfig> {
        Ok(self.config.lock().unwrap().clone())
    }
    async fn save(&self, config: &ForgetopConfig) -> Result<()> {
        *self.config.lock().unwrap() = config.clone();
        Ok(())
    }
}
