using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>
/// The forgetop terminal application: a tabbed shell (Pull Requests / Work Items
/// / Pipelines) over the bound providers, modelled on azdo.
/// </summary>
public sealed class ForgetopApp
{
    private readonly SectionService _sections;
    private readonly IConfigService _config;
    private readonly ThemeManager _theme;

    private Window _window = null!;
    private SectionView _prView = null!;
    private SectionView _workItemView = null!;
    private SectionView _pipelineView = null!;

    public ForgetopApp(SectionService sections, IConfigService config)
    {
        _sections = sections ?? throw new ArgumentNullException(nameof(sections));
        _config = config ?? throw new ArgumentNullException(nameof(config));
        _theme = new ThemeManager(config.Current.Ui.Theme);
    }

    public async Task RunAsync(CancellationToken ct = default)
    {
        Application.Init();
        try
        {
            Build();
            await ReloadAsync(ct).ConfigureAwait(false);
            ApplyTheme();
            Application.Run();
        }
        finally
        {
            Application.Shutdown();
        }
    }

    private void Build()
    {
        _prView = new SectionView("Pull Requests");
        _workItemView = new SectionView("Work Items");
        _pipelineView = new SectionView("Pipelines");

        var tabs = new TabView { X = 0, Y = 0, Width = Dim.Fill(), Height = Dim.Fill() };
        tabs.AddTab(new TabView.Tab("Pull Requests", _prView), andSelect: true);
        tabs.AddTab(new TabView.Tab("Work Items", _workItemView), andSelect: false);
        tabs.AddTab(new TabView.Tab("Pipelines", _pipelineView), andSelect: false);

        _window = new Window("forgetop — htop for your software forges")
        {
            X = 0,
            Y = 0,
            Width = Dim.Fill(),
            Height = Dim.Fill(1),
        };
        _window.Add(tabs);

        var statusBar = new StatusBar(
        [
            new StatusItem(Key.CtrlMask | Key.Q, "~^Q~ Quit", () => Application.RequestStop()),
            new StatusItem(Key.F1, "~F1~ Help", ShowHelp),
            new StatusItem(Key.F2, $"~F2~ Theme ({_theme.Current})", CycleTheme),
            new StatusItem(Key.F5, "~F5~ Refresh", () => _ = ReloadAsync()),
            new StatusItem(Key.Null, "Tab/←→ switch section", null),
        ]);

        Application.Top.Add(_window, statusBar);
    }

    private async Task ReloadAsync(CancellationToken ct = default)
    {
        _prView.SetData(await LoadPullRequestsAsync(ct).ConfigureAwait(false));
        _workItemView.SetData(await LoadWorkItemsAsync(ct).ConfigureAwait(false));
        _pipelineView.SetData(await LoadPipelinesAsync(ct).ConfigureAwait(false));
    }

    private async Task<SectionData> LoadPullRequestsAsync(CancellationToken ct)
    {
        var source = await _sections.GetPullRequestSourceAsync(ct).ConfigureAwait(false);
        if (source is null)
        {
            return SectionData.Unbound("Pull Requests");
        }

        var prs = await source.ListAsync(new PullRequestQuery(), ct).ConfigureAwait(false);
        return new SectionData(LabelFor(_config.Current.PullRequests?.ConnectionId), prs.Select(RowFormatter.PullRequest).ToList());
    }

    private async Task<SectionData> LoadWorkItemsAsync(CancellationToken ct)
    {
        var source = await _sections.GetWorkItemSourceAsync(ct).ConfigureAwait(false);
        if (source is null)
        {
            return SectionData.Unbound("Work Items");
        }

        var items = await source.ListAsync(new WorkItemQuery(), ct).ConfigureAwait(false);
        return new SectionData(LabelFor(_config.Current.WorkItems?.ConnectionId), items.Select(RowFormatter.WorkItem).ToList());
    }

    private async Task<SectionData> LoadPipelinesAsync(CancellationToken ct)
    {
        var feeds = await _sections.GetPipelineFeedsAsync(ct).ConfigureAwait(false);
        if (feeds.Count == 0)
        {
            return SectionData.Unbound("Pipelines");
        }

        var rows = new List<SectionRow>();
        foreach (var feed in feeds)
        {
            var runs = await feed.Source.ListRunsAsync(new PipelineRunQuery { Limit = 25 }, ct).ConfigureAwait(false);
            rows.AddRange(runs.Select(r => RowFormatter.PipelineRun(feed.Connection.DisplayName, r)));
        }

        var label = string.Join(" + ", feeds.Select(f => f.Connection.DisplayName));
        return new SectionData(label, rows);
    }

    private string LabelFor(string? connectionId)
    {
        var connection = connectionId is null ? null : _config.Current.FindConnection(connectionId);
        return connection is null ? "unbound" : $"{connection.DisplayName} ({connection.ProviderType})";
    }

    private void ApplyTheme()
    {
        _window.ColorScheme = _theme.Scheme();
        Application.Refresh();
    }

    private void CycleTheme()
    {
        var name = _theme.Next();
        _ = _config.SetThemeAsync(name);
        ApplyTheme();
    }

    private static void ShowHelp() => MessageBox.Query(
        "forgetop — keys",
        "\nTab / ← →   switch section\n↑ ↓          move in list\nF5           refresh\nF2           cycle theme\nF1           this help\n^Q           quit\n",
        "OK");
}
