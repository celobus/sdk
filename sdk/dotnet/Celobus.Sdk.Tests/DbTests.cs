using System.Text.Json.Nodes;
using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging.Abstractions;

namespace Celobus.Sdk.Tests;

[Entity(Key = KeyType.I64, Version = true, SoftDelete = true)]
public sealed partial record Memo(Id<Memo> Id, string Title, string? Body);

/// <summary><c>ctx.Db&lt;T&gt;()</c> and <c>ctx.Transaction</c>: what the SDK sends and how it reads the platform's answers.</summary>
public class DbTests
{
    private static Ctx Context(FakeSyscalls syscalls) => new(
        syscalls, new PluginInfo("tests", "1.0.0"),
        new RequestInfo(77, "Save", DateTimeOffset.UtcNow.AddSeconds(30), null),
        new Caller("tests", null), NullLogger.Instance, CancellationToken.None);

    private static ByteString Stored(long id, string title, long version) => ByteString.CopyFromUtf8(
        $$"""{"id":"{{id}}","title":"{{title}}","version":"{{version}}","createdAt":"2026-11-12T10:00:00.000000Z","updatedAt":"2026-11-12T10:00:01.000000Z"}""");

    private static SyscallResult Rows(params ByteString[] rows)
    {
        var page = new DbRows();
        page.Rows.AddRange(rows);
        return new SyscallResult { DbRows = page };
    }

    [Fact]
    public async Task CreatesReadsAndKeepsTheSystemColumns()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.DbCreate => Rows(Stored(1, "a", 1)),
            Syscall.CallOneofCase.DbGet when call.DbGet.Id == "1" => Rows(Stored(1, "a", 1)),
            _ => new SyscallResult { Error = "not_found: no memo 9" },
        });
        var db = Context(cell).Db<Memo>();
        var row = await db.Create(new Memo(default, "a", null));
        Assert.Equal((1L, "a", 1L), (row.Value.Id.ToInt64(), row.Value.Title, row.Version));
        Assert.Equal(new DateTimeOffset(2026, 11, 12, 10, 0, 0, TimeSpan.Zero), row.CreatedAt);
        Assert.Null(row.DeletedAt);
        Memo memo = await db.Get(new Id<Memo>(1));
        Assert.Equal("a", memo.Title);
        var missing = await Assert.ThrowsAsync<PluginError>(() => db.Get(new Id<Memo>(9)));
        Assert.Equal((404, "not_found"), (missing.Status, missing.Code));

        var create = cell.Calls[0].DbCreate;
        Assert.Equal((77UL, 0UL, "memo"), (create.RequestId, create.Tx, create.Entity));
        Assert.Equal("""{"id":"0","title":"a"}""", create.Rows[0].ToStringUtf8());
    }

    [Fact]
    public async Task FindsWithFiltersOrdersAndPages()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.DbFind when call.DbFind.All => Rows(Stored(1, "a", 1), Stored(2, "b", 1)),
            Syscall.CallOneofCase.DbFind => new SyscallResult
            {
                DbRows = new DbRows { Rows = { Stored(1, "a", 1) }, NextPageToken = "t1" },
            },
            Syscall.CallOneofCase.DbCount => new SyscallResult { DbCount = new DbCountResult { Count = 3 } },
            _ => new SyscallResult { Error = "invalid: no" },
        });
        var db = Context(cell).Db<Memo>();
        var page = await db.Find(Filter<Memo>.Eq(m => m.Title, "a") & !Filter<Memo>.IsNull(m => m.Body))
            .OrderByDescending(m => m.Title)
            .OrderBy(SystemColumn.CreatedAt)
            .Limit(10)
            .WithDeleted()
            .Page();
        Assert.Single(page.Rows);
        Assert.Equal("t1", page.Next);
        var all = await db.Find(Filter<Memo>.In(m => m.Title, ["a", "b"])).All();
        Assert.Equal(2, all.Count);
        Assert.Equal(3, await db.Count());
        Assert.True(await db.Exists(Filter<Memo>.Ne(m => m.Title, "z")));

        var find = cell.Calls[0].DbFind;
        Assert.Equal((10U, true, false), (find.Limit, find.WithDeleted, find.All));
        Assert.Equal([("title", true), ("created_at", false)], find.Order.Select(o => (o.Field, o.Desc)).ToArray());
        var and = find.Filter.And.Items;
        Assert.Equal(("title", DbCondition.Types.Op.Eq, "a"), (and[0].Condition.Field, and[0].Condition.Op, and[0].Condition.Values[0].StringValue));
        Assert.Equal(("body", DbCondition.Types.Op.IsNull), (and[1].Not.Condition.Field, and[1].Not.Condition.Op));
        Assert.True(cell.Calls[1].DbFind.All);
        Assert.Equal(2, cell.Calls[1].DbFind.Filter.Condition.Values.Count);
        Assert.Equal(1U, cell.Calls[3].DbFind.Limit);
    }

    [Fact]
    public async Task UpdatesDeletesAndTheirRefusals()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.DbUpdate when call.DbUpdate.ExpectedVersion == 1 => new SyscallResult
            {
                Error = "version_conflict: memo 1 is at version 2",
                ErrorDetails = new DbErrorDetails { CurrentVersion = 2 },
            },
            Syscall.CallOneofCase.DbUpdate => Rows(Stored(1, "b", 3)),
            Syscall.CallOneofCase.DbDelete when call.DbDelete.Force => new SyscallResult { Ok = new Empty() },
            Syscall.CallOneofCase.DbDelete => new SyscallResult
            {
                Error = "unique_conflict: another row has this title",
                ErrorDetails = new DbErrorDetails { Column = "title" },
            },
            Syscall.CallOneofCase.DbRestore => Rows(Stored(1, "b", 4)),
            _ => new SyscallResult { Error = "query_rows_limit: more than 1000 rows" },
        });
        var db = Context(cell).Db<Memo>();
        var patch = Patch<Memo>.Set(m => m.Title, "b").Clear(m => m.Body);
        var row = await db.Update(new Id<Memo>(1), patch).ExpectVersion(2);
        Assert.Equal(3L, row.Version);
        var stale = await Assert.ThrowsAsync<PluginError>(async () => await db.Update(new Id<Memo>(1), patch).ExpectVersion(1));
        Assert.Equal((409, "version_conflict", 2L), (stale.Status, stale.Code, (long)stale.Details!["current"]!));
        var dup = await Assert.ThrowsAsync<PluginError>(async () => await db.Delete(new Id<Memo>(1)));
        Assert.Equal((409, "unique_conflict", "title"), (dup.Status, dup.Code, (string)dup.Details!["column"]!));
        await db.DeleteForce(new Id<Memo>(1));
        Assert.Equal(4L, (await db.Restore(new Id<Memo>(1))).Version);
        var limit = await Assert.ThrowsAsync<PluginError>(() => db.All());
        Assert.Equal((422, "query_rows_limit"), (limit.Status, limit.Code));

        var update = cell.Calls[0].DbUpdate;
        Assert.Equal(("1", 2L), (update.Id, update.ExpectedVersion));
        Assert.Equal("""{"title":"b","body":null}""", update.Patch.ToStringUtf8());
        Assert.True(cell.Calls[3].DbDelete.Force);
    }

    [Fact]
    public async Task TransactionsCarryTheirIdAndEndAsTheirBlockDoes()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.TxBegin => new SyscallResult { TxBegun = new TxBegun { Tx = call.TxBegin.Tx == 0 ? 5UL : 6UL } },
            Syscall.CallOneofCase.DbCreate => Rows(Stored(1, "a", 1)),
            _ => new SyscallResult { Ok = new Empty() },
        });
        var ctx = Context(cell);
        var made = await ctx.Transaction(async tx =>
        {
            await tx.Db<Memo>().Create(new Memo(default, "a", null));
            // a block inside a block: a nested transaction, rolled back alone by its error; the outer one goes on
            await Assert.ThrowsAsync<PluginError>(() => tx.Transaction(async sp =>
            {
                await sp.Db<Memo>().Create(new Memo(default, "a", null));
                throw PluginError.Conflict("no");
            }));
            return 7;
        });
        Assert.Equal(7, made);
        await Assert.ThrowsAsync<PluginError>(() => ctx.Transaction(
            _ => throw PluginError.Invalid("nope"),
            new TxOptions { Isolation = TxIsolation.Serializable, ReadOnly = true }));

        var seen = cell.Calls.Select(c => c.CallCase switch
        {
            Syscall.CallOneofCase.TxBegin => $"begin {c.TxBegin.Tx} {c.TxBegin.Isolation} {c.TxBegin.ReadOnly}",
            Syscall.CallOneofCase.TxCommit => $"commit {c.TxCommit.Tx}",
            Syscall.CallOneofCase.TxRollback => $"rollback {c.TxRollback.Tx}",
            Syscall.CallOneofCase.DbCreate => $"create {c.DbCreate.Tx}",
            _ => c.CallCase.ToString(),
        }).ToArray();
        Assert.Equal(
            ["begin 0 ReadCommitted False", "create 5", "begin 5 ReadCommitted False", "create 6", "rollback 6", "commit 5",
             "begin 0 Serializable True", "rollback 5"],
            seen);
        Assert.Equal(77UL, cell.Calls[0].TxBegin.RequestId);
    }
}
