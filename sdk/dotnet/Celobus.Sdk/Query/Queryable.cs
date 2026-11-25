// Typed reads across entities: ctx.Query<T>() is an IQueryable<T>. A LINQ query over it (joins, a left join through
// `into .. DefaultIfEmpty()`, where, group by, orderby, select, Take) is translated from its expression tree into the
// query tree of the session (`celobus.v1.Query`, Translator.cs), never SQL, and read with ToListAsync,
// FirstOrDefaultAsync, CountAsync or PageAsync. The analyzer CELOBUS060 reports at compile time what the translation
// refuses (a call to an arbitrary method inside the query).
//
// ctx.Analytics().Query<T>() is the same on the analytic copy of the workspace's entities: the same LINQ, the same
// access rules and masks, the same answer for the data the copy holds. The copy follows the data, usually within a
// second, never with a promise: a caller does not see their own write in it at once, so a screen that shows what was
// just saved reads it with ctx.Query<T>(). Every analytic answer says when its data is from (ToListAsOfAsync,
// CountAsOfAsync, QueryPage.AsOf); it is never waited for and never refused for being old. Every source of one query
// reads one engine: a join or an Any of an analytic query starts from ctx.Analytics().Query<T>() too.
//
// A plugin's analytic view (a [View] record, declared with Plugin.View over Views.From<T>()) is read the same way:
// ctx.Query<Sales>() and ctx.Analytics().Query<Sales>() answer rows of the view's own type, and filter, group and order
// it as an entity. Query.Day .. Query.Year bucket a date or a moment, Query.DaysAgo(n) is a day relative to the day the
// query runs, Query.Between a range (Celobus.Sdk.Linq).

using System.Collections;
using System.Linq.Expressions;
using System.Runtime.CompilerServices;
using System.Text.Json;
using Celobus.V1;
using Google.Protobuf;

namespace Celobus.Sdk;

/// <summary>A row's label as <c>Query.Label(ref)</c> selects it: a person's (or a group's) name and avatar, another row's label column.</summary>
public sealed record Label(string Id, string Text, string? AvatarUrl);

/// <summary>
/// A page of a query's rows and the token of the next page (null: the last); from the analytic copy
/// (<c>ctx.Analytics()</c>), when its data is from.
/// </summary>
public sealed record QueryPage<T>(IReadOnlyList<T> Rows, string? Next)
{
    /// <summary>When the analytic copy's data is from (null from <c>ctx.Query&lt;T&gt;()</c>).</summary>
    public DateTimeOffset? AsOf { get; init; }

    /// <summary>Which copy answered (null from <c>ctx.Query&lt;T&gt;()</c>).</summary>
    public ulong? Generation { get; init; }
}

/// <summary>
/// An answer and, from the analytic copy (<c>ctx.Analytics()</c>), when its data is from (<see cref="AsOf"/> and
/// <see cref="Generation"/> are null from <c>ctx.Query&lt;T&gt;()</c>).
/// </summary>
public sealed record QueryAnswer<T>(T Rows, DateTimeOffset? AsOf, ulong? Generation);

/// <summary>
/// The analytic copy of the workspace's entities (<c>ctx.Analytics()</c>): <c>Query&lt;T&gt;()</c> is
/// <c>ctx.Query&lt;T&gt;()</c> read there, for reports and figures over many rows. Its answer may be slightly behind
/// (usually under a second, never promised) and says when its data is from. A query runs at most 10 s (422
/// <c>analytic_time_limit</c>) within the copy's memory (422 <c>analytic_memory_limit</c>); four of the workspace run
/// at once, two of the plugin (503 <c>analytic_busy</c> after a short wait); 503 <c>unavailable</c> while the copy is
/// being made, when its data is older than the workspace's limit (15 minutes unless the workspace sets another) or when
/// the workspace has none. Refused inside a transaction (400 <c>invalid</c>); <c>Query.Search</c> is
/// not available there. Text compares and orders by its bytes there. Decimals compute there with at most 20 digits
/// before the point and 18 after: a query that compares, orders, groups or aggregates a column holding a longer value
/// the caller may read (or binds one, or sums past them) is 500 <c>query_invalid</c>, naming the column, never
/// rounded. A group by a decimal, and its <c>Min</c> or <c>Max</c>, answer the shortest text of the number
/// (<c>1.5</c> for <c>1.5</c> and <c>1.50</c>).
/// </summary>
public sealed class Analytics
{
    private readonly Ctx _ctx;

    internal Analytics(Ctx ctx) => _ctx = ctx;

    /// <summary>
    /// <c>ctx.Query&lt;T&gt;()</c> on the analytic copy: the same LINQ, rules and answers, plus when its data is from.
    /// It is read with <c>ToListAsync</c>, <c>ToListAsOfAsync</c>, <c>FirstOrDefaultAsync</c>, <c>CountAsync</c>,
    /// <c>CountAsOfAsync</c> or <c>PageAsync</c>; the repository's reads (<c>Get</c>, <c>Find</c>, <c>All</c>,
    /// <c>Count</c>, <c>Exists</c>, <c>Aggregate</c>) read only the workspace's data, through <c>ctx.Query&lt;T&gt;()</c>.
    /// </summary>
    public CelobusQuery<T> Query<T>() where T : class => new(_ctx, Engine.Analytic);

    /// <summary>
    /// A snapshot of the analytic copy to ask for, that loads some of the plugin's entities stored in OLAP
    /// (<c>.Load&lt;T&gt;().RequestAsync()</c>, Olap.cs).
    /// </summary>
    public Snapshot Snapshot() => new(_ctx);
}

/// <summary>
/// The sources of an analytic view's query (<c>Plugin.View(..)</c>): <c>from l in Views.From&lt;OrderLine&gt;() join o
/// in Views.From&lt;Order&gt;() on l.OrderId equals o.Id .. select new Sales(..)</c>. A view's query is declared, never
/// read: the view itself is read with <c>ctx.Query&lt;Sales&gt;()</c> or <c>ctx.Analytics().Query&lt;Sales&gt;()</c>.
/// </summary>
public static class Views
{
    /// <summary>
    /// A source of a view's query: an entity of the plugin, a dependency's exported entity, the directory's, or another
    /// view of the plugin.
    /// </summary>
    public static CelobusQuery<T> From<T>() where T : class => new(null, Engine.Store);
}

/// <summary>
/// A root of a query: the entity, the context it reads as (none for a view's or a model's query), which data it reads,
/// and whether it is the entity's history (per day).
/// </summary>
internal interface IQueryRoot
{
    Type Entity { get; }
    Ctx? Ctx { get; }
    Engine Engine { get; }
    bool History { get; }
    bool EachDay { get; }
}

/// <summary>
/// A typed read across entities from the root <typeparamref name="T"/>: the plugin's own entity, a dependency's exported
/// one (which keeps the repository's reads: <see cref="Get"/>, <see cref="Find"/>, <see cref="All"/>,
/// <see cref="Count"/>, <see cref="Exists"/>, <see cref="Aggregate"/>), the directory's (<c>Platform.User</c>,
/// <c>Platform.Group</c>, <c>Platform.GroupMember</c>, <c>Platform.RoleAssignment</c>), or one of the plugin's
/// analytic views (<c>[View]</c>), whose rows are its own type.
/// </summary>
public sealed class CelobusQuery<T> : IOrderedQueryable<T>, IQueryRoot where T : class
{
    private readonly Ctx? _ctx;
    private readonly Engine _engine;
    private readonly bool _eachDay;
    private Db<T>? _db;

    internal CelobusQuery(Ctx? ctx, Engine engine = Engine.Store, bool eachDay = false)
    {
        _ctx = ctx;
        _engine = engine;
        _eachDay = eachDay;
        Provider = new QueryProvider(ctx);
        Expression = Expression.Constant(this);
    }

    internal Ctx? Context => _ctx;

    public Type ElementType => typeof(T);
    public Expression Expression { get; }
    public IQueryProvider Provider { get; }
    Type IQueryRoot.Entity => typeof(T);
    Ctx? IQueryRoot.Ctx => _ctx;
    Engine IQueryRoot.Engine => _engine;
    bool IQueryRoot.History => Schema.IsHistoryRow(typeof(T));
    bool IQueryRoot.EachDay => _eachDay;

    public IEnumerator<T> GetEnumerator() => throw QueryProvider.NotEnumerable();
    IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();

    private Db<T> Repository()
    {
        if (Translator.IsDirectory(typeof(T)))
            throw new InvalidOperationException($"{typeof(T).Name} is the directory's: read it with a query (ToListAsync)");
        if (Schema.IsView(typeof(T)))
            throw new InvalidOperationException($"{typeof(T).Name} is a view: read it with a query (ToListAsync, CountAsync, PageAsync)");
        if (Schema.IsLakeTable(typeof(T)))
            throw new InvalidOperationException($"{typeof(T).Name} is a lake table: read it with a query (ToListAsync, CountAsync, PageAsync)");
        if (Schema.IsHistoryRow(typeof(T)))
            throw new InvalidOperationException("a history is read with a query (ToListAsync, CountAsync, PageAsync)");
        if (_ctx is null)
            throw new InvalidOperationException("a view's query is declared, not read: read the view with ctx.Query<T>()");
        if (_engine != Engine.Store)
            throw new InvalidOperationException(
                "ctx.Analytics().Query<T>(): Get, Find, All, Count, Exists and Aggregate read the workspace's data; use ctx.Query<T>(), or a LINQ read (ToListAsync, CountAsync)");
        return _db ??= new Db<T>(_ctx, Schema.ForeignPlugin(typeof(T)) ?? "");
    }

    /// <summary>A row by id (the repository's read); 404 <c>not_found</c> when there is none or it is soft-deleted.</summary>
    public GetRow<T> Get(Id<T> id, bool withDeleted = false) => Repository().Get(id, withDeleted);

    /// <summary>The rows matching <paramref name="filter"/> (the repository's read): order, limit and page them.</summary>
    public Find<T> Find(Filter<T>? filter = null) => Repository().Find(filter);

    /// <summary>Every row (at most 1,000).</summary>
    public Task<IReadOnlyList<Stored<T>>> All() => Repository().All();

    /// <summary>The rows matching <paramref name="filter"/> (every row without one). A LINQ query counts with <c>CountAsync()</c>.</summary>
    public Task<long> Count(Filter<T>? filter = null) => Repository().Count(filter);

    public Task<bool> Exists(Filter<T> filter) => Repository().Exists(filter);

    /// <summary>Groups of the rows matching <paramref name="filter"/>, as <see cref="Db{T}.Aggregate"/>.</summary>
    public Aggregate<T> Aggregate(Filter<T>? filter = null) => Repository().Aggregate(filter);
}

/// <summary>A query after an operator: a node of its expression tree.</summary>
internal sealed class QueryNode<T>(QueryProvider provider, Expression expression) : IOrderedQueryable<T>
{
    public Type ElementType => typeof(T);
    public Expression Expression { get; } = expression;
    public IQueryProvider Provider { get; } = provider;
    public IEnumerator<T> GetEnumerator() => throw QueryProvider.NotEnumerable();
    IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();
}

internal sealed class QueryProvider(Ctx? ctx) : IQueryProvider
{
    internal Ctx? Ctx { get; } = ctx;

    public IQueryable CreateQuery(Expression expression) =>
        (IQueryable)Activator.CreateInstance(
            typeof(QueryNode<>).MakeGenericType(Translator.ElementOf(expression.Type)), this, expression)!;

    public IQueryable<TElement> CreateQuery<TElement>(Expression expression) => new QueryNode<TElement>(this, expression);

    public object Execute(Expression expression) => throw NotEnumerable();
    public TResult Execute<TResult>(Expression expression) => throw NotEnumerable();

    internal static NotSupportedException NotEnumerable() =>
        new("a query is read with ToListAsync(), FirstOrDefaultAsync(), CountAsync() or PageAsync(), never enumerated");
}

/// <summary>The reads of a query (<c>ctx.Query&lt;T&gt;()</c> and the LINQ operators after it).</summary>
public static class QueryReads
{
    /// <summary>Every row (at most 1,000, or <c>Take(n)</c>: 422 <c>query_rows_limit</c> past them).</summary>
    public static async Task<List<T>> ToListAsync<T>(this IQueryable<T> query,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0)
    {
        var (plan, r) = await Read(query, QueryRead.Types.Mode.All, null, file, line);
        return Rows<T>(plan, r);
    }

    /// <summary><see cref="ToListAsync{T}"/>, with when its data is from.</summary>
    public static async Task<QueryAnswer<List<T>>> ToListAsOfAsync<T>(this IQueryable<T> query,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0)
    {
        var (plan, r) = await Read(query, QueryRead.Types.Mode.All, null, file, line);
        var (asOf, generation) = Stamp(r.DbRows?.AsOf ?? 0, r.DbRows?.Generation ?? 0);
        return new QueryAnswer<List<T>>(Rows<T>(plan, r), asOf, generation);
    }

    /// <summary>The first row, or null.</summary>
    public static async Task<T?> FirstOrDefaultAsync<T>(this IQueryable<T> query,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0)
    {
        var (plan, r) = await Read(query, QueryRead.Types.Mode.First, null, file, line);
        return Rows<T>(plan, r).FirstOrDefault();
    }

    /// <summary>How many rows (or groups) the query has.</summary>
    public static async Task<long> CountAsync<T>(this IQueryable<T> query,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0)
    {
        return (await query.CountAsOfAsync(file, line)).Rows;
    }

    /// <summary><see cref="CountAsync{T}"/>, with when its data is from.</summary>
    public static async Task<QueryAnswer<long>> CountAsOfAsync<T>(this IQueryable<T> query,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0)
    {
        var (_, r) = await Read(query, QueryRead.Types.Mode.Count, null, file, line);
        var c = r.DbCount ?? throw PluginError.Unavailable("the platform answered a count with something else");
        var (asOf, generation) = Stamp(c.AsOf, c.Generation);
        return new QueryAnswer<long>((long)c.Count, asOf, generation);
    }

    /// <summary>A page (<c>Take(n)</c>, at most 100) after the token of the previous page: keyset over the order and the keys.</summary>
    public static async Task<QueryPage<T>> PageAsync<T>(this IQueryable<T> query, string? token = null,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0)
    {
        var (plan, r) = await Read(query, QueryRead.Types.Mode.Page, token, file, line);
        var next = r.DbRows?.NextPageToken;
        var (asOf, generation) = Stamp(r.DbRows?.AsOf ?? 0, r.DbRows?.Generation ?? 0);
        return new QueryPage<T>(Rows<T>(plan, r), string.IsNullOrEmpty(next) ? null : next) { AsOf = asOf, Generation = generation };
    }

    /// <summary>When an answer's data is from: unix milliseconds, 0 for an answer that is not analytic.</summary>
    private static (DateTimeOffset?, ulong?) Stamp(long asOf, ulong generation) =>
        asOf == 0 ? (null, null) : (DateTimeOffset.FromUnixTimeMilliseconds(asOf), generation);

    /// <summary>The query tree a query sends (<c>celobus.v1.Query</c>), in the protobuf JSON mapping.</summary>
    public static string TreeJson<T>(this IQueryable<T> query) => JsonFormatter.Default.Format(Translator.Translate(query.Expression).Tree);

    private static async Task<(Translated, SyscallResult)> Read<T>(IQueryable<T> query, QueryRead.Types.Mode mode,
        string? token, string file, int line)
    {
        if (query.Provider is not QueryProvider provider)
            throw new InvalidOperationException("not a query of ctx.Query<T>()");
        Translated plan;
        try
        {
            plan = Translator.Translate(query.Expression);
        }
        catch (NotSupportedException e)
        {
            throw new PluginError(500, "query_invalid", e.Message);
        }
        var ctx = provider.Ctx
                  ?? throw new InvalidOperationException("a view's query is declared, not read: read the view with ctx.Query<T>()");
        var r = await ctx.Syscall(new Syscall
        {
            Query = new QueryRead
            {
                RequestId = ctx.Request.Id,
                Tx = ctx.Tx,
                Query = plan.Tree,
                Mode = mode,
                PageToken = token ?? "",
                AsSystem = ctx.System,
                Source = $"{file}:{line}",
                Engine = plan.Engine,
            },
        });
        return (plan, r);
    }

    private static List<T> Rows<T>(Translated plan, SyscallResult r)
    {
        var rows = r.DbRows ?? throw PluginError.Unavailable("the platform answered a query with something else");
        var out_ = new List<T>(rows.Rows.Count);
        foreach (var raw in rows.Rows)
        {
            using var doc = JsonDocument.Parse(raw.Memory);
            out_.Add((T)plan.Read(doc.RootElement.Clone(), raw)!);
        }
        return out_;
    }
}
