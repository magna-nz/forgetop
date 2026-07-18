//! JSON shapes for the dashboard API, and the fetch functions that build them from the
//! same `SectionService` the TUI uses. Read-only in wave 1.

use std::sync::Arc;

use forgetop_core::config::PipelineSubscription;
use forgetop_core::domain::{
    CheckRun, CommentThread, Commit, FileChange, Notification, PipelineApproval, PipelineRun, PipelineRunStatus,
    ProviderType, PullRequest, PullRequestStatus, TimelineEvent, WorkItem,
};
use forgetop_core::launchpad::{self, EntryItem, PipeInput, PrInput, WiInput};
use forgetop_core::provider::{
    PipelineRunQuery, PipelineSource, PullRequestFilter, PullRequestQuery, PullRequestSource, WorkItemQuery,
    WorkItemSource,
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
    /// Pending approval gates on this run (empty unless it's in-flight and the provider
    /// supports approvals) — drives the approve/reject buttons.
    pub approvals: Vec<PipelineApproval>,
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

/// Which slice of pull requests the PR page is showing. The provider `list(query)` does the
/// filtering (the same path the TUI uses), so each view is correct for real providers, not just demo.
#[derive(Clone, Copy)]
pub enum PrView {
    /// Open PRs across the bound connections.
    All,
    /// Your own open PRs (authored by you, still open).
    Yours,
    /// PRs you authored that have merged (newest first is applied client-side by the default sort).
    MergedByYou,
    /// PRs where you're a requested reviewer.
    ReviewRequested,
}

impl PrView {
    /// Parses the `?view=` query param; anything unrecognised (or absent) means `All`.
    pub fn parse(s: Option<&str>) -> PrView {
        match s {
            Some("yours") => PrView::Yours,
            Some("merged") => PrView::MergedByYou,
            Some("review_requested") => PrView::ReviewRequested,
            _ => PrView::All,
        }
    }

    fn query(self) -> PullRequestQuery {
        let (filter, include_completed) = match self {
            // "Merged by you" needs completed PRs; we keep only the merged ones below.
            PrView::MergedByYou => (PullRequestFilter::Mine, true),
            PrView::Yours => (PullRequestFilter::Mine, false),
            PrView::ReviewRequested => (PullRequestFilter::ReviewRequested, false),
            PrView::All => (PullRequestFilter::All, false),
        };
        PullRequestQuery { filter, include_completed, limit: Some(50) }
    }

    /// `Mine + include_completed` also returns your closed-but-not-merged PRs; drop those.
    fn keep(self, pr: &PullRequest) -> bool {
        match self {
            PrView::MergedByYou => matches!(pr.status, PullRequestStatus::Merged),
            _ => true,
        }
    }
}

pub async fn pull_requests(sections: &SectionService, view: PrView) -> Vec<PrRow> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.pull_request_feeds().await {
        let query = view.query();
        for feed in feeds {
            if let Ok(list) = feed.source.list(&query).await {
                out.extend(list.into_iter().filter(|pr| view.keep(pr)).map(|pr| PrRow {
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
            let supports = feed.source.supports_approvals();
            for query in pipe_queries(&feed.subscription) {
                if let Ok(runs) = feed.source.list_runs(&query).await {
                    for run in runs {
                        // Only in-flight runs can be waiting on a gate — bound the extra calls.
                        let approvals = if supports && is_active(run.status) {
                            feed.source.pending_approvals(&run.id).await.unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        out.push(PipeRow {
                            connection_id: feed.connection.connection_id().to_string(),
                            connection: feed.connection.display_name().to_string(),
                            provider: feed.connection.provider_type(),
                            approvals,
                            run,
                        });
                    }
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

/// Which capped reference lists had more entries than they show — drives the "more…" links.
#[derive(Serialize)]
pub struct LaunchpadMore {
    pub needs_review: bool,
    pub your_work: bool,
    pub your_open_prs: bool,
    pub recently_merged: bool,
    pub recent_pipelines: bool,
}

/// The Launchpad payload: display-ordered rows plus per-bucket overflow flags.
#[derive(Serialize)]
pub struct LaunchpadResponse {
    pub rows: Vec<LaunchpadRow>,
    pub more: LaunchpadMore,
}

pub async fn launchpad(sections: &SectionService) -> LaunchpadResponse {
    let review = pr_inputs(sections, PullRequestFilter::ReviewRequested, false).await;
    let mine = pr_inputs(sections, PullRequestFilter::Mine, true).await;
    let wis = wi_inputs(sections).await;
    let pipes = pipe_inputs(sections).await;

    let built = launchpad::build(&review, &mine, &wis, &pipes);
    let rows = built
        .entries
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
        .collect();
    LaunchpadResponse {
        rows,
        more: LaunchpadMore {
            needs_review: built.overflow.needs_review,
            your_work: built.overflow.your_work,
            your_open_prs: built.overflow.your_open_prs,
            recently_merged: built.overflow.recently_merged,
            recent_pipelines: built.overflow.recent_pipelines,
        },
    }
}

// ---- pull request detail ----

/// Everything the PR detail view needs, fetched in one shot.
#[derive(Serialize)]
pub struct PrDetail {
    pub pull_request: PullRequest,
    pub threads: Vec<CommentThread>,
    pub timeline: Vec<TimelineEvent>,
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
        timeline: source.timeline(id).await.unwrap_or_default(),
        changes: source.changes(id).await.unwrap_or_default(),
        checks: source.checks(id).await.unwrap_or_default(),
        commits: source.commits(id).await.unwrap_or_default(),
    })
}

/// Files changed by a single commit on the PR (empty for providers without a per-commit diff API).
pub async fn pr_commit_changes(sections: &SectionService, conn: &str, id: &str, sha: &str) -> Option<Vec<FileChange>> {
    let source = pr_source(sections, conn).await?;
    Some(source.commit_changes(id, sha).await.unwrap_or_default())
}

// ---- work-item detail ----

/// Everything the work-item detail view needs.
#[derive(Serialize)]
pub struct WiDetail {
    pub work_item: WorkItem,
    pub threads: Vec<CommentThread>,
    pub timeline: Vec<TimelineEvent>,
}

/// Resolves the work-item source for a connection id.
pub async fn wi_source(sections: &SectionService, conn: &str) -> Option<Arc<dyn WorkItemSource>> {
    sections
        .work_item_feeds()
        .await
        .ok()?
        .into_iter()
        .find(|f| f.connection.connection_id() == conn)
        .map(|f| f.source)
}

pub async fn wi_detail(sections: &SectionService, conn: &str, id: &str) -> Option<WiDetail> {
    let source = wi_source(sections, conn).await?;
    let work_item = source.get(id).await.ok()?;
    // Comments are best-effort: a provider that doesn't expose them just yields empties.
    Some(WiDetail {
        work_item,
        threads: source.threads(id).await.unwrap_or_default(),
        timeline: source.timeline(id).await.unwrap_or_default(),
    })
}

// ---- pipeline detail ----

/// Everything the pipeline drill-in needs: the full run (stages → jobs → steps) plus any
/// pending approval gates the current user can act on.
#[derive(Serialize)]
pub struct PipelineDetail {
    pub run: PipelineRun,
    pub approvals: Vec<PipelineApproval>,
}

/// Resolves the pipeline source for a connection id.
pub async fn pipe_source(sections: &SectionService, conn: &str) -> Option<Arc<dyn PipelineSource>> {
    sections
        .pipeline_feeds()
        .await
        .ok()?
        .into_iter()
        .find(|f| f.connection.connection_id() == conn)
        .map(|f| f.source)
}

pub async fn pipeline_detail(sections: &SectionService, conn: &str, run_id: &str) -> Option<PipelineDetail> {
    let source = pipe_source(sections, conn).await?;
    let run = source.get_run(run_id).await.ok()?;
    // Only in-flight runs can be waiting on a gate — mirror the list endpoint's bound.
    let approvals = if source.supports_approvals() && is_active(run.status) {
        source.pending_approvals(run_id).await.unwrap_or_default()
    } else {
        Vec::new()
    };
    Some(PipelineDetail { run, approvals })
}

/// Logs for a run, optionally scoped to a single job. Best-effort: `None` when the connection
/// isn't found or the provider can't supply logs.
pub async fn pipeline_logs(sections: &SectionService, conn: &str, run_id: &str, job_id: Option<&str>) -> Option<String> {
    let source = pipe_source(sections, conn).await?;
    source.logs(run_id, job_id).await.ok()
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
