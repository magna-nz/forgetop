using System.Text.Json;
using System.Text.Json.Serialization;

namespace Forgetop.Core.Configuration;

/// <summary>Loads and saves the root configuration.</summary>
public interface IConfigStore
{
    Task<ForgetopConfig> LoadAsync(CancellationToken ct = default);
    Task SaveAsync(ForgetopConfig config, CancellationToken ct = default);
}

/// <summary>Resolves the on-disk location of forgetop's config file.</summary>
public static class ConfigPaths
{
    public const string DirectoryName = "forgetop";
    public const string FileName = "config.json";

    /// <summary>
    /// <c>$XDG_CONFIG_HOME/forgetop/config.json</c> when set, else the platform
    /// application-data folder.
    /// </summary>
    public static string Default()
    {
        var xdg = Environment.GetEnvironmentVariable("XDG_CONFIG_HOME");
        var root = !string.IsNullOrEmpty(xdg)
            ? xdg
            : Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
        return Path.Combine(root, DirectoryName, FileName);
    }
}

/// <summary>JSON-file implementation of <see cref="IConfigStore"/>.</summary>
public sealed class JsonConfigStore : IConfigStore
{
    private static readonly JsonSerializerOptions Options = new(JsonSerializerDefaults.General)
    {
        WriteIndented = true,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Converters = { new JsonStringEnumConverter() },
    };

    private readonly string _path;

    public JsonConfigStore(string? path = null) => _path = path ?? ConfigPaths.Default();

    public async Task<ForgetopConfig> LoadAsync(CancellationToken ct = default)
    {
        if (!File.Exists(_path))
        {
            return ForgetopConfig.Empty;
        }

        await using var stream = File.OpenRead(_path);
        var config = await JsonSerializer.DeserializeAsync<ForgetopConfig>(stream, Options, ct).ConfigureAwait(false);
        return config ?? ForgetopConfig.Empty;
    }

    public async Task SaveAsync(ForgetopConfig config, CancellationToken ct = default)
    {
        ArgumentNullException.ThrowIfNull(config);

        var directory = Path.GetDirectoryName(_path);
        if (!string.IsNullOrEmpty(directory))
        {
            Directory.CreateDirectory(directory);
        }

        // Write to a temp file then move, so a crash mid-write can't corrupt config.
        var temp = _path + ".tmp";
        await using (var stream = File.Create(temp))
        {
            await JsonSerializer.SerializeAsync(stream, config, Options, ct).ConfigureAwait(false);
        }

        File.Move(temp, _path, overwrite: true);
    }
}
