//! Raw Jira REST client for integration-test fixtures (create/delete issues).

use base64::Engine;
use reqwest::{Client, Method};
use serde_json::{json, Value};

use crate::harness;

pub struct JiraRaw {
    http: Client,
    api: String,
    project: String,
}

impl JiraRaw {
    pub fn from_env() -> Option<Self> {
        harness::init();
        let token = harness::env("FORGETOP_IT_JIRA_TOKEN")?;
        let site = harness::env("FORGETOP_IT_JIRA_SITE")?;
        let email = harness::env("FORGETOP_IT_JIRA_EMAIL")?;
        let project = harness::env("FORGETOP_IT_JIRA_PROJECT")?;
        let creds = base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}"));
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, format!("Basic {creds}").parse().unwrap());
        headers.insert(reqwest::header::ACCEPT, "application/json".parse().unwrap());
        let http = Client::builder().default_headers(headers).build().unwrap();
        Some(Self { http, api: format!("{}/rest/api/2", site.trim_end_matches('/')), project })
    }

    async fn send(&self, method: Method, url: &str, body: Option<Value>) -> Value {
        let mut req = self.http.request(method.clone(), url);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap_or_else(|e| panic!("{method} {url}: {e}"));
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        assert!(status.is_success(), "{method} {url} -> {status}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    pub async fn myself(&self) -> String {
        self.send(Method::GET, &format!("{}/myself", self.api), None).await["accountId"].as_str().unwrap_or_default().to_string()
    }

    /// Creates a Task assigned to `assignee`; returns its key (e.g. `IT-123`).
    pub async fn create_issue(&self, summary: &str, assignee: &str) -> String {
        let body = json!({ "fields": {
            "project": { "key": self.project },
            "summary": summary,
            "issuetype": { "name": "Task" },
            "assignee": { "accountId": assignee }
        }});
        self.send(Method::POST, &format!("{}/issue", self.api), Some(body)).await["key"].as_str().expect("issue key").to_string()
    }

    pub async fn delete_issue(&self, key: &str) {
        let _ = self.http.delete(format!("{}/issue/{key}", self.api)).send().await;
    }
}
