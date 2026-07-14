//! Wave-1 API tests: the server boots on localhost, enforces the session token, and every
//! read endpoint returns JSON. Uses an empty config (no bound connections), so the arrays
//! come back empty — the point here is the plumbing (routing, auth, serialization), not data.

use std::sync::Arc;

use forgetop_core::config::{ConfigStore, ForgetopConfig, InMemoryConfigStore};
use forgetop_core::domain::ProviderType;
use forgetop_core::provider::{Connection, ProviderRegistry};
use forgetop_core::secret::{InMemorySecretStore, SecretStore};
use forgetop_core::service::{ConfigService, ConnectionHealthService, ConnectionResolver, SectionService};
use forgetop_providers::demo::demo_factories;
use forgetop_server::{spawn, Deps};

/// Builds services over the in-memory demo providers. When `seed` is set, wires one
/// connection per provider bound to every section — the same shape as `forgetop --demo`.
async fn demo_deps(seed: bool) -> Deps {
    let registry = Arc::new(ProviderRegistry::new(demo_factories()));
    let store: Arc<dyn ConfigStore> = Arc::new(InMemoryConfigStore::new(ForgetopConfig::default()));
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::default());
    let config = Arc::new(ConfigService::new(store, secrets.clone(), registry.clone()));
    config.load().await.unwrap();

    if seed {
        let conn = |id: &str, provider: ProviderType| Connection {
            id: id.into(),
            provider_type: provider,
            display_name: id.into(),
            base_url: None,
            organization: None,
            project: None,
            repository: None,
            username: None,
            credential_ref: None,
        };
        config.add_or_update_connection(conn("github", ProviderType::GitHub), None).await.unwrap();
        config.add_or_update_connection(conn("linear", ProviderType::Linear), None).await.unwrap();
        config.bind_pull_requests("github").await.unwrap();
        config.bind_work_items("linear").await.unwrap();
        config.set_pipeline_auto_discover("github", true).await.unwrap();
    }

    let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry, secrets));
    let sections = Arc::new(SectionService::new(config.clone(), resolver.clone()));
    let health = Arc::new(ConnectionHealthService::new(config, resolver));
    Deps { sections, health }
}

async fn empty_deps() -> Deps {
    demo_deps(false).await
}

#[tokio::test]
async fn serves_json_and_enforces_the_session_token() {
    let server = spawn(empty_deps().await, 0).await.expect("server binds a free port");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    // No token → 401.
    let r = client.get(format!("{base}/api/pull-requests")).send().await.unwrap();
    assert_eq!(r.status(), 401, "unauthenticated request is rejected");

    // Wrong token → 401.
    let r = client.get(format!("{base}/api/health?t=nope")).send().await.unwrap();
    assert_eq!(r.status(), 401);

    // Token via query → 200 + a JSON array.
    let r = client.get(format!("{base}/api/pull-requests?t={}", server.token)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.json::<serde_json::Value>().await.unwrap().is_array());

    // Token via header works on every read endpoint.
    for path in ["/api/health", "/api/work-items", "/api/pipelines", "/api/notifications"] {
        let r = client
            .get(format!("{base}{path}"))
            .header("x-forgetop-token", &server.token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "{path} authorized");
        assert!(r.json::<serde_json::Value>().await.unwrap().is_array(), "{path} returns an array");
    }

    // The placeholder page loads with the token.
    let r = client.get(format!("{base}/?t={}", server.token)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.text().await.unwrap().contains("forgetop dashboard"));
}

#[tokio::test]
async fn seeded_demo_data_reaches_the_api() {
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    let get = |path: &str| {
        let url = format!("{base}{path}?t={}", server.token);
        let client = client.clone();
        async move { client.get(url).send().await.unwrap().json::<serde_json::Value>().await.unwrap() }
    };

    // A bound source per section returns rows, and each row carries its connection id.
    for (path, id) in [("/api/pull-requests", "github"), ("/api/work-items", "linear")] {
        let rows = get(path).await;
        let rows = rows.as_array().unwrap_or_else(|| panic!("{path} is an array"));
        assert!(!rows.is_empty(), "{path} returns demo rows");
        assert_eq!(rows[0]["connection_id"], id, "{path} tags the connection");
    }

    // Health reflects the two seeded connections.
    let health = get("/api/health").await;
    assert_eq!(health.as_array().unwrap().len(), 2);
}
