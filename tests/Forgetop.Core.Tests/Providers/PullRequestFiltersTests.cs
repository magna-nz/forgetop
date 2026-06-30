using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Core.Tests.Providers;

public class PullRequestFiltersTests
{
    private static readonly User Me = new() { Id = "u1", DisplayName = "Alice", Handle = "alice" };
    private static readonly User Other = new() { Id = "u2", DisplayName = "Bob", Handle = "bob" };

    private static IReadOnlyList<PullRequest> Sample() =>
    [
        new PullRequest { Id = "1", Title = "mine", Author = Me },
        new PullRequest { Id = "2", Title = "theirs, I review", Author = Other, Reviewers = [new Reviewer { User = Me }] },
        new PullRequest { Id = "3", Title = "theirs", Author = Other },
    ];

    [Fact]
    public void All_returns_everything()
    {
        Assert.Equal(3, PullRequestFilters.Apply(Sample(), PullRequestFilter.All, "alice").Count);
    }

    [Fact]
    public void Null_user_returns_everything()
    {
        Assert.Equal(3, PullRequestFilters.Apply(Sample(), PullRequestFilter.Mine, me: null).Count);
    }

    [Fact]
    public void Mine_matches_author_by_handle()
    {
        var result = PullRequestFilters.Apply(Sample(), PullRequestFilter.Mine, "alice");
        Assert.Equal("1", Assert.Single(result).Id);
    }

    [Fact]
    public void ReviewRequested_matches_reviewer()
    {
        var result = PullRequestFilters.Apply(Sample(), PullRequestFilter.ReviewRequested, "alice");
        Assert.Equal("2", Assert.Single(result).Id);
    }
}
