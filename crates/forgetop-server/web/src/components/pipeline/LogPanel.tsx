// The run pane's log panel: one job's log, folded into its steps' sections the way the TUI's
// LogView does (../../pipeline/runlog), with first-error jump, search, follow, wrap and raw (the
// log as fetched: no step sections, timestamps kept).
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
import { Icon } from "../icons";
import { Button, IconButton } from "../ui";
import { PaneLink, MONO_DIM } from "./parts";

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
  const [raw, setRaw] = useState(false);
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
    () =>
      !parsed
        ? []
        : raw
          ? parsed.lines.map((_, line) => ({ kind: "line" as const, line }))
          : layoutLog({ lines: parsed.lines, sections: parsed.sections, open, stepPassed }),
    [parsed, open, stepPassed, raw],
  );

  // Raw shows every line as fetched; otherwise a sectioned log drops its timestamps.
  const sectioned = !raw && (parsed?.sections.length ?? 0) > 0;
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
    <section
      className="flex flex-col overflow-hidden"
      style={{ borderRadius: 6, background: "var(--panel)", border: "1px solid var(--border)" }}
      aria-label={`Logs · ${title}`}
    >
      <div className="flex items-center gap-2 px-2.5 flex-wrap text-[12px]" style={{ minHeight: 30, borderBottom: "1px solid var(--border)", color: "var(--fg3)" }}>
        <span className="mono truncate" style={{ color: "var(--fg)" }}>
          {title}
        </span>
        <span>·</span>
        <span>{live ? "live" : "finished"}</span>
        {parsed && (
          <span className="num">
            · {fmtN(total)} lines
            {parsed.dropped > 0 && ` (showing last ${fmtN(parsed.lines.length)})`}
          </span>
        )}
        <span className="flex-1" />
        {parsed?.firstError != null && (
          <Button quiet sm style={{ color: "var(--red)" }} onClick={() => reveal(parsed.firstError!)} title="Scroll to the first error">
            Jump to error · line {fmtN(lineNo(parsed.firstError))}
          </Button>
        )}
        <Button quiet sm aria-pressed={wrap} style={wrap ? { color: "var(--fg)", background: "var(--active)" } : undefined} onClick={() => setWrap((w) => !w)}>
          Wrap
        </Button>
        <Button
          quiet
          sm
          aria-pressed={raw}
          title="The log as fetched: no step sections, timestamps kept"
          style={raw ? { color: "var(--fg)", background: "var(--active)" } : undefined}
          onClick={() => setRaw((r) => !r)}
        >
          Raw
        </Button>
        <Button
          quiet
          sm
          aria-pressed={follow}
          disabled={!live}
          title={live ? "Keep the newest lines in view" : "The job has finished"}
          style={follow ? { color: "var(--fg)", background: "var(--active)" } : undefined}
          onClick={() => setFollow((f) => !f)}
        >
          Follow
        </Button>
        <span className="inline-flex items-center gap-1">
          <input
            type="search"
            value={query}
            placeholder="Search"
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
            className="ft-input"
            style={{ width: 130, height: 22, fontSize: 12 }}
          />
          {query && <span className="num whitespace-nowrap">{matches.length ? `${Math.min(matchAt, matches.length - 1) + 1} of ${matches.length}` : "0 of 0"}</span>}
          <Button quiet sm aria-label="Previous match" title="Previous match (Shift+Enter)" disabled={!matches.length} onClick={() => step_(-1)} className="px-1">
            <Icon name="chevron-down" size={12} style={{ transform: "rotate(180deg)" }} />
          </Button>
          <Button quiet sm aria-label="Next match" title="Next match (Enter)" disabled={!matches.length} onClick={() => step_(1)} className="px-1">
            <Icon name="chevron-down" size={12} />
          </Button>
        </span>
        {job?.url && <PaneLink href={job.url}>Open job</PaneLink>}
        <IconButton icon="x" label="Close logs" size={13} onClick={onClose} style={{ width: 22, height: 22 }} />
      </div>
      <div
        ref={bodyRef}
        onScroll={onScroll}
        className="mono py-1.5"
        style={{ maxHeight: 420, overflow: "auto", lineHeight: `${ROW_PX}px`, color: "var(--fg2)" }}
        data-testid="log-body"
      >
        {q.isLoading && <div className="px-3" style={{ color: "var(--fg3)" }}>Loading logs…</div>}
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
                  className="flex w-full items-center gap-2 px-3 text-left text-[12px] hover:bg-[var(--hover)]"
                  style={{ color: "var(--fg3)" }}
                >
                  <Icon name="chevron-right" size={12} />
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
                  className="flex w-full items-center gap-2 px-3 text-left text-[12px] font-medium hover:bg-[var(--hover)]"
                  style={{ color: "var(--fg)" }}
                >
                  <Icon name={isOpen ? "chevron-down" : "chevron-right"} size={12} style={{ color: "var(--fg3)" }} />
                  <span className="truncate">{sectionName(s)}</span>
                  <span className="flex-1" />
                  <span className="shrink-0 font-normal" style={{ color: "var(--fg3)" }}>
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
                  color: isErr ? "var(--red)" : "var(--fg2)",
                  background: isCur ? "color-mix(in srgb, var(--yellow) 12%, transparent)" : undefined,
                  minWidth: wrap ? undefined : "max-content",
                }}
              >
                <span className="shrink-0 inline-flex items-center" style={{ width: 10, color: "var(--red)" }} aria-label={isFirst ? "first error" : undefined}>
                  {isFirst && <Icon name="arrow-right" size={10} />}
                </span>
                <span className="shrink-0 text-right select-none" style={{ width: 44, color: MONO_DIM }}>
                  {lineNo(r.line)}
                </span>
                {g ? (
                  <>
                    <span className="shrink-0" style={{ color: "var(--fg3)" }}>
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
