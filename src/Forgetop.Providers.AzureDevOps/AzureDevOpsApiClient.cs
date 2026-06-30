using System.Net.Http.Json;
using System.Text;
using System.Text.Json;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Providers.AzureDevOps;

/// <summary>
/// REST client over Azure DevOps. Project/repo come from the connection scope;
/// <c>api-version=7.1</c> is appended per request.
/// </summary>
public sealed class AzureDevOpsApiClient
{
    private const string Api = "api-version=7.1";

    private readonly HttpClient _http;
    private readonly string _project;
    private readonly string _repository;
    private string? _selfId;

    public AzureDevOpsApiClient(HttpClient http, string project, string repository)
    {
        _http = http ?? throw new ArgumentNullException(nameof(http));
        _project = project;
        _repository = repository;
    }

    // ---- Pull requests ----

    public async Task<IReadOnlyList<PullRequest>> ListPullRequestsAsync(PullRequestQuery query, CancellationToken ct)
    {
        var status = query.IncludeCompleted ? "all" : "active";
        var path = $"{_project}/_apis/git/repositories/{_repository}/pullrequests?searchCriteria.status={status}&$top={query.Limit ?? 50}&{Api}";
        using var doc = await GetAsync(path, ct).ConfigureAwait(false);
        var prs = doc.RootElement.Arr("value").Select(AzureDevOpsMapper.MapPullRequest).ToList();

        var me = query.Filter == PullRequestFilter.All ? null : await GetSelfIdAsync(ct).ConfigureAwait(false);
        return PullRequestFilters.Apply(prs, query.Filter, me);
    }

    public async Task<PullRequest> GetPullRequestAsync(string id, CancellationToken ct)
    {
        using var doc = await GetAsync(PrPath(id), ct).ConfigureAwait(false);
        return AzureDevOpsMapper.MapPullRequest(doc.RootElement);
    }

    public async Task<IReadOnlyList<CommentThread>> GetPullRequestThreadsAsync(string id, CancellationToken ct)
    {
        using var doc = await GetAsync($"{PrBase(id)}/threads?{Api}", ct).ConfigureAwait(false);
        return doc.RootElement.Arr("value").Select(t => new CommentThread
        {
            Id = t.Int("id")?.ToString() ?? "0",
            FilePath = t.Obj("threadContext")?.Str("filePath"),
            Line = t.Obj("threadContext")?.Obj("rightFileStart")?.Int("line"),
            IsResolved = t.Str("status") is "closed" or "fixed",
            Comments = t.Arr("comments").Select(c => new Comment
            {
                Id = c.Int("id")?.ToString() ?? "0",
                Author = c.Obj("author") is { } a ? AzureDevOpsMapper.MapUser(a) : new User { Id = "unknown", DisplayName = "unknown" },
                Body = c.Str("content") ?? string.Empty,
                CreatedAt = c.Date("publishedDate") ?? default,
            }).ToList(),
        }).ToList();
    }

    public async Task<IReadOnlyList<FileChange>> GetChangesAsync(string id, CancellationToken ct)
    {
        using var iterDoc = await GetAsync($"{PrBase(id)}/iterations?{Api}", ct).ConfigureAwait(false);
        var iterations = iterDoc.RootElement.Arr("value").ToList();
        if (iterations.Count == 0)
        {
            return [];
        }

        var lastIteration = iterations[^1].Int("id");
        using var doc = await GetAsync($"{PrBase(id)}/iterations/{lastIteration}/changes?{Api}", ct).ConfigureAwait(false);
        return doc.RootElement.Arr("changeEntries").Select(AzureDevOpsMapper.MapChangeEntry).ToList();
    }

    public Task AddPullRequestCommentAsync(string id, string body, CancellationToken ct) =>
        PostAsync($"{PrBase(id)}/threads?{Api}", new { comments = new[] { new { content = body, commentType = 1 } }, status = 1 }, ct);

    public async Task VoteAsync(string id, ReviewVote vote, CancellationToken ct)
    {
        var self = await GetSelfIdAsync(ct).ConfigureAwait(false);
        using var resp = await _http.PutAsJsonAsync(
            $"{PrBase(id)}/reviewers/{self}?{Api}",
            new { vote = AzureDevOpsMapper.ToVote(vote) }, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
    }

    public async Task MergeAsync(string id, MergeOptions options, CancellationToken ct)
    {
        using var prDoc = await GetAsync(PrPath(id), ct).ConfigureAwait(false);
        var sourceCommit = prDoc.RootElement.Obj("lastMergeSourceCommit")?.Str("commitId");
        var strategy = options.Strategy switch
        {
            MergeStrategy.Squash => "squash",
            MergeStrategy.Rebase => "rebase",
            _ => "noFastForward",
        };

        await PatchAsync(PrPath(id), new
        {
            status = "completed",
            lastMergeSourceCommit = new { commitId = sourceCommit },
            completionOptions = new { mergeStrategy = strategy, deleteSourceBranch = options.DeleteSourceRef },
        }, ct).ConfigureAwait(false);
    }

    // ---- Work items ----

    public async Task<IReadOnlyList<WorkItem>> ListWorkItemsAsync(WorkItemQuery query, CancellationToken ct)
    {
        var conditions = new List<string> { "[System.TeamProject] = @project" };
        if (query.MineOnly)
        {
            conditions.Add("[System.AssignedTo] = @me");
        }

        if (!query.IncludeCompleted)
        {
            conditions.Add("[System.State] NOT IN ('Closed', 'Done', 'Removed')");
        }

        var wiql = $"SELECT [System.Id] FROM WorkItems WHERE {string.Join(" AND ", conditions)} ORDER BY [System.ChangedDate] DESC";
        using var idDoc = await PostReadAsync($"{_project}/_apis/wit/wiql?$top={query.Limit ?? 50}&{Api}", new { query = wiql }, ct).ConfigureAwait(false);

        var ids = idDoc.RootElement.Arr("workItems").Select(w => w.Int("id")).Where(i => i is not null).Select(i => i!.Value).ToList();
        if (ids.Count == 0)
        {
            return [];
        }

        using var doc = await GetAsync($"_apis/wit/workitems?ids={string.Join(',', ids)}&{Api}", ct).ConfigureAwait(false);
        return doc.RootElement.Arr("value").Select(AzureDevOpsMapper.MapWorkItem).ToList();
    }

    public async Task<WorkItem> GetWorkItemAsync(string id, CancellationToken ct)
    {
        using var doc = await GetAsync($"_apis/wit/workitems/{id}?{Api}", ct).ConfigureAwait(false);
        return AzureDevOpsMapper.MapWorkItem(doc.RootElement);
    }

    public async Task SetWorkItemStateAsync(string id, string state, CancellationToken ct)
    {
        var patch = new[] { new { op = "add", path = "/fields/System.State", value = state } };
        using var content = new StringContent(JsonSerializer.Serialize(patch), Encoding.UTF8, "application/json-patch+json");
        using var resp = await _http.PatchAsync($"_apis/wit/workitems/{id}?{Api}", content, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
    }

    public Task AddWorkItemCommentAsync(string id, string body, CancellationToken ct) =>
        PostAsync($"{_project}/_apis/wit/workItems/{id}/comments?api-version=7.1-preview.3", new { text = body }, ct);

    // ---- Pipelines (build API) ----

    public async Task<IReadOnlyList<PipelineDefinition>> DiscoverAsync(CancellationToken ct)
    {
        using var doc = await GetAsync($"{_project}/_apis/build/definitions?{Api}", ct).ConfigureAwait(false);
        return doc.RootElement.Arr("value").Select(AzureDevOpsMapper.MapDefinition).ToList();
    }

    public async Task<IReadOnlyList<PipelineRun>> ListRunsAsync(PipelineRunQuery query, CancellationToken ct)
    {
        var path = $"{_project}/_apis/build/builds?$top={query.Limit ?? 25}&{Api}";
        if (query.DefinitionId is { } def)
        {
            path += $"&definitions={def}";
        }

        if (query.Branch is { } b)
        {
            path += $"&branchName={Uri.EscapeDataString("refs/heads/" + b)}";
        }

        using var doc = await GetAsync(path, ct).ConfigureAwait(false);
        return doc.RootElement.Arr("value").Select(AzureDevOpsMapper.MapBuild).ToList();
    }

    public async Task<PipelineRun> GetRunAsync(string runId, CancellationToken ct)
    {
        using var buildDoc = await GetAsync($"{_project}/_apis/build/builds/{runId}?{Api}", ct).ConfigureAwait(false);
        var run = AzureDevOpsMapper.MapBuild(buildDoc.RootElement);

        var stages = await ReadStagesAsync(runId, ct).ConfigureAwait(false);
        return stages.Count == 0 ? run : run with { Stages = stages };
    }

    public async Task<string> GetLogsAsync(string runId, string? jobId, CancellationToken ct)
    {
        using var doc = await GetAsync($"{_project}/_apis/build/builds/{runId}/timeline?{Api}", ct).ConfigureAwait(false);
        var lines = doc.RootElement.Arr("records")
            .Where(r => jobId is null || r.Str("id") == jobId)
            .OrderBy(r => r.Int("order") ?? 0)
            .Select(r => $"[{r.Str("type")}] {r.Str("name")}: {r.Str("state")}/{r.Str("result") ?? "-"}");
        return string.Join('\n', lines);
    }

    public Task TriggerAsync(string definitionId, string? branch, CancellationToken ct)
    {
        var body = branch is null
            ? (object)new { definition = new { id = int.Parse(definitionId) } }
            : new { definition = new { id = int.Parse(definitionId) }, sourceBranch = "refs/heads/" + branch };
        return PostAsync($"{_project}/_apis/build/builds?{Api}", body, ct);
    }

    private async Task<IReadOnlyList<PipelineStage>> ReadStagesAsync(string runId, CancellationToken ct)
    {
        using var doc = await GetAsync($"{_project}/_apis/build/builds/{runId}/timeline?{Api}", ct).ConfigureAwait(false);
        var records = doc.RootElement.Arr("records").ToList();

        PipelineRunStatus MapRecord(JsonElement r) => r.Str("state") == "completed"
            ? (r.Str("result") == "succeeded" ? PipelineRunStatus.Succeeded
                : r.Str("result") == "canceled" ? PipelineRunStatus.Canceled
                : PipelineRunStatus.Failed)
            : r.Str("state") == "inProgress" ? PipelineRunStatus.Running
            : PipelineRunStatus.Queued;

        return records.Where(r => r.Str("type") == "Stage").Select(stage => new PipelineStage
        {
            Name = stage.Str("name") ?? "stage",
            Status = MapRecord(stage),
            Jobs = records.Where(r => r.Str("type") == "Job" && r.Str("parentId") == stage.Str("id"))
                .Select(j => new PipelineJob
                {
                    Id = j.Str("id") ?? "0",
                    Name = j.Str("name") ?? "job",
                    Status = MapRecord(j),
                    StartedAt = j.Date("startTime"),
                    FinishedAt = j.Date("finishTime"),
                }).ToList(),
        }).ToList();
    }

    private async Task<string> GetSelfIdAsync(CancellationToken ct)
    {
        if (_selfId is not null)
        {
            return _selfId;
        }

        using var doc = await GetAsync($"_apis/connectionData?{Api}", ct).ConfigureAwait(false);
        _selfId = doc.RootElement.Obj("authenticatedUser")?.Str("id")
            ?? throw new InvalidOperationException("Could not resolve the authenticated Azure DevOps user.");
        return _selfId;
    }

    private string PrBase(string id) => $"{_project}/_apis/git/repositories/{_repository}/pullRequests/{id}";
    private string PrPath(string id) => $"{PrBase(id)}?{Api}";

    private async Task<JsonDocument> GetAsync(string path, CancellationToken ct)
    {
        using var resp = await _http.GetAsync(path, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
        await using var stream = await resp.Content.ReadAsStreamAsync(ct).ConfigureAwait(false);
        return await JsonDocument.ParseAsync(stream, default, ct).ConfigureAwait(false);
    }

    private async Task<JsonDocument> PostReadAsync(string path, object body, CancellationToken ct)
    {
        using var resp = await _http.PostAsJsonAsync(path, body, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
        await using var stream = await resp.Content.ReadAsStreamAsync(ct).ConfigureAwait(false);
        return await JsonDocument.ParseAsync(stream, default, ct).ConfigureAwait(false);
    }

    private async Task PostAsync(string path, object body, CancellationToken ct)
    {
        using var resp = await _http.PostAsJsonAsync(path, body, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
    }

    private async Task PatchAsync(string path, object body, CancellationToken ct)
    {
        using var resp = await _http.PatchAsJsonAsync(path, body, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
    }
}
