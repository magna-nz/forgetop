import { useCallback, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockFetch, renderWithClient } from "../../test/util";
import { ContextMenu, type MenuAnchor } from "../ui";
import type { RowMenuOpts } from "./shared";

const openWi = vi.fn();
vi.mock("../WiDetail", () => ({ useWiOpener: () => openWi, useCurrentWi: () => null }));

// Imported after the mock so the hook picks it up.
import { useWiRowMenu } from "./wi";

const sam = { id: "me", display_name: "Sam Rivera", handle: "sam", avatar_url: null };
const priya = { id: "u1", display_name: "Priya Nair", handle: "priya", avatar_url: null };

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const wiRow = (edit: (wi: any) => void = () => {}): any => {
  const work_item = {
    id: "w1",
    identifier: "ENG-1",
    repository: "acme/pay",
    title: "Retry the ledger sync",
    description: null,
    state: "In Progress",
    state_category: "Started",
    work_item_type: "Bug",
    assignee: sam,
    created_at: null,
    updated_at: null,
    url: "https://github.com/acme/pay/issues/1",
  };
  edit(work_item);
  return { connection_id: "c", connection: "GH", provider: "GitHub", work_item };
};

const ref = { conn: "c", repo: "acme/pay", id: "w1" };

/** Opens the built menu straight away, the way a right-clicked row would. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function Harness({ row, opts }: { row: any; opts?: RowMenuOpts }) {
  const build = useWiRowMenu();
  const [anchor, setAnchor] = useState<MenuAnchor | null>({ x: 10, y: 10 });
  const items = useCallback(() => build(row, opts), [build, row, opts]);
  return <ContextMenu anchor={anchor} onClose={() => setAnchor(null)} label="Work item actions" items={items} />;
}

const item = (name: string) => screen.getByRole("menuitem", { name });
/** The item labels of a menu panel, plain items and checkbox items alike, in order. */
const labelsIn = (menu: HTMLElement) =>
  [...menu.querySelectorAll('[role="menuitem"], [role="menuitemcheckbox"]')].map((m) => m.querySelector(".ft-menu-label")?.textContent);
const labels = () => labelsIn(screen.getByRole("menu", { name: "Work item actions" }));

const fixtures = (hidden: string[] = []) => ({
  "/api/prefs": { hidden_work_item_states: hidden },
  "/api/wi/states": ["Todo", "In Progress", "Done"],
  "/api/wi/assignees": [sam, priya],
});

beforeEach(() => openWi.mockClear());

describe("useWiRowMenu", () => {
  it("lists navigate+copy, act and the view preference, in that order", async () => {
    mockFetch({ get: fixtures() });
    renderWithClient(<Harness row={wiRow()} />);
    expect(labels()).toEqual([
      "Open",
      "Open in GitHub",
      "Copy link",
      "Copy ENG-1",
      "Move to",
      "Assign",
      "Edit title & description…",
      "Comment…",
      'Hide "In Progress" items',
    ]);
    expect(screen.getAllByRole("separator")).toHaveLength(2);
    // Until the preferences load, the toggle is there but disabled with the reason.
    const hide = screen.getByRole("menuitemcheckbox", { name: 'Hide "In Progress" items' });
    expect(hide).toHaveAttribute("aria-disabled", "true");
    expect(hide).toHaveAttribute("title", "Loading preferences…");
    await waitFor(() => expect(screen.getByRole("menuitemcheckbox", { name: 'Hide "In Progress" items' })).not.toHaveAttribute("aria-disabled"));
  });

  it("keeps the link items, disabled with the reason, when the provider gave no url; omits Copy without an identifier", () => {
    mockFetch({ get: fixtures() });
    renderWithClient(
      <Harness
        row={wiRow((wi) => {
          wi.url = null;
          wi.identifier = null;
        })}
      />,
    );
    expect(item("Open in GitHub")).toHaveAttribute("title", "No link from the provider");
    expect(item("Copy link")).toHaveAttribute("aria-disabled", "true");
    expect(labels()).not.toContain("Copy ENG-1");
  });

  it("Open goes to the pane without an intent", async () => {
    mockFetch({ get: fixtures() });
    renderWithClient(<Harness row={wiRow()} />);
    await userEvent.click(item("Open"));
    expect(openWi).toHaveBeenCalledWith(ref);
  });

  it("Move to lists the states with the current one checked and disabled, and posts /api/wi/state", async () => {
    const onActed = vi.fn();
    const { posts } = mockFetch({ get: fixtures() });
    renderWithClient(<Harness row={wiRow()} opts={{ onActed }} />);
    await userEvent.click(item("Move to"));
    const sub = await screen.findByRole("menu", { name: "Move to" });
    await within(sub).findByRole("menuitem", { name: "Done" });
    const current = within(sub).getByRole("menuitemcheckbox", { name: "In Progress" });
    expect(current).toHaveAttribute("aria-checked", "true");
    expect(current).toHaveAttribute("title", "Current state");
    expect(within(sub).getByRole("menuitem", { name: "Todo" })).not.toHaveAttribute("aria-disabled");

    await userEvent.click(within(sub).getByRole("menuitem", { name: "Done" }));
    await waitFor(() => expect(posts.find((p) => p.url.includes("/api/wi/state"))?.body).toEqual({ ...ref, state: "Done" }));
    expect(onActed).toHaveBeenCalledTimes(1);
  });

  it("Assign posts the chosen assignee_id, with the current assignee checked and disabled", async () => {
    const { posts } = mockFetch({ get: fixtures() });
    renderWithClient(<Harness row={wiRow()} />);
    await userEvent.click(item("Assign"));
    const sub = await screen.findByRole("menu", { name: "Assign" });
    await within(sub).findByRole("menuitem", { name: "Priya Nair" });
    expect(labelsIn(sub)).toEqual([
      "Unassigned",
      "Sam Rivera",
      "Priya Nair",
    ]);
    expect(within(sub).getByRole("menuitemcheckbox", { name: "Sam Rivera" })).toHaveAttribute("title", "Already assigned");
    await userEvent.click(within(sub).getByRole("menuitem", { name: "Priya Nair" }));
    await waitFor(() => expect(posts.find((p) => p.url.includes("/api/wi/assignee"))?.body).toEqual({ ...ref, assignee_id: "u1" }));
  });

  it("Unassigned posts assignee_id null, and is checked and disabled when nobody is assigned", async () => {
    const { posts } = mockFetch({ get: fixtures() });
    const { unmount } = renderWithClient(<Harness row={wiRow()} />);
    await userEvent.click(item("Assign"));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Unassigned" }));
    await waitFor(() => expect(posts.find((p) => p.url.includes("/api/wi/assignee"))?.body).toEqual({ ...ref, assignee_id: null }));
    unmount();

    renderWithClient(
      <Harness
        row={wiRow((wi) => {
          wi.assignee = null;
        })}
      />,
    );
    await userEvent.click(item("Assign"));
    const none = await screen.findByRole("menuitemcheckbox", { name: "Unassigned" });
    expect(none).toHaveAttribute("aria-checked", "true");
    expect(none).toHaveAttribute("title", "Already unassigned");
  });

  it("Edit… and Comment… open the pane with their intent", async () => {
    mockFetch({ get: fixtures() });
    const { unmount } = renderWithClient(<Harness row={wiRow()} />);
    await userEvent.click(item("Edit title & description…"));
    expect(openWi).toHaveBeenLastCalledWith(ref, "edit");
    unmount();

    renderWithClient(<Harness row={wiRow()} />);
    await userEvent.click(item("Comment…"));
    expect(openWi).toHaveBeenLastCalledWith(ref, "comment");
  });

  it("the hide toggle posts the list with the row's state added", async () => {
    const { posts } = mockFetch({ get: fixtures(["Done"]) });
    renderWithClient(<Harness row={wiRow()} />);
    const hide = await screen.findByRole("menuitemcheckbox", { name: 'Hide "In Progress" items' });
    await waitFor(() => expect(hide).not.toHaveAttribute("aria-disabled"));
    expect(hide).toHaveAttribute("aria-checked", "false");
    await userEvent.click(hide);
    await waitFor(() =>
      expect(posts.find((p) => p.url.includes("/api/prefs/hidden-work-item-state"))?.body).toEqual({ state: "In Progress", hidden: true }),
    );
  });

  it("is checked when the row's state is hidden, keeps the Hide label, and toggling posts it removed", async () => {
    const { posts } = mockFetch({ get: fixtures(["Done", "Todo"]) });
    renderWithClient(
      <Harness
        row={wiRow((wi) => {
          wi.state = "Done";
          wi.state_category = "Completed";
        })}
      />,
    );
    // The label is the same before and after prefs load; the tick is what says "hidden".
    const show = await screen.findByRole("menuitemcheckbox", { name: 'Hide "Done" items' });
    await waitFor(() => expect(show).toHaveAttribute("aria-checked", "true"));
    await userEvent.click(show);
    await waitFor(() => expect(posts.find((p) => p.url.includes("/api/prefs/hidden-work-item-state"))?.body).toEqual({ state: "Done", hidden: false }));
  });

  it("appends the caller's extra entries in a group of their own", () => {
    mockFetch({ get: fixtures() });
    renderWithClient(<Harness row={wiRow()} opts={{ extra: [{ label: "Dismiss from Command Center", icon: "minus-circle", hint: "D" }] }} />);
    expect(labels().at(-1)).toBe("Dismiss from Command Center");
    expect(screen.getAllByRole("separator")).toHaveLength(3);
  });
});
