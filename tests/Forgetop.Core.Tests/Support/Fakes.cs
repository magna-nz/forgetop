using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Core.Tests.Support;

/// <summary>A provider connection with no real sources — used for registry tests.</summary>
public sealed class FakeProviderConnection : IProviderConnection
{
    public required string ConnectionId { get; init; }
    public required ProviderType ProviderType { get; init; }
    public required string DisplayName { get; init; }
    public required ProviderCapabilities Capabilities { get; init; }

    public IPullRequestSource? PullRequests => null;
    public IWorkItemSource? WorkItems => null;
    public IPipelineSource? Pipelines => null;
}

/// <summary>A factory whose capabilities are configurable per test.</summary>
public sealed class FakeProviderFactory : IProviderFactory
{
    private readonly ProviderCapabilities _capabilities;

    public FakeProviderFactory(ProviderType providerType, ProviderCapabilities capabilities)
    {
        ProviderType = providerType;
        _capabilities = capabilities;
    }

    public ProviderType ProviderType { get; }

    public ProviderCapabilities DescribeCapabilities() => _capabilities;

    public IProviderConnection Create(Connection connection, string? secret) => new FakeProviderConnection
    {
        ConnectionId = connection.Id,
        ProviderType = connection.ProviderType,
        DisplayName = connection.DisplayName,
        Capabilities = _capabilities,
    };
}

/// <summary>In-memory config store for service tests; records save count.</summary>
public sealed class RecordingConfigStore : IConfigStore
{
    private ForgetopConfig _config = ForgetopConfig.Empty;

    public int SaveCount { get; private set; }

    public Task<ForgetopConfig> LoadAsync(CancellationToken ct = default) => Task.FromResult(_config);

    public Task SaveAsync(ForgetopConfig config, CancellationToken ct = default)
    {
        _config = config;
        SaveCount++;
        return Task.CompletedTask;
    }
}
