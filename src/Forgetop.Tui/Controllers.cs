using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;

namespace Forgetop.Tui;

/// <summary>Shared helper to label a section by its bound connection.</summary>
internal static class Labels
{
    public static string For(IConfigService config, string? connectionId)
    {
        var connection = connectionId is null ? null : config.Current.FindConnection(connectionId);
        return connection is null ? "unbound" : $"{connection.DisplayName} ({connection.ProviderType})";
    }
}

/// <summary>Backs the Pull Requests screen: filtered load + vote/merge/comment on the selected PR.</summary>
public sealed class PullRequestController(SectionService sections, IConfigService config)
{
    private List<PullRequest> _items = [];

    public PullRequestFilter Filter { get; private set; } = PullRequestFilter.All;

    public string CycleFilter()
    {
        Filter = Filter switch
        {
            PullRequestFilter.All => PullRequestFilter.Mine,
            PullRequestFilter.Mine => PullRequestFilter.ReviewRequested,
            _ => PullRequestFilter.All,
        };
        return Filter.ToString();
    }

    public async Task<SectionData> LoadAsync(CancellationToken ct = default)
    {
        var source = await sections.GetPullRequestSourceAsync(ct).ConfigureAwait(false);
        if (source is null)
        {
            _items = [];
            return SectionData.Unbound("Pull Requests");
        }

        _items = (await source.ListAsync(new PullRequestQuery { Filter = Filter }, ct).ConfigureAwait(false)).ToList();
        var label = $"{Labels.For(config, config.Current.PullRequests?.ConnectionId)} · {Filter}";
        return new SectionData(label, _items.Select(RowFormatter.PullRequest).ToList());
    }

    public PullRequest? At(int index) => index >= 0 && index < _items.Count ? _items[index] : null;

    public async Task<bool> VoteAsync(int index, ReviewVote vote, CancellationToken ct = default)
    {
        var pr = At(index);
        var source = await sections.GetPullRequestSourceAsync(ct).ConfigureAwait(false);
        if (pr is null || source is null)
        {
            return false;
        }

        await source.VoteAsync(pr.Id, vote, ct).ConfigureAwait(false);
        return true;
    }

    public async Task<bool> MergeAsync(int index, MergeOptions options, CancellationToken ct = default)
    {
        var pr = At(index);
        var source = await sections.GetPullRequestSourceAsync(ct).ConfigureAwait(false);
        if (pr is null || source is null)
        {
            return false;
        }

        await source.MergeAsync(pr.Id, options, ct).ConfigureAwait(false);
        return true;
    }

    public async Task<bool> CommentAsync(int index, string body, CancellationToken ct = default)
    {
        var pr = At(index);
        var source = await sections.GetPullRequestSourceAsync(ct).ConfigureAwait(false);
        if (pr is null || source is null)
        {
            return false;
        }

        await source.AddCommentAsync(pr.Id, body, ct).ConfigureAwait(false);
        return true;
    }

    public async Task<string> GetDiffTextAsync(int index, CancellationToken ct = default)
    {
        var pr = At(index);
        var source = await sections.GetPullRequestSourceAsync(ct).ConfigureAwait(false);
        if (pr is null || source is null)
        {
            return string.Empty;
        }

        return DetailFormatter.Diff(await source.GetChangesAsync(pr.Id, ct).ConfigureAwait(false));
    }

    public async Task<string> GetThreadsTextAsync(int index, CancellationToken ct = default)
    {
        var pr = At(index);
        var source = await sections.GetPullRequestSourceAsync(ct).ConfigureAwait(false);
        if (pr is null || source is null)
        {
            return string.Empty;
        }

        return DetailFormatter.Threads(await source.GetThreadsAsync(pr.Id, ct).ConfigureAwait(false));
    }
}

/// <summary>Backs the Work Items screen: load + state change / comment on the selected item.</summary>
public sealed class WorkItemController(SectionService sections, IConfigService config)
{
    private List<WorkItem> _items = [];

    public bool MineOnly { get; private set; }

    public bool ToggleMine()
    {
        MineOnly = !MineOnly;
        return MineOnly;
    }

    public async Task<SectionData> LoadAsync(CancellationToken ct = default)
    {
        var source = await sections.GetWorkItemSourceAsync(ct).ConfigureAwait(false);
        if (source is null)
        {
            _items = [];
            return SectionData.Unbound("Work Items");
        }

        _items = (await source.ListAsync(new WorkItemQuery { MineOnly = MineOnly }, ct).ConfigureAwait(false)).ToList();
        var label = $"{Labels.For(config, config.Current.WorkItems?.ConnectionId)}{(MineOnly ? " · Mine" : "")}";
        return new SectionData(label, _items.Select(RowFormatter.WorkItem).ToList());
    }

    public WorkItem? At(int index) => index >= 0 && index < _items.Count ? _items[index] : null;

    public async Task<bool> SetStateAsync(int index, string state, CancellationToken ct = default)
    {
        var item = At(index);
        var source = await sections.GetWorkItemSourceAsync(ct).ConfigureAwait(false);
        if (item is null || source is null)
        {
            return false;
        }

        await source.SetStateAsync(item.Id, state, ct).ConfigureAwait(false);
        return true;
    }

    public async Task<bool> CommentAsync(int index, string body, CancellationToken ct = default)
    {
        var item = At(index);
        var source = await sections.GetWorkItemSourceAsync(ct).ConfigureAwait(false);
        if (item is null || source is null)
        {
            return false;
        }

        await source.AddCommentAsync(item.Id, body, ct).ConfigureAwait(false);
        return true;
    }
}

/// <summary>A discoverable pipeline on a specific connection (for the subscribe picker).</summary>
public sealed record DiscoveredPipeline(string ConnectionId, string ConnectionLabel, PipelineDefinition Definition);

/// <summary>
/// Backs the Pipelines screen: aggregates runs across all bound connections, drills
/// into a run, fetches logs, triggers, and discovers/subscribes pipelines at runtime.
/// </summary>
public sealed class PipelineController(SectionService sections, IConfigService config)
{
    private readonly List<(PipelineFeed Feed, PipelineRun Run)> _items = [];

    public async Task<SectionData> LoadAsync(CancellationToken ct = default)
    {
        _items.Clear();
        var feeds = await sections.GetPipelineFeedsAsync(ct).ConfigureAwait(false);
        if (feeds.Count == 0)
        {
            return SectionData.Unbound("Pipelines");
        }

        var rows = new List<SectionRow>();
        foreach (var feed in feeds)
        {
            var runs = await feed.Source.ListRunsAsync(new PipelineRunQuery { Limit = 25 }, ct).ConfigureAwait(false);
            foreach (var run in runs)
            {
                _items.Add((feed, run));
                rows.Add(RowFormatter.PipelineRun(feed.Connection.DisplayName, run));
            }
        }

        var label = string.Join(" + ", feeds.Select(f => f.Connection.DisplayName));
        return new SectionData(label, rows);
    }

    public async Task<string> GetRunDetailAsync(int index, CancellationToken ct = default)
    {
        if (index < 0 || index >= _items.Count)
        {
            return string.Empty;
        }

        var (feed, run) = _items[index];
        var full = await feed.Source.GetRunAsync(run.Id, ct).ConfigureAwait(false);
        var logs = await feed.Source.GetLogsAsync(run.Id, null, ct).ConfigureAwait(false);
        var stages = full.Stages.Count == 0
            ? "(no stages)"
            : string.Join('\n', full.Stages.Select(s => $"  {s.Name}: {s.Status} ({string.Join(", ", s.Jobs.Select(j => $"{j.Name}:{j.Status}"))})"));
        return $"{feed.Connection.DisplayName} · {full.Name} #{full.Number}\nStatus: {full.Status}\n\nStages:\n{stages}\n\nLogs:\n{logs}";
    }

    public async Task<bool> TriggerAsync(int index, CancellationToken ct = default)
    {
        if (index < 0 || index >= _items.Count)
        {
            return false;
        }

        var (feed, run) = _items[index];
        await feed.Source.TriggerAsync(run.DefinitionId, run.Branch, ct).ConfigureAwait(false);
        return true;
    }

    public async Task<IReadOnlyList<DiscoveredPipeline>> DiscoverAsync(CancellationToken ct = default)
    {
        var feeds = await sections.GetPipelineFeedsAsync(ct).ConfigureAwait(false);
        var result = new List<DiscoveredPipeline>();
        foreach (var feed in feeds)
        {
            foreach (var def in await feed.Source.DiscoverAsync(ct).ConfigureAwait(false))
            {
                result.Add(new DiscoveredPipeline(feed.Connection.ConnectionId, feed.Connection.DisplayName, def));
            }
        }

        return result;
    }

    public Task SubscribeAsync(DiscoveredPipeline pipeline, CancellationToken ct = default) =>
        config.SubscribePipelineAsync(pipeline.ConnectionId, pipeline.Definition.Id, ct);

    public async Task<bool> UnsubscribeSelectedAsync(int index, CancellationToken ct = default)
    {
        if (index < 0 || index >= _items.Count)
        {
            return false;
        }

        var (feed, run) = _items[index];
        await config.UnsubscribePipelineAsync(feed.Connection.ConnectionId, run.DefinitionId, ct).ConfigureAwait(false);
        return true;
    }
}
