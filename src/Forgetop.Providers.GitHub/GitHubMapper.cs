using System.Text.Json;
using Forgetop.Core.Domain;

namespace Forgetop.Providers.GitHub;

/// <summary>Pure mappers from GitHub REST JSON to forgetop's domain model.</summary>
public static class GitHubMapper
{
    public static User MapUser(JsonElement el) => new()
    {
        Id = el.Int("id")?.ToString() ?? el.Str("login") ?? "unknown",
        DisplayName = el.Str("login") ?? "unknown",
        Handle = el.Str("login"),
        AvatarUrl = el.Str("avatar_url"),
    };

    public static PullRequest MapPullRequest(JsonElement el)
    {
        var merged = el.TryGetProperty("merged_at", out var m) && m.ValueKind == JsonValueKind.String;
        var state = el.Str("state");
        var draft = el.Bool("draft");

        var status = merged ? PullRequestStatus.Merged
            : state == "closed" ? PullRequestStatus.Closed
            : draft ? PullRequestStatus.Draft
            : PullRequestStatus.Open;

        var mergeable = draft ? MergeableState.Blocked : el.Str("mergeable_state") switch
        {
            "clean" or "unstable" or "has_hooks" => MergeableState.Mergeable,
            "dirty" => MergeableState.Conflicting,
            "blocked" or "behind" or "draft" => MergeableState.Blocked,
            _ => MergeableState.Unknown,
        };

        var number = el.Int("number");
        return new PullRequest
        {
            Id = number?.ToString() ?? el.Int("id")?.ToString() ?? "0",
            Number = number,
            Title = el.Str("title") ?? "(untitled)",
            Description = el.Str("body"),
            Author = el.Obj("user") is { } u ? MapUser(u) : Unknown,
            Status = status,
            IsDraft = draft,
            SourceRef = el.Obj("head")?.Str("ref"),
            TargetRef = el.Obj("base")?.Str("ref"),
            CreatedAt = el.Date("created_at") ?? default,
            UpdatedAt = el.Date("updated_at"),
            Url = el.Str("html_url"),
            Reviewers = el.Arr("requested_reviewers")
                .Select(r => new Reviewer { User = MapUser(r), Vote = ReviewVote.NoVote })
                .ToList(),
            Labels = el.Arr("labels").Select(l => l.Str("name") ?? "").Where(n => n.Length > 0).ToList(),
            Mergeable = mergeable,
            ChangedFiles = el.Int("changed_files") ?? 0,
            Additions = el.Int("additions") ?? 0,
            Deletions = el.Int("deletions") ?? 0,
        };
    }

    /// <summary>Aggregates a GitHub check-runs response into a roll-up status + counts.</summary>
    public static (CheckStatus Status, CheckSummary Summary) MapChecks(JsonElement el)
    {
        int successful = 0, inProgress = 0, failed = 0, neutral = 0;
        foreach (var run in el.Arr("check_runs"))
        {
            if (run.Str("status") != "completed")
            {
                inProgress++;
                continue;
            }

            switch (run.Str("conclusion"))
            {
                case "success": successful++; break;
                case "neutral" or "skipped": neutral++; break;
                default: failed++; break; // failure, timed_out, cancelled, action_required
            }
        }

        var summary = new CheckSummary { Successful = successful, InProgress = inProgress, Failed = failed, Neutral = neutral };
        var status = summary.Total == 0 ? CheckStatus.None
            : failed > 0 ? CheckStatus.Failed
            : inProgress > 0 ? CheckStatus.Pending
            : CheckStatus.Passed;
        return (status, summary);
    }

    /// <summary>True when an /issues item is actually a pull request (GitHub returns both).</summary>
    public static bool IsPullRequest(JsonElement issue) => issue.TryGetProperty("pull_request", out _);

    public static WorkItem MapIssue(JsonElement el)
    {
        var state = el.Str("state") ?? "open";
        var reason = el.Str("state_reason");
        var category = state == "closed"
            ? (reason == "not_planned" ? WorkItemStateCategory.Canceled : WorkItemStateCategory.Completed)
            : WorkItemStateCategory.Unstarted;

        var number = el.Int("number");
        return new WorkItem
        {
            Id = number?.ToString() ?? el.Int("id")?.ToString() ?? "0",
            Identifier = number is { } n ? $"#{n}" : null,
            Title = el.Str("title") ?? "(untitled)",
            Description = el.Str("body"),
            State = state,
            StateCategory = category,
            Assignee = el.Obj("assignee") is { } a ? MapUser(a) : null,
            CreatedAt = el.Date("created_at") ?? default,
            UpdatedAt = el.Date("updated_at"),
            Url = el.Str("html_url"),
        };
    }

    public static FileChange MapFileChange(JsonElement el)
    {
        var kind = el.Str("status") switch
        {
            "added" => FileChangeKind.Added,
            "removed" => FileChangeKind.Deleted,
            "renamed" => FileChangeKind.Renamed,
            _ => FileChangeKind.Modified,
        };

        return new FileChange
        {
            Path = el.Str("filename") ?? "(unknown)",
            Kind = kind,
            Additions = el.Int("additions") ?? 0,
            Deletions = el.Int("deletions") ?? 0,
            Patch = el.Str("patch"),
        };
    }

    public static PipelineDefinition MapWorkflow(JsonElement el) => new()
    {
        Id = el.Int("id")?.ToString() ?? el.Str("path") ?? "0",
        Name = el.Str("name") ?? "(workflow)",
        Path = el.Str("path"),
        Url = el.Str("html_url"),
    };

    public static PipelineRun MapRun(JsonElement el)
    {
        var status = el.Str("status"); // queued | in_progress | completed
        var conclusion = el.Str("conclusion"); // success | failure | cancelled | ...

        var runStatus = status switch
        {
            "completed" => conclusion switch
            {
                "success" => PipelineRunStatus.Succeeded,
                "cancelled" => PipelineRunStatus.Canceled,
                "skipped" => PipelineRunStatus.Canceled,
                _ => PipelineRunStatus.Failed,
            },
            "queued" or "requested" or "waiting" or "pending" => PipelineRunStatus.Queued,
            _ => PipelineRunStatus.Running,
        };

        var completed = status == "completed";
        return new PipelineRun
        {
            Id = el.Int("id")?.ToString() ?? "0",
            DefinitionId = el.Int("workflow_id")?.ToString() ?? "0",
            Number = el.Int("run_number"),
            Name = el.Str("name") ?? el.Str("display_title"),
            Status = runStatus,
            TriggeredBy = el.Obj("actor") is { } actor ? MapUser(actor) : null,
            Branch = el.Str("head_branch"),
            CommitSha = el.Str("head_sha"),
            StartedAt = el.Date("run_started_at") ?? el.Date("created_at"),
            FinishedAt = completed ? el.Date("updated_at") : null,
            Url = el.Str("html_url"),
        };
    }

    private static readonly User Unknown = new() { Id = "unknown", DisplayName = "unknown" };
}
