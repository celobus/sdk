using System.Text.Json.Nodes;
using Celobus.Sdk.Linq;
using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging.Abstractions;
using Query = Celobus.Sdk.Linq.Query;

namespace Celobus.Sdk.Tests;

[LakeTable(Layer = "core", Name = "stock_clean", Partition = [nameof(Day)])]
public sealed record StockClean(DateOnly Day, string Sku, decimal OnHand);

[LakeTable(Layer = "mart", Name = "stock_by_month", Partition = [nameof(Month)])]
public sealed record StockByMonth(DateOnly Month, long Skus, decimal OnHand);

/// <summary>
/// Models of the lake: what a plugin declares (<c>.Model(..)</c>, <c>Query.Parse</c>, <c>.LakeExportToModels</c>) and
/// what a refresh sends and answers, against a fake platform (the same cases as the Rust SDK's lake_model_tests.rs).
/// </summary>
public class LakeModelTests
{
    private static readonly Role Analyst = new("analyst") { Title = "Analyst" };
    private static readonly Layer Raw = new("raw") { Title = "As it arrived" };
    private static readonly Layer Core = new("core") { Title = "Cleaned" };
    private static readonly Layer Mart = new("mart") { Title = "For reports" };

    // the columns in the order of the table's fields, whatever the order of the select's
    private static readonly IQueryable<StockClean> Clean =
        from f in LakeModel.From<StockFileRow>()
        where f.Sku != ""
        select new StockClean(f.LoadedOn, f.Sku, Query.Parse<decimal>(f.OnHand));

    private static readonly IQueryable<StockByMonth> ByMonth =
        from s in LakeModel.From<StockClean>()
        group s by Query.Month(s.Day) into g
        select new StockByMonth(g.Key, g.Select(x => x.Sku).Distinct().LongCount(), g.Sum(x => x.OnHand));

    private static Plugin Retail() => Plugin.Create("retail").Version("1.0.0")
        .Role(Analyst).Layer(Raw).Layer(Core).Layer(Mart)
        .LakeTable<StockFileRow>().LakeReadAsSystem<StockFileRow>()
        .LakeTable<StockClean>().LakeReadAsSystem<StockClean>().Model(Clean)
        .LakeTable<StockByMonth>().Model(ByMonth).LakeRead<StockByMonth>(Analyst).LakeExportToModels<StockByMonth>();

    [Fact]
    public void APluginDeclaresItsModels()
    {
        var plugin = Retail();
        Assert.Empty(plugin.Check());
        var d = JsonNode.Parse(plugin.DescriptorJson())!;
        var expected = JsonNode.Parse("""
            [
              { "table": "stock_clean",
                "sources": [{ "entity": "stock_file", "alias": "stock_file" }],
                "filter": { "compare": { "left": { "column": { "source": "stock_file", "field": "sku" } }, "op": "ne", "values": [""] } },
                "columns": [
                  { "name": "day", "value": { "column": { "source": "stock_file", "field": "loaded_on" } } },
                  { "name": "sku", "value": { "column": { "source": "stock_file", "field": "sku" } } },
                  { "name": "on_hand", "value": { "column": { "source": "stock_file", "field": "on_hand", "parse": "decimal" } } }
                ] },
              { "table": "stock_by_month",
                "sources": [{ "entity": "stock_clean", "alias": "stock_clean" }],
                "group_by": [{ "source": "stock_clean", "field": "day", "bucket": "month" }],
                "columns": [
                  { "name": "month", "value": { "column": { "source": "stock_clean", "field": "day", "bucket": "month" } } },
                  { "name": "skus", "value": { "aggregate": { "fn": "count_distinct", "column": { "source": "stock_clean", "field": "sku" } } } },
                  { "name": "on_hand", "value": { "aggregate": { "fn": "sum", "column": { "source": "stock_clean", "field": "on_hand" } } } }
                ] }
            ]
            """)!;
        Assert.True(JsonNode.DeepEquals(expected, d["lake_models"]), d["lake_models"]!.ToJsonString());
        // the table other plugins' models may copy, apart from export
        var byMonth = d["lake_tables"]!.AsArray().First(t => (string)t!["name"]! == "stock_by_month")!.AsObject();
        Assert.True((bool)byMonth["export_models"]!);
        Assert.False(byMonth.ContainsKey("export"));
        var plain = JsonNode.Parse(Plugin.Create("plain").Version("1.0.0").DescriptorJson())!.AsObject();
        Assert.False(plain.ContainsKey("lake_models"));
    }

    [Fact]
    public void MistakesOfAModelAreReported()
    {
        static IEnumerable<string> Codes(Plugin p) => p.Check().Select(e => e.Code);
        // a model of a table the plugin does not declare
        Assert.Contains("E_LAKE_MODEL_TABLE", Codes(Plugin.Create("retail").Version("1.0.0").Layer(Raw).LakeTable<StockFileRow>().Model(Clean)));
        // what the translation refuses: an order in a model, a parse as a type the lake has not
        var ordered = from f in LakeModel.From<StockFileRow>() orderby f.Sku select new StockClean(f.LoadedOn, f.Sku, Query.Parse<decimal>(f.OnHand));
        Assert.Contains("E_LAKE_MODEL_DEFINE", Codes(Plugin.Create("retail").Version("1.0.0").Layer(Raw).Layer(Core)
            .LakeTable<StockFileRow>().LakeTable<StockClean>().Model(ordered)));
        var odd = from f in LakeModel.From<StockFileRow>() select new StockClean(f.LoadedOn, f.Sku, Query.Parse<byte>(f.OnHand));
        var errors = Plugin.Create("retail").Version("1.0.0").Layer(Raw).Layer(Core).LakeTable<StockFileRow>().LakeTable<StockClean>().Model(odd).Check();
        Assert.Contains(errors, e => e.Code == "E_LAKE_MODEL_DEFINE" && e.Message.Contains("Byte"));
        // lake tables with entities in one model
        var mixed = from f in LakeModel.From<StockFileRow>() join r in LakeModel.From<Rack>() on f.Sku equals r.Title
                    select new StockClean(f.LoadedOn, f.Sku, Query.Parse<decimal>(f.OnHand));
        Assert.Contains("E_LAKE_MODEL_MIXED", Codes(Plugin.Create("retail").Version("1.0.0").Layer(Raw).Layer(Core)
            .LakeTable<StockFileRow>().LakeTable<StockClean>().Model(mixed)));
        // an export to models of a table not declared
        Assert.Contains("E_LAKE_DEFINE", Codes(Plugin.Create("retail").Version("1.0.0").LakeExportToModels<StockClean>()));
    }

    internal static Ctx CtxWith(Func<Syscall, SyscallResult> answer, out FakeSyscalls platform, TimeSpan? deadline = null)
    {
        platform = new FakeSyscalls(answer);
        return new Ctx(platform, new PluginInfo("retail", "1.0.0"),
            new RequestInfo(77, "Refresh", DateTimeOffset.UtcNow + (deadline ?? TimeSpan.FromSeconds(60)), null),
            new Caller("retail", null), NullLogger.Instance, CancellationToken.None);
    }

    [Fact]
    public async Task ARefreshSendsItsTableAndPartitionsAndReadsWhatItDid()
    {
        var ctx = CtxWith(call => new SyscallResult
        {
            LakeRefreshed = new LakeRefreshed
            {
                Rows = 12, AsOf = call.LakeRefresh.Table == "stock_clean" ? 0 : 1_767_225_600_000,
                StalePartitions = call.LakeRefresh.Filter is null ? 0UL : 3UL,
            },
        }, out var platform);
        Assert.Equal(new Refreshed(12, null, 0), await ctx.Lake().RefreshAsync<StockClean>());
        var day = new DateOnly(2026, 1, 1);
        var out_ = await ctx.Lake().RefreshAsync(Filter<StockByMonth>.Eq(x => x.Month, day));
        Assert.Equal(DateTimeOffset.Parse("2026-01-01T00:00:00Z"), out_.AsOf);
        Assert.Equal(3, out_.StalePartitions);
        // a filter that names every partition is still a refresh of partitions
        await ctx.Lake().RefreshAsync(Filter<StockClean>.All);
        var calls = platform.Calls.Select(c => c.LakeRefresh).ToList();
        Assert.Equal(("stock_clean", 77UL), (calls[0].Table, calls[0].RequestId));
        Assert.Null(calls[0].Filter);
        Assert.Equal("""{ "condition": { "field": "month", "op": "EQ", "values": [ { "stringValue": "2026-01-01" } ] } }""",
            JsonFormatter.Default.Format(calls[1].Filter));
        Assert.NotNull(calls[2].Filter);
        Assert.Equal(DbFilter.NodeOneofCase.None, calls[2].Filter.NodeCase);
        // another plugin's table is not refreshed
        await Assert.ThrowsAsync<InvalidOperationException>(() => ctx.Lake().RefreshAsync<Delivery>());
    }

    [Theory]
    [InlineData("lake_model_rows_limit: the model computes more than 1000000 rows", 422)]
    [InlineData("lake_model_parse: f.on_hand holds a text that is not a decimal", 422)]
    [InlineData("lake_model_changed: the model of stock changed", 409)]
    [InlineData("lake_partitions_limit: too many partitions in one write: 1200, at most 1000: write fewer day at a time", 422)]
    [InlineData("capability_missing: the model reads product", 403)]
    [InlineData("unavailable: the analytic copy is loading", 503)]
    public async Task ARefreshsRefusalsCarryTheirStatus(string answer, int status)
    {
        var ctx = CtxWith(_ => new SyscallResult { Error = answer }, out _);
        var e = await Assert.ThrowsAsync<PluginError>(() => ctx.Lake().RefreshAsync<StockClean>());
        Assert.Equal((answer.Split(':')[0], status), (e.Code, e.Status));
    }

    [Fact]
    public void AParsedTextInALakeQueryIsSentWithItsParse()
    {
        var ctx = CtxWith(_ => new SyscallResult(), out _);
        var q = from f in ctx.Lake().Query<StockFileRow>()
                where Query.Parse<decimal>(f.OnHand) > 10m
                select new { OnHand = Query.Parse<decimal>(f.OnHand), Day = Query.Parse<DateOnly>(f.Sku) };
        var tree = JsonNode.Parse(q.TreeJson())!;
        Assert.Equal("PARSE_DECIMAL", (string)tree["select"]![0]!["column"]!["parse"]!);
        Assert.Equal("PARSE_DATE", (string)tree["select"]![1]!["column"]!["parse"]!);
        Assert.Equal("PARSE_DECIMAL", (string)tree["filter"]!["compare"]!["left"]!["column"]!["parse"]!);
        // a parse is a part of a query, never called
        Assert.Throws<NotSupportedException>(() => Query.Parse<decimal>("1"));
    }
}
