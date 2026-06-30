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
- .NET 10 LTS; `.slnx`; Terminal.Gui v2 API.
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

## Where we left off
Wave 3 done, tests green. **Paused for approval before Wave 4.**

## What's next
**Wave 4 — TUI shell** (MAG-61): Terminal.Gui v2 app shell (tabs, master/detail,
footer hints, themes, help modal), DI composition (registry from all factories +
config service + secret store), runs end-to-end against Demo. This is where the
DI container finally gets wired.

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
