//! Explicit, on-demand feedback delivery.
//!
//! This module deliberately does not install a global Sentry client. A report is only sent
//! after the user submits the feedback form, and diagnostics are only attached when requested.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use forgetop_core::diag::DiagnosticSnapshot;
use reqwest::Url;
use serde::{Deserialize, Serialize};

/// User-selected feedback category.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackCategory {
    Bug,
    Idea,
    Other,
}

/// The validated report passed to a delivery implementation.
#[derive(Clone, Debug)]
pub struct FeedbackReport {
    pub reference_id: String,
    pub category: FeedbackCategory,
    pub summary: String,
    pub details: String,
    pub contact: Option<String>,
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub diagnostics: Option<DiagnosticSnapshot>,
}

/// Injectable boundary between the local dashboard API and the private feedback destination.
#[async_trait]
pub trait FeedbackSink: Send + Sync {
    fn configured(&self) -> bool;
    async fn submit(&self, report: &FeedbackReport) -> Result<(), String>;
}

#[derive(Clone)]
pub(crate) struct SentryFeedbackSink {
    destination: Option<SentryDestination>,
    client: reqwest::Client,
}

#[derive(Clone)]
struct SentryDestination {
    envelope_url: Url,
    public_key: String,
    dsn: String,
}

impl SentryFeedbackSink {
    pub(crate) fn from_environment() -> Self {
        let dsn = std::env::var("FORGETOP_FEEDBACK_DSN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| option_env!("FORGETOP_FEEDBACK_DSN").map(str::to_owned));
        Self {
            destination: dsn.and_then(|dsn| SentryDestination::parse(&dsn)),
            client: reqwest::Client::new(),
        }
    }
}

impl SentryDestination {
    fn parse(raw: &str) -> Option<Self> {
        let dsn = Url::parse(raw).ok()?;
        if !matches!(dsn.scheme(), "http" | "https") {
            return None;
        }
        let public_key = dsn.username().to_owned();
        if public_key.is_empty() || dsn.host_str().is_none() {
            return None;
        }

        let mut segments: Vec<&str> = dsn
            .path_segments()?
            .filter(|segment| !segment.is_empty())
            .collect();
        let project_id = segments.pop()?;
        if project_id.is_empty() {
            return None;
        }
        let prefix = if segments.is_empty() {
            String::new()
        } else {
            format!("/{}", segments.join("/"))
        };

        let mut envelope_dsn = dsn.clone();
        envelope_dsn.set_password(None).ok()?;
        envelope_dsn.set_query(None);
        envelope_dsn.set_fragment(None);

        let mut envelope_url = envelope_dsn.clone();
        envelope_url.set_username("").ok()?;
        envelope_url.set_path(&format!("{prefix}/api/{project_id}/envelope/"));

        Some(Self {
            envelope_url,
            public_key,
            dsn: envelope_dsn.to_string(),
        })
    }
}

#[async_trait]
impl FeedbackSink for SentryFeedbackSink {
    fn configured(&self) -> bool {
        self.destination.is_some()
    }

    async fn submit(&self, report: &FeedbackReport) -> Result<(), String> {
        let destination = self
            .destination
            .as_ref()
            .ok_or_else(|| "feedback delivery is not configured".to_owned())?;
        let envelope = sentry_envelope(report, &destination.dsn)?;
        let auth = format!(
            "Sentry sentry_version=7, sentry_key={}, sentry_client=forgetop/{}",
            destination.public_key, report.version
        );
        let response = self
            .client
            .post(destination.envelope_url.clone())
            .timeout(Duration::from_secs(15))
            .header("content-type", "application/x-sentry-envelope")
            .header("x-sentry-auth", auth)
            .body(envelope)
            .send()
            .await
            .map_err(|_| "feedback request failed".to_owned())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "feedback destination returned HTTP {}",
                response.status().as_u16()
            ))
        }
    }
}

fn sentry_envelope(report: &FeedbackReport, dsn: &str) -> Result<Vec<u8>, String> {
    let event_id = report.reference_id.replace('-', "");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_owned())?
        .as_secs_f64();

    let mut extra = serde_json::json!({
        "reference_id": report.reference_id,
        "details": report.details,
    });
    if let Some(contact) = &report.contact {
        extra["contact"] = serde_json::Value::String(contact.clone());
    }
    let event = serde_json::json!({
        "event_id": event_id,
        "timestamp": timestamp,
        "platform": "native",
        "level": "info",
        "logger": "forgetop.feedback",
        "message": report.summary,
        "release": format!("forgetop@{}", report.version),
        "tags": {
            "feedback_category": report.category,
            "feedback_reference": report.reference_id,
            "forgetop_version": report.version,
            "os": report.os,
            "arch": report.arch,
        },
        "fingerprint": [report.reference_id],
        "extra": extra,
    });
    let event =
        serde_json::to_vec(&event).map_err(|_| "could not encode feedback event".to_owned())?;
    let envelope_header =
        serde_json::to_vec(&serde_json::json!({ "event_id": event_id, "dsn": dsn }))
            .map_err(|_| "could not encode feedback envelope".to_owned())?;
    let event_header = serde_json::to_vec(
        &serde_json::json!({ "type": "event", "content_type": "application/json", "length": event.len() }),
    )
    .map_err(|_| "could not encode feedback envelope".to_owned())?;

    let mut envelope = Vec::with_capacity(
        event.len() + report.diagnostics.as_ref().map_or(0, |d| d.bytes.len()) + 512,
    );
    append_item(&mut envelope, &envelope_header);
    append_item(&mut envelope, &event_header);
    append_item(&mut envelope, &event);

    if let Some(diagnostics) = &report.diagnostics {
        let attachment_header = serde_json::to_vec(&serde_json::json!({
            "type": "attachment",
            "content_type": "text/plain",
            "filename": "forgetop-diagnostics.log",
            "attachment_type": "event.attachment",
            "length": diagnostics.bytes.len(),
        }))
        .map_err(|_| "could not encode diagnostic attachment".to_owned())?;
        append_item(&mut envelope, &attachment_header);
        append_item(&mut envelope, &diagnostics.bytes);
    }
    Ok(envelope)
}

fn append_item(envelope: &mut Vec<u8>, item: &[u8]) {
    envelope.extend_from_slice(item);
    envelope.push(b'\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sentry_dsn_into_envelope_destination() {
        let destination =
            SentryDestination::parse("https://public:legacy-secret@example.invalid/prefix/42").unwrap();
        assert_eq!(destination.public_key, "public");
        assert!(!destination.dsn.contains("legacy-secret"));
        assert_eq!(
            destination.envelope_url.as_str(),
            "https://example.invalid/prefix/api/42/envelope/"
        );
    }

    #[test]
    fn rejects_dsn_without_public_key_or_project() {
        assert!(SentryDestination::parse("https://example.invalid/42").is_none());
        assert!(SentryDestination::parse("https://public@example.invalid").is_none());
        assert!(SentryDestination::parse("file://public@example.invalid/42").is_none());
    }

    #[test]
    fn envelope_only_contains_attachment_when_selected() {
        let mut report = FeedbackReport {
            reference_id: "4ed48b62-d374-4ec2-bcd2-32f99f29b405".into(),
            category: FeedbackCategory::Bug,
            summary: "Summary".into(),
            details: "Details".into(),
            contact: None,
            version: "1.2.3",
            os: "test-os",
            arch: "test-arch",
            diagnostics: None,
        };
        let without = sentry_envelope(&report, "https://public@example.invalid/42").unwrap();
        assert!(!String::from_utf8_lossy(&without).contains("\"type\":\"attachment\""));

        report.diagnostics = Some(DiagnosticSnapshot {
            bytes: b"sanitized diagnostics".to_vec(),
            size_bytes: 21,
            oldest_at: None,
            newest_at: None,
        });
        let with = sentry_envelope(&report, "https://public@example.invalid/42").unwrap();
        let text = String::from_utf8_lossy(&with);
        assert!(text.contains("\"type\":\"attachment\""));
        assert!(text.contains("sanitized diagnostics"));
    }
}
