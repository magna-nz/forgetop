# forgetop

A fast, keyboard-driven terminal UI for your pull requests, work items, and CI
pipelines - across **GitHub**, **GitLab**, **Azure DevOps**, **Linear**, **Jira**,
and **Bitbucket** - in one place.

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

## Why forgetop

Two popular TUIs inspired this: [gh-dash](https://github.com/dlvhdr/gh-dash) (GitHub only)
and [azdo](https://github.com/Elpulgo/azdo) (Azure DevOps only). forgetop's angle is
**one keyboard-driven tool across every forge**, with equal footing for code review
*and* CI.

| | **forgetop** | gh-dash | azdo |
| --- | :---: | :---: | :---: |
| Forges | GitHub · GitLab · Azure DevOps · Bitbucket · Linear · Jira | GitHub only | Azure only |
| PRs · Work items · Pipelines in one tool | ✅ all three | PRs + issues | ✅ (Azure) |
| Act on items (approve / merge / comment / state) | ✅ | ✅ (PRs) | partial |
| Inline diff review with **buffered line comments** | ✅ | preview only | ❌ |
| Pipeline drill-in (stages→jobs→steps, durations, logs) | ✅ | ❌ | ✅ |
| Quick-filter · sort · per-view prefs (persisted) | ✅ | ✅ | limited |
| **Desktop notifications** (pipeline fail, review requested, PR approved) | ✅ | ❌ | ❌ |
| Tokens in the OS keychain | ✅ | via `gh` | PAT in config |

Where gh-dash still leads: user-defined saved sections with search queries — on the
roadmap.

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

## Documentation

forgetop shows a **context-aware key glossary** along the bottom, so you rarely
need a reference. The full docs live at **[magna-nz.github.io/forgetop](https://magna-nz.github.io/forgetop/)**:

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

See [How it works](https://magna-nz.github.io/forgetop/#how-it-works) for the crate layout.

## License

[MIT](LICENSE)
