using Forgetop.Core.Domain;

namespace Forgetop.Core.Configuration;

/// <summary>Binds the PRs section to a single connection.</summary>
public sealed record PullRequestBinding
{
    public required string ConnectionId { get; init; }
}

/// <summary>Binds the Work Items section to a single connection.</summary>
public sealed record WorkItemBinding
{
    public required string ConnectionId { get; init; }
}

/// <summary>
/// One connection feeding the Pipelines section, plus the pipelines subscribed
/// from it. When <see cref="AutoDiscoverAll"/> is true, all discovered pipelines
/// are shown and <see cref="DefinitionIds"/> is ignored.
/// </summary>
public sealed record PipelineSubscription
{
    public required string ConnectionId { get; init; }
    public IReadOnlyList<string> DefinitionIds { get; init; } = [];
    public bool AutoDiscoverAll { get; init; }
}

/// <summary>Binds the Pipelines section to one or more connections (multi-source).</summary>
public sealed record PipelineBinding
{
    public IReadOnlyList<PipelineSubscription> Subscriptions { get; init; } = [];
}

/// <summary>Persisted UI state.</summary>
public sealed record UiState
{
    public string? Theme { get; init; }
    public Section ActiveSection { get; init; } = Section.PullRequests;
}

/// <summary>Root persisted configuration.</summary>
public sealed record ForgetopConfig
{
    public IReadOnlyList<Connection> Connections { get; init; } = [];
    public PullRequestBinding? PullRequests { get; init; }
    public WorkItemBinding? WorkItems { get; init; }
    public PipelineBinding? Pipelines { get; init; }
    public UiState Ui { get; init; } = new();

    public static ForgetopConfig Empty { get; } = new();

    public Connection? FindConnection(string id) =>
        Connections.FirstOrDefault(c => c.Id == id);
}
