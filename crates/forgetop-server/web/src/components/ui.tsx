import { motion } from "framer-motion";
import type { ReactNode } from "react";
import { initials, providerMeta } from "../format";
import type { ProviderType } from "../types";

export function Avatar({ name, size = 22 }: { name: string; size?: number }) {
  // Deterministic hue from the name so the same person keeps the same colour.
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) % 360;
  return (
    <span
      title={name}
      className="mono inline-flex items-center justify-center rounded-full font-semibold shrink-0"
      style={{
        width: size,
        height: size,
        fontSize: size * 0.4,
        background: `hsl(${h} 40% 30%)`,
        color: `hsl(${h} 60% 82%)`,
      }}
    >
      {initials(name)}
    </span>
  );
}

export function ProviderBadge({ provider, connection }: { provider: ProviderType; connection: string }) {
  const meta = providerMeta(provider);
  return (
    <span className="inline-flex items-center gap-1.5 text-xs" title={connection}>
      <span className="inline-block w-1.5 h-1.5 rounded-full shrink-0" style={{ background: meta.color }} />
      <span style={{ color: "var(--dim)" }}>{connection}</span>
    </span>
  );
}

export function Pill({ icon, label, color, spin = false }: { icon: string; label: string; color: string; spin?: boolean }) {
  return (
    <span
      className="inline-flex items-center gap-1.5 rounded-md px-2 py-0.5 text-xs font-medium whitespace-nowrap"
      style={{ color, background: "color-mix(in srgb, " + color + " 14%, transparent)" }}
    >
      <span className={spin ? "spin" : undefined}>{icon}</span>
      {label}
    </span>
  );
}

export function Chip({ children, title }: { children: ReactNode; title?: string }) {
  return (
    <span
      title={title}
      className="mono inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-xs"
      style={{ background: "var(--panel2)", color: "var(--dim)", border: "1px solid var(--border)" }}
    >
      {children}
    </span>
  );
}

export function Row({ children, index = 0, href }: { children: ReactNode; index?: number; href?: string | null }) {
  return (
    <motion.a
      href={href ?? undefined}
      target={href ? "_blank" : undefined}
      rel="noreferrer"
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.22, delay: Math.min(index * 0.02, 0.3), ease: "easeOut" }}
      className="group block rounded-lg px-4 py-3 transition-colors"
      style={{ background: "var(--card)", border: "1px solid var(--border)", cursor: href ? "pointer" : "default" }}
      onMouseEnter={(e) => (e.currentTarget.style.background = "var(--card-hover)")}
      onMouseLeave={(e) => (e.currentTarget.style.background = "var(--card)")}
    >
      {children}
    </motion.a>
  );
}

export function List({ children }: { children: ReactNode }) {
  return <div className="flex flex-col gap-2 p-5 max-w-5xl mx-auto">{children}</div>;
}

export function StateCard({ icon, title, sub }: { icon: string; title: string; sub?: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-24 text-center" style={{ color: "var(--dim)" }}>
      <div className="text-4xl opacity-60">{icon}</div>
      <div className="text-base" style={{ color: "var(--fg)" }}>
        {title}
      </div>
      {sub && <div className="text-sm max-w-sm">{sub}</div>}
    </div>
  );
}

export function Skeleton() {
  return (
    <List>
      {Array.from({ length: 5 }).map((_, i) => (
        <div
          key={i}
          className="rounded-lg px-4 py-3 pulse"
          style={{ background: "var(--card)", border: "1px solid var(--border)", height: 68 }}
        />
      ))}
    </List>
  );
}
