//! forgetop provider adapters: Demo, GitHub, Azure DevOps, Linear.

use std::sync::Arc;

use forgetop_core::provider::ProviderFactory;

pub mod azure;
pub mod demo;
pub mod github;
pub mod json;
pub mod linear;

/// All provider factories, for building a `ProviderRegistry`.
pub fn default_factories() -> Vec<Arc<dyn ProviderFactory>> {
    vec![
        Arc::new(demo::DemoFactory),
        Arc::new(github::GitHubFactory),
        Arc::new(azure::AzureDevOpsFactory),
        Arc::new(linear::LinearFactory),
    ]
}
