import { useNotifications } from "../api";
import { notificationMeta, relativeTime } from "../format";
import type { NotifRow } from "../types";
import { List, ProviderBadge, Row, Skeleton, StateCard } from "./ui";
import { ErrorState } from "./ErrorState";

export function Notifications() {
  const { data, isLoading, error } = useNotifications();
  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || data.length === 0)
    return <StateCard icon="✓" title="Inbox zero" sub="Review requests, mentions, and CI failures land here." />;

  return (
    <List>
      {data.map((row, i) => (
        <NotifCard key={`${row.connection_id}:${row.notification.id}`} row={row} index={i} />
      ))}
    </List>
  );
}

function NotifCard({ row, index }: { row: NotifRow; index: number }) {
  const n = row.notification;
  const meta = notificationMeta(n.kind);
  return (
    <Row index={index} href={n.url}>
      <div className="flex items-start gap-3">
        <span
          className="inline-flex items-center justify-center rounded-md shrink-0 mt-0.5"
          style={{ width: 26, height: 26, background: "color-mix(in srgb, " + meta.color + " 16%, transparent)", color: meta.color }}
        >
          {meta.icon}
        </span>
        <div className="flex-1 min-w-0">
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
        </div>
        <span className="text-xs whitespace-nowrap shrink-0" style={{ color: "var(--dim)" }}>
          {relativeTime(n.updated_at)}
        </span>
      </div>
    </Row>
  );
}
