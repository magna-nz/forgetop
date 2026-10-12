// Small building blocks shared by the pipeline run pane's sections (PipelineDetail.tsx).
import type { CSSProperties, ReactNode } from "react";
import { relativeTime, type GlyphName } from "../../format";
import type { AnnotationLevel, PipelineAnnotation, PipelineArtifact, PipelineRunStatus } from "../../types";
import { Icon } from "../icons";
import { Button, GroupHeading, StatusGlyph } from "../ui";

/** Line numbers and other quiet mono furniture. */
export const MONO_DIM = "color-mix(in srgb, var(--fg3) 60%, transparent)";

/** A pane section: a 12px group heading with a hairline (`Problems` left, a count right), then
 *  its rows. No box — the pane is the surface. */
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
    <section className={`flex flex-col gap-1 ${className}`} style={style}>
      <GroupHeading
        right={
          right != null ? (
            <span className="font-normal" style={{ color: "var(--fg3)" }}>
              {right}
            </span>
          ) : undefined
        }
      >
        <span className="truncate">{title}</span>
      </GroupHeading>
      {children}
    </section>
  );
}

/** The pane's small action button, on the shared `ft-btn` classes: `filled` is the primary,
 *  `neutral` the quiet one, otherwise outlined. `color` tints the label (red for "Jump to
 *  error"). `title` carries the reason when it is disabled. */
export function PaneButton({
  label,
  onClick,
  disabled,
  color,
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
  /** The primary action of a banner. */
  filled?: boolean;
  title?: string;
  ariaLabel?: string;
  /** A toggle's state (`aria-pressed`). */
  pressed?: boolean;
  /** Quiet rather than outlined. */
  neutral?: boolean;
}) {
  return (
    <Button
      sm
      primary={filled}
      quiet={neutral && !filled}
      onClick={onClick}
      disabled={disabled}
      title={title}
      aria-label={ariaLabel}
      aria-pressed={pressed}
      style={{
        ...(color && !filled ? { color } : {}),
        ...(pressed ? { background: "var(--active)", color: color ?? "var(--fg)" } : {}),
      }}
    >
      {label}
    </Button>
  );
}

/** A link styled like a quiet `PaneButton`, with the external glyph (Open job). */
export function PaneLink({ href, children, title }: { href: string; children: ReactNode; title?: string }) {
  return (
    <a href={href} target="_blank" rel="noreferrer" title={title} className="ft-btn ft-btn-quiet ft-btn-sm">
      {children}
      <Icon name="external" size={12} />
    </a>
  );
}

/** An icon-only link styled as `IconButton` (the pane header's "open at the provider"). */
export function ExternalIconLink({ href, label }: { href: string; label: string }) {
  return (
    <a href={href} target="_blank" rel="noreferrer" aria-label={label} title={label} className="ft-btn ft-btn-icon">
      <Icon name="external" size={15} />
    </a>
  );
}

/** The colour of a status as a bar (stage strip, history strip): the status colours, with
 *  anything that did not run in the neutral track colour. */
export function statusBarColor(s: PipelineRunStatus): string {
  switch (s) {
    case "Succeeded":
      return "var(--green)";
    case "Running":
      return "var(--blue)";
    case "Failed":
      return "var(--red)";
    case "Waiting":
    case "PartiallySucceeded":
      return "var(--yellow)";
    default:
      return "var(--active)";
  }
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

export const levelMeta = (l: AnnotationLevel): { glyph: GlyphName; color: string } =>
  l === "Failure"
    ? { glyph: "x-circle", color: "var(--red)" }
    : l === "Warning"
      ? { glyph: "warning", color: "var(--yellow)" }
      : { glyph: "dot", color: "var(--fg3)" };

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
      <div className="flex flex-col">
        {sorted.map((a, i) => {
          const m = levelMeta(a.level);
          const loc = annotationLocation(a);
          const message = (a.message.split("\n")[0] || a.title) ?? "";
          const body = (
            <>
              <StatusGlyph glyph={m.glyph} color={m.color} className="self-center" />
              {loc && (
                <span className="mono shrink-0" style={{ color: "var(--fg)", fontWeight: a.level === "Failure" ? 600 : undefined }}>
                  {loc}
                </span>
              )}
              <span className="min-w-0 truncate" style={{ color: a.level === "Failure" ? "var(--red)" : "var(--fg2)" }}>
                {message}
              </span>
            </>
          );
          const cls = "flex items-baseline gap-2 w-full text-left px-1.5 py-1 text-[12.5px] rounded-[5px]";
          return a.job_id ? (
            <button
              key={i}
              type="button"
              className={`${cls} hover:bg-[var(--hover)]`}
              title="Open this job's log"
              onClick={() => onOpenJob(a.job_id!)}
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
      {loading && <div className="text-[12.5px] py-1" style={{ color: "var(--fg3)" }}>Loading artifacts…</div>}
      {error && <div className="text-[12.5px] py-1" style={{ color: "var(--red)" }}>Couldn't load artifacts.</div>}
      {artifacts && artifacts.length === 0 && <div className="text-[12.5px] py-1" style={{ color: "var(--fg3)" }}>This run published no artifacts.</div>}
      {artifacts?.map((a) => (
        <div key={a.id} className="flex items-center gap-3 h-7 text-[12.5px]" style={{ borderBottom: "1px solid var(--border)" }}>
          <span className="mono min-w-0 truncate" style={{ color: "var(--fg)" }}>
            {a.name}
          </span>
          {a.size_bytes != null && (
            <span className="num shrink-0" style={{ color: "var(--fg3)" }}>
              {fmtBytes(a.size_bytes)}
            </span>
          )}
          {expiryText(a.expires_at) && (
            <span className="shrink-0" style={{ color: "var(--fg3)" }}>
              {expiryText(a.expires_at)}
            </span>
          )}
          <span className="flex-1" />
          {a.url && <PaneLink href={a.url}>Download</PaneLink>}
        </div>
      ))}
    </Card>
  );
}
