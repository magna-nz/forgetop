using System.Text.Json;

namespace Forgetop.Providers.Linear;

/// <summary>Small null-safe readers over <see cref="JsonElement"/>.</summary>
internal static class Json
{
    public static string? Str(this JsonElement el, string prop) =>
        el.TryGetProperty(prop, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    public static DateTimeOffset? Date(this JsonElement el, string prop) =>
        el.TryGetProperty(prop, out var v) && v.ValueKind == JsonValueKind.String && v.TryGetDateTimeOffset(out var d)
            ? d
            : null;

    public static JsonElement? Obj(this JsonElement el, string prop) =>
        el.TryGetProperty(prop, out var v) && v.ValueKind == JsonValueKind.Object ? v : null;

    public static IEnumerable<JsonElement> Nodes(this JsonElement el, string prop)
    {
        if (el.Obj(prop) is { } container &&
            container.TryGetProperty("nodes", out var nodes) &&
            nodes.ValueKind == JsonValueKind.Array)
        {
            return nodes.EnumerateArray();
        }

        return [];
    }
}
