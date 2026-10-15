/**
 * The inbox's triage strip: the line a PR or work-item pane shows above its title when it was
 * opened from the inbox — where you are in the inbox ("Inbox · 2 of 6"), why this reached you,
 * and the keys to move on (`k`/`j`, `e` for done).
 *
 * The pane knows nothing about the inbox: the opener hands it an {@link InboxOrigin} whose
 * callbacks do the moving. That keeps this module free of the pane modules (no import cycle) and
 * lets the Notifications page and the bell each say what "next" means for the list they show.
 */
import { useEffect, useSyncExternalStore } from "react";
import { useNotificationDetail } from "../api";
import type { CommentThread } from "../types";
import { Icon } from "./icons";
import { Button, Kbd } from "./ui";

/** Where a pane was opened from in the inbox, and how to move on from it. */
export interface InboxOrigin {
  /** The thread's key (`InboxThread.key`). */
  key: string;
  /** 1-based position among the threads the opener was showing, and how many there were. */
  index: number;
  total: number;
  phrase: string;
  why: string;
  actor?: string | null;
  excerpt?: string | null;
  /** The lead notification, for fetching its detail when the feed left actor/excerpt out. */
  conn: string;
  notificationId: string;
  needsDetail: boolean;
  url?: string | null;
  providerLabel: string;
  /** Open the previous / next thread in place; absent at either end. */
  onPrev?: () => void;
  onNext?: () => void;
  /** Marks the thread done and opens the next one: `next` when it opened, `none` when there was
   *  none left (the pane then closes), `stayed` when the next didn't open (the pane's leave guard
   *  refused) — the pane stays as it is. */
  onDone: () => DoneOutcome;
}

/** What the strip's Done led to; see {@link InboxOrigin.onDone}. */
export type DoneOutcome = "next" | "none" | "stayed";

// ---- which thread's pane is open ----
//
// The Notifications page marks that thread's row selected, and leaves j/k/e to the strip while
// a pane opened from the inbox is up (one handler per key, never two).

let activeKey: string | null = null;
const listeners = new Set<() => void>();
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};
const setActive = (key: string | null) => {
  if (activeKey === key) return;
  activeKey = key;
  listeners.forEach((l) => l());
};

/** The key of the inbox thread whose pane is open, or null. */
export const activeInboxKey = () => activeKey;
export const useActiveInboxKey = () => useSyncExternalStore(subscribe, activeInboxKey, activeInboxKey);

/** A key press the inbox may take: no modifier, nothing else claimed it, and not typed into a
 *  field or a menu. */
export function isTriageKey(e: KeyboardEvent): boolean {
  if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return false;
  const t = e.target as HTMLElement | null;
  return !t?.closest?.("input, textarea, select, [contenteditable=''], [contenteditable='true'], [role='menu'], [role='dialog'], [role='listbox']");
}

/** “excerpt…” quoted and cut to one glance. */
export function quote(text: string, max = 80): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return `“${flat.length > max ? `${flat.slice(0, max - 1).trimEnd()}…` : flat}”`;
}

/** The comment a pane opened from the inbox should ring: the one its opener named, else the one
 *  the thread's lazy detail named after the pane opened (a GitHub mention learns which comment
 *  mentioned you only from that detail, which a click can beat). Reads the strip's fetch; never
 *  fetches itself. */
export function useInboxTargetComment(commentId: string | null | undefined, origin: InboxOrigin | null | undefined): string | null {
  const late = useNotificationDetail(origin?.conn ?? "", origin?.notificationId ?? "", false).data?.target;
  return commentId ?? (origin && late?.kind === "Comment" ? late.id : null);
}

export function TriageStrip({ origin, onClose }: { origin: InboxOrigin; onClose: () => void }) {
  // Fetched once the pane has stayed on this thread a moment (a held `j` fetches nothing).
  const detail = useNotificationDetail(origin.conn, origin.notificationId, origin.needsDetail);
  const actor = origin.actor ?? detail.data?.actor ?? null;
  const excerpt = origin.excerpt ?? detail.data?.excerpt ?? null;
  // The server's phrase recomputed with the detail ("Priya mentioned you") over the feed's.
  const phrase = detail.data?.phrase || origin.phrase;
  const why = detail.data?.why || origin.why;

  useEffect(() => {
    setActive(origin.key);
    return () => {
      if (activeKey === origin.key) setActive(null);
    };
  }, [origin.key]);

  const done = () => {
    if (origin.onDone() === "none") onClose();
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isTriageKey(e)) return;
      if (e.key === "j" && origin.onNext) origin.onNext();
      else if (e.key === "k" && origin.onPrev) origin.onPrev();
      else if (e.key === "e") done();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [origin]);

  const said = excerpt ? ` · ${quote(excerpt)}` : actor && !phrase.includes(actor.split(/\s+/)[0]) ? ` · ${actor}` : "";
  return (
    <div className="flex items-center gap-2 mb-2.5 -mt-0.5 min-w-0 text-[12px]" style={{ color: "var(--fg3)" }} data-testid="triage-strip">
      <Icon name="inbox" size={13} />
      <span className="shrink-0" style={{ color: "var(--fg2)" }}>
        Inbox
        {origin.total > 0 && origin.index > 0 && (
          <span className="num">
            {" "}
            · {origin.index} of {origin.total}
          </span>
        )}
      </span>
      <span className="truncate min-w-0" title={why}>
        · {phrase}
        {said}
      </span>
      <span className="ml-auto flex items-center gap-1.5 shrink-0">
        <span className="inline-flex items-center gap-1 max-[900px]:hidden" aria-hidden="true">
          <Kbd>k</Kbd>
          <Kbd>j</Kbd>
        </span>
        {origin.url && (
          <a href={origin.url} target="_blank" rel="noreferrer" className="ft-btn ft-btn-sm" title={`Open on ${origin.providerLabel}`}>
            <Icon name="external" size={13} />
            Open on {origin.providerLabel}
          </a>
        )}
        <Button sm icon="check" onClick={done} title="Mark done and open the next (e)">
          Done <Kbd>e</Kbd>
        </Button>
      </span>
    </div>
  );
}

// ---- the comment a notification pointed at ----

/** The ring on the comment a notification pointed at: the accent edge (it is the pane's selected
 *  item) and the selected-row wash around it. */
export const TARGET_BOX = { background: "var(--hover)", border: "1px solid var(--accent)", boxShadow: "0 0 0 3px var(--sel)" } as const;

/** The small tag over a targeted comment. */
export function InboxTag() {
  return (
    <div className="flex items-center gap-1.5 mb-1.5 text-[12px]" style={{ color: "var(--fg2)" }}>
      <Icon name="inbox" size={12} />
      From your inbox
    </div>
  );
}

/** Whether `thread` holds the comment (or is the thread) a notification named — what decides a
 *  thread is shown, not what is ringed (see {@link ringsThread} / {@link ringedComment}). */
export const threadHolds = (thread: CommentThread, id: string | null | undefined) =>
  !!id && (thread.id === id || thread.comments.some((c) => c.id === id));

/** The notification named the thread itself: the whole thread is ringed. */
export const ringsThread = (thread: CommentThread, id: string | null | undefined) => !!id && thread.id === id;

/** The notification named one comment in `thread`: that comment, and only it, is ringed — on
 *  GitHub every conversation comment shares one thread, so ringing the thread would ring them
 *  all. Null when it named the thread itself, or nothing here. */
export const ringedComment = (thread: CommentThread, id: string | null | undefined): string | null =>
  id && thread.id !== id && thread.comments.some((c) => c.id === id) ? id : null;
