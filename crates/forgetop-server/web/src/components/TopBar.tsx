import { useIsFetching, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useCacheStatus, useInbox, useNotificationDetail } from "../api";
import { notificationMeta, shortAge, toTime } from "../format";
import { useNavigateSection } from "../nav";
import type { InboxThread, SectionId } from "../types";
import { Icon } from "./icons";
import { quotable, useInboxActions } from "./inboxActions";
import { quote } from "./TriageStrip";
import { Button, IconButton, Kbd, StatusGlyph } from "./ui";

/** How many threads the bell lists before "Open inbox →". */
const BELL_MAX = 8;

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
  const { data: inbox } = useInbox();
  const navigate = useNavigateSection();
  const actions = useInboxActions();
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  const notificationRef = useRef<HTMLDivElement>(null);
  // The inbox is what asks for you: the badge counts its unread threads, and the popover lists
  // them, read or not, newest first.
  const forYou = [...(inbox ?? [])].sort((a, b) => toTime(b.latest) - toTime(a.latest));
  const unread = forYou.filter((t) => t.unread).length;

  // Clicking a thread opens its item at the target, marks it read, and closes the popover — the
  // same `open` a row on the Notifications page makes, walking the For-you list.
  const openThread = (t: InboxThread) => {
    actions.open(t, forYou);
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
              <div className="flex items-center gap-1.5 h-9 px-3 text-[12px] font-medium" style={{ borderBottom: "1px solid var(--border)", color: "var(--fg2)" }}>
                For you
                <span className="num font-normal" style={{ color: "var(--fg3)" }}>
                  {unread}
                </span>
                {forYou.length > 0 && (
                  <Button quiet sm className="ml-auto -mr-1.5" onClick={() => void actions.doneAll(forYou)} title="Dismiss every thread here">
                    Dismiss all
                  </Button>
                )}
              </div>
              <div className="max-h-96 overflow-auto">
                {forYou.length === 0 ? (
                  <div className="px-3 py-4 text-[12.5px]" style={{ color: "var(--fg3)" }}>
                    Nothing needs you
                  </div>
                ) : (
                  forYou.slice(0, BELL_MAX).map((t) => <BellItem key={t.key} thread={t} onOpen={() => openThread(t)} />)
                )}
              </div>
              <div className="flex items-center h-9 px-3 text-[12px]" style={{ color: "var(--fg3)" }}>
                <button
                  type="button"
                  onClick={() => {
                    navigate("notifications");
                    setNotificationsOpen(false);
                  }}
                  className="ml-auto h-7 px-2 -mr-2 rounded text-[12.5px] font-medium transition-colors"
                  style={{ color: "var(--fg2)" }}
                  onMouseEnter={(e) => (e.currentTarget.style.background = "var(--hover)")}
                  onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                >
                  Open inbox →
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </header>
  );
}

/** One inbox thread in the bell: glyph, title and age; then why it's here, muted. */
function BellItem({ thread, onOpen }: { thread: InboxThread; onOpen: () => void }) {
  const lead = thread.lead;
  const n = lead.notification;
  const kind = notificationMeta(n.kind);
  // Never fetched from here (eight rows would be eight calls); a detail the page or a pane
  // already fetched is used.
  const detail = useNotificationDetail(lead.connection_id, n.id, false);
  const actor = n.actor ?? detail.data?.actor ?? null;
  const excerpt = quotable(n) ? (n.excerpt ?? detail.data?.excerpt ?? null) : null;
  const said = excerpt ? ` · ${quote(excerpt, 60)}` : actor && !lead.phrase.includes(actor.split(/\s+/)[0]) ? ` · ${actor}` : "";
  return (
    <button
      type="button"
      onClick={onOpen}
      className="flex w-full flex-col gap-0.5 px-3 py-2 text-left transition-colors"
      style={{ borderBottom: "1px solid var(--border)", cursor: "pointer" }}
      onMouseEnter={(e) => (e.currentTarget.style.background = "var(--hover)")}
      onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
    >
      <span className="flex w-full items-center gap-2.5 min-w-0">
        <StatusGlyph glyph={kind.glyph} color={thread.unread ? kind.color : "var(--fg3)"} title={kind.label} />
        <span
          className="min-w-0 flex-1 truncate text-[13px]"
          style={{ color: thread.unread ? "var(--fg)" : "var(--fg2)", fontWeight: thread.unread ? 600 : 400 }}
        >
          {n.title}
        </span>
        <span className="num shrink-0 text-[12px] whitespace-nowrap" style={{ color: "var(--fg3)" }}>
          {shortAge(thread.latest)}
        </span>
      </span>
      <span className="truncate pl-[24px] text-[12px]" style={{ color: "var(--fg3)" }} title={lead.why}>
        {lead.phrase}
        {said}
      </span>
    </button>
  );
}
