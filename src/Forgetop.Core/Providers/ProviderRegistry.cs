using Forgetop.Core.Domain;

namespace Forgetop.Core.Providers;

/// <summary>Builds a live <see cref="IProviderConnection"/> for one provider type.</summary>
public interface IProviderFactory
{
    ProviderType ProviderType { get; }

    /// <summary>The capabilities a connection of this provider type exposes.</summary>
    ProviderCapabilities DescribeCapabilities();

    /// <param name="secret">The resolved PAT, or null for providers that need none (Demo).</param>
    IProviderConnection Create(Connection connection, string? secret);
}

/// <summary>Resolves provider factories and creates connections by provider type.</summary>
public interface IProviderRegistry
{
    IReadOnlyCollection<ProviderType> AvailableProviders { get; }

    bool Supports(ProviderType provider);

    ProviderCapabilities DescribeCapabilities(ProviderType provider);

    IProviderConnection Create(Connection connection, string? secret);
}

/// <summary>Default registry backed by the set of registered factories.</summary>
public sealed class ProviderRegistry : IProviderRegistry
{
    private readonly IReadOnlyDictionary<ProviderType, IProviderFactory> _factories;

    public ProviderRegistry(IEnumerable<IProviderFactory> factories)
    {
        ArgumentNullException.ThrowIfNull(factories);
        _factories = factories.ToDictionary(f => f.ProviderType);
    }

    public IReadOnlyCollection<ProviderType> AvailableProviders => _factories.Keys.ToArray();

    public bool Supports(ProviderType provider) => _factories.ContainsKey(provider);

    public ProviderCapabilities DescribeCapabilities(ProviderType provider) =>
        Resolve(provider).DescribeCapabilities();

    public IProviderConnection Create(Connection connection, string? secret)
    {
        ArgumentNullException.ThrowIfNull(connection);
        return Resolve(connection.ProviderType).Create(connection, secret);
    }

    private IProviderFactory Resolve(ProviderType provider) =>
        _factories.TryGetValue(provider, out var factory)
            ? factory
            : throw new InvalidOperationException($"No provider registered for '{provider}'.");
}
