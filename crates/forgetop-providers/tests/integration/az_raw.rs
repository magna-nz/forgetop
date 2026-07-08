//! Raw Azure DevOps REST client for integration-test fixtures: pushing a branch +
//! file (the Git pushes API), opening PRs, creating work items, and queueing a
//! pre-created gated pipeline. Setup asserts; teardown is best-effort.

use base64::Engine;
use reqwest::{Client, Method};
use serde_json::{json, Value};

use crate::harness;

const API: &str = "api-version=7.1";
const ZERO_SHA: &str = "0000000000000000000000000000000000000000";

pub struct AzRaw {
    http: Client,
    base: String, // https://dev.azure.com/{org}
    project: String,
    repo: String,
}

impl AzRaw {
    pub fn from_env() -> Option<Self> {
        harness::init();
        let pat = harness::env("FORGETOP_IT_AZURE_PAT")?;
        let org = harness::env("FORGETOP_IT_AZURE_ORG")?;
        let project = harness::env("FORGETOP_IT_AZURE_PROJECT")?;
        let repo = harness::env("FORGETOP_IT_AZURE_REPO").unwrap_or_else(|| project.clone());
        let token = base64::engine::general_purpose::STANDARD.encode(format!(":{pat}"));
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, format!("Basic {token}").parse().unwrap());
        let http = Client::builder().default_headers(headers).build().unwrap();
        Some(Self { http, base: format!("https://dev.azure.com/{org}"), project, repo })
    }

    fn proj(&self, suffix: &str) -> String {
        format!("{}/{}{}", self.base, self.project, suffix)
    }
    fn git(&self, suffix: &str) -> String {
        self.proj(&format!("/_apis/git/repositories/{}{}", self.repo, suffix))
    }

    async fn raw(&self, method: Method, url: &str, body: Option<Value>, ct: &str) -> (reqwest::StatusCode, String) {
        let mut req = self.http.request(method.clone(), url);
        if let Some(b) = body {
            req = req.header(reqwest::header::CONTENT_TYPE, ct).body(serde_json::to_string(&b).unwrap());
        }
        let resp = req.send().await.unwrap_or_else(|e| panic!("{method} {url}: {e}"));
        let status = resp.status();
        (status, resp.text().await.unwrap_or_default())
    }

    async fn send(&self, method: Method, url: &str, body: Option<Value>) -> Value {
        let (status, text) = self.raw(method.clone(), url, body, "application/json").await;
        assert!(status.is_success(), "{method} {url} -> {status}: {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    async fn try_send(&self, method: Method, url: &str, body: Option<Value>) -> bool {
        self.raw(method, url, body, "application/json").await.0.is_success()
    }

    // ---- setup reads ----

    /// The authenticated user's unique name (email), for AssignedTo.
    pub async fn me_unique(&self) -> String {
        let v = self.send(Method::GET, &format!("{}/_apis/connectionData?{API}", self.base), None).await;
        v["authenticatedUser"]["uniqueName"].as_str().unwrap_or_default().to_string()
    }

    /// The authenticated user's identity id (GUID), for approval-check approvers.
    pub async fn me_id(&self) -> String {
        let v = self.send(Method::GET, &format!("{}/_apis/connectionData?{API}", self.base), None).await;
        v["authenticatedUser"]["id"].as_str().unwrap_or_default().to_string()
    }

    /// The repo's id (GUID), for pipeline-definition creation.
    pub async fn repo_id(&self) -> String {
        let v = self.send(Method::GET, &self.git(&format!("?{API}")), None).await;
        v["id"].as_str().unwrap_or_default().to_string()
    }

    /// (default branch name, its tip commit sha).
    pub async fn default_branch(&self) -> (String, String) {
        let repo = self.send(Method::GET, &self.git(&format!("?{API}")), None).await;
        let full = repo["defaultBranch"].as_str().unwrap_or("refs/heads/main").to_string();
        let name = full.strip_prefix("refs/heads/").unwrap_or("main").to_string();
        let refs = self.send(Method::GET, &self.git(&format!("/refs?filter=heads/{name}&{API}")), None).await;
        let sha = refs["value"][0]["objectId"].as_str().unwrap_or_default().to_string();
        (name, sha)
    }

    // ---- fixture creation ----

    /// Pushes a single added file to `branch` (whose current tip is `base_sha`).
    /// Works both to create a new branch and to add a commit to an existing one.
    pub async fn push_file(&self, branch: &str, base_sha: &str, path: &str, content: &str, message: &str) {
        let body = json!({
            "refUpdates": [{ "name": format!("refs/heads/{branch}"), "oldObjectId": base_sha }],
            "commits": [{
                "comment": message,
                "changes": [{
                    "changeType": "add",
                    "item": { "path": format!("/{path}") },
                    "newContent": { "content": content, "contentType": "rawtext" }
                }]
            }]
        });
        self.send(Method::POST, &self.git(&format!("/pushes?{API}")), Some(body)).await;
    }

    /// Opens a PR; returns its pullRequestId (the adapter's PR id).
    pub async fn open_pr(&self, source: &str, target: &str, title: &str) -> i64 {
        let body = json!({ "sourceRefName": format!("refs/heads/{source}"), "targetRefName": format!("refs/heads/{target}"), "title": title });
        let v = self.send(Method::POST, &self.git(&format!("/pullrequests?{API}")), Some(body)).await;
        v["pullRequestId"].as_i64().expect("pullRequestId")
    }

    /// Creates a $Task work item assigned to `assignee`; returns its id.
    pub async fn create_work_item(&self, title: &str, assignee: &str) -> i64 {
        let patch = json!([
            { "op": "add", "path": "/fields/System.Title", "value": title },
            { "op": "add", "path": "/fields/System.AssignedTo", "value": assignee }
        ]);
        let url = self.proj(&format!("/_apis/wit/workitems/$Task?{API}"));
        let (status, text) = self.raw(Method::POST, &url, Some(patch), "application/json-patch+json").await;
        assert!(status.is_success(), "create work item -> {status}: {text}");
        serde_json::from_str::<Value>(&text).unwrap()["id"].as_i64().expect("work item id")
    }

    /// Queues a run of a pipeline; returns the build/run id.
    pub async fn queue_pipeline(&self, pipeline_id: &str) -> String {
        let v = self.send(Method::POST, &self.proj(&format!("/_apis/pipelines/{pipeline_id}/runs?{API}")), Some(json!({}))).await;
        v["id"].as_i64().map(|n| n.to_string()).expect("run id")
    }

    /// Creates a deployment environment; returns its id.
    pub async fn create_environment(&self, name: &str) -> i64 {
        let v = self.send(Method::POST, &self.proj(&format!("/_apis/distributedtask/environments?{API}")), Some(json!({ "name": name, "description": "forgetop integration fixture" }))).await;
        v["id"].as_i64().expect("environment id")
    }

    /// Adds an Approval check to an environment with `approver_id` as sole approver.
    pub async fn add_approval_check(&self, env_id: i64, env_name: &str, approver_id: &str) -> i64 {
        let body = json!({
            "type": { "id": "8c6f20a7-a545-4486-9777-f762fafe0d4d", "name": "Approval" },
            "settings": {
                "approvers": [{ "id": approver_id }],
                "executionOrder": "anyOrder",
                "minRequiredApprovers": 1,
                "instructions": "forgetop integration",
                "blockedApprovers": []
            },
            "timeout": 43200,
            "resource": { "type": "environment", "id": env_id.to_string(), "name": env_name }
        });
        let v = self.send(Method::POST, &self.proj("/_apis/pipelines/checks/configurations?api-version=7.1-preview.1"), Some(body)).await;
        v["id"].as_i64().expect("check id")
    }

    /// Creates a YAML pipeline definition pointing at `yaml_path` in the repo;
    /// returns the definition id.
    pub async fn create_pipeline_def(&self, name: &str, yaml_path: &str, repo_id: &str, repo_name: &str) -> String {
        let body = json!({
            "name": name,
            "folder": "\\",
            "configuration": {
                "type": "yaml",
                "path": yaml_path,
                "repository": { "id": repo_id, "name": repo_name, "type": "azureReposGit" }
            }
        });
        let v = self.send(Method::POST, &self.proj(&format!("/_apis/pipelines?{API}")), Some(body)).await;
        v["id"].as_i64().map(|n| n.to_string()).expect("pipeline id")
    }

    // extra teardown
    pub async fn delete_environment_by_id(&self, id: i64) {
        self.try_send(Method::DELETE, &self.proj(&format!("/_apis/distributedtask/environments/{id}?{API}")), None).await;
    }
    pub async fn delete_check(&self, id: i64) {
        self.try_send(Method::DELETE, &self.proj(&format!("/_apis/pipelines/checks/configurations/{id}?api-version=7.1-preview.1")), None).await;
    }
    pub async fn delete_pipeline_def(&self, id: &str) {
        self.try_send(Method::DELETE, &self.proj(&format!("/_apis/build/definitions/{id}?{API}")), None).await;
    }
    /// Removes a file from `branch` (a push with a delete change).
    pub async fn delete_file(&self, path: &str, branch: &str, message: &str) {
        let refs = self.raw(Method::GET, &self.git(&format!("/refs?filter=heads/{branch}&{API}")), None, "application/json").await;
        let Ok(v) = serde_json::from_str::<Value>(&refs.1) else { return };
        let Some(sha) = v["value"][0]["objectId"].as_str() else { return };
        let body = json!({
            "refUpdates": [{ "name": format!("refs/heads/{branch}"), "oldObjectId": sha }],
            "commits": [{ "comment": message, "changes": [{ "changeType": "delete", "item": { "path": format!("/{path}") } }] }]
        });
        self.try_send(Method::POST, &self.git(&format!("/pushes?{API}")), Some(body)).await;
    }

    // ---- teardown (best-effort) ----

    pub async fn delete_work_item(&self, id: i64) {
        self.try_send(Method::DELETE, &self.proj(&format!("/_apis/wit/workitems/{id}?{API}")), None).await;
    }
    pub async fn abandon_pr(&self, id: i64) {
        self.try_send(Method::PATCH, &self.git(&format!("/pullrequests/{id}?{API}")), Some(json!({ "status": "abandoned" }))).await;
    }
    pub async fn delete_branch(&self, branch: &str) {
        // Deleting a ref = a push whose newObjectId is all-zeros.
        let refs = self.raw(Method::GET, &self.git(&format!("/refs?filter=heads/{branch}&{API}")), None, "application/json").await;
        let Ok(v) = serde_json::from_str::<Value>(&refs.1) else { return };
        let Some(sha) = v["value"][0]["objectId"].as_str() else { return };
        let body = json!({ "refUpdates": [{ "name": format!("refs/heads/{branch}"), "oldObjectId": sha, "newObjectId": ZERO_SHA }] });
        self.try_send(Method::POST, &self.git(&format!("/pushes?{API}")), Some(body)).await;
    }
    pub async fn delete_build(&self, build_id: &str) {
        self.try_send(Method::DELETE, &self.proj(&format!("/_apis/build/builds/{build_id}?{API}")), None).await;
    }
}
