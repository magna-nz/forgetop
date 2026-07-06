# forgetop v2 — Rust rewrite + web dashboard (DRAFT, pending agreement)

Rebuild forgetop in **Rust** (TUI via [ratatui](https://github.com/ratatui/ratatui)),
matching today's .NET functionality exactly, then add a **web dashboard** served by
the same binary. True-colour, colourful, easy to use.

The .NET implementation stays in the repo during the transition and is **deleted once
the Rust version reaches parity**.

## Goals
- 1:1 functional parity with the current app: **3 tabs** (Pull Requests, Work Items,
  Pipelines), **per-section provider binding**, **multi-provider pipelines**, filters,
  vote/merge/comment, state change, pipeline drill-in (stages → jobs → steps),
  connection health, 30s refresh, setup wizard, PAT storage.
- Colourful true-colour UI (ratatui) — no 16-colour ceiling.
- A **web dashboard** (Phase 2) served by `forgetop`, opened in the browser.

## Non-goals (unchanged)
Web/desktop native GUI beyond the served dashboard · full git client · YAML authoring ·
AI layer · OAuth (PAT only for v1) · GitLab/Bitbucket (later).

## Phase 1 — Rust TUI (parity)
- **Stack**: ratatui + crossterm (backend), tokio (async), reqwest (HTTP), serde
  (JSON), keyring (OS keychain, with env-var fallback).
- **Structure** (Cargo workspace):
  - `core` — domain types, capability-scoped provider traits, config + bindings,
    secret store, section/health services.
  - `providers` — GitHub, Azure DevOps, Linear, Demo (reqwest/GraphQL).
  - `tui` — ratatui app: tabs, tables, expand-below detail, drill-in tree, dialogs,
    theme, connections bar.
  - `cli` — the `forgetop` binary + setup wizard + DI wiring.
- **Behaviour to match**: everything in the current README's key map.

## Phase 2 — Web dashboard (after Phase 1)
- `forgetop` starts a **local web server** (axum) exposing a JSON API over the same
  core/providers, serves the built dashboard, and can open it in the browser.
- **Frontend**: a modern JS framework with **Material Design**, professional look.
  - **3 tabs** mirroring the TUI.
  - **Pull Requests**: rich list; click a PR → **detail panel** with all PR info
    (title, author, branches, reviewers, labels, checks/CI, mergeable, changed files,
    summary); **"Open in provider"** button (deep-links to GitHub/ADO/Linear).
  - **Pipelines**: list showing current state of each pipeline/run.
  - **Work Items**: list + detail.

## Decisions (locked)
1. **Dashboard frontend**: React + MUI (Material UI).
2. **Serve model**: `forgetop` runs the TUI and starts a local web server in the
   background; a keypress (`w`) opens the dashboard in the browser.
3. **Repo layout**: Rust Cargo workspace at the repo root (going-forward primary);
   .NET stays in its folders until deleted at parity.
4. **Toolchains present**: cargo 1.89, node 25.8, npm 11.
5. **Web server**: axum + tokio (shares core/providers with the TUI).

## Constraints / honesty
- The **Rust TUI** still can't be visually verified in this environment (no TTY) —
  verified by `cargo build`/`cargo test` + your run. ratatui's true colour removes the
  palette problems, though.
- The **web dashboard CAN be visually verified here** (run dev server + screenshot).
- This is a large, multi-wave effort spanning sessions.
