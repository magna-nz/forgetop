# Integration tests

forgetop ships a **live integration suite** that runs the provider adapters against
the real GitHub / GitLab / Azure DevOps APIs — the only way to be sure the request
signing, pagination, JSON decoding, and especially the **approval** paths work for
real. The suite creates its own fixtures (branches, PRs, issues, gated pipeline
runs) and tears them down again.

> **Secrets are never committed.** Credentials live only in your local, gitignored
> `.env` (for running locally) or in **GitHub Actions secrets** (for CI). Nothing in
> this repo contains a real token, and `.env` is in `.gitignore`.

## How it's gated

The suite is behind the `integration` Cargo feature on `forgetop-providers`, so a
normal `cargo test` never touches the network:

```sh
cargo test --workspace                 # unit tests only — no creds needed
cargo test -p forgetop-providers --features integration -- --nocapture --test-threads=1
```

Each provider's tests **skip** (print `SKIP <provider>: …` and pass) when its
environment variables are absent — so you can run just the providers you've set up.

## 1. Local setup

Copy the template and fill in the providers you want:

```sh
cp .env.example .env       # .env is gitignored
$EDITOR .env
```

Leave a provider's variables **blank to skip it**. A non-blank value makes the suite
try to authenticate, so only fill in what you intend to run.

## 2. What each provider needs

You create **one throwaway container** per provider (a repo/project/org). Everything
inside it — branches, PRs, issues, environments, pipeline runs — is created and
deleted by the tests, tagged with a `forgetop-it-<runid>` prefix. A sweeper deletes
any stray `forgetop-it-*` fixture from earlier runs at the start of each run, so a
crashed test can't leak.

### GitHub
| | |
| --- | --- |
| Container | A repo, e.g. `you/forgetop-itest`. **Make it public** — required reviewers on environments (the approval test) are free on public repos, paid on private. |
| Token | Classic PAT with scopes **`repo`** + **`workflow`**; your user must have admin on the repo. |
| Env | `FORGETOP_IT_GITHUB_TOKEN`, `FORGETOP_IT_GITHUB_REPO` (`owner/name`), `FORGETOP_IT_GITHUB_HOST` (only for GH Enterprise Server) |

### GitLab
| | |
| --- | --- |
| Container | A project; note its **numeric project id**. |
| Token | PAT with scope **`api`**; you're Maintainer/Owner. |
| Env | `FORGETOP_IT_GITLAB_TOKEN`, `FORGETOP_IT_GITLAB_PROJECT` (id), `FORGETOP_IT_GITLAB_HOST` (only for self-hosted) |

### Azure DevOps
| | |
| --- | --- |
| Container | An org + a **public** project (public ⇒ free Microsoft-hosted agents, dodges the parallelism-grant wait) + its default repo. |
| Token | PAT with **Full access** (it's throwaway). |
| Env | `FORGETOP_IT_AZURE_PAT`, `FORGETOP_IT_AZURE_ORG`, `FORGETOP_IT_AZURE_PROJECT`, `FORGETOP_IT_AZURE_REPO` |

The Azure approval test is **fully self-contained**: it pushes a gated YAML pipeline,
creates the environment + Approval check (you as approver) + pipeline definition,
queues a run, approves via the adapter, then deletes all of it. Nothing to pre-make —
just the org/project/repo container and a Full-access PAT.

### Linear (work items only)
| | |
| --- | --- |
| Container | A workspace; a team (its id is auto-detected, or pin one with `FORGETOP_IT_LINEAR_TEAM`). |
| Token | Personal API key (Settings → Security & access → API). |
| Env | `FORGETOP_IT_LINEAR_KEY`, `FORGETOP_IT_LINEAR_TEAM` (optional) |

### Jira (work items only)
| | |
| --- | --- |
| Container | A project with a **Task** issue type; note its **key** (e.g. `IT`). |
| Token | API token (id.atlassian.com → API tokens) + your account **email**. |
| Env | `FORGETOP_IT_JIRA_TOKEN`, `FORGETOP_IT_JIRA_EMAIL`, `FORGETOP_IT_JIRA_SITE` (`https://you.atlassian.net`), `FORGETOP_IT_JIRA_PROJECT` (key) |

## 3. Run it

```sh
cargo test -p forgetop-providers --features integration -- --nocapture --test-threads=1
```

`--test-threads=1` keeps provider output readable and avoids fixtures from parallel
tests colliding. Watch for lines like `github: connected to you/forgetop-itest`.

## 4. Running in CI

The `.github/workflows/integration.yml` workflow runs the suite on every (non-fork)
PR, on push to `main`, on `v*` tags, nightly (with a sweep), and on manual dispatch.
Add each variable as a **repository secret** (Settings → Secrets and variables →
Actions) — same names as the env vars above. Notes:

- Secrets are **not available to workflows from forked PRs** — those runs are skipped
  by an `if:` guard rather than failing.
- The GitHub approval test needs the container repo to be **public**.
- Free-tier agent/runner minutes apply; the approval tests are designed to reach the
  "waiting" gate *before* any runner/agent starts, so they cost ~nothing.
- Tests run `--test-threads=1` because a run's fixtures share one prefix.

### Required checks & merge gating

`main` has a **ruleset** ("Require tests on main", Settings → Rules → Rulesets) that
makes both **`build · test · clippy`** and **`live provider integration`** required.
What that means day-to-day:

- **On a PR** those two show as *Required*, and the **Merge** button stays disabled
  until both are green — nothing merges to `main` with failing or unrun tests.
- **Direct pushes to `main`** are effectively blocked (the commit would need already
  passing checks), so everything goes through the PR flow.
- **Fork PRs** skip the integration check (no secrets); GitHub treats a skipped
  required check as satisfied, so it doesn't block — a non-issue for your own branches.
- The ruleset uses **non-strict** status checks (`strict_required_status_checks_policy:
  false`), so a PR needn't be rebased onto the latest `main` before merging. Flip that
  to `true` if you want every PR up-to-date with `main` first.

### Gating releases on the suite

`release.yml` is autogenerated by cargo-dist and shouldn't be hand-edited, so releases
are gated by the same required checks: since releases are tagged from `main` and
nothing untested reaches `main`, released code has always passed the suite. Both CI
and Integration also re-run on the `v*` tag itself as a visible check.

### Sweeping leaked fixtures

A crashed test can leave a `forgetop-it-*` fixture behind. Set `FORGETOP_IT_SWEEP=1`
to delete all such leftovers before a run — the nightly CI run does this. Don't set it
for overlapping runs (it would delete another run's in-flight fixtures); normal runs
rely on each test's own teardown.

## Troubleshooting

| Symptom | Likely cause |
| --- | --- |
| `SKIP <provider>` | Its env vars are blank/absent — expected if you didn't set it up. |
| `401`/`403` on setup | Token missing a scope, or your user lacks admin/maintainer on the container. |
| GitHub gate never reaches "waiting" | Repo is **private** on a free plan (no environment required-reviewers), or the token user isn't the environment reviewer. |
| Azure pipeline never starts | No agent capacity — use a **public** project for free hosted agents. |
| Azure approval `A` missing / "view-only" | Expected — Azure surfaces the gate but its check isn't an actionable approval over the API; approve in the Azure UI. |
| `SKIP gitlab manual-job approval: CI can't run` | GitLab.com requires **account identity verification** (credit card/phone) before shared runners will run CI — validate at `https://gitlab.com/-/identity_verification`, or leave it (the other GitLab tests still run). |
| Leftover `forgetop-it-*` fixtures | A test panicked mid-run; the next run's sweeper removes them, or delete by hand. |
