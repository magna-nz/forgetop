//! Raw Linear GraphQL client for integration-test fixtures (create/archive issues).

use reqwest::Client;
use serde_json::{json, Value};

use crate::harness;

pub struct LnRaw {
    http: Client,
    base: String,
}

impl LnRaw {
    pub fn from_env() -> Option<Self> {
        harness::init();
        let key = harness::env("FORGETOP_IT_LINEAR_KEY")?;
        let mut headers = reqwest::header::HeaderMap::new();
        // Linear personal API keys are the raw Authorization value (no "Bearer").
        headers.insert(reqwest::header::AUTHORIZATION, key.parse().unwrap());
        let http = Client::builder().default_headers(headers).build().unwrap();
        Some(Self { http, base: "https://api.linear.app/graphql".into() })
    }

    async fn query(&self, query: &str, variables: Value) -> Value {
        let resp = self.http.post(&self.base).json(&json!({ "query": query, "variables": variables })).send().await.unwrap_or_else(|e| panic!("linear graphql: {e}"));
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        assert!(status.is_success(), "linear graphql -> {status}: {text}");
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        assert!(v["errors"].is_null(), "linear graphql errors: {}", v["errors"]);
        v["data"].clone()
    }

    pub async fn viewer_id(&self) -> String {
        self.query("{ viewer { id } }", json!({})).await["viewer"]["id"].as_str().unwrap_or_default().to_string()
    }

    /// A team id to create issues in. `FORGETOP_IT_LINEAR_TEAM` may be the team's id,
    /// key, or name (resolved here); blank uses the first team.
    pub async fn team_id(&self) -> String {
        let d = self.query("{ teams(first: 250) { nodes { id key name } } }", json!({})).await;
        let nodes = d["teams"]["nodes"].as_array().cloned().unwrap_or_default();
        if let Some(want) = harness::env("FORGETOP_IT_LINEAR_TEAM") {
            if let Some(t) = nodes.iter().find(|t| {
                [t["id"].as_str(), t["key"].as_str(), t["name"].as_str()].into_iter().flatten().any(|v| v.eq_ignore_ascii_case(&want))
            }) {
                return t["id"].as_str().unwrap_or_default().to_string();
            }
            panic!("no Linear team matches FORGETOP_IT_LINEAR_TEAM='{want}' (by id/key/name)");
        }
        nodes.first().and_then(|t| t["id"].as_str()).expect("at least one team").to_string()
    }

    /// Creates an issue assigned to `assignee`; returns its id.
    pub async fn create_issue(&self, team: &str, title: &str, assignee: &str) -> String {
        let m = "mutation($t:String!,$n:String!,$a:String!){ issueCreate(input:{teamId:$t,title:$n,assigneeId:$a}){ issue { id } } }";
        let d = self.query(m, json!({ "t": team, "n": title, "a": assignee })).await;
        d["issueCreate"]["issue"]["id"].as_str().expect("issue id").to_string()
    }

    /// As [`query`](Self::query) but never panics — for teardown, which must run to the end.
    async fn try_query(&self, query: &str, variables: Value) -> Option<Value> {
        let resp = self.http.post(&self.base).json(&json!({ "query": query, "variables": variables })).send().await.ok()?;
        let v: Value = resp.json().await.ok()?;
        v["errors"].is_null().then(|| v["data"].clone())
    }

    /// Creates a project in `team`; returns its (id, url).
    pub async fn create_project(&self, team: &str, name: &str) -> (String, String) {
        let m = "mutation($n:String!,$t:[String!]!){ projectCreate(input:{name:$n,teamIds:$t}){ project { id url } } }";
        let d = self.query(m, json!({ "n": name, "t": [team] })).await;
        let project = &d["projectCreate"]["project"];
        (project["id"].as_str().expect("project id").to_string(), project["url"].as_str().expect("project url").to_string())
    }

    /// Creates a milestone in `project`; returns its id.
    pub async fn create_milestone(&self, project: &str, name: &str) -> String {
        let m = "mutation($n:String!,$p:String!){ projectMilestoneCreate(input:{name:$n,projectId:$p}){ projectMilestone { id } } }";
        let d = self.query(m, json!({ "n": name, "p": project })).await;
        d["projectMilestoneCreate"]["projectMilestone"]["id"].as_str().expect("milestone id").to_string()
    }

    /// Creates an issue assigned to `assignee` in `project`, under its `milestone` when one is
    /// given; returns its id.
    pub async fn create_issue_in_project(&self, team: &str, title: &str, assignee: &str, project: &str, milestone: Option<&str>) -> String {
        let m = "mutation($t:String!,$n:String!,$a:String!,$p:String!,$m:String){ \
                 issueCreate(input:{teamId:$t,title:$n,assigneeId:$a,projectId:$p,projectMilestoneId:$m}){ issue { id } } }";
        let d = self.query(m, json!({ "t": team, "n": title, "a": assignee, "p": project, "m": milestone })).await;
        d["issueCreate"]["issue"]["id"].as_str().expect("issue id").to_string()
    }

    /// The ids of `team`'s started workflow states, in the team's own order — the doing first,
    /// whatever comes after it (review, QA…) next.
    pub async fn started_states(&self, team: &str) -> Vec<String> {
        let q = "query($t:ID!){ workflowStates(first: 50, filter: { type: { eq: \"started\" }, team: { id: { eq: $t } } }) { nodes { id position } } }";
        let d = self.query(q, json!({ "t": team })).await;
        let mut states: Vec<(f64, String)> = d["workflowStates"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| Some((n["position"].as_f64()?, n["id"].as_str()?.to_string())))
            .collect();
        states.sort_by(|a, b| a.0.total_cmp(&b.0));
        states.into_iter().map(|(_, id)| id).collect()
    }

    /// Creates an issue assigned to `assignee`, already in workflow state `state`; returns its id.
    pub async fn create_issue_in_state(&self, team: &str, title: &str, assignee: &str, state: &str) -> String {
        let m = "mutation($t:String!,$n:String!,$a:String!,$s:String!){ issueCreate(input:{teamId:$t,title:$n,assigneeId:$a,stateId:$s}){ issue { id } } }";
        let d = self.query(m, json!({ "t": team, "n": title, "a": assignee, "s": state })).await;
        d["issueCreate"]["issue"]["id"].as_str().expect("issue id").to_string()
    }

    /// Best-effort: deletes a project, and its milestones with it.
    pub async fn delete_project(&self, id: &str) {
        let _ = self.try_query("mutation($id:String!){ projectDelete(id:$id){ success } }", json!({ "id": id })).await;
    }

    /// Best-effort, for teardown: archives an issue without panicking if Linear refuses.
    pub async fn try_archive_issue(&self, id: &str) {
        let _ = self.try_query("mutation($id:String!){ issueArchive(id:$id){ success } }", json!({ "id": id })).await;
    }

    /// Deletes fixture projects earlier runs leaked — ones this key's user created, named with
    /// the run prefix, more than an hour old (so a concurrent run's live project is left alone).
    /// Best-effort: a workspace that refuses the query costs the sweep, not the test.
    pub async fn sweep_stale_projects(&self) -> usize {
        let before = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let q = "query($p:String!,$b:DateTimeOrDuration!){ projects(first: 50, filter: { name: { startsWith: $p }, creator: { isMe: { eq: true } }, createdAt: { lt: $b } }) { nodes { id } } }";
        let Some(d) = self.try_query(q, json!({ "p": harness::SWEEP_PREFIX, "b": before })).await else { return 0 };
        let ids: Vec<String> = d["projects"]["nodes"].as_array().into_iter().flatten().filter_map(|n| n["id"].as_str().map(str::to_string)).collect();
        for id in &ids {
            self.delete_project(id).await;
        }
        ids.len()
    }

    pub async fn archive_issue(&self, id: &str) {
        let _ = self.query("mutation($id:String!){ issueArchive(id:$id){ success } }", json!({ "id": id })).await;
    }

    /// Archives fixtures earlier runs leaked — issues this key's user created, titled with the
    /// run prefix, more than an hour old (so a concurrent run's live fixture is left alone).
    /// The lifecycle test archives its own issue only if it gets to the end; one that fails
    /// first leaves it behind, and on a free workspace those count towards the active-issue
    /// limit until `issueCreate` is refused outright. Returns how many were archived.
    pub async fn sweep_stale(&self) -> usize {
        let before = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let q = "query($p:String!,$b:DateTimeOrDuration!){ issues(first: 100, filter: { title: { startsWith: $p }, creator: { isMe: { eq: true } }, createdAt: { lt: $b } }) { nodes { id } } }";
        let d = self.query(q, json!({ "p": harness::SWEEP_PREFIX, "b": before })).await;
        let ids: Vec<String> = d["issues"]["nodes"].as_array().into_iter().flatten().filter_map(|n| n["id"].as_str().map(str::to_string)).collect();
        for id in &ids {
            self.archive_issue(id).await;
        }
        ids.len()
    }
}
