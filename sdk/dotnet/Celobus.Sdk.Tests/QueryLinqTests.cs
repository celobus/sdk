using System.Text.Json.Nodes;
using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging.Abstractions;
using Q = Celobus.Sdk.Linq.Query;
using Platform = Celobus.Sdk.Platform;

namespace Celobus.Sdk.Tests;

/// <summary>The entities of the conformance query (sdk/conformance/query/revenue.json), as a C# plugin declares them.</summary>
public static class Revenue
{
    public static class Crm
    {
        [Entity(Plugin = "crm", Key = KeyType.Uuid)]
        public sealed partial record Customer(Id<Customer> Id, string Name, Id<Platform.User> OwnerId);
    }

    public enum OrderStatus { Placed, Paid }

    [Entity]
    public sealed partial record Order(Id<Order> Id, string Number, OrderStatus Status, decimal Total,
        Id<Platform.User> CreatedBy, Id<Crm.Customer>? CustomerId, int Qty = 0);

    [Entity]
    public sealed partial record OrderLine(Id<OrderLine> Id, Id<Order> OrderId, string Sku, decimal Amount);

    /// <summary>The paid lines of orders over 10.50 placed by a clerk, per clerk and customer.</summary>
    public static IQueryable<Row> Build(Ctx ctx) =>
        (from l in ctx.Query<OrderLine>()
         join o in ctx.Query<Order>() on l.OrderId equals o.Id
         join c in ctx.Query<Crm.Customer>() on o.CustomerId equals (Id<Crm.Customer>?)c.Id into cs
         from c in cs.DefaultIfEmpty()
         where o.Status == OrderStatus.Paid && o.Total >= 10.50m
         where ctx.Query<Platform.RoleAssignment>().Any(r => r.UserId == o.CreatedBy && r.Role == "shop.clerk")
         group l by new { o.CreatedBy, CustomerId = c!.Id, c.Name } into g
         where g.Count() > 1
         orderby g.Sum(x => x.Amount) descending
         select new Row(Q.Label(g.Key.CreatedBy), g.Key.Name, g.Sum(x => x.Amount), g.Count()))
        .Take(20);

    public sealed record Row(Label? Clerk, string? Customer, decimal Revenue, int Lines);
}

/// <summary><c>ctx.Query&lt;T&gt;()</c> as LINQ: the tree its expression becomes, the rows it reads, what it refuses.</summary>
public class QueryLinqTests
{
    private static Ctx Context(FakeSyscalls syscalls) => new(
        syscalls, new PluginInfo("shop", "1.0.0"),
        new RequestInfo(77, "Read", DateTimeOffset.UtcNow.AddSeconds(30), null),
        new Caller("shop", null), NullLogger.Instance, CancellationToken.None);

    private static readonly FakeSyscalls Nothing = new(_ => new SyscallResult { Ok = new Empty() });

    [Fact]
    public void TheConformanceQueryIsTheSharedTree()
    {
        var tree = JsonNode.Parse(Revenue.Build(Context(Nothing)).TreeJson());
        var expected = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "conformance/revenue.json")));
        Assert.True(JsonNode.DeepEquals(expected, tree), tree!.ToJsonString());
    }

    [Fact]
    public async Task RowsComeBackTypedAndReadsSayTheirMode()
    {
        const string clerk = "0190f3c2-0000-7000-8000-000000000011";
        var cell = new FakeSyscalls(call => call.Query.Mode == QueryRead.Types.Mode.Count
            ? new SyscallResult { DbCount = new DbCountResult { Count = 2 } }
            : new SyscallResult
            {
                DbRows = new DbRows
                {
                    Rows =
                    {
                        ByteString.CopyFromUtf8($$"""[{"id":"{{clerk}}","text":"Sarah","avatarUrl":"https://a"},"Ada","30.00","3"]"""),
                        ByteString.CopyFromUtf8("""[null,null,"0","2"]"""),
                    },
                    NextPageToken = "next",
                },
            });
        var q = Revenue.Build(Context(cell));
        var rows = await q.ToListAsync();
        Assert.Equal(new Revenue.Row(new Label(clerk, "Sarah", "https://a"), "Ada", 30.00m, 3), rows[0]);
        Assert.Equal(new Revenue.Row(null, null, 0m, 2), rows[1]);
        var page = await q.PageAsync("t1");
        Assert.Equal("next", page.Next);
        Assert.NotNull(await q.FirstOrDefaultAsync());
        Assert.Equal(2, await q.CountAsync());
        Assert.Equal(
            [(QueryRead.Types.Mode.All, ""), (QueryRead.Types.Mode.Page, "t1"), (QueryRead.Types.Mode.First, ""), (QueryRead.Types.Mode.Count, "")],
            cell.Calls.Select(c => (c.Query.Mode, c.Query.PageToken)));
        Assert.All(cell.Calls, c => Assert.Equal(77UL, c.Query.RequestId));
        Assert.Matches(@"QueryLinqTests\.cs:\d+$", cell.Calls[0].Query.Source);
    }

    [Fact]
    public async Task ARootsRowsAndADirectoryQuery()
    {
        var cell = new FakeSyscalls(_ => new SyscallResult
        {
            DbRows = new DbRows
            {
                Rows =
                {
                    ByteString.CopyFromUtf8("""{"id":"0190f3c2-0000-7000-8000-0000000000aa","number":"SO-1","status":"paid","total":"1","createdBy":"u1","createdAt":"2026-11-12T10:00:00Z","updatedAt":"2026-11-12T10:00:00Z"}"""),
                },
            },
        });
        var ctx = Context(cell);
        var paid = await ctx.Query<Revenue.Order>().Where(o => o.Status == Revenue.OrderStatus.Paid).ToListAsync();
        Assert.Equal(("SO-1", Revenue.OrderStatus.Paid, null), (paid[0].Number, paid[0].Status, paid[0].CustomerId));

        var managers =
            from m in ctx.Query<Platform.GroupMember>()
            join u in ctx.Query<Platform.User>() on m.UserId equals u.Id
            where m.GroupId == new Id<Platform.Group>("g1") && u.Status == Platform.UserStatus.Active
            orderby u.DisplayName
            select new { u.Id, u.DisplayName, Group = Q.Label(m.GroupId) };
        var tree = JsonNode.Parse(managers.TreeJson())!;
        Assert.Equal("platform.group_member", (string?)tree["sources"]![0]!["entity"]);
        Assert.Equal("platform_user", (string?)tree["sources"]![1]!["alias"]);
        Assert.Equal("active", (string?)tree["filter"]!["and"]!["items"]![1]!["compare"]!["values"]![0]!["stringValue"]);
        Assert.Equal("display_name", (string?)tree["order"]![0]!["term"]!["column"]!["field"]);
    }

    [Fact]
    public void EntitiesJoinedTwiceAreNamedByTheirRangeVariables()
    {
        var ctx = Context(Nothing);
        var q =
            from o in ctx.Query<Revenue.Order>()
            join creator in ctx.Query<Platform.User>() on o.CreatedBy equals creator.Id
            join c in ctx.Query<Revenue.Crm.Customer>() on o.CustomerId equals (Id<Revenue.Crm.Customer>?)c.Id
            join owner in ctx.Query<Platform.User>() on c.OwnerId equals owner.Id into owners
            from owner in owners.DefaultIfEmpty()
            where o.Number.Contains("SO") && !ctx.Query<Revenue.OrderLine>().Any(l => l.OrderId == o.Id)
            select new { o.Number, By = creator.DisplayName, Owner = owner!.DisplayName };
        var tree = JsonNode.Parse(q.TreeJson())!;
        var sources = tree["sources"]!.AsArray();
        Assert.Equal(["order", "platform_user", "crm_customer", "owner"], sources.Select(s => (string?)s!["alias"]));
        Assert.Equal("LEFT", (string?)sources[3]!["join"]);
        Assert.Equal("CONTAINS", (string?)tree["filter"]!["and"]!["items"]![0]!["compare"]!["op"]);
        Assert.Equal("order_line", (string?)tree["filter"]!["and"]!["items"]![1]!["not"]!["exists"]!["source"]!["entity"]);
    }

    [Fact]
    public async Task WhatTheTranslationRefusesIsQueryInvalidAndNothingIsSent()
    {
        var cell = new FakeSyscalls(_ => new SyscallResult { Ok = new Empty() });
        var ctx = Context(cell);
        var q = ctx.Query<Revenue.Order>().Where(o => o.Number.ToUpperInvariant() == "X");
        var e = await Assert.ThrowsAsync<PluginError>(() => q.ToListAsync());
        Assert.Equal((500, "query_invalid"), (e.Status, e.Code));
        Assert.Empty(cell.Calls);
        Assert.Throws<NotSupportedException>(() => ctx.Query<Revenue.Order>().ToList());
    }

    [Fact]
    public async Task ThePlatformsQueryRefusalsComeBackTyped()
    {
        var cell = new FakeSyscalls(_ => new SyscallResult { Error = "query_join_limit: at most 8 entities" });
        var e = await Assert.ThrowsAsync<PluginError>(() => Context(cell).Query<Revenue.Order>().CountAsync());
        Assert.Equal((422, "query_join_limit"), (e.Status, e.Code));
    }
}
