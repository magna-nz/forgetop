// Vectors ported from the Rust tests in crates/forgetop-core/src/runlog.rs — the two frontends
// read logs through two implementations, so they must pass the same cases.
import { describe, expect, it } from "vitest";
import {
  failureSummary,
  findSection,
  firstErrorLine,
  initialSection,
  isErrorLine,
  isHiddenMarker,
  layoutLog,
  logLines,
  median,
  parseSections,
  parseStepSections,
  sectionContains,
  shortLocation,
  stepSection,
  stripAnsi,
  stripTimestamp,
  textLines,
  type LogSection,
} from "./runlog";

const lines = (...ls: string[]) => textLines(ls.join("\n"));
const names = (s: LogSection[]) => s.map((x) => x.name);
const span = (s: LogSection) => [s.start, s.end];

describe("runlog", () => {
  it("timestamps and the BOM are stripped and nothing else is", () => {
    expect(stripTimestamp("2024-05-01T10:00:00.1234567Z ##[group]Build")).toBe("##[group]Build");
    expect(stripTimestamp("﻿2024-05-01T10:00:00Z hello")).toBe("hello");
    expect(stripTimestamp("2024-05-01T10:00:00.5Z")).toBe("");
    expect(stripTimestamp("2024-05-01 not a stamp")).toBe("2024-05-01 not a stamp");
    expect(stripTimestamp("plain")).toBe("plain");
    expect(stripTimestamp("")).toBe("");
    expect(stripAnsi("\x1b[32;1mok\x1b[0m\r")).toBe("ok");
  });

  it("GitHub groups run to the next group because output follows the endgroup", () => {
    const log = lines(
      "2024-05-01T10:00:00.0000000Z Current runner version: '2.3'",
      "2024-05-01T10:00:01.0000000Z ##[group]Run actions/checkout@v4",
      "2024-05-01T10:00:01.1000000Z with: x",
      "2024-05-01T10:00:01.2000000Z ##[endgroup]",
      "2024-05-01T10:00:02.0000000Z Syncing repository",
      "2024-05-01T10:00:03.0000000Z ##[group]Run cargo test",
      "2024-05-01T10:00:03.1000000Z ##[endgroup]",
      "2024-05-01T10:00:04.0000000Z test a ... FAILED",
    );
    const s = parseSections(log);
    expect(names(s)).toEqual(["Run actions/checkout@v4", "Run cargo test"]);
    expect(span(s[0])).toEqual([1, 5]);
    expect(span(s[1])).toEqual([5, 8]);
    expect(sectionContains(s[0], 0)).toBe(false);
    expect(isHiddenMarker(log[3])).toBe(true);
    expect(isHiddenMarker(log[4])).toBe(false);
  });

  it("Azure sections close on Finishing", () => {
    const log = lines(
      "##[section]Starting: Initialize job",
      "Agent name: x",
      "##[section]Finishing: Initialize job",
      "between",
      "2024-05-01T10:00:00.1234567Z ##[section]Starting: dotnet test",
      "##[error]boom",
      "2024-05-01T10:00:09.1234567Z ##[section]Finishing: dotnet test",
    );
    const s = parseSections(log);
    expect(names(s)).toEqual(["Initialize job", "dotnet test"]);
    expect(span(s[0])).toEqual([0, 3]);
    expect(s.some((sec) => sectionContains(sec, 3))).toBe(false);
    expect(span(s[1])).toEqual([4, 7]);
    expect(isHiddenMarker(log[2])).toBe(true);
  });

  it("GitLab sections use their header and fold nested ones", () => {
    const log = lines(
      "Running with gitlab-runner 16",
      "section_start:1700000000:prepare_script\r\x1b[0K\x1b[0;36mPreparing environment\x1b[0m",
      "Running on runner-1",
      "section_end:1700000001:prepare_script\r\x1b[0K",
      'section_start:1700000002:step_script[collapsed=true]\r\x1b[0KExecuting "step_script" stage',
      "section_start:1700000003:inner\r\x1b[0Knested",
      "$ cargo test",
      "section_end:1700000004:inner\r\x1b[0K",
      "error: boom",
      "section_end:1700000005:step_script\r\x1b[0Ksection_start:1700000006:cleanup\r\x1b[0K",
      "bye",
      "section_end:1700000007:cleanup\r\x1b[0K",
    );
    const s = parseSections(log);
    expect(names(s)).toEqual(["Preparing environment", 'Executing "step_script" stage', "cleanup"]);
    expect(span(s[0])).toEqual([1, 4]);
    expect(span(s[1])).toEqual([4, 9]);
    expect(span(s[2])).toEqual([9, 12]);
    expect(isHiddenMarker(log[3])).toBe(true);
    expect(isHiddenMarker(log[9])).toBe(false);
  });

  it("a log without markers has no sections", () => {
    expect(parseSections(lines("one", "two"))).toEqual([]);
    expect(parseSections([])).toEqual([]);
    const s = parseSections(lines("##[section]Starting: Build", "still going"));
    expect(span(s[0])).toEqual([0, 2]);
  });

  it("steps find their sections by name, then loosely, then by position", () => {
    const s = parseSections(lines("##[group]Set up job", "##[group]Run actions/checkout@v4", "##[group]Run cargo test --workspace", "##[group]Clippy"));
    expect(findSection(s, "Set up job")).toBe(0);
    expect(findSection(s, "run ACTIONS/checkout@v4")).toBe(1);
    expect(findSection(s, "actions/checkout@v4")).toBe(1);
    expect(findSection(s, "cargo test")).toBe(2);
    expect(findSection(s, "clippy")).toBe(3);
    expect(findSection(s, "Deploy")).toBeNull();
    expect(findSection(s, "")).toBeNull();
    const steps = ["a", "b", "Unit suite", "d"];
    expect(stepSection(s, steps, 2)).toBe(2);
    expect(stepSection(s, steps.slice(0, 3), 2)).toBeNull();
  });

  it("with known steps a group inside a step's output stays part of that step", () => {
    const log = lines(
      "##[group]Run actions/checkout@v4",
      "##[endgroup]",
      "##[group]Run cargo test",
      "##[endgroup]",
      "##[group]Fetching crates",
      "inner output",
      "##[endgroup]",
      "test a ... ok",
      "##[group]Run cargo clippy",
      "ok",
    );
    const steps = ["Set up job", "actions/checkout@v4", "cargo test", "Clippy"];
    const s = parseStepSections(log, steps);
    expect(names(s)).toEqual(["Run actions/checkout@v4", "Run cargo test", "Run cargo clippy"]);
    expect([s[1].start, s[1].end, s[1].step]).toEqual([2, 8, 2]);
    expect(s[2].step).toBe(3);
    expect(stepSection(s, steps, 2)).toBe(1);
    expect(parseSections(log)).toHaveLength(4);
    expect(parseStepSections(log, ["alpha", "beta"])).toHaveLength(4);
  });

  it("a nested group that loosely names a later step does not take its place", () => {
    const log = lines(
      "##[group]Run cargo build",
      "##[endgroup]",
      "##[group]cargo test --no-run",
      "compiled test binaries",
      "##[endgroup]",
      "##[group]Run cargo test",
      "test a ... ok",
    );
    let s = parseStepSections(log, ["cargo build", "Test"]);
    expect(names(s)).toEqual(["Run cargo build", "Run cargo test"]);
    expect([s[0].start, s[0].end, s[0].step]).toEqual([0, 5, 0]);
    expect([s[1].start, s[1].step]).toEqual([5, 1]);
    s = parseStepSections(log, ["Set up job", "cargo build", "cargo test"]);
    expect(s.map((x) => x.step)).toEqual([1, 2]);
  });

  it("a log with Azure sections ignores groups for sectioning", () => {
    const log = lines(
      "##[section]Starting: Build",
      "##[group]Restore",
      "restored",
      "##[endgroup]",
      "##[section]Finishing: Build",
      "##[section]Starting: Test",
      "##[error]boom",
      "##[section]Finishing: Test",
    );
    const s = parseStepSections(log, ["Build", "Test"]);
    expect(names(s)).toEqual(["Build", "Test"]);
    expect([s[0].start, s[0].end, s[0].step]).toEqual([0, 5, 0]);
    expect(s[1].step).toBe(1);
  });

  it("a Rust panic names the test, the file and the message", () => {
    const log = lines(
      "running 184 tests",
      "test cache::tests::rewrite_edits_in_place ... FAILED",
      "failures:",
      "---- cache::tests::rewrite_edits_in_place stdout ----",
      "2024-05-01T10:00:00Z thread 'cache::tests::rewrite_edits_in_place' panicked at crates/forgetop-core/src/cache.rs:212:9:",
      "assertion `left == right` failed",
      "##[error]Process completed with exit code 101.",
    );
    const f = failureSummary(log)!;
    expect(f.test).toBe("rewrite_edits_in_place");
    expect(f.location).toBe("crates/forgetop-core/src/cache.rs:212");
    expect(f.message).toBe("assertion `left == right` failed");
    expect(f.line).toBe(4);
    expect(shortLocation(f.location!)).toBe("cache.rs:212");

    const old = failureSummary(lines("test it_works ... FAILED", "thread 'main' panicked at 'boom', src/lib.rs:3:5"))!;
    expect([old.test, old.location, old.message]).toEqual(["it_works", "src/lib.rs:3", "boom"]);
  });

  it("diagnostics, generic errors and CI errors are read in that order", () => {
    const rustc = failureSummary(lines("   Compiling x", "error[E0308]: mismatched types", "  --> src/main.rs:4:18", "   |", "error: could not compile `x`"))!;
    expect([rustc.location, rustc.message, rustc.line]).toEqual(["src/main.rs:4", "mismatched types", 1]);

    const generic = failureSummary(lines("error: linking with `cc` failed", "##[error]Process completed with exit code 1."))!;
    expect(generic.message).toBe("linking with `cc` failed");

    const ci = failureSummary(lines("ok", "2024-05-01T10:00:00Z ##[error]Process completed with exit code 2."))!;
    expect([ci.message, ci.line]).toEqual(["Process completed with exit code 2.", 1]);

    expect(failureSummary(lines("test a::b ... FAILED"))!.test).toBe("b");
    expect(failureSummary(lines("all good", "0 errors"))).toBeNull();
    expect(failureSummary([])).toBeNull();
  });

  it("error lines are matched past zero counts", () => {
    expect(isErrorLine("##[error]boom")).toBe(true);
    expect(isErrorLine("Process completed with exit code 2.")).toBe(true);
    expect(isErrorLine("Process completed with exit code 0.")).toBe(false);
    expect(isErrorLine("Failed: 0, Passed: 12")).toBe(false);
    expect(firstErrorLine(lines("ok", "error: x"))).toBe(1);
    // Unicode-aware word boundaries, as Rust's `is_alphanumeric`.
    expect(isErrorLine("éFAILED")).toBe(false);
    expect(isErrorLine("Tests Failed: 3")).toBe(true);
    expect(isErrorLine("Failed to retry")).toBe(false);
  });

  it("median is the middle value or the mean of the middle two", () => {
    expect(median([])).toBeNull();
    expect(median([7])).toBe(7);
    expect(median([98, 95, 101])).toBe(98);
    expect(median([10, 40, 20, 30])).toBe(25);
    expect(median([1, 2])).toBe(1);
  });
});

describe("log pane layout", () => {
  const log = lines(
    "=== logs for run r500 · job-r500 ===",
    "##[group]Set up job",
    "##[endgroup]",
    "##[group]Checkout",
    "##[endgroup]",
    "##[group]Test",
    "test a ... FAILED",
    "##[endgroup]",
  );
  const steps = ["Set up job", "Checkout", "Test"];
  const sections = parseStepSections(log, steps);
  const stepPassed = [true, true, false];

  it("folded passed steps merge into one group row, the failed one opens", () => {
    const first = initialSection({ sections, steps, targetStep: 2, firstError: firstErrorLine(log), live: false });
    expect(first).toBe(2);
    const rows = layoutLog({ lines: log, sections, open: new Set([first!]), stepPassed });
    expect(rows).toEqual([
      { kind: "line", line: 0 },
      { kind: "group", first: 0, last: 1 },
      { kind: "header", section: 2 },
      { kind: "line", line: 6 },
    ]);
  });

  it("an open passed section breaks the group, and an unsectioned log is every line", () => {
    const rows = layoutLog({ lines: log, sections, open: new Set([0]), stepPassed });
    expect(rows.slice(1, 3)).toEqual([
      { kind: "header", section: 0 },
      { kind: "header", section: 1 },
    ]);
    expect(layoutLog({ lines: ["a", "b"], sections: [], open: new Set(), stepPassed: [] })).toHaveLength(2);
  });

  it("lands on the first error's section, else the latest when live", () => {
    expect(initialSection({ sections, steps, targetStep: null, firstError: 6, live: false })).toBe(2);
    expect(initialSection({ sections, steps, targetStep: null, firstError: null, live: true })).toBe(2);
    expect(initialSection({ sections, steps, targetStep: null, firstError: null, live: false })).toBeNull();
  });

  it("caps a log to its tail and names an empty one", () => {
    expect(logLines("  \n")).toEqual({ lines: ["(no logs returned)"], dropped: 0 });
    expect(logLines("a\nb\nc\n", 2)).toEqual({ lines: ["b", "c"], dropped: 1 });
  });
});
