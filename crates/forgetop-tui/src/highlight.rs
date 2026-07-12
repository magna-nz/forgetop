//! Syntax highlighting for diff patches.
//!
//! We map a source line to a sequence of **semantic token kinds** ([`HlKind`]) and let the
//! UI colour those from the theme's *indexed* palette — never truecolor RGB, which washes
//! out on some terminals (see `theme.rs`). `synoptic` does the tokenising; it lives behind
//! this module's small interface so the backing crate stays swappable.
//!
//! Highlighting is **line-based**: each patch line is tokenised on its own, so multi-line
//! strings/comments don't carry context across lines. That's an accepted tradeoff for diff
//! fragments (where we rarely have the whole file anyway).

use synoptic::TokOpt;

/// A source language we can highlight. Detected from a file's extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    Json,
    Yaml,
}

impl Lang {
    /// The extension key `synoptic::from_extension` understands for this language.
    fn synoptic_ext(self) -> &'static str {
        match self {
            Lang::Rust => "rs",
            Lang::TypeScript => "ts",
            Lang::JavaScript => "js",
            Lang::Python => "py",
            Lang::Go => "go",
            Lang::Java => "java",
            Lang::Json => "json",
            Lang::Yaml => "yml",
        }
    }
}

/// Detect the language from a file path's extension, or `None` if we don't highlight it
/// (the caller then renders the line plain, exactly as before).
pub fn lang_for(path: &str) -> Option<Lang> {
    let ext = path.rsplit('.').next().filter(|e| *e != path)?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => Lang::Rust,
        "ts" | "tsx" => Lang::TypeScript,
        "js" | "jsx" | "mjs" | "cjs" => Lang::JavaScript,
        "py" | "pyw" => Lang::Python,
        "go" => Lang::Go,
        "java" => Lang::Java,
        "json" => Lang::Json,
        "yaml" | "yml" => Lang::Yaml,
        _ => return None,
    })
}

/// Semantic token kind — the UI maps each to an indexed theme colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlKind {
    Keyword,
    Type,
    Str,
    Comment,
    Number,
    Func,
    Punct,
    Plain,
}

/// Map a `synoptic` token category name to an [`HlKind`]. Unknown names fall back to plain.
pub fn kind_for_name(name: &str) -> HlKind {
    match name {
        "keyword" | "boolean" => HlKind::Keyword,
        "type" | "struct" | "namespace" | "attribute" => HlKind::Type,
        "string" | "character" => HlKind::Str,
        "comment" => HlKind::Comment,
        "digit" => HlKind::Number,
        "function" | "macro" => HlKind::Func,
        "operator" | "reference" => HlKind::Punct,
        _ => HlKind::Plain,
    }
}

/// A reusable single-language highlighter. Build one per file and call [`line`] per source
/// line — the underlying regexes compile once, then each line re-runs cheaply.
///
/// [`line`]: LineHighlighter::line
pub struct LineHighlighter {
    hl: synoptic::Highlighter,
}

impl LineHighlighter {
    /// Build a highlighter for `lang`, or `None` if the backing tokenizer has no rules for it.
    pub fn new(lang: Lang) -> Option<LineHighlighter> {
        synoptic::from_extension(lang.synoptic_ext(), 4).map(|hl| LineHighlighter { hl })
    }

    /// Tokenise one source line into `(text, kind)` spans covering the whole line in order.
    pub fn line(&mut self, text: &str) -> Vec<(String, HlKind)> {
        // Line-based: treat this line as a one-line document.
        self.hl.run(&[text.to_string()]);
        self.hl
            .line(0, text)
            .into_iter()
            .map(|tok| match tok {
                TokOpt::Some(s, name) => (s, kind_for_name(&name)),
                TokOpt::None(s) => (s, HlKind::Plain),
            })
            .collect()
    }
}

/// Convenience: tokenise a single line for `lang` in one call (builds a throwaway
/// highlighter — fine for tests; the renderer reuses a [`LineHighlighter`] across a file).
pub fn highlight_line(lang: Lang, text: &str) -> Vec<(String, HlKind)> {
    match LineHighlighter::new(lang) {
        Some(mut h) => h.line(text),
        None => vec![(text.to_string(), HlKind::Plain)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_language_from_extension() {
        assert_eq!(lang_for("src/main.rs"), Some(Lang::Rust));
        assert_eq!(lang_for("app/Component.tsx"), Some(Lang::TypeScript));
        assert_eq!(lang_for("index.mjs"), Some(Lang::JavaScript));
        assert_eq!(lang_for("deploy/values.yaml"), Some(Lang::Yaml));
        assert_eq!(lang_for("k8s.yml"), Some(Lang::Yaml));
        assert_eq!(lang_for("Main.JAVA"), Some(Lang::Java)); // case-insensitive
    }

    #[test]
    fn unknown_or_extensionless_paths_are_none() {
        assert_eq!(lang_for("README"), None);
        assert_eq!(lang_for("notes.txt"), None);
        assert_eq!(lang_for("Makefile"), None);
        assert_eq!(lang_for(".gitignore"), None); // leading-dot only → no real extension
    }

    #[test]
    fn token_names_map_to_kinds() {
        assert_eq!(kind_for_name("keyword"), HlKind::Keyword);
        assert_eq!(kind_for_name("string"), HlKind::Str);
        assert_eq!(kind_for_name("comment"), HlKind::Comment);
        assert_eq!(kind_for_name("digit"), HlKind::Number);
        assert_eq!(kind_for_name("function"), HlKind::Func);
        assert_eq!(kind_for_name("something-new"), HlKind::Plain);
    }

    /// True if some token's text contains `needle` and carries `kind`.
    fn has(tokens: &[(String, HlKind)], needle: &str, kind: HlKind) -> bool {
        tokens.iter().any(|(t, k)| *k == kind && t.contains(needle))
    }

    #[test]
    fn highlights_rust_keyword_number_string_and_comment() {
        let toks = highlight_line(Lang::Rust, r#"let n = 5; // note"#);
        // The full line is covered by the returned spans (nothing dropped).
        let joined: String = toks.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, r#"let n = 5; // note"#);
        assert!(has(&toks, "let", HlKind::Keyword), "`let` is a keyword: {toks:?}");
        assert!(has(&toks, "5", HlKind::Number), "`5` is a number: {toks:?}");
        assert!(has(&toks, "note", HlKind::Comment), "trailing `// note` is a comment: {toks:?}");
    }

    #[test]
    fn highlights_python_string() {
        let toks = highlight_line(Lang::Python, r#"name = "sam""#);
        assert!(has(&toks, "sam", HlKind::Str), "double-quoted string: {toks:?}");
    }

    #[test]
    fn unknown_language_still_covers_the_line_as_plain() {
        // (Exercised via the renderer for real; here we just confirm the join is lossless.)
        let toks = highlight_line(Lang::Json, r#"{"k": 1}"#);
        let joined: String = toks.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, r#"{"k": 1}"#);
    }
}
