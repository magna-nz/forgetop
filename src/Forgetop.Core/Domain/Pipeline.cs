namespace Forgetop.Core.Domain;

/// <summary>
/// A discoverable / subscribable pipeline definition (ADO pipeline, GitHub
/// Actions workflow, GitLab CI config).
/// </summary>
public sealed record PipelineDefinition
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public string? Path { get; init; }
    public string? Url { get; init; }
}

/// <summary>A step / task within a job.</summary>
public sealed record PipelineStep
{
    public required string Name { get; init; }
    public PipelineRunStatus Status { get; init; }
}

/// <summary>A single job within a pipeline run.</summary>
public sealed record PipelineJob
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public PipelineRunStatus Status { get; init; }
    public DateTimeOffset? StartedAt { get; init; }
    public DateTimeOffset? FinishedAt { get; init; }
    public IReadOnlyList<PipelineStep> Steps { get; init; } = [];
}

/// <summary>A stage grouping one or more jobs within a pipeline run.</summary>
public sealed record PipelineStage
{
    public required string Name { get; init; }
    public PipelineRunStatus Status { get; init; }
    public IReadOnlyList<PipelineJob> Jobs { get; init; } = [];
}

/// <summary>Provider-neutral CI run.</summary>
public sealed record PipelineRun
{
    public required string Id { get; init; }
    public required string DefinitionId { get; init; }
    public int? Number { get; init; }
    public string? Name { get; init; }
    public PipelineRunStatus Status { get; init; }

    public User? TriggeredBy { get; init; }
    public string? Branch { get; init; }
    public string? CommitSha { get; init; }

    public DateTimeOffset? StartedAt { get; init; }
    public DateTimeOffset? FinishedAt { get; init; }
    public string? Url { get; init; }

    public IReadOnlyList<PipelineStage> Stages { get; init; } = [];
}
