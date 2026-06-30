using System.Text;
using DiffPlex;
using DiffPlex.DiffBuilder;
using DiffPlex.DiffBuilder.Model;

namespace Forgetop.Providers.AzureDevOps;

/// <summary>
/// Builds a line-prefixed diff (+/-/space) from two file versions, since Azure
/// DevOps doesn't return patch text directly. Uses DiffPlex for the line diff.
/// </summary>
public static class UnifiedDiff
{
    private static readonly InlineDiffBuilder Builder = new(new Differ());

    public static (string Patch, int Additions, int Deletions) Build(string oldText, string newText)
    {
        var model = Builder.BuildDiffModel(oldText, newText);
        var sb = new StringBuilder();
        var additions = 0;
        var deletions = 0;

        foreach (var line in model.Lines)
        {
            switch (line.Type)
            {
                case ChangeType.Inserted:
                    sb.Append('+').AppendLine(line.Text);
                    additions++;
                    break;
                case ChangeType.Deleted:
                    sb.Append('-').AppendLine(line.Text);
                    deletions++;
                    break;
                case ChangeType.Unchanged:
                    sb.Append(' ').AppendLine(line.Text);
                    break;
                default:
                    break; // Imaginary / Modified padding lines
            }
        }

        return (sb.ToString().TrimEnd(), additions, deletions);
    }
}
