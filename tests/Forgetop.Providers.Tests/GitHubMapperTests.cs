using System.Text.Json;
using Forgetop.Core.Domain;
using Forgetop.Providers.GitHub;

namespace Forgetop.Providers.Tests;

public class GitHubMapperTests
{
    private static JsonElement Parse(string json) => JsonDocument.Parse(json).RootElement;

    [Fact]
    public void Maps_open_pull_request_with_reviewers()
    {
        var pr = GitHubMapper.MapPullRequest(Parse("""
        {
          "number": 42, "title": "Add feature", "body": "desc",
          "state": "open", "draft": false, "merged_at": null,
          "user": { "id": 7, "login": "octocat", "avatar_url": "http://a" },
          "head": { "ref": "feature/x" }, "base": { "ref": "main" },
          "created_at": "2026-06-01T10:00:00Z", "updated_at": "2026-06-02T10:00:00Z",
          "html_url": "http://pr",
          "requested_reviewers": [ { "id": 8, "login": "rev" } ]
        }
        """));

        Assert.Equal("42", pr.Id);
        Assert.Equal(42, pr.Number);
        Assert.Equal(PullRequestStatus.Open, pr.Status);
        Assert.Equal("octocat", pr.Author.Handle);
        Assert.Equal("feature/x", pr.SourceRef);
        Assert.Equal("main", pr.TargetRef);
        var reviewer = Assert.Single(pr.Reviewers);
        Assert.Equal(ReviewVote.NoVote, reviewer.Vote);
    }

    [Fact]
    public void Merged_pull_request_maps_to_merged()
    {
        var pr = GitHubMapper.MapPullRequest(Parse("""
        { "number": 1, "title": "t", "state": "closed", "merged_at": "2026-06-01T10:00:00Z", "user": { "login": "a" } }
        """));
        Assert.Equal(PullRequestStatus.Merged, pr.Status);
    }

    [Fact]
    public void Draft_pull_request_maps_to_draft()
    {
        var pr = GitHubMapper.MapPullRequest(Parse("""
        { "number": 2, "title": "t", "state": "open", "draft": true, "user": { "login": "a" } }
        """));
        Assert.Equal(PullRequestStatus.Draft, pr.Status);
        Assert.True(pr.IsDraft);
    }

    [Fact]
    public void Closed_issue_with_reason_maps_category()
    {
        var done = GitHubMapper.MapIssue(Parse("""
        { "number": 5, "title": "bug", "state": "closed", "state_reason": "completed", "assignee": { "login": "a" } }
        """));
        Assert.Equal(WorkItemStateCategory.Completed, done.StateCategory);

        var notPlanned = GitHubMapper.MapIssue(Parse("""
        { "number": 6, "title": "wontfix", "state": "closed", "state_reason": "not_planned" }
        """));
        Assert.Equal(WorkItemStateCategory.Canceled, notPlanned.StateCategory);
    }

    [Fact]
    public void Detects_pull_request_in_issues_feed()
    {
        Assert.True(GitHubMapper.IsPullRequest(Parse("""{ "number": 1, "pull_request": { "url": "x" } }""")));
        Assert.False(GitHubMapper.IsPullRequest(Parse("""{ "number": 2 }""")));
    }

    [Fact]
    public void Maps_workflow_and_failed_run()
    {
        var def = GitHubMapper.MapWorkflow(Parse("""
        { "id": 99, "name": "CI", "path": ".github/workflows/ci.yml", "html_url": "u" }
        """));
        Assert.Equal("99", def.Id);
        Assert.Equal("CI", def.Name);

        var run = GitHubMapper.MapRun(Parse("""
        {
          "id": 123, "workflow_id": 99, "run_number": 7, "name": "CI",
          "status": "completed", "conclusion": "failure",
          "head_branch": "main", "head_sha": "abc", "actor": { "login": "a" },
          "run_started_at": "2026-06-01T10:00:00Z", "updated_at": "2026-06-01T10:05:00Z", "html_url": "u"
        }
        """));
        Assert.Equal(PipelineRunStatus.Failed, run.Status);
        Assert.Equal("99", run.DefinitionId);
        Assert.Equal(7, run.Number);
        Assert.NotNull(run.FinishedAt);
    }
}
