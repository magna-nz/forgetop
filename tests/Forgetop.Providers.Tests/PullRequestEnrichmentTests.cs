using System.Text.Json;
using Forgetop.Core.Domain;
using Forgetop.Providers.AzureDevOps;
using Forgetop.Providers.GitHub;

namespace Forgetop.Providers.Tests;

public class PullRequestEnrichmentTests
{
    private static JsonElement Parse(string json) => JsonDocument.Parse(json).RootElement;

    [Fact]
    public void GitHub_maps_labels_mergeable_and_diff_stats()
    {
        var pr = GitHubMapper.MapPullRequest(Parse("""
        {
          "number": 7, "title": "t", "state": "open", "user": { "login": "a" },
          "mergeable_state": "clean", "changed_files": 4, "additions": 30, "deletions": 9,
          "labels": [ { "name": "banking" }, { "name": "enhancement" } ]
        }
        """));

        Assert.Equal(MergeableState.Mergeable, pr.Mergeable);
        Assert.Equal(4, pr.ChangedFiles);
        Assert.Equal(30, pr.Additions);
        Assert.Equal(9, pr.Deletions);
        Assert.Equal(["banking", "enhancement"], pr.Labels);
    }

    [Fact]
    public void GitHub_draft_is_blocked()
    {
        var pr = GitHubMapper.MapPullRequest(Parse("""{ "number": 1, "title": "t", "state": "open", "draft": true }"""));
        Assert.Equal(MergeableState.Blocked, pr.Mergeable);
    }

    [Fact]
    public void GitHub_aggregates_check_runs()
    {
        var (status, summary) = GitHubMapper.MapChecks(Parse("""
        {
          "check_runs": [
            { "status": "completed", "conclusion": "success" },
            { "status": "completed", "conclusion": "success" },
            { "status": "in_progress" },
            { "status": "completed", "conclusion": "neutral" }
          ]
        }
        """));

        Assert.Equal(CheckStatus.Pending, status); // an in-progress run with no failures
        Assert.Equal(2, summary.Successful);
        Assert.Equal(1, summary.InProgress);
        Assert.Equal(1, summary.Neutral);
    }

    [Fact]
    public void GitHub_checks_fail_when_any_run_failed()
    {
        var (status, _) = GitHubMapper.MapChecks(Parse("""
        { "check_runs": [ { "status": "completed", "conclusion": "success" }, { "status": "completed", "conclusion": "failure" } ] }
        """));
        Assert.Equal(CheckStatus.Failed, status);
    }

    [Theory]
    [InlineData("succeeded", false, MergeableState.Mergeable)]
    [InlineData("conflicts", false, MergeableState.Conflicting)]
    [InlineData("rejectedByPolicy", false, MergeableState.Blocked)]
    [InlineData("succeeded", true, MergeableState.Blocked)] // draft overrides
    public void AzureDevOps_maps_merge_status_and_draft(string mergeStatus, bool isDraft, MergeableState expected)
    {
        var pr = AzureDevOpsMapper.MapPullRequest(Parse($$"""
        { "pullRequestId": 1, "title": "t", "status": "active", "mergeStatus": "{{mergeStatus}}", "isDraft": {{(isDraft ? "true" : "false")}},
          "labels": [ { "name": "infra" } ] }
        """));

        Assert.Equal(expected, pr.Mergeable);
        Assert.Equal(["infra"], pr.Labels);
    }
}
