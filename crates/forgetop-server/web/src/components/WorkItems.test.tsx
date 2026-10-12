import { describe, it, expect, beforeEach } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { WorkItems } from "./WorkItems";
import { renderWithClient, mockFetch } from "../test/util";

interface Opts {
  conn?: string;
  state?: string;
  category?: string;
  parent?: { kind: string; id: string; title: string; url?: string } | null;
  updated?: string;
}

/** A work-item row. `updated` drives the default "Recently updated" sort (newest first). */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const wi = (id: string, title: string, o: Opts = {}): any => ({
  connection_id: o.conn ?? "c1",
  connection: o.conn ?? "c1",
  provider: "GitHub",
  work_item: {
    id,
    title,
    state: o.state ?? "Open",
    state_category: o.category ?? "Unstarted",
    work_item_type: "Task",
    updated_at: o.updated ?? "2026-07-01T00:00:00Z",
    ...(o.parent ? { parent: o.parent } : {}),
  },
});

const q4 = { kind: "Milestone", id: "m1", title: "Q4 reliability", url: "https://example.com/m1" };
const ledger = { kind: "Epic", id: "e1", title: "Ledger v2" };

const rowsFixture = [
  // Newest first after the default sort: a1, n1, b1, a2.
  wi("1", "Alpha one", { parent: q4, updated: "2026-07-04T00:00:00Z", state: "In Progress", category: "Started" }),
  wi("2", "No parent one", { updated: "2026-07-03T00:00:00Z" }),
  wi("3", "Beta one", { parent: ledger, updated: "2026-07-02T00:00:00Z", state: "Blocked", category: "Started" }),
  wi("4", "Alpha two", { parent: q4, updated: "2026-07-01T00:00:00Z", state: "Review", category: "Started" }),
];

const choose = (mode: string) => userEvent.selectOptions(screen.getByLabelText("Group"), mode);
const headers = () => screen.queryAllByRole("button", { expanded: false }).concat(screen.queryAllByRole("button", { expanded: true }));

// Most tests start from the flat list and switch grouping on, so they start with Off chosen;
// the ones about the default clear it again.
beforeEach(() => {
  localStorage.clear();
  localStorage.setItem("forgetop:work-items:group", "off");
});

describe("work items grouped by parent", () => {
  it("is grouped by parent by default, collapsed to the parents", async () => {
    localStorage.clear();
    mockFetch({ get: { "/api/work-items": rowsFixture } });
    renderWithClient(<WorkItems />);
    expect(await screen.findByRole("button", { name: /Q4 reliability/ })).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByLabelText("Group")).toHaveValue("parent");
    expect(headers()).toHaveLength(3);
    expect(screen.queryByText("Alpha one")).not.toBeInTheDocument();
  });

  it("is flat once Off is chosen: every title shown, no group headers", async () => {
    mockFetch({ get: { "/api/work-items": rowsFixture } });
    renderWithClient(<WorkItems />);
    for (const t of ["Alpha one", "No parent one", "Beta one", "Alpha two"]) {
      expect(await screen.findByText(t)).toBeInTheDocument();
    }
    expect(headers()).toHaveLength(0);
    expect(screen.queryByText("Q4 reliability")).not.toBeInTheDocument();
  });

  it("groups under parent headers with counts, collapsed until clicked", async () => {
    mockFetch({ get: { "/api/work-items": rowsFixture } });
    renderWithClient(<WorkItems />);
    await screen.findByText("Alpha one");
    await choose("parent");

    const q4Header = screen.getByRole("button", { name: /Q4 reliability/ });
    expect(q4Header).toHaveTextContent("Milestone");
    expect(q4Header).toHaveTextContent("2 items");
    expect(q4Header).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByRole("button", { name: /Ledger v2/ })).toHaveTextContent("1 item");
    expect(screen.getByRole("button", { name: /No parent/ })).toHaveTextContent("1 item");
    expect(screen.queryByText("Alpha one")).not.toBeInTheDocument();
    expect(screen.queryByText("Beta one")).not.toBeInTheDocument();

    await userEvent.click(q4Header);
    expect(q4Header).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Alpha one")).toBeInTheDocument();
    expect(screen.getByText("Alpha two")).toBeInTheDocument();
    expect(screen.queryByText("Beta one")).not.toBeInTheDocument();
    expect(screen.queryByText("No parent one")).not.toBeInTheDocument();

    await userEvent.click(q4Header);
    expect(screen.queryByText("Alpha one")).not.toBeInTheDocument();
  });

  it("orders groups by first appearance with 'No parent' always last", async () => {
    // Sorted: Alpha one (Q4) -> No parent one -> Beta one (Ledger) -> Alpha two (Q4).
    // The parentless row is second, yet its group still comes last.
    mockFetch({ get: { "/api/work-items": rowsFixture } });
    renderWithClient(<WorkItems />);
    await screen.findByText("Alpha one");
    await choose("parent");
    const names = headers().map((b) => b.textContent ?? "");
    expect(names).toHaveLength(3);
    expect(names[0]).toContain("Q4 reliability");
    expect(names[1]).toContain("Ledger v2");
    expect(names[2]).toContain("No parent");
  });

  it("keeps sorted order inside a group", async () => {
    mockFetch({ get: { "/api/work-items": rowsFixture } });
    renderWithClient(<WorkItems />);
    await screen.findByText("Alpha one");
    await choose("parent");
    await userEvent.click(screen.getByRole("button", { name: /Q4 reliability/ }));
    const one = screen.getByText("Alpha one");
    const two = screen.getByText("Alpha two");
    expect(one.compareDocumentPosition(two) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("never merges the same parent kind+id across connections", async () => {
    mockFetch({
      get: {
        "/api/work-items": [
          wi("1", "On c1", { conn: "c1", parent: q4 }),
          wi("2", "On c2", { conn: "c2", parent: q4 }),
        ],
      },
    });
    renderWithClient(<WorkItems />);
    await screen.findByText("On c1");
    await choose("parent");
    const hs = screen.getAllByRole("button", { name: /Q4 reliability/ });
    expect(hs).toHaveLength(2);
    hs.forEach((h) => expect(h).toHaveTextContent("1 item"));
    await userEvent.click(hs[0]);
    expect(screen.getByText("On c1")).toBeInTheDocument();
    expect(screen.queryByText("On c2")).not.toBeInTheDocument();
  });

  it("rolls up in-progress and blocked counts (blocked is not also in progress)", async () => {
    mockFetch({
      get: {
        "/api/work-items": [
          wi("1", "A", { parent: q4, state: "In Progress", category: "Started" }),
          wi("2", "B", { parent: q4, state: "Review", category: "Started" }),
          wi("3", "C", { parent: q4, state: "blocked", category: "Started" }),
          wi("4", "D", { parent: q4, state: "Open", category: "Unstarted" }),
          wi("5", "E", { parent: ledger, state: "Open", category: "Unstarted" }),
        ],
      },
    });
    renderWithClient(<WorkItems />);
    await screen.findByLabelText("Group");
    await choose("parent");
    const q4Header = screen.getByRole("button", { name: /Q4 reliability/ });
    expect(within(q4Header).getByText("2 in progress")).toBeInTheDocument();
    expect(within(q4Header).getByText("1 blocked")).toBeInTheDocument();
    const ledgerHeader = screen.getByRole("button", { name: /Ledger v2/ });
    expect(ledgerHeader).not.toHaveTextContent("in progress");
    expect(ledgerHeader).not.toHaveTextContent("blocked");
  });

  it("links the parent out in a new tab without toggling the section", async () => {
    mockFetch({ get: { "/api/work-items": rowsFixture } });
    renderWithClient(<WorkItems />);
    await screen.findByText("Alpha one");
    await choose("parent");
    const link = screen.getByRole("link", { name: /Q4 reliability/ });
    expect(link).toHaveAttribute("href", "https://example.com/m1");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noreferrer");
    await userEvent.click(link);
    expect(screen.getByRole("button", { name: /Q4 reliability/ })).toHaveAttribute("aria-expanded", "false");
    // The parent without a url gets no link.
    expect(screen.queryByRole("link", { name: /Ledger v2/ })).not.toBeInTheDocument();
  });

  it("stays a plain list when no row has a parent, even with grouping on", async () => {
    // Nothing to group by: a lone "No parent" section over everything would only hide the list.
    localStorage.clear();
    mockFetch({ get: { "/api/work-items": [wi("1", "Lonely"), wi("2", "Also lonely")] } });
    renderWithClient(<WorkItems />);
    expect(await screen.findByText("Lonely")).toBeInTheDocument();
    expect(screen.getByText("Also lonely")).toBeInTheDocument();
    expect(screen.getByLabelText("Group")).toHaveValue("parent");
    expect(headers()).toHaveLength(0);
    expect(screen.queryByText("No parent")).not.toBeInTheDocument();
  });

  it("remembers the choice in a fresh render", async () => {
    mockFetch({ get: { "/api/work-items": rowsFixture } });
    const first = renderWithClient(<WorkItems />);
    await screen.findByText("Alpha one");
    await choose("parent");
    expect(localStorage.getItem("forgetop:work-items:group")).toBe("parent");
    first.unmount();

    const second = renderWithClient(<WorkItems />);
    expect(await screen.findByRole("button", { name: /Q4 reliability/ })).toBeInTheDocument();
    expect(screen.getByLabelText("Group")).toHaveValue("parent");
    expect(screen.queryByText("Alpha one")).not.toBeInTheDocument();

    // And turning it off is remembered just the same, against the default.
    await choose("off");
    expect(localStorage.getItem("forgetop:work-items:group")).toBe("off");
    second.unmount();
    renderWithClient(<WorkItems />);
    expect(await screen.findByText("Alpha one")).toBeInTheDocument();
    expect(screen.getByLabelText("Group")).toHaveValue("off");
  });
});
