import { useQuery } from "@tanstack/react-query";
import type { HealthRow, LaunchpadRow, NotifRow, PipeRow, PrDetail, PrRef, PrRow, WiRow } from "./types";

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

export const usePrDetail = (ref: PrRef | null) =>
  useQuery({
    queryKey: ["pr-detail", ref?.conn, ref?.id],
    queryFn: () => api<PrDetail>(`/api/pr/detail?conn=${encodeURIComponent(ref!.conn)}&id=${encodeURIComponent(ref!.id)}`),
    enabled: !!ref,
  });

export const useLaunchpad = () => useQuery({ queryKey: ["launchpad"], queryFn: () => api<LaunchpadRow[]>("/api/launchpad") });
export const useHealth = () => useQuery({ queryKey: ["health"], queryFn: () => api<HealthRow[]>("/api/health") });
export const usePullRequests = () => useQuery({ queryKey: ["prs"], queryFn: () => api<PrRow[]>("/api/pull-requests") });
export const useWorkItems = () => useQuery({ queryKey: ["work-items"], queryFn: () => api<WiRow[]>("/api/work-items") });
export const usePipelines = () => useQuery({ queryKey: ["pipelines"], queryFn: () => api<PipeRow[]>("/api/pipelines") });
export const useNotifications = () =>
  useQuery({ queryKey: ["notifications"], queryFn: () => api<NotifRow[]>("/api/notifications") });
