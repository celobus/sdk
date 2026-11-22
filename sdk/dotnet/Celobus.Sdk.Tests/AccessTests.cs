using System.Text.Json.Nodes;
using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging.Abstractions;

namespace Celobus.Sdk.Tests;

[Message] public enum PageState { Draft, Published }

[Entity] public sealed partial record Book(Id<Book> Id, Id<Platform.User> OwnerId, Id<Platform.Group>? GroupId, IReadOnlyList<Id<Platform.User>> Readers);

[Entity] public sealed partial record Leaf(Id<Leaf> Id, Id<Book> BookId, Id<Platform.User> AuthorId, PageState State, string? Secret);

[Entity] public sealed partial record Rack(Id<Rack> Id, string Title);

[Entity(SoftDelete = true)] public sealed partial record RackMember(Id<RackMember> Id, Id<Rack> RackId, Id<Platform.User>? UserId, Id<Platform.Group>? TeamId, bool Keeper);

/// <summary>Access rules in the descriptor, and the system context on the session.</summary>
public class AccessTests
{
    private static readonly Role Manager = new("manager");
    private static readonly Role Auditor = new("auditor") { Strict = true };

    private static Plugin Notes() => Plugin.Create("notes").Version("1.0.0")
        .Role(Manager)
        .Role(Auditor)
        .Capability(Capability.AuthzBypass, "Archives leaves.")
        .Bypass<Leaf>(Perm.Read, Perm.Update)
        .Entity<Book>()
        .Entity<Leaf>()
        .Access<Book>(a =>
        {
            var owner = a.Relation("owner", a.Column(b => b.OwnerId));
            var team = a.Relation("team", a.Group(b => b.GroupId));
            a.Read(owner | team | Manager);
            a.Create(owner);
            a.Export(Perm.Read);
        })
        .Access<Leaf>(a =>
        {
            var author = a.Relation("author", a.Column(l => l.AuthorId));
            var book = a.Relation("book", a.Column(l => l.BookId));
            a.Read(author | book.Via(Perm.Read));
            a.Create(author & book.Via(Perm.Read));
            a.Update(author.When(Filter<Leaf>.Eq(l => l.State, PageState.Draft)) | Manager);
            a.Delete(author & Grant.Not(Auditor));
            a.Permit("publish", Manager);
            a.Guard(l => l.State).To(PageState.Published, Grant.Holds("publish")).ToAny(Role.WorkspaceAdmin);
            a.Mask(l => l.Secret, Manager);
        });

    [Fact]
    public void RulesAreDescribedAsThePlatformReadsThem()
    {
        var p = Notes();
        Assert.Empty(p.Check());
        var d = JsonNode.Parse(p.DescriptorJson())!;
        var leaf = d["entities"]![1]!["access"]!;
        static string J(string json) => JsonNode.Parse(json)!.ToJsonString();
        Assert.Equal(J("""{"any":[{"rel":"author"},{"via":{"rel":"book","perm":"read"}}]}"""), leaf["read"]!.ToJsonString());
        Assert.Equal(
            J("""{"any":[{"when":{"grant":{"rel":"author"},"filter":[{"field":"state","op":"eq","value":"draft"}]}},{"role":"manager"}]}"""),
            leaf["update"]!.ToJsonString());
        Assert.Equal(J("""{"all":[{"rel":"author"},{"not":{"role":"auditor"}}]}"""), leaf["delete"]!.ToJsonString());
        Assert.Equal(
            J("""[{"fields":["state"],"rules":[{"value":"published","grant":{"holds":"publish"}},{"value":null,"grant":{"role":"workspace.admin"}}]}]"""),
            leaf["guards"]!.ToJsonString());
        Assert.Equal(J("""[{"field":"secret","roles":["manager"]}]"""), leaf["masks"]!.ToJsonString());
        Assert.True(leaf["admin"]!.GetValue<bool>());
        Assert.Equal(J("""["read"]"""), d["entities"]![0]!["access"]!["export"]!.ToJsonString());
        Assert.Equal(
            J("""[{"name":"owner","kind":"column","field":"owner_id"},{"name":"team","kind":"group","field":"group_id"}]"""),
            d["entities"]![0]!["access"]!["relations"]!.ToJsonString());
        Assert.Equal(J("""[{"entity":"leaf","ops":["read","update"]}]"""), d["capabilities"]![0]!["on"]!.ToJsonString());
    }

    [Fact]
    public void ARelationThroughJunctionRowsIsDescribed()
    {
        var p = Plugin.Create("notes").Version("1.0.0")
            .Role(Manager)
            .Entity<Rack>()
            .Entity<RackMember>()
            .Access<Rack>(a =>
            {
                var member = a.Relation("member", a.Rows<RackMember>(m => m.RackId, m => m.UserId));
                var team = a.Relation("team", a.Rows<RackMember>(m => m.RackId, m => m.TeamId));
                var keeper = a.Relation("keeper",
                    a.Rows<RackMember>(m => m.RackId, m => m.UserId).When(Filter<RackMember>.Eq(m => m.Keeper, true)));
                a.Read(member | team | Manager);
                a.Update(keeper | Manager);
            })
            .Access<RackMember>(a =>
            {
                var rack = a.Relation("rack", a.Column(m => m.RackId));
                a.Read(rack.Via(Perm.Read));
                a.Create(rack.Via(Perm.Update));
            });
        Assert.Empty(p.Check());
        var d = JsonNode.Parse(p.DescriptorJson())!;
        static string J(string json) => JsonNode.Parse(json)!.ToJsonString();
        Assert.Equal(
            J("""
              [{"name":"member","kind":"rows","field":"user_id","rows":{"entity":"rack_member","target":"rack_id","filter":[]}},
               {"name":"team","kind":"rows","field":"team_id","rows":{"entity":"rack_member","target":"rack_id","filter":[]}},
               {"name":"keeper","kind":"rows","field":"user_id","rows":{"entity":"rack_member","target":"rack_id",
                 "filter":[{"field":"keeper","op":"eq","value":true}]}}]
              """),
            d["entities"]![0]!["access"]!["relations"]!.ToJsonString());
        Assert.Equal(J("""{"any":[{"rel":"member"},{"rel":"team"},{"role":"manager"}]}"""),
            d["entities"]![0]!["access"]!["read"]!.ToJsonString());
    }

    [Fact]
    public void MisplacedRulesAreErrors()
    {
        var codes = Plugin.Create("notes").Version("1.0.0")
            .Access<Book>(a => a.Read(Grant.Public))
            .Bypass<Book>(Perm.Read)
            .Check()
            .Select(e => e.Code)
            .ToList();
        Assert.Contains("E_ACCESS_ENTITY", codes);
        Assert.Contains("E_CAPABILITY_SCOPE", codes);
        var bare = JsonNode.Parse(Plugin.Create("bare").Version("1.0.0").Entity<Book>().DescriptorJson())!;
        Assert.Null(bare["entities"]![0]!["access"]);
    }

    [Fact]
    public async Task ASystemContextNamesItsReasonOnEveryCall()
    {
        var cell = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.DbCount => new SyscallResult { DbCount = new DbCountResult { Count = 3 } },
            Syscall.CallOneofCase.TxBegin => new SyscallResult { TxBegun = new TxBegun { Tx = 9 } },
            Syscall.CallOneofCase.TxCommit => new SyscallResult { Ok = new Empty() },
            _ => new SyscallResult { Error = "permission_denied: not yours" },
        });
        var ctx = new Ctx(cell, new PluginInfo("notes", "1.0.0"),
            new RequestInfo(77, "Count", DateTimeOffset.UtcNow.AddSeconds(30), null),
            new Caller("notes", null), NullLogger.Instance, CancellationToken.None);
        var system = ctx.AsSystem("archive");
        Assert.Equal(3, await system.Db<Leaf>().Count());
        Assert.Equal(3, await system.Transaction(tx => tx.Db<Leaf>().Count()));
        var denied = await Assert.ThrowsAsync<PluginError>(async () => await ctx.Db<Leaf>().Delete(new Id<Leaf>(Guid.Empty)));
        Assert.Equal((403, "permission_denied"), (denied.Status, denied.Code));
        Assert.Equal("archive", cell.Calls[0].DbCount.AsSystem);
        Assert.Equal("archive", cell.Calls[1].TxBegin.AsSystem);
        Assert.Equal(("archive", 9UL), (cell.Calls[2].DbCount.AsSystem, cell.Calls[2].DbCount.Tx));
        Assert.Equal("", cell.Calls[4].DbDelete.AsSystem);
    }
}
