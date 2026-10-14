import { useCallback, useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockFetch, renderWithClient } from "../../test/util";
import { ContextMenu, type MenuAnchor } from "../ui";
import type { RowMenuOpts } from "./shared";

const openPr = vi.fn();
vi.mock("../PrDetail", () => ({ usePrOpener: () => openPr, useCurrentPr: () => null }));

// Imported after the mock so the hook picks it up.
import { usePrRowMenu } from "./pr";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const prRow = (edit: (pr: any) => void = () => {}): any => {
  const pull_request = {
    id: "42",
    repository: "acme/pay",
    number: 1501,
    title: "Refactor the retry queue",
    description: null,
    author: { id: "me", display_name: "Me", handle: "me", avatar_url: null },
    status: "Open",
    is_draft: false,
    source_ref: "feat/x",
    target_ref: "main",
    reviewers: [{ user: { id: "u1", display_name: "Ada Lovelace", handle: "ada", avatar_url: null }, vote: "NoVote", is_required: false }],
    labels: [],
    checks: "Passed",
    check_summary: null,
    mergeable: "Mergeable",
    changed_files: 0,
    additions: 0,
    deletions: 0,
    created_at: null,
    updated_at: null,
    url: "https://github.com/acme/pay/pull/1501",
  };
  edit(pull_request);
  return { connection_id: "c", connection: "GH", provider: "GitHub", pull_request };
};

/** Opens the built menu straight away, the way a right-clicked row would. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function Harness({ row, opts }: { row: any; opts?: RowMenuOpts }) {
  const build = usePrRowMenu();
  const [anchor, setAnchor] = useState<MenuAnchor | null>({ x: 10, y: 10 });
  const items = useCallback(() => build(row, opts), [build, row, opts]);
  return <ContextMenu anchor={anchor} onClose={() => setAnchor(null)} label="Pull request actions" items={items} />;
}

const item = (name: string) => screen.getByRole("menuitem", { name });
const labels = () =>
  within(screen.getByRole("menu", { name: "Pull request actions" }))
    .getAllByRole("menuitem")
    .map((m) => m.querySelector(".ft-menu-label")?.textContent);

describe("usePrRowMenu", () => {
  it("an open PR offers Approve, Request changes, Merge, the review group and Close", () => {
    mockFetch({});
    renderWithClient(<Harness row={prRow()} />);
    expect(labels()).toEqual([
      "Open",
      "Open in GitHub",
      "Copy link",
      "Copy #1501",
      "Approve",
      "Request changes",
      "Merge",
      "Request reviewer",
      "Convert to draft",
      "Close pull request",
    ]);
    expect(item("Merge")).not.toHaveAttribute("aria-disabled");
    expect(screen.getAllByRole("separator")).toHaveLength(3);
  });

  it("Open goes to the pane", async () => {
    mockFetch({});
    renderWithClient(<Harness row={prRow()} />);
    await userEvent.click(item("Open"));
    expect(openPr).toHaveBeenCalledWith({ conn: "c", repo: "acme/pay", id: "42" });
  });

  it("disables Merge with the reason while checks are running", () => {
    mockFetch({});
    renderWithClient(<Harness row={prRow((p) => (p.checks = "Pending"))} />);
    expect(item("Merge")).toHaveAttribute("aria-disabled", "true");
    expect(item("Merge")).toHaveAttribute("title", "Checks are still running");
  });

  it("disables the links with a reason when the provider gave none", () => {
    mockFetch({});
    renderWithClient(<Harness row={prRow((p) => (p.url = null))} />);
    expect(item("Open in GitHub")).toHaveAttribute("title", "No link from the provider");
    expect(item("Copy link")).toHaveAttribute("aria-disabled", "true");
  });

  it("Approve posts the vote and tells the caller it acted", async () => {
    const { posts } = mockFetch({});
    const onActed = vi.fn();
    renderWithClient(<Harness row={prRow()} opts={{ onActed }} />);
    await userEvent.click(item("Approve"));
    expect(onActed).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({ url: "/api/pr/vote", body: { conn: "c", repo: "acme/pay", id: "42", vote: "Approved" } });
  });

  it("a draft offers Mark ready for review and no Approve", async () => {
    const { posts } = mockFetch({});
    renderWithClient(<Harness row={prRow((p) => ((p.is_draft = true), (p.status = "Draft")))} />);
    expect(labels()).toEqual(["Open", "Open in GitHub", "Copy link", "Copy #1501", "Mark ready for review", "Close pull request"]);
    expect(screen.queryByRole("menuitem", { name: "Approve" })).toBeNull();
    await userEvent.click(item("Mark ready for review"));
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({ url: "/api/pr/draft", body: { conn: "c", repo: "acme/pay", id: "42", draft: false } });
  });

  it("a merged PR offers Revert only", async () => {
    const { posts } = mockFetch({});
    renderWithClient(<Harness row={prRow((p) => (p.status = "Merged"))} />);
    expect(labels()).toEqual(["Open", "Open in GitHub", "Copy link", "Copy #1501", "Revert"]);
    await userEvent.click(item("Revert"));
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({ url: "/api/pr/revert", body: { conn: "c", repo: "acme/pay", id: "42" } });
  });

  it("a closed PR offers Reopen", async () => {
    const { posts } = mockFetch({});
    renderWithClient(<Harness row={prRow((p) => (p.status = "Closed"))} />);
    expect(labels()).toEqual(["Open", "Open in GitHub", "Copy link", "Copy #1501", "Reopen"]);
    await userEvent.click(item("Reopen"));
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({ url: "/api/pr/closed", body: { conn: "c", repo: "acme/pay", id: "42", closed: false } });
  });

  it("Close takes two clicks and posts closed:true", async () => {
    const { posts } = mockFetch({});
    const onActed = vi.fn();
    renderWithClient(<Harness row={prRow()} opts={{ onActed }} />);
    await userEvent.click(item("Close pull request"));
    expect(posts).toHaveLength(0);
    expect(onActed).not.toHaveBeenCalled();
    await userEvent.click(item("Click again to close"));
    expect(onActed).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({ url: "/api/pr/closed", body: { conn: "c", repo: "acme/pay", id: "42", closed: true } });
  });

  it("the reviewer submenu loads candidates and requests the one picked", async () => {
    const { posts, fetchMock } = mockFetch({
      get: {
        "/api/pr/reviewers": [
          { id: "u1", display_name: "Ada Lovelace", handle: "ada", avatar_url: null },
          { id: "u2", display_name: "Grace Hopper", handle: "grace", avatar_url: null },
        ],
      },
    });
    renderWithClient(<Harness row={prRow()} />);
    await userEvent.click(item("Request reviewer"));
    const grace = await screen.findByRole("menuitem", { name: "Grace Hopper" });
    expect(String(fetchMock.mock.calls[0][0])).toBe("/api/pr/reviewers?conn=c&id=42&repo=acme%2Fpay");
    // Ada is already a reviewer on the row.
    const ada = screen.getByRole("menuitemcheckbox", { name: "Ada Lovelace" });
    expect(ada).toHaveAttribute("aria-checked", "true");
    expect(ada).toHaveAttribute("title", "Already requested");
    await userEvent.click(grace);
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({ url: "/api/pr/request-reviewer", body: { conn: "c", repo: "acme/pay", id: "42", user_id: "u2" } });
  });

  it("Copy #N copies the number", async () => {
    mockFetch({});
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, "writeText");
    renderWithClient(<Harness row={prRow()} />);
    await user.click(item("Copy #1501"));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("#1501"));
  });

  it("appends the caller's extra entries last", () => {
    mockFetch({});
    renderWithClient(<Harness row={prRow((p) => (p.status = "Merged"))} opts={{ extra: [{ label: "Dismiss from Command Center" }] }} />);
    expect(labels().at(-1)).toBe("Dismiss from Command Center");
  });
});
