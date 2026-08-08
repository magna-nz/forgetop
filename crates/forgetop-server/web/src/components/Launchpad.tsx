import { useState } from "react";
import { motion } from "framer-motion";
import { useLaunchpad } from "../api";
import { checkMeta, pipeMeta, prStatusMeta, relativeTime, wiStateColor } from "../format";
import type { LaunchpadMore, LaunchpadRow, SectionId } from "../types";
import { Skeleton, StateCard, StatusBadge } from "./ui";
import { ErrorState } from "./ErrorState";
import { usePrOpener } from "./PrDetail";
import { useWiOpener } from "./WiDetail";
import { usePipelineOpener } from "./PipelineDetail";
import { useNavigateSection } from "../nav";

export function Launchpad() {
  const { data, isLoading, error } = useLaunchpad();
  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || data.rows.length === 0)
    return <StateCard icon="✦" title="All clear" sub="Nothing needs your attention right now." />;

  const left = groupByBucket(data.rows.filter((r) => r.column === 0));
  const right = groupByBucket(data.rows.filter((r) => r.column === 1));

  return (
    <div className="grid gap-5 p-5 max-w-6xl mx-auto lg:grid-cols-2">
      <Column heading="Needs you" groups={left} emptyHint="Nothing waiting on you." more={data.more} />
      <Column heading="Your work" groups={right} emptyHint="No open work." more={data.more} />
    </div>
  );
}

/** Where a bucket's "more…" link goes; presets the PR page view where relevant. */
function moreTarget(bucket: string, navigate: (s: SectionId) => void): (() => void) | null {
  switch (bucket) {
    case "needs_review":
      return () => {
        try {
          localStorage.setItem("forgetop_pr_view", "review_requested");
        } catch {
          /* ignore */
        }
        navigate("prs");
      };
    case "your_work":
      return () => navigate("work-items");
    case "your_open_prs":
      return () => {
        try {
          localStorage.setItem("forgetop_pr_view", "yours");
        } catch {
          /* ignore */
        }
        navigate("prs");
      };
    case "recently_merged":
      return () => {
        try {
          localStorage.setItem("forgetop_pr_view", "merged");
        } catch {
          /* ignore */
        }
        navigate("prs");
      };
    case "recent_pipelines":
      return () => navigate("pipelines");
    default:
      return null;
  }
}

interface Group {
  bucket: string;
  title: string;
  muted: boolean;
  rows: LaunchpadRow[];
}

function groupByBucket(rows: LaunchpadRow[]): Group[] {
  const groups: Group[] = [];
  for (const row of rows) {
    let g = groups[groups.length - 1];
    if (!g || g.bucket !== row.bucket) {
      g = { bucket: row.bucket, title: row.bucket_title, muted: row.muted, rows: [] };
      groups.push(g);
    }
    g.rows.push(row);
  }
  return groups;
}

function Column({ heading, groups, emptyHint, more }: { heading: string; groups: Group[]; emptyHint: string; more: LaunchpadMore }) {
  return (
    <section className="flex flex-col gap-5">
      <h2 className="text-xs font-semibold uppercase tracking-wider px-1" style={{ color: "var(--dim)" }}>
        {heading}
      </h2>
      {groups.length === 0 ? (
        <p className="text-sm px-1" style={{ color: "var(--dim)" }}>
          {emptyHint}
        </p>
      ) : (
        groups.map((g) => <BucketGroup key={g.bucket} group={g} more={more} />)
      )}
    </section>
  );
}

/** Buckets with no clean deep-link target: capped in the UI and revealed in place, a page at a time. */
const EXPAND_BUCKETS = new Set(["ready_to_merge", "needs_fixing"]);
const EXPAND_STEP = 5;

function BucketGroup({ group, more }: { group: Group; more: LaunchpadMore }) {
  const navigate = useNavigateSection();
  const isExpand = EXPAND_BUCKETS.has(group.bucket);
  const [limit, setLimit] = useState(EXPAND_STEP);

  // Expand buckets are returned whole and revealed in place; the rest are already capped by the
  // backend and their "more…" deep-links to the full page/view.
  const rows = isExpand ? group.rows.slice(0, limit) : group.rows;
  const expandMore = isExpand && group.rows.length > limit;
  const navMore = !isExpand && (more as unknown as Record<string, boolean>)[group.bucket] === true;
  const go = navMore ? moreTarget(group.bucket, navigate) : null;

  return (
    <div>
      <div className="flex items-center gap-2 px-1 mb-2">
        <span className="text-sm font-medium" style={{ color: group.muted ? "var(--dim)" : "var(--fg)" }}>
          {group.title}
        </span>
        <span className="mono text-xs rounded-full px-1.5" style={{ background: "var(--panel2)", color: "var(--dim)" }}>
          {group.rows.length}
        </span>
      </div>
      <div className="flex flex-col gap-1.5">
        {rows.map((row, i) => (
          <ItemRow key={`${row.connection_id}:${rowId(row)}`} row={row} index={i} muted={group.muted} />
        ))}
        {(go || expandMore) && (
          <button
            onClick={go ?? (() => setLimit((l) => l + EXPAND_STEP))}
            className="text-xs font-medium rounded-lg px-3 py-2 text-left transition-colors hover:brightness-110"
            style={{ color: "var(--accent)", background: "var(--card)", border: "1px solid var(--border)" }}
          >
            more…
          </button>
        )}
      </div>
    </div>
  );
}

function ItemRow({ row, index, muted }: { row: LaunchpadRow; index: number; muted: boolean }) {
  const { title, meta } = describe(row);
  const badge = statusBadge(row);
  const openPr = usePrOpener();
  const openWi = useWiOpener();
  const openPipe = usePipelineOpener();
  // Every card kind opens its in-app detail panel, matching the list views.
  const onClick =
    row.kind === "pr"
      ? () => openPr({ conn: row.connection_id, repo: row.pull_request.repository, id: row.pull_request.id })
      : row.kind === "wi"
        ? () => openWi({ conn: row.connection_id, repo: row.work_item.repository, id: row.work_item.id })
        : () => openPipe({ conn: row.connection_id, repo: row.run.repository, runId: row.run.id });
  const common = {
    initial: { opacity: 0, y: 4 },
    animate: { opacity: 1, y: 0 },
    transition: { duration: 0.2, delay: Math.min(index * 0.015, 0.25) },
    className: "group flex items-center gap-2.5 rounded-lg px-3 py-1.5 transition-colors w-full text-left",
    style: { background: "var(--card)", border: "1px solid var(--border)", opacity: muted ? 0.85 : 1, cursor: "pointer" },
    onMouseEnter: (e: React.MouseEvent<HTMLElement>) => (e.currentTarget.style.background = "var(--card-hover)"),
    onMouseLeave: (e: React.MouseEvent<HTMLElement>) => (e.currentTarget.style.background = "var(--card)"),
  };
  const inner = (
    <>
      <StatusBadge label={badge.label} color={badge.color} />
      <div className="flex-1 min-w-0">
        <div className="truncate text-sm" style={{ color: "var(--fg)" }}>
          {title}
        </div>
        <div className="flex items-center gap-2 mt-0.5">
          <span className="mono text-[11px] rounded px-1" style={{ background: "var(--panel2)", color: "var(--dim)" }}>
            {kindLabel(row.kind)}
          </span>
          {meta && (
            <span className="text-xs truncate" style={{ color: "var(--dim)" }}>
              {meta}
            </span>
          )}
        </div>
      </div>
    </>
  );
  return (
    <motion.button {...common} onClick={onClick}>
      {inner}
    </motion.button>
  );
}

function describe(row: LaunchpadRow): { title: string; meta: string } {
  if (row.kind === "pr") {
    const pr = row.pull_request;
    const c = pr.checks !== "None" ? ` · ${checkMeta(pr.checks).label}` : "";
    return {
      title: pr.title,
      meta: `${pr.number != null ? "#" + pr.number : ""}${relativeTime(pr.updated_at) ? " · " + relativeTime(pr.updated_at) : ""}${c}`,
    };
  }
  if (row.kind === "wi") {
    const wi = row.work_item;
    return {
      title: wi.title,
      meta: `${wi.identifier ?? ""}${relativeTime(wi.updated_at) ? " · " + relativeTime(wi.updated_at) : ""}`,
    };
  }
  const run = row.run;
  const label = run.name ?? (run.number != null ? `#${run.number}` : run.definition_id);
  return {
    title: row.definition_name ? `${row.definition_name} · ${label}` : label,
    meta: `${run.branch ?? ""}${relativeTime(run.finished_at ?? run.started_at) ? " · " + relativeTime(run.finished_at ?? run.started_at) : ""}`,
  };
}

const capitalize = (s: string): string => (s.length ? s[0].toUpperCase() + s.slice(1) : s);

/** The status word + colour shown on each Command Center row, replacing the old status dot.
 *  PRs → Open/Merged/Closed/Draft, work items → their state, pipelines → Running/Queued/…
 *  with a failed run reading as "Error". Colours reuse the shared status model. */
function statusBadge(row: LaunchpadRow): { label: string; color: string } {
  if (row.kind === "pr") {
    const s = prStatusMeta(row.pull_request);
    return { label: capitalize(s.label), color: s.color };
  }
  if (row.kind === "wi") {
    const wi = row.work_item;
    return { label: wi.state, color: wiStateColor(wi.state, wi.state_category) };
  }
  const m = pipeMeta(row.run.status);
  return { label: row.run.status === "Failed" ? "Error" : capitalize(m.label), color: m.color };
}

/// A row's identity within its connection. The repository is part of it: one connection now
/// spans an account, so `#7` exists in more than one repository and `connection_id:id` alone
/// stops being unique.
function rowId(row: LaunchpadRow): string {
  if (row.kind === "pr") return `${row.pull_request.repository ?? ""}:${row.pull_request.id}`;
  if (row.kind === "wi") return `${row.work_item.repository ?? ""}:${row.work_item.id}`;
  return `${row.run.repository ?? ""}:${row.run.id}`;
}

function kindLabel(kind: LaunchpadRow["kind"]): string {
  return kind === "pr" ? "Pull Request" : kind === "wi" ? "Work Item" : "Pipeline";
}
