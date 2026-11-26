// Files as values: the `File` type of messages, entity columns and job payloads, its JSON, and the options of a file
// field (`[MaxSize]`, `[Accept]`). Reading and writing the bytes is `ctx.Files` (Files.cs).
using System.Globalization;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Celobus.Sdk;

/// <summary>
/// A file: a typed value of a message field, an entity column or a job payload (<c>file</c> in the descriptor;
/// <c>File</c>, <c>File?</c>, <c>List&lt;File&gt;</c>). It names the file, never holds its bytes: read and write them
/// with <c>ctx.Files</c> (<see cref="Files.Open"/>, <see cref="Files.Create"/>). JSON:
/// <c>{"id", "name", "type", "size"}</c>, <c>size</c> in bytes as a string like every 64-bit integer
/// (<c>"2100000"</c>; a number is read too).
/// <para>
/// A file comes from a caller (an endpoint's <c>File</c> field: uploaded by a screen, the CLI or another client,
/// within the field's <see cref="MaxSizeAttribute"/> and <see cref="AcceptAttribute"/>), from another plugin (in a
/// call or its answer), or from the plugin's own code. It lives while a row, a pending job, a schedule or a live agent
/// run of the plugin holds it; a file nothing holds is collected after a grace period. There is no delete: remove it
/// from the record. A <c>File</c> in an endpoint's answer reaches the client with a short-lived URL to read it.
/// </para>
/// <para>
/// <b><c>File</c> and <c>System.IO.File</c>.</b> A project with implicit usings imports <c>System.IO</c>, so a bare
/// <c>File</c> next to <c>using Celobus.Sdk;</c> would name two types. The package declares
/// <c>global using File = Celobus.Sdk.File;</c> for the project that references it (the MSBuild item
/// <c>&lt;Using Include="Celobus.Sdk.File" Alias="File" /&gt;</c>; set the property <c>CelobusFileAlias</c> to
/// <c>false</c> to leave it out): in a plugin, <c>File</c> is this type and the file system's is written
/// <c>System.IO.File</c> (<c>System.IO.File.ReadAllText(path)</c>). <c>Stream</c>, <c>StreamReader</c> and the rest of
/// <c>System.IO</c> are unaffected. A project that references the SDK's sources rather than the package adds the same
/// item, or <c>using File = Celobus.Sdk.File;</c> at the top of a file.
/// </para>
/// </summary>
/// <param name="Id">The file's id.</param>
/// <param name="Name">Its name as uploaded or made (<c>report.csv</c>).</param>
/// <param name="ContentType">Its MIME type (<c>text/csv</c>), <c>type</c> in JSON; <c>application/octet-stream</c> when
/// unknown, or when the bytes do not match a declared image, audio, video or PDF type.</param>
/// <param name="Size">Its size in bytes.</param>
[JsonConverter(typeof(FileJsonConverter))]
public sealed record File(string Id, string Name, string ContentType, long Size)
{
    /// <summary>A file known by its id alone (its name, type and size are filled in where the platform reads it).</summary>
    public File(string id) : this(id, "", "", 0) { }
}

/// <summary>
/// The largest file a file field (<c>File</c>, <c>File?</c>, <c>List&lt;File&gt;</c>) takes: bytes
/// (<c>[MaxSize(1048576)]</c>) or a number and a unit (<c>[MaxSize("1 GiB")]</c>, <c>"500 MB"</c>, <c>"1.5 GiB"</c>;
/// <c>KiB</c>, <c>MiB</c>, <c>GiB</c>, <c>TiB</c> binary, <c>KB</c>, <c>MB</c>, <c>GB</c>, <c>TB</c> decimal, case
/// ignored). Without it, the workspace's limit applies. A larger upload is refused with 413 <c>file_too_large</c>.
/// </summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class MaxSizeAttribute : Attribute
{
    public MaxSizeAttribute(string size) { Text = size; }

    public MaxSizeAttribute(long bytes) { Text = bytes.ToString(CultureInfo.InvariantCulture); }

    /// <summary>The size as declared.</summary>
    public string Text { get; }

    /// <summary>The size in bytes; null when <see cref="Text"/> is not a size.</summary>
    public long? Bytes => FileSize.Parse(Text);
}

/// <summary>
/// The MIME types (<c>text/csv</c>) or patterns (<c>image/*</c>) a file field takes: <c>[Accept("text/csv")]</c>,
/// <c>[Accept("image/*", "application/pdf")]</c>. Without it, any type. Another type is refused with 415
/// <c>file_type_refused</c>.
/// </summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false, AllowMultiple = true)]
public sealed class AcceptAttribute(params string[] types) : Attribute
{
    public IReadOnlyList<string> Types { get; } = types;
}

/// <summary>Sizes as file fields declare them.</summary>
internal static class FileSize
{
    /// <summary>
    /// A size: bytes, or a number (fractions allowed) and a unit (binary <c>KiB</c>..<c>TiB</c>, decimal <c>B</c>,
    /// <c>KB</c>..<c>TB</c>, case ignored); null when it is not one, overflows, or is zero.
    /// </summary>
    public static long? Parse(string? text)
    {
        var s = (text ?? "").Trim();
        var split = 0;
        while (split < s.Length && (char.IsAsciiDigit(s[split]) || s[split] == '.')) split++;
        var (num, unit) = (s[..split], s[split..].Trim());
        if (num.Length == 0 || num.StartsWith('.') || num.EndsWith('.') || num.Count(c => c == '.') > 1) return null;
        ulong mult = unit.ToLowerInvariant() switch
        {
            "" or "b" => 1,
            "kb" => 1_000,
            "mb" => 1_000_000,
            "gb" => 1_000_000_000,
            "tb" => 1_000_000_000_000,
            "kib" => 1UL << 10,
            "mib" => 1UL << 20,
            "gib" => 1UL << 30,
            "tib" => 1UL << 40,
            _ => 0,
        };
        if (mult == 0) return null;
        var dot = num.IndexOf('.');
        var (whole, frac) = dot < 0 ? (num, "") : (num[..dot], num[(dot + 1)..]);
        if (!ulong.TryParse(whole, NumberStyles.None, CultureInfo.InvariantCulture, out var w)) return null;
        UInt128 bytes = (UInt128)w * mult;
        if (frac.Length > 0)
        {
            var digits = Math.Min(frac.Length, 18);
            var f = ulong.Parse(frac[..digits], NumberStyles.None, CultureInfo.InvariantCulture);
            UInt128 pow = 1;
            for (var i = 0; i < digits; i++) pow *= 10;
            bytes += (UInt128)f * mult / pow;
        }
        return bytes == 0 || bytes > long.MaxValue ? null : (long)bytes;
    }
}

/// <summary><see cref="File"/>: <c>{"id", "name", "type", "size"}</c>, the size a string (a number is read too).</summary>
internal sealed class FileJsonConverter : JsonConverter<File>
{
    public override File Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        if (reader.TokenType != JsonTokenType.StartObject) throw new JsonException("expected a file object ({\"id\": ..})");
        string id = "", name = "", type = "";
        long size = 0;
        while (reader.Read())
        {
            if (reader.TokenType == JsonTokenType.EndObject)
                return id.Length == 0 ? throw new FieldJsonException("id", "a file needs its id", null) : new File(id, name, type, size);
            var key = reader.GetString()!;
            reader.Read();
            if (reader.TokenType == JsonTokenType.Null) continue;
            switch (key)
            {
                case "id": id = Text(ref reader, key); break;
                case "name": name = Text(ref reader, key); break;
                case "type" or "content_type" or "contentType": type = Text(ref reader, key); break;
                case "size":
                    size = reader.TokenType switch
                    {
                        JsonTokenType.Number when reader.TryGetInt64(out var n) && n >= 0 => n,
                        JsonTokenType.String when long.TryParse(reader.GetString(), NumberStyles.None, CultureInfo.InvariantCulture, out var n) => n,
                        _ => throw new FieldJsonException("size", "a number of bytes", null),
                    };
                    break;
                default: reader.Skip(); break;
            }
        }
        throw new JsonException("unexpected end of the file object");
    }

    private static string Text(ref Utf8JsonReader reader, string key) =>
        reader.TokenType == JsonTokenType.String ? reader.GetString()! : throw new FieldJsonException(key, "a string", null);

    public override void Write(Utf8JsonWriter writer, File value, JsonSerializerOptions options)
    {
        writer.WriteStartObject();
        writer.WriteString("id", value.Id);
        writer.WriteString("name", value.Name);
        writer.WriteString("type", value.ContentType);
        writer.WriteString("size", value.Size.ToString(CultureInfo.InvariantCulture));
        writer.WriteEndObject();
    }
}
