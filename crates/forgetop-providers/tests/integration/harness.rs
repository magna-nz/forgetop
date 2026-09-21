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

/// The providers this run *requires*, from `FORGETOP_IT_PROVIDERS` (comma-separated,
/// e.g. `github,gitlab,azure,linear`). A provider named here whose credentials are
/// absent is a hard **failure**, not a skip.
///
/// This exists because a skipped provider is otherwise indistinguishable from a passing
/// one: the suite exits 0 either way, so a credential that was never added (or that
/// silently lapsed) reads as green. Naming a provider is the assertion that it really ran.
/// Providers left out of the list keep the old behaviour and skip quietly when unset.
pub fn required_providers() -> &'static [String] {
    static REQUIRED: OnceLock<Vec<String>> = OnceLock::new();
    REQUIRED.get_or_init(|| {
        init();
        env("FORGETOP_IT_PROVIDERS")
            .map(|raw| raw.split(',').map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty()).collect())
            .unwrap_or_default()
    })
}

/// Whether `provider` was named in `FORGETOP_IT_PROVIDERS`.
pub fn is_required(provider: &str) -> bool {
    // Exact match only. A qualified label ("azure pipeline") names an *optional* extra that
    // needs its own variable beyond the provider's credentials, so requiring "azure" must not
    // drag it in — listing the full label is how you'd opt into requiring one.
    required_providers().contains(&provider.to_ascii_lowercase())
}

/// Announces a credential-absence skip, or panics when the provider is required.
///
/// The counterpart to `skip_if_none!` for call sites that build their connection with
/// `let ... else` instead of the macro. Call it, then `return`.
pub fn skip_absent(provider: &str, detail: &str) {
    if is_required(provider) {
        panic!("{provider} is listed in FORGETOP_IT_PROVIDERS but its credentials are not set ({detail}) — add them, or drop it from the list");
    }
    eprintln!("SKIP {provider}: {detail}");
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

/// Parses `FORGETOP_IT_GITHUB_REPO` — accepts `owner/repo`, a full
/// `https://github.com/owner/repo[.git]` URL, or `git@…:owner/repo`.
pub fn github_owner_repo() -> Option<(String, String)> {
    let raw = env("FORGETOP_IT_GITHUB_REPO")?;
    let cleaned = raw.trim().trim_end_matches('/').trim_end_matches(".git");
    let parts: Vec<&str> = cleaned.split('/').filter(|p| !p.is_empty() && !p.contains(':') && *p != "github.com").collect();
    match parts.as_slice() {
        [.., owner, repo] => Some((owner.to_string(), repo.to_string())),
        _ => None,
    }
}

pub fn github() -> Option<GitHubIt> {
    init();
    let token = env("FORGETOP_IT_GITHUB_TOKEN")?;
    let (owner, repo) = github_owner_repo()?;
    let conn = Connection {
        id: "it-github".into(),
        provider_type: ProviderType::GitHub,
        display_name: "IT GitHub".into(),
        base_url: env("FORGETOP_IT_GITHUB_HOST"),
        organization: Some(owner.clone()),
        project: None,
        repository: Some(repo.clone()),
        username: None,
        credential_ref: None,
        // Live suites address a single repository, so the legacy fallback is what we exercise.
        repo_scope: None,
    };
    let conn = registry().create(&conn, Some(token)).ok()?;
    Some(GitHubIt { owner, repo, conn })
}

/// A live GitLab connection from `FORGETOP_IT_GITLAB_*`, or `None` to skip.
pub struct GitLabIt {
    pub project: String,
    pub conn: Arc<dyn ProviderConnection>,
}

pub fn gitlab() -> Option<GitLabIt> {
    init();
    let token = env("FORGETOP_IT_GITLAB_TOKEN")?;
    let project = env("FORGETOP_IT_GITLAB_PROJECT")?;
    let base_url = env("FORGETOP_IT_GITLAB_HOST").map(|h| format!("{}/api/v4", h.trim_end_matches('/')));
    let conn = Connection {
        id: "it-gitlab".into(),
        provider_type: ProviderType::GitLab,
        display_name: "IT GitLab".into(),
        base_url,
        organization: None,
        project: None,
        repository: Some(project.clone()),
        username: None,
        credential_ref: None,
        // Live suites address a single repository, so the legacy fallback is what we exercise.
        repo_scope: None,
    };
    let conn = registry().create(&conn, Some(token)).ok()?;
    Some(GitLabIt { project, conn })
}

/// A live Azure DevOps connection from `FORGETOP_IT_AZURE_*`, or `None` to skip.
pub struct AzureIt {
    pub org: String,
    pub project: String,
    pub conn: Arc<dyn ProviderConnection>,
}

pub fn azure() -> Option<AzureIt> {
    init();
    let pat = env("FORGETOP_IT_AZURE_PAT")?;
    let org = env("FORGETOP_IT_AZURE_ORG")?;
    let project = env("FORGETOP_IT_AZURE_PROJECT")?;
    let repo = env("FORGETOP_IT_AZURE_REPO").unwrap_or_else(|| project.clone());
    let conn = Connection {
        id: "it-azure".into(),
        provider_type: ProviderType::AzureDevOps,
        display_name: "IT Azure".into(),
        base_url: None,
        organization: Some(org.clone()),
        project: Some(project.clone()),
        repository: Some(repo),
        username: None,
        credential_ref: None,
        // Live suites address a single repository, so the legacy fallback is what we exercise.
        repo_scope: None,
    };
    let conn = registry().create(&conn, Some(pat)).ok()?;
    Some(AzureIt { org, project, conn })
}

/// A live Linear connection from `FORGETOP_IT_LINEAR_KEY`, or `None` to skip.
pub struct LinearIt {
    pub conn: Arc<dyn ProviderConnection>,
}

pub fn linear() -> Option<LinearIt> {
    init();
    let key = env("FORGETOP_IT_LINEAR_KEY")?;
    let conn = Connection {
        id: "it-linear".into(),
        provider_type: ProviderType::Linear,
        display_name: "IT Linear".into(),
        base_url: None,
        organization: None,
        project: None,
        repository: None,
        username: None,
        credential_ref: None,
        // Live suites address a single repository, so the legacy fallback is what we exercise.
        repo_scope: None,
    };
    let conn = registry().create(&conn, Some(key)).ok()?;
    Some(LinearIt { conn })
}

/// A live Jira connection from `FORGETOP_IT_JIRA_*`, or `None` to skip.
pub struct JiraIt {
    pub project: String,
    pub conn: Arc<dyn ProviderConnection>,
}

pub fn jira() -> Option<JiraIt> {
    init();
    let token = env("FORGETOP_IT_JIRA_TOKEN")?;
    let site = env("FORGETOP_IT_JIRA_SITE")?;
    let email = env("FORGETOP_IT_JIRA_EMAIL")?;
    let project = env("FORGETOP_IT_JIRA_PROJECT")?;
    let conn = Connection {
        id: "it-jira".into(),
        provider_type: ProviderType::Jira,
        display_name: "IT Jira".into(),
        base_url: Some(site),
        organization: None,
        project: Some(project.clone()),
        repository: None,
        username: Some(email),
        credential_ref: None,
        repo_scope: None,
    };
    let conn = registry().create(&conn, Some(token)).ok()?;
    Some(JiraIt { project, conn })
}

/// Runs a provider's sweep future only when `FORGETOP_IT_SWEEP` is set. Sweeping
/// deletes *all* `forgetop-it-*` fixtures, which is unsafe when CI runs concurrently
/// (one run would nuke another's in-flight fixtures), so normal runs rely on each
/// test's own teardown. Set the var locally or in a scheduled cleanup job.
pub async fn maybe_sweep<F: std::future::Future<Output = ()>>(sweep: F) {
    if env("FORGETOP_IT_SWEEP").is_some() {
        sweep.await;
    }
}

/// Poll windows sized for real APIs that are eventually-consistent and occasionally
/// slow. Generous on purpose — a degraded-but-working API shouldn't flake a test
/// (a whole run has been observed at ~4× normal duration when a provider is degraded).
pub const POLL_LIST: u64 = 45; // find a just-created item / commits / a gate clearing
pub const POLL_MERGE: u64 = 90; // a PR/MR settling to "merged"
pub const POLL_GATE: u64 = 180; // a pipeline run reaching its approval gate
pub const POLL_RUNNER: u64 = 300; // a dispatched run getting picked up by a hosted runner
pub const POLL_CANCEL: u64 = 240; // a cancelled run settling — the runner tears down a job that
                                  // may still be mid-`sleep`, so this sits well above the fixture's 120s

/// Retries a fallible write until it succeeds, or `timeout_secs` elapses (→ the last error).
///
/// Live write APIs reject occasionally for reasons that have nothing to do with the code
/// under test (Linear's transient `cannot delegate to Linear`, provider-side rate limits).
/// `poll` covers *reads* settling; this covers a *write* that deserves a second attempt.
#[allow(dead_code)]
pub async fn retry_write<T, E, F, Fut>(timeout_secs: u64, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let start = std::time::Instant::now();
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if start.elapsed().as_secs() >= timeout_secs {
                    return Err(e);
                }
                eprintln!("retrying after transient write error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
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
                // Named in FORGETOP_IT_PROVIDERS? Then its absence is a failure, not a skip —
                // otherwise a missing credential would quietly read as a pass.
                if $crate::harness::is_required($provider) {
                    panic!(
                        "{} is listed in FORGETOP_IT_PROVIDERS but its credentials are not set — \
                         add them, or drop it from the list",
                        $provider
                    );
                }
                eprintln!("SKIP {}: credentials not set in the environment", $provider);
                return;
            }
        }
    };
}

/// A live Bitbucket connection from `FORGETOP_IT_BITBUCKET_*`, or `None` to skip.
///
/// Bitbucket is the one provider CI has no secrets for, so this exists to make its discovery
/// path one `.env` paste away from being covered rather than permanently unverified.
#[allow(dead_code)]
pub struct BitbucketIt {
    pub workspace: String,
    pub repo: String,
    pub conn: Arc<dyn ProviderConnection>,
}

#[allow(dead_code)]
pub fn bitbucket() -> Option<BitbucketIt> {
    init();
    let username = env("FORGETOP_IT_BITBUCKET_USERNAME")?;
    let app_password = env("FORGETOP_IT_BITBUCKET_APP_PASSWORD")?;
    let workspace = env("FORGETOP_IT_BITBUCKET_WORKSPACE")?;
    let repo = env("FORGETOP_IT_BITBUCKET_REPO")?;
    let conn = Connection {
        id: "it-bitbucket".into(),
        provider_type: ProviderType::Bitbucket,
        display_name: "IT Bitbucket".into(),
        base_url: None,
        organization: Some(workspace.clone()),
        project: None,
        repository: Some(repo.clone()),
        username: Some(username),
        credential_ref: None,
        repo_scope: None,
    };
    let conn = registry().create(&conn, Some(app_password)).ok()?;
    Some(BitbucketIt { workspace, repo, conn })
}

/// A live connection with an explicit **repository scope**, for the account-scope tests.
/// `scope` entries are connection-relative (`owner/repo`, `group/project`, `project/repo`).
#[allow(dead_code)]
pub fn scoped(provider: ProviderType, scope: Vec<String>) -> Option<Arc<dyn ProviderConnection>> {
    init();
    let mut conn = match provider {
        ProviderType::GitHub => Connection {
            id: "it-github-scoped".into(),
            provider_type: provider,
            display_name: "IT GitHub".into(),
            base_url: env("FORGETOP_IT_GITHUB_HOST"),
            organization: None,
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
            repo_scope: None,
        },
        ProviderType::AzureDevOps => Connection {
            id: "it-azure-scoped".into(),
            provider_type: provider,
            display_name: "IT Azure".into(),
            base_url: None,
            organization: Some(env("FORGETOP_IT_AZURE_ORG")?),
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
            repo_scope: None,
        },
        ProviderType::GitLab => Connection {
            id: "it-gitlab-scoped".into(),
            provider_type: provider,
            display_name: "IT GitLab".into(),
            base_url: env("FORGETOP_IT_GITLAB_HOST").map(|h| format!("{}/api/v4", h.trim_end_matches('/'))),
            organization: None,
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
            repo_scope: None,
        },
        _ => return None,
    };
    conn.repo_scope = Some(scope);
    let secret = match provider {
        ProviderType::GitHub => env("FORGETOP_IT_GITHUB_TOKEN")?,
        ProviderType::AzureDevOps => env("FORGETOP_IT_AZURE_PAT")?,
        ProviderType::GitLab => env("FORGETOP_IT_GITLAB_TOKEN")?,
        _ => return None,
    };
    registry().create(&conn, Some(secret)).ok()
}
