import { useState } from "react";
import { usePrDecoration, usePullRequests, type PrView } from "../api";
import { prSignalMeta, prState, prStatusMeta, repoShort, sameRef, toTime } from "../format";
import type { PrRow } from "../types";
import { Icon } from "./icons";
import { Chip, List, Row, Skeleton, StateCard, StatusGlyph, Tabs } from "./ui";
import { ErrorState } from "./ErrorState";
import { useCurrentPr, usePrOpener } from "./PrDetail";
import { shortAge, useListView } from "./ControlBar";
import { useRepoScope } from "./RepoScope";

const PR_VIEWS: { key: PrView; label: string }[] = [
  { key: "all", label: "All" },
  { key: "yours", label: "Mine" },
  { key: "review_requested", label: "Review requested" },
  { key: "merged", label: "Merged" },
];

const EMPTY: Record<PrView, { title: string; sub: string }> = {
  all: { title: "No open pull requests", sub: "PRs you author or are asked to review show up here." },
  yours: { title: "No open PRs of yours", sub: "Pull requests you've opened that are still open show up here." },
  merged: { title: "Nothing merged recently", sub: "Pull requests you authored that have merged show up here." },
  review_requested: { title: "No review requests", sub: "Pull requests waiting on your review show up here." },
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
    // Only the view being shown has been fetched, so only its tab carries a count.
    views: (
      <Tabs
        label="Pull request views"
        value={view}
        onChange={setView}
        tabs={PR_VIEWS.map((v) => ({ id: v.key, label: v.label, count: v.key === view && data ? data.length : undefined }))}
      />
    ),
    // With nothing selected the picker is the empty state's action; one copy of it is enough.
    extra: scope.noneSelected ? null : scope.control,
  });

  return (
    <>
      {/* The bar stays up in every state: the views are how you leave an empty one. */}
      {bar}
      {isLoading ? (
        <Skeleton />
      ) : error ? (
        <ErrorState error={error} />
      ) : scope.noneSelected ? (
        // Distinct from "no pull requests": nothing was fetched because nothing was asked for.
        <StateCard
          icon={<Icon name="pull-request" size={24} />}
          title="No repositories selected"
          sub="This connection spans your whole account — choose which repositories it fetches from."
          action={scope.control}
        />
      ) : !data || data.length === 0 ? (
        <StateCard icon={<Icon name="pull-request" size={24} />} title={EMPTY[view].title} sub={EMPTY[view].sub} />
      ) : (
        <List>
          {rows.map((row) => (
            <PrListRow key={rowKey(row)} row={row} />
          ))}
        </List>
      )}
    </>
  );
}

/** One pull request: glyph · title · identifier · signal · age, and — unless `dense` — a second
 *  line with the author and the size of the change. */
function PrListRow({ row, dense = false }: { row: PrRow; dense?: boolean }) {
  const pr = row.pull_request;
  // The list endpoint no longer pays for the decorated fields (GitHub omits them from `/pulls`
  // entirely), so each visible row fetches its own — bounded by what's on screen rather than
  // multiplied by every repository in the scope.
  const { data: decoration } = usePrDecoration(
    row.needs_decoration ? { conn: row.connection_id, repo: pr.repository, id: pr.id, updatedAt: pr.updated_at } : null,
  );
  const additions = decoration?.additions ?? pr.additions;
  const deletions = decoration?.deletions ?? pr.deletions;
  const checkStatus = decoration?.checks ?? pr.checks;
  const mergeable = decoration?.mergeable ?? pr.mergeable;
  const status = prStatusMeta(pr);
  // The decorated values, not the bare row: the list endpoint doesn't carry checks or mergeable,
  // so asking the undecorated PR what stops it would answer from missing data.
  const decorated = { ...pr, checks: checkStatus, mergeable };
  const signal = prSignalMeta(decorated);
  // A blocker always shows; "no checks" stays silent, since it says nothing.
  const showSignal = checkStatus !== "None" || prState(decorated).kind === "blocked";
  const draft = pr.is_draft || pr.status === "Draft";
  const repo = repoShort(pr.repository);
  const ident = [repo, pr.number != null ? `#${pr.number}` : ""].filter(Boolean).join(" ");
  const open = usePrOpener();
  const current = useCurrentPr();
  return (
    <Row dense={dense} selected={sameRef(current, { conn: row.connection_id, repo: pr.repository, id: pr.id })} onClick={() => open({ conn: row.connection_id, repo: pr.repository, id: pr.id })}>
      <StatusGlyph glyph={status.glyph} color={status.color} title={status.label} />
      <span className="flex-1 min-w-0 flex flex-col">
        <span className="flex items-center gap-2 min-w-0">
          <span className="truncate min-w-0">
            <span>{pr.title}</span>
            {draft && <span style={{ color: "var(--fg3)" }}> · draft</span>}
          </span>
          {pr.labels.slice(0, 3).map((l) => (
            <Chip key={l} className="hidden xl:inline-flex shrink-0">
              {l}
            </Chip>
          ))}
        </span>
        {!dense && (
          <span className="truncate text-[12px]" style={{ color: "var(--fg3)" }}>
            {pr.author.display_name} ·{" "}
            <span className="num" style={{ color: "var(--green)" }}>
              +{additions}
            </span>{" "}
            <span className="num" style={{ color: "var(--red)" }}>
              −{deletions}
            </span>
            {mergeable === "Mergeable" && (
              <>
                {" · "}
                <span>mergeable</span>
              </>
            )}
          </span>
        )}
      </span>
      {/* With one connection spanning an account, the connection label no longer distinguishes
          rows — the repository is what tells them apart. */}
      {ident && (
        <span className="mono shrink-0 whitespace-nowrap" style={{ color: "var(--fg3)" }} title={pr.repository ?? undefined}>
          {ident}
        </span>
      )}
      <span className="inline-flex shrink-0" style={{ width: 14 }} title={showSignal ? signal.label : undefined}>
        {showSignal && (
          <>
            <StatusGlyph glyph={signal.glyph} color={signal.color} spin={signal.spin} />
            <span className="sr-only">{signal.label}</span>
          </>
        )}
      </span>
      <span className="num text-[12px] shrink-0 text-right whitespace-nowrap" style={{ width: 34, color: "var(--fg3)" }}>
        {shortAge(pr.updated_at)}
      </span>
    </Row>
  );
}
