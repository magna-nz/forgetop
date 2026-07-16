import { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { apiGet, useWorkItems, useWriteAction } from "../api";
import { relativeTime, toTime, wiStateColor } from "../format";
import type { WiRow } from "../types";
import { Avatar, Chip, List, ProviderBadge, Skeleton, StateCard } from "./ui";
import { ErrorState } from "./ErrorState";
import { useListView } from "./ControlBar";

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
  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.22, delay: Math.min(index * 0.02, 0.3), ease: "easeOut" }}
      className="flex items-start gap-3 rounded-lg px-4 py-3"
      style={{ background: "var(--card)", border: "1px solid var(--border)" }}
    >
      <a href={wi.url ?? undefined} target="_blank" rel="noreferrer" className="flex-1 min-w-0" style={{ cursor: wi.url ? "pointer" : "default" }}>
        <div className="flex items-center gap-2">
          <span className="inline-flex items-center gap-1.5 text-xs font-medium whitespace-nowrap" style={{ color }}>
            <span>●</span>
            {wi.state}
          </span>
          <span className="truncate font-medium" style={{ color: "var(--fg)" }}>
            {wi.title}
          </span>
        </div>
        <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <ProviderBadge provider={row.provider} connection={row.connection} />
          {wi.identifier && <span className="mono text-xs" style={{ color: "var(--dim)" }}>{wi.identifier}</span>}
          {wi.work_item_type && <Chip>{wi.work_item_type}</Chip>}
        </div>
      </a>
      <div className="flex flex-col items-end gap-2 shrink-0">
        <StateMenu row={row} />
        <div className="flex items-center gap-2">
          <span className="text-xs whitespace-nowrap" style={{ color: "var(--dim)" }}>
            {relativeTime(wi.updated_at)}
          </span>
          {wi.assignee && <Avatar name={wi.assignee.display_name} />}
        </div>
      </div>
    </motion.div>
  );
}

function StateMenu({ row }: { row: WiRow }) {
  const [open, setOpen] = useState(false);
  const [states, setStates] = useState<string[] | null>(null);
  const [loading, setLoading] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const { busy, error, run } = useWriteAction();

  // Fetch the allowed transitions lazily, the first time the menu opens.
  useEffect(() => {
    if (open && states === null && !loading) {
      setLoading(true);
      apiGet<string[]>(`/api/wi/states?conn=${encodeURIComponent(row.connection_id)}&id=${encodeURIComponent(row.work_item.id)}`)
        .then(setStates)
        .catch(() => setStates([]))
        .finally(() => setLoading(false));
    }
  }, [open, states, loading, row.connection_id, row.work_item.id]);

  // Close on outside click.
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const move = async (state: string) => {
    const ok = await run("/api/wi/state", { conn: row.connection_id, id: row.work_item.id, state }, ["work-items", "launchpad"]);
    if (ok) setOpen(false);
  };

  const options = (states ?? []).filter((s) => s.toLowerCase() !== row.work_item.state.toLowerCase());

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen((o) => !o)}
        className="text-xs rounded px-2 py-1"
        style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}
      >
        Move ▾
      </button>
      {open && (
        <div
          className="absolute right-0 mt-1 z-10 rounded-md py-1 min-w-40 shadow-lg"
          style={{ background: "var(--panel)", border: "1px solid var(--border)" }}
        >
          {loading && <div className="px-3 py-1.5 text-xs" style={{ color: "var(--dim)" }}>Loading…</div>}
          {!loading && options.length === 0 && (
            <div className="px-3 py-1.5 text-xs" style={{ color: "var(--dim)" }}>No transitions available</div>
          )}
          {options.map((s) => (
            <button
              key={s}
              disabled={busy}
              onClick={() => move(s)}
              className="block w-full text-left px-3 py-1.5 text-xs"
              style={{ color: "var(--fg)" }}
              onMouseEnter={(e) => (e.currentTarget.style.background = "var(--sel)")}
              onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
            >
              → {s}
            </button>
          ))}
          {error && <div className="px-3 py-1.5 text-xs" style={{ color: "var(--red)" }}>{error}</div>}
        </div>
      )}
    </div>
  );
}
