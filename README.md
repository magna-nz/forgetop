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

## Setup walkthrough

The first time you run `forgetop` (with no config yet) it launches a **setup
wizard**. Everything is a small modal dialog — **↑/↓** to move in a list, **Enter**
or **Tab → Enter** to accept, **Esc** (or *Cancel*) to skip a step.

**1. Welcome.** A dialog explains there are no connections yet and that you can
skip any section and finish later with `F3`. Press **Enter** to continue.

**2. For each section, "Configure *X* now?"** You're asked in turn about
**Pull Requests**, **Work Items**, then **Pipelines**. Choose **Yes** to set it up
now or **No** to skip (configure it later with `F3`). Sections are independent —
you don't have to use the same provider for each.

**3. Pick a provider.** Only providers that *can* serve that section are listed
(e.g. Linear appears for Work Items but not Pull Requests).

**4. Answer the prompts.** What you're asked depends on the provider:

| Provider | Prompts |
|----------|---------|
| **GitHub** | Display name · Organization (owner/org) · Repository · Personal Access Token |
| **Azure DevOps** | Display name · Organization · Project · Repository · Personal Access Token |
| **Linear** | Display name · Personal API key |
| **Demo** | Display name only (no token) |

- **Display name** is just a label shown in the tab header (e.g. `Acme/web`).
- The **token** is stored in your **OS keychain** (macOS Keychain / Windows DPAPI /
  Linux libsecret) — never written to disk in plaintext. Paste it and press Enter.

**5. Done.** forgetop confirms each section (e.g. *"Pull Requests is now served by
Acme/web"*) and drops you into the app with your data loaded.

### Worked example — PRs on GitHub, Pipelines on Azure DevOps, Work Items on Linear

```
Configure Pull Requests now? → Yes
  Provider            → GitHub
  Display name        → Acme/web
  Organization        → acme
  Repository          → web
  Personal Access Token → ghp_xxx           (scopes: repo, workflow)

Configure Work Items now? → Yes
  Provider            → Linear
  Display name        → Acme (Linear)
  Personal API key    → lin_api_xxx

Configure Pipelines now? → Yes
  Provider            → AzureDevOps
  Display name        → Acme CI
  Organization        → acme
  Project             → Platform
  Repository          → web
  Personal Access Token → (PAT: Code read, Build read/execute)
```

You now have GitHub PRs, Linear issues, and ADO pipelines side by side. The
Pipelines section can hold **more than one** connection — add another (e.g. GitHub
Actions) with `F3` and it aggregates the runs.

### Changing things later (`F3`)

Press **`F3`** any time to open the config screen:
- **Add → Pull Requests / Work Items / Pipelines** — runs the same prompts as above
  to bind (or re-bind) a section.
- **Remove: *name*** — deletes a connection and unbinds it everywhere.

Changes persist and the affected section refreshes immediately.

### Tokens & scopes

| Provider | Suggested scopes |
|----------|------------------|
| GitHub | `repo` + `workflow` |
| Azure DevOps | Code (read), Work Items (read/write), Build (read/execute) |
| Linear | a personal API key |

No keychain / prefer env vars? forgetop also reads `FORGETOP_PAT_<connection-id>`
as a fallback.

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
vs Azure DevOps numeric votes; "Issues" vs "Work Items"). See
[Setup walkthrough](#setup-walkthrough) for connecting them and token scopes.

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
