import { describe, it, expect, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderWithClient, mockFetch } from "../test/util";

// Spies for navigation + the in-app openers the Launchpad rows call.
const { navigate, openPr, openWi } = vi.hoisted(() => ({
  navigate: vi.fn(),
  openPr: vi.fn(),
  openWi: vi.fn(),
}));
vi.mock("../nav", () => ({ useNavigateSection: () => navigate }));
vi.mock("./PrDetail", () => ({ usePrOpener: () => openPr }));
vi.mock("./WiDetail", () => ({ useWiOpener: () => openWi }));
vi.mock("./PipelineDetail", () => ({ usePipelineOpener: () => vi.fn() }));

// Imported after the mocks so the component picks them up.
import { Launchpad } from "./Launchpad";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const wiRow = (bucket: string, title: string, id: string): any => ({
  bucket,
  bucket_title: bucket === "your_work" ? "Assigned to you" : bucket,
  column: 1,
  muted: false,
  connection_id: "c",
  connection: "GH",
  provider: "GitHub",
  kind: "wi",
  work_item: {
    id,
    identifier: id,
    title,
    description: null,
    state: "In Progress",
    state_category: "Started",
    work_item_type: "Task",
    assignee: null,
    created_at: null,
    updated_at: null,
    url: null,
  },
});

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const prRow = (bucket: string, title: string, id: string): any => ({
  bucket,
  bucket_title: bucket === "needs_fixing" ? "Needs fixing" : bucket,
  column: 0,
  muted: false,
  connection_id: "c",
  connection: "GH",
  provider: "GitHub",
  kind: "pr",
  pull_request: {
    id,
    number: Number(id),
    title,
    description: null,
    author: { id: "me", display_name: "Me", handle: "me", avatar_url: null },
    status: "Open",
    is_draft: false,
    source_ref: null,
    target_ref: null,
    reviewers: [],
    labels: [],
    checks: "None",
    check_summary: null,
    mergeable: "Mergeable",
    changed_files: 0,
    additions: 0,
    deletions: 0,
    created_at: null,
    updated_at: null,
    url: null,
  },
});

const noOverflow = { needs_review: false, your_work: false, your_open_prs: false, recently_merged: false, recent_pipelines: false };

describe("Launchpad", () => {
  it("groups by bucket and its 'more…' navigates for a capped reference list", async () => {
    mockFetch({
      get: {
        "/api/launchpad": {
          rows: [1, 2, 3, 4, 5].map((n) => wiRow("your_work", `Task ${n}`, String(n))),
          more: { ...noOverflow, your_work: true },
        },
      },
    });
    renderWithClient(<Launchpad />);

    expect(await screen.findByText("Assigned to you")).toBeInTheDocument();
    expect(screen.getByText("Task 1")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "more…" }));
    expect(navigate).toHaveBeenCalledWith("work-items");
  });

  it("reveals an expand bucket in place rather than navigating", async () => {
    mockFetch({
      get: {
        "/api/launchpad": {
          rows: [0, 1, 2, 3, 4, 5, 6].map((n) => prRow("needs_fixing", `Fix ${n}`, String(n + 1))),
          more: noOverflow,
        },
      },
    });
    renderWithClient(<Launchpad />);

    // First five shown, the rest hidden behind "more…".
    expect(await screen.findByText("Fix 0")).toBeInTheDocument();
    expect(screen.getByText("Fix 4")).toBeInTheDocument();
    expect(screen.queryByText("Fix 5")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "more…" }));
    expect(screen.getByText("Fix 5")).toBeInTheDocument();
    expect(screen.getByText("Fix 6")).toBeInTheDocument();
    expect(navigate).not.toHaveBeenCalled();
  });

  it("opens a row's item in-app", async () => {
    mockFetch({ get: { "/api/launchpad": { rows: [wiRow("your_work", "Open me", "42")], more: noOverflow } } });
    renderWithClient(<Launchpad />);

    await userEvent.click(await screen.findByText("Open me"));
    await waitFor(() => expect(openWi).toHaveBeenCalledWith({ conn: "c", id: "42" }));
  });
});
