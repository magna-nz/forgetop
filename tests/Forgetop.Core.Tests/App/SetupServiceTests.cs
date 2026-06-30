using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Forgetop.Core.Secrets;
using Forgetop.Providers.Demo;

namespace Forgetop.Core.Tests.App;

public class SetupServiceTests
{
    private static (SetupService setup, ConfigService config) Build()
    {
        var registry = new ProviderRegistry([new DemoProviderFactory()]);
        var config = new ConfigService(new InMemoryConfigStore(), new InMemorySecretStore(), registry);
        return (new SetupService(config), config);
    }

    [Fact]
    public async Task ConfigureSection_creates_connection_and_binds_pull_requests()
    {
        var (setup, config) = Build();
        var id = await setup.ConfigureSectionAsync(Section.PullRequests, ProviderType.Demo, "Demo Org");

        Assert.Single(config.Current.Connections);
        Assert.Equal(id, config.Current.PullRequests?.ConnectionId);
    }

    [Fact]
    public async Task ConfigureSection_pipelines_subscribes_autodiscover()
    {
        var (setup, config) = Build();
        await setup.ConfigureSectionAsync(Section.Pipelines, ProviderType.Demo, "Demo CI");

        var sub = Assert.Single(config.Current.Pipelines!.Subscriptions);
        Assert.True(sub.AutoDiscoverAll);
    }

    [Fact]
    public async Task RemoveConnection_unbinds()
    {
        var (setup, config) = Build();
        var id = await setup.ConfigureSectionAsync(Section.WorkItems, ProviderType.Demo, "Demo");

        await setup.RemoveConnectionAsync(id);

        Assert.Empty(config.Current.Connections);
        Assert.Null(config.Current.WorkItems);
    }
}
