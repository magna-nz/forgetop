//! JSON shapes for the dashboard API, and the fetch functions that build them from the
//! same `SectionService` the TUI uses. Read-only in wave 1.

use forgetop_core::config::PipelineSubscription;
use forgetop_core::domain::{Notification, PipelineRun, ProviderType, PullRequest, WorkItem};
use forgetop_core::provider::{PipelineRunQuery, PullRequestFilter, PullRequestQuery, WorkItemQuery};
use forgetop_core::service::{ConnectionHealthService, SectionService};
use serde::Serialize;

/// One item tagged with the connection it came from (the "Provider" column in the TUI).
#[derive(Serialize)]
pub struct PrRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub pull_request: PullRequest,
}

#[derive(Serialize)]
pub struct WiRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub work_item: WorkItem,
}

#[derive(Serialize)]
pub struct PipeRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub run: PipelineRun,
}

#[derive(Serialize)]
pub struct NotifRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub notification: Notification,
}

#[derive(Serialize)]
pub struct HealthRow {
    pub connection_id: String,
    pub display_name: String,
    pub provider: ProviderType,
    pub healthy: bool,
}

pub async fn pull_requests(sections: &SectionService) -> Vec<PrRow> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.pull_request_feeds().await {
        let query = PullRequestQuery { filter: PullRequestFilter::All, include_completed: false, limit: Some(50) };
        for feed in feeds {
            if let Ok(list) = feed.source.list(&query).await {
                out.extend(list.into_iter().map(|pr| PrRow {
                    connection_id: feed.connection.connection_id().to_string(),
                    connection: feed.connection.display_name().to_string(),
                    provider: feed.connection.provider_type(),
                    pull_request: pr,
                }));
            }
        }
    }
    out
}

pub async fn work_items(sections: &SectionService) -> Vec<WiRow> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.work_item_feeds().await {
        let query = WorkItemQuery { mine_only: true, include_completed: false, limit: Some(50) };
        for feed in feeds {
            if let Ok(list) = feed.source.list(&query).await {
                out.extend(list.into_iter().map(|wi| WiRow {
                    connection_id: feed.connection.connection_id().to_string(),
                    connection: feed.connection.display_name().to_string(),
                    provider: feed.connection.provider_type(),
                    work_item: wi,
                }));
            }
        }
    }
    out
}

/// Queries for a pipeline subscription — mirrors the TUI: all recent runs when auto-discovering,
/// else the subscribed definitions.
fn pipe_queries(sub: &PipelineSubscription) -> Vec<PipelineRunQuery> {
    if sub.definition_ids.is_empty() {
        vec![PipelineRunQuery { definition_id: None, branch: None, limit: Some(20) }]
    } else {
        sub.definition_ids
            .iter()
            .map(|id| PipelineRunQuery { definition_id: Some(id.clone()), branch: None, limit: Some(10) })
            .collect()
    }
}

pub async fn pipelines(sections: &SectionService) -> Vec<PipeRow> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.pipeline_feeds().await {
        for feed in feeds {
            for query in pipe_queries(&feed.subscription) {
                if let Ok(runs) = feed.source.list_runs(&query).await {
                    out.extend(runs.into_iter().map(|run| PipeRow {
                        connection_id: feed.connection.connection_id().to_string(),
                        connection: feed.connection.display_name().to_string(),
                        provider: feed.connection.provider_type(),
                        run,
                    }));
                }
            }
        }
    }
    out
}

pub async fn notifications(sections: &SectionService) -> Vec<NotifRow> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.notification_feeds().await {
        for feed in feeds {
            if let Ok(list) = feed.source.list().await {
                out.extend(list.into_iter().map(|n| NotifRow {
                    connection_id: feed.connection.connection_id().to_string(),
                    connection: feed.connection.display_name().to_string(),
                    provider: feed.connection.provider_type(),
                    notification: n,
                }));
            }
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.notification.updated_at)); // newest first
    out
}

pub async fn health(svc: &ConnectionHealthService) -> Vec<HealthRow> {
    svc.check_all()
        .await
        .into_iter()
        .map(|h| HealthRow {
            connection_id: h.connection.id.clone(),
            display_name: h.connection.display_name.clone(),
            provider: h.connection.provider_type,
            healthy: h.healthy,
        })
        .collect()
}
