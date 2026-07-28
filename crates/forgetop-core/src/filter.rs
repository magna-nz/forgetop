//! Client-side pull-request filtering (Mine / ReviewRequested) given the current user.

use crate::domain::{PullRequest, User};
use crate::provider::PullRequestFilter;

pub fn apply_pull_request_filter(prs: Vec<PullRequest>, filter: PullRequestFilter, me: Option<&str>) -> Vec<PullRequest> {
    match (filter, me) {
        (PullRequestFilter::All, _) => prs,
        (_, None) => prs,
        (PullRequestFilter::Mine, Some(me)) => prs.into_iter().filter(|p| is_user(&p.author, me)).collect(),
        (PullRequestFilter::ReviewRequested, Some(me)) => {
            prs.into_iter().filter(|p| p.reviewers.iter().any(|r| is_user(&r.user, me))).collect()
        }
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
