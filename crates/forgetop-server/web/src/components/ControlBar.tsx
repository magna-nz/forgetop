import { useMemo, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { Icon } from "./icons";
import { Select } from "./ui";

/** Persisted (per-browser) sort/filter state for a list page. */
export interface ListControls {
  sort: number;
  conn: string; // "" = all connections
  status: string; // "" = all statuses
  facet: string; // "" = all (data-derived facet, e.g. work-item type)
}

function usePref(key: string, defaultStatus = ""): [ListControls, (patch: Partial<ListControls>) => void] {
  const storageKey = `forgetop_ctl_${key}`;
  const [state, setState] = useState<ListControls>(() => {
    try {
      const s = localStorage.getItem(storageKey);
      if (s) return { sort: 0, conn: "", status: defaultStatus, facet: "", ...JSON.parse(s) };
    } catch {
      /* ignore */
    }
    return { sort: 0, conn: "", status: defaultStatus, facet: "" };
  });
  const patch = (p: Partial<ListControls>) =>
    setState((prev) => {
      const next = { ...prev, ...p };
      localStorage.setItem(storageKey, JSON.stringify(next));
      return next;
    });
  return [state, patch];
}

export interface SortOption<T> {
  label: string;
  cmp: (a: T, b: T) => number;
}

export interface StatusOption<T> {
  label: string;
  match: (r: T) => boolean;
}

/** Sort + connection/status filtering for a list, with a control bar and the processed rows.
 *  Preferences persist to localStorage, per page.
 *
 *  The bar is one 40px line under the top bar: the list's views (`views`, usually `<Tabs>`) on
 *  the left; on the right any page-specific controls (`extra`), the filters and the sort as quiet
 *  `<Select>`s, and the "N of M" count. */
export function useListView<T>(opts: {
  storageKey: string;
  rows: T[] | undefined;
  connId: (r: T) => string;
  connLabel: (r: T) => string;
  sorts: SortOption<T>[];
  statuses?: StatusOption<T>[];
  statusLabel?: string;
  /** Index into `statuses` to select on first visit (before the user picks). Defaults to "All". */
  defaultStatus?: number;
  /** A dropdown whose options are the distinct values present in the rows (e.g. work-item type).
   *  Only rendered when 2+ distinct values exist, so it reflects what's actually in the list. */
  facet?: { label: string; value: (r: T) => string | null | undefined };
  /** Left side of the bar: the list's saved views, as `<Tabs>`. */
  views?: ReactNode;
  /** Right side, before the filters: page controls such as a grouping `<Segmented>` or a scope picker. */
  extra?: ReactNode;
}): { rows: T[]; bar: React.ReactNode; total: number; sortIndex: number } {
  const [ctl, patch] = usePref(opts.storageKey, opts.defaultStatus != null ? String(opts.defaultStatus) : "");
  const all = opts.rows ?? [];

  const connections = useMemo(() => {
    const seen = new Map<string, string>();
    all.forEach((r) => seen.set(opts.connId(r), opts.connLabel(r)));
    return [...seen.entries()];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [all]);

  const facetValues = useMemo(() => {
    if (!opts.facet) return [];
    const seen = new Set<string>();
    all.forEach((r) => {
      const v = opts.facet!.value(r);
      if (v) seen.add(v);
    });
    return [...seen].sort((a, b) => a.localeCompare(b));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [all]);

  const sortIdx = Math.min(ctl.sort, opts.sorts.length - 1);
  const statusIdx = ctl.status === "" ? -1 : Number(ctl.status);

  const processed = useMemo(() => {
    let rows = all;
    if (ctl.conn) rows = rows.filter((r) => opts.connId(r) === ctl.conn);
    if (opts.statuses && statusIdx >= 0 && opts.statuses[statusIdx]) rows = rows.filter(opts.statuses![statusIdx].match);
    if (opts.facet && ctl.facet) rows = rows.filter((r) => opts.facet!.value(r) === ctl.facet);
    const cmp = opts.sorts[sortIdx]?.cmp;
    if (cmp) rows = [...rows].sort(cmp);
    return rows;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [all, ctl.conn, statusIdx, ctl.facet, sortIdx]);

  const bar = (
    <div className="w-full max-w-[1280px] mx-auto px-6">
      <div className="flex flex-wrap items-center gap-x-1 gap-y-1 min-h-10 py-1" style={{ borderBottom: "1px solid var(--border)" }}>
        {opts.views && <div className="self-end flex min-w-0">{opts.views}</div>}
        <div className="ml-auto flex items-center gap-1 min-w-0">
          {opts.extra}
          {connections.length > 1 && (
            <Select value={ctl.conn} onChange={(e) => patch({ conn: e.target.value })} label="Connection">
              <option value="">All connections</option>
              {connections.map(([id, label]) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </Select>
          )}
          {opts.statuses && opts.statuses.length > 0 && (
            <Select value={ctl.status} onChange={(e) => patch({ status: e.target.value })} label={opts.statusLabel ?? "Status"}>
              <option value="">Any status</option>
              {opts.statuses.map((s, i) => (
                <option key={i} value={i}>
                  {s.label}
                </option>
              ))}
            </Select>
          )}
          {opts.facet && facetValues.length > 1 && (
            <Select value={ctl.facet} onChange={(e) => patch({ facet: e.target.value })} label={opts.facet.label}>
              <option value="">Any {opts.facet.label.toLowerCase()}</option>
              {facetValues.map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </Select>
          )}
          <Select value={String(sortIdx)} onChange={(e) => patch({ sort: Number(e.target.value) })} label="Sort">
            {opts.sorts.map((s, i) => (
              <option key={i} value={i}>
                {s.label}
              </option>
            ))}
          </Select>
          {all.length > 0 && (
            <span className="num text-[12px] whitespace-nowrap pl-2" style={{ color: "var(--fg3)" }}>
              {processed.length} of {all.length}
            </span>
          )}
        </div>
      </div>
    </div>
  );

  return { rows: processed, bar, total: all.length, sortIndex: sortIdx };
}

/** A collapsible group header row: the list row look with a chevron, as a real toggle button
 *  carrying `aria-expanded`. (`Row` in ui.tsx takes no aria-expanded; this is the local stand-in
 *  until it does.) `height` overrides the row's 36px — an inline style, so it wins whatever the
 *  min-height class would not win; it goes inline. */
/** A group header that expands and collapses its section. `onContextMenu`, `onKeyDown` and
 *  `menuOpen` take a `useContextMenu()`'s handlers, the way `Row` does, so a header can carry
 *  a menu of its own (expand all, open the parent, …). */
export function ToggleRow({
  open,
  onToggle,
  height,
  style,
  onContextMenu,
  onKeyDown,
  menuOpen = false,
  children,
}: {
  open: boolean;
  onToggle: () => void;
  height?: number;
  style?: CSSProperties;
  onContextMenu?: (e: MouseEvent) => void;
  onKeyDown?: (e: KeyboardEvent) => void;
  menuOpen?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className="ft-row"
      aria-expanded={open}
      onClick={onToggle}
      onContextMenu={onContextMenu}
      onKeyDown={onKeyDown}
      data-menu-open={menuOpen ? "true" : undefined}
      style={{ ...(height ? { minHeight: height } : {}), ...style }}
    >
      <Icon name={open ? "chevron-down" : "chevron-right"} size={14} style={{ color: "var(--fg3)" }} />
      {children}
    </button>
  );
}

export { shortAge } from "../format";
