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

    private static char Symbol(FileChangeKind kind) => kind switch
    {
        FileChangeKind.Added => 'A',
        FileChangeKind.Deleted => 'D',
        FileChangeKind.Renamed => 'R',
        _ => 'M',
    };
}
