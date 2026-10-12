// Small building blocks shared by the pipeline run pane's cards (PipelineDetail.tsx).
import type { CSSProperties, ReactNode } from "react";
import { relativeTime } from "../../format";
import type { AnnotationLevel, PipelineAnnotation, PipelineArtifact, PipelineRunStatus } from "../../types";

export const MONO_DIM = "color-mix(in srgb, var(--dim) 60%, transparent)";

/** A bordered card with an uppercase title row (`HISTORY · …` left, a count right). */
export function Card({
  title,
  right,
  children,
  style,
  className = "",
}: {
  title: ReactNode;
  right?: ReactNode;
  children: ReactNode;
  style?: CSSProperties;
  className?: string;
}) {
  return (
    <section
      className={`rounded-lg px-3.5 py-3 flex flex-col gap-1.5 ${className}`}
      style={{ background: "var(--card)", border: "1px solid var(--border)", ...style }}
    >
      <div className="flex items-center justify-between gap-3">
        <span className="text-[11px] font-semibold uppercase tracking-wider truncate" style={{ color: "var(--dim)" }}>
          {title}
        </span>
        {right != null && (
          <span className="text-xs shrink-0" style={{ color: "var(--dim)" }}>
            {right}
          </span>
        )}
      </div>
      {children}
    </section>
  );
}

/** The pane's small outlined action button. `title` carries the reason when it is disabled. */
export function PaneButton({
  label,
  onClick,
  disabled,
  color = "var(--fg)",
  filled = false,
  title,
  ariaLabel,
  pressed,
  neutral = false,
}: {
  label: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  color?: string;
  /** Solid fill in `color` (the primary action of a banner). */
  filled?: boolean;
  title?: string;
  ariaLabel?: string;
  /** A toggle's state (`aria-pressed`). */
  pressed?: boolean;
  /** Neutral panel background rather than a coloured outline. */
  neutral?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      aria-label={ariaLabel}
      aria-pressed={pressed}
      className="rounded-md px-2.5 py-1 text-xs font-medium whitespace-nowrap transition-opacity"
      style={{
        color: filled ? "var(--bg)" : disabled ? "var(--dim)" : color,
        background: filled ? color : neutral || pressed ? "var(--panel2)" : "transparent",
        border: `1px solid ${disabled ? "var(--border)" : neutral ? "var(--border)" : color}`,
        opacity: disabled ? 0.6 : 1,
        cursor: disabled ? "not-allowed" : "pointer",
        fontWeight: filled ? 600 : undefined,
      }}
    >
      {label}
    </button>
  );
}

/** A link styled like `PaneButton` (Open ↗, Open job ↗). */
export function PaneLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="rounded-md px-2.5 py-1 text-xs whitespace-nowrap"
      style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}
    >
      {children}
    </a>
  );
}

/** How a status reads in the pane's badge. */
export function statusWord(s: PipelineRunStatus): string {
  switch (s) {
    case "Waiting":
      return "Waiting for approval";
    case "PartiallySucceeded":
      return "Partially succeeded";
    default:
      return s;
  }
}

// ---- problems ----

const LEVEL_RANK: Record<AnnotationLevel, number> = { Failure: 0, Warning: 1, Notice: 2 };

export const levelMeta = (l: AnnotationLevel): { icon: string; color: string } =>
  l === "Failure" ? { icon: "✗", color: "var(--red)" } : l === "Warning" ? { icon: "⚠", color: "var(--yellow)" } : { icon: "i", color: "var(--dim)" };

/** Most severe first, keeping the provider's order within a level. */
export const sortAnnotations = (xs: PipelineAnnotation[]) => [...xs].sort((a, b) => LEVEL_RANK[a.level] - LEVEL_RANK[b.level]);

export const annotationLocation = (a: PipelineAnnotation): string | null =>
  a.path ? (a.line != null ? `${a.path}:${a.line}` : a.path) : null;

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** `1 error · 2 warnings`. */
export function problemCounts(xs: PipelineAnnotation[]): string {
  const n = (l: AnnotationLevel) => xs.filter((a) => a.level === l).length;
  return [
    n("Failure") ? plural(n("Failure"), "error", "errors") : null,
    n("Warning") ? plural(n("Warning"), "warning", "warnings") : null,
    n("Notice") ? plural(n("Notice"), "notice", "notices") : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** The run's reported problems; a row naming a job opens that job's log. */
export function ProblemsCard({ annotations, onOpenJob }: { annotations: PipelineAnnotation[]; onOpenJob: (jobId: string) => void }) {
  const sorted = sortAnnotations(annotations);
  return (
    <Card title="Problems" right={problemCounts(sorted)}>
      <div className="flex flex-col gap-0.5">
        {sorted.map((a, i) => {
          const m = levelMeta(a.level);
          const loc = annotationLocation(a);
          const message = (a.message.split("\n")[0] || a.title) ?? "";
          const body = (
            <>
              <span className="shrink-0" style={{ color: m.color }} aria-hidden="true">
                {m.icon}
              </span>
              {loc && (
                <span className="mono shrink-0" style={{ color: "var(--fg)", fontWeight: a.level === "Failure" ? 600 : undefined }}>
                  {loc}
                </span>
              )}
              <span className="min-w-0 truncate" style={{ color: m.color }}>
                {message}
              </span>
            </>
          );
          const cls = "flex items-baseline gap-2 w-full text-left rounded px-1.5 py-0.5 text-xs";
          return a.job_id ? (
            <button
              key={i}
              type="button"
              className={cls}
              title="Open this job's log"
              onClick={() => onOpenJob(a.job_id!)}
              onMouseEnter={(e) => (e.currentTarget.style.background = "var(--sel)")}
              onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
            >
              {body}
            </button>
          ) : (
            <div key={i} className={cls}>
              {body}
            </div>
          );
        })}
      </div>
    </Card>
  );
}

// ---- artifacts ----

/** `1.2 MB`, `640 KB`, `12 B`. */
export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u += 1;
  }
  return `${v >= 10 ? Math.round(v) : v.toFixed(1)} ${units[u]}`;
}

/** `expires in 3d` / `expired 2d ago`. */
export function expiryText(iso?: string | null): string | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  if (t <= Date.now()) return `expired ${relativeTime(iso)}`;
  const secs = Math.round((t - Date.now()) / 1000);
  const mins = Math.round(secs / 60);
  if (mins < 60) return `expires in ${Math.max(1, mins)}m`;
  const hours = Math.round(mins / 60);
  if (hours < 48) return `expires in ${hours}h`;
  return `expires in ${Math.round(hours / 24)}d`;
}

export function ArtifactsCard({
  artifacts,
  loading,
  error,
}: {
  artifacts: PipelineArtifact[] | undefined;
  loading: boolean;
  error: boolean;
}) {
  return (
    <Card title="Artifacts" right={artifacts ? plural(artifacts.length, "file", "files") : undefined}>
      {loading && <div className="text-xs" style={{ color: "var(--dim)" }}>Loading artifacts…</div>}
      {error && <div className="text-xs" style={{ color: "var(--red)" }}>Couldn't load artifacts.</div>}
      {artifacts && artifacts.length === 0 && <div className="text-xs" style={{ color: "var(--dim)" }}>This run published no artifacts.</div>}
      {artifacts?.map((a) => (
        <div key={a.id} className="flex items-baseline gap-3 text-xs">
          <span className="mono min-w-0 truncate" style={{ color: "var(--fg)" }}>
            {a.name}
          </span>
          {a.size_bytes != null && <span className="shrink-0" style={{ color: "var(--dim)" }}>{fmtBytes(a.size_bytes)}</span>}
          {expiryText(a.expires_at) && <span className="shrink-0" style={{ color: "var(--dim)" }}>{expiryText(a.expires_at)}</span>}
          <span className="flex-1" />
          {a.url && (
            <a href={a.url} target="_blank" rel="noreferrer" className="shrink-0" style={{ color: "var(--accent)" }}>
              Download ↗
            </a>
          )}
        </div>
      ))}
    </Card>
  );
}
