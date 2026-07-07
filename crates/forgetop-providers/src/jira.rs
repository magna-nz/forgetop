//! Jira provider (Cloud, Work Items only): pure mappers (fixture-tested) + a reqwest
//! client. Uses REST v2 (plain-text bodies) with Basic auth (`email:api_token`).

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use forgetop_core::domain::*;
use forgetop_core::provider::*;
use forgetop_core::{Error, Result};
use reqwest::header::AUTHORIZATION;
use serde_json::{json, Value};

use crate::json::*;

fn prov<E: std::fmt::Display>(e: E) -> Error {
    Error::Provider(e.to_string())
}

fn unknown_user() -> User {
    User { id: "unknown".into(), display_name: "unknown".into(), handle: None, avatar_url: None }
}

/// Jira timestamps look like `2026-06-01T10:00:00.000+0000` — not quite RFC3339.
fn jira_date(v: &Value, key: &str) -> Option<DateTime<Utc>> {
    let s = v.get(key).and_then(|x| x.as_str())?;
    DateTime::parse_from_rfc3339(s)
        .ok()
        .or_else(|| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z").ok())
        .map(|d| d.with_timezone(&Utc))
}

// ---- mappers ----

pub fn map_user(v: &Value) -> User {
    User {
        id: get_str(v, "accountId").unwrap_or_else(|| "unknown".into()),
        display_name: get_str(v, "displayName").unwrap_or_else(|| "unknown".into()),
        handle: get_str(v, "emailAddress"),
        avatar_url: get_obj(v, "avatarUrls").and_then(|a| get_str(a, "48x48")),
    }
}

pub fn map_state_category(key: Option<&str>) -> WorkItemStateCategory {
    match key {
        Some("new") => WorkItemStateCategory::Unstarted,
        Some("indeterminate") => WorkItemStateCategory::Started,
        Some("done") => WorkItemStateCategory::Completed,
        _ => WorkItemStateCategory::Backlog,
    }
}

/// Maps a Jira issue. `site` is the base site URL used to build the browse link.
pub fn map_issue(v: &Value, site: &str) -> WorkItem {
    let fields = get_obj(v, "fields");
    let status = fields.and_then(|f| get_obj(f, "status"));
    let category = status
        .and_then(|s| get_obj(s, "statusCategory"))
        .and_then(|c| get_str(c, "key"));
    let key = get_str(v, "key");
    WorkItem {
        id: key.clone().unwrap_or_else(|| get_str(v, "id").unwrap_or_else(|| "0".into())),
        identifier: key.clone(),
        title: fields.and_then(|f| get_str(f, "summary")).unwrap_or_else(|| "(untitled)".into()),
        description: fields.and_then(|f| get_str(f, "description")),
        state: status.and_then(|s| get_str(s, "name")).unwrap_or_else(|| "Unknown".into()),
        state_category: map_state_category(category.as_deref()),
        work_item_type: fields.and_then(|f| get_obj(f, "issuetype")).and_then(|t| get_str(t, "name")),
        assignee: fields.and_then(|f| get_obj(f, "assignee")).map(map_user),
        created_at: fields.and_then(|f| jira_date(f, "created")),
        updated_at: fields.and_then(|f| jira_date(f, "updated")),
        url: key.map(|k| format!("{}/browse/{}", site.trim_end_matches('/'), k)),
    }
}

pub fn map_comment(v: &Value) -> Comment {
    Comment {
        id: get_str(v, "id").unwrap_or_else(|| "0".into()),
        author: get_obj(v, "author").map(map_user).unwrap_or_else(unknown_user),
        body: get_str(v, "body").unwrap_or_default(),
        created_at: jira_date(v, "created"),
    }
}

// ---- client ----

pub struct JiraClient {
    http: reqwest::Client,
    site: String,
    api: String,
    project: String,
}

impl JiraClient {
    async fn get_json(&self, url: &str) -> Result<Value> {
        let resp = self.http.get(url).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("GET {url} -> {}", resp.status())));
        }
        resp.json().await.map_err(prov)
    }

    async fn post_read(&self, url: &str, body: Value) -> Result<Value> {
        let resp = self.http.post(url).json(&body).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("POST {url} -> {}", resp.status())));
        }
        resp.json().await.map_err(prov)
    }

    async fn post_ok(&self, url: &str, body: Value) -> Result<()> {
        let resp = self.http.post(url).json(&body).send().await.map_err(prov)?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("POST {url} -> {}", resp.status())));
        }
        Ok(())
    }
}

pub struct JiraWi(pub Arc<JiraClient>);

#[async_trait]
impl WorkItemSource for JiraWi {
    async fn list(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>> {
        let mut jql = format!("project = {}", self.0.project);
        if !query.include_completed {
            jql.push_str(" AND statusCategory != Done");
        }
        if query.mine_only {
            jql.push_str(" AND assignee = currentUser()");
        }
        jql.push_str(" ORDER BY updated DESC");

        let body = json!({
            "jql": jql,
            "maxResults": query.limit.unwrap_or(50),
            "fields": ["summary", "description", "status", "assignee", "issuetype", "created", "updated"],
        });
        let data = self.0.post_read(&format!("{}/search", self.0.api), body).await?;
        Ok(get_arr(&data, "issues").iter().map(|i| map_issue(i, &self.0.site)).collect())
    }

    async fn get(&self, id: &str) -> Result<WorkItem> {
        let v = self.0.get_json(&format!("{}/issue/{id}", self.0.api)).await?;
        Ok(map_issue(&v, &self.0.site))
    }

    async fn threads(&self, id: &str) -> Result<Vec<CommentThread>> {
        let v = self.0.get_json(&format!("{}/issue/{id}/comment", self.0.api)).await?;
        let comments: Vec<Comment> = get_arr(&v, "comments").iter().map(map_comment).collect();
        Ok(if comments.is_empty() {
            vec![]
        } else {
            vec![CommentThread { id: format!("issue-{id}"), comments, file_path: None, line: None, is_resolved: false }]
        })
    }

    async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        // Jira changes state via a workflow transition; find the one that lands on `state`.
        let v = self.0.get_json(&format!("{}/issue/{id}/transitions", self.0.api)).await?;
        let transition_id = get_arr(&v, "transitions")
            .iter()
            .find(|t| {
                let to_name = get_obj(t, "to").and_then(|to| get_str(to, "name"));
                to_name.as_deref().map(|n| n.eq_ignore_ascii_case(state)).unwrap_or(false)
                    || get_str(t, "name").as_deref().map(|n| n.eq_ignore_ascii_case(state)).unwrap_or(false)
            })
            .and_then(|t| get_str(t, "id"))
            .ok_or_else(|| Error::Provider(format!("no transition to state '{state}'")))?;
        self.0
            .post_ok(&format!("{}/issue/{id}/transitions", self.0.api), json!({ "transition": { "id": transition_id } }))
            .await
    }

    async fn add_comment(&self, id: &str, body: &str) -> Result<()> {
        self.0.post_ok(&format!("{}/issue/{id}/comment", self.0.api), json!({ "body": body })).await
    }
    async fn available_states(&self, id: &str) -> Result<Vec<String>> {
        // The states this issue can transition to, from its workflow.
        let v = self.0.get_json(&format!("{}/issue/{id}/transitions", self.0.api)).await?;
        Ok(get_arr(&v, "transitions").iter().filter_map(|t| get_obj(t, "to").and_then(|to| get_str(to, "name"))).collect())
    }
}

pub struct JiraConnection {
    id: String,
    display_name: String,
    client: Arc<JiraClient>,
    caps: Capabilities,
}

#[async_trait]
impl ProviderConnection for JiraConnection {
    fn connection_id(&self) -> &str {
        &self.id
    }
    fn provider_type(&self) -> ProviderType {
        ProviderType::Jira
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
        Some(Arc::new(JiraWi(self.client.clone())))
    }
    fn pipelines(&self) -> Option<Arc<dyn PipelineSource>> {
        None
    }
    async fn check(&self) -> bool {
        self.client.get_json(&format!("{}/myself", self.client.api)).await.is_ok()
    }
}

pub fn jira_capabilities() -> Capabilities {
    Capabilities {
        supports_work_items: true,
        terminology: Terminology { work_items: "Issues".into(), ..Default::default() },
        ..Default::default()
    }
}

pub struct JiraFactory;

impl ProviderFactory for JiraFactory {
    fn provider_type(&self) -> ProviderType {
        ProviderType::Jira
    }
    fn describe_capabilities(&self) -> Capabilities {
        jira_capabilities()
    }
    fn create(&self, connection: &Connection, secret: Option<String>) -> Result<Arc<dyn ProviderConnection>> {
        let site = connection
            .base_url
            .clone()
            .ok_or_else(|| Error::Config("Jira connection requires a Site URL (https://your-site.atlassian.net)".into()))?;
        let project = connection.project.clone().ok_or_else(|| Error::Config("Jira connection requires a Project key".into()))?;
        let email = connection.username.clone().ok_or_else(|| Error::Config("Jira connection requires an Email".into()))?;

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(token) = secret {
            let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}"));
            headers.insert(AUTHORIZATION, format!("Basic {encoded}").parse().map_err(prov)?);
        }
        headers.insert(reqwest::header::ACCEPT, "application/json".parse().unwrap());
        let http = reqwest::Client::builder().default_headers(headers).build().map_err(prov)?;

        let site = site.trim_end_matches('/').to_string();
        let api = format!("{site}/rest/api/2");
        let client = Arc::new(JiraClient { http, site, api, project });
        Ok(Arc::new(JiraConnection {
            id: connection.id.clone(),
            display_name: connection.display_name.clone(),
            client,
            caps: jira_capabilities(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_issue_with_status_category_and_assignee() {
        let v: Value = serde_json::from_str(
            r#"{ "key": "ENG-42", "fields": {
                 "summary": "Fix the widget", "description": "details",
                 "status": { "name": "In Progress", "statusCategory": { "key": "indeterminate" } },
                 "issuetype": { "name": "Story" },
                 "assignee": { "accountId": "a1", "displayName": "Dana", "emailAddress": "d@x.io" },
                 "created": "2026-06-01T10:00:00.000+0000", "updated": "2026-06-02T11:00:00.000+0000" } }"#,
        )
        .unwrap();
        let wi = map_issue(&v, "https://acme.atlassian.net/");
        assert_eq!(wi.identifier.as_deref(), Some("ENG-42"));
        assert_eq!(wi.state, "In Progress");
        assert_eq!(wi.state_category, WorkItemStateCategory::Started);
        assert_eq!(wi.work_item_type.as_deref(), Some("Story"));
        assert_eq!(wi.assignee.unwrap().display_name, "Dana");
        assert_eq!(wi.url.as_deref(), Some("https://acme.atlassian.net/browse/ENG-42"));
        assert!(wi.created_at.is_some(), "Jira timestamp should parse");
    }

    #[test]
    fn maps_state_categories() {
        assert_eq!(map_state_category(Some("new")), WorkItemStateCategory::Unstarted);
        assert_eq!(map_state_category(Some("done")), WorkItemStateCategory::Completed);
        assert_eq!(map_state_category(None), WorkItemStateCategory::Backlog);
    }

    #[test]
    fn maps_comment() {
        let v: Value = serde_json::from_str(
            r#"{ "id": "10", "author": { "accountId": "u", "displayName": "Bob" }, "body": "nice", "created": "2026-06-01T10:00:00.000+0000" }"#,
        )
        .unwrap();
        let c = map_comment(&v);
        assert_eq!(c.author.display_name, "Bob");
        assert_eq!(c.body, "nice");
    }
}
