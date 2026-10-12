import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { useNotifications, usePipelines, usePullRequests, useWorkItems } from "../api";
import { notificationMeta, pipeMeta, prStatusMeta, wiStateColor, wiStateGlyph } from "../format";
import { usePrOpener } from "./PrDetail";
import { Icon } from "./icons";
import { Kbd, StatusGlyph, type GlyphName } from "./ui";
import type { SectionId } from "../types";

interface Command {
  id: string;
  label: string;
  /** Searched but not shown (kind and connection), so typing a connection name still finds it. */
  sublabel?: string;
  /** Mono identifier shown after the title: `#1501`, `OPS-1423`, a repository slug. */
  ident?: string;
  glyph: GlyphName;
  color: string;
  run: () => void;
}

/** Subsequence fuzzy score, or null when `query` isn't a subsequence of `text`. Rewards
 *  consecutive hits and word-boundary starts, mildly prefers shorter targets. */
function fuzzyScore(query: string, text: string): number | null {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (!q) return 0;
  let ti = 0;
  let score = 0;
  let streak = 0;
  for (const c of q) {
    let found = -1;
    for (let j = ti; j < t.length; j++) {
      if (t[j] === c) {
        found = j;
        break;
      }
    }
    if (found === -1) return null;
    if (found === ti) {
      streak++;
      score += 2 + streak;
    } else {
      streak = 0;
      score += 1;
      if (found === 0 || t[found - 1] === " " || t[found - 1] === "/") score += 3;
    }
    ti = found + 1;
  }
  return score - (t.length - q.length) * 0.05;
}

const NAV_COLOR = "var(--fg2)";

export function CommandPalette({
  open,
  onClose,
  onNavigate,
}: {
  open: boolean;
  onClose: () => void;
  onNavigate: (s: SectionId) => void;
}) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const prs = usePullRequests();
  const wis = useWorkItems();
  const pipes = usePipelines();
  const notifs = useNotifications();
  const openPr = usePrOpener();

  const commands = useMemo<Command[]>(() => {
    const go = (s: SectionId) => () => {
      onNavigate(s);
      onClose();
    };
    const openUrl = (url?: string | null, section?: SectionId) => () => {
      if (url) window.open(url, "_blank", "noreferrer");
      else if (section) onNavigate(section);
      onClose();
    };
    const nav: Command[] = [
      // "Command Center" = the launchpad section (code keeps the `launchpad` id).
      { id: "nav:launchpad", label: "Go to Command Center", glyph: "inbox", color: NAV_COLOR, run: go("launchpad") },
      { id: "nav:prs", label: "Go to Pull Requests", glyph: "pull-request", color: NAV_COLOR, run: go("prs") },
      { id: "nav:wi", label: "Go to Work Items", glyph: "circle-dot", color: NAV_COLOR, run: go("work-items") },
      { id: "nav:pipes", label: "Go to Pipelines", glyph: "workflow", color: NAV_COLOR, run: go("pipelines") },
      { id: "nav:notifs", label: "Go to Notifications", glyph: "bell", color: NAV_COLOR, run: go("notifications") },
    ];
    const prItems: Command[] = (prs.data ?? []).map((r) => {
      const meta = prStatusMeta(r.pull_request);
      return {
      id: `pr:${r.connection_id}:${r.pull_request.id}`,
      label: r.pull_request.title,
      sublabel: `PR${r.pull_request.number != null ? " #" + r.pull_request.number : ""} · ${r.connection}`,
      ident: r.pull_request.number != null ? `#${r.pull_request.number}` : undefined,
      glyph: meta.glyph,
      color: meta.color,
      // PRs open the in-app detail (with the diff + review), like clicking a PR row.
      run: () => {
        openPr({ conn: r.connection_id, repo: r.pull_request.repository, id: r.pull_request.id });
        onClose();
      },
      };
    });
    const wiItems: Command[] = (wis.data ?? []).map((r) => ({
      id: `wi:${r.connection_id}:${r.work_item.id}`,
      label: r.work_item.title,
      sublabel: `${r.work_item.identifier ?? "Issue"} · ${r.connection}`,
      ident: r.work_item.identifier ?? undefined,
      glyph: wiStateGlyph(r.work_item.state, r.work_item.state_category),
      color: wiStateColor(r.work_item.state, r.work_item.state_category),
      run: openUrl(r.work_item.url, "work-items"),
    }));
    const pipeItems: Command[] = (pipes.data ?? []).map((r) => {
      const meta = pipeMeta(r.run.status);
      return {
        id: `pipe:${r.connection_id}:${r.run.id}`,
        label: r.run.name ?? (r.run.number != null ? `Run #${r.run.number}` : r.run.definition_id),
        sublabel: `Pipeline · ${r.connection}`,
        ident: r.run.name != null && r.run.number != null ? `#${r.run.number}` : undefined,
        glyph: meta.glyph,
        color: meta.color,
        run: openUrl(r.run.url, "pipelines"),
      };
    });
    const notifItems: Command[] = (notifs.data ?? []).map((r) => {
      const meta = notificationMeta(r.notification.kind);
      return {
        id: `notif:${r.connection_id}:${r.notification.id}`,
        label: r.notification.title,
        sublabel: `${r.notification.context} · ${r.connection}`,
        ident: r.notification.context,
        glyph: meta.glyph,
        color: meta.color,
        run: openUrl(r.notification.url, "notifications"),
      };
    });
    return [...nav, ...prItems, ...wiItems, ...pipeItems, ...notifItems];
  }, [prs.data, wis.data, pipes.data, notifs.data, onNavigate, onClose, openPr]);

  const results = useMemo(() => {
    if (!query.trim()) return commands.slice(0, 8);
    return commands
      .map((c) => ({ c, score: Math.max(fuzzyScore(query, c.label) ?? -Infinity, (fuzzyScore(query, c.sublabel ?? "") ?? -Infinity) - 2) }))
      .filter((x) => x.score > -Infinity)
      .sort((a, b) => b.score - a.score)
      .slice(0, 12)
      .map((x) => x.c);
  }, [query, commands]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setSelected(0);
      // focus after the panel mounts
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  useEffect(() => setSelected(0), [query]);

  if (!open) return null;

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((s) => Math.min(s + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((s) => Math.max(s - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      results[selected]?.run();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  return (
    <AnimatePresence>
      <motion.div
        className="fixed inset-0 z-50 flex items-start justify-center"
        style={{ background: "rgba(0,0,0,0.4)", paddingTop: "18vh" }}
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.12 }}
        onClick={onClose}
      >
        <motion.div
          role="dialog"
          aria-label="Command palette"
          initial={{ opacity: 0, y: -8, scale: 0.98 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={{ duration: 0.14, ease: "easeOut" }}
          onClick={(e) => e.stopPropagation()}
          onKeyDown={onKeyDown}
          className="mx-4 overflow-hidden shadow-2xl"
          style={{
            width: 640,
            maxWidth: "calc(100vw - 2rem)",
            borderRadius: 10,
            background: "var(--raised)",
            border: "1px solid var(--border-strong)",
          }}
        >
          <div className="flex items-center gap-2.5 h-10 px-3.5" style={{ borderBottom: "1px solid var(--border)" }}>
            <Icon name="search" size={15} style={{ color: "var(--fg3)" }} />
            <input
              ref={inputRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Jump to a section or search PRs, issues, pipelines…"
              aria-label="Search"
              className="flex-1 min-w-0 h-full bg-transparent text-[13px] outline-none"
              style={{ color: "var(--fg)" }}
            />
          </div>
          <div className="max-h-80 overflow-auto py-1">
            {results.length === 0 ? (
              <div className="px-4 py-6 text-center text-[13px]" style={{ color: "var(--fg3)" }}>
                No matches
              </div>
            ) : (
              results.map((c, i) => (
                <button
                  key={c.id}
                  type="button"
                  onMouseEnter={() => setSelected(i)}
                  onClick={() => c.run()}
                  className="ft-row"
                  style={{
                    height: 36,
                    borderBottom: 0,
                    background: i === selected ? "var(--active)" : undefined,
                  }}
                >
                  <StatusGlyph glyph={c.glyph} color={c.color} />
                  <span className="truncate text-[13px]" style={{ color: "var(--fg)" }}>
                    {c.label}
                  </span>
                  {c.ident && (
                    <span className="mono shrink-0 truncate" style={{ color: "var(--fg3)", maxWidth: "40%" }}>
                      {c.ident}
                    </span>
                  )}
                  {i === selected && (
                    <span className="ml-auto shrink-0">
                      <Kbd>↵</Kbd>
                    </span>
                  )}
                </button>
              ))
            )}
          </div>
          <div
            className="flex items-center gap-4 px-3.5 h-9 text-[12px]"
            style={{ borderTop: "1px solid var(--border)", color: "var(--fg3)" }}
          >
            <span className="inline-flex items-center gap-1.5">
              <Kbd>↑</Kbd>
              <Kbd>↓</Kbd>
              navigate
            </span>
            <span className="inline-flex items-center gap-1.5">
              <Kbd>↵</Kbd>
              open
            </span>
            <span className="inline-flex items-center gap-1.5">
              <Kbd>esc</Kbd>
              close
            </span>
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
