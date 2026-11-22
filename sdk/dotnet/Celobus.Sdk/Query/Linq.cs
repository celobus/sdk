namespace Celobus.Sdk.Linq;

/// <summary>
/// What a query may say besides LINQ's own operators: labels, full-text search (<c>using Celobus.Sdk.Linq;</c>, a
/// namespace of its own so it does not meet the UI's <c>Celobus.Sdk.Ui.Query</c>).
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

    private static NotSupportedException OutsideQuery() =>
        new("Query.Label and Query.Search are parts of a query (ctx.Query<T>()), not methods to call");
}
