/**
 * The pull-request row menu: the same entries wherever a PR is a row (the Pull Requests list,
 * the Command Center). Groups: navigate + copy · act by status · review · destructive.
 *
 * Request bodies and optimistic patches mirror `PrDetail.tsx` (`vote`, `merge`, `revert`,
 * `changeState`, `requestReviewer`). A list knows neither who you are (`me`) nor what the
 * connection supports (`writes`), so votes and merges are not optimistic, and a refusal comes
 * back from the server as a message that is toasted.
 */
import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { apiGet, apiPost, prDetailKey, repoParam } from "../../api";
import { prState, providerMeta } from "../../format";
import { patchPullRequest } from "../../optimistic";
import type { PrRef, PullRequest, User } from "../../types";
import { usePrOpener } from "../PrDetail";
import { copyText, type MenuEntry } from "../ui";
import { openExternal, useRowWrite, type PrRowLike, type RowMenuOpts } from "./shared";

const NO_LINK = "No link from the provider";

/** Why Merge can't run from the row, or `false` when it may be tried. An unknown merge state
 *  (no flag from the provider, or not decorated yet) lets you try — the API decides, as in the
 *  pane. */
function mergeBlocked(pr: PullRequest): string | false {
  if (pr.is_draft || pr.status === "Draft") return "Draft";
  if (pr.mergeable === "Conflicting") return "This PR has merge conflicts";
  if (pr.mergeable === "Blocked") return "Blocked by branch policy";
  const running = pr.check_summary ? pr.check_summary.in_progress > 0 : pr.checks === "Pending";
  if (running) return "Checks are still running";
  if (pr.checks === "Failed") return "Checks failed";
  return false;
}

export function usePrRowMenu(): (row: PrRowLike, opts?: RowMenuOpts) => MenuEntry[] {
  const open = usePrOpener();
  const write = useRowWrite();
  const qc = useQueryClient();

  return useCallback(
    (row: PrRowLike, opts?: RowMenuOpts): MenuEntry[] => {
      const pr = row.pull_request;
      const ref: PrRef = { conn: row.connection_id, repo: pr.repository, id: pr.id };
      const invalidate = [["prs"], ["launchpad"], ["notifications"], ["inbox"], prDetailKey(ref)];
      const url = pr.url ?? null;
      const state = prState(pr).kind;
      const isOpen = state !== "merged" && state !== "closed" && state !== "draft";

      /** A write that changes the PR's standing: tell the caller, then run it. */
      const acted = (label: string, fn: () => Promise<unknown>, o: { optimistic?: () => void; progress?: string } = {}) => {
        opts?.onActed?.();
        return write(label, fn, { ...o, invalidate }).then(() => undefined);
      };
      /** A draft / close / reopen: yours to decide, so it shows at once in every cache. */
      const changeState = (label: string, path: string, body: Record<string, boolean>, edit: (p: PullRequest) => PullRequest) =>
        acted(label, () => apiPost(path, { conn: ref.conn, repo: ref.repo, id: ref.id, ...body }), {
          optimistic: () => patchPullRequest(qc, ref, edit),
        });
      const vote = (v: "Approved" | "Rejected") =>
        acted(
          v === "Approved" ? "Approved" : "Requested changes",
          () => apiPost("/api/pr/vote", { conn: ref.conn, repo: ref.repo, id: ref.id, vote: v }),
          { progress: v === "Approved" ? "Approving…" : "Requesting changes…" },
        );
      const merge = () =>
        acted(
          "Merged",
          async () => {
            try {
              await apiPost("/api/pr/merge", { conn: ref.conn, repo: ref.repo, id: ref.id, strategy: "Merge" });
            } catch {
              // Providers without a mergeable flag (e.g. Bitbucket) let you try — the API decides.
              throw new Error("Couldn't merge — the PR may not be mergeable.");
            }
          },
          { progress: "Merging…" },
        );
      const requestReviewer = (user: User) =>
        write(
          `Review requested from ${user.display_name}`,
          () => apiPost("/api/pr/request-reviewer", { conn: ref.conn, repo: ref.repo, id: ref.id, user_id: user.id }),
          {
            optimistic: () =>
              patchPullRequest(qc, ref, (p) =>
                p.reviewers.some((r) => r.user.id === user.id) ? p : { ...p, reviewers: [...p.reviewers, { user, vote: "NoVote", is_required: false }] },
              ),
            invalidate,
          },
        ).then(() => undefined);
      const loadReviewers = async (): Promise<MenuEntry[]> => {
        const users = await apiGet<User[]>(
          `/api/pr/reviewers?conn=${encodeURIComponent(ref.conn)}&id=${encodeURIComponent(ref.id)}${repoParam(ref.repo)}`,
        );
        if (users.length === 0) return [{ label: "No one to request", disabled: "No one to request" }];
        const already = new Set(pr.reviewers.map((r) => r.user.id));
        return users.map((u): MenuEntry => {
          const requested = already.has(u.id);
          return {
            id: `reviewer:${u.id}`,
            label: u.display_name,
            value: u.handle ?? undefined,
            icon: "user",
            checked: requested || undefined,
            disabled: requested ? "Already requested" : false,
            onSelect: () => requestReviewer(u),
          };
        });
      };

      const entries: MenuEntry[] = [
        { label: "Open", icon: "open-pane", hint: "↵", onSelect: () => void open(ref) },
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
      if (pr.number != null) {
        const n = `#${pr.number}`;
        entries.push({ id: "copy-number", label: `Copy ${n}`, icon: "copy", value: n, onSelect: () => copyText(n, n) });
      }

      entries.push({ type: "separator" });
      if (isOpen) {
        entries.push(
          { label: "Approve", icon: "check", iconColor: "var(--green)", onSelect: () => vote("Approved") },
          { label: "Request changes", icon: "x", iconColor: "var(--yellow)", onSelect: () => vote("Rejected") },
          { label: "Merge", icon: "pr-merged", iconColor: "var(--magenta)", disabled: mergeBlocked(pr), onSelect: merge },
        );
      } else if (state === "draft") {
        entries.push({
          label: "Mark ready for review",
          onSelect: () =>
            changeState("Marked ready for review", "/api/pr/draft", { draft: false }, (p) => ({ ...p, is_draft: false, status: "Open" })),
        });
      } else if (state === "closed") {
        entries.push({
          label: "Reopen",
          onSelect: () =>
            changeState("Reopened", "/api/pr/closed", { closed: false }, (p) => ({ ...p, status: p.is_draft ? "Draft" : "Open" })),
        });
      } else if (state === "merged") {
        entries.push({
          label: "Revert",
          onSelect: () =>
            write("Revert requested", () => apiPost("/api/pr/revert", { conn: ref.conn, repo: ref.repo, id: ref.id }), {
              progress: "Reverting…",
              invalidate,
            }).then(() => undefined),
        });
      }

      if (isOpen) {
        entries.push(
          { type: "separator" },
          { label: "Request reviewer", icon: "user-plus", submenu: loadReviewers },
          {
            label: "Convert to draft",
            onSelect: () =>
              changeState("Converted to draft", "/api/pr/draft", { draft: true }, (p) => ({ ...p, is_draft: true, status: "Draft" })),
          },
        );
      }

      if (isOpen || state === "draft") {
        entries.push(
          { type: "separator" },
          {
            label: "Close pull request",
            icon: "x",
            hint: "X",
            danger: true,
            confirm: "Click again to close",
            onSelect: () => changeState("Closed", "/api/pr/closed", { closed: true }, (p) => ({ ...p, status: "Closed" })),
          },
        );
      }

      if (opts?.extra?.length) entries.push({ type: "separator" }, ...opts.extra);
      return entries;
    },
    [open, write, qc],
  );
}
