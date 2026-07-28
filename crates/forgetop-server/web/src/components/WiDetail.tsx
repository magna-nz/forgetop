import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { AnimatePresence } from "framer-motion";
import { useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost, useWiDetail, wiDetailKey } from "../api";
import { relativeTime, wiStateColor } from "../format";
import type { CommentThread, User, WiRef, WorkItem } from "../types";
import { Avatar, Chip, Pill, SlideOver, Timeline } from "./ui";

// ---- opener context ----

const WiOpenerCtx = createContext<(ref: WiRef) => void>(() => {});
export const useWiOpener = () => useContext(WiOpenerCtx);

export function WiDetailProvider({ children }: { children: ReactNode }) {
  const [ref, setRef] = useState<WiRef | null>(null);
  const open = useCallback((r: WiRef) => setRef(r), []);
  return (
    <WiOpenerCtx.Provider value={open}>
      {children}
      <AnimatePresence>{ref && <WiDetailPanel wiRef={ref} onClose={() => setRef(null)} />}</AnimatePresence>
    </WiOpenerCtx.Provider>
  );
}

// ---- panel ----

function WiDetailPanel({ wiRef, onClose }: { wiRef: WiRef; onClose: () => void }) {
  const { data, isLoading, error } = useWiDetail(wiRef);
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    setNote(null);
    setEditing(false);
  }, [wiRef.conn, wiRef.id]);

  const refresh = () => {
    qc.invalidateQueries({ queryKey: wiDetailKey(wiRef) });
    qc.invalidateQueries({ queryKey: ["work-items"] });
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

  const wi = data?.work_item;
  const color = wi ? wiStateColor(wi.state, wi.state_category) : "var(--dim)";

  const header = (
    <>
      {wi && <Pill icon="●" label={wi.state} color={color} />}
      <div className="flex-1 min-w-0">
        <div className="font-medium truncate" style={{ color: "var(--fg)" }}>
          {wi?.title ?? (isLoading ? "Loading…" : "Work item")}
        </div>
        {wi && (
          <div className="flex items-center gap-2 mt-1 text-xs" style={{ color: "var(--dim)" }}>
            {wi.identifier && <span className="mono">{wi.identifier}</span>}
            {wi.work_item_type && <Chip>{wi.work_item_type}</Chip>}
            <span>{relativeTime(wi.updated_at)}</span>
          </div>
        )}
      </div>
      {wi?.url && (
        <a href={wi.url} target="_blank" rel="noreferrer" title="Open in provider" className="text-sm px-2 py-1" style={{ color: "var(--dim)" }}>
          ↗
        </a>
      )}
    </>
  );

  return (
    <SlideOver onClose={onClose} header={header}>
      {error && <div className="p-6 text-sm" style={{ color: "var(--red)" }}>Couldn't load this work item.</div>}

      {wi && data && (
        <div className="p-5 flex flex-col gap-4 max-w-3xl">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-xs" style={{ color: "var(--dim)" }}>
            <AssigneePicker
              wiRef={wiRef}
              assignee={wi.assignee}
              busy={busy}
              onPick={(assigneeId) =>
                act(assigneeId ? "Reassigned" : "Unassigned", () =>
                  apiPost("/api/wi/assignee", { conn: wiRef.conn, repo: wiRef.repo, id: wiRef.id, assignee_id: assigneeId }),
                )
              }
            />
            <MoveState wiRef={wiRef} current={wi.state} busy={busy} onMove={(state) => act(`Moved to ${state}`, () => apiPost("/api/wi/state", { conn: wiRef.conn, repo: wiRef.repo, id: wiRef.id, state }))} />
            {!editing && (
              <button onClick={() => setEditing(true)} className="rounded px-2 py-1" style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}>
                Edit
              </button>
            )}
          </div>

          {editing ? (
            <EditFields
              wi={wi}
              busy={busy}
              onCancel={() => setEditing(false)}
              onSave={(title, description) =>
                act("Saved", async () => {
                  await apiPost("/api/wi/update", { conn: wiRef.conn, repo: wiRef.repo, id: wiRef.id, title, description });
                  setEditing(false);
                })
              }
            />
          ) : (
            wi.description && (
              <div className="rounded-lg p-3 text-sm whitespace-pre-wrap" style={{ background: "var(--card)", border: "1px solid var(--border)", color: "var(--fg)" }}>
                {wi.description}
              </div>
            )
          )}

          {data.timeline.length > 0 && <Timeline events={data.timeline} />}

          <Comments
            threads={data.threads}
            busy={busy}
            onComment={(body) => act("Comment posted", () => apiPost("/api/wi/comment", { conn: wiRef.conn, repo: wiRef.repo, id: wiRef.id, body }))}
          />

          {note && (
            <div className="text-xs" style={{ color: note.endsWith("✓") ? "var(--green)" : "var(--red)" }}>
              {note}
            </div>
          )}
        </div>
      )}
    </SlideOver>
  );
}

// ---- move-state control ----

function MoveState({ wiRef, current, busy, onMove }: { wiRef: WiRef; current: string; busy: boolean; onMove: (state: string) => void }) {
  const [open, setOpen] = useState(false);
  const [states, setStates] = useState<string[] | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (open && states === null && !loading) {
      setLoading(true);
      apiGet<string[]>(`/api/wi/states?conn=${encodeURIComponent(wiRef.conn)}&id=${encodeURIComponent(wiRef.id)}${wiRef.repo ? `&repo=${encodeURIComponent(wiRef.repo)}` : ""}`)
        .then(setStates)
        .catch(() => setStates([]))
        .finally(() => setLoading(false));
    }
  }, [open, states, loading, wiRef.conn, wiRef.id]);

  const options = (states ?? []).filter((s) => s.toLowerCase() !== current.toLowerCase());

  return (
    <div className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        disabled={busy}
        className="text-xs rounded px-2 py-1"
        style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}
      >
        Move ▾
      </button>
      {open && (
        <div className="absolute left-0 mt-1 z-10 rounded-md py-1 min-w-40 shadow-lg" style={{ background: "var(--panel)", border: "1px solid var(--border)" }}>
          {loading && <div className="px-3 py-1.5" style={{ color: "var(--dim)" }}>Loading…</div>}
          {!loading && options.length === 0 && <div className="px-3 py-1.5" style={{ color: "var(--dim)" }}>No transitions available</div>}
          {options.map((s) => (
            <button
              key={s}
              disabled={busy}
              onClick={() => {
                onMove(s);
                setOpen(false);
              }}
              className="block w-full text-left px-3 py-1.5"
              style={{ color: "var(--fg)" }}
              onMouseEnter={(e) => (e.currentTarget.style.background = "var(--sel)")}
              onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
            >
              → {s}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ---- assignee picker ----

function AssigneePicker({ wiRef, assignee, busy, onPick }: { wiRef: WiRef; assignee?: User | null; busy: boolean; onPick: (assigneeId: string | null) => void }) {
  const [open, setOpen] = useState(false);
  const [users, setUsers] = useState<User[] | null>(null);
  const [loading, setLoading] = useState(false);

  // Fetch the assignable users lazily, the first time the menu opens.
  useEffect(() => {
    if (open && users === null && !loading) {
      setLoading(true);
      apiGet<User[]>(`/api/wi/assignees?conn=${encodeURIComponent(wiRef.conn)}&id=${encodeURIComponent(wiRef.id)}${wiRef.repo ? `&repo=${encodeURIComponent(wiRef.repo)}` : ""}`)
        .then(setUsers)
        .catch(() => setUsers([]))
        .finally(() => setLoading(false));
    }
  }, [open, users, loading, wiRef.conn, wiRef.id]);

  const pick = (assigneeId: string | null) => {
    onPick(assigneeId);
    setOpen(false);
  };

  return (
    <div className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        disabled={busy}
        className="flex items-center gap-1.5 rounded px-2 py-1"
        style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}
      >
        {assignee ? (
          <>
            <Avatar name={assignee.display_name} size={18} /> {assignee.display_name}
          </>
        ) : (
          "Unassigned"
        )}{" "}
        ▾
      </button>
      {open && (
        <div className="absolute left-0 mt-1 z-10 rounded-md py-1 min-w-48 max-h-64 overflow-auto shadow-lg" style={{ background: "var(--panel)", border: "1px solid var(--border)" }}>
          {loading && <div className="px-3 py-1.5" style={{ color: "var(--dim)" }}>Loading…</div>}
          {/* Only offer Unassign when there's actually an assignee — avoids a no-op clear
              (and the Azure "remove an unset field" 400), mirroring how Move hides the current state. */}
          {assignee && (
            <button
              disabled={busy}
              onClick={() => pick(null)}
              className="block w-full text-left px-3 py-1.5"
              style={{ color: "var(--dim)" }}
              onMouseEnter={(e) => (e.currentTarget.style.background = "var(--sel)")}
              onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
            >
              Unassign
            </button>
          )}
          {(users ?? []).map((u) => (
            <button
              key={u.id}
              disabled={busy}
              onClick={() => pick(u.id)}
              className="flex items-center gap-2 w-full text-left px-3 py-1.5"
              style={{ color: "var(--fg)" }}
              onMouseEnter={(e) => (e.currentTarget.style.background = "var(--sel)")}
              onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
            >
              <Avatar name={u.display_name} size={18} /> {u.display_name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ---- title / description edit ----

function EditFields({ wi, busy, onSave, onCancel }: { wi: WorkItem; busy: boolean; onSave: (title?: string, description?: string) => void; onCancel: () => void }) {
  const [title, setTitle] = useState(wi.title);
  const [description, setDescription] = useState(wi.description ?? "");
  const changedTitle = title !== wi.title;
  const changedDesc = description !== (wi.description ?? "");
  const dirty = changedTitle || changedDesc;
  const canSave = !busy && dirty && title.trim().length > 0;
  return (
    <div className="rounded-lg p-3 flex flex-col gap-2" style={{ background: "var(--card)", border: "1px solid var(--border)" }}>
      <input
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        placeholder="Title"
        className="rounded-md px-2 py-1.5 text-sm outline-none"
        style={{ background: "var(--bg)", color: "var(--fg)", border: "1px solid var(--border)" }}
      />
      <textarea
        value={description}
        onChange={(e) => setDescription(e.target.value)}
        placeholder="Description"
        rows={6}
        className="rounded-md px-2 py-1.5 text-sm outline-none"
        style={{ background: "var(--bg)", color: "var(--fg)", border: "1px solid var(--border)" }}
      />
      <div className="flex gap-2">
        <button
          disabled={!canSave}
          onClick={() => onSave(changedTitle ? title : undefined, changedDesc ? description : undefined)}
          className="rounded-md px-3 py-1.5 text-sm font-medium"
          style={{ color: "#12151b", background: "var(--accent)", opacity: canSave ? 1 : 0.45, cursor: canSave ? "pointer" : "not-allowed" }}
        >
          Save
        </button>
        <button onClick={onCancel} className="rounded-md px-3 py-1.5 text-sm" style={{ color: "var(--dim)", border: "1px solid var(--border)", background: "var(--panel2)" }}>
          Cancel
        </button>
      </div>
    </div>
  );
}

// ---- comments ----

function Comments({ threads, busy, onComment }: { threads: CommentThread[]; busy: boolean; onComment: (body: string) => void }) {
  const [draft, setDraft] = useState("");
  return (
    <div className="flex flex-col gap-3">
      {threads.length === 0 && <div className="text-sm" style={{ color: "var(--dim)" }}>No comments yet.</div>}
      {threads.map((t) => (
        <div key={t.id} className="rounded-lg px-3 py-2 text-xs" style={{ background: "var(--card)", border: "1px solid var(--border)" }}>
          {t.comments.map((c) => (
            <div key={c.id} className="flex gap-2 py-0.5">
              <Avatar name={c.author.display_name} size={16} />
              <span>
                <span className="font-medium" style={{ color: "var(--fg)" }}>{c.author.display_name}</span>{" "}
                <span style={{ color: "var(--dim)" }}>{c.body}</span>
              </span>
            </div>
          ))}
        </div>
      ))}
      <div>
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Add a comment…"
          rows={3}
          className="w-full rounded-lg p-3 text-sm outline-none"
          style={{ background: "var(--card)", color: "var(--fg)", border: "1px solid var(--border)" }}
        />
        <div className="mt-2">
          <button
            disabled={busy || !draft.trim()}
            onClick={() => {
              onComment(draft.trim());
              setDraft("");
            }}
            className="rounded-md px-3 py-1.5 text-sm font-medium"
            style={{
              color: "#12151b",
              background: "var(--accent)",
              opacity: busy || !draft.trim() ? 0.45 : 1,
              cursor: busy || !draft.trim() ? "not-allowed" : "pointer",
            }}
          >
            Comment
          </button>
        </div>
      </div>
    </div>
  );
}
