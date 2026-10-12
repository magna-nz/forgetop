// Reading a pipeline job's log text: per-step sections and the first error. A line-by-line port
// of crates/forgetop-core/src/runlog.rs (plus the TUI LogView's layout in
// crates/forgetop-tui/src/app.rs) so the dashboard folds and reads a log exactly as the terminal
// UI does. Keep the two in step: AGENTS.md forbids a logic fork between the frontends.
//
// Three section dialects are understood, one per forge that marks its steps in the log:
//
// * GitHub Actions — `##[group]Name` … `##[endgroup]`. The group only wraps a step's header; the
//   step's output follows the `##[endgroup]`, so a GitHub section runs to the next `##[group]`.
// * Azure Pipelines — `##[section]Starting: Name` … `##[section]Finishing: Name`.
// * GitLab CI — `section_start:<unix>:<id>[opts]\r\x1b[0K<header>` … `section_end:<unix>:<id>`;
//   a nested section folds into its outermost one.
//
// Lines may carry a leading ISO-8601 timestamp, which every matcher here looks past.
//
// Indices are UTF-16 code-unit offsets where Rust's are byte offsets; every slice is taken
// against the same string it was found in, so the results agree.

/** One step's stretch of a job log, as line indices into the text it was parsed from. */
export interface LogSection {
  /** The step name the marker carries (GitLab: its header text, or its id without one). */
  name: string;
  /** The line holding the opening marker. */
  start: number;
  /** One past the last line that belongs to the section (a closing marker included). */
  end: number;
  /** The job step this section was matched to, when `parseStepSections` read it. */
  step: number | null;
}

export function sectionContains(s: LogSection, line: number): boolean {
  return line >= s.start && line < s.end;
}

/** What a failed log says went wrong, as far as it can be read. Every part is optional. */
export interface FailureSummary {
  /** The line the summary was read from — where "jump to the error" should land. */
  line: number;
  test: string | null;
  /** `path:line`, as the log wrote the path. */
  location: string | null;
  message: string | null;
}

/** Most log lines a pane keeps; a longer log keeps its tail. Mirrors the TUI's `LOG_MAX_LINES`. */
export const LOG_MAX_LINES = 10_000;

/** Rust's `str::lines`: split on `\n`, a `\r` before it dropped, no empty line for a trailing
 *  newline. */
export function textLines(text: string): string[] {
  if (text === "") return [];
  const out = text.split("\n");
  if (out[out.length - 1] === "") out.pop();
  return out.map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l));
}

/** A fetched log as the pane holds it (the TUI's `LogView::set_text` + `cap_tail`): its lines,
 *  `(no logs returned)` for an empty one, capped to the last `LOG_MAX_LINES`; `dropped` is how
 *  many head lines the cap removed. */
export function logLines(text: string, max: number = LOG_MAX_LINES): { lines: string[]; dropped: number } {
  const lines = text.trim() === "" ? ["(no logs returned)"] : textLines(text);
  const dropped = Math.max(0, lines.length - max);
  return { lines: dropped > 0 ? lines.slice(dropped) : lines, dropped };
}

const isDigit = (c: string | undefined) => c !== undefined && c >= "0" && c <= "9";

/** `line` without a leading byte-order mark and ISO-8601 timestamp (`2024-05-01T10:00:00Z `,
 *  fractional seconds optional). Anything else comes back unchanged. */
export function stripTimestamp(input: string): string {
  const line = input.startsWith("﻿") ? input.slice(1) : input;
  const b = line;
  const digits = (from: number, to: number) => {
    for (let i = from; i < to; i++) if (!isDigit(b[i])) return false;
    return true;
  };
  const shape =
    b.length >= 19 &&
    digits(0, 4) &&
    b[4] === "-" &&
    digits(5, 7) &&
    b[7] === "-" &&
    digits(8, 10) &&
    b[10] === "T" &&
    digits(11, 13) &&
    b[13] === ":" &&
    digits(14, 16) &&
    b[16] === ":" &&
    digits(17, 19);
  if (!shape) return line;
  let i = 19;
  if (b[i] === ".") {
    i += 1;
    while (isDigit(b[i])) i += 1;
  }
  if (b[i] === "Z") i += 1;
  if (b[i] === " ") return line.slice(i + 1);
  if (i >= b.length) return "";
  return line;
}

/** `s` without ANSI escape sequences (colours, GitLab's `\x1b[0K` erase) or carriage returns. */
export function stripAnsi(s: string): string {
  const chars = Array.from(s);
  let out = "";
  let k = 0;
  while (k < chars.length) {
    const c = chars[k++];
    if (c === "\x1b") {
      if (chars[k] === "[") {
        k++;
        // Parameters and intermediates, then one final byte in `@`..`~`.
        while (k < chars.length) {
          const p = chars[k++];
          if (p >= "@" && p <= "~" && p.length === 1) break;
        }
      } else {
        k++;
      }
    } else if (c !== "\r") {
      out += c;
    }
  }
  return out;
}

type Marker = { kind: "open"; name: string } | { kind: "close" };

function marker(raw: string): Marker | null {
  const line = stripTimestamp(raw);
  if (line.startsWith("##[group]")) return { kind: "open", name: line.slice("##[group]".length).trim() };
  if (line.startsWith("##[section]Starting: ")) return { kind: "open", name: line.slice("##[section]Starting: ".length).trim() };
  if (line.startsWith("##[section]Finishing: ")) return { kind: "close" };
  return null;
}

type GitLab = { kind: "start"; id: string; header: string } | { kind: "end"; id: string };

const findIdx = (s: string, needle: string): number | null => {
  const i = s.indexOf(needle);
  return i < 0 ? null : i;
};

/** Every GitLab `section_start` / `section_end` marker on a line, in order. */
function gitlabMarkers(line: string): GitLab[] {
  const out: GitLab[] = [];
  let rest = line;
  for (;;) {
    const start = findIdx(rest, "section_start:");
    const end = findIdx(rest, "section_end:");
    let at: number;
    let isStart: boolean;
    if (start !== null && end !== null) {
      if (start < end) [at, isStart] = [start, true];
      else [at, isStart] = [end, false];
    } else if (start !== null) [at, isStart] = [start, true];
    else if (end !== null) [at, isStart] = [end, false];
    else break;
    const body = rest.slice(at + (isStart ? "section_start:".length : "section_end:".length));
    // `<unix>:<id>[opts]` up to the `\r`, `\x1b` or end of line.
    const m = /[\r\x1b\n]/.exec(body);
    const specEnd = m ? m.index : body.length;
    const spec = body.slice(0, specEnd);
    const colon = spec.indexOf(":");
    let id = colon >= 0 ? spec.slice(colon + 1) : spec;
    id = id.split("[")[0].trim();
    const after = body.slice(specEnd);
    const candidates = [findIdx(after, "section_start:"), findIdx(after, "section_end:")].filter((x): x is number => x !== null);
    const next = candidates.length ? Math.min(...candidates) : null;
    const headerRaw = after.slice(0, next ?? after.length);
    if (isStart) out.push({ kind: "start", id, header: stripAnsi(headerRaw).trim() });
    else out.push({ kind: "end", id });
    if (next === null) break;
    rest = after.slice(next);
  }
  return out;
}

/** Splits a job log into its per-step sections, in order. Lines before the first marker (and,
 *  for Azure, between a `Finishing` and the next `Starting`) belong to no section. */
export function parseSections(lines: string[]): LogSection[] {
  return parseWith(lines, true);
}

/** Like `parseSections`, using the job's `steps` (in the order they ran) to decide which markers
 *  are step boundaries. Each section it returns carries the step it was matched to. */
export function parseStepSections(lines: string[], steps: string[]): LogSection[] {
  const azure = lines.some((l) => stripTimestamp(l).startsWith("##[section]Starting: "));
  const all = parseWith(lines, !azure);
  if (steps.length === 0) return all;
  if (azure) {
    let next = 0;
    for (const sec of all) {
      for (let j = next; j < steps.length; j++) {
        if (nameMatches(sec.name, steps[j])) {
          sec.step = j;
          next = j + 1;
          break;
        }
      }
    }
    return all;
  }
  // GitHub: give each step, in order, the best of the groups still to come — looking only as far
  // as the first group that names a *later* step outright, which must be that step's.
  const score = (group: string, step: string): number => {
    if (nameMatchesStrictly(group, step)) return 3;
    if (nameMatches(group, step)) return group.trimStart().startsWith("Run ") ? 2 : 1;
    return 0;
  };
  const owner: (number | null)[] = all.map(() => null);
  let from = 0;
  steps.forEach((step, j) => {
    let horizon = all.length;
    for (let g = from; g < all.length; g++) {
      if (steps.slice(j + 1).some((later) => nameMatchesStrictly(all[g].name, later))) {
        horizon = g;
        break;
      }
    }
    // Highest score; among equal scores the lowest index (Rust's `max_by_key` with `Reverse(g)`).
    let best: number | null = null;
    let bestScore = 0;
    for (let g = from; g < horizon; g++) {
      const sc = score(all[g].name, step);
      if (sc > bestScore) {
        best = g;
        bestScore = sc;
      }
    }
    if (best !== null) {
      owner[best] = j;
      from = best + 1;
    }
  });
  const kept: LogSection[] = [];
  all.forEach((sec, i) => {
    const step = owner[i];
    const last = kept[kept.length - 1];
    if (step !== null) kept.push({ ...sec, step });
    // Content of the step before: that section now runs on through this one.
    else if (last && last.end === sec.start) last.end = sec.end;
  });
  if (kept.length === 0) return all;
  return kept;
}

const unrun = (s: string) => {
  const t = s.trimStart();
  return norm(t.startsWith("Run ") ? t.slice(4) : s);
};

/** Whether a section name names `step` outright: the same name, trimmed and case-insensitive,
 *  also past GitHub's `Run ` prefix — no substring guessing. */
function nameMatchesStrictly(section: string, step: string): boolean {
  return norm(section) === norm(step) || unrun(section) === unrun(step);
}

/** Whether a section name names `step`, by the passes `findSection` takes. */
function nameMatches(section: string, step: string): boolean {
  return findSection([{ name: section, start: 0, end: 0, step: null }], step) !== null;
}

function parseWith(lines: string[], groups: boolean): LogSection[] {
  const out: LogSection[] = [];
  let open: { name: string; start: number } | null = null;
  // GitLab nests sections; only the outermost one becomes a section here.
  let gitlabStack: string[] = [];

  const begin = (name: string, at: number) => {
    if (open) {
      out.push({ name: open.name, start: open.start, end: Math.max(at, open.start + 1), step: null });
      open = null;
    }
    // A section that closed on this very line gives the line up to the one opening here.
    const last = out[out.length - 1];
    if (last && last.end > at) {
      if (last.start < at) last.end = at;
      else out.pop();
    }
    open = { name: name === "" ? "(unnamed step)" : name, start: at };
  };

  lines.forEach((raw, i) => {
    if (raw.includes("section_start:") || raw.includes("section_end:")) {
      for (const m of gitlabMarkers(raw)) {
        if (m.kind === "start") {
          if (gitlabStack.length === 0) begin(m.header === "" ? m.id : m.header, i);
          gitlabStack.push(m.id);
        } else {
          const pos = gitlabStack.lastIndexOf(m.id);
          if (pos >= 0) {
            gitlabStack = gitlabStack.slice(0, pos);
            if (gitlabStack.length === 0 && open) {
              out.push({ name: open.name, start: open.start, end: i + 1, step: null });
              open = null;
            }
          }
        }
      }
      return;
    }
    const mk = marker(raw);
    if (mk?.kind === "open") {
      if (groups || !stripTimestamp(raw).startsWith("##[group]")) begin(mk.name, i);
    } else if (mk?.kind === "close") {
      if (open) {
        out.push({ name: open.name, start: open.start, end: i + 1, step: null });
        open = null;
      }
    }
  });
  const tail = open as { name: string; start: number } | null;
  if (tail) out.push({ name: tail.name, start: tail.start, end: Math.max(lines.length, tail.start + 1), step: null });
  return out;
}

/** A marker line with nothing to read on it once sections are drawn as headers: GitHub's
 *  `##[endgroup]`, Azure's `##[section]Finishing: …`, and a line holding only GitLab
 *  `section_end` markers. */
export function isHiddenMarker(line: string): boolean {
  const t = stripTimestamp(line);
  if (t.startsWith("##[endgroup]") || t.startsWith("##[section]Finishing: ")) return true;
  if (t.includes("section_end:") && !t.includes("section_start:")) {
    // Only markers and erase codes, no text of its own.
    let rest = stripAnsi(t);
    for (;;) {
      const at = rest.indexOf("section_end:");
      if (at < 0) break;
      const tail = rest.slice(at + "section_end:".length);
      const ws = /\s/.exec(tail);
      const len = ws ? ws.index : tail.length;
      rest = rest.slice(0, at) + tail.slice(len);
    }
    return rest.trim() === "";
  }
  return false;
}

function norm(s: string): string {
  return s.trim().toLowerCase();
}

const charCount = (s: string) => Array.from(s).length;

/** The section a step's log lives in, by name: exact, then trimmed and case-insensitive (also
 *  past GitHub's `Run ` prefix), then either name containing the other. */
export function findSection(sections: LogSection[], step: string): number | null {
  let i = sections.findIndex((s) => s.name === step);
  if (i >= 0) return i;
  const want = norm(step);
  if (want === "") return null;
  i = sections.findIndex((s) => norm(s.name) === want);
  if (i >= 0) return i;
  i = sections.findIndex((s) => unrun(s.name) === unrun(step));
  if (i >= 0) return i;
  if (charCount(want) >= 3) {
    i = sections.findIndex((s) => {
      const have = norm(s.name);
      return have.includes(want) || (charCount(have) >= 3 && want.includes(have));
    });
    if (i >= 0) return i;
  }
  return null;
}

/** The section for the `index`th of a job's `steps`: the one `parseStepSections` matched to it,
 *  else by name, else by position when the log has exactly one section per step. */
export function stepSection(sections: LogSection[], steps: string[], index: number): number | null {
  const i = sections.findIndex((s) => s.step === index);
  if (i >= 0) return i;
  if (index < 0 || index >= steps.length) return null;
  return findSection(sections, steps[index]) ?? (sections.length === steps.length ? index : null);
}

// ---- errors ----

/** Whether a log line reports a failure: CI annotations (`##[error]`), test-runner verdicts
 *  (`[FAIL]`, `FAILED`), compiler diagnostics (`error:`, `error[E…]`, `ERROR`, `Error:`),
 *  `npm ERR!`, and a non-zero exit code. Summary lines that count zero failures don't qualify. */
export function isErrorLine(line: string): boolean {
  if (line.includes("##[error]") || line.includes("[FAIL]") || line.includes("npm ERR!")) return true;
  const any = () => true;
  if (hasToken(line, "FAILED", any) || hasToken(line, "FAIL", any) || hasToken(line, "ERROR", any)) return true;
  // Title-case verdicts (`Failed!`, `Tests Failed: 3`) — but not prose like `Failed to retry`.
  if (hasToken(line, "Failed", (next) => next === "!" || next === ":")) return true;
  if (hasToken(line, "error", (next) => next === ":" || next === "[") || hasToken(line, "Error", (next) => next === ":")) return true;
  return nonzeroExitCode(line);
}

// Rust's `char::is_alphanumeric` is the Unicode Alphabetic property or a numeric category.
const WORD_CHAR = /^[\p{Alphabetic}\p{N}_]$/u;
const isWord = (c: string | undefined) => c !== undefined && WORD_CHAR.test(c);

/** The code point ending just before `i`. */
function charBefore(s: string, i: number): string | undefined {
  if (i <= 0) return undefined;
  const lo = s.charCodeAt(i - 1);
  if (lo >= 0xdc00 && lo <= 0xdfff && i >= 2) {
    const hi = s.charCodeAt(i - 2);
    if (hi >= 0xd800 && hi <= 0xdbff) return s.slice(i - 2, i);
  }
  return s[i - 1];
}

/** The code point starting at `i`. */
function charAt(s: string, i: number): string | undefined {
  if (i >= s.length) return undefined;
  const cp = s.codePointAt(i);
  return cp === undefined ? undefined : String.fromCodePoint(cp);
}

/** Start offsets of `word` in `line`, non-overlapping (Rust's `match_indices`). */
function matchIndices(line: string, word: string): number[] {
  const out: number[] = [];
  let from = 0;
  for (;;) {
    const i = line.indexOf(word, from);
    if (i < 0) return out;
    out.push(i);
    from = i + word.length;
  }
}

/** `word` as a whole token whose following character passes `nextOk`, and which isn't a zero
 *  count (`0 FAILED`, `ERROR: 0`). */
function hasToken(line: string, word: string, nextOk: (next: string | undefined) => boolean): boolean {
  return matchIndices(line, word).some((i) => {
    const before = charBefore(line, i);
    const rest = line.slice(i + word.length);
    const after = charAt(rest, 0);
    return !isWord(before) && !isWord(after) && nextOk(after) && !endsWithZero(line.slice(0, i)) && !startsWithZero(rest);
  });
}

/** `text` ends in the number 0 (ignoring trailing spaces), e.g. the `0 ` of `0 FAILED`. */
function endsWithZero(text: string): boolean {
  const t = text.trimEnd();
  return t.endsWith("0") && !isDigit(t[t.length - 2]);
}

/** `text` is a count of 0 after optional `:`/`=`/spaces, e.g. the `: 0` of `ERROR: 0`. */
function startsWithZero(text: string): boolean {
  const t = text.replace(/^[:= ]+/, "");
  return t.startsWith("0") && !isDigit(t[1]);
}

const asciiLower = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());

/** `exit code N` / `exited with code N` (any case) with N other than 0. */
function nonzeroExitCode(line: string): boolean {
  const lower = asciiLower(line);
  return ["exit code", "exited with code"].some((pat) =>
    matchIndices(lower, pat).some((i) => {
      const rest = lower.slice(i + pat.length).replace(/^[: ]+/, "");
      const digits = /^[0-9]*/.exec(rest)![0];
      return digits !== "" && digits.replace(/^0+/, "") !== "";
    }),
  );
}

/** Index of the first line `isErrorLine` matches. */
export function firstErrorLine(lines: string[]): number | null {
  const i = lines.findIndex((l) => isErrorLine(l));
  return i < 0 ? null : i;
}

/** `path:line[:col][:]` → `path:line`, when `s` looks like one. */
function locationOf(input: string): string | null {
  const s = input.trim().replace(/:+$/, "");
  // `rsplitn(3, ':')`: the last two segments, then everything before them.
  const i1 = s.lastIndexOf(":");
  if (i1 < 0) return null;
  const last = s.slice(i1 + 1);
  const head = s.slice(0, i1);
  const i2 = head.lastIndexOf(":");
  const mid = i2 < 0 ? head : head.slice(i2 + 1);
  const path = i2 < 0 ? null : head.slice(0, i2);
  const isNum = (p: string) => p !== "" && /^[0-9]+$/.test(p);
  // path:line:col
  if (path !== null && isNum(last) && isNum(mid) && path !== "") return `${path}:${mid}`;
  // path:line (the split took the path's own last segment as `mid`; rejoin it)
  if (path !== null && isNum(last) && path !== "") return `${path}:${mid}:${last}`;
  if (path === null && isNum(last) && mid !== "") return `${mid}:${last}`;
  return null;
}

/** The last `::` segment of a test path (`cache::tests::put` → `put`). */
function shortTest(name: string): string {
  const parts = name.split("::");
  return parts[parts.length - 1].trim();
}

/** A `path:line` shortened to the file's own name (`crates/core/src/cache.rs:212` →
 *  `cache.rs:212`), for one-line banners. */
export function shortLocation(location: string): string {
  const parts = location.split(/[/\\]/);
  return parts[parts.length - 1];
}

/** Reads the failure out of a job log. Recognised, strongest first: a Rust panic, a compiler
 *  diagnostic (`error[E…]: msg` with its `--> path:line:col`), a test runner's
 *  `test name ... FAILED`, a generic `error: msg`, and a CI `##[error]msg`. */
export function failureSummary(lines: string[]): FailureSummary | null {
  const clean = lines.map((l) => stripAnsi(stripTimestamp(l)));
  const nextText = (from: number): string | null => {
    for (const l of clean.slice(from, from + 3)) {
      const t = l.trim();
      if (t !== "") return t;
    }
    return null;
  };
  // The `--> path:line:col` a diagnostic points at, within its next few lines.
  const arrow = (from: number): string | null => {
    for (const l of clean.slice(from + 1, from + 7)) {
      const t = l.trimStart();
      if (t.startsWith("--> ")) {
        const loc = locationOf(t.slice(4));
        if (loc !== null) return loc;
      }
    }
    return null;
  };

  let failedTest: { line: number; name: string } | null = null;
  let panic: FailureSummary | null = null;
  let diagnostic: FailureSummary | null = null;
  let generic: FailureSummary | null = null;
  let ci: FailureSummary | null = null;

  for (let i = 0; i < clean.length; i++) {
    const t = clean[i].trim();
    if (failedTest === null && t.startsWith("test ")) {
      const rest = t.slice(5);
      if (rest.endsWith(" ... FAILED")) failedTest = { line: i, name: shortTest(rest.slice(0, -" ... FAILED".length)) };
    }
    if (panic === null) {
      const at = t.indexOf("panicked at ");
      if (at >= 0) {
        const pre = t.slice(0, at).trim();
        let thread: string | null = null;
        if (pre.startsWith("thread '")) {
          const s = pre.slice("thread '".length);
          if (s.endsWith("'")) {
            const n = shortTest(s.slice(0, -1));
            if (n !== "main") thread = n;
          }
        }
        const rest = t.slice(at + "panicked at ".length).trim();
        let location: string | null;
        let message: string | null;
        if (rest === "") {
          // Wrapped: the location sits on the next line, the message after it.
          location = i + 1 < clean.length ? locationOf(clean[i + 1]) : null;
          message = location !== null ? nextText(i + 2) : nextText(i + 1);
        } else if (rest.startsWith("'")) {
          // Old form: `panicked at 'message', path:line:col`.
          const stripped = rest.slice(1);
          const k = stripped.lastIndexOf("', ");
          if (k >= 0) {
            location = locationOf(stripped.slice(k + 3));
            message = stripped.slice(0, k);
          } else {
            location = null;
            message = stripped.replace(/'+$/, "");
          }
        } else {
          location = locationOf(rest);
          message = nextText(i + 1);
        }
        panic = { line: i, test: thread, location, message };
      }
    }
    if (diagnostic === null && t.startsWith("error[")) {
      const k = t.indexOf("]: ");
      if (k >= 0) diagnostic = { line: i, test: null, location: arrow(i), message: t.slice(k + 3).trim() };
    }
    if (generic === null && t.startsWith("error: ")) {
      generic = { line: i, test: null, location: arrow(i), message: t.slice("error: ".length).trim() };
    }
    if (ci === null) {
      const at = t.indexOf("##[error]");
      if (at >= 0) ci = { line: i, test: null, location: null, message: t.slice(at + "##[error]".length).trim() };
    }
  }

  if (panic) return { ...panic, test: panic.test ?? failedTest?.name ?? null };
  if (diagnostic) return diagnostic;
  if (failedTest) return { line: failedTest.line, test: failedTest.name, location: null, message: null };
  return generic ?? ci;
}

// ---- durations ----

/** The median of `values` (the mean of the middle two for an even count, rounded down), or
 *  `null` for none. */
export function median(values: number[]): number | null {
  if (values.length === 0) return null;
  const v = [...values].sort((a, b) => a - b);
  const mid = Math.floor(v.length / 2);
  return v.length % 2 === 0 ? Math.floor((v[mid - 1] + v[mid]) / 2) : v[mid];
}

// ---- the log pane's layout (the TUI's `LogView`) ----

/** One drawn row of a sectioned log: a line, a section's header, or two or more consecutive
 *  folded sections of passed steps drawn as one header. Mirrors the TUI's `LogRow`. */
export type LogRow = { kind: "line"; line: number } | { kind: "header"; section: number } | { kind: "group"; first: number; last: number };

/** Whether section `s` is a step that passed (the TUI's `LogView::section_passed`). */
export function sectionPassed(sections: LogSection[], stepPassed: boolean[], s: number): boolean {
  const step = sections[s]?.step;
  return step !== null && step !== undefined ? stepPassed[step] ?? false : false;
}

/** The section holding `line`, if any. */
export function sectionOfLine(sections: LogSection[], line: number): number | null {
  const i = sections.findIndex((s) => sectionContains(s, line));
  return i < 0 ? null : i;
}

/** The rows a log draws, from its lines, sections and open folds (the TUI's
 *  `LogView::relayout`). An unsectioned log is every line; a sectioned one draws each section as
 *  a header, its lines only when open, and hidden marker lines never. */
export function layoutLog({
  lines,
  sections,
  open,
  stepPassed,
}: {
  lines: string[];
  sections: LogSection[];
  open: Set<number>;
  stepPassed: boolean[];
}): LogRow[] {
  const rows: LogRow[] = [];
  if (sections.length === 0) {
    for (let l = 0; l < lines.length; l++) rows.push({ kind: "line", line: l });
    return rows;
  }
  const foldedPass = (s: number) => !open.has(s) && sectionPassed(sections, stepPassed, s);
  let line = 0;
  let next = 0;
  while (line < lines.length) {
    while (next < sections.length && sections[next].start < line) next += 1;
    let last = next;
    while (
      next < sections.length &&
      sections[next].start === line &&
      foldedPass(last) &&
      last + 1 < sections.length &&
      foldedPass(last + 1) &&
      sections[last + 1].start === sections[last].end
    ) {
      last += 1;
    }
    if (last > next) {
      rows.push({ kind: "group", first: next, last });
      line = Math.max(sections[last].end, line + 1);
      next = last + 1;
      continue;
    }
    const sec = next < sections.length && sections[next].start === line ? sections[next] : null;
    if (sec) {
      rows.push({ kind: "header", section: next });
      if (open.has(next)) {
        for (let l = sec.start + 1; l < Math.min(sec.end, lines.length); l++) {
          if (!isHiddenMarker(lines[l])) rows.push({ kind: "line", line: l });
        }
      }
      line = Math.max(sec.end, line + 1);
      next += 1;
    } else {
      if (!isHiddenMarker(lines[line])) rows.push({ kind: "line", line });
      line += 1;
    }
  }
  return rows;
}

/** The section to leave unfolded first: the step the pane was opened for, else the one holding
 *  the first error, else — for a live job — the latest (the TUI's `LogView::initial_section`). */
export function initialSection({
  sections,
  steps,
  targetStep,
  firstError,
  live,
}: {
  sections: LogSection[];
  steps: string[];
  targetStep: number | null;
  firstError: number | null;
  live: boolean;
}): number | null {
  if (targetStep !== null) {
    const s = stepSection(sections, steps, targetStep);
    if (s !== null) return s;
  }
  if (firstError !== null) {
    const s = sectionOfLine(sections, firstError);
    if (s !== null) return s;
  }
  return live && sections.length > 0 ? sections.length - 1 : null;
}
