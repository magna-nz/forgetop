using Forgetop.Core.Secrets;

namespace Forgetop.Core.Tests.Secrets;

public class InMemorySecretStoreTests
{
    [Fact]
    public async Task Set_then_get_round_trips_and_delete_removes()
    {
        var store = new InMemorySecretStore();
        await store.SetAsync("gh-1", "token");

        Assert.Equal("token", await store.GetAsync("gh-1"));

        await store.DeleteAsync("gh-1");
        Assert.Null(await store.GetAsync("gh-1"));
    }
}

public class EnvironmentSecretStoreTests
{
    [Fact]
    public async Task Get_reads_prefixed_env_var()
    {
        var key = "test-conn";
        var envName = EnvironmentSecretStore.EnvVarName(key);
        Assert.Equal("FORGETOP_PAT_TEST_CONN", envName);

        Environment.SetEnvironmentVariable(envName, "secret-from-env");
        try
        {
            var store = new EnvironmentSecretStore();
            Assert.Equal("secret-from-env", await store.GetAsync(key));
            Assert.False(store.IsWritable);
        }
        finally
        {
            Environment.SetEnvironmentVariable(envName, null);
        }
    }

    [Fact]
    public async Task Writes_are_not_supported()
    {
        var store = new EnvironmentSecretStore();
        await Assert.ThrowsAsync<NotSupportedException>(() => store.SetAsync("k", "v"));
        await Assert.ThrowsAsync<NotSupportedException>(() => store.DeleteAsync("k"));
    }
}

public class FallbackSecretStoreTests
{
    [Fact]
    public async Task Reads_primary_first_then_fallback()
    {
        var primary = new InMemorySecretStore();
        var fallback = new InMemorySecretStore();
        await fallback.SetAsync("only-in-fallback", "fb");
        await primary.SetAsync("in-primary", "pr");

        var store = new FallbackSecretStore(primary, fallback);

        Assert.Equal("pr", await store.GetAsync("in-primary"));
        Assert.Equal("fb", await store.GetAsync("only-in-fallback"));
        Assert.Null(await store.GetAsync("missing"));
    }

    [Fact]
    public async Task Writes_go_to_primary()
    {
        var primary = new InMemorySecretStore();
        var fallback = new InMemorySecretStore();
        var store = new FallbackSecretStore(primary, fallback);

        await store.SetAsync("k", "v");

        Assert.Equal("v", await primary.GetAsync("k"));
        Assert.Null(await fallback.GetAsync("k"));
    }
}
