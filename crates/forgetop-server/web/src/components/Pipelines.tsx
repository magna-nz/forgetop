import { motion } from "framer-motion";
import { usePipelines } from "../api";
import { pipeMeta, relativeTime, toTime } from "../format";
import type { PipeRow } from "../types";
import { Avatar, Chip, List, Pill, Skeleton, StateCard, StatusBadge } from "./ui";
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
  // A pending gate → a red "Approval needed" badge (like the PR Mergeable badge). Actions
  // (approve / re-run / cancel) live in the pane, not on the row.
  const needsApproval = row.approvals.length > 0;
  const open = usePipelineOpener();

  return (
    <motion.div
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.22, delay: Math.min(index * 0.02, 0.3), ease: "easeOut" }}
      className="flex items-center gap-3 rounded-lg px-3 py-1.5"
      style={{ background: "var(--card)", border: "1px solid var(--border)" }}
    >
      <button
        onClick={() => open({ conn: row.connection_id, runId: run.id })}
        className="flex-1 min-w-0 text-left flex items-center gap-2"
        style={{ cursor: "pointer" }}
      >
        {/* Fixed-width leading columns so the run number, branch, commit and title line up
            across rows regardless of how wide each status word is. */}
        <span className="shrink-0" style={{ width: 84 }}>
          <StatusBadge label={cap(meta.label)} color={meta.color} />
        </span>
        <span className="font-medium shrink-0 truncate" style={{ width: 72, color: "var(--fg)" }}>{label}</span>
        {run.branch && (
          <span className="shrink-0">
            <Chip title="branch">⑂ {run.branch}</Chip>
          </span>
        )}
        {run.commit_sha && <span className="mono text-xs shrink-0" style={{ color: "var(--dim)" }}>{run.commit_sha.slice(0, 7)}</span>}
        {run.title && <span className="truncate text-sm italic min-w-0" style={{ color: "var(--dim)" }}>{run.title}</span>}
        {needsApproval && (
          <span className="shrink-0">
            <Pill icon="⏳" label="Approval needed" color="var(--red)" />
          </span>
        )}
      </button>
      <div className="flex items-center gap-2 shrink-0">
        <span className="text-xs whitespace-nowrap" style={{ color: "var(--dim)" }}>
          {relativeTime(run.finished_at ?? run.started_at)}
        </span>
        {run.triggered_by && <Avatar name={run.triggered_by.display_name} />}
      </div>
    </motion.div>
  );
}

const cap = (s: string): string => (s.length ? s[0].toUpperCase() + s.slice(1) : s);
