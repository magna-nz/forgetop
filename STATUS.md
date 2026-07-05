# forgetop — Status

## What was built
Rust rewrite of forgetop (Cargo workspace: `forgetop-core`, `-providers`, `-tui`, `-cli`).
The original .NET implementation is being replaced and still lives in the repo until Rust parity (Wave 6).

- **Waves 1–4** (merged to `main`, PR #6): core domain + capability-scoped provider traits;
  config/secrets/services (secrets never in source — keychain/env only); providers
  (Demo, GitHub, Azure DevOps, Linear, all fixture-tested mappers); ratatui TUI shell with
  256-colour indexed themes, three column tables, connections bar, 30s refresh.
- **Wave 5 — interactions** (branch `feature/MAG-72-rust-wave5-interactions`, MAG-72 Done, **not yet PR'd**):
  - 5.1 PR filter (`f`), transient toast, context-aware footer key glossary (azdo-style)
  - 5.2 modal overlay system (confirm/picker/input) + PR actions: `a` approve, `x` reject,
    `m` merge (strategy picker), `c` comment
  - 5.3 full-screen PR diff+threads (`d`): file list + coloured unified patch (scroll) + comments
  - 5.4 work items: `s` state (candidates inferred from live data), `c` comment
  - 5.5 pipeline drill-in tree (`Enter`, stages→jobs→steps, collapsible) + `T` trigger

## Decisions made
- 256-colour **indexed** palettes, not truecolor RGB — Terminal.app doesn't do 24-bit and washed to teal.
- We own the input loop (crossterm reader thread → tokio `select!`) — no framework focus fights.
- `Key::Char` preserves raw chars so text input works; an open overlay swallows all keys.
- Footer glossary is context-aware by tab / screen / open overlay.
- WI state picker infers candidate states from the states seen in the live item list (provider-accurate).
- Large `Screen` variants boxed (clippy `large_enum_variant`).

## Where we left off
Wave 5 fully done, committed + pushed, MAG-72 marked Done. **No PR raised yet** for the Wave 5 branch.
44 tests pass, 0 warnings, clippy clean.

## What's next
- Raise a PR for the Wave 5 branch → `main` (user approves), then merge.
- **Wave 6 (MAG-73):** setup wizard, config UI (incl. pipeline discover/subscribe/unsubscribe —
  deferred from Wave 5 as configuration, not per-item interaction), native keychain wiring, README;
  then delete the .NET code at parity.
- **Phase 2 (later):** axum web server + React/MUI dashboard, `w` opens browser.

## Gotchas
- Gemini review is skipped in this env (`GEMINI_API_KEY` unset) — note it, don't block.
- Run the TUI in a real terminal: `cd /Users/danielanderson/Projects/forgetop && cargo run -- --demo`
  (`--demo` is fully in-memory; non-TTY just prints a guard message).
- Session shell cwd sometimes resets to `/Users/.../Bored`; prefix cargo commands with the forgetop path.
- Provider write methods (vote/merge/comment/set_state/trigger) are no-ops returning `Ok` in Demo,
  so actions toast success without changing demo data.
- `.NET` code still lives in the repo (root `Bored*`, `actuallybored`, `dfssef`) until Rust parity in Wave 6.
