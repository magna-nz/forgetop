//! Repository-scope machinery shared by the repo-addressed providers (GitHub, GitLab, Azure
//! DevOps, Bitbucket).
//!
//! A connection is an *account*, and what it fetches is a user-chosen set of repositories. Two
//! things follow, and both live here so all four providers behave the same way:
//!
//! * **Addressing** — a per-call repository, resolved from an [`ItemRef`]. With one repository in
//!   scope an unaddressed ref still resolves; with more than one there is no "own" repository to
//!   fall back on, so it is an error rather than a guess.
//! * **Fan-out** — list calls run over the whole scope with bounded concurrency, and the results
//!   are sorted and capped **once across the scope**, never per repository.

use std::future::Future;

use forgetop_core::provider::ItemRef;
use forgetop_core::{Error, Result};

/// How many repositories are fetched at once. Small: a wide scope should stay polite to the
/// provider's rate limit rather than finish a few hundred milliseconds sooner.
pub const MAX_CONCURRENT: usize = 4;

/// Resolves the repository to address a call at.
///
/// The ref's own repository wins. Only when it has none does the scope decide, and only a
/// single-repository scope can: a wider scope has no "own" repository, and quietly picking one
/// would act on the wrong repository instead of failing.
pub fn resolve_repo(item: &ItemRef, scope: &[String]) -> Result<String> {
    if let Some(repo) = &item.repo {
        return Ok(repo.clone());
    }
    match scope {
        [only] => Ok(only.clone()),
        [] => Err(Error::Config(
            "this connection has no repositories selected — choose some in the repository scope picker".into(),
        )),
        _ => Err(Error::Config(format!(
            "this connection spans {} repositories, so '{}' needs the repository it belongs to",
            scope.len(),
            item.id
        ))),
    }
}

/// Runs `f` once per repository in `scope`, at most [`MAX_CONCURRENT`] at a time, and returns
/// every row that came back.
///
/// A repository whose call fails is logged and dropped, not propagated: four repositories out of
/// five is a better answer than an error because of the fifth.
pub async fn fan_out<T, F, Fut>(scope: &[String], operation: &'static str, f: F) -> Vec<T>
where
    T: Send + 'static,
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Vec<T>>>,
{
    let mut out = Vec::new();
    for chunk in scope.chunks(MAX_CONCURRENT) {
        let results = futures_util::future::join_all(chunk.iter().map(|repo| f(repo.clone()))).await;
        for (repo, result) in chunk.iter().zip(results) {
            match result {
                Ok(rows) => out.extend(rows),
                Err(_) => forgetop_core::diag::log(operation, &format!("fetch failed for '{repo}' — skipping it")),
            }
        }
    }
    out
}

/// Orders rows newest-first and caps them **once across the whole scope**.
///
/// Concatenating each repository's already-capped page would put repo A's stale rows above repo
/// B's fresh ones and then call the result "the 50 most recent". Sorting is skipped for a
/// single-repository scope so such a connection keeps the provider's own ordering exactly, and
/// the cap is then a no-op (the request already asked for `limit`).
pub fn sort_and_cap<T, K: Ord>(mut rows: Vec<T>, scope_len: usize, limit: Option<u32>, key: impl Fn(&T) -> K) -> Vec<T> {
    if scope_len > 1 {
        rows.sort_by_key(|row| std::cmp::Reverse(key(row)));
    }
    if let Some(limit) = limit {
        rows.truncate(limit as usize);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use forgetop_core::provider::ItemRef;

    #[test]
    fn an_addressed_ref_always_wins() {
        let scope = vec!["acme/pay".to_string(), "acme/ledger".to_string()];
        assert_eq!(resolve_repo(&ItemRef::in_repo("acme/ledger", "7"), &scope).unwrap(), "acme/ledger");
    }

    #[test]
    fn a_single_repository_scope_resolves_an_unaddressed_ref() {
        // The pre-existing single-repository connection: nothing needs to change for it.
        let scope = vec!["acme/pay".to_string()];
        assert_eq!(resolve_repo(&ItemRef::new("7"), &scope).unwrap(), "acme/pay");
    }

    #[test]
    fn a_wider_scope_refuses_to_guess() {
        let scope = vec!["acme/pay".to_string(), "acme/ledger".to_string()];
        let err = resolve_repo(&ItemRef::new("7"), &scope).unwrap_err().to_string();
        assert!(err.contains("spans 2 repositories"), "got: {err}");
        // An empty scope is a real state, and says so rather than guessing either.
        assert!(resolve_repo(&ItemRef::new("7"), &[]).unwrap_err().to_string().contains("no repositories selected"));
    }

    #[tokio::test]
    async fn fan_out_drops_the_repository_that_failed_and_keeps_the_rest() {
        let scope: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let rows = fan_out(&scope, "test.fanout", |repo| async move {
            if repo == "b" {
                Err(Error::Provider("boom".into()))
            } else {
                Ok(vec![repo])
            }
        })
        .await;
        assert_eq!(rows, vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn capping_happens_once_across_the_scope() {
        // Repo A's stale rows must not outrank repo B's fresh ones just because A was fetched first.
        let rows = vec![("a", 1), ("a", 2), ("b", 9), ("b", 8)];
        let capped = sort_and_cap(rows, 2, Some(2), |r| r.1);
        assert_eq!(capped, vec![("b", 9), ("b", 8)]);
    }

    #[test]
    fn a_single_repository_scope_keeps_provider_order() {
        let rows = vec![("a", 1), ("a", 9), ("a", 5)];
        assert_eq!(sort_and_cap(rows.clone(), 1, None, |r| r.1), rows, "provider order preserved");
    }
}
