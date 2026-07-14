import { usePipelines } from "../api";
import { pipeMeta, relativeTime } from "../format";
import type { PipeRow } from "../types";
import { Avatar, Chip, List, Pill, ProviderBadge, Row, Skeleton, StateCard } from "./ui";
import { ErrorState } from "./ErrorState";

export function Pipelines() {
  const { data, isLoading, error } = usePipelines();
  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || data.length === 0)
    return <StateCard icon="◇" title="No pipeline runs" sub="Recent CI runs for your repositories appear here." />;

  return (
    <List>
      {data.map((row, i) => (
        <PipeCard key={`${row.connection_id}:${row.run.id}`} row={row} index={i} />
      ))}
    </List>
  );
}

function PipeCard({ row, index }: { row: PipeRow; index: number }) {
  const run = row.run;
  const meta = pipeMeta(run.status);
  const label = run.name ?? (run.number != null ? `Run #${run.number}` : run.definition_id);
  return (
    <Row index={index} href={run.url}>
      <div className="flex items-start gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <Pill icon={meta.icon} label={meta.label} color={meta.color} spin={meta.running} />
            <span className="truncate font-medium" style={{ color: "var(--fg)" }}>
              {label}
            </span>
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
            <ProviderBadge provider={row.provider} connection={row.connection} />
            {run.branch && <Chip title="branch">⑂ {run.branch}</Chip>}
            {run.commit_sha && <span className="mono text-xs" style={{ color: "var(--dim)" }}>{run.commit_sha.slice(0, 7)}</span>}
          </div>
        </div>
        <div className="flex flex-col items-end gap-2 shrink-0">
          <span className="text-xs whitespace-nowrap" style={{ color: "var(--dim)" }}>
            {relativeTime(run.finished_at ?? run.started_at)}
          </span>
          {run.triggered_by && <Avatar name={run.triggered_by.display_name} />}
        </div>
      </div>
    </Row>
  );
}
