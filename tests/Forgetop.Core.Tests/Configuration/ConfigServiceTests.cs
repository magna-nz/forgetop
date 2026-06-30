using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Forgetop.Core.Secrets;
using Forgetop.Core.Tests.Support;

namespace Forgetop.Core.Tests.Configuration;

public class ConfigServiceTests
{
    private static ProviderRegistry Registry() => new(new IProviderFactory[]
    {
        new FakeProviderFactory(ProviderType.GitHub, new ProviderCapabilities
        {
            SupportsPullRequests = true,
            SupportsPipelines = true,
        }),
        new FakeProviderFactory(ProviderType.Linear, new ProviderCapabilities { SupportsWorkItems = true }),
    });

    private static (ConfigService service, InMemoryConfigStore store, InMemorySecretStore secrets) Build()
    {
        var store = new InMemoryConfigStore();
        var secrets = new InMemorySecretStore();
        return (new ConfigService(store, secrets, Registry()), store, secrets);
    }

    private static Connection Conn(string id, ProviderType type) =>
        new() { Id = id, ProviderType = type, DisplayName = id };

    [Fact]
    public async Task AddConnection_persists_stores_secret_and_raises()
    {
        var (service, store, secrets) = Build();
        ConfigChangedEventArgs? captured = null;
        service.Changed += (_, e) => captured = e;

        await service.AddOrUpdateConnectionAsync(Conn("gh-1", ProviderType.GitHub), secret: "pat-123");

        Assert.Single(service.Current.Connections);
        Assert.Equal(1, store.SaveCount);
        Assert.Equal("pat-123", await secrets.GetAsync("gh-1"));
        Assert.NotNull(captured);
        Assert.Null(captured!.AffectedSection);
    }

    [Fact]
    public async Task AddConnection_with_secret_throws_on_readonly_store()
    {
        var service = new ConfigService(new InMemoryConfigStore(), new EnvironmentSecretStore(), Registry());

        await Assert.ThrowsAsync<InvalidOperationException>(
            () => service.AddOrUpdateConnectionAsync(Conn("gh-1", ProviderType.GitHub), secret: "pat"));
    }

    [Fact]
    public async Task BindPullRequests_succeeds_for_capable_connection()
    {
        var (service, _, _) = Build();
        await service.AddOrUpdateConnectionAsync(Conn("gh-1", ProviderType.GitHub));

        Section? affected = null;
        service.Changed += (_, e) => affected = e.AffectedSection;

        await service.BindPullRequestsAsync("gh-1");

        Assert.Equal("gh-1", service.Current.PullRequests?.ConnectionId);
        Assert.Equal(Section.PullRequests, affected);
    }

    [Fact]
    public async Task BindPullRequests_rejects_incapable_connection()
    {
        var (service, _, _) = Build();
        await service.AddOrUpdateConnectionAsync(Conn("lin-1", ProviderType.Linear));

        await Assert.ThrowsAsync<InvalidOperationException>(() => service.BindPullRequestsAsync("lin-1"));
        Assert.Null(service.Current.PullRequests);
    }

    [Fact]
    public async Task BindWorkItems_succeeds_for_linear()
    {
        var (service, _, _) = Build();
        await service.AddOrUpdateConnectionAsync(Conn("lin-1", ProviderType.Linear));

        await service.BindWorkItemsAsync("lin-1");

        Assert.Equal("lin-1", service.Current.WorkItems?.ConnectionId);
    }

    [Fact]
    public async Task RemoveConnection_cascades_to_bindings_and_secret()
    {
        var (service, _, secrets) = Build();
        await service.AddOrUpdateConnectionAsync(Conn("gh-1", ProviderType.GitHub), secret: "pat");
        await service.BindPullRequestsAsync("gh-1");
        await service.SubscribePipelineAsync("gh-1", "build.yml");

        await service.RemoveConnectionAsync("gh-1");

        Assert.Empty(service.Current.Connections);
        Assert.Null(service.Current.PullRequests);
        Assert.Empty(service.Current.Pipelines!.Subscriptions);
        Assert.Null(await secrets.GetAsync("gh-1"));
    }

    [Fact]
    public async Task Pipeline_subscribe_unsubscribe_and_autodiscover()
    {
        var (service, _, _) = Build();
        await service.AddOrUpdateConnectionAsync(Conn("gh-1", ProviderType.GitHub));

        await service.SubscribePipelineAsync("gh-1", "build.yml");
        await service.SubscribePipelineAsync("gh-1", "release.yml");
        await service.SubscribePipelineAsync("gh-1", "build.yml"); // duplicate ignored

        var sub = Assert.Single(service.Current.Pipelines!.Subscriptions);
        Assert.Equal(2, sub.DefinitionIds.Count);

        await service.SetPipelineAutoDiscoverAsync("gh-1", true);
        Assert.True(service.Current.Pipelines!.Subscriptions[0].AutoDiscoverAll);

        await service.UnsubscribePipelineAsync("gh-1", "build.yml");
        Assert.Single(service.Current.Pipelines!.Subscriptions[0].DefinitionIds);
    }

    [Fact]
    public async Task Subscribe_pipeline_rejects_incapable_connection()
    {
        var (service, _, _) = Build();
        await service.AddOrUpdateConnectionAsync(Conn("lin-1", ProviderType.Linear));

        await Assert.ThrowsAsync<InvalidOperationException>(() => service.SubscribePipelineAsync("lin-1", "x"));
    }

    [Fact]
    public async Task Load_reads_existing_config_into_current()
    {
        var store = new InMemoryConfigStore();
        await store.SaveAsync(new ForgetopConfig
        {
            Connections = [Conn("gh-1", ProviderType.GitHub)],
            PullRequests = new PullRequestBinding { ConnectionId = "gh-1" },
        });

        var service = new ConfigService(store, new InMemorySecretStore(), Registry());
        await service.LoadAsync();

        Assert.Single(service.Current.Connections);
        Assert.Equal("gh-1", service.Current.PullRequests?.ConnectionId);
    }
}
