// The SDK conformance plugin `notify`: the notification samples as one small plugin, the
// same as the Rust and TypeScript versions. A reply notifies who it mentions, makes its author follow the thread and
// tells the thread's other followers; a member follows a thread or stops; an agent's approval is asked with two
// buttons and resolved by the one pressed. Its `--describe` must equal ../descriptor.json.
using Celobus.Sdk;
using Celobus.Sdk.Ui;

namespace Notify;

public static class Kinds
{
    public static readonly NotificationKind Mention = new("mention")
        { Title = "Mentions", Description = "Someone wrote @you in a channel or a thread", Digest = true };
    public static readonly NotificationKind ThreadReply = new("thread_reply")
        { Title = "Replies in threads you follow", Description = "Arrives in Watching", Tab = NotificationTab.Watching };
}

public static class Screens
{
    public static readonly ScreenKey<ChannelParams> Channel = new("channel");
    public static readonly OverlayKey<OrderParams> OrderPanel = new("order_panel");
}

public static class NotifyPlugin
{
    public static readonly Role Member = new("member") { Title = "Member" };

    public static Plugin Define() => Plugin.Create("notify")
        .Title("Notify")
        .Describe("The SDK conformance plugin of notifications.")
        .Role(Member)
        .Capability(Capability.PushSend, "Tells people when they are mentioned or written to.")
        .Notification(Kinds.Mention)
        .Notification(Kinds.ThreadReply)
        .Entity<Channel>()
        .Access<Channel>(a => a.Read(Member))
        .Entity<Post>()
        .Access<Post>(a =>
        {
            a.Read(Member);
            a.Create(Member);
        })
        .Entity<Approval>()
        .Access<Approval>(a =>
        {
            var owner = a.Relation("owner", a.Column(x => x.Owner));
            a.Read(owner);
            a.Create(Member);
            a.Update(owner);
        })
        .Endpoint<PostReply>()
        .Endpoint<FollowThread>()
        .Endpoint<RequestApproval>()
        .Endpoint<Approve>()
        .Endpoint<Decline>()
        .Ui(new PluginUi()
            .Screen(Screen.Build(Screens.Channel, s =>
            {
                s.Title("Channel");
                return Ui.Text("The channel and its threads.");
            }))
            .Overlay(Overlay.SidePanel(Screens.OrderPanel, o =>
            {
                o.Title("Order");
                return Ui.Text("The order and its changes.");
            })));
}
