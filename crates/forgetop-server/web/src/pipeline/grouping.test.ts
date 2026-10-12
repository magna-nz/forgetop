// Ported from the TUI's grouping tests (crates/forgetop-tui/src/app.rs `pipe_lines_*`, ui.rs
// `pipelines_list_groups_by_pipeline_and_lands_collapsed`) and forgetop-core's shown-status tests.
import { describe, expect, it } from "vitest";
import type { PipelineRun, PipelineRunStatus, PipelineStage, PipeRow } from "../types";
import { gateStage, groupPipelines, nextGroupMode, parseGroupMode, pipeTriggerToken, shownStatus, stageStrip } from "./grouping";

function groupedRow(def: string, repo: string, branch: string, sha: string, at: string, status: PipelineRunStatus): PipeRow {
  return {
    connection_id: "c",
    connection: "GH",
    provider: "GitHub",
    definition_name: def,
    approvals: [],
    run: {
      id: `${def}-${sha}`,
      repository: repo,
      definition_id: def,
      number: 1,
      name: null,
      title: null,
      status,
      branch,
      commit_sha: sha || null,
      started_at: at || null,
      finished_at: null,
      stages: [],
    },
  };
}

const fannedOut = () => [
  groupedRow("CI", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", "Succeeded"),
  groupedRow("Integration", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", "Failed"),
  groupedRow("Release", "nz/app", "v1.0", "bbb", "2026-09-24T09:00:00Z", "Succeeded"),
  groupedRow("CI", "nz/app", "v1.0", "bbb", "2026-09-24T09:00:00Z", "Succeeded"),
];

describe("pipeline grouping", () => {
  it("cycles and parses modes like the TUI", () => {
    expect(parseGroupMode("trigger")).toBe("trigger");
    expect(parseGroupMode("nonsense")).toBe("pipeline");
    expect(parseGroupMode(null)).toBe("pipeline");
    expect(nextGroupMode("pipeline")).toBe("trigger");
    expect(nextGroupMode("branch")).toBe("off");
    expect(nextGroupMode("off")).toBe("pipeline");
  });

  it("groups by pipeline, ordered by most recent run", () => {
    const heads = groupPipelines(fannedOut(), "pipeline").map((h) => [h.subject, h.runs, h.failed]);
    // CI last ran 10:00, Integration 10:00 (ties keep list order), Release 09:00.
    expect(heads).toEqual([
      ["CI", 2, 0],
      ["Integration", 1, 1],
      ["Release", 1, 0],
    ]);
  });

  it("orders groups by the active sort, comparing each group's most recent run", () => {
    const ms = (r: PipeRow) => (r.run.started_at ? Date.parse(r.run.started_at) : 0);
    const oldest = (a: PipeRow, b: PipeRow) => ms(a) - ms(b);
    // The "Oldest" sort's output: B ran at 09:00, A at 10:00.
    const rows = [
      groupedRow("B", "nz/app", "main", "bbb", "2026-09-24T09:00:00Z", "Succeeded"),
      groupedRow("A", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", "Succeeded"),
    ];
    expect(groupPipelines(rows, "pipeline", oldest).map((h) => h.subject)).toEqual(["B", "A"]);
    // Without a sort: newest first.
    expect(groupPipelines(rows, "pipeline").map((h) => h.subject)).toEqual(["A", "B"]);
  });

  it("lands one header per pipeline carrying its repository and failures (sample_run rows)", () => {
    const row = (def: string, sha: string, status: PipelineRunStatus): PipeRow => {
      const r = groupedRow(def, "nz/app", "main", sha, "", status);
      r.run.definition_id = def.toLowerCase();
      return r;
    };
    const groups = groupPipelines([row("CI", "aaa", "Succeeded"), row("CI", "bbb", "Failed"), row("Integration", "aaa", "Succeeded")], "pipeline");
    expect(groups.map((g) => [g.subject, g.runs, g.failed, g.repo])).toEqual([
      ["CI", 2, 1, "nz/app"],
      ["Integration", 1, 0, "nz/app"],
    ]);
    expect(groupPipelines([row("CI", "aaa", "Succeeded")], "off")).toEqual([]);
  });

  it("a header shows the latest run's status, not the worst", () => {
    const [h] = groupPipelines(
      [groupedRow("CI", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", "Succeeded"), groupedRow("CI", "nz/app", "main", "bbb", "2026-09-24T09:00:00Z", "Failed")],
      "pipeline",
    );
    expect(h.status).toBe("Succeeded");
    expect([h.runs, h.failed]).toEqual([2, 1]);
    expect(h.started).toBe("2026-09-24T10:00:00Z");
    expect(h.latest.run.id).toBe("CI-aaa");
  });

  it("groups one push's fan-out by trigger", () => {
    const h = groupPipelines(fannedOut(), "trigger");
    expect(h).toHaveLength(2);
    expect(h[0].runs).toBe(2);
    expect(h[0].subject).toBe("main");
    expect(h[0].commit).toBe("aaa");
  });

  it("falls back to the start minute without a commit", () => {
    const h = groupPipelines(
      [
        groupedRow("CI", "nz/app", "main", "", "2026-09-24T10:00:30Z", "Succeeded"),
        groupedRow("Integration", "nz/app", "main", "", "2026-09-24T10:00:45Z", "Succeeded"),
        groupedRow("CI", "nz/app", "main", "", "2026-09-24T10:02:00Z", "Succeeded"),
      ],
      "trigger",
    );
    expect(h.map((g) => g.runs)).toEqual([1, 2]);
    expect(pipeTriggerToken(groupedRow("CI", "r", "main", "", "2026-09-24T10:00:30Z", "Succeeded"))).toBe("2026-09-24T10:00");
    expect(h[1].commit).toMatch(/^~\d\d:\d\d$/);
  });

  it("does not merge a branch across repositories", () => {
    const h = groupPipelines(
      [groupedRow("CI", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", "Succeeded"), groupedRow("CI", "nz/other", "main", "bbb", "2026-09-24T09:00:00Z", "Succeeded")],
      "branch",
    );
    expect(h).toHaveLength(2);
  });

  it("flags a group whose run has a gate this user can answer, and reads it Waiting", () => {
    const r = groupedRow("CI", "nz/app", "main", "aaa", "2026-09-24T10:00:00Z", "Running");
    r.approvals = [{ id: "production", name: "production", can_respond: true, blocks_run: true }];
    const [h] = groupPipelines([r], "pipeline");
    expect(h.approval).toBe(true);
    expect(h.status).toBe("Waiting");
  });
});

describe("shown status", () => {
  const runWith = (status: PipelineRunStatus, stages: PipelineStage[]): PipelineRun => ({ id: "1", definition_id: "d", status, stages });
  const stage = (status: PipelineRunStatus, name = "s"): PipelineStage => ({ name, status, jobs: [] });

  it("reads Waiting only when a gate holds it and nothing runs", () => {
    expect(shownStatus(runWith("Running", []), true)).toBe("Waiting");
    expect(shownStatus(runWith("Running", []), false)).toBe("Running");
    expect(shownStatus(runWith("Running", [stage("Succeeded"), stage("Running"), stage("Queued")]), true)).toBe("Running");
    expect(shownStatus(runWith("Running", [stage("Succeeded"), stage("Waiting")]), false)).toBe("Waiting");
    expect(shownStatus(runWith("Canceled", [stage("Waiting")]), true)).toBe("Canceled");
  });

  it("the strip lights the gated stage", () => {
    const run = runWith("Running", [stage("Succeeded", "Build"), stage("Queued", "Deploy Dev"), stage("Queued", "Deploy Prod")]);
    expect(gateStage(run, "Waiting", ["deploy prod"])).toBe(2);
    expect(gateStage(run, "Waiting", [])).toBe(1);
    expect(gateStage(run, "Running", ["Deploy Prod"])).toBeNull();
    expect(stageStrip(run, "Waiting", ["Deploy Prod"])).toEqual(["Succeeded", "Queued", "Waiting"]);
    expect(stageStrip(runWith("Running", []))).toEqual([]);
  });
});
