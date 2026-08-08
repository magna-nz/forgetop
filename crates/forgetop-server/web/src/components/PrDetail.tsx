import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { useQueryClient } from "@tanstack/react-query";
import { apiPost, prDetailKey, useConnections, usePrCommitChanges, usePrDetail } from "../api";
import { checkMeta, prStatusMeta, relativeTime, voteMeta } from "../format";
import { providerSupports, unsupportedMessage } from "../capabilities";
import { parsePatch } from "../diff";
import type { CheckRun, CommentThread, Commit, FileChange, FileChangeKind, LineComment, PrRef, ProviderType, Reviewer, TimelineEvent } from "../types";
import { Avatar, Chip, Pill, Timeline } from "./ui";

// ---- opener context ----

const PrOpenerCtx = createContext<(ref: PrRef) => void>(() => {});
export const usePrOpener = () => useContext(PrOpenerCtx);

export function PrDetailProvider({ children }: { children: ReactNode }) {
  const [ref, setRef] = useState<PrRef | null>(null);
  const open = useCallback((r: PrRef) => setRef(r), []);
  return (
    <PrOpenerCtx.Provider value={open}>
      {children}
      <AnimatePresence>{ref && <PrDetailPanel prRef={ref} onClose={() => setRef(null)} />}</AnimatePresence>
    </PrOpenerCtx.Provider>
  );
}

// ---- panel ----

type Tab = "conversation" | "commits" | "files";

function PrDetailPanel({ prRef, onClose }: { prRef: PrRef; onClose: () => void }) {
  const { data, isLoading, error } = usePrDetail(prRef);
  const connections = useConnections();
  const provider: ProviderType | undefined = connections.data?.find((c) => c.id === prRef.conn)?.provider;
  const qc = useQueryClient();
  const [tab, setTab] = useState<Tab>("conversation");
  const [pending, setPending] = useState<LineComment[]>([]);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  // When set, the Files tab shows a single commit's diff instead of the whole-PR changes.
  const [commitScope, setCommitScope] = useState<{ sha: string; label: string } | null>(null);
  const commitChanges = usePrCommitChanges(prRef, commitScope?.sha ?? null);

  const requestClose = () => {
    if (pending.length > 0 && !window.confirm(`Discard ${pending.length} unsubmitted comment(s)?`)) return;
    onClose();
  };

  useEffect(() => {
    setTab("conversation");
    setPending([]);
    setNote(null);
    setCommitScope(null);
  }, [prRef.conn, prRef.id]);

  // Esc closes (guarding unsubmitted comments via requestClose).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") requestClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending.length]);

  const refresh = () => {
    qc.invalidateQueries({ queryKey: prDetailKey(prRef) });
    qc.invalidateQueries({ queryKey: ["prs"] });
    qc.invalidateQueries({ queryKey: ["launchpad"] });
    // Acting on a PR (merge, approve, …) can clear its review-request / mention notification.
    qc.invalidateQueries({ queryKey: ["notifications"] });
  };

  const act = async (label: string, fn: () => Promise<void>) => {
    setBusy(true);
    setNote(null);
    try {
      await fn();
      setNote(`${label} ✓`);
      refresh();
    } catch (e) {
      setNote(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  // A verdict/merge action from the action bar closes the pane once it lands (brief delay so the
  // "✓" note flashes first), matching Merge — you've acted on this PR, so return to the list.
  const closeSoon = () => setTimeout(onClose, 500);
  const vote = (v: "Approved" | "Rejected") =>
    act(v === "Approved" ? "Approved" : "Requested changes", async () => {
      await apiPost("/api/pr/vote", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, vote: v });
      closeSoon();
    });
  const merge = () =>
    act("Merged", async () => {
      try {
        await apiPost("/api/pr/merge", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, strategy: "Merge" });
      } catch {
        // Providers without a mergeable flag (e.g. Bitbucket) let you try — the API decides.
        throw new Error("Couldn't merge — the PR may not be mergeable.");
      }
      closeSoon();
    });
  const revert = () =>
    act("Revert requested", async () => {
      await apiPost("/api/pr/revert", { conn: prRef.conn, repo: prRef.repo, id: prRef.id });
      closeSoon();
    });
  const reply = (threadId: string, body: string) =>
    act("Reply posted", () => apiPost("/api/pr/reply", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, thread_id: threadId, body }));
  const submitReview = (event: "Approved" | "Rejected" | "NoVote") =>
    act("Review submitted", async () => {
      await apiPost("/api/pr/review", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, event, comments: pending });
      setPending([]);
    });

  const pr = data?.pull_request;

  return (
    <motion.div
      className="fixed inset-0 z-40 flex justify-end"
      style={{ background: "rgba(0,0,0,0.5)" }}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.15 }}
      onClick={requestClose}
    >
      <motion.div
        initial={{ x: 40, opacity: 0.6 }}
        animate={{ x: 0, opacity: 1 }}
        exit={{ x: 40, opacity: 0 }}
        transition={{ duration: 0.22, ease: "easeOut" }}
        onClick={(e) => e.stopPropagation()}
        className="flex flex-col h-full w-full max-w-4xl"
        style={{ background: "var(--bg)", borderLeft: "1px solid var(--border)" }}
      >
        {/* header */}
        <div className="flex items-start gap-3 px-5 py-3.5 shrink-0" style={{ borderBottom: "1px solid var(--border)" }}>
          {pr && <Pill icon={prStatusMeta(pr).icon} label={prStatusMeta(pr).label} color={prStatusMeta(pr).color} />}
          <div className="flex-1 min-w-0">
            <div className="font-medium truncate" style={{ color: "var(--fg)" }}>
              {pr?.title ?? (isLoading ? "Loading…" : "Pull request")}
            </div>
            {pr && (
              <div className="flex items-center gap-2 mt-1 text-xs" style={{ color: "var(--dim)" }}>
                {pr.number != null && <span className="mono">#{pr.number}</span>}
                {pr.source_ref && (
                  <Chip>
                    {shortRef(pr.source_ref)} → {shortRef(pr.target_ref) || "?"}
                  </Chip>
                )}
                <span>{relativeTime(pr.updated_at)}</span>
              </div>
            )}
          </div>
          {pr?.url && (
            <a href={pr.url} target="_blank" rel="noreferrer" title="Open in provider" className="text-sm px-2 py-1" style={{ color: "var(--dim)" }}>
              ↗
            </a>
          )}
          <button onClick={requestClose} className="text-lg px-2 leading-none" style={{ color: "var(--dim)" }} title="Close (Esc)">
            ✕
          </button>
        </div>

        {error && <div className="p-6 text-sm" style={{ color: "var(--red)" }}>Couldn't load this pull request.</div>}

        {pr && data && (
          <>
            <MetaBar author={pr.author.display_name} reviewers={pr.reviewers} />

            {/* tabs */}
            <div className="flex items-center gap-1 px-4 shrink-0" style={{ borderBottom: "1px solid var(--border)" }}>
              {(["conversation", "commits", "files"] as Tab[]).map((t) => (
                <button
                  key={t}
                  onClick={() => setTab(t)}
                  className="px-3 py-2 text-sm capitalize rounded-t-md transition-colors"
                  style={{
                    color: tab === t ? "var(--fg)" : "var(--dim)",
                    borderBottom: tab === t ? "2px solid var(--accent)" : "2px solid transparent",
                  }}
                  onMouseEnter={(e) => {
                    e.currentTarget.style.background = "var(--card)";
                    if (tab !== t) e.currentTarget.style.color = "var(--fg)";
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.background = "transparent";
                    if (tab !== t) e.currentTarget.style.color = "var(--dim)";
                  }}
                >
                  {t}
                  {t === "files" && ` (${data.changes.length})`}
                  {t === "commits" && ` (${data.commits.length})`}
                </button>
              ))}
            </div>

            <div className="flex-1 overflow-auto">
              {tab === "files" && (
                <FilesTab
                  key={commitScope?.sha ?? "all"}
                  changes={commitScope ? (commitChanges.data ?? []) : data.changes}
                  threads={data.threads}
                  pending={pending}
                  busy={busy}
                  onReply={reply}
                  onAddPending={(c) => setPending((p) => [...p, c])}
                  onRemovePending={(i) => setPending((p) => p.filter((_, k) => k !== i))}
                  scope={
                    commitScope
                      ? { label: commitScope.label, loading: commitChanges.isLoading, onClear: () => setCommitScope(null) }
                      : undefined
                  }
                />
              )}
              {tab === "conversation" && (
                <ConversationTab
                  threads={data.threads}
                  description={pr.description}
                  timeline={data.timeline}
                  busy={busy}
                  onReply={reply}
                  onComment={(body) => act("Comment posted", () => apiPost("/api/pr/comment", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, body }))}
                />
              )}
              {tab === "commits" && (
                <CommitsTab
                  commits={data.commits}
                  onSelect={(sha, label) => {
                    setCommitScope({ sha, label });
                    setTab("files");
                  }}
                />
              )}
            </div>

            {/* action bar */}
            <div className="flex items-center gap-2 px-4 py-3 shrink-0 flex-wrap" style={{ borderTop: "1px solid var(--border)" }}>
              <ChecksBadge
                checks={data.checks}
                provider={provider}
                onUnsupported={() => provider && setNote(unsupportedMessage(provider))}
              />
              {pr.status === "Merged" ? (
                <div className="ml-auto flex gap-2">
                  <ActionButton disabled={busy} onClick={revert} label="Revert" color="var(--red)" primary />
                </div>
              ) : pending.length > 0 ? (
                <>
                  <span className="text-sm" style={{ color: "var(--accent)" }}>
                    {pending.length} pending comment{pending.length > 1 ? "s" : ""}
                  </span>
                  <div className="ml-auto flex gap-2">
                    <ActionButton disabled={busy} onClick={() => submitReview("NoVote")} label="Submit comments" />
                    <ActionButton disabled={busy} onClick={() => submitReview("Rejected")} label="Request changes" color="var(--red)" />
                    <ActionButton disabled={busy} onClick={() => submitReview("Approved")} label="Approve" color="var(--green)" primary />
                  </div>
                </>
              ) : (
                <div className="ml-auto flex gap-2">
                  <ActionButton disabled={busy} onClick={() => vote("Rejected")} label="Request changes" color="var(--red)" />
                  <ActionButton disabled={busy} onClick={() => vote("Approved")} label="Approve" color="var(--green)" />
                  <ActionButton
                    disabled={busy || pr.mergeable === "Conflicting" || pr.mergeable === "Blocked"}
                    title={
                      pr.mergeable === "Conflicting"
                        ? "This PR has merge conflicts"
                        : pr.mergeable === "Blocked"
                          ? "This PR is blocked (branch policy or draft)"
                          : undefined
                    }
                    onClick={merge}
                    label="Merge"
                    color="var(--magenta)"
                    primary
                  />
                </div>
              )}
            </div>
            {note && (
              <div className="px-4 pb-2 text-xs" style={{ color: note.endsWith("✓") ? "var(--green)" : "var(--red)" }}>
                {note}
              </div>
            )}
          </>
        )}
      </motion.div>
    </motion.div>
  );
}

function ActionButton({
  label,
  onClick,
  disabled,
  color = "var(--fg)",
  primary = false,
  title,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  color?: string;
  primary?: boolean;
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={title}
      className="rounded-md px-3 py-1.5 text-sm font-medium transition-opacity"
      style={{
        color: primary ? "#12151b" : color,
        background: primary ? color : "var(--panel2)",
        border: `1px solid ${primary ? color : "var(--border)"}`,
        opacity: disabled ? 0.45 : 1,
        cursor: disabled ? "not-allowed" : "pointer",
      }}
    >
      {label}
    </button>
  );
}

/** The header meta bar: who opened the PR + the reviewers with their vote marks (✓ / ✗ / ·). */
function MetaBar({ author, reviewers }: { author: string; reviewers: Reviewer[] }) {
  return (
    <div className="flex items-center gap-x-5 gap-y-1 px-5 py-2.5 flex-wrap text-xs shrink-0" style={{ borderBottom: "1px solid var(--border)", color: "var(--dim)" }}>
      <span className="flex items-center gap-1.5">
        <Avatar name={author} size={18} /> opened by <span style={{ color: "var(--fg)" }}>{author}</span>
      </span>
      {reviewers.length > 0 && (
        <span className="flex items-center gap-x-3 gap-y-1 flex-wrap">
          <span className="uppercase tracking-wider text-[10px]">Reviewers</span>
          {reviewers.map((r, i) => {
            const v = voteMeta(r.vote);
            return (
              <span key={i} className="flex items-center gap-1" title={`${r.user.display_name} — ${v.label}`}>
                <span className="w-3 text-center" style={{ color: r.vote === "NoVote" ? "var(--dim)" : v.color }}>{r.vote === "NoVote" ? "·" : v.icon}</span>
                <span style={{ color: "var(--fg)" }}>{r.user.display_name}</span>
              </span>
            );
          })}
        </span>
      )}
    </div>
  );
}

/** Action-bar checks badge: green "all passed" / red "N failed"; click opens a popover of every
 *  check. Greyed with the standard message when the provider can't report checks. */
function ChecksBadge({ checks, provider, onUnsupported }: { checks: CheckRun[]; provider: ProviderType | undefined; onUnsupported: () => void }) {
  const [open, setOpen] = useState(false);
  const supported = provider ? providerSupports(provider, "checks") : true;
  const base = "flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium whitespace-nowrap";
  if (!supported) {
    return (
      <button onClick={onUnsupported} title={provider ? unsupportedMessage(provider) : undefined} className={base} style={{ opacity: 0.55, cursor: "not-allowed", color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}>
        Checks
      </button>
    );
  }
  if (checks.length === 0) {
    return <span className={base} style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}>No checks</span>;
  }
  const failed = checks.filter((c) => c.status === "Failed").length;
  const color = failed > 0 ? "var(--red)" : "var(--green)";
  const label = failed > 0 ? `${failed} check${failed > 1 ? "s" : ""} failed` : "All checks passed";
  return (
    <div className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className={base}
        style={{ color, background: `color-mix(in srgb, ${color} 14%, transparent)`, border: `1px solid color-mix(in srgb, ${color} 30%, transparent)`, cursor: "pointer" }}
      >
        {failed > 0 ? "✗" : "✓"} {label}
      </button>
      {open && (
        <>
          <div className="fixed inset-0" style={{ zIndex: 20 }} onClick={() => setOpen(false)} />
          <div className="absolute bottom-full left-0 mb-2 w-72 rounded-lg p-1 max-h-64 overflow-auto shadow-2xl" style={{ zIndex: 21, background: "var(--panel)", border: "1px solid var(--border)" }}>
            {checks.map((c, i) => {
              const m = checkMeta(c.status);
              const inner = (
                <>
                  <span className="shrink-0" style={{ color: m.color }}>{m.icon}</span>
                  <span className="flex-1 truncate" style={{ color: "var(--fg)" }}>{c.name}</span>
                  <span className="capitalize shrink-0" style={{ color: m.color }}>{c.status.toLowerCase()}</span>
                  {c.url && <span className="shrink-0" style={{ color: "var(--dim)" }}>↗</span>}
                </>
              );
              const cls = "flex items-center gap-2 rounded px-2 py-1.5 text-xs";
              return c.url ? (
                <a key={i} href={c.url} target="_blank" rel="noreferrer" className={cls + " transition-colors"} onMouseEnter={(e) => (e.currentTarget.style.background = "var(--sel)")} onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}>
                  {inner}
                </a>
              ) : (
                <div key={i} className={cls}>{inner}</div>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}

// ---- files / diff ----

function FilesTab({
  changes,
  threads,
  pending,
  busy,
  onReply,
  onAddPending,
  onRemovePending,
  scope,
}: {
  changes: FileChange[];
  threads: CommentThread[];
  pending: LineComment[];
  busy: boolean;
  onReply: (threadId: string, body: string) => void;
  onAddPending: (c: LineComment) => void;
  onRemovePending: (index: number) => void;
  scope?: { label: string; loading: boolean; onClear: () => void };
}) {
  // Which file the left-hand list has selected (clamped in case the file set shrank).
  const [selected, setSelected] = useState(0);
  const sel = Math.min(selected, Math.max(0, changes.length - 1));
  // Scoped to one commit: show a banner with a way back to the whole-PR diff.
  const banner = scope && (
    <div className="flex items-center gap-3 rounded-lg px-3 py-2 text-xs" style={{ background: "var(--panel2)", border: "1px solid var(--border)" }}>
      <span style={{ color: "var(--dim)" }}>Showing commit</span>
      <span className="mono truncate" style={{ color: "var(--fg)" }}>{scope.label}</span>
      <button onClick={scope.onClear} className="ml-auto shrink-0" style={{ color: "var(--accent)" }}>
        ← Show all files
      </button>
    </div>
  );

  if (scope?.loading || changes.length === 0) {
    return (
      <div className="p-4 flex flex-col gap-4">
        {banner}
        {scope?.loading ? <Empty text="Loading commit diff…" /> : <Empty text={scope ? "No per-commit diff available for this provider." : "No file changes to show."} />}
      </div>
    );
  }

  const file = changes[sel];
  return (
    <div className="flex flex-col h-full">
      {banner && <div className="px-4 pt-4 shrink-0">{banner}</div>}
      <div className="flex flex-1 min-h-0">
        {/* file list */}
        <nav className="w-56 shrink-0 overflow-auto p-2 flex flex-col gap-0.5" style={{ borderRight: "1px solid var(--border)" }}>
          {changes.map((f, i) => {
            const active = i === sel;
            const k = kindTag(f.kind);
            const parts = f.path.split("/");
            const name = parts.pop() ?? f.path;
            const dir = parts.join("/");
            return (
              <button
                key={f.path}
                onClick={() => setSelected(i)}
                title={f.path}
                className="flex items-center gap-2 rounded-md px-2 py-1.5 text-left w-full transition-colors"
                style={{ background: active ? "var(--sel)" : "transparent" }}
                onMouseEnter={(e) => !active && (e.currentTarget.style.background = "var(--card)")}
                onMouseLeave={(e) => !active && (e.currentTarget.style.background = "transparent")}
              >
                <span className="mono text-xs shrink-0 w-3 text-center" style={{ color: k.color }} title={f.kind}>{k.letter}</span>
                <span className="flex flex-col min-w-0 flex-1">
                  <span className="truncate text-xs" style={{ color: "var(--fg)" }}>{name}</span>
                  {dir && <span className="truncate text-[10px]" style={{ color: "var(--dim)" }}>{dir}</span>}
                </span>
              </button>
            );
          })}
        </nav>
        {/* selected file's diff */}
        <div className="flex-1 overflow-auto p-4 min-w-0">
          <FileDiff key={file.path} file={file} threads={threads} pending={pending} busy={busy} onReply={onReply} onAddPending={onAddPending} onRemovePending={onRemovePending} />
        </div>
      </div>
    </div>
  );
}

/** Single-letter change-kind tag for the file list, coloured like a diff. */
function kindTag(kind: FileChangeKind): { letter: string; color: string } {
  switch (kind) {
    case "Added":
      return { letter: "A", color: "var(--green)" };
    case "Deleted":
      return { letter: "D", color: "var(--red)" };
    case "Renamed":
      return { letter: "R", color: "var(--cyan)" };
    default:
      return { letter: "M", color: "var(--yellow)" };
  }
}

function FileDiff({
  file,
  threads,
  pending,
  busy,
  onReply,
  onAddPending,
  onRemovePending,
}: {
  file: FileChange;
  threads: CommentThread[];
  pending: LineComment[];
  busy: boolean;
  onReply: (threadId: string, body: string) => void;
  onAddPending: (c: LineComment) => void;
  onRemovePending: (index: number) => void;
}) {
  const [composeLine, setComposeLine] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const lines = useMemo(() => (file.patch ? parsePatch(file.patch) : []), [file.patch]);
  const fileThreads = threads.filter((t) => t.file_path === file.path && t.line != null);

  const commit = (line: number) => {
    if (draft.trim()) onAddPending({ path: file.path, line, side: "New", body: draft.trim() });
    setDraft("");
    setComposeLine(null);
  };

  return (
    <div className="rounded-lg overflow-hidden" style={{ border: "1px solid var(--border)" }}>
      <div className="flex items-center gap-3 px-3 py-2 text-sm" style={{ background: "var(--panel)", borderBottom: "1px solid var(--border)" }}>
        <span className="mono truncate" style={{ color: "var(--fg)" }}>{file.path}</span>
        <span className="mono text-xs ml-auto shrink-0">
          <span style={{ color: "var(--green)" }}>+{file.additions}</span> <span style={{ color: "var(--red)" }}>−{file.deletions}</span>
        </span>
      </div>
      {lines.length === 0 ? (
        <div className="px-3 py-2 text-xs" style={{ color: "var(--dim)" }}>
          {file.kind} — no inline diff available.
        </div>
      ) : (
        <div className="mono text-xs overflow-x-auto">
          {lines.map((ln, i) => {
            const bg =
              ln.kind === "add" ? "rgba(135,215,135,0.10)" : ln.kind === "del" ? "rgba(255,135,135,0.10)" : "transparent";
            const marker = ln.kind === "add" ? "+" : ln.kind === "del" ? "−" : ln.kind === "hunk" ? "" : " ";
            const color = ln.kind === "hunk" ? "var(--cyan)" : ln.kind === "meta" ? "var(--dim)" : "var(--fg)";
            const canComment = (ln.kind === "add" || ln.kind === "context") && ln.newLine != null;
            const lineThreads = ln.newLine != null ? fileThreads.filter((t) => t.line === ln.newLine) : [];
            const linePending = ln.newLine != null ? pending.map((p, idx) => ({ p, idx })).filter((x) => x.p.path === file.path && x.p.line === ln.newLine) : [];
            return (
              <div key={i}>
                <div
                  className="group flex items-stretch"
                  style={{ background: bg, borderLeft: `2px solid ${ln.kind === "add" ? "var(--green)" : ln.kind === "del" ? "var(--red)" : "transparent"}` }}
                >
                  <span className="w-10 text-right pr-2 select-none shrink-0" style={{ color: "var(--dim)" }}>
                    {ln.newLine ?? ln.oldLine ?? ""}
                  </span>
                  {canComment ? (
                    <span className="w-6 shrink-0 flex items-center justify-center">
                      <button
                        className="flex items-center justify-center rounded opacity-0 group-hover:opacity-100 transition-opacity hover:brightness-110"
                        style={{ width: 16, height: 16, background: "var(--accent)", color: "var(--bg)", fontWeight: 600, lineHeight: 1 }}
                        title="Comment on this line"
                        aria-label="Comment on this line"
                        onClick={() => setComposeLine(ln.newLine!)}
                      >
                        +
                      </button>
                    </span>
                  ) : (
                    <span className="w-6 shrink-0" />
                  )}
                  <span className="pr-3 whitespace-pre" style={{ color }}>
                    {marker}
                    {ln.text}
                  </span>
                </div>
                {lineThreads.map((t) => (
                  <ThreadBox key={t.id} thread={t} busy={busy} onReply={onReply} />
                ))}
                {linePending.map(({ p, idx }) => (
                  <PendingBox key={idx} body={p.body} onRemove={() => onRemovePending(idx)} />
                ))}
                {composeLine === ln.newLine && canComment && (
                  <div className="p-2" style={{ background: "var(--panel)" }}>
                    <textarea
                      autoFocus
                      value={draft}
                      onChange={(e) => setDraft(e.target.value)}
                      placeholder="Leave a comment on this line…"
                      className="w-full rounded p-2 text-xs outline-none"
                      style={{ background: "var(--bg)", color: "var(--fg)", border: "1px solid var(--border)" }}
                      rows={2}
                    />
                    <div className="flex gap-2 mt-1.5">
                      <ActionButton label="Add comment" onClick={() => commit(ln.newLine!)} primary color="var(--accent)" />
                      <ActionButton label="Cancel" onClick={() => { setDraft(""); setComposeLine(null); }} />
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function ThreadBox({ thread, busy, onReply }: { thread: CommentThread; busy: boolean; onReply: (threadId: string, body: string) => void }) {
  const [replying, setReplying] = useState(false);
  const [draft, setDraft] = useState("");
  const submit = () => {
    const body = draft.trim();
    if (!body) return;
    onReply(thread.id, body);
    setDraft("");
    setReplying(false);
  };
  return (
    <div className="px-3 py-2 text-xs" style={{ background: "var(--panel)", borderTop: "1px solid var(--border)" }}>
      {thread.comments.map((c) => (
        <div key={c.id} className="flex gap-2 py-0.5">
          <Avatar name={c.author.display_name} size={16} />
          <span>
            <span className="font-medium" style={{ color: "var(--fg)" }}>{c.author.display_name}</span>{" "}
            <span style={{ color: "var(--dim)" }}>{c.body}</span>
          </span>
        </div>
      ))}
      {replying ? (
        <div className="mt-1.5 flex flex-col gap-1.5">
          <textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="Reply…"
            rows={2}
            autoFocus
            className="w-full rounded p-2 text-xs outline-none"
            style={{ background: "var(--card)", color: "var(--fg)", border: "1px solid var(--border)" }}
          />
          <div className="flex items-center gap-2">
            <ActionButton label="Reply" disabled={busy || !draft.trim()} primary color="var(--accent)" onClick={submit} />
            <button className="text-xs" style={{ color: "var(--dim)" }} onClick={() => { setReplying(false); setDraft(""); }}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button className="mt-1 text-xs" style={{ color: "var(--accent)" }} onClick={() => setReplying(true)}>
          ↳ Reply
        </button>
      )}
    </div>
  );
}

function PendingBox({ body, onRemove }: { body: string; onRemove: () => void }) {
  return (
    <div className="flex items-center gap-2 px-3 py-2 text-xs" style={{ background: "rgba(95,175,255,0.10)", borderTop: "1px solid var(--border)" }}>
      <span className="rounded px-1.5 py-0.5" style={{ background: "var(--accent)", color: "#12151b" }}>pending</span>
      <span className="flex-1" style={{ color: "var(--fg)" }}>{body}</span>
      <button onClick={onRemove} style={{ color: "var(--dim)" }} title="Remove">✕</button>
    </div>
  );
}

// ---- conversation / commits ----

function ConversationTab({
  threads,
  description,
  timeline,
  busy,
  onReply,
  onComment,
}: {
  threads: CommentThread[];
  description?: string | null;
  timeline: TimelineEvent[];
  busy: boolean;
  onReply: (threadId: string, body: string) => void;
  onComment: (body: string) => void;
}) {
  const [draft, setDraft] = useState("");
  const general = threads.filter((t) => !t.file_path);
  return (
    <div className="p-4 flex flex-col gap-3 max-w-3xl">
      {description && (
        <div className="rounded-lg p-3 text-sm whitespace-pre-wrap" style={{ background: "var(--card)", border: "1px solid var(--border)", color: "var(--fg)" }}>
          {description}
        </div>
      )}
      {timeline.length > 0 && <Timeline events={timeline} />}
      {general.length === 0 && <div className="text-xs px-1" style={{ color: "var(--dim)" }}>No comments yet.</div>}
      {general.map((t) => (
        <ThreadBox key={t.id} thread={t} busy={busy} onReply={onReply} />
      ))}
      <div className="mt-2">
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Add a comment…"
          rows={3}
          className="w-full rounded-lg p-3 text-sm outline-none"
          style={{ background: "var(--card)", color: "var(--fg)", border: "1px solid var(--border)" }}
        />
        <div className="mt-2">
          <ActionButton
            label="Comment"
            disabled={busy || !draft.trim()}
            primary
            color="var(--accent)"
            onClick={() => {
              onComment(draft.trim());
              setDraft("");
            }}
          />
        </div>
      </div>
    </div>
  );
}

function CommitsTab({ commits, onSelect }: { commits: Commit[]; onSelect: (sha: string, label: string) => void }) {
  if (commits.length === 0) return <Empty text="No commits to show." />;
  return (
    <div className="p-4 flex flex-col gap-1.5 max-w-3xl">
      {commits.map((c) => {
        const first = c.message.split("\n")[0];
        return (
          <button
            key={c.sha}
            onClick={() => onSelect(c.sha, `${c.sha.slice(0, 7)} ${first}`)}
            title="View this commit's diff"
            className="group flex items-center gap-3 rounded-lg px-3 py-2 text-left w-full transition-colors"
            style={{ background: "var(--card)", border: "1px solid var(--border)", cursor: "pointer" }}
            onMouseEnter={(e) => (e.currentTarget.style.background = "var(--card-hover)")}
            onMouseLeave={(e) => (e.currentTarget.style.background = "var(--card)")}
          >
            <span className="mono text-xs shrink-0" style={{ color: "var(--cyan)" }}>{c.sha.slice(0, 7)}</span>
            <span className="flex-1 truncate text-sm" style={{ color: "var(--fg)" }}>{first}</span>
            <span className="text-xs shrink-0" style={{ color: "var(--dim)" }}>{c.author}</span>
            <span className="text-xs shrink-0 opacity-0 transition-opacity group-hover:opacity-100" style={{ color: "var(--accent)" }}>diff →</span>
          </button>
        );
      })}
    </div>
  );
}

function Empty({ text }: { text: string }) {
  return (
    <div className="py-16 text-center text-sm" style={{ color: "var(--dim)" }}>
      {text}
    </div>
  );
}

function shortRef(ref?: string | null): string {
  return ref ? ref.replace(/^refs\/heads\//, "") : "";
}
