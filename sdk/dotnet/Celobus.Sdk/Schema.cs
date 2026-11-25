// The shape of messages, read by reflection: field order, names, descriptor types. Shared by the descriptor builder
// and the JSON codec.
using System.Collections.Concurrent;
using System.ComponentModel;
using System.Reflection;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Celobus.Sdk;

internal static class Names
{
    /// <summary>PascalCase → snake_case (<c>OwnerId</c> → <c>owner_id</c>, <c>HTTPStatus</c> → <c>http_status</c>, <c>Line2Total</c> → <c>line2_total</c>).</summary>
    public static string Snake(string s)
    {
        var sb = new StringBuilder(s.Length + 4);
        for (var i = 0; i < s.Length; i++)
        {
            var c = s[i];
            if (char.IsUpper(c))
            {
                if (i > 0 && s[i - 1] != '_' &&
                    (char.IsLower(s[i - 1]) || char.IsDigit(s[i - 1]) ||
                     (char.IsUpper(s[i - 1]) && i + 1 < s.Length && char.IsLower(s[i + 1]))))
                    sb.Append('_');
                sb.Append(char.ToLowerInvariant(c));
            }
            else sb.Append(c);
        }
        return sb.ToString();
    }

    /// <summary>snake_case → lowerCamelCase, as protobuf derives a field's JSON name.</summary>
    public static string Camel(string snake)
    {
        var sb = new StringBuilder(snake.Length);
        var upper = false;
        foreach (var c in snake)
        {
            if (c == '_') { upper = true; continue; }
            sb.Append(upper ? char.ToUpperInvariant(c) : c);
            upper = false;
        }
        return sb.ToString();
    }

    /// <summary>A plugin id → its service name (<c>orders</c> → <c>Orders</c>).</summary>
    public static string Service(string id) =>
        string.Concat(id.Split('_', StringSplitOptions.RemoveEmptyEntries).Select(p => char.ToUpperInvariant(p[0]) + p[1..]));

    /// <summary>The path of an endpoint: <c>/&lt;id&gt;.v1.&lt;Service&gt;/&lt;Name&gt;</c>.</summary>
    public static string Path(string plugin, string endpoint) => $"/{plugin}.v1.{Service(plugin)}/{endpoint}";

    /// <summary>A plugin id → a default title (<c>orders</c> → <c>Orders</c>).</summary>
    public static string Humanize(string id) => id.Length == 0 ? id : char.ToUpperInvariant(id[0]) + id[1..].Replace('_', ' ');
}

/// <summary>The descriptor type of a field (or of a repeated field's element).</summary>
internal enum FieldKind
{
    String, Bool, Int32, Int64, Double, Decimal, Uuid, Timestamp, Date, Bytes, Json, Message, Enum, Ref, Invalid,
}

internal enum CollectionKind { None, Array, List }

internal sealed class FieldSchema
{
    public required string Name { get; init; }          // snake_case
    public required string JsonName { get; init; }      // lowerCamelCase
    public required int Number { get; init; }
    public required PropertyInfo Property { get; init; }
    public required Type ClrType { get; init; }          // the property's type
    public required Type ElementType { get; init; }      // without Nullable<> and the collection
    public required FieldKind Kind { get; init; }
    public required bool Repeated { get; init; }
    public required bool Optional { get; init; }
    public required CollectionKind Collection { get; init; }
    public required string Description { get; init; }
    public int CtorIndex { get; init; } = -1;
    public string? Error { get; init; }                  // E_FIELD_TYPE message
    public bool Unique { get; init; }                    // [Unique] on an entity's column
    public bool Indexed { get; init; }                   // [Index] on an entity's column
    public bool Encrypted { get; init; }                 // [Encrypted] on an entity's column
    public string? Searchable { get; init; }             // [Searchable(lang)] on an entity's column: the language
    public string? Nav { get; init; }                    // [Ref(Nav = ..)]
    public string? Inverse { get; init; }                // [Ref(Inverse = ..)]
    public string? OnDelete { get; init; }               // [Ref(OnDelete = ..)], snake_case
    public bool History { get; init; }                   // [History] on an entity's column

    /// <summary>The descriptor type; refs resolve against the plugin id.</summary>
    public string DescriptorType(string pluginId) => Kind switch
    {
        FieldKind.String => "string",
        FieldKind.Bool => "bool",
        FieldKind.Int32 => "int32",
        FieldKind.Int64 => "int64",
        FieldKind.Double => "double",
        FieldKind.Decimal => "decimal",
        FieldKind.Uuid => "uuid",
        FieldKind.Timestamp => "timestamp",
        FieldKind.Date => "date",
        FieldKind.Bytes => "bytes",
        FieldKind.Json => "json",
        FieldKind.Message => $"message:{ElementType.Name}",
        FieldKind.Enum => $"enum:{ElementType.Name}",
        FieldKind.Ref => $"ref:{Schema.RefTarget(ElementType.GetGenericArguments()[0], pluginId)}",
        _ => $"?{ElementType.Name}",
    };

    /// <summary>The value a field takes when the JSON leaves it out (protobuf's defaults; null when optional).</summary>
    public object? Default()
    {
        if (Optional) return null;
        if (Repeated)
            return Collection == CollectionKind.Array
                ? Array.CreateInstance(ElementType, 0)
                : Activator.CreateInstance(typeof(List<>).MakeGenericType(ElementType));
        return Kind switch
        {
            FieldKind.String => "",
            FieldKind.Bytes => Array.Empty<byte>(),
            FieldKind.Timestamp => DateTimeOffset.UnixEpoch,
            FieldKind.Message => JsonSerializer.Deserialize("{}", ElementType, CelobusJson.Options),
            _ => ClrType.IsValueType ? Activator.CreateInstance(ClrType) : null,
        };
    }
}

/// <summary>A message (record or class with <c>[Message]</c> or <c>[Entity]</c>): its fields and how to build it.</summary>
internal sealed class MessageSchema
{
    private static readonly ConcurrentDictionary<Type, MessageSchema> Cache = new();

    public required Type Type { get; init; }
    public required string Name { get; init; }
    public required MessageKind Kind { get; init; }
    public required string Description { get; init; }
    public required IReadOnlyList<FieldSchema> Fields { get; init; }
    public ConstructorInfo? Constructor { get; init; }
    public EntityAttribute? Entity { get; init; }

    /// <summary><c>[OmitDefaults]</c> on a message (not an entity): fields at their default are left out of the JSON.</summary>
    public bool OmitDefaults { get; init; }

    public Dictionary<string, FieldSchema> ByJsonName { get; } = new(StringComparer.Ordinal);

    public static MessageSchema For(Type t) => Cache.GetOrAdd(t, Build);

    private static MessageSchema Build(Type t)
    {
        var entity = t.GetCustomAttribute<EntityAttribute>();
        var message = t.GetCustomAttribute<MessageAttribute>();
        var props = t.GetProperties(BindingFlags.Public | BindingFlags.Instance)
            .Where(p => p.GetMethod is { IsPublic: true } && p.GetIndexParameters().Length == 0)
            .ToList();

        // A primary constructor (or any public one whose parameters are all properties): its order is the field order.
        var ctor = t.GetConstructors(BindingFlags.Public | BindingFlags.Instance)
            .Where(c => c.GetParameters().Length > 0 && c.GetParameters().All(p =>
                props.Any(pr => pr.Name == p.Name && pr.PropertyType == p.ParameterType)))
            .OrderByDescending(c => c.GetParameters().Length)
            .FirstOrDefault();
        var parameters = ctor?.GetParameters() ?? [];

        var ordered = new List<(PropertyInfo Prop, ParameterInfo? Param, int Index)>();
        for (var i = 0; i < parameters.Length; i++)
            ordered.Add((props.First(p => p.Name == parameters[i].Name), parameters[i], i));
        // Other properties that can be set, in declaration order (base types first).
        ordered.AddRange(props
            .Where(p => parameters.All(pa => pa.Name != p.Name) && p.SetMethod is { IsPublic: true })
            .OrderBy(p => Depth(p.DeclaringType!))
            .ThenBy(p => p.MetadataToken)
            .Select(p => (p, (ParameterInfo?)null, -1)));

        var nullability = new NullabilityInfoContext(); // not thread-safe: one per build
        var fields = new List<FieldSchema>();
        foreach (var (prop, param, index) in ordered)
            fields.Add(Field(nullability, prop, param, index, fields.Count + 1));

        var schema = new MessageSchema
        {
            Type = t,
            Name = t.Name,
            Kind = message?.Kind ?? MessageKind.Message,
            Description = DescriptionOf(t),
            Fields = fields,
            Constructor = ctor,
            Entity = entity,
            OmitDefaults = entity is null && t.GetCustomAttribute<OmitDefaultsAttribute>() is not null,
        };
        foreach (var f in fields)
        {
            schema.ByJsonName[f.JsonName] = f;
            schema.ByJsonName.TryAdd(f.Name, f);
        }
        return schema;
    }

    private static int Depth(Type t)
    {
        var d = 0;
        for (var b = t.BaseType; b != null; b = b.BaseType) d++;
        return d;
    }

    private static FieldSchema Field(NullabilityInfoContext nullability, PropertyInfo prop, ParameterInfo? param, int ctorIndex, int number)
    {
        var name = Names.Snake(prop.Name);
        var type = prop.PropertyType;
        var optional = false;
        var element = type;
        var collection = CollectionKind.None;
        string? error = null;

        if (Nullable.GetUnderlyingType(type) is { } inner)
        {
            optional = true;
            element = inner;
        }
        else if (!type.IsValueType && nullability.Create(prop).ReadState == NullabilityState.Nullable)
        {
            optional = true;
        }

        if (element != typeof(byte[]) && element != typeof(string) && Schema.ElementOf(element) is { } coll)
        {
            (element, collection) = coll;
            optional = false; // a repeated field has no presence
            if (Nullable.GetUnderlyingType(element) != null)
                error = "a repeated field's element cannot be nullable";
        }

        var fieldKind = Schema.KindOf(element);
        if (fieldKind == FieldKind.Invalid)
            error ??= $"`{Schema.Display(type)}` is not a field type (a scalar, Id<T>, an enum, a [Message] type or a list of them)";

        return new FieldSchema
        {
            Name = name,
            JsonName = Names.Camel(name),
            Number = number,
            Property = prop,
            ClrType = type,
            ElementType = element,
            Kind = fieldKind,
            Repeated = collection != CollectionKind.None,
            Optional = optional,
            Collection = collection,
            Description = DescriptionOf(prop) is { Length: > 0 } d ? d : param is null ? "" : DescriptionOf(param),
            CtorIndex = ctorIndex,
            Error = error,
            Unique = prop.IsDefined(typeof(UniqueAttribute)) || param?.IsDefined(typeof(UniqueAttribute)) == true,
            Indexed = prop.IsDefined(typeof(IndexAttribute)) || param?.IsDefined(typeof(IndexAttribute)) == true,
            Encrypted = prop.IsDefined(typeof(EncryptedAttribute)) || param?.IsDefined(typeof(EncryptedAttribute)) == true,
            History = prop.IsDefined(typeof(HistoryAttribute)) || param?.IsDefined(typeof(HistoryAttribute)) == true,
            Searchable = (prop.GetCustomAttribute<SearchableAttribute>() ?? param?.GetCustomAttribute<SearchableAttribute>())?.Language,
            Nav = (prop.GetCustomAttribute<RefAttribute>() ?? param?.GetCustomAttribute<RefAttribute>())?.Nav,
            Inverse = (prop.GetCustomAttribute<RefAttribute>() ?? param?.GetCustomAttribute<RefAttribute>())?.Inverse,
            OnDelete = (prop.GetCustomAttribute<RefAttribute>() ?? param?.GetCustomAttribute<RefAttribute>())?.OnDelete is { } od
                && od != Sdk.OnDelete.Default
                ? Names.Snake(od.ToString())
                : null,
        };
    }

    internal static string DescriptionOf(ICustomAttributeProvider p)
    {
        var attrs = p.GetCustomAttributes(false);
        foreach (var a in attrs)
            switch (a)
            {
                case DescribeAttribute d: return d.Text;
                case DescriptionAttribute d: return d.Description;
            }
        return "";
    }

    /// <summary>Builds an instance from field values (by field index); missing values are the fields' defaults.</summary>
    public object Create(object?[] values, bool[] present)
    {
        for (var i = 0; i < Fields.Count; i++)
            if (!present[i]) values[i] = Fields[i].Default();

        object instance;
        if (Constructor != null)
        {
            var args = new object?[Constructor.GetParameters().Length];
            for (var i = 0; i < Fields.Count; i++)
                if (Fields[i].CtorIndex >= 0) args[Fields[i].CtorIndex] = values[i];
            instance = Constructor.Invoke(args);
        }
        else
        {
            instance = Activator.CreateInstance(Type, nonPublic: true)
                       ?? throw new InvalidOperationException($"cannot create {Type}");
        }
        for (var i = 0; i < Fields.Count; i++)
            if (Fields[i].CtorIndex < 0 && (present[i] || values[i] != null))
                Fields[i].Property.SetValue(instance, values[i]);
        return instance;
    }
}

internal static partial class Schema
{
    public static bool IsMessage(Type t) =>
        t.IsClass && (t.IsDefined(typeof(MessageAttribute), false) || t.IsDefined(typeof(EntityAttribute), false)
                      || t.IsDefined(typeof(ViewAttribute), false) || t.IsDefined(typeof(LakeTableAttribute), false));

    /// <summary>The row of an analytic view of the plugin (<c>[View]</c>).</summary>
    public static bool IsView(Type t) => t.IsClass && t.IsDefined(typeof(ViewAttribute), false);

    /// <summary>A view's name: <c>[View(Name = ..)]</c>, or the type's name in snake_case.</summary>
    public static string ViewName(Type t) => t.GetCustomAttribute<ViewAttribute>(false)?.Name ?? Names.Snake(t.Name);

    /// <summary>An entity of this plugin (<c>[Entity]</c> without <c>Plugin</c>).</summary>
    public static bool IsEntity(Type t) => t.IsClass && t.GetCustomAttribute<EntityAttribute>(false) is { Plugin: null };

    /// <summary>An entity whose rows are kept in the analytic copy only (<c>[Entity(Storage = EntityStorage.Olap)]</c>).</summary>
    public static bool IsOlap(Type t) => t.IsClass && t.GetCustomAttribute<EntityAttribute>(false) is { Storage: EntityStorage.Olap };

    /// <summary>A dependency's entity (<c>[Entity(Plugin = "customers")]</c>), read with <c>ctx.Query&lt;T&gt;()</c>.</summary>
    public static bool IsForeignEntity(Type t) => ForeignPlugin(t) is not null;

    /// <summary>The provider of a dependency's entity; null for any other type.</summary>
    public static string? ForeignPlugin(Type t) => t.IsClass ? t.GetCustomAttribute<EntityAttribute>(false)?.Plugin : null;

    public static FieldKind KindOf(Type t)
    {
        if (t == typeof(string)) return FieldKind.String;
        if (t == typeof(bool)) return FieldKind.Bool;
        if (t == typeof(int)) return FieldKind.Int32;
        if (t == typeof(long)) return FieldKind.Int64;
        if (t == typeof(double)) return FieldKind.Double;
        if (t == typeof(decimal)) return FieldKind.Decimal;
        if (t == typeof(Guid)) return FieldKind.Uuid;
        if (t == typeof(DateTimeOffset)) return FieldKind.Timestamp;
        if (t == typeof(DateOnly)) return FieldKind.Date;
        if (t == typeof(byte[])) return FieldKind.Bytes;
        if (t == typeof(JsonElement) || t == typeof(JsonNode) || t == typeof(JsonObject)) return FieldKind.Json;
        if (t.IsEnum) return FieldKind.Enum;
        if (t.IsGenericType && t.GetGenericTypeDefinition() == typeof(Id<>)) return FieldKind.Ref;
        if (IsMessage(t)) return FieldKind.Message;
        return FieldKind.Invalid;
    }

    /// <summary>The element type of a collection field (<c>T[]</c>, <c>IReadOnlyList&lt;T&gt;</c>, <c>List&lt;T&gt;</c>, …).</summary>
    public static (Type, CollectionKind)? ElementOf(Type t)
    {
        if (t.IsArray && t.GetArrayRank() == 1) return (t.GetElementType()!, CollectionKind.Array);
        if (!t.IsGenericType) return null;
        var def = t.GetGenericTypeDefinition();
        if (def == typeof(List<>) || def == typeof(IReadOnlyList<>) || def == typeof(IList<>) ||
            def == typeof(IEnumerable<>) || def == typeof(IReadOnlyCollection<>) || def == typeof(ICollection<>))
            return (t.GetGenericArguments()[0], CollectionKind.List);
        return null;
    }

    /// <summary>
    /// The ref target of <c>Id&lt;T&gt;</c>: <c>platform.user</c>, <c>&lt;plugin&gt;.&lt;entity&gt;</c> for the plugin's own
    /// entity, <c>&lt;provider&gt;.&lt;entity&gt;</c> for a dependency's.
    /// </summary>
    public static string RefTarget(Type target, string pluginId) =>
        target.GetCustomAttribute<RefTargetAttribute>()?.Target
        ?? $"{ForeignPlugin(target) ?? pluginId}.{Names.Snake(target.Name)}";

    public static string Display(Type t)
    {
        if (Nullable.GetUnderlyingType(t) is { } u) return Display(u) + "?";
        if (!t.IsGenericType) return t.Name;
        var name = t.Name[..t.Name.IndexOf('`')];
        return $"{name}<{string.Join(", ", t.GetGenericArguments().Select(Display))}>";
    }

    /// <summary>An enum's value names: its members in declaration order, snake_case.</summary>
    public static IReadOnlyList<string> EnumValues(Type t) =>
        t.GetFields(BindingFlags.Public | BindingFlags.Static).OrderBy(f => f.MetadataToken).Select(f => Names.Snake(f.Name)).ToList();
}
