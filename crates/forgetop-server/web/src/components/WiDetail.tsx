import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost, useWiDetail, wiDetailKey } from "../api";
import { relativeTime, wiStateColor, wiStateGlyph } from "../format";
import type { CommentThread, User, WiRef, WorkItem } from "../types";
import { addWiThread, localThread, meAsUser, patchWorkItem } from "../optimistic";
import { Avatar, Button, claimPane, GroupHeading, PaneHeader, SlideOver, Timeline, usePaneRegistration } from "./ui";
import { Icon } from "./icons";

// ---- opener context ----

const WiOpenerCtx = createContext<(ref: WiRef) => void>(() => {});
export const useWiOpener = () => useContext(WiOpenerCtx);
/** The work item open in the pane, so a list can mark its row as selected. */
const WiCurrentCtx = createContext<WiRef | null>(null);
export const useCurrentWi = () => useContext(WiCurrentCtx);

export function WiDetailProvider({ children }: { children: ReactNode }) {
  const [ref, setRef] = useState<WiRef | null>(null);
  const close = useCallback(() => setRef(null), []);
  usePaneRegistration("wi", close);
  // One pane at a time: opening a work item closes a PR or run pane (after its guard, if any).
  const open = useCallback((r: WiRef) => {
    if (claimPane("wi")) setRef(r);
  }, []);
  return (
    <WiOpenerCtx.Provider value={open}>
      <WiCurrentCtx.Provider value={ref}>
        {children}
        {ref && <WiDetailPanel wiRef={ref} onClose={close} />}
      </WiCurrentCtx.Provider>
    </WiOpenerCtx.Provider>
  );
}

// ---- local helpers (candidates for ui.tsx) ----

/** An icon-only link styled as `IconButton`: opening the item at its provider is navigation, so
 *  it stays a real `<a>`. */
function ExternalIconLink({ href, label }: { href: string; label: string }) {
  return (
    <a href={href} target="_blank" rel="noreferrer" aria-label={label} title={label} className="ft-btn ft-btn-icon">
      <Icon name="external" size={15} />
    </a>
  );
}

/** A small menu that opens above its trigger (the triggers live in the pane's footer). */
function MenuPanel({ children, wide = false }: { children: ReactNode; wide?: boolean }) {
  return (
    <div
      className={`absolute left-0 bottom-full mb-1 z-10 py-1 max-h-64 overflow-auto ${wide ? "min-w-52" : "min-w-44"}`}
      style={{ background: "var(--raised)", border: "1px solid var(--border-strong)", borderRadius: 6, boxShadow: "0 8px 24px rgba(0,0,0,0.35)" }}
    >
      {children}
    </div>
  );
}

const MENU_ITEM = "flex items-center gap-2 w-full h-7 px-3 text-left text-[12.5px] hover:bg-[var(--hover)] disabled:opacity-45";

const Sep = () => <span style={{ color: "var(--fg3)" }}>·</span>;

// ---- panel ----

/** What the last write said: `ok` once it is done (or optimistically assumed done). */
interface Note {
  text: string;
  ok: boolean;
}

function WiDetailPanel({ wiRef, onClose }: { wiRef: WiRef; onClose: () => void }) {
  const { data, isLoading, error } = useWiDetail(wiRef);
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<Note | null>(null);
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

  // Every write here is optimistic: the caller patches the caches before calling this, so the
  // note says it's done at once rather than when the provider answers.
  const act = async (label: string, fn: () => Promise<void>) => {
    setBusy(true);
    setNote({ text: label, ok: true });
    try {
      await fn();
      setNote({ text: label, ok: true });
    } catch (e) {
      setNote({ text: e instanceof Error ? e.message : String(e), ok: false });
    } finally {
      // Refreshed on both paths: after a success it confirms the optimistic patch the caller
      // already applied, after a failure it is the rollback — the server's truth replaces it.
      refresh();
      setBusy(false);
    }
  };

  const wi = data?.work_item;
  const color = wi ? wiStateColor(wi.state, wi.state_category) : "var(--fg3)";

  const header = (
    <PaneHeader
      glyph={wi ? wiStateGlyph(wi.state, wi.state_category) : undefined}
      color={color}
      title={wi?.title ?? (isLoading ? "Loading…" : "Work item")}
      ident={wi?.identifier ?? undefined}
      actions={wi?.url ? <ExternalIconLink href={wi.url} label="Open in provider" /> : undefined}
    >
      {wi && (
        <>
          <span className="font-medium" style={{ color }}>
            {wi.state}
          </span>
          {wi.work_item_type && (
            <>
              <Sep />
              <span>{wi.work_item_type}</span>
            </>
          )}
          <Sep />
          {wi.assignee ? (
            <span className="inline-flex items-center gap-1.5">
              <Avatar name={wi.assignee.display_name} size={18} />
              {wi.assignee.display_name}
            </span>
          ) : (
            <span style={{ color: "var(--fg3)" }}>Unassigned</span>
          )}
          {wi.parent && (
            <>
              <Sep />
              <span className="truncate" style={{ color: "var(--fg2)" }}>
                {wi.parent.kind} · {wi.parent.title}
              </span>
            </>
          )}
        </>
      )}
    </PaneHeader>
  );

  const footer = wi ? (
    <>
      <AssigneePicker
        wiRef={wiRef}
        assignee={wi.assignee}
        busy={busy}
        onPick={(user) => {
          // The picker already holds the full `User` it listed, so the new assignee is
          // knowable locally — name and avatar can change on the click.
          patchWorkItem(qc, wiRef, { assignee: user });
          act(user ? "Reassigned" : "Unassigned", () =>
            apiPost("/api/wi/assignee", { conn: wiRef.conn, repo: wiRef.repo, id: wiRef.id, assignee_id: user?.id ?? null }),
          );
        }}
      />
      <MoveState
        wiRef={wiRef}
        current={wi.state}
        busy={busy}
        onMove={(state) => {
          // Only the state *name* is knowable: /api/wi/states answers with raw names, so
          // `state_category` — which drives the state colour and the Command Center bucket —
          // is deliberately left as it was until the refetch supplies the real one.
          patchWorkItem(qc, wiRef, { state });
          act(`Moved to ${state}`, () => apiPost("/api/wi/state", { conn: wiRef.conn, repo: wiRef.repo, id: wiRef.id, state }));
        }}
      />
      {!editing && (
        <Button quiet onClick={() => setEditing(true)}>
          Edit
        </Button>
      )}
      {note && (
        <span
          role="status"
          className="ml-auto inline-flex items-center gap-1.5 text-[12px] min-w-0 truncate"
          style={{ color: note.ok ? "var(--green)" : "var(--red)" }}
        >
          <Icon name={note.ok ? "check" : "x-circle"} size={13} />
          {note.text}
        </span>
      )}
    </>
  ) : undefined;

  const facts: [string, ReactNode][] = [];
  if (wi?.repository) facts.push(["Repository", <span className="mono">{wi.repository}</span>]);
  if (wi?.parent)
    facts.push([
      wi.parent.kind,
      wi.parent.url ? (
        <a href={wi.parent.url} target="_blank" rel="noreferrer" style={{ color: "var(--accent-fg)" }}>
          {wi.parent.title}
        </a>
      ) : (
        wi.parent.title
      ),
    ]);
  if (wi?.created_at && relativeTime(wi.created_at)) facts.push(["Created", <span className="num">{relativeTime(wi.created_at)}</span>]);
  if (wi?.updated_at && relativeTime(wi.updated_at)) facts.push(["Updated", <span className="num">{relativeTime(wi.updated_at)}</span>]);

  return (
    <SlideOver onClose={onClose} header={header} footer={footer} width="md" label="Work item">
      {error && (
        <div className="px-5 py-6 text-[13px]" style={{ color: "var(--red)" }}>
          Couldn't load this work item.
        </div>
      )}

      {wi && data && (
        <div className="px-5 py-4 flex flex-col gap-5">
          {editing ? (
            <EditFields
              wi={wi}
              busy={busy}
              onCancel={() => setEditing(false)}
              onSave={(title, description) => {
                // `EditFields` only passes a field it actually changed; patch exactly that set, so
                // an untouched description is never rewritten with the form's copy of it.
                patchWorkItem(qc, wiRef, {
                  ...(title !== undefined ? { title } : {}),
                  ...(description !== undefined ? { description } : {}),
                });
                act("Saved", async () => {
                  await apiPost("/api/wi/update", { conn: wiRef.conn, repo: wiRef.repo, id: wiRef.id, title, description });
                  setEditing(false);
                });
              }}
            />
          ) : (
            wi.description && (
              <div className="text-[13px] whitespace-pre-wrap" style={{ color: "var(--fg)" }}>
                {wi.description}
              </div>
            )
          )}

          {facts.length > 0 && (
            <dl className="grid m-0 gap-x-4 gap-y-1.5 text-[12.5px]" style={{ gridTemplateColumns: "96px 1fr" }}>
              {facts.map(([label, value]) => (
                <div key={label} className="contents">
                  <dt style={{ color: "var(--fg3)" }}>{label}</dt>
                  <dd className="m-0 min-w-0 truncate" style={{ color: "var(--fg)" }}>
                    {value}
                  </dd>
                </div>
              ))}
            </dl>
          )}

          {data.timeline.length > 0 && <Timeline events={data.timeline} />}

          <Comments
            threads={data.threads}
            busy={busy}
            onComment={(body) => {
              // Who you are isn't known here, so the placeholder reads "You" until the refetch
              // brings the provider's copy with your name on it.
              addWiThread(qc, wiRef, localThread(meAsUser(), body));
              act("Comment posted", () => apiPost("/api/wi/comment", { conn: wiRef.conn, repo: wiRef.repo, id: wiRef.id, body }));
            }}
          />
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
      {/* The state change is the pane's one primary action while its picker is open. */}
      <Button primary={open} onClick={() => setOpen((o) => !o)} disabled={busy} aria-expanded={open} aria-haspopup="menu">
        Move
        <Icon name="chevron-down" size={12} />
      </Button>
      {open && (
        <MenuPanel>
          {loading && (
            <div className="px-3 h-7 flex items-center text-[12.5px]" style={{ color: "var(--fg3)" }}>
              Loading…
            </div>
          )}
          {!loading && options.length === 0 && (
            <div className="px-3 h-7 flex items-center text-[12.5px]" style={{ color: "var(--fg3)" }}>
              No transitions available
            </div>
          )}
          {options.map((s) => (
            <button
              key={s}
              type="button"
              disabled={busy}
              onClick={() => {
                onMove(s);
                setOpen(false);
              }}
              className={MENU_ITEM}
              style={{ color: "var(--fg)" }}
            >
              <Icon name="arrow-right" size={12} style={{ color: "var(--fg3)" }} />
              {s}
            </button>
          ))}
        </MenuPanel>
      )}
    </div>
  );
}

// ---- assignee picker ----

// `onPick` hands back the whole `User`, not just its id: the caller needs the display name and
// avatar to show the reassignment before the refetch, and this list is the only place that has
// them without another fetch.
function AssigneePicker({ wiRef, assignee, busy, onPick }: { wiRef: WiRef; assignee?: User | null; busy: boolean; onPick: (user: User | null) => void }) {
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

  const pick = (user: User | null) => {
    onPick(user);
    setOpen(false);
  };

  return (
    <div className="relative">
      <Button onClick={() => setOpen((o) => !o)} disabled={busy} aria-expanded={open} aria-haspopup="menu" title="Assign">
        {assignee ? (
          <>
            <Avatar name={assignee.display_name} size={18} /> {assignee.display_name}
          </>
        ) : (
          <>
            <Icon name="user" size={13} /> Unassigned
          </>
        )}
        <Icon name="chevron-down" size={12} />
      </Button>
      {open && (
        <MenuPanel wide>
          {loading && (
            <div className="px-3 h-7 flex items-center text-[12.5px]" style={{ color: "var(--fg3)" }}>
              Loading…
            </div>
          )}
          {/* Only offer Unassign when there's actually an assignee — avoids a no-op clear
              (and the Azure "remove an unset field" 400), mirroring how Move hides the current state. */}
          {assignee && (
            <button type="button" disabled={busy} onClick={() => pick(null)} className={MENU_ITEM} style={{ color: "var(--fg2)" }}>
              Unassign
            </button>
          )}
          {(users ?? []).map((u) => (
            <button key={u.id} type="button" disabled={busy} onClick={() => pick(u)} className={MENU_ITEM} style={{ color: "var(--fg)" }}>
              <Avatar name={u.display_name} size={18} /> {u.display_name}
            </button>
          ))}
        </MenuPanel>
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
    <div className="flex flex-col gap-2">
      <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" aria-label="Title" className="ft-input w-full" />
      <textarea
        value={description}
        onChange={(e) => setDescription(e.target.value)}
        placeholder="Description"
        aria-label="Description"
        rows={6}
        className="ft-input w-full"
      />
      <div className="flex gap-2">
        <Button primary sm disabled={!canSave} onClick={() => onSave(changedTitle ? title : undefined, changedDesc ? description : undefined)}>
          Save
        </Button>
        <Button quiet sm onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

// ---- comments ----

function Comments({ threads, busy, onComment }: { threads: CommentThread[]; busy: boolean; onComment: (body: string) => void }) {
  const [draft, setDraft] = useState("");
  const count = threads.reduce((n, t) => n + t.comments.length, 0);
  return (
    <div className="flex flex-col gap-2">
      <GroupHeading count={count || undefined}>Comments</GroupHeading>
      {threads.length === 0 && (
        <div className="text-[12.5px]" style={{ color: "var(--fg3)" }}>
          No comments yet.
        </div>
      )}
      {threads.map((t) => (
        <div
          key={t.id}
          className="flex flex-col gap-2 px-3 py-2"
          style={{ background: "var(--hover)", border: "1px solid var(--border)", borderRadius: 6 }}
        >
          {t.comments.map((c) => (
            <div key={c.id} className="flex flex-col gap-1">
              <div className="flex items-center gap-2 text-[12.5px]">
                <Avatar name={c.author.display_name} size={18} />
                <span className="font-medium" style={{ color: "var(--fg)" }}>
                  {c.author.display_name}
                </span>
                {c.created_at && (
                  <span className="num ml-auto text-[12px]" style={{ color: "var(--fg3)" }}>
                    {relativeTime(c.created_at)}
                  </span>
                )}
              </div>
              <div className="text-[13px] whitespace-pre-wrap" style={{ color: "var(--fg)" }}>
                {c.body}
              </div>
            </div>
          ))}
        </div>
      ))}
      <textarea
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        placeholder="Add a comment…"
        aria-label="Add a comment"
        rows={3}
        className="ft-input w-full mt-1"
      />
      <div>
        <Button
          primary
          sm
          disabled={busy || !draft.trim()}
          onClick={() => {
            onComment(draft.trim());
            setDraft("");
          }}
        >
          Comment
        </Button>
      </div>
    </div>
  );
}
