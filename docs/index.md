---
title: Documentation
---

# forgetop documentation

forgetop is a fast, keyboard-driven terminal UI for pull requests, work items, and
CI pipelines across GitHub, GitLab, Azure DevOps, Linear, Jira, and Bitbucket.

For install and a quick start, see the [README](https://github.com/magna-nz/forgetop#readme).
This page is the full reference.

- [Keybindings](#keybindings)
- [Filtering](#filtering)
- [Configuration](#configuration)
- [Themes](#themes)
- [How it works](#how-it-works)
- [Development](#development)

## Keybindings

forgetop shows a context-aware key glossary along the bottom - it only lists the
keys valid for wherever you are. The full set:

### Global

| Key | Action |
| --- | --- |
| `←` / `→`, `h` / `l`, `Tab`, `1`–`3` | Switch tab |
| `↑` / `↓`, `k` / `j` | Move selection |
| `/` | Quick-filter the current list (type to narrow, `Esc` clears) |
| `S` | Sort the current list by a column (re-pick the column to flip direction) |
| `o` | Open selected item in browser |
| `n` | Add a connection (wizard) |
| `v` | Choose which tabs are visible |
| `C` | Config / connections screen |
| `r` | Refresh · `t` cycle theme |
| `N` | Toggle desktop notifications |
| `?` | Show all keybindings (works anywhere) |
| `q` / `Ctrl-C` | Quit · `Esc` back / close |

### Pull Requests (list)

The list is browse-and-open; every write action lives inside the PR view.

| Key | Action |
| --- | --- |
| `Enter` | Open the full-screen PR view (Conversation / Commits / Checks / Diff) |
| `f` | Cycle filter (All / Mine / Review-requested) |

### Inside the PR view (after `Enter`)

| Key | Action |
| --- | --- |
| `←` / `→` | Switch sub-tab |
| `a` / `x` | Approve / request changes |
| `m` | Merge (choose strategy) |
| `c` | Comment (inline on a diff line, otherwise on the PR) |
| `Enter` (Commits) | Drill into that commit's diff |
| `Enter` (Diff, on a file) | Drop into a line cursor within the patch |
| `↑` / `↓` (line cursor) | Move line-by-line; the title shows the file line |
| `s` | Submit the buffered comments as one review (Comment / Approve / Request changes) |
| `Esc` | Step back (line cursor → file list → close) |

### Work Items

The Work Items list only shows items **assigned to you** — each provider resolves
the current user from your token (`@me`, `currentUser()`, `isMe`, …).

| Key | Action |
| --- | --- |
| `Enter` | Expand details |
| `s` | Change state |
| `f` | Choose which states to show (checklist, built from the states in view) |
| `c` | Comment |

### Pipelines

| Key | Action |
| --- | --- |
| `Enter` | Drill in (stages → jobs → steps) |
| `T` | Trigger a run |

The drill-in shows each node's **duration** and a short **failure reason** on
failed jobs (provider-specific: GitLab's failure reason, Azure's error/warning
counts). Inside the drill-in:

| Key | Action |
| --- | --- |
| `Enter` | Expand / collapse the selected node |
| `L` | View the selected job's logs (scrollable pane; `Esc` closes) |
| `o` | Open the selected job in the browser |
| `T` | Trigger a run |

## Filtering

forgetop has three complementary ways to cut a busy list down to what you care
about:

- **Quick-filter (`/`)** - available on every list. Type to filter rows live;
  every whitespace-separated token must match (case-insensitive) across the row's
  key fields (title, author, number, branch, labels/state/provider). `Enter`
  applies, `Esc` clears. The filter is remembered per tab.
- **Work-item state visibility (`f` on Work Items)** - opens a checklist of the
  distinct states currently in view (built from the loaded items, so it is always
  provider-accurate). Tick the states you want to see; the choice persists.
- **Pipeline subscriptions (`s` in the config screen)** - for a connection that
  discovers many pipelines, pick just the definitions you want to track. Persisted
  per connection.

## Sorting

Press `S` on any list (Pull Requests, Work Items, Pipelines) to pick a sort
column. Re-pick the same column to flip between ascending and descending; the
sorted column shows a `▲`/`▼` arrow in its header. Sorting composes with the
filters above, and each view's sort is remembered across restarts.

## Reviewing code

Open a PR, go to the **Diff** tab, and press `Enter` on a file to get a line
cursor. On any code line, `c` writes an inline comment - but nothing is sent
yet: comments are **buffered locally** (the line gets a `▎` marker) so you can
comment on several lines first. Press `s` to submit them all as one review, and
pick the verdict: **Comment**, **Approve**, or **Request changes**.

- **GitHub** submits them as a single native review.
- **GitLab** posts them as positioned discussions (and approves if you chose Approve).
- **Azure DevOps / Bitbucket** don't return inline patches, so the line cursor -
  and therefore line comments - aren't available there yet.

You can also drill into a single commit's diff from the **Commits** tab (`Enter`).

## Notifications

forgetop raises native desktop notifications on the events you care about:

- **Pipeline failed** — a run transitions into a failed state.
- **Review requested** — you're newly added as a reviewer on a PR.
- **Your PR approved** / **changes requested** — a reviewer votes on a PR you authored.

Press **`N`** anywhere to open a checklist and opt in/out of each event; the choice
is persisted. The first-run wizard also asks after you add your first connection.
Detection is seeded on load (no startup spam), de-duped, and re-seeded when you
change the settings — and enabling fires one confirmation notification so you can
verify they work on your machine. "Me" for the PR events is resolved from your
token by each provider (`@me` / `currentUser()` / `isMe`).

All events span **every bound provider** — pipeline failures and PR events fire
across all connections feeding those sections, not just the first.

## Configuration

Config is a small JSON file, created and managed for you:

| OS | Path |
| --- | --- |
| macOS | `~/Library/Application Support/forgetop/config.json` |
| Linux | `~/.config/forgetop/config.json` |
| Windows | `%APPDATA%\forgetop\config.json` |

**It never contains secrets** - only a reference to each token in the keychain.

### Tokens

Tokens are stored in your OS keychain under the service name `forgetop` (macOS
Keychain, Windows Credential Manager, Linux Secret Service). In headless
environments (CI, containers) you can instead supply a token via an environment
variable named `FORGETOP_PAT_<CONNECTION_ID>` (uppercased; non-alphanumeric
characters become `_`).

### Token scopes

| Provider | What to create | Scopes |
| --- | --- | --- |
| **GitHub** | Personal access token | `repo` (PRs, issues, checks); `workflow` / Actions read for pipelines |
| **GitLab** | Personal access token (Settings → Access Tokens) | `api` (merge requests, issues, pipelines) |
| **Azure DevOps** | Personal access token | Code *Read*, Work Items *Read &amp; Write*, Build *Read &amp; Execute* |
| **Linear** | Personal API key (Settings → Security &amp; access → API) | default |
| **Jira** | API token (id.atlassian.com → Security → API tokens) + your account email | default (account access) |
| **Bitbucket** | App password (Personal settings → App passwords) + your username | Pull requests, Pipelines (read &amp; write) |

## Themes

Cycle with `t`. Four built-in themes - `slate` (default), `dark`, `light`, and
`matrix` - using 256-colour palettes so they render correctly on every terminal.
Your choice is remembered.

## How it works

forgetop is a Rust [Cargo workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html):

| Crate | Responsibility |
| --- | --- |
| `forgetop-core` | Domain model, capability-scoped provider traits, config, secrets, services |
| `forgetop-providers` | GitHub, GitLab, Azure DevOps, Linear, Jira, Bitbucket, and Demo implementations |
| `forgetop-tui` | The [ratatui](https://ratatui.rs) terminal UI |
| `forgetop-cli` | The `forgetop` binary |

Providers advertise *capabilities* (which sections they support), so a connection
only offers what it can actually do - Linear appears for Work Items but not Pull
Requests, for example.

**Every section aggregates across providers.** Bind more than one connection to
Pull Requests, Work Items, or Pipelines (in the config screen: `p` / `w` open a
multi-select checklist; pipelines use `s`) and their items merge into one list,
tagged with a **Provider** column. Filtering, sorting, and actions all work across
the combined list, and each action targets the row's own provider.

## Development

```sh
cargo test        # run the test suite
cargo clippy      # lint
cargo run -- --demo
```

Source: [github.com/magna-nz/forgetop](https://github.com/magna-nz/forgetop).
