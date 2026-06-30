using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Providers;
using Forgetop.Core.Secrets;
using Forgetop.Providers.AzureDevOps;
using Forgetop.Providers.Demo;
using Forgetop.Providers.GitHub;
using Forgetop.Providers.Linear;
using Forgetop.Tui;
using Microsoft.Extensions.DependencyInjection;

namespace Forgetop.Cli;

/// <summary>Composition root: wires providers, config, secrets and the TUI app.</summary>
public static class AppHost
{
    public static ServiceProvider Build(bool demo)
    {
        var services = new ServiceCollection();

        // Providers
        services.AddSingleton<IProviderFactory, DemoProviderFactory>();
        services.AddSingleton<IProviderFactory, GitHubProviderFactory>();
        services.AddSingleton<IProviderFactory, AzureDevOpsProviderFactory>();
        services.AddSingleton<IProviderFactory, LinearProviderFactory>();
        services.AddSingleton<IProviderRegistry, ProviderRegistry>();

        // Storage — demo never touches the user's real config or keychain.
        if (demo)
        {
            services.AddSingleton<IConfigStore>(_ => new InMemoryConfigStore());
            services.AddSingleton<ISecretStore, InMemorySecretStore>();
        }
        else
        {
            services.AddSingleton<IConfigStore>(_ => new JsonConfigStore());
            services.AddSingleton<ISecretStore>(_ => OsSecretStore.CreateDefault());
        }

        services.AddSingleton<IConfigService, ConfigService>();
        services.AddSingleton<ConnectionResolver>();
        services.AddSingleton<SectionService>();
        services.AddSingleton<SetupService>();
        services.AddSingleton<ForgetopApp>();

        return services.BuildServiceProvider();
    }
}
