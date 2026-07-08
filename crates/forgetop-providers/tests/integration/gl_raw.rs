//! Raw GitLab REST client for integration-test fixtures (branches, files, MRs,
//! issues, pipelines) — the setup/teardown the adapter under test can't do.

use reqwest::{Client, Method};
use serde_json::{json, Value};

use crate::harness;

pub struct GlRaw {
    http: Client,
    base: String,
    project: String, // url-encoded project id/path
}

impl GlRaw {
    pub fn from_env() -> Option<Self> {
        harness::init();
        let token = harness::env("FORGETOP_IT_GITLAB_TOKEN")?;
        let project = enc(&harness::env("FORGETOP_IT_GITLAB_PROJECT")?);
        let base = harness::env("FORGETOP_IT_GITLAB_HOST").map(|h| format!("{}/api/v4", h.trim_end_matches('/'))).unwrap_or_else(|| "https://gitlab.com/api/v4".into());
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        let http = Client::builder().default_headers(headers).build().unwrap();
        Some(Self { http, base, project })
    }

    fn url(&self, suffix: &str) -> String {
        format!("{}/projects/{}{}", self.base, self.project, suffix)
    }

    async fn raw(&self, method: Method, url: &str, body: Option<Value>) -> (reqwest::StatusCode, String) {
        let mut req = self.http.request(method.clone(), url);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap_or_else(|e| panic!("{method} {url}: {e}"));
        let status = resp.status();
        (status, resp.text().await.unwrap_or_default())
    }

    async fn send(&self, method: Method, url: &str, body: Option<Value>) -> Value {
        let (status, text) = self.raw(method.clone(), url, body).await;
        assert!(status.is_success(), "{method} {url} -> {status}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    async fn try_send(&self, method: Method, url: &str, body: Option<Value>) -> bool {
        self.raw(method, url, body).await.0.is_success()
    }

    // ---- setup reads ----

    pub async fn me(&self) -> (i64, String) {
        let v = self.send(Method::GET, &format!("{}/user", self.base), None).await;
        (v["id"].as_i64().expect("user id"), v["username"].as_str().unwrap_or_default().to_string())
    }

    pub async fn default_branch(&self) -> String {
        let v = self.send(Method::GET, &self.url(""), None).await;
        v["default_branch"].as_str().unwrap_or("main").to_string()
    }

    // ---- fixture creation ----

    pub async fn create_branch(&self, name: &str, from: &str) {
        self.send(Method::POST, &self.url(&format!("/repository/branches?branch={name}&ref={from}")), None).await;
    }

    pub async fn put_file(&self, path: &str, content: &str, branch: &str, message: &str) {
        let url = self.url(&format!("/repository/files/{}", enc(path)));
        self.send(Method::POST, &url, Some(json!({ "branch": branch, "content": content, "commit_message": message }))).await;
    }

    /// Opens an MR; returns its iid (the adapter's PR id).
    pub async fn open_mr(&self, source: &str, target: &str, title: &str) -> i64 {
        let v = self.send(Method::POST, &self.url("/merge_requests"), Some(json!({ "source_branch": source, "target_branch": target, "title": title }))).await;
        v["iid"].as_i64().expect("mr iid")
    }

    /// Creates an issue assigned to `assignee_id`; returns its iid.
    pub async fn create_issue(&self, title: &str, assignee_id: i64) -> i64 {
        let v = self.send(Method::POST, &self.url("/issues"), Some(json!({ "title": title, "assignee_ids": [assignee_id] }))).await;
        v["iid"].as_i64().expect("issue iid")
    }

    /// Creates a pipeline on `git_ref` (its `.gitlab-ci.yml` must exist there).
    /// Returns the pipeline id, or `Err(body)` — e.g. when GitLab.com blocks CI on an
    /// unvalidated account ("Identity verification is required"), which the caller
    /// treats as a skip rather than a failure.
    pub async fn create_pipeline(&self, git_ref: &str) -> Result<i64, String> {
        let (status, text) = self.raw(Method::POST, &self.url("/pipeline"), Some(json!({ "ref": git_ref }))).await;
        if !status.is_success() {
            return Err(format!("{status}: {text}"));
        }
        Ok(serde_json::from_str::<Value>(&text).ok().and_then(|v| v["id"].as_i64()).unwrap_or_default())
    }

    // ---- teardown (best-effort) ----

    pub async fn delete_branch(&self, name: &str) {
        self.try_send(Method::DELETE, &self.url(&format!("/repository/branches/{}", enc(name))), None).await;
    }
    pub async fn delete_issue(&self, iid: i64) {
        self.try_send(Method::DELETE, &self.url(&format!("/issues/{iid}")), None).await;
    }
    pub async fn delete_pipeline(&self, id: i64) {
        self.try_send(Method::DELETE, &self.url(&format!("/pipelines/{id}")), None).await;
    }

    /// Best-effort cleanup of `forgetop-it-*` branches left by earlier runs.
    pub async fn sweep(&self) {
        let v = self.send(Method::GET, &self.url("/repository/branches?per_page=100&search=forgetop-it-"), None).await;
        if let Some(list) = v.as_array() {
            for b in list {
                if let Some(name) = b["name"].as_str().filter(|n| n.starts_with(harness::SWEEP_PREFIX)) {
                    self.delete_branch(name).await;
                }
            }
        }
    }
}

/// Minimal URL-encoding for path/id segments (slashes and dots are what matter).
fn enc(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
