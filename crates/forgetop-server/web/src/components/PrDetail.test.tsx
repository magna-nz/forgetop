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
});
