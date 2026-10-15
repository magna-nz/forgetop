/**
 * The work-item row menu: navigate and copy · move and assign, edit and comment · the
 * hide-this-state view preference. Shared by the Work Items list and the Command Center.
 */
import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { apiGet, apiPost, prefsKey, repoParam, usePrefs, wiDetailKey, type Prefs } from "../../api";
import { providerMeta, wiStateColor, wiStateGlyph } from "../../format";
import { patchWorkItem } from "../../optimistic";
import type { User, WiRef } from "../../types";
import { copyText, type MenuEntry } from "../ui";
import { useWiOpener } from "../WiDetail";
import { openExternal, useRowWrite, type RowMenuOpts, type WiRowLike } from "./shared";

const NO_LINK = "No link from the provider";

/** `conn=…&id=…[&repo=…]` — the query the per-item work-item endpoints take. */
const itemQuery = (ref: WiRef) => `conn=${encodeURIComponent(ref.conn)}&id=${encodeURIComponent(ref.id)}${repoParam(ref.repo)}`;

/**
 * Hides a work-item state from the Work Items list, or shows it again when it is hidden — the
 * one toggle behind both the row menu's `Hide "<state>" items` and the list's `Show "<state>"`
 * chips. Toggles against the cached prefs at call time, patches them at once, then posts that
 * one state's new setting (`POST /api/prefs/hidden-work-item-state`): the server merges it into
 * the saved list, so the TUI's own choices are never written over. Resolves to whether it saved.
 */
export function useToggleHiddenState(): (state: string) => Promise<boolean> {
  const qc = useQueryClient();
  const write = useRowWrite();
  return useCallback(
    (state: string) => {
      const hidden = qc.getQueryData<Prefs>(prefsKey)?.hidden_work_item_states ?? [];
      const isHidden = hidden.includes(state);
      const states = isHidden ? hidden.filter((s) => s !== state) : [...hidden, state];
      return write(isHidden ? `Showing "${state}" items` : `Hiding "${state}" items`, () => apiPost("/api/prefs/hidden-work-item-state", { state, hidden: !isHidden }), {
        optimistic: () => qc.setQueryData<Prefs>(prefsKey, (p) => ({ ...p, hidden_work_item_states: states })),
        invalidate: [prefsKey],
      });
    },
    [qc, write],
  );
}

/** Returns the builder for a work-item row's context menu (see the module comment). */
export function useWiRowMenu(): (row: WiRowLike, opts?: RowMenuOpts) => MenuEntry[] {
  const open = useWiOpener();
  const write = useRowWrite();
  const qc = useQueryClient();
  const prefs = usePrefs();
  const toggleHidden = useToggleHiddenState();
  const hidden = prefs.data?.hidden_work_item_states;

  return useCallback(
    (row: WiRowLike, opts?: RowMenuOpts): MenuEntry[] => {
      const wi = row.work_item;
      const ref: WiRef = { conn: row.connection_id, repo: wi.repository, id: wi.id };
      const invalidate = [["work-items"], ["launchpad"], wiDetailKey(ref)];
      const body = { conn: ref.conn, repo: ref.repo, id: ref.id };

      // Only the state *name* is knowable: /api/wi/states answers with raw names, so
      // `state_category` is left as it was until the refetch supplies the real one (as the pane).
      const moveTo = (state: string) => {
        const done = write(`Moved to ${state}`, () => apiPost("/api/wi/state", { ...body, state }), {
          optimistic: () => patchWorkItem(qc, ref, { state }),
          invalidate,
        });
        opts?.onActed?.();
        return done.then(() => undefined);
      };
      const assign = (user: User | null) => {
        const done = write(user ? `Assigned to ${user.display_name}` : "Unassigned", () => apiPost("/api/wi/assignee", { ...body, assignee_id: user?.id ?? null }), {
          optimistic: () => patchWorkItem(qc, ref, { assignee: user }),
          invalidate,
        });
        opts?.onActed?.();
        return done.then(() => undefined);
      };

      const entries: MenuEntry[] = [
        { label: "Open", icon: "open-pane", hint: "↵", onSelect: () => void open(ref) },
        {
          label: `Open in ${providerMeta(row.provider).label}`,
          icon: "external",
          hint: "o",
          disabled: wi.url ? false : NO_LINK,
          onSelect: () => {
            if (wi.url) openExternal(wi.url);
          },
        },
        {
          label: "Copy link",
          icon: "link",
          hint: "y",
          disabled: wi.url ? false : NO_LINK,
          onSelect: () => (wi.url ? copyText(wi.url, "link") : undefined),
        },
      ];
      if (wi.identifier) {
        const identifier = wi.identifier;
        entries.push({ label: `Copy ${identifier}`, icon: "copy", value: identifier, onSelect: () => copyText(identifier, identifier) });
      }

      entries.push(
        { type: "separator" },
        {
          label: "Move to",
          icon: "arrow-right",
          submenu: async () => {
            const states = await apiGet<string[]>(`/api/wi/states?${itemQuery(ref)}`);
            return states.map((state): MenuEntry => {
              const current = state.toLowerCase() === wi.state.toLowerCase();
              // Only the current state's category is known, so only it gets a status glyph: a
              // glyph guessed from the row's category would tell the user something false.
              return {
                id: `state:${state}`,
                label: state,
                ...(current
                  ? {
                      icon: wiStateGlyph(wi.state, wi.state_category),
                      iconColor: wiStateColor(wi.state, wi.state_category),
                      checked: true,
                      disabled: "Current state",
                    }
                  : {}),
                onSelect: () => moveTo(state),
              };
            });
          },
        },
        {
          label: "Assign",
          icon: "user",
          submenu: async () => {
            const users = await apiGet<User[]>(`/api/wi/assignees?${itemQuery(ref)}`);
            const unassigned: MenuEntry = {
              id: "assignee:none",
              label: "Unassigned",
              ...(wi.assignee ? {} : { checked: true, disabled: "Already unassigned" }),
              onSelect: () => assign(null),
            };
            return [
              unassigned,
              ...users.map((user): MenuEntry => ({
                id: `assignee:${user.id}`,
                label: user.display_name,
                value: user.handle ?? undefined,
                ...(wi.assignee?.id === user.id ? { checked: true, disabled: "Already assigned" } : {}),
                onSelect: () => assign(user),
              })),
            ];
          },
        },
        { label: "Edit title & description…", icon: "pencil", hint: "e", onSelect: () => void open(ref, "edit") },
        { label: "Comment…", icon: "comment", hint: "c", onSelect: () => void open(ref, "comment") },
        { type: "separator" },
        {
          id: "hide-state",
          // A checkbox: ticked means hidden, so the label stays "Hide…" either way.
          label: `Hide "${wi.state}" items`,
          checked: hidden?.includes(wi.state) ?? false,
          disabled: hidden ? false : "Loading preferences…",
          onSelect: () => toggleHidden(wi.state).then(() => undefined),
        },
      );

      if (opts?.extra?.length) entries.push({ type: "separator" }, ...opts.extra);
      return entries;
    },
    [open, write, qc, hidden, toggleHidden],
  );
}
