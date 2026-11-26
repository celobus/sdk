// The settings in the descriptor: the options of a settings field as the descriptor spells them, the form (derived
// from the fields, or laid out by the plugin and completed from them) and the definition checks of settings. The same
// rules every SDK follows, so each prints the same descriptor; `celobus plugin check` checks the rest (a default
// against its own validators, a pattern both regular expression engines read alike).
using System.Globalization;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;

namespace Celobus.Sdk;

internal static class SettingsDescribe
{
    /// <summary>The most fields the settings message has, and the most secrets among them.</summary>
    private const int MaxFields = 64;
    private const int MaxSecrets = 16;
    /// <summary>The largest default, as JSON.</summary>
    private const int MaxValueBytes = 64 << 10;

    /// <summary>The reserved method of the settings check hook: <c>/&lt;id&gt;.v1.&lt;Service&gt;/_SettingsCheck</c>.</summary>
    internal const string CheckMethod = "_SettingsCheck";

    /// <summary>A value of a field (a list too) as the descriptor spells it; null when it is not one of the field's type.</summary>
    public static JsonNode? Spell(FieldSchema f, object? v)
    {
        if (v is null) return null;
        if (!f.Repeated) return One(f.Kind, v);
        if (v is string || v is not System.Collections.IEnumerable items) return null;
        var array = new JsonArray();
        foreach (var x in items)
        {
            if (x is null || One(f.Kind, x) is not { } n) return null;
            array.Add(n);
        }
        return array;
    }

    /// <summary>One value as the descriptor spells it (numbers as numbers; decimals and dates as text).</summary>
    private static JsonNode? One(FieldKind kind, object v) => kind switch
    {
        FieldKind.Int32 or FieldKind.Int64 => v switch
        {
            int or long or short or byte or sbyte or ushort or uint => JsonValue.Create(Convert.ToInt64(v, CultureInfo.InvariantCulture)),
            _ => null,
        },
        FieldKind.Double => v switch
        {
            int or long or short or byte or sbyte or ushort or uint => JsonValue.Create(Convert.ToInt64(v, CultureInfo.InvariantCulture)),
            double d when double.IsFinite(d) => d == Math.Floor(d) && Math.Abs(d) < 1e15 ? JsonValue.Create((long)d) : JsonValue.Create(d),
            float x when float.IsFinite(x) => JsonValue.Create((double)x),
            _ => null,
        },
        FieldKind.Decimal => v switch
        {
            decimal m => JsonValue.Create(m.ToString(CultureInfo.InvariantCulture)),
            string s => JsonValue.Create(s),
            int or long => JsonValue.Create(Convert.ToString(v, CultureInfo.InvariantCulture)),
            double d when double.IsFinite(d) => JsonValue.Create(d.ToString("R", CultureInfo.InvariantCulture)),
            _ => null,
        },
        FieldKind.Date => v switch
        {
            DateOnly d => JsonValue.Create(d.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture)),
            string s => JsonValue.Create(s),
            _ => null,
        },
        FieldKind.Bool => v is bool b ? JsonValue.Create(b) : null,
        FieldKind.Enum => v is Enum e ? JsonValue.Create(Names.Snake(e.ToString())) : v is string s ? JsonValue.Create(s) : null,
        FieldKind.Ref => v is string s ? JsonValue.Create(s) : v.GetType().IsGenericType && v.GetType().GetGenericTypeDefinition() == typeof(Id<>)
            ? JsonValue.Create(v.ToString()) : null,
        FieldKind.String => v is string s ? JsonValue.Create(s) : null,
        _ => null,
    };

    /// <summary>A bound (<c>min</c>, <c>max</c>) as the descriptor spells it; as given when it is not of the field's type (refused later).</summary>
    private static JsonNode? Bound(FieldSchema f, object v) =>
        One(f.Kind, v) ?? v switch
        {
            string s => JsonValue.Create(s),
            int or long => JsonValue.Create(Convert.ToInt64(v, CultureInfo.InvariantCulture)),
            double d => JsonValue.Create(d),
            _ => JsonValue.Create(Convert.ToString(v, CultureInfo.InvariantCulture)),
        };

    /// <summary>Writes a settings field's options into its descriptor entry, each only when declared.</summary>
    public static void Keys(FieldSchema f, JsonObject field, string path, Action<string, string, string>? error)
    {
        var s = f.Setting;
        if (!s.Any) return;
        if (s.Label is not null) field["label"] = s.Label;
        if (s.HasDefault)
        {
            if (Spell(f, s.Default) is { } d) field["default"] = d;
            else error?.Invoke("E_SETTINGS_DEFAULT", $"{path}.default", $"the default is not a value of {f.DescriptorType("")}");
        }
        if (s.Section is not null) field["section"] = s.Section;
        if (s.Placeholder is not null) field["placeholder"] = s.Placeholder;
        if (s.Multiline) field["multiline"] = true;
        if (s.Min is not null) field["min"] = Bound(f, s.Min);
        if (s.Max is not null) field["max"] = Bound(f, s.Max);
        foreach (var (key, n) in new[] { ("min_len", s.MinLen), ("max_len", s.MaxLen), ("min_items", s.MinItems), ("max_items", s.MaxItems) })
        {
            if (n is not { } count) continue;
            if (count < 0) error?.Invoke("E_SETTINGS_VALIDATOR", $"{path}.{key}", $"{key} {count}: a count from 0");
            else field[key] = count;
        }
        if (s.Pattern is not null) field["pattern"] = s.Pattern;
        if (s.Format is not null) field["format"] = s.Format;
        if (s.Message is not null) field["message"] = s.Message;
    }

    // ------------------------------------------------------------------------------------------------ the form

    private enum Kind { String, Bool, Int32, Int64, Double, Decimal, Date, Secret, Enum, User }

    private static Kind? KindOf(JsonObject f, JsonArray enums)
    {
        var type = (string?)f["type"] ?? "";
        Kind? k = type switch
        {
            "string" => Kind.String,
            "bool" => Kind.Bool,
            "int32" => Kind.Int32,
            "int64" => Kind.Int64,
            "double" => Kind.Double,
            "decimal" => Kind.Decimal,
            "date" => Kind.Date,
            "secret" => Kind.Secret,
            "ref:platform.user" => Kind.User,
            _ when type.StartsWith("enum:", StringComparison.Ordinal) && EnumValues(enums, type) is not null => Kind.Enum,
            _ => null,
        };
        if (k is null) return null;
        if ((bool?)f["repeated"] == true && ((bool?)f["optional"] == true || k is not (Kind.String or Kind.Enum or Kind.User))) return null;
        return k;
    }

    private static List<string>? EnumValues(JsonArray enums, string type) =>
        enums.FirstOrDefault(e => (string?)e!["name"] == type["enum:".Length..])?["values"]?.AsArray().Select(v => (string)v!).ToList();

    private static string[] Controls(Kind k, bool repeated) => k switch
    {
        Kind.String => repeated ? ["textarea"] : ["text", "textarea"],
        Kind.Secret => ["secret"],
        Kind.Bool => ["switch", "checkbox"],
        Kind.Date => ["date"],
        Kind.Enum => repeated ? ["multi_select"] : ["select", "radio_cards"],
        Kind.User => ["member"],
        _ => ["number"],
    };

    private static bool IsRequired(JsonObject f) =>
        (bool?)f["optional"] != true && (bool?)f["repeated"] != true && f["default"] is null;

    /// <summary><c>thread_warm_hours</c> → <c>Thread warm hours</c>.</summary>
    private static string Humanize(string name) => Ui.UiJson.LabelOf(name);

    private static JsonArray EnumOptions(JsonObject f, JsonArray enums)
    {
        var type = (string?)f["type"] ?? "";
        var values = type.StartsWith("enum:", StringComparison.Ordinal) ? EnumValues(enums, type) ?? [] : [];
        return new JsonArray([.. values.Select(v => (JsonNode)new JsonObject { ["value"] = v, ["label"] = Humanize(v) })]);
    }

    /// <summary>What every input of a settings form carries from its field: <c>required</c>, and its help, placeholder, options.</summary>
    private static void Fill(JsonObject input, JsonObject f, JsonArray enums)
    {
        if (IsRequired(f)) input["required"] = true;
        else input.Remove("required");
        if (input["help"] is null && (string?)f["description"] is { Length: > 0 } help) input["help"] = help;
        if (input["placeholder"] is null && f["placeholder"] is { } placeholder) input["placeholder"] = placeholder.DeepClone();
        if ((input["options"] is null || input["options"] is JsonArray { Count: 0 })
            && (string?)input["control"] is "select" or "radio_cards" or "multi_select")
        {
            var o = EnumOptions(f, enums);
            if (o.Count > 0) input["options"] = o;
            else input.Remove("options");
        }
    }

    /// <summary>
    /// The form of a settings message the plugin lays out no form for: its fields in declaration order, in sections by
    /// their <c>section</c> (in the order each first appears; the fields without one in an untitled section, where the
    /// first of them appears).
    /// </summary>
    public static JsonArray DeriveForm(JsonArray fields, JsonArray enums)
    {
        var sections = new List<(string? Title, JsonArray Inputs)>();
        foreach (var node in fields)
        {
            var f = node!.AsObject();
            if (KindOf(f, enums) is not { } k) continue;
            var repeated = (bool?)f["repeated"] == true;
            var name = (string)f["name"]!;
            var control = (bool?)f["multiline"] == true && k == Kind.String && !repeated ? "textarea" : Controls(k, repeated)[0];
            var input = new JsonObject { ["name"] = name, ["control"] = control, ["label"] = (string?)f["label"] ?? Humanize(name) };
            Fill(input, f, enums);
            var title = (string?)f["section"];
            var at = sections.FindIndex(s => s.Title == title);
            if (at < 0)
            {
                sections.Add((title, []));
                at = sections.Count - 1;
            }
            sections[at].Inputs.Add(input);
        }
        return new JsonArray([.. sections.Select(s =>
        {
            var o = new JsonObject();
            if (s.Title is not null) o["title"] = s.Title;
            o["inputs"] = s.Inputs;
            return (JsonNode)o;
        })]);
    }

    /// <summary>A form the plugin laid out, its inputs filled from their fields as <see cref="DeriveForm"/> fills them.</summary>
    public static JsonArray CompleteForm(JsonArray sections, JsonArray fields, JsonArray enums)
    {
        foreach (var s in sections)
            foreach (var i in s!["inputs"]?.AsArray() ?? [])
                if (fields.FirstOrDefault(f => (string?)f!["name"] == (string?)i!["name"]) is JsonObject f)
                    Fill(i!.AsObject(), f, enums);
        return sections;
    }

    // ------------------------------------------------------------------------------------------------ checks

    private static readonly string[] SettingKeys =
        ["label", "default", "section", "placeholder", "multiline", "min", "max", "min_len", "max_len", "pattern", "format", "min_items", "max_items", "message"];

    private static readonly string[] NotOnSettings = ["options_from", "pick", "fields", "aside", "initial"];

    private static string Display(JsonObject f) => (bool?)f["repeated"] == true ? $"a list of {(string?)f["type"]}" : (string?)f["type"] ?? "";

    /// <summary>
    /// The definition checks of settings: the options of a setting and the secret type belong to the settings message's
    /// fields; the settings message's fields and form.
    /// </summary>
    public static void Errors(JsonArray messages, JsonArray enums, JsonObject? settings, Action<string, string, string> error)
    {
        var settingsName = (string?)settings?["message"];
        foreach (var m in messages)
        {
            var name = (string)m!["name"]!;
            if (name == settingsName) continue;
            foreach (var f in m["fields"]!.AsArray())
            {
                var path = $"messages.{name}.fields.{(string?)f!["name"]}";
                if (SettingKeys.Any(k => f[k] is not null))
                    error("E_FIELD_VALIDATOR_SCOPE", path,
                        "label, default, section, placeholder, multiline, validators and message belong to the fields of the settings message (declare it with [Message(Kind = MessageKind.Settings)] and Plugin.Settings<T>(), or check an endpoint's input in its handler)");
                if ((string?)f["type"] == "secret")
                    error("E_SETTINGS_FIELD_TYPE", path, "Secret is the type of a settings field only (keep the value in the plugin's settings, or use a string with [Encrypted] on an entity)");
            }
        }
        if (settings is null) return;
        if (messages.FirstOrDefault(m => (string?)m!["name"] == settingsName) is not JsonObject message) return;
        var fields = message["fields"]!.AsArray();
        var basePath = $"messages.{settingsName}";
        if (fields.Count > MaxFields) error("E_SETTINGS_LIMIT", basePath, $"{fields.Count} fields; settings have at most {MaxFields}");
        var secrets = fields.Count(f => (string?)f!["type"] == "secret");
        if (secrets > MaxSecrets) error("E_SETTINGS_LIMIT", basePath, $"{secrets} secrets; settings have at most {MaxSecrets}");
        foreach (var f in fields) FieldErrors(f!.AsObject(), enums, $"{basePath}.fields.{(string?)f["name"]}", error);

        // the form: every field once, each with a control its type takes
        var seen = new List<string>();
        var sections = settings["form"]?["sections"]?.AsArray() ?? [];
        for (var si = 0; si < sections.Count; si++)
        {
            var inputs = sections[si]!["inputs"]?.AsArray() ?? [];
            for (var ii = 0; ii < inputs.Count; ii++)
            {
                var i = inputs[ii]!.AsObject();
                var p = $"settings.form.sections[{si}].inputs[{ii}]";
                var name = (string?)i["name"] ?? "";
                if (fields.FirstOrDefault(x => (string?)x!["name"] == name) is not JsonObject f)
                {
                    error("E_SETTINGS_FORM_FIELD", p, $"{settingsName} has no field {name}");
                    continue;
                }
                if (seen.Contains(name)) error("E_SETTINGS_FORM_FIELD", p, $"{name} is in the form twice");
                seen.Add(name);
                if (NotOnSettings.Any(k => i[k] is not null))
                    error("E_SETTINGS_CONTROL", p, "a settings input takes no options_from, pick, fields, aside or initial (a field's default is its initial value)");
                if (KindOf(f, enums) is not { } k) continue;
                var allowed = Controls(k, (bool?)f["repeated"] == true);
                var control = (string?)i["control"] ?? "";
                if (!allowed.Contains(control))
                    error("E_SETTINGS_CONTROL", $"{p}.control", $"{name} ({Display(f)}) is filled with {string.Join(" or ", allowed)}, not {control}");
                var options = i["options"] as JsonArray ?? [];
                if (k == Kind.Enum)
                {
                    var values = EnumValues(enums, (string)f["type"]!) ?? [];
                    foreach (var o in options)
                        if (o?["value"] is not JsonValue v || !v.TryGetValue<string>(out var s) || !values.Contains(s))
                            error("E_SETTINGS_CONTROL", $"{p}.options", $"option {o?["value"]?.ToJsonString()} is not a value of {(string?)f["type"]}");
                }
                else if (options.Count > 0)
                    error("E_SETTINGS_CONTROL", $"{p}.options", $"{name} is not an enum: its input takes no options");
            }
        }
        foreach (var f in fields)
            if (!seen.Contains((string)f!["name"]!))
                error("E_SETTINGS_FORM_FIELD", "settings.form", $"{(string?)f["name"]} is not in the form (a settings form has every field of the settings message once)");
    }

    private static bool BoundOk(Kind k, JsonNode b)
    {
        if (b is not JsonValue v) return false;
        return k switch
        {
            Kind.Date => v.TryGetValue<string>(out var s) && DateOnly.TryParseExact(s, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out _),
            Kind.Decimal => v.TryGetValue<string>(out var s) && decimal.TryParse(s, NumberStyles.Number, CultureInfo.InvariantCulture, out _),
            Kind.Int32 or Kind.Int64 => v.GetValueKind() == System.Text.Json.JsonValueKind.Number && long.TryParse(v.ToJsonString(), NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out _),
            _ => v.GetValueKind() == System.Text.Json.JsonValueKind.Number,
        };
    }

    private static int? Compare(Kind k, JsonNode a, JsonNode b)
    {
        try
        {
            return k switch
            {
                Kind.Date => string.CompareOrdinal((string?)a, (string?)b),
                Kind.Decimal => decimal.Parse((string)a!, CultureInfo.InvariantCulture).CompareTo(decimal.Parse((string)b!, CultureInfo.InvariantCulture)),
                _ => double.Parse(a.ToJsonString(), CultureInfo.InvariantCulture).CompareTo(double.Parse(b.ToJsonString(), CultureInfo.InvariantCulture)),
            };
        }
        catch (Exception)
        {
            return null;
        }
    }

    /// <summary>A settings field's own checks: its type, its validators against its type, its default, its pattern.</summary>
    private static void FieldErrors(JsonObject f, JsonArray enums, string path, Action<string, string, string> error)
    {
        var name = (string?)f["name"];
        if (KindOf(f, enums) is not { } k)
        {
            error("E_SETTINGS_FIELD_TYPE", path,
                $"{name} is {Display(f)}: settings take string, bool, int32, int64, double, decimal, date, secret, an enum or a member, optional or not, and lists of strings, enums or members");
            return;
        }
        void Bad(string what, string why) => error("E_SETTINGS_VALIDATOR", $"{path}.{what}", why);
        var list = (bool?)f["repeated"] == true;
        var numeric = k is Kind.Int32 or Kind.Int64 or Kind.Double or Kind.Decimal or Kind.Date;
        if ((f["min"] is not null || f["max"] is not null) && !(numeric && !list))
            Bad("min", $"min and max bound a number, a decimal or a date; {name} is not one");
        else if (numeric)
        {
            foreach (var what in new[] { "min", "max" })
                if (f[what] is { } b && !BoundOk(k, b)) Bad(what, $"{what} {b.ToJsonString()} is not a value of {(string?)f["type"]}");
            if (f["min"] is { } lo && f["max"] is { } hi && BoundOk(k, lo) && BoundOk(k, hi) && Compare(k, lo, hi) > 0)
                Bad("min", $"min {lo.ToJsonString()} is above max {hi.ToJsonString()}");
        }
        var stringy = k is Kind.String or Kind.Secret;
        if ((f["min_len"] is not null || f["max_len"] is not null) && !stringy)
            Bad(f["min_len"] is not null ? "min_len" : "max_len", $"min_len and max_len bound a string or a secret; {name} is not one");
        if (f["min_len"] is { } a && f["max_len"] is { } z && (int)a > (int)z) Bad("min_len", $"min_len {a} is above max_len {z}");
        if (f["pattern"] is not null && k != Kind.String) Bad("pattern", $"a pattern checks a string; {name} is not one");
        else if ((string?)f["pattern"] is { } pattern)
        {
            try { _ = new Regex(pattern); }
            catch (ArgumentException e)
            {
                error("E_SETTINGS_PATTERN", $"{path}.pattern", $"\"{pattern}\": {e.Message} (use literals, classes, \\d, \\w, groups, alternation and repetition; anchor with ^ and $)");
            }
        }
        if (f["format"] is not null && k != Kind.String) Bad("format", $"url and email check a string; {name} is not one");
        if ((f["min_items"] is not null || f["max_items"] is not null) && !list)
            Bad("min_items", $"min_items and max_items bound a list; {name} is not one");
        if (f["min_items"] is { } ai && f["max_items"] is { } zi && (int)ai > (int)zi) Bad("min_items", $"min_items {ai} is above max_items {zi}");
        if ((bool?)f["multiline"] == true && !(k == Kind.String && !list)) Bad("multiline", $"multiline belongs to a string; {name} is not one");
        if (f["default"] is { } d)
        {
            if (k == Kind.Secret) error("E_SETTINGS_DEFAULT", $"{path}.default", "a secret has no default: an owner or an admin sets it");
            else if (d.ToJsonString().Length > MaxValueBytes) error("E_SETTINGS_LIMIT", $"{path}.default", "the default is larger than 64 KiB");
        }
    }
}
