import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
import { mockFetch } from "../../test/util";
import type { LaunchpadResponse, PipelineRunStatus, PipeRow } from "../../types";
import { ContextMenu, type MenuAnchor } from "../ui";
import { usePipelineRowMenu } from "./pipeline";

const { openRun } = vi.hoisted(() => ({ openRun: vi.fn() }));
vi.mock("../PipelineDetail", () => ({ usePipelineOpener: () => openRun }));

const SHA = "abc1234def5678900000";

function pipe(status: PipelineRunStatus, over: Partial<PipeRow["run"]> = {}): PipeRow {
  return {
    connection_id: "c1",
    connection: "GH",
    provider: "GitHub",
    definition_name: "CI",
    approvals: [],
    run: {
      id: "r1",
      repository: "acme/app",
      definition_id: "ci",
      number: 12,
      title: "CI run 12",
      status,
      branch: "feature/retry",
      commit_sha: SHA,
      started_at: "2026-07-01T00:00:00Z",
      finished_at: status === "Running" || status === "Waiting" ? null : "2026-07-01T00:05:00Z",
      url: "https://github.com/acme/app/actions/runs/r1",
      stages: [],
      ...over,
    },
  };
}

function Harness({ row, onActed }: { row: PipeRow; onActed?: () => void }) {
  const build = usePipelineRowMenu();
  const [anchor, setAnchor] = useState<MenuAnchor | null>({ x: 10, y: 10 });
  return <ContextMenu anchor={anchor} onClose={() => setAnchor(null)} label="Pipeline run actions" items={() => build(row, { onActed })} />;
}

/** Renders the menu over a client holding `row` in the pipelines list and the Command Center
 *  (kept, not garbage-collected, so the optimistic patches can be read back). */
function show(row: PipeRow, onActed?: () => void) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  qc.setQueryData<PipeRow[]>(["pipelines"], [row]);
  qc.setQueryData<LaunchpadResponse>(["launchpad"], {
    rows: [{ bucket: "b", bucket_title: "B", column: 0, muted: false, connection_id: row.connection_id, connection: row.connection, provider: row.provider, kind: "pipe", run: row.run }],
    more: { needs_review: false, your_work: false, your_open_prs: false, recently_merged: false, recent_pipelines: false },
  });
  render(
    <QueryClientProvider client={qc}>
      <Harness row={row} onActed={onActed} />
    </QueryClientProvider>,
  );
  return qc;
}

const item = (name: string) => screen.getByRole("menuitem", { name });
const isDisabled = (name: string) => item(name).getAttribute("aria-disabled") === "true";

let writeText: ReturnType<typeof vi.fn>;
beforeEach(() => {
  openRun.mockReset();
  writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
});
afterEach(() => vi.restoreAllMocks());

describe("pipeline row menu", () => {
  it("a running run offers Cancel (confirmed, optimistic) and holds both re-runs as Still running", async () => {
    const { posts } = mockFetch({});
    const onActed = vi.fn();
    const qc = show(pipe("Running"), onActed);
    expect(isDisabled("Re-run")).toBe(true);
    expect(item("Re-run")).toHaveAttribute("title", "Still running");
    expect(item("Re-run failed jobs")).toHaveAttribute("title", "Still running");

    fireEvent.click(item("Cancel run"));
    expect(posts).toHaveLength(0);
    fireEvent.click(item("Click again to cancel"));

    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({ url: "/api/pipeline/cancel", body: { conn: "c1", repo: "acme/app", run_id: "r1" } });
    expect(onActed).toHaveBeenCalledTimes(1);
    expect(qc.getQueryData<PipeRow[]>(["pipelines"])![0].run.status).toBe("Canceled");
    const lp = qc.getQueryData<LaunchpadResponse>(["launchpad"])!.rows[0];
    expect(lp.kind === "pipe" && lp.run.status).toBe("Canceled");
  });

  it("a failed run re-runs its failed jobs and shows its failing log", async () => {
    const { posts } = mockFetch({});
    const onActed = vi.fn();
    show(pipe("Failed"), onActed);
    expect(screen.queryByRole("menuitem", { name: "Cancel run" })).toBeNull();
    expect(isDisabled("Re-run")).toBe(false);
    expect(isDisabled("Show failing log")).toBe(false);
    expect(isDisabled("Re-run failed jobs")).toBe(false);

    fireEvent.click(item("Re-run failed jobs"));
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({ url: "/api/pipeline/rerun", body: { conn: "c1", repo: "acme/app", run_id: "r1", failed_only: true } });
    expect(onActed).toHaveBeenCalledTimes(1);
  });

  it("Show failing log opens the run's pane", () => {
    mockFetch({});
    show(pipe("Failed"));
    fireEvent.click(item("Show failing log"));
    expect(openRun).toHaveBeenCalledWith({ conn: "c1", repo: "acme/app", runId: "r1" }, { showLog: true });
  });

  it("a succeeded run has no failed jobs to re-run or show", () => {
    mockFetch({});
    show(pipe("Succeeded"));
    expect(isDisabled("Re-run")).toBe(false);
    expect(isDisabled("Re-run failed jobs")).toBe(true);
    expect(item("Re-run failed jobs")).toHaveAttribute("title", "No job has failed");
    expect(item("Show failing log")).toHaveAttribute("title", "No job has failed");
    expect(screen.queryByRole("menuitem", { name: "Cancel run" })).toBeNull();
  });

  it("a gate this user can answer is approved and rejected from the menu", async () => {
    const { posts } = mockFetch({});
    const onActed = vi.fn();
    const row = pipe("Waiting");
    row.approvals = [
      { id: "g1", name: "deploy-prod", can_respond: true, blocks_run: true },
      { id: "g2", name: "deploy-stage", can_respond: false, blocks_run: true },
    ];
    const qc = show(row, onActed);
    expect(item("Reject gate deploy-prod")).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: /deploy-stage/ })).toBeNull();

    fireEvent.click(item("Approve gate deploy-prod"));
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({
      url: "/api/pipeline/approval",
      body: { conn: "c1", repo: "acme/app", run_id: "r1", approval_id: "g1", decision: "Approve" },
    });
    expect(onActed).toHaveBeenCalledTimes(1);
    const after = qc.getQueryData<PipeRow[]>(["pipelines"])![0];
    expect(after.approvals.map((a) => a.id)).toEqual(["g2"]);
    expect(after.run.status).toBe("Running");
  });

  it("Copy commit shows the short sha and copies the full one", async () => {
    mockFetch({});
    show(pipe("Succeeded"));
    const copy = item("Copy commit");
    expect(copy).toHaveTextContent(SHA.slice(0, 7));
    expect(copy).toHaveAttribute("aria-keyshortcuts", "c");
    fireEvent.click(copy);
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(SHA));
  });

  it("Open pull request appears only when the run's PR link resolves", () => {
    mockFetch({});
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    show(pipe("Succeeded", { pull_request: 42 }));
    fireEvent.click(item("Open pull request #42"));
    expect(open).toHaveBeenCalledWith("https://github.com/acme/app/pull/42", "_blank", "noreferrer");
  });

  it("no Open pull request without a PR, or without a link to derive one from", () => {
    mockFetch({});
    show(pipe("Succeeded", { pull_request: 42, url: "https://ci.example.com/run/1" }));
    expect(screen.queryByRole("menuitem", { name: /Open pull request/ })).toBeNull();
  });

  it("a run without a link keeps Open in and Copy link, disabled with the reason", () => {
    mockFetch({});
    show(pipe("Succeeded", { url: null, commit_sha: null, branch: null }));
    expect(item("Open in GitHub")).toHaveAttribute("title", "No link from the provider");
    expect(isDisabled("Copy link")).toBe(true);
    expect(screen.queryByRole("menuitem", { name: "Copy commit" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "Copy branch" })).toBeNull();
  });
});
