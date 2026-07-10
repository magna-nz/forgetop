//! Launchpad triage engine: classifies aggregated items into action buckets.
//! Pure logic (no UI, no fetching) so the rules are unit-tested in isolation — this
//! is the part it's most important to get right.

use chrono::{DateTime, Utc};
use forgetop_core::domain::{CheckStatus, MergeableState, PipelineRun, PipelineRunStatus, ProviderType, PullRequest, WorkItem};

use crate::app::{pr_vote_flags, PipeRow, PrRow, WiRow};

/// The action bucket an item lands in. `ORDER` is the display/urgency order — the
/// things others are blocked on first, then what you can ship, then bounce-backs,
/// then your own backlog, then muted/informational.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    NeedsReview,
    ApprovalsWaiting,
    ReadyToMerge,
    NeedsFixing,
    YourWork,
    WaitingOnOthers,
    Drafts,
}

impl Bucket {
    /// Every bucket, in display (urgency) order.
    pub const ORDER: [Bucket; 7] = [
        Bucket::NeedsReview,
        Bucket::ApprovalsWaiting,
        Bucket::ReadyToMerge,
        Bucket::NeedsFixing,
        Bucket::YourWork,
        Bucket::WaitingOnOthers,
        Bucket::Drafts,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Bucket::NeedsReview => "Needs your review",
            Bucket::ApprovalsWaiting => "Approvals waiting",
            Bucket::ReadyToMerge => "Ready to merge",
            Bucket::NeedsFixing => "Needs fixing",
            Bucket::YourWork => "Assigned to you",
            Bucket::WaitingOnOthers => "Waiting on others",
            Bucket::Drafts => "Drafts",
        }
    }

    /// Muted buckets are informational — nothing for you to do right now.
    pub fn muted(&self) -> bool {
        matches!(self, Bucket::WaitingOnOthers | Bucket::Drafts)
    }

    /// Which Launchpad column this bucket lives in: 0 = left ("Needs you" — things
    /// ripe for action now), 1 = right ("Your work" — your backlog + parked PRs).
    pub fn column(&self) -> usize {
        match self {
            Bucket::NeedsReview | Bucket::ApprovalsWaiting | Bucket::ReadyToMerge | Bucket::NeedsFixing => 0,
            _ => 1,
        }
    }
}

/// Which feed a PR came from — i.e. the current user's relationship to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrRole {
    /// You're a requested reviewer.
    Reviewer,
    /// You authored it.
    Author,
}

/// Classifies a pull request into its action bucket.
pub fn classify_pr(pr: &PullRequest, role: PrRole) -> Bucket {
    match role {
        // If you're a requested reviewer, someone is blocked on you — top priority.
        PrRole::Reviewer => Bucket::NeedsReview,
        PrRole::Author => {
            if pr.is_draft {
                return Bucket::Drafts;
            }
            let (approved, changes) = pr_vote_flags(pr);
            let checks_failing = pr.checks == CheckStatus::Failed;
            let conflict = matches!(pr.mergeable, MergeableState::Conflicting);
            if changes || checks_failing || conflict {
                Bucket::NeedsFixing
            } else if approved && matches!(pr.mergeable, MergeableState::Mergeable) {
                Bucket::ReadyToMerge
            } else {
                Bucket::WaitingOnOthers
            }
        }
    }
}

/// Classifies a pipeline run, or `None` when it needs no attention.
pub fn classify_pipe(row: &PipeRow) -> Option<Bucket> {
    if row.awaiting_approval {
        Some(Bucket::ApprovalsWaiting)
    } else if matches!(row.run.status, PipelineRunStatus::Failed) {
        Some(Bucket::NeedsFixing)
    } else {
        None
    }
}

/// The kind of item behind a Launchpad row (drives what `Enter` opens).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Pr,
    Wi,
    Pipe,
}

impl EntryKind {
    /// Short type badge shown on each row.
    pub fn label(&self) -> &'static str {
        match self {
            EntryKind::Pr => "PR",
            EntryKind::Wi => "Issue",
            EntryKind::Pipe => "Pipeline",
        }
    }
}

/// The underlying domain object behind a Launchpad row, kept whole so a row can render
/// the same detail as its section's nav list.
pub enum EntryItem {
    Pr(PullRequest),
    Wi(WorkItem),
    Pipe(PipelineRun),
}

/// One actionable item on the Launchpad, resolved to its bucket + the full domain object.
pub struct Entry {
    pub bucket: Bucket,
    pub connection_id: String,
    /// Display name of the connection (for the "provider · connection" tag).
    pub connection: String,
    pub provider: ProviderType,
    pub item: EntryItem,
}

impl Entry {
    pub fn kind(&self) -> EntryKind {
        match self.item {
            EntryItem::Pr(_) => EntryKind::Pr,
            EntryItem::Wi(_) => EntryKind::Wi,
            EntryItem::Pipe(_) => EntryKind::Pipe,
        }
    }

    pub fn item_id(&self) -> &str {
        match &self.item {
            EntryItem::Pr(pr) => &pr.id,
            EntryItem::Wi(wi) => &wi.id,
            EntryItem::Pipe(run) => &run.id,
        }
    }

    pub fn title(&self) -> &str {
        match &self.item {
            EntryItem::Pr(pr) => &pr.title,
            EntryItem::Wi(wi) => &wi.title,
            EntryItem::Pipe(run) => run.name.as_deref().unwrap_or(&run.definition_id),
        }
    }

    /// Last activity, for the staleness cue + oldest-first ordering.
    pub fn updated_at(&self) -> Option<DateTime<Utc>> {
        match &self.item {
            EntryItem::Pr(pr) => pr.updated_at,
            EntryItem::Wi(wi) => wi.updated_at,
            EntryItem::Pipe(run) => run.finished_at.or(run.started_at),
        }
    }

    /// Stable identity of the underlying item, used to dismiss it from the Launchpad
    /// once you've acted on it (e.g. reviewed the PR) even before the next refetch.
    pub fn key(connection_id: &str, item_id: &str) -> String {
        format!("{connection_id}:{item_id}")
    }
}

fn bucket_rank(b: Bucket) -> usize {
    Bucket::ORDER.iter().position(|&x| x == b).unwrap_or(usize::MAX)
}

/// Sort key: known-and-older sorts before newer, unknown sorts last.
fn age_key(t: Option<DateTime<Utc>>) -> (u8, i64) {
    match t {
        Some(d) => (0, d.timestamp()),
        None => (1, 0),
    }
}

/// Builds the Launchpad rows from the aggregated feeds, already sorted into display
/// order: bucket by urgency, then oldest-activity first within each bucket.
pub fn build(prs_review: &[PrRow], prs_mine: &[PrRow], wis: &[WiRow], pipes: &[PipeRow]) -> Vec<Entry> {
    let pr_entry = |row: &PrRow, bucket: Bucket| Entry {
        bucket,
        connection_id: row.connection_id.clone(),
        connection: row.connection.clone(),
        provider: row.provider,
        item: EntryItem::Pr(row.pr.clone()),
    };

    let mut out: Vec<Entry> = Vec::new();
    out.extend(prs_review.iter().map(|r| pr_entry(r, classify_pr(&r.pr, PrRole::Reviewer))));
    out.extend(prs_mine.iter().map(|r| pr_entry(r, classify_pr(&r.pr, PrRole::Author))));
    out.extend(wis.iter().map(|r| Entry {
        bucket: Bucket::YourWork,
        connection_id: r.connection_id.clone(),
        connection: r.connection.clone(),
        provider: r.provider,
        item: EntryItem::Wi(r.wi.clone()),
    }));
    for r in pipes {
        if let Some(bucket) = classify_pipe(r) {
            out.push(Entry {
                bucket,
                connection_id: r.connection_id.clone(),
                connection: r.connection.clone(),
                provider: r.provider,
                item: EntryItem::Pipe(r.run.clone()),
            });
        }
    }

    out.sort_by(|a, b| bucket_rank(a.bucket).cmp(&bucket_rank(b.bucket)).then_with(|| age_key(a.updated_at()).cmp(&age_key(b.updated_at()))));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use forgetop_core::domain::*;

    fn user(id: &str) -> User {
        User { id: id.into(), display_name: id.into(), handle: None, avatar_url: None }
    }

    fn authored(draft: bool, votes: &[ReviewVote], checks: CheckStatus, mergeable: MergeableState) -> PullRequest {
        PullRequest {
            id: "1".into(),
            number: Some(1),
            title: "t".into(),
            description: None,
            author: user("me"),
            status: PullRequestStatus::Open,
            is_draft: draft,
            source_ref: None,
            target_ref: None,
            reviewers: votes.iter().map(|v| Reviewer { user: user("r"), vote: *v, is_required: false }).collect(),
            labels: vec![],
            checks,
            check_summary: None,
            mergeable,
            changed_files: 0,
            additions: 0,
            deletions: 0,
            created_at: None,
            updated_at: None,
            url: None,
        }
    }

    fn pipe(status: PipelineRunStatus, awaiting: bool) -> PipeRow {
        PipeRow {
            connection_id: "c".into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            awaiting_approval: awaiting,
            run: PipelineRun {
                id: "r".into(),
                definition_id: "ci".into(),
                number: Some(1),
                name: Some("CI".into()),
                status,
                triggered_by: None,
                branch: None,
                commit_sha: None,
                started_at: None,
                finished_at: None,
                url: None,
                stages: vec![],
            },
        }
    }

    #[test]
    fn reviewer_prs_always_need_review() {
        // Regardless of the PR's own state, being a requested reviewer wins.
        let pr = authored(false, &[ReviewVote::Approved], CheckStatus::Passed, MergeableState::Mergeable);
        assert_eq!(classify_pr(&pr, PrRole::Reviewer), Bucket::NeedsReview);
    }

    #[test]
    fn authored_prs_route_by_state() {
        use Bucket::*;
        let case = |draft, votes: &[ReviewVote], checks, merge| classify_pr(&authored(draft, votes, checks, merge), PrRole::Author);

        assert_eq!(case(true, &[], CheckStatus::None, MergeableState::Mergeable), Drafts);
        // Bounce-backs → needs fixing.
        assert_eq!(case(false, &[ReviewVote::Rejected], CheckStatus::Passed, MergeableState::Mergeable), NeedsFixing);
        assert_eq!(case(false, &[], CheckStatus::Failed, MergeableState::Mergeable), NeedsFixing);
        assert_eq!(case(false, &[], CheckStatus::Passed, MergeableState::Conflicting), NeedsFixing);
        // Approved + mergeable + checks fine → ship it.
        assert_eq!(case(false, &[ReviewVote::Approved], CheckStatus::Passed, MergeableState::Mergeable), ReadyToMerge);
        // Open, nothing wrong, but not yet approved → waiting on others.
        assert_eq!(case(false, &[], CheckStatus::Passed, MergeableState::Mergeable), WaitingOnOthers);
    }

    #[test]
    fn pipelines_route_to_approval_or_fixing() {
        assert_eq!(classify_pipe(&pipe(PipelineRunStatus::Running, true)), Some(Bucket::ApprovalsWaiting));
        assert_eq!(classify_pipe(&pipe(PipelineRunStatus::Failed, false)), Some(Bucket::NeedsFixing));
        assert_eq!(classify_pipe(&pipe(PipelineRunStatus::Succeeded, false)), None);
        // Awaiting approval outranks a failed status.
        assert_eq!(classify_pipe(&pipe(PipelineRunStatus::Failed, true)), Some(Bucket::ApprovalsWaiting));
    }

    #[test]
    fn bucket_order_is_urgency_first() {
        assert_eq!(Bucket::ORDER[0], Bucket::NeedsReview);
        assert!(!Bucket::NeedsReview.muted() && Bucket::Drafts.muted());
    }
}
