import { useHealth, useLaunchpad, usePipelines, usePullRequests, useWorkItems } from "../api";
import { pipelineCount } from "../format";
import { useTheme } from "../theme";
import type { SectionId } from "../types";
import { Icon, type IconName } from "./icons";

const NAV: { id: SectionId; label: string; icon: IconName }[] = [
  // "Command Center" is the user-facing name for the launchpad; code keeps the `launchpad` id.
  { id: "launchpad", label: "Command Center", icon: "inbox" },
  { id: "prs", label: "Pull Requests", icon: "pull-request" },
  { id: "work-items", label: "Work Items", icon: "circle-dot" },
  { id: "pipelines", label: "Pipelines", icon: "workflow" },
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
    pipelines: pipes.data && pipelineCount(pipes.data),
    settings: undefined,
  };

  const healthy = health.data?.filter((h) => h.healthy).length ?? 0;
  const total = health.data?.length ?? 0;
  const [theme, cycleTheme] = useTheme();

  const item = (id: SectionId, label: string, icon: IconName) => {
    const active = id === section;
    const count = counts[id];
    return (
      <button
        key={id}
        type="button"
        onClick={() => onSelect(id)}
        aria-current={active ? "page" : undefined}
        className="flex items-center gap-2.5 h-8 px-2.5 rounded-md text-left text-[13px] w-full transition-colors"
        style={{ color: active ? "var(--fg)" : "var(--fg2)", background: active ? "var(--active)" : "transparent" }}
        onMouseEnter={(e) => !active && (e.currentTarget.style.background = "var(--hover)")}
        onMouseLeave={(e) => !active && (e.currentTarget.style.background = "transparent")}
      >
        <Icon name={icon} size={16} strokeWidth={1.5} />
        <span className="flex-1 truncate">{label}</span>
        {count != null && count > 0 && (
          <span className="num text-[12px]" style={{ color: id === "launchpad" ? "var(--accent-fg)" : "var(--fg3)" }}>
            {count}
          </span>
        )}
      </button>
    );
  };

  return (
    <aside
      className={`shrink-0 min-w-0 h-full overflow-hidden transition-[width] duration-200 ease-out ${collapsed ? "w-0" : "w-60"}`}
      style={{ borderRight: collapsed ? "none" : "1px solid var(--border)" }}
    >
      <div className="flex flex-col w-60 h-full px-2.5 pt-3.5 pb-3" style={{ background: "var(--panel)" }}>
        <div className="flex items-center gap-2 px-2.5 pb-3.5">
          {/* The README mark: four forges linked to one core (docs/forgetop-mark.svg). */}
          <svg width="20" height="20" viewBox="0 0 256 256" fill="none" aria-hidden="true" data-icon="forgetop-mark">
            <g stroke="#8A79E0" strokeWidth="14" strokeLinecap="round">
              <path d="M128 56V88" />
              <path d="M200 128H168" />
              <path d="M128 200V168" />
              <path d="M56 128H88" />
            </g>
            <g fill="#8A79E0">
              <circle cx="128" cy="36" r="20" />
              <circle cx="220" cy="128" r="20" />
              <circle cx="128" cy="220" r="20" />
              <circle cx="36" cy="128" r="20" />
            </g>
            <rect x="88" y="88" width="80" height="80" rx="20" fill="#6C5BD4" />
          </svg>
          <span className="font-semibold text-[14px]" style={{ letterSpacing: "-0.01em" }}>
            forgetop
          </span>
          <span className="mono ml-auto text-[12px]" style={{ color: "var(--fg3)" }}>
            v{__APP_VERSION__}
          </span>
        </div>

        <nav className="flex flex-col gap-0.5" aria-label="Sections">
          {NAV.map((n) => item(n.id, n.label, n.icon))}
        </nav>
        <div className="mx-2.5 my-3" style={{ height: 1, background: "var(--border)" }} />
        <nav className="flex flex-col gap-0.5" aria-label="Workspace">
          {item("settings", "Settings", "settings")}
        </nav>

        <div className="mt-auto px-2.5">
          <a
            href="https://github.com/magna-nz/forgetop/issues/new?template=feedback.yml"
            target="_blank"
            rel="noopener noreferrer"
            className="flex items-center gap-2 h-8 -mx-2.5 px-2.5 rounded-md text-[12.5px] transition-colors"
            style={{ color: "var(--fg2)" }}
            onMouseEnter={(e) => (e.currentTarget.style.background = "var(--hover)")}
            onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
          >
            <Icon name="comment" size={14} strokeWidth={1.5} />
            <span>Give Feedback</span>
            <Icon name="external" size={12} className="ml-auto" style={{ color: "var(--fg3)" }} />
          </a>
        </div>

        <div className="mt-3 px-2.5 pt-3 text-[12px] flex flex-col gap-2" style={{ borderTop: "1px solid var(--border)", color: "var(--fg3)" }}>
          <div className="flex items-center gap-2">
            <span
              className="inline-block w-1.5 h-1.5 rounded-full"
              style={{ background: total === 0 ? "var(--fg3)" : healthy === total ? "var(--green)" : "var(--yellow)" }}
            />
            <span>{total === 0 ? "No connections" : `${healthy}/${total} connections healthy`}</span>
          </div>
          <div className="flex items-center justify-between">
            <span>Live · every 15s</span>
            <button
              type="button"
              onClick={cycleTheme}
              title="Switch theme"
              className="inline-flex items-center gap-1.5 capitalize rounded px-1 -mr-1 transition-colors"
              style={{ color: "var(--fg3)" }}
              onMouseEnter={(e) => (e.currentTarget.style.color = "var(--fg)")}
              onMouseLeave={(e) => (e.currentTarget.style.color = "var(--fg3)")}
            >
              {theme}
            </button>
          </div>
        </div>
      </div>
    </aside>
  );
}
