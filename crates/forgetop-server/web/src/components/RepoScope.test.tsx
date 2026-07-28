import { describe, it, expect } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PullRequests } from "./PullRequests";
import { PrDetailProvider } from "./PrDetail";
import { renderWithClient, mockFetch } from "../test/util";

/** A PR row in `repo`, so two rows can share a number across repositories. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const pr = (id: string, title: string, repo: string | null): any => ({
  connection_id: "c",
  connection: "GH",
  provider: "GitHub",
  needs_decoration: true,
  pull_request: {
    id,
    repository: repo,
    number: 7,
    title,
    description: null,
    author: { id: "me", display_name: "Me", handle: "me", avatar_url: null },
    status: "Open",
    is_draft: false,
    source_ref: "feat/x",
    target_ref: "main",
    reviewers: [],
    labels: [],
    checks: "None",
    check_summary: null,
    mergeable: "Unknown",
    changed_files: 0,
    additions: 0,
    deletions: 0,
    created_at: null,
    updated_at: "2026-07-01T00:00:00Z",
    url: null,
  },
});

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const connection = (repoScope: string[] | null): any => ({
  id: "c",
  provider: "GitHub",
  display_name: "GH",
  base_url: null,
  organization: null,
  project: null,
  repository: null,
  username: null,
  repo_scope: repoScope,
  has_token: true,
  sections: ["pull_requests"],
});

describe("repository scope", () => {
  it("distinguishes 'no repositories selected' from 'no pull requests'", async () => {
    // An empty scope is a real, chosen state: nothing was fetched because nothing was asked for.
    // Saying "No open pull requests" here would tell the user the opposite of what happened.
    mockFetch({
      get: {
        "view=all": [],
        "/api/connections/repositories": { repositories: ["acme/pay", "acme/web"], truncated: false },
        "/api/connections": [connection([])],
      },
    });
    renderWithClient(<PullRequests />);
    expect(await screen.findByText("No repositories selected")).toBeInTheDocument();
    expect(screen.queryByText("No open pull requests")).not.toBeInTheDocument();
  });

  it("reports how many of the reachable repositories are in scope", async () => {
    mockFetch({
      get: {
        "view=all": [pr("7", "In pay", "acme/pay")],
        "/api/connections/repositories": {
          repositories: ["acme/pay", "acme/web", "acme/ledger"],
          truncated: false,
        },
        "/api/connections": [connection(["acme/pay"])],
      },
    });
    renderWithClient(<PullRequests />);
    // The count is not decoration: five repositories' worth of PRs read as everything otherwise.
    expect(await screen.findByRole("button", { name: "Repos · 1 of 3" })).toBeInTheDocument();
  });

  it("marks a truncated candidate list rather than presenting a cap as a total", async () => {
    mockFetch({
      get: {
        "view=all": [pr("7", "In pay", "acme/pay")],
        "/api/connections/repositories": { repositories: ["acme/pay", "acme/web"], truncated: true },
        "/api/connections": [connection(["acme/pay"])],
      },
    });
    renderWithClient(<PullRequests />);
    expect(await screen.findByRole("button", { name: "Repos · 1 of 2+" })).toBeInTheDocument();
  });

  it("writes the chosen scope back and keeps an emptied one as an explicit choice", async () => {
    const { posts } = mockFetch({
      get: {
        "view=all": [pr("7", "In pay", "acme/pay")],
        "/api/connections/repositories": { repositories: ["acme/pay", "acme/web"], truncated: false },
        "/api/connections": [connection(["acme/pay"])],
      },
    });
    renderWithClient(<PullRequests />);
    await userEvent.click(await screen.findByRole("button", { name: "Repos · 1 of 2" }));

    // Ticking a second repository widens the scope…
    await userEvent.click(await screen.findByRole("checkbox", { name: "acme/web" }));
    expect(posts.at(-1)?.url).toContain("/api/connections/scope");
    expect(posts.at(-1)?.body).toEqual({ id: "c", scope: ["acme/pay", "acme/web"] });

    // …and clearing the only one sends an *empty list*, not "unset". They are different states:
    // an unset scope falls back to the legacy single repository, an empty one fetches nothing.
    await userEvent.click(screen.getByRole("checkbox", { name: "acme/pay" }));
    expect(posts.at(-1)?.body).toEqual({ id: "c", scope: [] });
  });

  it("keeps rows distinct and addressed when one connection spans several repositories", async () => {
    // The same PR number in two repositories: `connection_id:id` alone stops being a unique key,
    // and opening one has to reach the repository it actually lives in.
    const { fetchMock } = mockFetch({
      get: {
        "view=all": [pr("7", "Retries in pay", "acme/pay"), pr("7", "Retries in web", "acme/web")],
        "/api/connections/repositories": { repositories: ["acme/pay", "acme/web"], truncated: false },
        "/api/connections": [connection(["acme/pay", "acme/web"])],
        "/api/pr/detail": { pull_request: pr("7", "Retries in web", "acme/web").pull_request, threads: [], timeline: [], changes: [], checks: [], commits: [] },
      },
    });
    renderWithClient(
      <PrDetailProvider>
        <PullRequests />
      </PrDetailProvider>,
    );

    // Both rows render — a duplicate React key would have collapsed them into one.
    expect(await screen.findByText("Retries in pay")).toBeInTheDocument();
    expect(await screen.findByText("Retries in web")).toBeInTheDocument();
    // The repository chip is what tells them apart now the connection label can't.
    expect(screen.getByText("acme/pay")).toBeInTheDocument();
    expect(screen.getByText("acme/web")).toBeInTheDocument();

    await userEvent.click(screen.getByText("Retries in web"));
    const detail = fetchMock.mock.calls.map((c) => String(c[0])).find((u) => u.includes("/api/pr/detail"));
    expect(detail).toContain("repo=acme%2Fweb");
  });
});
