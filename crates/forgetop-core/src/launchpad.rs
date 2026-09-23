//! Launchpad triage engine: classifies aggregated items into action buckets.
//!
//! NAMING (for humans and AI): the user-facing name for this feature is the **"Command Center"**
//! — that is the label shown in the TUI tab, the dashboard sidebar, docs, etc. The code
//! deliberately keeps the original `launchpad` / `lp` identifiers (types, fields, API routes,
//! module names) unchanged. So "Command Center" and "launchpad" refer to the same thing; only the
//! display strings say "Command Center". Do not rename the code identifiers to match.
//!
//! Pure logic — no UI, no fetching — so the triage rules are unit-tested in isolation and,
//! more importantly, **shared** by the terminal UI and the web dashboard. Both frontends map
//! their own row types onto the [`PrInput`] / [`WiInput`] / [`PipeInput`] structs here and call
//! [`build`], so the two never disagree about what "needs your review" or "ready to merge" means.

use chrono::{DateTime, Utc};

use crate::domain::{
    CheckStatus, MergeableState, PipelineRun, PipelineRunStatus, ProviderType, PullRequest, PullRequestStatus,
    ReviewVote, User, WorkItem, WorkItemStateCategory,
};

/// The action bucket an item lands in. [`Bucket::ORDER`] is the display/urgency order — the
/// things others are blocked on first, then what you can ship, then bounce-backs, then your own
/// backlog, then muted/informational.
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

    /// A stable machine-readable key for the bucket (used as the JSON tag for the web UI).
    pub fn key(&self) -> &'static str {
        match self {
            Bucket::NeedsReview => "needs_review",
            Bucket::ApprovalsWaiting => "approvals_waiting",
            Bucket::ReadyToMerge => "ready_to_merge",
            Bucket::NeedsFixing => "needs_fixing",
            Bucket::YourWork => "your_work",
            Bucket::YourOpenPrs => "your_open_prs",
            Bucket::RecentPipelines => "recent_pipelines",
            Bucket::RecentlyMerged => "recently_merged",
        }
    }

    /// Muted buckets are reference lists (dim heading, not counted in the tab badge) — they
    /// restate items shown elsewhere (your full open-PR list, recent pipelines, recently-merged).
    pub fn muted(&self) -> bool {
        matches!(self, Bucket::YourOpenPrs | Bucket::RecentPipelines | Bucket::RecentlyMerged)
    }

    /// Which Launchpad column this bucket lives in: 0 = left ("Needs you" — things ripe for
    /// action now), 1 = right ("Your work" — your PRs, items, recently merged).
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

/// `(approved, changes_requested)` rolled up from a PR's reviewer votes.
pub fn pr_vote_flags(pr: &PullRequest) -> (bool, bool) {
    let approved = pr.reviewers.iter().any(|r| matches!(r.vote, ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions));
    let changes = pr.reviewers.iter().any(|r| matches!(r.vote, ReviewVote::Rejected));
    (approved, changes)
}

/// What is standing between a pull request and its merge — the reason an authored one lands in
/// [`Bucket::NeedsFixing`], so a row can *say* what a bucket only implies.
///
/// [`classify_pr`] buckets an **authored** PR on [`pr_blocker`] being `Some`, so for those the
/// displayed reason and the bucketing reason are one judgement and cannot drift.
///
/// Frontends also render it for rows they did *not* bucket this way — a PR awaiting your review
/// can carry a conflict or another reviewer's rejection, and saying so is useful even though the
/// row sits under "Needs your review". So treat it as "what blocks this PR", not as a claim
/// about which bucket the row is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrBlocker {
    /// The branch no longer merges cleanly.
    Conflicting,
    /// A reviewer asked for changes.
    ChangesRequested,
    /// Required checks came back red.
    ChecksFailing,
}

impl PrBlocker {
    /// A short label for the row's signal cell.
    pub fn label(&self) -> &'static str {
        match self {
            PrBlocker::Conflicting => "conflicts",
            PrBlocker::ChangesRequested => "changes",
            PrBlocker::ChecksFailing => "checks",
        }
    }
}

/// What's blocking `pr`, or `None` when nothing is.
///
/// Precedence is most-fundamental first: a conflict blocks the merge no matter what review and
/// CI say, and a review asking for changes outranks red checks, since addressing the review will
/// re-run them anyway. Only the winner is shown, so the row names one thing to go and do.
///
/// Drafts are never blocked — they aren't asking anything of you yet.
pub fn pr_blocker(pr: &PullRequest) -> Option<PrBlocker> {
    if pr.is_draft {
        return None;
    }
    let (_, changes) = pr_vote_flags(pr);
    if matches!(pr.mergeable, MergeableState::Conflicting) {
        Some(PrBlocker::Conflicting)
    } else if changes {
        Some(PrBlocker::ChangesRequested)
    } else if pr.checks == CheckStatus::Failed {
        Some(PrBlocker::ChecksFailing)
    } else {
        None
    }
}

/// Where a pull request stands, as one verdict — the single thing to tell someone who just
/// opened it.
///
/// Composed from [`pr_blocker`] rather than re-deriving it, so the PR list, the Command Center
/// row and the PR screen cannot disagree about the same pull request; each renders this at a
/// different length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    /// Shipped.
    Merged,
    /// Closed without merging.
    Closed,
    /// Not open for review yet.
    Draft,
    /// Something is in the author's way — see [`PrBlocker`].
    Blocked(PrBlocker),
    /// CI hasn't finished: neither blocked nor clear.
    ChecksRunning,
    /// Nothing blocking *and* approved and mergeable — the merge button is live.
    ReadyToMerge,
    /// Nothing blocking, but not yet approved.
    NothingBlocking,
}

impl PrState {
    /// True for the states that need the author to do something.
    pub fn is_blocked(&self) -> bool {
        matches!(self, PrState::Blocked(_))
    }
}

/// Where a pull request stands. See [`PrState`].
///
/// Lifecycle is tested before anything else: a merged or closed pull request keeps whatever
/// `mergeable` and reviewer votes it had when it was open, so asking what blocks it gives a
/// stale answer. A draft isn't blocked either — it isn't asking anything of anyone yet. Only
/// then does [`pr_blocker`] get a say, and unfinished CI is reported ahead of either "clear"
/// state so a half-green pull request never reads as done.
pub fn pr_state(pr: &PullRequest) -> PrState {
    match pr.status {
        PullRequestStatus::Merged => return PrState::Merged,
        PullRequestStatus::Closed => return PrState::Closed,
        PullRequestStatus::Draft => return PrState::Draft,
        PullRequestStatus::Open => {}
    }
    if pr.is_draft {
        return PrState::Draft;
    }
    if let Some(b) = pr_blocker(pr) {
        return PrState::Blocked(b);
    }
    let running = match &pr.check_summary {
        Some(s) => s.in_progress > 0,
        None => pr.checks == CheckStatus::Pending,
    };
    if running {
        return PrState::ChecksRunning;
    }
    let (approved, _) = pr_vote_flags(pr);
    if approved && matches!(pr.mergeable, MergeableState::Mergeable) {
        PrState::ReadyToMerge
    } else {
        PrState::NothingBlocking
    }
}

/// The reviewer whose rejection is holding this pull request up, when one is known — so a row can
/// say *who* asked for changes rather than only that someone did.
pub fn pr_changes_requested_by(pr: &PullRequest) -> Option<&User> {
    pr.reviewers.iter().find(|r| r.vote == ReviewVote::Rejected).map(|r| &r.user)
}

/// The reviewer who approved, when one is known.
pub fn pr_approved_by(pr: &PullRequest) -> Option<&User> {
    pr.reviewers
        .iter()
        .find(|r| matches!(r.vote, ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions))
        .map(|r| &r.user)
}

/// The left-column action bucket a PR lands in, or `None` when there's nothing to act on right
/// now (a draft, or one just waiting on others' review — those still show in your full open-PR
/// list on the right, but not as an action item).
pub fn classify_pr(pr: &PullRequest, role: PrRole) -> Option<Bucket> {
    match role {
        // If you're a requested reviewer, someone is blocked on you — top priority.
        PrRole::Reviewer => Some(Bucket::NeedsReview),
        PrRole::Author => {
            // Not redundant with the guard inside `pr_blocker`: this one also keeps an approved,
            // mergeable *draft* out of `ReadyToMerge` below. Removing it changes behaviour.
            if pr.is_draft {
                return None;
            }
            if pr_blocker(pr).is_some() {
                Some(Bucket::NeedsFixing)
            } else {
                let (approved, _) = pr_vote_flags(pr);
                if approved && matches!(pr.mergeable, MergeableState::Mergeable) {
                    Some(Bucket::ReadyToMerge)
                } else {
                    None // open, nothing wrong, just waiting on others
                }
            }
        }
    }
}

/// The human-readable title for a pipeline run: what it was *running*, not what ran it.
///
/// Prefers the run's own title — the commit subject or triggering PR title, which is what
/// GitHub's `display_title` and Bitbucket's commit message carry — since that says what
/// changed. Falls back to [`pipe_workflow`] — the pipeline it ran under — for the providers that
/// expose no per-run title (GitLab, Azure DevOps).
///
/// A blank title counts as absent: Bitbucket builds one from the first line of the commit
/// message, which is empty when the message starts with a newline.
pub fn pipe_title<'a>(run: &'a PipelineRun, definition_name: Option<&'a str>) -> &'a str {
    run.title
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| pipe_workflow(run, definition_name))
}

/// The pipeline a run belongs to — its definition name, falling back to the run's own name and
/// finally its definition id. Shared so both frontends label a run the same way.
pub fn pipe_workflow<'a>(run: &'a PipelineRun, definition_name: Option<&'a str>) -> &'a str {
    definition_name.or(run.name.as_deref()).unwrap_or(&run.definition_id)
}

/// How many entries each right-column reference list shows before a "more…" affordance. When a
/// bucket has more than this, [`Overflow`] flags it so the frontends can link to the full page.
const NEEDS_REVIEW_MAX: usize = 5;
const YOUR_WORK_MAX: usize = 5;
const YOUR_OPEN_PRS_MAX: usize = 5;
const RECENT_MERGE_MAX: usize = 5;
const RECENT_PIPELINE_MAX: usize = 5;
/// Recency window for the "Recently merged" section.
const RECENT_MERGE_DAYS: i64 = 7;

/// True for your PRs merged within the recency window (shown in "Recently merged").
fn merged_recently(pr: &PullRequest, now: DateTime<Utc>) -> bool {
    pr.status == PullRequestStatus::Merged && pr.updated_at.map(|t| (now - t).num_days() <= RECENT_MERGE_DAYS).unwrap_or(false)
}

/// Classifies a pipeline run, or `None` when it needs no attention.
pub fn classify_pipe(status: PipelineRunStatus, awaiting_approval: bool) -> Option<Bucket> {
    if awaiting_approval {
        Some(Bucket::ApprovalsWaiting)
    } else if matches!(status, PipelineRunStatus::Failed) {
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

/// The underlying domain object behind a Launchpad row, kept whole so a row can render the same
/// detail as its section's nav list.
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
            // What the run was building — see [`pipe_title`].
            EntryItem::Pipe { run, definition_name } => pipe_title(run, definition_name.as_deref()),
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

    /// Stable identity of the underlying item, used to dismiss it from the Launchpad once you've
    /// acted on it (e.g. reviewed the PR) even before the next refetch.
    pub fn key(connection_id: &str, item_id: &str) -> String {
        format!("{connection_id}:{item_id}")
    }
}

/// A pull request plus the connection it came from — one launchpad input row.
pub struct PrInput {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub pr: PullRequest,
}

/// A work item plus its connection.
pub struct WiInput {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub wi: WorkItem,
}

/// A pipeline run plus its connection and the two derived flags the classifier needs.
pub struct PipeInput {
    pub connection_id: String,
    pub connection: String,
    pub provider: ProviderType,
    pub run: PipelineRun,
    pub definition_name: Option<String>,
    pub awaiting_approval: bool,
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

/// Whether a capped reference bucket had more entries than it shows — drives the "more…" affordance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Overflow {
    pub needs_review: bool,
    pub your_work: bool,
    pub your_open_prs: bool,
    pub recently_merged: bool,
    pub recent_pipelines: bool,
}

/// The built Launchpad: display-ordered, capped rows plus per-bucket overflow flags.
pub struct Launchpad {
    pub entries: Vec<Entry>,
    pub overflow: Overflow,
}

/// In-progress work items sort before the rest of "Assigned to you" — `Started` is the
/// provider-neutral in-progress category, so this is correct for every provider.
fn wi_in_progress_rank(e: &Entry) -> u8 {
    match &e.item {
        EntryItem::Wi(wi) if wi.state_category == WorkItemStateCategory::Started => 0,
        _ => 1,
    }
}

/// Builds the Launchpad rows from the aggregated feeds, already sorted into display order:
/// bucket by urgency, then oldest-activity first within each bucket (newest-first for the recent
/// reference lists; in-progress-first for "Assigned to you"). Each reference list is capped, with
/// [`Overflow`] flagging the ones that had more.
pub fn build(prs_review: &[PrInput], prs_mine: &[PrInput], wis: &[WiInput], pipes: &[PipeInput]) -> Launchpad {
    let pr_entry = |row: &PrInput, bucket: Bucket| Entry {
        bucket,
        connection_id: row.connection_id.clone(),
        connection: row.connection.clone(),
        provider: row.provider,
        item: EntryItem::Pr(row.pr.clone()),
    };
    let pipe_entry = |r: &PipeInput, bucket: Bucket| Entry {
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
    // Your own PRs: an action bucket on the left when there's something to do, the full open-PR
    // list on the right, and recently-merged ones as a "shipped" footer.
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
    // Pipelines: a left action bucket when they need you (approval gate / failed), and the
    // recent-runs reference list on the right.
    for r in pipes {
        if let Some(bucket) = classify_pipe(r.run.status, r.awaiting_approval) {
            out.push(pipe_entry(r, bucket));
        }
        out.push(pipe_entry(r, Bucket::RecentPipelines));
    }

    // Bucket by urgency; within a bucket oldest-first, except the recent reference lists
    // (recently merged / recent pipelines), which read newest-first, and "Assigned to you", which
    // puts in-progress items first then newest activity.
    let newest_first = |b: Bucket| matches!(b, Bucket::RecentlyMerged | Bucket::RecentPipelines);
    out.sort_by(|a, b| {
        bucket_rank(a.bucket).cmp(&bucket_rank(b.bucket)).then_with(|| {
            if a.bucket == Bucket::YourWork {
                wi_in_progress_rank(a)
                    .cmp(&wi_in_progress_rank(b))
                    .then_with(|| age_key(b.updated_at()).cmp(&age_key(a.updated_at())))
            } else if newest_first(a.bucket) {
                age_key(b.updated_at()).cmp(&age_key(a.updated_at()))
            } else {
                age_key(a.updated_at()).cmp(&age_key(b.updated_at()))
            }
        })
    });

    // Cap the buckets that deep-link to a full page/view, flagging any that had more so a "more…"
    // link can show. "Needs your review" caps here (it links to the PR Review-requested view); the
    // other left-column buckets (ready-to-merge / needs-fixing) are left whole so the dashboard can
    // expand them in place.
    let total = |bucket: Bucket| out.iter().filter(|e| e.bucket == bucket).count();
    let overflow = Overflow {
        needs_review: total(Bucket::NeedsReview) > NEEDS_REVIEW_MAX,
        your_work: total(Bucket::YourWork) > YOUR_WORK_MAX,
        your_open_prs: total(Bucket::YourOpenPrs) > YOUR_OPEN_PRS_MAX,
        recently_merged: total(Bucket::RecentlyMerged) > RECENT_MERGE_MAX,
        recent_pipelines: total(Bucket::RecentPipelines) > RECENT_PIPELINE_MAX,
    };
    let (mut review, mut work, mut open_prs, mut merged, mut pipelines) = (0, 0, 0, 0, 0);
    out.retain(|e| match e.bucket {
        Bucket::NeedsReview => {
            review += 1;
            review <= NEEDS_REVIEW_MAX
        }
        Bucket::YourWork => {
            work += 1;
            work <= YOUR_WORK_MAX
        }
        Bucket::YourOpenPrs => {
            open_prs += 1;
            open_prs <= YOUR_OPEN_PRS_MAX
        }
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
    Launchpad { entries: out, overflow }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;

    fn user(id: &str) -> User {
        User { id: id.into(), display_name: id.into(), handle: None, avatar_url: None }
    }

    fn authored(draft: bool, votes: &[ReviewVote], checks: CheckStatus, mergeable: MergeableState) -> PullRequest {
        PullRequest {
            repository: None,
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

    fn pipe(status: PipelineRunStatus, awaiting: bool) -> PipeInput {
        PipeInput {
            connection_id: "c".into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            definition_name: Some("CI Build".into()),
            awaiting_approval: awaiting,
            run: PipelineRun {
                repository: None,
                id: "r".into(),
                definition_id: "ci".into(),
                number: Some(1),
                name: Some("CI".into()),
                title: None,
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

    fn pr_row(id: &str, pr: PullRequest) -> PrInput {
        let mut pr = pr;
        pr.id = id.into();
        PrInput { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, pr }
    }

    fn wi_row(id: &str, cat: WorkItemStateCategory, updated_h: i64) -> WiInput {
        let wi = WorkItem {
            repository: None,
            id: id.into(),
            identifier: Some(id.into()),
            title: "wi".into(),
            description: None,
            state: "s".into(),
            state_category: cat,
            work_item_type: None,
            assignee: None,
            created_at: None,
            updated_at: Some(Utc::now() - chrono::Duration::hours(updated_h)),
            url: None,
        };
        WiInput { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, wi }
    }

    #[test]
    fn reviewer_prs_always_need_review() {
        let pr = authored(false, &[ReviewVote::Approved], CheckStatus::Passed, MergeableState::Mergeable);
        assert_eq!(classify_pr(&pr, PrRole::Reviewer), Some(Bucket::NeedsReview));
    }

    #[test]
    fn authored_prs_route_by_state() {
        use Bucket::*;
        let case = |draft, votes: &[ReviewVote], checks, merge| classify_pr(&authored(draft, votes, checks, merge), PrRole::Author);

        assert_eq!(case(true, &[], CheckStatus::None, MergeableState::Mergeable), None);
        assert_eq!(case(false, &[], CheckStatus::Passed, MergeableState::Mergeable), None);
        assert_eq!(case(false, &[ReviewVote::Rejected], CheckStatus::Passed, MergeableState::Mergeable), Some(NeedsFixing));
        assert_eq!(case(false, &[], CheckStatus::Failed, MergeableState::Mergeable), Some(NeedsFixing));
        assert_eq!(case(false, &[], CheckStatus::Passed, MergeableState::Conflicting), Some(NeedsFixing));
        assert_eq!(case(false, &[ReviewVote::Approved], CheckStatus::Passed, MergeableState::Mergeable), Some(ReadyToMerge));
    }

    #[test]
    fn pipelines_route_to_approval_or_fixing() {
        assert_eq!(classify_pipe(PipelineRunStatus::Running, true), Some(Bucket::ApprovalsWaiting));
        assert_eq!(classify_pipe(PipelineRunStatus::Failed, false), Some(Bucket::NeedsFixing));
        assert_eq!(classify_pipe(PipelineRunStatus::Succeeded, false), None);
        assert_eq!(classify_pipe(PipelineRunStatus::Failed, true), Some(Bucket::ApprovalsWaiting));
    }

    /// Precedence is most-fundamental first, and only the winner shows: a conflict outranks a
    /// changes-requested review, which outranks red checks.
    #[test]
    fn pr_blocker_reports_one_reason_in_precedence_order() {
        use PrBlocker::*;
        let case = |draft, votes: &[ReviewVote], checks, merge| pr_blocker(&authored(draft, votes, checks, merge));

        assert_eq!(case(false, &[], CheckStatus::Passed, MergeableState::Mergeable), None);
        assert_eq!(case(false, &[], CheckStatus::Failed, MergeableState::Mergeable), Some(ChecksFailing));
        assert_eq!(case(false, &[ReviewVote::Rejected], CheckStatus::Passed, MergeableState::Mergeable), Some(ChangesRequested));
        assert_eq!(case(false, &[], CheckStatus::Passed, MergeableState::Conflicting), Some(Conflicting));
        // Every reason at once still names the most fundamental one.
        assert_eq!(case(false, &[ReviewVote::Rejected], CheckStatus::Failed, MergeableState::Conflicting), Some(Conflicting));
        assert_eq!(case(false, &[ReviewVote::Rejected], CheckStatus::Failed, MergeableState::Mergeable), Some(ChangesRequested));
        // A draft isn't asking anything of you yet.
        assert_eq!(case(true, &[ReviewVote::Rejected], CheckStatus::Failed, MergeableState::Conflicting), None);
    }

    /// The reason a row *displays* and the reason it was *bucketed* are the same judgement, so
    /// they can't drift apart: an authored PR is in NeedsFixing exactly when it has a blocker.
    #[test]
    fn needs_fixing_agrees_with_pr_blocker() {
        let votes: [&[ReviewVote]; 3] = [&[], &[ReviewVote::Approved], &[ReviewVote::Rejected]];
        for draft in [true, false] {
            for v in votes {
                for checks in [CheckStatus::None, CheckStatus::Passed, CheckStatus::Failed] {
                    for merge in [MergeableState::Mergeable, MergeableState::Conflicting, MergeableState::Unknown] {
                        let pr = authored(draft, v, checks, merge);
                        assert_eq!(
                            classify_pr(&pr, PrRole::Author) == Some(Bucket::NeedsFixing),
                            pr_blocker(&pr).is_some(),
                            "draft={draft} votes={v:?} checks={checks:?} merge={merge:?}"
                        );
                    }
                }
            }
        }
    }

    /// The run's own title wins (it says what changed); the definition name is the fallback for
    /// providers that expose no per-run title.
    #[test]
    fn pipe_title_prefers_the_run_title() {
        let mut input = pipe(PipelineRunStatus::Succeeded, false);
        assert_eq!(pipe_title(&input.run, input.definition_name.as_deref()), "CI Build");

        input.run.title = Some("Bump axum to 0.8".into());
        assert_eq!(pipe_title(&input.run, input.definition_name.as_deref()), "Bump axum to 0.8");

        // A blank title is as good as absent.
        input.run.title = Some("   ".into());
        assert_eq!(pipe_title(&input.run, input.definition_name.as_deref()), "CI Build");

        input.run.title = None;
        input.definition_name = None;
        assert_eq!(pipe_title(&input.run, None), "CI");
        input.run.name = None;
        assert_eq!(pipe_title(&input.run, None), "ci");
    }

    #[test]
    fn pr_state_tests_lifecycle_before_anything_else() {
        use PrState::*;
        // A merged PR keeps the conflicting merge state and rejected review it had while open.
        // Asking what blocks it would give a stale answer, so lifecycle short-circuits first.
        let stale = |status| {
            let mut pr = authored(false, &[ReviewVote::Rejected], CheckStatus::Failed, MergeableState::Conflicting);
            pr.status = status;
            pr_state(&pr)
        };
        assert_eq!(stale(PullRequestStatus::Merged), Merged);
        assert_eq!(stale(PullRequestStatus::Closed), Closed);
        assert_eq!(stale(PullRequestStatus::Draft), Draft);
        // `is_draft` is the same short-circuit for providers that leave `status` as Open.
        assert_eq!(pr_state(&authored(true, &[ReviewVote::Rejected], CheckStatus::Failed, MergeableState::Conflicting)), Draft);
    }

    #[test]
    fn pr_state_reports_the_blocker_then_running_checks_then_clear() {
        use PrState::*;
        let case = |votes: &[ReviewVote], checks, merge| pr_state(&authored(false, votes, checks, merge));

        assert_eq!(case(&[], CheckStatus::Passed, MergeableState::Conflicting), Blocked(PrBlocker::Conflicting));
        assert_eq!(case(&[ReviewVote::Rejected], CheckStatus::Passed, MergeableState::Mergeable), Blocked(PrBlocker::ChangesRequested));
        assert_eq!(case(&[], CheckStatus::Failed, MergeableState::Mergeable), Blocked(PrBlocker::ChecksFailing));
        assert_eq!(case(&[], CheckStatus::Pending, MergeableState::Mergeable), ChecksRunning);
        assert_eq!(case(&[ReviewVote::Approved], CheckStatus::Passed, MergeableState::Mergeable), ReadyToMerge);
        // Green, as asked: nothing is in the author's way even with nobody having approved.
        assert_eq!(case(&[], CheckStatus::Passed, MergeableState::Mergeable), NothingBlocking);
        assert_eq!(case(&[], CheckStatus::None, MergeableState::Unknown), NothingBlocking);
        // An approval can't outrank a blocker, and unfinished CI can't read as ready.
        assert_eq!(case(&[ReviewVote::Approved], CheckStatus::Failed, MergeableState::Mergeable), Blocked(PrBlocker::ChecksFailing));
        assert_eq!(case(&[ReviewVote::Approved], CheckStatus::Pending, MergeableState::Mergeable), ChecksRunning);
    }

    /// The per-check summary is the authority on whether CI is still going: `CheckStatus` alone
    /// rounds a half-finished run to Passed or Failed.
    #[test]
    fn running_checks_come_from_the_summary_when_there_is_one() {
        let with = |successful, in_progress, failed| {
            let mut pr = authored(false, &[], CheckStatus::Passed, MergeableState::Mergeable);
            pr.check_summary = Some(CheckSummary { successful, in_progress, failed, neutral: 0 });
            pr_state(&pr)
        };
        assert_eq!(with(5, 3, 0), PrState::ChecksRunning);
        assert_eq!(with(8, 0, 0), PrState::NothingBlocking);
        // A summary saying "nothing running" beats a stale Pending roll-up.
        let mut pr = authored(false, &[], CheckStatus::Pending, MergeableState::Mergeable);
        pr.check_summary = Some(CheckSummary { successful: 8, in_progress: 0, failed: 0, neutral: 0 });
        assert_eq!(pr_state(&pr), PrState::NothingBlocking);
    }

    #[test]
    fn reviewer_lookups_name_the_person_behind_the_verdict() {
        let mut pr = authored(false, &[ReviewVote::Approved, ReviewVote::Rejected], CheckStatus::Passed, MergeableState::Mergeable);
        pr.reviewers[0].user = user("alice");
        pr.reviewers[1].user = user("sam");
        assert_eq!(pr_changes_requested_by(&pr).map(|u| u.display_name.as_str()), Some("sam"));
        assert_eq!(pr_approved_by(&pr).map(|u| u.display_name.as_str()), Some("alice"));

        let none = authored(false, &[], CheckStatus::Passed, MergeableState::Mergeable);
        assert!(pr_changes_requested_by(&none).is_none() && pr_approved_by(&none).is_none());
    }

    #[test]
    fn pr_vote_flags_rolls_up_reviewers() {
        assert_eq!(pr_vote_flags(&authored(false, &[], CheckStatus::None, MergeableState::Mergeable)), (false, false));
        assert_eq!(pr_vote_flags(&authored(false, &[ReviewVote::Approved], CheckStatus::None, MergeableState::Mergeable)), (true, false));
        assert_eq!(pr_vote_flags(&authored(false, &[ReviewVote::Rejected], CheckStatus::None, MergeableState::Mergeable)), (false, true));
    }

    #[test]
    fn build_lists_every_run_in_recent_pipelines() {
        let out = build(&[], &[], &[], &[pipe(PipelineRunStatus::Failed, false), pipe(PipelineRunStatus::Succeeded, false)]).entries;
        let buckets: Vec<Bucket> = out.iter().map(|e| e.bucket).collect();
        assert_eq!(buckets.iter().filter(|&&b| b == Bucket::RecentPipelines).count(), 2);
        assert!(buckets.contains(&Bucket::NeedsFixing));
        assert_eq!(buckets.iter().filter(|&&b| b == Bucket::ApprovalsWaiting).count(), 0);
    }

    #[test]
    fn bucket_order_is_urgency_first() {
        assert_eq!(Bucket::ORDER[0], Bucket::NeedsReview);
        assert!(!Bucket::NeedsReview.muted() && Bucket::RecentlyMerged.muted());
    }

    #[test]
    fn build_places_your_prs_in_the_full_list_and_recent_merges() {
        let now = Utc::now();
        let mut merged = authored(false, &[], CheckStatus::Passed, MergeableState::Mergeable);
        merged.status = PullRequestStatus::Merged;
        merged.updated_at = Some(now - chrono::Duration::days(1));
        let mut old = authored(false, &[], CheckStatus::Passed, MergeableState::Mergeable);
        old.status = PullRequestStatus::Merged;
        old.updated_at = Some(now - chrono::Duration::days(60));

        let mine = vec![
            pr_row("ready", authored(false, &[ReviewVote::Approved], CheckStatus::Passed, MergeableState::Mergeable)),
            pr_row("draft", authored(true, &[], CheckStatus::None, MergeableState::Mergeable)),
            pr_row("merged", merged),
            pr_row("old", old),
        ];
        let out = build(&[], &mine, &[], &[]).entries;
        let buckets = |id: &str| out.iter().filter(|e| e.item_id() == id).map(|e| e.bucket).collect::<Vec<_>>();

        assert!(buckets("ready").contains(&Bucket::ReadyToMerge) && buckets("ready").contains(&Bucket::YourOpenPrs));
        assert_eq!(buckets("draft"), vec![Bucket::YourOpenPrs]);
        assert_eq!(buckets("merged"), vec![Bucket::RecentlyMerged]);
        assert!(buckets("old").is_empty());
    }

    #[test]
    fn your_work_caps_at_five_favours_in_progress_and_flags_overflow() {
        use WorkItemStateCategory as C;
        // Four in-progress + three not = seven assigned items (updated_h = hours ago).
        let wis = vec![
            wi_row("todo1", C::Unstarted, 1),
            wi_row("prog1", C::Started, 2),
            wi_row("prog2", C::Started, 3),
            wi_row("todo2", C::Backlog, 4),
            wi_row("prog3", C::Started, 5),
            wi_row("todo3", C::Unstarted, 6),
            wi_row("prog4", C::Started, 7),
        ];
        let lp = build(&[], &[], &wis, &[]);
        let work: Vec<&str> = lp.entries.iter().filter(|e| e.bucket == Bucket::YourWork).map(|e| e.item_id()).collect();
        assert_eq!(work.len(), 5, "capped at five");
        assert!(lp.overflow.your_work, "seven assigned flags overflow");
        // In-progress first (newest activity first within the group), then the rest by recency.
        assert_eq!(&work[..4], &["prog1", "prog2", "prog3", "prog4"], "in-progress items lead");
        assert_eq!(work[4], "todo1", "then the most recently updated of the rest");
    }

    #[test]
    fn needs_review_caps_at_five_and_flags_overflow() {
        let review: Vec<PrInput> = (0..6)
            .map(|i| pr_row(&format!("r{i}"), authored(false, &[], CheckStatus::Passed, MergeableState::Mergeable)))
            .collect();
        let lp = build(&review, &[], &[], &[]);
        assert_eq!(lp.entries.iter().filter(|e| e.bucket == Bucket::NeedsReview).count(), 5, "capped at five");
        assert!(lp.overflow.needs_review, "six review requests flags overflow");
    }

    #[test]
    fn your_open_prs_caps_at_five_and_flags_overflow() {
        let mine: Vec<PrInput> = (0..6)
            .map(|i| pr_row(&format!("pr{i}"), authored(true, &[], CheckStatus::None, MergeableState::Mergeable)))
            .collect();
        let lp = build(&[], &mine, &[], &[]);
        assert_eq!(lp.entries.iter().filter(|e| e.bucket == Bucket::YourOpenPrs).count(), 5, "capped at five");
        assert!(lp.overflow.your_open_prs, "six open PRs flags overflow");
    }

    #[test]
    fn expand_buckets_are_not_capped() {
        // "Ready to merge" / "Needs fixing" are revealed in place by the dashboard, so build keeps
        // them all rather than capping at 5. Seven of your PRs fail CI → seven needs-fixing rows.
        let mine: Vec<PrInput> = (0..7)
            .map(|i| pr_row(&format!("pr{i}"), authored(false, &[], CheckStatus::Failed, MergeableState::Mergeable)))
            .collect();
        let lp = build(&[], &mine, &[], &[]);
        assert_eq!(
            lp.entries.iter().filter(|e| e.bucket == Bucket::NeedsFixing).count(),
            7,
            "needs-fixing is returned whole, not capped",
        );
    }

    #[test]
    fn recent_pipelines_cap_at_five_and_flag_overflow() {
        let pipes: Vec<PipeInput> = (0..6).map(|_| pipe(PipelineRunStatus::Succeeded, false)).collect();
        let lp = build(&[], &[], &[], &pipes);
        assert_eq!(lp.entries.iter().filter(|e| e.bucket == Bucket::RecentPipelines).count(), 5, "capped at five");
        assert!(lp.overflow.recent_pipelines, "six runs flags overflow");
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
