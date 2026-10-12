# Dashboard design system — "Quiet ink"

The dashboard is a dense, keyboard-first work surface in the spirit of Linear and GitHub, not
a marketing dashboard. Everything below is already implemented in `src/index.css`,
`src/components/ui.tsx` and `src/components/icons.tsx`; a screen's job is to use it.

## Rules (in order of how often they are broken)

1. **Rows, not cards.** A list item is a `<Row>`: 36px tall, hairline below, hover tint,
   accent wash when selected. Never a bordered rounded box per item. Cards are only for a
   standalone block inside a pane (a comment, a code hunk, a gate banner).
2. **Status is a glyph plus a word.** Use `<StatusGlyph glyph=… color=…>` (14px stroke icon)
   with the state word in plain text where the word is needed. Never a tinted, bordered chip.
   `format.ts` meta functions (`prStatusMeta`, `checkMeta`, `pipeMeta`, `voteMeta`,
   `prSignalMeta`, `wiStateGlyph`, …) return a `glyph` and `color` for every state.
3. **One accent, three uses.** `var(--accent)` is for the single primary button on a screen,
   the selected row, and focus rings. Nothing else is orange. Active nav is neutral.
4. **Red means failed or blocked.** Amber is waiting / changes requested / in progress. Green is
   done / passing. Blue is running. Grey is draft, queued, todo. Purple is merged.
5. **Type: one family, five sizes.** IBM Plex Sans 13px body (`--fg`), 12px meta (`--fg2`,
   `--fg3`), 14px/600 section, 15px/600 page title, 17px/600 pane title. No uppercase tracked
   labels. No 11px text. Secondary text is the same size in a lighter colour, never smaller.
6. **Mono is for identifiers only**: `#1501`, `OPS-1423`, SHAs, branches, repository slugs.
   Use `className="mono"`. Never mono for sentences or metadata like "12h ago".
7. **Numbers are tabular** (`className="num"`) and right-aligned in a fixed-width column.
8. **Metadata never repeats the section.** On the Pull Requests page a row does not say
   "Pull Request". Order inside a row: glyph · title · identifier (mono) · signal glyph · age.
9. **Buttons**: `<Button primary>` once per screen (Merge, Approve the gate, Save), `<Button>`
   outlined for secondary, `<Button quiet>` for rare ones, `<Button danger>` for destructive.
   Keyboard hints live inside the button as `<Kbd>`. Fold rare actions into a "More ▾" menu.
10. **Filters are quiet menus** (`<Select>`: a native select dressed as a text button), grouping
    is a `<Segmented>` control, views are `<Tabs>`. Never a raw `<select>` with browser chrome.
11. **Detail is a split pane**, never a modal: `<SlideOver>` renders beside the content and the
    list shrinks. No backdrop, no dimming. Esc and the ✕ close it.
12. **No emoji or text glyphs as icons.** Use `<Icon name=…>`.
13. **Empty and loading states** use `<StateCard>` and `<Skeleton>` (both restyled); keep them.

## Tokens (`src/index.css`)

Surfaces `--bg` (app), `--panel` (sidebar), `--raised` (popovers), `--hover`, `--active`,
`--sel` (selected row wash), `--border`, `--border-strong`.
Text `--fg`, `--fg2`, `--fg3`. Accent `--accent`, `--accent-hover`, `--accent-fg` (links),
`--accent-contrast` (text on accent). Status `--green --red --yellow --blue --magenta --cyan`.
Diff `--diff-add`, `--diff-del`. Legacy `--card`, `--card-hover`, `--panel2`, `--dim` still
resolve but new code should not use them.

Four themes keep their names (slate = default, dark, light, matrix) and all define every token.

## Primitives (`src/components/ui.tsx`)

- `Row({ onClick | href, selected, dense, children })` — the list row.
- `StatusGlyph({ glyph, color, size, spin, title })` — status icon.
- `Button({ primary, quiet, danger, sm, icon, children, ...button props })`,
  `IconButton({ icon, label, ...})`, `Kbd`.
- `Select` (quiet native select), `Segmented({ options, value, onChange })`,
  `Tabs({ tabs, value, onChange })`.
- `SectionHeading({ children, count })` 14px/600; `GroupHeading({ children, count })` 12px/500
  with a hairline, for sub-groups inside a section.
- `Chip` (mono tag, no border), `Pill` (glyph + word, no box), `StatusBadge` (glyph + word; the
  old chip look is gone), `Avatar`, `ProviderBadge`, `Timeline`, `StateCard`, `Skeleton`,
  `List` (full-width, no gaps), `Divider`.
- `SlideOver({ onClose, header, children, footer, width: "md" | "lg" | "xl" })` — the split
  pane. Header gets the title block; `footer` the action bar.

## Layout

Sidebar 240px (`--panel`), top bar 48px with the page title on the left and Search ⌘K,
refresh and the bell on the right. Content scrolls inside `main`; the pane slot sits to its
right. Lists are full-width inside a `max-w-[1280px]` container with 24px side padding.

## Reference mockups

The approved mockups (static HTML) are the source of truth for look and spacing:
`Main.dc.html` (Command Center), `PullRequests.dc.html` (list + PR pane with the Files tab),
`Pipelines.dc.html` (grouped list + run pane), `Tokens.dc.html` (tokens, status set, buttons,
type scale). They live outside the repo; the implementer brief gives their path.
