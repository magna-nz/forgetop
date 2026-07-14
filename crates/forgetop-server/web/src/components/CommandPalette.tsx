import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { useNotifications, usePipelines, usePullRequests, useWorkItems } from "../api";
import type { SectionId } from "../types";

interface Command {
  id: string;
  label: string;
  sublabel?: string;
  icon: string;
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
      { id: "nav:launchpad", label: "Go to Launchpad", icon: "✦", run: go("launchpad") },
      { id: "nav:prs", label: "Go to Pull Requests", icon: "⇄", run: go("prs") },
      { id: "nav:wi", label: "Go to Work Items", icon: "◧", run: go("work-items") },
      { id: "nav:pipes", label: "Go to Pipelines", icon: "⛓", run: go("pipelines") },
      { id: "nav:notifs", label: "Go to Notifications", icon: "◔", run: go("notifications") },
    ];
    const prItems: Command[] = (prs.data ?? []).map((r) => ({
      id: `pr:${r.connection_id}:${r.pull_request.id}`,
      label: r.pull_request.title,
      sublabel: `PR${r.pull_request.number != null ? " #" + r.pull_request.number : ""} · ${r.connection}`,
      icon: "⇄",
      run: openUrl(r.pull_request.url, "prs"),
    }));
    const wiItems: Command[] = (wis.data ?? []).map((r) => ({
      id: `wi:${r.connection_id}:${r.work_item.id}`,
      label: r.work_item.title,
      sublabel: `${r.work_item.identifier ?? "Issue"} · ${r.connection}`,
      icon: "◧",
      run: openUrl(r.work_item.url, "work-items"),
    }));
    const pipeItems: Command[] = (pipes.data ?? []).map((r) => ({
      id: `pipe:${r.connection_id}:${r.run.id}`,
      label: r.run.name ?? (r.run.number != null ? `Run #${r.run.number}` : r.run.definition_id),
      sublabel: `Pipeline · ${r.connection}`,
      icon: "⛓",
      run: openUrl(r.run.url, "pipelines"),
    }));
    const notifItems: Command[] = (notifs.data ?? []).map((r) => ({
      id: `notif:${r.connection_id}:${r.notification.id}`,
      label: r.notification.title,
      sublabel: `${r.notification.context} · ${r.connection}`,
      icon: "◔",
      run: openUrl(r.notification.url, "notifications"),
    }));
    return [...nav, ...prItems, ...wiItems, ...pipeItems, ...notifItems];
  }, [prs.data, wis.data, pipes.data, notifs.data, onNavigate, onClose]);

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
        style={{ background: "rgba(0,0,0,0.5)", paddingTop: "12vh" }}
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.12 }}
        onClick={onClose}
      >
        <motion.div
          initial={{ opacity: 0, y: -8, scale: 0.98 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={{ duration: 0.14, ease: "easeOut" }}
          onClick={(e) => e.stopPropagation()}
          onKeyDown={onKeyDown}
          className="w-full max-w-xl mx-4 rounded-xl overflow-hidden shadow-2xl"
          style={{ background: "var(--panel)", border: "1px solid var(--border)" }}
        >
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Jump to a section or search PRs, issues, pipelines…"
            className="w-full bg-transparent px-4 py-3.5 text-sm outline-none"
            style={{ color: "var(--fg)", borderBottom: "1px solid var(--border)" }}
          />
          <div className="max-h-80 overflow-auto py-1.5">
            {results.length === 0 ? (
              <div className="px-4 py-6 text-center text-sm" style={{ color: "var(--dim)" }}>
                No matches
              </div>
            ) : (
              results.map((c, i) => (
                <button
                  key={c.id}
                  onMouseEnter={() => setSelected(i)}
                  onClick={() => c.run()}
                  className="flex w-full items-center gap-3 px-4 py-2 text-left"
                  style={{ background: i === selected ? "var(--sel)" : "transparent" }}
                >
                  <span className="w-4 text-center shrink-0" style={{ color: "var(--accent)" }}>
                    {c.icon}
                  </span>
                  <span className="flex-1 min-w-0">
                    <span className="block truncate text-sm" style={{ color: "var(--fg)" }}>
                      {c.label}
                    </span>
                    {c.sublabel && (
                      <span className="block truncate text-xs" style={{ color: "var(--dim)" }}>
                        {c.sublabel}
                      </span>
                    )}
                  </span>
                </button>
              ))
            )}
          </div>
          <div
            className="flex items-center gap-3 px-4 py-2 text-xs"
            style={{ borderTop: "1px solid var(--border)", color: "var(--dim)" }}
          >
            <span>↑↓ navigate</span>
            <span>↵ open</span>
            <span>esc close</span>
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
