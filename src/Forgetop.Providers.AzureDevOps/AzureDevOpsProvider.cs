using System.Net.Http.Headers;
using System.Text;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Providers.AzureDevOps;

internal sealed class AzureDevOpsPullRequestSource(AzureDevOpsApiClient client) : IPullRequestSource
{
    public Task<IReadOnlyList<PullRequest>> ListAsync(PullRequestQuery query, CancellationToken ct = default) => client.ListPullRequestsAsync(query, ct);
    public Task<PullRequest> GetAsync(string id, CancellationToken ct = default) => client.GetPullRequestAsync(id, ct);
    public Task<IReadOnlyList<CommentThread>> GetThreadsAsync(string pullRequestId, CancellationToken ct = default) => client.GetPullRequestThreadsAsync(pullRequestId, ct);
    public Task AddCommentAsync(string pullRequestId, string body, CancellationToken ct = default) => client.AddPullRequestCommentAsync(pullRequestId, body, ct);
    public Task VoteAsync(string pullRequestId, ReviewVote vote, CancellationToken ct = default) => client.VoteAsync(pullRequestId, vote, ct);
    public Task MergeAsync(string pullRequestId, MergeOptions options, CancellationToken ct = default) => client.MergeAsync(pullRequestId, options, ct);
}

internal sealed class AzureDevOpsWorkItemSource(AzureDevOpsApiClient client) : IWorkItemSource
{
    public Task<IReadOnlyList<WorkItem>> ListAsync(WorkItemQuery query, CancellationToken ct = default) => client.ListWorkItemsAsync(query, ct);
    public Task<WorkItem> GetAsync(string id, CancellationToken ct = default) => client.GetWorkItemAsync(id, ct);
    public Task<IReadOnlyList<CommentThread>> GetThreadsAsync(string workItemId, CancellationToken ct = default) => Task.FromResult<IReadOnlyList<CommentThread>>([]);
    public Task SetStateAsync(string workItemId, string state, CancellationToken ct = default) => client.SetWorkItemStateAsync(workItemId, state, ct);
    public Task AddCommentAsync(string workItemId, string body, CancellationToken ct = default) => client.AddWorkItemCommentAsync(workItemId, body, ct);
}

internal sealed class AzureDevOpsPipelineSource(AzureDevOpsApiClient client) : IPipelineSource
{
    public Task<IReadOnlyList<PipelineDefinition>> DiscoverAsync(CancellationToken ct = default) => client.DiscoverAsync(ct);
    public Task<IReadOnlyList<PipelineRun>> ListRunsAsync(PipelineRunQuery query, CancellationToken ct = default) => client.ListRunsAsync(query, ct);
    public Task<PipelineRun> GetRunAsync(string runId, CancellationToken ct = default) => client.GetRunAsync(runId, ct);
    public Task<string> GetLogsAsync(string runId, string? jobId = null, CancellationToken ct = default) => client.GetLogsAsync(runId, jobId, ct);
    public Task TriggerAsync(string definitionId, string? branch = null, CancellationToken ct = default) => client.TriggerAsync(definitionId, branch, ct);
}

/// <summary>A live Azure DevOps connection (PRs, Work Items, Pipelines).</summary>
public sealed class AzureDevOpsConnection : IProviderConnection
{
    public AzureDevOpsConnection(Connection connection, AzureDevOpsApiClient client)
    {
        ConnectionId = connection.Id;
        DisplayName = connection.DisplayName;
        PullRequests = new AzureDevOpsPullRequestSource(client);
        WorkItems = new AzureDevOpsWorkItemSource(client);
        Pipelines = new AzureDevOpsPipelineSource(client);
    }

    public string ConnectionId { get; }
    public ProviderType ProviderType => ProviderType.AzureDevOps;
    public string DisplayName { get; }
    public ProviderCapabilities Capabilities => AzureDevOpsProviderFactory.Caps;

    public IPullRequestSource? PullRequests { get; }
    public IWorkItemSource? WorkItems { get; }
    public IPipelineSource? Pipelines { get; }
}

/// <summary>Builds Azure DevOps connections with PAT (Basic) auth.</summary>
public sealed class AzureDevOpsProviderFactory : IProviderFactory
{
    internal static readonly ProviderCapabilities Caps = new()
    {
        SupportsPullRequests = true,
        SupportsWorkItems = true,
        SupportsPipelines = true,
        VoteStyle = VoteStyle.NumericVotes,
        SupportsMerge = true,
        SupportsInlineComments = true,
        SupportsPipelineTrigger = true,
        SupportsPipelineDiscovery = true,
    };

    public ProviderType ProviderType => ProviderType.AzureDevOps;

    public ProviderCapabilities DescribeCapabilities() => Caps;

    public IProviderConnection Create(Connection connection, string? secret)
    {
        var org = connection.Organization
            ?? throw new InvalidOperationException("Azure DevOps connection requires an Organization.");
        var project = connection.Project
            ?? throw new InvalidOperationException("Azure DevOps connection requires a Project.");
        var repo = connection.Repository ?? project;

        var baseUrl = connection.BaseUrl ?? $"https://dev.azure.com/{org}/";
        var http = new HttpClient { BaseAddress = new Uri(baseUrl.EndsWith('/') ? baseUrl : baseUrl + "/") };
        if (!string.IsNullOrEmpty(secret))
        {
            var basic = Convert.ToBase64String(Encoding.ASCII.GetBytes(":" + secret));
            http.DefaultRequestHeaders.Authorization = new AuthenticationHeaderValue("Basic", basic);
        }

        return new AzureDevOpsConnection(connection, new AzureDevOpsApiClient(http, project, repo));
    }
}
