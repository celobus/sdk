using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging.Abstractions;
using Platform = Celobus.Sdk.Platform;

namespace Celobus.Sdk.Tests;

// What `celobus plugin add customers` generates for C# (the shape the CLI's codegen writes), kept as is: the SDK must
// accept it.
public static partial class Deps
{
    public static partial class Customers
    {
        public const string Plugin = "customers";
        public static readonly string[] Calls = ["GetCustomer"];
        /// <summary><c>customers.customer</c>, read with <c>ctx.Query&lt;Deps.Customers.Customer&gt;()</c>.</summary>
        [Entity(Plugin = "customers", Key = KeyType.Uuid, SoftDelete = true, Version = true)]
        public sealed partial record Customer(Id<Customer> Id, string Name, string? Email);
        /// <summary>What this plugin reads: <c>.Reads(Deps.Customers.Plugin, Deps.Customers.Reads)</c>.</summary>
        public static readonly DependencyRead[] Reads = [new("customer", [new("id", "ref:customers.customer"), new("name", "string"), new("email", "optional string")])];
    }
}

[Message] public sealed record TallyRequest(long MinNotes, string? Prefix);
[Message] public sealed record Tally(Id<Platform.User> OwnerId, long Notes);

/// <summary><c>ctx.Query&lt;T&gt;()</c> (a dependency's entity) and <c>ctx.Sql</c> (SQL sent as text) against a fake platform.</summary>
public class QueryTests
{
    private static Ctx Context(FakeSyscalls syscalls) => new(
        syscalls, new PluginInfo("tests", "1.0.0"),
        new RequestInfo(77, "Read", DateTimeOffset.UtcNow.AddSeconds(30), null),
        new Caller("tests", null), NullLogger.Instance, CancellationToken.None);

    private static ByteString Customer(string id, string name, string? email) => ByteString.CopyFromUtf8(
        $$"""{"id":"{{id}}","name":"{{name}}"{{(email is null ? "" : $",\"email\":\"{email}\"")}},"version":"3","createdAt":"2026-11-12T10:00:00Z","updatedAt":"2026-11-12T10:00:01Z","deletedAt":"2026-11-12T11:00:00Z"}""");

    private static SyscallResult Rows(params ByteString[] rows)
    {
        var page = new DbRows();
        page.Rows.AddRange(rows);
        return new SyscallResult { DbRows = page };
    }

    private const string Ann = "0190a1b2-0000-7000-8000-000000000001";

    [Fact]
    public async Task QueryReadsADependencysEntity()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.DbGet => Rows(Customer(Ann, "Ann", "ann@example.com")),
            Syscall.CallOneofCase.DbFind when call.DbFind.All => Rows(Customer(Ann, "Ann", null), Customer(Ann, "Bob", null)),
            Syscall.CallOneofCase.DbFind => new SyscallResult
            {
                DbRows = new DbRows { Rows = { Customer(Ann, "Ann", null) }, NextPageToken = "p2" },
            },
            Syscall.CallOneofCase.DbCount => new SyscallResult { DbCount = new DbCountResult { Count = 4 } },
            _ => new SyscallResult { Error = "invalid: no" },
        });
        var customers = Context(cell).Query<Deps.Customers.Customer>();

        var ann = await customers.Get(new Id<Deps.Customers.Customer>(Ann), withDeleted: true);
        Assert.Equal(("Ann", "ann@example.com", 3L), (ann.Value.Name, ann.Value.Email, ann.Version));
        Assert.NotNull(ann.DeletedAt);
        var page = await customers.Find(Filter<Deps.Customers.Customer>.Eq(c => c.Name, "Ann")
                                        | Filter<Deps.Customers.Customer>.IsNotNull(c => c.Email))
            .OrderByDescending(c => c.Name)
            .OrderBy(SystemColumn.CreatedAt)
            .Limit(5)
            .PageToken("p1")
            .WithDeleted()
            .Page();
        Assert.Equal(("Ann", "p2"), (page.Rows[0].Value.Name, page.Next));
        Assert.Null(page.Rows[0].Value.Email);
        Assert.Equal(["Ann", "Bob"], (await customers.All()).Select(r => r.Value.Name));
        Assert.Equal(4, await customers.Count(Filter<Deps.Customers.Customer>.Ne(c => c.Name, "Zed")));
        Assert.True(await customers.Exists(Filter<Deps.Customers.Customer>.Eq(c => c.Name, "Ann")));
        Assert.Equal("Ann", (await customers.Find().First())!.Value.Name);

        // every call names the provider and its entity
        Assert.All(cell.Calls, c => Assert.Equal(("customers", "customer"), c.CallCase switch
        {
            Syscall.CallOneofCase.DbGet => (c.DbGet.Dependency, c.DbGet.Entity),
            Syscall.CallOneofCase.DbFind => (c.DbFind.Dependency, c.DbFind.Entity),
            Syscall.CallOneofCase.DbCount => (c.DbCount.Dependency, c.DbCount.Entity),
            _ => ("?", "?"),
        }));
        var get = cell.Calls[0].DbGet;
        Assert.Equal((77UL, 0UL, Ann, true), (get.RequestId, get.Tx, get.Id, get.WithDeleted));
        var find = cell.Calls[1].DbFind;
        Assert.Equal((5U, "p1", true, false), (find.Limit, find.PageToken, find.WithDeleted, find.All));
        Assert.Equal([("name", true), ("created_at", false)], find.Order.Select(o => (o.Field, o.Desc)).ToArray());
        Assert.Equal(("name", "Ann"), (find.Filter.Or.Items[0].Condition.Field, find.Filter.Or.Items[0].Condition.Values[0].StringValue));
        Assert.Equal(("email", DbCondition.Types.Op.IsNotNull), (find.Filter.Or.Items[1].Condition.Field, find.Filter.Or.Items[1].Condition.Op));
        Assert.True(cell.Calls[2].DbFind.All);
        Assert.Equal(("name", DbCondition.Types.Op.Ne), (cell.Calls[3].DbCount.Filter.Condition.Field, cell.Calls[3].DbCount.Filter.Condition.Op));
        Assert.Equal(1U, cell.Calls[4].DbFind.Limit);
    }

    [Fact]
    public async Task QueryInATransactionCarriesItsIdAndOwnReadsSendNoDependency()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.TxBegin => new SyscallResult { TxBegun = new TxBegun { Tx = 9 } },
            Syscall.CallOneofCase.DbCount => new SyscallResult { DbCount = new DbCountResult { Count = 1 } },
            _ => new SyscallResult { Ok = new Empty() },
        });
        await Context(cell).Transaction(async tx =>
        {
            await tx.Query<Deps.Customers.Customer>().Count();
            await tx.Db<Memo>().Count();
        });
        Assert.Equal((9UL, "customers"), (cell.Calls[1].DbCount.Tx, cell.Calls[1].DbCount.Dependency));
        Assert.Equal((9UL, "", "memo"), (cell.Calls[2].DbCount.Tx, cell.Calls[2].DbCount.Dependency, cell.Calls[2].DbCount.Entity));
    }

    [Fact]
    public async Task QueryRefusalsAreMappedAsTheDbOnes()
    {
        var cell = new FakeSyscalls(_ => new SyscallResult { Error = "permission_denied: the plugin does not read customers.customer" });
        var denied = await Assert.ThrowsAsync<PluginError>(() => Context(cell).Query<Deps.Customers.Customer>().Count());
        Assert.Equal((403, "permission_denied"), (denied.Status, denied.Code));
    }

    [Fact]
    public void OwnAndForeignEntitiesAreNotMixedUp()
    {
        var ctx = Context(new FakeSyscalls(_ => new SyscallResult { Ok = new Empty() }));
        // an own entity is the root of a query too (its tree names it without a plugin)
        Assert.Contains("\"entity\": \"memo\"", ctx.Query<Memo>().TreeJson());
        var foreign = Assert.Throws<InvalidOperationException>(() => ctx.Db<Deps.Customers.Customer>());
        Assert.Contains("ctx.Query<Customer>()", foreign.Message);
    }

    private const string ByOwner =
        "select owner_id, count(*) as notes from tests_memo group by owner_id having count(*) >= $1";

    [Fact]
    public async Task SqlSendsTheQueryAndItsParamsAsAJsonArray()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.TxBegin => new SyscallResult { TxBegun = new TxBegun { Tx = 4 } },
            Syscall.CallOneofCase.Sql => Rows(
                ByteString.CopyFromUtf8("""{"owner_id":"u1","notes":3}"""),
                ByteString.CopyFromUtf8("""{"owner_id":"u2","notes":2}""")),
            _ => new SyscallResult { Ok = new Empty() },
        });
        var ctx = Context(cell);
        // typed: each row read by column name
        var rows = await ctx.Sql<Tally>(ByOwner, 2L);
        Assert.Equal([("u1", 3L), ("u2", 2L)], rows.Select(r => (r.OwnerId.Value, r.Notes)));
        // untyped: the JSON objects as the platform sent them
        var json = await ctx.Sql("select owner_id, count(*) as notes from tests_memo group by owner_id");
        Assert.Equal(("u1", 3L), (json[0].GetProperty("owner_id").GetString(), json[0].GetProperty("notes").GetInt64()));
        // in a transaction it carries the transaction's id
        var everyone = await ctx.Transaction(tx => tx.Sql<Tally>(ByOwner, 1L));
        Assert.Equal(2, everyone.Count);

        var sent = cell.Calls[0].Sql;
        Assert.Equal((77UL, 0UL, ByOwner, false), (sent.RequestId, sent.Tx, sent.Query, sent.Unsafe));
        Assert.Equal("""["2"]""", sent.Params.ToStringUtf8());
        Assert.Equal("[]", cell.Calls[1].Sql.Params.ToStringUtf8());
        Assert.Equal(Syscall.CallOneofCase.TxBegin, cell.Calls[2].CallCase);
        var inTx = cell.Calls[3].Sql;
        Assert.Equal((4UL, ByOwner, """["1"]"""), (inTx.Tx, inTx.Query, inTx.Params.ToStringUtf8()));
    }

    [Fact]
    public async Task SqlBindsEveryValueInOrderAsJson()
    {
        var cell = new FakeSyscalls(_ => Rows());
        await Context(cell).Sql("select $1, $2, $3, $4, $5, $6, $7", "a", 1, 2L, true, null,
            new Id<Platform.User>("u1"), new DateTimeOffset(2026, 11, 12, 10, 0, 0, TimeSpan.Zero));
        Assert.Equal("""["a",1,"2",true,null,"u1","2026-11-12T10:00:00Z"]""", cell.Calls[0].Sql.Params.ToStringUtf8());
    }

    [Fact]
    public async Task UnsafeSqlSendsUnsafeAndAnswersTheReturningRows()
    {
        var cell = new FakeSyscalls(_ => Rows(ByteString.CopyFromUtf8("""{"owner_id":"u1","notes":1}""")));
        var gone = await Context(cell).UnsafeSql<Tally>(
            "delete from tests_memo where owner_id = $1 returning owner_id, 1 as notes", "u1");
        Assert.Equal(("u1", 1L), (gone[0].OwnerId.Value, gone[0].Notes));
        var sent = cell.Calls[0].Sql;
        Assert.Equal((77UL, 0UL, true, """["u1"]"""), (sent.RequestId, sent.Tx, sent.Unsafe, sent.Params.ToStringUtf8()));
        Assert.StartsWith("delete from tests_memo", sent.Query);
    }

    [Fact]
    public async Task SqlRefusalsAreMappedAsTheDbOnes()
    {
        var cell = new FakeSyscalls(_ => new SyscallResult { Error = "query_rows_limit: more than 1000 rows" });
        var limit = await Assert.ThrowsAsync<PluginError>(() => Context(cell).Sql("select * from tests_memo"));
        Assert.Equal((422, "query_rows_limit"), (limit.Status, limit.Code));

        var invalid = new FakeSyscalls(_ => new SyscallResult { Error = "invalid: the query: syntax error at or near \"selec\"" });
        var bad = await Assert.ThrowsAsync<PluginError>(() => Context(invalid).Sql("selec 1"));
        Assert.Equal((400, "invalid", "the query: syntax error at or near \"selec\""), (bad.Status, bad.Code, bad.Message));

        var denied = new FakeSyscalls(_ => new SyscallResult { Error = "permission_denied: not one of the plugin's tables" });
        var unsafeDenied = await Assert.ThrowsAsync<PluginError>(() => Context(denied).UnsafeSql("delete from chat_post"));
        Assert.Equal((403, "permission_denied"), (unsafeDenied.Status, unsafeDenied.Code));
    }
}
