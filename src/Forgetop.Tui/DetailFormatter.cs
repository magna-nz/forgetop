using System.Text;
using Forgetop.Core.Domain;

namespace Forgetop.Tui;

/// <summary>Pure formatters for the PR diff and comment-thread detail views.</summary>
public static class DetailFormatter
{
    public static string Diff(IReadOnlyList<FileChange> changes)
    {
        if (changes.Count == 0)
        {
            return "(no file changes)";
        }

        var sb = new StringBuilder();
        sb.AppendLine($"Changed files ({changes.Count}):");
        foreach (var change in changes)
        {
            sb.AppendLine($"  {Symbol(change.Kind)} {change.Path}  +{change.Additions} -{change.Deletions}");
        }

        foreach (var change in changes.Where(c => !string.IsNullOrEmpty(c.Patch)))
        {
            sb.AppendLine().AppendLine($"── {change.Path} ──").AppendLine(change.Patch);
        }

        return sb.ToString().TrimEnd();
    }

    public static string Threads(IReadOnlyList<CommentThread> threads)
    {
        if (threads.Count == 0)
        {
            return "(no comments)";
        }

        var sb = new StringBuilder();
        foreach (var thread in threads)
        {
            if (thread.FilePath is not null)
            {
                sb.AppendLine($"── {thread.FilePath}{(thread.Line is { } line ? $":{line}" : "")} ──");
            }

            foreach (var comment in thread.Comments)
            {
                sb.AppendLine($"{comment.Author.DisplayName} ({comment.CreatedAt:yyyy-MM-dd HH:mm}):");
                sb.AppendLine($"  {comment.Body}");
            }

            sb.AppendLine();
        }

        return sb.ToString().TrimEnd();
    }

    /// <summary>The PR overview shown when a PR row is expanded (Enter).</summary>
    public static string PrOverview(PullRequest pr)
    {
        var reviewers = pr.Reviewers.Count == 0
            ? "none"
            : string.Join(", ", pr.Reviewers.Select(r => $"{r.User.DisplayName} ({r.Vote})"));
        var labels = pr.Labels.Count == 0 ? "none" : string.Join(", ", pr.Labels);

        var merge = pr.IsDraft ? "Draft — cannot be merged" : pr.Mergeable switch
        {
            MergeableState.Mergeable => "Yes",
            MergeableState.Conflicting => "No — conflicts",
            MergeableState.Blocked => "Blocked",
            _ => "Unknown",
        };

        var checks = pr.CheckSummary is { } s
            ? $"{pr.Checks} — {s.Successful} ok, {s.InProgress} running, {s.Failed} failed, {s.Neutral} neutral"
            : pr.Checks.ToString();

        return string.Join('\n',
            $"#{pr.Number?.ToString() ?? pr.Id}  {pr.Title}",
            $"{pr.Author.DisplayName}  ·  {pr.SourceRef} → {pr.TargetRef}",
            "",
            $"Mergeable: {merge}",
            $"Checks:    {checks}",
            $"Reviewers: {reviewers}",
            $"Labels:    {labels}",
            $"Changes:   {pr.ChangedFiles} files  (+{pr.Additions} -{pr.Deletions})",
            "",
            "Summary:",
            pr.Description ?? "(no description)");
    }

    /// <summary>The work-item detail shown when a row is expanded (Enter).</summary>
    public static string WorkItemDetail(WorkItem item) => string.Join('\n',
        $"{item.Identifier ?? item.Id}  {item.Title}",
        $"State:    {item.State} ({item.StateCategory})",
        $"Type:     {item.Type ?? "-"}",
        $"Assignee: {item.Assignee?.DisplayName ?? "unassigned"}",
        "",
        "Description:",
        item.Description ?? "(none)");

    private static char Symbol(FileChangeKind kind) => kind switch
    {
        FileChangeKind.Added => 'A',
        FileChangeKind.Deleted => 'D',
        FileChangeKind.Renamed => 'R',
        _ => 'M',
    };
}
