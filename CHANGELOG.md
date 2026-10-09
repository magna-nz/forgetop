# Changelog

What changed in each release of forgetop, newest first, and **which pull requests went
into it**. `dist` reads this file when it cuts a GitHub release and uses the section
matching the version as the release body, above the install instructions — so a release
only names its pull requests if they are written down here first.

Adding a release: one `## <version> — <date>` section, a line per user-visible change,
and the pull request each one came from. `git log v<previous>..HEAD --merges` lists the
candidates.

## 1.6.5 — 2026-10-09

Fixed

- **An approved pull request leaves the terminal UI's Review list as you approve it.** Approving
  (or requesting changes) put your tick on the row straight away, but the row itself stayed in
  the review-requested list until the provider had accepted the vote and a full refetch had
  landed — a few seconds of a pull request that no longer wanted your review still sitting there
  after you pressed Escape. On GitHub, which stops listing you as a requested reviewer the moment
  you review, the row now goes as soon as the review is sent, comes back if the provider refuses
  it, and is left to the provider once a refetch confirms it. GitLab, Azure DevOps and Bitbucket
  keep you as a reviewer after you vote, so their rows stay listed as before. The demo mirrors
  GitHub so the behaviour can be seen without credentials.

## 1.6.4 — 2026-10-09

Fixed

- **Your pull requests no longer vanish from "Mine" on a busy repository.** Every forge's list
  endpoint returns a repository's newest 50 pull requests, and "Mine" and "Review" were picked
  out of that page — so on a repository that opens 50 pull requests in a few days, yours dropped
  off the list (and out of the Command Center) although nothing about them had changed, and came
  back only when the repository went quiet. Both views are now asked of the forge itself,
  however old the pull request is: GitHub searches `author:@me` / `review-requested:@me` across
  every repository in scope, and GitLab, Azure DevOps and Bitbucket pass the author or reviewer
  to each repository's list. The terminal UI and the dashboard alike; a GitHub reload that finds
  them unchanged costs the search and nothing per row.
  ([#224](https://github.com/magna-nz/forgetop/pull/224))

## 1.6.3 — 2026-10-08

Changed

- **Files you mark viewed stay marked.** In the terminal UI, the files you tick with `v` on a
  pull request's Diff tab are still ticked when you close the pull request and come back to
  it, for as long as forgetop is running. A file whose diff has changed since you marked it
  (a new commit, say) comes back unticked so it gets looked at again. The open Diff tab also
  now picks up a file rewritten with the same number of added and removed lines, rather than
  showing the old text until you reopen it.
  ([#223](https://github.com/magna-nz/forgetop/pull/223))
- **The terminal README GIF opens a pull request in the pane beside its list**, the way
  opening one from Pull Requests actually looks.
  ([#223](https://github.com/magna-nz/forgetop/pull/223))

## 1.6.2 — 2026-10-08

Changed

- **Approving, commenting and editing no longer wait on the provider.** In the terminal UI,
  your vote, comment, reply or submitted review shows on the pull request straight away, and a
  work item's new state, assignee, title, description or comment does too — the write goes out
  in the background instead of freezing the screen until the provider and a full refresh have
  answered. A refusal takes the change back and says why; a merge or revert still waits for the
  provider's answer, but no longer holds the screen while it does. The dashboard's pull-request
  and work-item panes show the same writes on the click.
  ([#222](https://github.com/magna-nz/forgetop/pull/222))

## 1.6.1 — 2026-10-06

Changed

- **No more Runs column on Pipelines.** In the terminal UI, what it said now sits beside the
  name: a pipeline reads `PatchLine · 2 failed`, an expanded run `─ 20261005.3 · develop`, and
  an ungrouped run `CI · #42`. The run count is gone — it was the fetch cap on almost every row.
  ([#220](https://github.com/magna-nz/forgetop/pull/220))
- **The Pipeline / Run column stops at 48 characters.** A release named with a sentence no
  longer pushes every column after it to the far edge; the name gives way with `…`, the branch
  or failure tally stays in view, and the selected row scrolls through the full name.
  ([#220](https://github.com/magna-nz/forgetop/pull/220))

## 1.6.0 — 2026-10-06

Changed

- **The pipeline pane keeps its size.** In the terminal UI, Enter on a pipeline run moves the
  keys into the pane instead of widening it; pull requests and work items still open wide for
  their diffs and threads. ([#218](https://github.com/magna-nz/forgetop/pull/218))
- **Logs get more room.** The step tree beside an open log is narrower, and a long step name
  gives up its middle (`Run dto…stable  6s`) so its time stays in view.
  ([#218](https://github.com/magna-nz/forgetop/pull/218))

Added

- **Pan long log lines.** With the log pane focused, ← / → (or h / l) scroll a long line
  across; the time column stays put and the pane's title says how far across you are.
  ([#218](https://github.com/magna-nz/forgetop/pull/218))

Fixed

- **The Pipelines badge counts pipelines, not runs.** The tab strip and the dashboard sidebar
  read `Pipelines (74)` for a list of 8 pipelines; both now count distinct pipelines.
  ([#217](https://github.com/magna-nz/forgetop/pull/217))

## 1.5.2 — 2026-10-05

Fixed

- **Azure DevOps runs waiting on an approval show as running.** A run parked on a gate (say,
  before a Prod stage) dropped out of the pipeline list, which showed the previous run's ✓
  instead. In-progress and queued runs now come back with the rest. Stages Azure skipped show as
  skipped (⊘) rather than failed (✗), and a stage that passed with warnings shows ▲.
  ([#214](https://github.com/magna-nz/forgetop/pull/214))

Changed

- **Pipelines say what they're waiting on.** A run held on an approval gate reads ⏸ Waiting,
  not ◐ Running — everywhere: the terminal UI, the Command Center and the dashboard — and only
  while nothing in it is still running. Stages that never ran read ⊘ Skipped instead of failed or
  cancelled (Azure DevOps, GitHub, GitLab). In the terminal UI:
  - The run view opens on what needs you: finished stages fold to one line, the gated stage says
    it's waiting on approval and for how long, stages that never ran fold into one row, and the
    time axis squeezes a long wait so the work keeps its width.
  - The Pipelines list names each run's state, shows its run number, what it's stuck on (a Now
    column), a glyph per stage and how long it took — an expanded pipeline is no longer a column
    of bare ticks. ([#215](https://github.com/magna-nz/forgetop/pull/215))

Docs

- Every Terminal/Dashboard switch on the docs site has a demo for each surface.
  ([#213](https://github.com/magna-nz/forgetop/pull/213))

Build

- `async-trait` 0.1.92, so clippy passes on Rust 1.99. ([#216](https://github.com/magna-nz/forgetop/pull/216))

## 1.5.1 — 2026-09-29

Fixed

- **Mine no longer sits on "Loading…" while All has rows.** In the terminal UI, the pull
  requests a refresh fetches are now cached together, with who you're signed in as, and every
  view (Mine, Review, All) is worked out from that cache at launch. Before, only the view on
  screen was cached, so the first launch after Pull Requests switched to opening on Mine had
  nothing to show for it. The list also shows pull requests as soon as they're fetched instead of
  waiting for work items, pipelines, notifications and repository discovery to finish. ([#211](https://github.com/magna-nz/forgetop/pull/211))
- **Azure DevOps runs show their stages.** A run's jobs are attached to the stage they belong to,
  so the pipeline view lists them instead of "No stages reported". A pipeline with no stages of its
  own shows its jobs under "Jobs". ([#209](https://github.com/magna-nz/forgetop/pull/209))

Docs

- Re-recorded the README GIFs against 1.5.0, and added short demos beside each feature on the
  docs site. ([#210](https://github.com/magna-nz/forgetop/pull/210))

## 1.5.0 — 2026-09-29

Added

- **Choose the list's columns.** In the terminal UI, `c` opens a checklist of the list's columns
  on Pull Requests, Work Items and Pipelines. Space toggles a column and Enter applies. Provider
  starts off, and your choice is saved. ([#207](https://github.com/magna-nz/forgetop/pull/207))
- **Choose pipelines in the dashboard.** The dashboard's Pipelines page has a
  `Pipelines · 0 of 154` button with a searchable checklist per connection and All/None. It is the
  same saved choice the terminal's `w` edits. ([#208](https://github.com/magna-nz/forgetop/pull/208))
- **Line numbers in diffs.** Every line of a pull request diff is numbered. Added and unchanged
  lines show their new line number, and removed lines show their old one. ([#207](https://github.com/magna-nz/forgetop/pull/207))

Changed

- **Pipelines are opt-in.** A connection bound to Pipelines starts with no pipelines chosen, and
  the header reads `Pipelines · 0 of 154` until you pick some with `w` (or the dashboard's
  picker). Your choice is saved. On the first run of this version, a connection that was
  fetching every pipeline is reset to none. A connection where you'd picked specific pipelines
  keeps them. ([#208](https://github.com/magna-nz/forgetop/pull/208))
- **Pipelines hide Provider and Repository by default.** Both can be turned on with `c`, and
  Provider now shows when it's on even with a single provider. ([#208](https://github.com/magna-nz/forgetop/pull/208))
- **Pull Requests open on Mine.** The views now run Mine, Review, All, and the tab lands on Mine.
  A saved list that still starts with the old All, Mine, Review order is reordered. ([#207](https://github.com/magna-nz/forgetop/pull/207))
- **New connections start with no repositories chosen.** The repositories a new connection can
  reach are still discovered, so the header reads `Repos · 0 of 38`. The list asks you to pick,
  and nothing is fetched until you do. The dashboard's empty state now has the picker too. ([#207](https://github.com/magna-nz/forgetop/pull/207))
- **`w` chooses repositories** (it was `g`), matching Pipelines, where `w` chooses pipelines. It
  is now in the footer. Sorted lists name their sort in the title (`· by Updated`), and a list on
  its own gets the highlighted title that Pipelines has. ([#207](https://github.com/magna-nz/forgetop/pull/207))

Fixed

- **Long pop-up lists scroll.** In the terminal UI, a checklist or picker taller than the screen
  (such as `w` on an Azure org with 150 pipelines) now scrolls with the cursor instead of running
  off the bottom. In a searchable checklist, Ctrl-A ticks or clears everything shown; elsewhere
  it's `a`. ([#208](https://github.com/magna-nz/forgetop/pull/208))
- **A stalled request no longer leaves a list loading forever.** Provider requests now give up
  after 10 seconds without a connection or 30 seconds of silence. Before this, one hung request
  blocked every later refresh, and a view such as Mine never left "Loading…". Switching views
  before the first refresh lands now shows that view's cached rows. ([#207](https://github.com/magna-nz/forgetop/pull/207))

## 1.4.0 — 2026-09-29

Changed

- **The item pane opens on Enter.** In the terminal UI, the Pull Requests and Work Items lists
  keep their full width. Enter opens the selected item in a pane over the right of the list, and
  Esc closes it. Pipelines still preview the selected run as you browse. `P` switches the automatic
  preview on or off per section, and your choice is saved. ([#205](https://github.com/magna-nz/forgetop/pull/205))
- **The list stays put under the pane.** Opening or closing the pane never moves a column.
  Rows it covers end in `…`, and the selected title scrolls so it can be read in full. ([#205](https://github.com/magna-nz/forgetop/pull/205))
- **Tab walks a PR's tabs in the pane.** With a pull request open in the pane, Tab and Shift-Tab
  move between Conversation, Commits, Checks and Diff. Tab moves between sections again once the
  pane is closed. ([#205](https://github.com/magna-nz/forgetop/pull/205))
- **More room for the diff.** The diff's file list is only as wide as its filenames need. A
  single-file diff drops the list and shows the file in the patch title. ([#205](https://github.com/magna-nz/forgetop/pull/205))

## 1.3.2 — 2026-09-25

Added

- **Dismiss notifications.** In the terminal UI's inbox, `d` dismisses the selected notification
  and `D` dismisses them all (after a confirm). Dismissed notifications stay hidden across
  refreshes and restarts, and come back if the thread gets new activity. Read state on the forge
  is left alone. ([#201](https://github.com/magna-nz/forgetop/pull/201))

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
