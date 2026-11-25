using System.Text.Json.Nodes;
using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging.Abstractions;
using J = Jobsdemo;
using Q = Celobus.Sdk.Linq.Query;

namespace Celobus.Sdk.Tests;

/// <summary>
/// Analytic views: their descriptor (the conformance plugin jobsdemo's), the conformance queries over them
/// (sdk/conformance/query/weekly.json, with a bucket and a relative day; estimates.json, a decimal compared with a
/// fractional value), how a view's rows come back, what a view refuses, and the definition errors.
/// </summary>
public class ViewTests
{
    private static Ctx Context(FakeSyscalls syscalls) => new(
        syscalls, new PluginInfo("jobsdemo", "0.1.0"),
        new RequestInfo(77, "Read", DateTimeOffset.UtcNow.AddSeconds(30), null),
        new Caller("jobsdemo", null), NullLogger.Instance, CancellationToken.None);

    /// <summary>The conformance query over the view recent_task: the recent tasks of the last week whose month falls in
    /// 2026, counted by month and stage.</summary>
    public static IQueryable<object> Weekly(Ctx ctx) =>
        (from r in ctx.Analytics().Query<J.RecentTask>()
         where r.Day >= Q.DaysAgo(7) && Q.Between(Q.Month(r.Day), new DateOnly(2026, 1, 1), new DateOnly(2026, 12, 1))
         group r by new { Month = Q.Month(r.Day), r.Stage } into g
         orderby g.Key.Month
         select (object)new WeekRow(g.Key.Month, g.Key.Stage, g.LongCount()))
        .Take(12);

    public sealed record WeekRow(DateOnly Month, string? Stage, long Tasks);

    /// <summary>The conformance query over the view long_estimate: the estimates of at least 20.50 hours, longest
    /// first.</summary>
    public static IQueryable<object> Estimates(Ctx ctx) =>
        from e in ctx.Analytics().Query<J.LongEstimate>()
        where e.Hours >= 20.50m
        orderby e.Hours descending
        select (object)new EstimateRow(e.Title, e.Hours);

    public sealed record EstimateRow(string Title, decimal Hours);

    [Fact]
    public async Task TheConformanceDecimalQueryIsTheSharedTreeOnTheAnalyticEngine()
    {
        var platform = new FakeSyscalls(_ => new SyscallResult { DbRows = new DbRows() });
        var q = Estimates(Context(platform));
        var expected = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "conformance/estimates.json")));
        var tree = JsonNode.Parse(q.TreeJson());
        Assert.True(JsonNode.DeepEquals(expected, tree), tree!.ToJsonString());
        await q.ToListAsync();
        var sent = platform.Calls.Single().Query;
        Assert.Equal(Engine.Analytic, sent.Engine);
        var wire = JsonNode.Parse(JsonFormatter.Default.Format(sent.Query));
        Assert.True(JsonNode.DeepEquals(expected, wire), wire!.ToJsonString());
        // the view's declaration writes its decimal as text too
        var d = JsonNode.Parse(J.JobsdemoPlugin.Define().DescriptorJson())!;
        var declared = d["views"]!.AsArray().Single(v => (string)v!["name"]! == "long_estimate")!;
        Assert.Equal("12.50", (string)declared["filter"]!["compare"]!["values"]![0]!);
    }

    [Fact]
    public async Task TheConformanceViewQueryIsTheSharedTreeOnTheAnalyticEngine()
    {
        var platform = new FakeSyscalls(_ => new SyscallResult { DbRows = new DbRows() });
        var q = Weekly(Context(platform));
        var expected = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "conformance/weekly.json")));
        var tree = JsonNode.Parse(q.TreeJson());
        Assert.True(JsonNode.DeepEquals(expected, tree), tree!.ToJsonString());
        await q.ToListAsync();
        var sent = platform.Calls.Single().Query;
        Assert.Equal(Engine.Analytic, sent.Engine);
        var wire = JsonNode.Parse(JsonFormatter.Default.Format(sent.Query));
        Assert.True(JsonNode.DeepEquals(expected, wire), wire!.ToJsonString());
    }

    [Fact]
    public void TheViewsAreTheConformanceDescriptorsAndRefuseNothing()
    {
        var plugin = J.JobsdemoPlugin.Define();
        Assert.Empty(plugin.Check());
        var d = JsonNode.Parse(plugin.DescriptorJson())!;
        var expected = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "conformance", "jobs.json")))!;
        Assert.True(JsonNode.DeepEquals(expected["views"], d["views"]), d["views"]!.ToJsonString());
        // the views' rows follow the topics' keys, the lake tables' rows follow them
        Assert.Equal(["RecentTask", "TasksByWeek", "LongEstimate", "TaskLoadRow", "TaskDay", "TaskHours", "IssueWeek", "BudgetCopy",
                "LongEstimateCopy", "OpenByDay"],
            d["messages"]!.AsArray().TakeLast(10).Select(m => (string)m!["name"]!));
    }

    [Fact]
    public async Task AViewsRowsComeBackAsItsOwnTypeOnEitherEngine()
    {
        var platform = new FakeSyscalls(call => new SyscallResult
        {
            DbRows = new DbRows
            {
                Rows = { ByteString.CopyFromUtf8("""{"day":"2026-11-02","title":"Write","stage":null}""") },
                AsOf = call.Query.Engine == Engine.Analytic ? 9 : 0,
                Generation = 1,
            },
        });
        var ctx = Context(platform);
        var rows = await ctx.Query<J.RecentTask>().ToListAsync();
        Assert.Equal(new J.RecentTask(new DateOnly(2026, 11, 2), "Write", null), rows.Single());
        var a = await ctx.Analytics().Query<J.RecentTask>().Where(r => r.Title == "Write").ToListAsOfAsync();
        Assert.Equal("Write", a.Rows.Single().Title);
        Assert.Equal(DateTimeOffset.FromUnixTimeMilliseconds(9), a.AsOf);
        Assert.All(platform.Calls, c => Assert.Equal("recent_task", c.Query.Query.Sources.Single().Entity));
    }

    [Fact]
    public async Task AViewIsReadNeverWritten()
    {
        var none = new FakeSyscalls(_ => new SyscallResult { Ok = new Empty() });
        var ctx = Context(none);
        Assert.Throws<InvalidOperationException>(() => ctx.Db<J.RecentTask>());
        Assert.Throws<InvalidOperationException>(() => ctx.Query<J.RecentTask>().Find());
        await Assert.ThrowsAsync<InvalidOperationException>(() => ctx.Query<J.RecentTask>().All());
        // a view's query is declared, never read
        await Assert.ThrowsAsync<InvalidOperationException>(() => J.JobsdemoPlugin.RecentTasks.ToListAsync());
        Assert.Empty(none.Calls);
    }

    [Fact]
    public void BucketsRelativeDaysAndPlainJoinsInAQuery()
    {
        var ctx = Context(new FakeSyscalls(_ => new SyscallResult { Ok = new Empty() }));
        var q =
            from t in ctx.Query<J.Task>()
            join s in ctx.Query<J.Stage>() on t.Status equals s.Name
            where Q.DaysAgo(30) <= Q.CreatedAt(t)
            group t by Q.Quarter(Q.CreatedAt(t)) into g
            orderby g.Key descending
            select new { Quarter = g.Key, N = g.Count() };
        var tree = JsonNode.Parse(q.TreeJson())!;
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""
            { "entity": "stage", "alias": "stage", "join": "INNER",
              "left": { "source": "task", "field": "status" }, "right": { "source": "stage", "field": "name" } }
            """), tree["sources"]![1]), tree.ToJsonString());
        // a relative day stands on the right
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""
            { "compare": { "left": { "column": { "source": "task", "field": "created_at" } }, "op": "GE", "right": { "daysAgo": 30 } } }
            """), tree["filter"]), tree.ToJsonString());
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""[{ "source": "task", "field": "created_at", "bucket": "QUARTER" }]"""),
            tree["groupBy"]), tree.ToJsonString());
        // outside a query they are not methods to call
        Assert.Throws<NotSupportedException>(() => Q.Month(DateOnly.MinValue));
        Assert.Throws<NotSupportedException>(() => DateOnly.MinValue >= Q.DaysAgo(1));
    }

    [Entity]
    public sealed record Due(Id<Due> Id, DateTimeOffset? DueAt, DateOnly? On, int Points);

    [Fact]
    public void OptionalDatesCompareWithARelativeDayAndBucket()
    {
        var ctx = Context(new FakeSyscalls(_ => new SyscallResult { Ok = new Empty() }));
        var q = from d in ctx.Query<Due>()
                where d.DueAt >= Q.DaysAgo(3) && Q.DaysAgo(1) > d.On && Q.Year(d.On) == new DateOnly(2026, 1, 1) && d.Points > 2
                select new { Month = Q.Month(d.DueAt), d.Points };
        var tree = JsonNode.Parse(q.TreeJson())!;
        var expected = JsonNode.Parse("""
            { "and": { "items": [
              { "and": { "items": [
                { "and": { "items": [
                  { "compare": { "left": { "column": { "source": "due", "field": "due_at" } }, "op": "GE", "right": { "daysAgo": 3 } } },
                  { "compare": { "left": { "column": { "source": "due", "field": "on" } }, "op": "LT", "right": { "daysAgo": 1 } } }
                ] } },
                { "compare": { "left": { "column": { "source": "due", "field": "on", "bucket": "YEAR" } }, "op": "EQ",
                               "values": [{ "stringValue": "2026-01-01" }] } }
              ] } },
              { "compare": { "left": { "column": { "source": "due", "field": "points" } }, "op": "GT", "values": [{ "intValue": "2" }] } }
            ] } }
            """);
        Assert.True(JsonNode.DeepEquals(expected, tree["filter"]), tree["filter"]!.ToJsonString());
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""{ "column": { "source": "due", "field": "due_at", "bucket": "MONTH" } }"""),
            tree["select"]![0]), tree.ToJsonString());
    }

    [View, Describe("Bad.")]
    public sealed record Bad(long N, string Extra);

    [View]
    public sealed record Ordered(long N);

    [View(Name = "task")]
    public sealed record Clash(long N);

    [Fact]
    public void AViewsMistakesAreDefinitionErrors()
    {
        var p = Plugin.Create("bad").Version("1.0.0")
            .Layer(new("audit"))
            .Entity<J.Task>()
            .View(from t in Views.From<J.Task>() group t by t.Status into g select new Bad(g.LongCount(), g.Key))
            .View(from t in Views.From<J.Task>() group t by t.Status into g select new Bad(g.LongCount(), g.Key + "!"))
            .View(from t in Views.From<J.Task>() group t by t.Status into g orderby g.Key select new Ordered(g.LongCount()))
            .View(from t in Views.From<J.Task>() group t by t.Status into g select new Clash(g.LongCount()));
        var errors = p.Check();
        Assert.Equal(
            [("E_VIEW_DUP", "views[1]"), ("E_VIEW_DEFINE", "views[1]"), ("E_VIEW_DEFINE", "views[2]"), ("E_VIEW_CLASH", "views[3]")],
            errors.Select(e => (e.Code, e.Path)));
    }
}
