<div align="center">
  <img src="https://raw.githubusercontent.com/magna-nz/forgetop/main/docs/forgetop-mark.svg" alt="forgetop logo" width="104" />
  <h1>forgetop</h1>
  <p><strong>Your work, across every forge in one command center.</strong></p>
  <p>A fast, keyboard-driven home for pull requests, work items, and CI pipelines in your terminal, browser, or both.</p>
  <p>
    <a href="https://github.com/magna-nz/forgetop/actions/workflows/ci.yml"><img src="https://github.com/magna-nz/forgetop/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI" /></a>
    <a href="https://github.com/magna-nz/forgetop/releases/latest"><img src="https://img.shields.io/github/v/release/magna-nz/forgetop?sort=semver&label=release" alt="Latest release" /></a>
    <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey" alt="Platforms: macOS, Linux, Windows" />
    <a href="https://github.com/magna-nz/forgetop/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-8A79E0" alt="MIT License" /></a>
    <a href="https://ko-fi.com/forgetop"><img src="https://img.shields.io/badge/Ko--fi-Support%20forgetop-FF5E5B?logo=ko-fi&logoColor=white" alt="Support forgetop on Ko-fi" /></a>
  </p>
  <p><a href="#install">Install</a> · <a href="#quick-start">Quick start</a> · <a href="#the-command-center">Command Center</a> · <a href="https://magna-nz.github.io/forgetop/">Documentation</a></p>
</div>

<br />

<div align="center">
  <img src="https://raw.githubusercontent.com/magna-nz/forgetop/main/docs/dashboard-live.gif" alt="forgetop dashboard live preview" width="820" />
  <br />
  <sub><strong>In your browser</strong></sub>
  <br /><br />
  <img src="https://raw.githubusercontent.com/magna-nz/forgetop/main/docs/terminal-live.gif" alt="forgetop terminal live preview" width="820" />
  <br />
  <sub><strong>In your terminal</strong></sub>
</div>

<br />

Reviews in GitHub, builds in Azure, tickets in Jira add up to a lot of tabs.
forgetop pulls your pull requests, work items, and pipelines into one
keyboard-driven command center, and lets you act on them: approve, merge,
comment, change state, drill into pipeline stages, and trigger runs. It supports
**GitHub**, **GitLab**, **Azure DevOps**, **Bitbucket**, **Linear**, and
**Jira**. The dashboard is served by forgetop itself on **`127.0.0.1`** with a
per-session token, and your tokens live in your OS keychain, never in plaintext.

## Supports

<div align="center">
  <table>
  <tr>
    <td align="center" width="130">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/magna-nz/forgetop/main/docs/providers/github-dark.svg" />
        <img src="https://raw.githubusercontent.com/magna-nz/forgetop/main/docs/providers/github.svg" width="38" height="38" alt="GitHub" />
      </picture>
      <br />
      <sub><b>GitHub</b></sub>
    </td>
    <td align="center" width="130">
      <img src="https://raw.githubusercontent.com/magna-nz/forgetop/main/docs/providers/gitlab.svg" width="38" height="38" alt="GitLab" />
      <br />
      <sub><b>GitLab</b></sub>
    </td>
    <td align="center" width="130">
      <img src="https://raw.githubusercontent.com/magna-nz/forgetop/main/docs/providers/azuredevops.svg" width="38" height="38" alt="Azure DevOps" />
      <br />
      <sub><b>Azure&nbsp;DevOps</b></sub>
    </td>
    <td align="center" width="130">
      <img src="https://raw.githubusercontent.com/magna-nz/forgetop/main/docs/providers/bitbucket.svg" width="38" height="38" alt="Bitbucket" />
      <br />
      <sub><b>Bitbucket</b></sub>
    </td>
    <td align="center" width="130">
      <img src="https://raw.githubusercontent.com/magna-nz/forgetop/main/docs/providers/linear.svg" width="38" height="38" alt="Linear" />
      <br />
      <sub><b>Linear</b></sub>
    </td>
    <td align="center" width="130">
      <img src="https://raw.githubusercontent.com/magna-nz/forgetop/main/docs/providers/jira.svg" width="38" height="38" alt="Jira" />
      <br />
      <sub><b>Jira</b></sub>
    </td>
  </tr>
  </table>
</div>

---

## The Command Center

forgetop opens on one ranked queue: *What needs my attention? What is blocked?
What can I ship now?*

- **Needs you**: review requests, pipeline gates, PRs ready to merge, and work that needs fixing.
- **Your work**: assigned tickets, your open PRs, and recent merges.

Every item has the same shape, so pull requests, work items, and pipelines compare
at a glance. See the [Command Center docs](https://magna-nz.github.io/forgetop/#command-center)
for the full bucket rules and keys.

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

Try it with no setup. Everything is in-memory, nothing is written:

```sh
forgetop --demo
```

It opens on the **[Command Center](https://magna-nz.github.io/forgetop/#command-center)**, your
triaged queue across both demo connections; press `Tab` (or `2`–`4`) for the per-type
lists.

Then run it for real:

```sh
forgetop
```

This opens the **terminal UI and the dashboard together** (the default).

## Documentation

forgetop shows a **context-aware key glossary** along the bottom, so you rarely
need a reference. The full docs live at **[magna-nz.github.io/forgetop](https://magna-nz.github.io/forgetop/)**:

- [Command Center](https://magna-nz.github.io/forgetop/#command-center): the cross-provider action inbox
- [Keybindings](https://magna-nz.github.io/forgetop/#keybindings): every key, per screen
- [Configuration &amp; tokens](https://magna-nz.github.io/forgetop/#configuration): config paths, keychain, token scopes per provider
- [Themes](https://magna-nz.github.io/forgetop/#themes)
- [How it works](https://magna-nz.github.io/forgetop/#how-it-works): architecture

## Contributing

```sh
cargo test        # run the test suite
cargo clippy      # lint
cargo run -- --demo
```

See [How it works](https://magna-nz.github.io/forgetop/#how-it-works) for the crate layout,
and [INTEGRATION.md](https://github.com/magna-nz/forgetop/blob/main/INTEGRATION.md) for the live provider integration tests.

## Support forgetop

forgetop is free and open source. If it saves you tab-switching, consider buying it a coffee:

<a href="https://ko-fi.com/forgetop"><img src="https://ko-fi.com/img/githubbutton_sm.svg" alt="Support forgetop on Ko-fi" /></a>

## License

[MIT](https://github.com/magna-nz/forgetop/blob/main/LICENSE)
