using System.Net.Http.Json;
using System.Text.Json;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Providers.Linear;

/// <summary>GraphQL client over the Linear API (work items only).</summary>
public sealed class LinearApiClient
{
    private const string IssueFields =
        "id identifier title description url createdAt updatedAt " +
        "state { name type } assignee { id name displayName avatarUrl }";

    private readonly HttpClient _http;

    public LinearApiClient(HttpClient http) => _http = http ?? throw new ArgumentNullException(nameof(http));

    public async Task<IReadOnlyList<WorkItem>> ListIssuesAsync(WorkItemQuery query, CancellationToken ct)
    {
        object? filter = BuildFilter(query);
        var gql = $"query Issues($first:Int!,$filter:IssueFilter){{ issues(first:$first,filter:$filter){{ nodes {{ {IssueFields} }} }} }}";

        using var doc = await PostAsync(gql, new { first = query.Limit ?? 50, filter }, ct).ConfigureAwait(false);
        return Data(doc).Nodes("issues").Select(LinearMapper.MapIssue).ToList();
    }

    public async Task<WorkItem> GetIssueAsync(string id, CancellationToken ct)
    {
        var gql = $"query Issue($id:String!){{ issue(id:$id){{ {IssueFields} }} }}";
        using var doc = await PostAsync(gql, new { id }, ct).ConfigureAwait(false);
        var issue = Data(doc).Obj("issue") ?? throw new InvalidOperationException($"Linear issue '{id}' not found.");
        return LinearMapper.MapIssue(issue);
    }

    public async Task<IReadOnlyList<CommentThread>> GetThreadsAsync(string issueId, CancellationToken ct)
    {
        const string gql = "query($id:String!){ issue(id:$id){ comments { nodes { id body createdAt user { id name displayName } } } } }";
        using var doc = await PostAsync(gql, new { id = issueId }, ct).ConfigureAwait(false);

        var issue = Data(doc).Obj("issue");
        if (issue is not { } i)
        {
            return [];
        }

        var comments = i.Nodes("comments").Select(LinearMapper.MapComment).ToList();
        return comments.Count == 0 ? [] : [new CommentThread { Id = $"issue-{issueId}", Comments = comments }];
    }

    public async Task SetStateAsync(string issueId, string stateName, CancellationToken ct)
    {
        const string lookup = "query($id:String!){ issue(id:$id){ team { states { nodes { id name } } } } }";
        using var stateDoc = await PostAsync(lookup, new { id = issueId }, ct).ConfigureAwait(false);

        var team = Data(stateDoc).Obj("issue")?.Obj("team")
            ?? throw new InvalidOperationException($"Could not resolve team for Linear issue '{issueId}'.");
        var match = team.Nodes("states")
            .FirstOrDefault(s => string.Equals(s.Str("name"), stateName, StringComparison.OrdinalIgnoreCase));
        var stateId = match.ValueKind == JsonValueKind.Object ? match.Str("id") : null;
        if (stateId is null)
        {
            throw new InvalidOperationException($"No workflow state named '{stateName}' on the issue's team.");
        }

        const string mutation = "mutation($id:String!,$stateId:String!){ issueUpdate(id:$id,input:{stateId:$stateId}){ success } }";
        using var _ = await PostAsync(mutation, new { id = issueId, stateId }, ct).ConfigureAwait(false);
    }

    public async Task AddCommentAsync(string issueId, string body, CancellationToken ct)
    {
        const string mutation = "mutation($id:String!,$body:String!){ commentCreate(input:{issueId:$id,body:$body}){ success } }";
        using var _ = await PostAsync(mutation, new { id = issueId, body }, ct).ConfigureAwait(false);
    }

    private static object? BuildFilter(WorkItemQuery query)
    {
        var hasState = !query.IncludeCompleted;
        return (query.MineOnly, hasState) switch
        {
            (true, true) => new { assignee = new { isMe = new { eq = true } }, state = new { type = new { nin = new[] { "completed", "canceled" } } } },
            (true, false) => new { assignee = new { isMe = new { eq = true } } },
            (false, true) => (object)new { state = new { type = new { nin = new[] { "completed", "canceled" } } } },
            _ => null,
        };
    }

    private static JsonElement Data(JsonDocument doc) =>
        doc.RootElement.Obj("data") ?? throw new InvalidOperationException("Linear response had no data.");

    private async Task<JsonDocument> PostAsync(string query, object variables, CancellationToken ct)
    {
        using var resp = await _http.PostAsJsonAsync(string.Empty, new { query, variables }, ct).ConfigureAwait(false);
        resp.EnsureSuccessStatusCode();
        await using var stream = await resp.Content.ReadAsStreamAsync(ct).ConfigureAwait(false);
        var doc = await JsonDocument.ParseAsync(stream, default, ct).ConfigureAwait(false);

        if (doc.RootElement.TryGetProperty("errors", out var errors) && errors.ValueKind == JsonValueKind.Array && errors.GetArrayLength() > 0)
        {
            var message = errors[0].Str("message") ?? "unknown GraphQL error";
            doc.Dispose();
            throw new InvalidOperationException($"Linear API error: {message}");
        }

        return doc;
    }
}
