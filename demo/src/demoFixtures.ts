import type { DemoState } from "./demoTypes";

const sam = { id: "sam", name: "Sam Rivera", handle: "@sam", initials: "SR" };
const priya = { id: "priya", name: "Priya Nair", handle: "@priya", initials: "PN" };
const marcus = { id: "marcus", name: "Marcus Lee", handle: "@marcus", initials: "ML" };
const elena = { id: "elena", name: "Elena Sokolova", handle: "@elena", initials: "ES" };
const tom = { id: "tom", name: "Tom Becker", handle: "@tom", initials: "TB" };

/** Curated, realistic data for the public preview. It is never fetched or persisted. */
export const demoFixture: DemoState = {
  currentUser: sam,
  people: [sam, priya, marcus, elena, tom],
  connections: [
    { id: "github", provider: "GitHub", name: "northwind/payments", healthy: true },
    { id: "gitlab", provider: "GitLab", name: "northwind-infra", healthy: true },
    { id: "bitbucket", provider: "Bitbucket", name: "northwind-data", healthy: true },
    { id: "linear", provider: "Linear", name: "Engineering", healthy: true },
    { id: "jira", provider: "Jira", name: "Operations", healthy: true },
  ],
  pullRequests: [
    {
      id: "github-1501", provider: "GitHub", repository: "northwind/payments", number: 1501,
      title: "Refactor the webhook retry queue", author: marcus, status: "open", sourceBranch: "refactor/webhook-retry", targetBranch: "main",
      description: "Moves delivery retries onto a dedicated queue with jittered exponential backoff. Review focus: the backoff maths and the dead-letter cutoff.",
      labels: ["reliability", "needs review"], checks: "passing", mergeable: true, additions: 210, deletions: 64, changedFiles: 8, updatedAt: "38 minutes ago",
      reviewers: [{ reviewer: priya, vote: "approved", summary: "Queue split looks good.", at: "2 hours ago" }, { reviewer: sam, vote: "pending" }],
      comments: [{ id: "pr-comment-1", author: priya, body: "Could we add one metric for retries exhausted?", createdAt: "2 hours ago" }],
      timeline: [{ id: "pr-event-1", kind: "review", actor: priya, message: "approved these changes", at: "2 hours ago" }, { id: "pr-event-2", kind: "comment", actor: marcus, message: "opened this pull request", at: "Yesterday" }],
      files: [{ path: "src/retry/queue.ts", additions: 78, deletions: 22, patch: "+ export class RetryQueue {\n+   async schedule(job: DeliveryJob) {\n+     return this.enqueue(job, jitteredBackoff(job.attempt));\n+   }\n+ }" }, { path: "src/retry/policy.ts", additions: 42, deletions: 8, patch: "+ export function jitteredBackoff(attempt: number) {\n+   return Math.min(300_000, 1_000 * 2 ** attempt) * (0.75 + Math.random() * 0.5);\n+ }" }],
    },
    {
      id: "github-1487", provider: "GitHub", repository: "northwind/payments", number: 1487,
      title: "Add idempotency keys to the payments API", author: sam, status: "open", sourceBranch: "feat/idempotency-keys", targetBranch: "main",
      description: "Adds client-supplied idempotency keys to charge and refund requests so a retry cannot double-charge a customer.",
      labels: ["payments", "api"], checks: "passing", mergeable: true, additions: 132, deletions: 18, changedFiles: 6, updatedAt: "3 hours ago",
      reviewers: [{ reviewer: priya, vote: "approved", summary: "Nice, especially the conflict response.", at: "1 hour ago" }, { reviewer: marcus, vote: "approved", at: "45 minutes ago" }],
      comments: [], timeline: [{ id: "pr-event-3", kind: "review", actor: marcus, message: "approved these changes", at: "45 minutes ago" }],
      files: [{ path: "src/http/idempotency.ts", additions: 63, deletions: 0, patch: "+ export async function withIdempotencyKey(key: string, request: Request) {\n+   return cache.getOrSet(key, request);\n+ }" }],
    },
    {
      id: "github-1492", provider: "GitHub", repository: "northwind/payments", number: 1492,
      title: "Bump Next.js to 14.2.5", author: sam, status: "open", sourceBranch: "chore/next-14", targetBranch: "main",
      description: "Routine upgrade with a failing visual-regression job to investigate.", labels: ["frontend", "dependencies"], checks: "failing", mergeable: false, additions: 40, deletions: 12, changedFiles: 3, updatedAt: "5 hours ago",
      reviewers: [{ reviewer: marcus, vote: "pending" }], comments: [], timeline: [{ id: "pr-event-4", kind: "comment", actor: sam, message: "requested a review", at: "5 hours ago" }],
      files: [{ path: "package.json", additions: 2, deletions: 2, patch: "- \"next\": \"14.2.4\"\n+ \"next\": \"14.2.5\"" }],
    },
    {
      id: "gitlab-318", provider: "GitLab", repository: "northwind-infra", number: 318,
      title: "Rotate the KMS signing keys", author: priya, status: "open", sourceBranch: "security/kms-rotation", targetBranch: "main",
      description: "Adds the new signing-key version with a safe verification overlap.", labels: ["security"], checks: "passing", mergeable: true, additions: 22, deletions: 8, changedFiles: 4, updatedAt: "10 hours ago",
      reviewers: [{ reviewer: sam, vote: "pending" }], comments: [], timeline: [{ id: "pr-event-5", kind: "comment", actor: priya, message: "requested your review", at: "10 hours ago" }],
      files: [{ path: "terraform/kms.tf", additions: 22, deletions: 8, patch: "+ resource \"aws_kms_key\" \"jwt_signing_v2\" {\n+   enable_key_rotation = true\n+ }" }],
    },
    {
      id: "bitbucket-64", provider: "Bitbucket", repository: "northwind-data", number: 64,
      title: "dbt: add revenue recognition model", author: sam, status: "open", sourceBranch: "feat/rev-rec", targetBranch: "main",
      description: "Adds the monthly revenue-recognition model, pending finance confirmation of the deferral schedule.", labels: ["dbt", "blocked"], checks: "passing", mergeable: false, additions: 180, deletions: 12, changedFiles: 5, updatedAt: "Yesterday",
      reviewers: [{ reviewer: tom, vote: "changes_requested", summary: "Please document the deferral assumption.", at: "Yesterday" }], comments: [], timeline: [{ id: "pr-event-6", kind: "review", actor: tom, message: "requested changes", at: "Yesterday" }],
      files: [{ path: "models/rev_rec_monthly.sql", additions: 96, deletions: 0, patch: "+ select month, sum(recognized_revenue) as revenue\n+ from invoice_schedule\n+ group by 1" }],
    },
    {
      id: "github-1450", provider: "GitHub", repository: "northwind/payments", number: 1450,
      title: "Cache the customer risk score", author: sam, status: "merged", sourceBranch: "perf/risk-score-cache", targetBranch: "main",
      description: "Caches computed risk scores for five minutes to reduce charge-path latency.", labels: ["performance"], checks: "passing", mergeable: false, additions: 63, deletions: 18, changedFiles: 4, updatedAt: "Yesterday",
      reviewers: [{ reviewer: priya, vote: "approved", at: "Yesterday" }], comments: [], timeline: [{ id: "pr-event-7", kind: "merge", actor: sam, message: "merged this pull request", at: "Yesterday" }],
      files: [{ path: "src/risk/cache.ts", additions: 63, deletions: 18, patch: "+ const ttl = 5 * 60_000;" }],
    },
  ],
  workItems: [
    { id: "github-842", provider: "GitHub", project: "northwind/payments", identifier: "#842", title: "Investigate elevated p99 on POST /charge", description: "p99 has climbed from 180ms to 600ms. Trace a slow request and confirm whether the risk-score cache is hit.", type: "Bug", category: "in_progress", state: "In Progress", assignee: sam, labels: ["performance", "urgent"], updatedAt: "3 hours ago", comments: [{ id: "wi-comment-1", author: marcus, body: "The trace samples point at a cache miss on the fallback path.", createdAt: "1 hour ago" }], timeline: [{ id: "wi-event-1", kind: "assignment", actor: sam, message: "assigned this to Sam Rivera", at: "Today" }] },
    { id: "github-851", provider: "GitHub", project: "northwind/payments", identifier: "#851", title: "Add retry-budget metrics to the sync worker", description: "Expose retries used and retry budget, then alert on consistent exhaustion.", type: "Task", category: "todo", state: "Todo", assignee: sam, labels: ["observability"], updatedAt: "Yesterday", comments: [], timeline: [] },
    { id: "linear-231", provider: "Linear", project: "Engineering", identifier: "ENG-231", title: "Design the ledger reconciliation job", description: "Define matching keys and an acceptable tolerance for processor settlement mismatches.", type: "Story", category: "in_progress", state: "In Progress", assignee: sam, labels: ["payments", "design"], updatedAt: "4 hours ago", comments: [], timeline: [] },
    { id: "linear-245", provider: "Linear", project: "Engineering", identifier: "ENG-245", title: "Spike: event sourcing for the payments ledger", description: "Timeboxed evaluation of append-only events and projections.", type: "Spike", category: "todo", state: "Todo", assignee: sam, labels: ["architecture"], updatedAt: "Yesterday", comments: [], timeline: [] },
    { id: "linear-198", provider: "Linear", project: "Engineering", identifier: "ENG-198", title: "Migrate feature flags to OpenFeature", description: "Migrate call sites after the platform provider is published.", type: "Story", category: "blocked", state: "Blocked", assignee: sam, labels: ["platform"], updatedAt: "2 days ago", comments: [], timeline: [] },
    { id: "jira-1423", provider: "Jira", project: "Operations", identifier: "OPS-1423", title: "SOC2: collect access-review evidence for Q3", description: "Export system access lists and collect owner sign-off before the auditor window.", type: "Task", category: "in_progress", state: "In Progress", assignee: sam, labels: ["compliance"], updatedAt: "2 hours ago", comments: [], timeline: [] },
  ],
  pipelines: [
    { id: "pipe-9142", provider: "GitHub", project: "northwind/payments", name: "payments / deploy", runNumber: 9142, branch: "main", commit: "a1b2c3d", status: "failed", triggeredBy: sam, startedAt: "18 minutes ago", jobs: [{ name: "unit tests", status: "passed", duration: "2m 14s" }, { name: "visual regression", status: "failed", duration: "1m 08s" }, { name: "deploy preview", status: "cancelled", duration: "—" }], logs: ["18:04:01 Starting visual regression", "18:04:45 Snapshot mismatch: checkout-summary", "18:05:09 Process completed with exit code 1"] },
    { id: "pipe-9138", provider: "GitLab", project: "northwind-infra", name: "infrastructure / plan", runNumber: 9138, branch: "security/kms-rotation", commit: "e4f5a6b", status: "running", triggeredBy: priya, startedAt: "9 minutes ago", jobs: [{ name: "validate", status: "passed", duration: "34s" }, { name: "terraform plan", status: "running", duration: "—" }], logs: ["18:13:20 Validating modules", "18:13:54 Plan: 2 to add, 0 to change, 0 to destroy", "18:14:02 Awaiting plan output"] },
    { id: "pipe-9124", provider: "Bitbucket", project: "northwind-data", name: "warehouse / dbt", runNumber: 9124, branch: "main", commit: "9c8d7e6", status: "passed", triggeredBy: tom, startedAt: "Yesterday", jobs: [{ name: "dbt test", status: "passed", duration: "4m 11s" }, { name: "publish docs", status: "passed", duration: "23s" }], logs: ["09:41:11 dbt test complete", "09:45:22 Published docs", "09:45:34 Pipeline succeeded"] },
  ],
  notifications: [
    { id: "notice-1", provider: "GitHub", target: "pull_request", targetId: "github-1501", title: "Review requested", context: "Marcus Lee requested your review on Refactor the webhook retry queue", updatedAt: "38 minutes ago", unread: true },
    { id: "notice-2", provider: "GitHub", target: "work_item", targetId: "github-842", title: "New comment", context: "Marcus Lee commented on #842", updatedAt: "1 hour ago", unread: true },
    { id: "notice-3", provider: "GitLab", target: "pipeline", targetId: "pipe-9138", title: "Pipeline running", context: "infrastructure / plan is waiting for plan output", updatedAt: "9 minutes ago", unread: true },
    { id: "notice-4", provider: "Bitbucket", target: "pull_request", targetId: "bitbucket-64", title: "Changes requested", context: "Tom Becker requested changes on dbt: add revenue recognition model", updatedAt: "Yesterday", unread: false },
  ],
  launchpad: { reviewRequested: ["github-1501", "gitlab-318"], needsAttention: ["github-1492", "bitbucket-64"], assignedWork: ["github-842", "github-851", "linear-231", "linear-245", "jira-1423"], pipelineAlerts: ["pipe-9142"] },
  lastAction: null,
};
