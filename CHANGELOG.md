# Changelog

What changed in each release of forgetop, newest first, and **which pull requests went
into it**. `dist` reads this file when it cuts a GitHub release and uses the section
matching the version as the release body, above the install instructions — so a release
only names its pull requests if they are written down here first.

Adding a release: one `## <version> — <date>` section, a line per user-visible change,
and the pull request each one came from. `git log v<previous>..HEAD --merges` lists the
candidates.

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
