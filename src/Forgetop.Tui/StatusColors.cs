using Forgetop.Core.Domain;
using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>Icons + colours for statuses, matching the azdo/gh-dash look with plain Unicode.</summary>
internal static class StatusColors
{
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
        PipelineRunStatus.Succeeded => Color.BrightGreen,
        PipelineRunStatus.Running or PipelineRunStatus.Queued => Color.BrightCyan,
        PipelineRunStatus.Failed => Color.BrightRed,
        PipelineRunStatus.PartiallySucceeded => Color.BrightYellow,
        PipelineRunStatus.Canceled => Color.Gray,
        _ => Color.Gray,
    };

    public static Color CheckColor(CheckStatus s) => s switch
    {
        CheckStatus.Passed => Color.BrightGreen,
        CheckStatus.Failed => Color.BrightRed,
        CheckStatus.Pending => Color.BrightCyan,
        _ => Color.Gray,
    };

    /// <summary>A colour scheme whose foreground is <paramref name="fg"/> over <paramref name="bg"/>.</summary>
    public static ColorScheme Scheme(Color fg, Color bg) =>
        new() { Normal = new Terminal.Gui.Attribute(fg, bg), Focus = new Terminal.Gui.Attribute(fg, bg) };
}
