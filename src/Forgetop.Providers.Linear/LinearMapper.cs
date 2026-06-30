using System.Text.Json;
using Forgetop.Core.Domain;

namespace Forgetop.Providers.Linear;

/// <summary>Pure mappers from Linear GraphQL JSON to forgetop's domain model.</summary>
public static class LinearMapper
{
    public static User MapUser(JsonElement el) => new()
    {
        Id = el.Str("id") ?? "unknown",
        DisplayName = el.Str("displayName") ?? el.Str("name") ?? "unknown",
        Handle = el.Str("name"),
        AvatarUrl = el.Str("avatarUrl"),
    };

    /// <summary>Linear workflow-state types map 1:1 onto our neutral categories.</summary>
    public static WorkItemStateCategory MapStateCategory(string? type) => type switch
    {
        "triage" => WorkItemStateCategory.Triage,
        "backlog" => WorkItemStateCategory.Backlog,
        "unstarted" => WorkItemStateCategory.Unstarted,
        "started" => WorkItemStateCategory.Started,
        "completed" => WorkItemStateCategory.Completed,
        "canceled" => WorkItemStateCategory.Canceled,
        _ => WorkItemStateCategory.Backlog,
    };

    public static WorkItem MapIssue(JsonElement el)
    {
        var state = el.Obj("state");
        return new WorkItem
        {
            Id = el.Str("id") ?? "unknown",
            Identifier = el.Str("identifier"),
            Title = el.Str("title") ?? "(untitled)",
            Description = el.Str("description"),
            State = state?.Str("name") ?? "Unknown",
            StateCategory = MapStateCategory(state?.Str("type")),
            Assignee = el.Obj("assignee") is { } a ? MapUser(a) : null,
            CreatedAt = el.Date("createdAt") ?? default,
            UpdatedAt = el.Date("updatedAt"),
            Url = el.Str("url"),
        };
    }

    public static Comment MapComment(JsonElement el) => new()
    {
        Id = el.Str("id") ?? "0",
        Author = el.Obj("user") is { } u ? MapUser(u) : new User { Id = "unknown", DisplayName = "unknown" },
        Body = el.Str("body") ?? string.Empty,
        CreatedAt = el.Date("createdAt") ?? default,
    };
}
