using System.Data;
using Terminal.Gui.App;
using Terminal.Gui.Drawing;
using Terminal.Gui.Input;
using Terminal.Gui.ViewBase;
using Terminal.Gui.Views;

namespace Forgetop.Tui;

/// <summary>
/// A section screen (v2): a full-width column table with a detail pane that
/// expands beneath it on Enter and collapses on Esc.
/// </summary>
public abstract class SectionView : View
{
    protected readonly TableView Table;
    private readonly FrameView _detailPane;
    private readonly Label _detailText;
    private bool _expanded;

    /// <summary>Raised on ←/→ to move between tabs (-1 / +1).</summary>
    public event Action<int>? TabSwitch;

    protected SectionView()
    {
        Width = Dim.Fill();
        Height = Dim.Fill();

        Table = new TableView { X = 0, Y = 0, Width = Dim.Fill(), Height = Dim.Fill(), FullRowSelect = true };
        Table.Style.ShowHorizontalHeaderUnderline = true;
        Table.Style.ShowHorizontalHeaderOverline = false;
        Table.Style.ShowVerticalCellLines = false;
        Table.Activated += (_, _) => OnActivated(SelectedRow);
        Table.KeyDown += OnKey;

        _detailPane = new FrameView { Title = "Details", X = 0, Y = Pos.Bottom(Table), Width = Dim.Fill(), Height = Dim.Fill(), Visible = false };
        _detailText = new Label { X = 0, Y = 0, Width = Dim.Fill(), Height = Dim.Fill() };
        _detailText.KeyDown += OnDetailKey;

        Add(Table, _detailPane);
    }

    private Color _background = new("#1e1e2e");

    protected int SelectedRow => Table.Value?.SelectedCell.Y ?? 0;

    public void ApplyScheme(Scheme scheme)
    {
        _background = scheme.Normal.Background;
        SetScheme(scheme);
    }

    /// <summary>Colour a column's cells per row (e.g. the status column).</summary>
    protected void ColorColumn(int columnIndex, Func<int, Color> rowColor)
    {
        Table.Style.ColumnStyles[columnIndex] = new ColumnStyle
        {
            ColorGetter = args => StatusColors.Scheme(rowColor(args.RowIndex), _background),
        };
    }

    protected static char KeyChar(Key key) => char.ToLowerInvariant((char)key.AsRune.Value);

    public abstract Task LoadDataAsync(CancellationToken ct = default);

    public abstract void Render();

    public async Task RefreshAsync(CancellationToken ct = default)
    {
        await LoadDataAsync(ct).ConfigureAwait(true);
        Render();
    }

    protected virtual void OnActivated(int row) { }

    protected virtual bool OnActionKey(Key key) => false;

    protected void SetTable(DataTable table) => Table.Table = new DataTableSource(table);

    protected void Expand(string detail)
    {
        _detailText.Text = detail;
        ShowDetailPane(_detailText);
    }

    protected void ExpandView(View content)
    {
        content.X = 0;
        content.Y = 0;
        content.Width = Dim.Fill();
        content.Height = Dim.Fill();
        content.KeyDown += OnDetailKey;
        ShowDetailPane(content);
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

    private void ShowDetailPane(View content)
    {
        _detailPane.RemoveAll();
        _detailPane.Add(content);
        _detailPane.Visible = true;
        _expanded = true;
        Table.Height = Dim.Percent(55);
        content.SetFocus();
        SetNeedsDraw();
    }

    private void Collapse()
    {
        _detailPane.Visible = false;
        _expanded = false;
        Table.Height = Dim.Fill();
        Table.SetFocus();
        SetNeedsDraw();
    }

    private void OnKey(object? sender, Key key)
    {
        if (key == Key.CursorLeft)
        {
            TabSwitch?.Invoke(-1);
            key.Handled = true;
        }
        else if (key == Key.CursorRight)
        {
            TabSwitch?.Invoke(1);
            key.Handled = true;
        }
        else if (OnActionKey(key))
        {
            key.Handled = true;
        }
    }

    private void OnDetailKey(object? sender, Key key)
    {
        if (key == Key.Esc && _expanded)
        {
            Collapse();
            key.Handled = true;
        }
    }
}
