import { motion } from "framer-motion";
import { usePipelines, useWriteAction } from "../api";
import { pipeMeta, relativeTime, toTime } from "../format";
import type { PipeRow } from "../types";
import { Avatar, Chip, List, Pill, ProviderBadge, Skeleton, StateCard } from "./ui";
import { ErrorState } from "./ErrorState";
import { useListView } from "./ControlBar";
import { usePipelineOpener } from "./PipelineDetail";

export function Pipelines() {
  const { data, isLoading, error } = usePipelines();
  const { rows, bar } = useListView<PipeRow>({
    storageKey: "pipelines",
    rows: data,
    connId: (r) => r.connection_id,
    connLabel: (r) => r.connection,
    sorts: [
      { label: "Most recent", cmp: (a, b) => toTime(b.run.finished_at ?? b.run.started_at) - toTime(a.run.finished_at ?? a.run.started_at) },
      { label: "Oldest", cmp: (a, b) => toTime(a.run.finished_at ?? a.run.started_at) - toTime(b.run.finished_at ?? b.run.started_at) },
    ],
    statuses: [
      { label: "Running", match: (r) => r.run.status === "Running" },
      { label: "Failed", match: (r) => r.run.status === "Failed" },
      { label: "Succeeded", match: (r) => r.run.status === "Succeeded" },
      { label: "Awaiting approval", match: (r) => r.approvals.some((a) => a.can_respond) },
    ],
    statusLabel: "Show",
  });

  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || data.length === 0)
    return <StateCard icon="◇" title="No pipeline runs" sub="Recent CI runs for your repositories appear here." />;

  return (
    <>
      {bar}
      <List>
        {rows.map((row, i) => (
          <PipeCard key={`${row.connection_id}:${row.run.id}`} row={row} index={i} />
        ))}
      </List>
    </>
  );
}

function PipeCard({ row, index }: { row: PipeRow; index: number }) {
  const run = row.run;
  const meta = pipeMeta(run.status);
  const label = run.name ?? (run.number != null ? `Run #${run.number}` : run.definition_id);
  const gates = row.approvals.filter((a) => a.can_respond);
  const { busy, error, run: act } = useWriteAction();
  const open = usePipelineOpener();

  const respond = (approvalId: string, decision: "Approve" | "Reject") =>
    act("/api/pipeline/approval", { conn: row.connection_id, run_id: run.id, approval_id: approvalId, decision }, ["pipelines", "launchpad"]);
  const retry = () => act("/api/pipeline/trigger", { conn: row.connection_id, definition_id: run.definition_id }, ["pipelines", "launchpad"]);

  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.22, delay: Math.min(index * 0.02, 0.3), ease: "easeOut" }}
      className="rounded-lg px-4 py-3"
      style={{ background: "var(--card)", border: "1px solid var(--border)" }}
    >
      <div className="flex items-start gap-3">
        <button
          onClick={() => open({ conn: row.connection_id, runId: run.id })}
          className="flex-1 min-w-0 text-left"
          style={{ cursor: "pointer" }}
        >
          <div className="flex items-center gap-2">
            <Pill icon={meta.icon} label={meta.label} color={meta.color} spin={meta.running} />
            <span className="truncate font-medium" style={{ color: "var(--fg)" }}>
              {label}
            </span>
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
            <ProviderBadge provider={row.provider} connection={row.connection} />
            {run.branch && <Chip title="branch">⑂ {run.branch}</Chip>}
            {run.commit_sha && <span className="mono text-xs" style={{ color: "var(--dim)" }}>{run.commit_sha.slice(0, 7)}</span>}
          </div>
        </button>
        <div className="flex flex-col items-end gap-2 shrink-0">
          <span className="text-xs whitespace-nowrap" style={{ color: "var(--dim)" }}>
            {relativeTime(run.finished_at ?? run.started_at)}
          </span>
          {run.triggered_by && <Avatar name={run.triggered_by.display_name} />}
        </div>
      </div>

      {(gates.length > 0 || run.status === "Failed") && (
        <div className="mt-3 flex flex-wrap items-center gap-2" style={{ borderTop: "1px solid var(--border)", paddingTop: "0.6rem" }}>
          {gates.map((g) => (
            <div key={g.id} className="flex items-center gap-1.5">
              <span className="text-xs" style={{ color: "var(--yellow)" }}>
                ⏳ {g.name}
              </span>
              <ActBtn label="Approve" color="var(--green)" disabled={busy} onClick={() => respond(g.id, "Approve")} />
              <ActBtn label="Reject" color="var(--red)" disabled={busy} onClick={() => respond(g.id, "Reject")} />
            </div>
          ))}
          {run.status === "Failed" && <ActBtn label="↻ Retry" color="var(--blue)" disabled={busy} onClick={retry} />}
          {error && <span className="text-xs" style={{ color: "var(--red)" }}>{error}</span>}
        </div>
      )}
    </motion.div>
  );
}

function ActBtn({ label, color, onClick, disabled }: { label: string; color: string; onClick: () => void; disabled?: boolean }) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="rounded px-2 py-0.5 text-xs font-medium"
      style={{ color, border: `1px solid ${color}`, background: "transparent", opacity: disabled ? 0.5 : 1, cursor: disabled ? "not-allowed" : "pointer" }}
    >
      {label}
    </button>
  );
}
