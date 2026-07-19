# AGENTS.md

Guidance for AI coding agents (Codex, Claude, Cursor, …) working in this repo.
Humans: this doubles as a quick contributor reference. For architecture and the dashboard
design intent see `SPEC.md`; for day-to-day status see `STATUS.md` (both are local/gitignored).

## What this is
**forgetop** — one Rust binary, **two frontends over one data/action layer**: a terminal UI
(`forgetop-tui`) and an embedded React/Vite web dashboard (`forgetop-server/web`), both built on
the shared `AppDeps` + `forgetop-core` provider traits. Six forges: GitHub, GitLab, Azure DevOps,
Bitbucket, Linear, Jira, plus a built-in `--demo`.

## Workspace layout
- `crates/forgetop-core` — domain types + capability-scoped provider **traits** (`PullRequestSource`,
  `WorkItemSource`, `PipelineSource`, …), the Command Center/launchpad logic, `AppDeps`.
- `crates/forgetop-providers` — one module per forge implementing the traits, plus `demo.rs`
  (canned, deterministic data for `--demo`). Integration tests in `tests/integration/`.
- `crates/forgetop-server` — axum HTTP server (`src/`) + the React SPA (`web/`), embedded via
  `rust-embed`.
- `crates/forgetop-tui` — the terminal UI.
- `crates/forgetop-cli` — the binary (package name **`forgetop`**, not `forgetop-cli`).

A capability the app exposes is almost always: a **trait method** (compile-safe default) →
**per-provider impl + demo** → a server **`/api/…` route** → **frontend** UI. Keep that shape.

## Build, run, test
```sh
# Web assets (needed before the binary embeds them)
npm --prefix crates/forgetop-server/web install      # first time
npm --prefix crates/forgetop-server/web run build

# Binary (embeds web/dist)
cargo build

# Run the dashboard against demo data (no credentials)
./target/debug/forgetop --dashboard --demo           # serves http://127.0.0.1:8177

# Rust checks
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Frontend checks (run inside crates/forgetop-server/web)
npx tsc --noEmit
npx vitest run

# Integration tests (real providers; needs FORGETOP_IT_* secrets)
cargo test -p forgetop-providers --features integration
```

### ⚠️ The web re-embed gotcha (read this)
`cargo build` does **not** reliably re-embed `crates/forgetop-server/web/dist` — `rust-embed`
bakes it at compile time and the server crate isn't always seen as changed. After
`npm run build`, force a re-embed:
```sh
FORGETOP_SKIP_WEB_BUILD=1 cargo build
```
Then **verify the served bundle matches the fresh build** before trusting live output:
```sh
curl -s http://127.0.0.1:8177/ | grep -o 'assets/index-[^"]*\.js'   # served (Vite hashes can contain '-')
ls crates/forgetop-server/web/dist/assets/index-*.js                # local
```
`web/dist` is gitignored (CI rebuilds it).

## Conventions
- **Two frontends, no logic fork.** Behaviour that both should share lives in `forgetop-core`
  (e.g. Command Center caps/ordering). Don't duplicate it in a frontend.
- **Scope discipline.** If a task says "dashboard" or "pane", change only the web dashboard —
  leave the TUI alone (and vice-versa).
- **Verify live.** For any non-trivial dashboard change, run `--demo` and confirm the behaviour in
  a browser, not just tests. Demo write actions persist within a run via in-memory stores in `demo.rs`.
- **Dashboard design system** (`crates/forgetop-server/web/src/components/ui.tsx`): use the shared
  `StatusBadge` (tinted bordered status chip), `Pill` (tinted rounded icon+label "attention" badge),
  and `Chip` (neutral tag). List rows are slim and single-line; detail panes expose only the actions
  common to **all** providers for that section (provider-specific extras are shown as info, or gated
  via `capabilities.ts`).
- **Match surrounding code.** Read the neighbouring provider/module first and mirror its request
  helpers, error handling, and naming. Don't introduce new patterns without cause.
- Security: the dashboard binds `127.0.0.1` only, gated by a per-session token. Never widen that.

## If you are a delegated (sandboxed) worker
When dispatched to implement one file:
- **Edit only the file you were told to.** Do not touch other crates/files.
- **Do not run `cargo`** (build/test/clippy/fmt) — the supervisor owns the build and will reconcile.
- Read the target file first and **mirror its existing patterns** (HTTP helpers, mappers, error types).
- Return a short summary of what you added.

## PRs
Branch off `main`; one PR per change; run tests + clippy + a live `--demo` check before raising it.
Note that CI has provider secrets for GitHub/GitLab/Azure/Jira/Linear but **not Bitbucket** — implement
Bitbucket, but it won't be covered by live integration tests.
