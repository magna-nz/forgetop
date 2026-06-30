using Forgetop.Core.App;
using Forgetop.Core.Configuration;
using Forgetop.Core.Providers;
using Terminal.Gui.App;
using Terminal.Gui.Input;
using Terminal.Gui.ViewBase;
using Terminal.Gui.Views;

namespace Forgetop.Tui;

/// <summary>
/// The forgetop terminal application (Terminal.Gui v2): a tabbed, true-colour
/// dashboard over the bound providers.
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
    private Tabs _tabs = null!;
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
            Application.Run(_window);
        }
        finally
        {
            _window?.Dispose();
            Application.Shutdown();
        }
    }

    private void Build()
    {
        _prView = new PullRequestsView(_prController) { Title = "Pull Requests" };
        _workItemView = new WorkItemsView(_workItemController) { Title = "Work Items" };
        _pipelineView = new PipelinesView(_pipelineController) { Title = "Pipelines" };

        _tabs = new Tabs { X = 0, Y = 0, Width = Dim.Fill(), Height = Dim.Fill(2), TabSpacing = 3 };
        _tabs.Add(_prView);
        _tabs.Add(_workItemView);
        _tabs.Add(_pipelineView);

        foreach (var view in Views)
        {
            view.TabSwitch += SwitchTab;
        }

        _connectionsBar = new ConnectionsBar { X = 0, Y = Pos.Bottom(_tabs) };

        var statusBar = new StatusBar(new[]
        {
            new Shortcut(Key.Q.WithCtrl, "Quit", () => Application.RequestStop(_window), "^Q"),
            new Shortcut(Key.F1, "Help", ShowHelp, "F1"),
            new Shortcut(Key.F2, "Theme", CycleTheme, "F2"),
            new Shortcut(Key.F3, "Config", OpenConfig, "F3"),
            new Shortcut(Key.F5, "Refresh", RefreshAll, "F5"),
        });

        _window = new Window { Title = "forgetop — htop for your software forges", X = 0, Y = 0, Width = Dim.Fill(), Height = Dim.Fill() };
        _window.Add(_tabs, _connectionsBar, statusBar);
    }

    private void SwitchTab(int delta)
    {
        var tabs = _tabs.TabCollection.ToList();
        if (tabs.Count == 0)
        {
            return;
        }

        var index = _tabs.Value is { } current ? tabs.IndexOf(current) : 0;
        if (index < 0)
        {
            index = 0;
        }

        _tabs.Value = tabs[(index + delta + tabs.Count) % tabs.Count];
    }

    private async Task RefreshAllAsync(CancellationToken ct)
    {
        foreach (var view in Views)
        {
            await view.RefreshAsync(ct).ConfigureAwait(true);
        }
    }

    private void StartAutoRefresh() =>
        Application.AddTimeout(RefreshInterval, () =>
        {
            foreach (var view in Views)
            {
                var captured = view;
                _ = Task.Run(async () =>
                {
                    try
                    {
                        await captured.LoadDataAsync().ConfigureAwait(false);
                        Application.Invoke(captured.Render);
                    }
                    catch
                    {
                        // ignore transient refresh failures
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
                Application.Invoke(() => _connectionsBar.Update(health));
            }
            catch
            {
                // ignore; next tick retries
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
        _window.SetScheme(scheme);
        _connectionsBar.Background = scheme.Normal.Background;
        foreach (var view in Views)
        {
            view.ApplyScheme(scheme);
        }

        Application.LayoutAndDraw(true);
    }

    private void CycleTheme()
    {
        var name = _theme.Next();
        _ = _config.SetThemeAsync(name);
        ApplyTheme();
    }

    private static void ShowHelp() => MessageBox.Query(
        Application.Instance,
        "forgetop — keys",
        "\nGlobal:  Tab / ← →  switch section   ↑ ↓  move   ↵ details   Esc collapse\n" +
        "         F5 refresh   F3 config   F2 theme   F1 help   ^Q quit\n\n" +
        "Pull Requests:  f filter   a approve   m merge   c comment   d diff   v comments\n" +
        "Work Items:     f mine   s set state   c comment\n" +
        "Pipelines:      ↵ jobs/steps   t trigger   d discover   u unsubscribe\n",
        "OK");
}
