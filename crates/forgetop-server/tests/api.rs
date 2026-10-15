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
            repo_scope: None,
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
    Deps { sections, health, config, secrets, cache: std::sync::Arc::new(forgetop_core::cache::CacheStore::disabled()) }
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

    // The launchpad aggregates across sections and tags each row with a triage bucket, plus
    // per-bucket overflow flags for the "more…" links.
    let lp = get("/api/launchpad").await;
    let rows = lp["rows"].as_array().expect("launchpad has a rows array");
    assert!(!rows.is_empty(), "launchpad returns triaged rows");
    assert!(rows[0]["bucket"].is_string(), "row carries a bucket key");
    assert!(rows[0]["kind"].is_string(), "row carries an item kind (flattened)");
    assert!(lp["more"]["your_work"].is_boolean(), "carries overflow flags");
}

#[tokio::test]
async fn pr_views_map_to_provider_queries() {
    // The three PR-page views resolve through the provider's `list(query)` — the same path the
    // TUI uses — so this exercises the real filtering, not a client-side slice.
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    let get = |view: &str| {
        let url = format!("{base}/api/pull-requests?view={view}&t={}", server.token);
        let client = client.clone();
        async move { client.get(url).send().await.unwrap().json::<serde_json::Value>().await.unwrap() }
    };
    let rows = |v: &serde_json::Value| v.as_array().unwrap().clone();

    // All: open PRs across connections — nothing merged.
    let all = rows(&get("all").await);
    assert!(!all.is_empty(), "all view returns rows");
    assert!(all.iter().all(|r| r["pull_request"]["status"] != "Merged"), "all view excludes merged");

    // Your PRs: your own open PRs — authored by you, none merged.
    let yours = rows(&get("yours").await);
    assert!(!yours.is_empty(), "your-PRs view returns rows");
    assert!(yours.iter().all(|r| r["pull_request"]["author"]["handle"] == "you"), "your-PRs view is authored by you");
    assert!(yours.iter().all(|r| r["pull_request"]["status"] != "Merged"), "your-PRs view excludes merged");

    // Recently merged by you: every row is merged and authored by you.
    let merged = rows(&get("merged").await);
    assert!(!merged.is_empty(), "merged view returns your merged PRs");
    assert!(merged.iter().all(|r| r["pull_request"]["status"] == "Merged"), "merged view is only merged");
    assert!(merged.iter().all(|r| r["pull_request"]["author"]["handle"] == "you"), "merged view is authored by you");

    // Review requested: every row lists you as a reviewer.
    let review = rows(&get("review_requested").await);
    assert!(!review.is_empty(), "review-requested view returns rows");
    assert!(
        review
            .iter()
            .all(|r| r["pull_request"]["reviewers"].as_array().unwrap().iter().any(|rv| rv["user"]["handle"] == "you")),
        "review-requested rows list you as a reviewer",
    );
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
    // The detail carries a timeline of events (the demo returns reviews/approvals).
    let timeline = detail["timeline"].as_array().expect("timeline array");
    assert!(!timeline.is_empty() && timeline[0]["kind"].is_string() && timeline[0]["summary"].is_string());

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

#[tokio::test]
async fn pipeline_capabilities_annotations_artifacts_and_rerun() {
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let hdr = |r: reqwest::RequestBuilder| r.header("x-forgetop-token", &tok);
    let detail_url = format!("{base}/api/pipeline/detail?conn=github&run_id=r500");

    // Detail carries the provider's capabilities.
    let detail: serde_json::Value = hdr(client.get(&detail_url)).send().await.unwrap().json().await.unwrap();
    let caps = &detail["capabilities"];
    assert_eq!(caps["supports_rerun"], true);
    assert_eq!(caps["supports_artifacts"], true);
    assert_eq!(caps["supports_approvals"], true);
    assert_eq!(caps["can_respond_approvals"], true);
    for key in ["supports_rerun_failed", "rerun_starts_new_run", "rerun_failed_starts_new_run"] {
        assert!(caps[key].is_boolean(), "capabilities carries {key}");
    }

    // Annotations: the failed run has some, a run without any returns an empty array.
    let annotations: serde_json::Value =
        hdr(client.get(format!("{base}/api/pipeline/annotations?conn=github&run_id=r500"))).send().await.unwrap().json().await.unwrap();
    assert!(!annotations.as_array().unwrap().is_empty(), "r500 has annotations");
    let none: serde_json::Value =
        hdr(client.get(format!("{base}/api/pipeline/annotations?conn=github&run_id=r493"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(none, serde_json::json!([]));

    // Artifacts.
    let artifacts: serde_json::Value =
        hdr(client.get(format!("{base}/api/pipeline/artifacts?conn=github&run_id=r207"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(artifacts.as_array().unwrap().len(), 7);

    // Re-run: the demo re-queues the same run (no new run id), and the detail reflects it.
    let rerun = hdr(client.post(format!("{base}/api/pipeline/rerun")))
        .json(&serde_json::json!({ "conn": "github", "run_id": "r500", "failed_only": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(rerun.status(), 200);
    let rerun: serde_json::Value = rerun.json().await.unwrap();
    assert_eq!(rerun["ok"], true);
    assert!(rerun["new_run_id"].is_null());
    let after: serde_json::Value = hdr(client.get(&detail_url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(after["run"]["status"], "Queued");

    // Unknown connection → 404.
    let missing_annotations = hdr(client.get(format!("{base}/api/pipeline/annotations?conn=nope&run_id=r500"))).send().await.unwrap();
    assert_eq!(missing_annotations.status(), 404);
    let missing_rerun = hdr(client.post(format!("{base}/api/pipeline/rerun")))
        .json(&serde_json::json!({ "conn": "nope", "run_id": "r500" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_rerun.status(), 404);
}

#[tokio::test]
async fn launchpad_shape_ordering_and_expand_buckets() {
    // The exact cap→overflow arithmetic is unit-tested in `forgetop_core::launchpad`; here we verify
    // the endpoint wires the flags through, keeps every capped list within its limit, orders
    // "Assigned to you" in-progress-first, and leaves the expand buckets uncapped.
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let lp: serde_json::Value =
        client.get(format!("{base}/api/launchpad?t={}", server.token)).send().await.unwrap().json().await.unwrap();
    let rows = lp["rows"].as_array().expect("rows array");
    let more = &lp["more"];
    assert!(!rows.is_empty(), "launchpad has rows");
    let count = |b: &str| rows.iter().filter(|r| r["bucket"] == b).count();

    // A boolean overflow flag exists for every capped bucket, and none exceeds its cap of 5.
    for k in ["needs_review", "your_work", "your_open_prs", "recently_merged", "recent_pipelines"] {
        assert!(more[k].is_boolean(), "{k} carries an overflow flag");
        assert!(count(k) <= 5, "{k} is capped at 5 (was {})", count(k));
    }

    // Expand buckets are returned whole (the dashboard reveals them in place) — no overflow flag.
    assert!(
        more.get("ready_to_merge").is_none() && more.get("needs_fixing").is_none(),
        "ready-to-merge / needs-fixing carry no overflow flag",
    );

    // "Assigned to you" favours in-progress: once a non-Started row appears, no Started follows.
    let cats: Vec<String> = rows
        .iter()
        .filter(|r| r["bucket"] == "your_work")
        .map(|r| r["work_item"]["state_category"].as_str().unwrap_or("").to_string())
        .collect();
    if let Some(i) = cats.iter().position(|c| c != "Started") {
        assert!(cats[i..].iter().all(|c| c != "Started"), "in-progress work items sort first: {cats:?}");
    }
}

#[tokio::test]
async fn pr_revert_endpoint_reaches_the_provider() {
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();

    // A merged demo PR reverts (demo is a no-op success).
    let ok = client
        .post(format!("{base}/api/pr/revert"))
        .header("x-forgetop-token", &tok)
        .json(&serde_json::json!({ "conn": "github", "id": "1450" }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    assert_eq!(ok.json::<serde_json::Value>().await.unwrap()["ok"], true);

    // An unknown connection is a 404, not a 500.
    let missing = client
        .post(format!("{base}/api/pr/revert"))
        .header("x-forgetop-token", &tok)
        .json(&serde_json::json!({ "conn": "nope", "id": "1450" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn pr_reply_posts_into_the_target_thread() {
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();

    // Pick a PR and one of its existing threads.
    let prs: serde_json::Value =
        client.get(format!("{base}/api/pull-requests")).header("x-forgetop-token", &tok).send().await.unwrap().json().await.unwrap();
    let pr = &prs[0];
    let conn = pr["connection_id"].as_str().unwrap();
    let id = pr["pull_request"]["id"].as_str().unwrap();
    let detail_url = format!("{base}/api/pr/detail?conn={conn}&id={id}");
    let detail: serde_json::Value = client.get(&detail_url).header("x-forgetop-token", &tok).send().await.unwrap().json().await.unwrap();
    let thread_id = detail["threads"][0]["id"].as_str().expect("a thread to reply to").to_string();

    // Reply into that thread.
    let reply = client
        .post(format!("{base}/api/pr/reply"))
        .header("x-forgetop-token", &tok)
        .json(&serde_json::json!({ "conn": conn, "id": id, "thread_id": thread_id, "body": "threaded reply" }))
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 200);
    assert_eq!(reply.json::<serde_json::Value>().await.unwrap()["ok"], true);

    // The reply comes back appended to the *same* thread it targeted.
    let after: serde_json::Value = client.get(&detail_url).header("x-forgetop-token", &tok).send().await.unwrap().json().await.unwrap();
    let in_thread = after["threads"].as_array().unwrap().iter().any(|t| {
        t["id"].as_str() == Some(thread_id.as_str())
            && t["comments"].as_array().map(|c| c.iter().any(|x| x["body"] == "threaded reply")).unwrap_or(false)
    });
    assert!(in_thread, "the reply lands inside the thread it was posted to");

    // An unknown connection is a 404, not a 500.
    let missing = client
        .post(format!("{base}/api/pr/reply"))
        .header("x-forgetop-token", &tok)
        .json(&serde_json::json!({ "conn": "nope", "id": id, "thread_id": thread_id, "body": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn pr_commit_changes_endpoint_returns_a_commits_diff() {
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();

    // Pick a PR and one of its commits.
    let prs: serde_json::Value =
        client.get(format!("{base}/api/pull-requests")).header("x-forgetop-token", &tok).send().await.unwrap().json().await.unwrap();
    let pr = &prs[0];
    let conn = pr["connection_id"].as_str().unwrap();
    let id = pr["pull_request"]["id"].as_str().unwrap();
    let detail: serde_json::Value =
        client.get(format!("{base}/api/pr/detail?conn={conn}&id={id}")).header("x-forgetop-token", &tok).send().await.unwrap().json().await.unwrap();
    let sha = detail["commits"][0]["sha"].as_str().expect("a commit sha").to_string();

    // That commit reports its own changed files.
    let changes: serde_json::Value = client
        .get(format!("{base}/api/pr/commit-changes?conn={conn}&id={id}&sha={sha}"))
        .header("x-forgetop-token", &tok)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let files = changes.as_array().expect("array of file changes");
    assert!(!files.is_empty(), "the commit reports its changed files");
    assert!(files[0]["path"].is_string());

    // An unknown connection is a 404, not a 500.
    let missing = client
        .get(format!("{base}/api/pr/commit-changes?conn=nope&id={id}&sha={sha}"))
        .header("x-forgetop-token", &tok)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn pr_file_text_endpoint_returns_a_files_full_text() {
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();

    // Pick a PR and one of the files its diff names.
    let prs: serde_json::Value =
        client.get(format!("{base}/api/pull-requests")).header("x-forgetop-token", &tok).send().await.unwrap().json().await.unwrap();
    let pr = &prs[0];
    let conn = pr["connection_id"].as_str().unwrap();
    let id = pr["pull_request"]["id"].as_str().unwrap();
    let detail: serde_json::Value =
        client.get(format!("{base}/api/pr/detail?conn={conn}&id={id}")).header("x-forgetop-token", &tok).send().await.unwrap().json().await.unwrap();
    let path = detail["changes"]
        .as_array()
        .expect("the PR's changed files")
        .iter()
        .filter_map(|f| f["path"].as_str())
        .find(|p| p.contains('/'))
        .expect("a file in a directory")
        .to_string();
    let get = |query: String| client.get(format!("{base}/api/pr/file-text?{query}")).header("x-forgetop-token", &tok).send();

    // The head's text, as plain text — the path's slashes encoded, as a browser's URLSearchParams would.
    let resp = get(format!("conn={conn}&id={id}&path={}", path.replace('/', "%2F"))).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/plain; charset=utf-8");
    let text = resp.text().await.unwrap();
    assert!(text.lines().count() > 1, "the whole file: {text:?}");

    // A path the forge has no text for, and an unknown connection, are 404s — not 500s.
    assert_eq!(get(format!("conn={conn}&id={id}&path=no%2Fsuch%2Ffile.rs")).await.unwrap().status(), 404);
    assert_eq!(get(format!("conn=nope&id={id}&path={path}")).await.unwrap().status(), 404);
    // As is a commit whose patch doesn't agree with the text the demo keeps.
    assert_eq!(get(format!("conn={conn}&id={id}&path=src%2Fhttp%2Fretry.rs&sha=a1b2c3d")).await.unwrap().status(), 404);
}

#[tokio::test]
async fn highlight_endpoint_tokenises_lines_with_the_shared_grammars() {
    let server = spawn(empty_deps().await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let post = |body: serde_json::Value| client.post(format!("{base}/api/highlight")).header("x-forgetop-token", &tok).json(&body).send();

    // A Terraform file: every line comes back as spans that cover it, in UTF-16 units.
    let lines = [r#"resource "azurerm_cdn_frontdoor_origin" "web" {"#, "\tenabled = true // 🚀 live", "  count = length(var.zones)"];
    let resp: serde_json::Value =
        post(serde_json::json!({ "path": "terraform/modules/web/main.tf", "lines": lines })).await.unwrap().json().await.unwrap();
    assert_eq!(resp["lang"], "hcl", "{resp}");
    let spans = resp["lines"].as_array().expect("one entry per line");
    assert_eq!(spans.len(), lines.len());
    for (line, spans) in lines.iter().zip(spans) {
        let total: u64 = spans.as_array().unwrap().iter().map(|s| s[0].as_u64().unwrap()).sum();
        assert_eq!(total, line.encode_utf16().count() as u64, "{line:?}: {spans}");
    }
    assert_eq!(spans[0][0], serde_json::json!([8, "keyword"]), "`resource` opens the line: {}", spans[0]);
    assert!(spans[1].as_array().unwrap().iter().any(|s| s[1] == "comment"), "{}", spans[1]);
    assert!(spans[2].as_array().unwrap().iter().any(|s| s[1] == "func"), "`length(`: {}", spans[2]);

    // A language the grammars don't cover: both fields null, so the client paints plain text.
    let resp: serde_json::Value = post(serde_json::json!({ "path": "notes.txt", "lines": ["hello"] })).await.unwrap().json().await.unwrap();
    assert_eq!(resp, serde_json::json!({ "lang": null, "lines": null }));

    // Token-gated like every other API route.
    let bare = client.post(format!("{base}/api/highlight")).json(&serde_json::json!({ "path": "a.rs", "lines": [] })).send().await.unwrap();
    assert_eq!(bare.status(), 401);
}

#[tokio::test]
async fn notifications_carry_drill_in_targets_and_mark_read() {
    let server = spawn(demo_deps(true).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let get = |c: &reqwest::Client| {
        c.get(format!("{base}/api/notifications")).header("x-forgetop-token", &tok)
    };

    let notifs: serde_json::Value = get(&client).send().await.unwrap().json().await.unwrap();
    let rows = notifs.as_array().unwrap();
    assert!(!rows.is_empty(), "demo has notifications");
    assert!(
        rows.iter().any(|n| {
            let t = &n["notification"]["item_type"];
            (t == "PullRequest" || t == "WorkItem") && n["notification"]["item_id"].is_string()
        }),
        "a notification carries item_type + item_id for in-app drill-in",
    );

    // Marking one read flips its unread flag on the next fetch.
    let n = &rows[0];
    let read = client
        .post(format!("{base}/api/notification/read"))
        .header("x-forgetop-token", &tok)
        .json(&serde_json::json!({ "conn": n["connection_id"], "id": n["notification"]["id"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    let after: serde_json::Value = get(&client).send().await.unwrap().json().await.unwrap();
    let same = after
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["notification"]["id"] == n["notification"]["id"])
        .expect("the notification is still listed");
    assert_eq!(same["notification"]["unread"], false, "it now reads as read");
}

/// Pipelines are opt-in on the dashboard too: a connection bound from the web starts with nothing
/// selected, the picker's endpoints read and write the selection, and re-saving the connection
/// form keeps it.
#[tokio::test]
async fn pipeline_selection_is_opt_in_and_survives_a_connection_resave() {
    let server = spawn(demo_deps(false).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let hdr = |r: reqwest::RequestBuilder| r.header("x-forgetop-token", &tok);

    let form = serde_json::json!({ "provider": "GitHub", "display_name": "GH", "bind_pipelines": true });
    let saved: serde_json::Value =
        hdr(client.post(format!("{base}/api/connections"))).json(&form).send().await.unwrap().json().await.unwrap();
    let id = saved["id"].as_str().unwrap().to_string();
    let defs_url = format!("{base}/api/pipelines/definitions?id={id}");
    let defs = || async { hdr(client.get(&defs_url)).send().await.unwrap().json::<serde_json::Value>().await.unwrap() };

    let d = defs().await;
    let all_ids: Vec<String> = d["definitions"].as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap().to_string()).collect();
    assert!(!all_ids.is_empty(), "discovery lists the demo pipelines");
    assert_eq!(d["all"], false);
    assert_eq!(d["selected"].as_array().unwrap().len(), 0, "nothing selected by default");
    let runs: serde_json::Value = hdr(client.get(format!("{base}/api/pipelines"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(runs.as_array().unwrap().len(), 0, "nothing selected fetches nothing");

    let pick = serde_json::json!({ "id": id, "ids": [all_ids[0]] });
    assert_eq!(hdr(client.post(format!("{base}/api/pipelines/selection"))).json(&pick).send().await.unwrap().status(), 200);
    assert_eq!(defs().await["selected"], serde_json::json!([all_ids[0]]));

    // The connection form re-saves every section it binds; that must not undo the pick.
    let resave = serde_json::json!({ "id": id, "provider": "GitHub", "display_name": "GH", "bind_pipelines": true });
    hdr(client.post(format!("{base}/api/connections"))).json(&resave).send().await.unwrap();
    assert_eq!(defs().await["selected"], serde_json::json!([all_ids[0]]), "kept across a re-save");

    let all = serde_json::json!({ "id": id, "all": true });
    hdr(client.post(format!("{base}/api/pipelines/selection"))).json(&all).send().await.unwrap();
    assert_eq!(defs().await["all"], true);

    let none = serde_json::json!({ "id": id, "ids": [] });
    hdr(client.post(format!("{base}/api/pipelines/selection"))).json(&none).send().await.unwrap();
    let d = defs().await;
    assert_eq!((d["all"].clone(), d["selected"].as_array().unwrap().len()), (serde_json::json!(false), 0));

    let missing = hdr(client.get(format!("{base}/api/pipelines/definitions?id=nope"))).send().await.unwrap();
    assert_eq!(missing.status(), 404);
}

/// The address of a launchpad row, as `(connection_id, kind, repository, id)` — what the
/// dismiss route takes.
fn launchpad_address(row: &serde_json::Value) -> (String, String, Option<String>, String) {
    let kind = row["kind"].as_str().expect("row kind").to_string();
    let item = &row[match kind.as_str() {
        "pr" => "pull_request",
        "wi" => "work_item",
        _ => "run",
    }];
    (
        row["connection_id"].as_str().unwrap().to_string(),
        kind,
        item["repository"].as_str().map(str::to_string),
        item["id"].as_str().unwrap().to_string(),
    )
}

/// Whether `rows` still holds the row at this address.
fn has_row(rows: &serde_json::Value, addr: &(String, String, Option<String>, String)) -> bool {
    rows["rows"].as_array().unwrap().iter().any(|r| &launchpad_address(r) == addr)
}

#[tokio::test]
async fn dismissing_a_launchpad_row_hides_it_from_the_next_fetch() {
    let deps = demo_deps(true).await;
    let server = spawn(deps.clone(), 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let hdr = |r: reqwest::RequestBuilder| r.header("x-forgetop-token", &tok);
    let fetch = || async { hdr(client.get(format!("{base}/api/launchpad"))).send().await.unwrap().json::<serde_json::Value>().await.unwrap() };

    let before = fetch().await;
    let rows = before["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "the demo Command Center has rows");
    let addr = launchpad_address(&rows[0]);

    let body = serde_json::json!({ "conn": addr.0, "kind": addr.1, "repo": addr.2, "id": addr.3 });
    let url = format!("{base}/api/launchpad/dismiss");
    let r = hdr(client.post(&url)).json(&body).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<serde_json::Value>().await.unwrap(), serde_json::json!({ "ok": true }));
    assert!(!has_row(&fetch().await, &addr), "the dismissed row is gone");
    // What was saved is one key, this item's. (Not a row count: the demo's write stores are
    // process-wide, so a test answering a gate or closing a PR meanwhile legitimately moves
    // other rows between buckets and in or out of the capped lists.)
    let saved = || deps.config.snapshot().ui.dismissed_launchpad_items;
    let key = format!("{}:{}:{}:{}", addr.0, addr.1, addr.2.as_deref().unwrap_or(""), addr.3);
    assert_eq!(saved(), vec![key.clone()], "exactly this item's key is saved");

    // Dismissing again is harmless, and the key is saved once.
    assert_eq!(hdr(client.post(&url)).json(&body).send().await.unwrap().status(), 200);
    assert!(!has_row(&fetch().await, &addr));
    assert_eq!(saved(), vec![key]);

    // The route sits behind the session token like every other.
    assert_eq!(client.post(&url).json(&body).send().await.unwrap().status(), 401);
}

#[tokio::test]
async fn a_legacy_dismissal_key_also_hides_a_launchpad_row() {
    let deps = demo_deps(true).await;
    let config = deps.config.clone();
    let server = spawn(deps, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let fetch = || async {
        client.get(format!("{base}/api/launchpad")).header("x-forgetop-token", &tok).send().await.unwrap().json::<serde_json::Value>().await.unwrap()
    };

    let rows = fetch().await;
    let addr = launchpad_address(&rows["rows"][0]);
    // The spelling saved before the kind and repository joined the key.
    config.set_dismissed_launchpad_items(vec![format!("{}:{}", addr.0, addr.3)]).await.unwrap();
    assert!(!has_row(&fetch().await, &addr), "a legacy `<conn>:<id>` key hides the entry");
}

#[tokio::test]
async fn an_unknown_dismiss_kind_is_a_bad_request_and_saves_nothing() {
    let deps = demo_deps(true).await;
    let config = deps.config.clone();
    let server = spawn(deps, 0).await.expect("server binds");
    let r = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{}/api/launchpad/dismiss", server.port))
        .header("x-forgetop-token", &server.token)
        .json(&serde_json::json!({ "conn": "github", "kind": "nope", "repo": null, "id": "1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert!(config.snapshot().ui.dismissed_launchpad_items.is_empty());
}

#[tokio::test]
async fn prefs_reflect_the_hidden_work_item_states() {
    let server = spawn(demo_deps(false).await, 0).await.expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();
    let tok = server.token.clone();
    let prefs = || async {
        let r = client.get(format!("{base}/api/prefs")).header("x-forgetop-token", &tok).send().await.unwrap();
        assert_eq!(r.status(), 200);
        r.json::<serde_json::Value>().await.unwrap()
    };
    let post = |state: &'static str, hidden: bool| {
        client
            .post(format!("{base}/api/prefs/hidden-work-item-state"))
            .header("x-forgetop-token", &tok)
            .json(&serde_json::json!({ "state": state, "hidden": hidden }))
            .send()
    };

    assert_eq!(prefs().await, serde_json::json!({ "hidden_work_item_states": [] }));
    let r = post("Done", true).await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<serde_json::Value>().await.unwrap(), serde_json::json!({ "ok": true }));
    // One state at a time, merged: hiding a second keeps the first, hiding it twice keeps one.
    post("Closed", true).await.unwrap();
    post("Closed", true).await.unwrap();
    assert_eq!(prefs().await["hidden_work_item_states"], serde_json::json!(["Done", "Closed"]));
    post("Done", false).await.unwrap();
    assert_eq!(prefs().await["hidden_work_item_states"], serde_json::json!(["Closed"]), "showing one keeps the other hidden");
}

/// A tiny client for the inbox tests: token on every call, JSON both ways.
struct Inbox {
    base: String,
    tok: String,
    client: reqwest::Client,
}

impl Inbox {
    async fn start(deps: Deps) -> Inbox {
        let server = spawn(deps, 0).await.expect("server binds");
        Inbox { base: format!("http://127.0.0.1:{}", server.port), tok: server.token, client: reqwest::Client::new() }
    }
    async fn get(&self, path: &str) -> serde_json::Value {
        let r = self.client.get(format!("{}{path}", self.base)).header("x-forgetop-token", &self.tok).send().await.unwrap();
        assert_eq!(r.status(), 200, "GET {path}");
        r.json().await.unwrap()
    }
    async fn post(&self, path: &str, body: serde_json::Value) -> reqwest::Response {
        self.client.post(format!("{}{path}", self.base)).header("x-forgetop-token", &self.tok).json(&body).send().await.unwrap()
    }
    /// `(connection, id)` of every row `/api/notifications` serves.
    async fn notification_ids(&self) -> Vec<(String, String)> {
        let rows = self.get("/api/notifications").await;
        rows.as_array().unwrap().iter().map(|r| (r["connection_id"].as_str().unwrap().into(), r["notification"]["id"].as_str().unwrap().into())).collect()
    }
}

fn row_id(r: &serde_json::Value) -> (String, String) {
    (r["connection_id"].as_str().unwrap().into(), r["notification"]["id"].as_str().unwrap().into())
}

#[tokio::test]
async fn the_inbox_folds_notifications_into_one_thread_per_item() {
    let api = Inbox::start(demo_deps(true).await).await;
    let rows = api.get("/api/notifications").await;
    let rows = rows.as_array().unwrap();
    assert!(!rows.is_empty(), "demo has notifications");
    for r in rows {
        // The computed fields ride on every row, so the frontend never re-derives them.
        assert!(r["phrase"].as_str().is_some_and(|p| !p.is_empty() && p != "activity"), "{r}");
        assert!(r["why"].as_str().is_some_and(|w| !w.is_empty()), "{r}");
        assert!(["ForYou", "Following", "Hidden"].contains(&r["tier"].as_str().unwrap()), "{r}");
        let key = r["dismiss_key"].as_str().unwrap();
        assert!(key.starts_with(&format!("{}:{}:", r["connection_id"].as_str().unwrap(), r["notification"]["id"].as_str().unwrap())), "{key}");
    }

    let threads = api.get("/api/inbox").await;
    let threads = threads.as_array().unwrap();
    assert!(!threads.is_empty());
    let mut shown = Vec::new();
    for t in threads {
        let lead = &t["lead"];
        assert_ne!(lead["tier"], "Hidden", "a reaction never leads");
        let item = |r: &serde_json::Value| (r["connection_id"].clone(), r["notification"]["item_type"].clone(), r["notification"]["item_id"].clone());
        let members: Vec<(String, String)> =
            t["members"].as_array().unwrap().iter().map(|m| (m["conn"].as_str().unwrap().into(), m["id"].as_str().unwrap().into())).collect();
        assert!(members.contains(&row_id(lead)), "the lead is a member: {t}");
        shown.push(row_id(lead));
        for m in t["more"].as_array().unwrap() {
            assert_eq!(item(m), item(lead), "every row of a thread is about the same item");
            assert!(members.contains(&row_id(m)));
            shown.push(row_id(m));
        }
        assert_eq!(t["tier"], lead["tier"]);
        assert_eq!(t["tier"], "ForYou", "the inbox lists only what waits on you: {t}");
    }
    // Newest thread first.
    let latest: Vec<&str> = threads.iter().filter_map(|t| t["latest"].as_str()).collect();
    assert!(latest.windows(2).all(|w| w[0] >= w[1]), "{latest:?}");
    // No notification is on two rows, and every unread one waiting on you is on one.
    let mut deduped = shown.clone();
    deduped.sort();
    deduped.dedup();
    assert_eq!(deduped.len(), shown.len(), "{shown:?}");
    for r in rows.iter().filter(|r| r["tier"] == "ForYou" && r["notification"]["unread"] == true) {
        assert!(shown.contains(&row_id(r)), "{r}");
    }
    // And something the demo only follows isn't listed at all.
    assert!(rows.iter().any(|r| r["tier"] == "Following" && !shown.contains(&row_id(r))));
}

#[tokio::test]
async fn done_dismisses_a_thread_everywhere_and_saves_the_tui_keys() {
    let deps = demo_deps(true).await;
    let api = Inbox::start(deps.clone()).await;
    let rows = api.get("/api/notifications").await;
    let threads = api.get("/api/inbox").await;
    let thread = &threads[0];
    let conn = thread["members"][0]["conn"].as_str().unwrap().to_string();
    let ids: Vec<String> = thread["members"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect();
    let keys: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["connection_id"] == conn.as_str() && ids.iter().any(|id| r["notification"]["id"] == id.as_str()))
        .map(|r| r["dismiss_key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(keys.len(), ids.len(), "every member is a served row");

    let r = api.post("/api/notification/done", serde_json::json!({ "conn": conn, "ids": ids })).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<serde_json::Value>().await.unwrap(), serde_json::json!({ "ok": true }));

    let after = api.get("/api/inbox").await;
    assert!(!after.as_array().unwrap().iter().any(|t| t["key"] == thread["key"]), "the thread is gone from /api/inbox");
    let left = api.notification_ids().await;
    assert!(!ids.iter().any(|id| left.contains(&(conn.clone(), id.clone()))), "and its rows from /api/notifications");
    let saved = deps.config.snapshot().ui.dismissed_notifications;
    for k in &keys {
        assert!(saved.contains(k), "{k} saved to the list the TUI reads: {saved:?}");
    }

    // Done again is harmless and saves each key once.
    assert_eq!(api.post("/api/notification/done", serde_json::json!({ "conn": conn, "ids": ids })).await.status(), 200);
    assert_eq!(deps.config.snapshot().ui.dismissed_notifications.len(), saved.len());

    // An unknown connection is a 404, and the route sits behind the token.
    assert_eq!(api.post("/api/notification/done", serde_json::json!({ "conn": "nope", "ids": ["x"] })).await.status(), 404);
    let bare = api.client.post(format!("{}/api/notification/done", api.base)).json(&serde_json::json!({ "conn": conn, "ids": ids })).send().await.unwrap();
    assert_eq!(bare.status(), 401);
}

#[tokio::test]
async fn a_dismissal_saved_by_the_tui_hides_the_row() {
    let deps = demo_deps(true).await;
    let api = Inbox::start(deps.clone()).await;
    let rows = api.get("/api/notifications").await;
    let first = &rows[0];
    let gone = row_id(first);
    deps.config.set_dismissed_notifications(vec![first["dismiss_key"].as_str().unwrap().to_string()]).await.unwrap();

    assert!(!api.notification_ids().await.contains(&gone), "/api/notifications hides it");
    let threads = api.get("/api/inbox").await;
    for t in threads.as_array().unwrap() {
        assert_ne!(row_id(&t["lead"]), gone);
        assert!(!t["more"].as_array().unwrap().iter().any(|m| row_id(m) == gone));
        assert!(!t["members"].as_array().unwrap().iter().any(|m| (m["conn"].as_str().unwrap().to_string(), m["id"].as_str().unwrap().to_string()) == gone));
    }
}

#[tokio::test]
async fn notification_detail_answers_and_fills_the_row() {
    let api = Inbox::start(demo_deps(true).await).await;
    let rows = api.get("/api/notifications").await;
    let rows = rows.as_array().unwrap();

    // Any notification answers with an actor/excerpt object (possibly empty).
    let (conn, id) = row_id(&rows[0]);
    let r = api.post("/api/notification/detail", serde_json::json!({ "conn": conn, "id": id })).await;
    assert_eq!(r.status(), 200);
    let d: serde_json::Value = r.json().await.unwrap();
    assert!(d.is_object() && d.get("actor").is_some() && d.get("excerpt").is_some(), "{d}");

    // One whose feed entry left the who and what out gets them from the provider, and from then
    // on its row carries them — and a phrase that names the actor.
    if let Some(lazy) = rows.iter().find(|r| r["notification"]["detail_ref"].is_string() && r["notification"]["actor"].is_null()) {
        let (conn, id) = row_id(lazy);
        let d: serde_json::Value = api.post("/api/notification/detail", serde_json::json!({ "conn": conn, "id": id })).await.json().await.unwrap();
        let actor = d["actor"].as_str().expect("the provider names who did it").to_string();
        let again = api.get("/api/notifications").await;
        let row = again.as_array().unwrap().iter().find(|r| row_id(r) == (conn.clone(), id.clone())).cloned().unwrap();
        assert_eq!(row["notification"]["actor"], actor.as_str());
        if row["notification"]["kind"] == "Mention" {
            let first = actor.split_whitespace().next().unwrap();
            assert!(row["phrase"].as_str().unwrap().starts_with(first), "{}", row["phrase"]);
            // The answer carries the phrase too, so the page can patch its row without a refetch,
            // and the comment the mention is — which the feed alone couldn't name.
            assert_eq!(d["phrase"], row["phrase"]);
            assert_eq!(d["target"]["kind"], "Comment", "{d}");
            assert_eq!(row["notification"]["target"], d["target"], "the row now opens on that comment");
        }
    }

    assert_eq!(api.post("/api/notification/detail", serde_json::json!({ "conn": conn, "id": "no-such" })).await.status(), 404);
}
