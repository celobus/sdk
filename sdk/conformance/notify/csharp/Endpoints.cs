using Celobus.Sdk;

namespace Notify;

/// <summary>
/// A reply in a thread: the members it mentions are told, its author follows the thread (if their setting says so),
/// and the thread's other followers are told, all in the transaction that writes the post.
/// </summary>
public sealed class PostReply : Endpoint<PostReplyRequest, PostView>
{
    public override void Configure(EndpointOptions e) => e.Describe("Posts a reply in a thread.").Roles(NotifyPlugin.Member);

    public override Task<PostView> Handle(Ctx ctx, PostReplyRequest req) => ctx.Transaction(async tx =>
    {
        var ch = await tx.Db<Channel>().Get(req.Channel);
        var created = await tx.Db<Post>().Create(new Post(default, req.Channel, req.Thread, req.Text));
        var thread = NotifyKey.Row(req.Thread); // key "post:<id>", subject post/<id>
        var link = NotifyLink.Screen(Screens.Channel, new ChannelParams(req.Channel.Value, req.Thread.Value));
        var source = new NotifySource("#" + ch.Value.Name, "hash");
        await tx.Notifications.Send(Kinds.Mention, new Notice { To = req.Mentioned, Key = thread,
            Title = $"mentioned you in #{ch.Value.Name}", Body = Snippet(req.Text), Source = source, Link = link });
        await tx.Notifications.Follow(thread, new Follow { Reason = FollowReason.Replied, Noun = "thread" });
        await tx.Notifications.Send(Kinds.ThreadReply, new Notice { Followers = thread, Except = req.Mentioned,
            Title = $"replied in #{ch.Value.Name}", Body = Snippet(req.Text), Source = source, Link = link });
        return new PostView(created.Value.Id, created.Value.Text);
    });

    private static string Snippet(string text) => text.Length <= 140 ? text : text[..140];
}

/// <summary>The caller follows a thread (or stops), and answers whether they follow it now.</summary>
public sealed class FollowThread : Endpoint<FollowThreadRequest, FollowState>
{
    public override void Configure(EndpointOptions e) => e.Describe("Follows a thread, or stops.").Roles(NotifyPlugin.Member).Ui();

    public override async Task<FollowState> Handle(Ctx ctx, FollowThreadRequest req)
    {
        var thread = NotifyKey.Row(req.Thread);
        if (req.Follow) await ctx.Notifications.Follow(thread, new Follow { Noun = "thread" });
        else await ctx.Notifications.Unfollow(thread);
        return new FollowState(await ctx.Notifications.Following(thread));
    }
}

/// <summary>An agent's approval: the Approval row and the ask in one transaction; the key is the ask's identity and the
/// caller (the agent's account) its actor.</summary>
public sealed class RequestApproval : Endpoint<RequestApprovalRequest, Approval>
{
    public override void Configure(EndpointOptions e) => e.Describe("Asks a member to approve an order's changes.").Roles(NotifyPlugin.Member);

    public override Task<Approval> Handle(Ctx ctx, RequestApprovalRequest req) => ctx.Transaction(async tx =>
    {
        var a = (await tx.Db<Approval>().Create(new Approval(default, req.Owner, req.Order, req.Summary, "pending"))).Value;
        await tx.Notifications.Ask(new Ask(AskKind.Approval, NotifyKey.Row(a.Id))
        {
            To = [req.Owner], Title = "needs your approval", Body = req.Summary,
            Source = new NotifySource($"#{req.Order}", "receipt"),
            Link = NotifyLink.Overlay(Screens.OrderPanel, new OrderParams(req.Order)), Review = "Review the changes",
            Buttons =
            [
                NotifyButton.PrimaryOf("Approve", Approve.Ref, new ApproveRequest(a.Id)) with { Icon = "check" },
                NotifyButton.Of("Decline", Decline.Ref, new DeclineRequest(a.Id)) with { Icon = "x" },
            ],
        });
        return a;
    });
}

/// <summary>Sets an approval's status and resolves its ask, for every member it asked.</summary>
internal static class Decision
{
    public static Task<Approval> Decide(Ctx ctx, Id<Approval> id, string status, string outcome) => ctx.Transaction(async tx =>
    {
        var a = await tx.Db<Approval>().Update(id, Patch<Approval>.Set(x => x.Status, status));
        await tx.Notifications.Resolve(NotifyKey.Row(id), $"{outcome} by {tx.Caller.DisplayName}");
        return a.Value;
    });
}

/// <summary>The ask's primary button.</summary>
public sealed class Approve : Endpoint<ApproveRequest, Approval>
{
    public override void Configure(EndpointOptions e) => e.Describe("Approves.").Ui();

    public override Task<Approval> Handle(Ctx ctx, ApproveRequest req) => Decision.Decide(ctx, req.Id, "approved", "Approved");
}

public sealed class Decline : Endpoint<DeclineRequest, Approval>
{
    public override void Configure(EndpointOptions e) => e.Describe("Declines.").Ui();

    public override Task<Approval> Handle(Ctx ctx, DeclineRequest req) => Decision.Decide(ctx, req.Id, "declined", "Declined");
}
