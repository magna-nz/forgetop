import type { CSSProperties } from "react";

/** The dashboard's icon set: 16-unit stroke paths, drawn at 14–16px. Status glyphs differ in
 *  shape, not only colour, so colour-blind users and the light theme keep the distinctions. */
export type IconName =
  // navigation
  | "inbox"
  | "pull-request"
  | "circle-dot"
  | "workflow"
  | "settings"
  // actions
  | "search"
  | "refresh"
  | "bell"
  | "sidebar"
  | "chevron-down"
  | "chevron-right"
  | "external"
  | "x"
  | "plus"
  | "check"
  | "comment"
  | "at"
  | "arrow-right"
  | "user"
  | "user-plus"
  | "copy"
  | "link"
  | "dots"
  | "pencil"
  | "shield"
  | "logs"
  | "rerun"
  | "square"
  | "minus-circle"
  | "git-branch"
  | "open-pane"
  // status
  | "pr-open"
  | "pr-draft"
  | "pr-merged"
  | "pr-closed"
  | "check-circle"
  | "x-circle"
  | "running"
  | "queued"
  | "clock"
  | "warning"
  | "half"
  | "circle"
  | "blocked"
  | "skipped"
  | "dot"
  | "review";

const PATHS: Record<IconName, string> = {
  inbox: '<path d="M2.5 9 4 3.5h8L13.5 9v4H2.5z"/><path d="M2.5 9h3l1.3 2h2.4l1.3-2h3"/>',
  "pull-request":
    '<circle cx="4" cy="3.5" r="1.75"/><circle cx="4" cy="12.5" r="1.75"/><circle cx="12" cy="12.5" r="1.75"/><path d="M4 5.25v5.5M12 10.75V7a2 2 0 0 0-2-2H8.5M10 3 8 5l2 2"/>',
  "circle-dot": '<circle cx="8" cy="8" r="6"/><circle cx="8" cy="8" r="1.6" fill="currentColor" stroke="none"/>',
  workflow: '<rect x="2" y="2" width="5" height="5" rx="1.2"/><rect x="9" y="9" width="5" height="5" rx="1.2"/><path d="M7 4.5h3.5a1 1 0 0 1 1 1V9"/>',
  settings:
    '<circle cx="8" cy="8" r="2.2"/><path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4"/>',
  search: '<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5 14 14"/>',
  refresh: '<path d="M13 8a5 5 0 1 1-1.5-3.6"/><path d="M13 2v3h-3"/>',
  bell: '<path d="M4 11V7a4 4 0 0 1 8 0v4l1 1.5H3z"/><path d="M6.5 14a1.5 1.5 0 0 0 3 0"/>',
  sidebar: '<rect x="2" y="3" width="12" height="10" rx="1.5"/><path d="M6 3v10"/>',
  "chevron-down": '<path d="M4 6l4 4 4-4"/>',
  "chevron-right": '<path d="M6 4l4 4-4 4"/>',
  external: '<path d="M6 3H3.5A1.5 1.5 0 0 0 2 4.5v8A1.5 1.5 0 0 0 3.5 14h8a1.5 1.5 0 0 0 1.5-1.5V10M9.5 2H14v4.5M14 2 7.5 8.5"/>',
  x: '<path d="M4 4l8 8M12 4l-8 8"/>',
  plus: '<path d="M8 3v10M3 8h10"/>',
  check: '<path d="M3.5 8.5l2.5 2.5 6.5-6.5"/>',
  comment: '<path d="M2.5 3.5h11v7h-6l-3 2.5v-2.5h-2z"/>',
  at: '<circle cx="8" cy="8" r="2.5"/><path d="M10.5 8v1a1.5 1.5 0 0 0 3 0V8a5.5 5.5 0 1 0-2.2 4.4"/>',
  "arrow-right": '<path d="M3 8h10M9 4l4 4-4 4"/>',
  user: '<circle cx="8" cy="5.5" r="2.75"/><path d="M2.75 14a5.25 5.25 0 0 1 10.5 0"/>',
  "user-plus": '<circle cx="6.5" cy="5.5" r="2.5"/><path d="M2 13.5a4.5 4.5 0 0 1 9 0"/><path d="M12.5 5v4M10.5 7h4"/>',
  copy: '<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 5.5V4A1.5 1.5 0 0 0 9 2.5H4A1.5 1.5 0 0 0 2.5 4v5A1.5 1.5 0 0 0 4 10.5h1.5"/>',
  link: '<path d="M6.8 9.2a2.6 2.6 0 0 0 3.7 0l2.2-2.2a2.6 2.6 0 0 0-3.7-3.7l-.9.9"/><path d="M9.2 6.8a2.6 2.6 0 0 0-3.7 0L3.3 9a2.6 2.6 0 0 0 3.7 3.7l.9-.9"/>',
  dots: '<circle cx="3.5" cy="8" r="1.15" fill="currentColor" stroke="none"/><circle cx="8" cy="8" r="1.15" fill="currentColor" stroke="none"/><circle cx="12.5" cy="8" r="1.15" fill="currentColor" stroke="none"/>',
  pencil: '<path d="M10.5 2.5l3 3L6 13H3v-3z"/><path d="M9 4l3 3"/>',
  shield: '<path d="M8 1.75 13 3.5v4c0 3.2-2.1 5.6-5 6.75C5.1 13.1 3 10.7 3 7.5v-4z"/>',
  logs: '<rect x="2.5" y="2.5" width="11" height="11" rx="1.5"/><path d="M5 6h6M5 8.5h6M5 11h3.5"/>',
  // A run arrow round a play mark: distinct from `refresh` (reload the list) on purpose.
  rerun: '<path d="M3 8a5 5 0 1 0 1.5-3.6"/><path d="M3 2v3h3"/><path d="M7 6.2v3.6L10 8z"/>',
  square: '<rect x="4" y="4" width="8" height="8" rx="1.5"/>',
  "minus-circle": '<circle cx="8" cy="8" r="6"/><path d="M5.5 8h5"/>',
  "git-branch":
    '<circle cx="4.5" cy="3.5" r="1.5"/><circle cx="4.5" cy="12.5" r="1.5"/><circle cx="11.5" cy="5" r="1.5"/><path d="M4.5 5v6M11.5 6.5c0 3-7 2-7 4.5"/>',
  "open-pane": '<rect x="2" y="3" width="12" height="10" rx="1.5"/><path d="M9.5 3v10"/>',
  "pr-open":
    '<circle cx="4" cy="3.5" r="1.75"/><circle cx="4" cy="12.5" r="1.75"/><circle cx="12" cy="12.5" r="1.75"/><path d="M4 5.25v5.5M12 10.75V7a2 2 0 0 0-2-2H8.5M10 3 8 5l2 2"/>',
  "pr-draft":
    '<circle cx="4" cy="3.5" r="1.75"/><circle cx="4" cy="12.5" r="1.75"/><circle cx="12" cy="12.5" r="1.75"/><path d="M4 5.25v5.5M12 10.75v-1M12 7.5v-1"/>',
  "pr-merged":
    '<circle cx="4" cy="3.5" r="1.75"/><circle cx="4" cy="12.5" r="1.75"/><circle cx="12" cy="8" r="1.75"/><path d="M4 5.25v5.5M4 10.5c0-3 2.5-2.5 6.25-2.5"/>',
  "pr-closed":
    '<circle cx="4" cy="3.5" r="1.75"/><circle cx="4" cy="12.5" r="1.75"/><path d="M4 5.25v5.5M10 4l4 4M14 4l-4 4"/>',
  "check-circle": '<circle cx="8" cy="8" r="6"/><path d="M5.5 8.2l1.7 1.7L10.8 6.4"/>',
  "x-circle": '<circle cx="8" cy="8" r="6"/><path d="M6 6l4 4M10 6l-4 4"/>',
  running: '<circle cx="8" cy="8" r="6" stroke-dasharray="3 2.4"/>',
  queued: '<circle cx="8" cy="8" r="6"/><path d="M5.5 8h5" stroke-dasharray="1.5 1.5"/>',
  clock: '<circle cx="8" cy="8" r="6"/><path d="M8 4.5V8l2.3 1.4"/>',
  warning: '<path d="M8 2.5 14 13H2z"/><path d="M8 6.5v3M8 11.3v.2"/>',
  half: '<circle cx="8" cy="8" r="6"/><path d="M8 2a6 6 0 0 1 0 12z" fill="currentColor" stroke="none"/>',
  circle: '<circle cx="8" cy="8" r="6"/>',
  blocked: '<circle cx="8" cy="8" r="6"/><path d="M4 4l8 8"/>',
  skipped: '<circle cx="8" cy="8" r="6"/><path d="M6 5.5 10 8l-4 2.5z" fill="currentColor" stroke="none"/>',
  dot: '<circle cx="8" cy="8" r="2.5" fill="currentColor" stroke="none"/>',
  review: '<circle cx="8" cy="8" r="6"/><path d="M5 8.5l2 2 4-4.5"/>',
};

export function Icon({
  name,
  size = 14,
  className,
  style,
  spin = false,
  title,
  strokeWidth = 1.6,
}: {
  name: IconName;
  size?: number;
  className?: string;
  style?: CSSProperties;
  spin?: boolean;
  /** When given, the icon is announced; otherwise it is decorative. */
  title?: string;
  strokeWidth?: number;
}) {
  const cls = [spin ? "spin" : "", className ?? ""].filter(Boolean).join(" ") || undefined;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={cls}
      style={{ flexShrink: 0, display: "inline-block", verticalAlign: "-2px", ...style }}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      data-icon={name}
      dangerouslySetInnerHTML={{ __html: PATHS[name] }}
    />
  );
}
