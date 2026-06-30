using System.Net.Http.Headers;
using Forgetop.Core.Domain;
using Forgetop.Core.Http;
using Forgetop.Core.Providers;

namespace Forgetop.Providers.GitHub;

/// <summary>A live GitHub connection (PRs, Issues, Actions).</summary>
public sealed class GitHubConnection : IProviderConnection
{
    public GitHubConnection(Connection connection, GitHubApiClient client)
    {
        ConnectionId = connection.Id;
        DisplayName = connection.DisplayName;
        PullRequests = new GitHubPullRequestSource(client);
        WorkItems = new GitHubWorkItemSource(client);
        Pipelines = new GitHubPipelineSource(client);
    }

    public string ConnectionId { get; }
    public ProviderType ProviderType => ProviderType.GitHub;
    public string DisplayName { get; }
    public ProviderCapabilities Capabilities => GitHubProviderFactory.Caps;

    public IPullRequestSource? PullRequests { get; }
    public IWorkItemSource? WorkItems { get; }
    public IPipelineSource? Pipelines { get; }
}

/// <summary>Builds GitHub connections, configuring a PAT-authenticated <see cref="HttpClient"/>.</summary>
public sealed class GitHubProviderFactory : IProviderFactory
{
    internal static readonly ProviderCapabilities Caps = new()
    {
        SupportsPullRequests = true,
        SupportsWorkItems = true,
        SupportsPipelines = true,
        VoteStyle = VoteStyle.BinaryApprove,
        SupportsMerge = true,
        SupportsInlineComments = true,
        SupportsPipelineTrigger = true,
        SupportsPipelineDiscovery = true,
        Terminology = new Terminology { WorkItems = "Issues" },
    };

    public ProviderType ProviderType => ProviderType.GitHub;

    public ProviderCapabilities DescribeCapabilities() => Caps;

    public IProviderConnection Create(Connection connection, string? secret)
    {
        var owner = connection.Organization
            ?? throw new InvalidOperationException("GitHub connection requires an owner (Organization).");
        var repo = connection.Repository
            ?? throw new InvalidOperationException("GitHub connection requires a Repository.");

        var http = new HttpClient(new RetryHandler(new HttpClientHandler()))
        {
            BaseAddress = new Uri(connection.BaseUrl ?? "https://api.github.com/"),
        };
        http.DefaultRequestHeaders.UserAgent.ParseAdd("forgetop");
        http.DefaultRequestHeaders.Accept.ParseAdd("application/vnd.github+json");
        if (!string.IsNullOrEmpty(secret))
        {
            http.DefaultRequestHeaders.Authorization = new AuthenticationHeaderValue("Bearer", secret);
        }

        return new GitHubConnection(connection, new GitHubApiClient(http, owner, repo));
    }
}
