/**
 * The inbox row menu: navigate and copy · read and done. Shared by the Notifications page's rows
 * (right-click, the hover-only `⋯`, Shift+F10). Every write goes through `useInboxActions`, the
 * same calls the row's click, its Done button and the pane's triage strip make.
 */
import { useCallback } from "react";
import { providerMeta } from "../../format";
import type { InboxThread } from "../../types";
import { opensInApp, useInboxActions } from "../inboxActions";
import { copyText, type MenuEntry } from "../ui";
import { openExternal } from "./shared";

const NO_LINK = "No link from the provider";

/** Returns the builder for an inbox row's context menu; `order` is the list the row sits in (what
 *  the opened pane's `j`/`k` walk). */
export function useNotificationRowMenu(): (thread: InboxThread, order: InboxThread[]) => MenuEntry[] {
  const actions = useInboxActions();
  return useCallback(
    (thread: InboxThread, order: InboxThread[]): MenuEntry[] => {
      const n = thread.lead.notification;
      const url = n.url ?? null;
      const provider = providerMeta(thread.lead.provider).label;
      return [
        {
          label: "Open",
          icon: opensInApp(n) ? "open-pane" : "external",
          hint: "↵",
          disabled: opensInApp(n) || url ? false : NO_LINK,
          onSelect: () => void actions.open(thread, order),
        },
        {
          label: `Open on ${provider}`,
          icon: "external",
          hint: "o",
          disabled: url ? false : NO_LINK,
          onSelect: () => {
            if (url) openExternal(url);
          },
        },
        { label: "Copy link", icon: "link", hint: "y", disabled: url ? false : NO_LINK, onSelect: () => (url ? copyText(url, "link") : undefined) },
        { type: "separator" },
        {
          label: "Mark read",
          icon: "check",
          disabled: thread.unread ? false : "Already read",
          onSelect: () => actions.markRead(thread),
        },
        { label: "Done", icon: "check-circle", hint: "e", onSelect: () => actions.done(thread) },
      ];
    },
    [actions],
  );
}
