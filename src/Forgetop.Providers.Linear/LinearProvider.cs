using Forgetop.Core.Domain;
using Forgetop.Core.Http;
using Forgetop.Core.Providers;

namespace Forgetop.Providers.Linear;

internal sealed class LinearWorkItemSource(LinearApiClient client) : IWorkItemSource
{
    public Task<IReadOnlyList<WorkItem>> ListAsync(WorkItemQuery query, CancellationToken ct = default) => client.ListIssuesAsync(query, ct);
    public Task<WorkItem> GetAsync(string id, CancellationToken ct = default) => client.GetIssueAsync(id, ct);
    public Task<IReadOnlyList<CommentThread>> GetThreadsAsync(string workItemId, CancellationToken ct = default) => client.GetThreadsAsync(workItemId, ct);
    public Task SetStateAsync(string workItemId, string state, CancellationToken ct = default) => client.SetStateAsync(workItemId, state, ct);
    public Task AddCommentAsync(string workItemId, string body, CancellationToken ct = default) => client.AddCommentAsync(workItemId, body, ct);
}

/// <summary>A live Linear connection — work items only.</summary>
public sealed class LinearConnection : IProviderConnection
{
    public LinearConnection(Connection connection, LinearApiClient client)
    {
        ConnectionId = connection.Id;
        DisplayName = connection.DisplayName;
        _client = client;
        WorkItems = new LinearWorkItemSource(client);
    }

    public string ConnectionId { get; }
    public ProviderType ProviderType => ProviderType.Linear;
    public string DisplayName { get; }
    public ProviderCapabilities Capabilities => LinearProviderFactory.Caps;

    public IPullRequestSource? PullRequests => null;
    public IWorkItemSource? WorkItems { get; }
    public IPipelineSource? Pipelines => null;

    private readonly LinearApiClient _client;
    public Task<bool> CheckAsync(CancellationToken ct = default) => _client.CheckAsync(ct);
}

/// <summary>Builds Linear connections authenticated with a personal API key.</summary>
public sealed class LinearProviderFactory : IProviderFactory
{
    internal static readonly ProviderCapabilities Caps = new()
    {
        SupportsWorkItems = true,
        Terminology = new Terminology { WorkItems = "Issues" },
    };

    public ProviderType ProviderType => ProviderType.Linear;

    public ProviderCapabilities DescribeCapabilities() => Caps;

    public IProviderConnection Create(Connection connection, string? secret)
    {
        var http = new HttpClient(new RetryHandler(new HttpClientHandler()))
        {
            BaseAddress = new Uri(connection.BaseUrl ?? "https://api.linear.app/graphql"),
        };
        if (!string.IsNullOrEmpty(secret))
        {
            // Linear personal API keys are sent as the raw Authorization value (no scheme).
            http.DefaultRequestHeaders.TryAddWithoutValidation("Authorization", secret);
        }

        return new LinearConnection(connection, new LinearApiClient(http));
    }
}
