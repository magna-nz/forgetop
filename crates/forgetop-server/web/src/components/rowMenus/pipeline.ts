/**
 * The pipeline-run row menu: the same entries wherever a run is a row (the Pipelines list, the
 * Command Center). Groups: navigate + copy · act (gates, re-runs) · look closer · destructive.
 *
 * Request bodies mirror `PipelineDetail.tsx` (`rerun`, `cancel`, `respond`) through the same
 * `api.ts` helpers. A list row carries no jobs and no capabilities, so re-runs go by the run's
 * status, are not optimistic (the provider decides whether a new run, with a new id, starts),
 * and a provider that refuses answers with a message that is toasted.
 */
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { apiPost, pipelineDetailKey, rerunPipeline, respondPipelineApproval } from "../../api";
import { providerMeta } from "../../format";
import { isActive } from "../../pipeline/grouping";
import type { LaunchpadResponse, PipeRef, PipelineApproval, PipelineDetail, PipelineRun, PipeRow } from "../../types";
import { usePipelineOpener } from "../PipelineDetail";
import { markCanceled, markResponded, NO_CAPS, prLink } from "../pipeline/runState";
import { copyText, type MenuEntry } from "../ui";
import { openExternal, useRowWrite, type PipeRowLike, type RowMenuOpts } from "./shared";

const NO_LINK = "No link from the provider";
const NO_FAILED_JOB = "No job has failed";
const STILL_RUNNING = "Still running";

/**
 * Applies an edit to a run in every cache it lives in: its detail pane, every pipelines list,
 * and the Command Center's `pipe` rows — the run twin of `patchPullRequest`. The edit takes a
 * detail so the pane's own `markCanceled` / `markResponded` apply unchanged; a list row is
 * lifted into one (it has no capabilities, and a Command Center row no approvals) and only its
 * run and approvals are written back. Fetches in flight are canceled first — landing after the
 * patch, they would undo it.
 */
export function patchPipelineRun(qc: QueryClient, ref: PipeRef, edit: (d: PipelineDetail) => PipelineDetail): void {
  const isTarget = (connectionId: string, run: PipelineRun) =>
    connectionId === ref.conn && run.id === ref.runId && (!ref.repo || !run.repository || run.repository === ref.repo);
  const lift = (run: PipelineRun, approvals: PipelineApproval[]): PipelineDetail => edit({ run, approvals, capabilities: NO_CAPS });

  void qc.cancelQueries({ queryKey: pipelineDetailKey(ref) });
  void qc.cancelQueries({ queryKey: ["pipelines"] });
  void qc.cancelQueries({ queryKey: ["launchpad"] });

  qc.setQueryData<PipelineDetail>(pipelineDetailKey(ref), (d) => d && edit(d));
  qc.setQueriesData<PipeRow[]>({ queryKey: ["pipelines"] }, (rows) =>
    rows?.map((r) => {
      if (!isTarget(r.connection_id, r.run)) return r;
      const next = lift(r.run, r.approvals);
      return { ...r, run: next.run, approvals: next.approvals };
    }),
  );
  qc.setQueriesData<LaunchpadResponse>({ queryKey: ["launchpad"] }, (lp) =>
    lp && {
      ...lp,
      rows: lp.rows.map((r) => (r.kind === "pipe" && isTarget(r.connection_id, r.run) ? { ...r, run: lift(r.run, []).run } : r)),
    },
  );
}

export function usePipelineRowMenu(): (row: PipeRowLike, opts?: RowMenuOpts) => MenuEntry[] {
  const open = usePipelineOpener();
  const write = useRowWrite();
  const qc = useQueryClient();

  return useCallback(
    (row: PipeRowLike, opts?: RowMenuOpts): MenuEntry[] => {
      const run = row.run;
      const ref: PipeRef = { conn: row.connection_id, repo: run.repository, runId: run.id };
      const invalidate = [["pipelines"], ["launchpad"], pipelineDetailKey(ref)];
      const url = run.url ?? null;
      const active = isActive(run.status);
      const pr = prLink(run);

      /** A write that changes the run's standing: tell the caller, then run it. */
      const acted = (label: string, fn: () => Promise<unknown>, o: { optimistic?: () => void; progress?: string } = {}) => {
        opts?.onActed?.();
        return write(label, fn, { ...o, invalidate }).then(() => undefined);
      };
      const respond = (a: PipelineApproval, decision: "Approve" | "Reject") =>
        acted(decision === "Approve" ? `Approved ${a.name}` : `Rejected ${a.name}`, () => respondPipelineApproval(ref, a.id, decision), {
          optimistic: () => patchPipelineRun(qc, ref, (d) => markResponded(d, a, decision)),
        });
      const rerun = (failedOnly: boolean) =>
        acted(failedOnly ? "Re-running failed jobs" : "Re-run started", () => rerunPipeline(ref, failedOnly), {
          progress: "Re-run requested…",
        });
      const cancel = () =>
        acted("Cancel requested", () => apiPost("/api/pipeline/cancel", { conn: ref.conn, repo: ref.repo, run_id: ref.runId }), {
          optimistic: () => patchPipelineRun(qc, ref, markCanceled),
        });

      const entries: MenuEntry[] = [
        { label: "Open", icon: "open-pane", hint: "↵", onSelect: () => open(ref) },
        {
          label: `Open in ${providerMeta(row.provider).label}`,
          icon: "external",
          hint: "o",
          disabled: url ? false : NO_LINK,
          onSelect: () => {
            if (url) openExternal(url);
          },
        },
        {
          label: "Copy link",
          icon: "link",
          hint: "y",
          disabled: url ? false : NO_LINK,
          onSelect: () => (url ? copyText(url, "link") : undefined),
        },
      ];
      const sha = run.commit_sha;
      if (sha) entries.push({ label: "Copy commit", icon: "copy", value: sha.slice(0, 7), hint: "c", onSelect: () => copyText(sha, "commit") });
      const branch = run.branch;
      if (branch) entries.push({ label: "Copy branch", icon: "git-branch", value: branch, onSelect: () => copyText(branch, "branch") });

      entries.push({ type: "separator" });
      for (const a of (row.approvals ?? []).filter((g) => g.can_respond)) {
        entries.push(
          {
            id: `approve:${a.id}`,
            label: `Approve gate ${a.name}`,
            icon: "shield",
            iconColor: "var(--green)",
            onSelect: () => respond(a, "Approve"),
          },
          {
            id: `reject:${a.id}`,
            label: `Reject gate ${a.name}`,
            icon: "shield",
            iconColor: "var(--yellow)",
            onSelect: () => respond(a, "Reject"),
          },
        );
      }
      const failedOrCanceled = run.status === "Failed" || run.status === "Canceled";
      entries.push(
        { label: "Re-run", icon: "rerun", hint: "r", disabled: active ? STILL_RUNNING : false, onSelect: () => rerun(false) },
        {
          label: "Re-run failed jobs",
          icon: "rerun",
          disabled: active ? STILL_RUNNING : failedOrCanceled ? false : NO_FAILED_JOB,
          onSelect: () => rerun(true),
        },
      );

      entries.push(
        { type: "separator" },
        {
          label: "Show failing log",
          icon: "logs",
          disabled: run.status === "Failed" ? false : NO_FAILED_JOB,
          onSelect: () => open(ref, { showLog: true }),
        },
      );
      if (pr) {
        const n = `#${run.pull_request}`;
        entries.push({ label: `Open pull request ${n}`, icon: "pull-request", value: n, onSelect: () => openExternal(pr) });
      }

      if (active) {
        entries.push(
          { type: "separator" },
          { label: "Cancel run", icon: "square", hint: "X", danger: true, confirm: "Click again to cancel", onSelect: cancel },
        );
      }

      if (opts?.extra?.length) entries.push({ type: "separator" }, ...opts.extra);
      return entries;
    },
    [open, write, qc],
  );
}
