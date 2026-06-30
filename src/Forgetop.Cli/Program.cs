using Forgetop.Cli;
using Forgetop.Core.Configuration;
using Forgetop.Tui;
using Microsoft.Extensions.DependencyInjection;

var demo = args.Contains("--demo");

if (!TerminalEnvironment.IsInteractive())
{
    Console.Error.WriteLine("forgetop is a full-screen terminal app and needs an interactive terminal (a TTY).");
    Console.Error.WriteLine("Run it directly in your terminal, e.g.:  forgetop --demo");
    return 1;
}

var services = AppHost.Build(demo);
var config = services.GetRequiredService<IConfigService>();
await config.LoadAsync();

if (demo)
{
    await DemoSetup.ApplyAsync(config);
}

var app = services.GetRequiredService<ForgetopApp>();
await app.RunAsync();
return 0;
