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
import { wiDetailKey } from "./api";
import type {
  ConnectionRow,
  HealthRow,
  LaunchpadResponse,
  NotifRow,
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
  // addressed ref has to match the repository too. An unaddressed ref (or a provider that isn't
  // repo-addressed, which leaves `repository` unset) resolves on the id alone, exactly as the
  // server does.
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
