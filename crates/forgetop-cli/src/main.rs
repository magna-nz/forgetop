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
use forgetop_providers::demo::demo_factories;
use forgetop_tui::AppDeps;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("forgetop: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Handle --version / --help before touching config or the keychain.
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("forgetop {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    let demo = args.iter().any(|a| a == "--demo" || a == "-d");
    let is_doctor = args.get(1).map(String::as_str) == Some("doctor");

    // Under --demo, every provider type resolves to a canned demo source (no network).
    let registry = Arc::new(ProviderRegistry::new(if demo { demo_factories() } else { default_factories() }));

    // Demo runs entirely in memory — nothing is written to disk or the keychain.
    let (store, secrets): (Arc<dyn ConfigStore>, Arc<dyn SecretStore>) = if demo {
        (Arc::new(InMemoryConfigStore::new(ForgetopConfig::default())), Arc::new(InMemorySecretStore::default()))
    } else {
        (Arc::new(JsonConfigStore::new(default_config_path())), Arc::from(default_secret_store()))
    };

    let config = Arc::new(ConfigService::new(store, secrets.clone(), registry.clone()));
    config.load().await?;
    let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry.clone(), secrets.clone()));

    // `forgetop doctor` prints a diagnostic and exits — no TTY needed.
    if is_doctor {
        return doctor(&config, &secrets, &resolver).await;
    }

    if !std::io::stdout().is_terminal() {
        eprintln!("forgetop needs an interactive terminal (TTY) to run the dashboard.");
        eprintln!("Tip: run `forgetop doctor` to check your connections.");
        return Ok(());
    }

    if demo {
        seed_demo(&config).await?;
    }

    let sections = Arc::new(SectionService::new(config.clone(), resolver.clone()));
    let health = Arc::new(ConnectionHealthService::new(config.clone(), resolver));

    let theme = config.snapshot().ui.theme.clone().unwrap_or_else(|| "slate".into());
    let deps = AppDeps { sections, health, config };
    forgetop_tui::run(deps, &theme).await
}

/// Diagnostic (`forgetop doctor`): config location, keychain access, and per-connection
/// token + connectivity — the fast way to see why a connection isn't working.
async fn doctor(config: &ConfigService, secrets: &Arc<dyn SecretStore>, resolver: &ConnectionResolver) -> Result<()> {
    let cfg = config.snapshot();
    println!("forgetop doctor\n");
    println!("Config:   {}", default_config_path().display());
    println!("          {} connection(s) configured", cfg.connections.len());
    match secrets.get("__forgetop_doctor_probe__") {
        Ok(_) => println!("Keychain: accessible\n"),
        Err(e) => println!("Keychain: NOT accessible — {e}\n"),
    }

    if cfg.connections.is_empty() {
        println!("No connections yet — run `forgetop` and press `n` to add one.");
        return Ok(());
    }

    let mut problems = 0;
    println!("Connections:");
    for conn in &cfg.connections {
        let token_found = match &conn.credential_ref {
            Some(r) => matches!(secrets.get(r), Ok(Some(_))),
            None => false,
        };
        let auth_ok = matches!(resolver.resolve(&conn.id).await, Ok(Some(live)) if live.check().await);
        if !(token_found && auth_ok) {
            problems += 1;
        }
        let token = if token_found { "found" } else { "MISSING" };
        let auth = if auth_ok { "ok" } else { "FAILED" };
        println!("  {} {:<13} {:<26} token: {:<8} auth: {}", status_mark(token_found, auth_ok), conn.provider_type.as_str(), conn.display_name, token, auth);
    }

    println!();
    if problems == 0 {
        println!("All good — every connection is authenticated and reachable.");
    } else {
        println!("{problems} connection(s) need attention — check token scopes/expiry, or re-add the token via `forgetop` → `C`.");
    }
    Ok(())
}

/// The status glyph for a connection: healthy, missing token, or auth failure.
fn status_mark(token_found: bool, auth_ok: bool) -> &'static str {
    match (token_found, auth_ok) {
        (true, true) => "✓",
        (false, _) => "⚠",
        (true, false) => "✗",
    }
}

/// Prints usage for `forgetop --help`.
fn print_help() {
    println!(
        r#"forgetop {version}
Keyboard-driven terminal UI for pull requests, work items, and CI across six forges.

Usage:
  forgetop            Launch the dashboard
  forgetop --demo     Launch with built-in demo data (no setup)
  forgetop doctor     Diagnose config, keychain access, and connection health

Options:
  -d, --demo          Run against built-in demo data
  -V, --version       Print version and exit
  -h, --help          Show this help and exit

Inside the app, press `?` for every keybinding.
Docs: https://magna-nz.github.io/forgetop/"#,
        version = env!("CARGO_PKG_VERSION")
    );
}

/// Seeds an in-memory Demo connection bound to all three sections.
async fn seed_demo(config: &ConfigService) -> Result<()> {
    // One connection per provider, named after it, so the Provider column across the
    // aggregated lists reads GitHub / GitLab / Bitbucket / Linear / Jira.
    let conn = |id: &str, provider: ProviderType, name: &str| Connection {
        id: id.into(),
        provider_type: provider,
        display_name: name.into(),
        base_url: None,
        organization: None,
        project: None,
        repository: None,
        username: None,
        credential_ref: None,
    };
    for c in [
        conn("github", ProviderType::GitHub, "GitHub"),
        conn("gitlab", ProviderType::GitLab, "GitLab"),
        conn("bitbucket", ProviderType::Bitbucket, "Bitbucket"),
        conn("linear", ProviderType::Linear, "Linear"),
        conn("jira", ProviderType::Jira, "Jira"),
    ] {
        config.add_or_update_connection(c, None).await?;
    }

    // Bind each section to the providers that serve it (mirrors the capability table).
    for id in ["github", "gitlab", "bitbucket"] {
        config.bind_pull_requests(id).await?;
    }
    for id in ["github", "gitlab", "linear", "jira"] {
        config.bind_work_items(id).await?;
    }
    for id in ["github", "gitlab", "bitbucket"] {
        config.set_pipeline_auto_discover(id, true).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::status_mark;

    #[test]
    fn status_mark_reflects_token_and_auth() {
        assert_eq!(status_mark(true, true), "✓");
        assert_eq!(status_mark(false, false), "⚠"); // no token dominates
        assert_eq!(status_mark(false, true), "⚠");
        assert_eq!(status_mark(true, false), "✗"); // token present but auth failed
    }
}
