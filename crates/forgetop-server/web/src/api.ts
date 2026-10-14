import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { ApprovalDecision, ConnectionRow, FileChange, HealthRow, Highlight, HlSpan, LaunchpadResponse, NotifRow, PipeRef, PipelineAnnotation, PipelineArtifact, PipelineDetail, PipelineSelection, PipeRow, PrDecoration, PrDetail, PrRef, ProviderInfo, PrRow, RepositoryPage, User, WiDetail, WiRef, WiRow } from "./types";

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
  noteCacheHeader(path, res);
  return (await res.json()) as T;
}

// ---- the shared cache ----------------------------------------------------------------------
//
// A list the server answered from the cache it shares with the terminal UI carries this header
// with the time that data was fetched. The page shows it at once, and while it is older than a
// poll interval polls fast until a live answer (no header) replaces it.

const FETCHED_AT_HEADER = "x-forgetop-fetched-at";
/** How old a cached answer may be before the page treats it as stale and polls for a live one. */
export const STALE_AFTER_MS = 30_000;
const POLL_MS = 15_000;
const STALE_POLL_MS = 2_000;

/** path → when the data in the last answer was fetched, for answers that came from the cache. */
const cachedAt = new Map<string, number>();
const listeners = new Set<() => void>();

function noteCacheHeader(path: string, res: Response) {
  const at = res.headers?.get?.(FETCHED_AT_HEADER);
  const t = at ? Date.parse(at) : NaN;
  const before = cachedAt.get(path);
  if (Number.isFinite(t)) cachedAt.set(path, t);
  else cachedAt.delete(path);
  if (before !== cachedAt.get(path)) listeners.forEach((l) => l());
}

/** Whether the last answer for `path` came from the cache and is older than `STALE_AFTER_MS`. */
export function isStale(path: string, now = Date.now()): boolean {
  const t = cachedAt.get(path);
  return t != null && now - t > STALE_AFTER_MS;
}

/** The age of the oldest stale list on screen, in ms, or null when everything is live. */
export function oldestStaleAgeMs(now = Date.now()): number | null {
  let oldest: number | null = null;
  for (const t of cachedAt.values()) {
    if (now - t > STALE_AFTER_MS && (oldest === null || t < oldest)) oldest = t;
  }
  return oldest === null ? null : now - oldest;
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};
let snapshot = 0;
const bump = () => {
  snapshot++;
};
listeners.add(bump);

/** Re-renders when a list's cache status changes; returns the oldest stale age (see above). */
export function useCacheStatus(): number | null {
  useSyncExternalStore(subscribe, () => snapshot);
  return oldestStaleAgeMs();
}

/** Query options for a polled list: fast polls while its last answer was stale from the cache. */
function listQuery<T>(key: readonly unknown[], path: string) {
  return {
    queryKey: key,
    queryFn: () => api<T>(path),
    refetchInterval: () => (isStale(path) ? STALE_POLL_MS : POLL_MS),
  };
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

/** A cheap, stable digest of `lines` for a query key — the text itself would make the key
 *  as big as the file. FNV-1a over the joined text; collisions only cost a wrong colour. */
function digestLines(lines: string[]): string {
  let h = 0x811c9dc5;
  for (const line of lines) {
    for (let i = 0; i < line.length; i++) {
      h ^= line.charCodeAt(i);
      h = Math.imul(h, 0x01000193);
    }
    h ^= 0x0a; // the newline between lines
    h = Math.imul(h, 0x01000193);
  }
  return `${lines.length}:${(h >>> 0).toString(16)}`;
}

/** Syntax spans for `lines` as the file at `path`, tokenised by the server with the grammars
 *  the TUI paints its diffs with (`POST /api/highlight`). `data` is one span list per line,
 *  or `null` — an unknown language, or an answer without spans — which paints the text plain,
 *  exactly as before. Keyed on the text's digest rather than the PR, so reopening a file
 *  reuses the answer and another commit's patch is its own entry. */
export const useHighlight = (path: string | null, lines: string[] | null) =>
  useQuery({
    queryKey: ["highlight", path ?? "", lines ? digestLines(lines) : ""],
    queryFn: async (): Promise<HlSpan[][] | null> => {
      const res = await apiPost<Highlight>("/api/highlight", { path, lines });
      return Array.isArray(res?.lines) ? res.lines : null;
    },
    enabled: !!path && !!lines && lines.length > 0,
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

export const useLaunchpad = () => useQuery(listQuery<LaunchpadResponse>(["launchpad"], "/api/launchpad"));
export const useHealth = () => useQuery({ queryKey: ["health"], queryFn: () => api<HealthRow[]>("/api/health") });
export const useProviders = () =>
  useQuery({ queryKey: ["providers"], queryFn: () => api<ProviderInfo[]>("/api/providers"), staleTime: Infinity });
export const useConnections = () => useQuery({ queryKey: ["connections"], queryFn: () => api<ConnectionRow[]>("/api/connections") });
/** Which slice of pull requests the PR page shows; maps to the backend `?view=` param. */
export type PrView = "all" | "yours" | "merged" | "review_requested";
export const usePullRequests = (view: PrView = "all") => useQuery(listQuery<PrRow[]>(["prs", view], `/api/pull-requests?view=${view}`));
export const useWorkItems = () => useQuery(listQuery<WiRow[]>(["work-items"], "/api/work-items"));
export const usePipelines = () => useQuery(listQuery<PipeRow[]>(["pipelines"], "/api/pipelines"));
export const useNotifications = () => useQuery(listQuery<NotifRow[]>(["notifications"], "/api/notifications"));
