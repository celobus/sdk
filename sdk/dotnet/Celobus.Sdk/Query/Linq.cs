namespace Celobus.Sdk.Linq;

/// <summary>
/// What a query may say besides LINQ's own operators: labels, full-text search, time buckets, a day relative to the
/// day the query runs, a range (<c>using Celobus.Sdk.Linq;</c>, a namespace of its own so it does not meet the UI's
/// <c>Celobus.Sdk.Ui.Query</c>). Each is a part of a query (<c>ctx.Query&lt;T&gt;()</c>, a view's query), never a
/// method to call.
/// </summary>
public static class Query
{
    /// <summary>
    /// The label of the row a ref names (or of the row of an <c>Id</c>): a person's name and avatar, another row's
    /// label column; null when the ref is empty or names a row the caller may not read. Inside a query only.
    /// </summary>
    public static Label? Label<T>(Id<T> reference) where T : class => throw OutsideQuery();

    /// <summary>The label of a row an optional ref names (see the other overload).</summary>
    public static Label? Label<T>(Id<T>? reference) where T : class => throw OutsideQuery();

    /// <summary>Full-text search of a searchable column (web search syntax: <c>"a phrase"</c>, <c>or</c>, <c>-word</c>). Inside a query only.</summary>
    public static bool Search(string? column, string words) => throw OutsideQuery();

    /// <summary>
    /// A row's system column <c>created_at</c> (an entity's record has no property for it): <c>Query.CreatedAt(o)</c>,
    /// a <c>DateTimeOffset</c> wherever a column stands (<c>Query.Month(Query.CreatedAt(o))</c>). Inside a query only.
    /// </summary>
    public static DateTimeOffset CreatedAt<T>(T row) where T : class => throw OutsideQuery();

    /// <summary>A row's system column <c>updated_at</c> (see <see cref="CreatedAt{T}"/>).</summary>
    public static DateTimeOffset UpdatedAt<T>(T row) where T : class => throw OutsideQuery();

    /// <summary>
    /// The day a moment falls on, in the workspace's time zone (UTC when it has none): a <c>DateOnly</c> wherever a
    /// column stands (compared, grouped, selected, ordered, joined). Inside a query only.
    /// </summary>
    public static DateOnly Day(DateTimeOffset column) => throw OutsideQuery();
    public static DateOnly? Day(DateTimeOffset? column) => throw OutsideQuery();
    public static DateOnly Day(DateOnly column) => throw OutsideQuery();
    public static DateOnly? Day(DateOnly? column) => throw OutsideQuery();

    /// <summary>The first day of the week a moment or a day falls in, from the workspace's first day of the week.</summary>
    public static DateOnly Week(DateTimeOffset column) => throw OutsideQuery();
    public static DateOnly? Week(DateTimeOffset? column) => throw OutsideQuery();
    public static DateOnly Week(DateOnly column) => throw OutsideQuery();
    public static DateOnly? Week(DateOnly? column) => throw OutsideQuery();

    /// <summary>The first day of the month a moment or a day falls in (in the workspace's time zone).</summary>
    public static DateOnly Month(DateTimeOffset column) => throw OutsideQuery();
    public static DateOnly? Month(DateTimeOffset? column) => throw OutsideQuery();
    public static DateOnly Month(DateOnly column) => throw OutsideQuery();
    public static DateOnly? Month(DateOnly? column) => throw OutsideQuery();

    /// <summary>The first day of the quarter a moment or a day falls in (in the workspace's time zone).</summary>
    public static DateOnly Quarter(DateTimeOffset column) => throw OutsideQuery();
    public static DateOnly? Quarter(DateTimeOffset? column) => throw OutsideQuery();
    public static DateOnly Quarter(DateOnly column) => throw OutsideQuery();
    public static DateOnly? Quarter(DateOnly? column) => throw OutsideQuery();

    /// <summary>The first day of the year a moment or a day falls in (in the workspace's time zone).</summary>
    public static DateOnly Year(DateTimeOffset column) => throw OutsideQuery();
    public static DateOnly? Year(DateTimeOffset? column) => throw OutsideQuery();
    public static DateOnly Year(DateOnly column) => throw OutsideQuery();
    public static DateOnly? Year(DateOnly? column) => throw OutsideQuery();

    /// <summary>
    /// Today in the workspace's time zone less <paramref name="days"/> days, worked out when the query runs (so a view
    /// of "the last 30 days" moves with the calendar): compared with a <c>DateOnly</c> column or a bucket as that day,
    /// with a <c>DateTimeOffset</c> column as the first moment of that day (<c>o.PaidAt &gt;= Query.DaysAgo(30)</c>).
    /// </summary>
    public static DaysAgo DaysAgo(int days) => new(days);

    /// <summary>Between <paramref name="from"/> and <paramref name="to"/>, both included (<c>&gt;= from &amp;&amp; &lt;= to</c>).</summary>
    public static bool Between<T>(T column, T from, T to) where T : struct, IComparable<T> => throw OutsideQuery();

    /// <summary>Between two values of an optional column (see the other overload).</summary>
    public static bool Between<T>(T? column, T from, T to) where T : struct, IComparable<T> => throw OutsideQuery();

    /// <summary>
    /// A lake table's text column, without the spaces around it, read as <typeparamref name="T"/> (in a model of the lake
    /// or a lake query only): <c>Query.Parse&lt;decimal&gt;(f.OnHand)</c>; <c>decimal</c>, <c>int</c>, <c>long</c>,
    /// <c>double</c>, <c>bool</c>, <c>DateOnly</c>, <c>DateTimeOffset</c>, <c>Guid</c>. A text that does not read so
    /// fails the query (a model's refresh: 422 <c>lake_model_parse</c>, naming the column; nothing is written). A decimal
    /// reads only within the lake's digits (20 before the point, 18 after), never rounded; an integer only as digits; a
    /// bool as <c>true</c> or <c>false</c>; a date as <c>YYYY-MM-DD</c>; a moment as RFC 3339 with its offset. An absent
    /// text stays absent.
    /// </summary>
    public static T Parse<T>(string? column) where T : struct => throw OutsideQuery();

    internal static NotSupportedException OutsideQuery() =>
        new("Query.Label, Query.Search, Query.CreatedAt, Query.UpdatedAt, the buckets (Query.Day .. Query.Year), Query.Between and Query.Parse are parts of a query (ctx.Query<T>()), not methods to call");
}

/// <summary>
/// A day relative to the day the query runs (<see cref="Query.DaysAgo"/>): a column (a <c>DateOnly</c>, a bucket, a
/// <c>DateTimeOffset</c>) compares with it inside a query; it is never compared in the plugin.
/// </summary>
public readonly struct DaysAgo(int days) : IEquatable<DaysAgo>
{
    /// <summary>How many days before today.</summary>
    public int Days { get; } = days;

    public bool Equals(DaysAgo other) => Days == other.Days;
    public override bool Equals(object? obj) => obj is DaysAgo d && Equals(d);
    public override int GetHashCode() => Days;

    public static bool operator ==(DateOnly column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator !=(DateOnly column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator <(DateOnly column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator <=(DateOnly column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator >(DateOnly column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator >=(DateOnly column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator ==(DateTimeOffset column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator !=(DateTimeOffset column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator <(DateTimeOffset column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator <=(DateTimeOffset column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator >(DateTimeOffset column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator >=(DateTimeOffset column, DaysAgo day) => throw Query.OutsideQuery();
    public static bool operator ==(DaysAgo day, DateOnly column) => throw Query.OutsideQuery();
    public static bool operator !=(DaysAgo day, DateOnly column) => throw Query.OutsideQuery();
    public static bool operator <(DaysAgo day, DateOnly column) => throw Query.OutsideQuery();
    public static bool operator <=(DaysAgo day, DateOnly column) => throw Query.OutsideQuery();
    public static bool operator >(DaysAgo day, DateOnly column) => throw Query.OutsideQuery();
    public static bool operator >=(DaysAgo day, DateOnly column) => throw Query.OutsideQuery();
    public static bool operator ==(DaysAgo day, DateTimeOffset column) => throw Query.OutsideQuery();
    public static bool operator !=(DaysAgo day, DateTimeOffset column) => throw Query.OutsideQuery();
    public static bool operator <(DaysAgo day, DateTimeOffset column) => throw Query.OutsideQuery();
    public static bool operator <=(DaysAgo day, DateTimeOffset column) => throw Query.OutsideQuery();
    public static bool operator >(DaysAgo day, DateTimeOffset column) => throw Query.OutsideQuery();
    public static bool operator >=(DaysAgo day, DateTimeOffset column) => throw Query.OutsideQuery();
    public static bool operator ==(DaysAgo a, DaysAgo b) => a.Equals(b);
    public static bool operator !=(DaysAgo a, DaysAgo b) => !a.Equals(b);
}
