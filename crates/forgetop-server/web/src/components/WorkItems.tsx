import { useWorkItems } from "../api";
import { relativeTime, wiStateColor } from "../format";
import type { WiRow } from "../types";
import { Avatar, Chip, List, ProviderBadge, Row, Skeleton, StateCard } from "./ui";
import { ErrorState } from "./ErrorState";

export function WorkItems() {
  const { data, isLoading, error } = useWorkItems();
  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || data.length === 0)
    return <StateCard icon="◇" title="No work items assigned" sub="Issues and tickets assigned to you appear here." />;

  return (
    <List>
      {data.map((row, i) => (
        <WiCard key={`${row.connection_id}:${row.work_item.id}`} row={row} index={i} />
      ))}
    </List>
  );
}

function WiCard({ row, index }: { row: WiRow; index: number }) {
  const wi = row.work_item;
  const color = wiStateColor(wi.state, wi.state_category);
  return (
    <Row index={index} href={wi.url}>
      <div className="flex items-start gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="inline-flex items-center gap-1.5 text-xs font-medium whitespace-nowrap" style={{ color }}>
              <span>●</span>
              {wi.state}
            </span>
            <span className="truncate font-medium" style={{ color: "var(--fg)" }}>
              {wi.title}
            </span>
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
            <ProviderBadge provider={row.provider} connection={row.connection} />
            {wi.identifier && <span className="mono text-xs" style={{ color: "var(--dim)" }}>{wi.identifier}</span>}
            {wi.work_item_type && <Chip>{wi.work_item_type}</Chip>}
          </div>
        </div>
        <div className="flex flex-col items-end gap-2 shrink-0">
          <span className="text-xs whitespace-nowrap" style={{ color: "var(--dim)" }}>
            {relativeTime(wi.updated_at)}
          </span>
          {wi.assignee && <Avatar name={wi.assignee.display_name} />}
        </div>
      </div>
    </Row>
  );
}
