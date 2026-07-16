import { usePullRequests } from "../api";
import { checkMeta, prStatusMeta, relativeTime, voteMeta } from "../format";
import type { PrRow } from "../types";
import { Avatar, Chip, List, Pill, ProviderBadge, Row, Skeleton, StateCard } from "./ui";
import { ErrorState } from "./ErrorState";
import { usePrOpener } from "./PrDetail";

export function PullRequests() {
  const { data, isLoading, error } = usePullRequests();
  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || data.length === 0)
    return <StateCard icon="◇" title="No open pull requests" sub="PRs you author or are asked to review show up here." />;

  return (
    <List>
      {data.map((row, i) => (
        <PrCard key={`${row.connection_id}:${row.pull_request.id}`} row={row} index={i} />
      ))}
    </List>
  );
}

function PrCard({ row, index }: { row: PrRow; index: number }) {
  const pr = row.pull_request;
  const status = prStatusMeta(pr);
  const checks = checkMeta(pr.checks);
  const open = usePrOpener();
  return (
    <Row index={index} onClick={() => open({ conn: row.connection_id, id: pr.id })}>
      <div className="flex items-start gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <Pill icon={status.icon} label={status.label} color={status.color} />
            <span className="truncate font-medium" style={{ color: "var(--fg)" }}>
              {pr.title}
            </span>
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
            <ProviderBadge provider={row.provider} connection={row.connection} />
            {pr.number != null && <span className="mono text-xs" style={{ color: "var(--dim)" }}>#{pr.number}</span>}
            {pr.source_ref && (
              <Chip title="source → target">
                {shortRef(pr.source_ref)} <span style={{ color: "var(--dim)" }}>→</span> {shortRef(pr.target_ref) || "?"}
              </Chip>
            )}
            <span className="mono text-xs">
              <span style={{ color: "var(--green)" }}>+{pr.additions}</span>{" "}
              <span style={{ color: "var(--red)" }}>−{pr.deletions}</span>
            </span>
            {pr.checks !== "None" && <Pill icon={checks.icon} label={checks.label} color={checks.color} spin={pr.checks === "Pending"} />}
            {pr.reviewers.length > 0 && <Reviewers reviewers={pr.reviewers} />}
            {pr.labels.slice(0, 3).map((l) => (
              <Chip key={l}>{l}</Chip>
            ))}
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
