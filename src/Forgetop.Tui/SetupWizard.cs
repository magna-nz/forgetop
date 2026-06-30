using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Tui;

/// <summary>Interactive flows (built on <see cref="Dialogs"/>) for first-run setup and runtime config.</summary>
public static class SetupWizard
{
    private static readonly Section[] Sections = [Section.PullRequests, Section.WorkItems, Section.Pipelines];

    /// <summary>Configure one section: pick provider, enter scope + PAT, then bind. Returns true if configured.</summary>
    public static async Task<bool> ConfigureSectionAsync(SetupService setup, IProviderRegistry registry, Section section)
    {
        var providers = registry.AvailableProviders
            .Where(p => registry.DescribeCapabilities(p).Supports(section))
            .Select(p => p.ToString())
            .ToList();

        var providerName = Dialogs.Pick($"Provider for {section}", providers);
        if (providerName is null || !Enum.TryParse<ProviderType>(providerName, out var provider))
        {
            return false;
        }

        var displayName = Dialogs.Prompt("Display name", "A friendly name for this connection:", provider.ToString());
        if (displayName is null)
        {
            return false;
        }

        string? organization = null, project = null, repository = null, pat = null;
        if (provider is ProviderType.GitHub or ProviderType.AzureDevOps)
        {
            organization = Dialogs.Prompt("Organization", provider == ProviderType.GitHub ? "GitHub owner/org:" : "Azure DevOps organization:");
        }

        if (provider is ProviderType.AzureDevOps)
        {
            project = Dialogs.Prompt("Project", "Azure DevOps project:");
        }

        if (provider is ProviderType.GitHub or ProviderType.AzureDevOps)
        {
            repository = Dialogs.Prompt("Repository", "Repository name:");
        }

        if (provider is not ProviderType.Demo)
        {
            pat = Dialogs.Prompt("Personal Access Token", "PAT (stored in your OS keychain):");
        }

        await setup.ConfigureSectionAsync(section, provider, displayName, organization, project, repository, pat);
        Dialogs.Info("Setup", $"{section} is now served by {displayName}.");
        return true;
    }

    /// <summary>First-run: walk the user through configuring each unbound section.</summary>
    public static async Task FirstRunAsync(SetupService setup, IProviderRegistry registry)
    {
        Dialogs.Info("Welcome to forgetop", "No connections yet. Let's bind each section to a provider.\nYou can skip any and configure later with F3.");
        foreach (var section in Sections)
        {
            if (Dialogs.Confirm("Setup", $"Configure {section} now?"))
            {
                await ConfigureSectionAsync(setup, registry, section);
            }
        }
    }

    /// <summary>Runtime config screen: add a connection to a section, or remove one.</summary>
    public static async Task ShowConfigAsync(SetupService setup, IProviderRegistry registry, IConfigService config)
    {
        while (true)
        {
            var options = new List<string>
            {
                "Add → Pull Requests",
                "Add → Work Items",
                "Add → Pipelines",
            };
            options.AddRange(config.Current.Connections.Select(c => $"Remove: {c.DisplayName} ({c.ProviderType})"));

            var choice = Dialogs.Pick("Configuration", options);
            if (choice is null)
            {
                return;
            }

            if (choice.StartsWith("Add → ", StringComparison.Ordinal))
            {
                var section = choice["Add → ".Length..] switch
                {
                    "Pull Requests" => Section.PullRequests,
                    "Work Items" => Section.WorkItems,
                    _ => Section.Pipelines,
                };
                await ConfigureSectionAsync(setup, registry, section);
            }
            else if (choice.StartsWith("Remove: ", StringComparison.Ordinal))
            {
                var connection = config.Current.Connections.FirstOrDefault(c => choice.Contains(c.DisplayName));
                if (connection is not null && Dialogs.Confirm("Remove", $"Remove '{connection.DisplayName}' and its bindings?"))
                {
                    await setup.RemoveConnectionAsync(connection.Id);
                }
            }
        }
    }
}
