// Ported from crates/forgetop-tui/src/ui.rs tests: `pipeline_view_flattens_stage_job_step`, the
// ci443/ci438 pane tests (folds, slowest note, axis ticks) and the job-ghost tests.
import { describe, expect, it } from "vitest";
import { axisLabel, fmtSecs } from "../format";
import type { PipelineJob, PipelineRun, PipelineRunStatus, PipelineStage } from "../types";
import { at, ci438, ci443, NOW, sampleRun } from "./fixtures.test-util";
import {
  attentionRow,
  autoSelect,
  axisTicks,
  criticalPath,
  criticalPathText,
  failedTarget,
  flatten,
  ghostKey,
  jobGhosts,
  maxTicksFor,
  slowestLeaves,
  stepFolds,
  TimeAxis,
  timelineAxis,
  timelineSpan,
  treeTitle,
  type TreeInput,
} from "./timeline";

const tree = (run: PipelineRun, extra: Partial<TreeInput> = {}): TreeInput => ({
  run,
  folds: {},
  gates: [],
  shown: run.status,
  canRespondApprovals: true,
  providerLabel: "GitHub",
  now: NOW,
  ...extra,
});

/** The TUI's `toggle_selected`: flip the fold of the node at `i`. */
function toggle(input: TreeInput, i: number): TreeInput {
  const n = flatten(input)[i];
  return n?.key ? { ...input, folds: { ...input.folds, [n.key]: !n.expanded } } : input;
}

describe("formats", () => {
  it("fmtSecs and axisLabel match the TUI", () => {
    expect([fmtSecs(5), fmtSecs(65), fmtSecs(3720), fmtSecs(-3)]).toEqual(["5s", "1m05s", "1h02m", "0s"]);
    expect([axisLabel(0), axisLabel(30), axisLabel(60), axisLabel(90), axisLabel(7200), axisLabel(3900)]).toEqual(["0s", "30s", "1m", "1m30", "2h", "1h05"]);
  });
});

describe("flatten", () => {
  it("flattens stage → job → step", () => {
    // One stage with one job: both levels are dropped and the steps sit at the top.
    let flat = flatten(tree(sampleRun()));
    expect(flat).toHaveLength(1);
    expect([flat[0].label, flat[0].depth, flat[0].step]).toEqual(["cargo build", 0, 0]);
    expect(flat[0].jobId).toBe("j1");

    // Two stages keep every level — but finished stages start folded, one line each.
    const run = sampleRun();
    run.stages.push(structuredClone(run.stages[0]));
    let input = tree(run);
    flat = flatten(input);
    expect(flat).toHaveLength(2);
    expect(flat[0].expanded).toBe(false);
    expect(flat[0].summary?.text).toBe("1 job");
    for (const i of [1, 0]) input = toggle(input, i);
    flat = flatten(input);
    expect(flat).toHaveLength(4);
    expect(flat[1].expanded).toBe(false);
    for (const i of [3, 1]) input = toggle(input, i);
    flat = flatten(input);
    expect(flat).toHaveLength(6);
    expect(flat[0].depth).toBe(0);
    expect(flat[2].depth).toBe(2);
    expect(Object.keys(input.folds).sort()).toEqual(["s0", "s0.j0", "s1", "s1.j0"]);
  });

  it("a passing run folds its cleanup steps and titles the tree by its job", () => {
    const run = ci443();
    const flat = flatten(tree(run));
    expect(treeTitle(run)).toBe("build · test · clippy");
    expect(flat.map((n) => n.label)).toContain("4 post & cleanup steps");
    expect(flat.map((n) => n.label)).not.toContain("Complete job");
    expect(flat.find((n) => n.label === "Build")?.duration).toBe("26s");
    const post = flat.find((n) => n.group)!;
    expect([post.key, post.expanded, post.status]).toEqual(["s0.j0.post", false, "Succeeded"]);
    expect(stepFolds(run.stages[0].jobs[0], false)).toEqual([0, 11]);
  });

  it("a failed run folds the passed steps and selects the failed one", () => {
    const run = ci438();
    const input = tree(run);
    const flat = flatten(input);
    expect(flat.map((n) => n.label)).toEqual(["8 steps passed", "Build", "Test", "Clippy"]);
    expect(flat[0].key).toBe("s0.j0.pass");
    expect(failedTarget(run)).toEqual({ jobId: "j1", name: "Test", step: 9 });
    const sel = autoSelect(input);
    expect(sel.openLogs).toBe(true);
    expect(flatten({ ...input, folds: sel.folds })[sel.selected!].label).toBe("Test");
  });

  it("a gated stage opens onto why it has no jobs; skipped stages fold into one row", () => {
    const stage = (name: string, status: PipelineRunStatus, jobs: PipelineJob[] = []): PipelineStage => ({ name, status, jobs });
    const job: PipelineJob = { id: "b", name: "build", status: "Succeeded", steps: [] };
    const gated: PipelineRun = {
      id: "r610",
      definition_id: "deploy",
      status: "Running",
      stages: [stage("Build", "Succeeded", [job]), stage("Deploy Prod", "Waiting"), stage("Post-deploy checks", "Queued")],
    };
    const input = tree(gated, { shown: "Waiting", gates: ["Deploy Prod"], canRespondApprovals: false, providerLabel: "Azure DevOps" });
    const flat = flatten(input);
    expect(flat.map((n) => n.label)).toEqual([
      "Build",
      "Deploy Prod",
      "approval required · its jobs start once approved",
      "o  open in Azure DevOps to approve",
      "Post-deploy checks",
    ]);
    expect(flat[4].note).toBe("after Deploy Prod");
    expect(attentionRow(input)).toBe(1);

    const failed: PipelineRun = {
      id: "r609",
      definition_id: "deploy",
      status: "Failed",
      stages: [stage("Build", "Failed", [{ ...job, status: "Failed", problem: "3 errors" }]), ...["Dev", "Staging", "Prod", "Checks"].map((n) => stage(n, "Skipped"))],
    };
    const f = flatten(tree(failed));
    expect(f).toHaveLength(3);
    expect([f[2].label, f[2].key, f[2].note, f[2].group]).toEqual(["4 stages skipped", "s1.skip", "Build failed", true]);
    expect(flatten(tree(failed, { folds: { "s1.skip": true } }))).toHaveLength(7);
  });
});

describe("timeline", () => {
  it("ticks at round steps and reads the slowest steps", () => {
    const run = ci443();
    const span = timelineSpan(run, run.status, new Map(), NOW)!;
    expect(span.total).toBe(101);
    const axis = timelineAxis(span, 40);
    const { step, ticks } = axisTicks(axis, maxTicksFor(40));
    expect(step).toBe(30);
    expect(ticks.map((t) => t.label)).toEqual(["0s", "30s", "1m", "1m30"]);
    expect(axis.col(101)).toBe(40);
    const slow = slowestLeaves(run, NOW)!;
    expect(slow.leaves).toEqual([
      { name: "Build", secs: 26 },
      { name: "Test", secs: 19 },
      { name: "Clippy", secs: 11 },
    ]);
    expect(slow.sharePct).toBe(55);
  });

  it("squeezes a long wait after a short stretch of work", () => {
    const axis = TimeAxis.parked(1000, 100, 40);
    expect(axis.squeeze).toEqual({ worked: 100, workW: 26 });
    expect(axis.pos(100)).toBe(26);
    expect(axis.pos(1000)).toBe(39);
    expect(axis.ticked()).toEqual([100, 26]);
    expect(TimeAxis.parked(100, 80, 40).squeeze).toBeNull();
  });
});

describe("job ghosts", () => {
  const ghostJob = (name: string, status: PipelineRunStatus, started?: Date): PipelineJob => ({
    id: name,
    name,
    status,
    started_at: started?.toISOString() ?? null,
    finished_at: null,
    steps: [],
  });
  const ghostRun = (stages: PipelineStage[]): PipelineRun => ({ ...ci443(), status: "Running", finished_at: null, stages });
  const ago = (s: number) => new Date(NOW.getTime() - s * 1000);
  const rel = (t: number) => Math.round((t - NOW.getTime()) / 1000);

  it("queued jobs in a stage run side by side and stages follow each other", () => {
    const stage = (name: string, jobs: PipelineJob[]): PipelineStage => ({ name, status: "Running", jobs });
    const run = ghostRun([
      stage("build", [ghostJob("a", "Running", ago(50))]),
      stage("test", [ghostJob("b", "Queued"), ghostJob("c", "Queued")]),
      stage("ship", [ghostJob("d", "Queued")]),
    ]);
    const jobs = new Map(Object.entries({ a: 100, b: 30, c: 60, d: 10 }));
    const g = jobGhosts({ run, shown: "Running", estimates: { jobs }, now: NOW });
    const span = (si: number, ji: number) => [rel(g.get(ghostKey(si, ji))!.start), rel(g.get(ghostKey(si, ji))!.end)];
    expect(span(0, 0)).toEqual([0, 50]);
    expect(span(1, 0)).toEqual([50, 80]);
    expect(span(1, 1)).toEqual([50, 110]);
    expect(span(2, 0)).toEqual([110, 120]);
    const cp = criticalPath(run, g)!;
    expect([cp.job.name, cp.waiting?.name, cp.leftSecs]).toEqual(["a", "b", 50]);
    expect(criticalPathText(cp)).toBe("a is the critical path: b waits ~50s on it");
    // With nothing queued behind it, the running job is just the long pole.
    const alone = ghostRun([stage("build", [ghostJob("a", "Running", ago(50))])]);
    const lone = criticalPath(alone, jobGhosts({ run: alone, shown: "Running", estimates: { jobs }, now: NOW }))!;
    expect(criticalPathText(lone)).toBe("a is the long pole · ~50s left");
    expect(jobGhosts({ run, shown: "Waiting", estimates: { jobs }, now: NOW }).size).toBe(0);
  });

  it("in a single stage a queued job waits on the matrix it follows", () => {
    const run = ghostRun([
      {
        name: "jobs",
        status: "Running",
        jobs: [
          ghostJob("build-local-artifacts (linux)", "Running", ago(50)),
          ghostJob("build-local-artifacts (windows)", "Running", ago(50)),
          ghostJob("lint", "Running", ago(20)),
          ghostJob("build-global-artifacts", "Queued"),
          ghostJob("host", "Queued"),
          ghostJob("announce", "Queued"),
        ],
      },
    ]);
    const jobs = new Map(
      Object.entries({
        "build-local-artifacts (linux)": 80,
        "build-local-artifacts (windows)": 100,
        lint: 120,
        "build-global-artifacts": 20,
        host: 10,
        announce: 5,
      }),
    );
    const g = jobGhosts({ run, shown: "Running", estimates: { jobs }, now: NOW });
    const start = (j: number) => rel(g.get(ghostKey(0, j))!.start);
    expect(start(3)).toBe(50);
    expect(start(4)).toBe(100);
    expect(start(5)).toBe(100);
    // lint is the long pole (+100); host's estimate starts as it ends.
    const cp = criticalPath(run, g)!;
    expect([cp.job.name, cp.waiting?.name, cp.leftSecs]).toEqual(["lint", "host", 100]);
    expect(criticalPathText(cp)).toBe("lint is the critical path: host waits ~1m40 on it");
  });

  it("an in-flight rerun starts its timeline at the new attempt", () => {
    const old = new Date(NOW.getTime() - 2 * 86400_000);
    const passed = { ...ghostJob("passed", "Succeeded", old), finished_at: new Date(old.getTime() + 40_000).toISOString() };
    const run: PipelineRun = {
      ...ci443(),
      status: "Running",
      started_at: at(35, 0),
      finished_at: null,
      stages: [{ name: "jobs", status: "Running", jobs: [passed, ghostJob("retried", "Running", ago(30))] }],
    };
    const span = timelineSpan(run, "Running", new Map(), NOW)!;
    expect(span.total).toBe(35);
  });
});
