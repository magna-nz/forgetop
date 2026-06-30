using System.Text.Json;
using Forgetop.Core.Domain;

namespace Forgetop.Providers.AzureDevOps;

/// <summary>Pure mappers from Azure DevOps REST JSON to forgetop's domain model.</summary>
public static class AzureDevOpsMapper
{
    public static User MapUser(JsonElement el) => new()
    {
        Id = el.Str("id") ?? el.Str("uniqueName") ?? "unknown",
        DisplayName = el.Str("displayName") ?? el.Str("uniqueName") ?? "unknown",
        Handle = el.Str("uniqueName"),
        AvatarUrl = el.Str("imageUrl"),
    };

    /// <summary>ADO numeric vote (-10..10) → neutral <see cref="ReviewVote"/>.</summary>
    public static ReviewVote MapVote(int vote) => vote switch
    {
        10 => ReviewVote.Approved,
        5 => ReviewVote.ApprovedWithSuggestions,
        -5 => ReviewVote.WaitingForAuthor,
        -10 => ReviewVote.Rejected,
        _ => ReviewVote.NoVote,
    };

    public static int ToVote(ReviewVote vote) => vote switch
    {
        ReviewVote.Approved => 10,
        ReviewVote.ApprovedWithSuggestions => 5,
        ReviewVote.WaitingForAuthor => -5,
        ReviewVote.Rejected => -10,
        _ => 0,
    };

    private static string? StripRef(string? refName) =>
        refName?.StartsWith("refs/heads/", StringComparison.Ordinal) == true ? refName["refs/heads/".Length..] : refName;

    public static PullRequest MapPullRequest(JsonElement el)
    {
        var status = el.Str("status") switch
        {
            "completed" => PullRequestStatus.Merged,
            "abandoned" => PullRequestStatus.Closed,
            _ when el.Bool("isDraft") => PullRequestStatus.Draft,
            _ => PullRequestStatus.Open,
        };

        var id = el.Int("pullRequestId");
        return new PullRequest
        {
            Id = id?.ToString() ?? "0",
            Number = id,
            Title = el.Str("title") ?? "(untitled)",
            Description = el.Str("description"),
            Author = el.Obj("createdBy") is { } c ? MapUser(c) : Unknown,
            Status = status,
            IsDraft = el.Bool("isDraft"),
            SourceRef = StripRef(el.Str("sourceRefName")),
            TargetRef = StripRef(el.Str("targetRefName")),
            CreatedAt = el.Date("creationDate") ?? default,
            Url = el.Str("url"),
            Reviewers = el.Arr("reviewers").Select(r => new Reviewer
            {
                User = MapUser(r),
                Vote = MapVote(r.Int("vote") ?? 0),
                IsRequired = r.Bool("isRequired"),
            }).ToList(),
        };
    }

    public static FileChange MapChangeEntry(JsonElement el)
    {
        var kind = el.Str("changeType") switch
        {
            "add" => FileChangeKind.Added,
            "delete" => FileChangeKind.Deleted,
            "rename" or "sourceRename" => FileChangeKind.Renamed,
            _ => FileChangeKind.Modified,
        };

        return new FileChange
        {
            Path = el.Obj("item")?.Str("path") ?? "(unknown)",
            Kind = kind,
        };
    }

    /// <summary>Maps a work item whose fields live under the <c>fields</c> object.</summary>
    public static WorkItem MapWorkItem(JsonElement el)
    {
        var fields = el.Obj("fields") ?? el;
        var state = Field(fields, "System.State") ?? "New";

        return new WorkItem
        {
            Id = el.Int("id")?.ToString() ?? "0",
            Identifier = el.Int("id") is { } n ? n.ToString() : null,
            Title = Field(fields, "System.Title") ?? "(untitled)",
            Description = Field(fields, "System.Description"),
            State = state,
            StateCategory = MapState(state),
            Type = Field(fields, "System.WorkItemType"),
            Assignee = fields.Obj("System.AssignedTo") is { } a ? MapUser(a) : null,
            CreatedAt = fields.Date("System.CreatedDate") ?? default,
            UpdatedAt = fields.Date("System.ChangedDate"),
            Url = el.Str("url"),
        };
    }

    public static WorkItemStateCategory MapState(string state) => state.ToLowerInvariant() switch
    {
        "new" or "proposed" or "to do" => WorkItemStateCategory.Unstarted,
        "active" or "committed" or "in progress" or "doing" => WorkItemStateCategory.Started,
        "resolved" => WorkItemStateCategory.Started,
        "closed" or "done" or "completed" => WorkItemStateCategory.Completed,
        "removed" => WorkItemStateCategory.Canceled,
        _ => WorkItemStateCategory.Backlog,
    };

    public static PipelineDefinition MapDefinition(JsonElement el) => new()
    {
        Id = el.Int("id")?.ToString() ?? "0",
        Name = el.Str("name") ?? "(pipeline)",
        Path = el.Str("path"),
        Url = el.Obj("_links")?.Obj("web")?.Str("href"),
    };

    public static PipelineRun MapBuild(JsonElement el)
    {
        var state = el.Str("status"); // none|inProgress|completed|cancelling|postponed|notStarted
        var result = el.Str("result"); // succeeded|partiallySucceeded|failed|canceled

        var status = state switch
        {
            "completed" => result switch
            {
                "succeeded" => PipelineRunStatus.Succeeded,
                "partiallySucceeded" => PipelineRunStatus.PartiallySucceeded,
                "canceled" => PipelineRunStatus.Canceled,
                _ => PipelineRunStatus.Failed,
            },
            "inProgress" or "cancelling" => PipelineRunStatus.Running,
            _ => PipelineRunStatus.Queued,
        };

        return new PipelineRun
        {
            Id = el.Int("id")?.ToString() ?? "0",
            DefinitionId = el.Obj("definition")?.Int("id")?.ToString() ?? "0",
            Number = null,
            Name = el.Str("buildNumber") ?? el.Obj("definition")?.Str("name"),
            Status = status,
            TriggeredBy = el.Obj("requestedFor") is { } u ? MapUser(u) : null,
            Branch = StripRef(el.Str("sourceBranch")),
            CommitSha = el.Str("sourceVersion"),
            StartedAt = el.Date("startTime") ?? el.Date("queueTime"),
            FinishedAt = el.Date("finishTime"),
            Url = el.Obj("_links")?.Obj("web")?.Str("href"),
        };
    }

    private static string? Field(JsonElement fields, string name) =>
        fields.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    private static readonly User Unknown = new() { Id = "unknown", DisplayName = "unknown" };
}
