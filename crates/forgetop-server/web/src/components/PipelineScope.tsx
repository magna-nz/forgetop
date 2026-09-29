import { useEffect, useMemo, useRef, useState } from "react";
import { useQueries, useQueryClient } from "@tanstack/react-query";
import { apiPost, fetchPipelineSelection, useConnections } from "../api";
import type { ConnectionRow, PipelineDefinition, PipelineSelection } from "../types";
import { setPipelineSelectionInCache } from "../optimistic";

/** Queries that go back to the provider when the selection changes. The selection gates
 *  **fetching**, so the lists it feeds have to refetch; the picker's own query is edited in place
 *  (see `setPipelineSelectionInCache`) rather than re-running discovery. */
const AFFECTED = ["pipelines", "launchpad"];

/** How many of a connection's discovered pipelines are fetched. */
function selectedCount(sel: PipelineSelection): number {
  if (sel.all) return sel.definitions.length;
  return sel.definitions.filter((d) => sel.selected.includes(d.id)).length;
}

/** Where a pipeline lives — Azure's project + folder, or the repository — so a busy org's
 *  pipelines can be told apart and searched by folder. Mirrors the terminal picker. */
export function pipelineLocation(d: PipelineDefinition): string {
  const folder = d.path && d.path.startsWith("\\") && d.path.length > 1 ? d.path : null;
  return [d.repository, folder].filter(Boolean).join(" ");
}

export interface PipelineScopeState {
  /** The "Pipelines · N of M" control for the list's filter bar; `null` with no pipeline
   *  connection, or before discovery has answered. */
  control: React.ReactNode;
  /** Every pipeline connection has nothing selected. Pipelines are opt-in, so this is the
   *  starting state — distinct from "no runs". */
  noneSelected: boolean;
}

/**
 * The per-connection pipeline selection, surfaced as a filter control. Pipelines are opt-in:
 * a connection fetches runs only of the pipelines ticked here (or of every pipeline, when all
 * are ticked — which also picks up ones created later). The selection is saved to the config, so
 * it is the same next run, and the same one the terminal's `w` picker edits.
 *
 * Connections come from the section's binding, not from the loaded rows: with nothing selected
 * there are no rows, and the control has to be there exactly then.
 */
export function usePipelineScope(): PipelineScopeState {
  const { data: connections } = useConnections();
  const [open, setOpen] = useState(false);

  const bound = useMemo(() => (connections ?? []).filter((c) => c.sections.includes("pipelines")), [connections]);

  const discovery = useQueries({
    queries: bound.map((c) => ({
      queryKey: ["pipeline-definitions", c.id],
      queryFn: () => fetchPipelineSelection(c.id),
      staleTime: 5 * 60_000,
      retry: false,
    })),
  });

  const loaded = discovery.map((q) => q.data).filter((d): d is PipelineSelection => !!d);
  const selected = loaded.reduce((n, d) => n + selectedCount(d), 0);
  const available = loaded.reduce((n, d) => n + d.definitions.length, 0);
  const noneSelected =
    bound.length > 0 && loaded.length === bound.length && loaded.every((d) => !d.all && d.selected.length === 0);

  if (bound.length === 0 || loaded.length === 0) return { control: null, noneSelected: false };

  return {
    noneSelected,
    control: (
      <PipelineScopeButton
        label={`Pipelines · ${selected} of ${available}`}
        open={open}
        onToggle={() => setOpen((o) => !o)}
        onClose={() => setOpen(false)}
        connections={bound}
        selections={discovery.map((q) => q.data)}
        failed={discovery.map((q) => q.isError)}
      />
    ),
  };
}

function PipelineScopeButton({
  label,
  open,
  onToggle,
  onClose,
  connections,
  selections,
  failed,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  connections: ConnectionRow[];
  selections: (PipelineSelection | undefined)[];
  failed: boolean[];
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, onClose]);

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={onToggle}
        aria-expanded={open}
        title="Which pipelines this list fetches runs of"
        className="rounded-md px-2.5 py-1.5 text-xs transition-colors"
        style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--card)" }}
        onMouseEnter={(e) => (e.currentTarget.style.color = "var(--fg)")}
        onMouseLeave={(e) => (e.currentTarget.style.color = "var(--dim)")}
      >
        {label}
      </button>
      {open && (
        <div
          className="absolute z-30 mt-1.5 w-96 max-w-[calc(100vw-2rem)] rounded-lg p-2 shadow-lg flex flex-col gap-3"
          style={{ background: "var(--panel)", border: "1px solid var(--border)" }}
        >
          {connections.map((c, i) => (
            <ConnectionPipelines
              key={c.id}
              connection={c}
              selection={selections[i]}
              failed={failed[i]}
              showName={connections.length > 1}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ConnectionPipelines({
  connection,
  selection,
  failed,
  showName,
}: {
  connection: ConnectionRow;
  selection: PipelineSelection | undefined;
  failed: boolean;
  showName: boolean;
}) {
  const qc = useQueryClient();
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);

  const defs = useMemo(
    () =>
      [...(selection?.definitions ?? [])].sort(
        (a, b) => pipelineLocation(a).localeCompare(pipelineLocation(b)) || a.name.localeCompare(b.name),
      ),
    [selection],
  );
  const ticked = (d: PipelineDefinition) => !!selection && (selection.all || selection.selected.includes(d.id));
  const q = query.trim().toLowerCase();
  const shown = defs.filter((d) => `${d.name} ${pipelineLocation(d)}`.toLowerCase().includes(q));

  const save = async (ids: string[]) => {
    // Every pipeline ticked is saved as "all", so one created later is picked up too.
    const all = defs.length > 0 && defs.every((d) => ids.includes(d.id));
    setError(null);
    setPipelineSelectionInCache(qc, connection.id, all, all ? [] : ids);
    try {
      await apiPost("/api/pipelines/selection", { id: connection.id, all, ids: all ? [] : ids });
      AFFECTED.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      // Nothing changed server-side, so pull the real selection back over the local guess.
      qc.invalidateQueries({ queryKey: ["pipeline-definitions", connection.id] });
    }
  };

  // Read from the cache at click time, not from this render: two clicks landing before a
  // re-render would otherwise both start from the same selection and the first would be lost.
  const current = (): string[] => {
    const sel = qc.getQueryData<PipelineSelection>(["pipeline-definitions", connection.id]) ?? selection;
    return sel ? defs.filter((d) => sel.all || sel.selected.includes(d.id)).map((d) => d.id) : [];
  };
  const toggle = (d: PipelineDefinition) => {
    const now = current();
    save(now.includes(d.id) ? now.filter((id) => id !== d.id) : [...now, d.id]);
  };
  // All/None acts on what the search shows, so "search a folder, tick all" works.
  const allShown = shown.length > 0 && shown.every(ticked);
  const toggleShown = () => {
    const now = current();
    const ids = new Set(shown.map((d) => d.id));
    const clear = shown.every((d) => now.includes(d.id));
    save(clear ? now.filter((id) => !ids.has(id)) : [...new Set([...now, ...ids])]);
  };

  return (
    <div className="flex flex-col gap-1.5">
      {showName && (
        <span className="px-1 text-xs font-medium" style={{ color: "var(--dim)" }}>
          {connection.display_name}
        </span>
      )}
      <div className="flex items-center gap-1.5">
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search pipelines…"
          aria-label={`Search ${connection.display_name} pipelines`}
          className="flex-1 min-w-0 rounded-md px-2 py-1.5 text-xs outline-none"
          style={{ background: "var(--card)", color: "var(--fg)", border: "1px solid var(--border)" }}
        />
        <button
          onClick={toggleShown}
          disabled={shown.length === 0}
          className="shrink-0 rounded-md px-2 py-1.5 text-xs"
          style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--card)" }}
        >
          {allShown ? "None" : "All"}
        </button>
      </div>
      <div className="max-h-72 overflow-y-auto flex flex-col">
        {selection === undefined ? (
          <span className="px-1 py-2 text-xs" style={{ color: failed ? "var(--red)" : "var(--dim)" }}>
            {failed ? "Couldn't list this connection's pipelines." : "Loading pipelines…"}
          </span>
        ) : shown.length === 0 ? (
          <span className="px-1 py-2 text-xs" style={{ color: "var(--dim)" }}>
            {defs.length === 0 ? "No pipelines found for this connection." : "Nothing matches that search."}
          </span>
        ) : (
          shown.map((d) => {
            const loc = pipelineLocation(d);
            return (
              <label
                key={d.id}
                className="flex items-center gap-2 rounded-md px-1.5 py-1 text-xs cursor-pointer min-w-0"
                style={{ color: "var(--fg)" }}
              >
                <input type="checkbox" checked={ticked(d)} onChange={() => toggle(d)} aria-label={d.name} />
                <span className="truncate">{d.name}</span>
                {loc && (
                  <span className="truncate mono" style={{ color: "var(--dim)" }}>
                    {loc}
                  </span>
                )}
              </label>
            );
          })
        )}
      </div>
      {error && (
        <span className="px-1 text-xs" style={{ color: "var(--red)" }}>
          {error}
        </span>
      )}
    </div>
  );
}
