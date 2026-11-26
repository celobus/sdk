// UI declarations: a plugin's sidebar, screens, overlays, home widgets and
// commands, built from the platform's catalog of primitives and bound to the plugin's own endpoints and topics. It is
// data in the descriptor (`ui`): every client renders it with its own components, and every read and action is the
// member's own call. Nothing here styles anything: tones, emphasis and density are meanings.
//
// Field references are lambdas over the message types (`o => o.Status`), resolved to the descriptor's field names
// (the snake_case property names), so a column, an argument or a condition over the wrong message or type does not
// compile. The document is the same JSON the Rust SDK writes (crates/descriptor/src/ui.rs).
using System.Linq.Expressions;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Celobus.Sdk.Ui;

/// <summary>A tone: a meaning (never a color) of a status, a toast, a notice, a row.</summary>
public enum Tone
{
    Neutral,
    Ok,
    Attention,
    Danger,
}

/// <summary>An overlay's size.</summary>
public enum Size
{
    Small,
    Medium,
    Large,
}

/// <summary>The gap between the children of a stack or a row.</summary>
public enum Gap
{
    Tight,
    Normal,
    Loose,
}

/// <summary>Collects what writing the document finds (the catalog texts of <see cref="Ui.Text(string, string)"/>).</summary>
internal sealed class UiWriter
{
    public Dictionary<string, string> Texts { get; } = new(StringComparer.Ordinal);
}

/// <summary>A piece of the document that writes itself as JSON (fresh nodes on every call).</summary>
internal interface IUiJson
{
    JsonNode ToJson(UiWriter w);
}

/// <summary>A JSON object under construction whose values may be builders (written when the document is).</summary>
internal sealed class UiObj : IUiJson
{
    private readonly List<(string Key, object Value, bool OmitEmpty)> _entries = [];

    /// <summary>Sets a member; null removes it.</summary>
    public object? this[string key]
    {
        get => _entries.FirstOrDefault(e => e.Key == key).Value;
        set => Put(key, value, false);
    }

    /// <summary>A list member, left out of the JSON while empty when <paramref name="omitEmpty"/>.</summary>
    public List<object> List(string key, bool omitEmpty = false)
    {
        var list = new List<object>();
        Put(key, list, omitEmpty);
        return list;
    }

    private void Put(string key, object? value, bool omitEmpty)
    {
        var i = _entries.FindIndex(e => e.Key == key);
        if (value is null)
        {
            if (i >= 0) _entries.RemoveAt(i);
            return;
        }
        if (i >= 0) _entries[i] = (key, value, omitEmpty);
        else _entries.Add((key, value, omitEmpty));
    }

    /// <summary>A deep copy of the builders under it (objects and lists; texts, conditions and values are immutable).</summary>
    public UiObj Clone()
    {
        var c = new UiObj();
        foreach (var (key, value, omitEmpty) in _entries) c._entries.Add((key, CloneValue(value), omitEmpty));
        return c;
    }

    private static object CloneValue(object v) => v switch
    {
        UiObj o => o.Clone(),
        List<object> l => l.Select(CloneValue).ToList(),
        _ => v,
    };

    public JsonNode ToJson(UiWriter w)
    {
        var o = new JsonObject();
        foreach (var (key, value, omitEmpty) in _entries)
        {
            if (omitEmpty && value is System.Collections.ICollection { Count: 0 }) continue;
            o[key] = UiJson.Of(value, w);
        }
        return o;
    }

    /// <summary><c>{"&lt;kind&gt;": {}}</c> and its body.</summary>
    public static (UiObj Outer, UiObj Body) Tagged(string kind)
    {
        var body = new UiObj();
        var outer = new UiObj { [kind] = body };
        return (outer, body);
    }
}

internal static class UiJson
{
    public static JsonNode? Of(object? v, UiWriter w) => v switch
    {
        null => null,
        IUiJson u => u.ToJson(w),
        JsonNode n => n.DeepClone(),
        string s => JsonValue.Create(s),
        bool b => JsonValue.Create(b),
        int i => JsonValue.Create(i),
        long l => JsonValue.Create(l),
        System.Collections.IEnumerable e => new JsonArray([.. e.Cast<object?>().Select(x => Of(x, w))]),
        _ => throw new InvalidOperationException($"{v.GetType()} is not a UI document value"),
    };

    /// <summary><c>Tone.Attention</c> → <c>attention</c>.</summary>
    public static string Name<TEnum>(TEnum value) where TEnum : struct, Enum => Names.Snake(value.ToString());

    /// <summary>A literal as the wire carries it (int64 and decimal as strings, enums by value name, …).</summary>
    public static JsonNode? Literal<T>(T value) =>
        value is null ? null : JsonSerializer.SerializeToNode(value, typeof(T), CelobusJson.Options);

    /// <summary><c>{"&lt;kind&gt;": value}</c>.</summary>
    public static JsonObject One(string kind, JsonNode? value) => new() { [kind] = value };

    /// <summary>A map of enum values to tones.</summary>
    public static UiObj Tones<TEnum>(IEnumerable<(TEnum Value, Tone Tone)> tones) where TEnum : struct, Enum
    {
        var o = new UiObj();
        foreach (var (value, tone) in tones) o[Names.Snake(value.ToString())] = Name(tone);
        return o;
    }

    /// <summary>The descriptor type of a param or state value (<c>string</c>, <c>enum:OrderStatus</c>, …).</summary>
    public static string TypeName(Type t)
    {
        var element = Nullable.GetUnderlyingType(t) ?? t;
        if (element != typeof(string) && element != typeof(byte[]) && Schema.ElementOf(element) is { } coll)
            element = coll.Item1;
        return Schema.KindOf(element) switch
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
            FieldKind.File => "file",
            FieldKind.Secret => "secret",
            FieldKind.Message => $"message:{element.Name}",
            FieldKind.Enum => $"enum:{element.Name}",
            FieldKind.Ref => $"ref:{Schema.RefTarget(element.GetGenericArguments()[0], "")}",
            _ => throw new ArgumentException($"`{Schema.Display(t)}` is not a field type"),
        };
    }

    /// <summary>The options a select gets by itself: an enum's values with their labels (<c>in_progress</c> → <c>In progress</c>).</summary>
    public static List<object> EnumOptions(Type t)
    {
        var element = Nullable.GetUnderlyingType(t) ?? t;
        if (!element.IsEnum) return [];
        return Schema.EnumValues(element)
            .Select(v => (object)new UiObj { ["value"] = v, ["label"] = LabelOf(v) })
            .ToList();
    }

    /// <summary><c>in_progress</c> → <c>In progress</c>.</summary>
    public static string LabelOf(string v)
    {
        var s = v.Replace('_', ' ');
        return s.Length == 0 ? s : char.ToUpperInvariant(s[0]) + s[1..];
    }
}

/// <summary>
/// Field lambdas (<c>o =&gt; o.Customer.Name</c>) → descriptor field paths (<c>customer.name</c>); a path made by
/// <see cref="Path{TRow, T}"/> (a field of a query's join, <see cref="Via{TRow, TJoin}.F{T}"/>) is its own.
/// </summary>
internal static class FieldPath
{
    private static readonly System.Reflection.MethodInfo AtMethod = typeof(FieldPath).GetMethod(nameof(At))!;

    /// <summary>The marker a path lambda calls (never run).</summary>
    public static T At<T>(string path) => throw new InvalidOperationException($"`{path}` is a path, not a value");

    /// <summary>A lambda over <typeparamref name="TRow"/> that stands for the path (<c>who.name</c>).</summary>
    public static Expression<Func<TRow, T>> Path<TRow, T>(string path) =>
        Expression.Lambda<Func<TRow, T>>(
            Expression.Call(AtMethod.MakeGenericMethod(typeof(T)), Expression.Constant(path)),
            Expression.Parameter(typeof(TRow), "r"));

    public static string Of(LambdaExpression e)
    {
        var parts = new List<string>();
        var body = Strip(e.Body);
        if (body is MethodCallExpression { Method.IsGenericMethod: true } call
            && call.Method.GetGenericMethodDefinition() == AtMethod
            && call.Arguments[0] is ConstantExpression { Value: string path })
            return path;
        while (body is MemberExpression { Member: PropertyInfo or FieldInfo } m)
        {
            // `x.Value` of a Nullable<T> is the same field
            if (Nullable.GetUnderlyingType(m.Member.DeclaringType!) is null) parts.Add(Names.Snake(m.Member.Name));
            body = Strip(m.Expression!);
        }
        if (body != e.Parameters[0] || parts.Count == 0)
            throw new ArgumentException($"`{e}` must name a field of the message (o => o.Status, o => o.Customer.Name)");
        parts.Reverse();
        return string.Join('.', parts);
    }

    private static Expression Strip(Expression x)
    {
        while (x is UnaryExpression { NodeType: ExpressionType.Convert or ExpressionType.ConvertChecked or ExpressionType.TypeAs } u)
            x = u.Operand;
        return x;
    }
}

/// <summary>
/// A text: a literal (<c>"Orders"</c>, by an implicit conversion), a catalog key with its English text
/// (<see cref="Ui.Text(string, string)"/>), or a value shown as text (a <see cref="Val{T}"/>, by an implicit conversion).
/// </summary>
public sealed class Txt : IUiJson
{
    private readonly string? _plain;
    private readonly string? _key;
    private readonly string? _en;
    private readonly JsonObject? _value;
    private readonly SortedDictionary<string, JsonNode> _args;

    private Txt(string? plain, string? key, string? en, JsonObject? value, SortedDictionary<string, JsonNode>? args = null)
    {
        _plain = plain;
        _key = key;
        _en = en;
        _value = value;
        _args = args ?? new(StringComparer.Ordinal);
    }

    public static implicit operator Txt(string text) => new(text, null, null, null);

    internal static Txt Catalog(string key, string en) => new(null, key, en, null);

    internal static Txt Of(JsonObject value) => new(null, null, null, value);

    /// <summary>
    /// An argument of a catalog text (<c>Ui.Text("orders.cancel", "Cancel order {number}?").Arg("number", r.Get(o =&gt; o.Number))</c>);
    /// a literal or a value text has none.
    /// </summary>
    public Txt Arg<T>(string name, Val<T> value)
    {
        if (_key is null) return this;
        var args = new SortedDictionary<string, JsonNode>(_args, StringComparer.Ordinal) { [name] = Val<T>.Json(value) };
        return new Txt(null, _key, _en, null, args);
    }

    JsonNode IUiJson.ToJson(UiWriter w)
    {
        if (_key is not null)
        {
            w.Texts[_key] = _en!;
            var o = new JsonObject { ["key"] = _key };
            if (_args.Count > 0)
            {
                var a = new JsonObject();
                foreach (var (k, v) in _args) a[k] = v.DeepClone();
                o["args"] = a;
            }
            return o;
        }
        return _value?.DeepClone() ?? JsonValue.Create(_plain ?? "");
    }
}

/// <summary>
/// A <see cref="Val{T}"/> (the only implementation): an optional field takes a value of its inner type through it, while
/// a literal for it converts to a <see cref="Val{T}"/> of the field's own type.
/// </summary>
public interface IVal<T>;

/// <summary>
/// A value of type <typeparamref name="T"/> from somewhere: a literal (any <typeparamref name="T"/> converts to one),
/// a param, state, a query field, a row field, a form field, the caller, a call result.
/// </summary>
public class Val<T> : IUiJson, IVal<T>
{
    internal JsonObject Node { get; }

    internal Val(JsonObject node) { Node = node; }

    internal Val(string kind, JsonNode? value) : this(UiJson.One(kind, value)) { }

    /// <summary>A literal.</summary>
    public static implicit operator Val<T>(T value) => new("lit", UiJson.Literal(value));

    /// <summary>The value shown as text.</summary>
    public static implicit operator Txt(Val<T> value) => Txt.Of(value.Node);

    JsonNode IUiJson.ToJson(UiWriter w) => Node.DeepClone();

    /// <summary>The value's JSON (a null value is the literal null).</summary>
    internal static JsonNode Json(IVal<T>? v) => (v as Val<T>)?.Node.DeepClone() ?? UiJson.One("lit", null);

    private Cond Pair(string op, Val<T>? other) => new(UiJson.One(op, new JsonArray(Json(this), Json(other))));

    public Cond Eq(Val<T> other) => Pair("eq", other);

    public Cond Ne(Val<T> other) => Pair("ne", other);

    public Cond Gt(Val<T> other) => Pair("gt", other);

    public Cond Lt(Val<T> other) => Pair("lt", other);

    public Cond Empty() => new(UiJson.One("empty", Json(this)));

    /// <summary>The value is one of these.</summary>
    public Cond In(params T[] set) => new(UiJson.One("in", new JsonObject
    {
        ["value"] = Json(this),
        ["set"] = new JsonArray([.. set.Select(UiJson.Literal)]),
    }));
}

/// <summary>A param of a screen or overlay, whatever its type (the params a state's <c>reset</c> names).</summary>
public interface IParamRef
{
    string Name { get; }
}

/// <summary>A param of a screen or overlay (a filter, a record id): its value, and the filter inputs that set it.</summary>
public sealed class ParamRef<T> : Val<T>, IParamRef
{
    public string Name { get; }

    internal ParamRef(string name) : base("param", name) { Name = name; }

    private Input Control(string control, Txt label) => new(Name, control, label);

    /// <summary>A select in a filter bar: the options of an enum (or of a nullable enum), or <c>.Option(..)</c>.</summary>
    public Input Select(Txt label)
    {
        var i = Control("select", label);
        i.Options.AddRange(UiJson.EnumOptions(typeof(T)));
        return i;
    }

    /// <summary>
    /// A filter that takes several values: the options of an enum, or <c>.Option(..)</c>. The param is a string: the
    /// chosen values joined with commas, in the options' order (<c>unpaid,pending</c>); the endpoint splits it.
    /// </summary>
    public Input Choices(Txt label)
    {
        var i = Control("multi_select", label);
        i.Options.AddRange(UiJson.EnumOptions(typeof(T)));
        return i;
    }

    public Input Search(Txt label) => Control("search", label);

    public Input Date(Txt label) => Control("date", label);
}

/// <summary>Options of params.</summary>
public static class ParamRefExtensions
{
    /// <summary>An optional param passed where a plain value is wanted: absent, the argument is left out (its default).</summary>
    public static Val<T> OrEmpty<T>(this ParamRef<T?> param) where T : struct => new("param", param.Name);

    /// <summary>The same for an optional param of a reference type (a <c>string?</c>).</summary>
    public static Val<T> OrEmpty<T>(this ParamRef<T?> param) where T : class => new("param", param.Name);
}

/// <summary>A state value of a screen or overlay (<c>s.State("view", "list")</c>).</summary>
public sealed class StateRef<T> : Val<T>
{
    public string Name { get; }

    internal StateRef(string name) : base("state", name) { Name = name; }
}

/// <summary>A presentation condition (never access control).</summary>
public sealed class Cond : IUiJson
{
    internal JsonObject Node { get; }

    internal Cond(JsonObject node) { Node = node; }

    JsonNode IUiJson.ToJson(UiWriter w) => Node.DeepClone();

    public Cond And(Cond other) => new(UiJson.One("and", new JsonArray(Node.DeepClone(), other.Node.DeepClone())));

    public Cond Or(Cond other) => new(UiJson.One("or", new JsonArray(Node.DeepClone(), other.Node.DeepClone())));

    public static Cond operator &(Cond a, Cond b) => a.And(b);

    public static Cond operator |(Cond a, Cond b) => a.Or(b);

    public static Cond operator !(Cond c) => Ui.Not(c);
}

/// <summary>The current row of a table, list, chart or row action.</summary>
public sealed class Row<TRow>
{
    internal Row() { }

    public Val<T> Get<T>(Expression<Func<TRow, T>> field) => new("row", FieldPath.Of(field));

    /// <summary>The row as a list item (<c>r.View(r.Get(o =&gt; o.Customer)).Subtitle(..)</c>): a table's compact view, a search result.</summary>
    public RowView<TRow> View(Txt title) => new(title);
}

/// <summary>The values of the enclosing form.</summary>
public sealed class FormVals<TReq>
{
    internal FormVals() { }

    public Val<T> Get<T>(Expression<Func<TReq, T>> field) => new("form", FieldPath.Of(field));
}

/// <summary>A declared action (a chain of steps) that buttons, menu items and row opens run.</summary>
public sealed class ActionRef
{
    public string Name { get; }

    internal ActionRef(string name) { Name = name; }
}
