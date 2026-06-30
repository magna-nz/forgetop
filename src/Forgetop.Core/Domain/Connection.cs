namespace Forgetop.Core.Domain;

/// <summary>
/// A configured connection to a provider: identity, optional scope, and a
/// reference to the PAT held in the secret store (never the secret itself).
/// This is both persisted in config and handed to a provider factory to build
/// a live <see cref="Providers.IProviderConnection"/>.
/// </summary>
public sealed record Connection
{
    public required string Id { get; init; }
    public required ProviderType ProviderType { get; init; }
    public required string DisplayName { get; init; }

    /// <summary>Override base URL for self-hosted / non-default instances.</summary>
    public string? BaseUrl { get; init; }

    // Optional scoping — interpreted per provider (org/owner, project, repo, team).
    public string? Organization { get; init; }
    public string? Project { get; init; }
    public string? Repository { get; init; }

    /// <summary>Key into the <c>ISecretStore</c> for this connection's PAT.</summary>
    public string? CredentialRef { get; init; }

    /// <summary>Generates a stable, readable id for a new connection.</summary>
    public static string NewId(ProviderType provider) =>
        $"{provider.ToString().ToLowerInvariant()}-{Guid.NewGuid():N}";
}
