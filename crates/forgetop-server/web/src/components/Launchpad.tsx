import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { apiPost, useLaunchpad } from "../api";
import {
  pipeMeta,
  pipeTitle,
  pipeWorkflow,
  prSignalMeta,
  prState,
  prStatusMeta,
  repoShort,
  sameRef,
  shortAge,
  wiStateColor,
  wiStateGlyph,
  type GlyphName,
} from "../format";
import type { LaunchpadMore, LaunchpadResponse, LaunchpadRow, SectionId } from "../types";
import {
  Button,
  ContextMenu,
  GroupHeading,
  Row,
  RowMenuButton,
  SectionHeading,
  Skeleton,
  StateCard,
  StatusGlyph,
  useContextMenu,
  type MenuEntry,
} from "./ui";
import { Icon } from "./icons";
import { ErrorState } from "./ErrorState";
import { useCurrentPr, usePrOpener } from "./PrDetail";
import { useCurrentWi, useWiOpener } from "./WiDetail";
import { useCurrentPipeline, usePipelineOpener } from "./PipelineDetail";
import { useNavigateSection } from "../nav";
import { usePrRowMenu } from "./rowMenus/pr";
import { useWiRowMenu } from "./rowMenus/wi";
import { usePipelineRowMenu } from "./rowMenus/pipeline";
import { useRowWrite } from "./rowMenus/shared";

/** Rows acted on this session (a PR you have just reviewed or merged): kept out of the Command
 *  Center until the page reloads, the TUI's `lp_dismissed`, because the forge's feed can keep
 *  ranking them for a while after the write. A permanent dismissal goes to the server instead. */
const actedThisSession = new Set<string>();
const rowKey = (row: LaunchpadRow) => `${row.connection_id}:${row.kind}:${rowId(row)}`;

export function Launchpad() {
  const { data, isLoading, error } = useLaunchpad();
  const rows = useMemo(() => data?.rows.filter((r) => !actedThisSession.has(rowKey(r))) ?? [], [data]);
  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || rows.length === 0)
    return <StateCard icon={<Icon name="check-circle" size={24} />} title="All clear" sub="Nothing needs your attention right now." />;

  return <Columns rows={rows} more={data.more} />;
}

/** The right column's reference lists, which get a column of their own when the width allows. */
const RECENT_BUCKETS = new Set(["recently_merged", "recent_pipelines"]);

/** How many columns a grid this wide draws: one under 900px, two up to 1520px, three beyond. The
 *  third carries the muted "Recent" lists so the two columns of work stay level; narrower, those
 *  fold back under "Your work". Width 0 (jsdom, first paint) draws the single column. */
const columnsFor = (width: number): 1 | 2 | 3 => (width >= 1520 ? 3 : width >= 900 ? 2 : 1);

/** Width of an element, tracked (0 where ResizeObserver isn't available, e.g. jsdom). */
function useWidth<T extends HTMLElement>(): [React.RefObject<T>, number] {
  const ref = useRef<T>(null);
  const [w, setW] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setW(el.getBoundingClientRect().width);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      const width = entries[0]?.borderBoxSize?.[0]?.inlineSize ?? el.getBoundingClientRect().width;
      setW(width);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, w];
}

function Columns({ rows, more }: { rows: LaunchpadRow[]; more: LaunchpadMore }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const cols = columnsFor(width);
  const left = groupByBucket(rows.filter((r) => r.column === 0));
  const rightRows = rows.filter((r) => r.column === 1);
  const yours = groupByBucket(cols === 3 ? rightRows.filter((r) => !RECENT_BUCKETS.has(r.bucket)) : rightRows);
  const recent = cols === 3 ? groupByBucket(rightRows.filter((r) => RECENT_BUCKETS.has(r.bucket))) : null;

  // Left-aligned and uncapped as a whole; each column caps at 720px so a row never stretches
  // past one eye-span on a wide monitor — the spare width goes to the right edge, not the rows.
  return (
    <div
      ref={ref}
      className="grid gap-x-11 gap-y-8 px-7 py-5 justify-start"
      style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 720px))` }}
    >
      <Column heading="Needs you" groups={left} emptyHint="Nothing waiting on you." more={more} />
      <Column heading="Your work" groups={yours} emptyHint="No open work." more={more} />
      {recent && <Column heading="Recent" groups={recent} emptyHint="Nothing recent." more={more} muted />}
    </div>
  );
}

/** Where a bucket's "more…" link goes; presets the PR page view where relevant. */
function moreTarget(bucket: string, navigate: (s: SectionId) => void): (() => void) | null {
  switch (bucket) {
    case "needs_review":
      return () => {
        try {
          localStorage.setItem("forgetop_pr_view", "review_requested");
        } catch {
          /* ignore */
        }
        navigate("prs");
      };
    case "your_work":
      return () => navigate("work-items");
    case "your_open_prs":
      return () => {
        try {
          localStorage.setItem("forgetop_pr_view", "yours");
        } catch {
          /* ignore */
        }
        navigate("prs");
      };
    case "recently_merged":
      return () => {
        try {
          localStorage.setItem("forgetop_pr_view", "merged");
        } catch {
          /* ignore */
        }
        navigate("prs");
      };
    case "recent_pipelines":
      return () => navigate("pipelines");
    default:
      return null;
  }
}

interface Group {
  bucket: string;
  title: string;
  muted: boolean;
  rows: LaunchpadRow[];
}

function groupByBucket(rows: LaunchpadRow[]): Group[] {
  const groups: Group[] = [];
  for (const row of rows) {
    let g = groups[groups.length - 1];
    if (!g || g.bucket !== row.bucket) {
      g = { bucket: row.bucket, title: row.bucket_title, muted: row.muted, rows: [] };
      groups.push(g);
    }
    g.rows.push(row);
  }
  return groups;
}

function Column({
  heading,
  groups,
  emptyHint,
  more,
  muted,
}: {
  heading: string;
  groups: Group[];
  emptyHint: string;
  more: LaunchpadMore;
  /** A column of reference lists: its heading reads as quiet as its groups do. */
  muted?: boolean;
}) {
  const total = groups.reduce((n, g) => n + g.rows.length, 0);
  return (
    <section className="min-w-0">
      <SectionHeading count={total}>
        <span style={muted ? { color: "var(--fg2)" } : undefined}>{heading}</span>
      </SectionHeading>
      {groups.length === 0 ? (
        <p className="mt-3 text-[13px]" style={{ color: "var(--fg3)" }}>
          {emptyHint}
        </p>
      ) : (
        groups.map((g) => <BucketGroup key={g.bucket} group={g} more={more} />)
      )}
    </section>
  );
}

/** Buckets with no clean deep-link target: capped in the UI and revealed in place, a page at a time. */
const EXPAND_BUCKETS = new Set(["ready_to_merge", "needs_fixing"]);
const EXPAND_STEP = 5;

function BucketGroup({ group, more }: { group: Group; more: LaunchpadMore }) {
  const navigate = useNavigateSection();
  const isExpand = EXPAND_BUCKETS.has(group.bucket);
  const [limit, setLimit] = useState(EXPAND_STEP);

  // Expand buckets are returned whole and revealed in place; the rest are already capped by the
  // backend and their "more…" deep-links to the full page/view.
  const rows = isExpand ? group.rows.slice(0, limit) : group.rows;
  const expandMore = isExpand && group.rows.length > limit;
  const navMore = !isExpand && (more as unknown as Record<string, boolean>)[group.bucket] === true;
  const go = navMore ? moreTarget(group.bucket, navigate) : null;

  return (
    <div className="mt-[18px]">
      <GroupHeading count={group.rows.length}>
        <span style={group.muted ? { color: "var(--fg3)" } : undefined}>{group.title}</span>
      </GroupHeading>
      {/* Rows carry their own 12px padding; pull them out so their content lines up with the headings. */}
      <div className="flex flex-col -mx-3">
        {rows.map((row) => (
          <ItemRow key={`${row.connection_id}:${row.kind}:${rowId(row)}`} row={row} bucket={group.bucket} />
        ))}
      </div>
      {(go || expandMore) && (
        <Button quiet sm className="mt-1 -ml-2" onClick={go ?? (() => setLimit((l) => l + EXPAND_STEP))}>
          more…
        </Button>
      )}
    </div>
  );
}

/** What one Command Center row draws, in reading order: glyph · title (· note) · identifier ·
 *  signal glyph · age. Work items put their key before the title instead. */
interface RowView {
  glyph: GlyphName;
  color: string;
  spin?: boolean;
  /** The state, announced on the glyph (the row no longer prints it as a word). */
  glyphTitle: string;
  title: string;
  note?: string;
  noteColor?: string;
  ident: string;
  /** Work items: the key leads, in a fixed column. */
  keyFirst?: boolean;
  signal?: { glyph: GlyphName; color: string; spin: boolean; title: string };
  age: string;
}

function ItemRow({ row, bucket }: { row: LaunchpadRow; bucket: string }) {
  const v = view(row, bucket);
  const openPr = usePrOpener();
  const openWi = useWiOpener();
  const openPipe = usePipelineOpener();
  const currentPr = useCurrentPr();
  const currentWi = useCurrentWi();
  const currentPipe = useCurrentPipeline();
  // The row whose item is open in the pane carries the selected wash.
  const selected =
    row.kind === "pr"
      ? sameRef(currentPr, { conn: row.connection_id, repo: row.pull_request.repository, id: row.pull_request.id })
      : row.kind === "wi"
        ? sameRef(currentWi, { conn: row.connection_id, repo: row.work_item.repository, id: row.work_item.id })
        : sameRef(currentPipe, { conn: row.connection_id, repo: row.run.repository, runId: row.run.id });
  // Every row kind opens its in-app detail panel, matching the list views.
  const onClick =
    row.kind === "pr"
      ? () => openPr({ conn: row.connection_id, repo: row.pull_request.repository, id: row.pull_request.id })
      : row.kind === "wi"
        ? () => openWi({ conn: row.connection_id, repo: row.work_item.repository, id: row.work_item.id })
        : () => openPipe({ conn: row.connection_id, repo: row.run.repository, runId: row.run.id });

  const menu = useContextMenu();
  const buildPr = usePrRowMenu();
  const buildWi = useWiRowMenu();
  const buildPipe = usePipelineRowMenu();
  const write = useRowWrite();
  const qc = useQueryClient();
  // Drops this row from the Command Center at once and keeps it out for the rest of the
  // session (the TUI's `lp_dismissed`): the forge may keep ranking a PR you have just reviewed
  // until its own feed catches up. A poll already in flight is cancelled first so it cannot land
  // over the edit.
  const dropRow = () => {
    actedThisSession.add(rowKey(row));
    void qc.cancelQueries({ queryKey: ["launchpad"] });
    qc.setQueriesData<LaunchpadResponse>({ queryKey: ["launchpad"] }, (lp) =>
      lp && { ...lp, rows: lp.rows.filter((r) => r !== row && !sameItem(r, row)) },
    );
  };
  const dismiss = () =>
    write("Dismissed", () => apiPost("/api/launchpad/dismiss", { conn: row.connection_id, kind: row.kind, ...itemAddress(row) }), {
      optimistic: dropRow,
      invalidate: [["launchpad"]],
    }).then(() => undefined);
  // The muted reference lists ("Recent…") are not asking for anything, so there is nothing to dismiss.
  const extra: MenuEntry[] | undefined = row.muted
    ? undefined
    : [{ type: "separator" }, { label: "Dismiss from Command Center", icon: "minus-circle", hint: "D", onSelect: dismiss }];
  const menuLabel = row.kind === "pr" ? "Pull request actions" : row.kind === "wi" ? "Work item actions" : "Pipeline run actions";
  // A PR leaves its bucket as soon as you act on it (the TUI does the same after a vote); work
  // items and runs are patched in place by their builders' optimistic writes.
  const build = (): MenuEntry[] =>
    row.kind === "pr" ? buildPr(row, { onActed: dropRow, extra }) : row.kind === "wi" ? buildWi(row, { extra }) : buildPipe(row, { extra });

  const ident: ReactNode = v.ident ? (
    <span className="mono shrink-0 truncate" style={{ color: "var(--fg3)", width: v.keyFirst ? 76 : undefined, maxWidth: v.keyFirst ? undefined : "40%" }}>
      {v.ident}
    </span>
  ) : v.keyFirst ? (
    <span className="shrink-0" style={{ width: 76 }} />
  ) : null;

  return (
    <>
      <Row onClick={onClick} selected={selected} onContextMenu={menu.onContextMenu} onKeyDown={menu.onKeyDown} menuOpen={menu.open}>
        <StatusGlyph glyph={v.glyph} color={v.color} spin={v.spin} title={v.glyphTitle} />
        {v.keyFirst && ident}
        <span className="flex-1 min-w-0 truncate">
          {v.title}
          {v.note && (
            <span style={{ color: v.noteColor ?? "var(--fg3)" }}>
              <span aria-hidden="true">{" · "}</span>
              {v.note}
            </span>
          )}
        </span>
        {!v.keyFirst && ident}
        {v.signal && <StatusGlyph glyph={v.signal.glyph} color={v.signal.color} spin={v.signal.spin} title={v.signal.title} />}
        <span className="num shrink-0 text-right text-[12px]" style={{ width: 34, color: "var(--fg3)" }}>
          {v.age}
        </span>
        <RowMenuButton onOpen={menu.openAt} />
      </Row>
      <ContextMenu anchor={menu.anchor} onClose={menu.close} label={menuLabel} items={build} />
    </>
  );
}

function view(row: LaunchpadRow, bucket: string): RowView {
  if (row.kind === "pr") {
    const pr = row.pull_request;
    const status = prStatusMeta(pr);
    const signal = prSignalMeta(pr);
    const st = prState(pr);
    const ident = [repoShort(pr.repository), pr.number != null ? `#${pr.number}` : ""].filter(Boolean).join(" ");
    const base = {
      glyph: status.glyph,
      color: status.color,
      glyphTitle: capitalize(status.label),
      title: pr.title,
      ident,
      age: shortAge(pr.updated_at),
    };
    // "Needs fixing" is the blocker's bucket: the row reads red and names what to fix.
    if (bucket === "needs_fixing") {
      return { ...base, color: "var(--red)", note: st.kind === "blocked" ? signal.label : undefined };
    }
    const open = st.kind !== "merged" && st.kind !== "closed";
    const hasSignal = open && (st.kind === "blocked" || pr.checks !== "None");
    return {
      ...base,
      note: st.kind === "draft" ? "draft" : undefined,
      signal: hasSignal ? { glyph: signal.glyph, color: signal.color, spin: signal.spin, title: signal.label } : undefined,
    };
  }
  if (row.kind === "wi") {
    const wi = row.work_item;
    const blocked = wi.state.toLowerCase() === "blocked";
    return {
      glyph: wiStateGlyph(wi.state, wi.state_category),
      color: wiStateColor(wi.state, wi.state_category),
      glyphTitle: wi.state,
      title: wi.title,
      note: blocked ? "blocked" : undefined,
      noteColor: blocked ? "var(--red)" : undefined,
      ident: wi.identifier ?? "",
      keyFirst: true,
      age: shortAge(wi.updated_at),
    };
  }
  const run = row.run;
  const m = pipeMeta(run.status);
  // The title is what the run was building; the workflow it ran under goes in the note — and is
  // dropped there when it *is* the title, so it never reads as "Integration · Integration".
  const title = pipeTitle(run, row.definition_name);
  const workflow = pipeWorkflow(run, row.definition_name);
  const note = [workflow === title ? "" : workflow, run.branch ?? ""].filter(Boolean).join(" · ");
  return {
    glyph: m.glyph,
    color: m.color,
    spin: m.running,
    glyphTitle: run.status === "Failed" ? "Error" : capitalize(m.label),
    title,
    note: note || undefined,
    ident: repoShort(run.repository),
    age: shortAge(run.finished_at ?? run.started_at),
  };
}

const capitalize = (s: string): string => (s.length ? s[0].toUpperCase() + s.slice(1) : s);

/// A row's identity within its connection. The repository is part of it: one connection now
/// spans an account, so `#7` exists in more than one repository and `connection_id:id` alone
/// stops being unique.
function rowId(row: LaunchpadRow): string {
  if (row.kind === "pr") return `${row.pull_request.repository ?? ""}:${row.pull_request.id}`;
  if (row.kind === "wi") return `${row.work_item.repository ?? ""}:${row.work_item.id}`;
  return `${row.run.repository ?? ""}:${row.run.id}`;
}

/** The repository and id a row's item is addressed by — what `/api/launchpad/dismiss` keys on. */
function itemAddress(row: LaunchpadRow): { repo: string | null; id: string } {
  if (row.kind === "pr") return { repo: row.pull_request.repository ?? null, id: row.pull_request.id };
  if (row.kind === "wi") return { repo: row.work_item.repository ?? null, id: row.work_item.id };
  return { repo: row.run.repository ?? null, id: row.run.id };
}

/** Whether two rows draw the same item (the same PR can sit in more than one bucket). */
function sameItem(a: LaunchpadRow, b: LaunchpadRow): boolean {
  return a.connection_id === b.connection_id && a.kind === b.kind && rowId(a) === rowId(b);
}
