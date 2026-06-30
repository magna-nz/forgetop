using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Forgetop.Core.Secrets;

namespace Forgetop.Core.Configuration;

/// <summary>Raised after any configuration mutation.</summary>
public sealed class ConfigChangedEventArgs : EventArgs
{
    public ConfigChangedEventArgs(ForgetopConfig config, Section? affectedSection)
    {
        Config = config;
        AffectedSection = affectedSection;
    }

    public ForgetopConfig Config { get; }

    /// <summary>The section that changed, or null for a broad change (e.g. a connection was removed).</summary>
    public Section? AffectedSection { get; }
}

/// <summary>
/// Holds the live configuration and applies mutations at runtime: each change is
/// validated against provider capabilities, persisted, and broadcast via
/// <see cref="Changed"/> so the affected section can refresh.
/// </summary>
public interface IConfigService
{
    ForgetopConfig Current { get; }
    event EventHandler<ConfigChangedEventArgs>? Changed;

    Task LoadAsync(CancellationToken ct = default);

    Task AddOrUpdateConnectionAsync(Connection connection, string? secret = null, CancellationToken ct = default);
    Task RemoveConnectionAsync(string connectionId, CancellationToken ct = default);

    Task BindPullRequestsAsync(string connectionId, CancellationToken ct = default);
    Task BindWorkItemsAsync(string connectionId, CancellationToken ct = default);
    Task UnbindSectionAsync(Section section, CancellationToken ct = default);

    Task SubscribePipelineAsync(string connectionId, string definitionId, CancellationToken ct = default);
    Task SetPipelineAutoDiscoverAsync(string connectionId, bool autoDiscoverAll, CancellationToken ct = default);
    Task UnsubscribePipelineAsync(string connectionId, string definitionId, CancellationToken ct = default);
    Task RemovePipelineConnectionAsync(string connectionId, CancellationToken ct = default);
}

/// <inheritdoc cref="IConfigService"/>
public sealed class ConfigService : IConfigService
{
    private readonly IConfigStore _store;
    private readonly ISecretStore _secrets;
    private readonly IProviderRegistry _registry;
    private readonly SemaphoreSlim _gate = new(1, 1);

    private ForgetopConfig _config = ForgetopConfig.Empty;

    public ConfigService(IConfigStore store, ISecretStore secrets, IProviderRegistry registry)
    {
        _store = store ?? throw new ArgumentNullException(nameof(store));
        _secrets = secrets ?? throw new ArgumentNullException(nameof(secrets));
        _registry = registry ?? throw new ArgumentNullException(nameof(registry));
    }

    public ForgetopConfig Current => _config;

    public event EventHandler<ConfigChangedEventArgs>? Changed;

    public async Task LoadAsync(CancellationToken ct = default)
    {
        await _gate.WaitAsync(ct).ConfigureAwait(false);
        try
        {
            _config = await _store.LoadAsync(ct).ConfigureAwait(false);
        }
        finally
        {
            _gate.Release();
        }

        Raise(affectedSection: null);
    }

    public Task AddOrUpdateConnectionAsync(Connection connection, string? secret = null, CancellationToken ct = default)
    {
        ArgumentNullException.ThrowIfNull(connection);

        return MutateAsync(null, ct, async config =>
        {
            var credentialRef = connection.CredentialRef ?? connection.Id;
            var stored = connection with { CredentialRef = credentialRef };

            if (secret is not null)
            {
                if (!_secrets.IsWritable)
                {
                    throw new InvalidOperationException(
                        "Secret store is read-only; provide the PAT via environment variable instead.");
                }

                await _secrets.SetAsync(credentialRef, secret, ct).ConfigureAwait(false);
            }

            var connections = config.Connections.Where(c => c.Id != stored.Id).Append(stored).ToList();
            return config with { Connections = connections };
        });
    }

    public Task RemoveConnectionAsync(string connectionId, CancellationToken ct = default) =>
        MutateAsync(null, ct, async config =>
        {
            var existing = config.FindConnection(connectionId);
            if (existing is null)
            {
                return config;
            }

            if (existing.CredentialRef is not null && _secrets.IsWritable)
            {
                await _secrets.DeleteAsync(existing.CredentialRef, ct).ConfigureAwait(false);
            }

            // Cascade: drop any section binding that referenced this connection.
            var pipelines = config.Pipelines is null
                ? null
                : config.Pipelines with
                {
                    Subscriptions = config.Pipelines.Subscriptions.Where(s => s.ConnectionId != connectionId).ToList(),
                };

            return config with
            {
                Connections = config.Connections.Where(c => c.Id != connectionId).ToList(),
                PullRequests = config.PullRequests?.ConnectionId == connectionId ? null : config.PullRequests,
                WorkItems = config.WorkItems?.ConnectionId == connectionId ? null : config.WorkItems,
                Pipelines = pipelines,
            };
        });

    public Task BindPullRequestsAsync(string connectionId, CancellationToken ct = default) =>
        MutateAsync(Section.PullRequests, ct, config =>
        {
            EnsureSupports(config, connectionId, Section.PullRequests);
            return Task.FromResult(config with { PullRequests = new PullRequestBinding { ConnectionId = connectionId } });
        });

    public Task BindWorkItemsAsync(string connectionId, CancellationToken ct = default) =>
        MutateAsync(Section.WorkItems, ct, config =>
        {
            EnsureSupports(config, connectionId, Section.WorkItems);
            return Task.FromResult(config with { WorkItems = new WorkItemBinding { ConnectionId = connectionId } });
        });

    public Task UnbindSectionAsync(Section section, CancellationToken ct = default) =>
        MutateAsync(section, ct, config => Task.FromResult(section switch
        {
            Section.PullRequests => config with { PullRequests = null },
            Section.WorkItems => config with { WorkItems = null },
            Section.Pipelines => config with { Pipelines = null },
            _ => config,
        }));

    public Task SubscribePipelineAsync(string connectionId, string definitionId, CancellationToken ct = default) =>
        MutatePipelineSubscription(connectionId, ct, sub => sub with
        {
            DefinitionIds = sub.DefinitionIds.Contains(definitionId)
                ? sub.DefinitionIds
                : sub.DefinitionIds.Append(definitionId).ToList(),
        });

    public Task SetPipelineAutoDiscoverAsync(string connectionId, bool autoDiscoverAll, CancellationToken ct = default) =>
        MutatePipelineSubscription(connectionId, ct, sub => sub with { AutoDiscoverAll = autoDiscoverAll });

    public Task UnsubscribePipelineAsync(string connectionId, string definitionId, CancellationToken ct = default) =>
        MutatePipelineSubscription(connectionId, ct, sub => sub with
        {
            DefinitionIds = sub.DefinitionIds.Where(id => id != definitionId).ToList(),
        });

    public Task RemovePipelineConnectionAsync(string connectionId, CancellationToken ct = default) =>
        MutateAsync(Section.Pipelines, ct, config =>
        {
            if (config.Pipelines is null)
            {
                return Task.FromResult(config);
            }

            return Task.FromResult(config with
            {
                Pipelines = config.Pipelines with
                {
                    Subscriptions = config.Pipelines.Subscriptions.Where(s => s.ConnectionId != connectionId).ToList(),
                },
            });
        });

    private Task MutatePipelineSubscription(string connectionId, CancellationToken ct, Func<PipelineSubscription, PipelineSubscription> update) =>
        MutateAsync(Section.Pipelines, ct, config =>
        {
            EnsureSupports(config, connectionId, Section.Pipelines);

            var binding = config.Pipelines ?? new PipelineBinding();
            var existing = binding.Subscriptions.FirstOrDefault(s => s.ConnectionId == connectionId)
                           ?? new PipelineSubscription { ConnectionId = connectionId };
            var updated = update(existing);

            var subscriptions = binding.Subscriptions
                .Where(s => s.ConnectionId != connectionId)
                .Append(updated)
                .ToList();

            return Task.FromResult(config with { Pipelines = binding with { Subscriptions = subscriptions } });
        });

    private void EnsureSupports(ForgetopConfig config, string connectionId, Section section)
    {
        var connection = config.FindConnection(connectionId)
            ?? throw new InvalidOperationException($"Unknown connection '{connectionId}'.");

        if (!_registry.DescribeCapabilities(connection.ProviderType).Supports(section))
        {
            throw new InvalidOperationException(
                $"Connection '{connection.DisplayName}' ({connection.ProviderType}) does not support {section}.");
        }
    }

    private async Task MutateAsync(Section? affectedSection, CancellationToken ct, Func<ForgetopConfig, Task<ForgetopConfig>> mutate)
    {
        await _gate.WaitAsync(ct).ConfigureAwait(false);
        try
        {
            var updated = await mutate(_config).ConfigureAwait(false);
            if (ReferenceEquals(updated, _config))
            {
                return;
            }

            await _store.SaveAsync(updated, ct).ConfigureAwait(false);
            _config = updated;
        }
        finally
        {
            _gate.Release();
        }

        Raise(affectedSection);
    }

    private void Raise(Section? affectedSection) =>
        Changed?.Invoke(this, new ConfigChangedEventArgs(_config, affectedSection));
}
