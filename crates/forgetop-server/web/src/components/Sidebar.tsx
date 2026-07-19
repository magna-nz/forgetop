import { motion } from "framer-motion";
import { useHealth, useLaunchpad, usePipelines, usePullRequests, useWorkItems } from "../api";
import { useTheme } from "../theme";
import type { SectionId } from "../types";

const NAV: { id: SectionId; label: string; icon: string }[] = [
  // "Command Center" is the user-facing name for the launchpad; code keeps the `launchpad` id.
  { id: "launchpad", label: "Command Center", icon: "✦" },
  { id: "prs", label: "Pull Requests", icon: "⇄" },
  { id: "work-items", label: "Work Items", icon: "◧" },
  { id: "pipelines", label: "Pipelines", icon: "⛓" },
  { id: "settings", label: "Settings", icon: "⚙" },
];

export function Sidebar({
  section,
  onSelect,
  collapsed,
}: {
  section: SectionId;
  onSelect: (s: SectionId) => void;
  collapsed: boolean;
}) {
  const lp = useLaunchpad();
  const prs = usePullRequests();
  const wis = useWorkItems();
  const pipes = usePipelines();
  const health = useHealth();

  // Notifications is no longer in the sidebar (it lives in the top-bar bell), so it has no count here.
  const counts: Partial<Record<SectionId, number | undefined>> = {
    // Launchpad badge counts only actionable (non-muted) items — things truly waiting on you.
    launchpad: lp.data?.rows.filter((r) => !r.muted).length,
    prs: prs.data?.length,
    "work-items": wis.data?.length,
    pipelines: pipes.data?.length,
    settings: undefined,
  };

  const healthy = health.data?.filter((h) => h.healthy).length ?? 0;
  const total = health.data?.length ?? 0;
  const [theme, cycleTheme] = useTheme();

  return (
    <aside
      className={`shrink-0 min-w-0 h-full overflow-hidden transition-[width] duration-200 ease-out ${collapsed ? "w-0" : "w-60"}`}
      style={{ borderRight: collapsed ? "none" : "1px solid var(--border)" }}
    >
      <div className="flex flex-col w-60 h-full" style={{ background: "var(--panel)" }}>
      <div className="flex items-center gap-2 px-5 h-14 shrink-0" style={{ borderBottom: "1px solid var(--border)" }}>
        <span className="text-lg" style={{ color: "var(--accent)" }}>
          ◿
        </span>
        <span className="font-semibold tracking-tight">forgetop</span>
        <span className="mono text-xs ml-auto" style={{ color: "var(--dim)" }}>
          v{__APP_VERSION__}
        </span>
      </div>

      <nav className="flex flex-col gap-1 p-3">
        {NAV.map((item) => {
          const active = item.id === section;
          const count = counts[item.id];
          return (
            <button
              key={item.id}
              onClick={() => onSelect(item.id)}
              className="relative flex items-center gap-3 rounded-md px-3 py-2 text-left text-sm transition-colors"
              style={{ color: active ? "var(--fg)" : "var(--dim)" }}
              onMouseEnter={(e) => !active && (e.currentTarget.style.color = "var(--fg)")}
              onMouseLeave={(e) => !active && (e.currentTarget.style.color = "var(--dim)")}
            >
              {active && (
                <motion.span
                  layoutId="nav-active"
                  className="absolute inset-0 rounded-md -z-0"
                  style={{ background: "var(--sel)" }}
                  transition={{ type: "spring", stiffness: 400, damping: 32 }}
                />
              )}
              <span className="relative z-10 w-4 text-center">{item.icon}</span>
              <span className="relative z-10 flex-1">{item.label}</span>
              {count != null && count > 0 && (
                <span
                  className="relative z-10 mono rounded-full px-1.5 text-xs"
                  style={{
                    background: item.id === "launchpad" ? "var(--accent)" : "var(--panel2)",
                    color: item.id === "launchpad" ? "#10233b" : "var(--dim)",
                  }}
                >
                  {count}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      <div className="mt-auto p-4 text-xs" style={{ borderTop: "1px solid var(--border)", color: "var(--dim)" }}>
        <div className="flex items-center gap-2">
          <span
            className="inline-block w-2 h-2 rounded-full"
            style={{ background: total === 0 ? "var(--dim)" : healthy === total ? "var(--green)" : "var(--yellow)" }}
          />
          {total === 0 ? "No connections" : `${healthy}/${total} connections healthy`}
        </div>
        <a
          href="https://github.com/magna-nz/forgetop/issues/new?template=feedback.yml"
          target="_blank"
          rel="noopener noreferrer"
          className="mt-3 flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-xs font-medium transition-colors"
          style={{
            color: "var(--dim)",
            background: "var(--panel2)",
            border: "1px solid var(--border)",
          }}
        >
          <span aria-hidden="true" style={{ color: "var(--accent)" }}>◇</span>
          <span>Give Feedback</span>
          <span className="ml-auto" aria-hidden="true">↗</span>
        </a>
        <div className="mt-2 flex items-center justify-between">
          <span className="opacity-70">Live · every 15s</span>
          <button
            onClick={cycleTheme}
            title="Switch theme"
            className="rounded px-1.5 py-0.5 capitalize"
            style={{ border: "1px solid var(--border)", background: "var(--panel2)", color: "var(--dim)" }}
          >
            ◐ {theme}
          </button>
        </div>
      </div>
      </div>
    </aside>
  );
}
