// The run pane's tree + timeline: `flatten`'s rows (../../pipeline/timeline) with a duration
// column and a bar lane on one shared time axis — bars for what ran, a now line while live, the
// gated stage's wait, and estimate "ghosts" for what is still to run. Below ~480px the lane is
// dropped and the plain tree remains. `StageStrip` is the one-line stage overview above it.
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { axisLabel, fmtSecs, pipeMeta } from "../../format";
import { isActive } from "../../pipeline/grouping";
import {
  axisTicks,
  criticalPath,
  criticalPathText,
  ghostKey,
  maxTicksFor,
  msOf,
  timelineAxis,
  timelineSpan,
  type Estimates,
  type FlatNode,
  type Ghost,
} from "../../pipeline/timeline";
import type { PipelineRun, PipelineRunStatus } from "../../types";
import { Icon } from "../icons";
import { StatusGlyph } from "../ui";
import { statusBarColor } from "./parts";

/** The label column (glyph, indent, name), the duration column, the ghost "~left" column. */
const LABEL_W = 230;
const DUR_W = 46;
const GHOST_W = 64;
const INDENT = 14;
/** Width of the chevron slot + glyph + gaps before the label text. */
const LEAD_W = 14 + 14 + 16;
/** Below this width the bar lane is dropped. */
const NARROW_PX = 480;

/** A stable identity for a row across re-flattens (indices move as folds change). */
export const nodeId = (n: FlatNode) =>
  `${n.stage}|${n.job ?? ""}|${n.step ?? ""}|${n.group ? (n.key ?? "") : ""}|${n.info ? n.label : ""}`;

const HATCH: CSSProperties = {
  border: "1px dashed var(--fg3)",
  background: "repeating-linear-gradient(45deg, color-mix(in srgb, var(--fg3) 35%, transparent) 0 3px, transparent 3px 6px)",
  boxSizing: "border-box",
};
const WAIT: CSSProperties = {
  background: "repeating-linear-gradient(90deg, color-mix(in srgb, var(--yellow) 55%, transparent) 0 4px, transparent 4px 7px)",
};

/** Width of an element, tracked (Infinity where ResizeObserver isn't available, e.g. jsdom). */
function useWidth<T extends HTMLElement>(): [React.RefObject<T>, number] {
  const ref = useRef<T>(null);
  const [w, setW] = useState(Infinity);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width;
      if (width) setW(width);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, w];
}

/** The header's count: `Steps · 15` for a single-job run (its steps are the rows), else `Jobs · 9`. */
function countLabel(run: PipelineRun): string {
  if (run.stages.length === 1 && run.stages[0].jobs.length === 1 && run.stages[0].jobs[0].steps.length > 0) {
    return `Steps · ${run.stages[0].jobs[0].steps.length}`;
  }
  return `Jobs · ${run.stages.reduce((acc, s) => acc + s.jobs.length, 0)}`;
}

const toneColor = (t: "neutral" | "warn" | "bad") => (t === "bad" ? "var(--red)" : t === "warn" ? "var(--yellow)" : "var(--fg3)");

/** One column per stage — a 3px bar in its colour, its name and duration under it — from the
 *  tree's top-level rows. Only for runs of several stages (a single stage drops its tree row). */
export function StageStrip({ run, nodes }: { run: PipelineRun; nodes: FlatNode[] }) {
  if (run.stages.length < 2) return null;
  const stages = nodes.filter((n) => n.depth === 0 && n.job === null && !n.info);
  if (stages.length === 0) return null;
  return (
    <div className="flex gap-2.5" role="list" aria-label="Stages">
      {stages.map((n) => {
        const m = pipeMeta(n.status);
        const ran = statusBarColor(n.status) !== "var(--active)" || n.status === "Canceled";
        return (
          <div key={`${n.stage}|${n.label}`} role="listitem" className="flex flex-col gap-[5px] min-w-0" style={{ flex: "1 1 0" }}>
            <div style={{ height: 3, borderRadius: 2, background: statusBarColor(n.status) }} />
            <div className="flex items-center gap-[5px] text-[12px] min-w-0 whitespace-nowrap" style={{ color: ran ? "var(--fg2)" : "var(--fg3)" }}>
              {ran && <StatusGlyph glyph={m.glyph} color={m.color} size={12} spin={m.running} />}
              <span className="truncate">{n.label}</span>
              {n.duration && (
                <span className="num shrink-0" style={{ color: "var(--fg3)" }}>
                  {n.duration}
                </span>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

export function RunTimeline({
  run,
  nodes,
  shown,
  ghosts,
  estimates,
  now,
  selectedId,
  onToggle,
  onSelect,
  providerLabel,
}: {
  run: PipelineRun;
  nodes: FlatNode[];
  shown: PipelineRunStatus;
  ghosts: Map<string, Ghost>;
  estimates: Estimates | null;
  now: Date;
  selectedId: string | null;
  onToggle: (key: string, open: boolean) => void;
  onSelect: (node: FlatNode) => void;
  providerLabel: string;
}) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const active = isActive(run.status);
  const span = timelineSpan(run, shown, ghosts, now);
  const lane = width >= NARROW_PX && span !== null;
  const axis = span ? timelineAxis(span, 1000) : null;
  const nowMs = now.getTime();
  /** Percent across the lane of an instant (epoch ms). */
  const pct = (ms: number) => (span && axis ? Math.min(100, Math.max(0, axis.pos((ms - span.t0) / 1000) / 10)) : 0);
  const showGhostCol = lane && ghosts.size > 0;
  const longestLeft = Math.max(-1, ...[...ghosts.values()].map((g) => g.left ?? -1));

  const lanePx = Number.isFinite(width) ? width - LABEL_W - LEAD_W - DUR_W - (showGhostCol ? GHOST_W : 0) - 40 : 480;
  const ticks = axis ? axisTicks(axis, maxTicksFor(Math.floor(lanePx / 8))).ticks : [];
  const nowPct = active ? pct(nowMs) : null;
  const squeezed = !!axis && axis.ticked()[0] !== axis.total;

  let footer: React.ReactNode = null;
  if (active) {
    const cp = criticalPath(run, ghosts);
    if (cp || ghosts.size > 0)
      footer = (
        <>
          {cp && <span>{criticalPathText(cp)}</span>}
          <span className="flex-1" />
          {ghosts.size > 0 && estimates && (
            <span className="inline-flex items-center gap-1.5">
              <span className="inline-block" style={{ width: 18, height: 8, borderRadius: 2, ...HATCH }} />
              estimate, from the last {estimates.jobRuns} passing {estimates.jobRuns === 1 ? "run" : "runs"}
            </span>
          )}
        </>
      );
  }

  // A single-job run lists that job's steps; name the job above them.
  const singleJob = run.stages.length === 1 && run.stages[0].jobs.length === 1 && run.stages[0].jobs[0].steps.length > 0 ? run.stages[0].jobs[0].name : null;

  // Notes for rows with no span sit where the run's work had got to: the latest bar start above them.
  let lastStartPct = 0;

  return (
    <div ref={ref} className="flex flex-col gap-px">
      <div className="flex items-center gap-2 px-2 text-[12px]" style={{ height: 24, color: "var(--fg3)" }}>
        <span className="flex items-center gap-2 min-w-0" style={{ flex: lane ? `0 0 ${LABEL_W + LEAD_W}px` : "1 1 auto" }}>
          <span className="shrink-0">{countLabel(run)}</span>
          {singleJob && (
            <span className="truncate" style={{ color: "var(--fg2)" }}>
              {singleJob}
            </span>
          )}
        </span>
        <span style={{ flex: `0 0 ${DUR_W}px` }} />
        {lane && (
          <div className="relative flex-1 num" style={{ height: 18 }} aria-hidden="true">
            {ticks
              .filter((t) => nowPct === null || Math.abs(t.pos / 10 - nowPct) > 7)
              .filter((t) => !squeezed || t.pos / 10 < 62)
              .map((t) => (
                <span
                  key={t.secs}
                  className="absolute whitespace-nowrap"
                  style={{ left: `${t.pos / 10}%`, transform: t.pos / 10 > 92 ? "translateX(-100%)" : undefined }}
                >
                  {t.label}
                </span>
              ))}
            {squeezed && span?.waitSecs != null && (
              <span className="absolute whitespace-nowrap" style={{ left: `${pct(span.last!) + 2}%`, color: "var(--yellow)" }}>
                waited {fmtSecs(span.waitSecs)}
              </span>
            )}
            {nowPct !== null && (
              <span
                className="absolute font-medium"
                style={{ left: `${nowPct}%`, color: "var(--fg2)", transform: nowPct > 92 ? "translateX(-100%)" : undefined }}
              >
                now
              </span>
            )}
          </div>
        )}
        {showGhostCol && <span style={{ flex: `0 0 ${GHOST_W}px` }} />}
      </div>

      {nodes.length === 0 ? (
        <div className="px-2 py-1 text-[12.5px]" style={{ color: "var(--fg3)" }}>
          No stages reported for this run.
        </div>
      ) : (
        nodes.map((n) => {
          const id = nodeId(n);
          if (n.info) {
            const link = n.label.startsWith("o  ");
            return (
              <div key={id} className="text-[12px] py-0.5" style={{ color: "var(--fg3)", paddingLeft: 8 + LEAD_W + n.depth * INDENT }}>
                {link ? (
                  run.url ? (
                    <a href={run.url} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1" style={{ color: "var(--accent-fg)" }}>
                      open in {providerLabel} to approve
                      <Icon name="external" size={12} />
                    </a>
                  ) : (
                    <span>open in {providerLabel} to approve</span>
                  )
                ) : (
                  n.label
                )}
              </div>
            );
          }
          const m = pipeMeta(n.status);
          const selected = id === selectedId;
          const isStage = n.job === null && !n.group && run.stages.length > 1;
          const gated = n.status === "Waiting" && n.job === null && !n.group;
          const dimLabel = n.group || n.status === "Queued" || n.status === "Skipped" || n.status === "Canceled";
          const labelColor = n.status === "Failed" ? "var(--red)" : gated ? "var(--yellow)" : dimLabel ? "var(--fg3)" : "var(--fg)";
          const start = msOf(n.startedAt);
          const fin = msOf(n.finishedAt);
          const end = fin ?? (start !== null && isActive(n.status) && !gated ? nowMs : null);

          let dur: { text: string; color: string } | null = n.duration ? { text: n.duration, color: "var(--fg3)" } : null;
          if (!dur && n.status === "Running" && start !== null) dur = { text: fmtSecs((nowMs - start) / 1000), color: "var(--blue)" };
          else if (!dur && gated && span?.waitSecs != null) dur = { text: fmtSecs(span.waitSecs), color: "var(--yellow)" };
          else if (!dur && n.status === "Skipped") dur = { text: "skipped", color: "var(--fg3)" };
          else if (!dur && n.status === "Queued") dur = { text: "queued", color: "var(--fg3)" };

          const ghost = n.job !== null && n.step === null && !n.group ? ghosts.get(ghostKey(n.stage, n.job)) : undefined;
          const jobName = n.job !== null ? run.stages[n.stage]?.jobs[n.job]?.name : undefined;
          const pastUsual =
            !ghost && n.step === null && !n.group && n.status === "Running" && jobName !== undefined && !!estimates?.jobs.has(jobName);

          let bar: React.ReactNode = null;
          if (lane && start !== null && end !== null) {
            const l = pct(start);
            lastStartPct = Math.max(lastStartPct, l);
            bar = (
              <span
                className="absolute"
                style={{ top: 0, bottom: 0, borderRadius: 2, left: `${l}%`, width: `${Math.max(0.4, pct(end) - l)}%`, background: m.color, opacity: n.group ? 0.55 : 1 }}
              />
            );
          } else if (lane && gated && span?.last != null) {
            const l = pct(span.last);
            lastStartPct = Math.max(lastStartPct, l);
            bar = <span className="absolute" style={{ top: 0, bottom: 0, borderRadius: 2, left: `${l}%`, width: `${Math.max(0.4, pct(nowMs) - l)}%`, ...WAIT }} />;
          } else if (lane && n.note) {
            bar = (
              <span className="absolute text-[12px] whitespace-nowrap leading-none" style={{ left: `${lastStartPct}%`, top: -2, color: "var(--fg3)" }}>
                {n.note}
              </span>
            );
          }

          const toggle = () => n.key && onToggle(n.key, !n.expanded);
          const click = () => (n.jobId && !n.group ? onSelect(n) : toggle());
          const aside = n.summary ? (
            <span className="text-[12px] truncate shrink-0 max-w-[45%]" style={{ color: toneColor(n.summary.tone) }}>
              {n.summary.text}
            </span>
          ) : n.problem ? (
            <span className="text-[12px] truncate" style={{ color: "var(--red)" }}>
              {n.problem}
            </span>
          ) : !lane && n.note ? (
            <span className="text-[12px] truncate" style={{ color: "var(--fg3)" }}>
              {n.note}
            </span>
          ) : null;

          return (
            <div
              key={id}
              className="flex items-center gap-2 px-2 hover:bg-[var(--hover)]"
              style={{ height: 28, borderRadius: 5, background: selected ? "var(--active)" : undefined }}
              data-selected={selected || undefined}
            >
              <span
                className="flex items-center gap-2 min-w-0"
                style={{ paddingLeft: n.depth * INDENT, flex: lane ? `0 0 ${LABEL_W + LEAD_W}px` : "1 1 auto", width: lane ? LABEL_W + LEAD_W : undefined }}
              >
                {n.key ? (
                  <button
                    type="button"
                    onClick={toggle}
                    aria-expanded={n.expanded}
                    aria-label={`${n.expanded ? "Collapse" : "Expand"} ${n.label}`}
                    className="inline-flex items-center justify-center shrink-0 p-0 border-0 bg-transparent cursor-pointer"
                    style={{ width: 14, color: "var(--fg3)" }}
                  >
                    <Icon name={n.expanded ? "chevron-down" : "chevron-right"} size={12} />
                  </button>
                ) : (
                  <span className="shrink-0" style={{ width: 14 }} aria-hidden="true" />
                )}
                <button
                  type="button"
                  onClick={click}
                  className="flex items-center gap-2 min-w-0 flex-1 text-left p-0 border-0 bg-transparent cursor-pointer text-[12.5px]"
                  style={{ font: "inherit", fontSize: 12.5 }}
                  aria-current={selected ? "true" : undefined}
                  title={n.jobId && !n.group ? `${n.label} — open its log` : n.label}
                >
                  <StatusGlyph glyph={m.glyph} color={m.color} spin={m.running} />
                  <span
                    className="truncate"
                    style={{ color: labelColor, fontWeight: n.status === "Failed" || gated || isStage ? 500 : undefined }}
                  >
                    {n.label}
                  </span>
                  {aside}
                </button>
              </span>
              <span className="mono num text-right shrink-0" style={{ flex: `0 0 ${DUR_W}px`, color: dur?.color ?? "var(--fg3)" }}>
                {dur?.text ?? ""}
              </span>
              {lane && (
                <div className="relative flex-1" style={{ height: 8, borderRadius: 2, background: "var(--hover)" }} aria-hidden="true">
                  {bar}
                  {ghost && (
                    <span
                      className="absolute"
                      style={{ top: 0, bottom: 0, borderRadius: 2, left: `${pct(ghost.start)}%`, width: `${Math.max(0.4, pct(ghost.end) - pct(ghost.start))}%`, ...HATCH }}
                    />
                  )}
                  {nowPct !== null && !(n.status === "Running" && start !== null) && (
                    <span className="absolute" style={{ top: -3, height: 14, left: `${nowPct}%`, borderLeft: "1px dashed var(--fg2)", opacity: 0.6 }} />
                  )}
                </div>
              )}
              {showGhostCol && (
                <span
                  className="mono num whitespace-nowrap shrink-0"
                  style={{ flex: `0 0 ${GHOST_W}px`, color: ghost?.left != null && ghost.left === longestLeft ? "var(--yellow)" : "var(--fg3)" }}
                >
                  {ghost?.left != null ? `~${axisLabel(ghost.left)}` : pastUsual ? "past usual" : ""}
                </span>
              )}
            </div>
          );
        })
      )}
      {footer && (
        <div className="flex items-center gap-1.5 text-[12px] px-2 pt-2 mt-1 flex-wrap" style={{ color: "var(--fg3)", borderTop: "1px solid var(--border)" }}>
          {footer}
        </div>
      )}
    </div>
  );
}
