using System.Data;
using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>
/// A section screen: a one-line header, a full-width column table, and a detail
/// pane that expands *beneath* the table on Enter (collapses on Esc).
/// </summary>
public abstract class SectionView : View
{
    protected readonly TableView Table;
    private readonly FrameView _detailPane;
    private readonly TextView _detailText;
    private bool _expanded;

    protected SectionView()
    {
        Width = Dim.Fill();
        Height = Dim.Fill();

        Table = new TableView
        {
            X = 0,
            Y = 0,
            Width = Dim.Fill(),
            Height = Dim.Fill(),
            FullRowSelect = true,
        };
        Table.Style.ShowHorizontalHeaderUnderline = true;
        Table.Style.ShowHorizontalHeaderOverline = false;
        Table.Style.ShowVerticalCellLines = false;
        Table.Style.ShowVerticalHeaderLines = false;
        Table.Style.AlwaysShowHeaders = true;
        Table.CellActivated += _ => OnActivated(Table.SelectedRow);
        Table.KeyPress += OnTableKey;

        _detailPane = new FrameView("Details")
        {
            X = 0,
            Y = Pos.Bottom(Table),
            Width = Dim.Fill(),
            Height = Dim.Fill(),
            Visible = false,
        };
        _detailText = new TextView { X = 0, Y = 0, Width = Dim.Fill(), Height = Dim.Fill(), ReadOnly = true, WordWrap = true };
        _detailText.KeyPress += OnDetailEsc;

        Add(Table, _detailPane);
    }

    protected int SelectedRow => Table.SelectedRow;

    /// <summary>Apply the active theme so the table (and its status colours) match.</summary>
    public void ApplyScheme(ColorScheme scheme)
    {
        ColorScheme = scheme;
        Table.ColorScheme = scheme;
    }

    /// <summary>Fetch data (network) — safe to call off the UI thread.</summary>
    public abstract Task LoadDataAsync(CancellationToken ct = default);

    /// <summary>Rebuild the table from already-loaded data — must run on the UI thread.</summary>
    public abstract void Render();

    public async Task RefreshAsync(CancellationToken ct = default)
    {
        await LoadDataAsync(ct).ConfigureAwait(true);
        Render();
    }

    /// <summary>Called when the user presses Enter on a row.</summary>
    protected virtual void OnActivated(int row) { }

    /// <summary>Handle a letter action key; return true if handled.</summary>
    protected virtual bool OnActionKey(KeyEvent keyEvent) => false;

    protected void SetTable(DataTable table) => Table.Table = table;

    /// <summary>Show plain-text detail beneath the table.</summary>
    protected void Expand(string detail)
    {
        _detailText.Text = detail;
        ShowDetailPane(_detailText);
    }

    /// <summary>Show an arbitrary (navigable) view beneath the table, e.g. a TreeView.</summary>
    protected void ExpandView(View content)
    {
        content.X = 0;
        content.Y = 0;
        content.Width = Dim.Fill();
        content.Height = Dim.Fill();
        content.KeyPress += OnDetailEsc;
        ShowDetailPane(content);
    }

    private void ShowDetailPane(View content)
    {
        _detailPane.RemoveAll();
        _detailPane.Add(content);
        _detailPane.Visible = true;
        _expanded = true;
        Table.Height = Dim.Percent(55);
        SetNeedsDisplay();
        content.SetFocus();
    }

    private void OnDetailEsc(KeyEventEventArgs args)
    {
        if (args.KeyEvent.Key == Key.Esc)
        {
            Collapse();
            args.Handled = true;
        }
    }

    private void Collapse()
    {
        _detailPane.Visible = false;
        _expanded = false;
        Table.Height = Dim.Fill();
        Table.SetFocus();
        SetNeedsDisplay();
    }

    protected void RunAction(Func<Task> action)
    {
        try
        {
            action().GetAwaiter().GetResult();
            RefreshAsync().GetAwaiter().GetResult();
        }
        catch (Exception ex)
        {
            Dialogs.Error("forgetop", ex.Message);
        }
    }

    protected void ShowDetailSafe(Func<Task<string>> load)
    {
        try
        {
            Expand(load().GetAwaiter().GetResult());
        }
        catch (Exception ex)
        {
            Dialogs.Error("forgetop", ex.Message);
        }
    }

    protected static char KeyChar(KeyEvent keyEvent) => char.ToLowerInvariant((char)keyEvent.KeyValue);

    /// <summary>Give the row table keyboard focus.</summary>
    public void FocusContent() => Table.SetFocus();

    /// <summary>
    /// ←/→ (tab switching) are intercepted by the TabView before the key reaches the
    /// table; ↑/↓/Enter are the table's own row navigation. Here we only add Esc
    /// (collapse the detail pane) and the section's letter actions.
    /// </summary>
    private void OnTableKey(KeyEventEventArgs args)
    {
        if (args.KeyEvent.Key == Key.Esc && _expanded)
        {
            Collapse();
            args.Handled = true;
            return;
        }

        if (OnActionKey(args.KeyEvent))
        {
            args.Handled = true;
        }
    }
}
