import { useState } from "react";
import { motion } from "framer-motion";
import { useQueryClient } from "@tanstack/react-query";
import { apiPost, useConnections, useHealth, useWriteAction } from "../api";
import { providerMeta } from "../format";
import type { ConnectionRow } from "../types";
import { ProviderBadge, Skeleton, StateCard } from "./ui";
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
              onChanged={refreshAll}
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
  onChanged,
}: {
  conn: ConnectionRow;
  healthy?: boolean;
  onEdit: () => void;
  onChanged: () => void;
}) {
  const meta = providerMeta(conn.provider);
  const { busy, run } = useWriteAction();
  const [tested, setTested] = useState<boolean | null>(null);

  const del = async () => {
    if (!window.confirm(`Remove "${conn.display_name}"? This also deletes its token from the keychain.`)) return;
    await run("/api/connections/delete", { id: conn.id }, []);
    onChanged();
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

  const dot = tested ?? healthy;
  return (
    <div className="flex items-center gap-3 rounded-lg px-4 py-3" style={{ background: "var(--card)", border: "1px solid var(--border)" }}>
      <span className="inline-block w-2.5 h-2.5 rounded-full shrink-0" style={{ background: meta.color }} title={meta.label} />
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-medium truncate" style={{ color: "var(--fg)" }}>{conn.display_name}</span>
          {dot != null && (
            <span
              className="text-xs px-1.5 rounded-full"
              style={{ color: dot ? "var(--green)" : "var(--red)", border: `1px solid ${dot ? "var(--green)" : "var(--red)"}` }}
            >
              {dot ? "connected" : "auth failed"}
            </span>
          )}
          {!conn.has_token && conn.provider !== "Demo" && (
            <span className="text-xs px-1.5 rounded-full" style={{ color: "var(--yellow)", border: "1px solid var(--yellow)" }}>
              no token
            </span>
          )}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1">
          <ProviderBadge provider={conn.provider} connection={meta.label} />
          {conn.sections.length > 0 ? (
            <span className="text-xs" style={{ color: "var(--dim)" }}>
              {conn.sections.map((s) => SECTION_LABEL[s] ?? s).join(" · ")}
            </span>
          ) : (
            <span className="text-xs" style={{ color: "var(--dim)" }}>not shown in any section</span>
          )}
        </div>
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
