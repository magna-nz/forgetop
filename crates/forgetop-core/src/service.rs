//! Runtime config service + resolver/section/health services.

use std::sync::{Arc, Mutex};

use crate::config::*;
use crate::domain::Section;
use crate::error::{Error, Result};
use crate::provider::*;
use crate::secret::SecretStore;

/// Holds the live config and applies mutations at runtime: each change is validated
/// against provider capabilities and persisted. Secrets go to the secret store; the
/// config only ever references them by key.
pub struct ConfigService {
    store: Arc<dyn ConfigStore>,
    secrets: Arc<dyn SecretStore>,
    registry: Arc<ProviderRegistry>,
    config: Mutex<ForgetopConfig>,
}

impl ConfigService {
    pub fn new(store: Arc<dyn ConfigStore>, secrets: Arc<dyn SecretStore>, registry: Arc<ProviderRegistry>) -> Self {
        Self { store, secrets, registry, config: Mutex::new(ForgetopConfig::default()) }
    }

    pub async fn load(&self) -> Result<()> {
        let cfg = self.store.load().await?;
        *self.config.lock().unwrap() = cfg;
        Ok(())
    }

    pub fn snapshot(&self) -> ForgetopConfig {
        self.config.lock().unwrap().clone()
    }

    async fn persist(&self, cfg: ForgetopConfig) -> Result<()> {
        self.store.save(&cfg).await?;
        *self.config.lock().unwrap() = cfg;
        Ok(())
    }

    fn ensure_supports(&self, cfg: &ForgetopConfig, connection_id: &str, section: Section) -> Result<()> {
        let conn = cfg
            .find_connection(connection_id)
            .ok_or_else(|| Error::Config(format!("unknown connection '{connection_id}'")))?;
        let caps = self
            .registry
            .describe(conn.provider_type)
            .ok_or_else(|| Error::Config(format!("provider '{}' not registered", conn.provider_type.as_str())))?;
        if !caps.supports(section) {
            return Err(Error::Config(format!(
                "connection '{}' ({}) does not support {section:?}",
                conn.display_name,
                conn.provider_type.as_str()
            )));
        }
        Ok(())
    }

    pub async fn add_or_update_connection(&self, mut connection: Connection, secret: Option<String>) -> Result<()> {
        let cred = connection.credential_ref.clone().unwrap_or_else(|| connection.id.clone());
        connection.credential_ref = Some(cred.clone());

        if let Some(secret) = secret {
            if !self.secrets.is_writable() {
                return Err(Error::Config(
                    "secret store is read-only; provide the PAT via environment variable instead".into(),
                ));
            }
            self.secrets.set(&cred, &secret)?;
        }

        let mut cfg = self.snapshot();
        cfg.connections.retain(|c| c.id != connection.id);
        cfg.connections.push(connection);
        self.persist(cfg).await
    }

    pub async fn remove_connection(&self, connection_id: &str) -> Result<()> {
        let mut cfg = self.snapshot();
        if let Some(existing) = cfg.find_connection(connection_id).cloned() {
            if let (Some(cred), true) = (existing.credential_ref.as_ref(), self.secrets.is_writable()) {
                let _ = self.secrets.delete(cred);
            }
        }
        cfg.connections.retain(|c| c.id != connection_id);
        if cfg.pull_requests.as_ref().is_some_and(|b| b.connection_id == connection_id) {
            cfg.pull_requests = None;
        }
        if cfg.work_items.as_ref().is_some_and(|b| b.connection_id == connection_id) {
            cfg.work_items = None;
        }
        if let Some(p) = &mut cfg.pipelines {
            p.subscriptions.retain(|s| s.connection_id != connection_id);
        }
        self.persist(cfg).await
    }

    pub async fn bind_pull_requests(&self, connection_id: &str) -> Result<()> {
        let mut cfg = self.snapshot();
        self.ensure_supports(&cfg, connection_id, Section::PullRequests)?;
        cfg.pull_requests = Some(PullRequestBinding { connection_id: connection_id.into() });
        self.persist(cfg).await
    }

    pub async fn bind_work_items(&self, connection_id: &str) -> Result<()> {
        let mut cfg = self.snapshot();
        self.ensure_supports(&cfg, connection_id, Section::WorkItems)?;
        cfg.work_items = Some(WorkItemBinding { connection_id: connection_id.into() });
        self.persist(cfg).await
    }

    pub async fn unbind_section(&self, section: Section) -> Result<()> {
        let mut cfg = self.snapshot();
        match section {
            Section::PullRequests => cfg.pull_requests = None,
            Section::WorkItems => cfg.work_items = None,
            Section::Pipelines => cfg.pipelines = None,
        }
        self.persist(cfg).await
    }

    async fn mutate_subscription(&self, connection_id: &str, f: impl FnOnce(&mut PipelineSubscription)) -> Result<()> {
        let mut cfg = self.snapshot();
        self.ensure_supports(&cfg, connection_id, Section::Pipelines)?;
        let binding = cfg.pipelines.get_or_insert_with(PipelineBinding::default);
        let sub = match binding.subscriptions.iter_mut().find(|s| s.connection_id == connection_id) {
            Some(existing) => existing,
            None => {
                binding.subscriptions.push(PipelineSubscription {
                    connection_id: connection_id.into(),
                    definition_ids: Vec::new(),
                    auto_discover_all: false,
                });
                binding.subscriptions.last_mut().unwrap()
            }
        };
        f(sub);
        self.persist(cfg).await
    }

    pub async fn subscribe_pipeline(&self, connection_id: &str, definition_id: &str) -> Result<()> {
        let definition_id = definition_id.to_string();
        self.mutate_subscription(connection_id, move |s| {
            if !s.definition_ids.contains(&definition_id) {
                s.definition_ids.push(definition_id);
            }
        })
        .await
    }

    pub async fn set_pipeline_auto_discover(&self, connection_id: &str, auto_discover_all: bool) -> Result<()> {
        self.mutate_subscription(connection_id, move |s| s.auto_discover_all = auto_discover_all).await
    }

    pub async fn unsubscribe_pipeline(&self, connection_id: &str, definition_id: &str) -> Result<()> {
        self.mutate_subscription(connection_id, move |s| s.definition_ids.retain(|d| d != definition_id)).await
    }

    pub async fn remove_pipeline_connection(&self, connection_id: &str) -> Result<()> {
        let mut cfg = self.snapshot();
        if let Some(p) = &mut cfg.pipelines {
            p.subscriptions.retain(|s| s.connection_id != connection_id);
        }
        self.persist(cfg).await
    }

    pub async fn set_theme(&self, theme: Option<String>) -> Result<()> {
        let mut cfg = self.snapshot();
        cfg.ui.theme = theme;
        self.persist(cfg).await
    }

    pub async fn set_hidden_sections(&self, hidden: Vec<Section>) -> Result<()> {
        let mut cfg = self.snapshot();
        cfg.ui.hidden_sections = hidden;
        self.persist(cfg).await
    }

    pub async fn set_hidden_work_item_states(&self, hidden: Vec<String>) -> Result<()> {
        let mut cfg = self.snapshot();
        cfg.ui.hidden_work_item_states = hidden;
        self.persist(cfg).await
    }

    /// Replaces a connection's tracked pipeline definitions with an explicit set
    /// (turns off auto-discovery). An empty list tracks nothing.
    pub async fn set_pipeline_definitions(&self, connection_id: &str, definition_ids: Vec<String>) -> Result<()> {
        self.mutate_subscription(connection_id, move |s| {
            s.auto_discover_all = false;
            s.definition_ids = definition_ids;
        })
        .await
    }
}

/// Turns a configured connection id into a live [`ProviderConnection`].
pub struct ConnectionResolver {
    config: Arc<ConfigService>,
    registry: Arc<ProviderRegistry>,
    secrets: Arc<dyn SecretStore>,
}

impl ConnectionResolver {
    pub fn new(config: Arc<ConfigService>, registry: Arc<ProviderRegistry>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { config, registry, secrets }
    }

    pub async fn resolve(&self, connection_id: &str) -> Result<Option<Arc<dyn ProviderConnection>>> {
        let cfg = self.config.snapshot();
        let conn = match cfg.find_connection(connection_id) {
            Some(c) => c.clone(),
            None => return Ok(None),
        };
        if !self.registry.supports(conn.provider_type) {
            return Ok(None);
        }
        let secret = match &conn.credential_ref {
            Some(r) => self.secrets.get(r)?,
            None => None,
        };
        Ok(Some(self.registry.create(&conn, secret)?))
    }
}

/// One pipeline connection feeding the Pipelines section, with its subscription.
pub struct PipelineFeed {
    pub connection: Arc<dyn ProviderConnection>,
    pub source: Arc<dyn PipelineSource>,
    pub subscription: PipelineSubscription,
}

/// Resolves the live source(s) backing each section from the current bindings.
pub struct SectionService {
    config: Arc<ConfigService>,
    resolver: Arc<ConnectionResolver>,
}

impl SectionService {
    pub fn new(config: Arc<ConfigService>, resolver: Arc<ConnectionResolver>) -> Self {
        Self { config, resolver }
    }

    pub async fn pull_request_source(&self) -> Result<Option<Arc<dyn PullRequestSource>>> {
        let cfg = self.config.snapshot();
        let Some(binding) = cfg.pull_requests else { return Ok(None) };
        Ok(self.resolver.resolve(&binding.connection_id).await?.and_then(|c| c.pull_requests()))
    }

    pub async fn work_item_source(&self) -> Result<Option<Arc<dyn WorkItemSource>>> {
        let cfg = self.config.snapshot();
        let Some(binding) = cfg.work_items else { return Ok(None) };
        Ok(self.resolver.resolve(&binding.connection_id).await?.and_then(|c| c.work_items()))
    }

    /// Resolves a connection's pipeline source directly, regardless of whether it is
    /// currently subscribed — used to discover definitions before subscribing.
    pub async fn pipeline_source_for(&self, connection_id: &str) -> Result<Option<Arc<dyn PipelineSource>>> {
        Ok(self.resolver.resolve(connection_id).await?.and_then(|c| c.pipelines()))
    }

    pub async fn pipeline_feeds(&self) -> Result<Vec<PipelineFeed>> {
        let cfg = self.config.snapshot();
        let Some(binding) = cfg.pipelines else { return Ok(Vec::new()) };
        let mut feeds = Vec::new();
        for sub in binding.subscriptions {
            if let Some(conn) = self.resolver.resolve(&sub.connection_id).await? {
                if let Some(source) = conn.pipelines() {
                    feeds.push(PipelineFeed { connection: conn, source, subscription: sub });
                }
            }
        }
        Ok(feeds)
    }
}

/// A configured connection and whether it's currently reachable/authed.
pub struct ConnectionHealth {
    pub connection: Connection,
    pub healthy: bool,
}

/// Probes each configured connection for the connections health bar.
pub struct ConnectionHealthService {
    config: Arc<ConfigService>,
    resolver: Arc<ConnectionResolver>,
}

impl ConnectionHealthService {
    pub fn new(config: Arc<ConfigService>, resolver: Arc<ConnectionResolver>) -> Self {
        Self { config, resolver }
    }

    pub async fn check_all(&self) -> Vec<ConnectionHealth> {
        let cfg = self.config.snapshot();
        let mut out = Vec::new();
        for connection in cfg.connections {
            let healthy = match self.resolver.resolve(&connection.id).await {
                Ok(Some(live)) => live.check().await,
                _ => false,
            };
            out.push(ConnectionHealth { connection, healthy });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ProviderType;
    use crate::secret::InMemorySecretStore;
    use async_trait::async_trait;

    struct FakeConn {
        caps: Capabilities,
    }
    #[async_trait]
    impl ProviderConnection for FakeConn {
        fn connection_id(&self) -> &str { "fake" }
        fn provider_type(&self) -> ProviderType { ProviderType::GitHub }
        fn display_name(&self) -> &str { "Fake" }
        fn capabilities(&self) -> &Capabilities { &self.caps }
        fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> { None }
        fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> { None }
        fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> { None }
        async fn check(&self) -> bool { true }
    }
    struct FakeFactory {
        provider: ProviderType,
        caps: Capabilities,
    }
    impl ProviderFactory for FakeFactory {
        fn provider_type(&self) -> ProviderType { self.provider }
        fn describe_capabilities(&self) -> Capabilities { self.caps.clone() }
        fn create(&self, _c: &Connection, _s: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
            Ok(Arc::new(FakeConn { caps: self.caps.clone() }))
        }
    }

    fn registry() -> Arc<ProviderRegistry> {
        Arc::new(ProviderRegistry::new(vec![
            Arc::new(FakeFactory {
                provider: ProviderType::GitHub,
                caps: Capabilities { supports_pull_requests: true, supports_pipelines: true, ..Default::default() },
            }),
            Arc::new(FakeFactory {
                provider: ProviderType::Linear,
                caps: Capabilities { supports_work_items: true, ..Default::default() },
            }),
        ]))
    }

    fn conn(id: &str, provider: ProviderType) -> Connection {
        Connection {
            id: id.into(),
            provider_type: provider,
            display_name: id.into(),
            base_url: None,
            organization: None,
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
        }
    }

    fn service() -> (Arc<ConfigService>, Arc<InMemorySecretStore>) {
        let secrets = Arc::new(InMemorySecretStore::default());
        let svc = Arc::new(ConfigService::new(
            Arc::new(InMemoryConfigStore::default()),
            secrets.clone(),
            registry(),
        ));
        (svc, secrets)
    }

    #[tokio::test]
    async fn add_connection_persists_and_stores_secret() {
        let (svc, secrets) = service();
        svc.add_or_update_connection(conn("gh-1", ProviderType::GitHub), Some("pat".into())).await.unwrap();
        assert_eq!(svc.snapshot().connections.len(), 1);
        assert_eq!(secrets.get("gh-1").unwrap().as_deref(), Some("pat"));
    }

    #[tokio::test]
    async fn bind_validates_capability() {
        let (svc, _) = service();
        svc.add_or_update_connection(conn("gh-1", ProviderType::GitHub), None).await.unwrap();
        svc.add_or_update_connection(conn("lin-1", ProviderType::Linear), None).await.unwrap();
        svc.bind_pull_requests("gh-1").await.unwrap();
        assert_eq!(svc.snapshot().pull_requests.unwrap().connection_id, "gh-1");
        assert!(svc.bind_pull_requests("lin-1").await.is_err());
    }

    #[tokio::test]
    async fn remove_connection_cascades() {
        let (svc, secrets) = service();
        svc.add_or_update_connection(conn("gh-1", ProviderType::GitHub), Some("pat".into())).await.unwrap();
        svc.bind_pull_requests("gh-1").await.unwrap();
        svc.subscribe_pipeline("gh-1", "ci").await.unwrap();
        svc.remove_connection("gh-1").await.unwrap();
        let cfg = svc.snapshot();
        assert!(cfg.connections.is_empty());
        assert!(cfg.pull_requests.is_none());
        assert!(cfg.pipelines.unwrap().subscriptions.is_empty());
        assert_eq!(secrets.get("gh-1").unwrap(), None);
    }

    #[tokio::test]
    async fn pipeline_subscribe_unsubscribe() {
        let (svc, _) = service();
        svc.add_or_update_connection(conn("gh-1", ProviderType::GitHub), None).await.unwrap();
        svc.subscribe_pipeline("gh-1", "ci").await.unwrap();
        svc.subscribe_pipeline("gh-1", "release").await.unwrap();
        svc.subscribe_pipeline("gh-1", "ci").await.unwrap(); // duplicate ignored
        assert_eq!(svc.snapshot().pipelines.unwrap().subscriptions[0].definition_ids.len(), 2);
        svc.unsubscribe_pipeline("gh-1", "ci").await.unwrap();
        assert_eq!(svc.snapshot().pipelines.unwrap().subscriptions[0].definition_ids.len(), 1);
    }

    #[tokio::test]
    async fn set_pipeline_definitions_replaces_and_disables_auto() {
        let (svc, _) = service();
        svc.add_or_update_connection(conn("gh-1", ProviderType::GitHub), None).await.unwrap();
        svc.set_pipeline_auto_discover("gh-1", true).await.unwrap();
        svc.set_pipeline_definitions("gh-1", vec!["ci".into(), "release".into()]).await.unwrap();
        let cfg = svc.snapshot();
        let sub = &cfg.pipelines.unwrap().subscriptions[0];
        assert!(!sub.auto_discover_all);
        assert_eq!(sub.definition_ids, vec!["ci".to_string(), "release".to_string()]);
    }
}
