import { describe, it, expect, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderWithClient, mockFetch } from "../test/util";

const { openPr } = vi.hoisted(() => ({ openPr: vi.fn() }));
vi.mock("./PrDetail", () => ({ usePrOpener: () => openPr }));
vi.mock("./WiDetail", () => ({ useWiOpener: () => vi.fn() }));
vi.mock("./PipelineDetail", () => ({ usePipelineOpener: () => vi.fn() }));

import { Notifications } from "./Notifications";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const notif = (id: string, o: { unread?: boolean; title?: string; item_id?: string | null } = {}): any => ({
  connection_id: "c",
  connection: "GH",
  provider: "GitHub",
  notification: {
    id,
    kind: "ReviewRequested",
    item_type: "PullRequest",
    item_id: o.item_id === undefined ? "10" : o.item_id,
    title: o.title ?? "Review this PR",
    context: "northwind/payments",
    url: "https://example.test/pr/10",
    unread: o.unread ?? true,
    updated_at: null,
  },
});

describe("Notifications", () => {
  it("defaults to Unread and opens a notification's item in-app", async () => {
    mockFetch({
      get: {
        "/api/notifications": [notif("a", { title: "An unread one" }), notif("b", { unread: false, title: "A read one" })],
      },
    });
    renderWithClient(<Notifications />);

    // The default filter hides already-read notifications.
    expect(await screen.findByText("An unread one")).toBeInTheDocument();
    expect(screen.queryByText("A read one")).not.toBeInTheDocument();

    // Clicking a PR notification opens it in-app.
    await userEvent.click(screen.getByText("An unread one"));
    expect(openPr).toHaveBeenCalledWith({ conn: "c", id: "10" });
  });

  it("marks a notification read via the read button", async () => {
    const { posts } = mockFetch({ get: { "/api/notifications": [notif("a", { title: "Ping" })] } });
    renderWithClient(<Notifications />);

    await screen.findByText("Ping");
    await userEvent.click(screen.getByTitle("Mark as read"));
    await waitFor(() => expect(posts.some((p) => p.url.includes("/api/notification/read"))).toBe(true));
  });
});
