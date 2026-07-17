import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { useQueryClient } from "@tanstack/react-query";
import { apiPost, usePrDetail } from "../api";
import { prStatusMeta, relativeTime, voteMeta } from "../format";
import { parsePatch } from "../diff";
import type { CheckRun, CommentThread, Commit, FileChange, LineComment, MergeableState, PrRef, Reviewer } from "../types";
import { Avatar, Chip, Pill } from "./ui";

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
  const qc = useQueryClient();
  const [tab, setTab] = useState<Tab>("conversation");
  const [pending, setPending] = useState<LineComment[]>([]);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const requestClose = () => {
    if (pending.length > 0 && !window.confirm(`Discard ${pending.length} unsubmitted comment(s)?`)) return;
    onClose();
  };

  useEffect(() => {
    setTab("conversation");
    setPending([]);
    setNote(null);
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
    qc.invalidateQueries({ queryKey: ["pr-detail", prRef.conn, prRef.id] });
    qc.invalidateQueries({ queryKey: ["prs"] });
    qc.invalidateQueries({ queryKey: ["launchpad"] });
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

  const vote = (v: "Approved" | "Rejected") =>
    act(v === "Approved" ? "Approved" : "Requested changes", () =>
      apiPost("/api/pr/vote", { conn: prRef.conn, id: prRef.id, vote: v }),
    );
  const merge = () =>
    act("Merged", async () => {
      await apiPost("/api/pr/merge", { conn: prRef.conn, id: prRef.id, strategy: "Merge" });
      setTimeout(onClose, 500);
    });
  const revert = () => act("Revert requested", () => apiPost("/api/pr/revert", { conn: prRef.conn, id: prRef.id }));
  const submitReview = (event: "Approved" | "Rejected" | "NoVote") =>
    act("Review submitted", async () => {
      await apiPost("/api/pr/review", { conn: prRef.conn, id: prRef.id, event, comments: pending });
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
            <MetaBar mergeable={pr.mergeable} reviewers={pr.reviewers} author={pr.author.display_name} checks={data.checks} />

            {/* tabs */}
            <div className="flex items-center gap-1 px-4 shrink-0" style={{ borderBottom: "1px solid var(--border)" }}>
              {(["conversation", "commits", "files"] as Tab[]).map((t) => (
                <button
                  key={t}
                  onClick={() => setTab(t)}
                  className="px-3 py-2 text-sm capitalize"
                  style={{
                    color: tab === t ? "var(--fg)" : "var(--dim)",
                    borderBottom: tab === t ? "2px solid var(--accent)" : "2px solid transparent",
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
                  changes={data.changes}
                  threads={data.threads}
                  pending={pending}
                  onAddPending={(c) => setPending((p) => [...p, c])}
                  onRemovePending={(i) => setPending((p) => p.filter((_, k) => k !== i))}
                />
              )}
              {tab === "conversation" && (
                <ConversationTab
                  threads={data.threads}
                  description={pr.description}
                  busy={busy}
                  onComment={(body) => act("Comment posted", () => apiPost("/api/pr/comment", { conn: prRef.conn, id: prRef.id, body }))}
                />
              )}
              {tab === "commits" && <CommitsTab commits={data.commits} />}
            </div>

            {/* action bar */}
            <div className="flex items-center gap-2 px-4 py-3 shrink-0 flex-wrap" style={{ borderTop: "1px solid var(--border)" }}>
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
                  <ActionButton disabled={busy || pr.mergeable === "Conflicting"} onClick={merge} label="Merge" color="var(--magenta)" primary />
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
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  color?: string;
  primary?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
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

function MetaBar({
  mergeable,
  reviewers,
  author,
  checks,
}: {
  mergeable: MergeableState;
  reviewers: Reviewer[];
  author: string;
  checks: CheckRun[];
}) {
  const passed = checks.filter((c) => c.status === "Passed").length;
  const failed = checks.filter((c) => c.status === "Failed").length;
  return (
    <div className="flex items-center gap-x-4 gap-y-2 px-5 py-2.5 flex-wrap text-xs shrink-0" style={{ borderBottom: "1px solid var(--border)", color: "var(--dim)" }}>
      <span className="flex items-center gap-1.5">
        <Avatar name={author} size={18} /> {author}
      </span>
      {reviewers.length > 0 && (
        <span className="flex items-center gap-2">
          reviewers:
          {reviewers.map((r, i) => {
            const v = voteMeta(r.vote);
            return (
              <span key={i} title={`${r.user.display_name} — ${v.label}`} style={{ color: v.color }}>
                {v.icon} {r.user.display_name}
              </span>
            );
          })}
        </span>
      )}
      {checks.length > 0 && (
        <span>
          checks: <span style={{ color: "var(--green)" }}>{passed}✓</span> {failed > 0 && <span style={{ color: "var(--red)" }}>{failed}✗</span>}
        </span>
      )}
      <span style={{ color: mergeable === "Conflicting" ? "var(--red)" : mergeable === "Mergeable" ? "var(--green)" : "var(--dim)" }}>
        {mergeable === "Conflicting" ? "conflicts" : mergeable === "Mergeable" ? "mergeable" : mergeable.toLowerCase()}
      </span>
    </div>
  );
}

// ---- files / diff ----

function FilesTab({
  changes,
  threads,
  pending,
  onAddPending,
  onRemovePending,
}: {
  changes: FileChange[];
  threads: CommentThread[];
  pending: LineComment[];
  onAddPending: (c: LineComment) => void;
  onRemovePending: (index: number) => void;
}) {
  if (changes.length === 0) return <Empty text="No file changes to show." />;
  return (
    <div className="p-4 flex flex-col gap-4">
      {changes.map((f) => (
        <FileDiff key={f.path} file={f} threads={threads} pending={pending} onAddPending={onAddPending} onRemovePending={onRemovePending} />
      ))}
    </div>
  );
}

function FileDiff({
  file,
  threads,
  pending,
  onAddPending,
  onRemovePending,
}: {
  file: FileChange;
  threads: CommentThread[];
  pending: LineComment[];
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
                  <ThreadBox key={t.id} thread={t} />
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

function ThreadBox({ thread }: { thread: CommentThread }) {
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
  busy,
  onComment,
}: {
  threads: CommentThread[];
  description?: string | null;
  busy: boolean;
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
      {general.length === 0 && !description && <Empty text="No conversation yet." />}
      {general.map((t) => (
        <ThreadBox key={t.id} thread={t} />
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

function CommitsTab({ commits }: { commits: Commit[] }) {
  if (commits.length === 0) return <Empty text="No commits to show." />;
  return (
    <div className="p-4 flex flex-col gap-1.5 max-w-3xl">
      {commits.map((c) => (
        <div key={c.sha} className="flex items-center gap-3 rounded-lg px-3 py-2" style={{ background: "var(--card)", border: "1px solid var(--border)" }}>
          <span className="mono text-xs shrink-0" style={{ color: "var(--cyan)" }}>{c.sha.slice(0, 7)}</span>
          <span className="flex-1 truncate text-sm" style={{ color: "var(--fg)" }}>{c.message.split("\n")[0]}</span>
          <span className="text-xs shrink-0" style={{ color: "var(--dim)" }}>{c.author}</span>
        </div>
      ))}
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
