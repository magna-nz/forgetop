using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;

namespace Forgetop.Core.App;

/// <summary>A configured connection and whether it's currently reachable/authed.</summary>
public sealed record ConnectionHealth(Connection Connection, bool Healthy);

/// <summary>Probes each configured connection for the connections health bar.</summary>
public sealed class ConnectionHealthService
{
    private readonly IConfigService _config;
    private readonly ConnectionResolver _resolver;

    public ConnectionHealthService(IConfigService config, ConnectionResolver resolver)
    {
        _config = config ?? throw new ArgumentNullException(nameof(config));
        _resolver = resolver ?? throw new ArgumentNullException(nameof(resolver));
    }

    public async Task<IReadOnlyList<ConnectionHealth>> CheckAllAsync(CancellationToken ct = default)
    {
        var results = new List<ConnectionHealth>();
        foreach (var connection in _config.Current.Connections)
        {
            bool healthy;
            try
            {
                var live = await _resolver.ResolveAsync(connection.Id, ct).ConfigureAwait(false);
                healthy = live is not null && await live.CheckAsync(ct).ConfigureAwait(false);
            }
            catch
            {
                healthy = false;
            }

            results.Add(new ConnectionHealth(connection, healthy));
        }

        return results;
    }
}
