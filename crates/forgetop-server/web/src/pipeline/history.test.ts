// Built on the TUI's `ci443`/`history_row`/`pane_app` fixtures (crates/forgetop-tui/src/ui.rs):
// the pane there reads "History · CI on this branch · 8 runs · median 1m37s · last failure #438".
import { describe, expect, it } from "vitest";
import { fmtSecs } from "../format";
import type { PipeRow } from "../types";
import { ci438, ci443, historyRow, NOW } from "./fixtures.test-util";
import { estimateCandidates, HISTORY_LEN, pipelineEstimates, pipelineHistory } from "./history";

/** `pane_app`: CI's recent runs on its branch, then the open run. */
function paneRows(): PipeRow[] {
  const rows = [
    historyRow(436, "Succeeded", 60, 98),
    historyRow(437, "Succeeded", 55, 95),
    historyRow(438, "Failed", 48, 88),
    historyRow(439, "Succeeded", 40, 101),
    historyRow(440, "Succeeded", 30, 96),
    historyRow(441, "Succeeded", 20, 99),
    historyRow(442, "Succeeded", 10, 97),
  ];
  const current = historyRow(443, "Succeeded", 0, 101);
  current.run = ci443();
  return [...rows, current];
}

describe("run history", () => {
  it("reads the branch's runs, the median of the others and the last failure", () => {
    const rows = paneRows();
    const h = pipelineHistory(rows, { connection_id: "c", run: ci443() }, undefined, new Map(), NOW)!;
    expect(h.name).toBe("CI");
    expect(h.total).toBe(8);
    expect(h.entries.map((e) => e.number)).toEqual([436, 437, 438, 439, 440, 441, 442, 443]);
    expect(h.current).toBe(7);
    expect(h.entries[2].status).toBe("Failed");
    expect(h.entries[7].secs).toBe(101);
    expect(fmtSecs(h.medianSecs!)).toBe("1m37s");
    expect(h.medianN).toBe(6);
    expect(h.allBranches).toBe(false);
    expect(h.lastFailure).toMatchObject({ number: 438, step: null });
    // With the failed run's detail in hand, the failed step is named.
    const withDetail = pipelineHistory(rows, { connection_id: "c", run: ci443() }, undefined, new Map([["r438", ci438()]]), NOW)!;
    expect(withDetail.lastFailure?.step).toBe("Test");
  });

  it("falls back to every branch when this one has a single run", () => {
    const rows = paneRows();
    const run = ci443();
    run.branch = "v1.2.0";
    rows[rows.length - 1].run = run;
    const h = pipelineHistory(rows, { connection_id: "c", run }, undefined, new Map(), NOW)!;
    expect(h.allBranches).toBe(true);
    expect(h.total).toBe(8);
    expect(pipelineHistory([], { connection_id: "c", run }, undefined, new Map(), NOW)).toBeNull();
  });

  it("keeps the open run inside its ten-wide window", () => {
    const rows = Array.from({ length: 14 }, (_, i) => historyRow(500 + i, "Succeeded", 100 - i, 60));
    const oldest = rows[0].run;
    const h = pipelineHistory(rows, { connection_id: "c", run: oldest }, undefined, new Map(), NOW)!;
    expect(h.entries).toHaveLength(HISTORY_LEN);
    expect(h.current).toBe(0);
    const newest = pipelineHistory(rows, { connection_id: "c", run: rows[13].run }, undefined, new Map(), NOW)!;
    expect(newest.entries[0].number).toBe(504);
    expect(newest.current).toBe(9);
  });
});

describe("estimates", () => {
  it("take the history's run median and per-job medians from recent succeeded runs", () => {
    const rows = paneRows();
    const running = ci443();
    running.status = "Running";
    running.finished_at = null;
    rows[rows.length - 1].run = running;
    const current = { connection_id: "c", run: running };
    expect(estimateCandidates(rows, current).map((r) => r.run.number)).toEqual([442, 441, 440]);
    const h = pipelineHistory(rows, current, undefined, new Map(), NOW);
    let e = pipelineEstimates(rows, current, h, new Map(), NOW);
    expect([e.runMedian, e.runN, e.jobRuns, e.jobs.size]).toEqual([97, 6, 0, 0]);
    const detail = ci443();
    detail.id = "r442";
    e = pipelineEstimates(rows, current, h, new Map([["r442", detail]]), NOW);
    expect(e.jobRuns).toBe(1);
    expect(e.jobs.get("build · test · clippy")).toBe(101);
    // No history: the median over every branch's succeeded runs.
    e = pipelineEstimates(rows, current, null, new Map(), NOW);
    expect([e.runMedian, e.runN]).toEqual([97, 6]);
  });
});
