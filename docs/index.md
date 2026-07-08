---
title: Documentation
---

# forgetop

A fast, keyboard-driven terminal UI for your **pull requests, work items, and CI
pipelines** — across GitHub, GitLab, Azure DevOps, Bitbucket, Linear, and Jira —
in one place. Triage and *act* on everything without leaving the terminal, and
without tab-hopping between forges.

This is the full reference. For install and a 60-second start, see the
[README](https://github.com/magna-nz/forgetop#readme).

- [What forgetop does](#what-forgetop-does)
- [Core concepts](#core-concepts)
- [Getting started](#getting-started)
- [Pull Requests](#pull-requests)
- [Work Items](#work-items)
- [Pipelines](#pipelines)
- [Filtering and sorting](#filtering-and-sorting)
- [Saved views](#saved-views)
- [Notifications](#notifications)
- [Keybindings](#keybindings)
- [Configuration](#configuration)
- [Themes](#themes)
- [How it works (architecture)](#how-it-works-architecture)
- [Development](#development)

## What forgetop does

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
  and Diff tabs; a line cursor in the patch; and **inline line comments buffered
  locally then submitted as one review** (Comment / Approve / Request changes).
- **Pipeline drill-in** — expand stages → jobs → steps with per-node **durations**,
  **failure reasons**, a scrollable **logs** pane, and open-in-browser.
- **Pipeline approvals** — see which runs are blocked on a gate you can action
  (red **Approval needed**), and approve / reject them without leaving the terminal
  (GitHub, Azure DevOps, GitLab).
- **Filter, sort, and shape** — a live quick-filter, per-column sorting, work-item
  state visibility, and pipeline subscriptions — all remembered per view.
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
only shows data from the connections bound to it. You manage bindings on the
config screen (`C`).

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

## Getting started

Try it with no setup — everything is in memory, nothing is written:

```sh
forgetop --demo
```

Then run it for real. On first launch (no connections) forgetop drops straight
into the **add-connection wizard**, which walks you through picking a provider,
entering details, pasting a token, and binding it to a section. It then offers a
notifications chooser.

```sh
forgetop
```

Press **`C`** any time to open the config screen and manage connections and
bindings; **`n`** starts the wizard again; **`?`** shows every keybinding.

### Diagnosing setup

If a connection isn't working, run the diagnostic — it checks the config location,
keychain access, and each connection's token + connectivity, without opening the UI:

```sh
forgetop doctor
```

It prints a line per connection (`✓` healthy, `⚠` no token, `✗` token present but
auth failed), so you can see at a glance whether it's a missing token, a bad scope,
or an expired credential.

## Pull Requests

The Pull Requests tab is **browse-and-open**: the list is for finding a PR; every
write action lives inside the PR view.

**On the list:**

- `Enter` — open the full-screen PR view.
- `f`, `[` / `]` — switch [saved views](#saved-views); PRs default to
  All / Mine / Review-requested.
- `/` — quick-filter by typing; `S` — sort by a column; `o` — open in browser.

**Inside the PR view** (four sub-tabs, switch with `←`/`→`):

- **Conversation** — description, reviewers, labels, and comment threads.
- **Commits** — one row per commit; `Enter` drills into *that commit's* diff.
- **Checks** — each named CI check with its status.
- **Diff** — the changed files; `Enter` on a file drops into a **line cursor** in
  the patch (`↑`/`↓` move line-by-line; the title shows the real file line).

Write actions from the view: `a` approve, `x` request changes, `m` merge (pick a
strategy), `c` comment, `o` open in browser.

### Reviewing code with line comments

In the Diff tab's line cursor, press `c` on a code line to write an inline
comment. Comments are **buffered locally** — the line gets a `▎` marker — so you
can comment on several lines first. Press **`s`** to submit them all as one
review, choosing the verdict: **Comment**, **Approve**, or **Request changes**.

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
- **Work-item state visibility (`f` on Work Items)** — show only chosen states.
- **Pipeline subscriptions (`s` in the config screen)** — track only chosen
  pipeline definitions per connection.

All of these compose, and all work across the aggregated (multi-provider) list.

## Saved views

A **saved view** is a named bundle of a section's shaping — its base filter, the
quick-filter text, the sort, and (on Work Items) which states are hidden. Views
live in a horizontal **view bar** above the list, gh-dash style, with the active
one highlighted. The bar appears once a section has more than one view.

Every section starts with sensible defaults: Pull Requests get **All**, **Mine**,
and **Review**; Work Items and Pipelines get a single **All**.

- `[` / `]` — switch to the previous / next view. On Pull Requests, `f` also
  advances to the next view.
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
choice is persisted, and the first-run wizard asks after your first connection.
Detection is seeded on load (no startup spam), de-duped, and re-seeded when you
change settings — and enabling fires one confirmation notification so you can
verify it works on your machine. Every event spans **all bound providers**, not
just the first.

## Keybindings

forgetop shows a **context-aware key glossary** along the bottom — it only lists
the keys valid for where you are. Press `?` for the full panel. The complete set:

### Global

| Key | Action |
| --- | --- |
| `←` / `→`, `h` / `l`, `Tab`, `1`–`3` | Switch tab |
| `↑` / `↓`, `k` / `j` | Move selection |
| `/` | Quick-filter the current list |
| `S` | Sort by a column (re-pick flips direction) |
| `o` | Open selected item in browser |
| `n` | Add a connection (wizard) |
| `v` | Choose which tabs are visible |
| `C` | Config / connections screen |
| `r` | Refresh · `t` cycle theme |
| `N` | Notifications — choose which events ping you |
| `?` | Show all keybindings (anywhere) |
| `q` / `Ctrl-C` | Quit · `Esc` back / close |

### Saved views

| Key | Action |
| --- | --- |
| `[` / `]` | Previous / next saved view |
| `f` (Pull Requests) | Next saved view |
| `V` | Save the current filter + sort + states as a view |
| `X` | Delete the current view |

### Pull Requests (list)

| Key | Action |
| --- | --- |
| `Enter` | Open the PR view |
| `f`, `[` / `]` | Switch saved views (defaults All / Mine / Review-requested) |

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

### Config / connections

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

Tokens are stored in your OS keychain under the service name `forgetop`. In
headless environments (CI, containers) you can instead supply a token via an
environment variable named `FORGETOP_PAT_<CONNECTION_ID>` (uppercased;
non-alphanumeric characters become `_`).

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

Cycle with `t`. Four built-in themes — `slate` (default), `dark`, `light`, and
`matrix` — using 256-colour palettes so they render correctly on every terminal
(including ones without truecolor). Your choice is remembered.

## How it works (architecture)

forgetop is a Rust [Cargo workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html):

| Crate | Responsibility |
| --- | --- |
| `forgetop-core` | Domain model, capability-scoped provider traits, config, secrets, services |
| `forgetop-providers` | GitHub, GitLab, Azure DevOps, Linear, Jira, Bitbucket, and Demo implementations |
| `forgetop-tui` | The [ratatui](https://ratatui.rs) terminal UI |
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
