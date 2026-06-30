using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Providers.GitHub;

internal sealed class GitHubPullRequestSource(GitHubApiClient client) : IPullRequestSource
{
    public Task<IReadOnlyList<PullRequest>> ListAsync(PullRequestQuery query, CancellationToken ct = default) => client.ListPullRequestsAsync(query, ct);
    public Task<PullRequest> GetAsync(string id, CancellationToken ct = default) => client.GetPullRequestAsync(id, ct);
    public Task<IReadOnlyList<CommentThread>> GetThreadsAsync(string pullRequestId, CancellationToken ct = default) => client.GetPullRequestThreadsAsync(pullRequestId, ct);
    public Task<IReadOnlyList<FileChange>> GetChangesAsync(string pullRequestId, CancellationToken ct = default) => client.GetChangesAsync(pullRequestId, ct);
    public Task AddCommentAsync(string pullRequestId, string body, CancellationToken ct = default) => client.AddPullRequestCommentAsync(pullRequestId, body, ct);
    public Task VoteAsync(string pullRequestId, ReviewVote vote, CancellationToken ct = default) => client.VoteAsync(pullRequestId, vote, ct);
    public Task MergeAsync(string pullRequestId, MergeOptions options, CancellationToken ct = default) => client.MergeAsync(pullRequestId, options, ct);
}

internal sealed class GitHubWorkItemSource(GitHubApiClient client) : IWorkItemSource
{
    public Task<IReadOnlyList<WorkItem>> ListAsync(WorkItemQuery query, CancellationToken ct = default) => client.ListIssuesAsync(query, ct);
    public Task<WorkItem> GetAsync(string id, CancellationToken ct = default) => client.GetIssueAsync(id, ct);
    public Task<IReadOnlyList<CommentThread>> GetThreadsAsync(string workItemId, CancellationToken ct = default) => client.GetPullRequestThreadsAsync(workItemId, ct);
    public Task SetStateAsync(string workItemId, string state, CancellationToken ct = default) => client.SetIssueStateAsync(workItemId, state, ct);
    public Task AddCommentAsync(string workItemId, string body, CancellationToken ct = default) => client.AddIssueCommentAsync(workItemId, body, ct);
}

internal sealed class GitHubPipelineSource(GitHubApiClient client) : IPipelineSource
{
    public Task<IReadOnlyList<PipelineDefinition>> DiscoverAsync(CancellationToken ct = default) => client.DiscoverWorkflowsAsync(ct);
    public Task<IReadOnlyList<PipelineRun>> ListRunsAsync(PipelineRunQuery query, CancellationToken ct = default) => client.ListRunsAsync(query, ct);
    public Task<PipelineRun> GetRunAsync(string runId, CancellationToken ct = default) => client.GetRunAsync(runId, ct);
    public Task<string> GetLogsAsync(string runId, string? jobId = null, CancellationToken ct = default) => client.GetLogsAsync(runId, jobId, ct);
    public Task TriggerAsync(string definitionId, string? branch = null, CancellationToken ct = default) => client.TriggerAsync(definitionId, branch, ct);
}
