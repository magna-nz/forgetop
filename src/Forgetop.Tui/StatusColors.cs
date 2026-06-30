using Forgetop.Core.Domain;
using Terminal.Gui.Drawing;
using Attribute = Terminal.Gui.Drawing.Attribute;

namespace Forgetop.Tui;

/// <summary>Icons + true-colour accents for statuses (Terminal.Gui v2).</summary>
internal static class StatusColors
{
    private static readonly Color Green = new("#a6e3a1");
    private static readonly Color Blue = new("#89b4fa");
    private static readonly Color Red = new("#f38ba8");
    private static readonly Color Yellow = new("#f9e2af");
    private static readonly Color Grey = new("#6c7086");

    public static string PipelineIcon(PipelineRunStatus s) => s switch
    {
        PipelineRunStatus.Succeeded => "✓",
        PipelineRunStatus.Running or PipelineRunStatus.Queued => "●",
        PipelineRunStatus.Failed => "✗",
        PipelineRunStatus.PartiallySucceeded => "▲",
        PipelineRunStatus.Canceled => "⊘",
        _ => "·",
    };

    public static string PipelineLabel(PipelineRunStatus s) => $"{PipelineIcon(s)} {s}";

    public static string CheckIcon(CheckStatus s) => s switch
    {
        CheckStatus.Passed => "✓",
        CheckStatus.Failed => "✗",
        CheckStatus.Pending => "●",
        _ => "·",
    };

    public static Color PipelineColor(PipelineRunStatus s) => s switch
    {
        PipelineRunStatus.Succeeded => Green,
        PipelineRunStatus.Running or PipelineRunStatus.Queued => Blue,
        PipelineRunStatus.Failed => Red,
        PipelineRunStatus.PartiallySucceeded => Yellow,
        _ => Grey,
    };

    public static Color CheckColor(CheckStatus s) => s switch
    {
        CheckStatus.Passed => Green,
        CheckStatus.Failed => Red,
        CheckStatus.Pending => Blue,
        _ => Grey,
    };

    public static Color HealthColor(bool healthy) => healthy ? Green : Red;

    public static Scheme Scheme(Color fg, Color bg) =>
        new() { Normal = new Attribute(fg, bg), Focus = new Attribute(fg, bg) };
}
