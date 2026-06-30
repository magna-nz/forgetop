using Forgetop.Core.Domain;

namespace Forgetop.Core.Providers;

/// <summary>How a provider expresses reviewer approval.</summary>
public enum VoteStyle
{
    /// <summary>Approve / request-changes / comment (GitHub, GitLab).</summary>
    BinaryApprove,

    /// <summary>Numeric votes, e.g. -10..+10 (Azure DevOps).</summary>
    NumericVotes,
}

/// <summary>
/// Provider-specific wording so the UI can relabel sections (e.g. "Merge
/// Requests" vs "Pull Requests", "Issues" vs "Work Items").
/// </summary>
public sealed record Terminology
{
    public string PullRequests { get; init; } = "Pull Requests";
    public string WorkItems { get; init; } = "Work Items";
    public string Pipelines { get; init; } = "Pipelines";
}

/// <summary>
/// Declares what a connection supports so the UI can hide or relabel features
/// that don't exist on a given platform.
/// </summary>
public sealed record ProviderCapabilities
{
    public bool SupportsPullRequests { get; init; }
    public bool SupportsWorkItems { get; init; }
    public bool SupportsPipelines { get; init; }

    // Pull-request features
    public VoteStyle VoteStyle { get; init; } = VoteStyle.BinaryApprove;
    public bool SupportsMerge { get; init; }
    public bool SupportsInlineComments { get; init; }

    // Pipeline features
    public bool SupportsPipelineTrigger { get; init; }
    public bool SupportsPipelineDiscovery { get; init; }

    public Terminology Terminology { get; init; } = new();

    /// <summary>True if the given section can be served by this connection.</summary>
    public bool Supports(Section section) => section switch
    {
        Section.PullRequests => SupportsPullRequests,
        Section.WorkItems => SupportsWorkItems,
        Section.Pipelines => SupportsPipelines,
        _ => false,
    };
}
