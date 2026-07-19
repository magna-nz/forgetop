import { motion } from "framer-motion";
import { useWorkItems } from "../api";
import { relativeTime, toTime, wiStateColor } from "../format";
import type { WiRow } from "../types";
import { Avatar, Chip, List, Skeleton, StateCard, StatusBadge } from "./ui";
import { ErrorState } from "./ErrorState";
import { useListView } from "./ControlBar";
import { useWiOpener } from "./WiDetail";

export function WorkItems() {
  const { data, isLoading, error } = useWorkItems();
  const { rows, bar } = useListView<WiRow>({
    storageKey: "work-items",
    rows: data,
    connId: (r) => r.connection_id,
    connLabel: (r) => r.connection,
    sorts: [
      { label: "Recently updated", cmp: (a, b) => toTime(b.work_item.updated_at) - toTime(a.work_item.updated_at) },
      { label: "Oldest", cmp: (a, b) => toTime(a.work_item.updated_at) - toTime(b.work_item.updated_at) },
      { label: "Title A–Z", cmp: (a, b) => a.work_item.title.localeCompare(b.work_item.title) },
    ],
    statuses: [
      { label: "In progress", match: (r) => r.work_item.state_category === "Started" },
      { label: "Blocked", match: (r) => r.work_item.state.toLowerCase() === "blocked" },
      { label: "Not started", match: (r) => ["Unstarted", "Backlog", "Triage"].includes(r.work_item.state_category) },
    ],
    statusLabel: "Show",
  });

  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || data.length === 0)
    return <StateCard icon="◇" title="No work items assigned" sub="Issues and tickets assigned to you appear here." />;

  return (
    <>
      {bar}
      <List>
        {rows.map((row, i) => (
          <WiCard key={`${row.connection_id}:${row.work_item.id}`} row={row} index={i} />
        ))}
      </List>
    </>
  );
}

function WiCard({ row, index }: { row: WiRow; index: number }) {
  const wi = row.work_item;
  const color = wiStateColor(wi.state, wi.state_category);
  const open = useWiOpener();
  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.22, delay: Math.min(index * 0.02, 0.3), ease: "easeOut" }}
      className="flex items-center gap-3 rounded-lg px-3 py-1.5"
      style={{ background: "var(--card)", border: "1px solid var(--border)" }}
    >
      <button
        onClick={() => open({ conn: row.connection_id, id: wi.id })}
        className="flex-1 min-w-0 text-left flex items-center gap-2"
        style={{ cursor: "pointer" }}
      >
        <StatusBadge label={wi.state} color={color} />
        {wi.identifier && <span className="mono text-xs shrink-0" style={{ color: "var(--dim)" }}>{wi.identifier}</span>}
        <span className="truncate font-medium min-w-0" style={{ color: "var(--fg)" }}>
          {wi.title}
        </span>
        {wi.work_item_type && (
          <span className="shrink-0">
            <Chip>
              {wi.work_item_type.toLowerCase() === "bug" && <BugIcon />}
              {wi.work_item_type}
            </Chip>
          </span>
        )}
      </button>
      <div className="flex items-center gap-2 shrink-0">
        <span className="text-xs whitespace-nowrap" style={{ color: "var(--dim)" }}>
          {relativeTime(wi.updated_at)}
        </span>
        {wi.assignee && <Avatar name={wi.assignee.display_name} />}
      </div>
    </motion.div>
  );
}

/** Small bug glyph shown inside the [Bug] type chip. Lucide "bug" icon (ISC), inlined so it
 *  inherits the chip's currentColor and stays offline-friendly. */
function BugIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="12"
      height="12"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="m8 2 1.88 1.88" />
      <path d="M14.12 3.88 16 2" />
      <path d="M9 7.13v-1a3.003 3.003 0 1 1 6 0v1" />
      <path d="M12 20c-3.3 0-6-2.7-6-6v-3a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v3c0 3.3-2.7 6-6 6" />
      <path d="M12 20v-9" />
      <path d="M6.53 9C4.6 8.8 3 7.1 3 5" />
      <path d="M6 13H2" />
      <path d="M3 21c0-2.1 1.7-3.9 3.8-4" />
      <path d="M20.97 5c0 2.1-1.6 3.8-3.5 4" />
      <path d="M22 13h-4" />
      <path d="M17.2 17c2.1.1 3.8 1.9 3.8 4" />
    </svg>
  );
}
