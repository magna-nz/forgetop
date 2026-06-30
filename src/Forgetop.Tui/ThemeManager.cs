using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>Named colour schemes with a cycle-through switcher; the current name persists to config.</summary>
public sealed class ThemeManager
{
    private static readonly string[] AllThemes = ["dark", "light", "matrix", "ocean"];

    public ThemeManager(string? initial)
    {
        Current = initial is not null && AllThemes.Contains(initial) ? initial : "dark";
    }

    public string Current { get; private set; }

    public IReadOnlyList<string> Themes => AllThemes;

    /// <summary>Advance to the next theme and return its name.</summary>
    public string Next()
    {
        var index = Array.IndexOf(AllThemes, Current);
        Current = AllThemes[(index + 1) % AllThemes.Length];
        return Current;
    }

    /// <summary>
    /// Build the colour scheme for the current theme. Call after <c>Application.Init</c>.
    /// "dark" uses a Black background so it adopts the host terminal's palette (modern,
    /// native look like azdo / gh-dash); the others are explicit.
    /// </summary>
    public ColorScheme Scheme() => Current switch
    {
        "light" => Build(Color.Black, Color.White, Color.Black, Color.BrightCyan),
        "matrix" => Build(Color.BrightGreen, Color.Black, Color.Black, Color.Green),
        "ocean" => Build(Color.White, Color.Blue, Color.Black, Color.BrightCyan),
        _ => Build(Color.White, Color.Black, Color.Black, Color.BrightCyan), // dark — terminal-native
    };

    private static ColorScheme Build(Color fg, Color bg, Color focusFg, Color focusBg) => new()
    {
        Normal = new Terminal.Gui.Attribute(fg, bg),
        Focus = new Terminal.Gui.Attribute(focusFg, focusBg),
        HotNormal = new Terminal.Gui.Attribute(Color.BrightYellow, bg),
        HotFocus = new Terminal.Gui.Attribute(Color.BrightYellow, focusBg),
        Disabled = new Terminal.Gui.Attribute(Color.DarkGray, bg),
    };
}
