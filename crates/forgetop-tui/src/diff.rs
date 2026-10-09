//! Unified-diff position helpers: mapping a patch line index to a real file line
//! and side, for the line cursor label, comment targets, and pending markers.

use std::collections::HashSet;

use forgetop_core::domain::{DiffSide, LineComment};

/// Parses a `@@ -old,n +new,n @@` hunk header into (old_start, new_start).
pub fn parse_hunk_header(line: &str) -> Option<(i64, i64)> {
    if !line.starts_with("@@") {
        return None;
    }
    let (mut old_start, mut new_start) = (None, None);
    for tok in line.split_whitespace() {
        if let Some(r) = tok.strip_prefix('-') {
            old_start = r.split(',').next().and_then(|s| s.parse().ok());
        } else if let Some(r) = tok.strip_prefix('+') {
            new_start = r.split(',').next().and_then(|s| s.parse().ok());
        }
    }
    Some((old_start?, new_start?))
}

/// Parses a hunk header into (old_start, old_count, new_start, new_count). An omitted count
/// (`@@ -3 +3 @@`) is 1, as unified diff spells a one-line range.
pub fn parse_hunk_range(line: &str) -> Option<(i64, i64, i64, i64)> {
    if !line.starts_with("@@") {
        return None;
    }
    let range = |r: &str| -> Option<(i64, i64)> {
        let mut it = r.split(',');
        let start = it.next()?.parse().ok()?;
        let count = match it.next() {
            Some(c) => c.parse().ok()?,
            None => 1,
        };
        Some((start, count))
    };
    let (mut old, mut new) = (None, None);
    for tok in line.split_whitespace().skip(1) {
        if tok.starts_with("@@") {
            break;
        }
        if let Some(r) = tok.strip_prefix('-') {
            old = range(r);
        } else if let Some(r) = tok.strip_prefix('+') {
            new = range(r);
        }
    }
    let ((os, oc), (ns, nc)) = (old?, new?);
    Some((os, oc, ns, nc))
}

/// A run of unchanged lines a patch leaves out above one of its hunks: new-side lines
/// `start..=end`, sitting above the `@@` header at patch line `hunk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    pub hunk: usize,
    pub start: i64,
    pub end: i64,
}

impl Gap {
    /// How many lines the gap holds.
    pub fn len(&self) -> usize {
        (self.end - self.start + 1).max(0) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The gaps between a patch's hunks, and before its first hunk when that doesn't start at
/// line 1, read from the hunk headers alone. Context lines are the same on both sides, so the
/// new-side numbers address the text a gap holds. (Lines after the last hunk aren't a gap: the
/// patch doesn't say where the file ends.)
pub fn hunk_gaps(patch: &str) -> Vec<Gap> {
    let mut gaps = Vec::new();
    // The first new-side line after the previous hunk (1 before any).
    let mut next = 1i64;
    for (i, line) in patch.lines().enumerate() {
        let Some((_, _, start, count)) = parse_hunk_range(line) else { continue };
        // A hunk with no new-side lines (`+5,0`) sits *after* new line 5, which is unchanged.
        let end = if count == 0 { start } else { start - 1 };
        if end >= next {
            gaps.push(Gap { hunk: i, start: next, end });
        }
        next = if count == 0 { start + 1 } else { start + count };
    }
    gaps
}

/// Whether `text` (the file on the diff's new side, one entry per line) is the file this patch
/// was cut from: each hunk's first new-side line reads the same in both, and every gap lies
/// inside the text. Anything else — a head that moved on, a different file — is unusable.
pub fn text_agrees(patch: &str, text: &[String]) -> bool {
    let lines: Vec<&str> = patch.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let Some((_, _, start, count)) = parse_hunk_range(line) else { continue };
        if count == 0 {
            continue;
        }
        // The hunk's new-side range has to lie inside the text too.
        if start < 1 || (start + count - 1) as usize > text.len() {
            return false;
        }
        let first = lines[i + 1..]
            .iter()
            .take_while(|l| !l.starts_with("@@"))
            .find(|l| l.starts_with(' ') || (l.starts_with('+') && !l.starts_with("+++")));
        if let Some(first) = first {
            if start < 1 || text.get(start as usize - 1).map(String::as_str) != Some(&first[1..]) {
                return false;
            }
        }
    }
    hunk_gaps(patch).iter().all(|g| g.end as usize <= text.len())
}

/// The lines revealed from each edge of a gap of `len` lines after one more step of `step`:
/// `(from the top, from the bottom)`. What's left once a step would cover it opens whole.
pub fn reveal_step(len: usize, (top, bottom): (usize, usize), step: usize) -> (usize, usize) {
    if len.saturating_sub(top + bottom) <= 2 * step {
        (len, 0)
    } else {
        (top + step, bottom + step)
    }
}

/// The file line + side that patch line `cursor` maps to, or `None` if it isn't a
/// commentable code line (a hunk/file header, or before the first hunk).
pub fn comment_target(patch: &str, cursor: usize) -> Option<(i64, DiffSide)> {
    let (mut old_ln, mut new_ln) = (0i64, 0i64);
    for (i, line) in patch.lines().enumerate() {
        if let Some((o, n)) = parse_hunk_header(line) {
            old_ln = o;
            new_ln = n;
            if i == cursor {
                return None; // the hunk header itself isn't commentable
            }
            continue;
        }
        if new_ln == 0 || line.starts_with("+++") || line.starts_with("---") {
            if i == cursor {
                return None;
            }
            continue;
        }
        let (side, ln) = match line.chars().next() {
            Some('+') => {
                let l = new_ln;
                new_ln += 1;
                (DiffSide::New, l)
            }
            Some('-') => {
                let l = old_ln;
                old_ln += 1;
                (DiffSide::Old, l)
            }
            _ => {
                let l = new_ln;
                old_ln += 1;
                new_ln += 1;
                (DiffSide::New, l)
            }
        };
        if i == cursor {
            return Some((ln, side));
        }
    }
    None
}

/// The file line number to print beside each patch line: the new-side number for added and
/// context lines, the old-side number for removed ones, `None` for headers. One pass, so a
/// renderer can number every row without calling [`comment_target`] per line.
pub fn gutter_numbers(patch: &str) -> Vec<Option<(i64, DiffSide)>> {
    let (mut old_ln, mut new_ln) = (0i64, 0i64);
    patch
        .lines()
        .map(|line| {
            if let Some((o, n)) = parse_hunk_header(line) {
                old_ln = o;
                new_ln = n;
                return None;
            }
            if new_ln == 0 || line.starts_with("+++") || line.starts_with("---") {
                return None;
            }
            Some(match line.chars().next() {
                Some('+') => {
                    new_ln += 1;
                    (new_ln - 1, DiffSide::New)
                }
                Some('-') => {
                    old_ln += 1;
                    (old_ln - 1, DiffSide::Old)
                }
                _ => {
                    old_ln += 1;
                    new_ln += 1;
                    (new_ln - 1, DiffSide::New)
                }
            })
        })
        .collect()
}

/// The patch display-line index whose **new-side** line number equals `target`, or `None`
/// if it isn't in this patch. Used to jump the cursor to a thread anchored at a file line.
pub fn patch_line_for_source_line(patch: &str, target: i64) -> Option<usize> {
    let mut new_ln = 0i64;
    for (i, line) in patch.lines().enumerate() {
        if let Some((_, n)) = parse_hunk_header(line) {
            new_ln = n;
            continue;
        }
        if new_ln == 0 || line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        match line.chars().next() {
            Some('-') => {} // removed line: doesn't exist on the new side
            _ => {
                if new_ln == target {
                    return Some(i);
                }
                new_ln += 1;
            }
        }
    }
    None
}

/// Human label for the file position of patch line `cursor` (e.g. `line 42`).
pub fn cursor_line_label(patch: &str, cursor: usize) -> Option<String> {
    // Hunk headers get a distinct label; commentable lines report their number.
    if patch.lines().nth(cursor).is_some_and(|l| l.starts_with("@@")) {
        return parse_hunk_header(patch.lines().nth(cursor)?).map(|(_, n)| format!("hunk @ {n}"));
    }
    match comment_target(patch, cursor)? {
        (l, DiffSide::New) => Some(format!("line {l}")),
        (l, DiffSide::Old) => Some(format!("line {l} (old)")),
    }
}

/// Patch line indices (for file `path`) that have a pending comment.
pub fn pending_marks(patch: &str, path: &str, pending: &[LineComment]) -> HashSet<usize> {
    let targets: HashSet<(i64, DiffSide)> =
        pending.iter().filter(|c| c.path == path).map(|c| (c.line, c.side)).collect();
    let mut marks = HashSet::new();
    if targets.is_empty() {
        return marks;
    }
    for i in 0..patch.lines().count() {
        if let Some(t) = comment_target(patch, i) {
            if targets.contains(&t) {
                marks.insert(i);
            }
        }
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_line_label_maps_hunk_lines_to_file_lines() {
        let patch = "@@ -10,3 +20,4 @@\n ctx\n+new\n-old";
        assert_eq!(cursor_line_label(patch, 0).as_deref(), Some("hunk @ 20"));
        assert_eq!(cursor_line_label(patch, 1).as_deref(), Some("line 20")); // context → new line 20
        assert_eq!(cursor_line_label(patch, 2).as_deref(), Some("line 21")); // added → new line 21
        assert_eq!(cursor_line_label(patch, 3).as_deref(), Some("line 11 (old)")); // removed → old line 11
    }

    #[test]
    fn gutter_numbers_agree_with_the_cursor_label() {
        let patch = "diff --git a/x b/x\n@@ -10,3 +20,4 @@\n ctx\n+new\n-old";
        assert_eq!(
            gutter_numbers(patch),
            vec![None, None, Some((20, DiffSide::New)), Some((21, DiffSide::New)), Some((11, DiffSide::Old))]
        );
        for i in 0..patch.lines().count() {
            assert_eq!(gutter_numbers(patch)[i], comment_target(patch, i), "row {i}");
        }
    }

    #[test]
    fn source_line_maps_back_to_the_patch_index() {
        let patch = "@@ -10,3 +20,4 @@\n ctx\n+new\n-old";
        assert_eq!(patch_line_for_source_line(patch, 20), Some(1)); // context row
        assert_eq!(patch_line_for_source_line(patch, 21), Some(2)); // added row
        assert_eq!(patch_line_for_source_line(patch, 22), None); // beyond the hunk
        assert_eq!(patch_line_for_source_line(patch, 11), None); // an old-side line isn't on the new side
    }

    #[test]
    fn parse_hunk_header_reads_starts() {
        assert_eq!(parse_hunk_header("@@ -10,3 +20,4 @@ fn foo()"), Some((10, 20)));
        assert_eq!(parse_hunk_header("@@ -1 +1 @@"), Some((1, 1)));
        assert_eq!(parse_hunk_header(" not a hunk"), None);
    }

    #[test]
    fn comment_target_picks_side_and_line() {
        let patch = "@@ -10,3 +20,4 @@\n ctx\n+new\n-old";
        assert_eq!(comment_target(patch, 0), None); // hunk header
        assert_eq!(comment_target(patch, 1), Some((20, DiffSide::New)));
        assert_eq!(comment_target(patch, 2), Some((21, DiffSide::New)));
        assert_eq!(comment_target(patch, 3), Some((11, DiffSide::Old)));
    }

    #[test]
    fn parse_hunk_range_reads_counts_and_defaults_them_to_one() {
        assert_eq!(parse_hunk_range("@@ -10,3 +20,4 @@ fn foo()"), Some((10, 3, 20, 4)));
        assert_eq!(parse_hunk_range("@@ -1 +1 @@"), Some((1, 1, 1, 1)));
        assert_eq!(parse_hunk_range("@@ -0,0 +1,2 @@"), Some((0, 0, 1, 2)));
        assert_eq!(parse_hunk_range(" ctx"), None);
    }

    #[test]
    fn gaps_come_from_the_hunk_headers() {
        // Before the first hunk (lines 1..=9), and between the two (lines 13..=29).
        let patch = "diff --git a/x b/x\n@@ -10,3 +10,3 @@\n a\n-b\n+c\n d\n@@ -30,2 +30,3 @@\n e\n+f\n g";
        assert_eq!(hunk_gaps(patch), vec![Gap { hunk: 1, start: 1, end: 9 }, Gap { hunk: 6, start: 13, end: 29 }]);
        assert_eq!(hunk_gaps(patch)[1].len(), 17);
        // A hunk that starts at line 1 has nothing above it; adjacent hunks have nothing between.
        assert_eq!(hunk_gaps("@@ -1,2 +1,2 @@\n a\n b\n@@ -3,1 +3,1 @@\n c"), vec![]);
        // A deletion-only hunk (`+5,0`) sits after new line 5, so line 5 is still in the gap.
        assert_eq!(hunk_gaps("@@ -1,1 +1,1 @@\n a\n@@ -6,1 +5,0 @@\n-x"), vec![Gap { hunk: 2, start: 2, end: 5 }]);
        // An added file has no gap.
        assert_eq!(hunk_gaps("@@ -0,0 +1,2 @@\n+a\n+b"), vec![]);
    }

    #[test]
    fn text_agrees_checks_each_hunk_start_and_the_gap_bounds() {
        let text: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
        let patch = "@@ -5,2 +5,2 @@\n line 5\n-old\n+line 6\n@@ -12,1 +12,1 @@\n line 12";
        assert!(text_agrees(patch, &text));
        let moved = "@@ -5,2 +5,2 @@\n line 4\n+line 6";
        assert!(!text_agrees(moved, &text), "the hunk's first line reads differently");
        assert!(!text_agrees("@@ -30,1 +30,1 @@\n line 30", &text), "the gap runs past the text");
        // A hunk whose range runs past the end of a (shorter) text, though its first line agrees.
        assert!(!text_agrees("@@ -18,5 +18,5 @@\n line 18\n line 19\n line 20\n+line 21\n+line 22", &text));
    }

    #[test]
    fn reveal_steps_ten_from_each_edge_and_opens_a_short_rest_whole() {
        assert_eq!(reveal_step(50, (0, 0), 10), (10, 10));
        assert_eq!(reveal_step(50, (10, 10), 10), (20, 20));
        assert_eq!(reveal_step(50, (20, 20), 10), (50, 0), "ten left: open it all");
        assert_eq!(reveal_step(20, (0, 0), 10), (20, 0), "a gap of 20 opens fully");
    }

    #[test]
    fn pending_marks_flags_commented_lines() {
        let patch = "@@ -10,3 +20,4 @@\n ctx\n+new\n-old";
        let pending = vec![
            LineComment { path: "a.rs".into(), line: 21, side: DiffSide::New, body: "x".into() },
            LineComment { path: "other.rs".into(), line: 20, side: DiffSide::New, body: "y".into() },
        ];
        let marks = pending_marks(patch, "a.rs", &pending);
        assert!(marks.contains(&2)); // the +new line
        assert!(!marks.contains(&1));
        assert_eq!(marks.len(), 1);
    }
}
