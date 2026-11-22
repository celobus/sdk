using System.Text.Json.Nodes;
using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging.Abstractions;

namespace Celobus.Sdk.Tests;

[Entity(Key = KeyType.I64, SoftDelete = true)]
public sealed partial record Shelf(Id<Shelf> Id, [Searchable] string Name, [Searchable("english")] string? Notes, int Slots);

[Entity(Key = KeyType.String)]
public sealed partial record Volume(
    Id<Volume> Id, [Unique] string Isbn, decimal Price, int Pages,
    [Ref(Nav = "shelf", Inverse = "books")] Id<Shelf>? ShelfId);

[Entity]
public sealed partial record Misdeclared(
    Id<Misdeclared> Id, [Searchable] int Count, [Searchable("klingon")] string Text,
    [Ref(Nav = "Bad Name")] Id<Shelf> ShelfId, [Ref(Inverse = "x")] string NotARef);

/// <summary>Search, aggregates, include, bulk writes and row locks: what the SDK sends and how it reads the answers.</summary>
public class RepositoryTests
{
    private static Ctx Context(FakeSyscalls syscalls, ulong tx = 0) => new(
        syscalls, new PluginInfo("tests", "1.0.0"),
        new RequestInfo(77, "Save", DateTimeOffset.UtcNow.AddSeconds(30), null),
        new Caller("tests", null), NullLogger.Instance, CancellationToken.None, tx);

    private static ByteString Json(string s) => ByteString.CopyFromUtf8(s);

    private static ByteString ShelfRow(long id, string name) =>
        Json($$"""{"id":"{{id}}","name":"{{name}}","slots":3,"createdAt":"2026-11-12T10:00:00.000000Z","updatedAt":"2026-11-12T10:00:00.000000Z"}""");

    private static ByteString VolumeRow(string id, long? shelf) =>
        Json($$"""{"id":"{{id}}","isbn":"i-{{id}}","price":"12.50","pages":100{{(shelf is { } s ? $",\"shelfId\":\"{s}\"" : "")}},"createdAt":"2026-11-12T10:00:00.000000Z","updatedAt":"2026-11-12T10:00:00.000000Z"}""");

    [Fact]
    public void ColumnModifiersAreDescribed()
    {
        var d = JsonNode.Parse(Plugin.Create("lib").Version("1.0.0").Entity<Shelf>().Entity<Volume>().DescriptorJson())!;
        var shelf = d["entities"]![0]!["fields"]!.AsArray();
        Assert.Equal("simple", (string?)shelf[1]!["searchable"]);
        Assert.Equal("english", (string?)shelf[2]!["searchable"]);
        Assert.Null(shelf[3]!["searchable"]);
        var shelfRef = d["entities"]![1]!["fields"]![4]!;
        Assert.Equal(("shelf", "books"), ((string?)shelfRef["nav"], (string?)shelfRef["inverse"]));
        Assert.Null(d["entities"]![1]!["fields"]![1]!["nav"]);
        // the entity's message has no column modifiers
        var message = d["messages"]!.AsArray().First(m => (string?)m!["name"] == "Volume")!;
        Assert.Null(message["fields"]![4]!["nav"]);
    }

    [Fact]
    public void ModifiersTheSdkCanSeeAreChecked()
    {
        var errors = Plugin.Create("lib").Version("1.0.0").Entity<Shelf>().Entity<Misdeclared>().Check()
            .Select(e => $"{e.Code} {e.Path}").ToArray();
        Assert.Contains("E_SEARCH_TYPE entities.misdeclared.fields.count", errors);
        Assert.Contains("E_SEARCH_LANGUAGE entities.misdeclared.fields.text.searchable", errors);
        Assert.Contains("E_REF_NAME entities.misdeclared.fields.shelf_id.nav", errors);
        Assert.Contains("E_REF_NAME entities.misdeclared.fields.not_a_ref.inverse", errors);
    }

    [Fact]
    public async Task SearchAndContainsAreConditions()
    {
        var cell = new FakeSyscalls(_ => new SyscallResult { DbRows = new DbRows { Rows = { ShelfRow(1, "a") } } });
        var db = Context(cell).Db<Shelf>();
        await db.Find(Filter<Shelf>.Search(s => s.Notes, "\"red apple\" -pear") & Filter<Shelf>.Contains(s => s.Name, "50%_")).All();
        var and = cell.Calls[0].DbFind.Filter.And.Items;
        Assert.Equal(("notes", DbCondition.Types.Op.Search, "\"red apple\" -pear"),
            (and[0].Condition.Field, and[0].Condition.Op, and[0].Condition.Values[0].StringValue));
        Assert.Equal(("name", DbCondition.Types.Op.Contains, "50%_"),
            (and[1].Condition.Field, and[1].Condition.Op, and[1].Condition.Values[0].StringValue));
    }

    [Fact]
    public async Task AggregatesDecodeTheirGroups()
    {
        static DbGroup G(string key, string value) => new() { Key = Json(key), Value = Json(value) };
        var cell = new FakeSyscalls(call => call.DbAggregate switch
        {
            { GroupBy: "" } => new SyscallResult { DbGroups = new DbGroups { Groups = { G("null", "\"7\"") } } },
            { Measure: DbAggregate.Types.Measure.Count } => new SyscallResult
            {
                DbGroups = new DbGroups { Groups = { G("\"3\"", "\"2\""), G("null", "\"1\"") } },
            },
            _ => new SyscallResult { DbGroups = new DbGroups { Groups = { G("\"3\"", "\"25.00\"") } } },
        });
        var db = Context(cell).Db<Volume>();
        Assert.Equal(7, await db.Aggregate().Count());
        var byShelf = await db.Aggregate(Filter<Volume>.Gt(b => b.Pages, 10)).WithDeleted().GroupBy(b => b.ShelfId).Count();
        Assert.Equal([(new Id<Shelf>(3), 2L), (null, 1L)], byShelf.Select(g => (g.Key, g.Count)).ToArray());
        var sums = await db.Aggregate().GroupBy(b => b.ShelfId).Sum(b => b.Price);
        Assert.Equal(25.00m, sums[0].Sum);

        var count = cell.Calls[1].DbAggregate;
        Assert.Equal(("volume", "shelf_id", DbAggregate.Types.Measure.Count, true), (count.Entity, count.GroupBy, count.Measure, count.WithDeleted));
        Assert.Equal("pages", count.Filter.Condition.Field);
        var sum = cell.Calls[2].DbAggregate;
        Assert.Equal((DbAggregate.Types.Measure.Sum, "price"), (sum.Measure, sum.Field));
    }

    [Fact]
    public async Task IncludedRowsComeWithTheirRows()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.DbGet => new SyscallResult
            {
                DbRows = new DbRows
                {
                    Rows = { ShelfRow(1, "a") },
                    Included = { Json($$"""{"books":[{{VolumeRow("b1", 1).ToStringUtf8()}},{{VolumeRow("b2", 1).ToStringUtf8()}}]}""") },
                },
            },
            _ => new SyscallResult
            {
                DbRows = new DbRows
                {
                    Rows = { VolumeRow("b1", 1), VolumeRow("b3", null) },
                    Included = { Json($$"""{"shelf":{{ShelfRow(1, "a").ToStringUtf8()}}}"""), Json("{}") },
                },
            },
        });
        var ctx = Context(cell);
        var shelf = await ctx.Db<Shelf>().Get(new Id<Shelf>(1)).Include("Books");
        Assert.Equal(["b1", "b2"], shelf.Included<Volume>("books").Select(b => b.Value.Id.Value).ToArray());
        var books = await ctx.Db<Volume>().Find().Include("shelf").Include(b => b.ShelfId).All();
        Assert.Equal("a", books[0].IncludedOne<Shelf>("shelf")!.Value.Name);
        Assert.Null(books[1].IncludedOne<Shelf>("shelf"));
        Assert.Empty(books[1].Included<Shelf>("nothing"));

        Assert.Equal(["books"], cell.Calls[0].DbGet.Include);
        Assert.Equal(["shelf", "shelf_id"], cell.Calls[1].DbFind.Include);
        Assert.Null(cell.Calls[0].DbGet.Lock);
        // a plain get is still a task
        Task<Stored<Shelf>> task = ctx.Db<Shelf>().Get(new Id<Shelf>(1));
        Assert.Equal("a", (await task).Value.Name);
    }

    [Fact]
    public async Task BulkWritesSendOneCall()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.DbUpdateWhere => new SyscallResult { DbCount = new DbCountResult { Count = 4 } },
            Syscall.CallOneofCase.DbUpsert => new SyscallResult { DbRows = new DbRows { Rows = { VolumeRow("b1", null) } } },
            _ => new SyscallResult { Error = "bulk_limit: more than 1000 rows" },
        });
        var db = Context(cell).Db<Volume>();
        Assert.Equal(4, await db.UpdateWhere(Filter<Volume>.All, Patch<Volume>.Set(b => b.Pages, 1)));
        var row = await db.Upsert(b => b.Isbn, new Volume(new Id<Volume>("b1"), "i-b1", 12.5m, 100, null));
        Assert.Equal("b1", row.Value.Id.Value);

        var update = cell.Calls[0].DbUpdateWhere;
        Assert.Null(update.Filter);
        Assert.Equal("""{"pages":1}""", update.Patch.ToStringUtf8());
        var upsert = cell.Calls[1].DbUpsert;
        Assert.Equal(("volume", "isbn"), (upsert.Entity, upsert.On));
        Assert.Contains("\"id\":\"b1\"", upsert.Row.ToStringUtf8());
    }

    [Fact]
    public async Task RowLocksExistInATransactionOnly()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.DbGet when call.DbGet.Lock?.Wait == DbLock.Types.Wait.Nowait =>
                new SyscallResult { Error = "row_locked: another transaction holds shelf 1" },
            Syscall.CallOneofCase.DbGet => new SyscallResult { DbRows = new DbRows { Rows = { ShelfRow(1, "a") } } },
            _ => new SyscallResult { DbRows = new DbRows { Rows = { ShelfRow(1, "a") } } },
        });
        var outside = Context(cell).Db<Shelf>();
        var refused = Assert.Throws<PluginError>(() => { outside.Get(new Id<Shelf>(1)).ForUpdate(); });
        Assert.Equal((400, "invalid"), (refused.Status, refused.Code));
        Assert.Throws<PluginError>(() => { outside.Find().SkipLocked(); });
        Assert.Empty(cell.Calls);

        var tx = Context(cell, tx: 5).Db<Shelf>();
        await tx.Get(new Id<Shelf>(1)).ForShare();
        await tx.Find(Filter<Shelf>.Gt(s => s.Slots, 0)).ForUpdate().SkipLocked().Limit(2).Page();
        var locked = await Assert.ThrowsAsync<PluginError>(async () => await tx.Get(new Id<Shelf>(1)).ForUpdate().NoWait());
        Assert.Equal((409, "row_locked"), (locked.Status, locked.Code));

        Assert.Equal((5UL, DbLock.Types.Strength.ForShare, DbLock.Types.Wait.Wait),
            (cell.Calls[0].DbGet.Tx, cell.Calls[0].DbGet.Lock.Strength, cell.Calls[0].DbGet.Lock.Wait));
        var find = cell.Calls[1].DbFind;
        Assert.Equal((DbLock.Types.Strength.ForUpdate, DbLock.Types.Wait.SkipLocked, 2U), (find.Lock.Strength, find.Lock.Wait, find.Limit));
    }
}
