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
  needs_decoration: true,
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
  // Every one of these rows has green CI. Before, all three showed the same "checks passing"
  // pill — the list reported a conflicted and a changes-requested PR as healthy.
  it("names what stops a pull request, not just how its checks went", async () => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const green = (id: string, title: string, f: (p: any) => void) => {
      const row = pr(id, title);
      row.needs_decoration = false;
      row.pull_request.checks = "Passed";
      row.pull_request.check_summary = { successful: 8, in_progress: 0, failed: 0, neutral: 0 };
      f(row.pull_request);
      return row;
    };
    mockFetch({
      get: {
        "view=all": [
          green("1", "A clean PR", () => {}),
          green("2", "A conflicted PR", (p) => (p.mergeable = "Conflicting")),
          green("3", "A rejected PR", (p) => {
            p.reviewers = [{ user: { id: "sam", display_name: "sam", handle: "sam", avatar_url: null }, vote: "Rejected", is_required: false }];
          }),
        ],
      },
    });
    renderWithClient(<PullRequests />);

    expect(await screen.findByText("A clean PR")).toBeInTheDocument();
    expect(screen.getByText("conflicts")).toBeInTheDocument();
    expect(screen.getByText("changes requested")).toBeInTheDocument();
    // Only the genuinely clean row still reports on its checks.
    expect(screen.getAllByText("checks passing")).toHaveLength(1);
  });

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

  it("badges a mergeable PR and omits the badge otherwise", async () => {
    const blocked = pr("2", "A blocked PR");
    blocked.pull_request.mergeable = "Blocked";
    mockFetch({ get: { "view=all": [pr("1", "A mergeable PR"), blocked] } });
    renderWithClient(<PullRequests />);

    // The mergeable PR carries exactly one "Mergeable" badge; the blocked one carries none.
    expect(await screen.findByText("A mergeable PR")).toBeInTheDocument();
    expect(await screen.findByText("A blocked PR")).toBeInTheDocument();
    expect(screen.getAllByText("Mergeable")).toHaveLength(1);
  });
});
