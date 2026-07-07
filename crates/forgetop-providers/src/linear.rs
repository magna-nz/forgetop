//! Linear provider (GraphQL, work items only): mappers + a reqwest client.

use std::sync::Arc;

use async_trait::async_trait;
use forgetop_core::domain::*;
use forgetop_core::provider::*;
use forgetop_core::{Error, Result};
use reqwest::header::AUTHORIZATION;
use serde_json::{json, Value};

use crate::json::*;

fn prov<E: std::fmt::Display>(e: E) -> Error {
    Error::Provider(e.to_string())
}

const ISSUE_FIELDS: &str =
    "id identifier title description url createdAt updatedAt state { name type } assignee { id name displayName avatarUrl }";

pub fn map_user(v: &Value) -> User {
    User {
        id: get_str(v, "id").unwrap_or_else(|| "unknown".into()),
        display_name: get_str(v, "displayName").or_else(|| get_str(v, "name")).unwrap_or_else(|| "unknown".into()),
        handle: get_str(v, "name"),
        avatar_url: get_str(v, "avatarUrl"),
    }
}

pub fn map_state_category(kind: Option<&str>) -> WorkItemStateCategory {
    match kind {
        Some("triage") => WorkItemStateCategory::Triage,
        Some("backlog") => WorkItemStateCategory::Backlog,
        Some("unstarted") => WorkItemStateCategory::Unstarted,
        Some("started") => WorkItemStateCategory::Started,
        Some("completed") => WorkItemStateCategory::Completed,
        Some("canceled") => WorkItemStateCategory::Canceled,
        _ => WorkItemStateCategory::Backlog,
    }
}

pub fn map_issue(v: &Value) -> WorkItem {
    let state = get_obj(v, "state");
    WorkItem {
        id: get_str(v, "id").unwrap_or_else(|| "unknown".into()),
        identifier: get_str(v, "identifier"),
        title: get_str(v, "title").unwrap_or_else(|| "(untitled)".into()),
        description: get_str(v, "description"),
        state: state.and_then(|s| get_str(s, "name")).unwrap_or_else(|| "Unknown".into()),
        state_category: map_state_category(state.and_then(|s| get_str(s, "type")).as_deref()),
        work_item_type: None,
        assignee: get_obj(v, "assignee").map(map_user),
        created_at: get_date(v, "createdAt"),
        updated_at: get_date(v, "updatedAt"),
        url: get_str(v, "url"),
    }
}

pub fn map_comment(v: &Value) -> Comment {
    Comment {
        id: get_str(v, "id").unwrap_or_else(|| "0".into()),
        author: get_obj(v, "user").map(map_user).unwrap_or_else(|| User { id: "unknown".into(), display_name: "unknown".into(), handle: None, avatar_url: None }),
        body: get_str(v, "body").unwrap_or_default(),
        created_at: get_date(v, "createdAt"),
    }
}

pub struct LinearClient {
    http: reqwest::Client,
    base: String,
}

impl LinearClient {
    async fn query(&self, query: &str, variables: Value) -> Result<Value> {
        let resp = self.http.post(&self.base).json(&json!({ "query": query, "variables": variables })).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("Linear -> {}", resp.status())));
        }
        let v: Value = resp.json().await.map_err(prov)?;
        if let Some(errors) = v.get("errors").and_then(|e| e.as_array()) {
            if let Some(first) = errors.first() {
                return Err(Error::Provider(format!("Linear API error: {}", get_str(first, "message").unwrap_or_default())));
            }
        }
        v.get("data").cloned().ok_or_else(|| Error::Provider("Linear response had no data".into()))
    }
}

pub struct LinearWi(pub Arc<LinearClient>);

#[async_trait]
impl WorkItemSource for LinearWi {
    async fn list(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>> {
        let filter = match (query.mine_only, !query.include_completed) {
            (true, true) => json!({ "assignee": { "isMe": { "eq": true } }, "state": { "type": { "nin": ["completed", "canceled"] } } }),
            (true, false) => json!({ "assignee": { "isMe": { "eq": true } } }),
            (false, true) => json!({ "state": { "type": { "nin": ["completed", "canceled"] } } }),
            (false, false) => Value::Null,
        };
        let gql = format!("query Issues($first:Int!,$filter:IssueFilter){{ issues(first:$first,filter:$filter){{ nodes {{ {ISSUE_FIELDS} }} }} }}");
        let data = self.0.query(&gql, json!({ "first": query.limit.unwrap_or(50), "filter": filter })).await?;
        Ok(get_obj(&data, "issues").map(|i| get_arr(i, "nodes").iter().map(map_issue).collect()).unwrap_or_default())
    }
    async fn get(&self, id: &str) -> Result<WorkItem> {
        let gql = format!("query Issue($id:String!){{ issue(id:$id){{ {ISSUE_FIELDS} }} }}");
        let data = self.0.query(&gql, json!({ "id": id })).await?;
        get_obj(&data, "issue").map(map_issue).ok_or_else(|| Error::NotFound(id.into()))
    }
    async fn threads(&self, id: &str) -> Result<Vec<CommentThread>> {
        let gql = "query($id:String!){ issue(id:$id){ comments { nodes { id body createdAt user { id name displayName } } } } }";
        let data = self.0.query(gql, json!({ "id": id })).await?;
        let Some(issue) = get_obj(&data, "issue") else { return Ok(vec![]) };
        let comments: Vec<Comment> = get_obj(issue, "comments").map(|c| get_arr(c, "nodes").iter().map(map_comment).collect()).unwrap_or_default();
        Ok(if comments.is_empty() { vec![] } else { vec![CommentThread { id: format!("issue-{id}"), comments, file_path: None, line: None, is_resolved: false }] })
    }
    async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        let lookup = "query($id:String!){ issue(id:$id){ team { states { nodes { id name } } } } }";
        let data = self.0.query(lookup, json!({ "id": id })).await?;
        let team = get_obj(&data, "issue").and_then(|i| get_obj(i, "team")).ok_or_else(|| Error::Provider(format!("no team for issue '{id}'")))?;
        let state_id = get_obj(team, "states")
            .map(|s| get_arr(s, "nodes"))
            .unwrap_or(&[])
            .iter()
            .find(|n| get_str(n, "name").as_deref().map(|x| x.eq_ignore_ascii_case(state)).unwrap_or(false))
            .and_then(|n| get_str(n, "id"))
            .ok_or_else(|| Error::Provider(format!("no workflow state named '{state}'")))?;
        let mutation = "mutation($id:String!,$stateId:String!){ issueUpdate(id:$id,input:{stateId:$stateId}){ success } }";
        self.0.query(mutation, json!({ "id": id, "stateId": state_id })).await.map(|_| ())
    }
    async fn add_comment(&self, id: &str, body: &str) -> Result<()> {
        let mutation = "mutation($id:String!,$body:String!){ commentCreate(input:{issueId:$id,body:$body}){ success } }";
        self.0.query(mutation, json!({ "id": id, "body": body })).await.map(|_| ())
    }
    async fn available_states(&self, id: &str) -> Result<Vec<String>> {
        let lookup = "query($id:String!){ issue(id:$id){ team { states { nodes { name } } } } }";
        let data = self.0.query(lookup, json!({ "id": id })).await?;
        let states = get_obj(&data, "issue")
            .and_then(|i| get_obj(i, "team"))
            .and_then(|t| get_obj(t, "states"))
            .map(|s| get_arr(s, "nodes"))
            .unwrap_or(&[])
            .iter()
            .filter_map(|n| get_str(n, "name"))
            .collect();
        Ok(states)
    }
}

pub struct LinearConnection {
    id: String,
    display_name: String,
    client: Arc<LinearClient>,
    caps: Capabilities,
}

#[async_trait]
impl ProviderConnection for LinearConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }
    fn provider_type(&self) -> ProviderType {
        ProviderType::Linear
    }
    fn display_name(&self) -> &str {
        &self.display_name
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn pull_requests(&self) -> Option<Arc<dyn PullRequestSource>> {
        None
    }
    fn work_items(&self) -> Option<Arc<dyn WorkItemSource>> {
        Some(Arc::new(LinearWi(self.client.clone())))
    }
    fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
        None
    }
    async fn check(&self) -> bool {
        self.client.query("query { viewer { id } }", Value::Null).await.is_ok()
    }
}

pub fn linear_capabilities() -> Capabilities {
    Capabilities { supports_work_items: true, terminology: Terminology { work_items: "Issues".into(), ..Default::default() }, ..Default::default() }
}

pub struct LinearFactory;

impl ProviderFactory for LinearFactory {
    fn provider_type(&self) -> ProviderType {
        ProviderType::Linear
    }
    fn describe_capabilities(&self) -> Capabilities {
        linear_capabilities()
    }
    fn create(&self, connection: &Connection, secret: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = secret {
            // Linear personal API keys are sent as the raw Authorization value.
            headers.insert(AUTHORIZATION, key.parse().map_err(prov)?);
        }
        let http = reqwest::Client::builder().default_headers(headers).build().map_err(prov)?;
        let client = Arc::new(LinearClient { http, base: connection.base_url.clone().unwrap_or_else(|| "https://api.linear.app/graphql".into()) });
        Ok(Arc::new(LinearConnection { id: connection.id.clone(), display_name: connection.display_name.clone(), client, caps: linear_capabilities() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_issue_with_state_and_assignee() {
        let v: Value = serde_json::from_str(
            r#"{ "id": "iss_1", "identifier": "ENG-42", "title": "t", "url": "u", "state": { "name": "In Progress", "type": "started" }, "assignee": { "id": "u1", "name": "dan", "displayName": "Dan" } }"#,
        )
        .unwrap();
        let wi = map_issue(&v);
        assert_eq!(wi.identifier.as_deref(), Some("ENG-42"));
        assert_eq!(wi.state, "In Progress");
        assert_eq!(wi.state_category, WorkItemStateCategory::Started);
        assert_eq!(wi.assignee.unwrap().display_name, "Dan");
    }

    #[test]
    fn maps_state_categories() {
        assert_eq!(map_state_category(Some("triage")), WorkItemStateCategory::Triage);
        assert_eq!(map_state_category(Some("completed")), WorkItemStateCategory::Completed);
        assert_eq!(map_state_category(None), WorkItemStateCategory::Backlog);
    }
}
