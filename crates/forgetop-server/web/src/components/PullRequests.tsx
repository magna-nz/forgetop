import { useState } from "react";
import { usePrDecoration, usePullRequests, type PrView } from "../api";
import { checkMeta, prStatusMeta, relativeTime, toTime, voteMeta } from "../format";
import type { PrRow } from "../types";
import { Avatar, Chip, List, Pill, Row, Skeleton, StateCard, StatusBadge } from "./ui";
import { ErrorState } from "./ErrorState";
import { usePrOpener } from "./PrDetail";
import { useListView } from "./ControlBar";
import { useRepoScope } from "./RepoScope";

const PR_VIEWS: { key: PrView; label: string }[] = [
  { key: "all", label: "All Pull Requests" },
  { key: "yours", label: "Your PRs" },
  { key: "merged", label: "Recently merged by you" },
  { key: "review_requested", label: "Review requested" },
];

const EMPTY: Record<PrView, { icon: string; title: string; sub: string }> = {
  all: { icon: "◇", title: "No open pull requests", sub: "PRs you author or are asked to review show up here." },
  yours: { icon: "◇", title: "No open PRs of yours", sub: "Pull requests you've opened that are still open show up here." },
  merged: { icon: "✓", title: "Nothing merged recently", sub: "Pull requests you authored that have merged show up here." },
  review_requested: { icon: "◇", title: "No review requests", sub: "Pull requests waiting on your review show up here." },
};

function usePrView(): [PrView, (v: PrView) => void] {
  const [view, setView] = useState<PrView>(() => {
    try {
      const s = localStorage.getItem("forgetop_pr_view");
      if (s === "all" || s === "yours" || s === "merged" || s === "review_requested") return s;
    } catch {
      /* ignore */
    }
    return "all";
  });
  const set = (v: PrView) => {
    setView(v);
    try {
      localStorage.setItem("forgetop_pr_view", v);
    } catch {
      /* ignore */
    }
  };
  return [view, set];
}

/** A row's key. The repository is part of it: one connection now spans an account, so `#7` exists
 *  in more than one repository and `connection_id:id` alone stops being unique. */
function rowKey(row: PrRow): string {
  return `${row.connection_id}:${row.pull_request.repository ?? ""}:${row.pull_request.id}`;
}

export function PullRequests() {
  const [view, setView] = usePrView();
  const { data, isLoading, error } = usePullRequests(view);
  const scope = useRepoScope("pull_requests");
  const { rows, bar } = useListView<PrRow>({
    storageKey: "prs",
    rows: data,
    connId: (r) => r.connection_id,
    connLabel: (r) => r.connection,
    sorts: [
      { label: "Recently updated", cmp: (a, b) => toTime(b.pull_request.updated_at) - toTime(a.pull_request.updated_at) },
      { label: "Oldest", cmp: (a, b) => toTime(a.pull_request.updated_at) - toTime(b.pull_request.updated_at) },
      { label: "Title A–Z", cmp: (a, b) => a.pull_request.title.localeCompare(b.pull_request.title) },
    ],
  });

  return (
    <>
      <ViewTabs view={view} onChange={setView} />
      {isLoading ? (
        <Skeleton />
      ) : error ? (
        <ErrorState error={error} />
      ) : scope.noneSelected ? (
        // Distinct from "no pull requests": nothing was fetched because nothing was asked for.
        <StateCard
          icon="◍"
          title="No repositories selected"
          sub="This connection spans your whole account — choose which repositories it fetches from."
        />
      ) : !data || data.length === 0 ? (
        <StateCard icon={EMPTY[view].icon} title={EMPTY[view].title} sub={EMPTY[view].sub} />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2 px-5 pt-4 max-w-5xl mx-auto">{scope.control}</div>
          {bar}
          <List>
            {rows.map((row, i) => (
              <PrCard key={rowKey(row)} row={row} index={i} />
            ))}
          </List>
        </>
      )}
    </>
  );
}

function ViewTabs({ view, onChange }: { view: PrView; onChange: (v: PrView) => void }) {
  return (
    <div className="flex px-5 pt-4 max-w-5xl mx-auto">
      <div className="inline-flex rounded-lg p-0.5" style={{ background: "var(--panel2)", border: "1px solid var(--border)" }}>
        {PR_VIEWS.map((v) => {
          const active = v.key === view;
          return (
            <button
              key={v.key}
              onClick={() => onChange(v.key)}
              aria-pressed={active}
              className="text-xs font-medium rounded-md px-3 py-1.5 transition-colors whitespace-nowrap"
              style={{ background: active ? "var(--accent)" : "transparent", color: active ? "var(--bg)" : "var(--dim)" }}
            >
              {v.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function PrCard({ row, index }: { row: PrRow; index: number }) {
  const pr = row.pull_request;
  // The list endpoint no longer pays for the decorated fields (GitHub omits them from `/pulls`
  // entirely), so each visible row fetches its own — bounded by what's on screen rather than
  // multiplied by every repository in the scope.
  const { data: decoration } = usePrDecoration({
    conn: row.connection_id,
    repo: pr.repository,
    id: pr.id,
    updatedAt: pr.updated_at,
  });
  const additions = decoration?.additions ?? pr.additions;
  const deletions = decoration?.deletions ?? pr.deletions;
  const checkStatus = decoration?.checks ?? pr.checks;
  const mergeable = decoration?.mergeable ?? pr.mergeable;
  const status = prStatusMeta(pr);
  const checks = checkMeta(checkStatus);
  const open = usePrOpener();
  return (
    <Row index={index} dense onClick={() => open({ conn: row.connection_id, repo: pr.repository, id: pr.id })}>
      <div className="flex items-start gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <StatusBadge label={cap(status.label)} color={status.color} />
            <span className="truncate font-medium" style={{ color: "var(--fg)" }}>
              {pr.title}
            </span>
          </div>
          <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1.5">
            {pr.number != null && <span className="mono text-xs" style={{ color: "var(--dim)" }}>#{pr.number}</span>}
            {/* With one connection spanning an account, the connection label no longer
                distinguishes rows — the repository is what tells them apart. */}
            {pr.repository && <Chip title="Repository">{pr.repository}</Chip>}
            {pr.source_ref && (
              <Chip title="source → target">
                {shortRef(pr.source_ref)} <span style={{ color: "var(--dim)" }}>→</span> {shortRef(pr.target_ref) || "?"}
              </Chip>
            )}
            <span className="mono text-xs">
              <span style={{ color: "var(--green)" }}>+{additions}</span>{" "}
              <span style={{ color: "var(--red)" }}>−{deletions}</span>
            </span>
            {checkStatus !== "None" && <Pill icon={checks.icon} label={checks.label} color={checks.color} spin={checkStatus === "Pending"} />}
            {pr.reviewers.length > 0 && <Reviewers reviewers={pr.reviewers} />}
            {pr.labels.slice(0, 3).map((l) => (
              <Chip key={l}>{l}</Chip>
            ))}
            {mergeable === "Mergeable" && <Pill icon="✓" label="Mergeable" color="var(--magenta)" />}
          </div>
        </div>
        <div className="flex flex-col items-end gap-2 shrink-0">
          <span className="text-xs whitespace-nowrap" style={{ color: "var(--dim)" }}>
            {relativeTime(pr.updated_at)}
          </span>
          <Avatar name={pr.author.display_name} />
        </div>
      </div>
    </Row>
  );
}

function Reviewers({ reviewers }: { reviewers: PrRow["pull_request"]["reviewers"] }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      {reviewers.slice(0, 4).map((r, i) => {
        const v = voteMeta(r.vote);
        return (
          <span key={i} title={`${r.user.display_name} — ${v.label}`} className="inline-flex items-center gap-0.5 text-xs" style={{ color: v.color }}>
            {v.icon}
          </span>
        );
      })}
    </span>
  );
}

function shortRef(ref?: string | null): string {
  if (!ref) return "";
  return ref.replace(/^refs\/heads\//, "");
}

const cap = (s: string): string => (s.length ? s[0].toUpperCase() + s.slice(1) : s);
