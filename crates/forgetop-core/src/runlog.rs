//! Reading a pipeline job's log text: per-step sections and the first error.
//! Shared by both frontends; pure functions over the text `PipelineSource::logs` returns.
//!
//! Three section dialects are understood, one per forge that marks its steps in the log:
//!
//! * GitHub Actions — `##[group]Name` … `##[endgroup]`. The group only wraps a step's header
//!   (the command it runs); the step's output follows the `##[endgroup]`. So a GitHub section
//!   runs from its `##[group]` to the next one, not to its own `##[endgroup]`.
//! * Azure Pipelines — `##[section]Starting: Name` … `##[section]Finishing: Name`.
//! * GitLab CI — raw traces mark `section_start:<unix>:<id>[opts]\r\x1b[0K<header>` …
//!   `section_end:<unix>:<id>\r\x1b[0K`; a nested section folds into its outermost one. (The
//!   GitLab provider rewrites its top-level sections to `##[group]<header>` before the log gets
//!   here and drops the inner ones, so this dialect is mostly for raw traces.)
//!
//! When the job's step names are known, [`parse_step_sections`] is the better reader: each step
//! claims the GitHub `##[group]` that best names it, in order, so groups a step opens in its own
//! output stay part of that step. And in a log with Azure markers, `##[group]` never
//! starts a section — Azure uses it for folding inside a step.
//!
//! Lines may carry a leading ISO-8601 timestamp (`2024-05-01T10:00:00.1234567Z `), which GitHub
//! and Azure both prepend; every matcher here looks past it.

use std::cmp::Reverse;

/// One step's stretch of a job log, as line indices into the text it was parsed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogSection {
    /// The step name the marker carries (GitLab: its header text, or its id without one).
    pub name: String,
    /// The line holding the opening marker.
    pub start: usize,
    /// One past the last line that belongs to the section (a closing marker included).
    pub end: usize,
    /// The job step this section was matched to, when [`parse_step_sections`] read it.
    pub step: Option<usize>,
}

impl LogSection {
    /// Lines in the section, its marker line included.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, line: usize) -> bool {
        line >= self.start && line < self.end
    }
}

/// What a line's first thing says, once its timestamp is out of the way.
enum Marker<'a> {
    Open(&'a str),
    /// An explicit close (Azure `Finishing`); GitHub's `##[endgroup]` is not one — see the
    /// module docs.
    Close,
}

/// `line` without a leading byte-order mark and ISO-8601 timestamp (`2024-05-01T10:00:00Z `,
/// fractional seconds optional). Anything else comes back unchanged.
pub fn strip_timestamp(line: &str) -> &str {
    let line = line.strip_prefix('\u{feff}').unwrap_or(line);
    let b = line.as_bytes();
    let digits = |r: std::ops::Range<usize>| r.clone().all(|i| b.get(i).is_some_and(u8::is_ascii_digit));
    let shape = b.len() >= 19
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19);
    if !shape {
        return line;
    }
    let mut i = 19;
    if b.get(i) == Some(&b'.') {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if b.get(i) == Some(&b'Z') {
        i += 1;
    }
    match b.get(i) {
        Some(b' ') => &line[i + 1..],
        None => "",
        _ => line,
    }
}

/// `s` without ANSI escape sequences (colours, GitLab's `\x1b[0K` erase) or carriage returns.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    // Parameters and intermediates, then one final byte in `@`..`~`.
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

fn marker(line: &str) -> Option<Marker<'_>> {
    let line = strip_timestamp(line);
    if let Some(name) = line.strip_prefix("##[group]") {
        return Some(Marker::Open(name.trim()));
    }
    if let Some(name) = line.strip_prefix("##[section]Starting: ") {
        return Some(Marker::Open(name.trim()));
    }
    if line.starts_with("##[section]Finishing: ") {
        return Some(Marker::Close);
    }
    None
}

/// One GitLab marker on a line, in the order they appear.
enum GitLab {
    Start { id: String, header: String },
    End { id: String },
}

/// Every GitLab `section_start` / `section_end` marker on a line. A trace can put an end and the
/// next start on one line, so this returns all of them.
fn gitlab_markers(line: &str) -> Vec<GitLab> {
    let mut out = Vec::new();
    let mut rest = line;
    loop {
        let start = rest.find("section_start:");
        let end = rest.find("section_end:");
        let (at, is_start) = match (start, end) {
            (Some(s), Some(e)) if s < e => (s, true),
            (Some(_), Some(e)) => (e, false),
            (Some(s), None) => (s, true),
            (None, Some(e)) => (e, false),
            (None, None) => break,
        };
        let body = &rest[at + if is_start { "section_start:".len() } else { "section_end:".len() }..];
        // `<unix>:<id>[opts]` up to the `\r`, `\x1b` or end of line.
        let spec_end = body.find(['\r', '\x1b', '\n']).unwrap_or(body.len());
        let spec = &body[..spec_end];
        let id = spec.split_once(':').map(|(_, id)| id).unwrap_or(spec);
        let id = id.split('[').next().unwrap_or(id).trim().to_string();
        let after = &body[spec_end..];
        // The next marker, if any, bounds this one's header text.
        let next = [after.find("section_start:"), after.find("section_end:")].into_iter().flatten().min();
        let header_raw = &after[..next.unwrap_or(after.len())];
        if is_start {
            out.push(GitLab::Start { id, header: strip_ansi(header_raw).trim().to_string() });
        } else {
            out.push(GitLab::End { id });
        }
        match next {
            Some(n) => rest = &after[n..],
            None => break,
        }
    }
    out
}

/// Splits a job log into its per-step sections, in order. Lines before the first marker (and,
/// for Azure, between a `Finishing` and the next `Starting`) belong to no section. A log with no
/// markers at all yields no sections, and callers show it as plain text.
pub fn parse_sections(lines: &[String]) -> Vec<LogSection> {
    parse_with(lines, true)
}

/// Like [`parse_sections`], using the job's `steps` (in the order they ran) to decide which
/// markers are step boundaries:
///
/// * a log with Azure `##[section]Starting:` markers is sectioned by those alone — its
///   `##[group]`s fold content inside a step;
/// * otherwise each step, in order, takes the best-matching group still to come: an exact name
///   (trimmed, case-insensitive, past GitHub's `Run `) over a loose one ([`find_section`]), and
///   a `Run …` step header over a group a step's own output opened — so a `cargo test --no-run`
///   group inside Build doesn't take the Test step's place. A step looks no further than the
///   first group that exactly names a later step. Every other group is content of the step
///   before it. When no group names any step the steps can't be told apart this way, and
///   every group is a section, as in [`parse_sections`].
///
/// Each section it returns carries the step it was matched to.
pub fn parse_step_sections(lines: &[String], steps: &[String]) -> Vec<LogSection> {
    let azure = lines.iter().any(|l| strip_timestamp(l).starts_with("##[section]Starting: "));
    let mut all = parse_with(lines, !azure);
    if steps.is_empty() {
        return all;
    }
    if azure {
        let mut next = 0;
        for sec in &mut all {
            if let Some(j) = (next..steps.len()).find(|&j| name_matches(&sec.name, &steps[j])) {
                sec.step = Some(j);
                next = j + 1;
            }
        }
        return all;
    }
    // GitHub: give each step, in order, the best of the groups still to come — looking only as
    // far as the first group that names a *later* step outright, which must be that step's.
    let score = |group: &str, step: &str| -> u8 {
        if name_matches_strictly(group, step) {
            3
        } else if name_matches(group, step) {
            // Every step header GitHub writes starts `Run ` (`Run cargo test`, `Run actions/…`);
            // a group opened by a step's own output usually doesn't.
            if group.trim_start().starts_with("Run ") { 2 } else { 1 }
        } else {
            0
        }
    };
    let mut owner: Vec<Option<usize>> = vec![None; all.len()];
    let mut from = 0;
    for (j, step) in steps.iter().enumerate() {
        let horizon = (from..all.len())
            .find(|&g| steps[j + 1..].iter().any(|later| name_matches_strictly(&all[g].name, later)))
            .unwrap_or(all.len());
        let best = (from..horizon).filter(|&g| score(&all[g].name, step) > 0).max_by_key(|&g| (score(&all[g].name, step), Reverse(g)));
        if let Some(g) = best {
            owner[g] = Some(j);
            from = g + 1;
        }
    }
    let mut kept: Vec<LogSection> = Vec::new();
    for (sec, step) in all.iter().zip(&owner) {
        match (step, kept.last_mut()) {
            (Some(j), _) => kept.push(LogSection { step: Some(*j), ..sec.clone() }),
            // Content of the step before: that section now runs on through this one.
            (None, Some(last)) if last.end == sec.start => last.end = sec.end,
            (None, _) => {}
        }
    }
    if kept.is_empty() {
        return all;
    }
    kept
}

/// Whether a section name names `step` outright: the same name, trimmed and case-insensitive,
/// also past GitHub's `Run ` prefix — no substring guessing.
fn name_matches_strictly(section: &str, step: &str) -> bool {
    let unrun = |s: &str| norm(s.trim_start().strip_prefix("Run ").unwrap_or(s));
    norm(section) == norm(step) || unrun(section) == unrun(step)
}

/// Whether a section name names `step`, by the passes [`find_section`] takes.
fn name_matches(section: &str, step: &str) -> bool {
    find_section(std::slice::from_ref(&LogSection { name: section.to_string(), start: 0, end: 0, step: None }), step).is_some()
}

fn parse_with(lines: &[String], groups: bool) -> Vec<LogSection> {
    let mut out: Vec<LogSection> = Vec::new();
    let mut open: Option<(String, usize)> = None;
    // GitLab nests sections; only the outermost one becomes a section here.
    let mut gitlab_stack: Vec<String> = Vec::new();

    fn begin(out: &mut Vec<LogSection>, open: &mut Option<(String, usize)>, name: &str, at: usize) {
        if let Some((prev, start)) = open.take() {
            out.push(LogSection { name: prev, start, end: at.max(start + 1), step: None });
        }
        // A section that closed on this very line (GitLab puts an end and the next start on one
        // line) gives the line up to the one opening here, so no line belongs to two sections.
        if let Some(last) = out.last_mut() {
            if last.end > at {
                if last.start < at {
                    last.end = at;
                } else {
                    out.pop();
                }
            }
        }
        let name = if name.is_empty() { "(unnamed step)" } else { name };
        *open = Some((name.to_string(), at));
    }

    for (i, raw) in lines.iter().enumerate() {
        if raw.contains("section_start:") || raw.contains("section_end:") {
            for m in gitlab_markers(raw) {
                match m {
                    GitLab::Start { id, header } => {
                        if gitlab_stack.is_empty() {
                            let name = if header.is_empty() { id.clone() } else { header };
                            begin(&mut out, &mut open, &name, i);
                        }
                        gitlab_stack.push(id);
                    }
                    GitLab::End { id } => {
                        if let Some(pos) = gitlab_stack.iter().rposition(|s| *s == id) {
                            gitlab_stack.truncate(pos);
                            if gitlab_stack.is_empty() {
                                if let Some((name, start)) = open.take() {
                                    out.push(LogSection { name, start, end: i + 1, step: None });
                                }
                            }
                        }
                    }
                }
            }
            continue;
        }
        match marker(raw) {
            Some(Marker::Open(name)) if groups || !strip_timestamp(raw).starts_with("##[group]") => {
                begin(&mut out, &mut open, name, i)
            }
            Some(Marker::Open(_)) => {}
            Some(Marker::Close) => {
                if let Some((name, start)) = open.take() {
                    out.push(LogSection { name, start, end: i + 1, step: None });
                }
            }
            None => {}
        }
    }
    if let Some((name, start)) = open {
        out.push(LogSection { name, start, end: lines.len().max(start + 1), step: None });
    }
    out
}

/// A marker line with nothing to read on it once sections are drawn as headers: GitHub's
/// `##[endgroup]`, Azure's `##[section]Finishing: …`, and a line holding only GitLab
/// `section_end` markers.
pub fn is_hidden_marker(line: &str) -> bool {
    let t = strip_timestamp(line);
    if t.starts_with("##[endgroup]") || t.starts_with("##[section]Finishing: ") {
        return true;
    }
    if t.contains("section_end:") && !t.contains("section_start:") {
        // Only markers and erase codes, no text of its own.
        let mut rest = strip_ansi(t);
        while let Some(at) = rest.find("section_end:") {
            let tail = &rest[at + "section_end:".len()..];
            let len = tail.find(char::is_whitespace).unwrap_or(tail.len());
            rest = format!("{}{}", &rest[..at], &tail[len..]);
        }
        return rest.trim().is_empty();
    }
    false
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// The section a step's log lives in, by name: exact, then trimmed and case-insensitive (also
/// past GitHub's `Run ` prefix), then either name containing the other. Azure names its sections
/// after the step's display name, so its steps match exactly; GitHub's `run:` steps are titled by
/// their command, which only the looser passes catch.
pub fn find_section(sections: &[LogSection], step: &str) -> Option<usize> {
    if let Some(i) = sections.iter().position(|s| s.name == step) {
        return Some(i);
    }
    let want = norm(step);
    if want.is_empty() {
        return None;
    }
    if let Some(i) = sections.iter().position(|s| norm(&s.name) == want) {
        return Some(i);
    }
    let unrun = |s: &str| norm(s.trim_start().strip_prefix("Run ").unwrap_or(s));
    if let Some(i) = sections.iter().position(|s| unrun(&s.name) == unrun(step)) {
        return Some(i);
    }
    if want.chars().count() >= 3 {
        if let Some(i) = sections.iter().position(|s| {
            let have = norm(&s.name);
            have.contains(&want) || (have.chars().count() >= 3 && want.contains(&have))
        }) {
            return Some(i);
        }
    }
    None
}

/// The section for the `index`th of a job's `steps`: the one [`parse_step_sections`] matched to
/// it, else by name ([`find_section`]), else by position when the log has exactly one section
/// per step.
pub fn step_section(sections: &[LogSection], steps: &[String], index: usize) -> Option<usize> {
    if let Some(i) = sections.iter().position(|s| s.step == Some(index)) {
        return Some(i);
    }
    let step = steps.get(index)?;
    find_section(sections, step).or_else(|| (sections.len() == steps.len()).then_some(index))
}

// ---- errors ----

/// Whether a log line reports a failure: CI annotations (`##[error]`), test-runner verdicts
/// (`[FAIL]`, `FAILED`), compiler diagnostics (`error:`, `error[E…]`, `ERROR`, `Error:`),
/// `npm ERR!`, and a non-zero exit code. Summary lines that count zero failures (`0 errors`,
/// `Failed: 0`, `exit code 0`) don't qualify.
pub fn is_error_line(line: &str) -> bool {
    if line.contains("##[error]") || line.contains("[FAIL]") || line.contains("npm ERR!") {
        return true;
    }
    if has_token(line, "FAILED", |_| true) || has_token(line, "FAIL", |_| true) || has_token(line, "ERROR", |_| true) {
        return true;
    }
    // Title-case verdicts (`Failed!`, `Tests Failed: 3`) — but not prose like `Failed to retry`.
    if has_token(line, "Failed", |next| matches!(next, Some('!' | ':'))) {
        return true;
    }
    if has_token(line, "error", |next| matches!(next, Some(':' | '['))) || has_token(line, "Error", |next| next == Some(':')) {
        return true;
    }
    nonzero_exit_code(line)
}

/// `word` as a whole token (not inside a longer identifier) whose following character passes
/// `next_ok`, and which isn't a zero count (`0 FAILED`, `ERROR: 0`).
fn has_token(line: &str, word: &str, next_ok: impl Fn(Option<char>) -> bool) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(word).any(|(i, _)| {
        let before = line[..i].chars().next_back();
        let rest = &line[i + word.len()..];
        let after = rest.chars().next();
        !before.is_some_and(is_word)
            && !after.is_some_and(is_word)
            && next_ok(after)
            && !ends_with_zero(&line[..i])
            && !starts_with_zero(rest)
    })
}

/// `text` ends in the number 0 (ignoring trailing spaces), e.g. the `0 ` of `0 FAILED`.
fn ends_with_zero(text: &str) -> bool {
    let t = text.trim_end();
    t.ends_with('0') && !t[..t.len() - 1].ends_with(|c: char| c.is_ascii_digit())
}

/// `text` is a count of 0 after optional `:`/`=`/spaces, e.g. the `: 0` of `ERROR: 0`.
fn starts_with_zero(text: &str) -> bool {
    let t = text.trim_start_matches([':', '=', ' ']);
    t.starts_with('0') && !t[1..].starts_with(|c: char| c.is_ascii_digit())
}

/// `exit code N` / `exited with code N` (any case) with N other than 0.
fn nonzero_exit_code(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    ["exit code", "exited with code"].iter().any(|pat| {
        lower.match_indices(pat).any(|(i, _)| {
            let rest = lower[i + pat.len()..].trim_start_matches([':', ' ']);
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            !digits.is_empty() && !digits.trim_start_matches('0').is_empty()
        })
    })
}

/// Index of the first line [`is_error_line`] matches.
pub fn first_error_line(lines: &[String]) -> Option<usize> {
    lines.iter().position(|l| is_error_line(l))
}

/// What a failed log says went wrong, as far as it can be read: the failing test, where
/// (`path:line`), and the message. Every part is optional; [`failure_summary`] returns `None`
/// only when nothing at all was recognised.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FailureSummary {
    /// The line the summary was read from — where "jump to the error" should land.
    pub line: usize,
    pub test: Option<String>,
    /// `path:line`, as the log wrote the path.
    pub location: Option<String>,
    pub message: Option<String>,
}

/// `path:line[:col][:]` → `path:line`, when `s` looks like one.
fn location_of(s: &str) -> Option<String> {
    let s = s.trim().trim_end_matches(':');
    let mut parts = s.rsplitn(3, ':');
    let last = parts.next()?;
    let mid = parts.next()?;
    let is_num = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit());
    match parts.next() {
        // path:line:col
        Some(path) if is_num(last) && is_num(mid) && !path.is_empty() => Some(format!("{path}:{mid}")),
        // path:line (the rsplit took the path's own last segment as `mid`; rejoin it)
        Some(path) if is_num(last) && !path.is_empty() => Some(format!("{path}:{mid}:{last}")),
        None if is_num(last) && !mid.is_empty() => Some(format!("{mid}:{last}")),
        _ => None,
    }
}

/// The last `::` segment of a test path (`cache::tests::put` → `put`).
fn short_test(name: &str) -> String {
    name.rsplit("::").next().unwrap_or(name).trim().to_string()
}

/// A `path:line` shortened to the file's own name (`crates/core/src/cache.rs:212` →
/// `cache.rs:212`), for one-line banners.
pub fn short_location(location: &str) -> String {
    location.rsplit(['/', '\\']).next().unwrap_or(location).to_string()
}

/// Reads the failure out of a job log. Recognised, strongest first: a Rust panic (`thread 't'
/// panicked at path:line:col:` with its message on the next line, or the older one-line form),
/// a compiler diagnostic (`error[E…]: msg` with its `--> path:line:col`), a test runner's
/// `test name ... FAILED`, a generic `error: msg`, and a CI `##[error]msg`.
pub fn failure_summary(lines: &[String]) -> Option<FailureSummary> {
    let clean: Vec<String> = lines.iter().map(|l| strip_ansi(strip_timestamp(l))).collect();
    let next_text = |from: usize| clean.iter().skip(from).take(3).map(|l| l.trim()).find(|l| !l.is_empty());
    // The `--> path:line:col` a diagnostic points at, within its next few lines.
    let arrow = |from: usize| {
        clean.iter().skip(from + 1).take(6).find_map(|l| l.trim_start().strip_prefix("--> ").and_then(location_of))
    };

    let mut failed_test: Option<(usize, String)> = None;
    let mut panic: Option<FailureSummary> = None;
    let mut diagnostic: Option<FailureSummary> = None;
    let mut generic: Option<FailureSummary> = None;
    let mut ci: Option<FailureSummary> = None;

    for (i, line) in clean.iter().enumerate() {
        let t = line.trim();
        if failed_test.is_none() {
            if let Some(rest) = t.strip_prefix("test ") {
                if let Some(name) = rest.strip_suffix(" ... FAILED") {
                    failed_test = Some((i, short_test(name)));
                }
            }
        }
        if panic.is_none() {
            if let Some(at) = t.find("panicked at ") {
                let thread = t[..at]
                    .trim()
                    .strip_prefix("thread '")
                    .and_then(|s| s.strip_suffix('\''))
                    .map(short_test)
                    .filter(|n| n != "main");
                let rest = t[at + "panicked at ".len()..].trim();
                let (location, message) = if rest.is_empty() {
                    // Wrapped: the location sits on the next line, the message after it.
                    let loc = clean.get(i + 1).and_then(|l| location_of(l));
                    let msg = if loc.is_some() { next_text(i + 2) } else { next_text(i + 1) };
                    (loc, msg.map(str::to_string))
                } else if let Some(stripped) = rest.strip_prefix('\'') {
                    // Old form: `panicked at 'message', path:line:col`.
                    match stripped.rsplit_once("', ") {
                        Some((msg, loc)) => (location_of(loc), Some(msg.to_string())),
                        None => (None, Some(stripped.trim_end_matches('\'').to_string())),
                    }
                } else {
                    (location_of(rest), next_text(i + 1).map(str::to_string))
                };
                panic = Some(FailureSummary { line: i, test: thread, location, message });
            }
        }
        if diagnostic.is_none() && t.starts_with("error[") {
            if let Some((_, msg)) = t.split_once("]: ") {
                diagnostic = Some(FailureSummary {
                    line: i,
                    test: None,
                    location: arrow(i),
                    message: Some(msg.trim().to_string()),
                });
            }
        }
        if generic.is_none() {
            if let Some(msg) = t.strip_prefix("error: ") {
                generic = Some(FailureSummary { line: i, test: None, location: arrow(i), message: Some(msg.trim().to_string()) });
            }
        }
        if ci.is_none() {
            if let Some(at) = t.find("##[error]") {
                let msg = t[at + "##[error]".len()..].trim();
                ci = Some(FailureSummary { line: i, test: None, location: None, message: Some(msg.to_string()) });
            }
        }
    }

    let test_name = failed_test.as_ref().map(|(_, n)| n.clone());
    if let Some(mut p) = panic {
        if p.test.is_none() {
            p.test = test_name;
        }
        return Some(p);
    }
    if let Some(d) = diagnostic {
        return Some(d);
    }
    if let Some((line, name)) = failed_test {
        return Some(FailureSummary { line, test: Some(name), location: None, message: None });
    }
    generic.or(ci)
}

// ---- durations ----

/// The median of `values` (the mean of the middle two for an even count, rounded down), or
/// `None` for none.
pub fn median(values: &[i64]) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_unstable();
    let mid = v.len() / 2;
    Some(if v.len().is_multiple_of(2) { (v[mid - 1] + v[mid]).div_euclid(2) } else { v[mid] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    fn names(sections: &[LogSection]) -> Vec<&str> {
        sections.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn timestamps_and_the_bom_are_stripped_and_nothing_else_is() {
        assert_eq!(strip_timestamp("2024-05-01T10:00:00.1234567Z ##[group]Build"), "##[group]Build");
        assert_eq!(strip_timestamp("\u{feff}2024-05-01T10:00:00Z hello"), "hello");
        assert_eq!(strip_timestamp("2024-05-01T10:00:00.5Z"), "");
        assert_eq!(strip_timestamp("2024-05-01 not a stamp"), "2024-05-01 not a stamp");
        assert_eq!(strip_timestamp("plain"), "plain");
        assert_eq!(strip_timestamp(""), "");
        assert_eq!(strip_ansi("\x1b[32;1mok\x1b[0m\r"), "ok");
    }

    #[test]
    fn github_groups_run_to_the_next_group_because_output_follows_the_endgroup() {
        let log = lines(
            "2024-05-01T10:00:00.0000000Z Current runner version: '2.3'\n\
             2024-05-01T10:00:01.0000000Z ##[group]Run actions/checkout@v4\n\
             2024-05-01T10:00:01.1000000Z with: x\n\
             2024-05-01T10:00:01.2000000Z ##[endgroup]\n\
             2024-05-01T10:00:02.0000000Z Syncing repository\n\
             2024-05-01T10:00:03.0000000Z ##[group]Run cargo test\n\
             2024-05-01T10:00:03.1000000Z ##[endgroup]\n\
             2024-05-01T10:00:04.0000000Z test a ... FAILED",
        );
        let s = parse_sections(&log);
        assert_eq!(names(&s), vec!["Run actions/checkout@v4", "Run cargo test"]);
        assert_eq!((s[0].start, s[0].end), (1, 5), "the output after endgroup is the step's");
        assert_eq!((s[1].start, s[1].end), (5, 8), "the last section runs to the end");
        assert!(!s[0].contains(0), "the preamble belongs to no section");
        assert!(is_hidden_marker(&log[3]));
        assert!(!is_hidden_marker(&log[4]));
    }

    #[test]
    fn azure_sections_close_on_finishing() {
        let log = lines(
            "##[section]Starting: Initialize job\n\
             Agent name: x\n\
             ##[section]Finishing: Initialize job\n\
             between\n\
             2024-05-01T10:00:00.1234567Z ##[section]Starting: dotnet test\n\
             ##[error]boom\n\
             2024-05-01T10:00:09.1234567Z ##[section]Finishing: dotnet test",
        );
        let s = parse_sections(&log);
        assert_eq!(names(&s), vec!["Initialize job", "dotnet test"]);
        assert_eq!((s[0].start, s[0].end), (0, 3));
        assert!(!s.iter().any(|sec| sec.contains(3)), "a line between sections belongs to neither");
        assert_eq!((s[1].start, s[1].end), (4, 7));
        assert!(is_hidden_marker(&log[2]));
    }

    #[test]
    fn gitlab_sections_use_their_header_and_fold_nested_ones() {
        let log = lines(
            "Running with gitlab-runner 16\n\
             section_start:1700000000:prepare_script\r\x1b[0K\x1b[0;36mPreparing environment\x1b[0m\n\
             Running on runner-1\n\
             section_end:1700000001:prepare_script\r\x1b[0K\n\
             section_start:1700000002:step_script[collapsed=true]\r\x1b[0KExecuting \"step_script\" stage\n\
             section_start:1700000003:inner\r\x1b[0Knested\n\
             $ cargo test\n\
             section_end:1700000004:inner\r\x1b[0K\n\
             error: boom\n\
             section_end:1700000005:step_script\r\x1b[0Ksection_start:1700000006:cleanup\r\x1b[0K\n\
             bye\n\
             section_end:1700000007:cleanup\r\x1b[0K",
        );
        let s = parse_sections(&log);
        assert_eq!(names(&s), vec!["Preparing environment", "Executing \"step_script\" stage", "cleanup"]);
        assert_eq!((s[0].start, s[0].end), (1, 4));
        assert_eq!((s[1].start, s[1].end), (4, 9), "the nested section folds into its parent");
        assert_eq!((s[2].start, s[2].end), (9, 12), "an end and a start on one line don't overlap");
        assert!(is_hidden_marker(&log[3]));
        assert!(!is_hidden_marker(&log[9]), "a line that also opens a section is its header");
    }

    #[test]
    fn a_log_without_markers_has_no_sections() {
        assert!(parse_sections(&lines("one\ntwo")).is_empty());
        assert!(parse_sections(&[]).is_empty());
        // An unclosed Azure section runs to the end of what has arrived so far.
        let s = parse_sections(&lines("##[section]Starting: Build\nstill going"));
        assert_eq!((s[0].start, s[0].end), (0, 2));
    }

    #[test]
    fn steps_find_their_sections_by_name_then_loosely_then_by_position() {
        let s = parse_sections(&lines("##[group]Set up job\n##[group]Run actions/checkout@v4\n##[group]Run cargo test --workspace\n##[group]Clippy"));
        assert_eq!(find_section(&s, "Set up job"), Some(0), "exact");
        assert_eq!(find_section(&s, "run ACTIONS/checkout@v4"), Some(1), "case-insensitive");
        assert_eq!(find_section(&s, "actions/checkout@v4"), Some(1), "past GitHub's Run prefix");
        assert_eq!(find_section(&s, "cargo test"), Some(2), "contains");
        assert_eq!(find_section(&s, "clippy"), Some(3));
        assert_eq!(find_section(&s, "Deploy"), None);
        assert_eq!(find_section(&s, ""), None);
        let steps: Vec<String> = ["a", "b", "Unit suite", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(step_section(&s, &steps, 2), Some(2), "one section per step: by position");
        assert_eq!(step_section(&s, &steps[..3], 2), None, "counts differ: no guess");
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn with_known_steps_a_group_inside_a_steps_output_stays_part_of_that_step() {
        let log = lines(
            "##[group]Run actions/checkout@v4\n\
             ##[endgroup]\n\
             ##[group]Run cargo test\n\
             ##[endgroup]\n\
             ##[group]Fetching crates\n\
             inner output\n\
             ##[endgroup]\n\
             test a ... ok\n\
             ##[group]Run cargo clippy\n\
             ok",
        );
        // Checkout's group is `Run actions/checkout@v4`: matched loosely; `Fetching crates` names no
        // step, so it is Test's output.
        let s = parse_step_sections(&log, &strings(&["Set up job", "actions/checkout@v4", "cargo test", "Clippy"]));
        assert_eq!(names(&s), vec!["Run actions/checkout@v4", "Run cargo test", "Run cargo clippy"]);
        assert_eq!((s[1].start, s[1].end, s[1].step), (2, 8, Some(2)), "the nested group is Test's");
        assert_eq!(s[2].step, Some(3));
        assert_eq!(step_section(&s, &strings(&["Set up job", "actions/checkout@v4", "cargo test", "Clippy"]), 2), Some(1));
        // A step matching nothing in order is skipped, not guessed.
        assert_eq!(parse_sections(&log).len(), 4, "without steps every group is a section");
        // No group names any step: every group stays a section.
        assert_eq!(parse_step_sections(&log, &strings(&["alpha", "beta"])).len(), 4);
    }

    #[test]
    fn a_nested_group_that_loosely_names_a_later_step_does_not_take_its_place() {
        let log = lines(
            "##[group]Run cargo build\n\
             ##[endgroup]\n\
             ##[group]cargo test --no-run\n\
             compiled test binaries\n\
             ##[endgroup]\n\
             ##[group]Run cargo test\n\
             test a ... ok",
        );
        let s = parse_step_sections(&log, &strings(&["cargo build", "Test"]));
        assert_eq!(names(&s), vec!["Run cargo build", "Run cargo test"], "the nested group stays Build's");
        assert_eq!((s[0].start, s[0].end, s[0].step), (0, 5, Some(0)));
        assert_eq!((s[1].start, s[1].step), (5, Some(1)));
        // Skipping a step that logged no group takes an exact name.
        let s = parse_step_sections(&log, &strings(&["Set up job", "cargo build", "cargo test"]));
        assert_eq!(s.iter().map(|x| x.step).collect::<Vec<_>>(), vec![Some(1), Some(2)]);
    }

    #[test]
    fn a_log_with_azure_sections_ignores_groups_for_sectioning() {
        let log = lines(
            "##[section]Starting: Build\n\
             ##[group]Restore\n\
             restored\n\
             ##[endgroup]\n\
             ##[section]Finishing: Build\n\
             ##[section]Starting: Test\n\
             ##[error]boom\n\
             ##[section]Finishing: Test",
        );
        let s = parse_step_sections(&log, &strings(&["Build", "Test"]));
        assert_eq!(names(&s), vec!["Build", "Test"]);
        assert_eq!((s[0].start, s[0].end, s[0].step), (0, 5, Some(0)));
        assert_eq!(s[1].step, Some(1));
    }

    #[test]
    fn a_rust_panic_names_the_test_the_file_and_the_message() {
        let log = lines(
            "running 184 tests\n\
             test cache::tests::rewrite_edits_in_place ... FAILED\n\
             failures:\n\
             ---- cache::tests::rewrite_edits_in_place stdout ----\n\
             2024-05-01T10:00:00Z thread 'cache::tests::rewrite_edits_in_place' panicked at crates/forgetop-core/src/cache.rs:212:9:\n\
             assertion `left == right` failed\n\
             ##[error]Process completed with exit code 101.",
        );
        let f = failure_summary(&log).expect("summary");
        assert_eq!(f.test.as_deref(), Some("rewrite_edits_in_place"));
        assert_eq!(f.location.as_deref(), Some("crates/forgetop-core/src/cache.rs:212"));
        assert_eq!(f.message.as_deref(), Some("assertion `left == right` failed"));
        assert_eq!(f.line, 4);
        assert_eq!(short_location(f.location.as_deref().unwrap()), "cache.rs:212");

        // The old one-line form, on the main thread: the test comes from the FAILED line.
        let old = lines("test it_works ... FAILED\nthread 'main' panicked at 'boom', src/lib.rs:3:5");
        let f = failure_summary(&old).unwrap();
        assert_eq!((f.test.as_deref(), f.location.as_deref(), f.message.as_deref()), (Some("it_works"), Some("src/lib.rs:3"), Some("boom")));
    }

    #[test]
    fn diagnostics_generic_errors_and_ci_errors_are_read_in_that_order() {
        let rustc = lines("   Compiling x\nerror[E0308]: mismatched types\n  --> src/main.rs:4:18\n   |\nerror: could not compile `x`");
        let f = failure_summary(&rustc).unwrap();
        assert_eq!((f.location.as_deref(), f.message.as_deref(), f.line), (Some("src/main.rs:4"), Some("mismatched types"), 1));

        let generic = lines("error: linking with `cc` failed\n##[error]Process completed with exit code 1.");
        assert_eq!(failure_summary(&generic).unwrap().message.as_deref(), Some("linking with `cc` failed"));

        let ci = lines("ok\n2024-05-01T10:00:00Z ##[error]Process completed with exit code 2.");
        let f = failure_summary(&ci).unwrap();
        assert_eq!((f.message.as_deref(), f.line), (Some("Process completed with exit code 2."), 1));

        let only_test = lines("test a::b ... FAILED");
        assert_eq!(failure_summary(&only_test).unwrap().test.as_deref(), Some("b"));

        assert_eq!(failure_summary(&lines("all good\n0 errors")), None);
        assert_eq!(failure_summary(&[]), None);
    }

    #[test]
    fn error_lines_are_matched_past_zero_counts() {
        assert!(is_error_line("##[error]boom"));
        assert!(is_error_line("Process completed with exit code 2."));
        assert!(!is_error_line("Process completed with exit code 0."));
        assert!(!is_error_line("Failed: 0, Passed: 12"));
        assert_eq!(first_error_line(&lines("ok\nerror: x")), Some(1));
    }

    #[test]
    fn median_is_the_middle_value_or_the_mean_of_the_middle_two() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[7]), Some(7));
        assert_eq!(median(&[98, 95, 101]), Some(98));
        assert_eq!(median(&[10, 40, 20, 30]), Some(25));
        assert_eq!(median(&[1, 2]), Some(1), "rounded down");
    }
}
