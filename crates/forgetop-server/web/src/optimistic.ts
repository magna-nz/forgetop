/**
 * Optimistic cache edits: what the dashboard shows the instant a mutation is issued, before the
 * refetch that confirms it lands.
 *
 * The rule these all follow: apply the edit locally, fire the request, then invalidate. The
 * refetch resolves onto an identical screen, so the network round trip stops being the moment
 * the user sees their own action take effect.
 *
 * Only ever encode a post-state that is genuinely knowable client-side. An action whose outcome
 * the server decides — a merge that may be refused, an id the server assigns — must wait for the
 * real answer rather than show a guess that can be wrong.
 */
import type { QueryClient } from "@tanstack/react-query";
import { prDetailKey, wiDetailKey } from "./api";
import type {
  Comment,
  CommentThread,
  ConnectionRow,
  HealthRow,
  LaunchpadResponse,
  NotifRow,
  PipelineSelection,
  PipeRow,
  PrDetail as PrDetailData,
  PrRef,
  PrRow,
  PullRequest,
  ReviewVote,
  User,
  WiDetail as WiDetailData,
  WiRef,
  WiRow,
  WorkItem,
} from "./types";

/** Lists whose rows each carry the connection they came from. Keys match by prefix, so "prs"
 *  covers every ["prs", view] the user has visited, not only the one on screen. */
const CONNECTION_TAGGED_LISTS = ["prs", "work-items", "pipelines", "notifications"];

/**
 * Drops everything a just-removed connection contributed.
 *
 * Kept separate from the component so it can be tested against a real QueryClient without a
 * render, a fetch stub, or a race against the invalidation that follows it.
 */
export function dropConnectionFromCache(qc: QueryClient, id: string): void {
  qc.setQueriesData<ConnectionRow[]>({ queryKey: ["connections"] }, (rows) => rows?.filter((c) => c.id !== id));
  qc.setQueriesData<HealthRow[]>({ queryKey: ["health"] }, (rows) => rows?.filter((h) => h.connection_id !== id));
  for (const key of CONNECTION_TAGGED_LISTS) {
    qc.setQueriesData<{ connection_id: string }[]>({ queryKey: [key] }, (rows) =>
      rows?.filter((r) => r.connection_id !== id),
    );
  }
  qc.setQueriesData<LaunchpadResponse>({ queryKey: ["launchpad"] }, (lp) =>
    lp && { ...lp, rows: lp.rows.filter((r) => r.connection_id !== id) },
  );
}

/**
 * Marks one notification read.
 *
 * The inbox list and the bell badge both derive from the same `["notifications"]` query, so the
 * row dimming and the count dropping are one edit, not two.
 */
export function markNotificationReadInCache(qc: QueryClient, conn: string, id: string): void {
  qc.setQueriesData<NotifRow[]>({ queryKey: ["notifications"] }, (rows) =>
    rows?.map((r) =>
      r.connection_id === conn && r.notification.id === id
        ? { ...r, notification: { ...r.notification, unread: false } }
        : r,
    ),
  );
}

/**
 * Applies a work-item edit to all three caches the same item lives in: its detail pane, the
 * work-items list, and the Command Center's `wi` rows. A per-action helper would be three
 * near-copies, and the one that got forgotten would be the pane the user is looking at.
 *
 * `patch` must only carry fields the caller genuinely knows — a moved state's
 * `state_category`, for instance, isn't derivable from the state name the picker returns, so it
 * is left stale until the refetch rather than guessed.
 */
export function patchWorkItem(qc: QueryClient, ref: Pick<WiRef, "conn" | "repo" | "id">, patch: Partial<WorkItem>): void {
  // An id only names one item *within a repository* — a connection spans an account, so an
  // addressed ref has to match the repository too. An unaddressed ref falls back to the id alone,
  // which on a multi-repository connection can touch more than one row; the server is stricter
  // and errors rather than guessing, so the refetch is what corrects it. Harmless because the
  // only ref without a repo comes from a detail pane that is showing one specific item.
  const isTarget = (connectionId: string, wi: WorkItem) =>
    connectionId === ref.conn && wi.id === ref.id && (!ref.repo || !wi.repository || wi.repository === ref.repo);
  const apply = (wi: WorkItem): WorkItem => ({ ...wi, ...patch });

  // Detail keys are exact, not prefixes — build it the one way `api.ts` builds it.
  qc.setQueryData<WiDetailData>(wiDetailKey(ref), (d) => d && { ...d, work_item: apply(d.work_item) });
  qc.setQueriesData<WiRow[]>({ queryKey: ["work-items"] }, (rows) =>
    rows?.map((r) => (isTarget(r.connection_id, r.work_item) ? { ...r, work_item: apply(r.work_item) } : r)),
  );
  qc.setQueriesData<LaunchpadResponse>({ queryKey: ["launchpad"] }, (lp) =>
    lp && {
      ...lp,
      rows: lp.rows.map((r) =>
        r.kind === "wi" && isTarget(r.connection_id, r.work_item) ? { ...r, work_item: apply(r.work_item) } : r,
      ),
    },
  );
}

/**
 * Records a connection's new repository scope.
 *
 * Only the connection row is knowable: the scope gates what the server *fetches*, so the item
 * lists it governs genuinely have to go back to the provider. This is what lets the picker's
 * "Repos · N of M" label move on the click while those lists reload behind it.
 */
export function setConnectionScopeInCache(qc: QueryClient, id: string, scope: string[]): void {
  qc.setQueriesData<ConnectionRow[]>({ queryKey: ["connections"] }, (rows) =>
    rows?.map((c) => (c.id === id ? { ...c, repo_scope: [...scope] } : c)),
  );
}

/**
 * Applies a new pipeline selection for one connection: the picker's ticks and count move at once,
 * and runs of a pipeline just unticked leave the list before the refetch lands. Newly ticked
 * pipelines' runs can't be known client-side, so they arrive with the refetch.
 */
export function setPipelineSelectionInCache(qc: QueryClient, id: string, all: boolean, selected: string[]): void {
  qc.setQueryData<PipelineSelection>(["pipeline-definitions", id], (cur) => cur && { ...cur, all, selected });
  if (all) return;
  qc.setQueriesData<PipeRow[]>({ queryKey: ["pipelines"] }, (rows) =>
    rows?.filter((r) => r.connection_id !== id || selected.includes(r.run.definition_id)),
  );
}

/**
 * Applies an edit to a pull request in every cache it lives in: its detail pane, every PR list,
 * and the Command Center's `pr` rows — the PR twin of {@link patchWorkItem}, matched the same way.
 */
export function patchPullRequest(qc: QueryClient, ref: Pick<PrRef, "conn" | "repo" | "id">, edit: (pr: PullRequest) => PullRequest): void {
  const isTarget = (connectionId: string, pr: PullRequest) =>
    connectionId === ref.conn && pr.id === ref.id && (!ref.repo || !pr.repository || pr.repository === ref.repo);

  qc.setQueryData<PrDetailData>(prDetailKey(ref), (d) => d && { ...d, pull_request: edit(d.pull_request) });
  qc.setQueriesData<PrRow[]>({ queryKey: ["prs"] }, (rows) =>
    rows?.map((r) => (isTarget(r.connection_id, r.pull_request) ? { ...r, pull_request: edit(r.pull_request) } : r)),
  );
  qc.setQueriesData<LaunchpadResponse>({ queryKey: ["launchpad"] }, (lp) =>
    lp && {
      ...lp,
      rows: lp.rows.map((r) =>
        r.kind === "pr" && isTarget(r.connection_id, r.pull_request) ? { ...r, pull_request: edit(r.pull_request) } : r,
      ),
    },
  );
}

/** You, as a placeholder comment or reviewer entry shows you before the provider has. */
export function meAsUser(me?: string | null): User {
  return { id: me ?? "you", display_name: me ?? "You", handle: me ?? null, avatar_url: null };
}

const sameUser = (u: User, me: string) =>
  [u.handle, u.display_name, u.id].some((n) => n != null && n.toLowerCase() === me.toLowerCase());

/** `pr` with your verdict recorded on it — added as a reviewer if you weren't one. Your verdict is
 *  yours to decide, so it is knowable client-side; who *you* are comes from the detail's `me`. */
export function withVote(pr: PullRequest, me: string, vote: ReviewVote): PullRequest {
  const mine = pr.reviewers.some((r) => sameUser(r.user, me));
  return {
    ...pr,
    reviewers: mine
      ? pr.reviewers.map((r) => (sameUser(r.user, me) ? { ...r, vote } : r))
      : [...pr.reviewers, { user: meAsUser(me), vote, is_required: false }],
  };
}

/** Placeholder ids start with this. They mean nothing to the provider, so nothing may be sent
 *  against one (a reply to a placeholder thread) until the refetch swaps in the real copy. */
const LOCAL_ID = "local-";
let localSeq = 0;

export const isLocalId = (id: string) => id.startsWith(LOCAL_ID);

/** A comment as shown before the provider lists it. */
export function localComment(author: User, body: string): Comment {
  return { id: `${LOCAL_ID}${Date.now()}-${localSeq++}`, author, body, created_at: new Date().toISOString() };
}

/** A new thread holding one placeholder comment: a top-level comment, or a review's line comment. */
export function localThread(author: User, body: string, at?: { path: string; line: number }): CommentThread {
  const comment = localComment(author, body);
  return { id: comment.id, comments: [comment], file_path: at?.path ?? null, line: at?.line ?? null, is_resolved: false };
}

/** Adds placeholder threads to a PR's detail pane. */
export function addPrThreads(qc: QueryClient, ref: Pick<PrRef, "conn" | "repo" | "id">, threads: CommentThread[]): void {
  qc.setQueryData<PrDetailData>(prDetailKey(ref), (d) => d && { ...d, threads: [...d.threads, ...threads] });
}

/** Adds a placeholder reply to one of a PR's threads. */
export function addPrReply(qc: QueryClient, ref: Pick<PrRef, "conn" | "repo" | "id">, threadId: string, comment: Comment): void {
  qc.setQueryData<PrDetailData>(prDetailKey(ref), (d) =>
    d && { ...d, threads: d.threads.map((t) => (t.id === threadId ? { ...t, comments: [...t.comments, comment] } : t)) },
  );
}

/** Adds a placeholder thread to a work item's detail pane. */
export function addWiThread(qc: QueryClient, ref: Pick<WiRef, "conn" | "repo" | "id">, thread: CommentThread): void {
  qc.setQueryData<WiDetailData>(wiDetailKey(ref), (d) => d && { ...d, threads: [...d.threads, thread] });
}
