//! Shared plumbing for the live integration suite: `.env` loading, credential
//! gating, a per-run resource prefix, and provider connection builders.

use std::sync::{Arc, OnceLock};

use forgetop_core::domain::ProviderType;
use forgetop_core::provider::{Connection, ProviderConnection, ProviderRegistry};

/// Loads `.env` once (best-effort) so local runs pick up credentials. In CI the
/// variables come straight from the environment and there's no file — that's fine.
pub fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = dotenvy::dotenv();
    });
}

/// A read of a non-empty environment variable.
pub fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// A stable, unique prefix for every resource this test run creates, so writes are
/// identifiable and a leaked fixture can be swept later. Shape: `forgetop-it-<hex>`.
/// (Used from Wave 2 onward, when tests start creating fixtures.)
#[allow(dead_code)]
pub fn run_prefix() -> &'static str {
    static PREFIX: OnceLock<String> = OnceLock::new();
    PREFIX.get_or_init(|| {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        // Short, lowercase, filesystem/ref-safe.
        format!("forgetop-it-{:x}", nanos as u64 & 0xffff_ffff)
    })
}

/// The prefix used to recognise *any* run's leftover fixtures for sweeping.
/// (Used from Wave 2 onward.)
#[allow(dead_code)]
pub const SWEEP_PREFIX: &str = "forgetop-it-";

/// A registry wired with the real provider factories.
pub fn registry() -> ProviderRegistry {
    ProviderRegistry::new(forgetop_providers::default_factories())
}

/// A live GitHub connection built from `FORGETOP_IT_GITHUB_*`, or `None` to skip.
pub struct GitHubIt {
    pub owner: String,
    pub repo: String,
    pub conn: Arc<dyn ProviderConnection>,
}

pub fn github() -> Option<GitHubIt> {
    init();
    let token = env("FORGETOP_IT_GITHUB_TOKEN")?;
    let full = env("FORGETOP_IT_GITHUB_REPO")?; // owner/repo
    let (owner, repo) = full.split_once('/')?;
    let conn = Connection {
        id: "it-github".into(),
        provider_type: ProviderType::GitHub,
        display_name: "IT GitHub".into(),
        base_url: env("FORGETOP_IT_GITHUB_HOST"),
        organization: Some(owner.to_string()),
        project: None,
        repository: Some(repo.to_string()),
        username: None,
        credential_ref: None,
    };
    let conn = registry().create(&conn, Some(token)).ok()?;
    Some(GitHubIt { owner: owner.to_string(), repo: repo.to_string(), conn })
}

/// Polls `f` every 2s until it yields `Some`, or `timeout_secs` elapses (→ `None`).
/// Used to wait on eventually-consistent API state without fixed sleeps.
#[allow(dead_code)]
pub async fn poll<T, F, Fut>(timeout_secs: u64, mut f: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let start = std::time::Instant::now();
    loop {
        if let Some(v) = f().await {
            return Some(v);
        }
        if start.elapsed().as_secs() >= timeout_secs {
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Skips the current test (returns) with a note when its credentials are absent.
#[macro_export]
macro_rules! skip_if_none {
    ($opt:expr, $provider:expr) => {
        match $opt {
            Some(v) => v,
            None => {
                eprintln!("SKIP {}: credentials not set in the environment", $provider);
                return;
            }
        }
    };
}
