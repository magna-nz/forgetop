import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { ApprovalDecision, ConnectionRow, FileChange, HealthRow, LaunchpadResponse, NotifRow, PipeRef, PipelineAnnotation, PipelineArtifact, PipelineDetail, PipelineSelection, PipeRow, PrDecoration, PrDetail, PrRef, ProviderInfo, PrRow, RepositoryPage, User, WiDetail, WiRef, WiRow } from "./types";

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
export const repoParam = (repo?: string | null) => (repo ? `&repo=${encodeURIComponent(repo)}` : "");

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

/** The people who can be asked to review a PR — fetched lazily (only once `enabled`, i.e. the
 *  request-reviewer menu has opened) and then kept for the session: the list is a repository's
 *  collaborators, which doesn't change while the pane is open. */
export const usePrReviewers = (ref: PrRef, enabled: boolean) =>
  useQuery({
    queryKey: ["pr-reviewers", ref.conn, ref.repo ?? "", ref.id],
    queryFn: () =>
      api<User[]>(`/api/pr/reviewers?conn=${encodeURIComponent(ref.conn)}&id=${encodeURIComponent(ref.id)}${repoParam(ref.repo)}`),
    enabled,
    staleTime: Infinity,
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

/** A file's full text at a commit (the PR head when `sha` is omitted) — fetched lazily, only
 *  once a Files-tab gap is clicked, to reveal the unchanged context around a hunk. A 404 means
 *  it's unavailable (binary, too large, or the provider doesn't support it); `retry: false` so
 *  that shows up promptly instead of after three retries.
 *
 *  `patchKey` (the file's own patch text) is folded into the query key purely so the cache
 *  can't go stale: the server names no head sha for "no `sha`" (the PR's own diff), so a
 *  refetched detail with a *different* patch (the PR moved on) is a different key, not a
 *  cache hit on text that no longer matches. */
export const usePrFileText = (ref: PrRef | null, path: string | null, sha: string | null, patchKey: string | null, enabled: boolean) =>
  useQuery({
    queryKey: ["pr-file-text", ref?.conn, ref?.repo ?? "", ref?.id, path ?? "", sha ?? "", patchKey ?? ""],
    queryFn: () =>
      apiGetText(
        `/api/pr/file-text?conn=${encodeURIComponent(ref!.conn)}&id=${encodeURIComponent(ref!.id)}&path=${encodeURIComponent(path!)}` +
          (sha ? `&sha=${encodeURIComponent(sha)}` : "") +
          repoParam(ref!.repo),
      ),
    enabled: enabled && !!ref && !!path,
    staleTime: Infinity,
    retry: false,
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

/** `conn=…&run_id=…[&repo=…]` — the query every per-run pipeline endpoint takes. */
const runQuery = (ref: PipeRef) =>
  `conn=${encodeURIComponent(ref.conn)}&run_id=${encodeURIComponent(ref.runId)}${repoParam(ref.repo)}`;

export const pipelineAnnotationsKey = (ref: Pick<PipeRef, "conn" | "repo" | "runId">) => [
  "pipeline-annotations",
  ref.conn,
  ref.repo ?? "",
  ref.runId,
];
export const pipelineArtifactsKey = (ref: Pick<PipeRef, "conn" | "repo" | "runId">) => [
  "pipeline-artifacts",
  ref.conn,
  ref.repo ?? "",
  ref.runId,
];
/** Every job log of one run — the prefix of `pipelineLogsKey`, to clear them together. */
export const pipelineRunLogsKey = (ref: Pick<PipeRef, "conn" | "repo" | "runId">) => [
  "pipeline-logs",
  ref.conn,
  ref.repo ?? "",
  ref.runId,
];
export const pipelineLogsKey = (ref: Pick<PipeRef, "conn" | "repo" | "runId">, jobId: string | null) => [
  ...pipelineRunLogsKey(ref),
  jobId ?? "",
];

/** The problems a run reported. Like the TUI, fetched only when the caller says so (a finished
 *  run) and never polled — the caller invalidates it when the run's status changes. */
export const usePipelineAnnotations = (ref: PipeRef | null, enabled: boolean) =>
  useQuery({
    queryKey: ref ? pipelineAnnotationsKey(ref) : ["pipeline-annotations", null],
    queryFn: () => api<PipelineAnnotation[]>(`/api/pipeline/annotations?${runQuery(ref!)}`),
    enabled: enabled && !!ref,
    refetchInterval: false,
  });

/** The files a run published — fetched lazily, once `enabled` (the artifacts panel opened). */
export const usePipelineArtifacts = (ref: PipeRef | null, enabled: boolean) =>
  useQuery({
    queryKey: ref ? pipelineArtifactsKey(ref) : ["pipeline-artifacts", null],
    queryFn: () => api<PipelineArtifact[]>(`/api/pipeline/artifacts?${runQuery(ref!)}`),
    enabled: enabled && !!ref,
    refetchInterval: false,
  });

/** One job's log text. A live job is re-fetched whole every 4s (the TUI's `LOG_POLL_TICKS`: the
 *  provider hands back the whole log on every call); a finished one is fetched once and kept.
 *  When a job stops being live it is asked once more, so the tail is complete (the TUI's
 *  `LogView::poll_step`). */
export const usePipelineLogs = (ref: PipeRef | null, jobId: string | null, live: boolean) => {
  const q = useQuery({
    queryKey: ref ? pipelineLogsKey(ref, jobId) : ["pipeline-logs", null],
    queryFn: () => apiGetText(`/api/pipeline/logs?${runQuery(ref!)}&job=${encodeURIComponent(jobId!)}`),
    enabled: !!ref && !!jobId,
    refetchInterval: live ? 4000 : false,
    staleTime: live ? 0 : Infinity,
  });
  const wasLive = useRef(live);
  const { refetch } = q;
  const hasData = q.data !== undefined;
  useEffect(() => {
    if (wasLive.current && !live && hasData) void refetch();
    wasLive.current = live;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [live]);
  return q;
};

/** Re-runs a finished run (only its failed jobs with `failedOnly`). `new_run_id` is the run the
 *  provider started, when it started a new one rather than a new attempt of this one. */
export const rerunPipeline = async (ref: PipeRef, failedOnly: boolean): Promise<{ ok: boolean; new_run_id: string | null }> => {
  const res = await apiPost<{ ok: boolean; new_run_id?: string | null } | undefined>("/api/pipeline/rerun", {
    conn: ref.conn,
    repo: ref.repo ?? undefined,
    run_id: ref.runId,
    failed_only: failedOnly,
  });
  return { ok: res?.ok ?? true, new_run_id: res?.new_run_id ?? null };
};

/** Approves or rejects a pending gate — the body of `PipelineApprovalReq` in actions.rs. */
export const respondPipelineApproval = (ref: PipeRef, approvalId: string, decision: ApprovalDecision, comment?: string) =>
  apiPost("/api/pipeline/approval", {
    conn: ref.conn,
    repo: ref.repo ?? undefined,
    run_id: ref.runId,
    approval_id: approvalId,
    decision,
    comment: comment ?? undefined,
  });

/** The repositories a connection could fetch from — the scope picker's candidate list. Only the
 *  picker calls this, so a provider whose discovery is wrong shows an empty picker; it cannot
 *  stop an already-scoped connection fetching. */
export const fetchConnectionRepositories = (connectionId: string) =>
  api<RepositoryPage>(`/api/connections/repositories?id=${encodeURIComponent(connectionId)}`);

/** A pipeline connection's discovered pipelines and its saved selection — the pipeline
 *  picker's candidates. */
export const fetchPipelineSelection = (connectionId: string) =>
  api<PipelineSelection>(`/api/pipelines/definitions?id=${encodeURIComponent(connectionId)}`);

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
/** Which slice of pull requests the PR page shows; maps to the backend `?view=` param. */
export type PrView = "all" | "yours" | "merged" | "review_requested";
export const usePullRequests = (view: PrView = "all") =>
  useQuery({ queryKey: ["prs", view], queryFn: () => api<PrRow[]>(`/api/pull-requests?view=${view}`) });
export const useWorkItems = () => useQuery({ queryKey: ["work-items"], queryFn: () => api<WiRow[]>("/api/work-items") });
export const usePipelines = () => useQuery({ queryKey: ["pipelines"], queryFn: () => api<PipeRow[]>("/api/pipelines") });
export const useNotifications = () =>
  useQuery({ queryKey: ["notifications"], queryFn: () => api<NotifRow[]>("/api/notifications") });
