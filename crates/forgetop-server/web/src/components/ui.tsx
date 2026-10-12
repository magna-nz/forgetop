import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
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
 *  opens the in-app detail; otherwise `href` links out to the provider. */
export function Row({
  children,
  href,
  onClick,
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
  dense?: boolean;
  selected?: boolean;
  className?: string;
  label?: string;
}) {
  void _index;
  const cls = `ft-row ${dense ? "min-h-[32px]" : ""} ${className ?? ""}`;
  if (onClick) {
    return (
      <button type="button" className={cls} onClick={onClick} data-selected={selected || undefined} aria-label={label}>
        {children}
      </button>
    );
  }
  return (
    <a className={cls} href={href ?? undefined} target={href ? "_blank" : undefined} rel="noreferrer" data-selected={selected || undefined} aria-label={label}>
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
