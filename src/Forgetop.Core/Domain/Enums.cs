namespace Forgetop.Core.Domain;

/// <summary>The three top-level sections of the app, each independently bound.</summary>
public enum Section
{
    PullRequests,
    WorkItems,
    Pipelines,
}

/// <summary>The platforms forgetop can talk to.</summary>
public enum ProviderType
{
    Demo,
    GitHub,
    AzureDevOps,
    Linear,
    GitLab,
    Bitbucket,
}

/// <summary>Provider-neutral pull-request / merge-request state.</summary>
public enum PullRequestStatus
{
    Open,
    Draft,
    Merged,
    Closed,
}

/// <summary>
/// Provider-neutral reviewer vote. Azure DevOps numeric votes and GitHub
/// review states both map onto this; see <see cref="Providers.VoteStyle"/>.
/// </summary>
public enum ReviewVote
{
    Rejected,
    WaitingForAuthor,
    NoVote,
    ApprovedWithSuggestions,
    Approved,
}

/// <summary>
/// Provider-neutral work-item lifecycle bucket. Concrete provider states
/// (e.g. "In Progress", "Done", Linear workflow states) map onto one of these
/// while the original label is preserved on <see cref="WorkItem.State"/>.
/// </summary>
public enum WorkItemStateCategory
{
    Triage,
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
}

/// <summary>Provider-neutral CI run status.</summary>
public enum PipelineRunStatus
{
    Queued,
    Running,
    Succeeded,
    PartiallySucceeded,
    Failed,
    Canceled,
}
