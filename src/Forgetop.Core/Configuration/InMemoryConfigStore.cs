namespace Forgetop.Core.Configuration;

/// <summary>
/// Non-persistent config store. Used for <c>--demo</c> so the demo session never
/// touches the user's real config file.
/// </summary>
public sealed class InMemoryConfigStore : IConfigStore
{
    private ForgetopConfig _config;

    public InMemoryConfigStore(ForgetopConfig? seed = null) => _config = seed ?? ForgetopConfig.Empty;

    public Task<ForgetopConfig> LoadAsync(CancellationToken ct = default) => Task.FromResult(_config);

    public Task SaveAsync(ForgetopConfig config, CancellationToken ct = default)
    {
        _config = config;
        return Task.CompletedTask;
    }
}
