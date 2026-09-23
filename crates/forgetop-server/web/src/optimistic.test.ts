import { describe, expect, it } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { dropConnectionFromCache } from "./optimistic";
import type { LaunchpadResponse } from "./types";

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
