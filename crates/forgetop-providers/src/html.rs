//! HTML → plain text, for provider fields that carry markup.
//!
//! Azure DevOps stores a work item's `System.Description` as HTML. Neither frontend renders HTML
//! — the TUI can't, and the dashboard would have to sanitise it first — so the markup is
//! flattened here, in the mapper, and both frontends get the same readable text.
//! See `azure::map_work_item`.

/// Element names we recognise. Necessary but not sufficient evidence of markup on its own —
/// see [`is_markup`] for what actually counts as proof.
const KNOWN_TAGS: &[&str] = &[
    "a", "abbr", "address", "article", "aside", "b", "big", "blockquote", "body", "br", "button", "canvas", "caption",
    "center", "cite", "code", "col", "colgroup", "dd", "del", "details", "div", "dl", "dt", "em", "embed", "fieldset",
    "figcaption", "figure", "font", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "hr",
    "html", "i", "iframe", "img", "input", "ins", "kbd", "label", "legend", "li", "link", "main", "mark", "meta",
    "nav", "ol", "option", "p", "picture", "pre", "q", "s", "samp", "script", "section", "select", "small", "source",
    "span", "strike", "strong", "style", "sub", "summary", "sup", "svg", "table", "tbody", "td", "textarea", "tfoot",
    "th", "thead", "time", "title", "tr", "tt", "u", "ul", "var", "video",
];

/// Containers whose boundary ends the current line. `br`/`hr` are deliberately absent — they get
/// their own arm in [`to_text`], because an explicit break means more than a structural one.
const BLOCK_TAGS: &[&str] = &[
    "address", "article", "aside", "blockquote", "body", "caption", "dd", "details", "div", "dl", "dt", "fieldset",
    "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "html", "legend", "li",
    "main", "nav", "ol", "p", "pre", "section", "summary", "table", "tbody", "tfoot", "thead", "tr", "ul",
];

/// Named entities worth decoding. Anything unlisted is left verbatim — a stray `&` in prose is far
/// more common than an exotic entity, and mangling it would be worse than leaving it alone.
const ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", "\u{a0}"),
    ("ensp", "\u{a0}"),
    ("emsp", "\u{a0}"),
    ("thinsp", "\u{a0}"),
    ("shy", ""),
    ("ndash", "–"),
    ("mdash", "—"),
    ("hellip", "…"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("sbquo", "‚"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("bdquo", "„"),
    ("bull", "•"),
    ("middot", "·"),
    ("copy", "©"),
    ("reg", "®"),
    ("trade", "™"),
    ("deg", "°"),
    ("plusmn", "±"),
    ("times", "×"),
    ("divide", "÷"),
    ("laquo", "«"),
    ("raquo", "»"),
    ("euro", "€"),
    ("pound", "£"),
    ("yen", "¥"),
    ("cent", "¢"),
    ("sect", "§"),
    ("para", "¶"),
    ("dagger", "†"),
    ("larr", "←"),
    ("rarr", "→"),
    ("uarr", "↑"),
    ("darr", "↓"),
    ("harr", "↔"),
    ("ne", "≠"),
    ("le", "≤"),
    ("ge", "≥"),
    ("frac12", "½"),
    ("frac14", "¼"),
    ("frac34", "¾"),
    ("sup2", "²"),
    ("sup3", "³"),
    ("micro", "µ"),
];

/// Void elements — `<br>` needs no closing tag, so on its own it's still proof of markup.
const VOID_TAGS: &[&str] =
    &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

/// A `<…>` run: either a tag we act on, or markup (comment, doctype) that only gets skipped.
enum Markup {
    Tag { name: String, closing: bool, self_closing: bool, terminated: bool },
    Skipped,
}

/// Where a tag's `>` was — and whether there was one at all.
struct TagEnd {
    end: usize,
    terminated: bool,
    self_closing: bool,
}

/// Flatten `input` to plain text. Text that isn't markup is returned unchanged.
pub fn to_text(input: &str) -> String {
    if !is_markup(input) {
        return input.to_string();
    }
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut pre_depth = 0usize;
    let mut list_depth = 0usize;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '<' => match parse_markup(&chars, i) {
                Some((Markup::Skipped, next)) => i = next,
                // `<b` that never closes is prose someone typed, not a tag — emitting it as text
                // keeps everything after it instead of consuming to the end of the description.
                Some((Markup::Tag { terminated: false, .. }, _)) => {
                    push_char(&mut out, '<', pre_depth > 0);
                    i += 1;
                }
                Some((Markup::Tag { name, closing, .. }, next)) => {
                    i = next;
                    match name.as_str() {
                        // Never content: drop the element body along with the tags.
                        "script" | "style" => {
                            if !closing {
                                i = skip_element(&chars, i, &name);
                            }
                        }
                        "pre" => {
                            pre_depth = if closing { pre_depth.saturating_sub(1) } else { pre_depth + 1 };
                            push_break(&mut out, false);
                        }
                        "ul" | "ol" => {
                            list_depth = if closing { list_depth.saturating_sub(1) } else { list_depth + 1 };
                            push_break(&mut out, false);
                        }
                        "li" => {
                            push_break(&mut out, false);
                            if !closing {
                                out.push_str(&"  ".repeat(list_depth.saturating_sub(1)));
                                out.push_str("- ");
                            }
                        }
                        // An explicit break is the author asking for a blank line; a boundary
                        // between block elements is just structure. Only the former may double.
                        "br" | "hr" => push_break(&mut out, true),
                        // Cells sit on one line, separated so the columns stay distinguishable.
                        "td" | "th" => {
                            if !closing && !out.is_empty() && !out.ends_with('\n') {
                                out.push_str(" | ");
                            }
                        }
                        n if BLOCK_TAGS.contains(&n) => push_break(&mut out, false),
                        _ => {}
                    }
                }
                // A bare `<` that starts nothing: it's text.
                None => {
                    push_char(&mut out, '<', pre_depth > 0);
                    i += 1;
                }
            },
            '&' => match parse_entity(&chars, i) {
                // Decoded text goes straight to the output and is never rescanned, so an escaped
                // `&lt;p&gt;` survives as literal `<p>` instead of being stripped as a tag.
                Some((decoded, next)) => {
                    for c in decoded.chars() {
                        push_char(&mut out, c, pre_depth > 0);
                    }
                    i = next;
                }
                None => {
                    push_char(&mut out, '&', pre_depth > 0);
                    i += 1;
                }
            },
            c => {
                push_char(&mut out, c, pre_depth > 0);
                i += 1;
            }
        }
    }
    // Hard spaces stayed U+00A0 so line ends couldn't eat them; they're ordinary spaces now.
    out.trim().replace('\u{a0}', " ")
}

/// Plain text → the HTML the field expects, for the write side of an HTML-valued field.
/// The text is escaped, not interpreted: the reader saw text, so the writer sends text.
///
/// Read-back is exact apart from three documented edges: tabs (an HTML field has nowhere to put
/// one), leading/trailing whitespace on the field as a whole (trimmed), and runs of more than one
/// blank line (capped at one, so a stray `<br>` storm can't push the content off screen).
pub fn to_html(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").split('\n').map(escape_line).collect::<Vec<_>>().join("<br>")
}

/// Escape one line, spelling every space that would otherwise be swallowed as `&nbsp;`. Leading
/// and repeated spaces are indentation the author typed — content, not decoration.
fn escape_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut after_space = true;
    for c in line.chars() {
        match c {
            ' ' if after_space => out.push_str("&nbsp;"),
            ' ' => {
                out.push(' ');
                after_space = true;
            }
            _ => {
                match c {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    c => out.push(c),
                }
                after_space = false;
            }
        }
    }
    // A space at the end of a line is swallowed just like a repeated one, so spell it out too.
    if out.ends_with(' ') {
        out.pop();
        out.push_str("&nbsp;");
    }
    out
}

/// Is this markup at all? The proof has to be something prose doesn't produce by accident: a
/// closing tag, a self-closing one, a void element like `<br>`, or an entity. A lone `<b …>` is
/// not enough — `if x<b and y>c` is arithmetic, and treating it as a tag would eat half the line.
fn is_markup(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '<' => match parse_markup(&chars, i) {
                Some((Markup::Tag { name, closing, self_closing, terminated: true }, next)) => {
                    if KNOWN_TAGS.contains(&name.as_str()) && (closing || self_closing || VOID_TAGS.contains(&name.as_str())) {
                        return true;
                    }
                    i = next;
                }
                Some((_, next)) => i = next,
                None => i += 1,
            },
            '&' => match parse_entity(&chars, i) {
                Some(_) => return true,
                None => i += 1,
            },
            _ => i += 1,
        }
    }
    false
}

/// Parse the `<…>` run starting at `start`. Returns the token and the index just past the `>`.
fn parse_markup(chars: &[char], start: usize) -> Option<(Markup, usize)> {
    let mut i = start + 1;
    if i >= chars.len() {
        return None;
    }
    // Comments and doctypes carry no content.
    if chars[i] == '!' {
        if chars[i..].starts_with(&['!', '-', '-']) {
            return Some((Markup::Skipped, find_seq(chars, i + 3, &['-', '-', '>']).unwrap_or(chars.len())));
        }
        return Some((Markup::Skipped, scan_to_gt(chars, i).end));
    }
    let closing = chars[i] == '/';
    if closing {
        i += 1;
    }
    if i >= chars.len() || !chars[i].is_ascii_alphabetic() {
        return None;
    }
    let name_start = i;
    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '-' | ':')) {
        i += 1;
    }
    let name: String = chars[name_start..i].iter().collect::<String>().to_ascii_lowercase();
    // The name has to end the tag or be followed by attributes — `a<b` is arithmetic, not markup.
    if i < chars.len() && !matches!(chars[i], '>' | '/' | ' ' | '\t' | '\n' | '\r') {
        return None;
    }
    let TagEnd { end, terminated, self_closing } = scan_to_gt(chars, i);
    Some((Markup::Tag { name, closing, self_closing, terminated }, end))
}

/// Find the `>` that closes a tag, stepping over quoted attribute values. Reports whether one was
/// there at all, and whether the tag closed itself (`<br/>`).
fn scan_to_gt(chars: &[char], mut i: usize) -> TagEnd {
    let mut quote: Option<char> = None;
    let mut last_solid = ' ';
    while i < chars.len() {
        match (quote, chars[i]) {
            (Some(q), c) if c == q => quote = None,
            (None, c @ ('"' | '\'')) => quote = Some(c),
            (None, '>') => return TagEnd { end: i + 1, terminated: true, self_closing: last_solid == '/' },
            _ => {}
        }
        if !chars[i].is_whitespace() {
            last_solid = chars[i];
        }
        i += 1;
    }
    TagEnd { end: chars.len(), terminated: false, self_closing: false }
}

/// Index just past `</name>`, or the end of input if it never closes.
fn skip_element(chars: &[char], from: usize, name: &str) -> usize {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '<' {
            if let Some((Markup::Tag { name: n, closing: true, .. }, next)) = parse_markup(chars, i) {
                if n == name {
                    return next;
                }
            }
        }
        i += 1;
    }
    chars.len()
}

/// Decode the entity at `start`, returning its text and the index just past the `;`.
fn parse_entity(chars: &[char], start: usize) -> Option<(String, usize)> {
    let end = (start + 1..chars.len().min(start + 34)).find(|&i| chars[i] == ';')?;
    let name: String = chars[start + 1..end].iter().collect();
    if name.is_empty() {
        return None;
    }
    let decoded = if let Some(digits) = name.strip_prefix('#') {
        let code = match digits.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => digits.parse::<u32>().ok()?,
        };
        // Decoding is not a licence to manufacture control characters: `&#7;` from a provider
        // would otherwise land in the TUI as a literal BEL. Anything unprintable stays verbatim.
        let c = char::from_u32(code)?;
        if c.is_control() && c != '\t' && c != '\n' {
            return None;
        }
        c.to_string()
    } else {
        let lower = name.to_ascii_lowercase();
        ENTITIES.iter().find(|(n, _)| *n == lower).map(|(_, t)| (*t).to_string())?
    };
    Some((decoded, end + 1))
}

/// Append one text character, collapsing insignificant whitespace outside `<pre>`. A no-break
/// space is significant by definition — it's how both Azure's editor and [`to_html`] spell an
/// indent — so it lands as a real space instead of collapsing into its neighbour.
fn push_char(out: &mut String, c: char, preformatted: bool) {
    if preformatted {
        out.push(c);
    } else if c == '\u{a0}' {
        out.push('\u{a0}');
    } else if c.is_whitespace() {
        if !(out.is_empty() || out.ends_with(' ') || out.ends_with('\n')) {
            out.push(' ');
        }
    } else {
        out.push(c);
    }
}

/// End the current line. A structural break — `</div><div>`, which is how Azure spells "next
/// line" — collapses into the previous one, so nesting doesn't become a wall of blank lines. An
/// explicit `<br>` is the author asking for space, so it may open one blank line (never more).
/// Only collapsible spaces are dropped; a hard one (still U+00A0 here) is content.
fn push_break(out: &mut String, explicit: bool) {
    while out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        return;
    }
    let limit = if explicit { 2 } else { 1 };
    if out.chars().rev().take_while(|&c| c == '\n').count() < limit {
        out.push('\n');
    }
}

/// Index just past the first occurrence of `seq` at or after `from`.
fn find_seq(chars: &[char], from: usize, seq: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(seq.len() - 1)).find(|&i| chars[i..i + seq.len()] == *seq).map(|i| i + seq.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_the_markup_azure_stores_a_description_as() {
        // Exactly the shape the work-item editor emits: a `<div>` per line, `&nbsp;` padding.
        let html = "<div>Login fails after&nbsp;the token expires.</div><div><br></div><div>Repro:</div>\
                    <ol><li>Sign in</li><li>Wait 1h</li></ol>";
        assert_eq!(to_text(html), "Login fails after the token expires.\n\nRepro:\n- Sign in\n- Wait 1h");
        assert!(!to_text(html).contains('<'));
    }

    #[test]
    fn decodes_entities_without_rescanning_them_as_tags() {
        // `&lt;p&gt;` is text the author escaped on purpose — it must survive as `<p>`, not vanish.
        assert_eq!(to_text("<p>Use &lt;p&gt; &amp; &quot;quotes&quot;</p>"), "Use <p> & \"quotes\"");
        assert_eq!(to_text("<p>&#65;&#x42;&hellip;</p>"), "AB…");
        // A control character is never decoded into the text a terminal will print.
        assert_eq!(to_text("<p>ring&#7;ring&#0;</p>"), "ring&#7;ring&#0;");
        // An unknown entity is prose, not markup, so it stays put.
        assert_eq!(to_text("<p>Tom &Jerry; &widget;</p>"), "Tom &Jerry; &widget;");
        // Escaped-only content carries no tag, but still has to be decoded rather than shown raw.
        assert_eq!(to_text("a &amp; b"), "a & b");
    }

    #[test]
    fn leaves_plain_text_alone() {
        // No HTML element in sight — `<` here is arithmetic and a generic, not a tag.
        for s in ["Bump when a < b", "Return Vec<String> instead", "Plain description.", ""] {
            assert_eq!(to_text(s), s);
        }
        // A lone tag-shaped run is not proof of markup: `<b …>` here is a comparison, and treating
        // it as a tag would swallow everything up to the `>`.
        assert_eq!(to_text("fails if x<b and y>c"), "fails if x<b and y>c");
        assert_eq!(to_text("truncate when len<max"), "truncate when len<max");
        // …whereas a closing tag, a void element or an entity is proof, and those are converted.
        assert_eq!(to_text("line one<br>line two"), "line one\nline two");
        assert_eq!(to_text("para one<br><br>para two"), "para one\n\npara two");
        // Structure alone never doubles, however deeply it nests.
        assert_eq!(to_text("<div><div><p>a</p></div></div><div>b</div>"), "a\nb");
        assert_eq!(to_text("<span>x</span>"), "x");
    }

    #[test]
    fn drops_scripts_styles_and_comments_entirely() {
        assert_eq!(to_text("<div>a<script>alert('x')</script><style>p{}</style><!-- note -->b</div>"), "ab");
    }

    #[test]
    fn keeps_preformatted_whitespace_and_separates_table_cells() {
        assert_eq!(to_text("<pre>fn main() {\n    ok();\n}</pre>"), "fn main() {\n    ok();\n}");
        assert_eq!(to_text("<table><tr><td>env</td><td>prod</td></tr></table>"), "env | prod");
    }

    #[test]
    fn treats_a_no_break_space_as_a_real_space() {
        // Azure's editor pads with `&nbsp;`; a single one is just a space between words…
        assert_eq!(to_text("<div>after&nbsp;the token</div>"), "after the token");
        // …but a run of them is indentation the author meant, so it survives rather than collapsing.
        assert_eq!(to_text("<div>&nbsp;&nbsp;&nbsp;&nbsp;indented</div>"), "indented");
        assert_eq!(to_text("<div>a&nbsp;&nbsp;&nbsp;b</div>"), "a   b");
    }

    #[test]
    fn round_trips_text_back_into_the_html_the_field_expects() {
        assert_eq!(to_html("a < b\nline two"), "a &lt; b<br>line two");
        assert_eq!(to_html("x\r\ny"), "x<br>y");
        // Editing a description must not quietly eat its indentation.
        let snippet = "steps:\n    - build\n    - deploy";
        assert_eq!(to_text(&to_html(snippet)), snippet);
        // What the reader saw is what the writer sends — typed markup is escaped, not honoured.
        assert_eq!(to_text(&to_html("<b>literal</b>")), "<b>literal</b>");
    }
}
