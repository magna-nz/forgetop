// The run pane's tree + timeline: `flatten`'s rows (../../pipeline/timeline) with a duration
// column and a bar lane on one shared time axis — bars for what ran, a now line while live, the
// gated stage's wait, and estimate "ghosts" for what is still to run. Below ~720px the lane is
// dropped and the plain tree remains.
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
  treeTitle,
  type Estimates,
  type FlatNode,
  type Ghost,
} from "../../pipeline/timeline";
import type { PipelineRun, PipelineRunStatus } from "../../types";
import { Card } from "./parts";

const LABEL_W = 340;
const DUR_W = 56;
const GHOST_W = 64;
/** Below this card width the bar lane is dropped. */
const NARROW_PX = 720;

/** A stable identity for a row across re-flattens (indices move as folds change). */
export const nodeId = (n: FlatNode) =>
  `${n.stage}|${n.job ?? ""}|${n.step ?? ""}|${n.group ? (n.key ?? "") : ""}|${n.info ? n.label : ""}`;

const HATCH: CSSProperties = {
  border: "1px dashed var(--dim)",
  background: "repeating-linear-gradient(45deg, color-mix(in srgb, var(--dim) 35%, transparent) 0 3px, transparent 3px 6px)",
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

function countLabel(run: PipelineRun): string {
  if (run.stages.length === 1 && run.stages[0].jobs.length === 1 && run.stages[0].jobs[0].steps.length > 0) {
    const n = run.stages[0].jobs[0].steps.length;
    return `${n} ${n === 1 ? "step" : "steps"}`;
  }
  const n = run.stages.reduce((acc, s) => acc + s.jobs.length, 0);
  return `${n} ${n === 1 ? "job" : "jobs"}`;
}

const toneColor = (t: "neutral" | "warn" | "bad") => (t === "bad" ? "var(--red)" : t === "warn" ? "var(--yellow)" : "var(--dim)");

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

  const lanePx = Number.isFinite(width) ? width - LABEL_W - DUR_W - (showGhostCol ? GHOST_W : 0) - 60 : 560;
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
              <span className="inline-block rounded-sm" style={{ width: 18, height: 8, ...HATCH }} />
              estimate, from the last {estimates.jobRuns} passing {estimates.jobRuns === 1 ? "run" : "runs"}
            </span>
          )}
        </>
      );
  }

  // Notes for rows with no span sit where the run's work had got to: the latest bar start above them.
  let lastStartPct = 0;

  return (
    <div ref={ref}>
      <Card title={treeTitle(run)} right={countLabel(run)}>
        {nodes.length === 0 ? (
          <div className="text-sm py-1" style={{ color: "var(--dim)" }}>
            No stages reported for this run.
          </div>
        ) : (
          <>
            {lane && (
              <div className="flex items-center gap-2.5" style={{ height: 16 }} aria-hidden="true">
                <span style={{ width: LABEL_W + 24, flex: "0 0 auto" }} />
                <span style={{ width: DUR_W, flex: "0 0 auto" }} />
                <div className="mono relative flex-1 text-[11px]" style={{ height: 16, color: "var(--dim)" }}>
                  {ticks
                    .filter((t) => nowPct === null || Math.abs(t.pos / 10 - nowPct) > 7)
                    .filter((t) => !squeezed || t.pos / 10 < 62)
                    .map((t) => (
                      <span key={t.secs} className="absolute whitespace-nowrap" style={{ left: `${t.pos / 10}%`, transform: t.pos / 10 > 92 ? "translateX(-100%)" : undefined }}>
                        {t.label}
                      </span>
                    ))}
                  {squeezed && span?.waitSecs != null && (
                    <span className="absolute whitespace-nowrap" style={{ left: `${pct(span.last!) + 2}%`, color: "var(--yellow)" }}>
                      ╌ waited {fmtSecs(span.waitSecs)} ╌
                    </span>
                  )}
                  {nowPct !== null && (
                    <span
                      className="absolute font-bold"
                      style={{ left: `${nowPct}%`, color: "var(--accent)", transform: nowPct > 92 ? "translateX(-100%)" : undefined }}
                    >
                      now
                    </span>
                  )}
                </div>
                {showGhostCol && <span style={{ width: GHOST_W, flex: "0 0 auto" }} />}
              </div>
            )}

            <div className="flex flex-col gap-0.5">
              {nodes.map((n) => {
                const id = nodeId(n);
                if (n.info) {
                  const link = n.label.startsWith("o  ");
                  return (
                    <div key={id} className="text-xs py-0.5" style={{ color: "var(--dim)", paddingLeft: 30 + n.depth * 18 }}>
                      {link ? (
                        run.url ? (
                          <a href={run.url} target="_blank" rel="noreferrer" style={{ color: "var(--accent)" }}>
                            open in {providerLabel} to approve ↗
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
                const labelColor = n.status === "Failed" ? "var(--red)" : gated ? "var(--yellow)" : dimLabel ? "var(--dim)" : "var(--fg)";
                const start = msOf(n.startedAt);
                const fin = msOf(n.finishedAt);
                const end = fin ?? (start !== null && isActive(n.status) && !gated ? nowMs : null);

                let dur: { text: string; color: string } | null = n.duration ? { text: n.duration, color: "var(--dim)" } : null;
                if (!dur && n.status === "Running" && start !== null) dur = { text: fmtSecs((nowMs - start) / 1000), color: "var(--accent)" };
                else if (!dur && gated && span?.waitSecs != null) dur = { text: fmtSecs(span.waitSecs), color: "var(--yellow)" };
                else if (!dur && n.status === "Skipped") dur = { text: "skipped", color: "var(--dim)" };
                else if (!dur && n.status === "Queued") dur = { text: "queued", color: "var(--dim)" };

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
                      className="absolute rounded-sm"
                      style={{ top: 0, height: 10, left: `${l}%`, width: `${Math.max(0.4, pct(end) - l)}%`, background: m.color, opacity: n.group ? 0.55 : 1 }}
                    />
                  );
                } else if (lane && gated && span?.last != null) {
                  const l = pct(span.last);
                  lastStartPct = Math.max(lastStartPct, l);
                  bar = <span className="absolute rounded-sm" style={{ top: 1, height: 8, left: `${l}%`, width: `${Math.max(0.4, pct(nowMs) - l)}%`, ...WAIT }} />;
                } else if (lane && n.note) {
                  bar = (
                    <span className="absolute text-[11px] whitespace-nowrap" style={{ left: `${lastStartPct}%`, top: -3, color: "var(--dim)" }}>
                      {n.note}
                    </span>
                  );
                }

                const toggle = () => n.key && onToggle(n.key, !n.expanded);
                const click = () => (n.jobId && !n.group ? onSelect(n) : toggle());

                return (
                  <div
                    key={id}
                    className="flex items-center gap-2.5 rounded"
                    style={{
                      background: selected ? "var(--sel)" : "transparent",
                      borderLeft: `3px solid ${selected ? (n.status === "Failed" ? "var(--red)" : gated ? "var(--yellow)" : "var(--accent)") : "transparent"}`,
                      padding: "3px 4px 3px 1px",
                    }}
                  >
                    <span
                      className="flex items-center gap-2 text-[13px] min-w-0"
                      style={{ width: lane ? LABEL_W + 24 - n.depth * 18 : undefined, marginLeft: n.depth * 18, flex: lane ? "0 0 auto" : "1 1 auto" }}
                    >
                      {n.key ? (
                        <button
                          type="button"
                          onClick={toggle}
                          aria-expanded={n.expanded}
                          aria-label={`${n.expanded ? "Collapse" : "Expand"} ${n.label}`}
                          className="text-[11px] w-3 shrink-0 text-center"
                          style={{ color: "var(--dim)" }}
                        >
                          {n.expanded ? "▾" : "▸"}
                        </button>
                      ) : (
                        <span className="text-[11px] w-3 shrink-0 text-center" style={{ color: "var(--dim)" }} aria-hidden="true">
                          ·
                        </span>
                      )}
                      <button
                        type="button"
                        onClick={click}
                        className="flex items-center gap-2 min-w-0 flex-1 text-left"
                        aria-current={selected ? "true" : undefined}
                        title={n.jobId && !n.group ? `${n.label} — open its log` : n.label}
                      >
                        <span className={`shrink-0${m.running ? " spin" : ""}`} style={{ color: m.color }} aria-hidden="true">
                          {m.icon}
                        </span>
                        <span
                          className="truncate"
                          style={{ color: labelColor, fontWeight: n.status === "Failed" || gated ? 600 : isStage ? 600 : undefined }}
                        >
                          {n.label}
                        </span>
                        {n.summary ? (
                          <span className="text-xs truncate shrink-0" style={{ color: toneColor(n.summary.tone) }}>
                            {n.summary.text}
                          </span>
                        ) : n.problem ? (
                          <span className="text-xs truncate" style={{ color: "var(--red)" }}>
                            {n.problem}
                          </span>
                        ) : null}
                        {!lane && n.note && (
                          <span className="text-xs truncate" style={{ color: "var(--dim)" }}>
                            {n.note}
                          </span>
                        )}
                      </button>
                    </span>
                    <span className="mono text-xs text-right" style={{ width: DUR_W, flex: "0 0 auto", color: dur?.color }}>
                      {dur?.text ?? ""}
                    </span>
                    {lane && (
                      <div className="relative flex-1" style={{ height: 10 }} aria-hidden="true">
                        {bar}
                        {ghost && (
                          <span
                            className="absolute rounded-sm"
                            style={{ top: 1, height: 8, left: `${pct(ghost.start)}%`, width: `${Math.max(0.4, pct(ghost.end) - pct(ghost.start))}%`, ...HATCH }}
                          />
                        )}
                        {nowPct !== null && !(n.status === "Running" && start !== null) && (
                          <span
                            className="absolute"
                            style={{ top: -2, height: 14, left: `${nowPct}%`, borderLeft: "1px dashed var(--accent)", opacity: 0.6 }}
                          />
                        )}
                      </div>
                    )}
                    {showGhostCol && (
                      <span
                        className="mono text-xs whitespace-nowrap"
                        style={{ width: GHOST_W, flex: "0 0 auto", color: ghost?.left != null && ghost.left === longestLeft ? "var(--yellow)" : "var(--dim)" }}
                      >
                        {ghost?.left != null ? `~${axisLabel(ghost.left)}` : pastUsual ? "past usual" : ""}
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          </>
        )}
        {footer && (
          <div className="flex items-center gap-1.5 text-xs pt-1.5 mt-1 flex-wrap" style={{ color: "var(--dim)", borderTop: "1px solid var(--border)" }}>
            {footer}
          </div>
        )}
      </Card>
    </div>
  );
}
