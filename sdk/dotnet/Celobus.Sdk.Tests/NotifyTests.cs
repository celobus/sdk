using System.Text.Json.Nodes;
using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging.Abstractions;
using N = Notify;

namespace Celobus.Sdk.Tests;

/// <summary>
/// <c>ctx.Notifications</c>: the kinds in the descriptor, what the SDK sends for each call (driven through the
/// conformance plugin <c>notify</c>'s handlers), and how it reads the platform's refusals, against syscalls answered in memory.
/// </summary>
public class NotifyTests
{
    private const string Sarah = "0193a4c2-0000-7000-8000-0000000000aa";
    private const string Tom = "0193a4c2-0000-7000-8000-0000000000bb";
    private const string ChannelId = "0193a4c2-0000-7000-8000-000000000c01";
    private const string ThreadId = "0193a4c2-0000-7000-8000-000000000001";
    private const string PostId = "0193a4c2-0000-7000-8000-000000000002";
    private const string ApprovalId = "0193a4c2-0000-7000-8000-00000000a001";

    private static Ctx Context(FakeSyscalls syscalls) => new(
        syscalls, new PluginInfo("notify", "0.1.0"),
        new RequestInfo(77, "PostReply", DateTimeOffset.UtcNow.AddSeconds(30), null),
        new Caller("notify", new Principal { UserId = Tom, Kind = Principal.Types.Kind.Person, DisplayName = "Tom Hale" }),
        NullLogger.Instance, CancellationToken.None);

    private static SyscallResult Row(string json)
    {
        var page = new DbRows();
        page.Rows.Add(ByteString.CopyFromUtf8(
            json[..^1] + ""","version":"1","createdAt":"2026-11-16T10:00:00.000000Z","updatedAt":"2026-11-16T10:00:00.000000Z"}"""));
        return new SyscallResult { DbRows = page };
    }

    private static readonly SyscallResult Ok = new() { Ok = new Empty() };

    private static FakeSyscalls Cell() => new(call => call.CallCase switch
    {
        Syscall.CallOneofCase.TxBegin => new SyscallResult { TxBegun = new TxBegun { Tx = 5 } },
        Syscall.CallOneofCase.DbGet => Row($$"""{"id":"{{ChannelId}}","name":"fulfilment"}"""),
        Syscall.CallOneofCase.DbCreate when call.DbCreate.Entity == "post" =>
            Row($$"""{"id":"{{PostId}}","channel":"{{ChannelId}}","thread":"{{ThreadId}}","text":"@Sarah can we hold #1042"}"""),
        Syscall.CallOneofCase.DbCreate or Syscall.CallOneofCase.DbUpdate =>
            Row($$"""{"id":"{{ApprovalId}}","owner":"{{Sarah}}","order":"1042","summary":"5 changes","status":"pending"}"""),
        Syscall.CallOneofCase.NotifyFollow when call.NotifyFollow.Op == NotifyFollow.Types.Op.Get =>
            new SyscallResult { NotifyFollowState = new NotifyFollowState { Following = true } },
        _ => Ok,
    });

    [Fact]
    public void NotifyPluginMatchesItsFixture()
    {
        var plugin = N.NotifyPlugin.Define();
        Assert.Empty(plugin.Check());
        var actual = JsonNode.Parse(plugin.DescriptorJson());
        var expected = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "conformance", "notify.json")));
        Assert.True(JsonNode.DeepEquals(expected, actual), actual!.ToJsonString());
    }

    [Fact]
    public void KindsAreDescribedOnlyWhenDeclaredAndNeedPushSend()
    {
        var bare = JsonNode.Parse(Plugin.Create("quiet").Version("1.0.0").DescriptorJson())!.AsObject();
        Assert.False(bare.ContainsKey("notifications"));
        var errors = Plugin.Create("chat").Version("1.0.0")
            .Notification(N.Kinds.Mention).Notification(N.Kinds.Mention).Notification(new NotificationKind("Bad") { Title = "Bad" })
            .Check();
        Assert.Equal(["E_NOTIFY_CAPABILITY", "E_NOTIFY_KIND_NAME", "E_NOTIFY_KIND_NAME"], errors.Select(e => e.Code));
    }

    [Fact]
    public async Task AReplyTellsWhomItMentionsFollowsTheThreadAndTellsItsFollowers()
    {
        var cell = Cell();
        var view = await new N.PostReply().Handle(Context(cell),
            new N.PostReplyRequest(new(ChannelId), new(ThreadId), "@Sarah can we hold #1042", [new(Sarah)]));
        Assert.Equal(PostId, view.Id.Value);

        var sends = cell.Calls.Where(c => c.CallCase == Syscall.CallOneofCase.NotifySend).Select(c => c.NotifySend).ToList();
        var mention = sends[0];
        Assert.Equal((77UL, 5UL, "mention", false), (mention.RequestId, mention.Tx, mention.Kind, mention.Followers));
        Assert.Equal([Sarah], mention.To);
        Assert.Equal(($"post:{ThreadId}", $"post/{ThreadId}"), (mention.Key, mention.Subject));
        Assert.Equal(NotifyActor.WhoOneofCase.Caller, mention.Actor.WhoCase);
        Assert.Equal(("mentioned you in #fulfilment", "#fulfilment", "hash"), (mention.Title, mention.Source.Label, mention.Source.Icon));
        Assert.Equal(("channel", ""), (mention.Link.Screen, mention.Link.Overlay));
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse($$"""{"c":"{{ChannelId}}","thread":"{{ThreadId}}"}"""), JsonNode.Parse(mention.Link.Args)));
        Assert.Null(mention.Ask);

        var follow = cell.Calls.Single(c => c.CallCase == Syscall.CallOneofCase.NotifyFollow).NotifyFollow;
        Assert.Equal((5UL, NotifyFollow.Types.Op.Follow, NotifyFollow.Types.Reason.Replied, "thread"), (follow.Tx, follow.Op, follow.Reason, follow.Noun));
        Assert.Empty(follow.Users);

        var reply = sends[1];
        Assert.Equal(("thread_reply", true, $"post:{ThreadId}"), (reply.Kind, reply.Followers, reply.Key));
        Assert.Empty(reply.To);
        Assert.Equal([Sarah], reply.Except);
        Assert.Equal(Syscall.CallOneofCase.TxCommit, cell.Calls[^1].CallCase);
    }

    [Fact]
    public async Task AnAskNamesItsButtonsByTheirEndpointsAndIsResolvedByThePressedOne()
    {
        var cell = Cell();
        await new N.RequestApproval().Handle(Context(cell), new N.RequestApprovalRequest(new(Sarah), "1042", "5 changes"));
        var ask = cell.Calls.Single(c => c.CallCase == Syscall.CallOneofCase.NotifySend).NotifySend;
        Assert.Equal(("", $"approval:{ApprovalId}", $"approval/{ApprovalId}"), (ask.Kind, ask.Key, ask.Subject));
        Assert.Equal([Sarah], ask.To);
        Assert.Equal(NotifyActor.WhoOneofCase.Caller, ask.Actor.WhoCase);
        Assert.Equal(("", "order_panel", """{"id":"1042"}"""), (ask.Link.Screen, ask.Link.Overlay, ask.Link.Args));
        Assert.Equal((NotifyAsk.Types.Kind.Approval, "Review the changes"), (ask.Ask.Kind, ask.Ask.Review));
        Assert.Equal(
            [("Approve", "Approve", $$"""{"id":"{{ApprovalId}}"}""", true, "check"), ("Decline", "Decline", $$"""{"id":"{{ApprovalId}}"}""", false, "x")],
            ask.Ask.Buttons.Select(b => (b.Label, b.Endpoint, b.Body, b.Primary, b.Icon)));

        await new N.Approve().Handle(Context(cell), new N.ApproveRequest(new(ApprovalId)));
        var resolve = cell.Calls.Single(c => c.CallCase == Syscall.CallOneofCase.NotifyResolve).NotifyResolve;
        Assert.Equal((77UL, 5UL, $"approval:{ApprovalId}", "Approved by Tom Hale"), (resolve.RequestId, resolve.Tx, resolve.Key, resolve.Outcome));
    }

    [Fact]
    public async Task FollowsUnfollowsAndReadsTheCallersState()
    {
        var cell = Cell();
        var state = await new N.FollowThread().Handle(Context(cell), new N.FollowThreadRequest(new(ThreadId), true));
        Assert.True(state.Following);
        await new N.FollowThread().Handle(Context(cell), new N.FollowThreadRequest(new(ThreadId), false));
        await Context(cell).Notifications.Follow(new NotifyKey("topic:1"), new Follow { Reason = FollowReason.Replied, Users = [new(Sarah)] });
        var f = cell.Calls.Select(c => c.NotifyFollow).ToList();
        Assert.Equal((NotifyFollow.Types.Op.Follow, NotifyFollow.Types.Reason.Manual, 0UL), (f[0].Op, f[0].Reason, f[0].Tx));
        Assert.Equal((NotifyFollow.Types.Op.Get, $"post:{ThreadId}"), (f[1].Op, f[1].Key));
        Assert.Equal((NotifyFollow.Types.Op.Unfollow, NotifyFollow.Types.Reason.Unspecified), (f[2].Op, f[2].Reason));
        Assert.Equal(("topic:1", "", Sarah, NotifyFollow.Types.Reason.Replied), (f[4].Key, f[4].Subject, f[4].Users.Single(), f[4].Reason));
    }

    [Fact]
    public async Task TheActorIsTheCallerAnAgentOrNobody()
    {
        var cell = Cell();
        var n = Context(cell).Notifications;
        await n.Send(N.Kinds.Mention, new Notice { To = [new(Sarah)], Title = "hi", Agent = "fulfilment" });
        await n.Send(N.Kinds.Mention, new Notice { To = [new(Sarah)], Title = "hi", NoActor = true });
        var actors = cell.Calls.Select(c => c.NotifySend.Actor).ToList();
        Assert.Equal(("fulfilment", NotifyActor.WhoOneofCase.None_), (actors[0].Agent, actors[1].WhoCase));
    }

    [Theory]
    [InlineData("invalid: kind `nope` is not declared", 400, "invalid")]
    [InlineData("capability_missing: the plugin does not declare push:send", 403, "capability_missing")]
    [InlineData("follow_limit: the key has 2000 followers", 409, "follow_limit")]
    [InlineData("rate_limited: too many sends", 429, "rate_limited")]
    public async Task ThePlatformsRefusalsKeepTheirStatusAndCode(string error, int status, string code)
    {
        var ctx = Context(new FakeSyscalls(_ => new SyscallResult { Error = error }));
        var e = await Assert.ThrowsAsync<PluginError>(() =>
            ctx.Notifications.Send(N.Kinds.Mention, new Notice { To = [new(Sarah)], Title = "hi" }));
        Assert.Equal((status, code), (e.Status, e.Code));
    }
}
