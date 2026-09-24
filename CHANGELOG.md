# Changelog

What changed in each release of forgetop, newest first, and **which pull requests went
into it**. `dist` reads this file when it cuts a GitHub release and uses the section
matching the version as the release body, above the install instructions — so a release
only names its pull requests if they are written down here first.

Adding a release: one `## <version> — <date>` section, a line per user-visible change,
and the pull request each one came from. `git log v<previous>..HEAD --merges` lists the
candidates.

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
