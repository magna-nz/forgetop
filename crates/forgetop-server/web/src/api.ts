import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { ConnectionRow, FileChange, HealthRow, LaunchpadResponse, NotifRow, PipeRef, PipelineDetail, PipeRow, Preferences, PrDecoration, PrDetail, PrRef, ProviderInfo, PrRow, RepositoryPage, WiDetail, WiRef, WiRow } from "./types";

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

/** GET a token-authenticated plain-text endpoint (e.g. pipeline logs). */
export async function apiGetText(path: string): Promise<string> {
  const res = await fetch(path, { headers: { "x-forgetop-token": TOKEN } });
  if (!res.ok) {
    throw new ApiError(res.status, res.status === 401 ? "Unauthorized — reopen the dashboard from forgetop." : `${res.status} ${res.statusText}`);
  }
  return res.text();
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

/** `&repo=…` for an addressed ref, or nothing. A repository is always a query parameter and
 *  never a path segment — an `owner/repo` contains a slash — and it stays optional so every
 *  link written before a connection spanned an account still resolves. */
const repoParam = (repo?: string | null) => (repo ? `&repo=${encodeURIComponent(repo)}` : "");

// Query keys are **positional**, so a key and the invalidation that clears it drift apart
// silently — the symptom is a detail view that goes stale after a mutation, intermittently.
// Building each one here means both sides can only ever use the same shape.
export const prDetailKey = (ref: Pick<PrRef, "conn" | "repo" | "id">) => ["pr-detail", ref.conn, ref.repo ?? "", ref.id];
export const wiDetailKey = (ref: Pick<WiRef, "conn" | "repo" | "id">) => ["wi-detail", ref.conn, ref.repo ?? "", ref.id];
export const pipelineDetailKey = (ref: Pick<PipeRef, "conn" | "repo" | "runId">) => [
  "pipeline-detail",
  ref.conn,
  ref.repo ?? "",
  ref.runId,
];

export const usePrDetail = (ref: PrRef | null) =>
  useQuery({
    queryKey: ref ? prDetailKey(ref) : ["pr-detail", null],
    queryFn: () =>
      api<PrDetail>(`/api/pr/detail?conn=${encodeURIComponent(ref!.conn)}&id=${encodeURIComponent(ref!.id)}${repoParam(ref!.repo)}`),
    enabled: !!ref,
  });

/** The files changed by a single commit on the PR (for the Commits → Files drill-in). */
export const usePrCommitChanges = (ref: PrRef | null, sha: string | null) =>
  useQuery({
    queryKey: ["pr-commit-changes", ref?.conn, ref?.repo ?? "", ref?.id, sha],
    queryFn: () =>
      api<FileChange[]>(
        `/api/pr/commit-changes?conn=${encodeURIComponent(ref!.conn)}&id=${encodeURIComponent(ref!.id)}&sha=${encodeURIComponent(sha!)}${repoParam(ref!.repo)}`,
      ),
    enabled: !!ref && !!sha,
  });

/** One row's decorated fields, fetched lazily because the list endpoint no longer pays for them.
 *  Keyed (and server-cached) on `updated_at` — the provider's own statement that the PR changed —
 *  so an entry can never outlive the change that invalidates it. */
export const usePrDecoration = (row: { conn: string; repo?: string | null; id: string; updatedAt?: string | null } | null) =>
  useQuery({
    queryKey: ["pr-decoration", row?.conn, row?.repo ?? "", row?.id, row?.updatedAt ?? ""],
    queryFn: () =>
      api<PrDecoration>(
        `/api/pr/decoration?conn=${encodeURIComponent(row!.conn)}&id=${encodeURIComponent(row!.id)}${repoParam(row!.repo)}` +
          (row!.updatedAt ? `&updated_at=${encodeURIComponent(row!.updatedAt)}` : ""),
      ),
    enabled: !!row,
    staleTime: 60_000,
  });

export const useWiDetail = (ref: WiRef | null) =>
  useQuery({
    queryKey: ref ? wiDetailKey(ref) : ["wi-detail", null],
    queryFn: () =>
      api<WiDetail>(`/api/wi/detail?conn=${encodeURIComponent(ref!.conn)}&id=${encodeURIComponent(ref!.id)}${repoParam(ref!.repo)}`),
    enabled: !!ref,
  });

export const usePipelineDetail = (ref: PipeRef | null) =>
  useQuery({
    queryKey: ref ? pipelineDetailKey(ref) : ["pipeline-detail", null],
    queryFn: () =>
      api<PipelineDetail>(
        `/api/pipeline/detail?conn=${encodeURIComponent(ref!.conn)}&run_id=${encodeURIComponent(ref!.runId)}${repoParam(ref!.repo)}`,
      ),
    enabled: !!ref,
  });

/** The repositories a connection could fetch from — the scope picker's candidate list. Only the
 *  picker calls this, so a provider whose discovery is wrong shows an empty picker; it cannot
 *  stop an already-scoped connection fetching. */
export const fetchConnectionRepositories = (connectionId: string) =>
  api<RepositoryPage>(`/api/connections/repositories?id=${encodeURIComponent(connectionId)}`);

export const useConnectionRepositories = (connectionId: string | null) =>
  useQuery({
    queryKey: ["connection-repositories", connectionId],
    queryFn: () => api<RepositoryPage>(`/api/connections/repositories?id=${encodeURIComponent(connectionId!)}`),
    enabled: !!connectionId,
    staleTime: 5 * 60_000,
  });

export const useLaunchpad = () => useQuery({ queryKey: ["launchpad"], queryFn: () => api<LaunchpadResponse>("/api/launchpad") });
export const useHealth = () => useQuery({ queryKey: ["health"], queryFn: () => api<HealthRow[]>("/api/health") });
export const useProviders = () =>
  useQuery({ queryKey: ["providers"], queryFn: () => api<ProviderInfo[]>("/api/providers"), staleTime: Infinity });
export const useConnections = () => useQuery({ queryKey: ["connections"], queryFn: () => api<ConnectionRow[]>("/api/connections") });
export const usePreferences = () => useQuery({ queryKey: ["preferences"], queryFn: () => api<Preferences>("/api/preferences") });
/** Which slice of pull requests the PR page shows; maps to the backend `?view=` param. */
export type PrView = "all" | "yours" | "merged" | "review_requested";
export const usePullRequests = (view: PrView = "all") =>
  useQuery({ queryKey: ["prs", view], queryFn: () => api<PrRow[]>(`/api/pull-requests?view=${view}`) });
export const useWorkItems = () => useQuery({ queryKey: ["work-items"], queryFn: () => api<WiRow[]>("/api/work-items") });
export const usePipelines = () => useQuery({ queryKey: ["pipelines"], queryFn: () => api<PipeRow[]>("/api/pipelines") });
export const useNotifications = () =>
  useQuery({ queryKey: ["notifications"], queryFn: () => api<NotifRow[]>("/api/notifications") });
