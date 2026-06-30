using Forgetop.Core.Configuration;
using Forgetop.Core.Providers;
using Forgetop.Core.Secrets;

namespace Forgetop.Core.App;

/// <summary>
/// Turns a configured connection id into a live <see cref="IProviderConnection"/>
/// by resolving its PAT from the secret store and asking the registry to build it.
/// </summary>
public sealed class ConnectionResolver
{
    private readonly IConfigService _config;
    private readonly IProviderRegistry _registry;
    private readonly ISecretStore _secrets;

    public ConnectionResolver(IConfigService config, IProviderRegistry registry, ISecretStore secrets)
    {
        _config = config ?? throw new ArgumentNullException(nameof(config));
        _registry = registry ?? throw new ArgumentNullException(nameof(registry));
        _secrets = secrets ?? throw new ArgumentNullException(nameof(secrets));
    }

    public async Task<IProviderConnection?> ResolveAsync(string connectionId, CancellationToken ct = default)
    {
        var connection = _config.Current.FindConnection(connectionId);
        if (connection is null || !_registry.Supports(connection.ProviderType))
        {
            return null;
        }

        var secret = connection.CredentialRef is null
            ? null
            : await _secrets.GetAsync(connection.CredentialRef, ct).ConfigureAwait(false);

        return _registry.Create(connection, secret);
    }
}
