---
title: Documentation
---

# forgetop

A fast, keyboard-driven command center for your **pull requests, work items, and CI
pipelines** — across GitHub, GitLab, Azure DevOps, Bitbucket, Linear, and Jira — in
one place. Triage and *act* on everything without tab-hopping between forges, as a
terminal UI **and** a web dashboard.

This is the full reference. For install and a 60-second start, see the
[README](https://github.com/magna-nz/forgetop#readme).

- [What forgetop does](#what-forgetop-does)
- [Core concepts](#core-concepts)
- [Getting started](#getting-started)
- [Web dashboard](#web-dashboard)
- [Launchpad](#launchpad)
- [Pull Requests](#pull-requests)
- [Work Items](#work-items)
- [Pipelines](#pipelines)
- [Filtering and sorting](#filtering-and-sorting)
- [Command palette](#command-palette)
- [Notification inbox](#notification-inbox)
- [Saved views](#saved-views)
- [Notifications](#notifications)
- [Keybindings](#keybindings)
- [Configuration](#configuration)
- [Themes](#themes)
- [How it works (architecture)](#how-it-works-architecture)
- [Development](#development)

## What forgetop does

- **The Launchpad** — the default landing screen: one cross-provider **action inbox**
  that triages every PR, work item, and pipeline into "what needs you", so you start
  on your queue instead of scrolling three separate lists.
- **Terminal *or* browser** — the same app as a keyboard-driven TUI *and* a local
  **web dashboard**, opened together by default. Everything below works in both.
- **Three sections in one dashboard** — Pull Requests, Work Items, and Pipelines,
  each a tab you can act on.
- **Six forges** — GitHub, GitLab, Azure DevOps, Bitbucket, Linear, and Jira, plus
  a built-in `--demo`.
- **Cross-provider aggregation** — bind several connections to a section and their
  items merge into one list, tagged by provider. All your PRs across GitHub *and*
  GitLab *and* Azure, in a single view.
- **Do work, not just watch it** — approve / request changes / merge / comment on
  PRs, change work-item states, trigger pipeline runs — all from the keyboard.
- **Real code review** — a full-screen PR view with Conversation, Commits, Checks,
  and Diff tabs; **syntax-highlighted** diffs grouped by directory, per-file
  **viewed** checkboxes, thread jump-navigation, a line cursor in the patch; and
  **inline line comments buffered locally then submitted as one review**
  (Comment / Approve / Request changes).
- **Pipeline drill-in** — expand stages → jobs → steps with per-node **durations**,
  **failure reasons**, a scrollable **logs** pane, and open-in-browser.
- **Pipeline approvals** — see which runs are blocked on a gate you can action
  (red **Approval needed**), and approve / reject them without leaving the terminal
  (GitHub, Azure DevOps, GitLab).
- **Filter, sort, and shape** — a live quick-filter, per-column sorting, work-item
  state visibility, and pipeline subscriptions — all remembered per view.
- **Command palette (`Ctrl-P`)** — fuzzy-jump to any PR, work item, or pipeline across
  every provider from one search box, without scrolling or switching tabs.
- **Notification inbox (`i`)** — your cross-provider notification stream (reviews, mentions,
  CI failures, assignments) in one list, with a top-left unread indicator; `Enter` drills
  straight into the item. GitHub, GitLab, and Linear.
- **Saved views** — bundle a filter + sort + state into a named view and flip
  between them from an always-visible view bar (`[` / `]`).
- **Desktop notifications** — get pinged when a pipeline fails, a review is
  requested, or your PR is approved / gets changes requested — across every
  connected provider.
- **Secure by default** — tokens live in your OS keychain; the config file only
  ever holds a reference.

## Core concepts

### Sections and bindings

forgetop has three **sections**: Pull Requests, Work Items, and Pipelines. Each is
bound to one or more **connections** (a configured account on a forge). A section
only shows data from the connections bound to it. You manage connections and
bindings in the [web dashboard](#web-dashboard) — press `C` in the TUI to open it.

### Providers and capabilities

Each provider advertises **capabilities** — which sections it can serve. A
connection only ever offers what it can actually do, so the UI never dangles a
dead option:

| Provider | Pull Requests | Work Items | Pipelines |
| --- | :---: | :---: | :---: |
| GitHub | yes | yes (Issues) | yes (Actions) |
| GitLab | yes (MRs) | yes (Issues) | yes (CI) |
| Azure DevOps | yes | yes | yes (Builds) |
| Bitbucket | yes | – | yes (Pipelines) |
| Linear | – | yes | – |
| Jira | – | yes | – |

### Cross-provider aggregation

Every section aggregates. Bind two or more connections to Pull Requests (or Work
Items, or Pipelines) and their items merge into a single list with a **Provider**
column so you can tell them apart. Filtering, sorting, and actions all work across
the combined list — and each action targets the row's *own* provider, so
approving a GitLab MR and merging a GitHub PR just work from the same screen.

Try it live: `forgetop --demo` seeds two demo connections so you can see the
merged lists immediately.

### Security

Tokens are written to the OS keychain (macOS Keychain, Windows Credential Manager,
Linux Secret Service). The JSON config only stores a *reference* to each token —
never the secret itself. For headless use you can supply a token via an
environment variable instead (see [Configuration](#configuration)).

The [web dashboard](#web-dashboard) is served on **`127.0.0.1` only**, gated by a
**per-session token** carried in its URL. Because its API can act on your behalf
(merge a PR, change a state), it never listens off-loopback and never accepts
cross-origin requests; the token is never returned by the API, written to config,
or logged.

## Getting started

Try it with no setup — everything is in memory, nothing is written:

```sh
forgetop --demo
```

Then run it for real:

```sh
forgetop
```

This opens the **terminal UI and the web dashboard together** (the default). On
first launch — before any connection has a token — the dashboard drops into a quick
setup: pick a provider, paste a token, choose which sections it feeds. Tokens go to
your OS keychain and are shared straight back to the terminal. Press **`C`** in the
TUI (or open the dashboard's **Settings**) to manage connections any time, and
**`?`** shows every keybinding. See [Web dashboard](#web-dashboard) for the run modes.

### Diagnosing setup

If a connection isn't working, run the diagnostic — it checks the config location,
keychain access, and each connection's token + connectivity, without opening the UI:

```sh
forgetop doctor
```

It prints a line per connection (`✓` healthy, `⚠` no token, `✗` token present but
auth failed), so you can see at a glance whether it's a missing token, a bad scope,
or an expired credential.

### Command-line options

| Command | What it does |
| --- | --- |
| `forgetop` | Launch — the terminal UI and the web dashboard together (default) |
| `forgetop --dashboard` | Serve **only** the web dashboard (headless — no TUI) |
| `forgetop --demo` (`-d`) | Launch with built-in demo data (no setup) |
| `forgetop doctor` | Diagnose config, keychain access, and connection health |
| `forgetop --version` (`-V`) | Print the version and exit |
| `forgetop --help` (`-h`) | Show usage and exit |

Set **`FORGETOP_STARTUP`** = `both` / `terminal_only` / `dashboard_only` to override
what launches for a single run (handy with `--demo`, whose config is in-memory).

## Web dashboard

forgetop ships the **same app as a local web dashboard** — the Launchpad, all three
lists, PR review with an inline-comment diff viewer, the command palette, sort/filter,
themes, and every write action (approve, merge, comment, submit a review, change a
work-item state, approve a pipeline gate, retry a run, mark a notification read). It's
a React app **built into the binary** and served by forgetop itself — no separate
install, no external network.

The terminal UI and the dashboard are two frontends over one core, so they show the
**same data**, act through the **same providers**, and share the **same config +
keychain** — a connection or a setting changed in one shows up in the other.

### Opening it

- **From the TUI:** press **`B`** (shown in the footer and `?` help on every screen).
- **Headless:** `forgetop --dashboard` serves it and opens your browser — no TTY, so
  it's handy over SSH with a forwarded port.
- By default, running `forgetop` opens **both** at once.

### When forgetop starts

A shared preference decides what launches — set it in the dashboard under **Settings →
When forgetop starts**, or press **`,`** in the TUI:

| Option | Behaviour |
| --- | --- |
| **Dashboard + terminal** (default) | Opens both together |
| **Terminal only** | Just the TUI (the server still runs in the background, so `B` works) |
| **Dashboard only** | Just the browser dashboard — same as `forgetop --dashboard` |

The choice is stored in your config and shared between the two. For a one-off override
(without changing the saved setting) use the `FORGETOP_STARTUP` env var above.

### Connections & settings

Connection setup lives in the dashboard's **Settings** page: add a provider from a
form tailored to it, paste a token (stored in the OS keychain), tick which sections it
feeds, and **Test** / **Edit** / **Delete** it. First launch with nothing set up opens
this automatically. Pressing **`C`** in the TUI jumps straight here.

### Security

The server binds **`127.0.0.1` only** and gates its API with a **per-session token** in
the URL — see [Security](#security).

## Launchpad

The **Launchpad** is the screen forgetop opens on (tab `1`). Instead of browsing by
type, it answers one question — *what needs me right now?* — by pulling every PR,
work item, and pipeline across every connected provider into a single, prioritised
page. The Pull Requests / Work Items / Pipelines tabs are still there as drill-downs
to their right.

It's split into **two columns**:

- **Needs you** (left) — things ripe for action now, in urgency order:
  - **Needs your review** — PRs where you're a requested reviewer.
  - **Approvals waiting** — pipeline runs blocked on a gate you can approve.
  - **Ready to merge** — your PRs that are approved, mergeable, and green.
  - **Needs fixing** — your PRs with changes requested / failing checks / conflicts,
    **and** failed pipeline runs.
- **Your work** (right) — your own things:
  - **Assigned to you** — work items assigned to you.
  - **Your open pull requests** — the full list of your open authored PRs (a PR that's
    also an action item on the left still appears here; this is the complete list).
  - **Recently merged** — your PRs merged in the last few days.

Every row is laid out on one aligned grid so PRs, pipelines (`CI`), and work items
(`WI`) read as siblings: a coloured **type badge**, a **status** signal (a PR shows
its review state — `✓ ok`, `○ review`, `✗ changes`, `◌ draft`; a pipeline its run
status; a work item its state), a short **#ref**, the **title**, one type-specific
**detail** (PR `+/-`, pipeline branch, work-item type), the **provider**, and the
**age** (which reddens once it's stale). For pipelines the title is the **pipeline
name** (e.g. `CI Build`) and the ref is the **run/release** (e.g. `10.1.100`).

**Getting around:**

- `↑` / `↓` (or `k` / `j`) — move within the focused column.
- `←` / `→` (or `h` / `l`) — switch between the two columns.
- `Enter` — open the selected item in its **full view** (the same PR / work-item /
  pipeline view as from the section tabs, with all its actions).
- `Esc` — from an item opened here, return to the Launchpad with the **same row still
  selected**.
- `Tab`, `1`–`4` — switch top-level tabs. `r` — refresh.

Two touches make it feel live: when you **act** on an item — submit a review, approve,
or merge a PR — it drops off the Launchpad immediately rather than lingering until the
next poll; and the **selected row's title scrolls** horizontally when it's too long to
fit, so you can always read it in full. The `Launchpad (N)` tab badge counts the items
that actually need you (the reference sections don't inflate it).

## Pull Requests

The Pull Requests tab is **browse-and-open**: the list is for finding a PR; every
write action lives inside the PR view.

**On the list:**

- `Enter` — open the full-screen PR view.
- `f` — **filter by status**: a checklist of Open / Draft / Merged / Closed; tick which
  to show. Defaults to Open + Draft; ticking Merged or Closed transparently widens the
  fetch to include completed PRs, so you can e.g. keep the **Mine** view but see your
  open *and* recently-merged PRs together. The list title shows the active set
  (`Pull Requests · Open, Merged`). Session-only.
- `[` / `]` — switch [saved views](#saved-views); PRs default to All / Mine / Review-requested.
- `/` — quick-filter by typing; `S` — sort by a column; `o` — open in browser.

**Inside the PR view** (four sub-tabs, switch with `←`/`→`):

- **Conversation** — description, reviewers, labels, and comment threads.
- **Commits** — one row per commit; `Enter` drills into *that commit's* diff.
- **Checks** — each named CI check with its status.
- **Diff** — the changed files, **grouped by directory** and **syntax-highlighted**;
  `Enter` on a file drops into a **line cursor** in the patch (`↑`/`↓` move
  line-by-line; the title shows the real file line).

Write actions from the view: `a` approve, `x` request changes, `m` merge (pick a
strategy), `c` comment, `o` open in browser.

### Reading the diff

- **Syntax highlighting** — patches are highlighted for common languages
  (Rust, TS/JS, Python, Go, Java, JSON, YAML), using the theme's colours; other
  languages render plain. The `+`/`-` add/remove colour is always kept.
- **Viewed checkboxes** — press **`v`** on a file to mark it reviewed (`[x]`); the
  file-list title tracks your progress as **`N/M reviewed`**. Session-only.
- **Directory grouping** — files are grouped under directory headers so a large PR
  is easier to scan.
- **Inline comment threads** — existing review comments render **inline in the diff,
  beneath the line they're on** (a left bar: accent = open, dim = resolved), so you read
  them in context instead of a side list. **`[`** / **`]`** jump the cursor between them.
  (Unanchored / PR-level comments stay on the Conversation tab.)

### Reviewing code with line comments

In the Diff tab's line cursor, press `c` on a code line to write an inline
comment. Comments are **buffered locally** — the line gets a `▎` marker — so you
can comment on several lines first. Press **`s`** to submit them all as one
review, choosing the verdict: **Comment**, **Approve**, or **Request changes**.
If you press `Esc` **or `q`** to leave with comments still buffered, forgetop asks whether
to **submit** or **leave without submitting** first, so you don't lose them by accident.

- **GitHub** posts it as a single native review.
- **GitLab** posts positioned discussions (and approves if you chose Approve).
- **Azure DevOps / Bitbucket** don't expose inline patches, so the line cursor —
  and line comments — aren't available there.

## Work Items

The Work Items tab shows only items **assigned to you** — each provider resolves
the current user from your token (`@me`, `currentUser()`, `isMe`, …), so there's
nothing to configure.

The list is browse-and-open; the write actions live in the item view:

- `Enter` — open the item.
- `f` (on the list) — a checklist of the **states currently in view**; pick which
  to show. Built from the loaded items, so it's always provider-accurate
  (`In Progress` vs `Inprogress`). Persisted.

Inside the item view (after `Enter`):

- `u` — **update state.** The choices are pulled from the provider itself: Jira's
  workflow transitions, Linear's team states, Azure's work-item-type states, or
  open/closed for GitHub/GitLab issues. (Falls back to the states seen across the
  list if a provider can't report them.)
- `c` — comment (works on every work-item provider).
- `o` — open in browser.

## Pipelines

The list shows a **Pipeline** column — the pipeline (definition) name, e.g. `CI Build`
or `CD (Release)` — separate from the **Run** column, which is the run's own name or
release (e.g. `10.1.100`, or its number when it has none). Sort by `Pipeline` to group
runs by their definition.

- `Enter` — drill into a run: a collapsible **stages → jobs → steps** tree.
- `T` — trigger a run.

Inside the drill-in each node shows its **duration**, and failed jobs show a short
**failure reason** (GitLab's reason, Azure's error/warning counts). Then:

- `Enter` — expand / collapse the selected node.
- `L` — open a scrollable **logs** pane for the selected job (`Esc` closes).
- `A` — approve / reject a waiting gate (see below).
- `o` — open the selected job in the browser.

For a connection that discovers many pipelines, open the config screen and press
`s` on it to **subscribe** to just the definitions you care about.

### Approvals

When a run is blocked on a manual gate — a **GitHub** environment required-reviewer,
an **Azure DevOps** approval check, or a **GitLab** manual job — forgetop surfaces it:

- The Pipelines list shows a red **Approval needed** column on that run, refreshed
  on the normal 30-second poll (and you can opt into a desktop
  [notification](#notifications) when a gate first appears).
- Inside the run, a banner reads **⏸ Approval needed: {environment}**. On providers
  where forgetop can act (**GitHub**, **GitLab**) press `A` to open a picker of
  Approve / Reject options per gate, then confirm.

Acting is **capability-scoped**:
- **GitHub / GitLab** — approve or reject in-app (on GitLab a reject cancels the
  manual job).
- **Azure DevOps** — **view-only**: the pending gate is surfaced, but Azure doesn't
  expose the environment check as an actionable approval over the API, so there's no
  `A` action — approve it in the Azure UI.
- **Bitbucket** — approvals aren't surfaced at all (its API can't resume a paused
  manual step); the run shows an explicit *"not supported"* note.

## Filtering and sorting

Four complementary ways to cut a busy view down:

- **Quick-filter (`/`)** — on any list, type to filter rows live; every
  whitespace-separated token must match (case-insensitive) across the row's key
  fields. `Enter` applies, `Esc` clears. Remembered per tab.
- **Sort (`S`)** — pick a column to sort by; re-pick to flip direction. The sorted
  column shows a `▲`/`▼` arrow. Persisted per view.
- **PR status (`f` on Pull Requests)** — show only chosen statuses (Open / Draft /
  Merged / Closed); see [Pull Requests](#pull-requests).
- **Work-item state visibility (`f` on Work Items)** — show only chosen states.
- **Pipeline subscriptions (`s` in the config screen)** — track only chosen
  pipeline definitions per connection.

All of these compose, and all work across the aggregated (multi-provider) list.

## Command palette

Press **`Ctrl-P`** (from the Launchpad or any list) to open the **command palette** — a
fuzzy jump across *everything already loaded*: every PR, work item, and pipeline, from
every connected provider, in one list. Type to filter (matching the title, author, branch,
identifier, or connection), `↑`/`↓` (or `Ctrl-n`/`Ctrl-p`) to move, `Enter` to open the
item's full view, `Esc` to dismiss. Each row carries a status dot in the usual
green/blue/red/grey colours, so you can scan state as you jump. It searches only what's
loaded — no network round-trip — so it's instant.

## Notification inbox

Press **`i`** (from the Launchpad or any list) to open the **Inbox** — your notification
*stream* aggregated across providers: review requests, @mentions, assignments, CI failures,
comments, and state changes, newest first. It answers *what just happened*, where the
Launchpad answers *what needs me now*.

A **`Notifications (N) [i]`** nav item sits at the **far right** of the tab bar on every
screen — dim grey when you're at zero, **bold yellow** with the count when there's something
waiting, and highlighted (accent) while the Inbox is open. It's a nav item, not part of the
`Tab` cycle (which stays on Launchpad → Pull Requests → Work Items → Pipelines).

- `↑`/`↓` move · **`Enter`** opens the item **in-app** (drills straight into that PR / work
  item) · `o` opens it in the browser · `x` marks the row read · `A` marks everything read ·
  `r` refreshes · `Esc` back. Opening or marking updates the count immediately.
- Each row shows a kind glyph in the status colours (CI failures red, reviews/assignments
  accent, mentions magenta), an unread dot, the title, the repo/project, the connection, and
  age. It refreshes on the 30s poll like everything else.

**Provider support.** The inbox is fed by the providers that expose a personal notification
feed:

| Provider | Notification inbox |
| --- | :---: |
| **GitHub** | ✅ (notifications) |
| **GitLab** | ✅ (todos) |
| **Linear** | ✅ (notifications) |
| **Azure DevOps** | — no personal feed |
| **Bitbucket** | — no personal feed |
| **Jira** | — no personal feed |

Azure DevOps, Bitbucket, and Jira don't offer a personal notification feed in their APIs, so
connections for those providers simply don't contribute to the inbox. (This is separate from
[desktop notifications](#notifications), which is about which events fire an OS ping.)

## Saved views

A **saved view** is a named bundle of a section's shaping — its base filter, the
quick-filter text, the sort, and (on Work Items) which states are hidden. Views
live in a horizontal **view bar** above the list, gh-dash style, with the active
one highlighted. The bar appears once a section has more than one view.

Every section starts with sensible defaults: Pull Requests get **All**, **Mine**,
and **Review**; Work Items and Pipelines get a single **All**.

- `[` / `]` — switch to the previous / next view.
- `V` — save the current filter + sort + states as a new view (you're prompted for
  a name), then switch to it.
- `X` — delete the current view (with a confirm; you can't delete the last one).

Switching a view re-applies the whole bundle at once, so a `Mine` view can pin a
different sort and quick-filter than `Review`. Views are persisted per section, so
they're waiting next time — except in `--demo`, where the in-memory config means
saved views last only for that session.

## Notifications

forgetop raises native desktop notifications on the events you choose:

- **Pipeline failed** — a run transitions into a failed state.
- **Pipeline approval needed** — a run first starts waiting on a gate you can
  approve.
- **Review requested** — you're newly a requested reviewer on a PR.
- **Your PR approved** / **changes requested** — a reviewer votes on a PR you
  authored.

Press **`N`** anywhere to open a checklist and opt in/out of each event; the
choice is persisted. Detection is seeded on load (no startup spam), de-duped, and
re-seeded when you change settings — and enabling fires one confirmation notification so you can
verify it works on your machine. Every event spans **all bound providers**, not
just the first.

## Keybindings

forgetop shows a **context-aware key glossary** along the bottom — it only lists
the keys valid for where you are. Press `?` for the full panel. The complete set:

### Global

| Key | Action |
| --- | --- |
| `←` / `→`, `h` / `l`, `Tab`, `1`–`4` | Switch tab (`1` = Launchpad) |
| `↑` / `↓`, `k` / `j` | Move selection |
| `Ctrl-P` | Command palette — fuzzy-jump to any PR, work item, or pipeline |
| `i` | Notification inbox (review requests, mentions, CI failures, assignments) |
| `/` | Quick-filter the current list |
| `S` | Sort by a column (re-pick flips direction) |
| `o` | Open selected item in browser |
| `B` | Open the web dashboard in your browser |
| `,` | Settings — what forgetop opens on launch |
| `v` | Choose which tabs are visible |
| `C` | Open the connections page (in the web dashboard) |
| `r` | Refresh · `t` cycle theme |
| `N` | Notifications — choose which events ping you |
| `?` | Show all keybindings (anywhere) |
| `q` / `Ctrl-C` | Quit · `Esc` back / close |

### Launchpad

| Key | Action |
| --- | --- |
| `↑` / `↓`, `k` / `j` | Move within the focused column |
| `←` / `→`, `h` / `l` | Switch between the two columns |
| `Enter` | Open the selected item in its full view |
| `Esc` (in the opened item) | Return to the Launchpad, same row selected |

### Saved views

| Key | Action |
| --- | --- |
| `[` / `]` | Previous / next saved view |
| `V` | Save the current filter + sort + states as a view |
| `X` | Delete the current view |

### Pull Requests (list)

| Key | Action |
| --- | --- |
| `Enter` | Open the PR view |
| `f` | Filter by status (Open / Draft / Merged / Closed) |
| `[` / `]` | Switch saved views (defaults All / Mine / Review-requested) |

### Inside the PR view

| Key | Action |
| --- | --- |
| `←` / `→` | Switch sub-tab |
| `a` / `x` | Approve / request changes |
| `m` | Merge (choose strategy) |
| `c` | Comment (inline on a diff line, otherwise the PR) |
| `Enter` (Commits) | Drill into that commit's diff |
| `Enter` (Diff, on a file) | Line cursor within the patch |
| `s` | Submit buffered line comments as one review |
| `Esc` | Step back (line cursor → file list → close) |

### Work Items

| Key | Action |
| --- | --- |
| `Enter` | Open the item |
| `f` | Choose which states to show |
| `u` (in the item view) | Update state (choices pulled from the provider) |
| `c` (in the item view) | Comment |

### Pipelines

| Key | Action |
| --- | --- |
| `Enter` | Drill in (stages → jobs → steps) |
| `Enter` (in drill-in) | Expand / collapse a node |
| `L` | View the selected job's logs |
| `A` (in drill-in) | Approve / reject a waiting gate (GitHub, GitLab; Azure view-only) |
| `o` | Open the selected job in the browser |
| `T` | Trigger a run |

### Connections (terminal fallback)

Connection management lives in the [web dashboard](#web-dashboard) — `C` opens it and
also shows a connections list in the terminal. That list still supports the original
keys as a fallback:

| Key | Action |
| --- | --- |
| `a` | Add a connection |
| `p` / `w` | Bind Pull Requests / Work Items (multi-select) |
| `s` | Pipeline subscriptions |
| `x` | Remove connection |

## Configuration

Config is a small JSON file, created and managed for you:

| OS | Path |
| --- | --- |
| macOS | `~/Library/Application Support/forgetop/config.json` |
| Linux | `~/.config/forgetop/config.json` |
| Windows | `%APPDATA%\forgetop\config.json` |

**It never contains secrets** — only a reference to each token in the keychain,
plus your bindings and view preferences (saved views, sorts, hidden states,
notification choices).

### Tokens

You add connections and paste tokens in the dashboard's **Settings** page (see
[Web dashboard](#web-dashboard)); each token is stored in your OS keychain under the
service name `forgetop`. In headless environments (CI, containers) you can instead
supply a token via an environment variable named `FORGETOP_PAT_<CONNECTION_ID>`
(uppercased; non-alphanumeric characters become `_`).

### Logs & diagnostics

forgetop keeps a small log file next to your config — **`forgetop.log`** in the same
directory (e.g. `~/.config/forgetop/forgetop.log`). It records:

- **Crashes** — if forgetop ever panics it restores your terminal (no garbled screen),
  writes the panic to the log, and prints the path so you can send it on.
- **Errors** — a failed action (approve / merge / comment / trigger / …) and background
  provider / auth / network fetch failures, which otherwise only flash on screen — each
  timestamped, so intermittent issues are reviewable after the fact.

`forgetop doctor` prints the log's path. Logging is best-effort and contains no secrets.
The running version is shown in the header (`▟ forgetop v…`) and via `forgetop --version`,
so it's easy to include when reporting an issue.

### Token scopes

| Provider | What to create | Scopes |
| --- | --- | --- |
| GitHub | Personal access token | `repo` (PRs, issues, checks); `workflow` / Actions read for pipelines |
| GitLab | Personal access token (Settings → Access Tokens) | `api` (merge requests, issues, pipelines) |
| Azure DevOps | Personal access token | Code Read, Work Items Read/Write, Build Read/Execute |
| Linear | Personal API key (Settings → Security and access → API) | default |
| Jira | API token (id.atlassian.com) + your account email | default (account access) |
| Bitbucket | App password (Personal settings → App passwords) + your username | Pull requests, Pipelines (read/write) |

## Themes

Four built-in themes — `slate` (default), `dark`, `light`, and `matrix`. In the
terminal, cycle with `t` (256-colour palettes, so they render correctly on every
terminal, including ones without truecolor); in the [web dashboard](#web-dashboard),
use the theme toggle in the sidebar footer. Each side remembers your choice (the
dashboard's per browser).

## How it works (architecture)

forgetop is a Rust [Cargo workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html):

| Crate | Responsibility |
| --- | --- |
| `forgetop-core` | Domain model, capability-scoped provider traits, config, secrets, services |
| `forgetop-providers` | GitHub, GitLab, Azure DevOps, Linear, Jira, Bitbucket, and Demo implementations |
| `forgetop-tui` | The [ratatui](https://ratatui.rs) terminal UI |
| `forgetop-server` | The web dashboard — an [axum](https://github.com/tokio-rs/axum) server + an embedded React SPA, over the same services |
| `forgetop-cli` | The `forgetop` binary |

A few design ideas hold it together:

- **Capability-scoped traits.** Providers implement `PullRequestSource`,
  `WorkItemSource`, and/or `PipelineSource` — only what they support. The core
  never assumes a provider can do something it can't, which is how a Linear
  connection appears for Work Items but not Pull Requests.
- **Sections resolve to feeds.** A section binds to a set of connections; the
  service resolves each to a live source (a "feed"). Aggregation is just iterating
  every feed and tagging each item with its connection — the same shape for PRs,
  Work Items, and Pipelines.
- **Config never holds secrets.** The config service persists bindings and
  preferences and stores only a `credential_ref` per connection; the actual token
  lives in the OS keychain via a separate secret store.
- **Immediate-mode UI, own input loop.** The TUI owns a dedicated input reader
  thread feeding a `tokio` loop, and redraws the whole frame each tick — so there
  are no framework focus fights, and every keystroke is dispatched explicitly.

## Development

```sh
cargo test        # run the test suite
cargo clippy      # lint
cargo run -- --demo
```

Source: [github.com/magna-nz/forgetop](https://github.com/magna-nz/forgetop).
