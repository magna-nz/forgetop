import { useMemo, useRef, useState, type MouseEvent } from "react";
import { usePipelines } from "../api";
import { fmtSecs, pipeMeta, sameRef, toTime } from "../format";
import type { PipelineRunStatus, PipeRow } from "../types";
import { groupPipelines, listStrip, parseGroupMode, pipeDefinitionName, rowGates, rowShownStatus } from "../pipeline/grouping";
import type { GroupMode, PipeGroup } from "../pipeline/grouping";
import { runSecs } from "../pipeline/timeline";
import { Icon } from "./icons";
import { Avatar, Button, Chip, ContextMenu, List, RowMenuButton, Segmented, Skeleton, StateCard, StatusGlyph, useContextMenu } from "./ui";
import { ErrorState } from "./ErrorState";
import { shortAge, ToggleRow, useListView, type SortOption } from "./ControlBar";
import { useCurrentPipeline, usePipelineOpener } from "./PipelineDetail";
import { useRepoScope } from "./RepoScope";
import { usePipelineScope } from "./PipelineScope";
import { usePipelineRowMenu } from "./rowMenus/pipeline";

const GROUP_KEY = "forgetop:pipelines:group";

const GROUP_OPTIONS: { value: GroupMode; label: string }[] = [
  { value: "pipeline", label: "By pipeline" },
  { value: "trigger", label: "By trigger" },
  { value: "branch", label: "By branch" },
  { value: "off", label: "Flat" },
];

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
  const chooseMode = (m: GroupMode) => {
    setMode(m);
    try {
      localStorage.setItem(GROUP_KEY, m);
    } catch {
      /* ignore */
    }
  };
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
    // The pipeline picker stays reachable in every state the bar shows: with nothing selected there
    // are no rows, and that is exactly when it is needed.
    extra: (
      <>
        {pipes.control}
        {scope.control}
        <span className="px-1">
          <Segmented label="Group" value={mode} onChange={chooseMode} options={GROUP_OPTIONS} />
        </span>
      </>
    ),
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
        icon={<Icon name="workflow" size={24} />}
        title="No repositories selected"
        sub="This connection spans your whole account — choose which repositories it fetches from."
        action={scope.control}
      />
    );
  if (pipes.noneSelected)
    // Pipelines are opt-in: nothing was fetched because nothing was chosen yet. The picker sits in
    // the card, like the empty repository scope's.
    return (
      <StateCard
        icon={<Icon name="workflow" size={24} />}
        title="No pipelines selected"
        sub="Choose which pipelines to show. Your choice is saved."
        action={pipes.control}
      />
    );
  if (!data || data.length === 0)
    return (
      <>
        {bar}
        <StateCard icon={<Icon name="workflow" size={24} />} title="No pipeline runs" sub="Recent runs of the pipelines you've chosen appear here." />
      </>
    );

  const toggle = (key: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  const now = new Date();

  return (
    <>
      {bar}
      <List>
        {groups.length > 0
          ? groups.map((g) => (
              <PipeGroupSection key={g.key} group={g} mode={mode} now={now} open={expanded.has(g.key)} onToggle={() => toggle(g.key)} />
            ))
          : rows.map((row) => <PipeRunRow key={rowKey(row)} row={row} now={now} />)}
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
    <>
      <ToggleRow open={open} onToggle={onToggle} height={40}>
        <StatusGlyph glyph={meta.glyph} color={meta.color} spin={meta.running} title={meta.label} />
        <span className="truncate font-semibold min-w-0 shrink">{group.subject}</span>
        {group.repo && (
          <span className="mono shrink-0 whitespace-nowrap" style={{ color: "var(--fg3)" }} title="repository">
            {group.repo}
          </span>
        )}
        {mode === "trigger" && group.commit && (
          <span className="mono shrink-0" style={{ color: "var(--fg3)" }}>
            {group.commit}
          </span>
        )}
        {(mode === "pipeline" || mode === "trigger") && branch && (
          <span className="mono truncate min-w-0" style={{ color: "var(--fg3)", maxWidth: 260 }} title="branch">
            {branch}
          </span>
        )}
        {group.failed > 0 && (
          <span className="text-[12px] shrink-0 whitespace-nowrap" style={{ color: "var(--red)" }}>
            {group.failed} failed
          </span>
        )}
        {group.approval && (
          <span className="text-[12px] shrink-0 whitespace-nowrap" style={{ color: "var(--yellow)" }}>
            waiting for approval
          </span>
        )}
        <span className="flex-1" />
        <StageStrip row={latest} />
        <span className="num text-[12px] shrink-0 whitespace-nowrap text-right" style={{ color: "var(--fg3)", minWidth: 44 }}>
          {group.runs} {group.runs === 1 ? "run" : "runs"}
        </span>
        <span className="num text-[12px] shrink-0 whitespace-nowrap text-right" style={{ color: "var(--fg3)", width: 30 }}>
          {shortAge(group.started)}
        </span>
      </ToggleRow>
      {open && group.rows.map((row) => <PipeRunRow key={rowKey(row)} row={row} now={now} mode={mode} />)}
    </>
  );
}

/** One small bar per stage (per job where the run has a single stage). Nothing for a run that
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
    <span role="img" aria-label={`${unit}: ${summary}`} title={`${unit}: ${summary}`} className="inline-flex items-center shrink-0" style={{ gap: 2 }}>
      {statuses.map((s, i) => (
        <span key={i} className={s === "Running" ? "pulse" : undefined} style={{ width: 5, height: 10, borderRadius: 1.5, background: barColor(s) }} />
      ))}
    </span>
  );
}

/** Done and live stages lit in their status colour; ones that haven't run (or won't) unlit. */
function barColor(s: PipelineRunStatus): string {
  switch (s) {
    case "Succeeded":
      return "var(--green)";
    case "Failed":
      return "var(--red)";
    case "Running":
      return "var(--blue)";
    case "Waiting":
    case "PartiallySucceeded":
      return "var(--yellow)";
    default:
      return "var(--active)";
  }
}

/** The first failed job's problem, as `job · problem`, when the run failed and one says why. */
function failedProblem(row: PipeRow): string | null {
  if (row.run.status !== "Failed") return null;
  for (const s of row.run.stages) for (const j of s.jobs) if (j.status === "Failed" && j.problem) return `${j.name} · ${j.problem}`;
  return null;
}

/** One run. Flat (no `mode`) it names its pipeline and carries its stage strip; under a header
 *  (34px, indented past the header's chevron and glyph) it names the pipeline only when that is
 *  what varies within the group. The row opens the run; the Approve button, shown only for a gate
 *  this user can answer, opens it too — the decision itself is taken in the pane's gate banner.
 *  Right-click, Shift+F10 or the ⋯ at the right edge opens the run's menu without opening it. */
function PipeRunRow({ row, now, mode }: { row: PipeRow; now: Date; mode?: GroupMode }) {
  const run = row.run;
  const child = mode !== undefined;
  const meta = pipeMeta(rowShownStatus(row));
  const pipelineName = child ? (mode === "pipeline" ? null : pipeDefinitionName(row)) : (row.definition_name ?? run.definition_id);
  const runName = child && run.number != null ? `#${run.number}` : (run.name ?? (run.number != null ? `#${run.number}` : ""));
  const problem = failedProblem(row);
  const secs = runSecs(run, now);
  const gate = row.approvals.find((a) => a.can_respond);
  const waitingOnOthers = !gate && row.approvals.length > 0;
  const openRun = usePipelineOpener();
  const current = useCurrentPipeline();
  const selected = sameRef(current, { conn: row.connection_id, repo: run.repository, runId: run.id });
  const open = () => openRun({ conn: row.connection_id, repo: run.repository, runId: run.id });
  const stop = (fn: () => void) => (e: MouseEvent) => {
    e.stopPropagation();
    fn();
  };
  const menu = useContextMenu();
  const build = usePipelineRowMenu();
  const rowEl = useRef<HTMLDivElement>(null);
  const runButton = useRef<HTMLButtonElement>(null);

  return (
    // The whole row is a click target for the mouse; the inner button is the keyboard and
    // screen-reader target. They are siblings of the Approve button, never its parent.
    <>
      <div
        ref={rowEl}
        className="ft-row"
        data-selected={selected || undefined}
        data-menu-open={menu.open ? "true" : undefined}
        style={{ minHeight: child ? 34 : 36, paddingLeft: child ? 44 : 12 }}
        onClick={open}
        onContextMenu={menu.onContextMenu}
        onKeyDown={menu.onKeyDown}
      >
        <button
          ref={runButton}
          type="button"
          onClick={stop(open)}
          className="flex items-center gap-2.5 flex-1 min-w-0 text-left rounded outline-none focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
          style={{ background: "transparent", border: 0, padding: 0, color: "inherit", font: "inherit", cursor: "pointer" }}
        >
          <StatusGlyph glyph={meta.glyph} color={meta.color} spin={meta.running} title={meta.label} />
          {runName && (
            <span className="mono shrink-0 whitespace-nowrap" style={{ color: "var(--fg3)", minWidth: 30 }}>
              {runName}
            </span>
          )}
          {pipelineName && <span className="font-medium shrink-0 whitespace-nowrap">{pipelineName}</span>}
          <span className="flex-1 min-w-0 truncate">
            {run.title && <span>{run.title}</span>}
            {problem && <span style={{ color: "var(--red)" }}>{run.title ? " " : ""}· {problem}</span>}
          </span>
          {run.branch && (
            <Chip title="branch" className="shrink-0 max-w-[200px] truncate">
              {run.branch}
            </Chip>
          )}
          {run.commit_sha && (
            <span className="mono shrink-0" style={{ color: "var(--fg3)" }}>
              {run.commit_sha.slice(0, 7)}
            </span>
          )}
        </button>
        {gate && (
          <Button
            sm
            onClick={stop(open)}
            title={`Open the run to approve ${gate.name}`}
            style={{ color: "var(--yellow)", borderColor: "color-mix(in srgb, var(--yellow) 40%, transparent)" }}
          >
            Open to approve {gate.name}
          </Button>
        )}
        {waitingOnOthers && (
          <span className="text-[12px] shrink-0 whitespace-nowrap" style={{ color: "var(--yellow)" }}>
            waiting for approval
          </span>
        )}
        {!child && <StageStrip row={row} />}
        <span className="num text-[12px] shrink-0 whitespace-nowrap text-right" style={{ width: 50, color: meta.running ? "var(--blue)" : "var(--fg3)" }}>
          {secs !== null ? fmtSecs(secs) : ""}
        </span>
        <span className="num text-[12px] shrink-0 whitespace-nowrap text-right" style={{ width: 30, color: "var(--fg3)" }}>
          {shortAge(run.finished_at ?? run.started_at)}
        </span>
        <span className="inline-flex shrink-0 justify-center" style={{ width: 18 }}>
          {run.triggered_by && <Avatar name={run.triggered_by.display_name} size={18} />}
        </span>
        <RowMenuButton onOpen={menu.openAt} />
      </div>
      <ContextMenu
        anchor={menu.anchor}
        onClose={menu.close}
        label="Pipeline run actions"
        items={() => build(row)}
        // The row itself takes no focus; a menu opened on it hands focus back to the run's button.
        returnFocusTo={menu.anchor?.origin === rowEl.current ? runButton.current : undefined}
      />
    </>
  );
}
