//! `forgetop` — htop for your software forges. Wires the provider registry and the
//! runtime services into the ratatui TUI.

use std::io::IsTerminal;
use std::sync::Arc;

use forgetop_core::config::{
    default_config_path, ConfigStore, ForgetopConfig, InMemoryConfigStore, JsonConfigStore,
};
use forgetop_core::domain::ProviderType;
use forgetop_core::provider::{Connection, ProviderRegistry};
use forgetop_core::secret::{default_secret_store, InMemorySecretStore, SecretStore};
use forgetop_core::service::{ConfigService, ConnectionHealthService, ConnectionResolver, SectionService};
use forgetop_core::Result;
use forgetop_providers::default_factories;
use forgetop_tui::AppDeps;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("forgetop: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let demo = std::env::args().any(|a| a == "--demo" || a == "-d");

    if !std::io::stdout().is_terminal() {
        eprintln!("forgetop needs an interactive terminal (TTY) to run the dashboard.");
        return Ok(());
    }

    let registry = Arc::new(ProviderRegistry::new(default_factories()));

    // Demo runs entirely in memory — nothing is written to disk or the keychain.
    let (store, secrets): (Arc<dyn ConfigStore>, Arc<dyn SecretStore>) = if demo {
        (Arc::new(InMemoryConfigStore::new(ForgetopConfig::default())), Arc::new(InMemorySecretStore::default()))
    } else {
        (Arc::new(JsonConfigStore::new(default_config_path())), Arc::from(default_secret_store()))
    };

    let config = Arc::new(ConfigService::new(store, secrets.clone(), registry.clone()));
    config.load().await?;

    if demo {
        seed_demo(&config).await?;
    }

    let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry.clone(), secrets));
    let sections = Arc::new(SectionService::new(config.clone(), resolver.clone()));
    let health = Arc::new(ConnectionHealthService::new(config.clone(), resolver));

    let theme = config.snapshot().ui.theme.clone().unwrap_or_else(|| "slate".into());
    let deps = AppDeps { sections, health, config };
    forgetop_tui::run(deps, &theme).await
}

/// Seeds an in-memory Demo connection bound to all three sections.
async fn seed_demo(config: &ConfigService) -> Result<()> {
    let conn = Connection {
        id: "demo".into(),
        provider_type: ProviderType::Demo,
        display_name: "Demo".into(),
        base_url: None,
        organization: None,
        project: None,
        repository: None,
        username: None,
        credential_ref: None,
    };
    config.add_or_update_connection(conn, None).await?;
    config.bind_pull_requests("demo").await?;
    config.bind_work_items("demo").await?;
    config.set_pipeline_auto_discover("demo", true).await?;
    Ok(())
}
