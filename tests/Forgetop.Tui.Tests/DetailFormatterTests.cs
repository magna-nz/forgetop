using Forgetop.Core.Domain;
using Forgetop.Tui;

namespace Forgetop.Tui.Tests;

public class DetailFormatterTests
{
    [Fact]
    public void Diff_lists_files_and_includes_patches()
    {
        var text = DetailFormatter.Diff(
        [
            new FileChange { Path = "a.cs", Kind = FileChangeKind.Added, Additions = 5, Deletions = 0, Patch = "@@ +a @@" },
            new FileChange { Path = "b.cs", Kind = FileChangeKind.Modified, Additions = 1, Deletions = 2 },
        ]);

        Assert.Contains("Changed files (2):", text);
        Assert.Contains("A a.cs  +5 -0", text);
        Assert.Contains("M b.cs  +1 -2", text);
        Assert.Contains("── a.cs ──", text);
        Assert.Contains("@@ +a @@", text);
    }

    [Fact]
    public void Diff_handles_no_changes()
    {
        Assert.Equal("(no file changes)", DetailFormatter.Diff([]));
    }

    [Fact]
    public void Threads_renders_authors_and_bodies()
    {
        var text = DetailFormatter.Threads(
        [
            new CommentThread
            {
                Id = "t1",
                FilePath = "a.cs",
                Line = 12,
                Comments =
                [
                    new Comment { Id = "c1", Author = new User { Id = "u", DisplayName = "Bob" }, Body = "nit", CreatedAt = DateTimeOffset.UnixEpoch },
                ],
            },
        ]);

        Assert.Contains("── a.cs:12 ──", text);
        Assert.Contains("Bob", text);
        Assert.Contains("nit", text);
    }

    [Fact]
    public void Threads_handles_empty()
    {
        Assert.Equal("(no comments)", DetailFormatter.Threads([]));
    }
}
