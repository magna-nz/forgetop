import { useEffect } from "react";
import { describe, it, expect } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { WiDetailProvider, useWiOpener } from "./WiDetail";
import { renderWithClient, mockFetch } from "../test/util";

function Opener({ conn, id }: { conn: string; id: string }) {
  const open = useWiOpener();
  useEffect(() => {
    open({ conn, id });
  }, [open, conn, id]);
  return null;
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
});
