import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { apiPost, useNotifications } from "../api";
import { notificationMeta, relativeTime, toTime } from "../format";
import type { NotifRow } from "../types";
import { markNotificationReadInCache } from "../optimistic";
import { Icon } from "./icons";
import { IconButton, List, Skeleton, StateCard, StatusGlyph } from "./ui";
import { ErrorState } from "./ErrorState";
import { useListView } from "./ControlBar";
import { usePrOpener } from "./PrDetail";
import { useWiOpener } from "./WiDetail";

export function Notifications() {
  const { data, isLoading, error } = useNotifications();
  const { rows, bar } = useListView<NotifRow>({
    storageKey: "notifications",
    rows: data,
    connId: (r) => r.connection_id,
    connLabel: (r) => r.connection,
    sorts: [
      { label: "Newest", cmp: (a, b) => toTime(b.notification.updated_at) - toTime(a.notification.updated_at) },
      { label: "Oldest", cmp: (a, b) => toTime(a.notification.updated_at) - toTime(b.notification.updated_at) },
    ],
    statuses: [
      { label: "Unread", match: (r) => r.notification.unread },
      { label: "Review requests", match: (r) => r.notification.kind === "ReviewRequested" },
      { label: "Mentions", match: (r) => r.notification.kind === "Mention" },
      { label: "CI failures", match: (r) => r.notification.kind === "CiFailed" },
    ],
    statusLabel: "Show",
    defaultStatus: 0, // Unread — the common inbox view
  });

  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || data.length === 0)
    return <StateCard icon={<Icon name="check-circle" size={24} />} title="Inbox zero" sub="Review requests, mentions, and CI failures land here." />;

  return (
    <>
      {bar}
      <List>
        {rows.map((row) => (
          <NotifRowItem key={`${row.connection_id}:${row.notification.id}`} row={row} />
        ))}
      </List>
    </>
  );
}

function NotifRowItem({ row }: { row: NotifRow }) {
  const n = row.notification;
  const meta = notificationMeta(n.kind);
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const openPr = usePrOpener();
  const openWi = useWiOpener();
  const conn = row.connection_id;

  const markReadNow = async () => {
    // Dim the row (and drop the bell count) on the click rather than when the network answers.
    markNotificationReadInCache(qc, conn, n.id);
    try {
      await apiPost("/api/notification/read", { conn, id: n.id });
    } finally {
      // Invalidated either way: on success the refetch confirms the local flag, on failure it is
      // what replaces it with the server's truth.
      qc.invalidateQueries({ queryKey: ["notifications"] });
      qc.invalidateQueries({ queryKey: ["launchpad"] });
    }
  };

  // Explicit mark-as-read button — shows a busy state.
  const markRead = async () => {
    setBusy(true);
    try {
      await markReadNow();
    } finally {
      setBusy(false);
    }
  };

  // Opening a notification marks it read, matching the TUI inbox. Like the TUI, only PRs and
  // work items drill in-app; everything else (pipelines, untyped activity, or a null item_id)
  // falls back to the provider link. Read is marked whichever path is taken.
  const onOpen = () => {
    if (n.unread) void markReadNow();
  };
  const openInApp =
    n.item_id == null
      ? null
      : n.item_type === "PullRequest"
        ? () => {
            onOpen();
            openPr({ conn, repo: n.repository, id: n.item_id! });
          }
        : n.item_type === "WorkItem"
          ? () => {
              onOpen();
              openWi({ conn, repo: n.repository, id: n.item_id! });
            }
          : null;

  // Unread rows read at full weight, read ones recede. Age is a fixed-width tabular column.
  const body = (
    <>
      <StatusGlyph glyph={meta.glyph} color={n.unread ? meta.color : "var(--fg3)"} />
      <span className="truncate" style={{ color: n.unread ? "var(--fg)" : "var(--fg2)", fontWeight: n.unread ? 500 : 400 }}>
        {n.title}
      </span>
      <span className="shrink-0 text-[12px]" style={{ color: "var(--fg3)" }}>
        {meta.label}
      </span>
      <span className="truncate text-[12px]" style={{ color: "var(--fg3)" }}>
        {n.context}
      </span>
    </>
  );
  const bodyClass = "flex flex-1 min-w-0 items-center gap-2.5 self-stretch text-left";

  // The row is a div carrying the shared `ft-row` look: a `Row` is itself a button or link, and
  // the mark-as-read button beside the open target can't nest inside it.
  return (
    <div className="ft-row group">
      {openInApp ? (
        <button type="button" onClick={openInApp} className={bodyClass} style={{ cursor: "pointer" }}>
          {body}
        </button>
      ) : (
        <a href={n.url ?? undefined} onClick={onOpen} target="_blank" rel="noreferrer" className={bodyClass} style={{ cursor: n.url ? "pointer" : "default" }}>
          {body}
        </a>
      )}
      {n.unread && (
        <IconButton
          icon="check"
          label="Mark as read"
          title="Mark as read"
          onClick={markRead}
          disabled={busy}
          className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity"
        />
      )}
      <span className="num shrink-0 w-14 text-right text-[12px] whitespace-nowrap" style={{ color: "var(--fg3)" }}>
        {relativeTime(n.updated_at)}
      </span>
    </div>
  );
}
