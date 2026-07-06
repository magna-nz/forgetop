# forgetop — Status

## What was built
forgetop is a Rust Cargo workspace (`forgetop-core`, `-providers`, `-tui`, `-cli`).
The original .NET implementation has been **deleted** now that the Rust version is at parity.

- **Waves 1–4** (merged, PR #6): core domain + capability-scoped provider traits; config/secrets/services
  (secrets only in keychain/env, never in source); providers (Demo, GitHub, Azure DevOps, Linear); ratatui
  TUI shell with 256-colour indexed themes.
- **Wave 5 — interactions** (merged, PR #7): PR filter/vote/merge/comment/diff, work-item state/comment,
  pipeline drill-in tree + trigger, open-in-browser, context-aware footer glossary.
- **Wave 6 — wizard/config/docs** (branch `feature/MAG-73-rust-wave6-wizard-config`, **not yet PR'd**):
  - 6.1 add-connection wizard (stepped modal: provider → fields → masked PAT → bind)
  - 6.2 config/connections screen (`C`): list, add, bind PR/WI, remove
  - 6.3 visible-tabs toggle (`v`): show/hide sections, persisted
  - 6.4 first-run (auto-launch wizard when nothing configured)
  - 6.5 pipeline subscribe checklist (`s` in config): discover + pick definitions
  - README (professional, gh-dash/azdo style, with `docs/demo.gif` + `docs/wizard.gif` placeholders)
  - **Deleted the .NET code** (src/, tests/, Directory.Build.props, Forgetop.slnx, old SPEC.md);
    renamed SPEC.rust.md → SPEC.md; replaced the .NET .gitignore with a lean Rust one.

## Decisions made
- 256-colour indexed palettes (Terminal.app has no truecolor).
- We own the input loop (crossterm reader thread → tokio select!); overlays/wizard swallow input.
- One generic checklist overlay (`ToggleKind`) powers both visible-tabs and pipeline-subscribe popups.
- WI state picker + first-run + section visibility all persist to config; tokens to the OS keychain.

## Where we left off
Wave 6 code complete on `feature/MAG-73-rust-wave6-wizard-config`. **59 tests pass, 0 warnings, clippy clean.**
.NET removed; Rust builds and tests green afterward. **No PR raised yet** for Wave 6.

## What's next
- Raise the Wave 6 PR → `main` (user approves + merges). Mark MAG-73 done after merge.
- Record the two README screen demos (add-connection wizard + dashboard) and drop into `docs/`.
- **Phase 2 (later):** axum web server + React/MUI dashboard, `w` opens browser.

## Gotchas
- Gemini review skipped in this env (`GEMINI_API_KEY` unset).
- Run in a real terminal: `cd /Users/danielanderson/Projects/forgetop && cargo run -- --demo`.
- Session shell cwd sometimes resets to `/Users/.../Bored`; prefix cargo commands with the forgetop path.
- Demo provider writes are no-ops returning `Ok`, so actions toast success without changing demo data.
- README references `docs/demo.gif` and `docs/wizard.gif` — placeholders to be added.
