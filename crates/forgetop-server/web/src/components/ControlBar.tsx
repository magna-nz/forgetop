import { useMemo, useState } from "react";

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
 *  Preferences persist to localStorage, per page. */
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
}): { rows: T[]; bar: React.ReactNode; total: number } {
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
    <div className="flex flex-wrap items-center gap-2 px-5 pt-4 max-w-5xl mx-auto">
      <Select value={String(sortIdx)} onChange={(v) => patch({ sort: Number(v) })} label="Sort">
        {opts.sorts.map((s, i) => (
          <option key={i} value={i}>{s.label}</option>
        ))}
      </Select>
      {connections.length > 1 && (
        <Select value={ctl.conn} onChange={(v) => patch({ conn: v })} label="Connection">
          <option value="">All connections</option>
          {connections.map(([id, label]) => (
            <option key={id} value={id}>{label}</option>
          ))}
        </Select>
      )}
      {opts.statuses && opts.statuses.length > 0 && (
        <Select value={ctl.status} onChange={(v) => patch({ status: v })} label={opts.statusLabel ?? "Status"}>
          <option value="">All</option>
          {opts.statuses.map((s, i) => (
            <option key={i} value={i}>{s.label}</option>
          ))}
        </Select>
      )}
      {opts.facet && facetValues.length > 1 && (
        <Select value={ctl.facet} onChange={(v) => patch({ facet: v })} label={opts.facet.label}>
          <option value="">All</option>
          {facetValues.map((v) => (
            <option key={v} value={v}>{v}</option>
          ))}
        </Select>
      )}
      <span className="text-xs ml-auto" style={{ color: "var(--dim)" }}>
        {processed.length} of {all.length}
      </span>
    </div>
  );

  return { rows: processed, bar, total: all.length };
}

function Select({
  value,
  onChange,
  label,
  children,
}: {
  value: string;
  onChange: (v: string) => void;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="flex items-center gap-1.5 text-xs" style={{ color: "var(--dim)" }}>
      {label}
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="rounded-md px-2 py-1 text-xs outline-none"
        style={{ background: "var(--panel2)", color: "var(--fg)", border: "1px solid var(--border)" }}
      >
        {children}
      </select>
    </label>
  );
}
