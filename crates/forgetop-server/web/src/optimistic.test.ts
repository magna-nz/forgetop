import { describe, expect, it } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { prDetailKey, wiDetailKey } from "./api";
import {
  addPrReply,
  addPrThreads,
  dropConnectionFromCache,
  isLocalId,
  localComment,
  localThread,
  markNotificationReadInCache,
  meAsUser,
  patchPullRequest,
  patchWorkItem,
  setConnectionScopeInCache,
  withVote,
} from "./optimistic";
import type {
  ConnectionRow,
  LaunchpadResponse,
  NotifRow,
  PrDetail as PrDetailData,
  PrRow,
  PullRequest,
  WiDetail as WiDetailData,
  WiRow,
  WorkItem,
} from "./types";

const row = (connection_id: string, id: string) => ({ connection_id, id });

describe("dropConnectionFromCache", () => {
  const seed = () => {
    const qc = new QueryClient();
    qc.setQueryData(["connections"], [{ id: "gone" }, { id: "kept" }]);
    qc.setQueryData(["health"], [row("gone", "h1"), row("kept", "h2")]);
    qc.setQueryData(["prs", "all"], [row("gone", "p1"), row("kept", "p2")]);
    qc.setQueryData(["prs", "mine"], [row("gone", "p3")]);
    qc.setQueryData(["work-items"], [row("gone", "w1"), row("kept", "w2")]);
    qc.setQueryData(["pipelines"], [row("gone", "l1")]);
    qc.setQueryData(["notifications"], [row("gone", "n1"), row("kept", "n2")]);
    qc.setQueryData(["launchpad"], {
      rows: [row("gone", "x1"), row("kept", "x2")],
      more: {},
    } as unknown as LaunchpadResponse);
    return qc;
  };

  const connectionsOf = (qc: QueryClient, key: unknown[]) =>
    (qc.getQueryData(key) as { connection_id: string }[]).map((r) => r.connection_id);

  it("drops the connection's rows from every list that carries them", () => {
    const qc = seed();

    dropConnectionFromCache(qc, "gone");

    expect((qc.getQueryData(["connections"]) as { id: string }[]).map((c) => c.id)).toEqual(["kept"]);
    expect(connectionsOf(qc, ["health"])).toEqual(["kept"]);
    expect(connectionsOf(qc, ["work-items"])).toEqual(["kept"]);
    expect(connectionsOf(qc, ["notifications"])).toEqual(["kept"]);
    expect(qc.getQueryData(["pipelines"])).toEqual([]);
  });

  // Query keys match by prefix, so a view the user visited earlier must be cleaned too — not
  // just the one currently on screen.
  it("cleans every PR view, not only the visible one", () => {
    const qc = seed();

    dropConnectionFromCache(qc, "gone");

    expect(connectionsOf(qc, ["prs", "all"])).toEqual(["kept"]);
    expect(qc.getQueryData(["prs", "mine"])).toEqual([]);
  });

  it("filters the Launchpad's rows while leaving the rest of the response intact", () => {
    const qc = seed();

    dropConnectionFromCache(qc, "gone");

    const lp = qc.getQueryData(["launchpad"]) as LaunchpadResponse;
    expect(lp.rows.map((r) => r.connection_id)).toEqual(["kept"]);
    expect(lp.more).toBeDefined();
  });

  it("leaves other connections alone and tolerates queries that were never fetched", () => {
    const qc = new QueryClient();
    qc.setQueryData(["prs", "all"], [row("kept", "p1")]);

    expect(() => dropConnectionFromCache(qc, "gone")).not.toThrow();
    expect(connectionsOf(qc, ["prs", "all"])).toEqual(["kept"]);
    expect(qc.getQueryData(["launchpad"])).toBeUndefined();
  });
});

describe("markNotificationReadInCache", () => {
  const notif = (connection_id: string, id: string, unread: boolean) =>
    ({ connection_id, notification: { id, unread, title: id } }) as unknown as NotifRow;

  const unreadFlags = (qc: QueryClient) =>
    (qc.getQueryData(["notifications"]) as NotifRow[]).map((r) => [r.notification.id, r.notification.unread]);

  // Ids are only unique *within* a connection, so a bare id match would dim a stranger's
  // notification on another connection at the same time.
  it("flips only the addressed row, leaving the same id on another connection alone", () => {
    const qc = new QueryClient();
    qc.setQueryData(["notifications"], [notif("c1", "n1", true), notif("c2", "n1", true), notif("c1", "n2", true)]);

    markNotificationReadInCache(qc, "c1", "n1");

    expect(unreadFlags(qc)).toEqual([
      ["n1", false],
      ["n1", true],
      ["n2", true],
    ]);
  });

  it("leaves the cached rows' identity intact for the rows it didn't touch", () => {
    const qc = new QueryClient();
    const untouched = notif("c1", "n2", true);
    qc.setQueryData(["notifications"], [notif("c1", "n1", true), untouched]);

    markNotificationReadInCache(qc, "c1", "n1");

    // React-Query re-renders on reference identity: the edited row must be a new object, and the
    // rest must not be, or every row in the inbox animates on one mark-read.
    const rows = qc.getQueryData(["notifications"]) as NotifRow[];
    expect(rows[1]).toBe(untouched);
    expect(rows[0].notification.unread).toBe(false);
  });

  it("tolerates the notifications query never having been fetched", () => {
    const qc = new QueryClient();

    expect(() => markNotificationReadInCache(qc, "c1", "n1")).not.toThrow();
    expect(qc.getQueryData(["notifications"])).toBeUndefined();
  });
});

describe("patchWorkItem", () => {
  const item = (id: string, over: Partial<WorkItem> = {}): WorkItem =>
    ({
      id,
      repository: "acme/pay",
      title: `Item ${id}`,
      description: "Original description",
      state: "In Progress",
      state_category: "Started",
      assignee: null,
      ...over,
    }) as WorkItem;

  const ref = { conn: "c1", repo: "acme/pay", id: "w1" };

  const seed = () => {
    const qc = new QueryClient();
    qc.setQueryData(wiDetailKey(ref), { work_item: item("w1"), threads: [], timeline: [] });
    qc.setQueryData(["work-items"], [
      { connection_id: "c1", work_item: item("w1") },
      { connection_id: "c2", work_item: item("w1") },
      { connection_id: "c1", work_item: item("w2") },
    ]);
    qc.setQueryData(["launchpad"], {
      rows: [
        { kind: "wi", connection_id: "c1", work_item: item("w1") },
        { kind: "pr", connection_id: "c1", pull_request: { id: "w1" } },
      ],
      more: {},
    } as unknown as LaunchpadResponse);
    return qc;
  };

  const listTitles = (qc: QueryClient) => (qc.getQueryData(["work-items"]) as WiRow[]).map((r) => r.work_item.title);

  it("patches the item in the detail, list and launchpad caches at once", () => {
    const qc = seed();

    patchWorkItem(qc, ref, { title: "Renamed" });

    expect((qc.getQueryData(wiDetailKey(ref)) as WiDetailData).work_item.title).toBe("Renamed");
    expect(listTitles(qc)[0]).toBe("Renamed");
    const lp = qc.getQueryData(["launchpad"]) as LaunchpadResponse;
    expect(lp.rows[0].kind === "wi" && lp.rows[0].work_item.title).toBe("Renamed");
  });

  // The same id on another connection, and another item on the same one, are different items.
  it("touches only the addressed item", () => {
    const qc = seed();

    patchWorkItem(qc, ref, { title: "Renamed" });

    expect(listTitles(qc)).toEqual(["Renamed", "Item w1", "Item w2"]);
  });

  // A connection spans an account, so `#7` can name a work item in more than one repository.
  it("does not patch a same-id item in a different repository", () => {
    const qc = new QueryClient();
    qc.setQueryData(["work-items"], [{ connection_id: "c1", work_item: item("w1", { repository: "acme/web" }) }]);

    patchWorkItem(qc, ref, { title: "Renamed" });

    expect(listTitles(qc)).toEqual(["Item w1"]);
  });

  // /api/wi/states answers with raw state names only, so the category that drives the state
  // colour and the Command Center bucket isn't derivable here — it must survive untouched and
  // wait for the refetch rather than be guessed.
  it("leaves state_category alone when the state name moves", () => {
    const qc = seed();

    patchWorkItem(qc, ref, { state: "Done" });

    const wi = (qc.getQueryData(wiDetailKey(ref)) as WiDetailData).work_item;
    expect(wi.state).toBe("Done");
    expect(wi.state_category).toBe("Started");
  });

  // The edit form omits a field it didn't change; the patch has to omit it too, or an untouched
  // description gets rewritten locally with a copy that may already be stale.
  it("writes only the fields the patch carries", () => {
    const qc = seed();

    patchWorkItem(qc, ref, { title: "Renamed" });

    const wi = (qc.getQueryData(wiDetailKey(ref)) as WiDetailData).work_item;
    expect(wi.description).toBe("Original description");
    expect(wi.assignee).toBeNull();
  });

  it("assigns and unassigns with the full user object the picker supplied", () => {
    const qc = seed();
    const user = { id: "u1", display_name: "Priya Nair" };

    patchWorkItem(qc, ref, { assignee: user });
    expect((qc.getQueryData(wiDetailKey(ref)) as WiDetailData).work_item.assignee).toEqual(user);

    patchWorkItem(qc, ref, { assignee: null });
    expect((qc.getQueryData(wiDetailKey(ref)) as WiDetailData).work_item.assignee).toBeNull();
  });

  it("tolerates caches that were never fetched", () => {
    const qc = new QueryClient();

    expect(() => patchWorkItem(qc, ref, { title: "Renamed" })).not.toThrow();
    expect(qc.getQueryData(wiDetailKey(ref))).toBeUndefined();
    expect(qc.getQueryData(["work-items"])).toBeUndefined();
    expect(qc.getQueryData(["launchpad"])).toBeUndefined();
  });
});

describe("setConnectionScopeInCache", () => {
  const scopes = (qc: QueryClient) => (qc.getQueryData(["connections"]) as ConnectionRow[]).map((c) => c.repo_scope);

  it("replaces one connection's scope and leaves the others alone", () => {
    const qc = new QueryClient();
    qc.setQueryData(["connections"], [
      { id: "c1", repo_scope: ["acme/pay"] },
      { id: "c2", repo_scope: ["other/repo"] },
    ]);

    setConnectionScopeInCache(qc, "c1", ["acme/pay", "acme/web"]);

    expect(scopes(qc)).toEqual([["acme/pay", "acme/web"], ["other/repo"]]);
  });

  // `[]` means "the user chose none" — a real scope, not an absent one — so it must be stored as
  // an empty array rather than skipped, or the label falls back to the legacy single repository.
  it("stores an empty scope rather than treating it as unset", () => {
    const qc = new QueryClient();
    qc.setQueryData(["connections"], [{ id: "c1", repository: "acme/pay", repo_scope: null }]);

    setConnectionScopeInCache(qc, "c1", []);

    expect(scopes(qc)).toEqual([[]]);
  });

  it("tolerates the connections query never having been fetched", () => {
    const qc = new QueryClient();

    expect(() => setConnectionScopeInCache(qc, "c1", ["acme/pay"])).not.toThrow();
    expect(qc.getQueryData(["connections"])).toBeUndefined();
  });
});

describe("patchPullRequest / withVote", () => {
  const priya = { id: "u1", display_name: "Priya Nair", handle: "priya", avatar_url: null };
  const pr = (id: string, repository = "acme/pay"): PullRequest =>
    ({ id, repository, title: "t", reviewers: [{ user: priya, vote: "NoVote", is_required: true }] }) as unknown as PullRequest;
  const ref = { conn: "c", repo: "acme/pay", id: "7" };

  const seed = () => {
    const qc = new QueryClient();
    qc.setQueryData(prDetailKey(ref), { pull_request: pr("7"), threads: [] } as unknown as PrDetailData);
    qc.setQueryData(["prs", "review"], [
      { connection_id: "c", pull_request: pr("7") },
      { connection_id: "c", pull_request: pr("7", "acme/other") },
    ] as unknown as PrRow[]);
    qc.setQueryData(["launchpad"], {
      rows: [{ kind: "pr", connection_id: "c", pull_request: pr("7") }],
      more: {},
    } as unknown as LaunchpadResponse);
    return qc;
  };
  const myVote = (p: PullRequest) => p.reviewers.find((r) => r.user.handle === "me")?.vote;

  it("shows your vote on the pane, every PR list and the Command Center — only on that repository's PR", () => {
    const qc = seed();

    patchPullRequest(qc, ref, (p) => withVote(p, "me", "Approved"));

    expect(myVote((qc.getQueryData(prDetailKey(ref)) as PrDetailData).pull_request)).toBe("Approved");
    const rows = qc.getQueryData(["prs", "review"]) as PrRow[];
    expect(myVote(rows[0].pull_request)).toBe("Approved");
    expect(myVote(rows[1].pull_request)).toBeUndefined();
    const lp = qc.getQueryData(["launchpad"]) as LaunchpadResponse;
    expect(lp.rows[0].kind === "pr" && myVote(lp.rows[0].pull_request)).toBe("Approved");
  });

  it("updates your existing reviewer entry rather than adding a second one", () => {
    const asReviewer = { ...pr("7"), reviewers: [{ user: { ...priya, id: "u9", display_name: "Sam", handle: "me" }, vote: "NoVote", is_required: true }] } as PullRequest;

    const voted = withVote(asReviewer, "me", "Rejected");

    expect(voted.reviewers).toHaveLength(1);
    expect(voted.reviewers[0]).toMatchObject({ vote: "Rejected", is_required: true });
  });
});

describe("placeholder comments", () => {
  const ref = { conn: "c", id: "7" };
  const seed = () => {
    const qc = new QueryClient();
    qc.setQueryData(prDetailKey(ref), {
      pull_request: {},
      threads: [{ id: "t1", comments: [{ id: "c1", author: meAsUser("bob"), body: "why?" }], is_resolved: false }],
    } as unknown as PrDetailData);
    return qc;
  };
  const threads = (qc: QueryClient) => (qc.getQueryData(prDetailKey(ref)) as PrDetailData).threads;

  it("adds a comment and a review's line comments as new threads, marked as local", () => {
    const qc = seed();

    addPrThreads(qc, ref, [localThread(meAsUser("me"), "LGTM"), localThread(meAsUser("me"), "nit", { path: "a.rs", line: 3 })]);

    const [, top, line] = threads(qc);
    expect(top).toMatchObject({ file_path: null, comments: [{ body: "LGTM", author: { handle: "me" } }] });
    expect(line).toMatchObject({ file_path: "a.rs", line: 3 });
    expect(isLocalId(top.id) && isLocalId(line.id)).toBe(true);
    expect(top.id).not.toBe(line.id);
  });

  it("adds a reply to its own thread", () => {
    const qc = seed();

    addPrReply(qc, ref, "t1", localComment(meAsUser(), "because"));

    expect(threads(qc)[0].comments.map((c) => [c.body, c.author.display_name])).toEqual([
      ["why?", "bob"],
      ["because", "You"],
    ]);
  });
});
