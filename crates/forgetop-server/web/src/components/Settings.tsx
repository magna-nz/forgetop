import { useState } from "react";
import { motion } from "framer-motion";
import { useQueryClient } from "@tanstack/react-query";
import { apiPost, useConnections, useHealth, useWriteAction } from "../api";
import type { ConnectionRow } from "../types";
import { dropConnectionFromCache } from "../optimistic";
import { providerMeta } from "../format";
import { Icon } from "./icons";
import { Button, IconButton, SectionHeading, Skeleton, StateCard, StatusBadge } from "./ui";
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
    ["connections", "health", "launchpad", "prs", "work-items", "pipelines", "notifications", "inbox"].forEach((k) =>
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
    <div className="px-6 py-5 max-w-[1280px] mx-auto">
      <div className="flex items-center justify-between gap-4 mb-3">
        <div>
          <SectionHeading>Connections</SectionHeading>
          <p className="m-0 mt-0.5 text-[12px]" style={{ color: "var(--fg3)" }}>
            Shared with the terminal app · tokens are stored in your OS keychain.
          </p>
        </div>
        <Button primary icon="plus" onClick={() => setEditing(null)}>
          Add connection
        </Button>
      </div>

      {!data || data.length === 0 ? (
        <StateCard icon={<Icon name="plus" size={24} />} title="No connections yet" sub="Add one to start pulling in pull requests, work items, and pipelines." />
      ) : (
        <div className="flex flex-col" style={{ borderTop: "1px solid var(--border)" }}>
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
  const [testing, setTesting] = useState(false);

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
    setTesting(true);
    try {
      const r = await apiPost<{ healthy: boolean }>("/api/connections/test", { id: conn.id });
      setTested(r.healthy);
    } catch {
      setTested(false);
    } finally {
      setTesting(false);
    }
  };

  const status = tested ?? healthy; // boolean | undefined
  return (
    <div className="flex items-center gap-3 min-h-[44px] px-3" style={{ borderBottom: "1px solid var(--border)" }}>
      <div className="flex-1 min-w-0 flex items-center gap-3">
        <span className="inline-block w-1.5 h-1.5 rounded-full shrink-0" style={{ background: providerMeta(conn.provider).color }} title={providerMeta(conn.provider).label} />
        <span className="font-medium truncate" style={{ color: "var(--fg)" }}>{conn.display_name}</span>
        {testing ? (
          <StatusBadge label="Testing" color="var(--blue)" glyph="running" />
        ) : (
          status != null && (
            <StatusBadge
              label={status ? "Connected" : "Auth failed"}
              color={status ? "var(--green)" : "var(--red)"}
              glyph={status ? "check-circle" : "x-circle"}
            />
          )
        )}
        <span className="text-[12px] truncate" style={{ color: "var(--fg3)" }}>
          {conn.sections.length > 0 ? conn.sections.map((s) => SECTION_LABEL[s] ?? s).join(" · ") : "not shown in any section"}
        </span>
      </div>
      <div className="flex items-center gap-1 shrink-0">
        <Button quiet sm onClick={test} disabled={busy || testing}>Test</Button>
        <Button quiet sm onClick={onEdit} disabled={busy}>Edit</Button>
        <Button danger sm onClick={del} disabled={busy}>Delete</Button>
      </div>
    </div>
  );
}

export function Modal({ title, children, onClose }: { title: string; children: React.ReactNode; onClose: () => void }) {
  return (
    <motion.div
      className="fixed inset-0 z-50 flex items-start justify-center"
      style={{ background: "rgba(0,0,0,0.4)", paddingTop: "8vh" }}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      onClick={onClose}
    >
      <motion.div
        role="dialog"
        aria-label={title}
        initial={{ opacity: 0, y: -8, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={{ duration: 0.15 }}
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-lg mx-4 overflow-hidden max-h-[84vh] overflow-y-auto shadow-2xl"
        style={{ background: "var(--raised)", border: "1px solid var(--border-strong)", borderRadius: 10 }}
      >
        <div className="flex items-center justify-between pl-5 pr-3 h-12" style={{ borderBottom: "1px solid var(--border)" }}>
          <SectionHeading>{title}</SectionHeading>
          <IconButton icon="x" label="Close" onClick={onClose} />
        </div>
        <div className="p-5">{children}</div>
      </motion.div>
    </motion.div>
  );
}
