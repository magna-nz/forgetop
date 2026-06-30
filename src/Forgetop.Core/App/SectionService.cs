using Forgetop.Core.Configuration;
using Forgetop.Core.Providers;

namespace Forgetop.Core.App;

/// <summary>One pipeline connection feeding the Pipelines section, with its subscription.</summary>
public sealed record PipelineFeed(IProviderConnection Connection, IPipelineSource Source, PipelineSubscription Subscription);

/// <summary>
/// Resolves the live source(s) backing each section from the current bindings.
/// Returns null/empty when a section is unbound or its connection can't be built.
/// </summary>
public sealed class SectionService
{
    private readonly IConfigService _config;
    private readonly ConnectionResolver _resolver;

    public SectionService(IConfigService config, ConnectionResolver resolver)
    {
        _config = config ?? throw new ArgumentNullException(nameof(config));
        _resolver = resolver ?? throw new ArgumentNullException(nameof(resolver));
    }

    public async Task<IPullRequestSource?> GetPullRequestSourceAsync(CancellationToken ct = default)
    {
        var binding = _config.Current.PullRequests;
        if (binding is null)
        {
            return null;
        }

        var connection = await _resolver.ResolveAsync(binding.ConnectionId, ct).ConfigureAwait(false);
        return connection?.PullRequests;
    }

    public async Task<IWorkItemSource?> GetWorkItemSourceAsync(CancellationToken ct = default)
    {
        var binding = _config.Current.WorkItems;
        if (binding is null)
        {
            return null;
        }

        var connection = await _resolver.ResolveAsync(binding.ConnectionId, ct).ConfigureAwait(false);
        return connection?.WorkItems;
    }

    /// <summary>All pipeline feeds (multi-source) currently bound to the Pipelines section.</summary>
    public async Task<IReadOnlyList<PipelineFeed>> GetPipelineFeedsAsync(CancellationToken ct = default)
    {
        var binding = _config.Current.Pipelines;
        if (binding is null)
        {
            return [];
        }

        var feeds = new List<PipelineFeed>();
        foreach (var subscription in binding.Subscriptions)
        {
            var connection = await _resolver.ResolveAsync(subscription.ConnectionId, ct).ConfigureAwait(false);
            if (connection?.Pipelines is { } source)
            {
                feeds.Add(new PipelineFeed(connection, source, subscription));
            }
        }

        return feeds;
    }
}
