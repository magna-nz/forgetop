# forgetop

**htop for your software forges** — a multi-provider terminal UI for pull
requests, work items, and CI pipelines across **GitHub, Azure DevOps, and
Linear**, from one keyboard-driven app.

Inspired by [Elpulgo/azdo](https://github.com/Elpulgo/azdo), but provider-agnostic:
each of the three sections binds to its **own** provider, so you can review PRs
on GitHub, watch pipelines on Azure DevOps, and track work items in Linear — at
the same time. The Pipelines section can aggregate **several** providers at once.

## Install

**As a .NET global tool (recommended).** Requires the [.NET 10 SDK/runtime](https://dotnet.microsoft.com/download).

```bash
dotnet tool install --global forgetop
forgetop --demo        # try it with mock data, no credentials
forgetop               # real usage — first run launches the setup wizard
```

Update / uninstall: `dotnet tool update --global forgetop` · `dotnet tool uninstall --global forgetop`.

**From source.**

```bash
git clone https://github.com/magna-nz/forgetop.git && cd forgetop
dotnet run --project src/Forgetop.Cli -- --demo      # run directly, or…
dotnet pack src/Forgetop.Cli -c Release -o ./nupkg   # …pack and install as a tool:
dotnet tool install --global --add-source ./nupkg forgetop
```

**Self-contained binary (no .NET install needed).**

```bash
dotnet publish src/Forgetop.Cli -c Release -r osx-arm64 --self-contained \
  -p:PublishSingleFile=true -o ./dist
./dist/forgetop --demo
```

Swap the runtime id for your platform: `osx-arm64`, `osx-x64`, `linux-x64`, `win-x64`.

**Homebrew** — planned: a tap serving the self-contained binaries above
(`brew install magna-nz/tap/forgetop`). Not yet available.

> forgetop is a full-screen TUI and needs an interactive terminal. Run it
> directly in your terminal — piping it / running in CI prints a friendly message and exits.

## First-run setup

On first launch (no config yet) forgetop walks you through binding each section:

1. **Pick a provider** for the section (e.g. GitHub for Pull Requests).
2. **Enter the scope** — owner/org, project, repo as the provider needs.
3. **Paste a Personal Access Token** — stored in your OS keychain (never written to disk in plaintext).

Skip any section and configure it later with **`F3`** inside the app. Each section
is independent, so you can mix providers (PRs on GitHub, pipelines on Azure DevOps,
work items on Linear). The Pipelines section can track **several** providers at once.

Suggested token scopes: **GitHub** `repo` + `workflow` · **Azure DevOps** Code (read),
Work Items (read/write), Build (read/execute) · **Linear** a personal API key.
No token? Set `FORGETOP_PAT_<connection-id>` env vars as a fallback.

## Keys

| Scope | Keys |
|-------|------|
| Global | `Tab`/`←→` switch section · `↑↓` move · `F5` refresh · `F3` config · `F2` theme · `F1` help · `^Q` quit |
| Pull Requests | `f` filter (All/Mine/ReviewRequested) · `a` approve · `m` merge · `c` comment · `d` diff/files · `v` comments |
| Work Items | `f` toggle mine · `s` set state · `c` comment |
| Pipelines | `↵` drill-in (stages + logs) · `t` trigger/re-run · `d` discover & subscribe · `u` unsubscribe |

## Providers & capabilities

| Provider | Pull Requests | Work Items | Pipelines | Auth |
|----------|:---:|:---:|:---:|------|
| GitHub | ✅ | ✅ Issues | ✅ Actions | PAT |
| Azure DevOps | ✅ | ✅ Work Items | ✅ Pipelines | PAT |
| Linear | — | ✅ | — | API key |
| Demo | ✅ | ✅ | ✅ | none |

GitLab and Bitbucket are planned for v2. Providers declare their capabilities,
so the UI hides/relabels what a platform doesn't support (e.g. GitHub "approve"
vs Azure DevOps numeric votes; "Issues" vs "Work Items").

### Tokens

Tokens are stored in your OS keychain (macOS Keychain, Windows DPAPI, Linux
libsecret). As a fallback, forgetop reads `FORGETOP_PAT_<KEY>` environment
variables. Suggested PAT scopes: GitHub `repo` + `workflow`; Azure DevOps Code
(read), Work Items (read/write), Build (read/execute); Linear a personal API key.

## Architecture

```
Forgetop.Core            domain model, capability-scoped source interfaces,
                         connection/binding config, runtime config service, secrets
Forgetop.Providers.*     GitHub / AzureDevOps / Linear / Demo adapters
Forgetop.Tui             Terminal.Gui shell, section screens, controllers, wizard
Forgetop.Cli             entry point + DI composition (the `forgetop` command)
```

A provider implements only the source interfaces it supports
(`IPullRequestSource` / `IWorkItemSource` / `IPipelineSource`). Adding a provider
means writing one adapter and zero UI changes.

## Development

```bash
dotnet build Forgetop.slnx
dotnet test  Forgetop.slnx
```

- .NET 10 · Terminal.Gui (v1) · Spectre.Console · xUnit
- See `SPEC.md` for the full design and `STATUS.md` for current state.
