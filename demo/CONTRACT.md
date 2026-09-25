# Public demo contract

This is a standalone, browser-only taste test. It intentionally does not import or call the
locally runnable `forgetop --demo` implementation.

## Initial experience

- Five healthy sample connections: GitHub, GitLab, Bitbucket, Linear, and Jira.
- Command Center, pull requests, work items, pipelines, and notifications match the visible
  dashboard behaviour of the local demo.
- Lists, filters, sorting, command palette, detail panes, diffs, checks, timelines, pipeline
  logs, unread notifications, sidebar controls, and theme selection are all in browser memory.

## Simulated actions

- PR comments, replies, reviews, approvals/requested changes, merges, and revert feedback.
- Work-item comments, assignment, title/description edits, and state-transition feedback.
- Pipeline cancellation and deterministic logs.
- Notification mark-read and drill-in.

All mutations are simulated locally. They never reach a provider or an application backend.

## Deliberate public-demo differences

- No setup wizard, connection management, credential fields, provider tests, OS keychain,
  startup preferences, dashboard token, real links to sample providers, or real write actions.
- No fetch/XHR/WebSocket calls, cookies, localStorage, sessionStorage, IndexedDB, telemetry, or
  uploads.
- **Reset demo** and a full browser refresh create a fresh deep-cloned fixture store.
