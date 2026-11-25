// The workspace's Data Lake: tables of rows kept for analysis and for BI tools, grouped in layers the plugin names
// (`raw`, `core`, `mart`: the platform names none). A plugin declares its layers and its lake tables, writes them from
// its code (usually a job: ReplaceAsync, AppendAsync, MergeAsync) and reads them with LINQ, as its entities.
//
//   public static readonly Layer Raw = new("raw") { Title = "As it arrived" };
//   public static readonly Layer Core = new("core") { Title = "Cleaned" };
//
//   [LakeTable(Layer = "raw", Name = "stock_file", Partition = [nameof(LoadedOn)]), Describe("Stock as the ERP sends it.")]
//   public sealed record StockFileRow(DateOnly LoadedOn, string Sku, string Warehouse, string OnHand);
//
//   [LakeTable(Layer = "core", Partition = [nameof(Day)], Key = [nameof(Day), nameof(Sku), nameof(Warehouse)])]
//   public sealed record Stock(DateOnly Day, string Sku, string Warehouse, decimal OnHand, long? Reserved);
//
//   Plugin.Create("retail").Role(Analyst).Layer(Raw).Layer(Core)
//       .LakeTable<StockFileRow>().LakeTable<Stock>()
//       .LakeRead<Stock>(Analyst)        // who reads it: holders of these roles, every row
//       .LakeReadAsSystem<Stock>()       // and the plugin's own system context
//       .LakeExport<Stock>();            // and other plugins that declare it (.LakeDependency<T>())
//
//   await ctx.Lake().Table<Stock>().MergeAsync(rows);
//   await ctx.Lake().Table<StockFileRow>().ReplaceAsync(Filter<StockFileRow>.Eq(x => x.LoadedOn, day), rows);
//   await ctx.Lake().Table<StockFileRow>().AppendAsync(rows, key: $"erp-{day}");    // the key: required outside a run
//   var low = await (from s in ctx.Lake().Query<Stock>() where s.OnHand < min orderby s.OnHand select s)
//       .Take(50).ToListAsync();
//
// Writes. A plugin writes only its own tables. Each write is one commit: its rows are all there, or none, and a reader
// never sees half of it. A write holds at most 1,000,000 rows (422 `lake_rows_limit`, refused before anything is
// sent; larger loads go partition by partition) in at most 1,000 partitions (422 `lake_partitions_limit`: write fewer
// partitions at a time); the SDK sends them in chunks of at most 10,000 rows and 4 MiB (a row
// over 4 MiB is 422 `bulk_limit`). The writes of one table run one at a time across the workspace (one that waits more
// than 60 s is 503 `lake_busy`); a plugin commits at most 120 writes a minute (`lake_busy` past them: write fewer,
// larger ones) and stages at most two at once; a commit runs at most 50 s (422 `lake_time_limit`, nothing written).
//  * ReplaceAsync(filter, rows) swaps the partitions the filter names for the rows: the filter compares partition
//    columns only, and a row it does not select refuses the write (400 `invalid`). A table without partition columns
//    is replaced whole (Filter<T>.All). Run again, it leaves the same table.
//  * AppendAsync(rows, key) adds the rows once per write key: a write whose key the table has had within 32 days adds
//    nothing (Written.Skipped). In a job's run the key is, by default, the run's id and the number of the append in the
//    run, so an attempt that runs again appends nothing twice; elsewhere an append without a key is refused (400
//    `invalid`) before anything is sent: a write may answer an error although it was written, and only its key makes
//    running it again safe.
//  * MergeAsync(rows) writes each row over the row with the same declared key, or adds it; two rows of one write with
//    the same key refuse it. Only on a table with a key.
// A row the table cannot hold refuses the whole write (400 `invalid`): a decimal with more than 20 digits before the
// point or 18 after, a text, bytes or JSON value over 1 MiB.
//
// Reads. ctx.Lake().Query<T>() takes the joins, filters, groups, aggregates, time buckets, orders, Take and pages of
// ctx.Query<T>(), over lake tables only (the plugin's own, or another plugin's it declares with .LakeDependency<T>()).
// Whoever holds one of a table's read roles reads every row of it; there are no row rules (anyone else: 403
// `permission_denied`). The plugin's system context (ctx.AsSystem(..)) reads only its own tables declared with
// .LakeReadAsSystem<T>() (403 `capability_missing` otherwise). A query runs at most 10 s (422 `lake_time_limit`),
// within the lake's memory (422 `lake_memory_limit`), four of the workspace at once (503 `lake_busy` after a short
// wait); at most 1,000 rows or pages of 100. Never inside a transaction. Decimals read back without trailing zeros
// (1.5). 503 `unavailable` when the workspace has no lake.
//
// Models. A lake table may be computed by the platform from other tables, when the plugin asks: its model
// (.Model(query)) is LINQ over LakeModel.From<T>() whose row is the table's, over lake tables (the plugin's own its system
// context reads, or another plugin's it declares and whose owner lets models copy it: .LakeExportToModels<T>()), or over
// the plugin's own entities and analytic views, or over the history of one of its entities (LakeModel.History<T>()); never
// lake tables with entities or views. ctx.Lake().RefreshAsync<T>() runs it and replaces the whole table with its rows
// (with a partition filter, only the partitions it names), in one commit; the rows never pass through the plugin. A
// refresh holds at most 1,000,000 rows (422 `lake_model_rows_limit`: refresh by partition); a text it parses
// (Query.Parse<decimal>(f.OnHand)) that does not read as its type fails it, naming the column (422
// `lake_model_parse`); after the model's declaration changed, a refresh of partitions answers how many partitions still
// hold the earlier declaration's rows (Refreshed.StalePartitions): the table mixes them until it is 0. A model over entities or views reads them as the plugin's system context
// (its authz:bypass of read must cover every entity under it: 403 `capability_missing`), from the workspace's analytic
// copy, and the read is audited: 503 `unavailable` while the copy is not ready or is older than what the table was last
// computed from (it waits up to 10 s for the copy to catch up: retry later); Refreshed.AsOf says when its data is from.
//
//   .LakeTable<Stock>()
//   .Model(from f in LakeModel.From<StockFileRow>()
//          where f.Sku != ""
//          select new Stock(f.LoadedOn, f.Sku, Query.Parse<decimal>(f.OnHand)))
//   var r = await ctx.Lake().RefreshAsync(Filter<Stock>.Eq(x => x.Day, day));    // r.Rows, r.AsOf
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text.Json;
using Celobus.V1;
using Google.Protobuf;

namespace Celobus.Sdk;

/// <summary>A layer of the lake: a name (snake_case, at most 32 characters) and a title: <c>new("raw") { Title = "As it arrived" }</c>.</summary>
public sealed record Layer(string Name)
{
    public string Title { get; init; } = "";
}

/// <summary>
/// The row of a lake table: a record whose public properties are its columns (scalars, enums and refs, never a list, a
/// message or an encrypted field), declared with <c>.LakeTable&lt;T&gt;()</c>, written with
/// <c>ctx.Lake().Table&lt;T&gt;()</c> and read with <c>ctx.Lake().Query&lt;T&gt;()</c>. Its <c>[Describe]</c> is the
/// table's description. With <see cref="Plugin"/>, another plugin's exported table this plugin reads
/// (<c>[LakeTable(Plugin = "retail", Name = "stock")]</c>, declared with <c>.LakeDependency&lt;T&gt;()</c>).
/// </summary>
[AttributeUsage(AttributeTargets.Class, Inherited = false)]
public sealed class LakeTableAttribute : Attribute
{
    /// <summary>The name of the layer the table belongs to (one the plugin declares with <c>.Layer(..)</c>).</summary>
    public string? Layer { get; set; }

    /// <summary>The table's name (snake_case, at most 32 characters); null: the type's name in snake_case.</summary>
    public string? Name { get; set; }

    /// <summary>Another plugin's table: its owner's plugin id (null: a table of this plugin).</summary>
    public string? Plugin { get; set; }

    /// <summary>
    /// The partition columns, in order (at most 4; <c>nameof(Day)</c>): what a replace swaps. Required <c>string</c>,
    /// <c>bool</c>, <c>int</c>, <c>long</c>, <c>Guid</c>, <c>DateOnly</c> or enum columns.
    /// </summary>
    public string[] Partition { get; set; } = [];

    /// <summary>The key columns (at most 8): what a merge writes over. Partition-like columns, a timestamp or a ref.</summary>
    public string[] Key { get; set; } = [];
}

/// <summary>What a write did: the rows it carried, and whether an append was skipped (its key was written before).</summary>
public sealed record Written(long Rows, bool Skipped);

/// <summary>
/// What a refresh did: the rows it wrote, and when the data of the analytic copy a model over entities or views read is
/// from (null for a model over lake tables; for a model over a history, the moment the lake had the history up to).
/// </summary>
/// <param name="Rows">The rows it wrote.</param>
/// <param name="AsOf">When the analytic copy's data a model over entities or views read is from (null over lake tables).</param>
/// <param name="StalePartitions">
/// After a refresh of partitions: how many partitions the table still holds that an earlier declaration of the model
/// computed (0: every row is this declaration's). The table mixes the two declarations' rows until it is 0, and its
/// readers see it so.
/// </param>
public sealed record Refreshed(long Rows, DateTimeOffset? AsOf, long StalePartitions);

/// <summary>
/// The workspace's Data Lake (<c>ctx.Lake()</c>): the writes of the plugin's lake tables, and LINQ over lake tables.
/// </summary>
public sealed class Lake
{
    private readonly Ctx _ctx;

    internal Lake(Ctx ctx) => _ctx = ctx;

    /// <summary>The writes of one of the plugin's lake tables (a <c>[LakeTable]</c> record).</summary>
    public LakeWriter<T> Table<T>() where T : class => new(_ctx);

    /// <summary>
    /// A query of lake tables from the root <typeparamref name="T"/> (the plugin's own, or another plugin's it declares):
    /// the LINQ of <c>ctx.Query&lt;T&gt;()</c>, read with <c>ToListAsync</c>, <c>FirstOrDefaultAsync</c>,
    /// <c>CountAsync</c> or <c>PageAsync</c>. Every source of it starts from <c>ctx.Lake().Query&lt;T&gt;()</c>.
    /// </summary>
    public CelobusQuery<T> Query<T>() where T : class
    {
        if (!Schema.IsLakeTable(typeof(T)))
            throw new InvalidOperationException($"{typeof(T).Name} has no [LakeTable] attribute: ctx.Lake() reads lake tables only");
        return new(_ctx, Engine.Lake);
    }

    /// <summary>
    /// A query of the history of the entity <typeparamref name="T"/> (History.cs): its changes as
    /// <see cref="HistoryRow{T}"/>, filtered, grouped and ordered by the entity's kept columns (<c>h.Row.Status</c>) and
    /// the history's own (<c>h.ValidFrom</c>); <c>.EachDay()</c> reads it per day. The plugin's own entity, or another
    /// plugin's it declares with <c>.HistoryDependency&lt;T&gt;()</c>.
    /// </summary>
    public CelobusQuery<HistoryRow<T>> History<T>() where T : class
    {
        if (!Schema.IsEntity(typeof(T)) && !Schema.IsForeignEntity(typeof(T)))
            throw new InvalidOperationException($"{typeof(T).Name} has no [Entity] attribute: a history is an entity's");
        return new(_ctx, Engine.Lake);
    }

    /// <summary>
    /// Runs the model of the plugin's lake table <typeparamref name="T"/> (<c>.Model(..)</c>): the platform computes its
    /// rows and replaces the whole table with them, in one commit.
    /// </summary>
    public Task<Refreshed> RefreshAsync<T>() where T : class => Refresh<T>(null);

    /// <summary>
    /// Runs the model of <typeparamref name="T"/> over the partitions <paramref name="partition"/> names (partition
    /// columns only, as a replace's filter): the model's rows of those partitions replace them. After the model's
    /// declaration changed, the answer says how many partitions still hold the earlier declaration's rows
    /// (<see cref="Refreshed.StalePartitions"/>). A
    /// filter that names no partition (<c>Filter&lt;T&gt;.All</c>) is sent as such and refused (400 <c>invalid</c>): a
    /// whole refresh is <see cref="RefreshAsync{T}()"/>.
    /// </summary>
    public Task<Refreshed> RefreshAsync<T>(Filter<T> partition) where T : class =>
        Refresh<T>(partition.Node ?? new DbFilter());

    private async Task<Refreshed> Refresh<T>(DbFilter? filter) where T : class
    {
        if (!Schema.IsLakeTable(typeof(T)))
            throw new InvalidOperationException($"{typeof(T).Name} has no [LakeTable] attribute");
        if (Schema.LakeOwner(typeof(T)) is { } owner)
            throw new InvalidOperationException($"{typeof(T).Name} is a lake table of {owner}: a plugin refreshes only its own");
        var r = new LakeRefresh { RequestId = _ctx.Request.Id, Table = Schema.LakeName(typeof(T)) };
        if (filter is not null) r.Filter = filter;
        var answer = (await _ctx.Syscall(new Syscall { LakeRefresh = r })).LakeRefreshed
                     ?? throw PluginError.Unavailable("platform: the lake refresh was answered with something else");
        return new Refreshed((long)answer.Rows, answer.AsOf > 0 ? DateTimeOffset.FromUnixTimeMilliseconds(answer.AsOf) : null,
            (long)answer.StalePartitions);
    }
}

/// <summary>The writes of the lake table <typeparamref name="T"/> (<c>ctx.Lake().Table&lt;T&gt;()</c>).</summary>
public sealed class LakeWriter<T> where T : class
{
    /// <summary>The rows of one write, of one chunk of it, and the bytes of a chunk's rows.</summary>
    public const int WriteRows = 1_000_000;
    public const int ChunkRows = 10_000;
    public const int ChunkBytes = 4 << 20;

    private readonly Ctx _ctx;

    internal LakeWriter(Ctx ctx) => _ctx = ctx;

    /// <summary>
    /// Swaps the partitions <paramref name="filter"/> names (partition columns only) for <paramref name="rows"/>; a table
    /// without partition columns is replaced whole with <c>Filter&lt;T&gt;.All</c>.
    /// </summary>
    public Task<Written> ReplaceAsync(Filter<T> filter, IEnumerable<T> rows) =>
        Send(LakeWrite.Types.Mode.Replace, filter.Node, "", rows);

    /// <summary>Writes each row over the row with the same key, or adds it.</summary>
    public Task<Written> MergeAsync(IEnumerable<T> rows) => Send(LakeWrite.Types.Mode.Merge, null, "", rows);

    /// <summary>
    /// Adds the rows, once per write key (at most 512 bytes): <paramref name="key"/>, or in a job's run by default the
    /// run's id and the number of the append in the run. Outside a run an append without a key is refused (400
    /// <c>invalid</c>) before anything is sent.
    /// </summary>
    public Task<Written> AppendAsync(IEnumerable<T> rows, string? key = null)
    {
        // the number is taken in a run whether or not a key is given, so the defaults stay the same in every attempt
        var fallback = _ctx.Job.IsRun ? $"{_ctx.Job.RunId}#{LakeAppends.Next(_ctx.Request)}" : "";
        key ??= fallback;
        if (string.IsNullOrWhiteSpace(key))
            return Task.FromException<Written>(PluginError.Invalid(
                $"an append of {Schema.LakeName(typeof(T))} outside a job's run needs a key: a write run again after an error appends once only under its key"));
        return Send(LakeWrite.Types.Mode.Append, null, key, rows);
    }

    /// <summary>The rows as chunks of at most <see cref="ChunkRows"/> rows and <see cref="ChunkBytes"/> bytes, each row its JSON.</summary>
    internal static List<List<ByteString>> Chunks(IEnumerable<T> rows)
    {
        var out_ = new List<List<ByteString>> { new() };
        var (size, n) = (0, 0L);
        foreach (var r in rows)
        {
            var b = ByteString.CopyFrom(RowJson(JsonSerializer.SerializeToUtf8Bytes(r, CelobusJson.Options)));
            if (b.Length > ChunkBytes)
                throw new PluginError(422, "bulk_limit", $"a row of {typeof(T).Name} is over {ChunkBytes >> 20} MiB");
            n++;
            if (n > WriteRows)
                throw new PluginError(422, "lake_rows_limit", $"a write holds at most {WriteRows} rows: write the rest in another write");
            if (out_[^1].Count == ChunkRows || size + b.Length > ChunkBytes)
            {
                out_.Add([]);
                size = 0;
            }
            size += b.Length;
            out_[^1].Add(b);
        }
        return out_;
    }

    /// <summary>
    /// A row's JSON with every character from U+007F on written as itself (UTF-8), as the other SDKs write it: the
    /// writer escapes some (a character outside the basic plane, U+2028), and a row's bytes decide its chunk.
    /// </summary>
    internal static byte[] RowJson(byte[] json)
    {
        if (Array.IndexOf(json, (byte)'\\') < 0) return json;
        var text = System.Text.Encoding.UTF8.GetString(json);
        var out_ = new System.Text.StringBuilder(text.Length);
        for (var i = 0; i < text.Length; i++)
        {
            var c = text[i];
            if (c != '\\' || i + 1 >= text.Length)
            {
                out_.Append(c);
                continue;
            }
            if (text[i + 1] != 'u' || i + 5 >= text.Length || !ushort.TryParse(text.AsSpan(i + 2, 4),
                    System.Globalization.NumberStyles.HexNumber, null, out var code) || code < 0x7F)
            {
                out_.Append(c).Append(text[i + 1]); // another escape, or a control character: as written
                i++;
                continue;
            }
            var ch = (char)code;
            if (char.IsHighSurrogate(ch) && i + 11 < text.Length && text[i + 6] == '\\' && text[i + 7] == 'u'
                && ushort.TryParse(text.AsSpan(i + 8, 4), System.Globalization.NumberStyles.HexNumber, null, out var low)
                && char.IsLowSurrogate((char)low))
            {
                out_.Append(ch).Append((char)low);
                i += 11;
            }
            else if (char.IsSurrogate(ch))
            {
                out_.Append(text, i, 6); // a lone surrogate stays escaped
                i += 5;
            }
            else
            {
                out_.Append(ch);
                i += 5;
            }
        }
        return System.Text.Encoding.UTF8.GetBytes(out_.ToString());
    }

    private async Task<Written> Send(LakeWrite.Types.Mode mode, DbFilter? filter, string key, IEnumerable<T> rows)
    {
        if (!Schema.IsLakeTable(typeof(T)))
            throw new InvalidOperationException($"{typeof(T).Name} has no [LakeTable] attribute");
        if (Schema.LakeOwner(typeof(T)) is { } owner)
            throw new InvalidOperationException($"{typeof(T).Name} is a lake table of {owner}: a plugin writes only its own");
        var parts = Chunks(rows);
        var table = Schema.LakeName(typeof(T));
        ulong write = 0;
        for (var i = 0; i < parts.Count; i++)
        {
            var first = write == 0;
            var w = new LakeWrite
            {
                RequestId = _ctx.Request.Id, Table = table, Mode = mode, Key = first ? key : "", Write = write,
                Last = i + 1 == parts.Count,
            };
            if (first && filter is not null) w.Filter = filter;
            w.Rows.AddRange(parts[i]);
            var r = await _ctx.Syscall(new Syscall { LakeWrite = w });
            var answer = r.LakeWritten ?? throw PluginError.Unavailable("platform: the lake write was answered with something else");
            write = answer.Write;
            if (answer.Committed || answer.Skipped) return new Written((long)answer.Rows, answer.Skipped);
        }
        throw PluginError.Unavailable("platform: the lake write was not ended");
    }
}

/// <summary>The lake appends of each request or run so far (a run's default write key is its id and the append's number).</summary>
internal static class LakeAppends
{
    private sealed class Count
    {
        public int N;
    }

    private static readonly ConditionalWeakTable<RequestInfo, Count> Appends = new();

    /// <summary>The number of the request's next append (1 first), whichever context of it appends.</summary>
    public static int Next(RequestInfo request) => Interlocked.Increment(ref Appends.GetOrCreateValue(request).N);
}

internal static partial class Schema
{
    /// <summary>The row of a lake table (<c>[LakeTable]</c>).</summary>
    public static bool IsLakeTable(Type t) => t.IsClass && t.IsDefined(typeof(LakeTableAttribute), false);

    public static LakeTableAttribute? LakeOf(Type t) => t.GetCustomAttribute<LakeTableAttribute>(false);

    /// <summary>The owner of another plugin's lake table; null for one of this plugin's (or any other type).</summary>
    public static string? LakeOwner(Type t) => t.IsClass ? LakeOf(t)?.Plugin : null;

    /// <summary>A lake table's name in its plugin: <c>[LakeTable(Name = ..)]</c>, or the type's name in snake_case.</summary>
    public static string LakeName(Type t) => LakeOf(t)?.Name ?? Names.Snake(t.Name);

    /// <summary>A lake table's name in a query: its own name, or <c>&lt;plugin&gt;.&lt;table&gt;</c>.</summary>
    public static string LakeSource(Type t) => LakeOwner(t) is { } p ? $"{p}.{LakeName(t)}" : LakeName(t);
}
