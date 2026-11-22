// Queries: named calls of the plugin's read-only `ui` endpoints that screens, overlays, badges and home widgets read.
using System.Linq.Expressions;
using System.Text.Json.Nodes;

namespace Celobus.Sdk.Ui;

/// <summary>Starts a query definition: <c>Query.Call(ListOrders.Ref).Arg(..).Rows(r =&gt; r.Orders).Paged().Follow(Orders)</c>.</summary>
public static class Query
{
    /// <summary>A call of one of the plugin's read-only <c>ui</c> endpoints.</summary>
    public static QueryDef<TReq, TRes, TRes> Call<TReq, TRes>(EndpointRef<TReq, TRes> endpoint)
        where TReq : class where TRes : class => new(endpoint.Name);
}

/// <summary>What is declared so far (shared by <c>QueryDef.Rows</c>, which only changes the row type).</summary>
internal sealed class QuerySpec(string endpoint)
{
    public string Endpoint { get; } = endpoint;
    public List<JsonObject> Args { get; } = [];
    public string? Rows { get; set; }
    public bool Paged { get; set; }
    public JsonObject? Feed { get; set; }
    public List<FollowKey> Follow { get; } = [];
    public int? Poll { get; set; }
    public int? MaxAge { get; set; }
    public Cond? When { get; set; }
    public JsonObject? Older { get; set; }
    public int? DebounceMs { get; set; }
    public List<JsonObject> Join { get; } = [];
}

/// <summary>A query definition over the endpoint's request and response; <typeparamref name="TRow"/> is what its rows are.</summary>
public sealed class QueryDef<TReq, TRes, TRow>
{
    private readonly QuerySpec _s;

    internal QueryDef(string endpoint) : this(new QuerySpec(endpoint)) { }

    private QueryDef(QuerySpec spec) { _s = spec; }

    /// <summary>Sets a request field to a value (a literal converts).</summary>
    public QueryDef<TReq, TRes, TRow> Arg<T>(Expression<Func<TReq, T>> field, Val<T> value)
    {
        _s.Args.Add(Args<TReq>.Of(FieldPath.Of(field), Val<T>.Json(value)));
        return this;
    }

    /// <summary>Sets an optional request field to a value of its inner type.</summary>
    public QueryDef<TReq, TRes, TRow> Arg<T>(Expression<Func<TReq, T?>> field, IVal<T> value) where T : struct
    {
        _s.Args.Add(Args<TReq>.Of(FieldPath.Of(field), Val<T>.Json(value)));
        return this;
    }

    /// <summary>The repeated field tables, lists and charts iterate.</summary>
    public QueryDef<TReq, TRes, TRow2> Rows<TRow2>(Expression<Func<TRes, IEnumerable<TRow2>>> field)
    {
        _s.Rows = FieldPath.Of(field);
        return new QueryDef<TReq, TRes, TRow2>(_s);
    }

    /// <summary><c>page_token</c>/<c>page_size</c> in the request, <c>next_page_token</c> in the response.</summary>
    public QueryDef<TReq, TRes, TRow> Paged()
    {
        _s.Paged = true;
        return this;
    }

    /// <summary>Re-run on the topic's invalidations (any key: the topic; one key: <c>Order.Key(..)</c>).</summary>
    public QueryDef<TReq, TRes, TRow> Follow(FollowKey topic)
    {
        _s.Follow.Add(topic);
        return this;
    }

    /// <summary>An append-only feed: <paramref name="after"/> takes the last cursor, <paramref name="cursor"/> gives it, rows are deduplicated by <paramref name="key"/>.</summary>
    public QueryDef<TReq, TRes, TRow> Feed<TA, TC, TK>(Expression<Func<TReq, TA>> after, Expression<Func<TRes, TC>> cursor,
        Expression<Func<TRow, TK>> key)
    {
        _s.Feed = new JsonObject
        {
            ["after"] = FieldPath.Of(after),
            ["cursor"] = FieldPath.Of(cursor),
            ["key"] = FieldPath.Of(key),
        };
        return this;
    }

    /// <summary>Re-run every <paramref name="seconds"/> (at least 15).</summary>
    public QueryDef<TReq, TRes, TRow> Poll(int seconds)
    {
        _s.Poll = seconds;
        return this;
    }

    /// <summary>The oldest the data may get while shown (seconds).</summary>
    public QueryDef<TReq, TRes, TRow> MaxAge(int seconds)
    {
        _s.MaxAge = seconds;
        return this;
    }

    /// <summary>Runs only while <paramref name="when"/> holds (a record panel reads once a row is selected).</summary>
    public QueryDef<TReq, TRes, TRow> When(Cond when)
    {
        _s.When = when;
        return this;
    }

    /// <summary>
    /// The page before the rows (catalog 3): the same endpoint read again with <paramref name="arg"/> set to the answer's
    /// <paramref name="token"/>; a <c>Ui.Stream</c> loads older pages with it.
    /// </summary>
    public QueryDef<TReq, TRes, TRow> Older<TT, TA>(Expression<Func<TRes, TT>> token, Expression<Func<TReq, TA>> arg)
    {
        _s.Older = new JsonObject { ["token"] = FieldPath.Of(token), ["arg"] = FieldPath.Of(arg) };
        return this;
    }

    /// <summary>
    /// Arguments that change wait this long before the read (ms, at most 2000; catalog 3): a preview of what is typed. An
    /// answer replaces the previous one only while its arguments are still current.
    /// </summary>
    public QueryDef<TReq, TRes, TRow> DebounceMs(int ms)
    {
        _s.DebounceMs = ms;
        return this;
    }

    /// <summary>
    /// A lookup (catalog 3): each row reads the item of the response's <paramref name="rows"/> whose
    /// <paramref name="key"/> equals the row's <paramref name="by"/>, through <paramref name="via"/>
    /// (<c>who.F(p =&gt; p.Name)</c> is the row's <c>who.name</c>):
    /// <c>.Rows(v =&gt; v.Messages).Join(who, v =&gt; v.People, p =&gt; p.Id, m =&gt; m.By)</c>.
    /// </summary>
    public QueryDef<TReq, TRes, TRow> Join<TJoin, TK>(Via<TRow, TJoin> via, Expression<Func<TRes, IEnumerable<TJoin>>> rows,
        Expression<Func<TJoin, TK>> key, Expression<Func<TRow, TK>> by)
    {
        _s.Join.Add(new JsonObject
        {
            ["as"] = via.Name,
            ["rows"] = FieldPath.Of(rows),
            ["key"] = FieldPath.Of(key),
            ["by"] = FieldPath.Of(by),
        });
        return this;
    }

    /// <summary>The document's query; <paramref name="minIntervalMs"/>: the followed topics' default (nav and home).</summary>
    internal JsonObject Build(string name, int? minIntervalMs = null)
    {
        var o = new JsonObject { ["name"] = name, ["endpoint"] = _s.Endpoint };
        if (_s.Args.Count > 0) o["args"] = new JsonArray([.. _s.Args.Select(a => a.DeepClone())]);
        if (_s.Rows is not null) o["rows"] = _s.Rows;
        if (_s.Paged) o["paged"] = true;
        if (_s.Feed is not null) o["feed"] = _s.Feed.DeepClone();
        if (_s.Follow.Count > 0)
            o["follow"] = new JsonArray([.. _s.Follow.Select(f =>
            {
                var j = new JsonObject { ["topic"] = f.Topic };
                if (f.Key.Count > 0) j["key"] = new JsonArray([.. f.Key.Select(k => k.DeepClone())]);
                if (minIntervalMs is { } ms) j["min_interval_ms"] = ms;
                return (JsonNode)j;
            })]);
        if (_s.Poll is { } poll) o["poll"] = poll;
        if (_s.MaxAge is { } age) o["max_age"] = age;
        if (_s.When is not null) o["when"] = _s.When.Node.DeepClone();
        if (_s.Older is not null) o["older"] = _s.Older.DeepClone();
        if (_s.DebounceMs is { } debounce) o["debounce_ms"] = debounce;
        if (_s.Join.Count > 0) o["join"] = new JsonArray([.. _s.Join.Select(j => j.DeepClone())]);
        return o;
    }
}

/// <summary>A declared query of a screen, an overlay or the document: read its fields, iterate its rows.</summary>
public sealed class Q<TRes, TRow>
{
    public string Name { get; }

    internal Q(string name) { Name = name; }

    /// <summary>A field of the response (one record).</summary>
    public Val<T> Field<T>(Expression<Func<TRes, T>> field) => Data<T>(FieldPath.Of(field));

    /// <summary>A field by its path (<c>customer.name</c>), unchecked.</summary>
    public Val<T> Path<T>(string path) => Data<T>(path);

    private Val<T> Data<T>(string field) => new("data", new JsonObject { ["query"] = Name, ["field"] = field });
}
