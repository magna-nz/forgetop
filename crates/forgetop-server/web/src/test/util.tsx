import type { ReactElement } from "react";
import { render } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { vi } from "vitest";

/** Renders a component inside a fresh React-Query client (retries off, no shared cache). */
export function renderWithClient(ui: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

/**
 * Stubs global `fetch`, routing GETs to JSON fixtures (by URL substring) and recording POSTs.
 * `onPost` may return the JSON body to answer with. Returns the recorded POSTs for assertions.
 */
export function mockFetch(routes: {
  get?: Record<string, unknown>;
  onPost?: (url: string, body: unknown) => unknown;
}) {
  const posts: { url: string; body: unknown }[] = [];
  const fn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = (init?.method ?? "GET").toUpperCase();
    if (method === "POST") {
      const body = init?.body ? JSON.parse(String(init.body)) : undefined;
      posts.push({ url, body });
      const answer = routes.onPost?.(url, body);
      return new Response(JSON.stringify(answer ?? { ok: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    const key = Object.keys(routes.get ?? {}).find((k) => url.includes(k));
    if (key) {
      return new Response(JSON.stringify(routes.get![key]), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    return new Response("not found", { status: 404 });
  });
  vi.stubGlobal("fetch", fn);
  return { fetchMock: fn, posts };
}
