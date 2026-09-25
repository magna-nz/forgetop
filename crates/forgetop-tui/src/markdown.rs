//! Markdown descriptions (PR and work-item bodies) as styled terminal lines.
//!
//! The markup is rendered, not shown: `## Summary` becomes a bold heading, `**x**` bold text,
//! `- [x]` a ticked box. A line break in the source stays a line break (as GitHub shows a PR
//! body), so plain text — Azure's flattened HTML, a Jira description — reads as it always did.
//! Wrapping is left to the `Paragraph` the lines land in.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Renders `text` as markdown. `base` is the colour of body text.
pub fn render(text: &str, theme: &Theme, base: Color) -> Vec<Line<'static>> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut r = Renderer {
        theme,
        base,
        lines: Vec::new(),
        cur: Vec::new(),
        bold: 0,
        italic: 0,
        strike: 0,
        heading: None,
        under_heading: false,
        quote: 0,
        code_block: false,
        table_head: false,
        lists: Vec::new(),
        marker: None,
        links: Vec::new(),
    };
    for event in Parser::new_ext(text, opts) {
        r.event(event);
    }
    r.flush();
    while r.lines.last().is_some_and(|l| l.spans.is_empty()) {
        r.lines.pop();
    }
    r.lines
}

struct Renderer<'t> {
    theme: &'t Theme,
    base: Color,
    lines: Vec<Line<'static>>,
    /// Spans of the line being built.
    cur: Vec<Span<'static>>,
    bold: usize,
    italic: usize,
    strike: usize,
    heading: Option<HeadingLevel>,
    /// The last line written was a heading, so the next block sits directly under it.
    under_heading: bool,
    quote: usize,
    code_block: bool,
    table_head: bool,
    /// Open lists, innermost last: the next number of an ordered list, `None` for bullets.
    lists: Vec<Option<u64>>,
    /// The marker (`• `, `3. `, `☑ `) waiting to start the first line of a list item.
    marker: Option<Span<'static>>,
    /// Open links: the destination, and where the link's text starts in `cur`.
    links: Vec<(String, usize)>,
}

impl Renderer<'_> {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) if self.code_block => {
                for l in t.lines() {
                    self.cur.push(Span::styled(format!("  {l}"), Style::default().fg(self.theme.cyan)));
                    self.flush();
                }
            }
            Event::Text(t) => self.cur.push(Span::styled(t.into_string(), self.style())),
            Event::Code(t) => self.cur.push(Span::styled(t.into_string(), Style::default().fg(self.theme.cyan))),
            // HTML comments are the prompts of a PR template; anything else is shown as written.
            Event::Html(t) | Event::InlineHtml(t) => {
                if t.trim_start().starts_with("<!--") {
                    return;
                }
                let dim = Style::default().fg(self.theme.dim);
                let mut parts = t.split('\n').peekable();
                while let Some(part) = parts.next() {
                    if !part.is_empty() {
                        self.cur.push(Span::styled(part.to_string(), dim));
                    }
                    if parts.peek().is_some() {
                        self.flush();
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak => self.flush(),
            Event::Rule => {
                self.gap();
                self.cur.push(Span::styled("─".repeat(40), Style::default().fg(self.theme.dim)));
                self.flush();
            }
            Event::TaskListMarker(done) => {
                let (glyph, color) = if done { ("☑ ", self.theme.green) } else { ("☐ ", self.theme.dim) };
                self.marker = Some(Span::styled(glyph, Style::default().fg(color)));
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => self.gap(),
            Tag::Heading { level, .. } => {
                self.gap();
                self.heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                self.gap();
                self.quote += 1;
            }
            Tag::CodeBlock(_) => {
                self.gap();
                self.code_block = true;
            }
            Tag::List(first) => {
                // A nested list continues its parent item; a top-level one is its own block.
                if self.lists.is_empty() {
                    self.gap();
                } else {
                    self.flush();
                }
                self.lists.push(first);
            }
            Tag::Item => {
                self.flush();
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    _ => "• ".to_string(),
                };
                self.marker = Some(Span::styled(marker, Style::default().fg(self.theme.accent)));
            }
            Tag::Table(_) => self.gap(),
            Tag::TableHead => self.table_head = true,
            Tag::TableCell => {
                if !self.cur.is_empty() {
                    self.cur.push(Span::styled(" │ ", Style::default().fg(self.theme.dim)));
                }
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => self.links.push((dest_url.into_string(), self.cur.len())),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Item | TagEnd::TableRow => self.flush(),
            TagEnd::Heading(_) => {
                self.flush();
                self.heading = None;
                self.under_heading = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote -= 1;
            }
            TagEnd::CodeBlock => {
                self.flush();
                self.code_block = false;
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::TableHead => {
                self.flush();
                self.table_head = false;
            }
            TagEnd::Emphasis => self.italic -= 1,
            TagEnd::Strong => self.bold -= 1,
            TagEnd::Strikethrough => self.strike -= 1,
            TagEnd::Link | TagEnd::Image => {
                let Some((url, from)) = self.links.pop() else { return };
                let text: String = self.cur[from..].iter().map(|s| s.content.as_ref()).collect();
                // A bare link already shows its address; otherwise name it after the text.
                if !url.is_empty() && text != url {
                    self.cur.push(Span::styled(format!(" ({url})"), Style::default().fg(self.theme.dim)));
                }
            }
            _ => {}
        }
    }

    /// The style for body text at the current nesting of emphasis, heading and link.
    fn style(&self) -> Style {
        let mut s = Style::default().fg(self.base);
        if self.heading.is_some() || self.table_head {
            s = s.fg(self.theme.accent).add_modifier(Modifier::BOLD);
        }
        if !self.links.is_empty() {
            s = s.fg(self.theme.blue).add_modifier(Modifier::UNDERLINED);
        }
        if self.bold > 0 {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            s = s.add_modifier(Modifier::CROSSED_OUT);
        }
        s
    }

    /// Ends the line being built, prefixed with its quote bars and list indentation.
    fn flush(&mut self) {
        if self.cur.is_empty() && self.marker.is_none() {
            return;
        }
        let mut spans = Vec::new();
        if self.quote > 0 {
            spans.push(Span::styled("│ ".repeat(self.quote), Style::default().fg(self.theme.dim)));
        }
        let depth = self.lists.len();
        match self.marker.take() {
            Some(marker) => {
                spans.push(Span::raw("  ".repeat(depth.saturating_sub(1))));
                spans.push(marker);
            }
            None if depth > 0 => spans.push(Span::raw("  ".repeat(depth))),
            None => {}
        }
        spans.append(&mut self.cur);
        self.lines.push(Line::from(spans));
        self.under_heading = false;
    }

    /// Ends the current line and opens a new block, with one blank line between blocks — except
    /// under a heading, and for the first block of a list item, which sits on the item's marker.
    fn gap(&mut self) {
        self.flush();
        if self.marker.is_none() && !self.under_heading && self.lines.last().is_some_and(|l| !l.spans.is_empty()) {
            self.lines.push(Line::from(""));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
    }

    fn render_plain(md: &str) -> Vec<String> {
        let theme = Theme::by_name("slate");
        plain(&render(md, &theme, theme.fg))
    }

    #[test]
    fn markup_is_rendered_not_shown() {
        let md = "## Summary\n- **Bug:** a `branch` filled the column\n- Fix in `lp_widths`\n\n## Test plan\n- [x] cargo test\n- [ ] live check";
        let out = render_plain(md);
        assert_eq!(
            out,
            vec![
                "Summary",
                "• Bug: a branch filled the column",
                "• Fix in lp_widths",
                "",
                "Test plan",
                "☑ cargo test",
                "☐ live check",
            ]
        );
        for line in &out {
            for leak in ["**", "##", "`", "[x]", "[ ]"] {
                assert!(!line.contains(leak), "{leak:?} leaked into {line:?}");
            }
        }
    }

    #[test]
    fn headings_and_emphasis_are_styled() {
        let theme = Theme::by_name("slate");
        let lines = render("# Title\n\nsome **bold** and *soft* and ~~gone~~", &theme, theme.fg);
        let heading = &lines[0].spans[0];
        assert_eq!(heading.style.fg, Some(theme.accent));
        assert!(heading.style.add_modifier.contains(Modifier::BOLD));
        let body = &lines[1].spans;
        let find = |t: &str| body.iter().find(|s| s.content == t).unwrap_or_else(|| panic!("{t} span")).style.add_modifier;
        assert!(find("bold").contains(Modifier::BOLD));
        assert!(find("soft").contains(Modifier::ITALIC));
        assert!(find("gone").contains(Modifier::CROSSED_OUT));
    }

    /// Azure's flattened HTML and Jira's text aren't markdown; they must read as they did.
    #[test]
    fn plain_text_keeps_its_lines_and_paragraphs() {
        let out = render_plain("First line\nsecond line\n\nNew paragraph.");
        assert_eq!(out, vec!["First line", "second line", "", "New paragraph."]);
    }

    #[test]
    fn nested_and_ordered_lists_indent() {
        let out = render_plain("1. one\n2. two\n   - inner\n3. three");
        assert_eq!(out, vec!["1. one", "2. two", "  • inner", "3. three"]);
    }

    #[test]
    fn code_quotes_links_and_tables() {
        let out = render_plain("```\nlet x = 1;\n```\n\n> quoted\n\nsee [docs](https://x.test) or https://y.test\n\n| a | b |\n|---|---|\n| 1 | 2 |");
        assert_eq!(
            out,
            vec![
                "  let x = 1;",
                "",
                "│ quoted",
                "",
                "see docs (https://x.test) or https://y.test",
                "",
                "a │ b",
                "1 │ 2",
            ]
        );
    }

    #[test]
    fn pr_template_comments_are_hidden() {
        assert_eq!(render_plain("<!-- describe your change -->\nReal text"), vec!["Real text"]);
    }
}
