import { useEffect, type ReactNode } from "react";
import { describe, it, expect } from "vitest";
import { act, render as rtlRender, renderHook, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { PipelineDetailProvider, usePipelineOpener } from "./PipelineDetail";
import { usePipelineLogs } from "../api";
import { renderWithClient, mockFetch } from "../test/util";
import { at, ci438, ci443, historyRow } from "../pipeline/fixtures.test-util";
import type { PipelineCapabilities, PipelineDetail, PipelineRun, PipeRow } from "../types";

function Opener({ conn, runId }: { conn: string; runId: string }) {
  const open = usePipelineOpener();
  useEffect(() => {
    open({ conn, runId });
  }, [open, conn, runId]);
  return null;
}

const render = (runId: string) =>
  renderWithClient(
    <PipelineDetailProvider>
      <Opener conn="c" runId={runId} />
    </PipelineDetailProvider>,
  );

const caps = (over: Partial<PipelineCapabilities> = {}): PipelineCapabilities => ({
  supports_rerun: true,
  supports_rerun_failed: true,
  rerun_starts_new_run: false,
  rerun_failed_starts_new_run: false,
  supports_artifacts: true,
  supports_approvals: true,
  can_respond_approvals: true,
  ...over,
});

const detail = (run: PipelineRun, over: Partial<PipelineDetail> = {}): PipelineDetail => ({ run, approvals: [], capabilities: caps(), ...over });

const connections = [{ id: "c", provider: "GitHub", display_name: "GH", has_token: true, sections: ["pipelines"] }];

/** CI's recent runs on ci443's branch, then the open run (the TUI's `pane_app`). */
function paneRows(): PipeRow[] {
  const rows = [
    historyRow(436, "Succeeded", 60, 98),
    historyRow(437, "Succeeded", 55, 95),
    historyRow(438, "Failed", 48, 88),
    historyRow(439, "Succeeded", 40, 101),
  ];
  const current = historyRow(443, "Succeeded", 0, 101);
  current.run = ci443();
  return [...rows, current];
}

const FAILED_LOG = [
  "##[group]Set up job",
  "Runner info",
  "##[endgroup]",
  "##[group]Build",
  "Compiling forgetop",
  "##[endgroup]",
  "##[group]Test",
  "running 1 test",
  "test x ... FAILED",
  "thread 'x' panicked at src/a.rs:12:9:",
  "assertion failed: ok",
  "##[error]Process completed with exit code 101.",
].join("\n");

/** A run parked on a gate: Build ran, Deploy Prod waits for approval. */
function gatedRun(): PipelineRun {
  const base = 3600;
  return {
    id: "g1",
    definition_id: "deploy",
    number: 2231,
    name: "payments-api",
    title: "fix: retry idempotency key lookup",
    status: "Running",
    branch: "main",
    commit_sha: "7d41c0e0",
    url: "https://github.com/acme/pay/actions/runs/1",
    started_at: at(base, 0),
    finished_at: null,
    stages: [
      {
        name: "Build",
        status: "Succeeded",
        jobs: [{ id: "b1", name: "build", status: "Succeeded", started_at: at(base, 0), finished_at: at(base, 300), steps: [] }],
      },
      { name: "Deploy Prod", status: "Queued", jobs: [] },
    ],
  };
}

/** A POST answer the test releases when it has looked at the screen. */
function held() {
  let resolve: (v: unknown) => void = () => {};
  const promise = new Promise<unknown>((r) => (resolve = r));
  return { promise, resolve };
}

const gate = { id: "a1", name: "Deploy Prod", can_respond: true, blocks_run: true };

describe("PipelineDetail", () => {
  it("a passed run shows its duration, history strip and timed steps", async () => {
    mockFetch({
      get: { "/api/pipeline/detail": detail(ci443()), "/api/pipelines": paneRows(), "/api/connections": connections, "/api/pipeline/annotations": [] },
    });
    render("r443");

    // Header: the run's total elapsed time.
    expect((await screen.findAllByText("1m41s")).length).toBeGreaterThan(0);
    // History strip from the list rows: one square per run, the open one current.
    expect(await screen.findByText("History · CI on this branch")).toBeInTheDocument();
    expect(screen.getByText("5 runs")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /#438 failed/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /#443 succeeded/ })).toHaveAttribute("aria-current", "true");
    expect(screen.getByText(/last failure/)).toHaveTextContent("last failure #438");
    // The single job's steps, with their durations.
    expect(screen.getByText("Run Swatinem/rust-cache@v2")).toBeInTheDocument();
    expect(screen.getByText("Build")).toBeInTheDocument();
    expect(screen.getByText("26s")).toBeInTheDocument();
    expect(screen.getByText("4 post & cleanup steps")).toBeInTheDocument();
    // No footer under a finished run's tree.
    expect(screen.queryByText("slowest")).not.toBeInTheDocument();
    // A passed run has no failure line and no log open.
    expect(screen.queryByText(/Jump to error/)).not.toBeInTheDocument();
  });

  it("a failed run opens the failed job's log, names the failure and lists its problems", async () => {
    mockFetch({
      get: {
        "/api/pipeline/detail": detail(ci438()),
        "/api/pipeline/logs": FAILED_LOG,
        "/api/pipeline/annotations": [
          { level: "Warning", message: "unused variable: `cap`", path: "src/b.rs", line: 3, job_id: "j1" },
          { level: "Failure", message: "assertion failed: ok", title: "x", path: "src/a.rs", line: 12, job_id: "j1" },
        ],
      },
    });
    render("r438");

    expect(await screen.findByText("✗ Test failed")).toBeInTheDocument();
    expect(await screen.findByText("Logs · Test")).toBeInTheDocument();
    expect(await screen.findByText("first error · line 9")).toBeInTheDocument();
    // The Test section is unfolded on the failure.
    const body = screen.getByTestId("log-body");
    expect(within(body).getByText("thread 'x' panicked at src/a.rs:12:9:")).toBeInTheDocument();
    expect(within(body).getByRole("button", { name: /Test/, expanded: true })).toBeInTheDocument();
    // Problems: most severe first.
    expect(await screen.findByText("Problems")).toBeInTheDocument();
    expect(screen.getByText("1 error · 1 warning")).toBeInTheDocument();
    // The failure line names the file only, like the TUI; the problem row keeps the whole path.
    expect(within(screen.getByRole("status")).getByText("a.rs:12")).toBeInTheDocument();
    expect(screen.getByText("src/a.rs:12")).toBeInTheDocument();
  });

  it("with no test name, the failure line shows the annotation's first message line", async () => {
    mockFetch({
      get: {
        "/api/pipeline/detail": detail(ci438()),
        "/api/pipeline/logs": FAILED_LOG,
        "/api/pipeline/annotations": [{ level: "Failure", message: "assertion failed: ok\nleft: 1", path: "src/a.rs", line: 12, job_id: "j1" }],
      },
    });
    render("r438");

    const line = await screen.findByRole("status");
    expect(await within(line).findByText("assertion failed: ok")).toBeInTheDocument();
    expect(within(line).getByText("a.rs:12")).toBeInTheDocument();
  });

  it("fetches a job's log once more when it stops being live", async () => {
    const { fetchMock } = mockFetch({ get: { "/api/pipeline/logs": "step 1\n" } });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
    const logCalls = () => fetchMock.mock.calls.filter((c) => String(c[0]).includes("/api/pipeline/logs")).length;
    const { result, rerender } = renderHook(({ live }) => usePipelineLogs({ conn: "c", runId: "r1" }, "j1", live), {
      wrapper,
      initialProps: { live: true },
    });
    await waitFor(() => expect(result.current.data).toBe("step 1\n"));
    expect(logCalls()).toBe(1);
    // The job finishes with its tail still unread.
    mockFetch({ get: { "/api/pipeline/logs": "step 1\nerror: boom\n" } });
    rerender({ live: false });
    await waitFor(() => expect(result.current.data).toBe("step 1\nerror: boom\n"));
  });

  it("switching runs never asks the new run for the previous run's job log", async () => {
    // The probe from the verifier: open r443, then failed r438 (its log opens), then r443 again.
    const failedRun = ci438();
    failedRun.stages.forEach((s) => s.jobs.forEach((j) => (j.id = "failed-job")));
    const { fetchMock } = mockFetch({
      get: {
        "detail?conn=c&run_id=r443": detail(ci443()),
        "detail?conn=c&run_id=r438": detail(failedRun),
        "/api/pipeline/logs": "##[group]Test\nerror: boom\n",
        "/api/pipeline/annotations": [],
      },
    });
    let openRun: (id: string) => void = () => {};
    function Opener2() {
      const open = usePipelineOpener();
      openRun = (id) => open({ conn: "c", runId: id });
      return null;
    }
    // Default gcTime: the first run's detail stays cached, as it would in the app.
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    rtlRender(
      <QueryClientProvider client={client}>
        <PipelineDetailProvider>
          <Opener2 />
        </PipelineDetailProvider>
      </QueryClientProvider>,
    );
    const urls = () => fetchMock.mock.calls.map((c) => String(c[0]));
    act(() => openRun("r443"));
    await waitFor(() => expect(urls().some((u) => u.includes("detail?conn=c&run_id=r443"))).toBe(true));
    act(() => openRun("r438"));
    expect(await screen.findByText(/^Logs · /)).toBeInTheDocument();
    act(() => openRun("r443"));
    await screen.findAllByText("1m41s");
    await new Promise((r) => setTimeout(r, 100));
    const logs = urls().filter((u) => u.includes("/api/pipeline/logs"));
    expect(logs.some((u) => u.includes("run_id=r438") && u.includes("job=failed-job"))).toBe(true);
    expect(logs.filter((u) => u.includes("run_id=r443"))).toEqual([]);
  });

  it("re-runs only the failed jobs, posting failed_only", async () => {
    // Hold the answer back: what shows meanwhile is the optimistic patch, not a refetch.
    const { promise: answer, resolve } = held();
    const { posts } = mockFetch({
      get: { "/api/pipeline/detail": detail(ci438()), "/api/pipeline/logs": FAILED_LOG, "/api/pipeline/annotations": [] },
      onPost: () => answer,
    });
    render("r438");

    await userEvent.click(await screen.findByRole("button", { name: "↻ Re-run failed jobs" }));
    await waitFor(() => {
      const post = posts.find((p) => p.url.includes("/api/pipeline/rerun"));
      expect(post).toBeDefined();
      expect(post!.body).toMatchObject({ conn: "c", run_id: "r438", failed_only: true });
    });
    // Shown queued at once: the run is in flight again, so Cancel replaces the re-run buttons.
    expect(await screen.findByRole("button", { name: "■ Cancel" })).toBeInTheDocument();
    expect(screen.getByText(/attempt 2/)).toBeInTheDocument();
    resolve({ ok: true });
  });

  it("disables Re-run failed jobs where the provider can't", async () => {
    mockFetch({
      get: {
        "/api/pipeline/detail": detail(ci438(), { capabilities: caps({ supports_rerun_failed: false }) }),
        "/api/connections": connections,
        "/api/pipeline/annotations": [],
      },
    });
    render("r438");

    const btn = await screen.findByRole("button", { name: "↻ Re-run failed jobs" });
    expect(btn).toBeDisabled();
    await waitFor(() => expect(btn).toHaveAttribute("title", "GitHub currently does not support this feature"));
    expect(screen.getByRole("button", { name: "↻ Re-run" })).toBeEnabled();
  });

  it("approves a gate from the banner", async () => {
    const { promise: answer, resolve } = held();
    const { posts } = mockFetch({
      get: { "/api/pipeline/detail": detail(gatedRun(), { approvals: [gate] }), "/api/connections": connections },
      onPost: () => answer,
    });
    render("g1");

    expect(await screen.findByText("⏸ Deploy Prod needs approval")).toBeInTheDocument();
    expect(screen.getByText("⏸ Waiting for approval")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "✓ Approve" }));
    await waitFor(() => {
      const post = posts.find((p) => p.url.includes("/api/pipeline/approval"));
      expect(post).toBeDefined();
      expect(post!.body).toMatchObject({ run_id: "g1", approval_id: "a1", decision: "Approve" });
    });
    // Answered: the banner goes before the provider confirms.
    expect(screen.queryByText("⏸ Deploy Prod needs approval")).not.toBeInTheDocument();
    expect(screen.getByText("Approved Deploy Prod ✓")).toBeInTheDocument();
    resolve({ ok: true });
  });

  it("rejects with an inline reason", async () => {
    const { posts } = mockFetch({ get: { "/api/pipeline/detail": detail(gatedRun(), { approvals: [gate] }), "/api/connections": connections } });
    render("g1");

    await userEvent.click(await screen.findByRole("button", { name: "✗ Reject" }));
    await userEvent.type(screen.getByPlaceholderText("Reason (optional)"), "not today");
    await userEvent.click(screen.getByRole("button", { name: "✗ Confirm reject" }));
    await waitFor(() => {
      const post = posts.find((p) => p.url.includes("/api/pipeline/approval"));
      expect(post?.body).toMatchObject({ approval_id: "a1", decision: "Reject", comment: "not today" });
    });
  });

  it("sends the user to the provider where the gate can't be answered here", async () => {
    mockFetch({
      get: {
        "/api/pipeline/detail": detail(gatedRun(), { approvals: [gate], capabilities: caps({ can_respond_approvals: false }) }),
        "/api/connections": connections,
      },
    });
    render("g1");

    expect(await screen.findByRole("link", { name: /Open in GitHub to approve/ })).toHaveAttribute("href", "https://github.com/acme/pay/actions/runs/1");
    expect(screen.queryByRole("button", { name: "✓ Approve" })).not.toBeInTheDocument();
  });
});
