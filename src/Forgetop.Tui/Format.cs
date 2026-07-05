using Forgetop.Core.Domain;

namespace Forgetop.Tui;

/// <summary>Small display formatters for dates and durations.</summary>
public static class Fmt
{
    public static string Date(DateTimeOffset dt) => dt == default ? "–" : dt.LocalDateTime.ToString("yyyy-MM-dd");

    public static string DateTime(DateTimeOffset? dt) =>
        dt is null || dt.Value == default ? "–" : dt.Value.LocalDateTime.ToString("yyyy-MM-dd HH:mm");

    public static string Duration(PipelineRun run)
    {
        if (run.StartedAt is not { } start)
        {
            return "–";
        }

        var end = run.FinishedAt
            ?? (run.Status is PipelineRunStatus.Running or PipelineRunStatus.Queued ? DateTimeOffset.UtcNow : null);
        if (end is null)
        {
            return "–";
        }

        var d = end.Value - start;
        if (d < TimeSpan.Zero)
        {
            return "–";
        }

        return d.TotalHours >= 1 ? $"{(int)d.TotalHours}h{d.Minutes}m"
            : d.TotalMinutes >= 1 ? $"{(int)d.TotalMinutes}m{d.Seconds}s"
            : $"{d.Seconds}s";
    }
}
