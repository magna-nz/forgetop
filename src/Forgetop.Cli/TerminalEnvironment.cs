namespace Forgetop.Cli;

/// <summary>Detects whether a usable interactive terminal (TTY) is attached.</summary>
public static class TerminalEnvironment
{
    public static bool IsInteractive()
    {
        if (Console.IsInputRedirected || Console.IsOutputRedirected)
        {
            return false;
        }

        // Windows uses its own console driver and doesn't rely on $TERM.
        if (OperatingSystem.IsWindows())
        {
            return true;
        }

        var term = Environment.GetEnvironmentVariable("TERM");
        return !string.IsNullOrEmpty(term) && term != "dumb";
    }
}
