import { useIsFetching, useQueryClient } from "@tanstack/react-query";
import { AnimatePresence, motion } from "framer-motion";
import type { SectionId } from "../types";

const META: Record<SectionId, { title: string; subtitle: string }> = {
  // "Command Center" is the user-facing name for the launchpad section (code keeps `launchpad`).
  launchpad: { title: "Command Center", subtitle: "Everything triaged by what needs you first — review, ship, fix, then your work." },
  prs: { title: "Pull Requests", subtitle: "Open PRs you author or are asked to review, across every connection." },
  "work-items": { title: "Work Items", subtitle: "Issues and tickets currently assigned to you." },
  pipelines: { title: "Pipelines", subtitle: "Recent CI runs across your repositories." },
  notifications: { title: "Notifications", subtitle: "Review requests, mentions, and CI failures." },
  settings: { title: "Settings", subtitle: "Manage the connections shared with the terminal app." },
};

export function TopBar({
  section,
  onOpenPalette,
  sidebarOpen,
  onToggleSidebar,
}: {
  section: SectionId;
  onOpenPalette: () => void;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
}) {
  const meta = META[section];
  const fetching = useIsFetching() > 0;
  const qc = useQueryClient();

  return (
    <header
      className="flex items-center gap-4 px-6 h-14 shrink-0"
      style={{ background: "var(--panel)", borderBottom: "1px solid var(--border)" }}
    >
      <button
        onClick={onToggleSidebar}
        title={sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
        aria-label={sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
        aria-expanded={sidebarOpen}
        className="-ml-2 flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-base transition-colors"
        style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}
        onMouseEnter={(e) => (e.currentTarget.style.color = "var(--fg)")}
        onMouseLeave={(e) => (e.currentTarget.style.color = "var(--dim)")}
      >
        ☰
      </button>
      <div className="min-w-0">
        <h1 className="text-sm font-semibold leading-tight">{meta.title}</h1>
        <p className="text-xs truncate" style={{ color: "var(--dim)" }}>
          {meta.subtitle}
        </p>
      </div>

      <div className="ml-auto flex items-center gap-3">
        <AnimatePresence>
          {fetching && (
            <motion.span
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="flex items-center gap-1.5 text-xs"
              style={{ color: "var(--dim)" }}
            >
              <span className="spin" style={{ color: "var(--accent)" }}>
                ◐
              </span>
              syncing
            </motion.span>
          )}
        </AnimatePresence>
        <button
          onClick={onOpenPalette}
          title="Command palette (⌘K)"
          className="flex items-center gap-2 rounded-md px-2.5 py-1 text-xs transition-colors"
          style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}
          onMouseEnter={(e) => (e.currentTarget.style.color = "var(--fg)")}
          onMouseLeave={(e) => (e.currentTarget.style.color = "var(--dim)")}
        >
          <span>Search</span>
          <kbd className="mono rounded px-1" style={{ background: "var(--bg)", border: "1px solid var(--border)" }}>
            ⌘K
          </kbd>
        </button>
        <button
          onClick={() => qc.invalidateQueries()}
          title="Refresh now"
          className="flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs transition-colors"
          style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}
          onMouseEnter={(e) => (e.currentTarget.style.color = "var(--fg)")}
          onMouseLeave={(e) => (e.currentTarget.style.color = "var(--dim)")}
        >
          ↻ Refresh
        </button>
      </div>
    </header>
  );
}
