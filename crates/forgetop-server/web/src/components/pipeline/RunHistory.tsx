// The run pane's history strip: recent runs of this pipeline (on this branch), a duration
// sparkline, the median and the last failure. Built from `pipelineHistory` (../../pipeline/history).
import { fmtSecs, pipeMeta, relativeTime } from "../../format";
import type { RunHistory } from "../../pipeline/history";
import { isActive } from "../../pipeline/grouping";
import { Card, MONO_DIM } from "./parts";

const num = (n: number | null) => (n != null ? `#${n}` : "run");

export function HistoryStrip({ history, onOpen }: { history: RunHistory; onOpen: (row: number) => void }) {
  const { entries, current } = history;
  const cur = current != null ? entries[current] : null;
  const secs = entries.map((e) => e.secs).filter((s): s is number => s !== null);
  const lo = secs.length ? Math.min(...secs) : 0;
  const hi = secs.length ? Math.max(...secs) : 0;
  const barH = (s: number | null) => (s === null ? 2 : hi > lo ? 4 + Math.round(((s - lo) / (hi - lo)) * 16) : 20);

  let delta: { text: string; color: string } | null = null;
  if (cur && cur.secs !== null && history.medianSecs && !isActive(cur.status)) {
    const pct = Math.round(((cur.secs - history.medianSecs) / history.medianSecs) * 100);
    delta =
      pct > 0
        ? { text: `+${pct}%`, color: "var(--yellow)" }
        : pct < 0
          ? { text: `−${Math.abs(pct)}%`, color: "var(--green)" }
          : { text: "±0%", color: "var(--dim)" };
  }
  const first = entries[0];
  const last = entries[entries.length - 1];
  const lf = history.lastFailure;
  const lfAge = lf ? relativeTime(lf.at) : "";

  return (
    <Card
      title={`History · ${history.name}${history.allBranches ? "" : " on this branch"}`}
      right={`${history.total} ${history.total === 1 ? "run" : "runs"}`}
      className="gap-2"
    >
      <div className="flex items-end gap-5 flex-wrap">
        <div className="flex items-end gap-1.5" role="group" aria-label="Recent runs, oldest to newest">
          {entries.map((e, i) => {
            const m = pipeMeta(e.status);
            const isCur = i === current;
            return (
              <button
                key={e.runId}
                type="button"
                aria-current={isCur ? "true" : undefined}
                title={`${num(e.number)} · ${e.secs !== null ? fmtSecs(e.secs) : m.label}`}
                aria-label={`${num(e.number)} ${m.label}${e.secs !== null ? ` in ${fmtSecs(e.secs)}` : ""}`}
                onClick={isCur ? undefined : () => onOpen(e.row)}
                className="inline-flex items-center justify-center rounded text-xs"
                style={{
                  width: 22,
                  height: 22,
                  color: isCur ? "var(--bg)" : m.color,
                  fontWeight: isCur ? 700 : undefined,
                  background: isCur ? m.color : `color-mix(in srgb, ${m.color} 14%, transparent)`,
                  border: `1px solid ${isCur ? m.color : `color-mix(in srgb, ${m.color} 30%, transparent)`}`,
                  boxShadow: isCur ? `0 0 0 2px color-mix(in srgb, ${m.color} 30%, transparent)` : undefined,
                  cursor: isCur ? "default" : "pointer",
                }}
              >
                <span className={isCur && m.running ? "spin" : undefined} aria-hidden="true">
                  {m.icon}
                </span>
              </button>
            );
          })}
        </div>
        <div className="flex items-end gap-[3px]" style={{ height: 22 }} aria-hidden="true" title="durations, oldest to newest">
          {entries.map((e, i) => {
            const isCur = i === current;
            const color = e.status === "Failed" ? "var(--red)" : isCur ? pipeMeta(e.status).color : MONO_DIM;
            return (
              <span
                key={e.runId}
                className="rounded-[1px]"
                style={{ width: 8, height: barH(e.secs), background: color, opacity: isCur && isActive(e.status) ? 0.7 : 1 }}
              />
            );
          })}
        </div>
        {history.medianSecs !== null && (
          <div className="text-xs pb-[3px]" style={{ color: "var(--dim)" }}>
            median <span style={{ color: "var(--fg)" }}>{fmtSecs(history.medianSecs)}</span>
            {delta && (
              <>
                {" "}
                · this run <span style={{ color: delta.color }}>{delta.text}</span>
              </>
            )}
          </div>
        )}
      </div>
      <div className="flex items-center gap-3.5 text-xs flex-wrap" style={{ color: "var(--dim)" }}>
        <span className="mono">
          {first === last ? (
            num(first.number)
          ) : (
            <>
              {num(first.number)} … <span style={{ color: current === entries.length - 1 ? "var(--fg)" : undefined }}>{num(last.number)}</span>
            </>
          )}
        </span>
        {lf ? (
          <span>
            last failure <span style={{ color: "var(--red)" }}>{num(lf.number)}</span>
            {lfAge && ` · ${lfAge}`}
            {lf.step && ` · in ${lf.step}`}
          </span>
        ) : (
          <span>no failures in the last {history.total}</span>
        )}
      </div>
    </Card>
  );
}
