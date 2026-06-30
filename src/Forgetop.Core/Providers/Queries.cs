using Forgetop.Core.Domain;

namespace Forgetop.Core.Providers;

/// <summary>Which slice of pull requests to list.</summary>
public enum PullRequestFilter
{
    All,
    Mine,
    ReviewRequested,
}

/// <summary>Query for listing pull requests.</summary>
public sealed record PullRequestQuery
{
    public PullRequestFilter Filter { get; init; } = PullRequestFilter.All;
    public bool IncludeCompleted { get; init; }
    public int? Limit { get; init; }
}

/// <summary>Query for listing work items.</summary>
public sealed record WorkItemQuery
{
    public bool MineOnly { get; init; }
    public bool IncludeCompleted { get; init; }
    public int? Limit { get; init; }
}

/// <summary>Query for listing pipeline runs, optionally scoped to one definition.</summary>
public sealed record PipelineRunQuery
{
    public string? DefinitionId { get; init; }
    public string? Branch { get; init; }
    public int? Limit { get; init; }
}

/// <summary>How a pull request should be merged.</summary>
public enum MergeStrategy
{
    Merge,
    Squash,
    Rebase,
}

/// <summary>Options for merging a pull request.</summary>
public sealed record MergeOptions
{
    public MergeStrategy Strategy { get; init; } = MergeStrategy.Merge;
    public bool DeleteSourceRef { get; init; }
}
