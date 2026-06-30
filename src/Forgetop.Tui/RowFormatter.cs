using Forgetop.Core.Domain;

namespace Forgetop.Tui;

/// <summary>Pure projections from domain entities to display rows.</summary>
public static class RowFormatter
{
    public static SectionRow PullRequest(PullRequest pr)
    {
        var display = $"#{pr.Number?.ToString() ?? pr.Id}  {pr.Title}  [{pr.Status}] · {pr.Author.DisplayName}";
        var votes = pr.Reviewers.Count == 0
            ? "none"
            : string.Join(", ", pr.Reviewers.Select(r => $"{r.User.DisplayName}: {r.Vote}"));
        var detail = string.Join('\n',
            pr.Title,
            $"Status:    {pr.Status}{(pr.IsDraft ? " (draft)" : "")}",
            $"Author:    {pr.Author.DisplayName}",
            $"Branch:    {pr.SourceRef} → {pr.TargetRef}",
            $"Reviewers: {votes}",
            "",
            pr.Description ?? "(no description)");
        return new SectionRow(display, detail);
    }

    public static SectionRow WorkItem(WorkItem item)
    {
        var id = item.Identifier ?? item.Id;
        var display = $"{id}  {item.Title}  [{item.State}]";
        var detail = string.Join('\n',
            item.Title,
            $"State:    {item.State} ({item.StateCategory})",
            $"Type:     {item.Type ?? "-"}",
            $"Assignee: {item.Assignee?.DisplayName ?? "unassigned"}",
            "",
            item.Description ?? "(no description)");
        return new SectionRow(display, detail);
    }

    public static SectionRow PipelineRun(string providerLabel, PipelineRun run)
    {
        var number = run.Number?.ToString() ?? run.Id;
        var display = $"{providerLabel} · {run.Name ?? "run"} #{number}  [{run.Status}]  {run.Branch}";
        var stages = run.Stages.Count == 0
            ? "(no stage detail)"
            : string.Join('\n', run.Stages.Select(s => $"  {s.Name}: {s.Status}"));
        var detail = string.Join('\n',
            $"{run.Name} #{number}",
            $"Status:  {run.Status}",
            $"Branch:  {run.Branch}  ({run.CommitSha})",
            $"By:      {run.TriggeredBy?.DisplayName ?? "-"}",
            "Stages:",
            stages);
        return new SectionRow(display, detail);
    }
}
