using Terminal.Gui.App;
using Terminal.Gui.Drawing;
using Terminal.Gui.ViewBase;
using Terminal.Gui.Views;

namespace Forgetop.Tui;

/// <summary>Small modal helpers used by the section screens (Terminal.Gui v2).</summary>
internal static class Dialogs
{
    public static string? Prompt(string title, string label, string initial = "")
    {
        var dialog = new Dialog { Title = title, Width = 64, Height = 9 };
        dialog.Add(new Label { X = 1, Y = 1, Text = label });
        var field = new TextField { X = 1, Y = 3, Width = Dim.Fill(2), Text = initial };
        dialog.Add(field);

        string? result = null;
        var ok = new Button { Text = "OK", IsDefault = true };
        ok.Accepting += (_, _) => { result = string.IsNullOrWhiteSpace(field.Text) ? null : field.Text; Application.RequestStop(dialog); };
        var cancel = new Button { Text = "Cancel" };
        cancel.Accepting += (_, _) => { result = null; Application.RequestStop(dialog); };
        dialog.AddButton(ok);
        dialog.AddButton(cancel);

        field.SetFocus();
        Application.Run(dialog);
        dialog.Dispose();
        return result;
    }

    public static string? Pick(string title, IReadOnlyList<string> options)
    {
        if (options.Count == 0)
        {
            Info(title, "Nothing available to choose.");
            return null;
        }

        var dialog = new Dialog { Title = title, Width = 72, Height = Math.Min(22, options.Count + 6) };
        var list = new ListView { X = 1, Y = 1, Width = Dim.Fill(2), Height = Dim.Fill(2) };
        list.SetSource(new System.Collections.ObjectModel.ObservableCollection<string>(options.ToList()));
        dialog.Add(list);

        string? result = null;
        var ok = new Button { Text = "Select", IsDefault = true };
        ok.Accepting += (_, _) =>
        {
            var i = list.SelectedItem ?? -1;
            if (i >= 0 && i < options.Count)
            {
                result = options[i];
            }

            Application.RequestStop(dialog);
        };
        var cancel = new Button { Text = "Cancel" };
        cancel.Accepting += (_, _) => { result = null; Application.RequestStop(dialog); };
        dialog.AddButton(ok);
        dialog.AddButton(cancel);

        list.SetFocus();
        Application.Run(dialog);
        dialog.Dispose();
        return result;
    }

    public static bool Confirm(string title, string message) =>
        MessageBox.Query(Application.Instance, title, message, "Yes", "No") == 0;

    public static void Info(string title, string message) =>
        MessageBox.Query(Application.Instance, title, message, "OK");

    public static void Error(string title, string message) =>
        MessageBox.ErrorQuery(Application.Instance, title, message, "OK");
}
