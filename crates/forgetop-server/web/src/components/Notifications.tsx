import { useState } from "react";
import { motion } from "framer-motion";
import { useQueryClient } from "@tanstack/react-query";
import { apiPost, useNotifications } from "../api";
import { notificationMeta, relativeTime, toTime } from "../format";
import type { NotifRow } from "../types";
import { List, ProviderBadge, Skeleton, StateCard } from "./ui";
import { ErrorState } from "./ErrorState";
import { useListView } from "./ControlBar";
import { usePrOpener } from "./PrDetail";
import { useWiOpener } from "./WiDetail";
import { usePipelineOpener } from "./PipelineDetail";

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
    return <StateCard icon="✓" title="Inbox zero" sub="Review requests, mentions, and CI failures land here." />;

  return (
    <>
      {bar}
      <List>
        {rows.map((row, i) => (
          <NotifCard key={`${row.connection_id}:${row.notification.id}`} row={row} index={i} />
        ))}
      </List>
    </>
  );
}

function NotifCard({ row, index }: { row: NotifRow; index: number }) {
  const n = row.notification;
  const meta = notificationMeta(n.kind);
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const openPr = usePrOpener();
  const openWi = useWiOpener();
  const openPipe = usePipelineOpener();

  // Open the referenced item in-app when the notification carries a drillable target.
  // Some notifications have no in-app target (item_id null, or item_type "Other" — e.g. a
  // GitHub CheckSuite) — those keep the external link.
  const conn = row.connection_id;
  const openInApp =
    n.item_id == null
      ? null
      : n.item_type === "PullRequest"
        ? () => openPr({ conn, id: n.item_id! })
        : n.item_type === "WorkItem"
          ? () => openWi({ conn, id: n.item_id! })
          : n.item_type === "Pipeline"
            ? () => openPipe({ conn, runId: n.item_id! })
            : null;

  const markRead = async () => {
    setBusy(true);
    try {
      await apiPost("/api/notification/read", { conn: row.connection_id, id: n.id });
      qc.invalidateQueries({ queryKey: ["notifications"] });
      qc.invalidateQueries({ queryKey: ["launchpad"] });
    } finally {
      setBusy(false);
    }
  };

  const body = (
    <>
      <div className="flex items-center gap-2">
        {n.unread && <span className="inline-block w-2 h-2 rounded-full shrink-0" style={{ background: "var(--accent)" }} title="unread" />}
        <span className="truncate font-medium" style={{ color: n.unread ? "var(--fg)" : "var(--dim)" }}>
          {n.title}
        </span>
      </div>
      <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="text-xs font-medium" style={{ color: meta.color }}>
          {meta.label}
        </span>
        <span className="text-xs" style={{ color: "var(--dim)" }}>
          {n.context}
        </span>
        <ProviderBadge provider={row.provider} connection={row.connection} />
      </div>
    </>
  );

  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.22, delay: Math.min(index * 0.02, 0.3), ease: "easeOut" }}
      className="group flex items-start gap-3 rounded-lg px-4 py-3"
      style={{ background: "var(--card)", border: "1px solid var(--border)" }}
    >
      <span
        className="inline-flex items-center justify-center rounded-md shrink-0 mt-0.5"
        style={{ width: 26, height: 26, background: "color-mix(in srgb, " + meta.color + " 16%, transparent)", color: meta.color }}
      >
        {meta.icon}
      </span>
      {openInApp ? (
        <button onClick={openInApp} className="flex-1 min-w-0 text-left" style={{ cursor: "pointer" }}>
          {body}
        </button>
      ) : (
        <a href={n.url ?? undefined} target="_blank" rel="noreferrer" className="flex-1 min-w-0" style={{ cursor: n.url ? "pointer" : "default" }}>
          {body}
        </a>
      )}
      <div className="flex items-center gap-3 shrink-0">
        {n.unread && (
          <button
            onClick={markRead}
            disabled={busy}
            title="Mark as read"
            className="text-xs rounded px-2 py-1 opacity-0 group-hover:opacity-100 transition-opacity"
            style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}
          >
            ✓ read
          </button>
        )}
        <span className="text-xs whitespace-nowrap" style={{ color: "var(--dim)" }}>
          {relativeTime(n.updated_at)}
        </span>
      </div>
    </motion.div>
  );
}
