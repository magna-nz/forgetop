import { describe, expect, it } from "vitest";
import { createDemoState, reduceDemo } from "./demoStore";

describe("browser-only demo store", () => {
  it("creates fresh fixture state after a reset", () => {
    const initial = createDemoState();
    const updated = reduceDemo(initial, { type: "pipeline.cancel", pipelineId: "pipe-9138" });
    const reset = reduceDemo(updated, { type: "reset" });

    expect(updated.pipelines.find((pipeline) => pipeline.id === "pipe-9138")?.status).toBe("cancelled");
    expect(reset.pipelines.find((pipeline) => pipeline.id === "pipe-9138")?.status).toBe("running");
    expect(reset).not.toBe(initial);
  });

  it("does not mutate the input state while simulating writes", () => {
    const initial = createDemoState();
    const updated = reduceDemo(initial, { type: "pr.merge", prId: "github-1501" });

    expect(initial.pullRequests.find((pullRequest) => pullRequest.id === "github-1501")?.status).toBe("open");
    expect(updated.pullRequests.find((pullRequest) => pullRequest.id === "github-1501")?.status).toBe("merged");
  });
});
