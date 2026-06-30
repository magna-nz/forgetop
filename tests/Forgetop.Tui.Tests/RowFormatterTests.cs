using Forgetop.Core.Domain;
using Forgetop.Tui;

namespace Forgetop.Tui.Tests;

public class RowFormatterTests
{
    [Fact]
    public void PullRequest_summarises_number_title_status()
    {
        var pr = new PullRequest
        {
            Id = "42", Number = 42, Title = "Add retry", Status = PullRequestStatus.Open,
            Author = new User { Id = "u", DisplayName = "Alice" },
            SourceRef = "feature/x", TargetRef = "main",
            Reviewers = [new Reviewer { User = new User { Id = "r", DisplayName = "Bob" }, Vote = ReviewVote.Approved }],
        };

        var row = RowFormatter.PullRequest(pr);
        Assert.Contains("#42", row.Display);
        Assert.Contains("Add retry", row.Display);
        Assert.Contains("Open", row.Display);
        Assert.Contains("feature/x → main", row.Detail);
        Assert.Contains("Bob: Approved", row.Detail);
    }

    [Fact]
    public void WorkItem_uses_identifier_and_state()
    {
        var wi = new WorkItem
        {
            Id = "w1", Identifier = "FOR-12", Title = "Design", State = "In Progress",
            StateCategory = WorkItemStateCategory.Started, Type = "Story",
            Assignee = new User { Id = "u", DisplayName = "Carol" },
        };

        var row = RowFormatter.WorkItem(wi);
        Assert.Contains("FOR-12", row.Display);
        Assert.Contains("In Progress", row.Display);
        Assert.Contains("Carol", row.Detail);
    }

    [Fact]
    public void PipelineRun_prefixes_provider_label()
    {
        var run = new PipelineRun
        {
            Id = "r1", DefinitionId = "ci", Number = 7, Name = "CI", Status = PipelineRunStatus.Failed, Branch = "main",
            Stages = [new PipelineStage { Name = "build", Status = PipelineRunStatus.Succeeded }],
        };

        var row = RowFormatter.PipelineRun("Demo Org", run);
        Assert.StartsWith("Demo Org", row.Display);
        Assert.Contains("#7", row.Display);
        Assert.Contains("Failed", row.Display);
        Assert.Contains("build: Succeeded", row.Detail);
    }
}

public class ThemeManagerTests
{
    [Fact]
    public void Defaults_to_dark_for_unknown_theme()
    {
        Assert.Equal("dark", new ThemeManager(null).Current);
        Assert.Equal("dark", new ThemeManager("nonsense").Current);
        Assert.Equal("light", new ThemeManager("light").Current);
    }

    [Fact]
    public void Next_cycles_through_all_themes_and_wraps()
    {
        var theme = new ThemeManager("dark");
        var seen = new List<string> { theme.Current };
        for (var i = 0; i < theme.Themes.Count; i++)
        {
            seen.Add(theme.Next());
        }

        // After N advances from the first, we should have wrapped back to the start.
        Assert.Equal("dark", theme.Current);
        Assert.Equal(theme.Themes.OrderBy(x => x), seen.Distinct().OrderBy(x => x));
    }
}
