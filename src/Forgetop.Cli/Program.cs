using System.ComponentModel;
using Forgetop.Core;
using Spectre.Console;
using Spectre.Console.Cli;

var app = new CommandApp<RootCommand>();
app.Configure(config => config.SetApplicationName(ForgetopInfo.Name));
return app.Run(args);

/// <summary>
/// Default command. Wave 1 just renders a banner; the TUI shell is wired in
/// from Wave 4 onwards.
/// </summary>
internal sealed class RootCommand : Command<RootCommand.Settings>
{
    public sealed class Settings : CommandSettings
    {
        [CommandOption("--demo")]
        [Description("Run with mock data, no credentials required.")]
        public bool Demo { get; init; }
    }

    protected override int Execute(CommandContext context, Settings settings, CancellationToken cancellation)
    {
        AnsiConsole.Write(new FigletText(ForgetopInfo.Name).Color(Color.Teal));
        AnsiConsole.MarkupLine($"[grey]{ForgetopInfo.Tagline}[/]");
        AnsiConsole.MarkupLine(settings.Demo
            ? "[yellow]demo mode[/] — mock data, no credentials needed."
            : "Run with [green]--demo[/] to try it without credentials.");
        return 0;
    }
}
