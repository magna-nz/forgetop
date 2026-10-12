import { useMemo, useState } from "react";
import { motion } from "framer-motion";
import { useWorkItems } from "../api";
import { relativeTime, toTime, wiStateColor } from "../format";
import type { WiRow, WorkItemParent } from "../types";
import { Avatar, Chip, List, Skeleton, StateCard, StatusBadge } from "./ui";
import { ErrorState } from "./ErrorState";
import { useListView } from "./ControlBar";
import { useWiOpener } from "./WiDetail";

const GROUP_KEY = "forgetop:work-items:group";
type GroupMode = "off" | "parent";

/** Grouped by parent unless the viewer chose otherwise. */
function readGroupMode(): GroupMode {
  try {
    return localStorage.getItem(GROUP_KEY) === "off" ? "off" : "parent";
  } catch {
    return "parent";
  }
}

interface WiGroup {
  key: string;
  parent: WorkItemParent | null; // null = the trailing "No parent" group
  rows: WiRow[];
}

/** Buckets already-sorted rows by parent identity. Groups keep the order of their first row;
 *  parentless rows form one final group. Rows keep their order within a group.
 *
 *  Empty when no row has a parent: there is nothing to group by, and a lone "No parent" section
 *  over the whole list would only hide it. That is what lets grouping be the default. */
function groupByParent(rows: WiRow[]): WiGroup[] {
  if (!rows.some((r) => r.work_item.parent)) return [];
  const groups = new Map<string, WiGroup>();
  const none: WiGroup = { key: "none", parent: null, rows: [] };
  for (const row of rows) {
    const p = row.work_item.parent;
    if (!p) {
      none.rows.push(row);
      continue;
    }
    const key = `${row.connection_id}|${p.kind}|${p.id}`;
    const g = groups.get(key);
    if (g) g.rows.push(row);
    else groups.set(key, { key, parent: p, rows: [row] });
  }
  return none.rows.length ? [...groups.values(), none] : [...groups.values()];
}

export function WorkItems() {
  const { data, isLoading, error } = useWorkItems();
  const [mode, setMode] = useState<GroupMode>(readGroupMode);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
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
      // In flight: being worked on, or past that and in review.
      { label: "In progress", match: (r) => r.work_item.state_category === "Started" || r.work_item.state_category === "Review" },
      { label: "Blocked", match: (r) => r.work_item.state.toLowerCase() === "blocked" },
      { label: "Not started", match: (r) => ["Unstarted", "Backlog", "Triage"].includes(r.work_item.state_category) },
    ],
    statusLabel: "Show",
    facet: { label: "Type", value: (r) => r.work_item.work_item_type },
  });

  const groups = useMemo(() => (mode === "parent" ? groupByParent(rows) : []), [mode, rows]);

  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || data.length === 0)
    return <StateCard icon="◇" title="No work items assigned" sub="Issues and tickets assigned to you appear here." />;

  const chooseMode = (m: GroupMode) => {
    setMode(m);
    try {
      localStorage.setItem(GROUP_KEY, m);
    } catch {
      /* ignore */
    }
  };
  const toggle = (key: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  return (
    <>
      {bar}
      <div className="flex flex-wrap items-center gap-2 px-5 pt-2 max-w-5xl mx-auto">
        <label className="flex items-center gap-1.5 text-xs" style={{ color: "var(--dim)" }}>
          Group
          <select
            value={mode}
            onChange={(e) => chooseMode(e.target.value === "parent" ? "parent" : "off")}
            className="rounded-md px-2 py-1 text-xs outline-none"
            style={{ background: "var(--panel2)", color: "var(--fg)", border: "1px solid var(--border)" }}
          >
            <option value="parent">Parent</option>
            <option value="off">Off</option>
          </select>
        </label>
      </div>
      <List>
        {groups.length > 0
          ? groups.map((g) => (
              <WiGroupSection key={g.key} group={g} open={expanded.has(g.key)} onToggle={() => toggle(g.key)} />
            ))
          : rows.map((row, i) => (
              <WiCard key={`${row.connection_id}:${row.work_item.repository ?? ""}:${row.work_item.id}`} row={row} index={i} />
            ))}
      </List>
    </>
  );
}

function WiGroupSection({ group, open, onToggle }: { group: WiGroup; open: boolean; onToggle: () => void }) {
  const { parent, rows } = group;
  const blocked = rows.filter((r) => r.work_item.state.toLowerCase() === "blocked").length;
  const unblockedIn = (category: string) =>
    rows.filter((r) => r.work_item.state_category === category && r.work_item.state.toLowerCase() !== "blocked").length;
  const inProgress = unblockedIn("Started");
  const inReview = unblockedIn("Review");
  return (
    <div className="flex flex-col gap-2">
      <div
        className="flex items-center gap-3 rounded-lg px-3 py-1.5"
        style={{ background: "var(--panel2)", border: "1px solid var(--border)" }}
      >
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={open}
          className="flex-1 min-w-0 text-left flex items-center gap-2"
          style={{ cursor: "pointer" }}
        >
          <span className="text-xs shrink-0" style={{ color: "var(--dim)" }} aria-hidden="true">
            {open ? "▾" : "▸"}
          </span>
          {parent && (
            <span className="shrink-0">
              <Chip>{parent.kind}</Chip>
            </span>
          )}
          <span className="truncate font-medium min-w-0" style={{ color: "var(--fg)" }}>
            {parent ? parent.title : "No parent"}
          </span>
          <span className="text-xs shrink-0" style={{ color: "var(--dim)" }}>
            {rows.length} {rows.length === 1 ? "item" : "items"}
          </span>
          {inProgress > 0 && (
            <span className="text-xs shrink-0" style={{ color: "var(--dim)" }}>
              {inProgress} in progress
            </span>
          )}
          {inReview > 0 && (
            <span className="text-xs shrink-0" style={{ color: "var(--cyan)" }}>
              {inReview} in review
            </span>
          )}
          {blocked > 0 && (
            <span className="text-xs shrink-0" style={{ color: "var(--red)" }}>
              {blocked} blocked
            </span>
          )}
        </button>
        {parent?.url && (
          <a
            href={parent.url}
            target="_blank"
            rel="noreferrer"
            className="text-xs shrink-0"
            style={{ color: "var(--dim)" }}
            aria-label={`Open ${parent.kind} ${parent.title} in a new tab`}
            title="Open in a new tab"
          >
            ↗
          </a>
        )}
      </div>
      {open &&
        rows.map((row, i) => (
          <WiCard key={`${row.connection_id}:${row.work_item.repository ?? ""}:${row.work_item.id}`} row={row} index={i} />
        ))}
    </div>
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
        onClick={() => open({ conn: row.connection_id, repo: wi.repository, id: wi.id })}
        className="flex-1 min-w-0 text-left flex items-center gap-2"
        style={{ cursor: "pointer" }}
      >
        <StatusBadge label={wi.state} color={color} />
        {wi.identifier && <span className="mono text-xs shrink-0" style={{ color: "var(--dim)" }}>{wi.identifier}</span>}
        <span className="truncate font-medium min-w-0" style={{ color: "var(--fg)" }}>
          {wi.title}
        </span>
        {wi.work_item_type && (
          <span className="shrink-0 inline-flex items-center gap-1.5" style={{ color: "var(--dim)" }}>
            <Chip>{wi.work_item_type}</Chip>
            {wi.work_item_type.toLowerCase() === "bug" && <BugIcon />}
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
