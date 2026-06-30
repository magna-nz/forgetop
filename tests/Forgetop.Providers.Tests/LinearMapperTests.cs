using System.Text.Json;
using Forgetop.Core.Domain;
using Forgetop.Providers.Linear;

namespace Forgetop.Providers.Tests;

public class LinearMapperTests
{
    private static JsonElement Parse(string json) => JsonDocument.Parse(json).RootElement;

    [Fact]
    public void Maps_issue_with_state_and_assignee()
    {
        var wi = LinearMapper.MapIssue(Parse("""
        {
          "id": "iss_1", "identifier": "ENG-42", "title": "t", "description": "d", "url": "u",
          "createdAt": "2026-06-01T10:00:00Z", "updatedAt": "2026-06-02T10:00:00Z",
          "state": { "name": "In Progress", "type": "started" },
          "assignee": { "id": "u1", "name": "dan", "displayName": "Dan" }
        }
        """));

        Assert.Equal("iss_1", wi.Id);
        Assert.Equal("ENG-42", wi.Identifier);
        Assert.Equal("In Progress", wi.State);
        Assert.Equal(WorkItemStateCategory.Started, wi.StateCategory);
        Assert.Equal("Dan", wi.Assignee?.DisplayName);
    }

    [Theory]
    [InlineData("triage", WorkItemStateCategory.Triage)]
    [InlineData("backlog", WorkItemStateCategory.Backlog)]
    [InlineData("unstarted", WorkItemStateCategory.Unstarted)]
    [InlineData("started", WorkItemStateCategory.Started)]
    [InlineData("completed", WorkItemStateCategory.Completed)]
    [InlineData("canceled", WorkItemStateCategory.Canceled)]
    [InlineData(null, WorkItemStateCategory.Backlog)]
    public void Maps_workflow_state_types(string? type, WorkItemStateCategory expected) =>
        Assert.Equal(expected, LinearMapper.MapStateCategory(type));

    [Fact]
    public void Maps_comment()
    {
        var c = LinearMapper.MapComment(Parse("""
        { "id": "c1", "body": "nice", "createdAt": "2026-06-01T10:00:00Z", "user": { "id": "u", "displayName": "Dan" } }
        """));
        Assert.Equal("nice", c.Body);
        Assert.Equal("Dan", c.Author.DisplayName);
    }
}
