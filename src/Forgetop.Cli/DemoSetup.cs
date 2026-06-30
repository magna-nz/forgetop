using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;

namespace Forgetop.Cli;

/// <summary>Seeds an in-memory demo configuration: one Demo connection bound to all three sections.</summary>
public static class DemoSetup
{
    public static async Task ApplyAsync(IConfigService config, CancellationToken ct = default)
    {
        await config.AddOrUpdateConnectionAsync(new Connection
        {
            Id = "demo",
            ProviderType = ProviderType.Demo,
            DisplayName = "Demo Org",
        }, secret: null, ct);

        await config.BindPullRequestsAsync("demo", ct);
        await config.BindWorkItemsAsync("demo", ct);
        await config.SetPipelineAutoDiscoverAsync("demo", autoDiscoverAll: true, ct);
    }
}
