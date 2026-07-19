import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { renderWithClient } from "../test/util";

vi.mock("../api", () => ({
  useHealth: () => ({ data: Array.from({ length: 5 }, (_, i) => ({ connection_id: String(i), healthy: true })) }),
  useLaunchpad: () => ({ data: { rows: [], more: {} } }),
  usePipelines: () => ({ data: [] }),
  usePullRequests: () => ({ data: [] }),
  useWorkItems: () => ({ data: [] }),
}));

import { Sidebar } from "./Sidebar";

describe("Sidebar", () => {
  it("places Give Feedback with connection health and navigates to its section", async () => {
    const onSelect = vi.fn();
    renderWithClient(<Sidebar section="launchpad" onSelect={onSelect} collapsed={false} />);

    const health = screen.getByText("5/5 connections healthy");
    const feedback = screen.getByRole("button", { name: "Give Feedback" });
    expect(health.parentElement?.parentElement).toContainElement(feedback);

    await userEvent.click(feedback);
    expect(onSelect).toHaveBeenCalledWith("feedback");
  });
});
