using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Providers.Demo;

internal sealed class DemoPullRequestSource : IPullRequestSource
{
    public Task<IReadOnlyList<PullRequest>> ListAsync(PullRequestQuery query, CancellationToken ct = default)
    {
        IEnumerable<PullRequest> prs = DemoData.PullRequests;
        if (!query.IncludeCompleted)
        {
            prs = prs.Where(p => p.Status is PullRequestStatus.Open or PullRequestStatus.Draft);
        }

        return Task.FromResult<IReadOnlyList<PullRequest>>(prs.ToList());
    }

    public Task<PullRequest> GetAsync(string id, CancellationToken ct = default) =>
        Task.FromResult(DemoData.PullRequests.First(p => p.Id == id));

    public Task<IReadOnlyList<CommentThread>> GetThreadsAsync(string pullRequestId, CancellationToken ct = default) =>
        Task.FromResult<IReadOnlyList<CommentThread>>(
        [
            new CommentThread
            {
                Id = "t1",
                Comments =
                [
                    new Comment { Id = "c1", Author = DemoData.Bob, Body = "Looks good — one nit on the jitter.", CreatedAt = DateTimeOffset.UtcNow.AddHours(-2) },
                ],
            },
        ]);

    public Task AddCommentAsync(string pullRequestId, string body, CancellationToken ct = default) => Task.CompletedTask;
    public Task VoteAsync(string pullRequestId, ReviewVote vote, CancellationToken ct = default) => Task.CompletedTask;
    public Task MergeAsync(string pullRequestId, MergeOptions options, CancellationToken ct = default) => Task.CompletedTask;
}

internal sealed class DemoWorkItemSource : IWorkItemSource
{
    public Task<IReadOnlyList<WorkItem>> ListAsync(WorkItemQuery query, CancellationToken ct = default)
    {
        IEnumerable<WorkItem> items = DemoData.WorkItems;
        if (!query.IncludeCompleted)
        {
            items = items.Where(w => w.StateCategory is not (WorkItemStateCategory.Completed or WorkItemStateCategory.Canceled));
        }

        return Task.FromResult<IReadOnlyList<WorkItem>>(items.ToList());
    }

    public Task<WorkItem> GetAsync(string id, CancellationToken ct = default) =>
        Task.FromResult(DemoData.WorkItems.First(w => w.Id == id));

    public Task<IReadOnlyList<CommentThread>> GetThreadsAsync(string workItemId, CancellationToken ct = default) =>
        Task.FromResult<IReadOnlyList<CommentThread>>([]);

    public Task SetStateAsync(string workItemId, string state, CancellationToken ct = default) => Task.CompletedTask;
    public Task AddCommentAsync(string workItemId, string body, CancellationToken ct = default) => Task.CompletedTask;
}

internal sealed class DemoPipelineSource : IPipelineSource
{
    public Task<IReadOnlyList<PipelineDefinition>> DiscoverAsync(CancellationToken ct = default) =>
        Task.FromResult(DemoData.PipelineDefinitions);

    public Task<IReadOnlyList<PipelineRun>> ListRunsAsync(PipelineRunQuery query, CancellationToken ct = default)
    {
        IEnumerable<PipelineRun> runs = DemoData.PipelineRuns;
        if (query.DefinitionId is not null)
        {
            runs = runs.Where(r => r.DefinitionId == query.DefinitionId);
        }

        return Task.FromResult<IReadOnlyList<PipelineRun>>(runs.ToList());
    }

    public Task<PipelineRun> GetRunAsync(string runId, CancellationToken ct = default) =>
        Task.FromResult(DemoData.PipelineRuns.First(r => r.Id == runId));

    public Task<string> GetLogsAsync(string runId, string? jobId = null, CancellationToken ct = default) =>
        Task.FromResult($"[demo] logs for run {runId}{(jobId is null ? "" : $" job {jobId}")}\nAll steps completed.");

    public Task TriggerAsync(string definitionId, string? branch = null, CancellationToken ct = default) => Task.CompletedTask;
}
