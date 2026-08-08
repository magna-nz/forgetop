# AGENTS.md

Guidance for AI coding agents (Codex, Claude, Cursor, …) working in this repo.
Humans: this doubles as a quick contributor reference. For architecture and the dashboard
design intent see `SPEC.md`; for day-to-day status see `STATUS.md`. Both are **gitignored**
(so a fresh clone won't have them) but are backed up to the private repo `magna-nz/dev-specs`
— locally at `~/Desktop/dev-specs/forgetop/`. If a file is missing from the repo root, read the
backup; if both exist, the most recently modified one is current.

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

## Where the dashboard code lives (`crates/forgetop-server/web/src/`)
- **Shared UI** — `components/ui.tsx`: `StatusBadge`, `Pill`, `Chip`, `Row`, `Avatar`, `SlideOver`, `Timeline`, `List`, …
- **Lists** — `components/PullRequests.tsx`, `WorkItems.tsx`, `Pipelines.tsx`; the Command Center is `Launchpad.tsx`. All lists share the sort/filter control bar + generic data-derived `facet` in `components/ControlBar.tsx` (`useListView`).
- **Detail panes** (right-hand slide-overs) — `components/PrDetail.tsx`, `WiDetail.tsx`, `PipelineDetail.tsx`. Each exposes an opener context (`usePrOpener`/`useWiOpener`/`usePipelineOpener`) and an `act()` helper that runs a write then invalidates the affected queries.
- **Types** — `types.ts` mirrors the Rust DTOs in `crates/forgetop-server/src/dto.rs`. **Keep them in sync.**
- **Data** — `api.ts` (TanStack Query hooks); provider-capability gating in `capabilities.ts`; status colours/labels in `format.ts`.

## A connection is an **account**, not a repository (read before touching a provider)
A connection's credentials reach the whole account — a GitHub PAT every repo it can see, a GitLab
token every project it's a member of, an Azure PAT the whole org, a Bitbucket app password the
whole workspace. So a repository is a **per-call address**, never connection identity.

- **Two spellings, one conversion.** *Host-qualified* (`github.com/acme/pay`) is for matching;
  *connection-relative* (`acme/pay`) is for addressing and is what a scope entry and any
  `ItemRef.repo` holds. Convert **only** via `forgetop_core::repo::to_connection_relative` — never
  with `split('/')` or `trim_start_matches` at a call site. Feeding one where the other belongs
  doesn't error, it silently mismatches.
- **Address items with `ItemRef { repo, id }`**, not a bare id: on a connection spanning several
  repositories, `#7` names more than one PR. A single-repository scope resolves an unaddressed
  ref; a wider one errors rather than guessing.
- **`Connection.repo_scope: Option<Vec<String>>`** — `None` = never established (fall back to the
  legacy single `repository`), `Some([])` = the user chose none (fetch nothing), `Some([…])` =
  fetch these. Fallbacks key on the scope being **absent**, never on it being **empty**.
- **List calls fan out** over the scope (`providers::scope::fan_out`, bounded concurrency), and
  sort + cap **once across the scope** (`sort_and_cap`) — never per repository.
- **Jira and Linear are out of scope permanently** — project- and team-addressed, so a repository
  scope has nothing to govern. **Azure work items and pipelines are project-addressed**: fan them
  out over the scope's *distinct projects*, not its repositories.
- ⚠️ **The demo provider cannot catch an addressing bug** — it never resolves a repository, so
  `--demo` passes whether or not addressing is right. Prove addressing with fixtures or the live
  suite (`tests/integration/scope.rs`).

## Adding a field or action across the stack (the common recipe)
Data flows **provider trait → provider impls + demo → server DTO → `types.ts` → component**. To add something end-to-end:
1. **Domain/trait** (`forgetop-core`): add the field to the domain struct, or a method to the provider trait *with a compile-safe default* (so providers adopt it incrementally).
2. **Providers + demo** (`forgetop-providers`): populate/implement it in each provider's mapper, and in `demo.rs` (give the demo believable data — the demo is how changes are verified). A method rippling across N provider files is a good Codex fan-out; a shared struct/trait change is not (it touches every provider + the TUI — keep it on the supervisor).
3. **Server** (`forgetop-server/src`): thread it through the DTO in `dto.rs` (and add a `POST /api/…` handler in `lib.rs` + `actions.rs` for a write action, mirroring an existing one).
4. **Frontend**: add the field to `types.ts`, then use it in the component.
5. **Verify** in `--demo`, add/adjust a `vitest` (frontend) or integration test (providers), run clippy + tsc.

Two panels already demonstrate the full pattern end-to-end: WI **reassign/edit** (`assignable_users`/`set_assignee`/`update_fields`) and pipeline **cancel** (`cancel_run`) — copy their shape.

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
