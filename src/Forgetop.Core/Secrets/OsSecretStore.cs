using System.Diagnostics;
using System.Runtime.Versioning;
using System.Security.Cryptography;
using System.Text;

namespace Forgetop.Core.Secrets;

/// <summary>Picks the right native secret store for the current OS, with an env-var fallback.</summary>
public static class OsSecretStore
{
    public const string Service = "forgetop";

    public static ISecretStore CreateDefault()
    {
        ISecretStore primary = OperatingSystem.IsMacOS() ? new KeychainSecretStore()
            : OperatingSystem.IsWindows() ? new DpapiSecretStore()
            : OperatingSystem.IsLinux() ? new SecretToolSecretStore()
            : new InMemorySecretStore();

        return new FallbackSecretStore(primary, new EnvironmentSecretStore());
    }
}

/// <summary>Runs a child process, returning exit code and stdout; optional stdin.</summary>
internal static class ProcessRunner
{
    public static async Task<(int ExitCode, string StdOut)> RunAsync(string file, IEnumerable<string> args, string? stdin, CancellationToken ct)
    {
        var psi = new ProcessStartInfo(file)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            RedirectStandardInput = stdin is not null,
            UseShellExecute = false,
        };
        foreach (var arg in args)
        {
            psi.ArgumentList.Add(arg);
        }

        using var process = Process.Start(psi) ?? throw new InvalidOperationException($"Could not start '{file}'.");
        if (stdin is not null)
        {
            await process.StandardInput.WriteAsync(stdin).ConfigureAwait(false);
            process.StandardInput.Close();
        }

        var stdout = await process.StandardOutput.ReadToEndAsync(ct).ConfigureAwait(false);
        await process.WaitForExitAsync(ct).ConfigureAwait(false);
        return (process.ExitCode, stdout);
    }
}

/// <summary>macOS Keychain via the <c>security</c> CLI.</summary>
public sealed class KeychainSecretStore : ISecretStore
{
    public bool IsWritable => true;

    public async Task<string?> GetAsync(string key, CancellationToken ct = default)
    {
        var (exit, stdout) = await ProcessRunner.RunAsync(
            "security", ["find-generic-password", "-a", OsSecretStore.Service, "-s", key, "-w"], null, ct).ConfigureAwait(false);
        return exit == 0 ? stdout.TrimEnd('\n') : null;
    }

    public async Task SetAsync(string key, string secret, CancellationToken ct = default)
    {
        // -U updates the item if it already exists.
        var (exit, _) = await ProcessRunner.RunAsync(
            "security", ["add-generic-password", "-U", "-a", OsSecretStore.Service, "-s", key, "-w", secret], null, ct).ConfigureAwait(false);
        if (exit != 0)
        {
            throw new InvalidOperationException($"Keychain write failed for '{key}'.");
        }
    }

    public async Task DeleteAsync(string key, CancellationToken ct = default) =>
        await ProcessRunner.RunAsync("security", ["delete-generic-password", "-a", OsSecretStore.Service, "-s", key], null, ct).ConfigureAwait(false);
}

/// <summary>Linux Secret Service via the <c>secret-tool</c> CLI (libsecret).</summary>
public sealed class SecretToolSecretStore : ISecretStore
{
    public bool IsWritable => true;

    public async Task<string?> GetAsync(string key, CancellationToken ct = default)
    {
        var (exit, stdout) = await ProcessRunner.RunAsync(
            "secret-tool", ["lookup", "service", OsSecretStore.Service, "key", key], null, ct).ConfigureAwait(false);
        return exit == 0 && stdout.Length > 0 ? stdout.TrimEnd('\n') : null;
    }

    public async Task SetAsync(string key, string secret, CancellationToken ct = default) =>
        await ProcessRunner.RunAsync(
            "secret-tool", ["store", "--label=forgetop", "service", OsSecretStore.Service, "key", key], secret, ct).ConfigureAwait(false);

    public async Task DeleteAsync(string key, CancellationToken ct = default) =>
        await ProcessRunner.RunAsync("secret-tool", ["clear", "service", OsSecretStore.Service, "key", key], null, ct).ConfigureAwait(false);
}

/// <summary>Windows DPAPI: per-user encrypted blobs under the config directory.</summary>
[SupportedOSPlatform("windows")]
public sealed class DpapiSecretStore : ISecretStore
{
    private readonly string _directory = Path.Combine(
        Path.GetDirectoryName(Configuration.ConfigPaths.Default())!, "secrets");

    public bool IsWritable => true;

    public Task<string?> GetAsync(string key, CancellationToken ct = default)
    {
        var path = PathFor(key);
        if (!File.Exists(path))
        {
            return Task.FromResult<string?>(null);
        }

        var protectedBytes = File.ReadAllBytes(path);
        var bytes = ProtectedData.Unprotect(protectedBytes, null, DataProtectionScope.CurrentUser);
        return Task.FromResult<string?>(Encoding.UTF8.GetString(bytes));
    }

    public Task SetAsync(string key, string secret, CancellationToken ct = default)
    {
        Directory.CreateDirectory(_directory);
        var protectedBytes = ProtectedData.Protect(Encoding.UTF8.GetBytes(secret), null, DataProtectionScope.CurrentUser);
        File.WriteAllBytes(PathFor(key), protectedBytes);
        return Task.CompletedTask;
    }

    public Task DeleteAsync(string key, CancellationToken ct = default)
    {
        File.Delete(PathFor(key));
        return Task.CompletedTask;
    }

    private string PathFor(string key) => Path.Combine(_directory, Convert.ToHexString(Encoding.UTF8.GetBytes(key)) + ".bin");
}
