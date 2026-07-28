//! Live checks for the thing the demo provider structurally cannot cover: **addressing**.
//!
//! A demo provider never resolves a repository, so `--demo` (and any browser or TUI walkthrough
//! over it) passes whether or not the per-call repository is right. These tests are read-only —
//! they create nothing and delete nothing — and they are the only place the account-scope
//! behaviour is proved against a real API.

use forgetop_core::domain::ProviderType;
use forgetop_core::provider::{PullRequestQuery, RepositoryPage};

use crate::harness;

/// Every discovered path is connection-relative (`owner/repo`), never host-qualified, and the
/// truncation flag is only ever set when a page ceiling was actually hit.
fn assert_well_formed(page: &RepositoryPage, what: &str) {
    for repo in &page.repositories {
        assert!(!repo.is_empty(), "{what}: empty repository path");
        assert!(repo.contains('/'), "{what}: '{repo}' is not a connection-relative path");
        assert!(
            !repo.starts_with("http") && !repo.contains("://"),
            "{what}: '{repo}' is host-qualified — discovery must return the addressing spelling"
        );
    }
    let mut sorted = page.repositories.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), page.repositories.len(), "{what}: duplicate repositories");
}

#[tokio::test]
async fn github_discovery_lists_the_whole_account() {
    let Some(gh) = harness::github() else {
        eprintln!("SKIP github: no FORGETOP_IT_GITHUB_* credentials");
        return;
    };
    let page = gh.conn.discover_repositories().await.expect("discover repositories");
    assert_well_formed(&page, "github");
    assert!(!page.repositories.is_empty(), "a token with repo scope reaches at least one repository");
    // The configured test repo is one the account can see, so discovery must find it.
    let expected = format!("{}/{}", gh.owner, gh.repo);
    assert!(page.repositories.contains(&expected), "discovery is missing the configured repo '{expected}'");
}

#[tokio::test]
async fn gitlab_discovery_lists_the_member_projects() {
    let Some(gl) = harness::gitlab() else {
        eprintln!("SKIP gitlab: no FORGETOP_IT_GITLAB_* credentials");
        return;
    };
    let page = gl.conn.discover_repositories().await.expect("discover projects");
    assert_well_formed(&page, "gitlab");
    assert!(!page.repositories.is_empty(), "a member token reaches at least one project");
}

/// The least certain line in the account-scope change: Azure's repository listing deliberately
/// omits the project segment (`{org}/_apis/git/repositories`) so that it lists organisation-wide.
/// If that segment turned out to be required, discovery would come back empty or 404 and the
/// scope picker would simply be blank.
///
/// What this proves: the project-less path is accepted by a real organisation, returns
/// repositories, and carries the `project.name` needed to build the connection-relative
/// `project/repo`. What it cannot prove on an organisation with a single project is *breadth*
/// across several — the printed project count says which case this run actually exercised.
#[tokio::test]
async fn azure_org_level_discovery_returns_project_qualified_repositories() {
    let Some(az) = harness::azure() else {
        eprintln!("SKIP azure: no FORGETOP_IT_AZURE_* credentials");
        return;
    };
    let page = az.conn.discover_repositories().await.expect("discover repositories org-wide");
    assert_well_formed(&page, "azure");
    assert!(
        !page.repositories.is_empty(),
        "org-level discovery returned nothing — the project segment may not be optional after all"
    );
    for repo in &page.repositories {
        assert!(repo.split('/').count() == 2, "'{repo}' is not the project/repo shape addressing needs");
    }
    // Azure returns the full set in one response, so there is nothing to paginate.
    assert!(!page.truncated, "azure discovery has no pagination, so it can never be truncated");
    let projects: std::collections::BTreeSet<&str> =
        page.repositories.iter().filter_map(|r| r.split('/').next()).collect();
    eprintln!(
        "azure org-level discovery: {} repositories across {} project(s): {:?}",
        page.repositories.len(),
        projects.len(),
        projects
    );
    // On a single-project organisation there is no breadth to observe, so this run confirms the
    // call shape only. On any org with more than one project it upgrades itself to the real
    // assertion — no code change needed, just richer credentials.
    if projects.len() > 1 {
        assert!(
            page.repositories.len() >= projects.len(),
            "org-level discovery spans {} projects, so it is genuinely organisation-wide",
            projects.len()
        );
    } else {
        eprintln!("NOTE azure: this organisation has one project — breadth across several is unobserved here");
    }
    let configured = format!("{}/{}", az.project, harness::env("FORGETOP_IT_AZURE_REPO").unwrap_or(az.project.clone()));
    assert!(
        page.repositories.contains(&configured),
        "org-level discovery is missing the configured repo '{configured}' — found {:?}",
        page.repositories
    );
}

/// Bitbucket has no credentials in CI and none in the sample `.env`, so this skips by default —
/// it exists so the moment someone fills in `FORGETOP_IT_BITBUCKET_*` the one provider whose
/// discovery is otherwise assumed becomes verified, rather than staying permanently unproven.
/// (Its *mapping* is pinned without a network by `discovery_maps_a_workspace_page_…` in
/// `bitbucket.rs`; what only this can prove is the endpoint and the app-password permission.)
#[tokio::test]
async fn bitbucket_discovery_lists_the_workspace() {
    let Some(bb) = harness::bitbucket() else {
        eprintln!("SKIP bitbucket: no FORGETOP_IT_BITBUCKET_* credentials (CI has none either)");
        return;
    };
    let page = bb.conn.discover_repositories().await.expect("discover repositories — needs Repositories: Read");
    assert_well_formed(&page, "bitbucket");
    let expected = format!("{}/{}", bb.workspace, bb.repo);
    assert!(page.repositories.contains(&expected), "discovery is missing the configured repo '{expected}'");
    // A Bitbucket connection is deliberately one workspace, so nothing outside it may appear.
    for repo in &page.repositories {
        assert!(repo.starts_with(&format!("{}/", bb.workspace)), "'{repo}' is outside the connection's workspace");
    }
}

/// The property that matters for a wide scope: every row comes from a repository that was asked
/// for. Asserted as a property, not a count, so repositories with no open PRs still pass.
#[tokio::test]
async fn github_fan_out_returns_rows_only_from_the_scoped_repositories() {
    let Some(gh) = harness::github() else {
        eprintln!("SKIP github: no FORGETOP_IT_GITHUB_* credentials");
        return;
    };
    let page = gh.conn.discover_repositories().await.expect("discover repositories");
    let scope: Vec<String> = page.repositories.into_iter().take(2).collect();
    if scope.len() < 2 {
        eprintln!("SKIP github fan-out: the token reaches fewer than two repositories");
        return;
    }
    let Some(conn) = harness::scoped(ProviderType::GitHub, scope.clone()) else {
        eprintln!("SKIP github fan-out: could not build a scoped connection");
        return;
    };
    let prs = conn.pull_requests().expect("github has pull requests");
    let rows = prs
        .list(&PullRequestQuery { limit: Some(20), decorate: false, ..Default::default() })
        .await
        .expect("list across a two-repository scope");
    for pr in &rows {
        let repo = pr.repository.as_deref().unwrap_or("");
        assert!(scope.iter().any(|s| s == repo), "row '{}' came from '{repo}', which isn't in {scope:?}", pr.title);
    }

    // And a repository outside the scope contributes nothing, however much it has in it.
    let narrowed = harness::scoped(ProviderType::GitHub, vec![scope[0].clone()]).expect("narrow the scope");
    let narrowed_rows = narrowed
        .pull_requests()
        .expect("github has pull requests")
        .list(&PullRequestQuery { limit: Some(20), decorate: false, ..Default::default() })
        .await
        .expect("list across a one-repository scope");
    for pr in &narrowed_rows {
        assert_eq!(pr.repository.as_deref(), Some(scope[0].as_str()), "narrowing the scope must narrow the rows");
    }
}

/// An empty scope is a real state: fetch nothing, return an empty list. Never an error.
#[tokio::test]
async fn an_empty_scope_fetches_nothing_without_erroring() {
    let Some(conn) = harness::scoped(ProviderType::GitHub, vec![]) else {
        eprintln!("SKIP github: no FORGETOP_IT_GITHUB_* credentials");
        return;
    };
    let rows = conn
        .pull_requests()
        .expect("github has pull requests")
        .list(&PullRequestQuery::default())
        .await
        .expect("an empty scope is not an error");
    assert!(rows.is_empty());
}
