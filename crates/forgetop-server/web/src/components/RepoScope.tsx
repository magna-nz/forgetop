import { useEffect, useMemo, useRef, useState } from "react";
import { useQueries, useQueryClient } from "@tanstack/react-query";
import { apiPost, fetchConnectionRepositories, useConnections } from "../api";
import { isRepoAddressed } from "../capabilities";
import type { ConnectionRow, RepositoryPage } from "../types";

/** Queries invalidated when the scope changes. The scope gates **fetching**, so every list that
 *  fetches has to go back to the provider — unlike the view tabs, which narrow rows already in
 *  hand. */
const AFFECTED = ["prs", "work-items", "pipelines", "launchpad", "connections"];

/** How many repositories a connection currently fetches from.
 *
 *  `repo_scope` is respected whenever it is present, **including when it is empty** — that is the
 *  user having chosen none, not the scope being unset. Only a genuinely absent scope falls back
 *  to the legacy single repository. */
function selectedCount(c: ConnectionRow): number {
  if (c.repo_scope) return c.repo_scope.length;
  return c.repository ? 1 : 0;
}

/** The connection's scope for the picker: its stored list, or the legacy single repository. */
function selectedRepos(c: ConnectionRow): string[] {
  if (c.repo_scope) return c.repo_scope;
  return c.repository ? [c.repository] : [];
}

export interface RepoScopeState {
  /** The control to render in the list's filter bar. `null` when no bound connection is
   *  repo-addressed (a Jira- or Linear-only section has no repositories to scope). */
  control: React.ReactNode;
  /** True when every repo-addressed connection has explicitly chosen no repositories. This is a
   *  distinct state from "nothing to show": the section fetched nothing because it was told to. */
  noneSelected: boolean;
}

/** The three sections a connection can be bound to, as the connections API spells them. */
export type ScopeSection = "pull_requests" | "work_items" | "pipelines";

/**
 * The per-connection repository scope, surfaced as a filter control.
 *
 * The scope is per connection rather than per lens, so pull requests, work items and pipelines
 * all read the same "the repositories I'm working in" — no wondering why a repository appears in
 * one lens and not another.
 *
 * Connections come from the section's **binding**, not from the loaded rows: an empty scope
 * returns no rows at all, and a control derived from rows would vanish exactly when the user
 * needs it to widen the scope again.
 */
export function useRepoScope(section: ScopeSection): RepoScopeState {
  const { data: connections } = useConnections();
  const [open, setOpen] = useState(false);

  const scoped = useMemo(
    () => (connections ?? []).filter((c) => c.sections.includes(section) && isRepoAddressed(c.provider)),
    [connections, section],
  );

  // One discovery call per connection, cached — the denominator in "5 of 37" is a real count, not
  // decoration: a user seeing five repositories' worth of PRs and believing that is everything is
  // a worse failure than a slow dashboard.
  const discovery = useQueries({
    queries: scoped.map((c) => ({
      queryKey: ["connection-repositories", c.id],
      queryFn: () => fetchConnectionRepositories(c.id),
      staleTime: 5 * 60_000,
      retry: false,
    })),
  });

  const selected = scoped.reduce((n, c) => n + selectedCount(c), 0);
  const available = discovery.reduce((n, q) => n + (q.data?.repositories.length ?? 0), 0);
  const truncated = discovery.some((q) => q.data?.truncated);
  const noneSelected = scoped.every((c) => c.repo_scope !== null && c.repo_scope !== undefined && c.repo_scope.length === 0);

  // Nothing chosen *and* nothing discoverable means there is genuinely nothing to scope — the
  // built-in demo connections, or a provider whose discovery didn't answer. Showing "0 of 0"
  // there would be a control that can't do anything.
  if (scoped.length === 0 || (selected === 0 && available === 0 && !noneSelected)) {
    return { control: null, noneSelected: false };
  }

  const total = available > 0 ? `${available}${truncated ? "+" : ""}` : "?";

  return {
    noneSelected,
    control: (
      <RepoScopeButton
        label={`Repos · ${selected} of ${total}`}
        open={open}
        onToggle={() => setOpen((o) => !o)}
        onClose={() => setOpen(false)}
        connections={scoped}
        pages={discovery.map((q) => q.data)}
      />
    ),
  };
}

function RepoScopeButton({
  label,
  open,
  onToggle,
  onClose,
  connections,
  pages,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  connections: ConnectionRow[];
  pages: (RepositoryPage | undefined)[];
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
        title="Which repositories this connection fetches from"
        className="rounded-md px-2.5 py-1.5 text-xs transition-colors"
        style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--card)" }}
        onMouseEnter={(e) => (e.currentTarget.style.color = "var(--fg)")}
        onMouseLeave={(e) => (e.currentTarget.style.color = "var(--dim)")}
      >
        {label}
      </button>
      {open && (
        <div
          className="absolute z-30 mt-1.5 w-80 rounded-lg p-2 shadow-lg"
          style={{ background: "var(--panel)", border: "1px solid var(--border)" }}
        >
          {connections.map((c, i) => (
            <ConnectionScope key={c.id} connection={c} page={pages[i]} showName={connections.length > 1} />
          ))}
        </div>
      )}
    </div>
  );
}

function ConnectionScope({
  connection,
  page,
  showName,
}: {
  connection: ConnectionRow;
  page: RepositoryPage | undefined;
  showName: boolean;
}) {
  const qc = useQueryClient();
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chosen = selectedRepos(connection);

  // Anything already chosen stays listed even if discovery didn't return it, so a saved scope is
  // never silently dropped by a provider whose discovery is incomplete.
  const all = useMemo(() => {
    const seen = new Set(chosen);
    return [...chosen, ...(page?.repositories ?? []).filter((r) => !seen.has(r))];
  }, [chosen, page]);

  const shown = all.filter((r) => r.toLowerCase().includes(query.trim().toLowerCase()));

  const save = async (next: string[]) => {
    setBusy(true);
    setError(null);
    try {
      await apiPost("/api/connections/scope", { id: connection.id, scope: next });
      AFFECTED.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const toggle = (repo: string) => save(chosen.includes(repo) ? chosen.filter((r) => r !== repo) : [...chosen, repo]);

  return (
    <div className="flex flex-col gap-1.5">
      {showName && (
        <span className="px-1 text-xs font-medium" style={{ color: "var(--dim)" }}>
          {connection.display_name}
        </span>
      )}
      <input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search repositories…"
        className="rounded-md px-2 py-1.5 text-xs outline-none"
        style={{ background: "var(--card)", color: "var(--fg)", border: "1px solid var(--border)" }}
      />
      <div className="max-h-64 overflow-y-auto flex flex-col">
        {page === undefined ? (
          <span className="px-1 py-2 text-xs" style={{ color: "var(--dim)" }}>
            Loading repositories…
          </span>
        ) : shown.length === 0 ? (
          <span className="px-1 py-2 text-xs" style={{ color: "var(--dim)" }}>
            {all.length === 0
              ? "No repositories found for this connection's credentials."
              : "Nothing matches that search."}
          </span>
        ) : (
          shown.map((repo) => (
            <label
              key={repo}
              className="flex items-center gap-2 rounded-md px-1.5 py-1 text-xs cursor-pointer"
              style={{ color: "var(--fg)" }}
            >
              <input type="checkbox" checked={chosen.includes(repo)} disabled={busy} onChange={() => toggle(repo)} />
              <span className="truncate mono">{repo}</span>
            </label>
          ))
        )}
      </div>
      {page?.truncated && (
        <span className="px-1 text-xs" style={{ color: "var(--dim)" }}>
          Showing the {page.repositories.length} most recently active — search doesn't reach past them.
        </span>
      )}
      {error && (
        <span className="px-1 text-xs" style={{ color: "var(--red)" }}>
          {error}
        </span>
      )}
    </div>
  );
}
