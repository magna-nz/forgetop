import { describe, expect, it, vi } from "vitest";
import { apiGet, isStale, oldestStaleAgeMs, STALE_AFTER_MS } from "./api";
import { cacheAgeLabel } from "./components/TopBar";

function answer(body: unknown, fetchedAt?: Date) {
  const headers = new Headers({ "content-type": "application/json" });
  if (fetchedAt) headers.set("x-forgetop-fetched-at", fetchedAt.toISOString());
  return new Response(JSON.stringify(body), { status: 200, headers });
}

describe("the shared cache", () => {
  it("marks a list stale when the server answered from an old cache entry, and live when it did not", async () => {
    const old = new Date(Date.now() - 10 * 60_000);
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => (String(input).includes("work-items") ? answer([], old) : answer([]))));
    await apiGet("/api/work-items");
    await apiGet("/api/pipelines");
    expect(isStale("/api/work-items")).toBe(true);
    expect(isStale("/api/pipelines")).toBe(false);
    const age = oldestStaleAgeMs();
    expect(age).not.toBeNull();
    expect(age!).toBeGreaterThan(9 * 60_000);

    // A fresh cached answer (younger than the poll interval) is not stale either.
    vi.stubGlobal("fetch", vi.fn(async () => answer([], new Date(Date.now() - STALE_AFTER_MS / 2))));
    await apiGet("/api/work-items");
    expect(isStale("/api/work-items")).toBe(false);

    // A live answer clears the mark.
    vi.stubGlobal("fetch", vi.fn(async () => answer([])));
    await apiGet("/api/work-items");
    expect(oldestStaleAgeMs()).toBeNull();
  });

  it("labels an age the way the top bar shows it", () => {
    expect(cacheAgeLabel(20_000)).toBe("under a minute");
    expect(cacheAgeLabel(4 * 60_000)).toBe("4m");
    expect(cacheAgeLabel(3 * 3_600_000)).toBe("3h");
    expect(cacheAgeLabel(5 * 86_400_000)).toBe("5d");
  });
});
