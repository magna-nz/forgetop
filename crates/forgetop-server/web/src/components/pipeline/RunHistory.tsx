// The run pane's history line: recent runs of this pipeline (on this branch) as a strip of small
// status bars, and the last failure. Built from `pipelineHistory` (../../pipeline/history). The
// median and this run's delta against it sit in the pane header (`historyDelta`).
import { fmtSecs, pipeMeta, relativeTime } from "../../format";
import type { RunHistory } from "../../pipeline/history";
import { isActive } from "../../pipeline/grouping";
import { statusBarColor } from "./parts";

const num = (n: number | null) => (n != null ? `#${n}` : "run");

/** This run against the median, once it has finished: `+12%` slower (amber), `−8%` faster. */
export function historyDelta(history: RunHistory): { text: string; color: string } | null {
  const cur = history.current != null ? history.entries[history.current] : null;
  if (!cur || cur.secs === null || !history.medianSecs || isActive(cur.status)) return null;
  const pct = Math.round(((cur.secs - history.medianSecs) / history.medianSecs) * 100);
  return pct > 0
    ? { text: `+${pct}%`, color: "var(--yellow)" }
    : pct < 0
      ? { text: `−${Math.abs(pct)}%`, color: "var(--green)" }
      : { text: "±0%", color: "var(--fg3)" };
}

export function HistoryStrip({ history, onOpen }: { history: RunHistory; onOpen: (row: number) => void }) {
  const { entries, current } = history;
  const first = entries[0];
  const last = entries[entries.length - 1];
  const lf = history.lastFailure;
  const lfAge = lf ? relativeTime(lf.at) : "";
  const range = first === last ? num(first.number) : `${num(first.number)} … ${num(last.number)}`;
  const scope = `${history.name}${history.allBranches ? "" : " on this branch"} · ${history.total} ${history.total === 1 ? "run" : "runs"} (${range})`;

  return (
    <span className="inline-flex items-center gap-2 text-[12px] min-w-0" style={{ color: "var(--fg3)" }}>
      <span title={scope}>History:</span>
      <span className="inline-flex items-center gap-[2px]" role="group" aria-label="Recent runs, oldest to newest">
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
              className="block p-0 border-0"
              style={{
                width: 5,
                height: 10,
                borderRadius: 1.5,
                background: statusBarColor(e.status),
                outline: isCur ? "1px solid var(--fg2)" : undefined,
                outlineOffset: isCur ? 1 : undefined,
                cursor: isCur ? "default" : "pointer",
              }}
            />
          );
        })}
      </span>
      {lf ? (
        <span className="truncate">
          last failure <span className="mono">{num(lf.number)}</span>
          {lfAge && ` · ${lfAge}`}
          {lf.step && ` · in ${lf.step}`}
        </span>
      ) : (
        <span className="truncate">no failures in the last {history.total}</span>
      )}
    </span>
  );
}
