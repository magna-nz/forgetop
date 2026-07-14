import { useIsFetching, useQueryClient } from "@tanstack/react-query";
import { AnimatePresence, motion } from "framer-motion";
import type { SectionId } from "../types";

const META: Record<SectionId, { title: string; subtitle: string }> = {
  prs: { title: "Pull Requests", subtitle: "Open PRs you author or are asked to review, across every connection." },
  "work-items": { title: "Work Items", subtitle: "Issues and tickets currently assigned to you." },
  pipelines: { title: "Pipelines", subtitle: "Recent CI runs across your repositories." },
  notifications: { title: "Notifications", subtitle: "Review requests, mentions, and CI failures." },
};

export function TopBar({ section }: { section: SectionId }) {
  const meta = META[section];
  const fetching = useIsFetching() > 0;
  const qc = useQueryClient();

  return (
    <header
      className="flex items-center gap-4 px-6 h-14 shrink-0"
      style={{ background: "var(--panel)", borderBottom: "1px solid var(--border)" }}
    >
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
