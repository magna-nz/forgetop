using Forgetop.Core.Domain;

namespace Forgetop.Providers.Demo;

/// <summary>Canned, deterministic data so <c>--demo</c> works with no credentials.</summary>
internal static class DemoData
{
    private static readonly DateTimeOffset Now = new(2026, 6, 30, 9, 0, 0, TimeSpan.Zero);

    public static readonly User Alice = new() { Id = "u1", DisplayName = "Alice Ng", Handle = "alice" };
    public static readonly User Bob = new() { Id = "u2", DisplayName = "Bob Reyes", Handle = "bob" };
    public static readonly User Carol = new() { Id = "u3", DisplayName = "Carol Diaz", Handle = "carol" };

    public static IReadOnlyList<PullRequest> PullRequests { get; } =
    [
        new PullRequest
        {
            Id = "101", Number = 101, Title = "Add retry policy to HTTP client",
            Description = "Wraps outbound calls in a Polly retry with jitter.\n\nReduces 5xx-related flakiness in the banking sync job.",
            Author = Alice, Status = PullRequestStatus.Open, SourceRef = "feature/retry", TargetRef = "main",
            CreatedAt = Now.AddHours(-30), UpdatedAt = Now.AddHours(-2),
            Reviewers =
            [
                new Reviewer { User = Bob, Vote = ReviewVote.Approved },
                new Reviewer { User = Carol, Vote = ReviewVote.WaitingForAuthor, IsRequired = true },
            ],
            Labels = ["banking", "enhancement"],
            Checks = CheckStatus.Passed,
            CheckSummary = new CheckSummary { Successful = 14, Neutral = 1 },
            Mergeable = MergeableState.Mergeable,
            ChangedFiles = 2, Additions = 24, Deletions = 2,
        },
        new PullRequest
        {
            Id = "102", Number = 102, Title = "Fix flaky pipeline cache key", Author = Bob,
            Description = "WIP — still narrowing down the cache key collision.",
            Status = PullRequestStatus.Draft, IsDraft = true, SourceRef = "fix/cache", TargetRef = "main",
            CreatedAt = Now.AddHours(-6),
            Reviewers = [new Reviewer { User = Alice, Vote = ReviewVote.NoVote }],
            Labels = ["wip", "ci"],
            Checks = CheckStatus.Pending,
            CheckSummary = new CheckSummary { Successful = 2, InProgress = 12, Neutral = 1 },
            Mergeable = MergeableState.Blocked,
            ChangedFiles = 3, Additions = 12, Deletions = 5,
        },
        new PullRequest
        {
            Id = "100", Number = 100, Title = "Bump dependencies", Author = Carol,
            Status = PullRequestStatus.Merged, SourceRef = "chore/bump", TargetRef = "main",
            CreatedAt = Now.AddDays(-3), UpdatedAt = Now.AddDays(-2),
            Labels = ["chore"],
            Checks = CheckStatus.Passed,
            Mergeable = MergeableState.Unknown,
            ChangedFiles = 8, Additions = 120, Deletions = 60,
        },
    ];

    public static IReadOnlyList<WorkItem> WorkItems { get; } =
    [
        new WorkItem
        {
            Id = "w1", Identifier = "FOR-12", Title = "Design the provider abstraction",
            State = "In Progress", StateCategory = WorkItemStateCategory.Started, Type = "Story",
            Assignee = Alice, CreatedAt = Now.AddDays(-5), UpdatedAt = Now.AddHours(-3),
        },
        new WorkItem
        {
            Id = "w2", Identifier = "FOR-13", Title = "Pipeline auto-discovery",
            State = "Todo", StateCategory = WorkItemStateCategory.Unstarted, Type = "Task",
            Assignee = Bob, CreatedAt = Now.AddDays(-4),
        },
        new WorkItem
        {
            Id = "w3", Identifier = "FOR-9", Title = "Spike: Terminal.Gui v2",
            State = "Done", StateCategory = WorkItemStateCategory.Completed, Type = "Spike",
            Assignee = Carol, CreatedAt = Now.AddDays(-9), UpdatedAt = Now.AddDays(-6),
        },
    ];

    public static IReadOnlyList<PipelineDefinition> PipelineDefinitions { get; } =
    [
        new PipelineDefinition { Id = "ci", Name = "CI", Path = ".github/workflows/ci.yml" },
        new PipelineDefinition { Id = "release", Name = "Release", Path = ".github/workflows/release.yml" },
    ];

    public static IReadOnlyList<PipelineRun> PipelineRuns { get; } =
    [
        new PipelineRun
        {
            Id = "r501", DefinitionId = "ci", Number = 501, Name = "CI", Status = PipelineRunStatus.Running,
            TriggeredBy = Alice, Branch = "feature/retry", CommitSha = "a1b2c3d", StartedAt = Now.AddMinutes(-4),
            Stages =
            [
                new PipelineStage
                {
                    Name = "build", Status = PipelineRunStatus.Succeeded,
                    Jobs = [new PipelineJob { Id = "j1", Name = "compile", Status = PipelineRunStatus.Succeeded }],
                },
                new PipelineStage
                {
                    Name = "test", Status = PipelineRunStatus.Running,
                    Jobs = [new PipelineJob { Id = "j2", Name = "unit", Status = PipelineRunStatus.Running }],
                },
            ],
        },
        new PipelineRun
        {
            Id = "r500", DefinitionId = "ci", Number = 500, Name = "CI", Status = PipelineRunStatus.Failed,
            TriggeredBy = Bob, Branch = "main", CommitSha = "9f8e7d6", StartedAt = Now.AddHours(-1),
            FinishedAt = Now.AddMinutes(-52),
            Stages =
            [
                new PipelineStage
                {
                    Name = "build", Status = PipelineRunStatus.Succeeded,
                    Jobs = [new PipelineJob { Id = "j10", Name = "compile", Status = PipelineRunStatus.Succeeded }],
                },
                new PipelineStage
                {
                    Name = "test", Status = PipelineRunStatus.Failed,
                    Jobs =
                    [
                        new PipelineJob { Id = "j11", Name = "unit", Status = PipelineRunStatus.Succeeded },
                        new PipelineJob { Id = "j12", Name = "integration", Status = PipelineRunStatus.Failed },
                    ],
                },
            ],
        },
        new PipelineRun
        {
            Id = "r207", DefinitionId = "release", Number = 207, Name = "Release", Status = PipelineRunStatus.Succeeded,
            TriggeredBy = Carol, Branch = "main", CommitSha = "1234abc", StartedAt = Now.AddDays(-2),
            FinishedAt = Now.AddDays(-2).AddMinutes(8),
            Stages =
            [
                new PipelineStage
                {
                    Name = "publish", Status = PipelineRunStatus.Succeeded,
                    Jobs =
                    [
                        new PipelineJob { Id = "j20", Name = "pack", Status = PipelineRunStatus.Succeeded },
                        new PipelineJob { Id = "j21", Name = "deploy", Status = PipelineRunStatus.Succeeded },
                    ],
                },
            ],
        },
    ];
}
