// Plugin settings: what an owner or an admin of the workspace sets for the installed plugin (a limit, a webhook, an API
// key). The plugin declares them as one [Message(Kind = MessageKind.Settings)] record — its properties' attributes
// ([Label], [Range], [Url], ..) and initial values (the defaults) give the form and the validators — and reads them with
// ctx.Settings<T>():
//
//   [Message(Kind = MessageKind.Settings)]
//   public sealed record ChatSettings
//   {
//       [Describe("After this many hours a thread no longer sticks to its agent.")]
//       [Label("Thread stays with its agent, hours"), Range(1, 48)]
//       public int ThreadWarmHours { get; init; } = 2;
//       [Label("Webhook URL"), Url, Section("Integrations")]
//       public string? WebhookUrl { get; init; }
//       [Section("Integrations")]
//       public Secret? WebhookToken { get; init; }
//   }
//
//   Plugin.Create("chat").Settings<ChatSettings>(s => s.OnChange(Check));
//   // in a handler
//   var s = ctx.Settings<ChatSettings>();
using System.ComponentModel;
using System.Globalization;
using System.Reflection;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;
using Celobus.Sdk.Ui;

namespace Celobus.Sdk;

/// <summary>
/// A secret setting (an API key, a token): a string the plugin reads with <see cref="Expose"/>, kept encrypted by the
/// platform and never shown again once saved. <see cref="ToString"/> prints <c>Secret(••••)</c>, so it stays out of
/// logs (a record holding it prints that too); in JSON it is a plain string. Only a property of the settings message is
/// one, and it has no default.
/// </summary>
public sealed class Secret(string value) : IEquatable<Secret>
{
    private readonly string _value = value;

    /// <summary>The secret itself: pass it where it is needed, never into a log or an answer.</summary>
    public string Expose() => _value;

    /// <summary><c>Secret(••••)</c>, never the value.</summary>
    public override string ToString() => "Secret(••••)";

    public bool Equals(Secret? other) => other is not null && string.Equals(_value, other._value, StringComparison.Ordinal);

    public override bool Equals(object? obj) => obj is Secret s && Equals(s);

    public override int GetHashCode() => StringComparer.Ordinal.GetHashCode(_value);
}

/// <summary>A <see cref="Secret"/> in JSON: a plain string.</summary>
internal sealed class SecretConverter : JsonConverter<Secret>
{
    public override Secret Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) =>
        reader.TokenType == JsonTokenType.String
            ? new Secret(reader.GetString()!)
            : throw new JsonException("expected a string");

    public override void Write(Utf8JsonWriter writer, Secret value, JsonSerializerOptions options) =>
        writer.WriteStringValue(value.Expose());
}

/// <summary>A setting's label in the settings form; by default its name in words (<c>ThreadWarmHours</c> is <c>Thread warm hours</c>).</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class LabelAttribute(string text) : Attribute
{
    public string Text { get; } = text;
}

/// <summary>The section of the settings form a setting is in (the settings without one share an untitled section).</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class SectionAttribute(string title) : Attribute
{
    public string Title { get; } = title;
}

/// <summary>A hint shown in a setting's empty input.</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class PlaceholderAttribute(string text) : Attribute
{
    public string Text { get; } = text;
}

/// <summary>A text setting filled in a text area.</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class MultilineAttribute : Attribute;

/// <summary>
/// A setting's lowest and highest values: <c>[Range(1, 48)]</c> for a number, <c>[Range("0.01", "1000")]</c> for a
/// decimal, <c>[Range("2020-01-01", "2030-12-31")]</c> for a date; one side alone: <c>[Range(Min = 1)]</c>.
/// </summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class RangeAttribute : Attribute
{
    public RangeAttribute() { }

    public RangeAttribute(object min, object max)
    {
        Min = min;
        Max = max;
    }

    /// <summary>The lowest value (null: none).</summary>
    public object? Min { get; set; }

    /// <summary>The highest value (null: none).</summary>
    public object? Max { get; set; }
}

/// <summary>The fewest characters (Unicode code points) of a text or a secret setting.</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class MinLengthAttribute(int length) : Attribute
{
    public int Length { get; } = length;
}

/// <summary>The most characters (Unicode code points) of a text or a secret setting.</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class MaxLengthAttribute(int length) : Attribute
{
    public int Length { get; } = length;
}

/// <summary>
/// A regular expression a text setting matches somewhere (anchor it with <c>^</c> and <c>$</c>): literals, classes,
/// <c>\d</c>, <c>\w</c>, groups, alternation and repetition.
/// </summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class PatternAttribute(string pattern) : Attribute
{
    public string Pattern { get; } = pattern;
}

/// <summary>A text setting that is an absolute <c>http</c> or <c>https</c> URL.</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class UrlAttribute : Attribute;

/// <summary>A text setting that is an email address.</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class EmailAttribute : Attribute;

/// <summary>The fewest items of a list setting.</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class MinItemsAttribute(int count) : Attribute
{
    public int Count { get; } = count;
}

/// <summary>The most items of a list setting.</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class MaxItemsAttribute(int count) : Attribute
{
    public int Count { get; } = count;
}

/// <summary>The text shown under a setting when any of its validators refuses a value, instead of the platform's own.</summary>
[AttributeUsage(AttributeTargets.Property | AttributeTargets.Parameter, Inherited = false)]
public sealed class ValidationMessageAttribute(string text) : Attribute
{
    public string Text { get; } = text;
}

/// <summary>A change of the settings the plugin checks before it is saved (<see cref="SettingsOptions{T}.OnChange"/>).</summary>
/// <param name="Previous">The settings before the change; null when they did not read (a required value was never set).</param>
/// <param name="Current">The settings to be saved, as <c>ctx.Settings&lt;T&gt;()</c> would read them.</param>
public sealed record SettingsChange<T>(T? Previous, T Current) where T : class;

/// <summary>
/// The plugin's settings, as <c>Plugin.Settings&lt;T&gt;(s =&gt; s.OnChange(..).Form(..))</c> declares them: an
/// optional check of each change and an optional form laid out by hand.
/// </summary>
public sealed class SettingsOptions<T> where T : class
{
    internal Func<Ctx, SettingsChange<T>, Task>? OnChangeHandler { get; private set; }
    internal Action<FormBuilder<T>>? FormLayout { get; private set; }

    internal SettingsOptions() { }

    /// <summary>
    /// Checks a change before it is saved, for what validators cannot say (fields that go together, an outside service
    /// that must answer). It runs as a system request (no caller: it reads the plugin's own data and calls out) within 10
    /// seconds. Returning lets the change be saved; throwing refuses it:
    /// <c>PluginError.Invalid(..).Field("webhook_url", "Didn't answer")</c> shows each message under its field, any
    /// other error its message.
    /// </summary>
    public SettingsOptions<T> OnChange(Func<Ctx, SettingsChange<T>, Task> handler)
    {
        OnChangeHandler = handler;
        return this;
    }

    /// <summary>
    /// Lays the form out by hand: <c>f =&gt; { f.Section("Agents"); f.Number(s =&gt; s.ThreadWarmHours, "Hours"); .. }</c>,
    /// every property once. Each input still gets its field's help (its description), placeholder, whether it is
    /// required and an enum's options, unless it says its own. Without it the form follows the record: its properties
    /// in order, in sections by their <c>[Section]</c>; an enum is a select, a list of enum values a multi-select, a
    /// bool a switch, a number a number, a <c>[Multiline]</c> string a text area, a list of strings a text area (one per
    /// line), a <see cref="Secret"/> a secret input, an <c>Id&lt;Platform.User&gt;</c> a member.
    /// </summary>
    public SettingsOptions<T> Form(Action<FormBuilder<T>> layout)
    {
        FormLayout = layout;
        return this;
    }
}

/// <summary>The settings as <c>Plugin.Settings</c> declared them.</summary>
internal sealed class SettingsDecl(Type type, Func<Ctx, JsonNode?, Task>? onChange, Func<JsonArray>? form)
{
    public Type Type { get; } = type;
    /// <summary>The check hook, given the body <c>{previous, current}</c>.</summary>
    public Func<Ctx, JsonNode?, Task>? OnChange { get; } = onChange;
    /// <summary>The form laid out by hand: its sections as JSON (null: derived).</summary>
    public Func<JsonArray>? Form { get; } = form;

    public static SettingsDecl Of<T>(SettingsOptions<T> o) where T : class
    {
        Func<Ctx, JsonNode?, Task>? hook = null;
        if (o.OnChangeHandler is { } handler)
            hook = (ctx, body) =>
            {
                var current = SettingsReader.Read<T>(body?["current"] as JsonObject);
                T? previous = null;
                if (body?["previous"] is JsonObject p)
                    try { previous = SettingsReader.Read<T>(p); }
                    catch (PluginError) { previous = null; }
                return handler(ctx, new SettingsChange<T>(previous, current));
            };
        Func<JsonArray>? form = null;
        if (o.FormLayout is { } layout)
            form = () =>
            {
                var f = new FormBuilder<T>();
                layout(f);
                var w = new UiWriter();
                return new JsonArray([.. f.Sections
                    .Where(s => s.Title is not null || s.Inputs.Count > 0)
                    .Select(s => s.ToObj().ToJson(w))]);
            };
        return new SettingsDecl(typeof(T), hook, form);
    }
}

/// <summary>The options of a settings field as declared (attributes and the initial value).</summary>
internal sealed record SettingOptions
{
    public static readonly SettingOptions None = new();

    public string? Label { get; init; }
    public string? Section { get; init; }
    public string? Placeholder { get; init; }
    public bool Multiline { get; init; }
    public object? Min { get; init; }
    public object? Max { get; init; }
    public int? MinLen { get; init; }
    public int? MaxLen { get; init; }
    public string? Pattern { get; init; }
    public string? Format { get; init; }
    public int? MinItems { get; init; }
    public int? MaxItems { get; init; }
    public string? Message { get; init; }
    public bool HasDefault { get; init; }
    public object? Default { get; init; }

    /// <summary>Whether any option is given.</summary>
    public bool Any => this != None;

    /// <summary>An instance of a settings message, for its properties' initial values; null when it has no constructor without parameters.</summary>
    public static object? Sample(Type t)
    {
        try
        {
            return t.GetConstructor(BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance, Type.EmptyTypes) is null
                ? null
                : Activator.CreateInstance(t, nonPublic: true);
        }
        catch (Exception)
        {
            return null;
        }
    }

    /// <summary>
    /// A field's options; <paramref name="settings"/>: a field of a settings message, whose <c>[DefaultValue]</c> and
    /// initial value (<paramref name="sample"/>'s) are its default (elsewhere <c>[DefaultValue]</c> means what it means to
    /// other libraries).
    /// </summary>
    public static SettingOptions Of(PropertyInfo prop, ParameterInfo? param, bool settings, object? sample)
    {
        TA? A<TA>() where TA : Attribute => prop.GetCustomAttribute<TA>() ?? param?.GetCustomAttribute<TA>();
        var range = A<RangeAttribute>();
        var (hasDefault, value) = settings ? DefaultOf(prop, A<DefaultValueAttribute>(), sample) : (false, null);
        var o = new SettingOptions
        {
            Label = A<LabelAttribute>()?.Text,
            Section = A<SectionAttribute>()?.Title,
            Placeholder = A<PlaceholderAttribute>()?.Text,
            Multiline = A<MultilineAttribute>() is not null,
            Min = range?.Min,
            Max = range?.Max,
            MinLen = A<MinLengthAttribute>()?.Length,
            MaxLen = A<MaxLengthAttribute>()?.Length,
            Pattern = A<PatternAttribute>()?.Pattern,
            Format = A<UrlAttribute>() is not null ? "url" : A<EmailAttribute>() is not null ? "email" : null,
            MinItems = A<MinItemsAttribute>()?.Count,
            MaxItems = A<MaxItemsAttribute>()?.Count,
            Message = A<ValidationMessageAttribute>()?.Text,
            HasDefault = hasDefault,
            Default = value,
        };
        return o == None ? None : o;
    }

    /// <summary>
    /// A setting's default: <c>[DefaultValue(..)]</c>, else its initial value when that is not the type's own (null,
    /// <c>0</c>, <c>false</c>, <c>""</c>, an empty list, an enum's first member: say those with <c>[DefaultValue]</c>).
    /// </summary>
    private static (bool, object?) DefaultOf(PropertyInfo prop, DefaultValueAttribute? attr, object? sample)
    {
        if (attr is not null) return (true, attr.Value);
        if (sample is null) return (false, null);
        object? v;
        try { v = prop.GetValue(sample); }
        catch (Exception) { return (false, null); }
        return v is null || IsZero(v) ? (false, null) : (true, v);
    }

    private static bool IsZero(object v) => v switch
    {
        string s => s.Length == 0,
        bool b => !b,
        Secret => false,
        System.Collections.ICollection c => c.Count == 0,
        System.Collections.IEnumerable e => !e.GetEnumerator().MoveNext(),
        _ => v.GetType().IsValueType && v.Equals(Activator.CreateInstance(v.GetType())),
    };
}

/// <summary>Settings JSON (the plugin's JSON mapping) read as the settings message.</summary>
internal static class SettingsReader
{
    /// <summary>
    /// The settings read as <typeparamref name="T"/>: the declared defaults fill what is absent; a required field with no
    /// value is a 409 <c>settings_required</c> naming it; a value that does not read is a 500.
    /// </summary>
    public static T Read<T>(JsonObject? json) where T : class => (T)Read(typeof(T), json);

    public static object Read(Type t, JsonObject? json)
    {
        var schema = MessageSchema.For(t);
        if (schema.Kind != MessageKind.Settings)
            throw new PluginError(500, "internal", $"{t.Name} is not a settings message: declare it with [Message(Kind = MessageKind.Settings)]");
        var obj = json?.DeepClone().AsObject() ?? new JsonObject();
        var missing = new List<string>();
        foreach (var f in schema.Fields)
        {
            if (obj[f.JsonName] is not null || obj[f.Name] is not null) continue;
            if (f.Setting.HasDefault && SettingsDescribe.Spell(f, f.Setting.Default) is { } d)
                obj[f.JsonName] = f is { Kind: FieldKind.Int64, Repeated: false } ? JsonValue.Create(d.ToJsonString()) : d;
            else if (!f.Optional && !f.Repeated) missing.Add(f.Name);
        }
        if (missing.Count > 0)
        {
            var error = new PluginError(409, "settings_required",
                $"The plugin needs setup: an owner or an admin sets {string.Join(", ", missing)} in its settings",
                new JsonObject { ["missing"] = new JsonArray([.. missing.Select(m => (JsonNode)m)]) });
            foreach (var m in missing) error.Field(m, "Required");
            throw error;
        }
        try
        {
            return JsonSerializer.Deserialize(obj, t, CelobusJson.Options) ?? throw new JsonException("null");
        }
        catch (Exception e) when (e is JsonException or FormatException or OverflowException or ArgumentException or InvalidOperationException)
        {
            throw new PluginError(500, "internal", $"the settings do not read as {t.Name}: {e.Message}");
        }
    }
}

/// <summary>The settings a process holds: the latest JSON the platform sent (or a test gave), and the value last read from it.</summary>
internal sealed class SettingsCache(Type? declared, JsonObject? json = null)
{
    private readonly Lock _lock = new();
    private ulong _rev;
    private JsonObject? _json = json;
    private (Type Type, object Value)? _read;

    /// <summary>The settings message the plugin declares (null: none, or a test's).</summary>
    public Type? Declared { get; } = declared;

    /// <summary>
    /// Takes settings from the platform: at registration always, after it only a revision not older than the held one.
    /// Empty bytes are no settings. Whether they were taken.
    /// </summary>
    public bool Update(ulong rev, ReadOnlySpan<byte> bytes, bool registered)
    {
        JsonObject? json = null;
        var text = Encoding.UTF8.GetString(bytes);
        if (!string.IsNullOrWhiteSpace(text))
        {
            try
            {
                json = JsonNode.Parse(text) as JsonObject ?? new JsonObject();
            }
            catch (JsonException)
            {
                return false;
            }
        }
        lock (_lock)
        {
            if (!registered && rev < _rev) return false;
            _rev = rev;
            _json = json;
            _read = null;
            return true;
        }
    }

    /// <summary>The settings read as <typeparamref name="T"/>: the defaults when the platform sent none.</summary>
    public T Get<T>() where T : class
    {
        if (Declared is not null && Declared != typeof(T))
            throw new PluginError(500, "internal", $"{typeof(T).Name} is not the plugin's settings: Plugin.Settings declares {Declared.Name}");
        JsonObject? json;
        lock (_lock)
        {
            if (_read is { } r && r.Type == typeof(T)) return (T)r.Value;
            json = _json;
        }
        if (json is null && Declared is null)
            throw new PluginError(500, "internal",
                $"the plugin declares no settings: Plugin.Settings<{typeof(T).Name}>() (in a test, ctx.WithSettings(..))");
        var value = SettingsReader.Read<T>(json);
        lock (_lock)
        {
            if (ReferenceEquals(_json, json)) _read = (typeof(T), value);
        }
        return value;
    }
}
