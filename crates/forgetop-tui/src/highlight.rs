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
    CSharp,
    Json,
    Yaml,
    Toml,
    Markdown,
    Shell,
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
            Lang::CSharp => "cs",
            Lang::Json => "json",
            Lang::Yaml => "yml",
            Lang::Toml => "toml",
            Lang::Markdown => "md",
            Lang::Shell => "sh",
        }
    }
}

/// Detect the language from a file path's extension, or `None` if we don't highlight it
/// (the caller then renders the line plain, exactly as before).
///
/// A leading-dot filename like `.bashrc` has no extension in the usual sense, but its
/// single segment lands in `ext` all the same — which is how shell dotfiles are matched.
pub fn lang_for(path: &str) -> Option<Lang> {
    let ext = path.rsplit('.').next().filter(|e| *e != path)?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => Lang::Rust,
        "ts" | "tsx" => Lang::TypeScript,
        "js" | "jsx" | "mjs" | "cjs" => Lang::JavaScript,
        "py" | "pyw" => Lang::Python,
        "go" => Lang::Go,
        "java" => Lang::Java,
        // MSBuild XML rides on the C# grammar: PascalCase tags/attributes land on its
        // `struct` rule, so elements colour as types and quoted values as strings.
        "cs" | "csproj" | "fsproj" | "vbproj" | "props" | "targets" => Lang::CSharp,
        "json" => Lang::Json,
        "yaml" | "yml" => Lang::Yaml,
        "toml" => Lang::Toml,
        "md" | "markdown" => Lang::Markdown,
        // `zsh` has no grammar of its own; the shell rules are close enough.
        "sh" | "bash" | "zsh" => Lang::Shell,
        // Shell dotfiles: `.bashrc` has no extension in the usual sense, but the segment
        // after its leading dot lands in `ext` all the same, so we match on that.
        "bashrc" | "bash_profile" | "bash_aliases" | "bash_logout" | "profile" | "zshrc"
        | "zshenv" | "zprofile" | "zlogin" | "zlogout" => Lang::Shell,
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
    /// A section title — a Markdown `#` heading or a TOML `[table]` header.
    Heading,
    /// A Markdown link or image target.
    Link,
    /// Emphasised prose: bold, italic and strikethrough all collapse here, since the
    /// distinction costs three more kinds and reads the same at diff scale.
    Emph,
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
        // Markup categories (Markdown, TOML).
        "heading" | "table" => HlKind::Heading,
        "link" | "image" => HlKind::Link,
        "bold" | "italic" | "strikethrough" => HlKind::Emph,
        "block" => HlKind::Str,     // inline code / fences read like strings
        "math" => HlKind::Number,
        "quote" | "list" | "linebreak" => HlKind::Comment,
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
        assert_eq!(lang_for("Controllers/SampleController.cs"), Some(Lang::CSharp));
        assert_eq!(lang_for("src/SampleApp.csproj"), Some(Lang::CSharp));
        assert_eq!(lang_for("Directory.Build.props"), Some(Lang::CSharp));
        assert_eq!(lang_for("build/Common.targets"), Some(Lang::CSharp));
        assert_eq!(lang_for("src/Lib.fsproj"), Some(Lang::CSharp));
        assert_eq!(lang_for("src/Legacy.vbproj"), Some(Lang::CSharp));
        assert_eq!(lang_for("Cargo.toml"), Some(Lang::Toml));
        assert_eq!(lang_for("README.md"), Some(Lang::Markdown));
        assert_eq!(lang_for("docs/guide.markdown"), Some(Lang::Markdown));
        assert_eq!(lang_for("scripts/release.sh"), Some(Lang::Shell));
        assert_eq!(lang_for("ci/setup.bash"), Some(Lang::Shell));
        assert_eq!(lang_for("tools/env.zsh"), Some(Lang::Shell));
    }

    #[test]
    fn detects_shell_dotfiles() {
        // No extension in the usual sense — the segment after the leading dot is what we match.
        assert_eq!(lang_for(".bashrc"), Some(Lang::Shell));
        assert_eq!(lang_for(".zshrc"), Some(Lang::Shell));
        assert_eq!(lang_for("home/.bash_profile"), Some(Lang::Shell));
        assert_eq!(lang_for("skel/.profile"), Some(Lang::Shell));
        // Only shell dotfiles opt in — an unknown one is still plain.
        assert_eq!(lang_for(".gitignore"), None);
        assert_eq!(lang_for(".editorconfig"), None);
        // A dotless file of the same name has no extension at all → no highlighting.
        assert_eq!(lang_for("bashrc"), None);
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
    fn highlights_csharp_keyword_type_and_string() {
        let toks = highlight_line(Lang::CSharp, r#"public class Sample { const string S = "x"; }"#);
        // The full line is covered by the returned spans (nothing dropped).
        let joined: String = toks.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, r#"public class Sample { const string S = "x"; }"#);
        assert!(has(&toks, "public", HlKind::Keyword), "`public` is a keyword: {toks:?}");
        assert!(has(&toks, "Sample", HlKind::Type), "PascalCase name is a type: {toks:?}");
        assert!(has(&toks, "x", HlKind::Str), "double-quoted string: {toks:?}");
    }

    #[test]
    fn highlights_toml_table_and_string() {
        let toks = highlight_line(Lang::Toml, r#"[package]"#);
        assert!(has(&toks, "[package]", HlKind::Heading), "table header is a heading: {toks:?}");
        let toks = highlight_line(Lang::Toml, r#"name = "forgetop" # ours"#);
        assert!(has(&toks, "forgetop", HlKind::Str), "quoted value: {toks:?}");
        assert!(has(&toks, "ours", HlKind::Comment), "trailing `#` comment: {toks:?}");
    }

    #[test]
    fn highlights_markdown_heading_and_code_span() {
        let toks = highlight_line(Lang::Markdown, "# Title");
        assert!(has(&toks, "# Title", HlKind::Heading), "`#` heading: {toks:?}");
        let toks = highlight_line(Lang::Markdown, "run `cargo test` now");
        assert!(has(&toks, "cargo test", HlKind::Str), "inline code reads as a string: {toks:?}");
    }

    #[test]
    fn highlights_shell_keyword_and_comment() {
        let toks = highlight_line(Lang::Shell, r#"if [ -f x ]; then echo "hi"; fi # note"#);
        let joined: String = toks.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, r#"if [ -f x ]; then echo "hi"; fi # note"#);
        assert!(has(&toks, "if", HlKind::Keyword), "`if` is a keyword: {toks:?}");
        assert!(has(&toks, "note", HlKind::Comment), "trailing `#` comment: {toks:?}");
    }

    #[test]
    fn unknown_language_still_covers_the_line_as_plain() {
        // (Exercised via the renderer for real; here we just confirm the join is lossless.)
        let toks = highlight_line(Lang::Json, r#"{"k": 1}"#);
        let joined: String = toks.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, r#"{"k": 1}"#);
    }
}
