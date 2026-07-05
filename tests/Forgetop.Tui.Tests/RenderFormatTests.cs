using Forgetop.Core.Domain;
using Forgetop.Tui;

namespace Forgetop.Tui.Tests;

public class FmtTests
{
    [Fact]
    public void Duration_of_finished_run_is_minutes_seconds()
    {
        var start = new DateTimeOffset(2026, 6, 30, 9, 0, 0, TimeSpan.Zero);
        var run = new PipelineRun
        {
            Id = "r", DefinitionId = "d", Status = PipelineRunStatus.Succeeded,
            StartedAt = start, FinishedAt = start.AddMinutes(7).AddSeconds(24),
        };
        Assert.Equal("7m24s", Fmt.Duration(run));
    }

    [Fact]
    public void Duration_unknown_when_no_start()
    {
        var run = new PipelineRun { Id = "r", DefinitionId = "d", Status = PipelineRunStatus.Queued };
        Assert.Equal("–", Fmt.Duration(run));
    }

    [Fact]
    public void Date_formats_iso()
    {
        var dt = new DateTimeOffset(2026, 6, 30, 0, 0, 0, TimeSpan.Zero);
        Assert.Equal("2026-06-30", Fmt.Date(dt));
    }
}

public class PrOverviewTests
{
    [Fact]
    public void Overview_includes_mergeable_checks_reviewers_labels_and_changes()
    {
        var pr = new PullRequest
        {
            Id = "1", Number = 1, Title = "Add retry", Author = new User { Id = "u", DisplayName = "Alice" },
            SourceRef = "feature/x", TargetRef = "main",
            Reviewers = [new Reviewer { User = new User { Id = "b", DisplayName = "Bob" }, Vote = ReviewVote.Approved }],
            Labels = ["banking"],
            Checks = CheckStatus.Passed,
            CheckSummary = new CheckSummary { Successful = 14, Neutral = 1 },
            Mergeable = MergeableState.Mergeable,
            ChangedFiles = 2, Additions = 24, Deletions = 2,
            Description = "Wraps calls in retry.",
        };

        var text = DetailFormatter.PrOverview(pr);
        Assert.Contains("Mergeable: Yes", text);
        Assert.Contains("Passed", text);
        Assert.Contains("Bob (Approved)", text);
        Assert.Contains("banking", text);
        Assert.Contains("2 files  (+24 -2)", text);
    }

    [Fact]
    public void Overview_marks_draft_as_not_mergeable()
    {
        var pr = new PullRequest
        {
            Id = "2", Number = 2, Title = "WIP", Author = new User { Id = "u", DisplayName = "Bob" },
            IsDraft = true, Mergeable = MergeableState.Blocked,
        };
        Assert.Contains("Draft — cannot be merged", DetailFormatter.PrOverview(pr));
    }
}
