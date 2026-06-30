# forgetop — Status

## What was built
**Waves 1–2 complete.** Repo `magna-nz/forgetop` (private), branch
`feature/MAG-57-build-forgetop-v1`.

**Wave 1 (Foundation):** .NET 10 `.slnx` solution, `Directory.Build.props`
(net10.0, nullable, warnings-as-errors), all source + test projects, deps
(Terminal.Gui 2.4.x, Spectre.Console, Spectre.Console.Cli, MS DI), `forgetop`
CLI banner + `--demo`.

**Wave 2 (Core contracts):** all in `Forgetop.Core`.
- **Domain/** — `User`, `Repository`, `Connection`, `PullRequest` (+ `Reviewer`,
  `CommentThread`, `Comment`), `WorkItem`, `PipelineDefinition`/`PipelineRun`/
  `PipelineStage`/`PipelineJob`, enums (`Section`, `ProviderType`, statuses,
  `ReviewVote`, `WorkItemStateCategory`).
- **Providers/** — capability-scoped `IPullRequestSource` / `IWorkItemSource` /
  `IPipelineSource`, `IProviderConnection`, `ProviderCapabilities` (+ `VoteStyle`,
  `Terminology`), query/option records, `IProviderFactory` + `IProviderRegistry`
  + `ProviderRegistry`.
- **Configuration/** — `ForgetopConfig` + per-section bindings (`PullRequestBinding`,
  `WorkItemBinding`, `PipelineBinding` w/ multi `PipelineSubscription`), `UiState`,
  `JsonConfigStore` (atomic temp-file write, enums-as-strings, XDG/AppData path),
  `IConfigService` + `ConfigService` (runtime mutation, capability validation,
  cascade-on-remove, scoped `Changed` events).
- **Secrets/** — `ISecretStore`, `EnvironmentSecretStore` (read-only `FORGETOP_PAT_*`),
  `InMemorySecretStore`, `FallbackSecretStore`.
- **26 tests pass** (registry dispatch, capability validation, config round-trip,
  runtime mutation + cascade, secret stores).

## Decisions made
- .NET 10 LTS; `.slnx`.
- **Terminal.Gui pinned to v1.18.1** (not v2): introspection showed 2.4.16 is a
  heavily-refactored, partially-incomplete API (no `TabView`/`Toplevel`,
  `Application.Run(IRunnable)`). v1 is the mature, complete, documented line.
  Only `Forgetop.Tui` is affected. (User approved.)
- Spectre.Console NOT yet used in the TUI (plain Terminal.Gui rendering); kept as a
  dependency for possible richer detail rendering later, else drop in Wave 6.
- `IProvider` split into capability-scoped source interfaces; connections expose
  only supported sources (null otherwise).
- `Connection` lives in Domain (consumed by both Providers factory and Config) to
  avoid a Providers→Configuration dependency.
- Config is immutable records; `ConfigService` rebuilds + persists + raises events
  per mutation, guarded by a semaphore.

**Wave 3 (Providers) — complete.** Each provider separates a pure mapper
(JSON→domain, fixture-tested) from its HTTP client.
- **Demo** — all 3 sources over `DemoData`; backs `--demo`.
- **GitHub** — `GitHubApiClient` (PRs/Issues/Actions), `GitHubMapper`, workflow
  discovery, PAT Bearer auth.
- **Azure DevOps** — `AzureDevOpsApiClient` (PRs, Work Items via WIQL+batch,
  Build pipelines + timeline stages), numeric-vote mapping, connectionData
  self-id for voting, Basic PAT auth.
- **Linear** — `LinearApiClient` (GraphQL, work items + comments + state change),
  state-type → category 1:1 mapping, raw API-key auth.
- **58 tests pass** (33 provider + 25 core).

**Wave 4 (TUI shell) — complete.**
- **Core/App** — `ConnectionResolver` (config id → live connection via registry+secrets)
  and `SectionService` (bound sources per section, pipelines multi-source). Tested.
- **Tui** (Terminal.Gui **v1.18.1**) — `ForgetopApp` (Init/Run/Shutdown, tabbed
  Window, StatusBar hints, F1 help, F2 theme cycle, F5 refresh, ^Q quit),
  `SectionView` (master/detail: ListView + detail TextView, frame title = bound
  provider), `ThemeManager` (dark/light/matrix, persists via `IConfigService.SetThemeAsync`),
  `RowFormatter` (pure domain→row projections).
- **Cli** — `AppHost` DI composition (all 4 factories + registry + config + secrets +
  app), `DemoSetup` (Demo connection bound to all sections), `--demo` entry point.
- **67 tests pass** (29 Core + 33 Providers + 5 Tui).

**Wave 5 (Screens & interactions) — implemented (one notable deferral).**
- **PR filters** (Wave 3 gap closed): `PullRequestFilters` (Core, tested) + provider
  integration (GitHub `/user`, ADO connectionData, Demo=alice).
- **Tui controllers** (pure, tested against Demo): `PullRequestController`
  (load/cycle-filter/vote/merge/comment), `WorkItemController` (load/set-state/comment),
  `PipelineController` (aggregate/drill-in detail+logs/trigger/discover/subscribe).
- **Tui views**: specialized `PullRequestsView` (f/a/m/c), `WorkItemsView` (s/c),
  `PipelinesView` (↵ drill-in, t trigger, d discover+subscribe) over an abstract
  `SectionView` with action-key handling; `Dialogs` (prompt/pick/confirm/info/error).
- **Pipelines live auto-refresh** every 5s (background fetch → MainLoop.Invoke).
- **PR diff + changed-files** (`d` key) with **real patches on every provider**:
  GitHub (native patch), Azure DevOps (fetch base/head blob content + DiffPlex line
  diff in `UnifiedDiff`), Demo (canned). `IPullRequestSource.GetChangesAsync` +
  `FileChange` domain; rendered by `DetailFormatter.Diff`. **Inline comment threads**
  (`v` key) via `DetailFormatter.Threads`.
- **87 tests pass** (33 Core + 40 Providers + 14 Tui).

### Minor gaps remaining (fold into Wave 6)
- Pipeline **unsubscribe** and work-item **mine filter** not surfaced in the UI.
- State change is **free-text** (no per-provider valid-state picker).

**Wave 6 (Wizard, config UI, secrets, docs) — complete.**
- **Native OS secret store**: `KeychainSecretStore` (macOS, verified on arm64),
  `DpapiSecretStore` (Windows), `SecretToolSecretStore` (Linux), `OsSecretStore.CreateDefault`
  with env-var fallback; wired into non-demo `AppHost`.
- **SetupService** (Core, tested): create connection + store PAT + bind section.
- **SetupWizard / config UI** (Tui): first-run flow (no connections) + `F3` config
  screen (add to any section / remove connection).
- **RetryHandler** (Core, tested): retries transient 5xx/408/429/network errors;
  attached to all real provider HttpClients.
- **Carryovers**: pipeline `u` unsubscribe, work-item `f` mine toggle.
- **README.md** written.
- **95 tests pass** (39 Core + 40 Providers + 16 Tui).

## Where we left off
**All 6 waves complete; PR raised.** Build clean (0 warnings), 95 tests green.
Packaged as a .NET global tool (verified: pack → install → run). macOS Keychain
verified on arm64.
- **PR: https://github.com/magna-nz/forgetop/pull/1** (base `main`) — awaiting user
  merge; user will test `--demo` in their terminal after.

## What's next
- User merges PR #1 → then mark MAG-57 done.
- v2 backlog: GitLab + Bitbucket providers; OAuth device flow; Spectre-rich rendering
  in detail panes; per-provider valid-state picker.

## Gotchas
- Session cwd is `/Users/.../Bored`; build with absolute paths to `/Users/.../forgetop`.
- **Native OS secret store NOT implemented yet** — only env fallback + in-memory +
  composite. Native keychain (DPAPI/libsecret/Keychain) deferred to Wave 6.
- **DI container still not wired** — registry/config service are plain classes; DI
  composition happens in Wave 4 (TUI) / Cli.
- Gemini review needs `GEMINI_API_KEY` (was unset at Wave 1).
- Spectre.Console.Cli 0.55: `Command.Execute` override is `protected` + takes `CancellationToken`.
- **PR filter gap:** PR `ListAsync` honours only open/all state — `Mine`/`ReviewRequested`
  filters are NOT implemented for PRs (GitHub needs the search API, ADO needs creator/
  reviewer params). Work-item `MineOnly` IS implemented. Address in Wave 5.
- **GitHub run logs** return a job-status summary, not raw text (full logs are a zip).
- Provider **write paths** (vote/merge/comment/setState/trigger) are implemented but
  only covered by mapper/read tests — live verification happens in Wave 6 with real PATs.
- Each connection builds its own `HttpClient` (fine for a TUI; not via IHttpClientFactory).
- **Live TUI not verifiable here** (no TTY): Wave 4 validated by compile + Core data-path
  tests + DI graph. Run `dotnet run --project src/Forgetop.Cli -- --demo` in a real
  terminal to see it. Terminal.Gui API was introspected via a scratchpad reflection probe.
- Wave 4 loads each section synchronously before `Application.Run` (await once); live
  auto-refresh is Wave 5. Non-demo secret store is InMemory primary (non-persistent) +
  Env fallback until the native store lands in Wave 6.
