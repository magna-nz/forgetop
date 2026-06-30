using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>
/// A master/detail section pane: list of rows on the left, selected row's detail
/// on the right. Subclasses load their data and handle action keys.
/// </summary>
public abstract class SectionView : FrameView
{
    private readonly ListView _list;
    private readonly TextView _detail;
    private readonly string _sectionName;
    private IReadOnlyList<SectionRow> _rows = [];

    protected SectionView(string sectionName)
    {
        _sectionName = sectionName;
        Title = sectionName;

        _list = new ListView
        {
            X = 0,
            Y = 0,
            Width = Dim.Percent(45),
            Height = Dim.Fill(),
            AllowsMarking = false,
        };
        _list.SelectedItemChanged += OnSelectedItemChanged;

        _detail = new TextView
        {
            X = Pos.Right(_list) + 1,
            Y = 0,
            Width = Dim.Fill(),
            Height = Dim.Fill(),
            ReadOnly = true,
            WordWrap = true,
        };

        Add(_list, _detail);
    }

    protected int SelectedIndex => _list.SelectedItem;

    /// <summary>Load this section's data (called on initial load and on refresh).</summary>
    protected abstract Task<SectionData> LoadAsync(CancellationToken ct = default);

    public async Task ReloadAsync(CancellationToken ct = default) => Apply(await LoadAsync(ct).ConfigureAwait(true));

    public void Apply(SectionData data)
    {
        Title = $"{_sectionName}  ·  {data.ProviderLabel}";
        _rows = data.Rows;
        _list.SetSource(data.Rows.Select(r => r.Display).ToList());
        UpdateDetail(_rows.Count > 0 ? 0 : -1);
    }

    public override bool ProcessKey(KeyEvent keyEvent) => OnActionKey(keyEvent) || base.ProcessKey(keyEvent);

    /// <summary>Handle a section-specific action key; return true if handled.</summary>
    protected virtual bool OnActionKey(KeyEvent keyEvent) => false;

    /// <summary>Replace the detail pane text (e.g. pipeline drill-in).</summary>
    protected void ShowDetail(string text) => _detail.Text = text;

    /// <summary>Load detail text asynchronously and show it, surfacing any error.</summary>
    protected void ShowDetailSafe(Func<Task<string>> load)
    {
        try
        {
            ShowDetail(load().GetAwaiter().GetResult());
        }
        catch (Exception ex)
        {
            Dialogs.Error("forgetop", ex.Message);
        }
    }

    /// <summary>Run an async action, refresh the list, and surface any error.</summary>
    protected void RunAction(Func<Task> action)
    {
        try
        {
            action().GetAwaiter().GetResult();
            ReloadAsync().GetAwaiter().GetResult();
        }
        catch (Exception ex)
        {
            Dialogs.Error("forgetop", ex.Message);
        }
    }

    protected static char KeyChar(KeyEvent keyEvent) => char.ToLowerInvariant((char)keyEvent.KeyValue);

    private void OnSelectedItemChanged(ListViewItemEventArgs args) => UpdateDetail(args.Item);

    private void UpdateDetail(int index) =>
        _detail.Text = index >= 0 && index < _rows.Count ? _rows[index].Detail : string.Empty;
}
