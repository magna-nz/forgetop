//! A raw GitHub REST client used only by the integration tests to **create and
//! tear down fixtures** (branches, files, PRs, issues, environments, workflow
//! dispatches). The provider adapter under test only reads/acts — it can't open a
//! PR or create an environment, so those setup operations live here.
//!
//! Setup calls assert on failure (a broken fixture should fail the test loudly);
//! teardown/sweep calls are best-effort and swallow errors.

use base64::Engine;
use reqwest::{Client, Method};
use serde_json::{json, Value};

use crate::harness;

pub struct GhRaw {
    http: Client,
    base: String,
    owner: String,
    repo: String,
}

impl GhRaw {
    /// Builds from `FORGETOP_IT_GITHUB_*`, or `None` when creds are absent (skip).
    pub fn from_env() -> Option<Self> {
        harness::init();
        let token = harness::env("FORGETOP_IT_GITHUB_TOKEN")?;
        let (owner, repo) = harness::env("FORGETOP_IT_GITHUB_REPO")?.split_once('/').map(|(o, r)| (o.to_string(), r.to_string()))?;
        let base = harness::env("FORGETOP_IT_GITHUB_HOST").unwrap_or_else(|| "https://api.github.com".into());
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::USER_AGENT, "forgetop-it".parse().unwrap());
        headers.insert(reqwest::header::ACCEPT, "application/vnd.github+json".parse().unwrap());
        headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        let http = Client::builder().default_headers(headers).build().unwrap();
        Some(Self { http, base, owner, repo })
    }

    fn url(&self, suffix: &str) -> String {
        format!("{}/repos/{}/{}{}", self.base, self.owner, self.repo, suffix)
    }

    /// Sends a request, asserting a 2xx (for setup steps that must succeed).
    async fn send(&self, method: Method, url: &str, body: Option<Value>) -> Value {
        let (status, text) = self.raw(method.clone(), url, body).await;
        assert!(status.is_success(), "{method} {url} -> {status}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    /// Best-effort request for teardown/sweep — returns whether it succeeded.
    async fn try_send(&self, method: Method, url: &str, body: Option<Value>) -> bool {
        self.raw(method, url, body).await.0.is_success()
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

    // ---- reads used during setup ----

    /// The authenticated user's (numeric id, login).
    pub async fn me(&self) -> (i64, String) {
        let v = self.send(Method::GET, &format!("{}/user", self.base), None).await;
        (v["id"].as_i64().expect("user id"), v["login"].as_str().expect("login").to_string())
    }

    pub async fn default_branch(&self) -> String {
        let v = self.send(Method::GET, &self.url(""), None).await;
        v["default_branch"].as_str().unwrap_or("main").to_string()
    }

    pub async fn branch_sha(&self, branch: &str) -> String {
        let v = self.send(Method::GET, &self.url(&format!("/git/ref/heads/{branch}")), None).await;
        v["object"]["sha"].as_str().expect("branch sha").to_string()
    }

    // ---- fixture creation ----

    pub async fn create_branch(&self, name: &str, from_sha: &str) {
        self.send(Method::POST, &self.url("/git/refs"), Some(json!({ "ref": format!("refs/heads/{name}"), "sha": from_sha }))).await;
    }

    /// Creates or updates a file on `branch`. Returns the new content sha.
    pub async fn put_file(&self, path: &str, content: &str, branch: &str, message: &str) -> String {
        let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
        // Include the existing sha if the file already exists (update vs create).
        let existing = self.raw(Method::GET, &self.url(&format!("/contents/{path}?ref={branch}")), None).await;
        let mut body = json!({ "message": message, "content": encoded, "branch": branch });
        if existing.0.is_success() {
            if let Ok(v) = serde_json::from_str::<Value>(&existing.1) {
                if let Some(sha) = v["sha"].as_str() {
                    body["sha"] = json!(sha);
                }
            }
        }
        let v = self.send(Method::PUT, &self.url(&format!("/contents/{path}")), Some(body)).await;
        v["content"]["sha"].as_str().unwrap_or_default().to_string()
    }

    pub async fn delete_file(&self, path: &str, branch: &str, message: &str) {
        let existing = self.raw(Method::GET, &self.url(&format!("/contents/{path}?ref={branch}")), None).await;
        let Ok(v) = serde_json::from_str::<Value>(&existing.1) else { return };
        let Some(sha) = v["sha"].as_str() else { return };
        self.try_send(Method::DELETE, &self.url(&format!("/contents/{path}")), Some(json!({ "message": message, "sha": sha, "branch": branch }))).await;
    }

    /// Opens a PR; returns its number (the adapter's PR id).
    pub async fn open_pr(&self, head: &str, base: &str, title: &str) -> i64 {
        let v = self.send(Method::POST, &self.url("/pulls"), Some(json!({ "title": title, "head": head, "base": base, "body": "forgetop integration fixture" }))).await;
        v["number"].as_i64().expect("pr number")
    }

    /// Creates an issue assigned to `assignee`; returns its number.
    pub async fn create_issue(&self, title: &str, assignee: &str) -> i64 {
        let v = self.send(Method::POST, &self.url("/issues"), Some(json!({ "title": title, "assignees": [assignee], "body": "forgetop integration fixture" }))).await;
        v["number"].as_i64().expect("issue number")
    }

    /// Creates/updates a deployment environment requiring `reviewer_id` to approve.
    /// Note: required reviewers on **public** repos are free; on private repos they
    /// need a paid plan — the approval test needs a public container repo.
    pub async fn put_environment(&self, name: &str, reviewer_id: i64) {
        self.send(
            Method::PUT,
            &self.url(&format!("/environments/{name}")),
            Some(json!({ "reviewers": [{ "type": "User", "id": reviewer_id }] })),
        )
        .await;
    }

    /// Dispatches a `workflow_dispatch` workflow by file name, retrying briefly while
    /// GitHub registers a freshly-committed workflow.
    pub async fn dispatch(&self, workflow_file: &str, git_ref: &str) {
        let url = self.url(&format!("/actions/workflows/{workflow_file}/dispatches"));
        for attempt in 0..10 {
            if self.try_send(Method::POST, &url, Some(json!({ "ref": git_ref }))).await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            assert!(attempt < 9, "workflow {workflow_file} never became dispatchable");
        }
    }

    /// The (run id, status) pairs for a workflow's recent runs, newest first.
    pub async fn workflow_runs(&self, workflow_file: &str) -> Vec<(String, String)> {
        let v = self.send(Method::GET, &self.url(&format!("/actions/workflows/{workflow_file}/runs?per_page=20")), None).await;
        v["workflow_runs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| Some((r["id"].as_i64()?.to_string(), r["status"].as_str().unwrap_or_default().to_string())))
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---- teardown (best-effort) ----

    pub async fn delete_ref(&self, branch: &str) {
        self.try_send(Method::DELETE, &self.url(&format!("/git/refs/heads/{branch}")), None).await;
    }
    pub async fn close_issue(&self, number: i64) {
        self.try_send(Method::PATCH, &self.url(&format!("/issues/{number}")), Some(json!({ "state": "closed" }))).await;
    }
    pub async fn delete_environment(&self, name: &str) {
        self.try_send(Method::DELETE, &self.url(&format!("/environments/{name}")), None).await;
    }
    pub async fn delete_run(&self, run_id: &str) {
        self.try_send(Method::DELETE, &self.url(&format!("/actions/runs/{run_id}")), None).await;
    }

    /// Best-effort cleanup of leftovers from previous runs (prefix `forgetop-it-`):
    /// deletes stray branches + environments and closes stray issues. Guards against
    /// fixtures leaked by a panicked test.
    pub async fn sweep(&self) {
        // Branches.
        let refs = self.raw(Method::GET, &self.url("/git/refs/heads/forgetop-it-"), None).await;
        if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&refs.1) {
            for r in items {
                if let Some(name) = r["ref"].as_str().and_then(|s| s.strip_prefix("refs/heads/")) {
                    self.delete_ref(name).await;
                }
            }
        }
        // Environments.
        let envs = self.send(Method::GET, &self.url("/environments?per_page=100"), None).await;
        if let Some(list) = envs["environments"].as_array() {
            for e in list {
                if let Some(name) = e["name"].as_str().filter(|n| n.starts_with(harness::SWEEP_PREFIX)) {
                    self.delete_environment(name).await;
                }
            }
        }
        // Open issues we created.
        let issues = self.send(Method::GET, &self.url("/issues?state=open&per_page=100"), None).await;
        if let Some(list) = issues.as_array() {
            for i in list {
                let is_ours = i["title"].as_str().is_some_and(|t| t.starts_with(harness::SWEEP_PREFIX));
                if is_ours {
                    if let Some(n) = i["number"].as_i64() {
                        self.close_issue(n).await;
                    }
                }
            }
        }
    }
}
