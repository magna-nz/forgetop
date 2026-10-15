import { afterEach, describe, it, expect, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import userEvent from "@testing-library/user-event";
import { renderWithClient, mockFetch } from "../test/util";
import { PrDetailProvider } from "./PrDetail";
import { WiDetailProvider } from "./WiDetail";
import { Notifications, dayGroup } from "./Notifications";
import { setPaneGuard } from "./ui";
import { useInboxActions, type InboxActions } from "./inboxActions";
import { patchInboxDetailInCache } from "../optimistic";
import type { InboxThread, NotifRow, NotificationKind, NotificationTarget, NotificationTier } from "../types";

const HOUR = 3600_000;
const ago = (h: number) => new Date(Date.now() - h * HOUR).toISOString();

const TIER: Record<NotificationKind, NotificationTier> = {
  ReviewRequested: "ForYou",
  Mention: "ForYou",
  Assigned: "ForYou",
  CiFailed: "ForYou",
  Due: "ForYou",
  Comment: "Following",
  StateChange: "Following",
  NewItem: "Following",
  Other: "Following",
  Reaction: "Hidden",
};

const row = (
  id: string,
  o: {
    kind?: NotificationKind;
    unread?: boolean;
    title?: string;
    item?: string;
    type?: "PullRequest" | "WorkItem";
    phrase?: string;
    excerpt?: string;
    target?: NotificationTarget;
    detailRef?: string;
    h?: number;
  } = {},
): NotifRow => {
  const kind = o.kind ?? "ReviewRequested";
  return {
    connection_id: "c",
    connection: "GH",
    provider: "GitHub",
    notification: {
      id,
      kind,
      item_type: o.type ?? "PullRequest",
      item_id: o.item ?? "10",
      title: o.title ?? "Review this PR",
      context: "northwind/payments",
      url: `https://example.test/${id}`,
      unread: o.unread ?? true,
      updated_at: ago(o.h ?? 1),
      reason: "",
      excerpt: o.excerpt,
      target: o.target ?? { kind: "Overview" },
      detail_ref: o.detailRef ?? null,
    },
    phrase: o.phrase ?? "review requested",
    why: "your review was requested",
    tier: TIER[kind],
    dismiss_key: `c:${id}:0`,
  };
};

const thread = (lead: NotifRow, more: NotifRow[] = [], hidden: NotifRow[] = []): InboxThread => {
  const all = [lead, ...more, ...hidden];
  return {
    key: `c|${lead.notification.item_type}|${lead.notification.item_id}`,
    tier: lead.tier,
    unread: [lead, ...more].some((r) => r.notification.unread),
    latest: lead.notification.updated_at,
    lead,
    more,
    members: all.map((r) => ({ conn: r.connection_id, id: r.notification.id, unread: r.notification.unread })),
  };
};

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const prDetail = (o: { threads?: any[]; checks?: any[] } = {}): any => ({
  pull_request: {
    id: "10",
    number: 10,
    title: "Add idempotency keys",
    description: "desc",
    author: { id: "me", display_name: "Me", handle: "me", avatar_url: null },
    status: "Open",
    is_draft: false,
    source_ref: "a",
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
  threads: o.threads ?? [],
  timeline: [],
  changes: [],
  checks: o.checks ?? [],
  commits: [],
  writes: { resolve_threads: false, draft: false, close: false, reopen: false, request_reviewer: false },
});

const priya = { id: "u1", display_name: "Priya Nair", handle: "priya", avatar_url: null };
const comment = (id: string, body: string) => ({ id, author: priya, body, created_at: null });

function renderInbox() {
  return renderWithClient(
    <PrDetailProvider>
      <WiDetailProvider>
        <Notifications />
      </WiDetailProvider>
    </PrDetailProvider>,
  );
}

/** The inbox rows on screen, by their title. */
const rowTitles = () => [...document.querySelectorAll("[data-thread]")].map((r) => r.querySelector("button span.truncate")?.textContent);
const rowOf = (title: string) => screen.getByText(title).closest("[data-thread]") as HTMLElement;

describe("Notifications", () => {
  it("splits the inbox into For you / Following / All, counting unread threads", async () => {
    mockFetch({
      get: {
        "/api/inbox": [
          thread(row("a", { title: "Needs your review", item: "1", h: 1 })),
          thread(row("b", { kind: "Mention", unread: false, title: "Read mention", item: "2", h: 2 })),
          thread(row("c", { kind: "Comment", title: "A comment you follow", item: "3", h: 3 })),
          thread(row("d", { kind: "StateChange", title: "A state you follow", item: "4", h: 4 })),
        ],
        "/api/connections": [],
      },
    });
    renderInbox();

    expect(await screen.findByText("Needs your review")).toBeInTheDocument();
    const tab = (name: RegExp) => screen.getByRole("tab", { name });
    expect(tab(/For you/)).toHaveAttribute("aria-selected", "true");
    expect(within(tab(/For you/)).getByText("1")).toBeInTheDocument();
    expect(within(tab(/Following/)).getByText("2")).toBeInTheDocument();
    expect(within(tab(/All/)).getByText("3")).toBeInTheDocument();
    // For you: what asks for you, read or not; nothing you merely follow.
    expect(rowTitles()).toEqual(["Needs your review", "Read mention"]);

    await userEvent.click(tab(/Following/));
    expect(rowTitles()).toEqual(["A comment you follow", "A state you follow"]);
    await userEvent.click(tab(/All/));
    expect(rowTitles()).toHaveLength(4);
    // Grouped by day.
    expect(screen.getByText("Today")).toBeInTheDocument();
  });

  it("draws one row per thread: the lead's phrase and excerpt, and +N for the rest", async () => {
    mockFetch({
      get: {
        "/api/inbox": [
          thread(
            row("m", { kind: "Mention", phrase: "Priya mentioned you", excerpt: "Can we land this before the freeze?", title: "Idempotency keys" }),
            [row("c1", { kind: "Comment", phrase: "Sam commented" }), row("c2", { kind: "Comment", phrase: "new comment" })],
          ),
        ],
        "/api/connections": [],
      },
    });
    renderInbox();

    const r = rowOf(await screen.findByText("Idempotency keys").then((e) => e.textContent!));
    expect(document.querySelectorAll("[data-thread]")).toHaveLength(1);
    expect(within(r).getByText(/Priya mentioned you · “Can we land this before the freeze\?”/)).toBeInTheDocument();
    expect(within(r).getByText("+2")).toBeInTheDocument();
    expect(within(r).getByText("#10")).toBeInTheDocument();
  });

  it("opens a comment notification on the PR's Conversation, at the comment, and marks the thread read", async () => {
    const lead = row("m", { kind: "Mention", title: "Idempotency keys", target: { kind: "Comment", id: "c-key" } });
    const hidden = row("r", { kind: "Reaction", unread: true });
    const { posts } = mockFetch({
      get: {
        "/api/inbox": [thread(lead, [], [hidden]), thread(row("x", { title: "Another", item: "11" }))],
        "/api/pr/detail": prDetail({
          threads: [
            { id: "t0", comments: [comment("c-other", "An older remark")], file_path: null, line: null, is_resolved: false },
            { id: "t1", comments: [comment("c-key", "@sam can we land this today?")], file_path: null, line: null, is_resolved: false },
          ],
        }),
        "/api/connections": [],
      },
    });
    renderInbox();

    await userEvent.click(await screen.findByText("Idempotency keys"));
    const pane = await screen.findByRole("region", { name: "Pull request" });
    await within(pane).findByText("@sam can we land this today?");
    expect(within(pane).getByRole("tab", { name: /Conversation/ })).toHaveAttribute("aria-selected", "true");
    // The comment it named is marked, and says why.
    const marked = pane.querySelector("[data-target]") as HTMLElement;
    expect(marked).toHaveTextContent("@sam can we land this today?");
    expect(within(marked).getByText("From your inbox")).toBeInTheDocument();
    expect(pane.querySelectorAll("[data-target]")).toHaveLength(1);
    // The triage strip says where in the inbox this is.
    expect(within(pane).getByTestId("triage-strip")).toHaveTextContent("Inbox · 1 of 2");
    // Every unread member is marked read, the hidden reaction included.
    await waitFor(() => expect(posts.find((p) => p.url.includes("/api/notification/read-many"))?.body).toEqual({ conn: "c", ids: ["m", "r"] }));
    // And the row is selected while its pane is open.
    expect(rowOf("Idempotency keys")).toHaveAttribute("data-selected", "true");
  });

  it("opens a CI failure on the PR's Checks tab with the failing check selected", async () => {
    mockFetch({
      get: {
        "/api/inbox": [thread(row("f", { kind: "CiFailed", title: "Bump Next.js", target: { kind: "Checks" } }))],
        "/api/pr/detail": prDetail({
          checks: [
            { name: "build", status: "Passed", url: null },
            { name: "e2e", status: "Failed", url: null },
          ],
        }),
        "/api/connections": [],
      },
    });
    renderInbox();

    await userEvent.click(await screen.findByText("Bump Next.js"));
    const pane = await screen.findByRole("region", { name: "Pull request" });
    await waitFor(() => expect(within(pane).getByRole("tab", { name: /Checks/ })).toHaveAttribute("aria-selected", "true"));
    expect(within(pane).getByText("e2e").closest(".ft-row")).toHaveAttribute("data-selected", "true");
    expect(within(pane).getByText("build").closest(".ft-row")).not.toHaveAttribute("data-selected");
  });

  it("done takes the thread out at once and posts every member id", async () => {
    let answer!: () => void;
    const held = new Promise<void>((r) => (answer = r));
    const { posts } = mockFetch({
      get: {
        "/api/inbox": [thread(row("a", { title: "Ping", item: "1" }), [row("b", { kind: "Comment", item: "1" })], [row("z", { kind: "Reaction", item: "1" })])],
        "/api/connections": [],
      },
      onPost: async (url) => {
        if (url.includes("/api/notification/done")) await held;
        return { ok: true };
      },
    });
    renderInbox();

    await screen.findByText("Ping");
    await userEvent.click(within(rowOf("Ping")).getByRole("button", { name: "Done" }));
    // Gone before the server has answered.
    expect(screen.queryByText("Ping")).not.toBeInTheDocument();
    await waitFor(() => expect(posts.find((p) => p.url.includes("/api/notification/done"))?.body).toEqual({ conn: "c", ids: ["a", "b", "z"] }));
    answer();
  });

  it("walks the list with j/k, and j swaps the open pane in place", async () => {
    mockFetch({
      get: {
        "/api/inbox": [thread(row("a", { title: "First", item: "1", h: 1 })), thread(row("b", { title: "Second", item: "2", h: 2 }))],
        "/api/pr/detail": prDetail(),
        "/api/connections": [],
      },
    });
    renderInbox();
    await screen.findByText("First");

    fireEvent.keyDown(window, { key: "j" });
    expect(rowOf("First")).toHaveAttribute("data-selected", "true");
    fireEvent.keyDown(window, { key: "j" });
    expect(rowOf("Second")).toHaveAttribute("data-selected", "true");
    fireEvent.keyDown(window, { key: "k" });
    expect(rowOf("First")).toHaveAttribute("data-selected", "true");
    expect(rowOf("Second")).not.toHaveAttribute("data-selected");

    // Enter opens the selected row; j then moves the pane to the next thread.
    fireEvent.keyDown(window, { key: "Enter" });
    const pane = await screen.findByRole("region", { name: "Pull request" });
    await waitFor(() => expect(within(pane).getByTestId("triage-strip")).toHaveTextContent("Inbox · 1 of 2"));
    fireEvent.keyDown(window, { key: "j" });
    await waitFor(() => expect(within(pane).getByTestId("triage-strip")).toHaveTextContent("Inbox · 2 of 2"));
    expect(rowOf("Second")).toHaveAttribute("data-selected", "true");
  });

  it("offers open, provider link, copy, read and done on the row menu", async () => {
    mockFetch({ get: { "/api/inbox": [thread(row("a", { title: "Ping" }))], "/api/connections": [] } });
    renderInbox();
    await screen.findByText("Ping");

    fireEvent.contextMenu(rowOf("Ping"));
    const menu = screen.getByRole("menu", { name: "Notification actions" });
    const labels = [...menu.querySelectorAll('[role="menuitem"]')].map((m) => m.querySelector(".ft-menu-label")?.textContent);
    expect(labels).toEqual(["Open", "Open on GitHub", "Copy link", "Mark read", "Done"]);
  });

  it("says so when a view is empty", async () => {
    mockFetch({ get: { "/api/inbox": [thread(row("c", { kind: "Comment", title: "Followed" }))], "/api/connections": [] } });
    renderInbox();
    expect(await screen.findByText("Nothing needs you")).toBeInTheDocument();
  });
});

describe("Notifications: the detail a feed left out", () => {
  afterEach(() => vi.useRealTimers());

  const detailPosts = (posts: { url: string; body: unknown }[]) => posts.filter((p) => p.url.includes("/api/notification/detail"));
  const inboxGets = (fetchMock: ReturnType<typeof mockFetch>["fetchMock"]) =>
    fetchMock.mock.calls.filter(([url, init]) => String(url).includes("/api/inbox") && !init?.method).length;
  const answer = {
    actor: "Priya Nair",
    excerpt: "Can we land this before the freeze?",
    target: { kind: "Comment", id: "c2" },
    phrase: "Priya mentioned you",
    why: "Priya @-mentioned you in a comment",
  };

  it("a held j across twenty rows fetches one detail, for the row it stops on, and patches it in without refetching the inbox", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const threads = Array.from({ length: 20 }, (_, i) =>
      thread(row(`n${i}`, { kind: "Mention", phrase: "mentioned you", title: `Row ${i}`, item: String(100 + i), detailRef: `r${i}`, h: 1 + i / 100 })),
    );
    const { posts, fetchMock } = mockFetch({
      get: { "/api/inbox": threads, "/api/connections": [] },
      onPost: (url) => (url.includes("/api/notification/detail") ? answer : { ok: true }),
    });
    renderInbox();
    await screen.findByText("Row 0");
    const gets = inboxGets(fetchMock);

    // A held key: a row every 30ms, never resting long enough to fetch.
    for (let i = 0; i < 20; i++) {
      fireEvent.keyDown(window, { key: "j" });
      act(() => vi.advanceTimersByTime(30));
    }
    expect(rowOf("Row 19")).toHaveAttribute("data-selected", "true");
    expect(detailPosts(posts)).toHaveLength(0);

    // It rests on the last row: one call, for that row.
    act(() => vi.advanceTimersByTime(300));
    await waitFor(() => expect(detailPosts(posts)).toHaveLength(1));
    expect(detailPosts(posts)[0].body).toEqual({ conn: "c", id: "n19" });
    // The row reads the server's recomputed phrase and the excerpt, written into the cached inbox.
    expect(await within(rowOf("Row 19")).findByText(/Priya mentioned you · “Can we land this before the freeze\?”/)).toBeInTheDocument();
    expect(inboxGets(fetchMock)).toBe(gets);
  });

  it("a detail that failed is not asked for again when the row is selected again", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { posts } = mockFetch({
      get: {
        "/api/inbox": [
          thread(row("a", { kind: "Mention", title: "First", item: "1", detailRef: "r", h: 1 })),
          thread(row("b", { title: "Second", item: "2", h: 2 })),
        ],
        "/api/connections": [],
      },
      onPost: (url) => {
        if (url.includes("/api/notification/detail")) throw new Error("rate limited");
        return { ok: true };
      },
    });
    renderInbox();
    await screen.findByText("First");

    fireEvent.keyDown(window, { key: "j" });
    act(() => vi.advanceTimersByTime(300));
    await waitFor(() => expect(detailPosts(posts)).toHaveLength(1));

    // Off and back on, each time long enough to settle.
    fireEvent.keyDown(window, { key: "j" });
    act(() => vi.advanceTimersByTime(400));
    fireEvent.keyDown(window, { key: "k" });
    act(() => vi.advanceTimersByTime(400));
    expect(rowOf("First")).toHaveAttribute("data-selected", "true");
    await act(async () => {
      await Promise.resolve();
    });
    expect(detailPosts(posts)).toHaveLength(1);
  });

  it("opens a mention at the comment its detail found, ringing that comment only", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockFetch({
      get: {
        "/api/inbox": [thread(row("m", { kind: "Mention", phrase: "mentioned you", title: "Idempotency keys", detailRef: "r" }))],
        "/api/pr/detail": prDetail({
          threads: [
            {
              id: "pr-10",
              comments: [comment("c1", "Opening remark"), comment("c2", "@sam can we land this today?"), comment("c3", "Later reply")],
              file_path: null,
              line: null,
              is_resolved: false,
            },
          ],
        }),
        "/api/connections": [],
      },
      onPost: (url) => (url.includes("/api/notification/detail") ? answer : { ok: true }),
    });
    renderInbox();
    await screen.findByText("Idempotency keys");

    fireEvent.keyDown(window, { key: "j" });
    act(() => vi.advanceTimersByTime(300));
    await within(rowOf("Idempotency keys")).findByText(/Priya mentioned you/);

    // The row was listed pointing at the PR as a whole; the cache now points at the comment.
    fireEvent.keyDown(window, { key: "Enter" });
    const pane = await screen.findByRole("region", { name: "Pull request" });
    await within(pane).findByText("@sam can we land this today?");
    const marked = pane.querySelectorAll("[data-target]");
    expect(marked).toHaveLength(1);
    expect(marked[0]).toHaveAttribute("data-comment-id", "c2");
    expect(within(pane).getByTestId("triage-strip")).toHaveTextContent("Priya mentioned you");
  });

  it("a mention clicked before its detail arrives moves the open pane to the comment once it does", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockFetch({
      get: {
        "/api/inbox": [thread(row("m", { kind: "Mention", phrase: "mentioned you", title: "Idempotency keys", detailRef: "r" }))],
        "/api/pr/detail": prDetail({
          threads: [
            {
              id: "pr-10",
              comments: [comment("c1", "Opening remark"), comment("c2", "@sam can we land this today?"), comment("c3", "Later reply")],
              file_path: null,
              line: null,
              is_resolved: false,
            },
          ],
        }),
        "/api/connections": [],
      },
      onPost: (url) => (url.includes("/api/notification/detail") ? answer : { ok: true }),
    });
    renderInbox();
    await userEvent.click(await screen.findByText("Idempotency keys"));
    const pane = await screen.findByRole("region", { name: "Pull request" });
    await within(pane).findByText("@sam can we land this today?");
    expect(pane.querySelectorAll("[data-target]")).toHaveLength(0);

    // The pane settles on the thread, the detail lands, and the mention is ringed.
    act(() => vi.advanceTimersByTime(300));
    await waitFor(() => expect(pane.querySelectorAll("[data-target]")).toHaveLength(1));
    expect(pane.querySelector("[data-target]")).toHaveAttribute("data-comment-id", "c2");
  });
});

describe("useInboxActions().open", () => {
  it("opens the thread as the cache holds it now, not the caller's older copy", async () => {
    const stale = thread(row("m", { kind: "Mention", phrase: "mentioned you", title: "Idempotency keys", detailRef: "r" }));
    mockFetch({
      get: {
        "/api/pr/detail": prDetail({
          threads: [
            {
              id: "pr-10",
              comments: [comment("c1", "Opening remark"), comment("c2", "@sam can we land this today?"), comment("c3", "Later reply")],
              file_path: null,
              line: null,
              is_resolved: false,
            },
          ],
        }),
        "/api/connections": [],
      },
    });
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    qc.setQueryData(["inbox"], [stale]);
    // The detail landed after the caller took its copy, and pointed the thread at a comment.
    patchInboxDetailInCache(qc, "c", "m", { target: { kind: "Comment", id: "c2" }, phrase: "Priya mentioned you", why: "w" });
    let actions!: InboxActions;
    function Grab() {
      actions = useInboxActions();
      return null;
    }
    render(
      <QueryClientProvider client={qc}>
        <PrDetailProvider>
          <Grab />
        </PrDetailProvider>
      </QueryClientProvider>,
    );

    let opened = false;
    act(() => {
      opened = actions.open(stale, [stale]);
    });
    expect(opened).toBe(true);
    const pane = await screen.findByRole("region", { name: "Pull request" });
    await within(pane).findByText("@sam can we land this today?");
    const marked = pane.querySelectorAll("[data-target]");
    expect(marked).toHaveLength(1);
    expect(marked[0]).toHaveAttribute("data-comment-id", "c2");
  });
});

describe("Notifications: a pane that won't leave", () => {
  afterEach(() => setPaneGuard("pr", null));

  it("j doesn't open or mark read the next thread, and e marks done but stays", async () => {
    const { posts } = mockFetch({
      get: {
        "/api/inbox": [thread(row("a", { title: "First", item: "1", h: 1 })), thread(row("b", { title: "Second", item: "2", h: 2 }))],
        "/api/pr/detail": prDetail(),
        "/api/connections": [],
      },
    });
    renderInbox();
    await userEvent.click(await screen.findByText("First"));
    const pane = await screen.findByRole("region", { name: "Pull request" });
    await waitFor(() => expect(within(pane).getByTestId("triage-strip")).toHaveTextContent("Inbox · 1 of 2"));
    await waitFor(() => expect(posts.some((p) => p.url.includes("/api/notification/read-many"))).toBe(true));
    const reads = () => posts.filter((p) => p.url.includes("/api/notification/read-many")).map((p) => p.body);
    expect(reads()).toEqual([{ conn: "c", ids: ["a"] }]);

    // Unsubmitted work in the pane: its guard refuses to let it go.
    setPaneGuard("pr", () => false);
    fireEvent.keyDown(window, { key: "j" });
    expect(reads()).toEqual([{ conn: "c", ids: ["a"] }]);
    expect(within(pane).getByTestId("triage-strip")).toHaveTextContent("Inbox · 1 of 2");

    fireEvent.keyDown(window, { key: "e" });
    await waitFor(() => expect(posts.find((p) => p.url.includes("/api/notification/done"))?.body).toEqual({ conn: "c", ids: ["a"] }));
    expect(reads()).toEqual([{ conn: "c", ids: ["a"] }]);
    // The pane stays where it was rather than closing or pretending to move on.
    expect(screen.getByRole("region", { name: "Pull request" })).toBeInTheDocument();
    expect(within(pane).getByTestId("triage-strip")).toHaveTextContent("Inbox · 1 of 2");
  });
});

describe("dayGroup", () => {
  it("files by the newest event's day", () => {
    const now = new Date(2026, 9, 15, 12, 0, 0);
    expect(dayGroup(new Date(2026, 9, 15, 1).toISOString(), now)).toBe("Today");
    expect(dayGroup(new Date(2026, 9, 14, 23).toISOString(), now)).toBe("Yesterday");
    expect(dayGroup(new Date(2026, 9, 13, 23).toISOString(), now)).toBe("Earlier");
    expect(dayGroup(null, now)).toBe("Earlier");
  });
});
