namespace Forgetop.Core.Domain;

/// <summary>A person on a provider (author, reviewer, assignee, …).</summary>
public sealed record User
{
    public required string Id { get; init; }
    public required string DisplayName { get; init; }
    public string? Handle { get; init; }
    public string? AvatarUrl { get; init; }
}

/// <summary>A repository / project a section may be scoped to.</summary>
public sealed record Repository
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public string? FullName { get; init; }
    public string? DefaultBranch { get; init; }
    public string? Url { get; init; }
}

/// <summary>A threaded comment on a pull request or work item.</summary>
public sealed record Comment
{
    public required string Id { get; init; }
    public required User Author { get; init; }
    public required string Body { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
}

/// <summary>
/// A discussion thread. For inline PR comments <see cref="FilePath"/> and
/// <see cref="Line"/> are set; for general discussion they are null.
/// </summary>
public sealed record CommentThread
{
    public required string Id { get; init; }
    public required IReadOnlyList<Comment> Comments { get; init; } = [];
    public string? FilePath { get; init; }
    public int? Line { get; init; }
    public bool IsResolved { get; init; }
}
