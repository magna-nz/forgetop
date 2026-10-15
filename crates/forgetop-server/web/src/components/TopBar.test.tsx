import { describe, it, expect } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderWithClient, mockFetch } from "../test/util";
import { TopBar } from "./TopBar";
import type { InboxThread, NotificationKind, NotificationTier } from "../types";

const thread = (id: string, kind: NotificationKind, tier: NotificationTier, o: { unread?: boolean; phrase?: string; actor?: string } = {}): InboxThread => {
  const unread = o.unread ?? true;
  return {
    key: `c|${id}`,
    tier,
    unread,
    latest: new Date().toISOString(),
    lead: {
      connection_id: "c",
      connection: "GH",
      provider: "GitHub",
      notification: {
        id,
        kind,
        item_type: "Other",
        item_id: null,
        title: `Title ${id}`,
        context: "ctx",
        url: null,
        unread,
        updated_at: new Date().toISOString(),
        actor: o.actor,
      },
      phrase: o.phrase ?? "review requested",
      why: "",
      tier,
      dismiss_key: `c:${id}:0`,
    },
    more: [],
    members: [{ conn: "c", id, unread }],
  };
};

const bar = () => <TopBar section="launchpad" onOpenPalette={() => {}} sidebarOpen onToggleSidebar={() => {}} />;

describe("TopBar bell", () => {
  it("counts unread threads and lists every inbox thread, with no Following footer", async () => {
    mockFetch({
      get: {
        "/api/inbox": [
          thread("a", "ReviewRequested", "ForYou"),
          thread("b", "Mention", "ForYou", { phrase: "Priya mentioned you" }),
          thread("r", "Assigned", "ForYou", { unread: false }),
        ],
      },
    });
    renderWithClient(bar());

    const bell = await screen.findByRole("button", { name: "Notifications, 2 unread" });
    await userEvent.click(bell);
    expect(screen.getByText("Title a")).toBeInTheDocument();
    expect(screen.getByText("Title b")).toBeInTheDocument();
    expect(screen.getByText("Title r")).toBeInTheDocument();
    // Two lines: the title, then why it's here.
    expect(screen.getByText("Title b").closest("button")).toHaveTextContent("Priya mentioned you");
    expect(screen.queryByText(/Following/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open inbox →" })).toBeInTheDocument();
  });

  it("marks every inbox thread done in one request per connection", async () => {
    let answer!: () => void;
    const held = new Promise<void>((r) => (answer = r));
    const { posts } = mockFetch({
      get: { "/api/inbox": [thread("a", "ReviewRequested", "ForYou"), thread("b", "Mention", "ForYou")] },
      // Held, so the screen is looked at before the refetch that follows the answer.
      onPost: () => held.then(() => ({ ok: true })),
    });
    renderWithClient(bar());

    await userEvent.click(await screen.findByRole("button", { name: /Notifications, 2 unread/ }));
    await userEvent.click(screen.getByRole("button", { name: "Mark all done" }));
    expect(screen.queryByText("Title a")).not.toBeInTheDocument();
    await waitFor(() => expect(posts.filter((p) => p.url.includes("/api/notification/done")).map((p) => p.body)).toEqual([{ conn: "c", ids: ["a", "b"] }]));
    expect(within(screen.getByRole("banner")).queryByText("Nothing needs you")).toBeInTheDocument();
    answer();
  });
});
