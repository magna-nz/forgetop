//! JSON shapes for the dashboard API, and the fetch functions that build them from the
//! same `SectionService` the TUI uses. Read-only in wave 1.

use std::sync::Arc;

use forgetop_core::config::PipelineSubscription;
use forgetop_core::domain::{
    CheckRun, CommentThread, Commit, FileChange, Notification, PipelineApproval, PipelineDefinition, PipelineRun,
    PipelineRunStatus, ProviderType, PullRequest, PullRequestStatus, TimelineEvent, TimelineEventKind, WorkItem,
};
use forgetop_core::launchpad::{self, EntryItem, PipeInput, PrInput, WiInput};
use forgetop_core::provider::{
    ItemRef, PipelineRunQuery, PipelineSource, PrDecoration, PullRequestFilter, PullRequestQuery, PullRequestSource,
    WorkItemQuery, WorkItemSource,
};
use forgetop_core::service::{ConnectionHealthService, SectionService};
use serde::Serialize;

fn log_fetch_failure(operation: &'static str) {
    forgetop_core::diag::log(operation, "provider fetch failed");
}

/// One item tagged with the connection it came from (the "Provider" column in the TUI).
#[derive(Serialize)]
pub struct PrRow {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub pull_request: PullRequest,
    /// True when this row's decorated fields are missing and worth fetching per row. Only the
    /// providers whose list endpoint omits them say yes, so the other three cost nothing.
    pub needs_decoration: bool,
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
    /// The pipeline (definition) name this run belongs to, resolved from discovery — shown
    /// before the run name in the list. `None` if discovery doesn't name it.
    pub definition_name: Option<String>,
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
        // The dashboard renders decorated fields (mergeable, +/-) per visible row from
        // `/api/pr/decoration`, so the list itself doesn't pay for them. On a five-repository
        // scope that is the difference between one list call per repo and ~50 extra per repo.
        PullRequestQuery { filter, include_completed, limit: Some(50), decorate: false }
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
    if let Ok(feeds) = sections.pull_request_feeds().await.inspect_err(|_| log_fetch_failure("dashboard.pull_requests.feeds")) {
        let query = view.query();
        for feed in feeds {
            // The list skipped decoration, so say whether these rows are actually missing anything.
            let needs_decoration = feed.source.list_omits_decoration();
            if let Ok(list) = feed.source.list(&query).await.inspect_err(|_| log_fetch_failure("dashboard.pull_requests.list")) {
                out.extend(list.into_iter().filter(|pr| view.keep(pr)).map(|pr| PrRow {
                    connection_id: feed.connection.connection_id().to_string(),
                    connection: feed.connection.display_name().to_string(),
                    provider: feed.connection.provider_type(),
                    pull_request: pr,
                    needs_decoration,
                }));
            }
        }
    }
    out
}

pub async fn work_items(sections: &SectionService) -> Vec<WiRow> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.work_item_feeds().await.inspect_err(|_| log_fetch_failure("dashboard.work_items.feeds")) {
        let query = WorkItemQuery { mine_only: true, include_completed: false, limit: Some(50) };
        for feed in feeds {
            if let Ok(list) = feed.source.list(&query).await.inspect_err(|_| log_fetch_failure("dashboard.work_items.list")) {
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
///
/// A subscribed definition id is only unique within its repository, so each query is addressed at
/// the repository discovery says the definition belongs to. Without that, a connection spanning
/// several repositories would ask every one of them about a definition only one of them has.
fn pipe_queries(sub: &PipelineSubscription, defs: &[PipelineDefinition]) -> Vec<PipelineRunQuery> {
    if sub.definition_ids.is_empty() {
        return vec![PipelineRunQuery { definition_id: None, repository: None, branch: None, limit: Some(20) }];
    }
    sub.definition_ids
        .iter()
        .map(|id| PipelineRunQuery {
            repository: defs.iter().find(|d| &d.id == id).and_then(|d| d.repository.clone()),
            definition_id: Some(id.clone()),
            branch: None,
            limit: Some(10),
        })
        .collect()
}

pub async fn pipelines(sections: &SectionService) -> Vec<PipeRow> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.pipeline_feeds().await.inspect_err(|_| log_fetch_failure("dashboard.pipelines.feeds")) {
        for feed in feeds {
            let defs = feed
                .source
                .discover()
                .await
                .inspect_err(|_| log_fetch_failure("dashboard.pipelines.discover"))
                .unwrap_or_default();
            let def_names: std::collections::HashMap<String, String> =
                defs.iter().map(|d| (d.id.clone(), d.name.clone())).collect();
            let supports = feed.source.supports_approvals();
            for query in pipe_queries(&feed.subscription, &defs) {
                if let Ok(runs) = feed.source.list_runs(&query).await.inspect_err(|_| log_fetch_failure("dashboard.pipelines.list")) {
                    for run in runs {
                        // Only in-flight runs can be waiting on a gate — bound the extra calls.
                        let approvals = if supports && is_active(run.status) {
                            feed.source
                                .pending_approvals(&run.item_ref())
                                .await
                                .inspect_err(|_| log_fetch_failure("dashboard.pipelines.pending_approvals"))
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        out.push(PipeRow {
                            connection_id: feed.connection.connection_id().to_string(),
                            connection: feed.connection.display_name().to_string(),
                            provider: feed.connection.provider_type(),
                            definition_name: def_names.get(&run.definition_id).cloned(),
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
    if let Ok(feeds) = sections.notification_feeds().await.inspect_err(|_| log_fetch_failure("dashboard.notifications.feeds")) {
        for feed in feeds {
            if let Ok(list) = feed.source.list().await.inspect_err(|_| log_fetch_failure("dashboard.notifications.list")) {
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

/// The query behind every Command Center PR fetch. It **decorates**, for two separate reasons —
/// either one alone would be enough:
///
/// * it *ranks* on `mergeable` ([`launchpad::classify_pr`], author branch), and
/// * it *renders* `checks` on every PR row, whatever the role.
///
/// So "decorate only the rows we rank on" would be wrong here: a review-requested row is never
/// ranked on a decorated field but is still shown with its check status, and turning decoration
/// off would blank that out. The cost stays bounded because providers decorate **after** the
/// cross-scope sort and cap, so it scales with the rows returned, not with the repository scope.
///
/// The list page is the opposite case and does turn it off — it fetches decoration per visible
/// row from `/api/pr/decoration` instead.
fn launchpad_query(filter: PullRequestFilter, include_completed: bool) -> PullRequestQuery {
    PullRequestQuery { filter, include_completed, limit: Some(50), decorate: true }
}

/// Fetches PRs for a role into launchpad inputs. `include_completed` is on for your own PRs so
/// recently-merged ones can surface.
async fn pr_inputs(sections: &SectionService, filter: PullRequestFilter, include_completed: bool) -> Vec<PrInput> {
    let mut out = Vec::new();
    if let Ok(feeds) = sections.pull_request_feeds().await.inspect_err(|_| log_fetch_failure("dashboard.launchpad.pull_request_feeds")) {
        let query = launchpad_query(filter, include_completed);
        for feed in feeds {
            if let Ok(list) = feed.source.list(&query).await.inspect_err(|_| log_fetch_failure("dashboard.launchpad.pull_requests")) {
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
    if let Ok(feeds) = sections.work_item_feeds().await.inspect_err(|_| log_fetch_failure("dashboard.launchpad.work_item_feeds")) {
        let query = WorkItemQuery { mine_only: true, include_completed: false, limit: Some(50) };
        for feed in feeds {
            if let Ok(list) = feed.source.list(&query).await.inspect_err(|_| log_fetch_failure("dashboard.launchpad.work_items")) {
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
    if let Ok(feeds) = sections.pipeline_feeds().await.inspect_err(|_| log_fetch_failure("dashboard.launchpad.pipeline_feeds")) {
        for feed in feeds {
            let defs = feed
                .source
                .discover()
                .await
                .inspect_err(|_| log_fetch_failure("dashboard.launchpad.pipeline_discovery"))
                .unwrap_or_default();
            let def_names: std::collections::HashMap<String, String> =
                defs.iter().map(|d| (d.id.clone(), d.name.clone())).collect();
            let supports = feed.source.supports_approvals();
            for query in pipe_queries(&feed.subscription, &defs) {
                if let Ok(runs) = feed.source.list_runs(&query).await.inspect_err(|_| log_fetch_failure("dashboard.launchpad.pipelines")) {
                    for run in runs {
                        let awaiting_approval = supports
                            && is_active(run.status)
                            && feed
                                .source
                                .pending_approvals(&run.item_ref())
                                .await
                                .inspect_err(|_| {
                                    log_fetch_failure("dashboard.launchpad.pipeline_approvals")
                                })
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
        .inspect_err(|_| log_fetch_failure("dashboard.pull_requests.feeds"))
        .ok()?
        .into_iter()
        .find(|f| f.connection.connection_id() == conn)
        .map(|f| f.source)
}

/// Fold each conversation comment into the timeline as a lightweight "commented" event, so the
/// activity feed is complete (the provider timelines carry actions, not comments). Inline diff
/// comments are skipped — those belong to code review, shown on the diff. Sorted oldest → newest.
fn with_comment_events(mut timeline: Vec<TimelineEvent>, threads: &[CommentThread]) -> Vec<TimelineEvent> {
    for t in threads.iter().filter(|t| t.file_path.is_none()) {
        for c in &t.comments {
            timeline.push(TimelineEvent {
                actor: Some(c.author.clone()),
                kind: TimelineEventKind::Commented,
                summary: "commented".into(),
                at: c.created_at,
            });
        }
    }
    timeline.sort_by_key(|a| a.at);
    timeline
}

pub async fn pr_detail(sections: &SectionService, conn: &str, item: &ItemRef) -> Option<PrDetail> {
    let source = pr_source(sections, conn).await?;
    let pull_request = source.get(item).await.inspect_err(|_| log_fetch_failure("dashboard.pr_detail.get")).ok()?;
    // The detail extras are best-effort: a provider that doesn't expose one just yields empties.
    let threads = source
        .threads(item)
        .await
        .inspect_err(|_| log_fetch_failure("dashboard.pr_detail.threads"))
        .unwrap_or_default();
    let timeline = with_comment_events(
        source
            .timeline(item)
            .await
            .inspect_err(|_| log_fetch_failure("dashboard.pr_detail.timeline"))
            .unwrap_or_default(),
        &threads,
    );
    Some(PrDetail {
        pull_request,
        threads,
        timeline,
        changes: source
            .changes(item)
            .await
            .inspect_err(|_| log_fetch_failure("dashboard.pr_detail.changes"))
            .unwrap_or_default(),
        checks: source
            .checks(item)
            .await
            .inspect_err(|_| log_fetch_failure("dashboard.pr_detail.checks"))
            .unwrap_or_default(),
        commits: source
            .commits(item)
            .await
            .inspect_err(|_| log_fetch_failure("dashboard.pr_detail.commits"))
            .unwrap_or_default(),
    })
}

/// The fields the PR list leaves out, for one pull request — fetched per visible row rather than
/// for every row of every repository in the scope.
pub async fn pr_decoration(sections: &SectionService, conn: &str, item: &ItemRef) -> Option<PrDecoration> {
    let source = pr_source(sections, conn).await?;
    source.decorate(item).await.inspect_err(|_| log_fetch_failure("dashboard.pr_decoration")).ok()
}

/// Files changed by a single commit on the PR (empty for providers without a per-commit diff API).
pub async fn pr_commit_changes(sections: &SectionService, conn: &str, item: &ItemRef, sha: &str) -> Option<Vec<FileChange>> {
    let source = pr_source(sections, conn).await?;
    Some(
        source
            .commit_changes(item, sha)
            .await
            .inspect_err(|_| log_fetch_failure("dashboard.pr_commit_changes"))
            .unwrap_or_default(),
    )
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
        .inspect_err(|_| log_fetch_failure("dashboard.work_items.feeds"))
        .ok()?
        .into_iter()
        .find(|f| f.connection.connection_id() == conn)
        .map(|f| f.source)
}

pub async fn wi_detail(sections: &SectionService, conn: &str, item: &ItemRef) -> Option<WiDetail> {
    let source = wi_source(sections, conn).await?;
    let work_item = source.get(item).await.inspect_err(|_| log_fetch_failure("dashboard.wi_detail.get")).ok()?;
    // Comments are best-effort: a provider that doesn't expose them just yields empties.
    let threads = source
        .threads(item)
        .await
        .inspect_err(|_| log_fetch_failure("dashboard.wi_detail.threads"))
        .unwrap_or_default();
    let timeline = with_comment_events(
        source
            .timeline(item)
            .await
            .inspect_err(|_| log_fetch_failure("dashboard.wi_detail.timeline"))
            .unwrap_or_default(),
        &threads,
    );
    Some(WiDetail { work_item, threads, timeline })
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
        .inspect_err(|_| log_fetch_failure("dashboard.pipelines.feeds"))
        .ok()?
        .into_iter()
        .find(|f| f.connection.connection_id() == conn)
        .map(|f| f.source)
}

pub async fn pipeline_detail(sections: &SectionService, conn: &str, run: &ItemRef) -> Option<PipelineDetail> {
    let source = pipe_source(sections, conn).await?;
    let run = source.get_run(run).await.inspect_err(|_| log_fetch_failure("dashboard.pipeline_detail.get")).ok()?;
    // Only in-flight runs can be waiting on a gate — mirror the list endpoint's bound.
    let approvals = if source.supports_approvals() && is_active(run.status) {
        source
            .pending_approvals(&run.item_ref())
            .await
            .inspect_err(|_| log_fetch_failure("dashboard.pipeline_detail.pending_approvals"))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Some(PipelineDetail { run, approvals })
}

/// Logs for a run, optionally scoped to a single job. Best-effort: `None` when the connection
/// isn't found or the provider can't supply logs.
pub async fn pipeline_logs(sections: &SectionService, conn: &str, run: &ItemRef, job_id: Option<&str>) -> Option<String> {
    let source = pipe_source(sections, conn).await?;
    source.logs(run, job_id).await.inspect_err(|_| log_fetch_failure("dashboard.pipeline_logs.get")).ok()
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

#[cfg(test)]
mod tests {
    use super::*;
    use forgetop_core::domain::{CheckStatus, MergeableState, Reviewer, ReviewVote, User};

    fn pr(reviewers: Vec<Reviewer>) -> PullRequest {
        PullRequest {
            repository: Some("acme/pay".into()),
            id: "7".into(),
            number: Some(7),
            title: "Add retries".into(),
            description: None,
            author: User { id: "me".into(), display_name: "me".into(), handle: Some("me".into()), avatar_url: None },
            status: PullRequestStatus::Open,
            is_draft: false,
            source_ref: None,
            target_ref: None,
            reviewers,
            labels: vec![],
            checks: CheckStatus::Passed,
            check_summary: None,
            mergeable: MergeableState::Mergeable,
            changed_files: 0,
            additions: 0,
            deletions: 0,
            created_at: None,
            updated_at: None,
            url: None,
        }
    }

    fn approver() -> Reviewer {
        Reviewer {
            user: User { id: "priya".into(), display_name: "Priya".into(), handle: None, avatar_url: None },
            vote: ReviewVote::Approved,
            is_required: false,
        }
    }

    /// Every Command Center fetch decorates — for *both* roles, not just the ranked one.
    ///
    /// It is tempting to decorate only what the classifier ranks on, since `classify_pr` reads
    /// `mergeable` on the author branch alone. That is a trap: the Command Center also *renders*
    /// `checks` on every PR row, so a review-requested row fetched undecorated still looks
    /// wrong — it silently loses its check status. Both halves are asserted below, so removing
    /// either reason still leaves the other holding this test up.
    #[test]
    fn the_launchpad_decorates_every_fetch_it_ranks_or_renders_from() {
        for filter in [PullRequestFilter::Mine, PullRequestFilter::ReviewRequested, PullRequestFilter::All] {
            assert!(launchpad_query(filter, false).decorate, "{filter:?} feeds rows the Command Center renders checks on");
        }

        // Reason 1 — ranking: the same PR lands in a different bucket on `mergeable` alone, so an
        // undecorated "mine" fetch would rank an approved, mergeable PR as not ready to merge.
        let ready = pr(vec![approver()]);
        let mut conflicting = pr(vec![approver()]);
        conflicting.mergeable = MergeableState::Conflicting;
        assert_ne!(
            launchpad::classify_pr(&ready, launchpad::PrRole::Author).map(|b| b.key()),
            launchpad::classify_pr(&conflicting, launchpad::PrRole::Author).map(|b| b.key()),
        );

        // Reason 2 — rendering: a review-requested row is never *ranked* on a decorated field…
        let mut decorated = pr(vec![]);
        decorated.mergeable = MergeableState::Conflicting;
        decorated.checks = CheckStatus::Failed;
        assert_eq!(
            launchpad::classify_pr(&pr(vec![]), launchpad::PrRole::Reviewer).map(|b| b.key()),
            launchpad::classify_pr(&decorated, launchpad::PrRole::Reviewer).map(|b| b.key()),
            "ranking alone would say the review fetch needs no decoration"
        );
        // …but `checks` is a decorated field the row is drawn with, which is why it does.
        assert_ne!(decorated.checks, CheckStatus::None, "checks is what an undecorated row would lose");
    }

    /// Only the provider whose list endpoint actually omits the decorated fields asks the
    /// dashboard to fetch them per row. GitLab, Azure and Bitbucket fill everything they have
    /// from the list payload, so asking them would be one call per row returning what we had.
    #[tokio::test]
    async fn only_providers_whose_list_omits_decoration_ask_for_a_per_row_fetch() {
        use forgetop_providers::{bitbucket, github, gitlab};
        use forgetop_core::provider::{Connection, ProviderFactory};

        let conn = |provider, organization, repository, username| Connection {
            id: "c".into(),
            provider_type: provider,
            display_name: "c".into(),
            base_url: None,
            organization,
            project: None,
            repository,
            username,
            credential_ref: None,
            repo_scope: None,
        };
        let gh = github::GitHubFactory.create(&conn(ProviderType::GitHub, None, Some("acme/pay".into()), None), None).unwrap();
        assert!(gh.pull_requests().unwrap().list_omits_decoration(), "GitHub's /pulls omits them");

        let gl = gitlab::GitLabFactory.create(&conn(ProviderType::GitLab, None, Some("g/p".into()), None), None).unwrap();
        assert!(!gl.pull_requests().unwrap().list_omits_decoration(), "GitLab fills them from the list");

        let bb = bitbucket::BitbucketFactory
            .create(&conn(ProviderType::Bitbucket, Some("ws".into()), Some("repo".into()), Some("u".into())), None)
            .unwrap();
        assert!(!bb.pull_requests().unwrap().list_omits_decoration(), "Bitbucket fills them from the list");
    }

    /// A subscribed definition id is only unique within its repository, so each pipeline query
    /// must be addressed at the repository discovery says the definition belongs to.
    #[test]
    fn pipeline_queries_are_addressed_at_the_definition_s_own_repository() {
        let defs = vec![
            PipelineDefinition { repository: Some("acme/pay".into()), id: "ci".into(), name: "CI".into(), path: None, url: None },
            PipelineDefinition { repository: Some("acme/web".into()), id: "release".into(), name: "Release".into(), path: None, url: None },
        ];
        let sub = PipelineSubscription {
            connection_id: "gh".into(),
            definition_ids: vec!["ci".into(), "release".into()],
            auto_discover_all: false,
        };
        let queries = pipe_queries(&sub, &defs);
        assert_eq!(queries[0].repository.as_deref(), Some("acme/pay"));
        assert_eq!(queries[1].repository.as_deref(), Some("acme/web"));

        // Auto-discovery has no definition to place, so it fans out over the whole scope.
        let all = PipelineSubscription { connection_id: "gh".into(), definition_ids: vec![], auto_discover_all: true };
        assert_eq!(pipe_queries(&all, &defs)[0].repository, None);
    }
}
