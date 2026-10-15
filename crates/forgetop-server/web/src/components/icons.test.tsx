import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Icon, type IconName } from "./icons";
import {
  checkMeta,
  notificationMeta,
  pipeMeta,
  prSignalMeta,
  prStatusMeta,
  timelineMeta,
  voteMeta,
  wiStateGlyph,
} from "../format";
import type { CheckStatus, NotificationKind, PipelineRunStatus, PullRequest, ReviewVote, TimelineEventKind } from "../types";

const NAMES: IconName[] = [
  "inbox", "pull-request", "circle-dot", "workflow", "settings", "search", "refresh", "bell", "sidebar",
  "chevron-down", "chevron-right", "external", "x", "plus", "check", "comment", "at", "arrow-right", "user",
  "user-plus", "copy", "link", "dots", "pencil", "shield", "logs", "rerun", "square", "minus-circle", "git-branch", "open-pane",
  "pr-open", "pr-draft", "pr-merged", "pr-closed", "check-circle", "x-circle", "running", "queued", "clock",
  "warning", "half", "circle", "blocked", "skipped", "dot", "review", "smile",
];

describe("Icon", () => {
  it("draws every name as a 16-unit stroke path", () => {
    for (const name of NAMES) {
      const { container, unmount } = render(<Icon name={name} />);
      const svg = container.querySelector("svg")!;
      expect(svg, name).toHaveAttribute("viewBox", "0 0 16 16");
      expect(svg.innerHTML.length, name).toBeGreaterThan(10);
      unmount();
    }
  });
});

const pr = (over: Partial<PullRequest>): PullRequest =>
  ({
    id: "1",
    title: "t",
    status: "Open",
    is_draft: false,
    checks: "Passed",
    reviewers: [],
    ...over,
  }) as unknown as PullRequest;

describe("every status meta names a glyph the icon set draws", () => {
  const known = new Set<string>(NAMES);
  it("pull requests, checks, votes, pipelines, work items", () => {
    for (const status of ["Open", "Merged", "Closed", "Draft"] as const) expect(known.has(prStatusMeta(pr({ status })).glyph)).toBe(true);
    expect(prStatusMeta(pr({ is_draft: true })).glyph).toBe("pr-draft");
    for (const c of ["Passed", "Failed", "Pending", "None"] as CheckStatus[]) expect(known.has(checkMeta(c).glyph)).toBe(true);
    for (const v of ["Approved", "ApprovedWithSuggestions", "Rejected", "WaitingForAuthor", "NoVote"] as ReviewVote[]) expect(known.has(voteMeta(v).glyph)).toBe(true);
    for (const s of ["Succeeded", "Running", "Queued", "Failed", "PartiallySucceeded", "Canceled", "Waiting", "Skipped"] as PipelineRunStatus[])
      expect(known.has(pipeMeta(s).glyph), s).toBe(true);
    expect(wiStateGlyph("Blocked", "Started")).toBe("blocked");
    expect(wiStateGlyph("Done", "Completed")).toBe("check-circle");
    expect(wiStateGlyph("In Progress", "Started")).toBe("half");
    expect(wiStateGlyph("In Review", "Review")).toBe("review");
    expect(wiStateGlyph("Todo", "Unstarted")).toBe("circle");
    expect(known.has(prSignalMeta(pr({ checks: "Failed" })).glyph)).toBe(true);
  });

  it("notifications and timeline events", () => {
    for (const k of ["ReviewRequested", "Mention", "Assigned", "CiFailed", "Comment", "StateChange", "NewItem", "Due", "Reaction", "Other"] as NotificationKind[])
      expect(known.has(notificationMeta(k).glyph), k).toBe(true);
    for (const k of ["Approved", "ChangesRequested", "Reviewed", "Commented", "Merged", "Closed", "Reopened", "StateChanged", "Assigned", "Labeled", "Committed"] as TimelineEventKind[])
      expect(known.has(timelineMeta(k).glyph), k).toBe(true);
  });

  it("running and changes-requested take the design's colours", () => {
    expect(checkMeta("Pending").color).toBe("var(--blue)");
    expect(voteMeta("Rejected").color).toBe("var(--yellow)");
    expect(notificationMeta("ReviewRequested").color).not.toBe("var(--accent)");
  });
});
