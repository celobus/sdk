// An entity's history in the workspace's Data Lake: every change of the columns the entity keeps, each its own row with
// when it was made and by whom, kept by the platform without a time limit in a layer the plugin declares.
//
//   [Entity(SoftDelete = true), History("audit")]          // or History("audit", All = true, Unmasked = true)
//   public sealed record Ticket(Id<Ticket> Id, string Title, [History] TicketStatus Status,
//       [History] Id<Platform.User>? Assignee, string Body);
//
//   Plugin.Create("helpdesk").Layer(Audit).Entity<Ticket>()
//       .HistoryRead<Ticket>(Analyst)          // who reads it: holders of these roles, every row, every version
//       .HistoryReadAsSystem<Ticket>();        // and the plugin's own system context (a model over it needs it)
//
//   var trail = await (from h in ctx.Lake().History<Ticket>()
//                      where h.Row.Id == id
//                      orderby h.ValidFrom
//                      select h).ToListAsync();        // HistoryRow<Ticket>: h.Change, h.ChangedBy, h.Row.Status, ..
//   var perDay = await (from h in ctx.Lake().History<Ticket>().EachDay()
//                       group h by h.Day into g
//                       select new { Day = g.Key, Open = g.LongCount() }).ToListAsync();
//
// What is kept. The columns marked [History], or with [History("audit", All = true)] every column (the ones a later
// version adds too) but the encrypted ones, the lists and messages and, unless Unmasked = true is said too, the masked
// ones (the lake has no masks; `celobus plugin check` lists what All left out). A kept column is a scalar, an enum or a
// ref.
//
// A row is a change: the entity's Id, the kept columns as they were after it (HistoryRow.Row; a row written before a
// column was kept has no value of it: HistoryRow.Has), ValidFrom (when it was made), ValidTo (the row's next change;
// null for the current version; equal to ValidFrom on a delete), Change (baseline, insert, update, delete, restore) and
// who made it: ChangedBy (the person or service account, null for a system context or a job without one),
// ChangedByKind (person, service, job, system, platform, anonymous) and ChangedByPlugin (the plugin whose call made
// it). Two changes are never merged; an update that changes no kept column is none; a soft delete is a delete, undoing
// it a restore. When the history begins (the install of a version that keeps it, or an upgrade that keeps a new
// column) every row is written as it is then, as baseline; on a large table in the background, and until it is whole
// a read of the history is refused (unavailable, with how far it is). Changes reach the lake within seconds, in order.
//
// Per day. EachDay() reads one row per day each version was in force at the day's last moment, in the workspace's time
// zone, with its Day: from the day of ValidFrom to the day before that of ValidTo, to today for the current version (a
// version that began and ended within one day, and a delete, have none). So counting the rows of a day counts what was
// there at its end.
//
// Who reads it. The holders of the roles of .HistoryRead<T>(..), every row of it, every version, without the entity's
// access rules or masks; the plugin's own system context only with .HistoryReadAsSystem<T>(); another plugin only when
// the owner exports it (.HistoryExport<T>()) and that plugin declares it (.HistoryDependency<T>()). A history is read in
// the lake only, with the lake's limits; a model reads it too (LakeModel.History<T>()). Nothing erases one row's history (a
// delete leaves every past version), and ChangedBy holds people's ids.
using System.Linq.Expressions;
using System.Reflection;
using System.Text.Json;
using Celobus.V1;
using Google.Protobuf;

namespace Celobus.Sdk;

/// <summary>
/// A change of a row of <typeparamref name="T"/>, as a history read answers it (<c>ctx.Lake().History&lt;T&gt;()</c>).
/// In a query, <c>h.Row.Status</c> names a kept column and <c>h.ValidFrom</c> .. <c>h.Day</c> the history's own.
/// </summary>
public sealed class HistoryRow<T> where T : class
{
    internal HistoryRow(Id<T> id, T row, IReadOnlySet<string> held, DateTimeOffset validFrom, DateTimeOffset? validTo,
        string change, Id<Platform.User>? changedBy, string changedByKind, string? changedByPlugin, DateOnly? day)
    {
        Id = id;
        Row = row;
        _held = held;
        ValidFrom = validFrom;
        ValidTo = validTo;
        Change = change;
        ChangedBy = changedBy;
        ChangedByKind = changedByKind;
        ChangedByPlugin = changedByPlugin;
        Day = day;
    }

    private readonly IReadOnlySet<string> _held;

    /// <summary>The row's id.</summary>
    public Id<T> Id { get; }

    /// <summary>
    /// The kept columns as they were after the change. A column the history does not keep, or one this change has no
    /// value of (a row written before the column was kept), is its default here: <see cref="Has"/> tells them apart.
    /// </summary>
    public T Row { get; }

    /// <summary>When the change was made.</summary>
    public DateTimeOffset ValidFrom { get; }

    /// <summary>The row's next change; null for the current version; on a delete, the delete's own moment.</summary>
    public DateTimeOffset? ValidTo { get; }

    /// <summary><c>baseline</c>, <c>insert</c>, <c>update</c>, <c>delete</c>, <c>restore</c>.</summary>
    public string Change { get; }

    /// <summary>The person or service account the change was made for; null for a system context or a job without one.</summary>
    public Id<Platform.User>? ChangedBy { get; }

    /// <summary><c>person</c>, <c>service</c>, <c>job</c>, <c>system</c>, <c>platform</c>, <c>anonymous</c>.</summary>
    public string ChangedByKind { get; }

    /// <summary>The plugin whose call made it (this plugin, or another that wrote its rows); null for the platform's own.</summary>
    public string? ChangedByPlugin { get; }

    /// <summary>Read per day (<c>EachDay()</c>): the day.</summary>
    public DateOnly? Day { get; }

    /// <summary>Whether this change has a value of the kept column (<c>h.Has(t =&gt; t.Status)</c>).</summary>
    public bool Has<TV>(Expression<Func<T, TV>> column)
    {
        var body = column.Body is UnaryExpression { NodeType: ExpressionType.Convert } u ? u.Operand : column.Body;
        if (body is not MemberExpression { Member: PropertyInfo p } m || m.Expression != column.Parameters[0])
            throw new ArgumentException("HistoryRow.Has names a column of the row: h.Has(t => t.Status)", nameof(column));
        return _held.Contains(Names.Snake(p.Name));
    }

    /// <summary>A row of the history as the platform sends it (protobuf JSON names).</summary>
    internal static HistoryRow<T> Read(ByteString raw)
    {
        using var doc = JsonDocument.Parse(raw.Memory);
        var json = doc.RootElement;
        if (json.ValueKind != JsonValueKind.Object)
            throw PluginError.Unavailable("platform: a history row that is not an object");
        T row;
        try
        {
            row = JsonSerializer.Deserialize<T>(raw.Span, CelobusJson.Options)
                  ?? throw PluginError.Unavailable("platform: an empty history row");
        }
        catch (JsonException e)
        {
            throw PluginError.Unavailable($"platform: a history row this plugin cannot read: {e.Message}");
        }
        var held = new HashSet<string>();
        foreach (var f in MessageSchema.For(typeof(T)).Fields)
            if (json.TryGetProperty(f.JsonName, out var v) && v.ValueKind != JsonValueKind.Null) held.Add(f.Name);
        string? Text(string name) =>
            json.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
        string Required(string name) =>
            Text(name) ?? throw PluginError.Unavailable($"platform: a history row without {name}");
        static DateTimeOffset Moment(string s) =>
            DateTimeOffset.Parse(s, System.Globalization.CultureInfo.InvariantCulture, System.Globalization.DateTimeStyles.RoundtripKind);
        var id = Text("id") is { Length: > 0 } key ? new Id<T>(key) : default;
        return new HistoryRow<T>(
            id,
            row,
            held,
            Moment(Required("validFrom")),
            Text("validTo") is { } to ? Moment(to) : null,
            Required("change"),
            Text("changedBy") is { Length: > 0 } by ? new Id<Platform.User>(by) : null,
            Required("changedByKind"),
            Text("changedByPlugin"),
            Text("day") is { } day ? DateOnly.ParseExact(day, "yyyy-MM-dd", System.Globalization.CultureInfo.InvariantCulture) : null);
    }
}

/// <summary>A history query read per day (<c>ctx.Lake().History&lt;T&gt;().EachDay()</c>, <c>LakeModel.History&lt;T&gt;().EachDay()</c>).</summary>
public static class HistoryQueries
{
    /// <summary>
    /// One row per day each version was in force at the day's last moment, in the workspace's time zone, with its
    /// <see cref="HistoryRow{T}.Day"/>.
    /// </summary>
    public static CelobusQuery<HistoryRow<T>> EachDay<T>(this CelobusQuery<HistoryRow<T>> history) where T : class =>
        new(history.Context, Engine.Lake, eachDay: true);
}

/// <summary>
/// The sources of a model's query (<c>Plugin.Model(..)</c>): <c>from f in LakeModel.From&lt;StockFileRow&gt;() ..
/// select new Stock(..)</c>. A model's query is declared, never read: its table is refreshed with
/// <c>ctx.Lake().RefreshAsync&lt;T&gt;()</c> and read with <c>ctx.Lake().Query&lt;T&gt;()</c>.
/// </summary>
public static class LakeModel
{
    /// <summary>
    /// A source of a model: a lake table (the plugin's own its system context reads, or another plugin's it declares and
    /// whose owner lets models copy it), or an entity or an analytic view of the plugin (never both in one model).
    /// </summary>
    public static CelobusQuery<T> From<T>() where T : class =>
        new(null, Schema.IsLakeTable(typeof(T)) ? Engine.Lake : Engine.Store);

    /// <summary>
    /// The history of one of the plugin's own entities as a model's root (<c>.EachDay()</c> for one row per day): its
    /// columns are the entity's kept ones (<c>h.Row.Status</c>) and the history's own (<c>h.Day</c>). Such a model reads
    /// the lake: it may join lake tables, never entities. It reads the history as the plugin's system context: the
    /// history must say so (<c>.HistoryReadAsSystem&lt;T&gt;()</c>).
    /// </summary>
    public static CelobusQuery<HistoryRow<T>> History<T>() where T : class => new(null, Engine.Lake);
}

internal static partial class Schema
{
    /// <summary>A row of an entity's history (<c>HistoryRow&lt;T&gt;</c>).</summary>
    public static bool IsHistoryRow(Type t) => t.IsGenericType && t.GetGenericTypeDefinition() == typeof(HistoryRow<>);

    /// <summary>A source of the lake: a lake table, or an entity's history.</summary>
    public static bool IsLakeSource(Type t) => IsLakeTable(t) || IsHistoryRow(t);

    /// <summary>The entity whose history a <c>HistoryRow&lt;T&gt;</c> is.</summary>
    public static Type HistoryOf(Type t) => t.GetGenericArguments()[0];

    /// <summary>The columns every history has besides the entity's <c>id</c> and its kept ones.</summary>
    public static readonly HashSet<string> HistoryColumns =
        ["valid_from", "valid_to", "change", "changed_by", "changed_by_kind", "changed_by_plugin", "day", "seq"];
}
