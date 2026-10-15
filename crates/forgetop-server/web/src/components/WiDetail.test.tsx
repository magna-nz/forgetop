import { useEffect } from "react";
import { describe, it, expect } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { WiDetailProvider, useWiOpener } from "./WiDetail";
import { renderWithClient, mockFetch } from "../test/util";

function Opener({ conn, id, intent }: { conn: string; id: string; intent?: "edit" | "comment" }) {
  const open = useWiOpener();
  useEffect(() => {
    open({ conn, id }, intent);
  }, [open, conn, id, intent]);
  return null;
}

/** Opens the item again on a click — a second open of the item already in the pane. */
function OpenButton({ conn, id, intent }: { conn: string; id: string; intent: "edit" | "comment" }) {
  const open = useWiOpener();
  return (
    <button type="button" onClick={() => open({ conn, id }, intent)}>
      Reopen with {intent}
    </button>
  );
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const detail = (): any => ({
  work_item: {
    id: "w1",
    identifier: "ENG-1",
    title: "Original title",
    description: "Original description",
    state: "In Progress",
    state_category: "Started",
    work_item_type: "Bug",
    assignee: { id: "me", display_name: "Sam Rivera", handle: "you", avatar_url: null },
    created_at: null,
    updated_at: null,
    url: null,
  },
  timeline: [],
  threads: [],
});

const users = [
  { id: "me", display_name: "Sam Rivera", handle: "you", avatar_url: null },
  { id: "u1", display_name: "Priya Nair", handle: "priya", avatar_url: null },
];

describe("WiDetail", () => {
  it("reassigns via the picker, posting /api/wi/assignee with the chosen id", async () => {
    const { posts } = mockFetch({ get: { "/api/wi/detail": detail(), "/api/wi/assignees": users } });
    renderWithClient(
      <WiDetailProvider>
        <Opener conn="c" id="w1" />
      </WiDetailProvider>,
    );

    // The picker trigger shows the current assignee; open it and pick someone else.
    await userEvent.click(await screen.findByRole("button", { name: /Sam Rivera/ }));
    await userEvent.click(await screen.findByRole("button", { name: /Priya Nair/ }));

    await waitFor(() =>
      expect(
        posts.some((p) => p.url.includes("/api/wi/assignee") && (p.body as { assignee_id?: string }).assignee_id === "u1"),
      ).toBe(true),
    );
  });

  it("unassigns via the picker, posting assignee_id null", async () => {
    const { posts } = mockFetch({ get: { "/api/wi/detail": detail(), "/api/wi/assignees": users } });
    renderWithClient(
      <WiDetailProvider>
        <Opener conn="c" id="w1" />
      </WiDetailProvider>,
    );

    await userEvent.click(await screen.findByRole("button", { name: /Sam Rivera/ }));
    await userEvent.click(await screen.findByRole("button", { name: "Unassign" }));

    await waitFor(() =>
      expect(
        posts.some((p) => p.url.includes("/api/wi/assignee") && (p.body as { assignee_id?: string | null }).assignee_id == null),
      ).toBe(true),
    );
  });

  it("hides Unassign when the item is already unassigned", async () => {
    const d = detail();
    d.work_item.assignee = null;
    mockFetch({ get: { "/api/wi/detail": d, "/api/wi/assignees": users } });
    renderWithClient(
      <WiDetailProvider>
        <Opener conn="c" id="w1" />
      </WiDetailProvider>,
    );

    // The trigger reads "Unassigned"; opening it must NOT offer an Unassign action.
    await userEvent.click(await screen.findByRole("button", { name: /Unassigned/ }));
    await screen.findByRole("button", { name: /Priya Nair/ }); // dropdown is open
    expect(screen.queryByRole("button", { name: "Unassign" })).not.toBeInTheDocument();
  });

  it("moves the state via the footer menu, posting /api/wi/state", async () => {
    const { posts } = mockFetch({ get: { "/api/wi/detail": detail(), "/api/wi/states": ["Todo", "In Progress", "Done"] } });
    renderWithClient(
      <WiDetailProvider>
        <Opener conn="c" id="w1" />
      </WiDetailProvider>,
    );

    const move = await screen.findByRole("button", { name: "Move" });
    expect(move).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(move);
    expect(move).toHaveAttribute("aria-expanded", "true");
    // The current state is not offered.
    await screen.findByRole("button", { name: "Done" });
    expect(screen.queryByRole("button", { name: "In Progress" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Done" }));

    await waitFor(() => expect(posts.some((p) => p.url.includes("/api/wi/state") && (p.body as { state?: string }).state === "Done")).toBe(true));
    expect(within(screen.getByRole("status")).getByText("Moved to Done")).toBeInTheDocument();
  });

  it("edits the title, posting only the changed field to /api/wi/update", async () => {
    const { posts } = mockFetch({ get: { "/api/wi/detail": detail() } });
    renderWithClient(
      <WiDetailProvider>
        <Opener conn="c" id="w1" />
      </WiDetailProvider>,
    );

    await userEvent.click(await screen.findByRole("button", { name: "Edit" }));
    const title = screen.getByPlaceholderText("Title");
    await userEvent.clear(title);
    await userEvent.type(title, "New title");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      const post = posts.find((p) => p.url.includes("/api/wi/update"));
      expect(post).toBeDefined();
      const body = post!.body as { title?: string; description?: string };
      expect(body.title).toBe("New title");
      // description was untouched, so it must be omitted (left unchanged server-side).
      expect(body.description).toBeUndefined();
    });
  });

  it("a comment shows before the provider answers", async () => {
    let release: () => void = () => {};
    const answer = new Promise<unknown>((resolve) => (release = () => resolve({ ok: true })));
    mockFetch({ get: { "/api/wi/detail": detail() }, onPost: () => answer });
    renderWithClient(
      <WiDetailProvider>
        <Opener conn="c" id="w1" />
      </WiDetailProvider>,
    );

    await userEvent.type(await screen.findByPlaceholderText("Add a comment…"), "On it");
    await userEvent.click(screen.getByRole("button", { name: "Comment" }));
    expect(await screen.findByText("On it")).toBeInTheDocument();
    expect(within(screen.getByRole("status")).getByText("Comment posted")).toBeInTheDocument();
    release();
  });

  it("opening with the edit intent shows the edit form once the item loads", async () => {
    mockFetch({ get: { "/api/wi/detail": detail() } });
    renderWithClient(
      <WiDetailProvider>
        <Opener conn="c" id="w1" intent="edit" />
      </WiDetailProvider>,
    );
    expect(await screen.findByRole("textbox", { name: "Title" })).toHaveValue("Original title");
    expect(screen.getByRole("textbox", { name: "Description" })).toHaveValue("Original description");
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
  });

  it("opening with the comment intent focuses the comment box", async () => {
    mockFetch({ get: { "/api/wi/detail": detail() } });
    renderWithClient(
      <WiDetailProvider>
        <Opener conn="c" id="w1" intent="comment" />
      </WiDetailProvider>,
    );
    const box = await screen.findByRole("textbox", { name: "Add a comment" });
    await waitFor(() => expect(box).toHaveFocus());
    expect(screen.getByRole("button", { name: "Edit" })).toBeInTheDocument();
  });

  it("reopening the open item with another intent still applies it", async () => {
    mockFetch({ get: { "/api/wi/detail": detail() } });
    renderWithClient(
      <WiDetailProvider>
        <Opener conn="c" id="w1" />
        <OpenButton conn="c" id="w1" intent="edit" />
      </WiDetailProvider>,
    );
    await screen.findByRole("button", { name: "Edit" });
    expect(screen.queryByRole("textbox", { name: "Title" })).not.toBeInTheDocument();
    // No intent: neither the editor nor the comment box takes over.
    expect(screen.getByRole("textbox", { name: "Add a comment" })).not.toHaveFocus();

    await userEvent.click(screen.getByRole("button", { name: "Reopen with edit" }));
    expect(await screen.findByRole("textbox", { name: "Title" })).toBeInTheDocument();

    // Cancel, then the same intent again: it is a new open, so it applies again.
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("textbox", { name: "Title" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Reopen with edit" }));
    expect(await screen.findByRole("textbox", { name: "Title" })).toBeInTheDocument();
  });

  it("opened at a comment, scrolls to it and marks it; the triage strip's Done closes the last thread", async () => {
    const d = detail();
    const priya = { id: "u1", display_name: "Priya Nair", handle: "priya", avatar_url: null };
    d.threads = [
      { id: "th1", comments: [{ id: "k1", author: priya, body: "Earlier note", created_at: null }], file_path: null, line: null, is_resolved: false },
      { id: "th2", comments: [{ id: "k2", author: priya, body: "What tolerance do we accept?", created_at: null }], file_path: null, line: null, is_resolved: false },
    ];
    mockFetch({ get: { "/api/wi/detail": d } });
    let done = 0;
    function TargetOpener() {
      const open = useWiOpener();
      useEffect(() => {
        open({ conn: "c", id: "w1" }, undefined, {
          commentId: "k2",
          fromInbox: {
            key: "t",
            index: 1,
            total: 1,
            phrase: "Sam commented",
            why: "you follow this issue",
            conn: "c",
            notificationId: "n1",
            needsDetail: false,
            providerLabel: "Linear",
            onDone: () => {
              done++;
              return "none";
            },
          },
        });
      }, [open]);
      return null;
    }
    renderWithClient(
      <WiDetailProvider>
        <TargetOpener />
      </WiDetailProvider>,
    );

    const pane = await screen.findByRole("region", { name: "Work item" });
    await within(pane).findByText("What tolerance do we accept?");
    const marked = pane.querySelector("[data-target]") as HTMLElement;
    expect(marked).toHaveTextContent("What tolerance do we accept?");
    expect(marked).not.toHaveTextContent("Earlier note");
    expect(within(marked).getByText("From your inbox")).toBeInTheDocument();
    expect(within(pane).getByTestId("triage-strip")).toHaveTextContent("Inbox · 1 of 1");

    await userEvent.click(within(pane).getByRole("button", { name: /Dismiss/ }));
    expect(done).toBe(1);
    await waitFor(() => expect(screen.queryByRole("region", { name: "Work item" })).not.toBeInTheDocument());
  });

  it("a comment in a thread of several rings that comment only, not its thread", async () => {
    const d = detail();
    const priya = { id: "u1", display_name: "Priya Nair", handle: "priya", avatar_url: null };
    d.threads = [
      {
        id: "th1",
        comments: [
          { id: "k1", author: priya, body: "First note", created_at: null },
          { id: "k2", author: priya, body: "What tolerance do we accept?", created_at: null },
          { id: "k3", author: priya, body: "Third note", created_at: null },
        ],
        file_path: null,
        line: null,
        is_resolved: false,
      },
    ];
    mockFetch({ get: { "/api/wi/detail": d } });
    function TargetOpener() {
      const open = useWiOpener();
      useEffect(() => {
        open({ conn: "c", id: "w1" }, undefined, { commentId: "k2" });
      }, [open]);
      return null;
    }
    renderWithClient(
      <WiDetailProvider>
        <TargetOpener />
      </WiDetailProvider>,
    );

    const pane = await screen.findByRole("region", { name: "Work item" });
    await within(pane).findByText("What tolerance do we accept?");
    const marked = pane.querySelectorAll("[data-target]");
    expect(marked).toHaveLength(1);
    expect(marked[0]).toHaveAttribute("data-comment-id", "k2");
    expect(marked[0]).not.toHaveTextContent("First note");
    expect(marked[0]).not.toHaveTextContent("Third note");
    expect(within(marked[0] as HTMLElement).getByText("From your inbox")).toBeInTheDocument();
    expect(within(pane).getAllByText("From your inbox")).toHaveLength(1);
  });
});
