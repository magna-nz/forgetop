using System.Text.Json;

namespace Forgetop.Providers.AzureDevOps;

/// <summary>Small null-safe readers over <see cref="JsonElement"/>.</summary>
internal static class Json
{
    public static string? Str(this JsonElement el, string prop) =>
        el.TryGetProperty(prop, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    public static int? Int(this JsonElement el, string prop) =>
        el.TryGetProperty(prop, out var v) && v.ValueKind == JsonValueKind.Number ? v.GetInt32() : null;

    public static bool Bool(this JsonElement el, string prop) =>
        el.TryGetProperty(prop, out var v) && v.ValueKind == JsonValueKind.True;

    public static DateTimeOffset? Date(this JsonElement el, string prop) =>
        el.TryGetProperty(prop, out var v) && v.ValueKind == JsonValueKind.String && v.TryGetDateTimeOffset(out var d)
            ? d
            : null;

    public static JsonElement? Obj(this JsonElement el, string prop) =>
        el.TryGetProperty(prop, out var v) && v.ValueKind == JsonValueKind.Object ? v : null;

    public static IEnumerable<JsonElement> Arr(this JsonElement el, string prop) =>
        el.TryGetProperty(prop, out var v) && v.ValueKind == JsonValueKind.Array
            ? v.EnumerateArray()
            : [];
}
