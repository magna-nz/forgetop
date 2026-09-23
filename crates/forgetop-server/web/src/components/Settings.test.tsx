import { describe, it, expect, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Settings } from "./Settings";
import { renderWithClient } from "../test/util";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const connection = (id: string, name: string): any => ({
  id,
  provider: "GitHub",
  display_name: name,
  sections: ["pull_requests"],
  has_token: true,
  repo_scope: ["acme/pay"],
});

describe("Settings · removing a connection", () => {
  const confirmYes = () => vi.spyOn(window, "confirm").mockReturnValue(true);

  it("drops the card when the delete is accepted, and the refetch agrees", async () => {
    confirmYes();
    // The stub models the server: once the delete lands, the connection is really gone. Without
    // that, the refetch legitimately restores the card and the test would be asserting a race.
    let deleted = false;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if ((init?.method ?? "GET").toUpperCase() === "POST") {
          deleted = true;
          return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
        }
        const rows = url.includes("/api/connections") && !deleted ? [connection("gone", "Doomed")] : [];
        return new Response(JSON.stringify(rows), { status: 200, headers: { "content-type": "application/json" } });
      }),
    );

    renderWithClient(<Settings />);
    await screen.findByText("Doomed");

    await userEvent.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(screen.queryByText("Doomed")).not.toBeInTheDocument());
  });

  /**
   * The failure mode this guards: `useWriteAction.run` reports a rejected write by returning
   * false rather than throwing. Dropping the rows regardless showed the connection deleted and
   * then, one refetch later, silently undeleted — with nothing saying why.
   *
   * The refetch is deliberately left hanging here. Let it answer and it restores the row on both
   * the fixed and the broken code, which is precisely what made the flicker easy to miss; with
   * it pending, an optimistic drop is the only thing that can take the card off screen.
   */
  it("keeps the connection on screen when the delete is refused", async () => {
    confirmYes();
    let refused = false;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if ((init?.method ?? "GET").toUpperCase() === "POST") {
          refused = true;
          return new Response(JSON.stringify({ error: "keychain is locked" }), { status: 500 });
        }
        if (refused) return new Promise<Response>(() => {}); // never settles
        const rows = url.includes("/api/connections") ? [connection("gone", "Doomed")] : [];
        return new Response(JSON.stringify(rows), { status: 200, headers: { "content-type": "application/json" } });
      }),
    );

    renderWithClient(<Settings />);
    await screen.findByText("Doomed");

    await userEvent.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(refused).toBe(true));
    expect(screen.getByText("Doomed")).toBeInTheDocument();
  });
});
