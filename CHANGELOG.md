# Changelog

What changed in each release of forgetop, newest first, and **which pull requests went
into it**. `dist` reads this file when it cuts a GitHub release and uses the section
matching the version as the release body, above the install instructions — so a release
only names its pull requests if they are written down here first.

Adding a release: one `## <version> — <date>` section, a line per user-visible change,
and the pull request each one came from. `git log v<previous>..HEAD --merges` lists the
candidates.

## 1.3.1 — 2026-09-25

Added

- **Markdown in descriptions.** PR and work-item descriptions in the terminal UI render their
  markdown: headings, bold, italics, `code`, bullet and numbered lists, `- [x]` task boxes, code
  blocks, quotes, links and tables. Plain-text descriptions read as before. ([#200](https://github.com/magna-nz/forgetop/pull/200))

Fixed

- **Command Center titles cut to three characters.** A long branch name in one row squeezed every
  title on that side of the Command Center, and pushed the person and age past the pane border.
  The branch column is now capped, and it scrolls when its row is selected. ([#200](https://github.com/magna-nz/forgetop/pull/200))

Docs

- A new terminal demo GIF in the README, recorded against 1.3.1. ([#200](https://github.com/magna-nz/forgetop/pull/200))

## 1.3.0 — 2026-09-25

Added

- **A redesigned pipeline run pane** in the terminal UI: the preview beside the Pipelines list,
  focused with `Enter`/`p`, and the full-screen view.
  - A header with the run's event, branch, commit, title, attempt and linked PR.
  - A **history strip** of the pipeline's recent runs on the branch: status, a duration
    sparkline, the median, this run against it, and the last failure. `←`/`→` open the older
    or newer run.
  - **Timeline bars** for every job and step on one time axis. Post and cleanup steps fold
    into one line.
  - For a running run, the estimated finish, shaded estimates for running and queued jobs,
    and which job is holding the run up.
  - **Step log sections.** `Enter` on a step opens its part of the job log. Sections fold
    (`z`/`Z`), and a failed run opens on its first error.
  - A **failure line** naming the failing test and `file:line`, and a **Problems** panel of
    the run's annotations (`e`).
  - **Rerun** (`R`), **rerun failed jobs** (`F`), **artifacts** (`a`) and **copy commit**
    (`c`).

  The pane uses new provider support for rerun, artifacts and annotations on GitHub, GitLab
  and Azure DevOps. Bitbucket supports rerun and annotations only. ([#198](https://github.com/magna-nz/forgetop/pull/198))
- **Activity** on work items and on the PR Conversation tab. ([#196](https://github.com/magna-nz/forgetop/pull/196))
- **`@` assign** with a searchable picker, and **`e` edit** for titles (in place) and
  descriptions (in `$EDITOR`). ([#196](https://github.com/magna-nz/forgetop/pull/196))
- **`X` cancel** for a queued or running run. ([#196](https://github.com/magna-nz/forgetop/pull/196)) It updates on screen straight away and
  rolls back if the provider refuses. ([#198](https://github.com/magna-nz/forgetop/pull/198))
- **Mouse support.** Click a tab or row to select it, and click again to open. The wheel
  scrolls the pane under the pointer. Turn it off with `ui.mouse = false`. ([#197](https://github.com/magna-nz/forgetop/pull/197))

Changed

- **Needs your review** ages count from when the PR was opened. They turn yellow past
  `ui.review_sla_hours` (default 24h) and red at three times that. ([#197](https://github.com/magna-nz/forgetop/pull/197))
- On a pipeline run, `F` reruns the failed jobs; everywhere else it still opens feedback.
  ([#198](https://github.com/magna-nz/forgetop/pull/198))

Fixed

- Wrapped detail panes scroll to their real end. ([#196](https://github.com/magna-nz/forgetop/pull/196))
- GitLab job logs keep their step sections. ([#198](https://github.com/magna-nz/forgetop/pull/198))

Release

- 1.3.0 version bump. ([#198](https://github.com/magna-nz/forgetop/pull/198))

## 1.2.1 — 2026-09-24

Added

- **Ctrl-K command palette** in the terminal UI, from every screen (Ctrl-P still works).
  It searches everything already loaded: the current screen's actions (each shows its key),
  PRs, work items and runs, destinations, saved views, people, pipeline repositories,
  themes and settings, and every keybinding from `?` help. Prefixes narrow it: `>` actions,
  `:` commands (`:theme`, `:view`, `:go`, `:merge`), `@` people, `#` ids, `?` keys.
  ([#195](https://github.com/magna-nz/forgetop/pull/195))

Changed

- The footer is shorter. Moving, tab walking, Enter hints on lists, feedback, saved views,
  repos, find and visible tabs are left to `?` help and the palette. ([#195](https://github.com/magna-nz/forgetop/pull/195))
- Every footer leads with a yellow `Ctrl-K search anywhere`. `B browser dashboard` is now
  `B dashboard`. ([#195](https://github.com/magna-nz/forgetop/pull/195))

Release

- 1.2.1 version bump. ([#195](https://github.com/magna-nz/forgetop/pull/195))

## 1.2.0 — 2026-09-24

Added

- A **preview pane** beside the Pull Requests, Work Items and Pipelines lists on terminals
  140+ columns wide: the selected item's view, with its detail fetched once the cursor rests.
  `Enter` / `p` move focus into it and its keys work there; `P` hides it per section.
  ([#192](https://github.com/magna-nz/forgetop/pull/192))
- **Live pipeline logs.** The log pane sits beside the stages/jobs/steps tree, refreshes while
  a job runs and follows the tail until you scroll up. `E` jumps to the first error, `/`
  searches with `n`/`N`, and a failed run opens on its failing step.
  ([#193](https://github.com/magna-nz/forgetop/pull/193))
- **Shift-Tab** walks the tab strip backwards, wrapping, from any screen — the mirror of Tab.
  ([#194](https://github.com/magna-nz/forgetop/pull/194))

Changed

- Only `Tab` moves the top nav from a section list; the arrows and `h`/`l` no longer do.
  ([#192](https://github.com/magna-nz/forgetop/pull/192))

Fixed

- The log pane shows a job's real output on GitHub, GitLab, Azure DevOps and Bitbucket, where
  it used to show a one-line status per job.
  ([#193](https://github.com/magna-nz/forgetop/pull/193))

Release

- 1.2.0 version bump. ([#194](https://github.com/magna-nz/forgetop/pull/194))

## 1.1.4 — 2026-09-24

Changed

- The **repository** leads a Pipelines row. It sat out past `Started`, at the far right edge —
  the last place you look, for the one field that says *where* a run happened. It now reads
  first, immediately left of the pipeline/branch column.
  ([#191](https://github.com/magna-nz/forgetop/pull/191))
- A group header is coloured like a row instead of like chrome. Accent is what the borders,
  the pane titles and the live tab are painted in, and the Pipelines subject column was the
  only row content anywhere using it — which made the tab read as a different application
  next to the Title column on Pull Requests and Work Items. Bold still sets a roll-up apart
  from the runs beneath it. ([#191](https://github.com/magna-nz/forgetop/pull/191))
- Every run state is named. The status column spelled out four of its six states and drew the
  other two, so the two outcomes that matter most were a symbol to decode — and said nothing
  at all where colour is lost. Glyph *and* word now, with `Succeeded` in place of `Passed`.
  ([#191](https://github.com/magna-nz/forgetop/pull/191))
- Inside a group, the state is written where it **changes**. A run whose line above already
  said it shows the glyph alone, so a group that simply passed no longer stacks the same word
  four deep and the run that broke the streak is what the eye lands on. Nothing is hidden —
  every row still carries its own coloured glyph — and an ungrouped list repeats as before,
  since there is no header above a row to have said it.
  ([#191](https://github.com/magna-nz/forgetop/pull/191))

Release

- 1.1.4 version bump. ([#191](https://github.com/magna-nz/forgetop/pull/191))

## 1.1.3 — 2026-09-24

Fixed

- `Esc` and `q` step back instead of quitting. `Esc` on a section list used to quit forgetop
  outright, and `q` quit from every screen it was bound on — both against the contract the
  help panel already stated ("Esc back / close"), and neither advertised in the list footer,
  so the only way to find them was to lose a session to one. Both keys now close what is open
  and return to where it was opened from, and do nothing at the Command Center, which is the
  root. **Ctrl-C is the only key that leaves the app.** The footers, the help panel and the
  docs keymap all say so. ([#190](https://github.com/magna-nz/forgetop/pull/190))

Release

- 1.1.3 version bump. ([#190](https://github.com/magna-nz/forgetop/pull/190))

## 1.1.2 — 2026-09-24

Added

- The Pipelines list groups its runs — by pipeline, by trigger, by branch, or off — and lands
  collapsed, so the tab opens on one line per pipeline with a roll-up the flat list could not
  compute: `Integration · 5 runs · 3 failed`. A header reports its *latest* run, so it says
  where the pipeline stands now, with the failures beneath it carried by the count. `G` cycles
  the grouping, `Enter` or `Space` opens a group, `z` and `Z` close and open every one, and the
  choice persists. ([#188](https://github.com/magna-nz/forgetop/pull/188))
- The list's columns are chosen for what is on screen. Provider appears only when more than one
  provider is showing, Commit only when grouped by trigger, Approval only when a gate is
  waiting — and the space that frees gives the **repository** a column of its own, which the
  flat table never had room for. A shared owner prefix is elided, so `magna-nz/forgetop` reads
  as `forgetop`. On a narrow pane the context columns give way before the waiting gate does.
  ([#188](https://github.com/magna-nz/forgetop/pull/188))
- Sort by **Repository**, alphabetically. Sorting now orders the groups rather than the runs
  inside them — a grouped list is collapsed, so a sort applied inside a group moved nothing you
  could see, which made "Sort by Pipeline" while grouped by pipeline a no-op.
  ([#188](https://github.com/magna-nz/forgetop/pull/188))

Fixed

- The quick filter can see the pipeline name and the repository. `/forgetop` matched nothing
  while the column was showing it. ([#188](https://github.com/magna-nz/forgetop/pull/188))

## 1.1.1 — 2026-09-24

Fixed

- Tab keeps walking the tab strip — Command Center, Pull Requests, Work Items, Pipelines —
  from inside an open pull request, work item or pipeline run. The full-screen views used to
  answer every key themselves, so Tab did nothing there until you pressed Esc back out to a
  list, even though the tab strip and the help panel both advertise it.
  ([#185](https://github.com/magna-nz/forgetop/pull/185))

## 1.1.0 — 2026-09-23

Added

- Every surface that shows a pull request names the state it is in — blocked, waiting on
  review, ready to merge — instead of leaving you to read it off the checks. The Command
  Center rows gained a repository column.
  ([#183](https://github.com/magna-nz/forgetop/pull/183))

Release

- 1.1.0 version bump. ([#184](https://github.com/magna-nz/forgetop/pull/184))
