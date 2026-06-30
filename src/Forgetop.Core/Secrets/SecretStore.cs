namespace Forgetop.Core.Secrets;

/// <summary>
/// Stores per-connection PATs. The production implementation will wrap the OS
/// native store (DPAPI / libsecret / Keychain); v1 ships an environment-variable
/// fallback plus an in-memory store for tests.
/// </summary>
public interface ISecretStore
{
    Task<string?> GetAsync(string key, CancellationToken ct = default);
    Task SetAsync(string key, string secret, CancellationToken ct = default);
    Task DeleteAsync(string key, CancellationToken ct = default);

    /// <summary>False for read-only stores (e.g. the env-var fallback).</summary>
    bool IsWritable { get; }
}

/// <summary>
/// Read-only fallback that resolves secrets from environment variables of the
/// form <c>FORGETOP_PAT_{KEY}</c> (key upper-cased, non-alphanumerics → '_').
/// </summary>
public sealed class EnvironmentSecretStore : ISecretStore
{
    public const string Prefix = "FORGETOP_PAT_";

    public bool IsWritable => false;

    public Task<string?> GetAsync(string key, CancellationToken ct = default)
    {
        var value = Environment.GetEnvironmentVariable(EnvVarName(key));
        return Task.FromResult(string.IsNullOrEmpty(value) ? null : value);
    }

    public Task SetAsync(string key, string secret, CancellationToken ct = default) =>
        throw new NotSupportedException("The environment secret store is read-only.");

    public Task DeleteAsync(string key, CancellationToken ct = default) =>
        throw new NotSupportedException("The environment secret store is read-only.");

    public static string EnvVarName(string key)
    {
        var sanitized = new string(key.Select(c => char.IsLetterOrDigit(c) ? char.ToUpperInvariant(c) : '_').ToArray());
        return Prefix + sanitized;
    }
}

/// <summary>In-memory store for tests and the Demo provider.</summary>
public sealed class InMemorySecretStore : ISecretStore
{
    private readonly Dictionary<string, string> _secrets = new(StringComparer.Ordinal);

    public bool IsWritable => true;

    public Task<string?> GetAsync(string key, CancellationToken ct = default) =>
        Task.FromResult(_secrets.GetValueOrDefault(key));

    public Task SetAsync(string key, string secret, CancellationToken ct = default)
    {
        _secrets[key] = secret;
        return Task.CompletedTask;
    }

    public Task DeleteAsync(string key, CancellationToken ct = default)
    {
        _secrets.Remove(key);
        return Task.CompletedTask;
    }
}

/// <summary>
/// Tries a writable primary store first (e.g. OS keychain), then falls back to
/// a read-only secondary (e.g. environment variables) on read.
/// </summary>
public sealed class FallbackSecretStore : ISecretStore
{
    private readonly ISecretStore _primary;
    private readonly ISecretStore _fallback;

    public FallbackSecretStore(ISecretStore primary, ISecretStore fallback)
    {
        _primary = primary ?? throw new ArgumentNullException(nameof(primary));
        _fallback = fallback ?? throw new ArgumentNullException(nameof(fallback));
    }

    public bool IsWritable => _primary.IsWritable;

    public async Task<string?> GetAsync(string key, CancellationToken ct = default) =>
        await _primary.GetAsync(key, ct).ConfigureAwait(false)
        ?? await _fallback.GetAsync(key, ct).ConfigureAwait(false);

    public Task SetAsync(string key, string secret, CancellationToken ct = default) =>
        _primary.SetAsync(key, secret, ct);

    public Task DeleteAsync(string key, CancellationToken ct = default) =>
        _primary.DeleteAsync(key, ct);
}
