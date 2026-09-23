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
import type { ConnectionRow, HealthRow, LaunchpadResponse } from "./types";

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
