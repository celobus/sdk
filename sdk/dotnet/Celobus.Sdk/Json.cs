// The protobuf JSON mapping of messages: lowerCamelCase names, int64 and decimal as strings, enums by value name,
// timestamps as RFC 3339 in UTC, dates as yyyy-MM-dd, bytes as base64, absent (null) fields left out. Reading also
// accepts the snake_case names, numbers for int64 and decimal, and leaves unknown fields aside.
using System.Collections.Concurrent;
using System.Globalization;
using System.Reflection;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Celobus.Sdk;

/// <summary>The JSON options of the wire (and of <c>ctx.Call</c> inputs that are not messages).</summary>
public static class CelobusJson
{
    public static JsonSerializerOptions Options { get; } = Create();

    private static JsonSerializerOptions Create()
    {
        var o = new JsonSerializerOptions(JsonSerializerDefaults.Web)
        {
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
            NumberHandling = JsonNumberHandling.AllowReadingFromString | JsonNumberHandling.AllowNamedFloatingPointLiterals,
            Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        };
        o.Converters.Add(new Int64Converter());
        o.Converters.Add(new DecimalConverter());
        o.Converters.Add(new TimestampConverter());
        o.Converters.Add(new DateConverter());
        o.Converters.Add(new EnumConverterFactory());
        o.Converters.Add(new IdConverterFactory());
        o.Converters.Add(new SecretConverter());
        o.Converters.Add(new MessageConverterFactory());
        o.MakeReadOnly(populateMissingResolver: true);
        return o;
    }

    public static string Serialize<T>(T value) => JsonSerializer.Serialize(value, CelobusJson.Options);
    public static T? Deserialize<T>(string json) => JsonSerializer.Deserialize<T>(json, CelobusJson.Options);
}

internal sealed class Int64Converter : JsonConverter<long>
{
    public override long Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) =>
        reader.TokenType == JsonTokenType.String
            ? long.Parse(reader.GetString()!, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture)
            : reader.GetInt64();

    public override void Write(Utf8JsonWriter writer, long value, JsonSerializerOptions options) =>
        writer.WriteStringValue(value.ToString(CultureInfo.InvariantCulture));
}

internal sealed class DecimalConverter : JsonConverter<decimal>
{
    public override decimal Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) =>
        reader.TokenType == JsonTokenType.String
            ? decimal.Parse(reader.GetString()!, NumberStyles.Number | NumberStyles.AllowExponent, CultureInfo.InvariantCulture)
            : reader.GetDecimal();

    public override void Write(Utf8JsonWriter writer, decimal value, JsonSerializerOptions options) =>
        writer.WriteStringValue(value.ToString(CultureInfo.InvariantCulture));
}

/// <summary><c>google.protobuf.Timestamp</c>: UTC with <c>Z</c> and 0, 3, 6 or 9 fractional digits.</summary>
internal sealed class TimestampConverter : JsonConverter<DateTimeOffset>
{
    public override DateTimeOffset Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) =>
        DateTimeOffset.Parse(reader.GetString()!, CultureInfo.InvariantCulture, DateTimeStyles.RoundtripKind);

    public override void Write(Utf8JsonWriter writer, DateTimeOffset value, JsonSerializerOptions options) =>
        writer.WriteStringValue(Format(value));

    internal static string Format(DateTimeOffset value)
    {
        var utc = value.UtcDateTime;
        var ticks = utc.Ticks % TimeSpan.TicksPerSecond; // 100 ns units
        var fraction = ticks == 0 ? ""
            : ticks % 10_000 == 0 ? "." + (ticks / 10_000).ToString("D3", CultureInfo.InvariantCulture)
            : ticks % 10 == 0 ? "." + (ticks / 10).ToString("D6", CultureInfo.InvariantCulture)
            : "." + (ticks * 100).ToString("D9", CultureInfo.InvariantCulture);
        return utc.ToString("yyyy-MM-dd'T'HH:mm:ss", CultureInfo.InvariantCulture) + fraction + "Z";
    }
}

/// <summary><c>celobus.v1.Date</c>: <c>yyyy-MM-dd</c>.</summary>
internal sealed class DateConverter : JsonConverter<DateOnly>
{
    public override DateOnly Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) =>
        DateOnly.ParseExact(reader.GetString()!, "yyyy-MM-dd", CultureInfo.InvariantCulture);

    public override void Write(Utf8JsonWriter writer, DateOnly value, JsonSerializerOptions options) =>
        writer.WriteStringValue(value.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture));
}

/// <summary>Enums by their value names (snake_case members); reading also accepts the member name or the number.</summary>
internal sealed class EnumConverterFactory : JsonConverterFactory
{
    public override bool CanConvert(Type typeToConvert) => typeToConvert.IsEnum;

    public override JsonConverter CreateConverter(Type typeToConvert, JsonSerializerOptions options) =>
        (JsonConverter)Activator.CreateInstance(typeof(EnumConverter<>).MakeGenericType(typeToConvert))!;

    private sealed class EnumConverter<T> : JsonConverter<T> where T : struct, Enum
    {
        private readonly Dictionary<T, string> _names = new();
        private readonly Dictionary<string, T> _values = new(StringComparer.Ordinal);

        public EnumConverter()
        {
            foreach (var f in typeof(T).GetFields(BindingFlags.Public | BindingFlags.Static))
            {
                var value = (T)f.GetValue(null)!;
                var name = Names.Snake(f.Name);
                _names.TryAdd(value, name);
                _values[name] = value;
                _values.TryAdd(f.Name, value);
            }
        }

        public override T Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
        {
            if (reader.TokenType == JsonTokenType.Number)
            {
                var n = reader.GetInt32();
                var v = (T)Enum.ToObject(typeof(T), n);
                if (_names.ContainsKey(v)) return v;
                throw new JsonException($"{n} is not a value of {typeof(T).Name}");
            }
            var s = reader.GetString()!;
            return _values.TryGetValue(s, out var value)
                ? value
                : throw new JsonException($"`{s}` is not a value of {typeof(T).Name}");
        }

        public override void Write(Utf8JsonWriter writer, T value, JsonSerializerOptions options)
        {
            if (!_names.TryGetValue(value, out var name))
                throw new JsonException($"{value} is not a value of {typeof(T).Name}");
            writer.WriteStringValue(name);
        }
    }
}

/// <summary><c>Id&lt;T&gt;</c>: a JSON string (an int64 key too); reading also accepts a number.</summary>
internal sealed class IdConverterFactory : JsonConverterFactory
{
    public override bool CanConvert(Type typeToConvert) =>
        typeToConvert.IsGenericType && typeToConvert.GetGenericTypeDefinition() == typeof(Id<>);

    public override JsonConverter CreateConverter(Type typeToConvert, JsonSerializerOptions options) =>
        (JsonConverter)Activator.CreateInstance(typeof(IdConverter<>).MakeGenericType(typeToConvert.GetGenericArguments()))!;

    private sealed class IdConverter<T> : JsonConverter<Id<T>> where T : class
    {
        public override Id<T> Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
        {
            var s = reader.TokenType == JsonTokenType.Number
                ? reader.GetInt64().ToString(CultureInfo.InvariantCulture)
                : reader.GetString();
            return string.IsNullOrEmpty(s) ? default : new Id<T>(s);
        }

        public override void Write(Utf8JsonWriter writer, Id<T> value, JsonSerializerOptions options) =>
            writer.WriteStringValue(value.Value);
    }
}

/// <summary><c>[Message]</c> and <c>[Entity]</c> types, field by field from their <see cref="MessageSchema"/>.</summary>
internal sealed class MessageConverterFactory : JsonConverterFactory
{
    private static readonly ConcurrentDictionary<Type, JsonConverter> Converters = new();

    public override bool CanConvert(Type typeToConvert) => Schema.IsMessage(typeToConvert);

    public override JsonConverter CreateConverter(Type typeToConvert, JsonSerializerOptions options) =>
        Converters.GetOrAdd(typeToConvert,
            t => (JsonConverter)Activator.CreateInstance(typeof(MessageConverter<>).MakeGenericType(t))!);

    private sealed class MessageConverter<T> : JsonConverter<T> where T : class
    {
        private readonly MessageSchema _schema = MessageSchema.For(typeof(T));

        public override T Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
        {
            if (reader.TokenType != JsonTokenType.StartObject)
                throw new JsonException($"expected an object for {_schema.Name}");
            var fields = _schema.Fields;
            var values = new object?[fields.Count];
            var present = new bool[fields.Count];
            while (reader.Read())
            {
                if (reader.TokenType == JsonTokenType.EndObject)
                {
                    // a single file that is not optional has no default: the JSON must give it
                    foreach (var f in fields)
                        if (f is { Kind: FieldKind.File, Optional: false, Repeated: false } && !present[f.Number - 1])
                            throw new FieldJsonException(f.Name, "a file is required", null);
                    return (T)_schema.Create(values, present);
                }
                var name = reader.GetString()!;
                reader.Read();
                if (!_schema.ByJsonName.TryGetValue(name, out var field))
                {
                    reader.Skip();
                    continue;
                }
                var i = field.Number - 1;
                if (reader.TokenType == JsonTokenType.Null)
                {
                    present[i] = false; // null is the field's default
                    continue;
                }
                if (field.Error != null) throw new FieldJsonException(field.Name, field.Error, null);
                try
                {
                    values[i] = JsonSerializer.Deserialize(ref reader, field.ClrType, options);
                }
                catch (FieldJsonException e)
                {
                    throw new FieldJsonException($"{field.Name}.{e.FieldPath}", e.Reason, e);
                }
                catch (Exception e) when (e is JsonException or FormatException or OverflowException or InvalidOperationException)
                {
                    throw new FieldJsonException(field.Name, Reason(e), e);
                }
                present[i] = true;
            }
            throw new JsonException("unexpected end of the object");
        }

        // STJ's messages end with the path and position of the inner reader, which mean nothing to the caller
        private static string Reason(Exception e)
        {
            var m = e.Message;
            var cut = m.IndexOf(" Path: ", StringComparison.Ordinal);
            return cut >= 0 ? m[..cut] : m;
        }

        public override void Write(Utf8JsonWriter writer, T value, JsonSerializerOptions options)
        {
            writer.WriteStartObject();
            foreach (var field in _schema.Fields)
            {
                var v = field.Property.GetValue(value);
                if (v is null || v is JsonElement { ValueKind: JsonValueKind.Undefined }) continue;
                if (_schema.OmitDefaults && !field.Optional)
                {
                    if (IsDefault(field, v)) continue;
                    if (field is { Kind: FieldKind.Message, Repeated: false })
                    {
                        // a message whose JSON is {} is left out too
                        var m = JsonSerializer.SerializeToElement(v, field.ClrType, options);
                        if (m.ValueKind == JsonValueKind.Object && !m.EnumerateObject().Any()) continue;
                        writer.WritePropertyName(field.JsonName);
                        m.WriteTo(writer);
                        continue;
                    }
                }
                writer.WritePropertyName(field.JsonName);
                JsonSerializer.Serialize(writer, v, field.ClrType, options);
            }
            writer.WriteEndObject();
        }

        /// <summary>
        /// The protobuf default of a scalar or a list that has one (<c>""</c>, <c>0</c>, <c>false</c>, an empty list or
        /// bytes): what <c>[OmitDefaults]</c> leaves out. Never an enum, a decimal, a uuid, a time or a JSON value.
        /// </summary>
        private static bool IsDefault(FieldSchema field, object v)
        {
            if (field.Repeated || field.Kind == FieldKind.Bytes)
                return v is System.Collections.ICollection { Count: 0 }
                       || (v is System.Collections.IEnumerable e && !e.GetEnumerator().MoveNext());
            return field.Kind switch
            {
                FieldKind.String => v is "",
                FieldKind.Bool => v is false,
                FieldKind.Int32 or FieldKind.Int64 or FieldKind.Double =>
                    Convert.ToDouble(v, CultureInfo.InvariantCulture) == 0,
                _ => false,
            };
        }
    }
}

/// <summary>A field of a message could not be read: its snake_case path (<c>lines.quantity</c>) and why.</summary>
internal sealed class FieldJsonException(string fieldPath, string reason, Exception? inner)
    : JsonException($"{fieldPath}: {reason}", inner)
{
    public string FieldPath { get; } = fieldPath;
    public string Reason { get; } = reason;
}
