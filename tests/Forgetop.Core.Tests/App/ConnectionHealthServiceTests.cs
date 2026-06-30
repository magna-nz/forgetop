using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Forgetop.Core.Secrets;
using Forgetop.Providers.Demo;

namespace Forgetop.Core.Tests.App;

public class ConnectionHealthServiceTests
{
    [Fact]
    public async Task Reports_healthy_for_resolvable_connection()
    {
        var registry = new ProviderRegistry([new DemoProviderFactory()]);
        var config = new ConfigService(new InMemoryConfigStore(), new InMemorySecretStore(), registry);
        await config.AddOrUpdateConnectionAsync(new Connection { Id = "demo", ProviderType = ProviderType.Demo, DisplayName = "Demo" });

        var service = new ConnectionHealthService(config, new ConnectionResolver(config, registry, new InMemorySecretStore()));
        var health = await service.CheckAllAsync();

        var item = Assert.Single(health);
        Assert.True(item.Healthy);
        Assert.Equal("Demo", item.Connection.DisplayName);
    }

    [Fact]
    public async Task Reports_unhealthy_when_provider_not_registered()
    {
        var demoRegistry = new ProviderRegistry([new DemoProviderFactory()]);
        var config = new ConfigService(new InMemoryConfigStore(), new InMemorySecretStore(), demoRegistry);
        // Seed a connection whose provider the registry can't build.
        await config.AddOrUpdateConnectionAsync(new Connection { Id = "demo", ProviderType = ProviderType.Demo, DisplayName = "Demo" });
        var emptyRegistry = new ProviderRegistry([]);

        var service = new ConnectionHealthService(config, new ConnectionResolver(config, emptyRegistry, new InMemorySecretStore()));
        var health = await service.CheckAllAsync();

        Assert.False(Assert.Single(health).Healthy);
    }
}
