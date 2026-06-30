using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Forgetop.Core.Tests.Support;

namespace Forgetop.Core.Tests.Providers;

public class CapabilitiesTests
{
    [Theory]
    [InlineData(Section.PullRequests, true, false, false, true)]
    [InlineData(Section.WorkItems, false, true, false, true)]
    [InlineData(Section.Pipelines, false, false, true, true)]
    [InlineData(Section.Pipelines, true, true, false, false)]
    public void Supports_reflects_flags(Section section, bool pr, bool wi, bool pipe, bool expected)
    {
        var caps = new ProviderCapabilities
        {
            SupportsPullRequests = pr,
            SupportsWorkItems = wi,
            SupportsPipelines = pipe,
        };

        Assert.Equal(expected, caps.Supports(section));
    }
}

public class ProviderRegistryTests
{
    private static ProviderRegistry BuildRegistry() => new(new IProviderFactory[]
    {
        new FakeProviderFactory(ProviderType.GitHub, new ProviderCapabilities { SupportsPullRequests = true }),
        new FakeProviderFactory(ProviderType.Linear, new ProviderCapabilities { SupportsWorkItems = true }),
    });

    [Fact]
    public void AvailableProviders_lists_registered()
    {
        var registry = BuildRegistry();
        Assert.Contains(ProviderType.GitHub, registry.AvailableProviders);
        Assert.Contains(ProviderType.Linear, registry.AvailableProviders);
        Assert.False(registry.Supports(ProviderType.AzureDevOps));
    }

    [Fact]
    public void Create_dispatches_to_matching_factory()
    {
        var registry = BuildRegistry();
        var connection = new Connection
        {
            Id = "gh-1",
            ProviderType = ProviderType.GitHub,
            DisplayName = "GitHub",
        };

        var live = registry.Create(connection, secret: "pat");

        Assert.Equal(ProviderType.GitHub, live.ProviderType);
        Assert.True(live.Capabilities.SupportsPullRequests);
    }

    [Fact]
    public void Create_throws_for_unregistered_provider()
    {
        var registry = BuildRegistry();
        var connection = new Connection
        {
            Id = "ado-1",
            ProviderType = ProviderType.AzureDevOps,
            DisplayName = "ADO",
        };

        Assert.Throws<InvalidOperationException>(() => registry.Create(connection, null));
    }
}
