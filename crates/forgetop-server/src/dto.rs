//! JSON shapes for the dashboard API, and the fetch functions that build them from the
//! same `SectionService` the TUI uses. Read-only in wave 1.

use std::sync::Arc;

use forgetop_core::config::PipelineSubscription;
use forgetop_core::domain::{
    CheckRun, CommentThread, Commit, FileChange, Notification, PipelineRun, PipelineRunStatus, ProviderType,
    PullRequest, WorkItem,
};
use forgetop_core::launchpad::{self, EntryItem, PipeInput, PrInput, WiInput};
use forgetop_core::provider::{
    PipelineRunQuery, PullRequestFilter, PullRequestQuery, PullRequestSource, WorkItemQuery,
};
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

// ---- launchpad ----

/// The domain object behind a launchpad row, tagged so the frontend can switch on it.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchpadItem {
    Pr { pull_request: PullRequest },
    Wi { work_item: WorkItem },
    Pipe { run: PipelineRun, definition_name: Option<String> },
}

/// One launchpad entry: its triage bucket (with display metadata) plus the item itself.
#[derive(Serialize)]
pub struct LaunchpadRow {
    pub bucket: &'static str,
    pub bucket_title: &'static str,
    pub column: usize,
    pub muted: bool,
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    #[serde(flatten)]
    pub item: LaunchpadItem,
}

fn is_active(status: PipelineRunStatus) -> bool {
    matches!(status, PipelineRunStatus::Queued | PipelineRunStatus::Running)
}

/// Fetches PRs for a role into launchpad inputs. `include_completed` is on for your own PRs so
/// recently-merged ones can surface.
async fn pr_inputs(sections: &SectionService, filter: PullRequestFilter, include_completed: bool) -> Vec<PrInput> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.pull_request_feeds().await {
        let query = PullRequestQuery { filter, include_completed, limit: Some(50) };
        for feed in feeds {
            if let Ok(list) = feed.source.list(&query).await {
                out.extend(list.into_iter().map(|pr| PrInput {
                    connection_id: feed.connection.connection_id().to_string(),
                    connection: feed.connection.display_name().to_string(),
                    provider: feed.connection.provider_type(),
                    pr,
                }));
            }
        }
    }
    out
}

async fn wi_inputs(sections: &SectionService) -> Vec<WiInput> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.work_item_feeds().await {
        let query = WorkItemQuery { mine_only: true, include_completed: false, limit: Some(50) };
        for feed in feeds {
            if let Ok(list) = feed.source.list(&query).await {
                out.extend(list.into_iter().map(|wi| WiInput {
                    connection_id: feed.connection.connection_id().to_string(),
                    connection: feed.connection.display_name().to_string(),
                    provider: feed.connection.provider_type(),
                    wi,
                }));
            }
        }
    }
    out
}

/// Pipeline inputs with the two derived flags the classifier needs: the pipeline's display name
/// (from discovery) and whether an in-flight run is waiting on an approval gate you can respond
/// to. Mirrors the TUI's pipeline reload.
async fn pipe_inputs(sections: &SectionService) -> Vec<PipeInput> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.pipeline_feeds().await {
        for feed in feeds {
            let def_names: std::collections::HashMap<String, String> =
                feed.source.discover().await.unwrap_or_default().into_iter().map(|d| (d.id, d.name)).collect();
            let supports = feed.source.supports_approvals();
            for query in pipe_queries(&feed.subscription) {
                if let Ok(runs) = feed.source.list_runs(&query).await {
                    for run in runs {
                        let awaiting_approval = supports
                            && is_active(run.status)
                            && feed
                                .source
                                .pending_approvals(&run.id)
                                .await
                                .map(|a| a.iter().any(|x| x.can_respond))
                                .unwrap_or(false);
                        out.push(PipeInput {
                            connection_id: feed.connection.connection_id().to_string(),
                            connection: feed.connection.display_name().to_string(),
                            provider: feed.connection.provider_type(),
                            definition_name: def_names.get(&run.definition_id).cloned(),
                            awaiting_approval,
                            run,
                        });
                    }
                }
            }
        }
    }
    out
}

pub async fn launchpad(sections: &SectionService) -> Vec<LaunchpadRow> {
    let review = pr_inputs(sections, PullRequestFilter::ReviewRequested, false).await;
    let mine = pr_inputs(sections, PullRequestFilter::Mine, true).await;
    let wis = wi_inputs(sections).await;
    let pipes = pipe_inputs(sections).await;

    launchpad::build(&review, &mine, &wis, &pipes)
        .into_iter()
        .map(|e| LaunchpadRow {
            bucket: e.bucket.key(),
            bucket_title: e.bucket.title(),
            column: e.bucket.column(),
            muted: e.bucket.muted(),
            connection_id: e.connection_id,
            connection: e.connection,
            provider: e.provider,
            item: match e.item {
                EntryItem::Pr(pr) => LaunchpadItem::Pr { pull_request: pr },
                EntryItem::Wi(wi) => LaunchpadItem::Wi { work_item: wi },
                EntryItem::Pipe { run, definition_name } => LaunchpadItem::Pipe { run, definition_name },
            },
        })
        .collect()
}

// ---- pull request detail ----

/// Everything the PR detail view needs, fetched in one shot.
#[derive(Serialize)]
pub struct PrDetail {
    pub pull_request: PullRequest,
    pub threads: Vec<CommentThread>,
    pub changes: Vec<FileChange>,
    pub checks: Vec<CheckRun>,
    pub commits: Vec<Commit>,
}

/// Resolves the PR source for a connection id (the one the action/detail is scoped to).
pub async fn pr_source(sections: &SectionService, conn: &str) -> Option<Arc<dyn PullRequestSource>> {
    sections
        .pull_request_feeds()
        .await
        .ok()?
        .into_iter()
        .find(|f| f.connection.connection_id() == conn)
        .map(|f| f.source)
}

pub async fn pr_detail(sections: &SectionService, conn: &str, id: &str) -> Option<PrDetail> {
    let source = pr_source(sections, conn).await?;
    let pull_request = source.get(id).await.ok()?;
    // The detail extras are best-effort: a provider that doesn't expose one just yields empties.
    Some(PrDetail {
        pull_request,
        threads: source.threads(id).await.unwrap_or_default(),
        changes: source.changes(id).await.unwrap_or_default(),
        checks: source.checks(id).await.unwrap_or_default(),
        commits: source.commits(id).await.unwrap_or_default(),
    })
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
