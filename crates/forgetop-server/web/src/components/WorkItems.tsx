import { useCallback, useMemo, useState } from "react";
import { usePrefs, useWorkItems } from "../api";
import { providerMeta, sameRef, toTime, wiStateColor, wiStateGlyph } from "../format";
import type { WiRow, WorkItemParent } from "../types";
import { Icon } from "./icons";
import { Avatar, Button, ContextMenu, copyText, List, Row, RowMenuButton, Select, Skeleton, StateCard, StatusGlyph, useContextMenu, type MenuEntry } from "./ui";
import { ErrorState } from "./ErrorState";
import { shortAge, ToggleRow, useListView } from "./ControlBar";
import { useCurrentWi, useWiOpener } from "./WiDetail";
import { useToggleHiddenState, useWiRowMenu } from "./rowMenus/wi";
import { openExternal } from "./rowMenus/shared";

const GROUP_KEY = "forgetop:work-items:group";
type GroupMode = "off" | "parent";

/** Grouped by parent unless the viewer chose otherwise. */
function readGroupMode(): GroupMode {
  try {
    return localStorage.getItem(GROUP_KEY) === "off" ? "off" : "parent";
  } catch {
    return "parent";
  }
}

interface WiGroup {
  key: string;
  parent: WorkItemParent | null; // null = the trailing "No parent" group
  rows: WiRow[];
}

/** Buckets already-sorted rows by parent identity. Groups keep the order of their first row;
 *  parentless rows form one final group. Rows keep their order within a group.
 *
 *  Empty when no row has a parent: there is nothing to group by, and a lone "No parent" section
 *  over the whole list would only hide it. That is what lets grouping be the default. */
function groupByParent(rows: WiRow[]): WiGroup[] {
  if (!rows.some((r) => r.work_item.parent)) return [];
  const groups = new Map<string, WiGroup>();
  const none: WiGroup = { key: "none", parent: null, rows: [] };
  for (const row of rows) {
    const p = row.work_item.parent;
    if (!p) {
      none.rows.push(row);
      continue;
    }
    const key = `${row.connection_id}|${p.kind}|${p.id}`;
    const g = groups.get(key);
    if (g) g.rows.push(row);
    else groups.set(key, { key, parent: p, rows: [row] });
  }
  return none.rows.length ? [...groups.values(), none] : [...groups.values()];
}

export function WorkItems() {
  const { data, isLoading, error } = useWorkItems();
  const hiddenStates = usePrefs().data?.hidden_work_item_states;
  const toggleHidden = useToggleHiddenState();
  // The hidden states are a view preference (shared with the TUI): their rows are dropped before
  // the list view sees them, so its count and the groups agree with what is shown.
  const visible = useMemo(
    () => (hiddenStates?.length ? data?.filter((r) => !hiddenStates.includes(r.work_item.state)) : data),
    [data, hiddenStates],
  );
  const [mode, setMode] = useState<GroupMode>(readGroupMode);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const chooseMode = (m: GroupMode) => {
    setMode(m);
    try {
      localStorage.setItem(GROUP_KEY, m);
    } catch {
      /* ignore */
    }
  };
  const { rows, bar } = useListView<WiRow>({
    storageKey: "work-items",
    rows: visible,
    connId: (r) => r.connection_id,
    connLabel: (r) => r.connection,
    sorts: [
      { label: "Recently updated", cmp: (a, b) => toTime(b.work_item.updated_at) - toTime(a.work_item.updated_at) },
      { label: "Oldest", cmp: (a, b) => toTime(a.work_item.updated_at) - toTime(b.work_item.updated_at) },
      { label: "Title A–Z", cmp: (a, b) => a.work_item.title.localeCompare(b.work_item.title) },
    ],
    statuses: [
      // In flight: being worked on, or past that and in review.
      { label: "In progress", match: (r) => r.work_item.state_category === "Started" || r.work_item.state_category === "Review" },
      { label: "Blocked", match: (r) => r.work_item.state.toLowerCase() === "blocked" },
      { label: "Not started", match: (r) => ["Unstarted", "Backlog", "Triage"].includes(r.work_item.state_category) },
    ],
    statusLabel: "Show",
    facet: { label: "Type", value: (r) => r.work_item.work_item_type },
    extra: (
      <>
        <Select label="Group" value={mode} onChange={(e) => chooseMode(e.target.value === "parent" ? "parent" : "off")}>
          <option value="parent">By parent</option>
          <option value="off">Flat</option>
        </Select>
        {(hiddenStates ?? []).map((state) => (
          <Button key={state} quiet sm icon="x" aria-label={`Show "${state}"`} title={`Show "${state}" items`} onClick={() => void toggleHidden(state)}>
            {state}
          </Button>
        ))}
      </>
    ),
  });

  const groups = useMemo(() => (mode === "parent" ? groupByParent(rows) : []), [mode, rows]);

  if (isLoading) return <Skeleton />;
  if (error) return <ErrorState error={error} />;
  if (!data || data.length === 0)
    return <StateCard icon={<Icon name="circle-dot" size={24} />} title="No work items assigned" sub="Issues and tickets assigned to you appear here." />;

  const toggle = (key: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  return (
    <>
      {bar}
      <List>
        {groups.length > 0
          ? groups.map((g) => (
              <WiGroupSection
                key={g.key}
                group={g}
                open={expanded.has(g.key)}
                onToggle={() => toggle(g.key)}
                onExpandAll={() => setExpanded(new Set(groups.map((x) => x.key)))}
                onCollapseAll={() => setExpanded(new Set())}
              />
            ))
          : rows.map((row) => <WiListRow key={wiKey(row)} row={row} />)}
      </List>
    </>
  );
}

const wiKey = (row: WiRow) => `${row.connection_id}:${row.work_item.repository ?? ""}:${row.work_item.id}`;

const isBlocked = (row: WiRow) => row.work_item.state.toLowerCase() === "blocked";

function WiGroupSection({
  group,
  open,
  onToggle,
  onExpandAll,
  onCollapseAll,
}: {
  group: WiGroup;
  open: boolean;
  onToggle: () => void;
  onExpandAll: () => void;
  onCollapseAll: () => void;
}) {
  const { parent, rows } = group;
  const menu = useContextMenu();
  // The header's own menu: the section, then the parent it stands for. The items beneath keep
  // their own row menus.
  const groupMenu = (): MenuEntry[] => {
    const provider = providerMeta(rows[0].provider).label;
    const entries: MenuEntry[] = [
      { label: open ? "Collapse" : "Expand", icon: open ? "chevron-down" : "chevron-right", hint: "↵", onSelect: onToggle },
      { label: "Expand all", onSelect: onExpandAll },
      { label: "Collapse all", onSelect: onCollapseAll },
    ];
    if (parent) {
      const noLink = parent.url ? false : "No link from the provider";
      entries.push(
        { type: "separator" },
        { label: `Open ${parent.kind.toLowerCase()} in ${provider}`, icon: "external", hint: "o", disabled: noLink, onSelect: () => openExternal(parent.url!) },
        { label: "Copy link", icon: "link", hint: "y", disabled: noLink, onSelect: () => copyText(parent.url!, "link") },
      );
    }
    return entries;
  };
  const blocked = rows.filter(isBlocked).length;
  const unblockedIn = (category: string) => rows.filter((r) => r.work_item.state_category === category && !isBlocked(r)).length;
  const inProgress = unblockedIn("Started");
  const inReview = unblockedIn("Review");
  const counts: { text: string; color: string }[] = [{ text: `${rows.length} ${rows.length === 1 ? "item" : "items"}`, color: "var(--fg3)" }];
  if (inProgress > 0) counts.push({ text: `${inProgress} in progress`, color: "var(--fg3)" });
  if (inReview > 0) counts.push({ text: `${inReview} in review`, color: "var(--cyan)" });
  if (blocked > 0) counts.push({ text: `${blocked} blocked`, color: "var(--red)" });
  return (
    <>
      {/* The link sits beside the toggle, not inside it: a link within a button is not a link. */}
      <div className="relative">
        <ToggleRow
          open={open}
          onToggle={onToggle}
          onContextMenu={menu.onContextMenu}
          onKeyDown={menu.onKeyDown}
          menuOpen={menu.open}
          style={parent?.url ? { paddingRight: 44 } : undefined}
        >
          {parent && (
            <span className="text-[12px] shrink-0" style={{ color: "var(--fg3)" }}>
              {parent.kind}
            </span>
          )}
          <span className="truncate font-medium min-w-0">{parent ? parent.title : "No parent"}</span>
          <span className="flex items-center gap-1.5 shrink-0 text-[12px] num">
            {counts.map((c, i) => (
              <span key={c.text} className="inline-flex items-center gap-1.5">
                {i > 0 && (
                  <span aria-hidden="true" style={{ color: "var(--fg3)" }}>
                    ·
                  </span>
                )}
                <span style={{ color: c.color }}>{c.text}</span>
              </span>
            ))}
          </span>
        </ToggleRow>
        {parent?.url && (
          <a
            href={parent.url}
            target="_blank"
            rel="noreferrer"
            className="ft-btn ft-btn-icon absolute right-1 top-1/2 -translate-y-1/2"
            aria-label={`Open ${parent.kind} ${parent.title} in a new tab`}
            title="Open in a new tab"
          >
            <Icon name="external" size={14} />
          </a>
        )}
      </div>
      <ContextMenu anchor={menu.anchor} onClose={menu.close} label={parent ? `${parent.kind} actions` : "Group actions"} items={groupMenu} />
      {open && rows.map((row) => <WiListRow key={wiKey(row)} row={row} indent />)}
    </>
  );
}

/** One work item: glyph · key · title · type · assignee · age. Under a group header it is
 *  indented past the chevron so its glyph lines up with the parent's title. */
function WiListRow({ row, indent = false }: { row: WiRow; indent?: boolean }) {
  const wi = row.work_item;
  const open = useWiOpener();
  const current = useCurrentWi();
  const menu = useContextMenu();
  const build = useWiRowMenu();
  const items = useCallback(() => build(row), [build, row]);
  const blocked = isBlocked(row);
  return (
    <>
      <Row
        selected={sameRef(current, { conn: row.connection_id, repo: wi.repository, id: wi.id })}
        onClick={() => open({ conn: row.connection_id, repo: wi.repository, id: wi.id })}
        onContextMenu={menu.onContextMenu}
        onKeyDown={menu.onKeyDown}
        menuOpen={menu.open}
        className={indent ? "pl-9!" : undefined}
      >
        <StatusGlyph glyph={wiStateGlyph(wi.state, wi.state_category)} color={wiStateColor(wi.state, wi.state_category)} title={wi.state} />
        <span className="mono shrink-0 truncate" style={{ width: 76, color: "var(--fg3)" }}>
          {wi.identifier}
        </span>
        <span className="flex-1 min-w-0 truncate">
          <span>{wi.title}</span>
          {blocked && <span style={{ color: "var(--red)" }}> · blocked</span>}
        </span>
        {wi.work_item_type && (
          <span className="text-[12px] shrink-0 whitespace-nowrap" style={{ color: "var(--fg3)" }}>
            {wi.work_item_type}
          </span>
        )}
        <span className="inline-flex shrink-0 justify-center" style={{ width: 18 }}>
          {wi.assignee && <Avatar name={wi.assignee.display_name} size={18} />}
        </span>
        <span className="num text-[12px] shrink-0 text-right whitespace-nowrap" style={{ width: 34, color: "var(--fg3)" }}>
          {shortAge(wi.updated_at)}
        </span>
        <RowMenuButton onOpen={menu.openAt} />
      </Row>
      <ContextMenu anchor={menu.anchor} onClose={menu.close} label="Work item actions" items={items} />
    </>
  );
}
