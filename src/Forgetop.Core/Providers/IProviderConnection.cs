using Forgetop.Core.Domain;

namespace Forgetop.Core.Providers;

/// <summary>
/// A live, authenticated connection to a provider. Exposes only the source
/// capabilities it actually supports — unsupported sources are null, and
/// <see cref="Capabilities"/> describes the same surface declaratively.
/// </summary>
public interface IProviderConnection
{
    string ConnectionId { get; }
    ProviderType ProviderType { get; }
    string DisplayName { get; }
    ProviderCapabilities Capabilities { get; }

    IPullRequestSource? PullRequests { get; }
    IWorkItemSource? WorkItems { get; }
    IPipelineSource? Pipelines { get; }
}
