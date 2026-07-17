import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { ConnectionRow, HealthRow, LaunchpadRow, NotifRow, PipeRow, Preferences, PrDetail, PrRef, ProviderInfo, PrRow, WiRow } from "./types";

// The session token arrives once in the URL (`/?t=…`). We stash it in sessionStorage (so a
// refresh keeps working) and strip it from the visible URL, then replay it on every API call.
const TOKEN: string = (() => {
  const url = new URL(window.location.href);
  const fromUrl = url.searchParams.get("t");
  const stored = sessionStorage.getItem("forgetop_token");
  const token = fromUrl ?? stored ?? "";
  if (fromUrl) {
    sessionStorage.setItem("forgetop_token", fromUrl);
    url.searchParams.delete("t");
    window.history.replaceState({}, "", url.pathname + url.search + url.hash);
  }
  return token;
})();

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

async function api<T>(path: string): Promise<T> {
  const res = await fetch(path, { headers: { "x-forgetop-token": TOKEN } });
  if (!res.ok) {
    throw new ApiError(res.status, res.status === 401 ? "Unauthorized — reopen the dashboard from forgetop." : `${res.status} ${res.statusText}`);
  }
  return (await res.json()) as T;
}

/** GET a token-authenticated JSON endpoint (for lazy fetches outside the query cache). */
export function apiGet<T>(path: string): Promise<T> {
  return api<T>(path);
}

/** POST a JSON body to a write endpoint. Throws ApiError with the server's message on failure. */
export async function apiPost<T = unknown>(path: string, body: unknown): Promise<T> {
  const res = await fetch(path, {
    method: "POST",
    headers: { "x-forgetop-token": TOKEN, "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new ApiError(res.status, text || `${res.status} ${res.statusText}`);
  }
  const ct = res.headers.get("content-type") ?? "";
  return (ct.includes("application/json") ? await res.json() : undefined) as T;
}

/** A tiny mutation helper: POST a write, then invalidate the given query keys. */
export function useWriteAction() {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = async (path: string, body: unknown, invalidate: string[] = []): Promise<boolean> => {
    setBusy(true);
    setError(null);
    try {
      await apiPost(path, body);
      invalidate.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return false;
    } finally {
      setBusy(false);
    }
  };
  return { busy, error, run };
}

export const usePrDetail = (ref: PrRef | null) =>
  useQuery({
    queryKey: ["pr-detail", ref?.conn, ref?.id],
    queryFn: () => api<PrDetail>(`/api/pr/detail?conn=${encodeURIComponent(ref!.conn)}&id=${encodeURIComponent(ref!.id)}`),
    enabled: !!ref,
  });

export const useLaunchpad = () => useQuery({ queryKey: ["launchpad"], queryFn: () => api<LaunchpadRow[]>("/api/launchpad") });
export const useHealth = () => useQuery({ queryKey: ["health"], queryFn: () => api<HealthRow[]>("/api/health") });
export const useProviders = () =>
  useQuery({ queryKey: ["providers"], queryFn: () => api<ProviderInfo[]>("/api/providers"), staleTime: Infinity });
export const useConnections = () => useQuery({ queryKey: ["connections"], queryFn: () => api<ConnectionRow[]>("/api/connections") });
export const usePreferences = () => useQuery({ queryKey: ["preferences"], queryFn: () => api<Preferences>("/api/preferences") });
export const usePullRequests = () => useQuery({ queryKey: ["prs"], queryFn: () => api<PrRow[]>("/api/pull-requests") });
export const useWorkItems = () => useQuery({ queryKey: ["work-items"], queryFn: () => api<WiRow[]>("/api/work-items") });
export const usePipelines = () => useQuery({ queryKey: ["pipelines"], queryFn: () => api<PipeRow[]>("/api/pipelines") });
export const useNotifications = () =>
  useQuery({ queryKey: ["notifications"], queryFn: () => api<NotifRow[]>("/api/notifications") });
