# Changelog

What changed in each release of forgetop, newest first, and **which pull requests went
into it**. `dist` reads this file when it cuts a GitHub release and uses the section
matching the version as the release body, above the install instructions — so a release
only names its pull requests if they are written down here first.

Adding a release: one `## <version> — <date>` section, a line per user-visible change,
and the pull request each one came from. `git log v<previous>..HEAD --merges` lists the
candidates.

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
