import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { prStateLine } from "./format";
import type { CheckStatus, MergeableState, PullRequest, PullRequestStatus, ReviewVote } from "./types";

interface Case {
  name: string;
  status: PullRequestStatus;
  is_draft: boolean;
  checks: CheckStatus;
  mergeable: MergeableState;
  votes: [string, ReviewVote][];
  summary: PullRequest["check_summary"];
  expect: string;
}

// The judgement lives in forgetop-core, but the sentence is built twice — here and in the TUI's
// `ui::pr_state_line` — because the two frontends share no runtime. AGENTS.md forbids a logic
// fork between them, so both suites read these cases. See testdata/README.md.
const cases: Case[] = JSON.parse(readFileSync(resolve(__dirname, "../../../../testdata/pr_state_cases.json"), "utf8"));

const base = (c: Case): PullRequest => ({
  id: "1",
  number: 1,
  title: "t",
  description: null,
  author: { id: "me", display_name: "Me", handle: "me", avatar_url: null },
  status: c.status,
  is_draft: c.is_draft,
  source_ref: "feat",
  target_ref: "main",
  reviewers: c.votes.map(([name, vote]) => ({
    user: { id: name, display_name: name, handle: name, avatar_url: null },
    vote,
    is_required: false,
  })),
  labels: [],
  checks: c.checks,
  check_summary: c.summary,
  mergeable: c.mergeable,
  changed_files: 0,
  additions: 0,
  deletions: 0,
  created_at: null,
  updated_at: null,
  url: null,
});

describe("prStateLine", () => {
  it.each(cases.map((c) => [c.name, c] as const))("%s", (_name, c) => {
    const { icon, text } = prStateLine(base(c));
    // Wording, not spacing: the terminal pads its glyph, this panel uses a CSS gap.
    expect(`${icon} ${text}`.split(/\s+/).join(" ")).toBe(c.expect);
  });

  it("covers every case in the shared fixture", () => {
    expect(cases.length).toBeGreaterThanOrEqual(10);
  });
});
