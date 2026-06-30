using Forgetop.Core.Domain;

namespace Forgetop.Core.Providers;

/// <summary>
/// Applies <see cref="PullRequestFilter"/> client-side given the authenticated
/// user's identity. Providers fetch "all" then narrow with this so the rule lives
/// in one tested place.
/// </summary>
public static class PullRequestFilters
{
    public static IReadOnlyList<PullRequest> Apply(IReadOnlyList<PullRequest> pullRequests, PullRequestFilter filter, string? me)
    {
        if (filter == PullRequestFilter.All || string.IsNullOrEmpty(me))
        {
            return pullRequests;
        }

        return filter switch
        {
            PullRequestFilter.Mine => pullRequests.Where(p => IsUser(p.Author, me)).ToList(),
            PullRequestFilter.ReviewRequested => pullRequests.Where(p => p.Reviewers.Any(r => IsUser(r.User, me))).ToList(),
            _ => pullRequests,
        };
    }

    private static bool IsUser(User user, string me) =>
        string.Equals(user.Handle, me, StringComparison.OrdinalIgnoreCase) ||
        string.Equals(user.DisplayName, me, StringComparison.OrdinalIgnoreCase) ||
        string.Equals(user.Id, me, StringComparison.OrdinalIgnoreCase);
}
