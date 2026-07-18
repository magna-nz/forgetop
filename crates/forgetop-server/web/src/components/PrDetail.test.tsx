import { useEffect } from "react";
import { describe, it, expect } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PrDetailProvider, usePrOpener } from "./PrDetail";
import { renderWithClient, mockFetch } from "../test/util";

function Opener({ conn, id }: { conn: string; id: string }) {
  const open = usePrOpener();
  useEffect(() => {
    open({ conn, id });
  }, [open, conn, id]);
  return null;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const detail = (status: string): any => ({
  pull_request: {
    id: "1450",
    number: 1,
    title: "Cache the customer risk score",
    description: "desc",
    author: { id: "me", display_name: "Me", handle: "me", avatar_url: null },
    status,
    is_draft: false,
    source_ref: "perf/cache",
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
  threads: [],
  changes: [],
  checks: [],
  commits: [],
});

describe("PrDetail action bar", () => {
  it("a merged PR shows only Revert, and clicking it posts /api/pr/revert", async () => {
    const { posts } = mockFetch({ get: { "/api/pr/detail": detail("Merged") } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );

    expect(await screen.findByRole("button", { name: "Revert" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Merge" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Revert" }));
    await waitFor(() => expect(posts.some((p) => p.url.includes("/api/pr/revert"))).toBe(true));
  });

  it("an open PR shows Request changes / Approve / Merge and no Revert", async () => {
    mockFetch({ get: { "/api/pr/detail": detail("Open") } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );

    expect(await screen.findByRole("button", { name: "Merge" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Approve" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Request changes" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Revert" })).not.toBeInTheDocument();
  });

  it("a PR with changes requested reads as 'changes requested', not 'mergeable'", async () => {
    const d = detail("Open"); // mergeable: "Mergeable" at the conflict level
    d.pull_request.reviewers = [
      { user: { id: "u2", display_name: "Marcus Lee", handle: "marcus", avatar_url: null }, vote: "Rejected", is_required: true },
    ];
    mockFetch({ get: { "/api/pr/detail": d } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1501" />
      </PrDetailProvider>,
    );

    expect(await screen.findByText("changes requested")).toBeInTheDocument();
    expect(screen.queryByText("mergeable")).not.toBeInTheDocument();
  });

  it("Checks tab: an unsupported provider greys the checks and pops the standard message", async () => {
    const d = detail("Open");
    d.checks = [{ name: "build", status: "Passed", url: null }];
    mockFetch({
      get: {
        "/api/pr/detail": d,
        "/api/connections": [{ id: "c", provider: "Demo", display_name: "Demo", has_token: true, sections: [] }],
      },
    });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: /checks/i }));
    await userEvent.click(await screen.findByRole("button", { name: /build/i }));
    expect(await screen.findByText("Demo currently does not support this feature")).toBeInTheDocument();
  });

  it("Checks tab: a supporting provider links each check to the provider", async () => {
    const d = detail("Open");
    d.checks = [{ name: "build", status: "Passed", url: "https://gh.test/checks/1" }];
    mockFetch({
      get: {
        "/api/pr/detail": d,
        "/api/connections": [{ id: "c", provider: "GitHub", display_name: "gh", has_token: true, sections: [] }],
      },
    });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: /checks/i }));
    const link = await screen.findByRole("link", { name: /build/i });
    expect(link).toHaveAttribute("href", "https://gh.test/checks/1");
  });

  it("replying to a conversation thread posts /api/pr/reply with the thread id", async () => {
    const withThread = detail("Open");
    withThread.threads = [
      {
        id: "t-42",
        file_path: null,
        line: null,
        is_resolved: false,
        comments: [{ id: "c1", author: { id: "bob", display_name: "Bob", handle: "bob", avatar_url: null }, body: "Nit here", created_at: null }],
      },
    ];
    const { posts } = mockFetch({ get: { "/api/pr/detail": withThread } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );

    // Open the reply box on the thread, type, and send.
    await userEvent.click(await screen.findByRole("button", { name: "↳ Reply" }));
    await userEvent.type(screen.getByPlaceholderText("Reply…"), "Good point");
    await userEvent.click(screen.getByRole("button", { name: "Reply" }));

    await waitFor(() => {
      const reply = posts.find((p) => p.url.includes("/api/pr/reply"));
      expect(reply).toBeTruthy();
      expect(reply!.body).toMatchObject({ thread_id: "t-42", body: "Good point" });
    });
  });
});
