import { screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { renderWithClient } from "../test/util";

let pipelines: unknown[] = [];

vi.mock("../api", () => ({
  useHealth: () => ({ data: Array.from({ length: 5 }, (_, i) => ({ connection_id: String(i), healthy: true })) }),
  useLaunchpad: () => ({ data: { rows: [], more: {} } }),
  usePipelines: () => ({ data: pipelines }),
  usePullRequests: () => ({ data: [] }),
  useWorkItems: () => ({ data: [] }),
}));

import { Sidebar } from "./Sidebar";

describe("Sidebar", () => {
  it("places a safe Give Feedback link above the connection-health divider", () => {
    renderWithClient(<Sidebar section="launchpad" onSelect={vi.fn()} collapsed={false} />);

    const health = screen.getByText("5/5 connections healthy");
    const feedback = screen.getByRole("link", { name: "Give Feedback" });
    expect(feedback.compareDocumentPosition(health) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(health.parentElement).not.toContainElement(feedback);
    expect(feedback).toHaveAttribute(
      "href",
      "https://github.com/magna-nz/forgetop/issues/new?template=feedback.yml",
    );
    expect(feedback).toHaveAttribute("target", "_blank");
    expect(feedback).toHaveAttribute("rel", expect.stringContaining("noopener"));
    expect(feedback).toHaveAttribute("rel", expect.stringContaining("noreferrer"));
  });

  it("badges Pipelines with how many pipelines there are, not how many runs", () => {
    const run = (conn: string, repo: string, def: string, id: string) => ({
      connection_id: conn,
      connection: conn,
      provider: "GitHub",
      approvals: [],
      run: { id, definition_id: def, repository: repo, status: "Succeeded" },
    });
    // CI ran three times: one pipeline. The same definition in another repository is another.
    pipelines = [
      run("c", "nz/app", "CI", "1"),
      run("c", "nz/app", "CI", "2"),
      run("c", "nz/app", "CI", "3"),
      run("c", "nz/app", "Release", "4"),
      run("c", "nz/other", "CI", "5"),
    ];
    renderWithClient(<Sidebar section="launchpad" onSelect={vi.fn()} collapsed={false} />);

    expect(screen.getByRole("button", { name: /Pipelines/ })).toHaveTextContent(/Pipelines3$/);
    pipelines = [];
  });
});
