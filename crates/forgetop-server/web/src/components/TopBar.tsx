import { useIsFetching, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { apiPost, useCacheStatus, useNotifications } from "../api";
import { notificationMeta, relativeTime, toTime } from "../format";
import { useNavigateSection } from "../nav";
import type { NotifRow, SectionId } from "../types";
import { markNotificationReadInCache } from "../optimistic";
import { Icon } from "./icons";
import { usePrOpener } from "./PrDetail";
import { Button, IconButton, Kbd, StatusGlyph } from "./ui";
import { useWiOpener } from "./WiDetail";

const TITLES: Record<SectionId, string> = {
  // "Command Center" is the user-facing name for the launchpad section (code keeps `launchpad`).
  launchpad: "Command Center",
  prs: "Pull Requests",
  "work-items": "Work Items",
  pipelines: "Pipelines",
  notifications: "Notifications",
  settings: "Settings",
};

/** "4m", "2h", "3d" — how old the cached rows on screen are. */
export function cacheAgeLabel(ms: number): string {
  const m = Math.round(ms / 60_000);
  if (m < 1) return "under a minute";
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h}h`;
  return `${Math.round(h / 24)}d`;
}

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
  const fetching = useIsFetching() > 0;
  const staleAge = useCacheStatus();
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
      // Same shared edit the Notifications page uses: the badge drops on the click, and the
      // invalidation that follows confirms it — or, if the write failed, undoes it.
      markNotificationReadInCache(qc, conn, n.id);
      void apiPost("/api/notification/read", { conn, id: n.id }).finally(() => {
        qc.invalidateQueries({ queryKey: ["notifications"] });
        qc.invalidateQueries({ queryKey: ["launchpad"] });
      });
    }
    // A notification names the repository its item lives in — that is what lets the inbox open
    // an item on a connection spanning several repositories.
    if (n.item_id != null && n.item_type === "PullRequest") openPr({ conn, repo: n.repository, id: n.item_id });
    else if (n.item_id != null && n.item_type === "WorkItem") openWi({ conn, repo: n.repository, id: n.item_id });
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
    <header className="flex items-center gap-2 px-4 h-12 shrink-0" style={{ borderBottom: "1px solid var(--border)" }}>
      <IconButton
        icon="sidebar"
        label={sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
        aria-expanded={sidebarOpen}
        onClick={onToggleSidebar}
        className="-ml-1"
      />
      <h1 className="m-0 text-[15px] font-semibold leading-tight ml-1" style={{ letterSpacing: "-0.01em" }}>
        {TITLES[section]}
      </h1>
      {staleAge !== null ? (
        <span className="flex items-center gap-1.5 text-[12px] ml-2" style={{ color: "var(--fg3)" }} title="Shown from the cache shared with the terminal UI; a live refresh is on its way">
          <StatusGlyph glyph="running" spin color="var(--fg3)" />
          from {cacheAgeLabel(staleAge)} ago · refreshing
        </span>
      ) : (
        fetching && (
          <span className="flex items-center gap-1.5 text-[12px] ml-2" style={{ color: "var(--fg3)" }}>
            <StatusGlyph glyph="running" spin color="var(--fg3)" />
            syncing
          </span>
        )
      )}

      <div className="ml-auto flex items-center gap-1">
        <Button quiet icon="search" onClick={onOpenPalette} title="Command palette (⌘K)">
          Search <Kbd>⌘K</Kbd>
        </Button>
        <IconButton icon="refresh" label="Refresh" title="Refresh now" onClick={() => qc.invalidateQueries()} />
        <div className="relative" ref={notificationRef}>
          <button
            type="button"
            onClick={() => setNotificationsOpen((open) => !open)}
            title="Notifications"
            aria-label={unread > 0 ? `Notifications, ${unread} unread` : "Notifications"}
            aria-expanded={notificationsOpen}
            className="ft-btn ft-btn-icon relative"
            style={{ color: unread > 0 ? "var(--fg)" : undefined }}
          >
            <Icon name="bell" size={15} />
            {unread > 0 && (
              <span
                className="num absolute -top-0.5 -right-0.5 min-w-[16px] h-4 px-1 rounded-full text-[10px] font-semibold flex items-center justify-center"
                style={{ background: "var(--accent)", color: "var(--accent-contrast)" }}
              >
                {unread}
              </span>
            )}
          </button>
          {notificationsOpen && (
            <div
              className="absolute right-0 top-full z-30 mt-1.5 w-[360px] overflow-hidden rounded-lg shadow-xl"
              style={{ background: "var(--raised)", border: "1px solid var(--border-strong)" }}
            >
              <div className="flex items-center h-9 px-3 text-[12px] font-medium" style={{ borderBottom: "1px solid var(--border)", color: "var(--fg2)" }}>
                Notifications
                {unread > 0 && (
                  <span className="num ml-1.5" style={{ color: "var(--fg3)" }}>
                    {unread} unread
                  </span>
                )}
              </div>
              <div className="max-h-80 overflow-auto">
                {orderedNotifications.length === 0 ? (
                  <div className="px-3 py-4 text-[12.5px]" style={{ color: "var(--fg3)" }}>
                    No notifications
                  </div>
                ) : (
                  orderedNotifications.map((row) => {
                    const n = row.notification;
                    const kind = notificationMeta(n.kind);
                    return (
                      <button
                        key={`${row.connection_id}:${n.id}`}
                        type="button"
                        onClick={() => openNotification(row)}
                        className="flex w-full items-center gap-2.5 px-3 h-10 text-left transition-colors"
                        style={{ borderBottom: "1px solid var(--border)", cursor: "pointer" }}
                        onMouseEnter={(e) => (e.currentTarget.style.background = "var(--hover)")}
                        onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                      >
                        <StatusGlyph glyph={kind.glyph} color={n.unread ? kind.color : "var(--fg3)"} title={kind.label} />
                        <span className="min-w-0 flex-1 truncate text-[12.5px]" style={{ color: n.unread ? "var(--fg)" : "var(--fg2)", fontWeight: n.unread ? 500 : 400 }}>
                          {n.title}
                        </span>
                        <span className="num shrink-0 text-[12px] whitespace-nowrap" style={{ color: "var(--fg3)" }}>
                          {relativeTime(n.updated_at)}
                        </span>
                      </button>
                    );
                  })
                )}
              </div>
              <button
                type="button"
                onClick={() => {
                  navigate("notifications");
                  setNotificationsOpen(false);
                }}
                className="w-full h-9 px-3 text-left text-[12.5px] font-medium transition-colors"
                style={{ color: "var(--fg2)" }}
                onMouseEnter={(e) => (e.currentTarget.style.background = "var(--hover)")}
                onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
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
