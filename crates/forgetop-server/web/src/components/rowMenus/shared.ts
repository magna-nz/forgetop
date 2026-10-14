/**
 * What every row menu builder shares: the row shapes they accept, the options a list can pass,
 * and the one way a menu item runs a write.
 *
 * A builder is returned by a hook (`usePrRowMenu()`, …) because hooks cannot run per row; the
 * list calls the builder lazily when a menu opens, so a closed menu costs a row nothing.
 */
import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { notify } from "../ui";
import type { PipelineApproval, PipelineRun, ProviderType, PullRequest, WorkItem } from "../../types";

export interface RowMenuOpts {
  /** Called right after a standing-changing write is fired (vote, merge, close, draft, state
   *  move, assign, cancel, re-run, gate decision) — before it resolves. The Command Center uses
   *  it to drop the row from its bucket at once. */
  onActed?: () => void;
  /** Extra entries appended after the builder's own (the Command Center's Dismiss). */
  extra?: import("../ui").MenuEntry[];
}

/** The fields a PR row carries, in a list or in the Command Center. */
export type PrRowLike = { connection_id: string; connection: string; provider: ProviderType; pull_request: PullRequest };
/** The fields a work-item row carries, in a list or in the Command Center. */
export type WiRowLike = { connection_id: string; connection: string; provider: ProviderType; work_item: WorkItem };
/** The fields a pipeline row carries. The Command Center's rows have no `approvals`. */
export type PipeRowLike = {
  connection_id: string;
  connection: string;
  provider: ProviderType;
  run: PipelineRun;
  definition_name?: string | null;
  approvals?: PipelineApproval[];
};

/**
 * Runs a write from a menu item: `optimistic` first (what the write decides on its own shows at
 * once), then the request, then the listed queries are invalidated — after a success to confirm
 * the optimistic patch, after a failure as the rollback. The outcome is a toast, since a row has
 * no note line of its own. Returns whether the write succeeded.
 */
export function useRowWrite(): (
  label: string,
  fn: () => Promise<unknown>,
  opts: { optimistic?: () => void; invalidate: string[][]; progress?: string },
) => Promise<boolean> {
  const qc = useQueryClient();
  return useCallback(
    async (label, fn, opts) => {
      opts.optimistic?.();
      if (opts.optimistic) notify(`${label} ✓`);
      else if (opts.progress) notify(opts.progress);
      let ok = false;
      try {
        await fn();
        if (!opts.optimistic) notify(`${label} ✓`);
        ok = true;
      } catch (e) {
        notify(e instanceof Error ? e.message : String(e), { error: true });
      } finally {
        for (const key of opts.invalidate) qc.invalidateQueries({ queryKey: key });
      }
      return ok;
    },
    [qc],
  );
}

/** Opens a provider URL in a new tab, the way every "Open in …" affordance does. */
export function openExternal(url: string): void {
  window.open(url, "_blank", "noreferrer");
}
