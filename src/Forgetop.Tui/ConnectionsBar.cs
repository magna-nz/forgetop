using Forgetop.Core.App;
using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>
/// A one-line bar listing each configured connection with a green/red dot for
/// whether it's currently reachable. Sits at the bottom of the window.
/// </summary>
public sealed class ConnectionsBar : View
{
    public ConnectionsBar()
    {
        Width = Dim.Fill();
        Height = 1;
    }

    public void Update(IReadOnlyList<ConnectionHealth> health)
    {
        RemoveAll();

        if (health.Count == 0)
        {
            Add(new Label { X = 1, Y = 0, Text = "no connections — press F3 to add" });
            return;
        }

        var x = 1;
        foreach (var item in health)
        {
            var text = $"● {item.Connection.DisplayName}";
            Add(new Label
            {
                X = x,
                Y = 0,
                Text = text,
                ColorScheme = StatusColors.Scheme(item.Healthy ? Color.BrightGreen : Color.BrightRed, ColorScheme?.Normal.Background ?? Color.Black),
            });
            x += text.Length + 3;
        }
    }
}
