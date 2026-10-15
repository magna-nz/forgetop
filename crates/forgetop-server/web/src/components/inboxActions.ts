/**
 * What every inbox surface (the Notifications page, the bell, the row menu, the pane's triage
 * strip) does to a thread: open it at its target, mark it read, mark it done. One hook, so the
 * page and the bell can't drift apart.
 *
 * Every write is optimistic (the standing rule): the caches change first, then the request goes
 * out, then the inbox, the flat list and the Command Center are invalidated — after a success to
 * confirm, after a failure as the rollback.
 */
import { useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { apiPost, inboxKey, NOTIFICATION_QUERIES } from "../api";
import { providerMeta } from "../format";
import { markThreadReadInCache, removeThreadFromCache } from "../optimistic";
import type { InboxThread, Notification, NotifRow } from "../types";
import { usePrOpener, type PrOpenTarget } from "./PrDetail";
import type { InboxOrigin } from "./TriageStrip";
import { notify } from "./ui";
import { useWiOpener } from "./WiDetail";

/** Whether a notification's item opens in a pane here (a PR or a work item with an id); anything
 *  else opens its provider link. */
export const opensInApp = (n: Notification) => n.item_id != null && (n.item_type === "PullRequest" || n.item_type === "WorkItem");

/** Where in the PR pane a notification lands. A state change and the item itself open as usual. */
export function prTarget(n: Notification): Pick<PrOpenTarget, "tab" | "commentId"> {
  switch (n.target?.kind) {
    case "Comment":
      return { tab: "conversation", commentId: n.target.id };
    case "Checks":
      return { tab: "checks" };
    case "Diff":
      return { tab: "files" };
    default:
      return {};
  }
}

/** A lead whose feed entry left the who and what out, and can fetch them. */
export const needsDetail = (n: Notification) => !!n.detail_ref && !n.actor && !n.excerpt;

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export interface InboxActions {
  /** Opens a thread's item at its target, as the inbox holds it now (a detail fetched since may
   *  have pointed it at a comment), and marks the thread read once its pane has opened; `order`
   *  is the list the caller shows, which is what the pane's `j`/`k` walk. False when the open
   *  pane's leave guard refused — nothing opened and nothing was marked read. */
  open: (thread: InboxThread, order: InboxThread[]) => boolean;
  markRead: (thread: InboxThread) => Promise<void>;
  done: (thread: InboxThread) => Promise<void>;
  /** Done on several threads at once (the bell's "Mark all done"): one request per connection. */
  doneAll: (threads: InboxThread[]) => Promise<void>;
}

export function useInboxActions(): InboxActions {
  const qc = useQueryClient();
  const openPr = usePrOpener();
  const openWi = useWiOpener();

  return useMemo(() => {
    const invalidate = () => NOTIFICATION_QUERIES.forEach((queryKey) => qc.invalidateQueries({ queryKey }));
    const connOf = (t: InboxThread) => t.members[0]?.conn ?? t.lead.connection_id;

    async function markRead(t: InboxThread) {
      const ids = t.members.filter((m) => m.unread).map((m) => m.id);
      if (!t.unread && ids.length === 0) return;
      markThreadReadInCache(qc, t);
      try {
        if (ids.length > 0) await apiPost("/api/notification/read-many", { conn: connOf(t), ids });
      } catch (e) {
        notify(`Couldn't mark read: ${errorText(e)}`, { error: true });
      } finally {
        invalidate();
      }
    }

    async function doneAll(threads: InboxThread[]) {
      if (threads.length === 0) return;
      threads.forEach((t) => removeThreadFromCache(qc, t));
      const byConn = new Map<string, string[]>();
      for (const t of threads) byConn.set(connOf(t), [...(byConn.get(connOf(t)) ?? []), ...t.members.map((m) => m.id)]);
      try {
        await Promise.all([...byConn].map(([conn, ids]) => apiPost("/api/notification/done", { conn, ids })));
      } catch (e) {
        notify(`Couldn't mark done: ${errorText(e)}`, { error: true });
      } finally {
        invalidate();
      }
    }

    const done = (t: InboxThread) => doneAll([t]);

    /** The thread under `key` as the inbox holds it now, with the walk order resolved the same
     *  way. False when it is gone or its pane didn't open. */
    function openKey(key: string, keys: string[]): boolean {
      const threads = qc.getQueryData<InboxThread[]>(inboxKey) ?? [];
      const byKey = new Map(threads.map((t) => [t.key, t]));
      const thread = byKey.get(key);
      if (!thread) return false;
      return open(
        thread,
        keys.map((k) => byKey.get(k)).filter((t): t is InboxThread => !!t),
      );
    }

    function open(given: InboxThread, order: InboxThread[]): boolean {
      // The caller's copy can predate a detail that has since landed in the cache (and moved the
      // target from the item to the comment that mentions you): open the thread as it is now.
      const thread = qc.getQueryData<InboxThread[]>(inboxKey)?.find((t) => t.key === given.key) ?? given;
      const lead: NotifRow = thread.lead;
      const n = lead.notification;
      if (!opensInApp(n)) {
        void markRead(thread);
        if (n.url) window.open(n.url, "_blank", "noopener,noreferrer");
        return true;
      }
      // The pane walks the threads that open in a pane; a link-only one would leave it.
      const keys = order.filter((t) => opensInApp(t.lead.notification)).map((t) => t.key);
      if (!keys.includes(thread.key)) keys.unshift(thread.key);
      const i = keys.indexOf(thread.key);
      const fromInbox: InboxOrigin = {
        key: thread.key,
        index: i + 1,
        total: keys.length,
        phrase: lead.phrase,
        why: lead.why,
        actor: n.actor,
        excerpt: n.excerpt,
        conn: lead.connection_id,
        notificationId: n.id,
        needsDetail: needsDetail(n),
        url: n.url,
        providerLabel: providerMeta(lead.provider).label,
        onPrev: i > 0 ? () => openKey(keys[i - 1], keys) : undefined,
        onNext: i < keys.length - 1 ? () => openKey(keys[i + 1], keys) : undefined,
        onDone: () => {
          const next = keys[i + 1] ?? keys[i - 1];
          void done(thread);
          if (!next) return "none";
          if (
            openKey(
              next,
              keys.filter((k) => k !== thread.key),
            )
          )
            return "next";
          // Done stands; the move on didn't happen (the pane kept unsaved work, or the next
          // thread has gone), so say so rather than leave a pane that looks like it moved.
          notify("Marked done — the next thread didn't open");
          return "stayed";
        },
      };
      const ref = { conn: lead.connection_id, repo: n.repository, id: n.item_id! };
      // Read only once the pane is showing it: a pane that refused to leave (unsubmitted
      // comments) means this thread was never seen.
      const opened =
        n.item_type === "PullRequest"
          ? openPr(ref, { ...prTarget(n), fromInbox })
          : openWi(ref, undefined, { commentId: n.target?.kind === "Comment" ? n.target.id : undefined, fromInbox });
      if (opened) void markRead(thread);
      return opened;
    }

    return { open, markRead, done, doneAll };
  }, [qc, openPr, openWi]);
}
