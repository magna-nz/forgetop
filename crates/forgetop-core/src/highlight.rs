//! Syntax highlighting for diff patches — shared by the TUI and the dashboard.
//!
//! We map a source line to a sequence of **semantic token kinds** ([`HlKind`]) and let each
//! frontend colour those from its own palette: the TUI from the theme's *indexed* colours
//! (never truecolor RGB, which washes out on some terminals — see the TUI's `theme.rs`), the
//! dashboard from the same-named CSS tokens via `POST /api/highlight`. `synoptic` does the
//! tokenising; it lives behind this module's small interface so the backing crate stays
//! swappable. Languages `synoptic` has no grammar for (HCL) get a hand-written one here.
//!
//! Highlighting is **line-based**: each patch line is tokenised on its own, so multi-line
//! strings/comments don't carry context across lines. That's an accepted tradeoff for diff
//! fragments (where we rarely have the whole file anyway).

use serde::Serialize;
use std::sync::OnceLock;
use synoptic::{Highlighter, TokOpt};

/// The width the tokenizer expands a tab to before matching. [`LineHighlighter::spans`]
/// undoes that expansion, so the two must agree.
const TAB_WIDTH: usize = 4;

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
    /// HashiCorp Configuration Language — Terraform `.tf`/`.tfvars` and generic `.hcl`.
    Hcl,
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
            // No synoptic grammar: `highlighter()` builds ours instead of asking by extension.
            Lang::Hcl => "hcl",
        }
    }

    /// The language's wire name (`"rust"`, `"hcl"`, …), as the dashboard's `/api/highlight`
    /// response reports it.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::TypeScript => "typescript",
            Lang::JavaScript => "javascript",
            Lang::Python => "python",
            Lang::Go => "go",
            Lang::Java => "java",
            Lang::CSharp => "csharp",
            Lang::Json => "json",
            Lang::Yaml => "yaml",
            Lang::Toml => "toml",
            Lang::Markdown => "markdown",
            Lang::Shell => "shell",
            Lang::Hcl => "hcl",
        }
    }

    /// A fresh tokenizer for this language, or `None` when the backing crate has no rules
    /// for it. Languages with a grammar of our own are cloned from a lazily-built template.
    fn highlighter(self) -> Option<Highlighter> {
        match self {
            Lang::Hcl => Some(hcl_highlighter().clone()),
            other => synoptic::from_extension(other.synoptic_ext(), TAB_WIDTH),
        }
    }
}

/// The HCL grammar. Line-based like everything else here, so a heredoc or a `/* … */`
/// comment spanning lines is only coloured on the line that opens it.
///
/// Rules are deliberately anchored so a word means what it means in HCL rather than
/// everywhere it appears: block types (`resource`, `data`, …) only at the start of a line,
/// meta-arguments (`count`, `for_each`, …) only when assigned, and `var.`/`local.`/`data.`
/// only as a reference prefix — so `data "aws_vpc" "main" {` and `data.aws_vpc.main.id`
/// read differently, which is what an editor does.
fn hcl_highlighter() -> &'static Highlighter {
    static HL: OnceLock<Highlighter> = OnceLock::new();
    HL.get_or_init(|| {
        let mut h = Highlighter::new(TAB_WIDTH);
        // Comments are *bounded* (opener to end of line) rather than a `(#.*)$` keyword: a
        // keyword regex is matched once per line without overlap, so a `#` inside a string
        // would claim the match, be discarded for sitting inside the string, and leave a real
        // comment after it uncoloured. A bounded opener inside a string is simply skipped and
        // the next one still opens. Defined first so an opener wins a tie on start column.
        h.bounded("comment", r"/\*", r"\*/", false);
        h.bounded("comment", "#", "$", false);
        h.bounded("comment", "//", "$", false);
        // Strings, with `${…}` interpolation tokenised as code inside them.
        h.bounded_interp("string", "\"", "\"", r"\$\{", r"\}", true);
        // Block types open a line and are followed by a label, a brace or nothing:
        // `resource "aws_s3_bucket" "logs" {`, `  lifecycle {`. The tail is required so a
        // reference continuing a line (`data.x.y`, `module.vpc.id`) or an attribute of the
        // same name (`content = …`) isn't mistaken for one. Only the name is the token.
        h.keyword(
            "keyword",
            r#"^\s*(resource|data|variable|output|module|provider|locals|terraform|backend|provisioner|connection|dynamic|content|lifecycle|moved|import|check|removed|required_providers|cloud)\s*(?:"|\{|$)"#,
        );
        // Meta-arguments and the conventional top-of-block attributes, only when assigned.
        h.keyword("keyword", r"^\s*(count|for_each|depends_on|provider|source|version|required_version)\s*=");
        // Expression keywords: for-expressions and template directives.
        h.keyword("keyword", r"\b(for|in|if|else|endif|endfor)\b");
        h.keyword("boolean", r"\b(true|false|null)\b");
        // Type constructors: `list(string)`, `optional(number, 1)`. Listed before functions so
        // the tie at `list(` goes to the type. Not after a `.`: `var.number` is an attribute.
        h.keyword("type", r"(?:^|[^.\w])(string|number|bool|list|map|set|object|tuple|any|optional)\b");
        // Reference prefixes: `var.name`, `local.x`, `each.key`, `module.vpc.id`.
        h.keyword("namespace", r"\b(var|local|module|data|each|path|terraform|self)\.");
        h.keyword("function", r"\b([a-z_][a-z0-9_]*)\(");
        h.keyword("digit", r"\b(\d+(?:\.\d+)?)\b");
        h
    })
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
        // Terraform and its variable files, plus generic HCL (Packer, Nomad, Consul).
        "tf" | "tfvars" | "hcl" => Lang::Hcl,
        _ => return None,
    })
}

/// Semantic token kind — each frontend maps it to a colour of its own. Serialises as its
/// lowercase name (`"keyword"`, `"str"`, …), which is the `kind` the dashboard reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
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
    hl: Highlighter,
}

impl LineHighlighter {
    /// Build a highlighter for `lang`, or `None` if the backing tokenizer has no rules for it.
    pub fn new(lang: Lang) -> Option<LineHighlighter> {
        lang.highlighter().map(|hl| LineHighlighter { hl })
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

impl LineHighlighter {
    /// Tokenise one source line into `(len, kind)` spans over the **original** text, with
    /// `len` in UTF-16 code units — what a JavaScript string slices by — so the dashboard can
    /// colour the line it already holds without the text making a round trip. Adjacent spans
    /// of one kind are merged. The tokenizer expands tabs to spaces before matching; here
    /// each tab is mapped back to the single unit it is in the source, taking the kind of the
    /// column it starts in.
    pub fn spans(&mut self, text: &str) -> Vec<(usize, HlKind)> {
        // One kind per expanded column, in the tokenizer's tab-expanded coordinates.
        let expanded: Vec<HlKind> = self
            .line(text)
            .into_iter()
            .flat_map(|(t, k)| std::iter::repeat_n(k, t.chars().count()))
            .collect();
        let mut out: Vec<(usize, HlKind)> = Vec::new();
        let mut col = 0;
        for c in text.chars() {
            let kind = expanded.get(col).copied().unwrap_or(HlKind::Plain);
            col += if c == '\t' { TAB_WIDTH } else { 1 };
            let units = c.len_utf16();
            match out.last_mut() {
                Some((len, k)) if *k == kind => *len += units,
                _ => out.push((units, kind)),
            }
        }
        out
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
        assert_eq!(lang_for("terraform/modules/web/frontdoor/main.tf"), Some(Lang::Hcl));
        assert_eq!(lang_for("env/prod.tfvars"), Some(Lang::Hcl));
        assert_eq!(lang_for("packer/build.hcl"), Some(Lang::Hcl));
        assert_eq!(lang_for("infra/MAIN.TF"), Some(Lang::Hcl)); // case-insensitive
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

    /// Every returned span, in order, re-joins to the input (nothing dropped or reordered).
    fn covers(tokens: &[(String, HlKind)], line: &str) {
        let joined: String = tokens.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, line);
    }

    /// The kind of the span whose text is exactly `needle`, if there is one.
    fn kind_of(tokens: &[(String, HlKind)], needle: &str) -> Option<HlKind> {
        tokens.iter().find(|(t, _)| t == needle).map(|(_, k)| *k)
    }

    #[test]
    fn hcl_block_header_keyword_and_label_strings() {
        let line = r#"resource "azurerm_cdn_frontdoor_origin" "web" {"#;
        let toks = highlight_line(Lang::Hcl, line);
        covers(&toks, line);
        assert_eq!(kind_of(&toks, "resource"), Some(HlKind::Keyword), "{toks:?}");
        assert!(has(&toks, "azurerm_cdn_frontdoor_origin", HlKind::Str), "labels are strings: {toks:?}");
        assert!(has(&toks, "web", HlKind::Str), "{toks:?}");
        // Nested block types indent; `dynamic "x" {` and `lifecycle {` are keywords too.
        let toks = highlight_line(Lang::Hcl, r#"  dynamic "private_link" {"#);
        assert_eq!(kind_of(&toks, "dynamic"), Some(HlKind::Keyword), "{toks:?}");
        let toks = highlight_line(Lang::Hcl, "  lifecycle {");
        assert_eq!(kind_of(&toks, "lifecycle"), Some(HlKind::Keyword), "{toks:?}");
    }

    #[test]
    fn hcl_block_type_only_at_line_start_but_reference_prefix_anywhere() {
        // `data` opening a block is a keyword …
        let toks = highlight_line(Lang::Hcl, r#"data "azurerm_subscription" "current" {}"#);
        assert_eq!(kind_of(&toks, "data"), Some(HlKind::Keyword), "{toks:?}");
        // … while `data.` inside an expression is a reference prefix (a type-coloured namespace).
        let line = "  target = data.azurerm_subscription.current.id";
        let toks = highlight_line(Lang::Hcl, line);
        covers(&toks, line);
        assert_eq!(kind_of(&toks, "data"), Some(HlKind::Type), "{toks:?}");
        // The attribute name on the left stays plain.
        assert!(has(&toks, "target", HlKind::Plain), "{toks:?}");
        let toks = highlight_line(Lang::Hcl, "  name = var.origin_name");
        assert_eq!(kind_of(&toks, "var"), Some(HlKind::Type), "{toks:?}");
        let toks = highlight_line(Lang::Hcl, "  key = each.key");
        assert_eq!(kind_of(&toks, "each"), Some(HlKind::Type), "{toks:?}");
    }

    #[test]
    fn hcl_reference_continuing_a_line_is_not_a_block_type() {
        // A block-type name at the start of a continuation line, as a reference prefix …
        for line in ["    data.azurerm_subscription.current.id,", "    module.vpc.vpc_id,", "  terraform.workspace"] {
            let toks = highlight_line(Lang::Hcl, line);
            covers(&toks, line);
            let first = toks.iter().find(|(t, _)| !t.trim().is_empty()).unwrap();
            assert_eq!(first.1, HlKind::Type, "{line:?}: reference prefix, not block type: {toks:?}");
        }
        // … or as an attribute of the same name, is not a block keyword.
        let toks = highlight_line(Lang::Hcl, "    content = jsonencode(local.x)");
        assert!(!has(&toks, "content", HlKind::Keyword) && has(&toks, "content", HlKind::Plain), "{toks:?}");
        assert_eq!(kind_of(&toks, "jsonencode"), Some(HlKind::Func), "{toks:?}");
        // While the real block forms still are: labelled, bare, and at end of line.
        for line in [r#"data "azurerm_subscription" "current" {"#, "  content {", "terraform", "locals{"] {
            let toks = highlight_line(Lang::Hcl, line);
            let first = toks.iter().find(|(t, _)| !t.trim().is_empty()).unwrap();
            assert_eq!(first.1, HlKind::Keyword, "{line:?}: {toks:?}");
        }
    }

    #[test]
    fn hcl_trailing_comment_after_a_string_holding_a_comment_marker() {
        for (line, comment) in [
            (r#"  description = "issue #123 // not a comment" # real"#, "# real"),
            (r#"  url = "http://x" // c"#, "// c"),
            (r#"  source = "git::https://example.net/mods//vpc?ref=v1" # pinned"#, "# pinned"),
            (r#"  x = "a \" # b" # c"#, "# c"),
        ] {
            let toks = highlight_line(Lang::Hcl, line);
            covers(&toks, line);
            assert_eq!(kind_of(&toks, comment), Some(HlKind::Comment), "{line:?}: {toks:?}");
            // The marker inside the string stays part of the string.
            assert!(toks.iter().all(|(t, k)| *k != HlKind::Comment || t == comment), "{line:?}: {toks:?}");
        }
        // A comment with no string before it, and one on a line of its own, still work.
        let toks = highlight_line(Lang::Hcl, "  ttl = 600 # ten minutes");
        assert_eq!(kind_of(&toks, "# ten minutes"), Some(HlKind::Comment), "{toks:?}");
        let toks = highlight_line(Lang::Hcl, "// header");
        assert_eq!(toks, vec![("// header".to_string(), HlKind::Comment)]);
    }

    #[test]
    fn hcl_type_name_after_a_dot_is_an_attribute() {
        let toks = highlight_line(Lang::Hcl, "  n = var.number + local.list[0]");
        assert!(!has(&toks, "number", HlKind::Type) && has(&toks, "number", HlKind::Plain), "{toks:?}");
        assert!(!has(&toks, "list", HlKind::Type) && has(&toks, "list", HlKind::Plain), "{toks:?}");
        assert_eq!(kind_of(&toks, "var"), Some(HlKind::Type), "{toks:?}");
        let toks = highlight_line(Lang::Hcl, "  type = number");
        assert_eq!(kind_of(&toks, "number"), Some(HlKind::Type), "{toks:?}");
    }

    #[test]
    fn hcl_meta_arguments_only_when_assigned() {
        let toks = highlight_line(Lang::Hcl, "  for_each = var.origins");
        assert_eq!(kind_of(&toks, "for_each"), Some(HlKind::Keyword), "{toks:?}");
        let toks = highlight_line(Lang::Hcl, "  count = length(var.zones)");
        assert_eq!(kind_of(&toks, "count"), Some(HlKind::Keyword), "{toks:?}");
        assert_eq!(kind_of(&toks, "length"), Some(HlKind::Func), "{toks:?}");
        // `count` as a plain attribute reference (`x.count`) is not a keyword.
        let toks = highlight_line(Lang::Hcl, "  total = local.count + 1");
        assert!(!has(&toks, "count", HlKind::Keyword), "{toks:?}");
        assert_eq!(kind_of(&toks, "1"), Some(HlKind::Number), "{toks:?}");
    }

    #[test]
    fn hcl_string_with_interpolation_comments_and_literals() {
        let line = r#"  description = "Approved by ${var.actor} for Front Door" # INF-1067"#;
        let toks = highlight_line(Lang::Hcl, line);
        covers(&toks, line);
        assert!(has(&toks, "Approved by ", HlKind::Str), "{toks:?}");
        assert!(has(&toks, "for Front Door", HlKind::Str), "{toks:?}");
        assert_eq!(kind_of(&toks, "var"), Some(HlKind::Type), "interpolation is tokenised: {toks:?}");
        assert!(has(&toks, "INF-1067", HlKind::Comment), "{toks:?}");
        // `//` comments and the C-style opener on the same line.
        let toks = highlight_line(Lang::Hcl, "  enabled = true // always");
        assert_eq!(kind_of(&toks, "true"), Some(HlKind::Keyword), "{toks:?}");
        assert!(has(&toks, "always", HlKind::Comment), "{toks:?}");
        let toks = highlight_line(Lang::Hcl, "  /* legacy */ ttl = 600");
        assert!(has(&toks, "legacy", HlKind::Comment), "{toks:?}");
        assert_eq!(kind_of(&toks, "600"), Some(HlKind::Number), "{toks:?}");
        // A keyword inside a string stays a string.
        let toks = highlight_line(Lang::Hcl, r#"  note = "resource for data""#);
        assert!(!has(&toks, "resource", HlKind::Keyword), "{toks:?}");
        assert!(!has(&toks, "for", HlKind::Keyword), "{toks:?}");
    }

    #[test]
    fn hcl_types_and_for_expressions() {
        let line = "  type = list(object({ name = string, ttl = optional(number, 60) }))";
        let toks = highlight_line(Lang::Hcl, line);
        covers(&toks, line);
        assert_eq!(kind_of(&toks, "list"), Some(HlKind::Type), "type wins over function: {toks:?}");
        assert_eq!(kind_of(&toks, "string"), Some(HlKind::Type), "{toks:?}");
        assert_eq!(kind_of(&toks, "optional"), Some(HlKind::Type), "{toks:?}");
        let line = "  ids = [for o in var.origins : o.id if o.enabled]";
        let toks = highlight_line(Lang::Hcl, line);
        covers(&toks, line);
        assert_eq!(kind_of(&toks, "for"), Some(HlKind::Keyword), "{toks:?}");
        assert_eq!(kind_of(&toks, "in"), Some(HlKind::Keyword), "{toks:?}");
        assert_eq!(kind_of(&toks, "if"), Some(HlKind::Keyword), "{toks:?}");
    }

    /// Spans for `line`, asserting they cover it exactly in UTF-16 units.
    fn spans_covering(lang: Lang, line: &str) -> Vec<(usize, HlKind)> {
        let spans = LineHighlighter::new(lang).unwrap().spans(line);
        let total: usize = spans.iter().map(|(n, _)| n).sum();
        assert_eq!(total, line.encode_utf16().count(), "spans cover the line: {spans:?}");
        spans
    }

    #[test]
    fn spans_cover_the_line_in_utf16_units_and_merge_neighbours() {
        let spans = spans_covering(Lang::Hcl, r#"resource "aws_s3_bucket" "logs" {"#);
        // `resource` is one keyword span; the space, two label strings and brace follow.
        assert_eq!(spans[0], (8, HlKind::Keyword), "{spans:?}");
        assert_eq!(spans[1], (1, HlKind::Plain), "{spans:?}");
        assert_eq!(spans[2], (15, HlKind::Str), "quoted label incl. quotes: {spans:?}");
        // Adjacent same-kind runs merge: nothing in the result repeats a kind back to back.
        for w in spans.windows(2) {
            assert_ne!(w[0].1, w[1].1, "merged: {spans:?}");
        }
    }

    #[test]
    fn spans_count_a_tab_as_one_unit_and_an_astral_char_as_two() {
        // The tokenizer sees the tab as four spaces; the source has one character.
        let line = "\tttl = 600 # 🚀 lift-off";
        let spans = spans_covering(Lang::Hcl, line);
        // The tab and the plain `ttl = ` after it merge: 1 + 6 units, not 4 + 6.
        assert_eq!(spans[0], (7, HlKind::Plain), "the leading tab is one unit: {spans:?}");
        let (n, k) = *spans.last().unwrap();
        assert_eq!(k, HlKind::Comment, "{spans:?}");
        // "# 🚀 lift-off" is 12 chars but 13 UTF-16 units (the rocket is a surrogate pair).
        assert_eq!(n, 13, "{spans:?}");
        assert!(spans.contains(&(3, HlKind::Number)), "{spans:?}");
    }

    #[test]
    fn spans_of_an_empty_line_are_empty() {
        assert!(LineHighlighter::new(Lang::Hcl).unwrap().spans("").is_empty());
    }

    #[test]
    fn lang_names_are_lowercase_and_distinct() {
        let names: Vec<&str> = [Lang::Rust, Lang::TypeScript, Lang::CSharp, Lang::Hcl].iter().map(|l| l.name()).collect();
        assert_eq!(names, ["rust", "typescript", "csharp", "hcl"]);
    }

    #[test]
    fn hl_kind_serialises_as_lowercase_name() {
        assert_eq!(serde_json::to_string(&HlKind::Keyword).unwrap(), "\"keyword\"");
        assert_eq!(serde_json::to_string(&HlKind::Str).unwrap(), "\"str\"");
        assert_eq!(serde_json::to_string(&HlKind::Plain).unwrap(), "\"plain\"");
    }

    #[test]
    fn unknown_language_still_covers_the_line_as_plain() {
        // (Exercised via the renderer for real; here we just confirm the join is lossless.)
        let toks = highlight_line(Lang::Json, r#"{"k": 1}"#);
        let joined: String = toks.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, r#"{"k": 1}"#);
    }
}
