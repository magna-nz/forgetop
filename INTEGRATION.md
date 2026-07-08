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

## 3. Run it

```sh
cargo test -p forgetop-providers --features integration -- --nocapture --test-threads=1
```

`--test-threads=1` keeps provider output readable and avoids fixtures from parallel
tests colliding. Watch for lines like `github: connected to you/forgetop-itest`.

## 4. Running in CI

Add the same variables as **repository secrets** (Settings → Secrets and variables →
Actions), then the `integration` workflow injects them as env. Notes:

- Secrets are **not available to workflows from forked PRs** — those runs skip the
  suite rather than fail.
- The GitHub approval test needs the container repo to be **public**.
- Free-tier agent/runner minutes apply; the approval tests are designed to reach the
  "waiting" gate *before* any runner/agent starts, so they cost ~nothing.

## Troubleshooting

| Symptom | Likely cause |
| --- | --- |
| `SKIP <provider>` | Its env vars are blank/absent — expected if you didn't set it up. |
| `401`/`403` on setup | Token missing a scope, or your user lacks admin/maintainer on the container. |
| GitHub gate never reaches "waiting" | Repo is **private** on a free plan (no environment required-reviewers), or the token user isn't the environment reviewer. |
| Azure pipeline never starts | No agent capacity — use a **public** project for free hosted agents. |
| Leftover `forgetop-it-*` fixtures | A test panicked mid-run; the next run's sweeper removes them, or delete by hand. |
