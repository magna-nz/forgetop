# forgetop — Spec (DRAFT, pending agreement)

> A multi-provider terminal UI for managing pull requests, work items, and CI
> pipelines across GitHub, Azure DevOps, GitLab and Bitbucket — from one tool,
> without leaving the terminal.
>
> Inspired by [Elpulgo/azdo](https://github.com/Elpulgo/azdo), but provider-agnostic.

## 1. Problem & differentiator

`azdo` is a polished TUI but it is **Azure DevOps only**. Most developers work
across more than one platform (e.g. GitHub for OSS, Azure DevOps at work). The
core value of forgetop is a **single, consistent TUI over any supported provider** —
the provider abstraction *is* the product.

Secondary differentiator (later phase, not v1): an optional AI assist layer
(PR summaries, pipeline failure diagnosis) the incumbent lacks.

## 2. Goals

- One tool, one set of keybindings, across multiple git platforms.
- Reach feature parity with azdo's core: PRs, work items/issues, pipelines.
- Prove the abstraction is real by shipping **two providers in v1** (you cannot
  validate a multi-provider design with one provider).
- Secure credential storage, fast startup, demo mode for zero-config trial.

## 3. Non-goals (explicitly out of scope)

- Web or desktop GUI — terminal only.
- Being a full git client (no clone/commit/push; we manage platform objects,
  not the working tree).
- Editing pipeline/YAML definitions — we view and trigger, not author.
- Writing code reviews *for* you (AI layer is a later phase, off by default).
- Replacing the provider web UIs for admin/settings tasks.

## 4. Target users

Developers and team leads who context-switch between git platforms daily and
want a fast, keyboard-driven cockpit instead of multiple browser tabs.

## 5. Provider scope & capability matrix

Each section binds to a provider **independently** (see §7a). A provider only
implements the capabilities it actually supports — so the abstraction is split
by capability, not by provider.

| Provider      | Pull Requests | Work Items / Issues | Pipelines / CI | v1? |
|---------------|:---:|:---:|:---:|:---:|
| GitHub        | ✅ | ✅ (Issues) | ✅ (Actions) | **v1** |
| Azure DevOps  | ✅ | ✅ (Work Items) | ✅ (Pipelines) | **v1** |
| Linear        | — | ✅ | — | **v1** |
| Demo          | ✅ | ✅ | ✅ | **v1** |
| GitLab        | ✅ (MRs) | ✅ (Issues) | ✅ (CI) | v2 |
| Bitbucket     | ✅ | — | ✅ (Pipelines) | v2 |

v1 ships GitHub + Azure DevOps + Linear (+ Demo). This deliberately covers the
headline use case: **PRs on GitHub, Pipelines on Azure DevOps, Work Items on
Linear — all in one tool.** GitLab/Bitbucket follow once the binding model is
proven.

## 6. Core features (v1)

- **Three sections, independently bound** — Pull Requests, Work Items, Pipelines.
  Each section is wired to its own provider connection(s); they need not be the
  same provider (e.g. PRs→GitHub, Pipelines→ADO, Work Items→Linear).
- **Pull Requests / Merge Requests**: list, filter (mine / review-requested /
  all), view diff + files, view & post comments, approve / vote, merge.
- **Work Items / Issues**: list, filter, view, change state, comment.
- **Pipelines / CI runs** — **multi-source**: aggregate runs from more than one
  connection (e.g. GitHub Actions + ADO Pipelines). Per pipeline: live
  auto-refresh, drill into stages/jobs, view logs, re-run / trigger.
  - **Discovery**: auto-discover available pipelines/workflows for a connection
    where the API allows; otherwise add them manually by ID/path on first use;
    and add/remove a build/pipeline subscription **while the app is running**.
- **Runtime reconfiguration**: connections and section bindings can be changed
  from inside the running app, not only in the setup wizard. Changes persist and
  refresh the affected section live.
- **PAT auth per connection**, secure secret storage with env-var fallback.
- **Demo mode**: mock data, no credentials needed.
- **Setup wizard**, **themes**, **help/keybinding modal**, **state persistence**.
- **Look & feel**: modelled on azdo — top tab bar, master/detail panes,
  context-aware footer keybinding hints, theme switcher.

## 7. Unified domain model (the abstraction)

A provider-neutral core that each provider adapter maps to/from:

- `PullRequest` (covers PR / MR) — author, status, reviewers, votes, source/
  target refs, comment threads, diff.
- `WorkItem` (covers Issue / Work Item / Linear issue) — title, state, assignee,
  discussion.
- `PipelineRun` (covers Actions run / ADO Pipeline / GitLab CI) — status,
  stages, jobs, logs, triggerability.
- `PipelineDefinition` — a discoverable/subscribable pipeline/workflow.
- `Repository`, `Connection`, `Account`, `User`.

Capabilities are **split into per-section source interfaces**; a provider
implements only the ones it supports:

```
IPullRequestSource  : list / get / diff / comment / vote|approve / merge
IWorkItemSource     : list / get / setState / comment
IPipelineSource     : discover / list runs / get / logs / trigger
IProviderConnection : identity + which of the above it exposes (Capabilities)
```

Providers also declare **feature flags** so the UI can gracefully hide/relabel
things that differ (GitHub "approve" vs ADO numeric votes; "Issues" vs "Work
Items"; whether merge is supported).

## 7a. Connections & section bindings (config model)

- A **Connection** = { provider type, account/org/base URL, PAT credential ref }.
  PAT stored via `ISecretStore`; the config only references it.
- A **section binding** maps a section to one or more connections:
  - `PullRequests` → one connection (must implement `IPullRequestSource`).
  - `WorkItems` → one connection (must implement `IWorkItemSource`).
  - `Pipelines` → **one or more** connections (each `IPipelineSource`), plus a
    set of subscribed `PipelineDefinition`s per connection (auto-discovered or
    manually added).
- Bindings are validated against connection capabilities, are editable at
  runtime, and persist across sessions.

## 8. Architecture (proposed)

- `Forgetop.Core` — domain model, capability-scoped source interfaces,
  connection/binding config model, runtime-mutable config service, secret store
  abstraction, app state, provider registry.
- `Forgetop.Providers.GitHub`, `Forgetop.Providers.AzureDevOps`,
  `Forgetop.Providers.Linear`, `Forgetop.Providers.Demo` — each implements the
  subset of source interfaces it supports.
- `Forgetop.Tui` — the terminal app shell (azdo-like), section screens,
  keybindings, themes, runtime-config UI.
- `Forgetop.Cli` — entry point (`forgetop` command), setup wizard, DI wiring.
- Tests per project (xUnit), provider adapters tested against recorded fixtures.

## 9. Decisions (locked)

1. **TUI framework.** **Terminal.Gui (gui.cs) for the app shell** (windows,
   focus, keybindings, live refresh) + **Spectre.Console for rich content
   rendering** (markup, tables, diffs inside views).
2. **Cross-platform secret storage.** Per-OS native store (DPAPI on Windows,
   libsecret on Linux, macOS Keychain) behind a `ISecretStore` abstraction,
   with an environment-variable fallback. Provider choice of wrapper lib TBD in
   Wave 1.
3. **Auth.** **PAT only for v1** for both GitHub and Azure DevOps. OAuth device
   flow deferred to a later phase.
4. **Target framework.** **.NET 10 (LTS)** — current LTS as of build start and
   the only SDK installed locally. (Originally scoped as net8; revised up.)
5. **Fresh repo.** Built in a clean repo/folder `forgetop` (the old `Bored`
   scaffold is abandoned, not migrated). GitHub remote: `magna-nz/forgetop`.

## 10. Success criteria for v1

- A user binds **PRs → GitHub, Pipelines → Azure DevOps, Work Items → Linear**
  and operates all three from one app with identical interaction patterns.
- The Pipelines section shows runs from **two connections at once** (e.g. GitHub
  Actions + ADO), with pipelines auto-discovered or manually subscribed.
- Connections and section bindings can be added/changed **while the app runs**,
  and persist.
- Adding a new provider means implementing only the source interface(s) it
  supports — zero UI changes.
- `--demo` runs with no credentials.
