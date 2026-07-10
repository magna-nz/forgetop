//! Launchpad triage engine: classifies aggregated items into action buckets.
//! Pure logic (no UI, no fetching) so the rules are unit-tested in isolation — this
//! is the part it's most important to get right.

use forgetop_core::domain::{CheckStatus, MergeableState, PipelineRunStatus, PullRequest};

use crate::app::{pr_vote_flags, PipeRow};

/// The action bucket an item lands in. `ORDER` is the display/urgency order — the
/// things others are blocked on first, then what you can ship, then bounce-backs,
/// then your own backlog, then muted/informational.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    NeedsReview,
    ApprovalsWaiting,
    ReadyToMerge,
    NeedsAttention,
    Broken,
    YourWork,
    WaitingOnOthers,
    Drafts,
}

impl Bucket {
    /// Every bucket, in display (urgency) order.
    pub const ORDER: [Bucket; 8] = [
        Bucket::NeedsReview,
        Bucket::ApprovalsWaiting,
        Bucket::ReadyToMerge,
        Bucket::NeedsAttention,
        Bucket::Broken,
        Bucket::YourWork,
        Bucket::WaitingOnOthers,
        Bucket::Drafts,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Bucket::NeedsReview => "Needs your review",
            Bucket::ApprovalsWaiting => "Approvals waiting",
            Bucket::ReadyToMerge => "Ready to merge",
            Bucket::NeedsAttention => "Needs your attention",
            Bucket::Broken => "Broken",
            Bucket::YourWork => "Your work",
            Bucket::WaitingOnOthers => "Waiting on others",
            Bucket::Drafts => "Drafts",
        }
    }

    /// Muted buckets are informational — nothing for you to do right now.
    pub fn muted(&self) -> bool {
        matches!(self, Bucket::WaitingOnOthers | Bucket::Drafts)
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
                Bucket::NeedsAttention
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
        Some(Bucket::Broken)
    } else {
        None
    }
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
        // Bounce-backs → attention.
        assert_eq!(case(false, &[ReviewVote::Rejected], CheckStatus::Passed, MergeableState::Mergeable), NeedsAttention);
        assert_eq!(case(false, &[], CheckStatus::Failed, MergeableState::Mergeable), NeedsAttention);
        assert_eq!(case(false, &[], CheckStatus::Passed, MergeableState::Conflicting), NeedsAttention);
        // Approved + mergeable + checks fine → ship it.
        assert_eq!(case(false, &[ReviewVote::Approved], CheckStatus::Passed, MergeableState::Mergeable), ReadyToMerge);
        // Open, nothing wrong, but not yet approved → waiting on others.
        assert_eq!(case(false, &[], CheckStatus::Passed, MergeableState::Mergeable), WaitingOnOthers);
    }

    #[test]
    fn pipelines_route_to_approval_or_broken() {
        assert_eq!(classify_pipe(&pipe(PipelineRunStatus::Running, true)), Some(Bucket::ApprovalsWaiting));
        assert_eq!(classify_pipe(&pipe(PipelineRunStatus::Failed, false)), Some(Bucket::Broken));
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
