using System.Net.Http.Json;
using System.Text.Json;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Providers.GitHub;

/// <summary>
/// Thin REST client over the GitHub API. Parses responses and maps them to the
/// domain via <see cref="GitHubMapper"/>; no domain JSON escapes this type.
/// </summary>
public sealed class GitHubApiClient
{
    private readonly HttpClient _http;
    private readonly string _owner;
    private readonly string _repo;
    private string? _selfLogin;

    public GitHubApiClient(HttpClient http, string owner, string repo)
    {
        _http = http ?? throw new ArgumentNullException(nameof(http));
        _owner = owner;
        _repo = repo;
    }

    private string Repo => $"repos/{_owner}/{_repo}";

    public async Task<IReadOnlyList<PullRequest>> ListPullRequestsAsync(PullRequestQuery query, CancellationToken ct)
    {
        var state = query.IncludeCompleted ? "all" : "open";
        using var doc = await GetAsync($"{Repo}/pulls?state={state}&per_page={query.Limit ?? 50}", ct).ConfigureAwait(false);
        var prs = doc.RootElement.EnumerateArray().Select(GitHubMapper.MapPullRequest).ToList();

        var me = query.Filter == PullRequestFilter.All ? null : await GetSelfLoginAsync(ct).ConfigureAwait(false);
        return PullRequestFilters.Apply(prs, query.Filter, me);
    }

    private async Task<string?> GetSelfLoginAsync(CancellationToken ct)
    {
        if (_selfLogin is not null)
        {
            return _selfLogin;
        }

        using var doc = await GetAsync("user", ct).ConfigureAwait(false);
        _selfLogin = doc.RootElement.Str("login");
        return _selfLogin;
    }

    public async Task<PullRequest> GetPullRequestAsync(string id, CancellationToken ct)
    {
        using var doc = await GetAsync($"{Repo}/pulls/{id}", ct).ConfigureAwait(false);
        return GitHubMapper.MapPullRequest(doc.RootElement);
    }

    public async Task<IReadOnlyList<CommentThread>> GetPullRequestThreadsAsync(string id, CancellationToken ct)
    {
        using var doc = await GetAsync($"{Repo}/issues/{id}/comments?per_page=100", ct).ConfigureAwait(false);
        var comments = doc.RootElement.EnumerateArray().Select(c => new Comment
        {
            Id = c.Int("id")?.ToString() ?? "0",
            Author = c.Obj("user") is { } u ? GitHubMapper.MapUser(u) : new User { Id = "unknown", DisplayName = "unknown" },
            Body = c.Str("body") ?? string.Empty,
            CreatedAt = c.Date("created_at") ?? default,
        }).ToList();

        return comments.Count == 0
            ? []
            : [new CommentThread { Id = $"pr-{id}", Comments = comments }];
    }

    public async Task<IReadOnlyList<FileChange>> GetChangesAsync(string id, CancellationToken ct)
    {
        using var doc = await GetAsync($"{Repo}/pulls/{id}/files?per_page=100", ct).ConfigureAwait(false);
        return doc.RootElement.EnumerateArray().Select(GitHubMapper.MapFileChange).ToList();
    }

    public Task AddPullRequestCommentAsync(string id, string body, CancellationToken ct) =>
        PostAsync($"{Repo}/issues/{id}/comments", new { body }, ct);

    public Task VoteAsync(string id, ReviewVote vote, CancellationToken ct)
    {
        var @event = vote switch
        {
            ReviewVote.Approved or ReviewVote.ApprovedWithSuggestions => "APPROVE",
            ReviewVote.Rejected => "REQUEST_CHANGES",
            _ => "COMMENT",
        };
        return PostAsync($"{Repo}/pulls/{id}/reviews", new { @event }, ct);
    }

    public Task MergeAsync(string id, MergeOptions options, CancellationToken ct)
    {
        var method = options.Strategy switch
        {
            MergeStrategy.Squash => "squash",
            MergeStrategy.Rebase => "rebase",
            _ => "merge",
        };
        return PutAsync($"{Repo}/pulls/{id}/merge", new { merge_method = method }, ct);
    }

    public async Task<IReadOnlyList<WorkItem>> ListIssuesAsync(WorkItemQuery query, CancellationToken ct)
    {
        var state = query.IncludeCompleted ? "all" : "open";
        var path = $"{Repo}/issues?state={state}&per_page={query.Limit ?? 50}";
        if (query.MineOnly)
        {
            path += "&assignee=@me"; // honored when the token user is the assignee filter target
        }

        using var doc = await GetAsync(path, ct).ConfigureAwait(false);
        return doc.RootElement.EnumerateArray()
            .Where(e => !GitHubMapper.IsPullRequest(e))
            .Select(GitHubMapper.MapIssue)
            .ToList();
    }

    public async Task<WorkItem> GetIssueAsync(string id, CancellationToken ct)
    {
        using var doc = await GetAsync($"{Repo}/issues/{id}", ct).ConfigureAwait(false);
        return GitHubMapper.MapIssue(doc.RootElement);
    }

    public Task SetIssueStateAsync(string id, string state, CancellationToken ct) =>
        PatchAsync($"{Repo}/issues/{id}", new { state }, ct);

    public Task AddIssueCommentAsync(string id, string body, CancellationToken ct) =>
        PostAsync($"{Repo}/issues/{id}/comments", new { body }, ct);

    public async Task<IReadOnlyList<PipelineDefinition>> DiscoverWorkflowsAsync(CancellationToken ct)
    {
        using var doc = await GetAsync($"{Repo}/actions/workflows?per_page=100", ct).ConfigureAwait(false);
        return doc.RootElement.Arr("workflows").Select(GitHubMapper.MapWorkflow).ToList();
    }

    public async Task<IReadOnlyList<PipelineRun>> ListRunsAsync(PipelineRunQuery query, CancellationToken ct)
    {
        var path = query.DefinitionId is { } def
            ? $"{Repo}/actions/workflows/{def}/runs"
            : $"{Repo}/actions/runs";
        path += $"?per_page={query.Limit ?? 25}";
        if (query.Branch is { } b)
        {
            path += $"&branch={Uri.EscapeDataString(b)}";
        }

        using var doc = await GetAsync(path, ct).ConfigureAwait(false);
        return doc.RootElement.Arr("workflow_runs").Select(GitHubMapper.MapRun).ToList();
    }

    public async Task<PipelineRun> GetRunAsync(string runId, CancellationToken ct)
    {
        using var runDoc = await GetAsync($"{Repo}/actions/runs/{runId}", ct).ConfigureAwait(false);
        var run = GitHubMapper.MapRun(runDoc.RootElement);

        using var jobsDoc = await GetAsync($"{Repo}/actions/runs/{runId}/jobs", ct).ConfigureAwait(false);
        var jobs = jobsDoc.RootElement.Arr("jobs").Select(MapJob).ToList();
        return jobs.Count == 0
            ? run
            : run with { Stages = [new PipelineStage { Name = "jobs", Status = run.Status, Jobs = jobs }] };
    }

    public async Task<string> GetLogsAsync(string runId, string? jobId, CancellationToken ct)
    {
        // Full run logs are a zip archive; summarise job statuses instead for the TUI.
        using var jobsDoc = await GetAsync($"{Repo}/actions/runs/{runId}/jobs", ct).ConfigureAwait(false);
        var lines = jobsDoc.RootElement.Arr("jobs")
            .Where(j => jobId is null || j.Int("id")?.ToString() == jobId)
            .Select(j => $"{j.Str("name")}: {j.Str("status")}/{j.Str("conclusion") ?? "-"}");
        return string.Join('\n', lines);
    }

    public Task TriggerAsync(string definitionId, string? branch, CancellationToken ct) =>
        PostAsync($"{Repo}/actions/workflows/{definitionId}/dispatches", new { @ref = branch ?? "main" }, ct);

    private static PipelineJob MapJob(JsonElement el)
    {
        var status = el.Str("status");
        var conclusion = el.Str("conclusion");
        var runStatus = status == "completed"
            ? (conclusion == "success" ? PipelineRunStatus.Succeeded
                : conclusion == "cancelled" ? PipelineRunStatus.Canceled
                : PipelineRunStatus.Failed)
            : status == "queued" ? PipelineRunStatus.Queued
            : PipelineRunStatus.Running;

        return new PipelineJob
        {
            Id = el.Int("id")?.ToString() ?? "0",
            Name = el.Str("name") ?? "(job)",
            Status = runStatus,
            StartedAt = el.Date("started_at"),
            FinishedAt = el.Date("completed_at"),
        };
    }

    private async Task<JsonDocument> GetAsync(string path, CancellationToken ct)
    {
        using var resp = await _http.GetAsync(path, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
        await using var stream = await resp.Content.ReadAsStreamAsync(ct).ConfigureAwait(false);
        return await JsonDocument.ParseAsync(stream, default, ct).ConfigureAwait(false);
    }

    private async Task PostAsync(string path, object body, CancellationToken ct)
    {
        using var resp = await _http.PostAsJsonAsync(path, body, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
    }

    private async Task PutAsync(string path, object body, CancellationToken ct)
    {
        using var resp = await _http.PutAsJsonAsync(path, body, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
    }

    private async Task PatchAsync(string path, object body, CancellationToken ct)
    {
        using var resp = await _http.PatchAsJsonAsync(path, body, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
    }
}
