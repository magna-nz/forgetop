# testdata

Fixtures shared by more than one language.

`pr_state_cases.json` pins the pull-request verdict sentence. The judgement lives in
`forgetop-core` (`launchpad::pr_state`) but the sentence is built twice — once in the TUI
(`ui::pr_state_line`) and once in the web dashboard (`format.ts`'s `prStateLine`) — because the
two frontends share no runtime. AGENTS.md forbids a logic fork between them, so both test suites
read these cases and assert the same output:

- `crates/forgetop-tui/src/ui.rs` — `pr_state_sentences_match_the_shared_fixture`
- `crates/forgetop-server/web/src/format.test.ts`

Change the wording on one side and the other side's suite fails. Add a case here when you add a
state.

Each case supplies only the fields that vary; both sides fill the rest from their own base
fixture, with `target_ref` of `main`.
