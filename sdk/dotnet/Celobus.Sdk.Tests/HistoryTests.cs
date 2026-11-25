using System.Text.Json.Nodes;
using Celobus.Sdk.Linq;
using System.Reflection;
using Celobus.V1;
using Google.Protobuf;

namespace Celobus.Sdk.Tests;

[Message] public enum TicketStatus { Open, Closed }

[Entity(SoftDelete = true), History("audit")]
public sealed record Ticket(Id<Ticket> Id, string Title, [History] TicketStatus Status, [History] Id<Platform.User>? Assignee,
    string Body);

/// <summary>Every column kept, but the list, the encrypted one and the masked one.</summary>
[Entity, History("audit", All = true)]
public sealed record Contract(Id<Contract> Id, string Title, decimal? Amount, List<string> Tags, [Encrypted] string? Secret,
    string? Notes);

/// <summary>Open tickets at the end of each day.</summary>
[LakeTable(Layer = "mart", Name = "open_by_day", Partition = [nameof(Day)])]
public sealed record OpenTickets(DateOnly Day, Id<Platform.User>? Assignee, long Open);

/// <summary>Another plugin's entity whose history this plugin reads.</summary>
[Entity(Plugin = "support")]
public sealed record Case(Id<Case> Id, string State);

/// <summary>
/// Entities' histories: what a plugin declares, what a history read and a model over a history send, against a fake
/// platform (the same cases as the Rust SDK's history_tests.rs).
/// </summary>
public class HistoryTests
{
    private static readonly Role Analyst = new("analyst") { Title = "Analyst" };
    private static readonly Role Agent = new("agent") { Title = "Agent" };
    private static readonly Layer Audit = new("audit") { Title = "History of records" };
    private static readonly Layer Mart = new("mart") { Title = "For reports" };

    private static readonly IQueryable<OpenTickets> OpenByDay =
        from h in LakeModel.History<Ticket>().EachDay()
        where h.Row.Status != TicketStatus.Closed
        group h by new { h.Day, h.Row.Assignee } into g
        select new OpenTickets(g.Key.Day!.Value, g.Key.Assignee, g.LongCount());

    private static Plugin Helpdesk() => Plugin.Create("helpdesk").Version("1.0.0")
        .Role(Analyst).Role(Agent).Layer(Audit).Layer(Mart)
        .Entity<Ticket>().HistoryRead<Ticket>(Analyst).HistoryReadAsSystem<Ticket>().HistoryExport<Ticket>()
        .Entity<Contract>()
        .Access<Contract>(a =>
        {
            a.Read(Agent);
            a.Mask(c => c.Notes, Agent);
        })
        .LakeTable<OpenTickets>().Model(OpenByDay)
        .HistoryDependency<Case>();

    [Fact]
    public void AnEntityDeclaresItsHistoryAndItsKeptColumns()
    {
        var plugin = Helpdesk();
        Assert.Empty(plugin.Check());
        var d = JsonNode.Parse(plugin.DescriptorJson())!;
        var ticket = d["entities"]![0]!;
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""{ "layer": "audit", "read": ["analyst"], "system_read": true, "export": true }"""),
            ticket["history"]), ticket["history"]!.ToJsonString());
        static IEnumerable<string> Kept(JsonNode e) =>
            e["fields"]!.AsArray().Where(f => f!["history"] is not null).Select(f => (string)f!["name"]!);
        Assert.Equal(["status", "assignee"], Kept(ticket));
        // all: a flag on every column it covers, none on the list, the encrypted and the masked one
        var contract = d["entities"]![1]!;
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""{ "layer": "audit", "all": true }"""), contract["history"]));
        Assert.Equal(["title", "amount"], Kept(contract));
        // the message of the row carries no history flag
        var message = d["messages"]!.AsArray().First(m => (string)m!["name"]! == "Ticket")!;
        Assert.Empty(Kept(message));
        // the model over the history per day
        var model = d["lake_models"]![0]!;
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""[{ "entity": "ticket", "alias": "ticket", "history": true, "each_day": true }]"""),
            model["sources"]), model["sources"]!.ToJsonString());
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""[{ "source": "ticket", "field": "day" }, { "source": "ticket", "field": "assignee" }]"""),
            model["group_by"]), model["group_by"]!.ToJsonString());
        // another plugin's history read: named in its dependency's lake
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""[{ "plugin": "support", "calls": [], "lake": ["case"] }]"""), d["dependencies"]));
    }

    [Fact]
    public void ReadRolesAreListedOnceEachWhereFirstNamed()
    {
        var d = JsonNode.Parse(Plugin.Create("helpdesk").Version("1.0.0").Role(Analyst).Role(Agent).Layer(Audit).Layer(Mart)
            .Entity<Ticket>().HistoryRead<Ticket>(Analyst, Agent).HistoryRead<Ticket>(Analyst)
            .LakeTable<OpenTickets>().LakeRead<OpenTickets>(Agent, Analyst, Agent).DescriptorJson())!;
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""["analyst", "agent"]"""), d["entities"]![0]!["history"]!["read"]));
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""["agent", "analyst"]"""), d["lake_tables"]![0]!["read"]));
    }

    [Fact]
    public void MistakesOfAHistoryAreReported()
    {
        static IEnumerable<string> Codes(Plugin p) => p.Check().Select(e => e.Code);
        // its layer is not declared
        Assert.Contains("E_HISTORY_LAYER", Codes(Plugin.Create("helpdesk").Version("1.0.0").Entity<Ticket>()));
        // a read of a history an entity does not keep
        Assert.Contains("E_HISTORY_DECLARE", Codes(Plugin.Create("helpdesk").Version("1.0.0").Role(Analyst).Entity<Rack>().HistoryRead<Rack>(Analyst)));
        // a role the plugin does not declare
        Assert.Contains("E_HISTORY_ROLE", Codes(Plugin.Create("helpdesk").Version("1.0.0").Layer(Audit).Entity<Ticket>().HistoryRead<Ticket>(Analyst)));
        // not another plugin's entity
        Assert.Contains("E_LAKE_DEFINE", Codes(Plugin.Create("helpdesk").Version("1.0.0").Layer(Audit).Entity<Ticket>().HistoryDependency<Ticket>()));
    }

    private const string Row = """
        {"id":"0190f3c2-0000-7000-8000-000000000101","status":"open","validFrom":"2026-11-01T10:00:00Z","validTo":null,
         "change":"insert","changedBy":"0190f3c2-0000-7000-8000-0000000000a1","changedByKind":"person","changedByPlugin":"helpdesk"}
        """;

    [Fact]
    public async Task AHistoryReadNamesTheHistoryAndAnswersItsRows()
    {
        var ctx = LakeModelTests.CtxWith(_ => new SyscallResult { DbRows = new DbRows { Rows = { ByteString.CopyFromUtf8(Row) } } },
            out var platform);
        var id = new Id<Ticket>("0190f3c2-0000-7000-8000-000000000101");
        var rows = await (from h in ctx.Lake().History<Ticket>() where h.Row.Id == id orderby h.ValidFrom select h).ToListAsync();
        var r = Assert.Single(rows);
        Assert.Equal(("insert", "person", "helpdesk"), (r.Change, r.ChangedByKind, r.ChangedByPlugin));
        Assert.Equal(new Id<Platform.User>("0190f3c2-0000-7000-8000-0000000000a1"), r.ChangedBy);
        Assert.Equal(DateTimeOffset.Parse("2026-11-01T10:00:00Z"), r.ValidFrom);
        Assert.Null(r.ValidTo);
        Assert.Null(r.Day);
        Assert.Equal(id, r.Id);
        Assert.Equal(TicketStatus.Open, r.Row.Status);
        // a kept column the row has no value of: its default in Row, and Has says so
        Assert.True(r.Has(t => t.Status));
        Assert.False(r.Has(t => t.Assignee));
        Assert.Null(r.Row.Assignee);
        var q = platform.Calls[0].Query;
        Assert.Equal(Engine.Lake, q.Engine);
        Assert.Equal(("ticket", "ticket", true, false), (q.Query.Sources[0].Entity, q.Query.Sources[0].Alias, q.Query.Sources[0].History, q.Query.Sources[0].EachDay));
        Assert.Equal("id", q.Query.Filter.Compare.Left.Column.Field);
        Assert.Equal("valid_from", q.Query.Order[0].Term.Column.Field);

        // per day, counted by day
        var perDay = LakeModelTests.CtxWith(_ => new SyscallResult { DbRows = new DbRows { Rows = { ByteString.CopyFromUtf8("""["2026-11-02","3"]""") } } },
            out var dayPlatform);
        var days = await (from h in perDay.Lake().History<Ticket>().EachDay()
                          group h by h.Day into g
                          select new { Day = g.Key, Open = g.LongCount() }).ToListAsync();
        Assert.Equal(new DateOnly(2026, 11, 2), days[0].Day);
        Assert.Equal(3, days[0].Open);
        var tree = dayPlatform.Calls[0].Query.Query;
        Assert.True(tree.Sources[0].History && tree.Sources[0].EachDay);
        Assert.Equal("day", tree.GroupBy[0].Field);

        // another plugin's history, by its qualified name
        var theirs = JsonNode.Parse(ctx.Lake().History<Case>().TreeJson())!;
        Assert.Equal("support.case", (string)theirs["sources"]![0]!["entity"]!);
        Assert.Equal("support_case", (string)theirs["sources"]![0]!["alias"]!);
        // a history is read with a query, never as a lake table nor through the repository
        Assert.Throws<InvalidOperationException>(() => ctx.Lake().Query<HistoryRow<Ticket>>());
        Assert.Throws<InvalidOperationException>(() => { _ = ctx.Lake().History<Ticket>().Get(default); });
    }

    [Fact]
    public void AHistoryRowsColumnsAreNullableAsTheyMayBeAbsent()
    {
        // what the types say: a history's own optional columns are nullable, a kept column is the entity's own type
        Assert.Equal(typeof(DateTimeOffset?), typeof(HistoryRow<Ticket>).GetProperty(nameof(HistoryRow<Ticket>.ValidTo))!.PropertyType);
        Assert.Equal(typeof(Id<Platform.User>?), typeof(HistoryRow<Ticket>).GetProperty(nameof(HistoryRow<Ticket>.ChangedBy))!.PropertyType);
        Assert.Equal(typeof(DateOnly?), typeof(HistoryRow<Ticket>).GetProperty(nameof(HistoryRow<Ticket>.Day))!.PropertyType);
        var ctx = new NullabilityInfoContext();
        Assert.Equal(NullabilityState.Nullable, ctx.Create(typeof(HistoryRow<Ticket>).GetProperty(nameof(HistoryRow<Ticket>.ChangedByPlugin))!).ReadState);
        Assert.Equal(NullabilityState.NotNull, ctx.Create(typeof(HistoryRow<Ticket>).GetProperty(nameof(HistoryRow<Ticket>.Change))!).ReadState);
    }

    [Fact]
    public async Task ARowWithoutItsHistoryColumnsIsRefused()
    {
        var ctx = LakeModelTests.CtxWith(_ => new SyscallResult { DbRows = new DbRows { Rows = { ByteString.CopyFromUtf8("""{"id":"x"}""") } } }, out _);
        await Assert.ThrowsAsync<PluginError>(() => ctx.Lake().History<Ticket>().ToListAsync());
    }
}
