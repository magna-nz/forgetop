//! Command palette (`Ctrl-P`) — a global fuzzy jump across every already-fetched item.
//!
//! This module is the **pure core**: a flat [`PaletteItem`] list built from the app's
//! PR / work-item / pipeline rows, and [`rank`], which fuzzy-filters and orders them for a
//! query. No UI, no I/O, no provider calls — everything operates on data already in memory.

use chrono::{DateTime, Utc};
use forgetop_core::domain::User;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

use crate::app::{PipeRow, PrRow, WiRow};

/// Which kind of item a palette result routes to — decides the "open" path on `Enter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKind {
    Pr,
    Wi,
    Pipe,
}

/// One searchable row in the palette. Holds only what's needed to display a result and to
/// re-open the underlying item — never the heavy domain struct itself. The app re-resolves
/// the full `PullRequest` / `WorkItem` / `PipelineRun` from its own lists by `(kind, id)`.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteItem {
    pub kind: PaletteKind,
    /// Provider-native id of the underlying item.
    pub id: String,
    /// The connection the item came from — actions resolve their source through it.
    pub connection_id: String,
    /// Primary match text (the item's title).
    pub title: String,
    /// Secondary match text: author / repo / identifier / branch / connection, so results
    /// are disambiguable and searchable by more than the title.
    pub subtitle: String,
    /// Recency key (updated/finished time) — the tiebreak when scores are equal and the
    /// sole order for an empty query.
    pub sort_ts: Option<DateTime<Utc>>,
}

/// A user's most human-recognisable handle, for a subtitle.
fn who(user: &User) -> &str {
    user.handle.as_deref().unwrap_or(&user.display_name)
}

/// Join the non-empty parts of a subtitle with " · ".
fn subtitle(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn pr_candidate(row: &PrRow) -> PaletteItem {
    let branch = row.pr.source_ref.as_deref().unwrap_or("");
    PaletteItem {
        kind: PaletteKind::Pr,
        id: row.pr.id.clone(),
        connection_id: row.connection_id.clone(),
        title: row.pr.title.clone(),
        subtitle: subtitle(&[who(&row.pr.author), branch, &row.connection]),
        sort_ts: row.pr.updated_at,
    }
}

pub fn wi_candidate(row: &WiRow) -> PaletteItem {
    let ident = row.wi.identifier.as_deref().unwrap_or("");
    let ty = row.wi.work_item_type.as_deref().unwrap_or("");
    PaletteItem {
        kind: PaletteKind::Wi,
        id: row.wi.id.clone(),
        connection_id: row.connection_id.clone(),
        title: row.wi.title.clone(),
        subtitle: subtitle(&[ident, ty, &row.connection]),
        sort_ts: row.wi.updated_at,
    }
}

pub fn pipe_candidate(row: &PipeRow) -> PaletteItem {
    // Title is the pipeline (definition) name — matching how pipelines read elsewhere —
    // falling back to the run's own name.
    let title = row
        .definition_name
        .clone()
        .or_else(|| row.run.name.clone())
        .unwrap_or_else(|| "pipeline".to_string());
    let run_name = row.run.name.as_deref().unwrap_or("");
    let branch = row.run.branch.as_deref().unwrap_or("");
    PaletteItem {
        kind: PaletteKind::Pipe,
        id: row.run.id.clone(),
        connection_id: row.connection_id.clone(),
        title,
        subtitle: subtitle(&[run_name, branch, &row.connection]),
        sort_ts: row.run.finished_at.or(row.run.started_at),
    }
}

/// Build the full candidate set from the app's three row lists, in a stable order
/// (PRs, then work items, then pipelines). Ordering only matters for the empty-query and
/// equal-score cases, both of which are then resolved by recency in [`rank`].
pub fn build_candidates(prs: &[PrRow], wis: &[WiRow], pipes: &[PipeRow]) -> Vec<PaletteItem> {
    prs.iter()
        .map(pr_candidate)
        .chain(wis.iter().map(wi_candidate))
        .chain(pipes.iter().map(pipe_candidate))
        .collect()
}

/// Newest-first comparison on `sort_ts`; items without a timestamp sort last.
fn by_recency_desc(a: &PaletteItem, b: &PaletteItem) -> std::cmp::Ordering {
    b.sort_ts.cmp(&a.sort_ts)
}

/// Fuzzy-filter and order `candidates` for `query`, returning indices into `candidates`.
///
/// - Empty/whitespace query → every candidate, most-recent first.
/// - Otherwise → only candidates whose title *or* subtitle fuzzy-matches, best score
///   first, ties broken by recency. Case-insensitive (SkimMatcherV2 is smart-case).
pub fn rank(query: &str, candidates: &[PaletteItem]) -> Vec<usize> {
    if query.trim().is_empty() {
        let mut idx: Vec<usize> = (0..candidates.len()).collect();
        idx.sort_by(|&a, &b| by_recency_desc(&candidates[a], &candidates[b]));
        return idx;
    }

    // ignore_case (not the default smart-case) so an uppercase query still matches — a
    // palette should filter predictably regardless of how the query is typed.
    let matcher = SkimMatcherV2::default().ignore_case();
    let mut scored: Vec<(usize, i64)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let title = matcher.fuzzy_match(&c.title, query);
            let sub = matcher.fuzzy_match(&c.subtitle, query);
            title.max(sub).map(|score| (i, score))
        })
        .collect();

    scored.sort_by(|&(ai, asc), &(bi, bsc)| {
        bsc.cmp(&asc)
            .then_with(|| by_recency_desc(&candidates[ai], &candidates[bi]))
    });
    scored.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> Option<DateTime<Utc>> {
        Some(Utc.timestamp_opt(secs, 0).unwrap())
    }

    fn item(kind: PaletteKind, title: &str, subtitle: &str, ts_secs: i64) -> PaletteItem {
        PaletteItem {
            kind,
            id: format!("{title}-id"),
            connection_id: "conn".into(),
            title: title.into(),
            subtitle: subtitle.into(),
            sort_ts: ts(ts_secs),
        }
    }

    fn titles(order: &[usize], candidates: &[PaletteItem]) -> Vec<String> {
        order.iter().map(|&i| candidates[i].title.clone()).collect()
    }

    #[test]
    fn empty_query_returns_all_most_recent_first() {
        let c = vec![
            item(PaletteKind::Pr, "old", "", 100),
            item(PaletteKind::Wi, "new", "", 300),
            item(PaletteKind::Pipe, "mid", "", 200),
        ];
        assert_eq!(titles(&rank("", &c), &c), vec!["new", "mid", "old"]);
        // Whitespace-only is treated as empty.
        assert_eq!(rank("   ", &c).len(), 3);
    }

    #[test]
    fn items_without_a_timestamp_sort_last() {
        let mut c = vec![
            item(PaletteKind::Pr, "dated", "", 100),
            item(PaletteKind::Pr, "undated", "", 0),
        ];
        c[1].sort_ts = None;
        assert_eq!(titles(&rank("", &c), &c), vec!["dated", "undated"]);
    }

    #[test]
    fn filters_out_non_matches() {
        let c = vec![
            item(PaletteKind::Pr, "Migrate billing", "", 100),
            item(PaletteKind::Pr, "Fix login redirect", "", 100),
        ];
        assert_eq!(titles(&rank("migrate", &c), &c), vec!["Migrate billing"]);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let c = vec![item(PaletteKind::Pr, "Migrate Billing", "", 100)];
        assert_eq!(rank("MIGRATE", &c).len(), 1);
        assert_eq!(rank("migrate", &c).len(), 1);
    }

    #[test]
    fn matches_on_subtitle_when_title_does_not() {
        let c = vec![
            item(PaletteKind::Wi, "Untitled task", "PAY-412 · Bug", 100),
            item(PaletteKind::Wi, "Other", "ENG-9 · Story", 100),
        ];
        assert_eq!(titles(&rank("pay-412", &c), &c), vec!["Untitled task"]);
    }

    #[test]
    fn stronger_match_ranks_higher() {
        // A contiguous substring match should beat a scattered subsequence match.
        let c = vec![
            item(PaletteKind::Pr, "b-a-r-b-a-z", "", 100), // scattered "bar"
            item(PaletteKind::Pr, "bar service", "", 100), // contiguous "bar"
        ];
        assert_eq!(titles(&rank("bar", &c), &c)[0], "bar service");
    }

    #[test]
    fn equal_scores_break_ties_by_recency() {
        // Identical text → identical scores → newer one first.
        let c = vec![
            item(PaletteKind::Pr, "deploy pipeline", "acme", 100),
            item(PaletteKind::Pr, "deploy pipeline", "acme", 500),
        ];
        let order = rank("deploy", &c);
        assert_eq!(c[order[0]].sort_ts, ts(500));
        assert_eq!(c[order[1]].sort_ts, ts(100));
    }
}
