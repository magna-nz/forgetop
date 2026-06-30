using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Providers;
using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>
/// The forgetop terminal application: a tabbed shell (Pull Requests / Work Items
/// / Pipelines) over the bound providers, modelled on azdo.
/// </summary>
public sealed class ForgetopApp
{
    private static readonly TimeSpan PipelineRefreshInterval = TimeSpan.FromSeconds(5);

    private readonly IConfigService _config;
    private readonly SetupService _setup;
    private readonly IProviderRegistry _registry;
    private readonly ThemeManager _theme;

    private readonly PullRequestController _prController;
    private readonly WorkItemController _workItemController;
    private readonly PipelineController _pipelineController;

    private Window _window = null!;
    private PullRequestsView _prView = null!;
    private WorkItemsView _workItemView = null!;
    private PipelinesView _pipelineView = null!;

    public ForgetopApp(SectionService sections, IConfigService config, SetupService setup, IProviderRegistry registry)
    {
        ArgumentNullException.ThrowIfNull(sections);
        _config = config ?? throw new ArgumentNullException(nameof(config));
        _setup = setup ?? throw new ArgumentNullException(nameof(setup));
        _registry = registry ?? throw new ArgumentNullException(nameof(registry));
        _theme = new ThemeManager(config.Current.Ui.Theme);

        _prController = new PullRequestController(sections, config);
        _workItemController = new WorkItemController(sections, config);
        _pipelineController = new PipelineController(sections, config);
    }

    public async Task RunAsync(CancellationToken ct = default)
    {
        Application.Init();
        try
        {
            Build();
            if (_config.Current.Connections.Count == 0)
            {
                await SetupWizard.FirstRunAsync(_setup, _registry).ConfigureAwait(true);
            }

            await _prView.ReloadAsync(ct).ConfigureAwait(true);
            await _workItemView.ReloadAsync(ct).ConfigureAwait(true);
            await _pipelineView.ReloadAsync(ct).ConfigureAwait(true);
            ApplyTheme();
            StartPipelineAutoRefresh();
            Application.Run();
        }
        finally
        {
            Application.Shutdown();
        }
    }

    private void Build()
    {
        _prView = new PullRequestsView(_prController);
        _workItemView = new WorkItemsView(_workItemController);
        _pipelineView = new PipelinesView(_pipelineController);

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
            new StatusItem(Key.F3, "~F3~ Config", OpenConfig),
            new StatusItem(Key.F5, "~F5~ Refresh", RefreshAll),
            new StatusItem(Key.Null, "actions: see F1", null),
        ]);

        Application.Top.Add(_window, statusBar);
    }

    private void StartPipelineAutoRefresh() =>
        Application.MainLoop.AddTimeout(PipelineRefreshInterval, loop =>
        {
            _ = Task.Run(async () =>
            {
                try
                {
                    var data = await _pipelineController.LoadAsync().ConfigureAwait(false);
                    Application.MainLoop.Invoke(() => _pipelineView.Apply(data));
                }
                catch
                {
                    // Ignore transient refresh failures; the next tick retries.
                }
            });
            return true;
        });

    private void OpenConfig()
    {
        try
        {
            SetupWizard.ShowConfigAsync(_setup, _registry, _config).GetAwaiter().GetResult();
            RefreshAll();
        }
        catch (Exception ex)
        {
            Dialogs.Error("forgetop", ex.Message);
        }
    }

    private void RefreshAll()
    {
        try
        {
            _prView.ReloadAsync().GetAwaiter().GetResult();
            _workItemView.ReloadAsync().GetAwaiter().GetResult();
            _pipelineView.ReloadAsync().GetAwaiter().GetResult();
        }
        catch (Exception ex)
        {
            Dialogs.Error("forgetop", ex.Message);
        }
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
        "\nGlobal\n" +
        "  Tab / ← →   switch section      ↑ ↓   move in list\n" +
        "  F5 refresh   F3 config   F2 theme   F1 help   ^Q quit\n\n" +
        "Pull Requests\n" +
        "  f  cycle filter (All/Mine/ReviewRequested)\n" +
        "  a  approve   m  merge   c  comment   d  diff/files   v  comments\n\n" +
        "Work Items\n" +
        "  f  toggle mine   s  set state   c  comment\n\n" +
        "Pipelines\n" +
        "  ↵  drill-in (stages + logs)   t  trigger/re-run   d  discover & subscribe   u  unsubscribe\n",
        "OK");
}
