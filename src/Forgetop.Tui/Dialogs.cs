using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>Small modal helpers used by the section screens.</summary>
internal static class Dialogs
{
    public static string? Prompt(string title, string label, string initial = "")
    {
        var dialog = new Dialog(title, 64, 9);
        dialog.Add(new Label { X = 1, Y = 1, Text = label });
        var field = new TextField { X = 1, Y = 3, Width = Dim.Fill() - 2, Text = initial };
        dialog.Add(field);

        string? result = null;
        var ok = new Button("OK", is_default: true);
        ok.Clicked += () => { result = field.Text?.ToString() ?? string.Empty; Application.RequestStop(); };
        var cancel = new Button("Cancel");
        cancel.Clicked += () => { result = null; Application.RequestStop(); };
        dialog.AddButton(ok);
        dialog.AddButton(cancel);

        field.SetFocus();
        Application.Run(dialog);
        return string.IsNullOrWhiteSpace(result) ? null : result;
    }

    public static string? Pick(string title, IReadOnlyList<string> options)
    {
        if (options.Count == 0)
        {
            Info(title, "Nothing available to choose.");
            return null;
        }

        var dialog = new Dialog(title, 72, Math.Min(22, options.Count + 6));
        var list = new ListView(options.ToList())
        {
            X = 1,
            Y = 1,
            Width = Dim.Fill() - 2,
            Height = Dim.Fill() - 2,
        };
        dialog.Add(list);

        string? result = null;
        var ok = new Button("Select", is_default: true);
        ok.Clicked += () =>
        {
            var i = list.SelectedItem;
            if (i >= 0 && i < options.Count)
            {
                result = options[i];
            }

            Application.RequestStop();
        };
        var cancel = new Button("Cancel");
        cancel.Clicked += () => { result = null; Application.RequestStop(); };
        dialog.AddButton(ok);
        dialog.AddButton(cancel);

        list.SetFocus();
        Application.Run(dialog);
        return result;
    }

    public static bool Confirm(string title, string message) =>
        MessageBox.Query(title, message, "Yes", "No") == 0;

    public static void Info(string title, string message) =>
        MessageBox.Query(title, message, "OK");

    public static void Error(string title, string message) =>
        MessageBox.ErrorQuery(title, message, "OK");
}
