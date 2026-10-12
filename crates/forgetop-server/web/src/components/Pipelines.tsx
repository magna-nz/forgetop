import { useMemo, useState } from "react";
import { motion } from "framer-motion";
import { usePipelines } from "../api";
import { fmtSecs, pipeMeta, relativeTime, toTime } from "../format";
import type { PipelineRunStatus, PipeRow } from "../types";
import { groupPipelines, listStrip, parseGroupMode, pipeDefinitionName, rowGates, rowShownStatus } from "../pipeline/grouping";
import type { GroupMode, PipeGroup } from "../pipeline/grouping";
import { runSecs } from "../pipeline/timeline";
import { Avatar, Chip, List, Pill, Skeleton, StateCard, StatusBadge } from "./ui";
import { ErrorState } from "./ErrorState";
import { useListView, type SortOption } from "./ControlBar";
import { usePipelineOpener } from "./PipelineDetail";
import { useRepoScope } from "./RepoScope";
import { usePipelineScope } from "./PipelineScope";

const GROUP_KEY = "forgetop:pipelines:group";

/** Grouped by pipeline unless the viewer chose otherwise. */
function readGroupMode(): GroupMode {
  try {
    return parseGroupMode(localStorage.getItem(GROUP_KEY));
  } catch {
    return "pipeline";
  }
}

const SORTS: SortOption<PipeRow>[] = [
  { label: "Most recent", cmp: (a, b) => toTime(b.run.finished_at ?? b.run.started_at) - toTime(a.run.finished_at ?? a.run.started_at) },
  { label: "Oldest", cmp: (a, b) => toTime(a.run.finished_at ?? a.run.started_at) - toTime(b.run.finished_at ?? b.run.started_at) },
];

const rowKey = (row: PipeRow) => `${row.connection_id}:${row.run.repository ?? ""}:${row.run.id}`;

export function Pipelines() {
  const { data, isLoading, error } = usePipelines();
  const [mode, setMode] = useState<GroupMode>(readGroupMode);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const scope = useRepoScope("pipelines");
  const pipes = usePipelineScope();
  const { rows, bar, sortIndex } = useListView<PipeRow>({
    storageKey: "pipelines",
    rows: data,
    connId: (r) => r.connection_id,
    connLabel: (r) => r.connection,
    sorts: SORTS,
    statuses: [
      { label: "Running", match: (r) => r.run.status === "Running" },
      { label: "Waiting", match: (r) => r.run.status === "Waiting" },
      { label: "Failed", match: (r) => r.run.status === "Failed" },
      { label: "Succeeded", match: (r) => r.run.status === "Succeeded" },
      { label: "Awaiting approval", match: (r) => r.approvals.some((a) => a.can_respond) },
    ],
    statusLabel: "Show",
  });

  // A view over the rows already sorted and filtered: every mode shows the same runs. The sort
  // orders the groups too, by each one's most recent run (the TUI's `pipe_lines`).
  const sortCmp = SORTS[sortIndex]?.cmp;
  const groups = useMemo(() => groupPipelines(rows, mode, sortCmp), [mode, rows, sortCmp]);

  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (scope.noneSelected)
    // Distinct from "nothing to show": nothing was fetched because nothing was asked for.
    return (
      <StateCard
        icon="◍"
        title="No repositories selected"
        sub="This connection spans your whole account — choose which repositories it fetches from."
        action={scope.control}
      />
    );
  // The pipeline picker stays reachable in every state below: with nothing selected there are no
  // rows, and that is exactly when it is needed.
  const controls = (
    <div className="flex flex-wrap items-center gap-2 px-5 pt-4 max-w-5xl mx-auto">
      {pipes.control}
      {scope.control}
    </div>
  );
  if (pipes.noneSelected)
    // Pipelines are opt-in: nothing was fetched because nothing was chosen yet. The picker sits in
    // the card, like the empty repository scope's.
    return (
      <StateCard
        icon="◇"
        title="No pipelines selected"
        sub="Choose which pipelines to show. Your choice is saved."
        action={pipes.control}
      />
    );
  if (!data || data.length === 0)
    return (
      <>
        {controls}
        <StateCard icon="◇" title="No pipeline runs" sub="Recent runs of the pipelines you've chosen appear here." />
      </>
    );

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
  const now = new Date();

  return (
    <>
      {controls}
      {bar}
      <div className="flex flex-wrap items-center gap-2 px-5 pt-2 max-w-5xl mx-auto">
        <label className="flex items-center gap-1.5 text-xs" style={{ color: "var(--dim)" }}>
          Group
          <select
            value={mode}
            onChange={(e) => chooseMode(parseGroupMode(e.target.value))}
            className="rounded-md px-2 py-1 text-xs outline-none"
            style={{ background: "var(--panel2)", color: "var(--fg)", border: "1px solid var(--border)" }}
          >
            <option value="pipeline">Pipeline</option>
            <option value="trigger">Trigger</option>
            <option value="branch">Branch</option>
            <option value="off">Off</option>
          </select>
        </label>
      </div>
      <List>
        {groups.length > 0
          ? groups.map((g) => (
              <PipeGroupSection key={g.key} group={g} mode={mode} now={now} open={expanded.has(g.key)} onToggle={() => toggle(g.key)} />
            ))
          : rows.map((row, i) => <PipeCard key={rowKey(row)} row={row} index={i} now={now} />)}
      </List>
    </>
  );
}

function PipeGroupSection({
  group,
  mode,
  now,
  open,
  onToggle,
}: {
  group: PipeGroup;
  mode: GroupMode;
  now: Date;
  open: boolean;
  onToggle: () => void;
}) {
  const meta = pipeMeta(group.status);
  const latest = group.latest;
  const branch = latest.run.branch;
  return (
    <div className="flex flex-col gap-2">
      <div className="rounded-lg px-3 py-1.5" style={{ background: "var(--panel2)", border: "1px solid var(--border)" }}>
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={open}
          className="w-full min-w-0 text-left flex items-center gap-2"
          style={{ cursor: "pointer" }}
        >
          <span className="text-xs shrink-0" style={{ color: "var(--dim)", width: 14 }} aria-hidden="true">
            {open ? "▾" : "▸"}
          </span>
          {/* The same fixed status column as the run rows, so names line up. */}
          <span className="shrink-0" style={{ width: 84 }}>
            <StatusBadge label={cap(meta.label)} color={meta.color} />
          </span>
          <span className="truncate font-semibold min-w-0" style={{ color: "var(--fg)" }}>
            {group.subject}
          </span>
          {group.failed > 0 && (
            <span className="text-xs shrink-0 whitespace-nowrap" style={{ color: "var(--red)" }}>
              · {group.failed} failed
            </span>
          )}
          {group.repo && (
            <span className="shrink-0">
              <Chip title="repository">{group.repo}</Chip>
            </span>
          )}
          {mode === "trigger" && group.commit && (
            <span className="mono text-xs shrink-0" style={{ color: "var(--dim)" }}>
              {group.commit}
            </span>
          )}
          {(mode === "pipeline" || mode === "trigger") && branch && (
            <span className="min-w-0 truncate" style={{ maxWidth: 260 }}>
              <Chip title="branch">⑂ {branch}</Chip>
            </span>
          )}
          <StageStrip row={latest} />
          {group.approval && (
            <span className="shrink-0">
              <Pill icon="⏳" label="Approval needed" color="var(--red)" />
            </span>
          )}
          <span className="flex-1" />
          <span className="text-xs shrink-0 whitespace-nowrap" style={{ color: "var(--dim)" }}>
            {group.runs} {group.runs === 1 ? "run" : "runs"}
          </span>
          <span className="text-xs shrink-0 whitespace-nowrap" style={{ color: "var(--dim)" }}>
            {relativeTime(group.started)}
          </span>
          {latest.run.triggered_by && <Avatar name={latest.run.triggered_by.display_name} />}
        </button>
      </div>
      {open &&
        group.rows.map((row, i) => (
          <PipeCard key={rowKey(row)} row={row} index={i} now={now} child={{ mode, last: i === group.rows.length - 1 }} />
        ))}
    </div>
  );
}

/** One small square per stage (per job where the run has a single stage). Nothing for a run that
 *  carries no stages. */
function StageStrip({ row }: { row: PipeRow }) {
  const { unit, statuses } = listStrip(row.run, rowShownStatus(row), rowGates(row));
  if (statuses.length === 0) return null;
  const counts = new Map<string, number>();
  for (const s of statuses) {
    const label = pipeMeta(s).label;
    counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  const summary = [...counts].map(([label, n]) => `${n} ${label}`).join(", ");
  return (
    <span role="img" aria-label={`${unit}: ${summary}`} title={`${unit}: ${summary}`} className="inline-flex items-center shrink-0" style={{ gap: 3 }}>
      {statuses.map((s, i) => (
        <span
          key={i}
          className={s === "Running" ? "pulse" : undefined}
          style={{ width: 7, height: 7, borderRadius: 2, background: squareColor(s) }}
        />
      ))}
    </span>
  );
}

/** Done and live stages in their status colour; ones that haven't run (or won't) muted. */
function squareColor(s: PipelineRunStatus): string {
  return s === "Queued" || s === "Skipped" ? "color-mix(in srgb, var(--dim) 45%, var(--border))" : pipeMeta(s).color;
}

/** The first failed job's problem, as `✗ job · problem`, when the run failed and one says why. */
function failedProblem(row: PipeRow): string | null {
  if (row.run.status !== "Failed") return null;
  for (const s of row.run.stages)
    for (const j of s.jobs) if (j.status === "Failed" && j.problem) return `✗ ${j.name} · ${j.problem}`;
  return null;
}

function PipeCard({
  row,
  index,
  now,
  child,
}: {
  row: PipeRow;
  index: number;
  now: Date;
  /** Set when the row sits under a group header: its grouping and whether it is the last one. */
  child?: { mode: GroupMode; last: boolean };
}) {
  const run = row.run;
  const shown = rowShownStatus(row);
  const meta = pipeMeta(shown);
  // Flat: the pipeline (definition) name first, then the run's own name/number. Under a header
  // the pipeline name is only shown when it is what varies within the group.
  const pipelineName = child
    ? child.mode === "pipeline"
      ? null
      : pipeDefinitionName(row)
    : (row.definition_name ?? run.definition_id);
  const runName = child && run.number != null ? `#${run.number}` : (run.name ?? (run.number != null ? `#${run.number}` : ""));
  const problem = failedProblem(row);
  const secs = runSecs(run, now);
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
      style={{ background: "var(--card)", border: "1px solid var(--border)", ...(child ? { marginLeft: 16 } : {}) }}
    >
      <button
        onClick={() => open({ conn: row.connection_id, repo: run.repository, runId: run.id })}
        className="flex-1 min-w-0 text-left flex items-center gap-2"
        style={{ cursor: "pointer" }}
      >
        {child && (
          <span className="mono text-xs shrink-0" style={{ color: "var(--dim)", opacity: 0.6, width: 14 }} aria-hidden="true">
            {child.last ? "└" : "├"}
          </span>
        )}
        {/* Fixed-width status column so the pipeline name lines up across rows. */}
        <span className="shrink-0" style={{ width: 84 }}>
          <StatusBadge label={cap(meta.label)} color={meta.color} />
        </span>
        {pipelineName && <span className="font-medium shrink-0" style={{ color: "var(--fg)" }}>{pipelineName}</span>}
        {runName && <span className="mono text-xs shrink-0" style={{ color: "var(--dim)" }}>{runName}</span>}
        {run.branch && (
          <span className="shrink-0">
            <Chip title="branch">⑂ {run.branch}</Chip>
          </span>
        )}
        {run.commit_sha && <span className="mono text-xs shrink-0" style={{ color: "var(--dim)" }}>{run.commit_sha.slice(0, 7)}</span>}
        {problem ? (
          <span className="truncate text-xs min-w-0" style={{ color: "var(--red)" }}>{problem}</span>
        ) : (
          run.title && <span className="truncate text-sm italic min-w-0" style={{ color: "var(--dim)" }}>{run.title}</span>
        )}
        {needsApproval && (
          <span className="shrink-0">
            <Pill icon="⏳" label="Approval needed" color="var(--red)" />
          </span>
        )}
      </button>
      <div className="flex items-center gap-2 shrink-0">
        <StageStrip row={row} />
        {secs !== null && (
          <span className="text-xs whitespace-nowrap" style={{ color: meta.running ? "var(--accent)" : "var(--dim)" }}>
            {fmtSecs(secs)}
          </span>
        )}
        <span className="text-xs whitespace-nowrap" style={{ color: "var(--dim)" }}>
          {relativeTime(run.finished_at ?? run.started_at)}
        </span>
        {run.triggered_by && <Avatar name={run.triggered_by.display_name} />}
      </div>
    </motion.div>
  );
}

const cap = (s: string): string => (s.length ? s[0].toUpperCase() + s.slice(1) : s);
