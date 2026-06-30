using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Providers;
using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>
/// The forgetop terminal application: a tabbed shell (Pull Requests / Work Items
/// / Pipelines) over the bound providers, modelled on azdo / gh-dash.
/// </summary>
public sealed class ForgetopApp
{
    private static readonly TimeSpan RefreshInterval = TimeSpan.FromSeconds(30);

    private readonly IConfigService _config;
    private readonly SetupService _setup;
    private readonly IProviderRegistry _registry;
    private readonly ConnectionHealthService _health;
    private readonly ThemeManager _theme;

    private readonly PullRequestController _prController;
    private readonly WorkItemController _workItemController;
    private readonly PipelineController _pipelineController;

    private Window _window = null!;
    private TabView _tabs = null!;
    private ConnectionsBar _connectionsBar = null!;
    private PullRequestsView _prView = null!;
    private WorkItemsView _workItemView = null!;
    private PipelinesView _pipelineView = null!;

    public ForgetopApp(SectionService sections, IConfigService config, SetupService setup, IProviderRegistry registry, ConnectionHealthService health)
    {
        ArgumentNullException.ThrowIfNull(sections);
        _config = config ?? throw new ArgumentNullException(nameof(config));
        _setup = setup ?? throw new ArgumentNullException(nameof(setup));
        _registry = registry ?? throw new ArgumentNullException(nameof(registry));
        _health = health ?? throw new ArgumentNullException(nameof(health));
        _theme = new ThemeManager(config.Current.Ui.Theme);

        _prController = new PullRequestController(sections, config);
        _workItemController = new WorkItemController(sections, config);
        _pipelineController = new PipelineController(sections, config);
    }

    private SectionView[] Views => [_prView, _workItemView, _pipelineView];

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

            await RefreshAllAsync(ct).ConfigureAwait(true);
            ApplyTheme();
            StartAutoRefresh();
            RefreshHealth();
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

        _tabs = new TabView { X = 0, Y = 0, Width = Dim.Fill(), Height = Dim.Fill() };
        _tabs.AddTab(new TabView.Tab("   Pull Requests   ", _prView), andSelect: true);
        _tabs.AddTab(new TabView.Tab("   Work Items   ", _workItemView), andSelect: false);
        _tabs.AddTab(new TabView.Tab("   Pipelines   ", _pipelineView), andSelect: false);

        foreach (var view in Views)
        {
            view.TabSwitch += delta => _tabs.SwitchTabBy(delta);
        }

        _window = new Window("forgetop — htop for your software forges")
        {
            X = 0,
            Y = 0,
            Width = Dim.Fill(),
            Height = Dim.Fill(2),
        };
        _window.Add(_tabs);

        _connectionsBar = new ConnectionsBar { X = 0, Y = Pos.Bottom(_window) };

        var statusBar = new StatusBar(
        [
            new StatusItem(Key.CtrlMask | Key.Q, "~^Q~ Quit", () => Application.RequestStop()),
            new StatusItem(Key.F1, "~F1~ Help", ShowHelp),
            new StatusItem(Key.F2, $"~F2~ Theme ({_theme.Current})", CycleTheme),
            new StatusItem(Key.F3, "~F3~ Config", OpenConfig),
            new StatusItem(Key.F5, "~F5~ Refresh", RefreshAll),
            new StatusItem(Key.Null, "←/→ tabs · ↵ details · actions: F1", null),
        ]);

        Application.Top.Add(_window, _connectionsBar, statusBar);
    }

    private async Task RefreshAllAsync(CancellationToken ct)
    {
        foreach (var view in Views)
        {
            await view.RefreshAsync(ct).ConfigureAwait(true);
        }
    }

    private void StartAutoRefresh() =>
        Application.MainLoop.AddTimeout(RefreshInterval, loop =>
        {
            foreach (var view in Views)
            {
                var captured = view;
                _ = Task.Run(async () =>
                {
                    try
                    {
                        await captured.LoadDataAsync().ConfigureAwait(false);
                        Application.MainLoop.Invoke(captured.Render);
                    }
                    catch
                    {
                        // ignore transient refresh failures; the next tick retries
                    }
                });
            }

            RefreshHealth();
            return true;
        });

    private void RefreshHealth() =>
        _ = Task.Run(async () =>
        {
            try
            {
                var health = await _health.CheckAllAsync().ConfigureAwait(false);
                Application.MainLoop.Invoke(() => _connectionsBar.Update(health));
            }
            catch
            {
                // ignore; the next tick retries
            }
        });

    private void RefreshAll()
    {
        try
        {
            RefreshAllAsync(CancellationToken.None).GetAwaiter().GetResult();
        }
        catch (Exception ex)
        {
            Dialogs.Error("forgetop", ex.Message);
        }
    }

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

    private void ApplyTheme()
    {
        var scheme = _theme.Scheme();
        _window.ColorScheme = scheme;
        _connectionsBar.ColorScheme = scheme;
        foreach (var view in Views)
        {
            view.ApplyScheme(scheme);
        }

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
        "  ↵  expand details   Esc  collapse\n" +
        "  F5 refresh   F3 config   F2 theme   F1 help   ^Q quit\n\n" +
        "Pull Requests\n" +
        "  f  cycle filter (All/Mine/ReviewRequested)\n" +
        "  a  approve   m  merge   c  comment   d  diff/files   v  comments\n\n" +
        "Work Items\n" +
        "  f  toggle mine   s  set state   c  comment\n\n" +
        "Pipelines\n" +
        "  ↵  jobs + logs   t  trigger/re-run   d  discover & subscribe   u  unsubscribe\n",
        "OK");
}
