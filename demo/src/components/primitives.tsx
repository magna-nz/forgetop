import { useEffect, useRef, type ReactNode } from "react";

export function PageHeader({ eyebrow, title, description, actions }: { eyebrow?: string; title: string; description?: string; actions?: ReactNode }) {
  return <div className="demo-page-header">
    <div>
      {eyebrow && <p className="demo-eyebrow">{eyebrow}</p>}
      <h1>{title}</h1>
      {description && <p className="demo-subhead">{description}</p>}
    </div>
    {actions && <div className="demo-header-actions">{actions}</div>}
  </div>;
}

export function SectionCard({ title, action, children, className = "" }: { title?: string; action?: ReactNode; children: ReactNode; className?: string }) {
  return <section className={`demo-section-card ${className}`}>
    {(title || action) && <div className="demo-section-card-heading">
      {title && <h2>{title}</h2>}
      {action && <div>{action}</div>}
    </div>}
    {children}
  </section>;
}

export function StatCard({ label, value, detail, tone = "accent", onClick }: { label: string; value: string | number; detail?: string; tone?: "accent" | "green" | "yellow" | "red" | "blue"; onClick?: () => void }) {
  const content = <>
    <span className="demo-stat-label">{label}</span>
    <strong className={`demo-stat-value is-${tone}`}>{value}</strong>
    {detail && <span className="demo-stat-detail">{detail}</span>}
  </>;
  return onClick ? <button type="button" className="demo-stat-card" onClick={onClick}>{content}</button> : <div className="demo-stat-card">{content}</div>;
}

export function List({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`demo-list ${className}`}>{children}</div>;
}

export function ListRow({ leading, title, subtitle, meta, badge, onClick, children }: { leading?: ReactNode; title: ReactNode; subtitle?: ReactNode; meta?: ReactNode; badge?: ReactNode; onClick?: () => void; children?: ReactNode }) {
  const body = <>
    {leading && <div className="demo-row-leading">{leading}</div>}
    <div className="demo-row-main"><div className="demo-row-title">{title}</div>{subtitle && <div className="demo-row-subtitle">{subtitle}</div>}</div>
    {badge && <div className="demo-row-badge">{badge}</div>}
    {meta && <div className="demo-row-meta">{meta}</div>}
    {children && <div className="demo-row-extra">{children}</div>}
  </>;
  return onClick ? <button type="button" className="demo-list-row" onClick={onClick}>{body}</button> : <div className="demo-list-row">{body}</div>;
}

export function StatusBadge({ children, tone = "neutral" }: { children: ReactNode; tone?: "neutral" | "green" | "yellow" | "red" | "blue" | "purple" }) {
  return <span className={`demo-status-badge is-${tone}`}>{children}</span>;
}

export function Chip({ children }: { children: ReactNode }) {
  return <span className="demo-chip">{children}</span>;
}

export function DetailDrawer({ open, title, subtitle, onClose, children, footer, wide = false }: { open: boolean; title: string; subtitle?: string; onClose: () => void; children: ReactNode; footer?: ReactNode; wide?: boolean }) {
  const drawerRef = useRef<HTMLElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return;
    previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const focusable = () => Array.from(drawerRef.current?.querySelectorAll<HTMLElement>("button, [href], input, select, textarea, [tabindex]:not([tabindex='-1'])") ?? []).filter((element) => !element.hasAttribute("disabled"));
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") { onClose(); return; }
      if (event.key !== "Tab") return;
      const controls = focusable();
      if (controls.length === 0) { event.preventDefault(); return; }
      const first = controls[0]; const last = controls[controls.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
      if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
    };
    const closeButton = drawerRef.current?.querySelector<HTMLElement>("button[aria-label='Close detail']");
    closeButton?.focus();
    document.addEventListener("keydown", onKeyDown);
    return () => { document.removeEventListener("keydown", onKeyDown); previousFocus.current?.focus(); };
  }, [open, onClose]);

  if (!open) return null;
  return <div className="demo-drawer-layer" role="presentation">
    <button className="demo-drawer-backdrop" type="button" aria-label="Close detail" onClick={onClose} />
    <aside ref={drawerRef} className={`demo-detail-drawer${wide ? " is-wide" : ""}`} role="dialog" aria-modal="true" aria-label={title}>
      <header className="demo-drawer-heading">
        <div><h2>{title}</h2>{subtitle && <p>{subtitle}</p>}</div>
        <button className="demo-icon-button" type="button" onClick={onClose} aria-label="Close detail">×</button>
      </header>
      <div className="demo-drawer-content">{children}</div>
      {footer && <footer className="demo-drawer-footer">{footer}</footer>}
    </aside>
  </div>;
}

export function EmptyState({ title, description, action }: { title: string; description?: string; action?: ReactNode }) {
  return <div className="demo-empty-state"><span aria-hidden="true">◇</span><h2>{title}</h2>{description && <p>{description}</p>}{action}</div>;
}
