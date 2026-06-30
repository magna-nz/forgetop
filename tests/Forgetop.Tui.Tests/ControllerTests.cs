using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Forgetop.Core.Secrets;
using Forgetop.Providers.Demo;
using Forgetop.Tui;

namespace Forgetop.Tui.Tests;

public class ControllerTests
{
    private static async Task<(SectionService sections, ConfigService config)> BoundDemoAsync()
    {
        var registry = new ProviderRegistry([new DemoProviderFactory()]);
        var config = new ConfigService(new InMemoryConfigStore(), new InMemorySecretStore(), registry);
        await config.AddOrUpdateConnectionAsync(new Connection { Id = "demo", ProviderType = ProviderType.Demo, DisplayName = "Demo Org" });
        await config.BindPullRequestsAsync("demo");
        await config.BindWorkItemsAsync("demo");
        await config.SetPipelineAutoDiscoverAsync("demo", autoDiscoverAll: true);
        var sections = new SectionService(config, new ConnectionResolver(config, registry, new InMemorySecretStore()));
        return (sections, config);
    }

    [Fact]
    public async Task PullRequestController_loads_and_cycles_filter()
    {
        var (sections, config) = await BoundDemoAsync();
        var controller = new PullRequestController(sections, config);

        var all = await controller.LoadAsync();
        Assert.NotEmpty(all.Rows);

        Assert.Equal("Mine", controller.CycleFilter());
        var mine = await controller.LoadAsync();
        // Demo's "alice" authored PR #101; Mine should narrow the list.
        Assert.True(mine.Rows.Count <= all.Rows.Count);
        Assert.NotEmpty(mine.Rows);
    }

    [Fact]
    public async Task PullRequestController_vote_and_merge_succeed_on_selected()
    {
        var (sections, config) = await BoundDemoAsync();
        var controller = new PullRequestController(sections, config);
        await controller.LoadAsync();

        Assert.True(await controller.VoteAsync(0, ReviewVote.Approved));
        Assert.True(await controller.MergeAsync(0, new MergeOptions()));
        Assert.False(await controller.VoteAsync(999, ReviewVote.Approved)); // out of range
    }

    [Fact]
    public async Task WorkItemController_loads_and_sets_state()
    {
        var (sections, config) = await BoundDemoAsync();
        var controller = new WorkItemController(sections, config);
        var data = await controller.LoadAsync();

        Assert.NotEmpty(data.Rows);
        Assert.True(await controller.SetStateAsync(0, "Done"));
    }

    [Fact]
    public async Task PipelineController_aggregates_discovers_and_subscribes()
    {
        var (sections, config) = await BoundDemoAsync();
        var controller = new PipelineController(sections, config);

        var data = await controller.LoadAsync();
        Assert.NotEmpty(data.Rows);

        var detail = await controller.GetRunDetailAsync(0);
        Assert.Contains("Logs:", detail);

        var discovered = await controller.DiscoverAsync();
        Assert.NotEmpty(discovered);

        await controller.SubscribeAsync(discovered[0]);
        Assert.NotNull(config.Current.Pipelines);
    }
}
