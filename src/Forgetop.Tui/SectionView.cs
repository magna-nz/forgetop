using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>
/// A master/detail section pane: a list of rows on the left, the selected row's
/// detail on the right. The frame title shows the bound provider.
/// </summary>
public sealed class SectionView : FrameView
{
    private readonly ListView _list;
    private readonly TextView _detail;
    private readonly string _sectionName;
    private IReadOnlyList<SectionRow> _rows = [];

    public SectionView(string sectionName)
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

    public void SetData(SectionData data)
    {
        Title = $"{_sectionName}  ·  {data.ProviderLabel}";
        _rows = data.Rows;
        _list.SetSource(data.Rows.Select(r => r.Display).ToList());
        UpdateDetail(_rows.Count > 0 ? 0 : -1);
    }

    private void OnSelectedItemChanged(ListViewItemEventArgs args) => UpdateDetail(args.Item);

    private void UpdateDetail(int index) =>
        _detail.Text = index >= 0 && index < _rows.Count ? _rows[index].Detail : string.Empty;
}
