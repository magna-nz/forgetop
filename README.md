# forgetop

A fast, keyboard-driven terminal UI for your pull requests, work items, and CI
pipelines - across **GitHub**, **GitLab**, **Azure DevOps**, **Linear**, **Jira**,
and **Bitbucket** - in one place.

📖 **Docs: [magna-nz.github.io/forgetop](https://magna-nz.github.io/forgetop/)**

[![CI](https://github.com/magna-nz/forgetop/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/magna-nz/forgetop/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/magna-nz/forgetop?sort=semver&label=release)](https://github.com/magna-nz/forgetop/releases/latest)
[![Docs](https://img.shields.io/badge/docs-magna--nz.github.io%2Fforgetop-blue)](https://magna-nz.github.io/forgetop/)
[![Rust](https://img.shields.io/badge/built%20with-Rust-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

<div align="center">
  <img src="docs/demo.gif" alt="forgetop demo" width="820">
  <br/>
  <sub>Live dashboard - Pull Requests, Work Items, Pipelines.</sub>
</div>

---

Modern teams spread work across several forges. forgetop pulls your pull requests,
work items, and pipelines into one terminal dashboard - and lets you *act* on them
(approve, merge, comment, change state, drill into pipeline stages, trigger runs)
without leaving the keyboard. Tokens live in your OS keychain, never in plaintext.

## The Launchpad

forgetop opens on the **Launchpad** - a single, cross-provider **action inbox** that
answers *what needs me right now?* It triages every PR, work item, and pipeline across
every connected provider into two prioritised columns:

- **Needs you** - PRs awaiting your review, pipeline gates you can approve, your PRs
  that are ready to merge, and anything of yours that's failing.
- **Your work** - work items assigned to you, your open pull requests, and what you've
  recently merged.

Every row reads the same way - a `PR` / `CI` / `WI` type badge, a status signal, the
title, change stats / branch, provider, and age - so a review, a broken build, and a
task line up at a glance. `Enter` opens any item in its full view; `Esc` brings you back
to the same row; and acting on something (review, approve, merge) clears it from the
queue immediately. The Pull Requests / Work Items / Pipelines tabs remain as drill-downs.

See the [Launchpad docs](https://magna-nz.github.io/forgetop/#launchpad) for the full
bucket rules and keys.

## Why forgetop

Two popular TUIs inspired this: [gh-dash](https://github.com/dlvhdr/gh-dash) (GitHub only)
and [azdo](https://github.com/Elpulgo/azdo) (Azure DevOps only). forgetop's angle is
**one keyboard-driven tool across every forge**, with equal footing for code review
*and* CI.

| Capability | forgetop | gh-dash | azdo |
| :--- | :---: | :---: | :---: |
| Forges supported | **6** | GitHub | Azure |
| Cross-provider action inbox (Launchpad) | ✅ | ❌ | ❌ |
| PRs + Work items + Pipelines | ✅ | PRs only | ✅ Azure |
| Act (approve / merge / comment) | ✅ | ✅ PRs | partial |
| Inline line-comment review | ✅ | preview | ❌ |
| Pipeline drill-in + logs | ✅ | ❌ | ✅ |
| Approve pipeline gates in-terminal | ✅ | ❌ | ❌ |
| Filter · sort · saved prefs | ✅ | ✅ | limited |
| Cross-provider aggregation | ✅ | ❌ | ❌ |
| Desktop notifications | ✅ | ❌ | ❌ |
| Tokens in OS keychain | ✅ | via `gh` | PAT |

## Providers

Each section binds to a provider that supports it; the Pipelines section can
aggregate several at once.

| Provider | Pull Requests | Work Items | Pipelines |
| --- | :---: | :---: | :---: |
| **GitHub** | ✅ | ✅ (Issues) | ✅ (Actions) |
| **GitLab** | ✅ (MRs) | ✅ (Issues) | ✅ (CI) |
| **Azure DevOps** | ✅ | ✅ | ✅ (Builds) |
| **Bitbucket** | ✅ | - | ✅ (Pipelines) |
| **Linear** | - | ✅ | - |
| **Jira** | - | ✅ | - |
| **Demo** | ✅ | ✅ | ✅ |

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

Try it with no setup - everything is in-memory, nothing is written:

```sh
forgetop --demo
```

It opens on the **[Launchpad](https://magna-nz.github.io/forgetop/#launchpad)** — your
triaged queue across both demo connections; press `Tab` (or `2`–`4`) for the per-type
lists.

Then run it for real. On first launch forgetop drops straight into the
**add-connection wizard**:

```sh
forgetop
```

<div align="center">
  <img src="docs/wizard.gif" alt="Adding a connection with the wizard" width="720">
  <br/>
  <sub>The first-run wizard: pick a provider, enter details, paste a token, bind a section.</sub>
</div>

The wizard stores your token in the OS keychain and binds the connection to a
section. Press **`C`** any time to manage connections and bindings.

If something isn't connecting, run `forgetop doctor` — it checks your config,
keychain access, and each connection's token + connectivity.

## Documentation

forgetop shows a **context-aware key glossary** along the bottom, so you rarely
need a reference. The full docs live at **[magna-nz.github.io/forgetop](https://magna-nz.github.io/forgetop/)**:

- [Launchpad](https://magna-nz.github.io/forgetop/#launchpad) - the cross-provider action inbox
- [Keybindings](https://magna-nz.github.io/forgetop/#keybindings) - every key, per screen
- [Configuration &amp; tokens](https://magna-nz.github.io/forgetop/#configuration) - config paths, keychain, token scopes per provider
- [Themes](https://magna-nz.github.io/forgetop/#themes)
- [How it works](https://magna-nz.github.io/forgetop/#how-it-works) - architecture

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
