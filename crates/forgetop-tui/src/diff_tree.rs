//! The Diff tab's file list as a tree: folders and files in display order, built from the
//! changed paths alone so the renderer, the cursor and the mouse all agree on what each row is.

use std::collections::HashSet;

/// One row of the diff file list as drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    /// A folder. `key` is its full directory path (`""` is the flat layout's `(root)`) and is
    /// what folds are recorded by; `label` is what the row prints; `files` are every file the
    /// row stands for (its whole subtree in the tree, its own files in the flat layout), in
    /// display order — what `v` marks and the summary lists.
    Dir { key: String, label: String, depth: usize, open: bool, files: Vec<usize> },
    /// A file, by its index into the diff's file list.
    File { idx: usize, depth: usize },
}

impl TreeRow {
    pub fn depth(&self) -> usize {
        match self {
            TreeRow::Dir { depth, .. } | TreeRow::File { depth, .. } => *depth,
        }
    }
}

/// A directory in the trie the rows are cut from.
#[derive(Default)]
struct Node {
    name: String,
    dirs: Vec<Node>,
    /// (base name, file index).
    files: Vec<(String, usize)>,
}

impl Node {
    fn child(&mut self, name: &str) -> &mut Node {
        let at = match self.dirs.iter().position(|d| d.name == name) {
            Some(at) => at,
            None => {
                self.dirs.push(Node { name: name.to_string(), ..Node::default() });
                self.dirs.len() - 1
            }
        };
        &mut self.dirs[at]
    }

    /// Folders first, then files, each alphabetical, all the way down.
    fn sort(&mut self) {
        self.dirs.sort_by(|a, b| a.name.cmp(&b.name));
        self.files.sort();
        for d in &mut self.dirs {
            d.sort();
        }
    }

    /// Every file under this node, in display order.
    fn all_files(&self, out: &mut Vec<usize>) {
        for d in &self.dirs {
            d.all_files(out);
        }
        out.extend(self.files.iter().map(|(_, i)| *i));
    }
}

fn build(paths: &[&str]) -> Node {
    let mut root = Node::default();
    for (idx, path) in paths.iter().enumerate() {
        let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let base = parts.pop().unwrap_or(path).to_string();
        let mut node = &mut root;
        for part in parts {
            node = node.child(part);
        }
        node.files.push((base, idx));
    }
    root.sort();
    root
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") }
}

/// The file list as a tree: at each level folders first, then files, each alphabetical; a
/// folder whose only child is one folder (and which holds no files itself) shares its row
/// (`forgetop-core/src/`), and root-level files come after the root's folders. Empty path
/// segments name no folder, so Azure's `/src/a.rs` sits under `src/`. Only files
/// `include` accepts are listed, and a folder only when something under it is. A folder in
/// `folded` is listed shut, its contents left out.
pub fn tree_rows(paths: &[&str], folded: &HashSet<String>, include: &dyn Fn(usize) -> bool) -> Vec<TreeRow> {
    fn emit(node: &Node, prefix: &str, depth: usize, folded: &HashSet<String>, include: &dyn Fn(usize) -> bool, out: &mut Vec<TreeRow>) {
        for dir in &node.dirs {
            let (mut cur, mut key, mut label) = (dir, join(prefix, &dir.name), dir.name.clone());
            while cur.files.is_empty() && cur.dirs.len() == 1 {
                cur = &cur.dirs[0];
                key = join(&key, &cur.name);
                label = format!("{label}/{}", cur.name);
            }
            let mut files = Vec::new();
            cur.all_files(&mut files);
            if !files.iter().any(|&i| include(i)) {
                continue;
            }
            let open = !folded.contains(&key);
            out.push(TreeRow::Dir { key: key.clone(), label: format!("{label}/"), depth, open, files });
            if open {
                emit(cur, &key, depth + 1, folded, include, out);
            }
        }
        for &(_, idx) in &node.files {
            if include(idx) {
                out.push(TreeRow::File { idx, depth });
            }
        }
    }
    let mut out = Vec::new();
    emit(&build(paths), "", 0, folded, include, &mut out);
    out
}

/// The file list flattened to one level, for a list too narrow for the tree: one folder row
/// per directory that holds files (in the tree's order, `(root)` last), labelled with its full
/// path, its files beneath it. Inclusion and folding work as in [`tree_rows`].
pub fn flat_rows(paths: &[&str], folded: &HashSet<String>, include: &dyn Fn(usize) -> bool) -> Vec<TreeRow> {
    fn groups(node: &Node, key: &str, out: &mut Vec<(String, Vec<usize>)>) {
        for d in &node.dirs {
            let k = join(key, &d.name);
            if !d.files.is_empty() {
                out.push((k.clone(), d.files.iter().map(|(_, i)| *i).collect()));
            }
            groups(d, &k, out);
        }
    }
    let root = build(paths);
    let mut all = Vec::new();
    groups(&root, "", &mut all);
    if !root.files.is_empty() {
        all.push((String::new(), root.files.iter().map(|(_, i)| *i).collect()));
    }
    let mut out = Vec::new();
    for (key, files) in all {
        if !files.iter().any(|&i| include(i)) {
            continue;
        }
        let open = !folded.contains(&key);
        let label = if key.is_empty() { "(root)".to_string() } else { format!("{key}/") };
        out.push(TreeRow::Dir { key, label, depth: 0, open, files: files.clone() });
        if open {
            out.extend(files.into_iter().filter(|&i| include(i)).map(|idx| TreeRow::File { idx, depth: 1 }));
        }
    }
    out
}

/// `label` cut from the left with `…` to fit `width` columns (`…providers/src/http/`).
pub fn left_truncate(label: &str, width: usize) -> String {
    let n = label.chars().count();
    if n <= width {
        return label.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let keep: String = label.chars().skip(n - (width - 1)).collect();
    format!("…{keep}")
}

/// How many times `query` occurs in the file's path and patch, ASCII-case-insensitively —
/// the same matching the log pane's search uses.
pub fn file_hits(path: &str, patch: Option<&str>, query: &str) -> usize {
    use crate::app::match_ranges;
    match_ranges(path, query).len() + patch.map_or(0, |p| p.lines().map(|l| match_ranges(l, query).len()).sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(rows: &[TreeRow], paths: &[&str]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                TreeRow::Dir { label, depth, open, .. } => format!("{}{}{label}", "  ".repeat(*depth), if *open { "▾ " } else { "▸ " }),
                TreeRow::File { idx, depth } => format!("{}{}", "  ".repeat(*depth), paths[*idx].rsplit('/').next().unwrap()),
            })
            .collect()
    }

    const PATHS: [&str; 6] = [
        "README.md",
        "crates/forgetop-core/src/provider.rs",
        "crates/forgetop-tui/src/app.rs",
        "crates/forgetop-tui/src/ui.rs",
        "crates/forgetop-tui/Cargo.toml",
        "Cargo.lock",
    ];

    #[test]
    fn folders_come_first_chains_share_a_row_and_root_files_come_last() {
        let rows = tree_rows(&PATHS, &HashSet::new(), &|_| true);
        assert_eq!(
            labels(&rows, &PATHS),
            vec![
                "▾ crates/",
                "  ▾ forgetop-core/src/",
                "    provider.rs",
                "  ▾ forgetop-tui/",
                "    ▾ src/",
                "      app.rs",
                "      ui.rs",
                "    Cargo.toml",
                "Cargo.lock",
                "README.md",
            ]
        );
        let TreeRow::Dir { key, files, .. } = &rows[1] else { panic!() };
        assert_eq!(key, "crates/forgetop-core/src", "a shared row is keyed by its deepest folder");
        assert_eq!(files, &vec![1]);
        let TreeRow::Dir { files, .. } = &rows[0] else { panic!() };
        assert_eq!(files, &vec![1, 2, 3, 4], "a folder stands for its whole subtree, in display order");
    }

    #[test]
    fn a_folded_folder_hides_its_subtree_and_a_search_drops_empty_folders() {
        let folded: HashSet<String> = ["crates/forgetop-tui".to_string()].into();
        let rows = tree_rows(&PATHS, &folded, &|_| true);
        assert_eq!(labels(&rows, &PATHS)[3..], ["  ▸ forgetop-tui/", "Cargo.lock", "README.md"]);

        let rows = tree_rows(&PATHS, &HashSet::new(), &|i| i == 3);
        // Rows keep their shape under a search (a chain is never re-cut around what matched).
        assert_eq!(labels(&rows, &PATHS), vec!["▾ crates/", "  ▾ forgetop-tui/", "    ▾ src/", "      ui.rs"]);
    }

    #[test]
    fn the_flat_layout_has_one_folder_per_directory_and_root_last() {
        let rows = flat_rows(&PATHS, &HashSet::new(), &|_| true);
        assert_eq!(
            labels(&rows, &PATHS),
            vec![
                "▾ crates/forgetop-core/src/",
                "  provider.rs",
                "▾ crates/forgetop-tui/",
                "  Cargo.toml",
                "▾ crates/forgetop-tui/src/",
                "  app.rs",
                "  ui.rs",
                "▾ (root)",
                "  Cargo.lock",
                "  README.md",
            ]
        );
        let TreeRow::Dir { files, .. } = &rows[2] else { panic!() };
        assert_eq!(files, &vec![4], "a flat folder holds only its own files");
    }

    #[test]
    fn a_leading_slash_is_no_folder() {
        // Azure DevOps spells its paths from the repository root: `/src/a.rs`.
        let paths = ["/README.md", "/src/a.rs", "/src/b.rs"];
        let rows = tree_rows(&paths, &HashSet::new(), &|_| true);
        assert_eq!(labels(&rows, &paths), vec!["▾ src/", "  a.rs", "  b.rs", "README.md"]);
        let TreeRow::Dir { key, .. } = &rows[0] else { panic!() };
        assert_eq!(key, "src");
        let rows = flat_rows(&paths, &HashSet::new(), &|_| true);
        assert_eq!(labels(&rows, &paths), vec!["▾ src/", "  a.rs", "  b.rs", "▾ (root)", "  README.md"]);
    }

    #[test]
    fn labels_are_cut_from_the_left() {
        assert_eq!(left_truncate("crates/forgetop-providers/src/http/", 20), "…providers/src/http/");
        assert_eq!(left_truncate("src/", 20), "src/");
    }

    #[test]
    fn hits_count_the_path_and_every_patch_occurrence() {
        assert_eq!(file_hits("src/retry.rs", Some("@@ -1 +1 @@\n-Retry\n+retry retry"), "retry"), 4);
        assert_eq!(file_hits("src/a.rs", None, "zzz"), 0);
    }
}
