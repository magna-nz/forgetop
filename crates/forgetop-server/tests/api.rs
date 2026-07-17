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

    let resolver = Arc::new(ConnectionResolver::new(config.clone(), registry, secrets.clone()));
    let sections = Arc::new(SectionService::new(config.clone(), resolver.clone()));
    let health = Arc::new(ConnectionHealthService::new(config.clone(), resolver));
    Deps { sections, health, config, secrets }
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

    // The SPA shell is served openly (only the API is token-gated) — an unknown route
    // falls back to index.html so client-side routing survives a refresh.
    let r = client.get(format!("{base}/pipelines")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let ct = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    assert!(ct.contains("text/html"), "index served as html, got {ct}");
    assert!(r.text().await.unwrap().contains("forgetop"), "shell mentions forgetop");
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

    // The launchpad aggregates across sections and tags each row with a triage bucket.
    let lp = get("/api/launchpad").await;
    let rows = lp.as_array().expect("launchpad is an array");
    assert!(!rows.is_empty(), "launchpad returns triaged rows");
    assert!(rows[0]["bucket"].is_string(), "row carries a bucket key");
    assert!(rows[0]["kind"].is_string(), "row carries an item kind (flattened)");
}

#[tokio::test]
async fn pr_detail_and_write_actions_reach_the_provider() {
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();

    // Pick a real PR from the list.
    let prs: serde_json::Value = client
        .get(format!("{base}/api/pull-requests"))
        .header("x-forgetop-token", &tok)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pr = &prs[0];
    let conn = pr["connection_id"].as_str().unwrap();
    let id = pr["pull_request"]["id"].as_str().unwrap();

    let detail_url = format!("{base}/api/pr/detail?conn={conn}&id={id}");
    let detail = client.get(&detail_url).header("x-forgetop-token", &tok).send().await.unwrap();
    assert_eq!(detail.status(), 200);
    let detail: serde_json::Value = detail.json().await.unwrap();
    assert_eq!(detail["pull_request"]["id"], id);
    assert!(detail["changes"].is_array() && detail["commits"].is_array());

    // Submitting a review with a line comment (the demo persists it as a thread).
    let review = client
        .post(format!("{base}/api/pr/review"))
        .header("x-forgetop-token", &tok)
        .json(&serde_json::json!({
            "conn": conn, "id": id, "event": "Approved",
            "comments": [{ "path": "src/lib.rs", "line": 3, "side": "New", "body": "nice" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(review.status(), 200);

    // It now comes back in the detail threads.
    let after: serde_json::Value = client.get(&detail_url).header("x-forgetop-token", &tok).send().await.unwrap().json().await.unwrap();
    let has_comment = after["threads"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["comments"].as_array().map(|c| c.iter().any(|x| x["body"] == "nice")).unwrap_or(false));
    assert!(has_comment, "the submitted line comment is persisted and returned");

    // Writes are gated by the token like everything under /api.
    let unauth = client
        .post(format!("{base}/api/pr/merge"))
        .json(&serde_json::json!({ "conn": conn, "id": id }))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401, "an unauthenticated merge is rejected");

    // A bad connection id is a 404, not a 500.
    let missing = client
        .post(format!("{base}/api/pr/vote"))
        .header("x-forgetop-token", &tok)
        .json(&serde_json::json!({ "conn": "nope", "id": id, "vote": "Approved" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn connection_management_round_trip_and_never_leaks_the_token() {
    // Start empty (no connections) and set one up entirely through the API.
    let server = spawn(demo_deps(false).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let hdr = |r: reqwest::RequestBuilder| r.header("x-forgetop-token", &tok);

    // Provider schema is available and includes a secret field for GitHub.
    let provs: serde_json::Value = hdr(client.get(format!("{base}/api/providers"))).send().await.unwrap().json().await.unwrap();
    let github = provs.as_array().unwrap().iter().find(|p| p["provider"] == "GitHub").expect("GitHub offered");
    assert!(github["fields"].as_array().unwrap().iter().any(|f| f["secret"] == true));

    // Unauthenticated writes are refused.
    let unauth = client
        .post(format!("{base}/api/connections"))
        .json(&serde_json::json!({ "provider": "GitHub", "display_name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);

    // Create a GitHub connection with a token, bound to Pull Requests.
    const SECRET: &str = "ghp_thisisasupersecrettoken";
    let saved: serde_json::Value = hdr(client.post(format!("{base}/api/connections")))
        .json(&serde_json::json!({
            "provider": "GitHub", "display_name": "Work GitHub", "repository": "acme/app",
            "token": SECRET, "bind_pull_requests": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = saved["id"].as_str().expect("returns the new id").to_string();

    // It shows up, reports a token is set + the binding — and the raw response NEVER contains the token.
    let raw = hdr(client.get(format!("{base}/api/connections"))).send().await.unwrap().text().await.unwrap();
    assert!(!raw.contains(SECRET), "the token must never be returned by the API");
    let conns: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let row = conns.as_array().unwrap().iter().find(|c| c["id"] == id.as_str()).expect("connection listed");
    assert_eq!(row["has_token"], true);
    assert_eq!(row["display_name"], "Work GitHub");
    assert!(row["sections"].as_array().unwrap().iter().any(|s| s == "pull_requests"));

    // Test-connection round-trips (demo GitHub authenticates).
    let health: serde_json::Value =
        hdr(client.post(format!("{base}/api/connections/test"))).json(&serde_json::json!({ "id": id })).send().await.unwrap().json().await.unwrap();
    assert!(health["healthy"].is_boolean());

    // Delete removes it.
    let del = hdr(client.post(format!("{base}/api/connections/delete"))).json(&serde_json::json!({ "id": id })).send().await.unwrap();
    assert_eq!(del.status(), 200);
    let after: serde_json::Value = hdr(client.get(format!("{base}/api/connections"))).send().await.unwrap().json().await.unwrap();
    assert!(after.as_array().unwrap().iter().all(|c| c["id"] != id.as_str()), "connection is gone");
}

#[tokio::test]
async fn startup_preference_round_trips() {
    let server = spawn(demo_deps(false).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let hdr = |r: reqwest::RequestBuilder| r.header("x-forgetop-token", &tok);

    // Defaults to "both".
    let prefs: serde_json::Value = hdr(client.get(format!("{base}/api/preferences"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(prefs["startup_mode"], "both");

    // Change it, and it sticks.
    let set = hdr(client.post(format!("{base}/api/preferences/startup")))
        .json(&serde_json::json!({ "mode": "terminal_only" }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), 200);
    let after: serde_json::Value = hdr(client.get(format!("{base}/api/preferences"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(after["startup_mode"], "terminal_only");

    // Reading preferences still needs the token.
    let unauth = client.get(format!("{base}/api/preferences")).send().await.unwrap();
    assert_eq!(unauth.status(), 401);
}

#[tokio::test]
async fn work_item_pipeline_and_notification_writes() {
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let hdr = |r: reqwest::RequestBuilder| r.header("x-forgetop-token", &tok);

    // Work item: read available states, then transition.
    let wis: serde_json::Value =
        hdr(client.get(format!("{base}/api/work-items"))).send().await.unwrap().json().await.unwrap();
    let wi = &wis[0];
    let (wconn, wid) = (wi["connection_id"].as_str().unwrap(), wi["work_item"]["id"].as_str().unwrap());
    let states: serde_json::Value =
        hdr(client.get(format!("{base}/api/wi/states?conn={wconn}&id={wid}"))).send().await.unwrap().json().await.unwrap();
    assert!(states.is_array(), "available states is an array");
    let set = hdr(client.post(format!("{base}/api/wi/state")))
        .json(&serde_json::json!({ "conn": wconn, "id": wid, "state": "In Progress" }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), 200);

    // Pipeline: trigger a run for a discovered definition.
    let pipes: serde_json::Value =
        hdr(client.get(format!("{base}/api/pipelines"))).send().await.unwrap().json().await.unwrap();
    let pipe = &pipes[0];
    let (pconn, def) = (pipe["connection_id"].as_str().unwrap(), pipe["run"]["definition_id"].as_str().unwrap());
    let trig = hdr(client.post(format!("{base}/api/pipeline/trigger")))
        .json(&serde_json::json!({ "conn": pconn, "definition_id": def }))
        .send()
        .await
        .unwrap();
    assert_eq!(trig.status(), 200);

    // The pipelines list surfaces the approval gate on the in-flight run, and we can respond to it.
    let gate = pipes
        .as_array()
        .unwrap()
        .iter()
        .find_map(|p| p["approvals"].as_array().and_then(|a| a.first()).map(|g| (p["connection_id"].clone(), p["run"]["id"].clone(), g["id"].clone())))
        .expect("a run is waiting on an approval gate");
    let approve = hdr(client.post(format!("{base}/api/pipeline/approval")))
        .json(&serde_json::json!({ "conn": gate.0, "run_id": gate.1, "approval_id": gate.2, "decision": "Approve" }))
        .send()
        .await
        .unwrap();
    assert_eq!(approve.status(), 200);

    // Notification: mark one read.
    let notifs: serde_json::Value =
        hdr(client.get(format!("{base}/api/notifications"))).send().await.unwrap().json().await.unwrap();
    let n = &notifs[0];
    let read = hdr(client.post(format!("{base}/api/notification/read")))
        .json(&serde_json::json!({ "conn": n["connection_id"], "id": n["notification"]["id"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
}

#[tokio::test]
async fn work_item_and_pipeline_detail_reach_the_provider() {
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let hdr = |r: reqwest::RequestBuilder| r.header("x-forgetop-token", &tok);

    // --- work item detail ---
    let wis: serde_json::Value =
        hdr(client.get(format!("{base}/api/work-items"))).send().await.unwrap().json().await.unwrap();
    let wi = &wis[0];
    let (wconn, wid) = (wi["connection_id"].as_str().unwrap(), wi["work_item"]["id"].as_str().unwrap());

    let wi_detail_url = format!("{base}/api/wi/detail?conn={wconn}&id={wid}");
    let detail = hdr(client.get(&wi_detail_url)).send().await.unwrap();
    assert_eq!(detail.status(), 200);
    let detail: serde_json::Value = detail.json().await.unwrap();
    assert_eq!(detail["work_item"]["id"], wid);
    assert!(detail["threads"].is_array(), "detail carries a threads array");

    // A comment posted through the API comes back on the item's threads (the demo persists it).
    let comment = hdr(client.post(format!("{base}/api/wi/comment")))
        .json(&serde_json::json!({ "conn": wconn, "id": wid, "body": "looking into this" }))
        .send()
        .await
        .unwrap();
    assert_eq!(comment.status(), 200);
    let after: serde_json::Value = hdr(client.get(&wi_detail_url)).send().await.unwrap().json().await.unwrap();
    let has_comment = after["threads"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["comments"].as_array().map(|c| c.iter().any(|x| x["body"] == "looking into this")).unwrap_or(false));
    assert!(has_comment, "the posted work-item comment is persisted and returned");

    // --- pipeline detail + logs ---
    let pipes: serde_json::Value =
        hdr(client.get(format!("{base}/api/pipelines"))).send().await.unwrap().json().await.unwrap();
    let pipe = &pipes[0];
    let (pconn, run_id) = (pipe["connection_id"].as_str().unwrap(), pipe["run"]["id"].as_str().unwrap());

    let detail = hdr(client.get(format!("{base}/api/pipeline/detail?conn={pconn}&run_id={run_id}"))).send().await.unwrap();
    assert_eq!(detail.status(), 200);
    let detail: serde_json::Value = detail.json().await.unwrap();
    assert_eq!(detail["run"]["id"], run_id);
    assert!(detail["run"]["stages"].is_array(), "run carries a stages array");
    assert!(detail["approvals"].is_array(), "detail carries an approvals array");

    // Logs for the first job on the run (fall back to the whole run if it has no jobs).
    let job = detail["run"]["stages"]
        .as_array()
        .and_then(|s| s.iter().find_map(|st| st["jobs"].as_array().and_then(|j| j.first()).map(|j| j["id"].as_str().unwrap().to_string())));
    let logs_url = match &job {
        Some(j) => format!("{base}/api/pipeline/logs?conn={pconn}&run_id={run_id}&job={j}"),
        None => format!("{base}/api/pipeline/logs?conn={pconn}&run_id={run_id}"),
    };
    let logs = hdr(client.get(&logs_url)).send().await.unwrap();
    assert_eq!(logs.status(), 200);
    assert!(logs.text().await.unwrap().contains("logs for run"), "logs come back as plain text");

    // --- token gating + missing-connection handling ---
    let unauth = client.get(&wi_detail_url).send().await.unwrap();
    assert_eq!(unauth.status(), 401, "detail is token-gated");
    let missing_wi = hdr(client.get(format!("{base}/api/wi/detail?conn=nope&id={wid}"))).send().await.unwrap();
    assert_eq!(missing_wi.status(), 404, "a bad connection is a 404, not a 500");
    let missing_pipe = hdr(client.get(format!("{base}/api/pipeline/detail?conn=nope&run_id={run_id}"))).send().await.unwrap();
    assert_eq!(missing_pipe.status(), 404);
}
