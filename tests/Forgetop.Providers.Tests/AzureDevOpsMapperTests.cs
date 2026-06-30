using System.Text.Json;
using Forgetop.Core.Domain;
using Forgetop.Providers.AzureDevOps;

namespace Forgetop.Providers.Tests;

public class AzureDevOpsMapperTests
{
    private static JsonElement Parse(string json) => JsonDocument.Parse(json).RootElement;

    [Theory]
    [InlineData(10, ReviewVote.Approved)]
    [InlineData(5, ReviewVote.ApprovedWithSuggestions)]
    [InlineData(0, ReviewVote.NoVote)]
    [InlineData(-5, ReviewVote.WaitingForAuthor)]
    [InlineData(-10, ReviewVote.Rejected)]
    public void Maps_numeric_votes_both_ways(int numeric, ReviewVote vote)
    {
        Assert.Equal(vote, AzureDevOpsMapper.MapVote(numeric));
        Assert.Equal(numeric, AzureDevOpsMapper.ToVote(vote));
    }

    [Fact]
    public void Maps_active_pull_request_with_reviewer()
    {
        var pr = AzureDevOpsMapper.MapPullRequest(Parse("""
        {
          "pullRequestId": 10, "title": "t", "description": "d", "status": "active", "isDraft": false,
          "createdBy": { "id": "g1", "displayName": "Dan", "uniqueName": "dan@x" },
          "sourceRefName": "refs/heads/feature/x", "targetRefName": "refs/heads/main",
          "creationDate": "2026-06-01T10:00:00Z", "url": "u",
          "reviewers": [ { "id": "r1", "displayName": "Rev", "vote": 10, "isRequired": true } ]
        }
        """));

        Assert.Equal("10", pr.Id);
        Assert.Equal(PullRequestStatus.Open, pr.Status);
        Assert.Equal("feature/x", pr.SourceRef);
        Assert.Equal("main", pr.TargetRef);
        var reviewer = Assert.Single(pr.Reviewers);
        Assert.Equal(ReviewVote.Approved, reviewer.Vote);
        Assert.True(reviewer.IsRequired);
    }

    [Fact]
    public void Completed_pull_request_maps_to_merged()
    {
        var pr = AzureDevOpsMapper.MapPullRequest(Parse("""{ "pullRequestId": 1, "title": "t", "status": "completed" }"""));
        Assert.Equal(PullRequestStatus.Merged, pr.Status);
    }

    [Fact]
    public void Maps_work_item_fields()
    {
        var wi = AzureDevOpsMapper.MapWorkItem(Parse("""
        {
          "id": 55,
          "fields": {
            "System.Title": "WI", "System.State": "Active", "System.WorkItemType": "Bug",
            "System.CreatedDate": "2026-06-01T10:00:00Z",
            "System.AssignedTo": { "id": "u", "displayName": "Dan" }
          },
          "url": "u"
        }
        """));

        Assert.Equal("55", wi.Id);
        Assert.Equal("Active", wi.State);
        Assert.Equal(WorkItemStateCategory.Started, wi.StateCategory);
        Assert.Equal("Bug", wi.Type);
        Assert.Equal("Dan", wi.Assignee?.DisplayName);
    }

    [Theory]
    [InlineData("New", WorkItemStateCategory.Unstarted)]
    [InlineData("Done", WorkItemStateCategory.Completed)]
    [InlineData("Removed", WorkItemStateCategory.Canceled)]
    public void Maps_state_categories(string state, WorkItemStateCategory expected) =>
        Assert.Equal(expected, AzureDevOpsMapper.MapState(state));

    [Fact]
    public void Maps_succeeded_build()
    {
        var run = AzureDevOpsMapper.MapBuild(Parse("""
        {
          "id": 900, "buildNumber": "20260601.1", "status": "completed", "result": "succeeded",
          "sourceBranch": "refs/heads/main", "sourceVersion": "abc",
          "definition": { "id": 3, "name": "CI" },
          "requestedFor": { "id": "u", "displayName": "Dan" },
          "startTime": "2026-06-01T10:00:00Z", "finishTime": "2026-06-01T10:05:00Z",
          "_links": { "web": { "href": "u" } }
        }
        """));

        Assert.Equal("900", run.Id);
        Assert.Equal("3", run.DefinitionId);
        Assert.Equal(PipelineRunStatus.Succeeded, run.Status);
        Assert.Equal("main", run.Branch);
        Assert.Equal("u", run.Url);
    }
}
