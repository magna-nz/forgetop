using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Providers.Demo;

/// <summary>A fully-featured in-memory connection backing <c>--demo</c>.</summary>
public sealed class DemoConnection : IProviderConnection
{
    public DemoConnection(string connectionId, string displayName)
    {
        ConnectionId = connectionId;
        DisplayName = displayName;
    }

    public string ConnectionId { get; }
    public ProviderType ProviderType => ProviderType.Demo;
    public string DisplayName { get; }
    public ProviderCapabilities Capabilities => DemoProviderFactory.Caps;

    public IPullRequestSource? PullRequests { get; } = new DemoPullRequestSource();
    public IWorkItemSource? WorkItems { get; } = new DemoWorkItemSource();
    public IPipelineSource? Pipelines { get; } = new DemoPipelineSource();
}

/// <summary>Factory for the Demo provider (needs no credentials).</summary>
public sealed class DemoProviderFactory : IProviderFactory
{
    internal static readonly ProviderCapabilities Caps = new()
    {
        SupportsPullRequests = true,
        SupportsWorkItems = true,
        SupportsPipelines = true,
        SupportsMerge = true,
        SupportsInlineComments = true,
        SupportsPipelineTrigger = true,
        SupportsPipelineDiscovery = true,
    };

    public ProviderType ProviderType => ProviderType.Demo;

    public ProviderCapabilities DescribeCapabilities() => Caps;

    public IProviderConnection Create(Connection connection, string? secret) =>
        new DemoConnection(connection.Id, connection.DisplayName);
}
