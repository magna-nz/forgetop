import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ButtonHTMLAttributes,
  type CSSProperties,
  type ReactNode,
  type SelectHTMLAttributes,
} from "react";
import { createPortal } from "react-dom";
import { initials, providerMeta, relativeTime, timelineMeta, type GlyphName } from "../format";
import type { ProviderType, TimelineEvent } from "../types";
import { Icon, type IconName } from "./icons";

export type { GlyphName };

/* ---- status ---------------------------------------------------------------------------- */

/** A status as a 14px stroke glyph. Shapes differ per state (see icons.tsx), colour comes from
 *  the matching `format.ts` meta. `title` makes it announced; otherwise pair it with a word. */
export function StatusGlyph({
  glyph,
  color,
  size = 14,
  spin = false,
  title,
  className,
  style,
}: {
  glyph: GlyphName;
  color?: string;
  size?: number;
  spin?: boolean;
  title?: string;
  className?: string;
  style?: CSSProperties;
}) {
  return <Icon name={glyph} size={size} spin={spin} title={title} className={className} style={{ color, ...style }} />;
}

/** Glyph + word, no box. `icon` (a text glyph) is still accepted for callers not yet moved to
 *  `glyph`; it renders in the same slot. */
export function Pill({
  icon,
  glyph,
  label,
  color,
  spin = false,
  title,
}: {
  icon?: string;
  glyph?: GlyphName;
  label: string;
  color: string;
  spin?: boolean;
  title?: string;
}) {
  return (
    <span className="inline-flex items-center gap-1.5 text-[12px] font-medium whitespace-nowrap" style={{ color }} title={title}>
      {glyph ? <StatusGlyph glyph={glyph} spin={spin} /> : icon ? <span className={spin ? "spin" : undefined}>{icon}</span> : null}
      {label}
    </span>
  );
}

/** The state word with its glyph. Shared by the lists and the Command Center so they read as
 *  one set. The tinted bordered chip this used to be is gone (DESIGN.md rule 2). */
export function StatusBadge({ label, color, glyph, title }: { label: string; color: string; glyph?: GlyphName; title?: string }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-[12px] font-medium whitespace-nowrap shrink-0" style={{ color }} title={title}>
      {glyph && <StatusGlyph glyph={glyph} />}
      {label}
    </span>
  );
}

/** A mono tag for identifiers: branches, SHAs, repository slugs, keys. */
export function Chip({ children, title, className }: { children: ReactNode; title?: string; className?: string }) {
  return (
    <span
      title={title}
      className={`mono inline-flex items-center gap-1 rounded h-5 px-1.5 text-[11.5px] whitespace-nowrap ${className ?? ""}`}
      style={{ background: "var(--active)", color: "var(--fg2)" }}
    >
      {children}
    </span>
  );
}

/* ---- buttons and controls -------------------------------------------------------------- */

export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return <kbd className={`ft-kbd ${className ?? ""}`}>{children}</kbd>;
}

export function Button({
  primary = false,
  quiet = false,
  danger = false,
  sm = false,
  icon,
  className,
  children,
  type = "button",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  primary?: boolean;
  quiet?: boolean;
  danger?: boolean;
  sm?: boolean;
  icon?: IconName;
}) {
  const cls = [
    "ft-btn",
    primary && "ft-btn-primary",
    quiet && "ft-btn-quiet",
    danger && "ft-btn-danger",
    sm && "ft-btn-sm",
    className,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <button type={type} className={cls} {...rest}>
      {icon && <Icon name={icon} size={13} />}
      {children}
    </button>
  );
}

/** Icon-only button; `label` is its accessible name and default tooltip. */
export function IconButton({
  icon,
  label,
  size = 15,
  className,
  title,
  type = "button",
  ...rest
}: Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> & { icon: IconName; label: string; size?: number }) {
  return (
    <button type={type} aria-label={label} title={title ?? label} className={`ft-btn ft-btn-icon ${className ?? ""}`} {...rest}>
      <Icon name={icon} size={size} />
    </button>
  );
}

/** A native <select> dressed as a quiet menu button. Stays a real select so keyboard, screen
 *  readers and tests keep working; the browser draws the open list. */
export function Select({ label, className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement> & { label?: string }) {
  return (
    <span className={`relative inline-flex items-center ${className ?? ""}`}>
      <select className="ft-select" aria-label={label} {...rest}>
        {children}
      </select>
      <Icon name="chevron-down" size={12} className="pointer-events-none absolute right-2" style={{ color: "var(--fg3)" }} />
    </span>
  );
}

export function Segmented<T extends string>({
  options,
  value,
  onChange,
  label,
}: {
  options: { value: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
  label?: string;
}) {
  return (
    <div className="ft-seg" role="group" aria-label={label}>
      {options.map((o) => (
        <button key={o.value} type="button" aria-pressed={o.value === value} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Tabs<T extends string>({
  tabs,
  value,
  onChange,
  label,
  className,
}: {
  tabs: { id: T; label: string; count?: ReactNode; countColor?: string }[];
  value: T;
  onChange: (id: T) => void;
  label?: string;
  className?: string;
}) {
  return (
    <div role="tablist" aria-label={label} className={`flex items-end ${className ?? ""}`}>
      {tabs.map((t) => (
        <button key={t.id} role="tab" type="button" aria-selected={t.id === value} className="ft-tab" onClick={() => onChange(t.id)}>
          {t.label}
          {t.count != null && (
            <span className="num ml-1.5 font-normal" style={{ color: t.countColor ?? "var(--fg3)" }}>
              {t.count}
            </span>
          )}
        </button>
      ))}
    </div>
  );
}

/* ---- headings and layout --------------------------------------------------------------- */

/** 14px/600 section title with a muted count. */
export function SectionHeading({ children, count, right, className }: { children: ReactNode; count?: ReactNode; right?: ReactNode; className?: string }) {
  return (
    <div className={`flex items-baseline gap-2 ${className ?? ""}`}>
      <h2 className="m-0 text-[14px] font-semibold" style={{ color: "var(--fg)" }}>
        {children}
      </h2>
      {count != null && (
        <span className="num text-[13px]" style={{ color: "var(--fg3)" }}>
          {count}
        </span>
      )}
      {right && <span className="ml-auto flex items-center gap-2">{right}</span>}
    </div>
  );
}

/** 12px/500 group label with a hairline — a sub-group inside a section or a list. */
export function GroupHeading({ children, count, right, className }: { children: ReactNode; count?: ReactNode; right?: ReactNode; className?: string }) {
  return (
    <div
      className={`flex items-center gap-2 h-7 text-[12px] font-medium ${className ?? ""}`}
      style={{ color: "var(--fg2)", borderBottom: "1px solid var(--border)" }}
    >
      {children}
      {count != null && (
        <span className="num font-normal" style={{ color: "var(--fg3)" }}>
          {count}
        </span>
      )}
      {right && <span className="ml-auto flex items-center gap-2">{right}</span>}
    </div>
  );
}

export function Divider({ className }: { className?: string }) {
  return <div className={className} style={{ height: 1, background: "var(--border)" }} />;
}

/** A list row: 36px, hairline below, hover tint, accent wash when selected. A click handler
 *  opens the in-app detail; otherwise `href` links out to the provider. `onContextMenu` and
 *  `onKeyDown` take a `useContextMenu()`'s handlers; `menuOpen` keeps the hover tint (and the
 *  row's `RowMenuButton`) showing while that menu is open. */
export function Row({
  children,
  href,
  onClick,
  onContextMenu,
  onKeyDown,
  menuOpen = false,
  dense = false,
  selected = false,
  className,
  label,
  // Kept for callers that still pass a stagger index; rows no longer animate in.
  index: _index,
}: {
  children: ReactNode;
  index?: number;
  href?: string | null;
  onClick?: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  onKeyDown?: (e: React.KeyboardEvent) => void;
  menuOpen?: boolean;
  dense?: boolean;
  selected?: boolean;
  className?: string;
  label?: string;
}) {
  void _index;
  const cls = `ft-row ${dense ? "min-h-[32px]" : ""} ${className ?? ""}`;
  const menuProps = { onContextMenu, onKeyDown, "data-menu-open": menuOpen ? "true" : undefined };
  if (onClick) {
    return (
      <button type="button" className={cls} onClick={onClick} data-selected={selected || undefined} aria-label={label} {...menuProps}>
        {children}
      </button>
    );
  }
  return (
    <a
      className={cls}
      href={href ?? undefined}
      target={href ? "_blank" : undefined}
      rel="noreferrer"
      data-selected={selected || undefined}
      aria-label={label}
      {...menuProps}
    >
      {children}
    </a>
  );
}

/** The list container: full width inside the content column, rows draw their own hairlines. */
export function List({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={`flex flex-col w-full max-w-[1280px] mx-auto px-6 pb-6 ${className ?? ""}`}>{children}</div>;
}

export function StateCard({ icon, title, sub, action }: { icon?: ReactNode; title: string; sub?: string; action?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-1.5 py-20 text-center" style={{ color: "var(--fg3)" }}>
      {icon && <div className="text-2xl mb-1 opacity-70">{icon}</div>}
      <div className="text-[14px] font-medium" style={{ color: "var(--fg)" }}>
        {title}
      </div>
      {sub && (
        <div className="text-[13px] max-w-sm" style={{ color: "var(--fg2)" }}>
          {sub}
        </div>
      )}
      {action && <div className="mt-3">{action}</div>}
    </div>
  );
}

export function Skeleton({ rows = 6 }: { rows?: number }) {
  return (
    <List>
      {Array.from({ length: rows }).map((_, i) => (
        <div key={i} className="flex items-center gap-3 h-9 pulse" style={{ borderBottom: "1px solid var(--border)" }}>
          <span className="rounded-full" style={{ width: 14, height: 14, background: "var(--active)" }} />
          <span className="rounded" style={{ width: `${34 + ((i * 17) % 40)}%`, height: 10, background: "var(--active)" }} />
          <span className="ml-auto rounded" style={{ width: 36, height: 10, background: "var(--active)" }} />
        </div>
      ))}
    </List>
  );
}

/* ---- detail pane ----------------------------------------------------------------------- */

/** Where `SlideOver` renders: the column to the right of the page content. `App` provides the
 *  element; without one (tests, storybook-style renders) the pane renders in place. */
export const PaneSlotContext = createContext<HTMLElement | null>(null);

/** One pane at a time: each detail provider registers how to close its pane, and opening any
 *  pane closes the others first. A pane may also register a guard that can veto leaving it
 *  (the PR pane asks before discarding unsubmitted comments). */
const paneClosers = new Map<string, () => void>();
let paneGuard: { kind: string; canLeave: () => boolean } | null = null;

export function usePaneRegistration(kind: string, close: () => void) {
  useEffect(() => {
    paneClosers.set(kind, close);
    return () => {
      paneClosers.delete(kind);
      if (paneGuard?.kind === kind) paneGuard = null;
    };
  }, [kind, close]);
}

/** Registers (or clears, with `null`) the open pane's leave guard. */
export function setPaneGuard(kind: string, canLeave: (() => boolean) | null) {
  paneGuard = canLeave ? { kind, canLeave } : null;
}

/** Called by an opener before it shows `kind`: asks the open pane's guard, then closes the
 *  other kinds. Returns false when the guard refused and nothing should open. */
export function claimPane(kind: string): boolean {
  if (paneGuard && !paneGuard.canLeave()) return false;
  paneGuard = null;
  for (const [k, close] of paneClosers) if (k !== kind) close();
  return true;
}

/** The pane's narrowest useful width, and the least the content column keeps beside it. */
const PANE_MIN = 360;
const CONTENT_MIN = 320;
const PANE_STEP = 24;
const paneWidthKey = (size: string) => `forgetop:pane:${size}`;

/** The pane's right edge and the widest it may go: the content column beside it (whatever is
 *  left of the sidebar) keeps `CONTENT_MIN`. Without layout (tests) the viewport stands in. */
function paneBounds(pane: HTMLElement | null): { right: number; max: number } {
  const rect = pane?.getBoundingClientRect();
  const right = rect && rect.right > 0 ? rect.right : window.innerWidth;
  // pane → slot → the row holding the content column and the slot.
  const row = pane?.parentElement?.parentElement;
  const left = row?.getBoundingClientRect().left ?? 0;
  return { right, max: Math.max(PANE_MIN, right - left - CONTENT_MIN) };
}

function clampPaneWidth(px: number, max: number): number {
  return Math.round(Math.min(max, Math.max(PANE_MIN, px)));
}

function storedPaneWidth(size: string): number | null {
  try {
    const v = Number(localStorage.getItem(paneWidthKey(size)));
    return Number.isFinite(v) && v > 0 ? v : null;
  } catch {
    return null;
  }
}

/** The pane's width when the user has dragged it (else null for the size's CSS default), and
 *  the grip on its left edge that sets it: drag, ←/→ by 24px, double-click to reset. The width
 *  is remembered per pane size, so a wider PR pane does not widen the work-item one. */
function usePaneWidth(size: "md" | "lg" | "xl") {
  const [px, setPx] = useState<number | null>(() => storedPaneWidth(size));
  const [dragging, setDragging] = useState(false);
  const paneRef = useRef<HTMLElement | null>(null);
  useEffect(() => setPx(storedPaneWidth(size)), [size]);

  const commit = (next: number | null) => {
    setPx(next);
    try {
      if (next == null) localStorage.removeItem(paneWidthKey(size));
      else localStorage.setItem(paneWidthKey(size), String(next));
    } catch {
      /* per-viewer convenience only */
    }
  };
  const current = () => px ?? paneRef.current?.getBoundingClientRect().width ?? PANE_MIN;

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button > 0) return; // secondary buttons; a synthetic event without one still drags
    e.preventDefault();
    // The pane hugs the right edge, so its width is the distance from the pointer to it.
    const { right, max } = paneBounds(paneRef.current);
    const target = e.currentTarget;
    target.setPointerCapture?.(e.pointerId);
    setDragging(true);
    let last = current();
    const move = (ev: PointerEvent) => {
      if (!Number.isFinite(ev.clientX)) return;
      last = clampPaneWidth(right - ev.clientX, max);
      setPx(last);
    };
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      target.removeEventListener("pointercancel", up);
      setDragging(false);
      commit(last);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
    target.addEventListener("pointercancel", up);
  };
  // The pane hugs the right edge, so ← expands it and → shrinks it.
  const resizeKey = (key: string): boolean => {
    const { max } = paneBounds(paneRef.current);
    if (key === "ArrowLeft") commit(clampPaneWidth(current() + PANE_STEP, max));
    else if (key === "ArrowRight") commit(clampPaneWidth(current() - PANE_STEP, max));
    else if (key === "Home") commit(max);
    else if (key === "End") commit(PANE_MIN);
    else return false;
    return true;
  };
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (resizeKey(e.key)) e.preventDefault();
  };
  // ←/→ resize the open pane from anywhere on the page that is not a text field or a control
  // with its own arrow keys (tabs, menus, lists), so the footer hint is true wherever focus is.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      const t = e.target as HTMLElement | null;
      if (t && (t.closest("input, textarea, select, [contenteditable=''], [contenteditable='true'], [role='tablist'], [role='listbox'], [role='menu'], [role='separator']"))) return;
      if (resizeKey(e.key)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [px, size]);

  const grip = (
    <div
      ref={(el) => {
        paneRef.current = (el?.parentElement as HTMLElement | null) ?? null;
      }}
      className="ft-pane-grip"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize pane"
      aria-valuemin={PANE_MIN}
      aria-valuenow={px ?? undefined}
      tabIndex={0}
      title="Drag to resize · double-click to reset"
      data-dragging={dragging || undefined}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      onDoubleClick={() => commit(null)}
    />
  );
  return { px, grip };
}

/** The right-hand detail pane, beside the content (never a modal). Esc and ✕ close it. `header`
 *  is the title block (see `PaneHeader`), `footer` the action bar. */
export function SlideOver({
  onClose,
  header,
  children,
  footer,
  width = "md",
  label = "Details",
}: {
  onClose: () => void;
  header: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  /** `3xl`/`4xl`/`5xl` are the pre-redesign names and map to md/lg/xl. */
  width?: "md" | "lg" | "xl" | "3xl" | "4xl" | "5xl";
  label?: string;
}) {
  const slot = useContext(PaneSlotContext);
  const size = width === "3xl" ? "md" : width === "4xl" ? "lg" : width === "5xl" ? "xl" : width;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  const { px, grip } = usePaneWidth(size);

  const pane = (
    <aside className={`ft-pane ft-pane-${size} fade-in`} role="region" aria-label={label} style={px != null ? { width: px } : undefined}>
      {grip}
      <div className="flex items-start gap-3 px-5 pt-4 pb-3 shrink-0" style={{ borderBottom: "1px solid var(--border)" }}>
        <div className="flex-1 min-w-0">{header}</div>
        <IconButton icon="x" label="Close" title="Close (Esc)" onClick={onClose} className="-mr-2 -mt-1" />
      </div>
      <div className="flex-1 overflow-auto min-h-0">{children}</div>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5 px-5 py-2.5 shrink-0" style={{ borderTop: "1px solid var(--border)", background: "var(--panel)" }}>
        <span className="inline-flex max-[900px]:hidden items-center gap-1.5 text-[12px] shrink-0 mr-2" style={{ color: "var(--fg3)" }} aria-hidden="true">
          <Kbd>←</Kbd> expand <Kbd>→</Kbd> shrink
        </span>
        {footer}
      </div>
    </aside>
  );
  return slot ? createPortal(pane, slot) : pane;
}

/** The standard pane title block: glyph, 17px title, mono identifier, optional second line. */
export function PaneHeader({
  glyph,
  color,
  title,
  ident,
  actions,
  children,
}: {
  glyph?: GlyphName;
  color?: string;
  title: ReactNode;
  ident?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="min-w-0">
      <div className="flex items-center gap-2.5 min-w-0">
        {glyph && <StatusGlyph glyph={glyph} color={color} size={16} />}
        <h2 className="m-0 text-[17px] font-semibold leading-tight truncate" style={{ letterSpacing: "-0.012em" }}>
          {title}
        </h2>
        {ident && (
          <span className="mono shrink-0" style={{ color: "var(--fg3)" }}>
            {ident}
          </span>
        )}
        {actions && <span className="ml-auto flex items-center gap-1 shrink-0">{actions}</span>}
      </div>
      {children && (
        <div className="flex items-center gap-2 flex-wrap mt-1.5 text-[12.5px]" style={{ color: "var(--fg2)" }}>
          {children}
        </div>
      )}
    </div>
  );
}

/* ---- people and providers -------------------------------------------------------------- */

export function Avatar({ name, size = 20 }: { name: string; size?: number }) {
  // Deterministic hue from the name so the same person keeps the same colour.
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) % 360;
  return (
    <span
      title={name}
      className="inline-flex items-center justify-center rounded-full font-semibold shrink-0"
      style={{
        width: size,
        height: size,
        fontSize: Math.max(8, size * 0.42),
        background: `hsl(${h} 28% 34%)`,
        color: `hsl(${h} 40% 88%)`,
      }}
    >
      {initials(name)}
    </span>
  );
}

export function ProviderBadge({ provider, connection }: { provider: ProviderType; connection: string }) {
  const meta = providerMeta(provider);
  return (
    <span className="inline-flex items-center gap-1.5 text-[12px]" title={connection}>
      <span className="inline-block w-1.5 h-1.5 rounded-full shrink-0" style={{ background: meta.color }} />
      <span style={{ color: "var(--fg3)" }}>{connection}</span>
    </span>
  );
}

/** The event timeline (approvals, merges, comments, state changes, …), newest first. Shared by
 *  the PR and work-item panes; events arrive oldest→newest and are shown with the newest on top. */
export function Timeline({ events }: { events: TimelineEvent[] }) {
  const ordered = [...events].reverse();
  return (
    <div className="flex flex-col">
      <GroupHeading>Timeline</GroupHeading>
      {ordered.map((e, i) => {
        const m = timelineMeta(e.kind);
        return (
          <div key={i} className="flex items-center gap-2.5 h-7 text-[12.5px]" style={{ borderBottom: "1px solid var(--border)" }}>
            <StatusGlyph glyph={m.glyph} color={e.kind === "Commented" ? "var(--fg3)" : m.color} />
            <span className="font-medium shrink-0" style={{ color: "var(--fg)" }}>
              {e.actor?.display_name ?? "Someone"}
            </span>
            <span className="truncate" style={{ color: "var(--fg2)" }}>
              {e.summary}
            </span>
            <span className="num ml-auto shrink-0 text-[12px]" style={{ color: "var(--fg3)" }}>
              {e.at ? relativeTime(e.at) : ""}
            </span>
          </div>
        );
      })}
    </div>
  );
}

/* ---- context menus --------------------------------------------------------------------- */

/** One line of a row's context menu. Group order is navigate+copy · act · view preference ·
 *  destructive, with a separator between groups; leading, trailing and doubled separators are
 *  dropped, so a builder can emit a group's separator without checking the group is empty. */
export type MenuEntry =
  | { type: "separator" }
  | {
      type?: "item";
      /** Stable key; defaults to the label when it is a string. */
      id?: string;
      label: ReactNode;
      icon?: IconName;
      /** Colour for the icon only (a status colour var); default currentColor. */
      iconColor?: string;
      /** Trailing mono text (an identifier, a branch). Rendered in --fg2, truncated. */
      value?: string;
      /** Keyboard hint shown right-aligned in mono, e.g. "↵", "o", "y", "D", "X". */
      hint?: string;
      /** A string disables the item and becomes its tooltip (the reason). */
      disabled?: string | false;
      /** Red text + icon; confirm-in-place when `confirm` is set. */
      danger?: boolean;
      /** Label shown after the first click; the second click within 4s runs onSelect. */
      confirm?: string;
      /** Renders role=menuitemcheckbox with a checkbox glyph. */
      checked?: boolean;
      /** Submenu items, or a loader called when the submenu opens (shows "Loading…" meanwhile,
       *  "Couldn't load" on rejection). The entry gets a chevron and aria-haspopup. */
      submenu?: MenuEntry[] | (() => Promise<MenuEntry[]>);
      /** Runs on activation; the menu closes first unless `keepOpen`. */
      onSelect?: () => void | Promise<void>;
      keepOpen?: boolean;
    };

type MenuItemEntry = Exclude<MenuEntry, { type: "separator" }>;

export interface MenuAnchor {
  /** Viewport coordinates of the pointer, or an element to anchor below-left of. */
  x?: number;
  y?: number;
  el?: HTMLElement | null;
  /** The element the menu was opened from (the row); focus returns there on close unless
   *  `ContextMenu` is given `returnFocusTo`. `useContextMenu` fills it in. */
  origin?: HTMLElement | null;
}

/** Headless state for one row's menu. */
export function useContextMenu(): {
  open: boolean;
  anchor: MenuAnchor | null;
  /** Attach to the row: `onContextMenu={menu.onContextMenu}`; prevents the native menu. */
  onContextMenu: (e: React.MouseEvent) => void;
  /** Attach to the row: `onKeyDown={menu.onKeyDown}` — Shift+F10 / ContextMenu key. */
  onKeyDown: (e: React.KeyboardEvent) => void;
  /** Open anchored to an element (the ⋯ button). */
  openAt: (el: HTMLElement) => void;
  close: () => void;
} {
  const [anchor, setAnchor] = useState<MenuAnchor | null>(null);
  const onContextMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setAnchor({ x: e.clientX, y: e.clientY, origin: e.currentTarget as HTMLElement });
  }, []);
  const onKeyDown = useCallback((e: React.KeyboardEvent) => {
    if ((e.shiftKey && e.key === "F10") || e.key === "ContextMenu") {
      e.preventDefault();
      e.stopPropagation();
      const row = e.currentTarget as HTMLElement;
      setAnchor({ el: row, origin: row });
    }
  }, []);
  const openAt = useCallback((el: HTMLElement) => setAnchor({ el, origin: el }), []);
  const close = useCallback(() => setAnchor(null), []);
  return { open: anchor != null, anchor, onContextMenu, onKeyDown, openAt, close };
}

/** The floating menu. Renders nothing when `anchor` is null. Portals to document.body. */
export function ContextMenu(props: {
  anchor: MenuAnchor | null;
  onClose: () => void;
  /** Built lazily on open so a row never pays for a closed menu. */
  items: MenuEntry[] | (() => MenuEntry[]);
  /** aria-label of the menu, e.g. "Pull request actions". */
  label: string;
  /** Element to return focus to on close (the row); defaults to the previously focused one. */
  returnFocusTo?: HTMLElement | null;
}): ReactNode {
  if (!props.anchor) return null;
  return createPortal(<MenuRoot {...props} anchor={props.anchor} />, document.body);
}

const isSeparator = (e: MenuEntry | undefined): e is { type: "separator" } => e?.type === "separator";

/** Drops leading, trailing and doubled separators. */
function tidyEntries(entries: MenuEntry[]): MenuEntry[] {
  const out: MenuEntry[] = [];
  for (const e of entries) {
    if (isSeparator(e)) {
      if (out.length > 0 && !isSeparator(out[out.length - 1])) out.push(e);
    } else out.push(e);
  }
  while (isSeparator(out[out.length - 1])) out.pop();
  return out;
}

const itemIndexes = (entries: MenuEntry[]) => entries.flatMap((e, i) => (isSeparator(e) ? [] : [i]));
const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Runs an item's action; a throw or rejection becomes an error toast rather than vanishing. */
function runAction(fn: (() => void | Promise<void>) | undefined) {
  if (!fn) return;
  try {
    const r = fn();
    if (r && typeof r.then === "function") r.then(undefined, (e: unknown) => notify(errorText(e), { error: true }));
  } catch (e) {
    notify(errorText(e), { error: true });
  }
}

const MENU_EDGE = 8;
const SUBMENU_HOVER_MS = 120;
const CONFIRM_MS = 4000;

type MenuPlace = { anchor: MenuAnchor } | { beside: () => HTMLElement | null | undefined };

/** Where a `w`×`h` panel goes: at the pointer, below-left of an element, or to the right of a
 *  parent item — flipped up/left when it would overflow, then clamped inside the viewport. */
function placeMenu(w: number, h: number, place: MenuPlace): { left: number; top: number } {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  let left: number;
  let top: number;
  if ("beside" in place) {
    const r = place.beside()?.getBoundingClientRect();
    if (!r) return { left: MENU_EDGE, top: MENU_EDGE };
    // 5px up: the panel's 1px border and 4px inset line its first item up with the parent.
    left = r.right + 2;
    top = r.top - 5;
    if (left + w > vw - MENU_EDGE) left = r.left - w - 2;
  } else if (place.anchor.el) {
    const r = place.anchor.el.getBoundingClientRect();
    left = r.left;
    top = r.bottom + 4;
    if (left + w > vw - MENU_EDGE) left = r.right - w;
    if (top + h > vh - MENU_EDGE) top = r.top - h - 4;
  } else {
    left = place.anchor.x ?? 0;
    top = place.anchor.y ?? 0;
    if (left + w > vw - MENU_EDGE) left -= w;
    if (top + h > vh - MENU_EDGE) top -= h;
  }
  const clamp = (v: number, max: number) => Math.max(MENU_EDGE, Math.min(v, max));
  return { left: clamp(left, vw - w - MENU_EDGE), top: clamp(top, vh - h - MENU_EDGE) };
}

/** The open menu: the root panel plus any submenu, the close-on-outside listeners, and
 *  returning focus when it goes. Mounted only while open, so `items()` runs once per open (and
 *  again only when the caller re-renders with a new `items`). */
function MenuRoot({
  anchor,
  onClose,
  items,
  label,
  returnFocusTo,
}: {
  anchor: MenuAnchor;
  onClose: () => void;
  items: MenuEntry[] | (() => MenuEntry[]);
  label: string;
  returnFocusTo?: HTMLElement | null;
}) {
  const layerRef = useRef<HTMLDivElement>(null);
  const entries = useMemo(() => tidyEntries(typeof items === "function" ? items() : items), [items]);
  // Read during the first render, before the menu takes focus.
  const [previous] = useState(() => (document.activeElement instanceof HTMLElement ? document.activeElement : null));
  const focusTarget = returnFocusTo ?? anchor.origin ?? previous;
  const focusTargetRef = useRef(focusTarget);
  focusTargetRef.current = focusTarget;
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  // Whether closing hands focus back. A click elsewhere or the window losing focus does not:
  // the user has put focus somewhere else on purpose.
  const restoreRef = useRef(true);
  const close = useCallback((restore: boolean) => {
    restoreRef.current = restore;
    onCloseRef.current();
  }, []);
  // Which panel holds focus (0 = root): the one roving tab stop lives there.
  const [focusDepth, setFocusDepth] = useState(0);

  useEffect(() => {
    const inside = (t: EventTarget | null) => t instanceof Node && !!layerRef.current?.contains(t);
    const onDown = (e: MouseEvent) => {
      if (!inside(e.target)) close(false);
    };
    const onScroll = (e: Event) => {
      if (!inside(e.target)) close(inside(document.activeElement));
    };
    const onResize = () => close(inside(document.activeElement));
    const onBlur = () => close(false);
    document.addEventListener("mousedown", onDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    window.addEventListener("blur", onBlur);
    return () => {
      document.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
      window.removeEventListener("blur", onBlur);
    };
  }, [close]);

  useEffect(
    () => () => {
      const t = focusTargetRef.current;
      if (restoreRef.current && t && t.isConnected && t !== document.body) t.focus({ preventScroll: true });
    },
    [],
  );

  // The menu is rendered from inside a row in the React tree: keep its events from bubbling
  // there (a click would open the pane), and from the window listeners (Esc closes the pane,
  // ←/→ resize it).
  const stop = (e: React.SyntheticEvent) => e.stopPropagation();
  return (
    <div
      ref={layerRef}
      className="ft-menu-layer"
      onClick={stop}
      onMouseDown={stop}
      onPointerDown={stop}
      onKeyDown={stop}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
      }}
    >
      <MenuPanel
        entries={entries}
        label={label}
        depth={0}
        place={{ anchor }}
        autoFocus={1}
        focusDepth={focusDepth}
        setFocusDepth={setFocusDepth}
        close={close}
      />
    </div>
  );
}

type SubmenuState = {
  index: number;
  /** Bumped to move focus into the submenu; 0 while it is only hover-opened. */
  focusReq: number;
} & ({ state: "loading" } | { state: "error"; error: string } | { state: "ready"; entries: MenuEntry[] });

/** One `role="menu"` panel (the root or a submenu) with its roving focus, keyboard model,
 *  confirm-in-place and its own open submenu. */
function MenuPanel({
  entries,
  label,
  depth,
  place,
  autoFocus,
  focusDepth,
  setFocusDepth,
  close,
  onBack,
  onPanelEnter,
}: {
  entries: MenuEntry[];
  label: string;
  depth: number;
  place: MenuPlace;
  /** Non-zero: the active item takes focus once the panel is placed (again on each change). */
  autoFocus: number;
  focusDepth: number;
  setFocusDepth: (depth: number) => void;
  close: (restore: boolean) => void;
  /** ← in a submenu: close it and focus its parent item. */
  onBack?: () => void;
  /** The pointer reached this submenu: the parent keeps it open. */
  onPanelEnter?: () => void;
}) {
  const uid = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  const itemEls = useRef<(HTMLButtonElement | null)[]>([]);
  const indexes = itemIndexes(entries);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [activeState, setActive] = useState(indexes[0] ?? -1);
  const active = indexes.includes(activeState) ? activeState : (indexes[0] ?? -1);
  const [armed, setArmed] = useState<number | null>(null);
  const [sub, setSub] = useState<SubmenuState | null>(null);
  const subRef = useRef(sub);
  subRef.current = sub;
  const focusDepthRef = useRef(focusDepth);
  focusDepthRef.current = focusDepth;
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const armTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const loaded = useRef(new Map<number, MenuEntry[]>());
  const subToken = useRef(0);

  const clearHover = () => {
    if (hoverTimer.current !== undefined) clearTimeout(hoverTimer.current);
    hoverTimer.current = undefined;
  };
  useEffect(
    () => () => {
      clearHover();
      if (armTimer.current !== undefined) clearTimeout(armTimer.current);
    },
    [],
  );

  // Measure after render and place; until then the panel is laid out but hidden, so it never
  // flashes at 0,0. Width is max-content, so the measurement does not depend on where it sits.
  useLayoutEffect(() => {
    const el = panelRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const next = placeMenu(r.width, r.height, place);
    setPos((cur) => (cur && cur.left === next.left && cur.top === next.top ? cur : next));
  });
  const placed = pos !== null;
  // A hidden element cannot take focus, so focus waits for placement.
  useLayoutEffect(() => {
    if (placed && autoFocus > 0) itemEls.current[active]?.focus({ preventScroll: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [placed, autoFocus]);

  const focusIndex = (i: number) => {
    if (i < 0) return;
    setActive(i);
    itemEls.current[i]?.focus({ preventScroll: true });
  };

  const settle = (index: number, token: number, next: SubmenuState) => {
    if (subToken.current !== token) return;
    // Focus follows the content in only if it was already inside the submenu.
    const into = focusDepthRef.current > depth;
    setSub((s) => (s && s.index === index ? { ...next, focusReq: into ? s.focusReq + 1 : 0 } : s));
  };

  const openSub = (i: number, focus: boolean) => {
    clearHover();
    const e = entries[i];
    if (!e || isSeparator(e) || !e.submenu || e.disabled) return;
    const cur = subRef.current;
    if (cur && cur.index === i) {
      if (focus) setSub({ ...cur, focusReq: cur.focusReq + 1 });
      return;
    }
    const token = ++subToken.current;
    const focusReq = focus ? 1 : 0;
    const source = e.submenu;
    const cached = Array.isArray(source) ? source : loaded.current.get(i);
    if (cached) {
      setSub({ index: i, focusReq, state: "ready", entries: cached });
      return;
    }
    if (Array.isArray(source)) return;
    setSub({ index: i, focusReq, state: "loading" });
    source().then(
      (list) => {
        loaded.current.set(i, list);
        settle(i, token, { index: i, focusReq: 0, state: "ready", entries: list });
      },
      (err: unknown) => settle(i, token, { index: i, focusReq: 0, state: "error", error: errorText(err) || "Couldn't load" }),
    );
  };

  const closeSub = () => {
    clearHover();
    subToken.current++;
    setSub(null);
  };

  const move = (to: number) => {
    if (indexes.length === 0) return;
    closeSub();
    focusIndex(indexes[(to + indexes.length) % indexes.length]);
  };

  const activate = (i: number, fromKeyboard: boolean) => {
    const e = entries[i];
    if (!e || isSeparator(e) || e.disabled) return;
    if (e.submenu) {
      openSub(i, fromKeyboard);
      return;
    }
    if (e.confirm && armed !== i) {
      setArmed(i);
      if (armTimer.current !== undefined) clearTimeout(armTimer.current);
      armTimer.current = setTimeout(() => setArmed(null), CONFIRM_MS);
      return;
    }
    setArmed(null);
    if (!e.keepOpen) close(true);
    runAction(e.onSelect);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const at = indexes.indexOf(active);
    const entry = entries[active];
    const item = entry && !isSeparator(entry) ? entry : null;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        move(at + 1);
        return;
      case "ArrowUp":
        e.preventDefault();
        move(at - 1);
        return;
      case "Home":
        e.preventDefault();
        move(0);
        return;
      case "End":
        e.preventDefault();
        move(indexes.length - 1);
        return;
      case "ArrowRight":
        e.preventDefault();
        if (item?.submenu && !item.disabled) openSub(active, true);
        return;
      case "ArrowLeft":
        e.preventDefault();
        onBack?.();
        return;
      case "Escape":
      case "Tab":
        e.preventDefault();
        close(true);
        return;
    }
    // Enter and Space reach the focused <button> as a native click.
    if (e.key.length !== 1 || e.key === " " || e.metaKey || e.ctrlKey || e.altKey) return;
    // Type-ahead: the item whose hint is that key, else the first whose label starts with it.
    const byHint = indexes.find((i) => (entries[i] as MenuItemEntry).hint === e.key);
    const key = e.key.toLowerCase();
    const byLabel = indexes.find((i) =>
      (itemEls.current[i]?.querySelector(".ft-menu-label")?.textContent ?? "").trim().toLowerCase().startsWith(key),
    );
    const to = byHint ?? byLabel;
    if (to !== undefined) {
      e.preventDefault();
      if (to !== active) closeSub();
      focusIndex(to);
    }
  };

  const onItemEnter = (i: number) => {
    focusIndex(i);
    clearHover();
    const e = entries[i] as MenuItemEntry;
    if (e.submenu && !e.disabled) {
      if (subRef.current?.index !== i) hoverTimer.current = setTimeout(() => openSub(i, false), SUBMENU_HOVER_MS);
    } else if (subRef.current) {
      hoverTimer.current = setTimeout(closeSub, SUBMENU_HOVER_MS);
    }
  };

  let subPanel: ReactNode = null;
  if (sub) {
    const parent = entries[sub.index];
    if (parent && !isSeparator(parent)) {
      const live = Array.isArray(parent.submenu) ? parent.submenu : null;
      const subEntries: MenuEntry[] =
        sub.state === "loading"
          ? [{ id: "loading", label: "Loading…", disabled: "Loading…" }]
          : sub.state === "error"
            ? [{ id: "error", label: "Couldn't load", disabled: sub.error }]
            : (() => {
                const list = tidyEntries(live ?? sub.entries);
                return list.length > 0 ? list : [{ id: "empty", label: "Nothing to choose", disabled: "Nothing to choose" }];
              })();
      const index = sub.index;
      subPanel = (
        <MenuPanel
          key={`${index}:${sub.state}`}
          entries={subEntries}
          label={typeof parent.label === "string" ? parent.label : "Submenu"}
          depth={depth + 1}
          place={{ beside: () => itemEls.current[index] }}
          autoFocus={sub.focusReq}
          focusDepth={focusDepth}
          setFocusDepth={setFocusDepth}
          close={close}
          onBack={() => {
            closeSub();
            focusIndex(index);
          }}
          onPanelEnter={clearHover}
        />
      );
    }
  }

  return (
    <>
      <div
        ref={panelRef}
        role="menu"
        aria-label={label}
        aria-orientation="vertical"
        className="ft-menu"
        data-depth={depth}
        style={pos ? { left: pos.left, top: pos.top } : { left: 0, top: 0, visibility: "hidden" }}
        onKeyDown={onKeyDown}
        onMouseEnter={onPanelEnter}
      >
        {entries.map((entry, i) => {
          if (isSeparator(entry)) return <div key={`sep-${i}`} role="separator" className="ft-menu-sep" />;
          const disabled = entry.disabled || null;
          const isArmed = armed === i && !!entry.confirm;
          const hasSub = !!entry.submenu;
          const checkbox = entry.checked !== undefined;
          // A disabled item reads as disabled: the status colour is dropped with the action.
          const iconColor = disabled || entry.danger ? undefined : entry.iconColor;
          const valueId = entry.value ? `${uid}-v${i}` : undefined;
          return (
            <button
              key={`${i}:${entry.id ?? (typeof entry.label === "string" ? entry.label : "")}`}
              ref={(el) => {
                itemEls.current[i] = el;
              }}
              type="button"
              role={checkbox ? "menuitemcheckbox" : "menuitem"}
              aria-checked={checkbox ? entry.checked : undefined}
              aria-disabled={disabled ? "true" : undefined}
              aria-haspopup={hasSub ? "menu" : undefined}
              aria-expanded={hasSub ? sub?.index === i : undefined}
              aria-keyshortcuts={entry.hint}
              aria-describedby={valueId}
              title={disabled ?? undefined}
              tabIndex={i === active && focusDepth === depth ? 0 : -1}
              className="ft-menu-item"
              data-danger={entry.danger || undefined}
              data-armed={isArmed || undefined}
              onClick={(ev) => activate(i, ev.detail === 0)}
              onFocus={() => {
                setActive(i);
                setFocusDepth(depth);
              }}
              onMouseEnter={() => onItemEnter(i)}
            >
              <span className="ft-menu-icon" aria-hidden="true">
                {entry.icon ? (
                  <Icon name={entry.icon} size={14} style={iconColor ? { color: iconColor } : undefined} />
                ) : checkbox ? (
                  <span className="ft-menu-check">{entry.checked && <Icon name="check" size={10} strokeWidth={2} />}</span>
                ) : null}
              </span>
              <span className="ft-menu-label">{isArmed ? entry.confirm : entry.label}</span>
              {entry.value && (
                // Out of the name (the label is the name) but still the description.
                <span id={valueId} className="ft-menu-value mono" aria-hidden="true">
                  {entry.value}
                </span>
              )}
              {checkbox && entry.icon && entry.checked && <Icon name="check" size={13} className="ft-menu-tick" />}
              {entry.hint && (
                <span className="ft-menu-hint" aria-hidden="true">
                  {entry.hint}
                </span>
              )}
              {hasSub && <Icon name="chevron-right" size={12} className="ft-menu-sub" />}
            </button>
          );
        })}
        {entries.length === 0 && (
          <div className="ft-menu-empty" role="none">
            No actions
          </div>
        )}
      </div>
      {subPanel}
    </>
  );
}

/** The ⋯ affordance at a row's right edge. Hidden until the row is hovered/focused or the menu
 *  is open (CSS). Clicking it calls `onOpen(buttonEl)` and stops propagation so the row's own
 *  click (open the pane) does not fire.
 *
 *  It is a `<span role="button" tabIndex={0}>` rather than a `<button>` because it renders inside
 *  the row, which is itself a `<button>` (or an `<a>`), and interactive content nested in either
 *  is invalid HTML; so it reproduces Enter/Space activation by hand, and stops those events (and
 *  the click) from propagating — and prevents the click's default, so an `<a>` row does not
 *  navigate — because the row would otherwise treat them as its own activation. */
export function RowMenuButton({ onOpen, label = "More actions" }: { onOpen: (el: HTMLElement) => void; label?: string }): ReactNode {
  return (
    <span
      role="button"
      // Out of the tab order and the accessibility tree: it is the pointer's way to the menu a
      // keyboard user opens with Shift+F10 / the Menu key, and a second tab stop per row (plus
      // "More actions" on every row's name) would cost more than it gives.
      tabIndex={-1}
      aria-hidden="true"
      title={label}
      className="ft-row-menu"
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onOpen(e.currentTarget);
      }}
      onKeyDown={(e) => {
        if (e.key !== "Enter" && e.key !== " ") return;
        e.preventDefault();
        e.stopPropagation();
        onOpen(e.currentTarget);
      }}
    >
      <Icon name="dots" size={14} />
    </span>
  );
}

/* ---- toasts ---------------------------------------------------------------------------- */

type Toast = { id: number; message: string; error: boolean };
const TOAST_MS = 4000;
const TOAST_ERROR_MS = 8000;
const TOAST_MAX = 4;
let toasts: Toast[] = [];
let toastSeq = 0;
const toastListeners = new Set<() => void>();
const emitToasts = () => toastListeners.forEach((l) => l());
const subscribeToasts = (l: () => void) => {
  toastListeners.add(l);
  return () => {
    toastListeners.delete(l);
  };
};
const toastSnapshot = () => toasts;

/** Shows a quiet bottom-left note for 4s (8s and red for errors). */
export function notify(message: string, opts?: { error?: boolean }): void {
  const error = !!opts?.error;
  const id = ++toastSeq;
  toasts = [...toasts, { id, message, error }].slice(-TOAST_MAX);
  emitToasts();
  setTimeout(
    () => {
      toasts = toasts.filter((t) => t.id !== id);
      emitToasts();
    },
    error ? TOAST_ERROR_MS : TOAST_MS,
  );
}

/** Mounted once in App.tsx, beside the pane slot. Newest at the bottom. */
export function Toasts(): ReactNode {
  const list = useSyncExternalStore(subscribeToasts, toastSnapshot, toastSnapshot);
  return (
    <div className="ft-toasts">
      {list.map((t) => (
        <div key={t.id} role={t.error ? "alert" : "status"} className="ft-toast" data-error={t.error || undefined}>
          {t.message}
        </div>
      ))}
    </div>
  );
}

/** Copies text and notifies "Copied <what>" or the failure. The async clipboard API is tried
 *  first; where it is missing, or refuses (an embedded or unfocused page denies the write
 *  permission), the selection-based copy is the fallback before giving up. */
export async function copyText(text: string, what: string): Promise<void> {
  try {
    try {
      if (!navigator.clipboard?.writeText) throw new Error("no clipboard API");
      await navigator.clipboard.writeText(text);
    } catch {
      copyWithTextarea(text);
    }
    notify(`Copied ${what}`);
  } catch (e) {
    notify(`Couldn't copy ${what}: ${errorText(e)}`, { error: true });
  }
}

/** Where the async clipboard API is missing or refused: copy through a selected, off-screen
 *  textarea. */
function copyWithTextarea(text: string) {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.setAttribute("readonly", "");
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  try {
    if (!document.execCommand("copy")) throw new Error("the browser refused");
  } finally {
    ta.remove();
  }
}
