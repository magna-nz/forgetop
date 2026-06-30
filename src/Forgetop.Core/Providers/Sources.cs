using Forgetop.Core.Domain;

namespace Forgetop.Core.Providers;

/// <summary>Pull-request capability. Implemented only by connections that support PRs.</summary>
public interface IPullRequestSource
{
    Task<IReadOnlyList<PullRequest>> ListAsync(PullRequestQuery query, CancellationToken ct = default);
    Task<PullRequest> GetAsync(string id, CancellationToken ct = default);
    Task<IReadOnlyList<CommentThread>> GetThreadsAsync(string pullRequestId, CancellationToken ct = default);
    Task AddCommentAsync(string pullRequestId, string body, CancellationToken ct = default);
    Task VoteAsync(string pullRequestId, ReviewVote vote, CancellationToken ct = default);
    Task MergeAsync(string pullRequestId, MergeOptions options, CancellationToken ct = default);
}

/// <summary>Work-item capability. Implemented only by connections that support work items.</summary>
public interface IWorkItemSource
{
    Task<IReadOnlyList<WorkItem>> ListAsync(WorkItemQuery query, CancellationToken ct = default);
    Task<WorkItem> GetAsync(string id, CancellationToken ct = default);
    Task<IReadOnlyList<CommentThread>> GetThreadsAsync(string workItemId, CancellationToken ct = default);
    Task SetStateAsync(string workItemId, string state, CancellationToken ct = default);
    Task AddCommentAsync(string workItemId, string body, CancellationToken ct = default);
}

/// <summary>Pipeline / CI capability. Implemented only by connections that support pipelines.</summary>
public interface IPipelineSource
{
    /// <summary>List the pipelines/workflows available on this connection (for subscription).</summary>
    Task<IReadOnlyList<PipelineDefinition>> DiscoverAsync(CancellationToken ct = default);

    Task<IReadOnlyList<PipelineRun>> ListRunsAsync(PipelineRunQuery query, CancellationToken ct = default);
    Task<PipelineRun> GetRunAsync(string runId, CancellationToken ct = default);

    /// <summary>Raw logs for a run, or a single job within it when <paramref name="jobId"/> is set.</summary>
    Task<string> GetLogsAsync(string runId, string? jobId = null, CancellationToken ct = default);

    Task TriggerAsync(string definitionId, string? branch = null, CancellationToken ct = default);
}
