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

    /// A team id to create issues in — `FORGETOP_IT_LINEAR_TEAM` or the first team.
    pub async fn team_id(&self) -> String {
        if let Some(t) = harness::env("FORGETOP_IT_LINEAR_TEAM") {
            return t;
        }
        self.query("{ teams(first: 1) { nodes { id } } }", json!({})).await["teams"]["nodes"][0]["id"].as_str().expect("a team").to_string()
    }

    /// Creates an issue assigned to `assignee`; returns its id.
    pub async fn create_issue(&self, team: &str, title: &str, assignee: &str) -> String {
        let m = "mutation($t:String!,$n:String!,$a:String!){ issueCreate(input:{teamId:$t,title:$n,assigneeId:$a}){ issue { id } } }";
        let d = self.query(m, json!({ "t": team, "n": title, "a": assignee })).await;
        d["issueCreate"]["issue"]["id"].as_str().expect("issue id").to_string()
    }

    pub async fn archive_issue(&self, id: &str) {
        let _ = self.query("mutation($id:String!){ issueArchive(id:$id){ success } }", json!({ "id": id })).await;
    }
}
