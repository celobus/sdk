using System.Collections.Immutable;
using System.Text.Json.Nodes;
using Celobus.Sdk.Analyzers;
using Celobus.Sdk.Linq;
using Celobus.V1;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Diagnostics;
using Query = Celobus.Sdk.Linq.Query;

namespace Celobus.Sdk.Tests;

[LakeTable(Layer = "mart", Name = "metrics", Partition = [nameof(Day)])]
public sealed record MetricRow(DateOnly Day, Id<Shop> ShopId, string Metric, string Value);

[Entity] public sealed record Shop(Id<Shop> Id, string Name);

/// <summary>One metric of one shop per day.</summary>
[Entity(Storage = EntityStorage.Olap, OlapKey = [nameof(Day), nameof(Shop), nameof(Metric)])]
public sealed record ShopMetric(Id<ShopMetric> Id, DateOnly Day, Id<Shop> Shop, string Metric, decimal? Value);

/// <summary>
/// Entities stored in OLAP: what a plugin declares (<c>Storage</c>, <c>OlapKey</c>, <c>.Load</c>), what a request for a
/// snapshot and its ticket send and read, and what refuses such an entity (the same cases as the Rust SDK's
/// load_tests.rs), against a fake platform.
/// </summary>
public class OlapTests
{
    private static readonly Role Analyst = new("analyst") { Title = "Analyst" };
    private static readonly Layer Mart = new("mart") { Title = "For reports" };

    // the columns in the order of the entity's fields, whatever the order of the select's
    private static readonly IQueryable MetricLoad =
        from m in LakeModel.From<MetricRow>()
        where m.Day >= Query.DaysAgo(400)
        select new { Value = Query.Parse<decimal>(m.Value), m.Day, Shop = m.ShopId, m.Metric };

    private static Plugin Retail() => Plugin.Create("retail").Version("1.0.0")
        .Role(Analyst).Layer(Mart)
        .LakeTable<MetricRow>().LakeReadAsSystem<MetricRow>()
        .Entity<Shop>().Access<Shop>(a => a.Read(Analyst))
        .Entity<ShopMetric>().Access<ShopMetric>(a => a.Read(Analyst))
        .Load<ShopMetric>(MetricLoad);

    [Fact]
    public void APluginDeclaresAnEntityStoredInOlapAndItsLoad()
    {
        var plugin = Retail();
        Assert.Empty(plugin.Check());
        var d = JsonNode.Parse(plugin.DescriptorJson())!;
        var e = d["entities"]!.AsArray().First(x => (string)x!["name"]! == "shop_metric")!;
        Assert.Equal("olap", (string)e["storage"]!);
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""["day", "shop", "metric"]"""), e["olap_key"]));
        var shop = d["entities"]!.AsArray().First(x => (string)x!["name"]! == "shop")!.AsObject();
        Assert.False(shop.ContainsKey("storage") || shop.ContainsKey("olap_key"));
        var expected = JsonNode.Parse("""
            [{ "entity": "shop_metric",
               "sources": [{ "entity": "metrics", "alias": "metrics" }],
               "filter": { "compare": { "left": { "column": { "source": "metrics", "field": "day" } }, "op": "ge", "right": { "days_ago": 400 } } },
               "columns": [
                 { "name": "day", "value": { "column": { "source": "metrics", "field": "day" } } },
                 { "name": "shop", "value": { "column": { "source": "metrics", "field": "shop_id" } } },
                 { "name": "metric", "value": { "column": { "source": "metrics", "field": "metric" } } },
                 { "name": "value", "value": { "column": { "source": "metrics", "field": "value", "parse": "decimal" } } }
               ] }]
            """);
        Assert.True(JsonNode.DeepEquals(expected, d["loads"]), d["loads"]!.ToJsonString());
        Assert.False(JsonNode.Parse(Plugin.Create("plain").Version("1.0.0").DescriptorJson())!.AsObject().ContainsKey("loads"));
    }

    [Fact]
    public void MistakesOfAnEntityStoredInOlapAreReported()
    {
        Plugin Base() => Plugin.Create("retail").Version("1.0.0").Role(Analyst).Layer(Mart)
            .LakeTable<MetricRow>().LakeReadAsSystem<MetricRow>().Entity<Shop>();
        static IEnumerable<string> Codes(Plugin p) => p.Check().Select(e => e.Code);
        // no load
        Assert.Contains("E_OLAP_LOAD", Codes(Base().Entity<ShopMetric>()));
        // a load of an ordinary entity
        Assert.Contains("E_LOAD_ENTITY", Codes(Base().Load<Shop>(from m in LakeModel.From<MetricRow>() select new { Name = m.Metric })));
        // a required column left out, a column the entity has not, id given
        var shortLoad = from m in LakeModel.From<MetricRow>() select new { m.Day, Shop = m.ShopId };
        Assert.Contains("E_LOAD_COLUMN", Codes(Base().Entity<ShopMetric>().Load<ShopMetric>(shortLoad)));
        var odd = from m in LakeModel.From<MetricRow>() select new { m.Day, Shop = m.ShopId, m.Metric, Nope = m.Value };
        Assert.Contains("E_LOAD_COLUMN", Codes(Base().Entity<ShopMetric>().Load<ShopMetric>(odd)));
        // an optional column may be left out
        var sparse = from m in LakeModel.From<MetricRow>() select new { m.Day, Shop = m.ShopId, m.Metric };
        Assert.Empty(Base().Entity<ShopMetric>().Load<ShopMetric>(sparse).Check());
    }

    [Fact]
    public async Task ARequestAnswersATicketAndWaitFollowsItToItsEnd()
    {
        var asked = 0;
        var ctx = LakeModelTests.CtxWith(call =>
        {
            if (call.AnalyticsLoad is { } r)
                return new SyscallResult { AnalyticsTicket = new AnalyticsTicket { Ticket = "t1", Entities = { r.Entities }, RequestedAt = 1_795_000_000_000 } };
            asked++;
            return new SyscallResult
            {
                AnalyticsTicket = new AnalyticsTicket
                {
                    Ticket = "t1",
                    State = asked >= 2 ? AnalyticsTicket.Types.State.Done : AnalyticsTicket.Types.State.Building,
                    Generation = 7,
                    Loads = { new AnalyticsLoadOutcome { Entity = "shop_metric", Rows = 3, LoadedAt = 1_795_000_000_000 } },
                },
            };
        }, out var platform);
        var ticket = await ctx.Analytics().Snapshot().Load<ShopMetric>().Load<ShopMetric>().RequestAsync();
        Assert.Equal(("t1", LoadTicketState.Pending), (ticket.Id, ticket.Last.State));
        Assert.Equal(["shop_metric"], ticket.Last.Entities);
        var done = await ticket.WaitAsync();
        Assert.Equal((LoadTicketState.Done, 7L, 3L), (done.State, done.Generation, done.Loads[0].Rows));
        Assert.NotNull(done.Loads[0].LoadedAt);
        Assert.Equal(["shop_metric"], platform.Calls[0].AnalyticsLoad.Entities);
        Assert.Equal([30_000u, 30_000u], platform.Calls.Skip(1).Select(c => c.AnalyticsLoadStatus.WaitMs));
        Assert.Equal(LoadTicketState.Done, (await LoadTicket.Of(ctx, "t1").StatusAsync()).State);
        Assert.Equal(0u, platform.Calls[^1].AnalyticsLoadStatus.WaitMs);
    }

    [Fact]
    public async Task AWaitInARequestEndsBeforeItsDeadlineWithTheTicketPending()
    {
        var ctx = LakeModelTests.CtxWith(call =>
        {
            if (call.AnalyticsLoadStatus is { } s) Thread.Sleep((int)Math.Min(s.WaitMs, 500));
            return new SyscallResult { AnalyticsTicket = new AnalyticsTicket { Ticket = "t3", NotBefore = 1_795_000_600_000 } };
        }, out var platform, TimeSpan.FromSeconds(4));
        var ticket = await ctx.Analytics().Snapshot().Load<ShopMetric>().RequestAsync();
        Assert.Equal(1_795_000_600_000, ticket.Last.NotBefore?.ToUnixTimeMilliseconds());
        var began = DateTimeOffset.UtcNow;
        var e = await Assert.ThrowsAsync<PluginError>(() => ticket.WaitAsync());
        Assert.True(DateTimeOffset.UtcNow - began < TimeSpan.FromSeconds(3));
        Assert.Equal((503, "analytic_load_pending"), (e.Status, e.Code));
        Assert.Equal("t3", (string?)e.Details!["ticket"]);
        Assert.Equal("pending", (string?)e.Details!["state"]);
        Assert.Equal(1_795_000_600_000, (long?)e.Details!["not_before"]);
        Assert.All(platform.Calls.Skip(1), c => Assert.True(c.AnalyticsLoadStatus.WaitMs <= 2_000));
    }

    [Theory]
    [InlineData("analytic_load_rows_limit", 422)]
    [InlineData("analytic_load_time_limit", 422)]
    [InlineData("analytic_load_duplicate", 422)]
    [InlineData("analytic_load_invalid", 422)]
    [InlineData("analytic_load_failed", 503)]
    [InlineData("", 503)]
    public async Task AFailedTicketIsItsLoadsError(string code, int status)
    {
        var ctx = LakeModelTests.CtxWith(_ => new SyscallResult
        {
            AnalyticsTicket = new AnalyticsTicket { Ticket = "t2", State = AnalyticsTicket.Types.State.Failed, Code = code, Error = "failed" },
        }, out _);
        var e = await Assert.ThrowsAsync<PluginError>(() => LoadTicket.Of(ctx, "t2").WaitAsync());
        Assert.Equal((code.Length == 0 ? "analytic_load_failed" : code, status), (e.Code, e.Status));
    }

    [Theory]
    [InlineData("analytic_load_limit: past the bound", 429)]
    [InlineData("analytic_load_busy: 16 waits", 503)]
    [InlineData("not_loaded: no load yet", 409)]
    public async Task RefusalsCarryTheirStatus(string answer, int status)
    {
        var ctx = LakeModelTests.CtxWith(_ => new SyscallResult { Error = answer }, out _);
        var e = await Assert.ThrowsAsync<PluginError>(() => ctx.Analytics().Snapshot().Load<ShopMetric>().RequestAsync());
        Assert.Equal((answer.Split(':')[0], status), (e.Code, e.Status));
    }

    [Fact]
    public async Task AnEntityStoredInOlapIsReadWithAnalyticsOnly()
    {
        var ctx = LakeModelTests.CtxWith(_ => new SyscallResult { DbRows = new DbRows() }, out var platform);
        await (from m in ctx.Analytics().Query<ShopMetric>() where m.Metric == "footfall" select m).ToListAsync();
        Assert.Equal(Engine.Analytic, platform.Calls[0].Query.Engine);
        Assert.Throws<InvalidOperationException>(() => ctx.Db<ShopMetric>());
        Assert.Throws<InvalidOperationException>(() => ctx.Query<ShopMetric>());
        // an ordinary entity, or none, is no load: refused before anything is sent
        await Assert.ThrowsAsync<PluginError>(() => ctx.Analytics().Snapshot().Load<Shop>().RequestAsync());
        await Assert.ThrowsAsync<PluginError>(() => ctx.Analytics().Snapshot().RequestAsync());
        Assert.Single(platform.Calls);
    }

    [Fact]
    public async Task AnEntityStoredInOlapsOwnRowsAreRead()
    {
        var ctx = LakeModelTests.CtxWith(_ => new SyscallResult
        {
            DbRows = new DbRows
            {
                Rows = { Google.Protobuf.ByteString.CopyFromUtf8("""{"id":"0190f3c2-0000-7000-8000-000000000101","day":"2026-11-20","shop":"0190f3c2-0000-7000-8000-000000000201","metric":"footfall","value":"12.5","createdAt":"2026-11-21T02:00:00Z","updatedAt":"2026-11-21T02:00:00Z"}""") },
                AsOf = 1_795_140_000_000, Generation = 7,
            },
        }, out _);
        var rows = await ctx.Analytics().Query<ShopMetric>().ToListAsOfAsync();
        var r = Assert.Single(rows.Rows);
        Assert.Equal(("footfall", 12.5m), (r.Metric, r.Value));
        Assert.Equal(7UL, rows.Generation);
    }

    [Fact]
    public async Task TheAnalyzerRefusesDbAndQueryOfAnEntityStoredInOlap()
    {
        const string source = """
            using System;
            using System.Linq;
            using Celobus.Sdk;
            [Entity(Storage = EntityStorage.Olap, OlapKey = [nameof(Day)])] public sealed record Daily(Id<Daily> Id, DateOnly Day, long Visits);
            [Entity] public sealed record Plain(Id<Plain> Id, string Name);
            public static class Q
            {
                public static object A(Ctx ctx) => ctx.Db<Daily>();
                public static object B(Ctx ctx) => ctx.Query<Daily>().Where(d => d.Visits > 1);
                public static object C(Ctx ctx) => ctx.Analytics().Query<Daily>();
                public static object D(Ctx ctx) => ctx.Db<Plain>();
            }
            """;
        var tpa = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator);
        var refs = tpa.Select(p => MetadataReference.CreateFromFile(p)).ToList();
        refs.Add(MetadataReference.CreateFromFile(typeof(Ctx).Assembly.Location));
        var compilation = CSharpCompilation.Create("snippet", [CSharpSyntaxTree.ParseText(source)], refs,
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
        Assert.Empty(compilation.GetDiagnostics().Where(d => d.Severity == DiagnosticSeverity.Error));
        ImmutableArray<Diagnostic> d = await compilation.WithAnalyzers([new OlapAnalyzer()]).GetAnalyzerDiagnosticsAsync();
        Assert.Equal(2, d.Length);
        Assert.All(d, x => Assert.Equal((OlapAnalyzer.Id, DiagnosticSeverity.Error), (x.Id, x.Severity)));
    }
}
