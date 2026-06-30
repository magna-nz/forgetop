using Forgetop.Cli;
using Forgetop.Core.Configuration;
using Forgetop.Tui;
using Microsoft.Extensions.DependencyInjection;

var demo = args.Contains("--demo");

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
