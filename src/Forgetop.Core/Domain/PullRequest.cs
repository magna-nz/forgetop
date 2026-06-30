namespace Forgetop.Core.Domain;

/// <summary>How a file changed in a pull request.</summary>
public enum FileChangeKind
{
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// <summary>One changed file in a pull request, optionally with a unified-diff patch.</summary>
public sealed record FileChange
{
    public required string Path { get; init; }
    public FileChangeKind Kind { get; init; }
    public int Additions { get; init; }
    public int Deletions { get; init; }

    /// <summary>Unified-diff patch text when the provider supplies it (GitHub does; ADO doesn't).</summary>
    public string? Patch { get; init; }
}

/// <summary>A reviewer on a pull request and their current vote.</summary>
public sealed record Reviewer
{
    public required User User { get; init; }
    public ReviewVote Vote { get; init; } = ReviewVote.NoVote;
    public bool IsRequired { get; init; }
}

/// <summary>Provider-neutral pull request / merge request.</summary>
public sealed record PullRequest
{
    public required string Id { get; init; }
    public int? Number { get; init; }
    public required string Title { get; init; }
    public string? Description { get; init; }
    public required User Author { get; init; }
    public PullRequestStatus Status { get; init; }
    public bool IsDraft { get; init; }

    public string? SourceRef { get; init; }
    public string? TargetRef { get; init; }

    public IReadOnlyList<Reviewer> Reviewers { get; init; } = [];

    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset? UpdatedAt { get; init; }
    public string? Url { get; init; }
}
