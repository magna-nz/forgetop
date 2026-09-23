import { useState } from "react";
import { motion } from "framer-motion";
import { useQueryClient } from "@tanstack/react-query";
import { apiPost, useConnections, useHealth, useWriteAction } from "../api";
import type { ConnectionRow } from "../types";
import { dropConnectionFromCache } from "../optimistic";
import { Skeleton, StateCard, StatusBadge } from "./ui";
import { ErrorState } from "./ErrorState";
import { ConnectionForm } from "./ConnectionForm";

const SECTION_LABEL: Record<string, string> = {
  pull_requests: "Pull Requests",
  work_items: "Work Items",
  pipelines: "Pipelines",
};

export function Settings() {
  const { data, isLoading, error } = useConnections();
  const health = useHealth();
  const qc = useQueryClient();
  const [editing, setEditing] = useState<ConnectionRow | null | undefined>(undefined); // undefined = closed

  const refreshAll = () => {
    ["connections", "health", "launchpad", "prs", "work-items", "pipelines", "notifications"].forEach((k) =>
      qc.invalidateQueries({ queryKey: [k] }),
    );
  };

  /**
   * Reflects a removal locally before the refetch, so the card and its rows disappear on the
   * click rather than when the network answers. See `dropConnectionFromCache`.
   */
  const dropConnection = (id: string) => {
    dropConnectionFromCache(qc, id);
    refreshAll();
  };

  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;

  const healthById = new Map((health.data ?? []).map((h) => [h.connection_id, h.healthy]));

  return (
    <div className="p-5 max-w-4xl mx-auto">
      <div className="flex items-center justify-between mb-4">
        <div>
          <h2 className="text-sm font-semibold" style={{ color: "var(--fg)" }}>Connections</h2>
          <p className="text-xs" style={{ color: "var(--dim)" }}>
            Shared with the terminal app · tokens are stored in your OS keychain.
          </p>
        </div>
        <button
          onClick={() => setEditing(null)}
          className="rounded-md px-3 py-1.5 text-sm font-medium"
          style={{ background: "var(--accent)", color: "#0c1a2b" }}
        >
          + Add connection
        </button>
      </div>

      {!data || data.length === 0 ? (
        <StateCard icon="⊕" title="No connections yet" sub="Add one to start pulling in pull requests, work items, and pipelines." />
      ) : (
        <div className="flex flex-col gap-2">
          {data.map((c) => (
            <ConnectionCard
              key={c.id}
              conn={c}
              healthy={healthById.get(c.id)}
              onEdit={() => setEditing(c)}
              onRemoved={dropConnection}
            />
          ))}
        </div>
      )}

      {editing !== undefined && (
        <Modal title={editing ? "Edit connection" : "Add connection"} onClose={() => setEditing(undefined)}>
          <ConnectionForm
            initial={editing}
            onCancel={() => setEditing(undefined)}
            onSaved={() => {
              setEditing(undefined);
              refreshAll();
            }}
          />
        </Modal>
      )}
    </div>
  );
}

function ConnectionCard({
  conn,
  healthy,
  onEdit,
  onRemoved,
}: {
  conn: ConnectionRow;
  healthy?: boolean;
  onEdit: () => void;
  /** Reports the removed id so the caller can drop its rows before refetching. */
  onRemoved: (id: string) => void;
}) {
  const { busy, run } = useWriteAction();
  const [tested, setTested] = useState<boolean | null>(null);

  const del = async () => {
    if (!window.confirm(`Remove "${conn.display_name}"? This also deletes its token from the keychain.`)) return;
    // `run` reports a failure by returning false, not by throwing. Dropping the rows regardless
    // would show the connection deleted and then, a refetch later, silently undeleted.
    if (await run("/api/connections/delete", { id: conn.id }, [])) {
      onRemoved(conn.id);
    }
  };
  const test = async () => {
    setTested(null);
    try {
      const r = await apiPost<{ healthy: boolean }>("/api/connections/test", { id: conn.id });
      setTested(r.healthy);
    } catch {
      setTested(false);
    }
  };

  const status = tested ?? healthy; // boolean | undefined
  return (
    <div className="flex items-center gap-3 rounded-lg px-3 py-2" style={{ background: "var(--card)", border: "1px solid var(--border)" }}>
      <div className="flex-1 min-w-0 flex items-center gap-2">
        {/* Status badge follows the app convention (tinted StatusBadge) and sits before the name. */}
        {status != null && <StatusBadge label={status ? "Connected" : "Auth failed"} color={status ? "var(--green)" : "var(--red)"} />}
        <span className="font-medium truncate" style={{ color: "var(--fg)" }}>{conn.display_name}</span>
        <span className="text-xs shrink-0" style={{ color: "var(--dim)" }}>
          {conn.sections.length > 0 ? conn.sections.map((s) => SECTION_LABEL[s] ?? s).join(" · ") : "not shown in any section"}
        </span>
      </div>
      <div className="flex items-center gap-1.5 shrink-0">
        <SmallBtn label="Test" onClick={test} disabled={busy} />
        <SmallBtn label="Edit" onClick={onEdit} disabled={busy} />
        <SmallBtn label="Delete" onClick={del} disabled={busy} danger />
      </div>
    </div>
  );
}

function SmallBtn({ label, onClick, disabled, danger }: { label: string; onClick: () => void; disabled?: boolean; danger?: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="text-xs rounded px-2 py-1"
      style={{
        color: danger ? "var(--red)" : "var(--dim)",
        border: "1px solid var(--border)",
        background: "var(--panel2)",
        opacity: disabled ? 0.5 : 1,
      }}
    >
      {label}
    </button>
  );
}

export function Modal({ title, children, onClose }: { title: string; children: React.ReactNode; onClose: () => void }) {
  return (
    <motion.div
      className="fixed inset-0 z-50 flex items-start justify-center"
      style={{ background: "rgba(0,0,0,0.5)", paddingTop: "8vh" }}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      onClick={onClose}
    >
      <motion.div
        initial={{ opacity: 0, y: -8, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={{ duration: 0.15 }}
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-lg mx-4 rounded-xl overflow-hidden max-h-[84vh] overflow-y-auto"
        style={{ background: "var(--panel)", border: "1px solid var(--border)" }}
      >
        <div className="flex items-center justify-between px-5 py-3.5" style={{ borderBottom: "1px solid var(--border)" }}>
          <span className="font-semibold" style={{ color: "var(--fg)" }}>{title}</span>
          <button onClick={onClose} className="text-lg leading-none" style={{ color: "var(--dim)" }}>✕</button>
        </div>
        <div className="p-5">{children}</div>
      </motion.div>
    </motion.div>
  );
}
