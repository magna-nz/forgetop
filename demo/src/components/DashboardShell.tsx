import type { ReactNode, RefObject } from "react";

export type DemoSection = "launchpad" | "pull-requests" | "work-items" | "pipelines";

export const DEMO_FEEDBACK_URL =
  "https://github.com/magna-nz/forgetop/issues/new?template=feedback.yml";

const navItems: { id: DemoSection; label: string; icon: string }[] = [
  { id: "launchpad", label: "Command Center", icon: "✦" },
  { id: "pull-requests", label: "Pull Requests", icon: "⇄" },
  { id: "work-items", label: "Work Items", icon: "◧" },
  { id: "pipelines", label: "Pipelines", icon: "⛓" },
];

export function DemoSidebar({
  section,
  onSectionChange,
  counts = {},
  onReset,
}: {
  section: DemoSection;
  onSectionChange: (section: DemoSection) => void;
  counts?: Partial<Record<DemoSection, number>>;
  onReset: () => void;
}) {
  return (
    <aside className="demo-sidebar">
      <div className="demo-brand-row">
        <span className="demo-logo" aria-hidden="true">◿</span>
        <span className="demo-brand">forgetop</span>
        <span className="demo-version">DEMO</span>
      </div>

      <nav className="demo-navigation" aria-label="Dashboard sections">
        {navItems.map((item) => {
          const active = item.id === section;
          const count = counts[item.id];
          return (
            <button
              key={item.id}
              className={`demo-nav-item${active ? " is-active" : ""}`}
              type="button"
              aria-current={active ? "page" : undefined}
              onClick={() => onSectionChange(item.id)}
            >
              <span aria-hidden="true">{item.icon}</span>
              <span className="demo-nav-label">{item.label}</span>
              {count != null && count > 0 && <span className="demo-nav-count">{count}</span>}
            </button>
          );
        })}
      </nav>

      <div className="demo-sidebar-bottom">
        <a
          className="demo-feedback-link"
          href={DEMO_FEEDBACK_URL}
          target="_blank"
          rel="noopener noreferrer"
        >
          <span aria-hidden="true">◇</span>
          <span>Give Feedback</span>
          <span className="demo-link-arrow" aria-hidden="true">↗</span>
        </a>
        <button className="demo-reset-link" type="button" onClick={onReset}>
          <span aria-hidden="true">↺</span>
          Reset demo
        </button>
      </div>

      <div className="demo-connection" aria-label="5 of 5 connections healthy">
        <span className="demo-healthy-dot" aria-hidden="true" />
        5/5 connections healthy
        <span className="demo-connection-note">Simulated · refresh resets</span>
      </div>
    </aside>
  );
}

export function DemoTopBar({
  search,
  onSearchChange,
  notificationCount = 0,
  onNotifications,
  theme = "slate",
  onThemeToggle,
  searchInputRef,
}: {
  search: string;
  onSearchChange: (value: string) => void;
  notificationCount?: number;
  onNotifications?: () => void;
  theme?: string;
  onThemeToggle?: () => void;
  searchInputRef?: RefObject<HTMLInputElement>;
}) {
  return (
    <header className="demo-topbar">
      <label className="demo-search">
        <span aria-hidden="true">⌕</span>
        <input
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          ref={searchInputRef}
          placeholder="Search your work…"
          aria-label="Search your work"
        />
        <kbd>⌘K</kbd>
      </label>
      <div className="demo-topbar-actions">
        <button className="demo-icon-button" type="button" onClick={onNotifications} aria-label="Notifications">
          <span aria-hidden="true">♧</span>
          {notificationCount > 0 && <span className="demo-notification-count">{notificationCount}</span>}
        </button>
        <button className="demo-theme-button" type="button" onClick={onThemeToggle} title="Switch theme">
          ◐ {theme}
        </button>
      </div>
    </header>
  );
}

export function DemoNotice() {
  return (
    <div className="demo-notice" role="note">
      <span aria-hidden="true">◇</span>
      <span><strong>Interactive public demo.</strong> Sample data only—nothing here connects to your accounts or persists after refresh.</span>
    </div>
  );
}

export function DashboardShell({ sidebar, topbar, children }: { sidebar: ReactNode; topbar: ReactNode; children: ReactNode }) {
  return (
    <main className="demo-shell">
      <div className="demo-app-frame">
        {sidebar}
        <div className="demo-main-area">
          {topbar}
          <div className="demo-page-scroll">{children}</div>
        </div>
      </div>
    </main>
  );
}
