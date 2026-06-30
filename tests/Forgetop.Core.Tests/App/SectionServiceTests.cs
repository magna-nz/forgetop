using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Forgetop.Core.Secrets;
using Forgetop.Providers.Demo;

namespace Forgetop.Core.Tests.App;

public class SectionServiceTests
{
    private static async Task<(SectionService section, ConfigService config)> BuildAsync(bool bind)
    {
        var registry = new ProviderRegistry([new DemoProviderFactory()]);
        var config = new ConfigService(new Support.RecordingConfigStore(), new InMemorySecretStore(), registry);

        if (bind)
        {
            await config.AddOrUpdateConnectionAsync(new Connection
            {
                Id = "demo", ProviderType = ProviderType.Demo, DisplayName = "Demo",
            });
            await config.BindPullRequestsAsync("demo");
            await config.BindWorkItemsAsync("demo");
            await config.SubscribePipelineAsync("demo", "ci");
        }

        var section = new SectionService(config, new ConnectionResolver(config, registry, new InMemorySecretStore()));
        return (section, config);
    }

    [Fact]
    public async Task Resolves_bound_pull_request_source_and_loads()
    {
        var (section, _) = await BuildAsync(bind: true);
        var source = await section.GetPullRequestSourceAsync();
        Assert.NotNull(source);
        var prs = await source!.ListAsync(new PullRequestQuery());
        Assert.NotEmpty(prs);
    }

    [Fact]
    public async Task Resolves_bound_work_item_source()
    {
        var (section, _) = await BuildAsync(bind: true);
        Assert.NotNull(await section.GetWorkItemSourceAsync());
    }

    [Fact]
    public async Task Resolves_pipeline_feeds_for_each_subscription()
    {
        var (section, _) = await BuildAsync(bind: true);
        var feeds = await section.GetPipelineFeedsAsync();
        var feed = Assert.Single(feeds);
        Assert.Equal("demo", feed.Connection.ConnectionId);
        Assert.Contains("ci", feed.Subscription.DefinitionIds);
    }

    [Fact]
    public async Task Unbound_sections_return_null_and_empty()
    {
        var (section, _) = await BuildAsync(bind: false);
        Assert.Null(await section.GetPullRequestSourceAsync());
        Assert.Null(await section.GetWorkItemSourceAsync());
        Assert.Empty(await section.GetPipelineFeedsAsync());
    }
}
