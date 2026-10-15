import { Fragment, useEffect, useRef, useState } from "react";
import { useInbox, useNotificationDetail } from "../api";
import { notificationMeta, shortAge, toTime } from "../format";
import type { InboxThread } from "../types";
import { Icon } from "./icons";
import { needsDetail, opensInApp, quotable, useInboxActions } from "./inboxActions";
import { useNotificationRowMenu } from "./rowMenus/notification";
import { activeInboxKey, isTriageKey, quote, useActiveInboxKey } from "./TriageStrip";
import { ContextMenu, GroupHeading, IconButton, List, RowMenuButton, Skeleton, StateCard, StatusGlyph, useContextMenu } from "./ui";
import { ErrorState } from "./ErrorState";
import { useListView } from "./ControlBar";

/** Which day heading a thread sits under, by its newest event. */
export function dayGroup(iso: string | null | undefined, now = new Date()): "Today" | "Yesterday" | "Earlier" {
  const t = toTime(iso);
  const midnight = new Date(now);
  midnight.setHours(0, 0, 0, 0);
  if (t >= midnight.getTime()) return "Today";
  if (t >= midnight.getTime() - 24 * 3600_000) return "Yesterday";
  return "Earlier";
}

/** A row's identifier: a work item's own (`ENG-231`, `#842`), a pull request's number as `#1501`. */
const identOf = (t: InboxThread) => {
  const id = t.lead.notification.item_id;
  if (!id) return null;
  return /^\d+$/.test(id) ? `#${id}` : id;
};

/** The inbox is one list: what asks for you. The server sends nothing else. */
export function Notifications() {
  const { data, isLoading, error } = useInbox();
  const unread = (data ?? []).filter((t) => t.unread).length;
  const actions = useInboxActions();
  const activeKey = useActiveInboxKey();
  const [cursor, setCursor] = useState<string | null>(null);

  const { rows, bar } = useListView<InboxThread>({
    storageKey: "notifications",
    rows: data,
    connId: (t) => t.lead.connection_id,
    connLabel: (t) => t.lead.connection,
    sorts: [
      { label: "Newest", cmp: (a, b) => toTime(b.latest) - toTime(a.latest) },
      { label: "Oldest", cmp: (a, b) => toTime(a.latest) - toTime(b.latest) },
    ],
    views: (
      <span className="flex items-center gap-1.5 h-8 text-[12px] font-medium" style={{ color: "var(--fg2)" }}>
        For you
        <span className="num font-normal" style={{ color: "var(--fg3)" }}>
          {unread} unread
        </span>
      </span>
    ),
  });

  // The open pane's thread is the selected row; when it closes, the cursor stays on it.
  useEffect(() => {
    if (activeKey) setCursor(activeKey);
  }, [activeKey]);
  const selected = activeKey ?? cursor;

  // j/k/Enter/e on the list — unless a pane opened from the inbox is up: its strip owns the keys
  // then (and swaps the pane in place), so a key never does two things.
  const latest = useRef({ rows, selected, actions });
  latest.current = { rows, selected, actions };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (activeInboxKey() || !isTriageKey(e)) return;
      const { rows, selected, actions } = latest.current;
      if (rows.length === 0) return;
      const i = rows.findIndex((t) => t.key === selected);
      const at = (k: number) => rows[Math.max(0, Math.min(rows.length - 1, k))];
      if (e.key === "j") setCursor(at(i < 0 ? 0 : i + 1).key);
      else if (e.key === "k") setCursor(at(i < 0 ? 0 : i - 1).key);
      else if (e.key === "Enter" && i >= 0 && (e.target as HTMLElement | null)?.closest?.("button, a") == null) actions.open(rows[i], rows);
      else if (e.key === "e" && i >= 0) {
        const next = rows[i + 1] ?? rows[i - 1];
        setCursor(next ? next.key : null);
        void actions.done(rows[i]);
      } else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;

  const groups: { day: string; threads: InboxThread[] }[] = [];
  for (const t of rows) {
    const day = dayGroup(t.latest);
    const last = groups[groups.length - 1];
    if (last?.day === day) last.threads.push(t);
    else groups.push({ day, threads: [t] });
  }

  return (
    <>
      {bar}
      {rows.length === 0 ? (
        <StateCard
          icon={<Icon name="check-circle" size={24} />}
          title="Nothing needs you"
          sub="Review requests, mentions, assignments, failures and due dates land here."
        />
      ) : (
        <List>
          {groups.map((g) => (
            <Fragment key={g.day}>
              <GroupHeading className="mt-3" count={g.threads.length}>
                {g.day}
              </GroupHeading>
              {g.threads.map((t) => (
                <ThreadRow key={t.key} thread={t} order={rows} selected={t.key === selected} onSelect={() => setCursor(t.key)} />
              ))}
            </Fragment>
          ))}
        </List>
      )}
    </>
  );
}

/** One inbox row: glyph · title · identifier · why it's here (+N) · age, with Done on hover. Unread
 *  reads by weight, never by colour — the accent is for the selected row. */
function ThreadRow({ thread, order, selected, onSelect }: { thread: InboxThread; order: InboxThread[]; selected: boolean; onSelect: () => void }) {
  const lead = thread.lead;
  const n = lead.notification;
  const meta = notificationMeta(n.kind);
  const actions = useInboxActions();
  const menu = useContextMenu();
  const build = useNotificationRowMenu();
  // The who and what are fetched for the row in hand only — never for every row on a reload.
  const detail = useNotificationDetail(lead.connection_id, n.id, selected && needsDetail(n));
  const actor = n.actor ?? detail.data?.actor ?? null;
  const excerpt = quotable(n) ? (n.excerpt ?? detail.data?.excerpt ?? null) : null;
  const said = excerpt ? ` · ${quote(excerpt, 60)}` : actor && !lead.phrase.includes(actor.split(/\s+/)[0]) ? ` · ${actor}` : "";
  const ident = identOf(thread);
  const unread = thread.unread;

  const open = () => {
    onSelect();
    actions.open(thread, order);
  };
  const bodyClass = "flex flex-1 min-w-0 items-center gap-2.5 self-stretch text-left";

  // The row is a div carrying the shared `ft-row` look: a `Row` is itself a button, and the Done
  // button beside the open target can't nest inside it. It is also a size container: squeezed
  // beside an open pane, the identifier and the `+N` step aside so the title keeps the room.
  return (
    <>
      <div
        className="ft-row group @container"
        data-selected={selected || undefined}
        data-menu-open={menu.open ? "true" : undefined}
        onContextMenu={menu.onContextMenu}
        onKeyDown={menu.onKeyDown}
        data-thread={thread.key}
      >
        <button
          type="button"
          onClick={open}
          className={bodyClass}
          style={{ cursor: "pointer" }}
          title={opensInApp(n) ? undefined : n.url ? "Opens on the provider" : undefined}
        >
          <StatusGlyph glyph={meta.glyph} color={unread ? meta.color : "var(--fg3)"} title={meta.label} />
          <span className="truncate min-w-0 shrink" style={{ color: unread ? "var(--fg)" : "var(--fg2)", fontWeight: unread ? 600 : 400 }}>
            {n.title}
          </span>
          {ident && (
            <span className="mono shrink-0 text-[12px] @max-[480px]:hidden" style={{ color: "var(--fg3)" }}>
              {ident}
            </span>
          )}
          <span className="truncate min-w-0 flex-1 text-[12px]" style={{ color: "var(--fg3)" }} title={lead.why}>
            {lead.phrase}
            {said}
          </span>
          {thread.more.length > 0 && (
            <span className="num shrink-0 text-[12px] @max-[480px]:hidden" style={{ color: "var(--fg3)" }} title={`${thread.more.length} more on this item`}>
              +{thread.more.length}
            </span>
          )}
        </button>
        <IconButton
          icon="check"
          label="Dismiss"
          title="Dismiss (e)"
          onClick={() => void actions.done(thread)}
          className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity"
        />
        <span className="num shrink-0 text-right text-[12px] whitespace-nowrap" style={{ width: 34, color: "var(--fg3)" }}>
          {shortAge(thread.latest)}
        </span>
        <RowMenuButton onOpen={menu.openAt} />
      </div>
      <ContextMenu anchor={menu.anchor} onClose={menu.close} label="Notification actions" items={() => build(thread, order)} />
    </>
  );
}
