namespace Forgetop.Tui;

/// <summary>A single row in a section list: a one-line summary plus detail text.</summary>
public sealed record SectionRow(string Display, string Detail)
{
    public override string ToString() => Display;
}

/// <summary>The rendered contents of one section tab.</summary>
public sealed record SectionData(string ProviderLabel, IReadOnlyList<SectionRow> Rows)
{
    public static SectionData Unbound(string section) =>
        new($"{section}: not configured", [new SectionRow($"No provider bound to {section}.", "Configure it from the setup wizard (coming in a later wave).")]);
}
