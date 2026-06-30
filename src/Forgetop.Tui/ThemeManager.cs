using Terminal.Gui.Drawing;
using Attribute = Terminal.Gui.Drawing.Attribute;

namespace Forgetop.Tui;

/// <summary>
/// True-colour themes (Terminal.Gui v2). "slate" is a Catppuccin-ish dark palette;
/// the others are alternates. F2 cycles them and the choice persists.
/// </summary>
public sealed class ThemeManager
{
    private static readonly string[] AllThemes = ["slate", "dark", "light", "matrix"];

    public ThemeManager(string? initial)
    {
        Current = initial is not null && AllThemes.Contains(initial) ? initial : "slate";
    }

    public string Current { get; private set; }

    public IReadOnlyList<string> Themes => AllThemes;

    public string Next()
    {
        var index = Array.IndexOf(AllThemes, Current);
        Current = AllThemes[(index + 1) % AllThemes.Length];
        return Current;
    }

    /// <summary>The base scheme for the current theme. Call after <c>Application.Init</c>.</summary>
    public Scheme Scheme() => Current switch
    {
        "dark" => Build("#d0d0d0", "#101014", "#ffffff", "#2a2a36"),
        "light" => Build("#1f2430", "#eceff4", "#1f2430", "#cfd8e3"),
        "matrix" => Build("#39ff14", "#000000", "#000000", "#0f3d0f"),
        _ => Build("#cdd6f4", "#1e1e2e", "#1e1e2e", "#89b4fa"), // slate
    };

    /// <summary>The window/background colour, for views that paint their own background.</summary>
    public Color Background() => Scheme().Normal.Background;

    private static Scheme Build(string fg, string bg, string selFg, string selBg) => new()
    {
        Normal = new Attribute(new Color(fg), new Color(bg)),
        HotNormal = new Attribute(new Color(fg), new Color(bg)),
        Focus = new Attribute(new Color(selFg), new Color(selBg)),
        HotFocus = new Attribute(new Color(selFg), new Color(selBg)),
        Disabled = new Attribute(new Color("#6c7086"), new Color(bg)),
    };
}
