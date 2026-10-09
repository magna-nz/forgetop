//! forgetop provider adapters: Demo, GitHub, Azure DevOps, Linear, GitLab, Jira, Bitbucket.

use std::sync::Arc;

use forgetop_core::provider::ProviderFactory;

pub mod azure;
pub mod bitbucket;
mod content;
pub mod demo;
pub mod github;
pub mod gitlab;
pub mod html;
pub mod jira;
pub mod json;
pub mod linear;
pub mod scope;
#[cfg(test)]
pub(crate) mod test_http;

/// An HTTP client for a provider, with its auth headers and a ceiling on every request.
///
/// Without the ceiling one request that never answers — a half-open connection, a proxy that
/// swallows the reply — hangs the whole refresh: the terminal fetches single-flight, so every
/// later refresh waits on it and the list sits on "Loading…" for good.
///
/// The ceiling is on silence, not on the whole request: a job log of tens of megabytes on a
/// slow link is still arriving, and a total timeout would cut it off part-way.
pub(crate) fn http_client(headers: reqwest::header::HeaderMap) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(30))
        .build()
}

/// All real provider factories, for building a `ProviderRegistry`. The demo provider is
/// registered separately (see [`demo::demo_factories`]) only under `--demo`.
pub fn default_factories() -> Vec<Arc<dyn ProviderFactory>> {
    vec![
        Arc::new(github::GitHubFactory),
        Arc::new(azure::AzureDevOpsFactory),
        Arc::new(linear::LinearFactory),
        Arc::new(gitlab::GitLabFactory),
        Arc::new(jira::JiraFactory),
        Arc::new(bitbucket::BitbucketFactory),
    ]
}
