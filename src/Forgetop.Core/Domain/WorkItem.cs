namespace Forgetop.Core.Domain;

/// <summary>Provider-neutral work item / issue (GitHub Issue, ADO Work Item, Linear issue).</summary>
public sealed record WorkItem
{
    public required string Id { get; init; }
    public string? Identifier { get; init; }
    public required string Title { get; init; }
    public string? Description { get; init; }

    /// <summary>The provider's own state label, e.g. "In Progress", "Done".</summary>
    public required string State { get; init; }

    /// <summary>The provider-neutral bucket <see cref="State"/> maps onto.</summary>
    public WorkItemStateCategory StateCategory { get; init; }

    public string? Type { get; init; }
    public User? Assignee { get; init; }

    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset? UpdatedAt { get; init; }
    public string? Url { get; init; }
}
