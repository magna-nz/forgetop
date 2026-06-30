using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Forgetop.Providers.Demo;

namespace Forgetop.Providers.Tests;

public class DemoProviderTests
{
    private static IProviderConnection Connection() =>
        new DemoProviderFactory().Create(
            new Connection { Id = "demo", ProviderType = ProviderType.Demo, DisplayName = "Demo" },
            secret: null);

    [Fact]
    public void Factory_declares_all_sections()
    {
        var caps = new DemoProviderFactory().DescribeCapabilities();
        Assert.True(caps.SupportsPullRequests);
        Assert.True(caps.SupportsWorkItems);
        Assert.True(caps.SupportsPipelines);
    }

    [Fact]
    public void Connection_exposes_all_three_sources()
    {
        var conn = Connection();
        Assert.NotNull(conn.PullRequests);
        Assert.NotNull(conn.WorkItems);
        Assert.NotNull(conn.Pipelines);
    }

    [Fact]
    public async Task PullRequests_excludes_completed_by_default()
    {
        var prs = await Connection().PullRequests!.ListAsync(new PullRequestQuery());
        Assert.All(prs, p => Assert.True(p.Status is PullRequestStatus.Open or PullRequestStatus.Draft));
    }

    [Fact]
    public async Task PullRequests_includes_completed_when_requested()
    {
        var prs = await Connection().PullRequests!.ListAsync(new PullRequestQuery { IncludeCompleted = true });
        Assert.Contains(prs, p => p.Status == PullRequestStatus.Merged);
    }

    [Fact]
    public async Task Pipelines_can_filter_by_definition()
    {
        var runs = await Connection().Pipelines!.ListRunsAsync(new PipelineRunQuery { DefinitionId = "release" });
        Assert.All(runs, r => Assert.Equal("release", r.DefinitionId));
    }

    [Fact]
    public async Task WorkItems_excludes_completed_by_default()
    {
        var items = await Connection().WorkItems!.ListAsync(new WorkItemQuery());
        Assert.DoesNotContain(items, w => w.StateCategory == WorkItemStateCategory.Completed);
    }
}
