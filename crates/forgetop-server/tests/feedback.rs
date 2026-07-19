use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use forgetop_core::config::{ConfigStore, ForgetopConfig, InMemoryConfigStore};
use forgetop_core::provider::ProviderRegistry;
use forgetop_core::secret::{InMemorySecretStore, SecretStore};
use forgetop_core::service::{
    ConfigService, ConnectionHealthService, ConnectionResolver, SectionService,
};
use forgetop_providers::demo::demo_factories;
use forgetop_server::{spawn_with_feedback_sink, Deps, FeedbackReport, FeedbackSink};

#[derive(Default)]
struct RecordingSink {
    configured: bool,
    fail: bool,
    reports: Mutex<Vec<FeedbackReport>>,
}

#[async_trait]
impl FeedbackSink for RecordingSink {
    fn configured(&self) -> bool {
        self.configured
    }

    async fn submit(&self, report: &FeedbackReport) -> Result<(), String> {
        self.reports.lock().unwrap().push(report.clone());
        if self.fail {
            Err("simulated destination failure".into())
        } else {
            Ok(())
        }
    }
}

async fn empty_deps() -> Deps {
    let registry = Arc::new(ProviderRegistry::new(demo_factories()));
    let store: Arc<dyn ConfigStore> = Arc::new(InMemoryConfigStore::new(ForgetopConfig::default()));
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::default());
    let config = Arc::new(ConfigService::new(store, secrets.clone(), registry.clone()));
    config.load().await.unwrap();
    let resolver = Arc::new(ConnectionResolver::new(
        config.clone(),
        registry,
        secrets.clone(),
    ));
    let sections = Arc::new(SectionService::new(config.clone(), resolver.clone()));
    let health = Arc::new(ConnectionHealthService::new(config.clone(), resolver));
    Deps {
        sections,
        health,
        config,
        secrets,
    }
}

async fn server_with(
    sink: Arc<RecordingSink>,
) -> (forgetop_server::Server, String, reqwest::Client) {
    let server = spawn_with_feedback_sink(empty_deps().await, 0, sink)
        .await
        .expect("server binds");
    let base = format!("http://127.0.0.1:{}", server.port);
    (server, base, reqwest::Client::new())
}

#[tokio::test]
async fn feedback_status_and_preview_are_authenticated() {
    let sink = Arc::new(RecordingSink {
        configured: true,
        ..Default::default()
    });
    let (server, base, client) = server_with(sink).await;

    let unauthenticated = client
        .get(format!("{base}/api/feedback/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), 401);

    let status = client
        .get(format!("{base}/api/feedback/status"))
        .header("x-forgetop-token", &server.token)
        .send()
        .await
        .unwrap();
    assert_eq!(status.status(), 200);
    let status: serde_json::Value = status.json().await.unwrap();
    assert_eq!(status["configured"], true);
    assert!(status["diagnostics"]["size_bytes"].is_number());
    assert!(
        status["diagnostics"]["oldest_at"].is_null()
            || status["diagnostics"]["oldest_at"].is_string()
    );
    assert!(
        status["diagnostics"]["newest_at"].is_null()
            || status["diagnostics"]["newest_at"].is_string()
    );

    let preview = client
        .get(format!("{base}/api/feedback/diagnostics"))
        .header("x-forgetop-token", &server.token)
        .send()
        .await
        .unwrap();
    assert_eq!(preview.status(), 200);
    assert_eq!(
        preview.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    assert!(preview.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .contains("forgetop-diagnostics.log"));
    let _valid_utf8 = preview.text().await.unwrap();
}

#[tokio::test]
async fn feedback_validation_rejects_bad_or_oversized_fields() {
    let sink = Arc::new(RecordingSink {
        configured: true,
        ..Default::default()
    });
    let (server, base, client) = server_with(sink.clone()).await;
    let post = |body: serde_json::Value| {
        client
            .post(format!("{base}/api/feedback"))
            .header("x-forgetop-token", &server.token)
            .json(&body)
            .send()
    };

    for body in [
        serde_json::json!({ "category": "bug", "summary": " ", "details": "details", "attach_diagnostics": false }),
        serde_json::json!({ "category": "bug", "summary": "s".repeat(121), "details": "details", "attach_diagnostics": false }),
        serde_json::json!({ "category": "idea", "summary": "summary", "details": " ", "attach_diagnostics": false }),
        serde_json::json!({ "category": "other", "summary": "summary", "details": "d".repeat(10_001), "attach_diagnostics": false }),
        serde_json::json!({ "category": "bug", "summary": "summary", "details": "details", "contact": "c".repeat(321), "attach_diagnostics": false }),
        serde_json::json!({ "category": "invalid", "summary": "summary", "details": "details", "attach_diagnostics": false }),
    ] {
        let response = post(body).await.unwrap();
        assert_eq!(response.status(), 422);
    }
    assert!(
        sink.reports.lock().unwrap().is_empty(),
        "invalid feedback never reaches the sink"
    );
}

#[tokio::test]
async fn submission_returns_reference_and_diagnostics_are_opt_in() {
    let sink = Arc::new(RecordingSink {
        configured: true,
        ..Default::default()
    });
    let (server, base, client) = server_with(sink.clone()).await;
    let submit = |attach_diagnostics: bool, marker: &str| {
        client
            .post(format!("{base}/api/feedback"))
            .header("x-forgetop-token", &server.token)
            .json(&serde_json::json!({
                "category": "bug",
                "summary": format!("  Summary {marker}  "),
                "details": format!("  Details {marker}  "),
                "contact": "  person@example.com  ",
                "attach_diagnostics": attach_diagnostics
            }))
            .send()
    };

    let first = submit(false, "without-log").await.unwrap();
    assert_eq!(first.status(), 200);
    let first: serde_json::Value = first.json().await.unwrap();
    uuid::Uuid::parse_str(first["reference_id"].as_str().unwrap()).expect("reference is a UUID");

    let second = submit(true, "with-log").await.unwrap();
    assert_eq!(second.status(), 200);
    let second: serde_json::Value = second.json().await.unwrap();
    uuid::Uuid::parse_str(second["reference_id"].as_str().unwrap()).expect("reference is a UUID");

    let reports = sink.reports.lock().unwrap();
    assert_eq!(reports.len(), 2);
    assert!(
        reports[0].diagnostics.is_none(),
        "unchecked diagnostics are not captured or delivered"
    );
    assert!(
        reports[1].diagnostics.is_some(),
        "checked diagnostics are delivered as an immutable snapshot"
    );
    assert_eq!(reports[0].summary, "Summary without-log");
    assert_eq!(reports[0].details, "Details without-log");
    assert_eq!(reports[0].contact.as_deref(), Some("person@example.com"));
    assert!(!reports[0].version.is_empty());
    assert!(!reports[0].os.is_empty());
    assert!(!reports[0].arch.is_empty());
    assert_eq!(
        reports[0].reference_id,
        first["reference_id"].as_str().unwrap()
    );
    assert_eq!(
        reports[1].reference_id,
        second["reference_id"].as_str().unwrap()
    );
}

#[tokio::test]
async fn unavailable_and_failed_sinks_return_clear_errors_without_feedback_text() {
    let unavailable = Arc::new(RecordingSink::default());
    let (server, base, client) = server_with(unavailable.clone()).await;
    let secret_feedback = "feedback-marker-that-must-not-be-returned";
    let body = serde_json::json!({
        "category": "other",
        "summary": secret_feedback,
        "details": secret_feedback,
        "attach_diagnostics": false
    });
    let response = client
        .post(format!("{base}/api/feedback"))
        .header("x-forgetop-token", &server.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    assert!(!response.text().await.unwrap().contains(secret_feedback));
    assert!(
        unavailable.reports.lock().unwrap().is_empty(),
        "unconfigured sink is never called"
    );

    let failing = Arc::new(RecordingSink {
        configured: true,
        fail: true,
        ..Default::default()
    });
    let (server, base, client) = server_with(failing.clone()).await;
    let response = client
        .post(format!("{base}/api/feedback"))
        .header("x-forgetop-token", &server.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    assert!(!response.text().await.unwrap().contains(secret_feedback));
    assert_eq!(failing.reports.lock().unwrap().len(), 1);

    let diagnostics = forgetop_core::diag::snapshot().unwrap().text();
    assert!(
        !diagnostics.contains(secret_feedback),
        "feedback contents are never written to diagnostics"
    );
}
