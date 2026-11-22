// An entity's access rules (`.Access<Note>(a => ..)`): who reads, creates, changes and deletes its rows, who may
// change which column to what (guards), which columns only some roles read (masks), and what other plugins may
// inherit with `via` (export). The platform enforces them on every read and write; the SDK only
// describes them. Grants combine with `|` (any of), `&` (all of) and `!` (not).
// Undeclared means denied: an entity without rules is read and written by no one but a system context
// (`ctx.AsSystem(reason)`, within the plugin's `authz:bypass`).
//
//   .Access<Note>(a =>
//   {
//       var author = a.Relation("author", a.Column(n => n.AuthorId));
//       var member = a.Relation("member", a.Rows<NoteMember>(m => m.NoteId, m => m.UserId));
//       a.Read(author | member | Roles.Manager);
//       a.Update(author.When(Filter<Note>.Eq(n => n.State, State.Draft)) | Roles.Manager);
//       a.Permit("publish", Roles.Manager);
//       a.Guard(n => n.State).To(State.Published, Grant.Holds("publish"));
//       a.Mask(n => n.Secret, Roles.Manager);
//   })
using System.Linq.Expressions;
using System.Text.Json;
using System.Text.Json.Nodes;
using Celobus.V1;

namespace Celobus.Sdk;

/// <summary>A permission: an operation (<c>Perm.Read</c>, <c>Create</c>, <c>Update</c>, <c>Delete</c>) or a custom action (<c>new("publish")</c>).</summary>
public sealed record Perm(string Name)
{
    public static readonly Perm Read = new("read");
    public static readonly Perm Create = new("create");
    public static readonly Perm Update = new("update");
    public static readonly Perm Delete = new("delete");
}

/// <summary>What a relation of a row names: a column of a person or a row, a list of people, a group, the rows of a junction.</summary>
public sealed record Rel(string Kind, string Field)
{
    /// <summary>A relation through rows: the junction (<c>entity</c>, <c>target</c>, <c>filter</c>).</summary>
    internal JsonObject? Rows { get; init; }
}

/// <summary>
/// A relation through the rows of the junction entity <typeparamref name="TJ"/> (<c>a.Rows&lt;ChannelMember&gt;(m =&gt;
/// m.ChannelId, m =&gt; m.UserId)</c>); <c>.When(filter)</c> counts only the junction rows matching a filter on its own
/// fields. Converts to a <see cref="Rel"/>.
/// </summary>
public sealed class RowsRel<TJ> where TJ : class
{
    private readonly string _target, _subject;
    private readonly JsonNode[] _filter;

    internal RowsRel(string target, string subject, JsonNode[] filter)
    {
        _target = target;
        _subject = subject;
        _filter = filter;
    }

    /// <summary>Only the junction rows matching <paramref name="filter"/> (equalities, <c>In</c> and null tests, with <c>&amp;</c>).</summary>
    public RowsRel<TJ> When(Filter<TJ> filter) =>
        new(_target, _subject, [.. _filter.Select(x => x.DeepClone()), .. Grant.Conditions(filter.Node)]);

    public static implicit operator Rel(RowsRel<TJ> r)
    {
        var t = typeof(TJ);
        // another plugin's entity is named with its plugin, which the platform's check refuses
        var entity = Schema.ForeignPlugin(t) is { } plugin ? $"{plugin}.{Names.Snake(t.Name)}" : Names.Snake(t.Name);
        return new Rel("rows", r._subject)
        {
            Rows = new JsonObject
            {
                ["entity"] = entity,
                ["target"] = r._target,
                ["filter"] = new JsonArray([.. r._filter.Select(x => x.DeepClone())]),
            },
        };
    }
}

/// <summary>A grant: who passes. From a relation, a <see cref="Role"/>, <see cref="Public"/>, <see cref="Authenticated"/>, <see cref="Holds"/>.</summary>
public class Grant
{
    internal JsonNode Node { get; }

    internal Grant(JsonNode node) { Node = node; }

    /// <summary>Everyone, members or not.</summary>
    public static Grant Public => new(JsonValue.Create("public"));

    /// <summary>Every member.</summary>
    public static Grant Authenticated => new(JsonValue.Create("authenticated"));

    /// <summary>Whoever the grant of a custom action admits.</summary>
    public static Grant Holds(string action) => new(new JsonObject { ["holds"] = action });

    /// <summary>Not the grant.</summary>
    public static Grant Not(Grant g) => new(new JsonObject { ["not"] = g.Node.DeepClone() });

    public static implicit operator Grant(Role r) => new(new JsonObject { ["role"] = r.Name });

    public static Grant operator |(Grant a, Grant b) => Join("any", a, b);

    public static Grant operator &(Grant a, Grant b) => Join("all", a, b);

    public static Grant operator !(Grant a) => Not(a);

    private static Grant Join(string op, Grant a, Grant b)
    {
        var items = a.Node is JsonObject o && o.Count == 1 && o[op] is JsonArray list
            ? new JsonArray([.. list.Select(x => x!.DeepClone())])
            : new JsonArray(a.Node.DeepClone());
        items.Add(b.Node.DeepClone());
        return new Grant(new JsonObject { [op] = items });
    }

    /// <summary>This grant, on the rows matching <paramref name="filter"/> (equalities, <c>In</c> and null tests, with <c>&amp;</c>).</summary>
    public Grant When<T>(Filter<T> filter) where T : class =>
        new(new JsonObject
        {
            ["when"] = new JsonObject
            {
                ["grant"] = Node.DeepClone(),
                ["filter"] = new JsonArray([.. Conditions(filter.Node)]),
            },
        });

    internal static IEnumerable<JsonNode> Conditions(DbFilter? f)
    {
        if (f is null) yield break;
        if (f.NodeCase == DbFilter.NodeOneofCase.And)
        {
            foreach (var x in f.And.Items)
                foreach (var c in Conditions(x)) yield return c;
            yield break;
        }
        if (f.NodeCase != DbFilter.NodeOneofCase.Condition)
        {
            yield return new JsonObject { ["field"] = "", ["op"] = "unsupported", ["value"] = null };
            yield break;
        }
        var cond = f.Condition;
        static JsonNode? Value(DbValue v) => v.KindCase switch
        {
            DbValue.KindOneofCase.StringValue => JsonValue.Create(v.StringValue),
            DbValue.KindOneofCase.IntValue => JsonValue.Create(v.IntValue),
            DbValue.KindOneofCase.DoubleValue => JsonValue.Create(v.DoubleValue),
            DbValue.KindOneofCase.BoolValue => JsonValue.Create(v.BoolValue),
            _ => null,
        };
        var (op, value) = cond.Op switch
        {
            DbCondition.Types.Op.Eq => ("eq", cond.Values.Count > 0 ? Value(cond.Values[0]) : null),
            DbCondition.Types.Op.Ne => ("ne", cond.Values.Count > 0 ? Value(cond.Values[0]) : null),
            DbCondition.Types.Op.In => ("in", (JsonNode?)new JsonArray([.. cond.Values.Select(Value)])),
            DbCondition.Types.Op.IsNull => ("is_null", null),
            DbCondition.Types.Op.IsNotNull => ("is_not_null", null),
            _ => ("unsupported", null),
        };
        yield return new JsonObject { ["field"] = cond.Field, ["op"] = op, ["value"] = value };
    }
}

/// <summary>A relation of the entity's rows: a grant by itself, or <c>.Via(perm)</c> for a relation to a row.</summary>
public sealed class Relation : Grant
{
    public string Name { get; }

    internal Relation(string name) : base(new JsonObject { ["rel"] = name }) { Name = name; }

    /// <summary>The caller holds <paramref name="perm"/> on the row this relation points to.</summary>
    public Grant Via(Perm perm) => new(new JsonObject { ["via"] = new JsonObject { ["rel"] = Name, ["perm"] = perm.Name } });
}

/// <summary>The rules of the entity <typeparamref name="T"/>, as <c>.Access&lt;T&gt;(a =&gt; ..)</c> declares them.</summary>
public sealed class Access<T> where T : class
{
    private readonly JsonArray _relations = [];
    private JsonNode? _read, _create, _update, _delete;
    private readonly JsonArray _permits = [];
    private readonly JsonArray _guards = [];
    private readonly JsonArray _masks = [];
    private readonly JsonArray _export = [];
    private bool _admin = true;

    private static string Name(LambdaExpression field) => Ui.FieldPath.Of(field);

    /// <summary>A column naming a person, or a row of an entity whose permissions <c>Via</c> inherits.</summary>
    public Rel Column<TV>(Expression<Func<T, TV>> field) => new("column", Name(field));

    /// <summary>A group: its members.</summary>
    public Rel Group<TV>(Expression<Func<T, TV>> field) => new("group", Name(field));

    /// <summary>
    /// The rows of the junction entity <typeparamref name="TJ"/> (one of this plugin's) whose <paramref name="target"/>
    /// column points to the row: each makes the person (<c>Id&lt;Platform.User&gt;</c>) or the group
    /// (<c>Id&lt;Platform.Group&gt;</c>: its members) its <paramref name="subject"/> column names a subject of the
    /// relation, as the junction's rows are when the statement runs.
    /// </summary>
    public RowsRel<TJ> Rows<TJ>(Expression<Func<TJ, object?>> target, Expression<Func<TJ, object?>> subject) where TJ : class =>
        new(Name(target), Name(subject), []);

    public Relation Relation(string name, Rel rel)
    {
        var decl = new JsonObject { ["name"] = name, ["kind"] = rel.Kind, ["field"] = rel.Field };
        if (rel.Rows is { } rows) decl["rows"] = rows.DeepClone();
        _relations.Add(decl);
        return new Relation(name);
    }

    public void Read(Grant g) => _read = g.Node.DeepClone();

    /// <summary>Checks the new row (an owner relation must name the caller).</summary>
    public void Create(Grant g) => _create = g.Node.DeepClone();

    /// <summary>Checks the row as it was; the row as it becomes must stay readable to the writer.</summary>
    public void Update(Grant g) => _update = g.Node.DeepClone();

    public void Delete(Grant g) => _delete = g.Node.DeepClone();

    /// <summary>A custom action and its grant (what <see cref="Grant.Holds"/> names).</summary>
    public void Permit(string action, Grant g) =>
        _permits.Add(new JsonObject { ["action"] = action, ["grant"] = g.Node.DeepClone() });

    /// <summary>Who may change a column, on update: <c>.To(value, grant)</c>, <c>.ToAny(grant)</c>.</summary>
    public Guard<TV> Guard<TV>(Expression<Func<T, TV>> field) => new(Add([Name(field)]));

    /// <summary>Who may change any of several columns (<c>.ToAny(grant)</c>).</summary>
    public Guard<object> Guard(params Expression<Func<T, object?>>[] fields) => new(Add([.. fields.Select(Name)]));

    private JsonArray Add(string[] fields)
    {
        var rules = new JsonArray();
        _guards.Add(new JsonObject { ["fields"] = new JsonArray([.. fields.Select(f => (JsonNode)f)]), ["rules"] = rules });
        return rules;
    }

    /// <summary>Only these roles read the column (an optional one: others read it as absent).</summary>
    public void Mask<TV>(Expression<Func<T, TV>> field, params Role[] roles) =>
        _masks.Add(new JsonObject
        {
            ["field"] = Name(field),
            ["roles"] = new JsonArray([.. roles.Select(r => (JsonNode)r.Name)]),
        });

    /// <summary>Permissions other plugins may inherit with <c>Via</c>.</summary>
    public void Export(params Perm[] perms)
    {
        foreach (var p in perms) _export.Add(p.Name);
    }

    /// <summary><c>workspace.admin</c> stands in for the roles that are not strict (the default); <c>false</c> for rows admins must not see.</summary>
    public void AdminBypass(bool yes) => _admin = yes;

    internal JsonObject ToJson() => new()
    {
        ["relations"] = _relations.DeepClone(),
        ["read"] = _read?.DeepClone(),
        ["create"] = _create?.DeepClone(),
        ["update"] = _update?.DeepClone(),
        ["delete"] = _delete?.DeepClone(),
        ["permits"] = _permits.DeepClone(),
        ["guards"] = _guards.DeepClone(),
        ["masks"] = _masks.DeepClone(),
        ["export"] = _export.DeepClone(),
        ["admin"] = _admin,
    };
}

/// <summary>The rules of a guard (see <see cref="Access{T}.Guard{TV}"/>).</summary>
public sealed class Guard<TV>
{
    private readonly JsonArray _rules;

    internal Guard(JsonArray rules) { _rules = rules; }

    /// <summary>To <paramref name="value"/>, by whoever <paramref name="g"/> admits (evaluated on the row as it was).</summary>
    public Guard<TV> To(TV value, Grant g)
    {
        var v = JsonSerializer.SerializeToNode(value, typeof(TV), CelobusJson.Options);
        _rules.Add(new JsonObject { ["value"] = v, ["grant"] = g.Node.DeepClone() });
        return this;
    }

    /// <summary>To any value.</summary>
    public Guard<TV> ToAny(Grant g)
    {
        _rules.Add(new JsonObject { ["value"] = null, ["grant"] = g.Node.DeepClone() });
        return this;
    }
}
