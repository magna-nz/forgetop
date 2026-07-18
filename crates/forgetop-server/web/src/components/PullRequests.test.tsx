import { describe, it, expect } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PullRequests } from "./PullRequests";
import { renderWithClient, mockFetch } from "../test/util";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const pr = (id: string, title: string, status = "Open"): any => ({
  connection_id: "c",
  connection: "GH",
  provider: "GitHub",
  pull_request: {
    id,
    number: 1,
    title,
    description: null,
    author: { id: "me", display_name: "Me", handle: "me", avatar_url: null },
    status,
    is_draft: false,
    source_ref: "feat/x",
    target_ref: "main",
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

describe("PullRequests", () => {
  it("offers the four views and refetches when one is picked", async () => {
    mockFetch({
      get: {
        "view=all": [pr("1", "An open PR")],
        "view=merged": [pr("2", "A merged PR", "Merged")],
        "view=yours": [],
        "view=review_requested": [],
      },
    });
    renderWithClient(<PullRequests />);

    for (const label of ["All Pull Requests", "Your PRs", "Recently merged by you", "Review requested"]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }

    // Defaults to All → shows the open PR.
    expect(await screen.findByText("An open PR")).toBeInTheDocument();

    // Switching view refetches for that slice.
    await userEvent.click(screen.getByRole("button", { name: "Recently merged by you" }));
    expect(await screen.findByText("A merged PR")).toBeInTheDocument();
  });

  it("shows a view-specific empty state", async () => {
    mockFetch({ get: { "view=all": [] } });
    renderWithClient(<PullRequests />);
    expect(await screen.findByText("No open pull requests")).toBeInTheDocument();
  });
});
