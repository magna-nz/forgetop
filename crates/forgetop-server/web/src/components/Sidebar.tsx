import { motion } from "framer-motion";
import { useHealth, useLaunchpad, useNotifications, usePipelines, usePullRequests, useWorkItems } from "../api";
import { useTheme } from "../theme";
import type { SectionId } from "../types";

const NAV: { id: SectionId; label: string; icon: string }[] = [
  { id: "launchpad", label: "Launchpad", icon: "✦" },
  { id: "prs", label: "Pull Requests", icon: "⇄" },
  { id: "work-items", label: "Work Items", icon: "◧" },
  { id: "pipelines", label: "Pipelines", icon: "⛓" },
  { id: "notifications", label: "Notifications", icon: "◔" },
];

export function Sidebar({ section, onSelect }: { section: SectionId; onSelect: (s: SectionId) => void }) {
  const lp = useLaunchpad();
  const prs = usePullRequests();
  const wis = useWorkItems();
  const pipes = usePipelines();
  const notifs = useNotifications();
  const health = useHealth();

  const counts: Record<SectionId, number | undefined> = {
    // Launchpad badge counts only actionable (non-muted) items — things truly waiting on you.
    launchpad: lp.data?.filter((r) => !r.muted).length,
    prs: prs.data?.length,
    "work-items": wis.data?.length,
    pipelines: pipes.data?.length,
    notifications: notifs.data?.filter((n) => n.notification.unread).length,
  };

  const healthy = health.data?.filter((h) => h.healthy).length ?? 0;
  const total = health.data?.length ?? 0;
  const [theme, cycleTheme] = useTheme();

  return (
    <aside
      className="flex flex-col w-60 shrink-0 h-full"
      style={{ background: "var(--panel)", borderRight: "1px solid var(--border)" }}
    >
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
                    background: item.id === "notifications" || item.id === "launchpad" ? "var(--accent)" : "var(--panel2)",
                    color: item.id === "notifications" || item.id === "launchpad" ? "#10233b" : "var(--dim)",
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
        <div className="mt-1.5 flex items-center justify-between">
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
    </aside>
  );
}
