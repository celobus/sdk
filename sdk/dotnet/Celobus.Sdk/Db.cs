// The repository of the plugin's own entities (ctx.Db<T>()) and declared transactions (ctx.Transaction). The platform
// keeps the rows; the SDK sends typed requests over the session: rows
// in the entity's JSON mapping, filters as a tree of conditions on fields (Filter<Note>.Eq(n => n.Title, "a"),
// combined with &, | and !), orders. Reads return Stored<T>: the entity and its system columns.
using System.Linq.Expressions;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using Celobus.V1;
using Google.Protobuf;

namespace Celobus.Sdk;

/// <summary>The columns the store keeps on every entity (<c>DeletedAt</c> and <c>Version</c> when declared).</summary>
public enum SystemColumn
{
    CreatedAt,
    UpdatedAt,
    DeletedAt,
    Version,
}

/// <summary>A condition on the rows of <typeparamref name="T"/>; combined with <c>&amp;</c>, <c>|</c> and <c>!</c>.</summary>
public sealed class Filter<T> where T : class
{
    internal DbFilter? Node { get; }

    private Filter(DbFilter? node) { Node = node; }

    /// <summary>Every row.</summary>
    public static Filter<T> All { get; } = new(null);

    private static Filter<T> Condition(string field, DbCondition.Types.Op op, IEnumerable<object?> values)
    {
        var c = new DbCondition { Field = field, Op = op };
        c.Values.AddRange(values.Select(DbValues.Of));
        return new Filter<T>(new DbFilter { Condition = c });
    }

    private static string Field<TV>(Expression<Func<T, TV>> field) => Ui.FieldPath.Of(field);

    public static Filter<T> Eq<TV>(Expression<Func<T, TV>> field, TV value) => Condition(Field(field), DbCondition.Types.Op.Eq, [value]);
    /// <summary>Not equal (a null is not equal to a value).</summary>
    public static Filter<T> Ne<TV>(Expression<Func<T, TV>> field, TV value) => Condition(Field(field), DbCondition.Types.Op.Ne, [value]);
    public static Filter<T> Lt<TV>(Expression<Func<T, TV>> field, TV value) => Condition(Field(field), DbCondition.Types.Op.Lt, [value]);
    public static Filter<T> Le<TV>(Expression<Func<T, TV>> field, TV value) => Condition(Field(field), DbCondition.Types.Op.Le, [value]);
    public static Filter<T> Gt<TV>(Expression<Func<T, TV>> field, TV value) => Condition(Field(field), DbCondition.Types.Op.Gt, [value]);
    public static Filter<T> Ge<TV>(Expression<Func<T, TV>> field, TV value) => Condition(Field(field), DbCondition.Types.Op.Ge, [value]);

    /// <summary>Any of the values.</summary>
    public static Filter<T> In<TV>(Expression<Func<T, TV>> field, IEnumerable<TV> values) =>
        Condition(Field(field), DbCondition.Types.Op.In, values.Cast<object?>());

    /// <summary>
    /// Full-text search of a <c>[Searchable]</c> column: <paramref name="words"/> as PostgreSQL's
    /// <c>websearch_to_tsquery</c> reads them in the column's language (<c>"a phrase"</c>, <c>or</c>, <c>-word</c>).
    /// A column that is not searchable is 400 <c>invalid</c>.
    /// </summary>
    public static Filter<T> Search(Expression<Func<T, string?>> field, string words) =>
        Condition(Field(field), DbCondition.Types.Op.Search, [words]);

    /// <summary>A case-insensitive substring of a string column, taken literally (<c>%</c> and <c>_</c> are not wildcards).</summary>
    public static Filter<T> Contains(Expression<Func<T, string?>> field, string part) =>
        Condition(Field(field), DbCondition.Types.Op.Contains, [part]);

    public static Filter<T> IsNull<TV>(Expression<Func<T, TV>> field) => Condition(Field(field), DbCondition.Types.Op.IsNull, []);
    public static Filter<T> IsNotNull<TV>(Expression<Func<T, TV>> field) => Condition(Field(field), DbCondition.Types.Op.IsNotNull, []);

    /// <summary>A comparison on a system column: <c>Filter&lt;Note&gt;.On(SystemColumn.CreatedAt, ">=", since)</c>.</summary>
    public static Filter<T> On(SystemColumn column, string op, object value) => Condition(DbValues.Name(column), op switch
    {
        "=" => DbCondition.Types.Op.Eq,
        "!=" => DbCondition.Types.Op.Ne,
        "<" => DbCondition.Types.Op.Lt,
        "<=" => DbCondition.Types.Op.Le,
        ">" => DbCondition.Types.Op.Gt,
        ">=" => DbCondition.Types.Op.Ge,
        _ => throw new ArgumentException($"{op} is not one of = != < <= > >="),
    }, [value]);

    public static Filter<T> operator &(Filter<T> a, Filter<T> b) => a.Node is null ? b : b.Node is null ? a
        : new Filter<T>(new DbFilter { And = new DbFilters { Items = { a.Node, b.Node } } });

    public static Filter<T> operator |(Filter<T> a, Filter<T> b) => a.Node is null || b.Node is null ? All
        : new Filter<T>(new DbFilter { Or = new DbFilters { Items = { a.Node, b.Node } } });

    public static Filter<T> operator !(Filter<T> a) => a.Node is null
        ? new Filter<T>(new DbFilter { Or = new DbFilters() }) // not every row: none
        : new Filter<T>(new DbFilter { Not = a.Node });
}

internal static class DbValues
{
    /// <summary>A value as a filter parameter (the platform converts it to the column's type).</summary>
    public static DbValue Of(object? value)
    {
        var node = JsonSerializer.SerializeToNode(value, value?.GetType() ?? typeof(object), CelobusJson.Options);
        if (node is not JsonValue v) return new DbValue { StringValue = node?.ToJsonString() ?? "" };
        return v.GetValueKind() switch
        {
            JsonValueKind.True => new DbValue { BoolValue = true },
            JsonValueKind.False => new DbValue { BoolValue = false },
            JsonValueKind.Number when v.TryGetValue<long>(out var l) => new DbValue { IntValue = l },
            JsonValueKind.Number => new DbValue { DoubleValue = v.GetValue<double>() },
            _ => new DbValue { StringValue = v.GetValue<string>() },
        };
    }

    public static string Name(SystemColumn c) => c switch
    {
        SystemColumn.CreatedAt => "created_at",
        SystemColumn.UpdatedAt => "updated_at",
        SystemColumn.DeletedAt => "deleted_at",
        _ => "version",
    };
}

/// <summary>A row as the repository returns it: the entity (<see cref="Value"/>) and its system columns (not a UI <c>Row</c>).</summary>
public sealed record Stored<T>(T Value, DateTimeOffset CreatedAt, DateTimeOffset UpdatedAt, DateTimeOffset? DeletedAt, long? Version)
    where T : class
{
    public static implicit operator T(Stored<T> row) => row.Value;

    /// <summary>What <c>.Include(..)</c> brought with the row: nav name → a row, or a list of rows.</summary>
    internal JsonObject? IncludedJson { get; init; }

    /// <summary>
    /// The row a forward navigation names (<c>.Include("customer")</c>, or the ref column's name), as read under its
    /// entity's own rules: null when there is none or the caller may not read it, or when it was not included.
    /// </summary>
    public Stored<TI>? IncludedOne<TI>(string nav) where TI : class =>
        IncludedJson?[Names.Snake(nav)] is JsonObject o ? Stored<TI>.Read(o, null) : null;

    /// <summary>The rows of an inverse navigation (<c>.Include("lines")</c>) the caller may read, in id order; empty when it was not included.</summary>
    public IReadOnlyList<Stored<TI>> Included<TI>(string nav) where TI : class =>
        IncludedJson?[Names.Snake(nav)] is JsonArray a
            ? [.. a.OfType<JsonObject>().Select(o => Stored<TI>.Read(o, null))]
            : [];

    internal static Stored<T> Read(ByteString raw, ByteString? included = null)
    {
        var json = JsonNode.Parse(raw.Span) as JsonObject ?? throw PluginError.Unavailable("platform: a row is not a JSON object");
        JsonObject? inc = null;
        if (included is { Length: > 0 })
            inc = JsonNode.Parse(included.Span) as JsonObject ?? throw PluginError.Unavailable("platform: an include is not a JSON object");
        return Read(json, inc);
    }

    internal static Stored<T> Read(JsonObject json, JsonObject? included)
    {
        var value = json.Deserialize<T>(CelobusJson.Options) ?? throw PluginError.Unavailable("platform: an empty row");
        DateTimeOffset? Time(string k) => json[k] is JsonValue t ? DateTimeOffset.Parse(t.GetValue<string>(), System.Globalization.CultureInfo.InvariantCulture) : null;
        long? version = json["version"] is JsonValue v ? long.Parse(v.ToString(), System.Globalization.CultureInfo.InvariantCulture) : null;
        return new Stored<T>(value, Time("createdAt") ?? default, Time("updatedAt") ?? default, Time("deletedAt"), version)
        {
            IncludedJson = included,
        };
    }
}

/// <summary>The changes of an update: fields set to a value, or cleared (null); the others stay.</summary>
public sealed class Patch<T> where T : class
{
    internal JsonObject Fields { get; } = new();

    private Patch() { }

    public static Patch<T> Set<TV>(Expression<Func<T, TV>> field, TV value) => new Patch<T>().And(field, value);

    /// <summary>Sets another field.</summary>
    public Patch<T> And<TV>(Expression<Func<T, TV>> field, TV value)
    {
        Fields[Names.Camel(Ui.FieldPath.Of(field))] = JsonSerializer.SerializeToNode(value, CelobusJson.Options);
        return this;
    }

    /// <summary>Sets an optional field to null.</summary>
    public Patch<T> Clear<TV>(Expression<Func<T, TV>> field)
    {
        Fields[Names.Camel(Ui.FieldPath.Of(field))] = null;
        return this;
    }
}

/// <summary>A page of rows and the token of the next one (null: the last page).</summary>
public sealed record Page<T>(IReadOnlyList<Stored<T>> Rows, string? Next) where T : class;

/// <summary>The repository of an entity of the plugin: <c>ctx.Db&lt;Order&gt;()</c> (in a transaction, <c>tx.Db&lt;Order&gt;()</c>).</summary>
public sealed class Db<T> where T : class
{
    private readonly Ctx _ctx;

    /// <summary>The repository of an entity of the plugin, or (with <paramref name="dependency"/>) the reads of a dependency's.</summary>
    internal Db(Ctx ctx, string dependency = "")
    {
        _ctx = ctx;
        Entity = Names.Snake(typeof(T).Name);
        Key = typeof(T).GetCustomAttribute<EntityAttribute>()?.Key ?? KeyType.Uuid;
        Dependency = dependency;
    }

    internal string Entity { get; }

    /// <summary>The provider of a dependency's entity (<c>ctx.Query&lt;T&gt;()</c>); empty for the plugin's own.</summary>
    internal string Dependency { get; }
    private KeyType Key { get; }
    internal ulong RequestId => _ctx.Request.Id;
    internal ulong Tx => _ctx.Tx;
    internal string AsSystem => _ctx.System;

    internal Task<SyscallResult> Send(Syscall call) => _ctx.Syscall(call);

    internal static List<Stored<T>> RowsOf(SyscallResult r)
    {
        var rows = r.DbRows ?? throw PluginError.Unavailable("platform: unexpected syscall result");
        return [.. rows.Rows.Select((raw, i) => Stored<T>.Read(raw, i < rows.Included.Count ? rows.Included[i] : null))];
    }

    internal static Stored<T> One(SyscallResult r) =>
        RowsOf(r).FirstOrDefault() ?? throw PluginError.Unavailable("the platform answered without the row");

    /// <summary>
    /// Row locks exist inside a declared transaction only (<c>tx.Db&lt;T&gt;()</c>): outside one they are refused here,
    /// before anything is sent.
    /// </summary>
    internal void RequireTransaction(string what)
    {
        if (Tx == 0)
            throw PluginError.Invalid($"{what} locks rows inside a transaction only: call it on tx.Db<{typeof(T).Name}>() in ctx.Transaction(..)");
        if (Dependency != "") throw PluginError.Invalid($"{what} locks the plugin's own rows only, not a dependency's");
    }

    /// <summary>
    /// A row by id; 404 <c>not_found</c> when there is none or it is soft-deleted (<paramref name="withDeleted"/> reads
    /// those too). Awaited as it is, or first given <c>.Include(..)</c> and, in a transaction, a row lock
    /// (<c>.ForUpdate()</c>, <c>.ForShare()</c>, <c>.NoWait()</c>).
    /// </summary>
    public GetRow<T> Get(Id<T> id, bool withDeleted = false) => new(this, id, withDeleted);

    /// <summary>
    /// Groups of the rows matching <paramref name="filter"/> (every row without one), counted or summed:
    /// <c>Aggregate(f).GroupBy(n => n.OwnerId).Count()</c>; at most 1,000 groups (422 <c>query_rows_limit</c>).
    /// </summary>
    public Aggregate<T> Aggregate(Filter<T>? filter = null) => new(this, filter ?? Filter<T>.All);

    /// <summary>
    /// Changes the fields the patch names on every row matching <paramref name="filter"/> (<c>Filter&lt;T&gt;.All</c>:
    /// every row), in one statement: the rows changed. Each row passes the update rule and the guards as an
    /// <c>Update</c> does (one that does not refuses all); more than 1,000 rows is 422 <c>bulk_limit</c>, nothing written.
    /// </summary>
    public async Task<long> UpdateWhere(Filter<T> filter, Patch<T> patch)
    {
        var r = await Send(new Syscall
        {
            DbUpdateWhere = new DbUpdateWhere
            {
                RequestId = RequestId, Tx = Tx, AsSystem = AsSystem, Entity = Entity, Filter = filter.Node,
                Patch = ByteString.CopyFromUtf8(patch.Fields.ToJsonString(CelobusJson.Options)),
            },
        });
        return (long)(r.DbCount ?? throw PluginError.Unavailable("platform: unexpected syscall result")).Count;
    }

    /// <summary>
    /// Creates <paramref name="row"/>, or changes the row with the same value of the unique column
    /// <paramref name="on"/> to it (<c>n => n.Id</c> on a <c>string</c>-key entity). The create rule applies to a new
    /// row, the update rule and the guards to a changed one.
    /// </summary>
    public async Task<Stored<T>> Upsert<TV>(Expression<Func<T, TV>> on, T row) => One(await Send(new Syscall
    {
        DbUpsert = new DbUpsert
        {
            RequestId = RequestId, Tx = Tx, AsSystem = AsSystem, Entity = Entity, On = Ui.FieldPath.Of(on), Row = RowBytes(row),
        },
    }));

    private ByteString RowBytes(T row)
    {
        var json = JsonSerializer.SerializeToNode(row, CelobusJson.Options) as JsonObject ?? [];
        if (json["id"] is not JsonValue id || id.ToString() == "")
            json["id"] = Key switch { KeyType.I64 => "0", KeyType.Uuid => Guid.Empty.ToString(), _ => "" };
        return ByteString.CopyFromUtf8(json.ToJsonString(CelobusJson.Options));
    }

    /// <summary>The rows matching <paramref name="filter"/> (every row without one): order, limit and page it, then read them.</summary>
    public Find<T> Find(Filter<T>? filter = null) => new(this, filter ?? Filter<T>.All);

    /// <summary>Every row (at most 1,000).</summary>
    public Task<IReadOnlyList<Stored<T>>> All() => Find().All();

    public async Task<long> Count(Filter<T>? filter = null)
    {
        var r = await Send(new Syscall
        {
            DbCount = new DbCount
            {
                RequestId = RequestId, Tx = Tx, AsSystem = AsSystem, Entity = Entity, Filter = (filter ?? Filter<T>.All).Node,
                Dependency = Dependency,
            },
        });
        return (long)(r.DbCount ?? throw PluginError.Unavailable("platform: unexpected syscall result")).Count;
    }

    public async Task<bool> Exists(Filter<T> filter) => await Find(filter).First() is not null;

    /// <summary>A new row: the store makes a <c>uuid</c> or <c>i64</c> key (leave <c>Id</c> default); a <c>string</c> key is the row's.</summary>
    public async Task<Stored<T>> Create(T row) =>
        (await CreateMany([row])).FirstOrDefault() ?? throw PluginError.Unavailable("the platform answered without the row");

    /// <summary>New rows, in one statement (at most 1,000).</summary>
    public async Task<IReadOnlyList<Stored<T>>> CreateMany(IEnumerable<T> rows)
    {
        var create = new DbCreate { RequestId = RequestId, Tx = Tx, AsSystem = AsSystem, Entity = Entity };
        foreach (var row in rows) create.Rows.Add(RowBytes(row));
        return RowsOf(await Send(new Syscall { DbCreate = create }));
    }

    /// <summary>Changes the fields the patch names; <c>.ExpectVersion(v)</c> checks the row's version.</summary>
    public Update<T> Update(Id<T> id, Patch<T> patch) => new(this, id, patch);

    /// <summary>Deletes a row (marks it, on a soft-delete entity); <c>.ExpectVersion(v)</c> checks its version.</summary>
    public Delete<T> Delete(Id<T> id) => new(this, id, force: false);

    /// <summary>Removes a row of a soft-delete entity for good.</summary>
    public Delete<T> DeleteForce(Id<T> id) => new(this, id, force: true);

    /// <summary>Brings a soft-deleted row back.</summary>
    public async Task<Stored<T>> Restore(Id<T> id) => One(await Send(new Syscall
    {
        DbRestore = new DbRestore { RequestId = RequestId, Tx = Tx, AsSystem = AsSystem, Entity = Entity, Id = id.Value },
    }));
}

/// <summary>A query being built: <c>db.Find(filter).OrderBy(..).Limit(..)</c>, read with <c>All()</c>, <c>First()</c> or <c>Page()</c>.</summary>
public sealed class Find<T> where T : class
{
    private readonly Db<T> _db;
    private readonly Filter<T> _filter;
    private readonly List<DbOrder> _order = [];
    private uint _limit;
    private string _token = "";
    private bool _withDeleted;
    private readonly List<string> _include = [];
    private readonly DbLock _lock = new();

    internal Find(Db<T> db, Filter<T> filter)
    {
        _db = db;
        _filter = filter;
    }

    /// <summary>Orders by a field; called again, by the next one (ties end by id).</summary>
    public Find<T> OrderBy<TV>(Expression<Func<T, TV>> field) => Order(Ui.FieldPath.Of(field), false);

    public Find<T> OrderByDescending<TV>(Expression<Func<T, TV>> field) => Order(Ui.FieldPath.Of(field), true);

    /// <summary>Orders by a system column.</summary>
    public Find<T> OrderBy(SystemColumn column, bool descending = false) => Order(DbValues.Name(column), descending);

    private Find<T> Order(string field, bool desc)
    {
        _order.Add(new DbOrder { Field = field, Desc = desc });
        return this;
    }

    /// <summary>Rows per page (100 at most; 100 by default).</summary>
    public Find<T> Limit(uint n)
    {
        _limit = n;
        return this;
    }

    /// <summary>The page after the one whose <c>Next</c> this is.</summary>
    public Find<T> PageToken(string token)
    {
        _token = token;
        return this;
    }

    /// <summary>Soft-deleted rows too.</summary>
    public Find<T> WithDeleted()
    {
        _withDeleted = true;
        return this;
    }

    /// <summary>
    /// Brings a navigation with each row: a forward nav (or its ref column's name) or an inverse nav, read under its
    /// entity's own rules (<c>row.IncludedOne&lt;Customer&gt;("customer")</c>, <c>row.Included&lt;Line&gt;("lines")</c>).
    /// The rows it brings count toward the 1,000 of a read.
    /// </summary>
    public Find<T> Include(string nav)
    {
        _include.Add(Names.Snake(nav));
        return this;
    }

    /// <summary>A forward navigation by its ref column: <c>.Include(n => n.CustomerId)</c> (read with <c>IncludedOne(nameof(..))</c>).</summary>
    public Find<T> Include<TV>(Expression<Func<T, TV>> refColumn) => Include(Ui.FieldPath.Of(refColumn));

    /// <summary>Locks the rows read until the transaction ends (inside <c>ctx.Transaction</c> only; else 400 <c>invalid</c>).</summary>
    public Find<T> ForUpdate() => Lock(DbLock.Types.Strength.ForUpdate, "ForUpdate");

    /// <summary>A shared lock of the rows read (inside <c>ctx.Transaction</c> only).</summary>
    public Find<T> ForShare() => Lock(DbLock.Types.Strength.ForShare, "ForShare");

    /// <summary>Leaves out the rows another transaction holds (a work queue: concurrent runs take different rows).</summary>
    public Find<T> SkipLocked() => Wait(DbLock.Types.Wait.SkipLocked, "SkipLocked");

    /// <summary>Fails at once with 409 <c>row_locked</c> instead of waiting for a row another transaction holds.</summary>
    public Find<T> NoWait() => Wait(DbLock.Types.Wait.Nowait, "NoWait");

    private Find<T> Lock(DbLock.Types.Strength strength, string what)
    {
        _db.RequireTransaction(what);
        _lock.Strength = strength;
        return this;
    }

    private Find<T> Wait(DbLock.Types.Wait wait, string what)
    {
        _db.RequireTransaction(what);
        _lock.Wait = wait;
        return this;
    }

    private Task<SyscallResult> Send(bool all, uint limit)
    {
        var find = new DbFind
        {
            RequestId = _db.RequestId, Tx = _db.Tx, AsSystem = _db.AsSystem, Entity = _db.Entity, Filter = _filter.Node, Limit = limit,
            PageToken = _token, WithDeleted = _withDeleted, All = all, Dependency = _db.Dependency,
        };
        find.Order.AddRange(_order);
        find.Include.AddRange(_include);
        if (_lock.Strength != DbLock.Types.Strength.LockNone || _lock.Wait != DbLock.Types.Wait.Wait) find.Lock = _lock.Clone();
        return _db.Send(new Syscall { DbFind = find });
    }

    /// <summary>Every matching row: more than 1,000 is 422 <c>query_rows_limit</c> (page through them instead).</summary>
    public async Task<IReadOnlyList<Stored<T>>> All() => Db<T>.RowsOf(await Send(true, 0));

    public async Task<Stored<T>?> First() => Db<T>.RowsOf(await Send(false, 1)).FirstOrDefault();

    public async Task<Page<T>> Page()
    {
        var r = await Send(false, _limit);
        var next = r.DbRows?.NextPageToken;
        return new Page<T>(Db<T>.RowsOf(r), string.IsNullOrEmpty(next) ? null : next);
    }
}

/// <summary>
/// A read of one row being built: <c>await db.Get(id)</c>, or <c>await db.Get(id).Include("lines")</c>, or in a
/// transaction <c>await tx.Db&lt;T&gt;().Get(id).ForUpdate()</c>. Converts to <c>Task&lt;Stored&lt;T&gt;&gt;</c>.
/// </summary>
public sealed class GetRow<T> where T : class
{
    private readonly Db<T> _db;
    private readonly Id<T> _id;
    private readonly bool _withDeleted;
    private readonly List<string> _include = [];
    private readonly DbLock _lock = new();

    internal GetRow(Db<T> db, Id<T> id, bool withDeleted)
    {
        _db = db;
        _id = id;
        _withDeleted = withDeleted;
    }

    /// <summary>Brings a navigation with the row (see <see cref="Find{T}.Include(string)"/>).</summary>
    public GetRow<T> Include(string nav)
    {
        _include.Add(Names.Snake(nav));
        return this;
    }

    /// <summary>A forward navigation by its ref column.</summary>
    public GetRow<T> Include<TV>(Expression<Func<T, TV>> refColumn) => Include(Ui.FieldPath.Of(refColumn));

    /// <summary>Locks the row until the transaction ends (inside <c>ctx.Transaction</c> only; else 400 <c>invalid</c>).</summary>
    public GetRow<T> ForUpdate()
    {
        _db.RequireTransaction("ForUpdate");
        _lock.Strength = DbLock.Types.Strength.ForUpdate;
        return this;
    }

    /// <summary>A shared lock of the row (inside <c>ctx.Transaction</c> only).</summary>
    public GetRow<T> ForShare()
    {
        _db.RequireTransaction("ForShare");
        _lock.Strength = DbLock.Types.Strength.ForShare;
        return this;
    }

    /// <summary>Fails at once with 409 <c>row_locked</c> when another transaction holds the row.</summary>
    public GetRow<T> NoWait()
    {
        _db.RequireTransaction("NoWait");
        _lock.Wait = DbLock.Types.Wait.Nowait;
        return this;
    }

    private async Task<Stored<T>> Send()
    {
        var get = new DbGet
        {
            RequestId = _db.RequestId, Tx = _db.Tx, AsSystem = _db.AsSystem, Entity = _db.Entity, Id = _id.Value,
            WithDeleted = _withDeleted, Dependency = _db.Dependency,
        };
        get.Include.AddRange(_include);
        if (_lock.Strength != DbLock.Types.Strength.LockNone || _lock.Wait != DbLock.Types.Wait.Wait) get.Lock = _lock.Clone();
        return Db<T>.One(await _db.Send(new Syscall { DbGet = get }));
    }

    public TaskAwaiter<Stored<T>> GetAwaiter() => Send().GetAwaiter();

    /// <summary>The read as a task (what <c>Get</c> returned before it took options).</summary>
    public Task<Stored<T>> AsTask() => Send();

    public static implicit operator Task<Stored<T>>(GetRow<T> get) => get.Send();
}

/// <summary>An aggregate being built: <c>db.Aggregate(filter).GroupBy(..)</c>, then <c>Count()</c> or <c>Sum(..)</c>.</summary>
public sealed class Aggregate<T> where T : class
{
    private readonly Db<T> _db;
    private readonly Filter<T> _filter;
    private bool _withDeleted;

    internal Aggregate(Db<T> db, Filter<T> filter)
    {
        _db = db;
        _filter = filter;
    }

    /// <summary>Soft-deleted rows too.</summary>
    public Aggregate<T> WithDeleted()
    {
        _withDeleted = true;
        return this;
    }

    /// <summary>One group per value of a field (null values are a group of their own, last).</summary>
    public Grouped<T, TK> GroupBy<TK>(Expression<Func<T, TK>> field) => new(this, Ui.FieldPath.Of(field));

    /// <summary>One group per value of a system column.</summary>
    public Grouped<T, TK> GroupBy<TK>(SystemColumn column) => new(this, DbValues.Name(column));

    /// <summary>The rows matching.</summary>
    public async Task<long> Count() => (await Groups("", DbAggregate.Types.Measure.Count, "")).Select(g => Value<long>(g)).FirstOrDefault();

    public Task<long> Sum(Expression<Func<T, int>> field) => SumOne<long>(field);
    public Task<long> Sum(Expression<Func<T, int?>> field) => SumOne<long>(field);
    public Task<long> Sum(Expression<Func<T, long>> field) => SumOne<long>(field);
    public Task<long> Sum(Expression<Func<T, long?>> field) => SumOne<long>(field);
    public Task<decimal> Sum(Expression<Func<T, decimal>> field) => SumOne<decimal>(field);
    public Task<decimal> Sum(Expression<Func<T, decimal?>> field) => SumOne<decimal>(field);
    public Task<double> Sum(Expression<Func<T, double>> field) => SumOne<double>(field);
    public Task<double> Sum(Expression<Func<T, double?>> field) => SumOne<double>(field);

    private async Task<TV> SumOne<TV>(LambdaExpression field) =>
        (await Groups("", DbAggregate.Types.Measure.Sum, Ui.FieldPath.Of(field))).Select(g => Value<TV>(g)).FirstOrDefault()!;

    internal async Task<IReadOnlyList<DbGroup>> Groups(string groupBy, DbAggregate.Types.Measure measure, string field)
    {
        var r = await _db.Send(new Syscall
        {
            DbAggregate = new DbAggregate
            {
                RequestId = _db.RequestId, Tx = _db.Tx, AsSystem = _db.AsSystem, Entity = _db.Entity, Filter = _filter.Node,
                WithDeleted = _withDeleted, Dependency = _db.Dependency, GroupBy = groupBy, Measure = measure, Field = field,
            },
        });
        return (r.DbGroups ?? throw PluginError.Unavailable("platform: unexpected syscall result")).Groups;
    }

    internal static TV Value<TV>(DbGroup g) => Decode<TV>(g.Value);

    internal static TV Decode<TV>(ByteString json) =>
        JsonSerializer.Deserialize<TV>(json.Span, CelobusJson.Options)!;
}

/// <summary>An aggregate by groups: <c>Count()</c> → <c>(Key, Count)</c>, <c>Sum(..)</c> → <c>(Key, Sum)</c>, in key order.</summary>
public sealed class Grouped<T, TK> where T : class
{
    private readonly Aggregate<T> _agg;
    private readonly string _groupBy;

    internal Grouped(Aggregate<T> agg, string groupBy)
    {
        _agg = agg;
        _groupBy = groupBy;
    }

    public async Task<IReadOnlyList<(TK Key, long Count)>> Count() =>
        [.. (await _agg.Groups(_groupBy, DbAggregate.Types.Measure.Count, "")).Select(g => (Aggregate<T>.Decode<TK>(g.Key), Aggregate<T>.Value<long>(g)))];

    public Task<IReadOnlyList<(TK Key, long Sum)>> Sum(Expression<Func<T, int>> field) => SumBy<long>(field);
    public Task<IReadOnlyList<(TK Key, long Sum)>> Sum(Expression<Func<T, int?>> field) => SumBy<long>(field);
    public Task<IReadOnlyList<(TK Key, long Sum)>> Sum(Expression<Func<T, long>> field) => SumBy<long>(field);
    public Task<IReadOnlyList<(TK Key, long Sum)>> Sum(Expression<Func<T, long?>> field) => SumBy<long>(field);
    public Task<IReadOnlyList<(TK Key, decimal Sum)>> Sum(Expression<Func<T, decimal>> field) => SumBy<decimal>(field);
    public Task<IReadOnlyList<(TK Key, decimal Sum)>> Sum(Expression<Func<T, decimal?>> field) => SumBy<decimal>(field);
    public Task<IReadOnlyList<(TK Key, double Sum)>> Sum(Expression<Func<T, double>> field) => SumBy<double>(field);
    public Task<IReadOnlyList<(TK Key, double Sum)>> Sum(Expression<Func<T, double?>> field) => SumBy<double>(field);

    private async Task<IReadOnlyList<(TK Key, TV Sum)>> SumBy<TV>(LambdaExpression field) =>
        [.. (await _agg.Groups(_groupBy, DbAggregate.Types.Measure.Sum, Ui.FieldPath.Of(field)))
            .Select(g => (Aggregate<T>.Decode<TK>(g.Key), Aggregate<T>.Value<TV>(g)))];
}

/// <summary>An update being built: <c>await db.Update(id, patch).ExpectVersion(v)</c>.</summary>
public sealed class Update<T> where T : class
{
    private readonly Db<T> _db;
    private readonly Id<T> _id;
    private readonly Patch<T> _patch;
    private long? _expected;

    internal Update(Db<T> db, Id<T> id, Patch<T> patch)
    {
        _db = db;
        _id = id;
        _patch = patch;
    }

    /// <summary>Refuses the update (409 <c>version_conflict</c>) unless the row is at this version.</summary>
    public Update<T> ExpectVersion(long? version)
    {
        _expected = version;
        return this;
    }

    private async Task<Stored<T>> Send()
    {
        var update = new DbUpdate
        {
            RequestId = _db.RequestId, Tx = _db.Tx, AsSystem = _db.AsSystem, Entity = _db.Entity, Id = _id.Value,
            Patch = ByteString.CopyFromUtf8(_patch.Fields.ToJsonString(CelobusJson.Options)),
        };
        if (_expected is { } v) update.ExpectedVersion = v;
        var rows = Db<T>.RowsOf(await _db.Send(new Syscall { DbUpdate = update }));
        return rows.FirstOrDefault() ?? throw PluginError.Unavailable("the platform answered without the row");
    }

    public TaskAwaiter<Stored<T>> GetAwaiter() => Send().GetAwaiter();
}

/// <summary>A delete being built: <c>await db.Delete(id).ExpectVersion(v)</c>.</summary>
public sealed class Delete<T> where T : class
{
    private readonly Db<T> _db;
    private readonly Id<T> _id;
    private readonly bool _force;
    private long? _expected;

    internal Delete(Db<T> db, Id<T> id, bool force)
    {
        _db = db;
        _id = id;
        _force = force;
    }

    public Delete<T> ExpectVersion(long? version)
    {
        _expected = version;
        return this;
    }

    private async Task Send()
    {
        var delete = new DbDelete { RequestId = _db.RequestId, Tx = _db.Tx, AsSystem = _db.AsSystem, Entity = _db.Entity, Id = _id.Value, Force = _force };
        if (_expected is { } v) delete.ExpectedVersion = v;
        await _db.Send(new Syscall { DbDelete = delete });
    }

    public TaskAwaiter GetAwaiter() => Send().GetAwaiter();
}

/// <summary>The isolation of a declared transaction.</summary>
public enum TxIsolation
{
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

/// <summary>How a declared transaction runs: read committed by default; not for a nested one (it rolls back alone).</summary>
public sealed record TxOptions
{
    public TxIsolation Isolation { get; init; } = TxIsolation.ReadCommitted;

    /// <summary>Refuses writes.</summary>
    public bool ReadOnly { get; init; }
}
