//! Client-side pull-request filtering (Mine / ReviewRequested) given the current user.

use crate::domain::{PullRequest, User};
use crate::provider::PullRequestFilter;

pub fn apply_pull_request_filter(prs: Vec<PullRequest>, filter: PullRequestFilter, me: Option<&str>) -> Vec<PullRequest> {
    prs.into_iter().filter(|pr| pull_request_matches(pr, filter, me)).collect()
}

/// Whether one pull request belongs in `filter` for `me` — the row-at-a-time form of
/// [`apply_pull_request_filter`], which is written in terms of it.
///
/// Exposed so a caller holding one unfiltered pool can derive the same views the providers would
/// have returned, without reimplementing the matching rules. That equivalence is the whole point:
/// two copies of "is this mine" that disagree would show a different list depending on whether the
/// rows arrived pre-filtered or were filtered locally.
///
/// `me` of `None` means the identity could not be established, and every pull request matches —
/// an unfiltered list under a "Mine" heading is wrong, but silently showing none of your pull
/// requests is worse. See [`PullRequestSource::current_user`](crate::provider::PullRequestSource::current_user).
pub fn pull_request_matches(pr: &PullRequest, filter: PullRequestFilter, me: Option<&str>) -> bool {
    let Some(me) = me else { return true };
    match filter {
        PullRequestFilter::All => true,
        PullRequestFilter::Mine => is_user(&pr.author, me),
        PullRequestFilter::ReviewRequested => pr.reviewers.iter().any(|r| is_user(&r.user, me)),
    }
}

fn is_user(user: &User, me: &str) -> bool {
    user.handle.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(me))
        || user.display_name.eq_ignore_ascii_case(me)
        || user.id.eq_ignore_ascii_case(me)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;

    fn user(id: &str, handle: &str) -> User {
        User { id: id.into(), display_name: id.into(), handle: Some(handle.into()), avatar_url: None }
    }

    fn pr(id: &str, author: User, reviewers: Vec<User>) -> PullRequest {
        PullRequest {
            repository: None,
            id: id.into(),
            number: None,
            title: id.into(),
            description: None,
            author,
            status: PullRequestStatus::Open,
            is_draft: false,
            source_ref: None,
            target_ref: None,
            reviewers: reviewers
                .into_iter()
                .map(|u| Reviewer { user: u, vote: ReviewVote::NoVote, is_required: false })
                .collect(),
            labels: vec![],
            checks: CheckStatus::None,
            check_summary: None,
            mergeable: MergeableState::Unknown,
            changed_files: 0,
            additions: 0,
            deletions: 0,
            created_at: None,
            updated_at: None,
            url: None,
        }
    }

    #[test]
    fn all_and_null_user_pass_through() {
        let prs = vec![pr("1", user("me", "alice"), vec![])];
        assert_eq!(apply_pull_request_filter(prs.clone(), PullRequestFilter::All, Some("alice")).len(), 1);
        assert_eq!(apply_pull_request_filter(prs, PullRequestFilter::Mine, None).len(), 1);
    }

    #[test]
    fn mine_matches_author() {
        let prs = vec![
            pr("1", user("me", "alice"), vec![]),
            pr("2", user("them", "bob"), vec![user("me", "alice")]),
        ];
        let result = apply_pull_request_filter(prs, PullRequestFilter::Mine, Some("alice"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "1");
    }

    #[test]
    fn review_requested_matches_reviewer() {
        let prs = vec![
            pr("1", user("me", "alice"), vec![]),
            pr("2", user("them", "bob"), vec![user("me", "alice")]),
        ];
        let result = apply_pull_request_filter(prs, PullRequestFilter::ReviewRequested, Some("alice"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "2");
    }
}
