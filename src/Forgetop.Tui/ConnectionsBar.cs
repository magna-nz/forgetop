using Forgetop.Core.App;
using Terminal.Gui.Drawing;
using Terminal.Gui.ViewBase;
using Terminal.Gui.Views;

namespace Forgetop.Tui;

/// <summary>One-line bar: each connection with a green/red reachability dot.</summary>
public sealed class ConnectionsBar : View
{
    public ConnectionsBar()
    {
        Width = Dim.Fill();
        Height = 1;
    }

    /// <summary>Background colour to paint the dots against (set from the theme).</summary>
    public Color Background { get; set; } = new("#1e1e2e");

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
            var label = new Label { X = x, Y = 0, Text = text };
            label.SetScheme(StatusColors.Scheme(StatusColors.HealthColor(item.Healthy), Background));
            Add(label);
            x += text.Length + 3;
        }
    }
}
