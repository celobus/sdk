// Typed reads across entities: ctx.Query<T>() is an IQueryable<T>. A LINQ query over it (joins, a left join through
// `into .. DefaultIfEmpty()`, where, group by, orderby, select, Take) is translated from its expression tree into the
// query tree of the session (`celobus.v1.Query`, Translator.cs), never SQL, and read with ToListAsync,
// FirstOrDefaultAsync, CountAsync or PageAsync. The analyzer CELOBUS060 reports at compile time what the translation
// refuses (a call to an arbitrary method inside the query).

using System.Collections;
using System.Linq.Expressions;
using System.Runtime.CompilerServices;
using System.Text.Json;
using Celobus.V1;
using Google.Protobuf;

namespace Celobus.Sdk;

/// <summary>A row's label as <c>Query.Label(ref)</c> selects it: a person's (or a group's) name and avatar, another row's label column.</summary>
public sealed record Label(string Id, string Text, string? AvatarUrl);

/// <summary>A page of a query's rows and the token of the next page (null: the last).</summary>
public sealed record QueryPage<T>(IReadOnlyList<T> Rows, string? Next);

/// <summary>A root of a query: the entity and the context it reads as.</summary>
internal interface IQueryRoot
{
    Type Entity { get; }
    Ctx Ctx { get; }
}

/// <summary>
/// A typed read across entities from the root <typeparamref name="T"/>: the plugin's own entity, a dependency's exported
/// one (which keeps the repository's reads: <see cref="Get"/>, <see cref="Find"/>, <see cref="All"/>,
/// <see cref="Count"/>, <see cref="Exists"/>, <see cref="Aggregate"/>), or the directory's (<c>Platform.User</c>,
/// <c>Platform.Group</c>, <c>Platform.GroupMember</c>, <c>Platform.RoleAssignment</c>).
/// </summary>
public sealed class CelobusQuery<T> : IOrderedQueryable<T>, IQueryRoot where T : class
{
    private readonly Ctx _ctx;
    private Db<T>? _db;

    internal CelobusQuery(Ctx ctx)
    {
        _ctx = ctx;
        Provider = new QueryProvider(ctx);
        Expression = Expression.Constant(this);
    }

    public Type ElementType => typeof(T);
    public Expression Expression { get; }
    public IQueryProvider Provider { get; }
    Type IQueryRoot.Entity => typeof(T);
    Ctx IQueryRoot.Ctx => _ctx;

    public IEnumerator<T> GetEnumerator() => throw QueryProvider.NotEnumerable();
    IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();

    private Db<T> Repository()
    {
        if (Translator.IsDirectory(typeof(T)))
            throw new InvalidOperationException($"{typeof(T).Name} is the directory's: read it with a query (ToListAsync)");
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

internal sealed class QueryProvider(Ctx ctx) : IQueryProvider
{
    internal Ctx Ctx { get; } = ctx;

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
        var (_, r) = await Read(query, QueryRead.Types.Mode.Count, null, file, line);
        return (long)(r.DbCount ?? throw PluginError.Unavailable("the platform answered a count with something else")).Count;
    }

    /// <summary>A page (<c>Take(n)</c>, at most 100) after the token of the previous page: keyset over the order and the keys.</summary>
    public static async Task<QueryPage<T>> PageAsync<T>(this IQueryable<T> query, string? token = null,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0)
    {
        var (plan, r) = await Read(query, QueryRead.Types.Mode.Page, token, file, line);
        var next = r.DbRows?.NextPageToken;
        return new QueryPage<T>(Rows<T>(plan, r), string.IsNullOrEmpty(next) ? null : next);
    }

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
        var ctx = provider.Ctx;
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
