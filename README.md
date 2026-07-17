# forgetop

A fast, keyboard-driven command center for your pull requests, work items, and CI
pipelines — across **GitHub**, **GitLab**, **Azure DevOps**, **Bitbucket**, **Linear**,
and **Jira** — in your **terminal**, your **browser**, or both.

📖 **Docs: [magna-nz.github.io/forgetop](https://magna-nz.github.io/forgetop/)**

[![CI](https://github.com/magna-nz/forgetop/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/magna-nz/forgetop/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/magna-nz/forgetop?sort=semver&label=release)](https://github.com/magna-nz/forgetop/releases/latest)
[![Docs](https://img.shields.io/badge/docs-magna--nz.github.io%2Fforgetop-blue)](https://magna-nz.github.io/forgetop/)
[![Rust](https://img.shields.io/badge/built%20with-Rust-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

<div align="center">
  <table>
    <tr>
      <td align="center" width="50%"><img src="docs/launchpad2.gif" alt="The Launchpad in the terminal" width="400"></td>
      <td align="center" width="50%"><img src="docs/dashboard.gif" alt="The Launchpad in the web dashboard" width="400"></td>
    </tr>
    <tr>
      <td align="center"><sub><b>In your terminal</b></sub></td>
      <td align="center"><sub><b>…or your browser</b></sub></td>
    </tr>
  </table>
  <br/>
  <sub>The <b>Launchpad</b> — the same triaged, actionable queue in your terminal and your browser. Same data, same actions. Your call.</sub>
</div>

---

Checking GitHub for reviews, Azure for failing builds, and Jira for tickets is a lot of
tabs. forgetop pulls your pull requests, work items, and pipelines into one command center
— and lets you *act* on them (approve, merge, comment, change state, drill into pipeline
stages, trigger runs). Run it as a fast terminal UI, a browser dashboard, or **both at
once** (the default). Tokens live in your OS keychain, never in plaintext.

## The Launchpad

forgetop opens on the **Launchpad** — one cross-provider queue answering *what needs me
right now?* Every PR, work item, and pipeline that needs you — across every connected
forge — lands on a single page, ordered by urgency, in two columns:

- **Needs you** — reviews requested of you, pipeline gates to approve, PRs ready to
  merge, and anything of yours that's broken.
- **Your work** — your assigned tickets, open PRs, and recent merges.

Every item is shown in the same shape, so they're comparable at a glance — and you act on
any of them inline (approve a review, merge, clear a pipeline gate) without switching tabs.
Nothing slips through the cracks.

See the [Launchpad docs](https://magna-nz.github.io/forgetop/#launchpad) for the full
bucket rules and keys.

Two more ways to stay on top of things: **`i`** opens a cross-provider **notification
inbox** (mentions, review requests, CI failures, assignments), and **`Ctrl-P`** is a
**fuzzy command palette** that jumps to any PR, work item, or pipeline across every forge.

## Why forgetop

Two popular TUIs inspired this: [gh-dash](https://github.com/dlvhdr/gh-dash) (GitHub only)
and [azdo](https://github.com/Elpulgo/azdo) (Azure DevOps only). forgetop's angle is
**one keyboard-driven tool across every forge**, with equal footing for code review
*and* CI.

| Capability | forgetop | gh-dash | azdo |
| :--- | :---: | :---: | :---: |
| Forges supported | **6** | GitHub | Azure |
| Cross-provider action inbox (Launchpad) | ✅ | ❌ | ❌ |
| Cross-provider notification inbox | ✅ | ❌ | ❌ |
| PRs + Work items + Pipelines | ✅ | PRs only | ✅ Azure |
| Act (approve / merge / comment) | ✅ | ✅ PRs | partial |
| Inline line-comment review | ✅ | preview | ❌ |
| Pipeline drill-in + logs | ✅ | ❌ | ✅ |
| Approve pipeline gates in-terminal | ✅ | ❌ | ❌ |
| Filter · sort · saved prefs | ✅ | ✅ | limited |
| Fuzzy command palette (jump to anything) | ✅ | ❌ | ❌ |
| Cross-provider aggregation | ✅ | ❌ | ❌ |
| Desktop notifications | ✅ | ❌ | ❌ |
| Tokens in OS keychain | ✅ | via `gh` | PAT |

…and all of it works the same in the **terminal UI** and the **web dashboard** - they're two
frontends over one core.

## Providers

**GitHub · GitLab · Azure DevOps · Bitbucket · Linear · Jira** (plus a built-in `--demo`).
Each section — Pull Requests, Work Items, Pipelines — binds to the forges that serve it, and
Pipelines can aggregate several at once. For exactly what each provider serves (and the
notification-feed support), see the
[capability matrix](https://magna-nz.github.io/forgetop/#providers-and-capabilities).

## Install

**Homebrew** (macOS / Linux):

```sh
brew install magna-nz/tap/forgetop
```

**Shell installer** (macOS / Linux):

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/magna-nz/forgetop/releases/latest/download/forgetop-installer.sh | sh
```

**Windows** (PowerShell):

```powershell
irm https://github.com/magna-nz/forgetop/releases/latest/download/forgetop-installer.ps1 | iex
```

Or grab a prebuilt binary for your platform from the
[latest release](https://github.com/magna-nz/forgetop/releases/latest) (macOS
Apple Silicon + Intel, Linux x86_64 + arm64, Windows x86_64).

## Quick start

Try it with no setup — everything is in-memory, nothing is written:

```sh
forgetop --demo
```

It opens on the **[Launchpad](https://magna-nz.github.io/forgetop/#launchpad)** — your
triaged queue across both demo connections; press `Tab` (or `2`–`4`) for the per-type
lists.

Then run it for real:

```sh
forgetop
```

This opens the **terminal UI and the dashboard together** (the default). On first launch —
before any connection has a token — the dashboard drops into a quick setup: pick a provider,
paste a token, choose which sections it feeds. Tokens go to your OS keychain and are shared
straight back to the terminal. Press **`C`** in the TUI (or open **Settings**) to manage
connections any time.

If something isn't connecting, run `forgetop doctor` — it checks your config, keychain
access, and each connection's token + connectivity.

## Terminal or browser

The dashboard is the **same app** as the TUI — Launchpad, all three lists, PR review with an
inline-comment diff viewer, the command palette, sort/filter, themes, and every write action
— served by forgetop itself, built into the binary, on **`127.0.0.1` only** with a
per-session token. No separate install, no external network.

- `forgetop` opens **both** (the default). Press **`B`** in the TUI to open the browser, or
  run **`forgetop --dashboard`** for browser-only (handy over SSH with a forwarded port).
- Choose what launches under **Settings → When forgetop starts** (or **`,`** in the TUI):
  *dashboard + terminal*, *terminal only*, or *dashboard only* — stored in your config,
  shared between the two.

## Documentation

forgetop shows a **context-aware key glossary** along the bottom, so you rarely
need a reference. The full docs live at **[magna-nz.github.io/forgetop](https://magna-nz.github.io/forgetop/)**:

- [Launchpad](https://magna-nz.github.io/forgetop/#launchpad) — the cross-provider action inbox
- [Keybindings](https://magna-nz.github.io/forgetop/#keybindings) — every key, per screen
- [Configuration &amp; tokens](https://magna-nz.github.io/forgetop/#configuration) — config paths, keychain, token scopes per provider
- [Themes](https://magna-nz.github.io/forgetop/#themes)
- [How it works](https://magna-nz.github.io/forgetop/#how-it-works) — architecture

## Contributing

```sh
cargo test        # run the test suite
cargo clippy      # lint
cargo run -- --demo
```

See [How it works](https://magna-nz.github.io/forgetop/#how-it-works) for the crate layout,
and [INTEGRATION.md](INTEGRATION.md) for the live provider integration tests.

## License

[MIT](LICENSE)
