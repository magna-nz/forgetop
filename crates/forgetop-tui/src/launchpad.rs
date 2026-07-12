//! Launchpad triage engine: classifies aggregated items into action buckets.
//! Pure logic (no UI, no fetching) so the rules are unit-tested in isolation — this
//! is the part it's most important to get right.

use chrono::{DateTime, Utc};
use forgetop_core::domain::{CheckStatus, MergeableState, PipelineRun, PipelineRunStatus, ProviderType, PullRequest, PullRequestStatus, WorkItem};

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
    YourOpenPrs,
    RecentPipelines,
    RecentlyMerged,
}

impl Bucket {
    /// Every bucket, in display (urgency) order.
    pub const ORDER: [Bucket; 8] = [
        Bucket::NeedsReview,
        Bucket::ApprovalsWaiting,
        Bucket::ReadyToMerge,
        Bucket::NeedsFixing,
        Bucket::YourWork,
        Bucket::YourOpenPrs,
        Bucket::RecentlyMerged,
        Bucket::RecentPipelines,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Bucket::NeedsReview => "Needs your review",
            Bucket::ApprovalsWaiting => "Approvals waiting",
            Bucket::ReadyToMerge => "Ready to merge",
            Bucket::NeedsFixing => "Needs fixing",
            Bucket::YourWork => "Assigned to you",
            Bucket::YourOpenPrs => "Your open pull requests",
            Bucket::RecentPipelines => "Recent pipelines",
            Bucket::RecentlyMerged => "Your recently merged pull requests",
        }
    }

    /// Muted buckets are reference lists (dim heading, not counted in the tab badge) —
    /// they restate items shown elsewhere (your full open-PR list, recent pipelines,
    /// recently-merged).
    pub fn muted(&self) -> bool {
        matches!(self, Bucket::YourOpenPrs | Bucket::RecentPipelines | Bucket::RecentlyMerged)
    }

    /// Which Launchpad column this bucket lives in: 0 = left ("Needs you" — things
    /// ripe for action now), 1 = right ("Your work" — your PRs, items, recently merged).
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

/// The left-column action bucket a PR lands in, or `None` when there's nothing to act
/// on right now (a draft, or one just waiting on others' review — those still show in
/// your full open-PR list on the right, but not as an action item).
pub fn classify_pr(pr: &PullRequest, role: PrRole) -> Option<Bucket> {
    match role {
        // If you're a requested reviewer, someone is blocked on you — top priority.
        PrRole::Reviewer => Some(Bucket::NeedsReview),
        PrRole::Author => {
            if pr.is_draft {
                return None;
            }
            let (approved, changes) = pr_vote_flags(pr);
            let checks_failing = pr.checks == CheckStatus::Failed;
            let conflict = matches!(pr.mergeable, MergeableState::Conflicting);
            if changes || checks_failing || conflict {
                Some(Bucket::NeedsFixing)
            } else if approved && matches!(pr.mergeable, MergeableState::Mergeable) {
                Some(Bucket::ReadyToMerge)
            } else {
                None // open, nothing wrong, just waiting on others
            }
        }
    }
}

/// Recency window for the "Recently merged" section, and how many to show.
const RECENT_MERGE_DAYS: i64 = 7;
const RECENT_MERGE_MAX: usize = 5;
/// How many recent pipeline runs to show in the right-column reference list.
const RECENT_PIPELINE_MAX: usize = 6;

/// True for your PRs merged within the recency window (shown in "Recently merged").
fn merged_recently(pr: &PullRequest, now: DateTime<Utc>) -> bool {
    pr.status == PullRequestStatus::Merged && pr.updated_at.map(|t| (now - t).num_days() <= RECENT_MERGE_DAYS).unwrap_or(false)
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
    Pipe { run: PipelineRun, definition_name: Option<String> },
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
            EntryItem::Pipe { .. } => EntryKind::Pipe,
        }
    }

    pub fn item_id(&self) -> &str {
        match &self.item {
            EntryItem::Pr(pr) => &pr.id,
            EntryItem::Wi(wi) => &wi.id,
            EntryItem::Pipe { run, .. } => &run.id,
        }
    }

    pub fn title(&self) -> &str {
        match &self.item {
            EntryItem::Pr(pr) => &pr.title,
            EntryItem::Wi(wi) => &wi.title,
            // The pipeline name (e.g. "CI Build"), falling back to the run name / id.
            EntryItem::Pipe { run, definition_name } => {
                definition_name.as_deref().or(run.name.as_deref()).unwrap_or(&run.definition_id)
            }
        }
    }

    /// Last activity, for the staleness cue + oldest-first ordering.
    pub fn updated_at(&self) -> Option<DateTime<Utc>> {
        match &self.item {
            EntryItem::Pr(pr) => pr.updated_at,
            EntryItem::Wi(wi) => wi.updated_at,
            EntryItem::Pipe { run, .. } => run.finished_at.or(run.started_at),
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
    let pipe_entry = |r: &PipeRow, bucket: Bucket| Entry {
        bucket,
        connection_id: r.connection_id.clone(),
        connection: r.connection.clone(),
        provider: r.provider,
        item: EntryItem::Pipe { run: r.run.clone(), definition_name: r.definition_name.clone() },
    };

    let now = Utc::now();
    let mut out: Vec<Entry> = Vec::new();

    // PRs where you're a requested reviewer → Needs your review.
    for r in prs_review {
        if let Some(bucket) = classify_pr(&r.pr, PrRole::Reviewer) {
            out.push(pr_entry(r, bucket));
        }
    }
    // Your own PRs: an action bucket on the left when there's something to do, the full
    // open-PR list on the right, and recently-merged ones as a "shipped" footer.
    for r in prs_mine {
        match r.pr.status {
            PullRequestStatus::Merged => {
                if merged_recently(&r.pr, now) {
                    out.push(pr_entry(r, Bucket::RecentlyMerged));
                }
            }
            PullRequestStatus::Closed => {} // abandoned — don't surface
            _ => {
                if let Some(bucket) = classify_pr(&r.pr, PrRole::Author) {
                    out.push(pr_entry(r, bucket));
                }
                out.push(pr_entry(r, Bucket::YourOpenPrs));
            }
        }
    }
    // Work items assigned to you → Assigned to you.
    out.extend(wis.iter().map(|r| Entry {
        bucket: Bucket::YourWork,
        connection_id: r.connection_id.clone(),
        connection: r.connection.clone(),
        provider: r.provider,
        item: EntryItem::Wi(r.wi.clone()),
    }));
    // Pipelines: a left action bucket when they need you (approval gate / failed), and
    // the recent-runs reference list on the right.
    for r in pipes {
        if let Some(bucket) = classify_pipe(r) {
            out.push(pipe_entry(r, bucket));
        }
        out.push(pipe_entry(r, Bucket::RecentPipelines));
    }

    // Bucket by urgency; within a bucket oldest-first, except the recent reference lists
    // (recently merged / recent pipelines), which read newest-first.
    let newest_first = |b: Bucket| matches!(b, Bucket::RecentlyMerged | Bucket::RecentPipelines);
    out.sort_by(|a, b| {
        bucket_rank(a.bucket).cmp(&bucket_rank(b.bucket)).then_with(|| {
            if newest_first(a.bucket) {
                age_key(b.updated_at()).cmp(&age_key(a.updated_at()))
            } else {
                age_key(a.updated_at()).cmp(&age_key(b.updated_at()))
            }
        })
    });
    // Keep the reference footers short (already newest-first after the sort).
    let (mut merged, mut pipelines) = (0, 0);
    out.retain(|e| match e.bucket {
        Bucket::RecentlyMerged => {
            merged += 1;
            merged <= RECENT_MERGE_MAX
        }
        Bucket::RecentPipelines => {
            pipelines += 1;
            pipelines <= RECENT_PIPELINE_MAX
        }
        _ => true,
    });
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
            definition_name: Some("CI Build".into()),
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
        assert_eq!(classify_pr(&pr, PrRole::Reviewer), Some(Bucket::NeedsReview));
    }

    #[test]
    fn authored_prs_route_by_state() {
        use Bucket::*;
        let case = |draft, votes: &[ReviewVote], checks, merge| classify_pr(&authored(draft, votes, checks, merge), PrRole::Author);

        // Drafts and open-but-waiting PRs aren't action items (they show in your open-PR list).
        assert_eq!(case(true, &[], CheckStatus::None, MergeableState::Mergeable), None);
        assert_eq!(case(false, &[], CheckStatus::Passed, MergeableState::Mergeable), None);
        // Bounce-backs → needs fixing.
        assert_eq!(case(false, &[ReviewVote::Rejected], CheckStatus::Passed, MergeableState::Mergeable), Some(NeedsFixing));
        assert_eq!(case(false, &[], CheckStatus::Failed, MergeableState::Mergeable), Some(NeedsFixing));
        assert_eq!(case(false, &[], CheckStatus::Passed, MergeableState::Conflicting), Some(NeedsFixing));
        // Approved + mergeable + checks fine → ship it.
        assert_eq!(case(false, &[ReviewVote::Approved], CheckStatus::Passed, MergeableState::Mergeable), Some(ReadyToMerge));
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
    fn build_lists_every_run_in_recent_pipelines() {
        let out = build(&[], &[], &[], &[pipe(PipelineRunStatus::Failed, false), pipe(PipelineRunStatus::Succeeded, false)]);
        let buckets: Vec<Bucket> = out.iter().map(|e| e.bucket).collect();
        // Both runs show in the recent-pipelines reference list …
        assert_eq!(buckets.iter().filter(|&&b| b == Bucket::RecentPipelines).count(), 2);
        // … and the failed one *also* surfaces as a left-column action.
        assert!(buckets.contains(&Bucket::NeedsFixing));
        // The succeeded run only appears in the reference list, not as an action.
        assert_eq!(buckets.iter().filter(|&&b| b == Bucket::ApprovalsWaiting).count(), 0);
    }

    #[test]
    fn bucket_order_is_urgency_first() {
        assert_eq!(Bucket::ORDER[0], Bucket::NeedsReview);
        assert!(!Bucket::NeedsReview.muted() && Bucket::RecentlyMerged.muted());
    }

    #[test]
    fn build_places_your_prs_in_the_full_list_and_recent_merges() {
        let row = |id: &str, pr: PullRequest| {
            let mut pr = pr;
            pr.id = id.into();
            PrRow { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, pr }
        };
        let now = Utc::now();
        let mut merged = authored(false, &[], CheckStatus::Passed, MergeableState::Mergeable);
        merged.status = PullRequestStatus::Merged;
        merged.updated_at = Some(now - chrono::Duration::days(1));
        let mut old = authored(false, &[], CheckStatus::Passed, MergeableState::Mergeable);
        old.status = PullRequestStatus::Merged;
        old.updated_at = Some(now - chrono::Duration::days(60));

        let mine = vec![
            row("ready", authored(false, &[ReviewVote::Approved], CheckStatus::Passed, MergeableState::Mergeable)),
            row("draft", authored(true, &[], CheckStatus::None, MergeableState::Mergeable)),
            row("merged", merged),
            row("old", old),
        ];
        let out = build(&[], &mine, &[], &[]);
        let buckets = |id: &str| out.iter().filter(|e| e.item_id() == id).map(|e| e.bucket).collect::<Vec<_>>();

        // A ready-to-merge PR shows both as a left action and in the full open-PR list.
        assert!(buckets("ready").contains(&Bucket::ReadyToMerge) && buckets("ready").contains(&Bucket::YourOpenPrs));
        // A draft is only in the full list (no action).
        assert_eq!(buckets("draft"), vec![Bucket::YourOpenPrs]);
        // A fresh merge shows in Recently merged; a 60-day-old one is dropped entirely.
        assert_eq!(buckets("merged"), vec![Bucket::RecentlyMerged]);
        assert!(buckets("old").is_empty());
    }

    #[test]
    fn merged_recently_respects_the_window() {
        let now = Utc::now();
        let mut pr = authored(false, &[], CheckStatus::Passed, MergeableState::Mergeable);
        pr.status = PullRequestStatus::Merged;
        pr.updated_at = Some(now - chrono::Duration::days(2));
        assert!(merged_recently(&pr, now));
        pr.updated_at = Some(now - chrono::Duration::days(30));
        assert!(!merged_recently(&pr, now), "old merges drop off");
        pr.status = PullRequestStatus::Open;
        pr.updated_at = Some(now);
        assert!(!merged_recently(&pr, now), "open PRs aren't 'recently merged'");
    }
}
