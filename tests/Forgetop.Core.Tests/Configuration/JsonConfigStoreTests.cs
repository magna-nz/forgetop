using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;

namespace Forgetop.Core.Tests.Configuration;

public class JsonConfigStoreTests : IDisposable
{
    private readonly string _path = Path.Combine(Path.GetTempPath(), $"forgetop-test-{Guid.NewGuid():N}", "config.json");

    [Fact]
    public async Task Load_missing_file_returns_empty()
    {
        var store = new JsonConfigStore(_path);
        var config = await store.LoadAsync();
        Assert.Empty(config.Connections);
    }

    [Fact]
    public async Task Save_then_load_round_trips()
    {
        var store = new JsonConfigStore(_path);
        var config = new ForgetopConfig
        {
            Connections =
            [
                new Connection { Id = "gh-1", ProviderType = ProviderType.GitHub, DisplayName = "GitHub", Organization = "acme" },
            ],
            PullRequests = new PullRequestBinding { ConnectionId = "gh-1" },
            Pipelines = new PipelineBinding
            {
                Subscriptions = [new PipelineSubscription { ConnectionId = "gh-1", DefinitionIds = ["build.yml"] }],
            },
            Ui = new UiState { Theme = "dark", ActiveSection = Section.Pipelines },
        };

        await store.SaveAsync(config);
        var loaded = await store.LoadAsync();

        Assert.Single(loaded.Connections);
        Assert.Equal(ProviderType.GitHub, loaded.Connections[0].ProviderType);
        Assert.Equal("acme", loaded.Connections[0].Organization);
        Assert.Equal("gh-1", loaded.PullRequests?.ConnectionId);
        Assert.Equal("build.yml", loaded.Pipelines?.Subscriptions[0].DefinitionIds[0]);
        Assert.Equal(Section.Pipelines, loaded.Ui.ActiveSection);
    }

    [Fact]
    public async Task Enums_persist_as_strings()
    {
        var store = new JsonConfigStore(_path);
        await store.SaveAsync(new ForgetopConfig
        {
            Connections = [new Connection { Id = "lin-1", ProviderType = ProviderType.Linear, DisplayName = "Linear" }],
        });

        var json = await File.ReadAllTextAsync(_path);
        Assert.Contains("\"Linear\"", json);
        Assert.DoesNotContain("\"ProviderType\": 3", json);
    }

    public void Dispose()
    {
        var dir = Path.GetDirectoryName(_path);
        if (dir is not null && Directory.Exists(dir))
        {
            Directory.Delete(dir, recursive: true);
        }
    }
}
