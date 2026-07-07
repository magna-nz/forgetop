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
