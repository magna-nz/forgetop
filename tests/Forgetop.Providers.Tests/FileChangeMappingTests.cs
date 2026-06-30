using System.Text.Json;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Forgetop.Providers.Demo;

namespace Forgetop.Providers.Tests;

public class FileChangeMappingTests
{
    private static JsonElement Parse(string json) => JsonDocument.Parse(json).RootElement;

    [Fact]
    public void GitHub_maps_file_with_patch()
    {
        var change = Forgetop.Providers.GitHub.GitHubMapper.MapFileChange(Parse("""
        { "filename": "src/a.cs", "status": "modified", "additions": 3, "deletions": 1, "patch": "@@ -1 +1,3 @@" }
        """));

        Assert.Equal("src/a.cs", change.Path);
        Assert.Equal(FileChangeKind.Modified, change.Kind);
        Assert.Equal(3, change.Additions);
        Assert.Equal(1, change.Deletions);
        Assert.Equal("@@ -1 +1,3 @@", change.Patch);
    }

    [Theory]
    [InlineData("added", FileChangeKind.Added)]
    [InlineData("removed", FileChangeKind.Deleted)]
    [InlineData("renamed", FileChangeKind.Renamed)]
    public void GitHub_maps_status_kinds(string status, FileChangeKind expected)
    {
        var change = Forgetop.Providers.GitHub.GitHubMapper.MapFileChange(Parse($$"""{ "filename": "f", "status": "{{status}}" }"""));
        Assert.Equal(expected, change.Kind);
    }

    [Fact]
    public void AzureDevOps_maps_change_entry_path_and_kind()
    {
        var change = Forgetop.Providers.AzureDevOps.AzureDevOpsMapper.MapChangeEntry(Parse("""
        { "changeType": "edit", "item": { "path": "/src/a.cs" } }
        """));

        Assert.Equal("/src/a.cs", change.Path);
        Assert.Equal(FileChangeKind.Modified, change.Kind);
        Assert.Null(change.Patch);
    }

    [Fact]
    public void AzureDevOps_unified_diff_marks_inserts_and_deletes()
    {
        var (patch, additions, deletions) = Forgetop.Providers.AzureDevOps.UnifiedDiff.Build(
            oldText: "one\ntwo\nthree",
            newText: "one\nTWO\nthree\nfour");

        Assert.Equal(2, additions);  // "TWO" and "four"
        Assert.Equal(1, deletions);  // "two"
        Assert.Contains("+TWO", patch);
        Assert.Contains("-two", patch);
        Assert.Contains("+four", patch);
        Assert.Contains(" one", patch); // unchanged line kept with space prefix
    }

    [Fact]
    public async Task Demo_returns_canned_changes()
    {
        var conn = new DemoProviderFactory().Create(
            new Connection { Id = "demo", ProviderType = ProviderType.Demo, DisplayName = "Demo" }, null);

        var changes = await conn.PullRequests!.GetChangesAsync("101");
        Assert.NotEmpty(changes);
        Assert.Contains(changes, c => c.Kind == FileChangeKind.Added && c.Patch is not null);
    }
}
