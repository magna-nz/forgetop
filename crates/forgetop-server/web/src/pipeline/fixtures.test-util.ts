// Run fixtures shared by the pipeline tests, ported from crates/forgetop-tui/src/ui.rs tests
// (`sample_run`, `ci443`, `ci438`, `history_row`).
import type { PipelineRun, PipelineRunStatus, PipelineStep, PipeRow } from "../types";

export const NOW = new Date("2026-10-12T12:00:00Z");

/** `secs` seconds after a fixed instant `base` seconds before NOW, as ISO. */
export const at = (base: number, secs: number) => new Date(NOW.getTime() - base * 1000 + secs * 1000).toISOString();

export function sampleRun(): PipelineRun {
  return {
    id: "r1",
    definition_id: "ci",
    number: 101,
    name: "CI",
    title: null,
    status: "Running",
    triggered_by: { id: "u", display_name: "Dana" },
    branch: "main",
    commit_sha: null,
    started_at: null,
    finished_at: null,
    stages: [
      {
        name: "Build",
        status: "Succeeded",
        jobs: [
          {
            id: "j1",
            name: "compile",
            status: "Succeeded",
            started_at: null,
            finished_at: null,
            steps: [{ name: "cargo build", status: "Succeeded", started_at: null, finished_at: null }],
          },
        ],
      },
    ],
  };
}

/** CI #443 from the mockup: one job, eleven steps and four cleanup steps, finished 12m ago. */
export function ci443(): PipelineRun {
  const base = 12 * 60 + 101;
  const spans: [string, number, number][] = [
    ["Set up job", 0, 2],
    ["Run actions/checkout@v4", 2, 1],
    ["Run dtolnay/rust-toolchain@stable", 3, 6],
    ["Run Swatinem/rust-cache@v2", 9, 9],
    ["Run actions/setup-node@v4", 18, 6],
    ["Install dashboard deps", 24, 3],
    ["Dashboard tests", 27, 8],
    ["Build dashboard SPA", 35, 7],
    ["Build", 42, 26],
    ["Test", 68, 19],
    ["Clippy", 87, 11],
    ["Post Run actions/setup-node@v4", 98, 0],
    ["Post Run Swatinem/rust-cache@v2", 98, 1],
    ["Post Run actions/checkout@v4", 99, 0],
    ["Complete job", 99, 0],
  ];
  const steps: PipelineStep[] = spans.map(([name, s, d]) => ({ name, status: "Succeeded", started_at: at(base, s), finished_at: at(base, s + d) }));
  return {
    event: "push",
    attempt: 1,
    pull_request: 195,
    repository: null,
    id: "r443",
    definition_id: "ci",
    number: 443,
    name: "CI",
    title: "feat(tui): lead every footer with a yellow Ctrl-K search anywhere",
    status: "Succeeded",
    branch: "claude/m4-action-palette-shortcut-f096cc",
    commit_sha: "1a4c3aeb2f0d6c4e8a9b1c2d3e4f5a6b7c8d9e0f",
    started_at: at(base, 0),
    finished_at: at(base, 101),
    stages: [
      {
        name: "jobs",
        status: "Succeeded",
        jobs: [{ id: "j1", name: "build · test · clippy", status: "Succeeded", started_at: at(base, 0), finished_at: at(base, 101), steps }],
      },
    ],
  };
}

/** CI #438: failed in Test, Clippy never ran. */
export function ci438(): PipelineRun {
  const run = ci443();
  run.id = "r438";
  run.number = 438;
  run.status = "Failed";
  run.event = "pull_request";
  run.branch = "claude/cache-rewrite-7ab2";
  run.commit_sha = "9c0e1d2aa";
  const job = run.stages[0].jobs[0];
  job.status = "Failed";
  job.steps = job.steps.slice(0, 11);
  job.steps[9].status = "Failed";
  job.steps[10] = { name: "Clippy", status: "Canceled", started_at: null, finished_at: null };
  run.stages[0].status = "Failed";
  return run;
}

/** A list row for a run of CI on ci443's branch, `hours` ago, lasting `secs`. */
export function historyRow(n: number, status: PipelineRunStatus, hours: number, secs: number): PipeRow {
  const run = ci443();
  run.id = `r${n}`;
  run.number = n;
  run.status = status;
  run.stages = [];
  const start = NOW.getTime() - hours * 3600_000;
  run.started_at = new Date(start).toISOString();
  run.finished_at = new Date(start + secs * 1000).toISOString();
  return { connection_id: "c", connection: "GH", provider: "GitHub", run, definition_name: "CI", approvals: [] };
}
