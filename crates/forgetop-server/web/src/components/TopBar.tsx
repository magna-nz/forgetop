import { useIsFetching, useQueryClient } from "@tanstack/react-query";
import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { apiPost, useNotifications } from "../api";
import { notificationMeta, relativeTime, toTime } from "../format";
import { useNavigateSection } from "../nav";
import type { NotifRow, SectionId } from "../types";
import { usePrOpener } from "./PrDetail";
import { useWiOpener } from "./WiDetail";

const META: Record<SectionId, { title: string; subtitle: string }> = {
  // "Command Center" is the user-facing name for the launchpad section (code keeps `launchpad`).
  launchpad: { title: "Command Center", subtitle: "Things that need your attention." },
  prs: { title: "Pull Requests", subtitle: "Open PRs you author or are asked to review, across every connection." },
  "work-items": { title: "Work Items", subtitle: "Issues and tickets currently assigned to you." },
  pipelines: { title: "Pipelines", subtitle: "Recent CI runs across your repositories." },
  notifications: { title: "Notifications", subtitle: "Review requests, mentions, and CI failures." },
  settings: { title: "Settings", subtitle: "Manage the connections shared with the terminal app." },
  feedback: { title: "Give Feedback", subtitle: "Share a private report, with diagnostics only when you choose." },
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
  const { data: notifications } = useNotifications();
  const navigate = useNavigateSection();
  const openPr = usePrOpener();
  const openWi = useWiOpener();
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  const notificationRef = useRef<HTMLDivElement>(null);
  const unread = notifications?.filter((r) => r.notification.unread).length ?? 0;
  // Newest first — the standard notification-bell order.
  const orderedNotifications = [...(notifications ?? [])].sort((a, b) => toTime(b.notification.updated_at) - toTime(a.notification.updated_at));

  // Clicking a popup notification opens its item (PR/work-item in-app, else the provider URL),
  // marks it read, and closes the popup — same behaviour as a row on the Notifications page.
  const openNotification = (row: NotifRow) => {
    const n = row.notification;
    const conn = row.connection_id;
    if (n.unread) {
      void apiPost("/api/notification/read", { conn, id: n.id }).then(() => {
        qc.invalidateQueries({ queryKey: ["notifications"] });
        qc.invalidateQueries({ queryKey: ["launchpad"] });
      });
    }
    if (n.item_id != null && n.item_type === "PullRequest") openPr({ conn, id: n.item_id });
    else if (n.item_id != null && n.item_type === "WorkItem") openWi({ conn, id: n.item_id });
    else if (n.url) window.open(n.url, "_blank", "noopener,noreferrer");
    setNotificationsOpen(false);
  };

  useEffect(() => {
    if (!notificationsOpen) return;
    const onMouseDown = (e: MouseEvent) => {
      if (notificationRef.current && !notificationRef.current.contains(e.target as Node)) setNotificationsOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setNotificationsOpen(false);
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [notificationsOpen]);

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
        <div className="relative" ref={notificationRef}>
          <button
            onClick={() => setNotificationsOpen((open) => !open)}
            title="Notifications"
            aria-label="Notifications"
            aria-expanded={notificationsOpen}
            className="flex items-center gap-1.5 rounded-md px-2 py-1 text-xs transition-colors"
            style={{ color: unread > 0 ? "var(--fg)" : "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)", fontWeight: unread > 0 ? 700 : undefined }}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" />
              <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
            </svg>
            {unread > 0 && (
              <span className="mono rounded-full px-1.5 text-xs" style={{ background: "var(--accent)", color: "#10233b" }}>
                {unread}
              </span>
            )}
          </button>
          {notificationsOpen && (
            <div className="absolute right-0 top-full z-30 mt-2 w-[340px] overflow-hidden rounded-lg shadow-lg" style={{ background: "var(--panel)", border: "1px solid var(--border)" }}>
              <div className="px-3 py-2 text-xs font-semibold" style={{ borderBottom: "1px solid var(--border)", color: "var(--fg)" }}>
                Notifications
              </div>
              <div className="max-h-80 overflow-auto">
                {orderedNotifications.length === 0 ? (
                  <div className="px-3 py-3 text-xs" style={{ color: "var(--dim)" }}>No notifications</div>
                ) : (
                  orderedNotifications.map((row) => {
                    const n = row.notification;
                    const kind = notificationMeta(n.kind);
                    return (
                      <button
                        key={`${row.connection_id}:${n.id}`}
                        type="button"
                        onClick={() => openNotification(row)}
                        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors"
                        style={{ borderBottom: "1px solid var(--border)", cursor: "pointer" }}
                        onMouseEnter={(e) => (e.currentTarget.style.background = "var(--sel)")}
                        onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                      >
                        <div className="min-w-0 flex-1">
                          <div className="truncate text-xs font-medium" style={{ color: n.unread ? "var(--fg)" : "var(--dim)" }}>{n.title}</div>
                          <div className="text-[11px]" style={{ color: kind.color }}>{kind.label}</div>
                        </div>
                        <span className="shrink-0 text-[11px] whitespace-nowrap" style={{ color: "var(--dim)" }}>{relativeTime(n.updated_at)}</span>
                      </button>
                    );
                  })
                )}
              </div>
              <button
                onClick={() => {
                  navigate("notifications");
                  setNotificationsOpen(false);
                }}
                className="w-full px-3 py-2 text-left text-xs font-medium transition-colors"
                style={{ borderTop: "1px solid var(--border)", color: "var(--fg)" }}
              >
                More →
              </button>
            </div>
          )}
        </div>
      </div>
    </header>
  );
}
