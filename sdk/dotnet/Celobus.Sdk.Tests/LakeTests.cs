using System.Text.Json.Nodes;
using Celobus.Sdk.Linq;
using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging.Abstractions;

namespace Celobus.Sdk.Tests;

[LakeTable(Layer = "raw", Name = "stock_file", Partition = [nameof(LoadedOn)]), Describe("Stock as the ERP sends it.")]
public sealed record StockFileRow(DateOnly LoadedOn, string Sku, string OnHand);

[LakeTable(Layer = "core", Partition = [nameof(Day)], Key = [nameof(Day), nameof(Sku)])]
public sealed record Stock(DateOnly Day, string Sku, decimal OnHand, long? Reserved);

/// <summary>Another plugin's exported table.</summary>
[LakeTable(Plugin = "erp", Name = "deliveries")]
public sealed record Delivery(DateOnly Day, string Sku);

/// <summary>
/// The Data Lake: what a plugin declares of its layers and lake tables, what a write sends (chunks, keys, filters) and
/// what it refuses before sending, and what a lake query sends, against a fake platform (the same cases as the Rust
/// SDK's).
/// </summary>
public class LakeTests
{
    private static readonly Role Analyst = new("analyst") { Title = "Analyst" };
    private static readonly Layer Raw = new("raw") { Title = "As it arrived" };
    private static readonly Layer Core = new("core") { Title = "Cleaned" };

    private static Plugin Retail() => Plugin.Create("retail").Version("1.0.0")
        .Role(Analyst).Layer(Raw).Layer(Core)
        .LakeTable<StockFileRow>().LakeTable<Stock>()
        .LakeRead<Stock>(Analyst).LakeReadAsSystem<Stock>().LakeExport<Stock>()
        .LakeDependency<Delivery>();

    [Fact]
    public void APluginDeclaresItsLayersAndLakeTables()
    {
        var plugin = Retail();
        Assert.Empty(plugin.Check());
        var d = JsonNode.Parse(plugin.DescriptorJson())!;
        var expected = JsonNode.Parse("""
            {
              "layers": [{ "name": "raw", "title": "As it arrived" }, { "name": "core", "title": "Cleaned" }],
              "lake_tables": [
                { "name": "stock_file", "layer": "raw", "message": "StockFileRow", "description": "Stock as the ERP sends it.",
                  "partition": ["loaded_on"] },
                { "name": "stock", "layer": "core", "message": "Stock", "partition": ["day"], "key": ["day", "sku"],
                  "read": ["analyst"], "system_read": true, "export": true }
              ],
              "dependencies": [{ "plugin": "erp", "calls": [], "lake": ["deliveries"] }]
            }
            """)!;
        foreach (var member in new[] { "layers", "lake_tables", "dependencies" })
            Assert.True(JsonNode.DeepEquals(expected[member], d[member]), d[member]!.ToJsonString());
        Assert.Equal(["StockFileRow", "Stock"], d["messages"]!.AsArray().Select(m => (string)m!["name"]!));
        // nothing of the lake for a plugin without it
        var plain = JsonNode.Parse(Plugin.Create("plain").Version("1.0.0").DescriptorJson())!.AsObject();
        Assert.False(plain.ContainsKey("layers") || plain.ContainsKey("lake_tables"));
    }

    [Fact]
    public void MistakesOfTheLakeDeclarationsAreReported()
    {
        static IEnumerable<string> Codes(Plugin p) => p.Check().Select(e => e.Code);
        Assert.Contains("E_LAKE_TABLE_LAYER", Codes(Plugin.Create("retail").Version("1.0.0").Role(Analyst).Layer(Raw).LakeTable<Stock>()));
        Assert.Contains("E_LAKE_DEFINE", Codes(Plugin.Create("retail").Version("1.0.0").Role(Analyst).Layer(Core).LakeRead<Stock>(Analyst)));
        Assert.Contains("E_LAKE_DEFINE", Codes(Plugin.Create("retail").Version("1.0.0").LakeTable<Delivery>()));
        Assert.Contains("E_LAKE_DEFINE", Codes(Plugin.Create("retail").Version("1.0.0").Layer(Core).LakeDependency<Stock>()));
        Assert.Contains("E_LAKE_ROLE", Codes(Plugin.Create("retail").Version("1.0.0").Layer(Core).LakeTable<Stock>().LakeRead<Stock>(Analyst)));
    }

    /// <summary>A context of request 77 (or of a run's attempt) whose lake writes and queries a fake platform answers.</summary>
    private static (Ctx Ctx, FakeSyscalls Platform) LakeCtx(string? run = null, int attempt = 1)
    {
        ulong rows = 0;
        var platform = new FakeSyscalls(call =>
        {
            if (call.LakeWrite is { } w)
            {
                if (w.Write == 0) rows = 0;
                rows += (ulong)w.Rows.Count;
                return new SyscallResult
                {
                    LakeWritten = new LakeWritten
                    {
                        Write = 41, Rows = rows, Committed = w.Last && w.Key != "seen", Skipped = w.Last && w.Key == "seen",
                    },
                };
            }
            if (call.Query is not null)
                return new SyscallResult { DbRows = new DbRows { Rows = { ByteString.CopyFromUtf8("""{"day":"2026-01-01","sku":"a","onHand":"1.5"}""") } } };
            return new SyscallResult { Error = "unavailable: not here" };
        });
        var ctx = new Ctx(platform, new PluginInfo("retail", "1.0.0"),
            new RequestInfo(77, "Load", DateTimeOffset.UtcNow.AddSeconds(30), null),
            new Caller("retail", null), NullLogger.Instance, CancellationToken.None);
        return (run is null ? ctx : ctx.WithRun(run, attempt, 3), platform);
    }

    private static readonly DateOnly Day = new(2026, 1, 1);

    [Fact]
    public async Task AWriteGoesInChunksAndCommitsWithTheLast()
    {
        var (ctx, platform) = LakeCtx();
        var rows = Enumerable.Range(0, 25_001).Select(i => new StockFileRow(Day, $"s{i}", "1"));
        var written = await ctx.Lake().Table<StockFileRow>().ReplaceAsync(Filter<StockFileRow>.Eq(x => x.LoadedOn, Day), rows);
        Assert.Equal(new Written(25_001, false), written);
        var writes = platform.Calls.Select(c => c.LakeWrite).ToList();
        Assert.Equal([(10_000, 0UL, false), (10_000, 41UL, false), (5_001, 41UL, true)], writes.Select(w => (w.Rows.Count, w.Write, w.Last)));
        Assert.Equal(("stock_file", LakeWrite.Types.Mode.Replace, 77UL), (writes[0].Table, writes[0].Mode, writes[0].RequestId));
        Assert.Equal("""{ "condition": { "field": "loaded_on", "op": "EQ", "values": [ { "stringValue": "2026-01-01" } ] } }""",
            JsonFormatter.Default.Format(writes[0].Filter));
        Assert.Null(writes[1].Filter);
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""{"loadedOn":"2026-01-01","sku":"s0","onHand":"1"}"""),
            JsonNode.Parse(writes[0].Rows[0].ToStringUtf8())));
        // a table replaced whole sends no filter; an empty write is one chunk
        await ctx.Lake().Table<StockFileRow>().ReplaceAsync(Filter<StockFileRow>.All, []);
        Assert.Null(platform.Calls[^1].LakeWrite.Filter);
        Assert.True(platform.Calls[^1].LakeWrite.Last);
    }

    [Fact]
    public async Task AnAppendCarriesItsKeyAndARunsDefaultOne()
    {
        var (ctx, platform) = LakeCtx();
        var row = new Stock(Day, "a", 1.5m, null);
        Assert.True((await ctx.Lake().Table<Stock>().AppendAsync([row], "seen")).Skipped);
        // outside a run without a key: refused before anything is sent
        var before = platform.Calls.Count;
        Assert.Equal("invalid", (await Assert.ThrowsAsync<PluginError>(() => ctx.Lake().Table<Stock>().AppendAsync([row]))).Code);
        Assert.Equal(400, (await Assert.ThrowsAsync<PluginError>(() => ctx.Lake().Table<Stock>().AppendAsync([row], "  "))).Status);
        Assert.Equal(before, platform.Calls.Count);
        // in a run: its id and the append's number, the same in every attempt; a given key still takes a number, and the
        // system context counts on
        var keys = new List<string>();
        foreach (var attempt in new[] { 1, 2 })
        {
            var (run, seen) = LakeCtx("run-7", attempt);
            await run.Lake().Table<Stock>().AppendAsync([row]);
            await run.Lake().Table<Stock>().AppendAsync([row], "mine");
            await run.AsSystem("the rest").Lake().Table<Stock>().AppendAsync([row]);
            keys.AddRange(seen.Calls.Select(c => c.LakeWrite.Key));
        }
        Assert.Equal(["run-7#1", "mine", "run-7#3", "run-7#1", "mine", "run-7#3"], keys);
        await ctx.Lake().Table<Stock>().MergeAsync([row]);
        Assert.Equal((LakeWrite.Types.Mode.Merge, ""), (platform.Calls[^1].LakeWrite.Mode, platform.Calls[^1].LakeWrite.Key));
    }

    [Fact]
    public async Task ALakeQueryReadsLakeTablesByNameOnTheLake()
    {
        var (ctx, platform) = LakeCtx();
        var rows = await (from s in ctx.Lake().Query<Stock>() where s.Sku == "a" select s).ToListAsync();
        Assert.Equal(1.5m, rows[0].OnHand);
        await (from d in ctx.Lake().Query<Delivery>() select new { d.Day, d.Sku }).ToListAsync().ContinueWith(_ => { });
        var reads = platform.Calls.Where(c => c.Query is not null).Select(c => c.Query).ToList();
        Assert.Equal(Engine.Lake, reads[0].Engine);
        Assert.Equal(("stock", "stock"), (reads[0].Query.Sources[0].Entity, reads[0].Query.Sources[0].Alias));
        Assert.Equal(("erp.deliveries", "erp_deliveries"), (reads[1].Query.Sources[0].Entity, reads[1].Query.Sources[0].Alias));
        // never read as an entity, nor written with the repository
        await Assert.ThrowsAnyAsync<Exception>(() => ctx.Query<Stock>().ToListAsync());
        Assert.Throws<InvalidOperationException>(() => ctx.Db<Stock>());
        Assert.Throws<InvalidOperationException>(() => ctx.Lake().Query<Stock>().Find());
        Assert.Throws<InvalidOperationException>(() => ctx.Lake().Query<StockFileRowNotLake>());
    }

    [Message] public sealed record StockFileRowNotLake(string Sku);

    [Fact]
    public async Task AnotherPluginsTableIsNotWritten()
    {
        var (ctx, _) = LakeCtx();
        var e = await Assert.ThrowsAsync<InvalidOperationException>(() => ctx.Lake().Table<Delivery>().MergeAsync([new Delivery(Day, "a")]));
        Assert.Contains("writes only its own", e.Message);
    }

    [Fact]
    public async Task RowsPastAMillionAreRefusedBeforeAnythingIsSentAndChunksKeepUnder4MiB()
    {
        var many = Enumerable.Range(0, LakeWriter<StockFileRow>.WriteRows + 1).Select(_ => new StockFileRow(Day, "", ""));
        Assert.Equal("lake_rows_limit", Assert.Throws<PluginError>(() => LakeWriter<StockFileRow>.Chunks(many)).Code);
        var big = new string('x', 1 << 20);
        var sized = LakeWriter<StockFileRow>.Chunks(Enumerable.Range(0, 9).Select(_ => new StockFileRow(Day, big, "")));
        Assert.Equal([3, 3, 3], sized.Select(c => c.Count));
        Assert.Equal("bulk_limit", Assert.Throws<PluginError>(() =>
            LakeWriter<StockFileRow>.Chunks([new StockFileRow(Day, new string('x', LakeWriter<StockFileRow>.ChunkBytes), "")])).Code);
        var (ctx, platform) = LakeCtx();
        Assert.Equal(422, (await Assert.ThrowsAsync<PluginError>(() => ctx.Lake().Table<StockFileRow>().MergeAsync(many))).Status);
        Assert.Empty(platform.Calls);
    }

    [Fact]
    public void ARowIsItsJsonAsTheOtherSdksWriteIt()
    {
        // what decides the chunks: the bytes of each row, with no character escaped that the others write as it is
        var row = LakeWriter<StockFileRow>.Chunks([new StockFileRow(Day, "é'<&+\"\\ 😀 \u2028", "")])[0][0];
        Assert.Equal("{\"loadedOn\":\"2026-01-01\",\"sku\":\"é'<&+\\\"\\\\ 😀 \u2028\",\"onHand\":\"\"}", row.ToStringUtf8());
    }

    [Theory]
    [InlineData("lake_busy: other writes of this table ran for more than 60 s; retry", 503)]
    [InlineData("lake_rows_limit: a write holds at most 1000000 rows", 422)]
    [InlineData("lake_time_limit: the query took longer than 10 s", 422)]
    [InlineData("lake_memory_limit: the query needs more memory", 422)]
    public async Task TheLakesRefusalsCarryTheirStatus(string answer, int status)
    {
        var platform = new FakeSyscalls(_ => new SyscallResult { Error = answer });
        var ctx = new Ctx(platform, new PluginInfo("retail", "1.0.0"),
            new RequestInfo(1, "Load", DateTimeOffset.UtcNow.AddSeconds(30), null),
            new Caller("retail", null), NullLogger.Instance, CancellationToken.None);
        var e = await Assert.ThrowsAsync<PluginError>(() => ctx.Lake().Table<Stock>().MergeAsync([new Stock(Day, "a", 1m, null)]));
        Assert.Equal((answer.Split(':')[0], status), (e.Code, e.Status));
    }
}
