using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;

namespace Forgetop.Core.App;

/// <summary>
/// Orchestrates configuring a section from the setup wizard / config UI: creates
/// a connection, stores its PAT, and binds it to the chosen section.
/// </summary>
public sealed class SetupService
{
    private readonly IConfigService _config;

    public SetupService(IConfigService config) => _config = config ?? throw new ArgumentNullException(nameof(config));

    public async Task<string> ConfigureSectionAsync(
        Section section,
        ProviderType provider,
        string displayName,
        string? organization = null,
        string? project = null,
        string? repository = null,
        string? pat = null,
        CancellationToken ct = default)
    {
        var connection = new Connection
        {
            Id = Connection.NewId(provider),
            ProviderType = provider,
            DisplayName = displayName,
            Organization = organization,
            Project = project,
            Repository = repository,
        };

        await _config.AddOrUpdateConnectionAsync(connection, pat, ct).ConfigureAwait(false);

        switch (section)
        {
            case Section.PullRequests:
                await _config.BindPullRequestsAsync(connection.Id, ct).ConfigureAwait(false);
                break;
            case Section.WorkItems:
                await _config.BindWorkItemsAsync(connection.Id, ct).ConfigureAwait(false);
                break;
            case Section.Pipelines:
                await _config.SetPipelineAutoDiscoverAsync(connection.Id, autoDiscoverAll: true, ct).ConfigureAwait(false);
                break;
        }

        return connection.Id;
    }

    public Task RemoveConnectionAsync(string connectionId, CancellationToken ct = default) =>
        _config.RemoveConnectionAsync(connectionId, ct);
}
