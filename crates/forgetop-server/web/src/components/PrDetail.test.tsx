import { useEffect } from "react";
import { describe, it, expect } from "vitest";
import { screen, waitFor, waitForElementToBeRemoved, within } from "@testing-library/react";
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
  timeline: [],
  changes: [],
  checks: [],
  commits: [],
  writes: { resolve_threads: true, draft: true, close: true, reopen: true, request_reviewer: true },
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

  // Same verdict the TUI shows above its tabs. Before this you had to assemble it yourself from
  // the reviewer icons, the merge button's tooltip and the checks badge.
  it("states where the pull request stands, above the meta bar", async () => {
    const d = detail("Open");
    d.pull_request.mergeable = "Conflicting";
    mockFetch({ get: { "/api/pr/detail": d } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );

    expect(await screen.findByText(/Blocked — conflicts with/)).toBeInTheDocument();
  });

  // The detail endpoint returns the check runs but not always the summary; without rolling them
  // up the line can only say "failing" where the TUI says "1 of 3 failed".
  it("counts the failures from the returned check runs when no summary came with the PR", async () => {
    const d = detail("Open");
    d.pull_request.checks = "Failed";
    d.pull_request.check_summary = null;
    d.checks = [
      { name: "build", status: "Passed", url: null },
      { name: "test", status: "Passed", url: null },
      { name: "lint", status: "Failed", url: null },
    ];
    mockFetch({ get: { "/api/pr/detail": d } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );

    expect(await screen.findByText("Blocked — 1 of 3 checks failed")).toBeInTheDocument();
  });

  it("selecting an action bar verdict closes the pane", async () => {
    mockFetch({ get: { "/api/pr/detail": detail("Open") } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    // The panel unmounts once the action lands (after the brief post-action delay).
    await waitForElementToBeRemoved(() => screen.queryByText("Cache the customer risk score"), { timeout: 2000 });
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

  it("greys out Merge when the PR is not mergeable", async () => {
    const d = detail("Open");
    d.pull_request.mergeable = "Conflicting";
    mockFetch({ get: { "/api/pr/detail": d } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );
    expect(await screen.findByRole("button", { name: "Merge" })).toBeDisabled();
  });

  it("shows a friendly toast when a merge is rejected by the provider", async () => {
    mockFetch({
      get: { "/api/pr/detail": detail("Open") },
      onPost: (url) => {
        if (url.includes("/api/pr/merge")) throw new Error("409 not mergeable");
        return { ok: true };
      },
    });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Merge" }));
    expect(await screen.findByText(/Couldn't merge — the PR may not be mergeable\./)).toBeInTheDocument();
  });

  it("Conversation shows the timeline and the reviewers with their vote marks", async () => {
    const d = detail("Open");
    const priya = { id: "u1", display_name: "Priya Nair", handle: "p", avatar_url: null };
    d.timeline = [{ actor: priya, kind: "Approved", summary: "approved these changes", at: null }];
    d.pull_request.reviewers = [
      { user: priya, vote: "Approved", is_required: true },
      { user: { id: "u2", display_name: "Marcus Lee", handle: "m", avatar_url: null }, vote: "Rejected", is_required: true },
    ];
    mockFetch({ get: { "/api/pr/detail": d } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );
    expect(await screen.findByText("Timeline")).toBeInTheDocument();
    expect(screen.getByText("approved these changes")).toBeInTheDocument();
    expect(screen.getByText("Reviewers")).toBeInTheDocument();
    expect(screen.getByText("Marcus Lee")).toBeInTheDocument();
  });

  it("action bar checks badge shows failures and opens a checks popover", async () => {
    const d = detail("Open");
    d.checks = [
      { name: "build", status: "Passed", url: null },
      { name: "integration", status: "Failed", url: "https://ci.test/integration" },
    ];
    mockFetch({ get: { "/api/pr/detail": d, "/api/connections": [{ id: "c", provider: "GitHub", display_name: "gh", has_token: true, sections: [] }] } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: /1 check failed/i }));
    expect(await screen.findByRole("link", { name: /integration/i })).toHaveAttribute("href", "https://ci.test/integration");
  });

  it("Files tab: a left-hand file list switches the shown diff", async () => {
    const d = detail("Open");
    d.changes = [
      { path: "src/a.rs", kind: "Modified", additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+newA" },
      { path: "src/b.rs", kind: "Added", additions: 2, deletions: 0, patch: "@@ -0,0 +1,2 @@\n+newB\n+two" },
    ];
    mockFetch({ get: { "/api/pr/detail": d } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: /files/i }));
    // First file's diff is shown by default (its header carries the full path).
    expect(await screen.findByText("src/a.rs")).toBeInTheDocument();
    // Clicking the second file in the list switches the shown diff.
    await userEvent.click(screen.getByRole("button", { name: /b\.rs/i }));
    expect(await screen.findByText("src/b.rs")).toBeInTheDocument();
    expect(screen.queryByText("src/a.rs")).not.toBeInTheDocument();
  });

  it("Commits tab: selecting a commit shows that commit's diff in Files", async () => {
    const d = detail("Open");
    d.commits = [{ sha: "abc1234def", message: "Add retry policy", author: "alice", date: null, url: null }];
    mockFetch({
      get: {
        "/api/pr/detail": d,
        "/api/pr/commit-changes": [{ path: "src/retry.rs", kind: "Added", additions: 6, deletions: 0, patch: "@@ -0,0 +1,1 @@\n+x" }],
      },
    });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: /commits/i }));
    await userEvent.click(await screen.findByRole("button", { name: /Add retry policy/i }));
    // Now on the Files tab, scoped to that commit's diff, with a way back.
    expect(await screen.findByText("Showing commit")).toBeInTheDocument();
    expect(await screen.findByText("src/retry.rs")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Show all files/i })).toBeInTheDocument();
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

  // ---- optimistic writes: what you did shows before the provider answers ----

  /** A POST that doesn't answer until `release` is called. */
  const held = () => {
    let release: () => void = () => {};
    const answer = new Promise<unknown>((resolve) => (release = () => resolve({ ok: true })));
    return { answer, release: () => release() };
  };

  it("an approval shows your tick on the reviewers before the provider answers", async () => {
    const d = detail("Open");
    d.me = "sam";
    const post = held();
    mockFetch({ get: { "/api/pr/detail": d }, onPost: () => post.answer });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );

    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByText("Approved ✓")).toBeInTheDocument();
    expect(screen.getByText("sam")).toBeInTheDocument();
    // Still open: the pane closes once the provider has said yes, so a refusal can be read.
    expect(screen.getByText("Cache the customer risk score")).toBeInTheDocument();
    post.release();
    await waitForElementToBeRemoved(() => screen.queryByText("Cache the customer risk score"), { timeout: 2000 });
  });

  it("a comment shows on the conversation before the provider answers, and can't be replied to yet", async () => {
    const d = detail("Open");
    d.me = "sam";
    const post = held();
    const { posts } = mockFetch({ get: { "/api/pr/detail": d }, onPost: () => post.answer });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );

    await userEvent.type(await screen.findByPlaceholderText("Add a comment…"), "Ship it");
    await userEvent.click(screen.getByRole("button", { name: "Comment" }));
    expect(await screen.findByText("Ship it")).toBeInTheDocument();
    expect(screen.getByText("Posting…")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "↳ Reply" })).not.toBeInTheDocument();
    expect(posts.find((p) => p.url.includes("/api/pr/comment"))?.body).toMatchObject({ body: "Ship it" });
    post.release();
  });

  it("a merge says it's under way, and only says merged once the provider has", async () => {
    const post = held();
    mockFetch({ get: { "/api/pr/detail": detail("Open") }, onPost: () => post.answer });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1" />
      </PrDetailProvider>,
    );

    await userEvent.click(await screen.findByRole("button", { name: "Merge" }));
    expect(await screen.findByText("Merging…")).toBeInTheDocument();
    expect(screen.queryByText("Merged ✓")).not.toBeInTheDocument();
    post.release();
    expect(await screen.findByText("Merged ✓")).toBeInTheDocument();
  });

  // ---- state changes, thread resolution, reviewer requests ----

  const gh = [{ id: "c", provider: "GitHub", display_name: "gh", has_token: true, sections: [] }];
  const bob = { id: "bob", display_name: "Bob", handle: "bob", avatar_url: null };

  it("a draft offers Ready for review, and no approve or merge", async () => {
    const d = detail("Draft");
    d.pull_request.is_draft = true;
    const { posts } = mockFetch({ get: { "/api/pr/detail": d } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );

    await userEvent.click(await screen.findByRole("button", { name: "Ready for review" }));
    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Merge" })).not.toBeInTheDocument();
    await waitFor(() => expect(posts.find((p) => p.url.includes("/api/pr/draft"))?.body).toMatchObject({ id: "1450", draft: false }));
  });

  it("a closed PR offers Reopen, and no approve or merge", async () => {
    const { posts } = mockFetch({ get: { "/api/pr/detail": detail("Closed") } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );

    await userEvent.click(await screen.findByRole("button", { name: "Reopen" }));
    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Merge" })).not.toBeInTheDocument();
    await waitFor(() => expect(posts.find((p) => p.url.includes("/api/pr/closed"))?.body).toMatchObject({ closed: false }));
  });

  it("an open PR closes only on a second, confirming click — and shows closed before the provider answers", async () => {
    const post = held();
    const { posts } = mockFetch({ get: { "/api/pr/detail": detail("Open") }, onPost: () => post.answer });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );

    expect(await screen.findByRole("button", { name: "Convert to draft" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(posts.some((p) => p.url.includes("/api/pr/closed"))).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Confirm close" }));
    expect(posts.find((p) => p.url.includes("/api/pr/closed"))?.body).toMatchObject({ closed: true });
    // Optimistic: the pane is already showing a closed PR.
    expect(await screen.findByRole("button", { name: "Reopen" })).toBeInTheDocument();
    post.release();
  });

  it("Resolve posts the thread id and flips its badge before the provider answers", async () => {
    const d = detail("Open");
    d.threads = [
      { id: "t-7", file_path: null, line: null, is_resolved: false, comments: [{ id: "c1", author: bob, body: "Cap the backoff", created_at: null }] },
      { id: "t-8", file_path: "src/a.rs", line: 3, is_resolved: false, comments: [{ id: "c2", author: bob, body: "inline", created_at: null }] },
    ];
    const post = held();
    const { posts } = mockFetch({ get: { "/api/pr/detail": d }, onPost: () => post.answer });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );

    // Both threads are open, inline ones included.
    expect(await screen.findByText("Comments · 2 open")).toBeInTheDocument();
    expect(screen.getByText("Open")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Resolve" }));
    expect(posts.find((p) => p.url.includes("/api/pr/resolve-thread"))?.body).toMatchObject({ id: "1450", thread_id: "t-7", resolved: true });
    expect(await screen.findByText("Resolved")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reopen" })).toBeInTheDocument();
    expect(screen.getByText("Comments · 1 open")).toBeInTheDocument();
    post.release();
  });

  it("the reviewer menu fetches /api/pr/reviewers once, leaves out the author, and marks who is already reviewing", async () => {
    const d = detail("Open");
    d.pull_request.reviewers = [{ user: bob, vote: "NoVote", is_required: false }];
    const people = [
      { id: "me", display_name: "Me", handle: "me", avatar_url: null },
      bob,
      { id: "carol", display_name: "Carol", handle: "carol", avatar_url: null },
    ];
    const post = held();
    const { fetchMock, posts } = mockFetch({ get: { "/api/pr/detail": d, "/api/pr/reviewers": people }, onPost: () => post.answer });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );
    const reviewerFetches = () => fetchMock.mock.calls.filter(([u]) => String(u).includes("/api/pr/reviewers")).length;

    const toggle = await screen.findByRole("button", { name: /Request reviewer/ });
    expect(reviewerFetches()).toBe(0);
    await userEvent.click(toggle);
    const carol = await screen.findByRole("button", { name: /Carol/ });
    expect(screen.getByRole("button", { name: /Bob.*reviewing/ })).toBeDisabled();
    const menu = screen.getByPlaceholderText("Search people…").closest("div.absolute") as HTMLElement;
    expect(within(menu).queryByText("Me")).not.toBeInTheDocument();
    expect(within(menu).getByText("Carol")).toBeInTheDocument();
    await userEvent.type(screen.getByPlaceholderText("Search people…"), "car");
    expect(screen.queryByRole("button", { name: /Bob/ })).not.toBeInTheDocument();

    await userEvent.click(carol);
    expect(posts.find((p) => p.url.includes("/api/pr/request-reviewer"))?.body).toMatchObject({ id: "1450", user_id: "carol" });
    expect(await screen.findByText("Review requested from Carol ✓")).toBeInTheDocument();
    // Optimistic: Carol is on the reviewers row already.
    expect(screen.getByText("Carol")).toBeInTheDocument();

    // Reopening the menu reuses the list rather than fetching it again.
    post.release();
    await waitFor(() => expect(screen.getByRole("button", { name: /Request reviewer/ })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: /Request reviewer/ }));
    expect(await screen.findByRole("button", { name: /Carol/ })).toBeInTheDocument();
    expect(reviewerFetches()).toBe(1);
  });

  it("keeps the controls visible but disabled, with the standard message, when the provider supports none of them", async () => {
    const none = { resolve_threads: false, draft: false, close: false, reopen: false, request_reviewer: false };
    const open = detail("Open");
    open.writes = none;
    open.threads = [{ id: "t-7", file_path: null, line: null, is_resolved: false, comments: [{ id: "c1", author: bob, body: "hi", created_at: null }] }];
    mockFetch({ get: { "/api/pr/detail": open, "/api/connections": gh } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );

    const message = "GitHub currently does not support this feature";
    for (const name of ["Convert to draft", "Close"]) {
      const button = await screen.findByRole("button", { name });
      expect(button).toBeDisabled();
      await waitFor(() => expect(button).toHaveAttribute("title", message));
    }
    const request = screen.getByRole("button", { name: /Request reviewer/ });
    expect(request).toBeDisabled();
    expect(request).toHaveAttribute("title", message);
    // The thread still says where it stands; there is just nothing to press.
    expect(screen.getByText("Open")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Resolve" })).not.toBeInTheDocument();
  });

  it("a thread the forge cannot resolve shows its state but no Resolve button", async () => {
    const d = detail("Open");
    d.threads = [
      { id: "pr-7", file_path: null, line: null, is_resolved: false, is_resolvable: false, comments: [{ id: "c1", author: bob, body: "flat conversation", created_at: null }] },
      { id: "t-8", file_path: "src/a.rs", line: 3, is_resolved: false, is_resolvable: true, comments: [{ id: "c2", author: bob, body: "inline", created_at: null }] },
    ];
    mockFetch({ get: { "/api/pr/detail": d } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );
    expect(await screen.findByText("Comments · 2 open")).toBeInTheDocument();
    // The bundled conversation shows its state but offers nothing to press; the diff thread does.
    expect(screen.getByText("flat conversation")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Resolve" })).not.toBeInTheDocument();
  });

  it("a draft and a closed PR disable their state change when unsupported", async () => {
    const none = { resolve_threads: false, draft: false, close: false, reopen: false, request_reviewer: false };
    const draft = detail("Draft");
    draft.writes = none;
    mockFetch({ get: { "/api/pr/detail": draft, "/api/connections": gh } });
    const first = renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );
    expect(await screen.findByRole("button", { name: "Ready for review" })).toBeDisabled();
    first.unmount();

    const closed = detail("Closed");
    closed.writes = none;
    mockFetch({ get: { "/api/pr/detail": closed, "/api/connections": gh } });
    renderWithClient(
      <PrDetailProvider>
        <Opener conn="c" id="1450" />
      </PrDetailProvider>,
    );
    const reopen = await screen.findByRole("button", { name: "Reopen" });
    expect(reopen).toBeDisabled();
    await waitFor(() => expect(reopen).toHaveAttribute("title", "GitHub currently does not support this feature"));
  });
});
