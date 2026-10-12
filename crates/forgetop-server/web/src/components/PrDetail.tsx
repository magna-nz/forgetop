import { createContext, Fragment, useCallback, useContext, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { apiPost, prDetailKey, useConnections, usePrCommitChanges, usePrDetail, usePrFileText, usePrReviewers } from "../api";
import { checkMeta, checkSummaryOf, providerMeta, prStateLine, prStatusMeta, relativeTime } from "../format";
import { providerSupports, unsupportedMessage } from "../capabilities";
import { parsePatch, type DiffLine } from "../diff";
import { buildDiffTree, collectFiles, filterTree, findMatches, flattenTree, searchFiles, type DiffTreeFile, type DiffTreeFolder, type FileMatch } from "../diffTree";
import { addPrReply, addPrThreads, isLocalId, localComment, localThread, meAsUser, patchPullRequest, setPrThreadResolved, withVote } from "../optimistic";
import type { CheckRun, CheckStatus, CommentThread, Commit, FileChange, FileChangeKind, LineComment, PrRef, PrWriteSupport, ProviderType, PullRequest, ReviewVote, Reviewer, TimelineEvent, User } from "../types";
import { Avatar, Button, Chip, claimPane, GroupHeading, IconButton, PaneHeader, setPaneGuard, SlideOver, StatusBadge, StatusGlyph, Tabs, Timeline, usePaneRegistration } from "./ui";
import { Icon } from "./icons";

// ---- opener context ----

const PrOpenerCtx = createContext<(ref: PrRef) => void>(() => {});
export const usePrOpener = () => useContext(PrOpenerCtx);
/** The pull request open in the pane, so a list can mark its row as selected. */
const PrCurrentCtx = createContext<PrRef | null>(null);
export const useCurrentPr = () => useContext(PrCurrentCtx);

export function PrDetailProvider({ children }: { children: ReactNode }) {
  const [ref, setRef] = useState<PrRef | null>(null);
  const close = useCallback(() => setRef(null), []);
  usePaneRegistration("pr", close);
  // Opening a PR closes any other pane first, and asks the open PR pane's guard (unsubmitted
  // comments) before switching away from it — the same check Esc and ✕ make.
  const open = useCallback((r: PrRef) => {
    if (claimPane("pr")) setRef(r);
  }, []);
  return (
    <PrOpenerCtx.Provider value={open}>
      <PrCurrentCtx.Provider value={ref}>
        {children}
        {ref && <PrDetailPanel prRef={ref} onClose={close} />}
      </PrCurrentCtx.Provider>
    </PrOpenerCtx.Provider>
  );
}

// ---- panel ----

type Tab = "conversation" | "commits" | "checks" | "files";

/** What a detail without a `writes` field (an older server) supports: nothing optional. */
const NO_WRITES: PrWriteSupport = { resolve_threads: false, draft: false, close: false, reopen: false, request_reviewer: false };

/** Signature of a thread resolve/reopen handler; absent where the provider can't do it. */
type OnResolve = (threadId: string, resolved: boolean) => void;

/** A muted middle-dot separator for the header's meta lines. */
const Sep = () => <span style={{ color: "var(--fg3)" }}>·</span>;

function PrDetailPanel({ prRef, onClose }: { prRef: PrRef; onClose: () => void }) {
  const { data, isLoading, error } = usePrDetail(prRef);
  const connections = useConnections();
  const provider: ProviderType | undefined = connections.data?.find((c) => c.id === prRef.conn)?.provider;
  const qc = useQueryClient();
  const [tab, setTab] = useState<Tab>("conversation");
  const [pending, setPending] = useState<LineComment[]>([]);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  // The footer's "More" menu: the rare state changes (convert to draft, close).
  const [moreOpen, setMoreOpen] = useState(false);
  // When set, the Files tab shows a single commit's diff instead of the whole-PR changes.
  const [commitScope, setCommitScope] = useState<{ sha: string; label: string } | null>(null);
  const commitChanges = usePrCommitChanges(prRef, commitScope?.sha ?? null);
  // Close is the one state change you can't take back on every forge, so it takes two clicks.
  const [confirmClose, setConfirmClose] = useState(false);
  useEffect(() => {
    if (!confirmClose) return;
    const t = setTimeout(() => setConfirmClose(false), 4000);
    return () => clearTimeout(t);
  }, [confirmClose]);

  // Esc and the pane's ✕ both come through here (SlideOver owns the key listener), so
  // unsubmitted comments are guarded either way.
  const canLeave = () => pending.length === 0 || window.confirm(`Discard ${pending.length} unsubmitted comment(s)?`);
  const requestClose = () => {
    if (!canLeave()) return;
    onClose();
  };
  // Switching to another PR (a list row, the palette, the bell) goes through the same guard.
  useEffect(() => {
    setPaneGuard("pr", pending.length > 0 ? canLeave : null);
    return () => setPaneGuard("pr", null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending.length]);

  // A pending delayed close (after a verdict or merge) must not outlive the PR it was for.
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
  }, []);

  useEffect(() => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    setTab("conversation");
    setPending([]);
    setNote(null);
    setCommitScope(null);
    setConfirmClose(false);
    setMoreOpen(false);
  }, [prRef.conn, prRef.id]);

  const refresh = () => {
    qc.invalidateQueries({ queryKey: prDetailKey(prRef) });
    qc.invalidateQueries({ queryKey: ["prs"] });
    qc.invalidateQueries({ queryKey: ["launchpad"] });
    // Acting on a PR (merge, approve, …) can clear its review-request / mention notification.
    qc.invalidateQueries({ queryKey: ["notifications"] });
  };

  // Who you are on this connection, for showing your vote and comments before the provider does.
  const me = data?.me ?? null;
  const you = meAsUser(me);

  /**
   * Runs a write. With `optimistic`, what the write decides on its own shows at once — the
   * caches are patched and the note says it's done — then the request goes out. Without it (a
   * merge or revert: the provider decides whether it happens) the note says it's under way.
   * Refreshed on both paths: after a success it confirms the optimistic patch, after a failure it
   * is the rollback — the server's truth replaces it.
   */
  const act = async (
    label: string,
    fn: () => Promise<void>,
    opts: { optimistic?: () => void; progress?: string; onError?: () => void } = {},
  ) => {
    setBusy(true);
    if (opts.optimistic) {
      opts.optimistic();
      setNote(`${label} ✓`);
    } else {
      setNote(opts.progress ?? null);
    }
    try {
      await fn();
      setNote(`${label} ✓`);
    } catch (e) {
      opts.onError?.();
      setNote(e instanceof Error ? e.message : String(e));
    } finally {
      refresh();
      setBusy(false);
    }
  };

  // A verdict/merge action from the action bar closes the pane once it lands (brief delay so the
  // "✓" note flashes first), matching Merge — you've acted on this PR, so return to the list.
  // Only once it lands: a refusal has to stay on screen to be read.
  const closeSoon = () => {
    closeTimer.current = setTimeout(onClose, 500);
  };
  const markVote = (v: ReviewVote) => {
    if (me) patchPullRequest(qc, prRef, (p) => withVote(p, me, v));
  };
  const vote = (v: "Approved" | "Rejected") =>
    act(
      v === "Approved" ? "Approved" : "Requested changes",
      async () => {
        await apiPost("/api/pr/vote", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, vote: v });
        closeSoon();
      },
      { optimistic: () => markVote(v) },
    );
  const merge = () =>
    act(
      "Merged",
      async () => {
        try {
          await apiPost("/api/pr/merge", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, strategy: "Merge" });
        } catch {
          // Providers without a mergeable flag (e.g. Bitbucket) let you try — the API decides.
          throw new Error("Couldn't merge — the PR may not be mergeable.");
        }
        closeSoon();
      },
      { progress: "Merging…" },
    );
  const revert = () =>
    act(
      "Revert requested",
      async () => {
        await apiPost("/api/pr/revert", { conn: prRef.conn, repo: prRef.repo, id: prRef.id });
        closeSoon();
      },
      { progress: "Requesting a revert…" },
    );
  const comment = (body: string) =>
    act("Comment posted", () => apiPost("/api/pr/comment", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, body }), {
      optimistic: () => addPrThreads(qc, prRef, [localThread(you, body)]),
    });
  const reply = (threadId: string, body: string) =>
    act("Reply posted", () => apiPost("/api/pr/reply", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, thread_id: threadId, body }), {
      optimistic: () => addPrReply(qc, prRef, threadId, localComment(you, body)),
    });
  const submitReview = (event: "Approved" | "Rejected" | "NoVote") => {
    const sent = pending;
    return act("Review submitted", () => apiPost("/api/pr/review", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, event, comments: sent }), {
      optimistic: () => {
        setPending([]);
        addPrThreads(qc, prRef, sent.map((c) => localThread(you, c.body, { path: c.path, line: c.line })));
        if (event !== "NoVote") markVote(event);
      },
      // A refused review's comments go back in the buffer, ahead of any added since.
      onError: () => setPending((p) => [...sent, ...p]),
    });
  };

  // ---- optional writes (gated on what the connection says it supports) ----

  const writes: PrWriteSupport = data?.writes ?? NO_WRITES;
  // Unsupported controls stay visible but disabled, titled with the standard message.
  const unsupported = provider ? unsupportedMessage(provider) : "This provider currently does not support this feature";
  const gate = (supported: boolean) => (supported ? undefined : unsupported);

  const resolveThread: OnResolve = (threadId, resolved) =>
    act(
      resolved ? "Thread resolved" : "Thread reopened",
      () => apiPost("/api/pr/resolve-thread", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, thread_id: threadId, resolved }),
      {
        optimistic: () => setPrThreadResolved(qc, prRef, threadId, resolved),
        onError: () => setPrThreadResolved(qc, prRef, threadId, !resolved),
      },
    );

  /** A draft / close / reopen: the new state is yours to decide, so it shows at once in every
   *  cache the PR lives in, and is put back if the provider refuses. */
  const changeState = (label: string, path: string, body: Record<string, boolean>, edit: (p: PullRequest) => PullRequest) => {
    const current = data?.pull_request;
    const before = current ? { status: current.status, is_draft: current.is_draft } : null;
    return act(label, () => apiPost(path, { conn: prRef.conn, repo: prRef.repo, id: prRef.id, ...body }), {
      optimistic: () => patchPullRequest(qc, prRef, edit),
      onError: () => before && patchPullRequest(qc, prRef, (p) => ({ ...p, ...before })),
    });
  };
  const markReady = () =>
    changeState("Marked ready for review", "/api/pr/draft", { draft: false }, (p) => ({ ...p, is_draft: false, status: "Open" }));
  const convertToDraft = () =>
    changeState("Converted to draft", "/api/pr/draft", { draft: true }, (p) => ({ ...p, is_draft: true, status: "Draft" }));
  const closePr = () => {
    setConfirmClose(false);
    return changeState("Closed", "/api/pr/closed", { closed: true }, (p) => ({ ...p, status: "Closed" }));
  };
  const reopenPr = () =>
    changeState("Reopened", "/api/pr/closed", { closed: false }, (p) => ({ ...p, status: p.is_draft ? "Draft" : "Open" }));

  const requestReviewer = (user: User) =>
    act(
      `Review requested from ${user.display_name}`,
      () => apiPost("/api/pr/request-reviewer", { conn: prRef.conn, repo: prRef.repo, id: prRef.id, user_id: user.id }),
      {
        optimistic: () =>
          patchPullRequest(qc, prRef, (p) =>
            p.reviewers.some((r) => r.user.id === user.id) ? p : { ...p, reviewers: [...p.reviewers, { user, vote: "NoVote", is_required: false }] },
          ),
        // The menu only offers people not already reviewing, so the entry to take back is ours.
        onError: () =>
          patchPullRequest(qc, prRef, (p) => ({ ...p, reviewers: p.reviewers.filter((r) => !(r.user.id === user.id && r.vote === "NoVote")) })),
      },
    );

  const pr = data?.pull_request;
  const isDraft = !!pr && (pr.status === "Draft" || pr.is_draft);
  const onResolve = writes.resolve_threads ? resolveThread : undefined;
  const status = pr ? prStatusMeta(pr) : null;
  const providerLabel = provider ? providerMeta(provider).label : "provider";
  const failedChecks = data ? data.checks.filter((c) => c.status === "Failed").length : 0;

  const header = (
    <>
      <PaneHeader
        glyph={status?.glyph}
        color={status?.color}
        title={pr?.title ?? (isLoading ? "Loading…" : "Pull request")}
        ident={pr?.number != null ? `#${pr.number}` : undefined}
        actions={
          pr?.url ? (
            // A link out, so a real anchor dressed as the icon button.
            <a
              href={pr.url}
              target="_blank"
              rel="noreferrer"
              aria-label={`Open on ${providerLabel}`}
              title={`Open on ${providerLabel}`}
              className="ft-btn ft-btn-icon"
            >
              <Icon name="external" size={15} />
            </a>
          ) : undefined
        }
      >
        {pr && data && <MetaLine pr={pr} changes={data.changes} />}
      </PaneHeader>
      {pr && data && (
        <StateLine
          pr={pr}
          checks={data.checks}
          request={
            <RequestReviewer
              prRef={prRef}
              author={pr.author}
              reviewers={pr.reviewers}
              supported={writes.request_reviewer}
              unsupportedTitle={unsupported}
              busy={busy}
              onPick={requestReviewer}
            />
          }
        />
      )}
      {pr && data && (
        // Pulled down onto the header's hairline, so the active tab's underline sits on it.
        <Tabs
          label="Pull request sections"
          className="mt-3.5 -mb-3"
          value={tab}
          onChange={setTab}
          tabs={[
            { id: "conversation", label: "Conversation", count: data.threads.filter((t) => !t.file_path).length },
            { id: "commits", label: "Commits", count: data.commits.length },
            {
              id: "checks",
              label: "Checks",
              count: failedChecks > 0 ? `${failedChecks} failed` : data.checks.length,
              countColor: failedChecks > 0 ? "var(--red)" : undefined,
            },
            { id: "files", label: "Files", count: data.changes.length },
          ]}
        />
      )}
    </>
  );

  const footer =
    pr && data ? (
      <div className="flex items-center gap-2 flex-wrap w-full min-w-0">
        <div className="flex items-center gap-3 min-w-0 flex-wrap">
          <ChecksSummary checks={data.checks} provider={provider} onOpen={() => setTab("checks")} />
          {pr.status !== "Merged" && pr.status !== "Closed" && pending.length > 0 && (
            <>
              <span className="inline-flex items-center gap-1.5 text-[12.5px]" style={{ color: "var(--fg2)" }}>
                <StatusGlyph glyph="comment" color="var(--yellow)" />
                <span className="num">{pending.length} pending</span>
              </span>
              <Button disabled={busy} onClick={() => submitReview("NoVote")}>
                Submit comments
              </Button>
            </>
          )}
          {note && (
            <span className="text-[12.5px]" style={{ color: noteColor(note) }}>
              {note}
            </span>
          )}
        </div>
        <div className="ml-auto flex items-center gap-1.5 shrink-0">
          {pr.status === "Merged" ? (
            <Button disabled={busy} onClick={revert}>
              Revert
            </Button>
          ) : pr.status === "Closed" ? (
            <Button primary disabled={busy || !writes.reopen} title={gate(writes.reopen)} onClick={reopenPr}>
              Reopen
            </Button>
          ) : pending.length > 0 ? (
            <>
              <Button disabled={busy} onClick={() => submitReview("Rejected")}>
                Request changes
              </Button>
              <Button primary disabled={busy} onClick={() => submitReview("Approved")}>
                Approve
              </Button>
            </>
          ) : isDraft ? (
            <Button primary disabled={busy || !writes.draft} title={gate(writes.draft)} onClick={markReady}>
              Ready for review
            </Button>
          ) : (
            <>
              <MoreMenu open={moreOpen} onToggle={setMoreOpen}>
                <Button
                  quiet
                  style={MENU_ITEM}
                  disabled={busy || !writes.draft}
                  title={gate(writes.draft)}
                  onClick={() => {
                    setMoreOpen(false);
                    convertToDraft();
                  }}
                >
                  Convert to draft
                </Button>
                {confirmClose ? (
                  <Button
                    danger
                    style={MENU_ITEM}
                    disabled={busy || !writes.close}
                    title={gate(writes.close) ?? "Click again to close without merging"}
                    onClick={() => {
                      setMoreOpen(false);
                      closePr();
                    }}
                  >
                    Confirm close
                  </Button>
                ) : (
                  <Button
                    quiet
                    style={MENU_ITEM}
                    disabled={busy || !writes.close}
                    title={gate(writes.close) ?? "Close without merging"}
                    onClick={() => setConfirmClose(true)}
                  >
                    Close pull request
                  </Button>
                )}
              </MoreMenu>
              <Button disabled={busy} onClick={() => vote("Rejected")}>
                Request changes
              </Button>
              <Button disabled={busy} onClick={() => vote("Approved")}>
                Approve
              </Button>
              <Button
                primary
                disabled={busy || pr.mergeable === "Conflicting" || pr.mergeable === "Blocked"}
                title={
                  pr.mergeable === "Conflicting"
                    ? "This PR has merge conflicts"
                    : pr.mergeable === "Blocked"
                      ? "This PR is blocked (branch policy or draft)"
                      : undefined
                }
                onClick={merge}
              >
                Merge
              </Button>
            </>
          )}
        </div>
      </div>
    ) : undefined;

  return (
    <SlideOver width="xl" label="Pull request" onClose={requestClose} header={header} footer={footer}>
      {error && (
        <div className="px-5 py-6 text-[13px]" style={{ color: "var(--red)" }}>
          Couldn't load this pull request.
        </div>
      )}
      {pr && data && (
        <>
          {tab === "files" && (
            <FilesTab
              key={commitScope?.sha ?? "all"}
              prRef={prRef}
              sha={commitScope?.sha ?? null}
              changes={commitScope ? (commitChanges.data ?? []) : data.changes}
              threads={data.threads}
              pending={pending}
              busy={busy}
              onReply={reply}
              onResolve={onResolve}
              onAddPending={(c) => setPending((p) => [...p, c])}
              onRemovePending={(i) => setPending((p) => p.filter((_, k) => k !== i))}
              scope={
                commitScope ? { label: commitScope.label, loading: commitChanges.isLoading, onClear: () => setCommitScope(null) } : undefined
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
              onResolve={onResolve}
              onComment={comment}
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
          {tab === "checks" && <ChecksTab checks={data.checks} provider={provider} />}
        </>
      )}
    </SlideOver>
  );
}

/** A done note is green, one still under way muted, anything else is an error. */
const noteColor = (note: string) => (note.endsWith("✓") ? "var(--green)" : note.endsWith("…") ? "var(--fg3)" : "var(--red)");

/** A footer-menu entry: a quiet button stretched to the menu's width, label on the left. */
const MENU_ITEM = { width: "100%", justifyContent: "flex-start" } as const;

/** "More ▾": the rare actions, folded out of the footer. Opens upward (the footer is at the
 *  bottom of the pane); a click anywhere else closes it. */
function MoreMenu({ open, onToggle, children }: { open: boolean; onToggle: (open: boolean) => void; children: ReactNode }) {
  return (
    <span className="relative inline-flex">
      <Button quiet aria-expanded={open} onClick={() => onToggle(!open)}>
        More
        <Icon name="chevron-down" size={12} />
      </Button>
      {open && (
        <>
          <div className="fixed inset-0" style={{ zIndex: 20 }} onClick={() => onToggle(false)} />
          <div
            className="absolute right-0 bottom-full mb-1.5 flex flex-col gap-0.5 rounded-md p-1 shadow-lg"
            style={{ zIndex: 21, minWidth: 190, background: "var(--raised)", border: "1px solid var(--border-strong)" }}
          >
            {children}
          </div>
        </>
      )}
    </span>
  );
}

/** The header's second line: who opened it and when, the branches, and the size of the change. */
function MetaLine({ pr, changes }: { pr: PullRequest; changes: FileChange[] }) {
  // The PR's own counts, or the changes' when the provider didn't fill them in.
  const fromChanges = pr.changed_files === 0 && changes.length > 0;
  const files = fromChanges ? changes.length : pr.changed_files;
  const adds = fromChanges ? changes.reduce((n, c) => n + c.additions, 0) : pr.additions;
  const dels = fromChanges ? changes.reduce((n, c) => n + c.deletions, 0) : pr.deletions;
  const age = relativeTime(pr.created_at);
  return (
    <>
      <Avatar name={pr.author.display_name} size={20} />
      <span>
        {pr.author.display_name} opened{age ? ` ${age}` : ""}
      </span>
      {pr.source_ref && (
        <>
          <Sep />
          <Chip>{shortRef(pr.source_ref)}</Chip>
          <Icon name="arrow-right" size={12} style={{ color: "var(--fg3)" }} />
          <Chip>{shortRef(pr.target_ref) || "?"}</Chip>
        </>
      )}
      <Sep />
      <span className="num">
        {files} file{files === 1 ? "" : "s"}
      </span>
      <span className="num" style={{ color: "var(--green)" }}>
        +{adds}
      </span>
      <span className="num" style={{ color: "var(--red)" }}>
        −{dels}
      </span>
    </>
  );
}

/** A reviewer's vote, as the word the header line uses. */
function voteWord(vote: ReviewVote): string {
  switch (vote) {
    case "Approved":
    case "ApprovedWithSuggestions":
      return "approved";
    case "Rejected":
      return "requested changes";
    case "WaitingForAuthor":
      return "waiting";
    case "NoVote":
      return "pending";
  }
}

/** Where this pull request stands, in one line — the same verdict the TUI shows above its tabs,
 *  and the same one the Command Center row and the PR list abbreviate — followed by who is
 *  reviewing and how they voted, and the control to ask someone else. */
function StateLine({ pr, checks, request }: { pr: PullRequest; checks: CheckRun[]; request: ReactNode }) {
  // The detail endpoint returns the runs but not always the summary; completing it here lets the
  // line say "1 of 8 failed" instead of just "failing". The judgement itself is unchanged.
  const withSummary = pr.check_summary ?? (checks.length > 0 ? checkSummaryOf(checks) : null);
  const { glyph, text, color } = prStateLine({ ...pr, check_summary: withSummary });
  return (
    <div className="flex items-center gap-2 flex-wrap mt-2.5 text-[12.5px]">
      <StatusGlyph glyph={glyph} color={color} />
      <span style={{ color }}>{text}</span>
      <Sep />
      {pr.reviewers.map((r: Reviewer, i) => {
        const word = voteWord(r.vote);
        return (
          <span key={i} className="inline-flex items-center gap-1.5" style={{ color: "var(--fg2)" }} title={`${r.user.display_name} — ${word}`}>
            <Avatar name={r.user.display_name} size={16} />
            <span className="sr-only">{r.user.display_name}</span>
            {word}
          </span>
        );
      })}
      {request}
    </div>
  );
}

/** "+ reviewer": a searchable menu of the people who can review this PR, fetched the first time
 *  it opens and kept for the session. The author is left out (no forge lets you review your own
 *  PR); anyone already reviewing is listed but can't be picked again. */
function RequestReviewer({
  prRef,
  author,
  reviewers,
  supported,
  unsupportedTitle,
  busy,
  onPick,
}: {
  prRef: PrRef;
  author: User;
  reviewers: Reviewer[];
  supported: boolean;
  unsupportedTitle: string;
  busy: boolean;
  onPick: (user: User) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  // Enabled from the first open on, so reopening the menu reuses the list (staleTime: Infinity).
  const [wanted, setWanted] = useState(false);
  const users = usePrReviewers(prRef, supported && wanted);

  const isAuthor = (u: User) => u.id === author.id || (!!u.handle && !!author.handle && u.handle.toLowerCase() === author.handle.toLowerCase());
  const already = new Set(reviewers.map((r) => r.user.id));
  const q = query.trim().toLowerCase();
  const list = (users.data ?? [])
    .filter((u) => !isAuthor(u))
    .filter((u) => !q || u.display_name.toLowerCase().includes(q) || (u.handle ?? "").toLowerCase().includes(q));
  const disabled = !supported || busy;

  const close = () => {
    setOpen(false);
    setQuery("");
  };

  return (
    <span className="relative inline-flex">
      <Button
        quiet
        sm
        icon="plus"
        aria-label="Request reviewer"
        aria-expanded={open}
        onClick={() => {
          setWanted(true);
          setOpen((o) => !o);
        }}
        disabled={disabled}
        title={supported ? "Ask someone to review this pull request" : unsupportedTitle}
      >
        reviewer
      </Button>
      {open && (
        <>
          <div className="fixed inset-0" style={{ zIndex: 20 }} onClick={close} />
          <div
            className="absolute left-0 top-full mt-1 w-64 rounded-md py-1 shadow-lg"
            style={{ zIndex: 21, background: "var(--raised)", border: "1px solid var(--border-strong)" }}
          >
            <div className="px-1.5 pb-1">
              <input
                autoFocus
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search people…"
                aria-label="Search people"
                className="ft-input w-full"
              />
            </div>
            <div className="max-h-56 overflow-auto text-[12.5px]">
              {users.isLoading && (
                <div className="px-3 py-1.5" style={{ color: "var(--fg3)" }}>
                  Loading…
                </div>
              )}
              {users.isError && (
                <div className="px-3 py-1.5" style={{ color: "var(--red)" }}>
                  Couldn't load reviewers
                </div>
              )}
              {!users.isLoading && !users.isError && list.length === 0 && (
                <div className="px-3 py-1.5" style={{ color: "var(--fg3)" }}>
                  No one to request
                </div>
              )}
              {list.map((u) => {
                const requested = already.has(u.id);
                return (
                  <button
                    key={u.id}
                    type="button"
                    disabled={requested || busy}
                    onClick={() => {
                      onPick(u);
                      close();
                    }}
                    className={`flex items-center gap-2 w-full text-left px-3 ${requested ? "" : "hover:bg-[var(--hover)]"}`}
                    style={{ height: 30, color: requested ? "var(--fg3)" : "var(--fg)", cursor: requested ? "default" : "pointer" }}
                  >
                    <Avatar name={u.display_name} size={18} />
                    <span className="flex-1 truncate">{u.display_name}</span>
                    {requested && (
                      <span className="text-[12px] shrink-0" style={{ color: "var(--fg3)" }}>
                        reviewing
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
          </div>
        </>
      )}
    </span>
  );
}

/** Footer, left: what failed, and how many passed. Clicking it opens the Checks tab. Nothing when
 *  no check failed (the state line and the Checks tab carry the rest). */
function ChecksSummary({ checks, provider, onOpen }: { checks: CheckRun[]; provider: ProviderType | undefined; onOpen: () => void }) {
  const supported = provider ? providerSupports(provider, "checks") : true;
  const failed = checks.filter((c) => c.status === "Failed");
  if (!supported || failed.length === 0) return null;
  const passed = checks.filter((c) => c.status === "Passed").length;
  const what = failed.length === 1 ? `${failed[0].name} failed` : `${failed.length} checks failed`;
  return (
    <Button quiet onClick={onOpen} title="Show checks" style={{ paddingLeft: 0, color: "var(--red)" }}>
      <StatusGlyph glyph="x-circle" color="var(--red)" />
      <span>{what}</span>
      <span className="num" style={{ color: "var(--fg3)" }}>
        · {passed} of {checks.length} passed
      </span>
    </Button>
  );
}

/** A check run's status, as the word its row shows. */
function checkWord(status: CheckStatus): string {
  switch (status) {
    case "Passed":
      return "passed";
    case "Failed":
      return "failed";
    case "Pending":
      return "running";
    case "None":
      return "neutral";
  }
}

/** The Checks tab: one row per check run, linking out to the run where the provider gives a URL.
 *  Greyed with the standard message when the provider can't report checks. */
function ChecksTab({ checks, provider }: { checks: CheckRun[]; provider: ProviderType | undefined }) {
  const supported = provider ? providerSupports(provider, "checks") : true;
  if (!supported && provider) return <Empty text={unsupportedMessage(provider)} />;
  if (checks.length === 0) return <Empty text="No checks reported for this pull request." />;
  return (
    <div className="flex flex-col px-2 py-2">
      {checks.map((c, i) => {
        const m = checkMeta(c.status);
        const inner = (
          <>
            <StatusGlyph glyph={m.glyph} color={m.color} spin={c.status === "Pending"} />
            <span className="flex-1 truncate">{c.name}</span>
            <span className="text-[12px] shrink-0" style={{ color: m.color }}>
              {checkWord(c.status)}
            </span>
            {c.url && <Icon name="external" size={13} style={{ color: "var(--fg3)" }} />}
          </>
        );
        return c.url ? (
          <a key={i} href={c.url} target="_blank" rel="noreferrer" className="ft-row" style={{ minHeight: 32 }}>
            {inner}
          </a>
        ) : (
          <div key={i} className="ft-row" style={{ minHeight: 32, cursor: "default" }}>
            {inner}
          </div>
        );
      })}
    </div>
  );
}

// ---- files / diff ----

function FilesTab({
  prRef,
  sha,
  changes,
  threads,
  pending,
  busy,
  onReply,
  onResolve,
  onAddPending,
  onRemovePending,
  scope,
}: {
  prRef: PrRef;
  sha: string | null;
  changes: FileChange[];
  threads: CommentThread[];
  pending: LineComment[];
  busy: boolean;
  onReply: (threadId: string, body: string) => void;
  onResolve?: OnResolve;
  onAddPending: (c: LineComment) => void;
  onRemovePending: (index: number) => void;
  scope?: { label: string; loading: boolean; onClear: () => void };
}) {
  // Which file the tree has selected, by path (an index would drift as folders fold/filter).
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  // Folders the user has folded shut — every folder starts open, so this tracks the exception.
  const [closed, setClosed] = useState<Set<string>>(() => new Set());
  // Folds made while a search is in effect, kept apart from `closed` so a fold toggled during
  // a search can't leak into "what was folded before the search" — clearing the search just
  // stops consulting this set, giving the real folds back untouched. Mirrors the TUI's
  // `folded`/`search_folded` split (crates/forgetop-tui/src/app.rs).
  const [searchClosed, setSearchClosed] = useState<Set<string>>(() => new Set());
  const [query, setQuery] = useState("");
  // Files ticked "Viewed", by path. A reading aid for this pane only: nothing is sent anywhere,
  // and it starts empty for each diff (the tab remounts per commit scope).
  const [viewed, setViewed] = useState<Set<string>>(() => new Set());

  const tree = useMemo(() => buildDiffTree(changes), [changes]);
  // `null` means "not searching" (show everything, respect `closed`); otherwise only files in
  // the map are kept, and folders default open (ancestors of a match are never hidden) unless
  // the user folded them during *this* search.
  const matches = useMemo(() => (query.trim() ? searchFiles(changes, query) : null), [changes, query]);
  const displayTree = useMemo(() => (matches ? filterTree(tree, matches) : tree), [tree, matches]);
  const activeClosed = matches ? searchClosed : closed;
  const rows = useMemo(() => {
    const isOpen = (p: string) => !activeClosed.has(p);
    return flattenTree(displayTree, isOpen);
  }, [displayTree, activeClosed]);

  const totalFiles = changes.length;
  const visibleFiles = matches ? matches.size : totalFiles;
  const hiddenCount = Math.max(0, totalFiles - visibleFiles);
  const viewedCount = changes.filter((c) => viewed.has(c.path)).length;

  // Every edit to the query starts its search-time folds fresh (mirrors the TUI's
  // `search_changed`), so a fold made under one query never carries into the next, and the
  // real `closed` set is never touched while searching.
  useEffect(() => {
    setSearchClosed(new Set());
  }, [query]);

  // A search that drops the current selection jumps to the first match, in tree order.
  useEffect(() => {
    if (!matches) return;
    if (selectedPath && matches.has(selectedPath)) return;
    const first = collectFiles(tree).find((f) => matches.has(f.path));
    setSelectedPath(first ? first.path : null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [matches, tree]);

  const toggleFolder = (path: string) => {
    const setter = matches ? setSearchClosed : setClosed;
    setter((s) => {
      const next = new Set(s);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const setFileViewed = (path: string, on: boolean) =>
    setViewed((s) => {
      const next = new Set(s);
      if (on) next.add(path);
      else next.delete(path);
      return next;
    });

  // Scoped to one commit: show a banner with a way back to the whole-PR diff.
  const banner = scope && (
    <div
      className="flex items-center gap-3 rounded-md px-3 text-[12.5px]"
      style={{ height: 34, background: "var(--hover)", border: "1px solid var(--border)" }}
    >
      <span style={{ color: "var(--fg3)" }}>Showing commit</span>
      <span className="mono truncate" style={{ color: "var(--fg)" }}>
        {scope.label}
      </span>
      <Button quiet sm className="ml-auto shrink-0" onClick={scope.onClear}>
        Show all files
      </Button>
    </div>
  );

  if (scope?.loading || changes.length === 0) {
    return (
      <div className="px-5 py-4 flex flex-col gap-4">
        {banner}
        {scope?.loading ? <Empty text="Loading commit diff…" /> : <Empty text={scope ? "No per-commit diff available for this provider." : "No file changes to show."} />}
      </div>
    );
  }

  // The default selection is the first file in tree order (folders-then-files, alphabetical) —
  // not `changes[0]`, which is just whatever order the provider happened to list them in.
  const file = changes.find((f) => f.path === selectedPath) ?? collectFiles(tree)[0]?.file ?? changes[0];

  return (
    <div className="flex flex-col h-full">
      {banner && <div className="px-5 pt-3 shrink-0">{banner}</div>}
      <div className="flex flex-1 min-h-0">
        {/* file tree */}
        <nav
          aria-label="Changed files"
          className="shrink-0 overflow-auto flex flex-col gap-px"
          style={{ width: 220, padding: "12px 10px", borderRight: "1px solid var(--border)" }}
        >
          <span className="relative flex items-center mb-1.5 shrink-0">
            <Icon name="search" size={12} className="pointer-events-none absolute" style={{ left: 9, color: "var(--fg3)" }} />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape" && query) {
                  e.stopPropagation();
                  setQuery("");
                }
              }}
              placeholder="Filter files"
              aria-label="Filter files"
              className="ft-input w-full"
              style={{ paddingLeft: 26 }}
            />
          </span>
          {rows.map(({ depth, node }) =>
            node.type === "folder" ? (
              <FolderRow key={node.path} node={node} depth={depth} open={!activeClosed.has(node.path)} onToggle={() => toggleFolder(node.path)} />
            ) : (
              <FileRow
                key={node.path}
                node={node}
                depth={depth}
                active={node.path === file.path}
                match={matches?.get(node.path) ?? null}
                query={query}
                onSelect={() => setSelectedPath(node.path)}
              />
            ),
          )}
          {hiddenCount > 0 && (
            <div className="px-2.5 py-1 text-[12px]" style={{ color: "var(--fg3)" }}>
              {hiddenCount} file{hiddenCount === 1 ? "" : "s"} hidden
            </div>
          )}
          <div className="mt-auto px-2.5 pt-2 text-[12px] num shrink-0" style={{ color: "var(--fg3)" }}>
            {viewedCount} of {totalFiles} viewed
          </div>
        </nav>
        {/* selected file's diff */}
        <div className="flex-1 overflow-auto min-w-0" style={{ padding: "12px 18px 12px 14px" }}>
          <FileDiff
            key={file.path}
            prRef={prRef}
            sha={sha}
            file={file}
            threads={threads}
            pending={pending}
            busy={busy}
            onReply={onReply}
            onResolve={onResolve}
            onAddPending={onAddPending}
            onRemovePending={onRemovePending}
            searchQuery={query}
            viewed={viewed.has(file.path)}
            onViewed={(on) => setFileViewed(file.path, on)}
          />
        </div>
      </div>
    </div>
  );
}

/** A folder row in the Files-tab tree: a muted directory label that folds. Starts open; clicking
 *  toggles it. Folded, it shows the aggregate file count and +/- it's hiding, mirroring the
 *  folder-count convention used elsewhere in the dashboard. */
function FolderRow({ node, depth, open, onToggle }: { node: DiffTreeFolder; depth: number; open: boolean; onToggle: () => void }) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      title={node.path}
      className="flex items-center gap-1.5 w-full text-left rounded-[5px] shrink-0 hover:bg-[var(--hover)]"
      style={{ height: 24, paddingLeft: 6 + depth * 12, paddingRight: 8, marginTop: 4, fontSize: 12, color: "var(--fg3)" }}
    >
      <Icon name={open ? "chevron-down" : "chevron-right"} size={12} />
      <span className="truncate flex-1">{node.name}/</span>
      {!open && (
        <span className="mono num shrink-0 flex items-center gap-1">
          {node.fileCount}
          <span style={{ color: "var(--green)" }}>+{node.additions}</span>
          <span style={{ color: "var(--red)" }}>−{node.deletions}</span>
        </span>
      )}
    </button>
  );
}

/** A file row in the Files-tab tree: change letter · name · +/-. While searching, shows its hit
 *  count (path hits + patch hits) instead, with matches in its name highlighted. */
function FileRow({
  node,
  depth,
  active,
  match,
  query,
  onSelect,
}: {
  node: DiffTreeFile;
  depth: number;
  active: boolean;
  match: FileMatch | null;
  query: string;
  onSelect: () => void;
}) {
  const f = node.file;
  const k = kindTag(f.kind);
  const hits = match ? match.pathHits + match.patchHits : 0;
  return (
    <button
      type="button"
      onClick={onSelect}
      title={f.path}
      aria-current={active || undefined}
      className={`flex items-center gap-2 w-full text-left rounded-[5px] shrink-0 ${active ? "" : "hover:bg-[var(--hover)]"}`}
      style={{
        height: 26,
        paddingLeft: 10 + depth * 12,
        paddingRight: 8,
        fontSize: 12.5,
        color: active ? "var(--fg)" : "var(--fg2)",
        background: active ? "var(--active)" : undefined,
      }}
    >
      <span className="mono shrink-0 text-center font-medium" style={{ width: 12, color: k.color }} title={f.kind}>
        {k.letter}
      </span>
      <span className="truncate flex-1 min-w-0">{match ? <Highlighted text={node.name} query={query} /> : node.name}</span>
      {match ? (
        <span className="mono num shrink-0" style={{ color: "var(--fg3)" }}>
          {hits} hit{hits === 1 ? "" : "s"}
        </span>
      ) : (
        <span className="mono num shrink-0">
          {f.additions > 0 && <span style={{ color: "var(--green)" }}>+{f.additions}</span>}
          {f.additions > 0 && f.deletions > 0 && " "}
          {f.deletions > 0 && <span style={{ color: "var(--red)" }}>−{f.deletions}</span>}
        </span>
      )}
    </button>
  );
}

/** Wraps every ASCII case-insensitive occurrence of `query` in `text` with `<mark>`. */
function Highlighted({ text, query }: { text: string; query: string }) {
  const ranges = query ? findMatches(text, query) : [];
  if (ranges.length === 0) return <>{text}</>;
  const parts: ReactNode[] = [];
  let last = 0;
  ranges.forEach(([s, e], i) => {
    if (s > last) parts.push(text.slice(last, s));
    parts.push(<mark key={i}>{text.slice(s, e)}</mark>);
    last = e;
  });
  if (last < text.length) parts.push(text.slice(last));
  return <>{parts}</>;
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

/** A hunk header's range, lifted out of its `DiffLine` for the gap arithmetic below. */
type HunkMeta = { oldStart: number; oldCount: number; newStart: number; newCount: number };

/** Diff-line geometry, shared by patch lines and revealed context lines. */
const LINE_H = 21;
const NUM_W = 38;
const GUTTER_W = 20;
/** Where a card under a diff line (thread, pending comment, composer) starts: past the gutter. */
const CARD_MARGIN = `4px 12px 4px ${NUM_W + GUTTER_W}px`;

function FileDiff({
  prRef,
  sha,
  file,
  threads,
  pending,
  busy,
  onReply,
  onResolve,
  onAddPending,
  onRemovePending,
  searchQuery,
  viewed,
  onViewed,
}: {
  prRef: PrRef;
  sha: string | null;
  file: FileChange;
  threads: CommentThread[];
  pending: LineComment[];
  busy: boolean;
  onReply: (threadId: string, body: string) => void;
  onResolve?: OnResolve;
  onAddPending: (c: LineComment) => void;
  onRemovePending: (index: number) => void;
  searchQuery: string;
  viewed: boolean;
  onViewed: (on: boolean) => void;
}) {
  const viewedId = useId();
  const [composeLine, setComposeLine] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  // Fetched once, lazily, the first time any gap in this file is clicked — shared by every gap.
  const [textRequested, setTextRequested] = useState(false);
  const fileText = usePrFileText(prRef, file.path, sha, file.patch ?? null, textRequested);
  const lines = useMemo(() => (file.patch ? parsePatch(file.patch) : []), [file.patch]);
  const fileThreads = threads.filter((t) => t.file_path === file.path && t.line != null);

  // Group the flat line list back into per-hunk chunks (header line included) so the gaps
  // between them — the unchanged lines a hunk header doesn't carry — can be worked out. A
  // provider without hunk headers at all (Azure's patches have none) puts everything in
  // `preamble` instead — it renders plainly, with no gap rows (there's nothing to anchor one to).
  const { preamble, hunks } = useMemo(() => {
    const pre: DiffLine[] = [];
    const list: { meta: HunkMeta; lines: DiffLine[] }[] = [];
    let cur: { meta: HunkMeta; lines: DiffLine[] } | null = null;
    for (const ln of lines) {
      if (ln.kind === "hunk") {
        cur = {
          meta: { oldStart: ln.oldStart ?? 1, oldCount: ln.oldCount ?? 0, newStart: ln.newStart ?? 1, newCount: ln.newCount ?? 0 },
          lines: [ln],
        };
        list.push(cur);
      } else if (cur) {
        cur.lines.push(ln);
      } else {
        pre.push(ln);
      }
    }
    return { preamble: pre, hunks: list };
  }, [lines]);

  // Only the new-side range matters here: the fetched text is the file's content at this
  // commit, which is exactly the diff's new side. Ported from the TUI's `hunk_gaps`
  // (crates/forgetop-tui/src/diff.rs) line for line, including the `+N,0` (deletion-only hunk)
  // case: such a hunk sits *after* new line N (not before it), so the gap before it runs
  // through N inclusive, and the next gap starts at N+1.
  const gaps = useMemo(() => {
    const list: { beforeIndex: number; newStart: number; newCount: number }[] = [];
    let next = 1;
    hunks.forEach((h, i) => {
      const { newStart: start, newCount: count } = h.meta;
      const end = count === 0 ? start : start - 1;
      if (end >= next) list.push({ beforeIndex: i, newStart: next, newCount: end - next + 1 });
      next = count === 0 ? start + 1 : start + count;
    });
    return list;
  }, [hunks]);

  const fileLines = useMemo(() => (fileText.data != null ? fileText.data.split("\n") : null), [fileText.data]);

  // The fetched text might no longer be this diff's base (the PR moved on since) — if a hunk's
  // own first line disagrees with what the fetch says is there, or any gap/hunk range runs
  // past the text's last line, don't trust any of it.
  const textUnavailable = useMemo(() => {
    if (fileText.isError) return true;
    if (!fileLines) return false;
    for (const h of hunks) {
      const firstContent = h.lines.find((l) => l.newLine === h.meta.newStart);
      if (firstContent && fileLines[h.meta.newStart - 1] !== firstContent.text) return true;
      if (h.meta.newCount > 0 && h.meta.newStart + h.meta.newCount - 1 > fileLines.length) return true;
    }
    for (const g of gaps) {
      if (g.newStart + g.newCount - 1 > fileLines.length) return true;
    }
    return false;
  }, [fileText.isError, fileLines, hunks, gaps]);

  const commit = (line: number) => {
    if (draft.trim()) onAddPending({ path: file.path, line, side: "New", body: draft.trim() });
    setDraft("");
    setComposeLine(null);
  };

  const renderDiffLine = (ln: DiffLine, key: string) => {
    const bg = ln.kind === "add" ? "var(--diff-add)" : ln.kind === "del" ? "var(--diff-del)" : "transparent";
    const numColor = ln.kind === "add" ? "var(--green)" : ln.kind === "del" ? "var(--red)" : "var(--fg3)";
    const marker = ln.kind === "add" ? "+" : ln.kind === "del" ? "−" : ln.kind === "hunk" ? "" : " ";
    const color = ln.kind === "hunk" ? "var(--blue)" : ln.kind === "meta" ? "var(--fg3)" : "var(--fg)";
    const canComment = (ln.kind === "add" || ln.kind === "context") && ln.newLine != null;
    const lineThreads = ln.newLine != null ? fileThreads.filter((t) => t.line === ln.newLine) : [];
    const linePending = ln.newLine != null ? pending.map((p, idx) => ({ p, idx })).filter((x) => x.p.path === file.path && x.p.line === ln.newLine) : [];
    return (
      <div key={key}>
        <div className="group mono flex items-stretch" style={{ background: bg, minHeight: LINE_H, lineHeight: `${LINE_H}px` }}>
          <span className="text-right select-none shrink-0 num" style={{ width: NUM_W, paddingRight: 10, color: numColor }}>
            {ln.kind === "hunk" ? "" : (ln.newLine ?? ln.oldLine ?? "")}
          </span>
          <span className="shrink-0 flex items-center justify-center" style={{ width: GUTTER_W }}>
            {canComment && (
              <button
                type="button"
                className="flex items-center justify-center rounded opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity"
                style={{ width: 16, height: 16, background: "var(--blue)", color: "var(--bg)" }}
                title="Comment on this line"
                aria-label="Comment on this line"
                onClick={() => setComposeLine(ln.newLine!)}
              >
                <Icon name="plus" size={11} strokeWidth={2} />
              </button>
            )}
          </span>
          <span className="pr-3 whitespace-pre" style={{ color }}>
            {marker}
            {searchQuery ? <Highlighted text={ln.text} query={searchQuery} /> : ln.text}
          </span>
        </div>
        {lineThreads.map((t) => (
          <div key={t.id} style={{ margin: CARD_MARGIN }}>
            <ThreadBox thread={t} busy={busy} onReply={onReply} onResolve={onResolve} />
          </div>
        ))}
        {linePending.map(({ p, idx }) => (
          <div key={idx} style={{ margin: CARD_MARGIN }}>
            <PendingBox body={p.body} onRemove={() => onRemovePending(idx)} />
          </div>
        ))}
        {composeLine === ln.newLine && canComment && (
          <div className="flex flex-col gap-2" style={{ margin: CARD_MARGIN }}>
            <textarea
              autoFocus
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="Leave a comment on this line…"
              aria-label="Comment on this line"
              className="ft-input w-full"
              rows={2}
            />
            <div className="flex items-center gap-1.5">
              <Button primary sm onClick={() => commit(ln.newLine!)}>
                Add comment
              </Button>
              <Button
                quiet
                sm
                onClick={() => {
                  setDraft("");
                  setComposeLine(null);
                }}
              >
                Cancel
              </Button>
            </div>
          </div>
        )}
      </div>
    );
  };

  return (
    <div>
      <div
        className="flex items-center gap-2.5 px-2.5 rounded-t-md"
        style={{ height: 30, background: "var(--hover)", border: "1px solid var(--border)" }}
      >
        <span className="mono truncate" style={{ color: "var(--fg)" }}>
          {file.path}
        </span>
        <span className="num text-[12px] shrink-0" style={{ color: "var(--green)" }}>
          +{file.additions}
        </span>
        {file.deletions > 0 && (
          <span className="num text-[12px] shrink-0" style={{ color: "var(--red)" }}>
            −{file.deletions}
          </span>
        )}
        <span className="ml-auto inline-flex items-center gap-1.5 text-[12px] shrink-0" style={{ color: "var(--fg3)" }}>
          <input id={viewedId} type="checkbox" checked={viewed} onChange={(e) => onViewed(e.target.checked)} style={{ margin: 0 }} />
          <label htmlFor={viewedId}>Viewed</label>
        </span>
      </div>
      <div className="rounded-b-md overflow-x-auto py-1.5" style={{ border: "1px solid var(--border)", borderTop: 0 }}>
        {lines.length === 0 ? (
          <div className="px-3 py-1 text-[12.5px]" style={{ color: "var(--fg3)" }}>
            {file.kind} — no inline diff available.
          </div>
        ) : (
          <>
            {preamble.map((ln, i) => renderDiffLine(ln, `pre-${i}`))}
            {hunks.map((h, i) => (
              <Fragment key={i}>
                {gaps
                  .filter((g) => g.beforeIndex === i)
                  .map((g) => (
                    <GapRow
                      key={`gap-${i}`}
                      newStart={g.newStart}
                      newCount={g.newCount}
                      fileLines={fileLines}
                      requested={textRequested}
                      loading={textRequested && fileText.isLoading}
                      unavailable={textUnavailable}
                      onRequestText={() => setTextRequested(true)}
                      highlightQuery={searchQuery}
                    />
                  ))}
                {h.lines.map((ln, li) => renderDiffLine(ln, `${i}-${li}`))}
              </Fragment>
            ))}
          </>
        )}
      </div>
    </div>
  );
}

/** A collapsed run of unchanged lines before/between hunks. Click reveals up to 10 lines from
 *  each edge (fully, when the gap is 20 lines or fewer); "Show all" reveals the whole thing in
 *  one go. The file's text is fetched lazily (shared across every gap in the file) on first
 *  click, and the row reports "loading…" or unavailability while that's in flight or fails. */
function GapRow({
  newStart,
  newCount,
  fileLines,
  requested,
  loading,
  unavailable,
  onRequestText,
  highlightQuery,
}: {
  newStart: number;
  newCount: number;
  fileLines: string[] | null;
  requested: boolean;
  loading: boolean;
  unavailable: boolean;
  onRequestText: () => void;
  highlightQuery: string;
}) {
  const [revealed, setRevealed] = useState(false);
  const [all, setAll] = useState(false);
  const strip = { minHeight: 24, paddingLeft: NUM_W + GUTTER_W - 18, paddingRight: 10, color: "var(--fg3)" } as const;

  if (requested && !loading && unavailable) {
    return (
      <div className="flex items-center text-[12px] italic" style={strip}>
        context isn't available for this file
      </div>
    );
  }

  if (!revealed) {
    return (
      <div className="flex items-center gap-2 w-full text-[12px]" style={{ ...strip, background: "var(--hover)" }}>
        <button
          type="button"
          className="flex-1 flex items-center gap-1.5 text-left hover:text-[var(--fg)]"
          style={{ minHeight: 24 }}
          aria-label={newCount > 20 ? "Show 10 more unchanged lines" : `Show all ${newCount} unchanged lines`}
          onClick={() => {
            onRequestText();
            setRevealed(true);
          }}
        >
          <Icon name="chevron-down" size={12} /> {newCount} unchanged line{newCount === 1 ? "" : "s"}
        </button>
        {newCount > 20 && (
          <Button
            quiet
            sm
            className="shrink-0"
            aria-label={`Show all ${newCount} unchanged lines`}
            onClick={() => {
              onRequestText();
              setRevealed(true);
              setAll(true);
            }}
          >
            Show all
          </Button>
        )}
      </div>
    );
  }

  if (loading) {
    return (
      <div className="flex items-center text-[12px]" style={strip}>
        loading…
      </div>
    );
  }

  if (!fileLines) return null;

  if (all || newCount <= 20) {
    return <>{sliceContextLines(fileLines, newStart, newCount, highlightQuery)}</>;
  }

  // More than 20 lines and not "show all": reveal 10 from each edge, and leave the untouched
  // middle collapsed behind its own (smaller) gap row.
  const middleCount = newCount - 20;
  return (
    <>
      {sliceContextLines(fileLines, newStart, 10, highlightQuery)}
      <GapRow
        newStart={newStart + 10}
        newCount={middleCount}
        fileLines={fileLines}
        requested={true}
        loading={false}
        unavailable={false}
        onRequestText={() => {}}
        highlightQuery={highlightQuery}
      />
      {sliceContextLines(fileLines, newStart + newCount - 10, 10, highlightQuery)}
    </>
  );
}

/** Renders `count` lines of `fileLines` starting at new-side line `start` as plain context
 *  rows — no gutter comment button, since revealed lines don't need a composer. */
function sliceContextLines(fileLines: string[], start: number, count: number, highlightQuery: string) {
  const out: ReactNode[] = [];
  for (let i = 0; i < count; i++) {
    const newLine = start + i;
    const text = fileLines[newLine - 1] ?? "";
    out.push(
      <div key={`ctx-${newLine}`} className="mono flex items-stretch" style={{ minHeight: LINE_H, lineHeight: `${LINE_H}px` }}>
        <span className="text-right select-none shrink-0 num" style={{ width: NUM_W, paddingRight: 10, color: "var(--fg3)" }}>
          {newLine}
        </span>
        <span className="shrink-0" style={{ width: GUTTER_W }} />
        <span className="pr-3 whitespace-pre" style={{ color: "var(--fg)" }}>
          {" "}
          {highlightQuery ? <Highlighted text={text} query={highlightQuery} /> : text}
        </span>
      </div>,
    );
  }
  return out;
}

/** A comment thread, as a small card: each comment's author, age and body; the thread's state
 *  and Resolve on the first line; Reply at the foot. Used in the diff and on Conversation. */
function ThreadBox({
  thread,
  busy,
  onReply,
  onResolve,
}: {
  thread: CommentThread;
  busy: boolean;
  onReply: (threadId: string, body: string) => void;
  /** Absent when the provider can't resolve threads — the state still shows, the button doesn't. */
  onResolve?: OnResolve;
}) {
  const [replying, setReplying] = useState(false);
  const [draft, setDraft] = useState("");
  const submit = () => {
    const body = draft.trim();
    if (!body) return;
    onReply(thread.id, body);
    setDraft("");
    setReplying(false);
  };
  const controls = (
    <span className="ml-auto flex items-center gap-2 shrink-0">
      <StatusBadge
        glyph={thread.is_resolved ? "check-circle" : "circle"}
        label={thread.is_resolved ? "Resolved" : "Open"}
        color={thread.is_resolved ? "var(--green)" : "var(--yellow)"}
      />
      {onResolve && thread.is_resolvable !== false && !isLocalId(thread.id) && (
        <Button quiet sm disabled={busy} onClick={() => onResolve(thread.id, !thread.is_resolved)}>
          {thread.is_resolved ? "Unresolve" : "Resolve"}
        </Button>
      )}
    </span>
  );
  return (
    <div className="rounded-md px-2.5 py-2 text-[12.5px]" style={{ background: "var(--hover)", border: "1px solid var(--border)" }}>
      {thread.comments.length === 0 && <div className="flex items-center gap-2">{controls}</div>}
      {thread.comments.map((c, i) => {
        const age = relativeTime(c.created_at);
        return (
          <div key={c.id} className={i > 0 ? "mt-2.5" : undefined}>
            <div className="flex items-center gap-2" style={{ minHeight: 22 }}>
              <Avatar name={c.author.display_name} size={16} />
              <span className="font-medium" style={{ color: "var(--fg)" }}>
                {c.author.display_name}
              </span>
              {age && <span style={{ color: "var(--fg3)" }}>{age}</span>}
              {i === 0 && controls}
            </div>
            <div className="mt-1 whitespace-pre-wrap" style={{ color: "var(--fg)" }}>
              {c.body}
            </div>
          </div>
        );
      })}
      {isLocalId(thread.id) ? (
        <div className="mt-1.5" style={{ color: "var(--fg3)" }}>
          Posting…
        </div>
      ) : replying ? (
        <div className="mt-2 flex flex-col gap-2">
          <textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="Reply…"
            aria-label="Reply"
            rows={2}
            autoFocus
            className="ft-input w-full"
          />
          <div className="flex items-center gap-1.5">
            <Button sm disabled={busy || !draft.trim()} onClick={submit}>
              Reply
            </Button>
            <Button
              quiet
              sm
              onClick={() => {
                setReplying(false);
                setDraft("");
              }}
            >
              Cancel
            </Button>
          </div>
        </div>
      ) : (
        <Button quiet sm className="mt-1.5" style={{ marginLeft: -8 }} onClick={() => setReplying(true)}>
          Reply…
        </Button>
      )}
    </div>
  );
}

/** A line comment buffered for the review, not yet sent. */
function PendingBox({ body, onRemove }: { body: string; onRemove: () => void }) {
  return (
    <div className="flex items-center gap-2 rounded-md px-2.5 py-1 text-[12.5px]" style={{ background: "var(--hover)", border: "1px solid var(--border)" }}>
      <StatusGlyph glyph="clock" color="var(--yellow)" />
      <span className="shrink-0" style={{ color: "var(--yellow)" }}>
        Pending
      </span>
      <span className="flex-1 min-w-0 whitespace-pre-wrap" style={{ color: "var(--fg)" }}>
        {body}
      </span>
      <IconButton icon="x" label="Remove pending comment" size={13} onClick={onRemove} />
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
  onResolve,
  onComment,
}: {
  threads: CommentThread[];
  description?: string | null;
  timeline: TimelineEvent[];
  busy: boolean;
  onReply: (threadId: string, body: string) => void;
  onResolve?: OnResolve;
  onComment: (body: string) => void;
}) {
  const [draft, setDraft] = useState("");
  const general = threads.filter((t) => !t.file_path);
  // Counts every open thread on the PR — inline diff threads included — since those are the
  // ones a review leaves waiting on you.
  const openCount = threads.filter((t) => !t.is_resolved).length;
  return (
    <div className="px-5 py-4 flex flex-col gap-5">
      {description && (
        <div className="text-[13px] whitespace-pre-wrap" style={{ color: "var(--fg)", lineHeight: 1.55 }}>
          {description}
        </div>
      )}
      {timeline.length > 0 && <Timeline events={timeline} />}
      <div className="flex flex-col gap-2">
        <GroupHeading>{openCount > 0 ? `Comments · ${openCount} open` : "Comments"}</GroupHeading>
        {general.length === 0 && (
          <div className="text-[12.5px]" style={{ color: "var(--fg3)" }}>
            No comments yet.
          </div>
        )}
        {general.map((t) => (
          <ThreadBox key={t.id} thread={t} busy={busy} onReply={onReply} onResolve={onResolve} />
        ))}
      </div>
      <div className="flex flex-col gap-2">
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Add a comment…"
          aria-label="Add a comment"
          rows={3}
          className="ft-input w-full"
        />
        <div>
          <Button
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
    </div>
  );
}

function CommitsTab({ commits, onSelect }: { commits: Commit[]; onSelect: (sha: string, label: string) => void }) {
  if (commits.length === 0) return <Empty text="No commits to show." />;
  return (
    <div className="flex flex-col px-2 py-2">
      {commits.map((c) => {
        const first = c.message.split("\n")[0];
        const age = relativeTime(c.date);
        return (
          <button
            key={c.sha}
            type="button"
            onClick={() => onSelect(c.sha, `${c.sha.slice(0, 7)} ${first}`)}
            title="View this commit's diff"
            className="ft-row group"
            style={{ minHeight: 32 }}
          >
            <span className="mono shrink-0" style={{ color: "var(--fg3)" }}>
              {c.sha.slice(0, 7)}
            </span>
            <span className="flex-1 truncate">{first}</span>
            <span className="text-[12px] shrink-0" style={{ color: "var(--fg2)" }}>
              {c.author}
            </span>
            <span className="num text-[12px] shrink-0 text-right" style={{ width: 56, color: "var(--fg3)" }}>
              {age}
            </span>
            <Icon name="arrow-right" size={13} className="opacity-0 group-hover:opacity-100 transition-opacity" style={{ color: "var(--fg3)" }} />
          </button>
        );
      })}
    </div>
  );
}

function Empty({ text }: { text: string }) {
  return (
    <div className="py-16 text-center text-[13px]" style={{ color: "var(--fg3)" }}>
      {text}
    </div>
  );
}

function shortRef(ref?: string | null): string {
  return ref ? ref.replace(/^refs\/heads\//, "") : "";
}
