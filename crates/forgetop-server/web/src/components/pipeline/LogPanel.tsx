// The run pane's log panel: one job's log, folded into its steps' sections the way the TUI's
// LogView does (../../pipeline/runlog), with first-error jump, search, follow and wrap.
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { usePipelineLogs } from "../../api";
import {
  firstErrorLine,
  initialSection,
  isErrorLine,
  layoutLog,
  logLines,
  parseStepSections,
  sectionOfLine,
  stripAnsi,
  stripTimestamp,
  type LogSection,
} from "../../pipeline/runlog";
import type { PipeRef, PipelineJob } from "../../types";
import { PaneButton, PaneLink, MONO_DIM } from "./parts";

const ROW_PX = 20;
const fmtN = (n: number) => n.toLocaleString("en-US");
const GUTTER = /^(\d\d:\d\d:\d\d)(\s+)([\s\S]*)$/;

/** The log text a row shows: a sectioned log drops each line's ISO timestamp (as the TUI does);
 *  ANSI escapes never reach the browser. */
const displayText = (raw: string, sectioned: boolean) => stripAnsi(sectioned ? stripTimestamp(raw) : raw);

function highlight(text: string, query: string): ReactNode {
  if (!query) return text;
  const lower = text.toLowerCase();
  const q = query.toLowerCase();
  const out: ReactNode[] = [];
  let from = 0;
  for (;;) {
    const i = lower.indexOf(q, from);
    if (i < 0) break;
    if (i > from) out.push(text.slice(from, i));
    out.push(
      <mark key={i} style={{ background: "var(--yellow)", color: "var(--bg)", fontWeight: 700 }}>
        {text.slice(i, i + q.length)}
      </mark>,
    );
    from = i + q.length;
  }
  if (out.length === 0) return text;
  out.push(text.slice(from));
  return out;
}

export interface LogPanelProps {
  pipeRef: PipeRef;
  jobId: string;
  job: PipelineJob | undefined;
  /** The step the panel was opened for, when a step row was clicked. */
  step: number | null;
  live: boolean;
  /** Bumped by the pane to ask for a jump to the first error (the failure line's button). */
  jumpToken: number;
  onClose: () => void;
}

export const LogPanel = memo(function LogPanel({ pipeRef, jobId, job, step, live, jumpToken, onClose }: LogPanelProps) {
  const q = usePipelineLogs(pipeRef, jobId, live);
  const steps = useMemo(() => job?.steps.map((s) => s.name) ?? [], [job]);
  const stepsKey = steps.join("\u0001");
  const stepPassed = useMemo(() => job?.steps.map((s) => s.status === "Succeeded") ?? [], [job]);
  // The step to land on: the one asked for, else the job's failed step (the TUI's `target_step`).
  const targetStep = step ?? (job ? job.steps.findIndex((s) => s.status === "Failed") : -1);

  const parsed = useMemo(() => {
    if (q.data === undefined) return null;
    const { lines, dropped } = logLines(q.data);
    const sections = parseStepSections(lines, steps);
    return { lines, dropped, sections, firstError: firstErrorLine(lines) };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q.data, stepsKey]);

  // Open sections, by absolute start line (start + dropped) so they survive a live re-fetch whose
  // capped head moved.
  const [openAbs, setOpenAbs] = useState<Set<number>>(new Set());
  const foldsReady = useRef(false);
  const lastAbsStart = useRef(-1);
  const absStart = (s: LogSection) => s.start + (parsed?.dropped ?? 0);

  const [follow, setFollow] = useState(live);
  const [wrap, setWrap] = useState(false);
  const [query, setQuery] = useState("");
  const [matchAt, setMatchAt] = useState(0);
  const [scrollTo, setScrollTo] = useState<number | "bottom" | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const handledJump = useRef(0);

  // First sectioned parse: open only the landing section; later polls of a live job open the
  // sections that appeared since (the TUI rule).
  useEffect(() => {
    if (!parsed || parsed.sections.length === 0) return;
    const { sections, dropped, firstError } = parsed;
    const maxStart = Math.max(...sections.map((s) => s.start + dropped));
    if (!foldsReady.current) {
      foldsReady.current = true;
      const land = initialSection({ sections, steps, targetStep: targetStep >= 0 ? targetStep : null, firstError, live });
      setOpenAbs(new Set(land !== null ? [sections[land].start + dropped] : []));
      // Not following: land on the first error inside the opened section.
      if (!live && firstError !== null && land !== null && sectionOfLine(sections, firstError) === land) setScrollTo(firstError);
    } else if (live) {
      const fresh = sections.filter((s) => s.start + dropped > lastAbsStart.current).map((s) => s.start + dropped);
      if (fresh.length) setOpenAbs((o) => new Set([...o, ...fresh]));
    }
    lastAbsStart.current = maxStart;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [parsed]);

  const open = useMemo(() => {
    const set = new Set<number>();
    parsed?.sections.forEach((s, i) => openAbs.has(absStart(s)) && set.add(i));
    return set;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [parsed, openAbs]);

  const rows = useMemo(
    () => (parsed ? layoutLog({ lines: parsed.lines, sections: parsed.sections, open, stepPassed }) : []),
    [parsed, open, stepPassed],
  );

  const sectioned = (parsed?.sections.length ?? 0) > 0;
  const matches = useMemo(() => {
    if (!parsed || !query) return [];
    const ql = query.toLowerCase();
    const out: number[] = [];
    parsed.lines.forEach((l, i) => displayText(l, sectioned).toLowerCase().includes(ql) && out.push(i));
    return out;
  }, [parsed, query, sectioned]);
  const current = matches.length ? matches[Math.min(matchAt, matches.length - 1)] : null;

  /** Unfold the section holding `line` (if folded) and scroll to it. */
  const reveal = useCallback(
    (line: number) => {
      if (!parsed) return;
      const s = sectionOfLine(parsed.sections, line);
      if (s !== null) {
        const abs = parsed.sections[s].start + parsed.dropped;
        setOpenAbs((o) => (o.has(abs) ? o : new Set([...o, abs])));
      }
      setFollow(false);
      setScrollTo(line);
    },
    [parsed],
  );

  // The pane's "Jump to error".
  useEffect(() => {
    if (jumpToken > handledJump.current && parsed) {
      handledJump.current = jumpToken;
      if (parsed.firstError !== null) reveal(parsed.firstError);
    }
  }, [jumpToken, parsed, reveal]);

  // Follow: stay on the tail as a live log grows.
  useEffect(() => {
    if (follow && parsed) setScrollTo("bottom");
  }, [follow, parsed]);

  useLayoutEffect(() => {
    const body = bodyRef.current;
    if (scrollTo === null || !body) return;
    if (scrollTo === "bottom") body.scrollTop = body.scrollHeight;
    else {
      const el = body.querySelector<HTMLElement>(`[data-line="${scrollTo}"]`);
      if (el) body.scrollTop = Math.max(0, el.offsetTop - body.offsetTop - 2 * ROW_PX);
    }
    setScrollTo(null);
  }, [scrollTo, rows]);

  const onScroll = () => {
    const body = bodyRef.current;
    if (body && follow && body.scrollTop + body.clientHeight < body.scrollHeight - 8) setFollow(false);
  };

  const step_ = (dir: 1 | -1) => {
    if (!matches.length) return;
    const next = (Math.min(matchAt, matches.length - 1) + dir + matches.length) % matches.length;
    setMatchAt(next);
    reveal(matches[next]);
  };

  const toggleSections = (idx: number[]) => {
    if (!parsed) return;
    const abs = idx.map((i) => parsed.sections[i].start + parsed.dropped);
    setOpenAbs((o) => {
      const n = new Set(o);
      const opening = !abs.every((a) => n.has(a));
      abs.forEach((a) => (opening ? n.add(a) : n.delete(a)));
      return n;
    });
  };

  const title = step !== null && job?.steps[step] ? job.steps[step].name : (job?.name ?? "Job");
  const total = parsed ? parsed.lines.length + parsed.dropped : 0;
  const lineNo = (l: number) => l + 1 + (parsed?.dropped ?? 0);
  const sectionName = (s: LogSection) => (s.step !== null && steps[s.step] ? steps[s.step] : s.name);

  return (
    <section className="rounded-lg flex flex-col overflow-hidden" style={{ background: "var(--panel)", border: "1px solid var(--border)" }} aria-label={`Logs · ${title}`}>
      <div className="flex items-center gap-2.5 px-3 py-2 flex-wrap" style={{ borderBottom: "1px solid var(--border)" }}>
        <span className="text-[11px] font-semibold uppercase tracking-wider" style={{ color: "var(--dim)" }}>
          Logs · {title}
        </span>
        {parsed && (
          <span className="text-xs" style={{ color: "var(--dim)" }}>
            {fmtN(total)} lines
            {parsed.dropped > 0 && ` (showing last ${fmtN(parsed.lines.length)} lines)`}
          </span>
        )}
        {parsed?.firstError != null && (
          <button type="button" className="text-xs" style={{ color: "var(--red)" }} onClick={() => reveal(parsed.firstError!)} title="Scroll to the first error">
            first error · line {fmtN(lineNo(parsed.firstError))}
          </button>
        )}
        <span className="flex-1" />
        <label className="inline-flex items-center gap-1.5 rounded-md px-2 py-0.5 text-xs" style={{ background: "var(--bg)", border: "1px solid var(--border)" }}>
          <span style={{ color: "var(--dim)" }} aria-hidden="true">
            /
          </span>
          <input
            type="search"
            value={query}
            placeholder="search"
            aria-label="Search log"
            onChange={(e) => {
              setQuery(e.target.value);
              setMatchAt(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                step_(e.shiftKey ? -1 : 1);
              }
              if (e.key === "Escape" && query) {
                e.stopPropagation();
                setQuery("");
              }
            }}
            className="mono bg-transparent outline-none text-xs"
            style={{ width: 120, color: "var(--yellow)", border: 0 }}
          />
          {query && (
            <span style={{ color: "var(--dim)" }}>
              {matches.length ? `${Math.min(matchAt, matches.length - 1) + 1} of ${matches.length}` : "0 of 0"}
            </span>
          )}
        </label>
        <PaneButton label="↑" ariaLabel="Previous match" neutral disabled={!matches.length} onClick={() => step_(-1)} />
        <PaneButton label="↓" ariaLabel="Next match" neutral disabled={!matches.length} onClick={() => step_(1)} />
        <PaneButton
          label="Follow"
          neutral
          pressed={follow}
          disabled={!live}
          title={live ? "Keep the newest lines in view" : "The job has finished"}
          onClick={() => setFollow((f) => !f)}
        />
        <PaneButton label="Wrap" neutral pressed={wrap} onClick={() => setWrap((w) => !w)} />
        {job?.url && <PaneLink href={job.url}>Open job ↗</PaneLink>}
        <button type="button" aria-label="Close logs" onClick={onClose} className="text-sm px-1 leading-none" style={{ color: "var(--dim)" }}>
          ✕
        </button>
      </div>
      <div
        ref={bodyRef}
        onScroll={onScroll}
        className="mono text-xs py-1.5"
        style={{ maxHeight: 420, overflow: "auto", lineHeight: `${ROW_PX}px` }}
        data-testid="log-body"
      >
        {q.isLoading && <div className="px-3" style={{ color: "var(--dim)" }}>Loading logs…</div>}
        {q.error && <div className="px-3" style={{ color: "var(--red)" }}>Couldn't load logs.</div>}
        {parsed &&
          rows.map((r) => {
            if (r.kind === "group") {
              const first = parsed.sections[r.first];
              const last = parsed.sections[r.last];
              const n = r.last - r.first + 1;
              const count = last.end - first.start;
              const idx = Array.from({ length: n }, (_, k) => r.first + k);
              return (
                <button
                  key={`g${r.first}`}
                  type="button"
                  aria-expanded={false}
                  onClick={() => toggleSections(idx)}
                  className="flex w-full items-center gap-2 px-3 text-left"
                  style={{ color: "var(--dim)" }}
                >
                  <span aria-hidden="true">▸</span>
                  <span className="truncate">
                    {n} steps · {sectionName(first)} … {sectionName(last)}
                  </span>
                  <span className="flex-1" />
                  <span className="shrink-0">{fmtN(count)} lines</span>
                </button>
              );
            }
            if (r.kind === "header") {
              const s = parsed.sections[r.section];
              const isOpen = open.has(r.section);
              return (
                <button
                  key={`h${r.section}`}
                  type="button"
                  aria-expanded={isOpen}
                  onClick={() => toggleSections([r.section])}
                  className="flex w-full items-center gap-2 px-3 text-left"
                  style={{ color: isOpen ? "var(--accent)" : "var(--fg)", fontWeight: isOpen ? 700 : undefined }}
                >
                  <span style={{ color: "var(--dim)", fontWeight: 400 }} aria-hidden="true">
                    {isOpen ? "▾" : "▸"}
                  </span>
                  <span className="truncate">{sectionName(s)}</span>
                  <span className="flex-1" />
                  <span className="shrink-0" style={{ color: "var(--dim)", fontWeight: 400 }}>
                    {isOpen ? `line ${fmtN(lineNo(s.start))}` : `${fmtN(Math.max(0, s.end - s.start - 1))} lines`}
                  </span>
                </button>
              );
            }
            const raw = parsed.lines[r.line];
            const text = displayText(raw, sectioned);
            const isErr = isErrorLine(raw);
            const isFirst = r.line === parsed.firstError;
            const isCur = r.line === current;
            const g = GUTTER.exec(text);
            return (
              <div
                key={`l${r.line}`}
                data-line={r.line}
                className="flex gap-3 px-3"
                style={{
                  whiteSpace: wrap ? "pre-wrap" : "pre",
                  overflowWrap: wrap ? "anywhere" : undefined,
                  color: isErr ? "var(--red)" : "var(--fg)",
                  background: isCur ? "var(--sel)" : undefined,
                  minWidth: wrap ? undefined : "max-content",
                }}
              >
                <span className="shrink-0 font-bold" style={{ width: 10, color: "var(--red)" }} aria-label={isFirst ? "first error" : undefined}>
                  {isFirst ? "▶" : ""}
                </span>
                <span className="shrink-0 text-right select-none" style={{ width: 44, color: MONO_DIM }}>
                  {lineNo(r.line)}
                </span>
                {g ? (
                  <>
                    <span className="shrink-0" style={{ color: "var(--dim)" }}>
                      {g[1]}
                    </span>
                    <span className="min-w-0">{highlight(g[3], query)}</span>
                  </>
                ) : (
                  <span className="min-w-0">{highlight(text, query)}</span>
                )}
              </div>
            );
          })}
      </div>
    </section>
  );
});
