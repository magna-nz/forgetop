import { describe, it, expect, beforeEach } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Pipelines } from "./Pipelines";
import { PipelineDetailProvider } from "./PipelineDetail";
import { renderWithClient, mockFetch } from "../test/util";
import { listStrip } from "../pipeline/grouping";
import type { PipelineRunStatus, PipelineStage, PipeRow } from "../types";

const job = (id: string, status: PipelineRunStatus, problem?: string) => ({
  id,
  name: id,
  status,
  steps: [],
  ...(problem ? { problem } : {}),
});

/** A pipeline row. `at` drives the default "Most recent" sort and the group order. */
function pipe(def: string, id: string, number: number, status: PipelineRunStatus, at: string, stages: PipelineStage[] = []): PipeRow {
  return {
    connection_id: "c1",
    connection: "GH",
    provider: "GitHub",
    definition_name: def,
    approvals: [],
    run: {
      id,
      repository: "acme/app",
      definition_id: def.toLowerCase(),
      number,
      name: null,
      title: `${def} run ${number}`,
      status,
      triggered_by: { id: "u", display_name: "Dana" },
      branch: "main",
      commit_sha: `${id}0000000000`,
      started_at: at,
      finished_at: status === "Running" ? null : at,
      stages,
    },
  };
}

const rowsFixture = [
  pipe("CI", "ci2", 2, "Failed", "2026-07-03T00:00:00Z", [
    { name: "Build", status: "Succeeded", jobs: [job("build", "Succeeded")] },
    { name: "Test", status: "Failed", jobs: [job("test", "Failed", "3 tests failed")] },
    { name: "Deploy", status: "Skipped", jobs: [job("deploy", "Skipped")] },
  ]),
  pipe("CI", "ci1", 1, "Succeeded", "2026-07-02T00:00:00Z"),
  pipe("Release", "rel1", 7, "Running", "2026-07-01T00:00:00Z"),
];

const choose = (mode: string) => userEvent.selectOptions(screen.getByLabelText("Group"), mode);
const headers = () => screen.queryAllByRole("button", { expanded: false }).concat(screen.queryAllByRole("button", { expanded: true }));
const render = () =>
  renderWithClient(
    <PipelineDetailProvider>
      <Pipelines />
    </PipelineDetailProvider>,
  );

beforeEach(() => localStorage.clear());

describe("pipelines list grouping", () => {
  it("is grouped by pipeline by default, collapsed to the headers", async () => {
    mockFetch({ get: { "/api/pipelines": rowsFixture } });
    render();
    const ci = await screen.findByRole("button", { name: /\bCI\b/ });
    expect(screen.getByLabelText("Group")).toHaveValue("pipeline");
    expect(screen.getAllByRole("button", { expanded: false })).toHaveLength(2);
    expect(ci).toHaveAttribute("aria-expanded", "false");
    expect(ci).toHaveTextContent("· 1 failed");
    expect(ci).toHaveTextContent("2 runs");
    expect(screen.getByRole("button", { name: /Release/ })).toHaveTextContent("1 run");
    expect(screen.queryByText("CI run 1")).not.toBeInTheDocument();
  });

  it("expanding a header reveals its runs", async () => {
    mockFetch({ get: { "/api/pipelines": rowsFixture } });
    render();
    const ci = await screen.findByRole("button", { name: /\bCI\b/ });
    await userEvent.click(ci);
    expect(ci).toHaveAttribute("aria-expanded", "true");
    // The failed run shows its problem in place of the title; the other its title.
    expect(screen.getByText("✗ test · 3 tests failed")).toBeInTheDocument();
    expect(screen.getByText("CI run 1")).toBeInTheDocument();
    expect(screen.getByText("#2")).toBeInTheDocument();
    expect(screen.getByText("#1")).toBeInTheDocument();
    expect(screen.queryByText("Release run 7")).not.toBeInTheDocument();
    await userEvent.click(ci);
    expect(screen.queryByText("CI run 1")).not.toBeInTheDocument();
  });

  it("Off lists every run flat with no headers, and the choice persists", async () => {
    mockFetch({ get: { "/api/pipelines": rowsFixture } });
    const { unmount } = render();
    await screen.findByRole("button", { name: /\bCI\b/ });
    await choose("off");
    for (const t of ["✗ test · 3 tests failed", "CI run 1", "Release run 7"]) expect(screen.getByText(t)).toBeInTheDocument();
    expect(headers()).toHaveLength(0);
    expect(localStorage.getItem("forgetop:pipelines:group")).toBe("off");

    unmount();
    render();
    expect(await screen.findByText("Release run 7")).toBeInTheDocument();
    expect(screen.getByLabelText("Group")).toHaveValue("off");
    expect(headers()).toHaveLength(0);
  });

  it("draws one square per stage, labelled with the counts", async () => {
    localStorage.setItem("forgetop:pipelines:group", "off");
    mockFetch({ get: { "/api/pipelines": rowsFixture } });
    render();
    await screen.findByText("Release run 7");
    const strips = screen.getAllByRole("img", { name: /^stages:/ });
    // Only the failed CI run carries stages.
    expect(strips).toHaveLength(1);
    expect(strips[0]).toHaveAccessibleName("stages: 1 succeeded, 1 failed, 1 skipped");
    expect(strips[0].children).toHaveLength(3);
  });
});

describe("listStrip", () => {
  const run = (stages: PipelineStage[]) => pipe("CI", "x", 1, "Running", "2026-07-01T00:00:00Z", stages).run;

  it("is one status per stage for a multi-stage run", () => {
    const r = run([
      { name: "Build", status: "Succeeded", jobs: [job("a", "Succeeded"), job("b", "Succeeded")] },
      { name: "Deploy", status: "Running", jobs: [job("c", "Running")] },
    ]);
    expect(listStrip(r)).toEqual({ unit: "stages", statuses: ["Succeeded", "Running"] });
  });

  it("is one status per job for a single-stage run", () => {
    const r = run([{ name: "jobs", status: "Running", jobs: [job("a", "Succeeded"), job("b", "Running"), job("c", "Queued")] }]);
    expect(listStrip(r)).toEqual({ unit: "jobs", statuses: ["Succeeded", "Running", "Queued"] });
  });

  it("marks a gated single stage's first queued job Waiting", () => {
    const r = run([{ name: "jobs", status: "Queued", jobs: [job("a", "Succeeded"), job("b", "Queued"), job("c", "Queued")] }]);
    expect(listStrip(r, "Waiting", [])).toEqual({ unit: "jobs", statuses: ["Succeeded", "Waiting", "Queued"] });
  });

  it("is empty for a run with no stages", () => {
    expect(listStrip(run([]))).toEqual({ unit: "stages", statuses: [] });
  });
});
