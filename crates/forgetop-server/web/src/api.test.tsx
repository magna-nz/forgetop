import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { usePrFileText } from "./api";
import { mockFetch } from "./test/util";

function wrapperFor(client: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

describe("usePrFileText", () => {
  // The server names no head sha for the PR's own diff (no `sha` param), so the cache has
  // nothing else to key on when the PR moves on — the file's own patch text stands in for it.
  it("fetches again when the file's patch text changes, even for the same path and sha", async () => {
    const { fetchMock } = mockFetch({ get: { "/api/pr/file-text": "line1\nline2" } });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
    const { result, rerender } = renderHook(
      ({ patchKey }: { patchKey: string }) => usePrFileText({ conn: "c", id: "1" }, "src/a.rs", null, patchKey, true),
      { wrapper: wrapperFor(client), initialProps: { patchKey: "patch-v1" } },
    );
    await waitFor(() => expect(result.current.data).toBe("line1\nline2"));
    expect(fetchMock).toHaveBeenCalledTimes(1);

    rerender({ patchKey: "patch-v2" });
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
  });

  it("serves the second call from cache when nothing about the file changed", async () => {
    const { fetchMock } = mockFetch({ get: { "/api/pr/file-text": "line1\nline2" } });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
    const { result, rerender } = renderHook(
      ({ patchKey }: { patchKey: string }) => usePrFileText({ conn: "c", id: "1" }, "src/a.rs", null, patchKey, true),
      { wrapper: wrapperFor(client), initialProps: { patchKey: "patch-v1" } },
    );
    await waitFor(() => expect(result.current.data).toBe("line1\nline2"));
    rerender({ patchKey: "patch-v1" });
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
  });
});
