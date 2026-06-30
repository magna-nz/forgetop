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

## Where we left off
Wave 2 done, tests green. **Paused for approval before Wave 3.**

## What's next
**Wave 3 — Providers** (MAG-60), 4 parallel tasks: Demo (all 3 sources), GitHub
(PRs/Issues/Actions + discovery), Azure DevOps (PRs/Work Items/Pipelines +
discovery), Linear (Work Items). Each implements only its supported source
interfaces, fixture-based tests.

## Gotchas
- Session cwd is `/Users/.../Bored`; build with absolute paths to `/Users/.../forgetop`.
- **Native OS secret store NOT implemented yet** — only env fallback + in-memory +
  composite. Native keychain (DPAPI/libsecret/Keychain) deferred to Wave 6.
- **DI container still not wired** — registry/config service are plain classes; DI
  composition happens in Wave 4 (TUI) / Cli.
- Gemini review needs `GEMINI_API_KEY` (was unset at Wave 1).
- Spectre.Console.Cli 0.55: `Command.Execute` override is `protected` + takes `CancellationToken`.
