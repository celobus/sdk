using Celobus.Sdk;
using Platform = Celobus.Sdk.Platform;

namespace Notify;

[Entity]
public sealed partial record Channel(Id<Channel> Id, string Name);

// Thread absent: the post starts a thread
[Entity]
public sealed partial record Post(Id<Post> Id, Id<Channel> Channel, Id<Post>? Thread, string Text);

// An agent's request for a person's approval; the ask in their inbox is keyed by it. Status: pending, approved or
// declined.
[Entity]
public sealed partial record Approval(Id<Approval> Id, Id<Platform.User> Owner, string Order, string Summary, string Status);

[Message] public sealed partial record ChannelParams(string C, string? Thread = null);
[Message] public sealed partial record OrderParams(string Id);

// Mentioned: the members the text mentions
[Message] public sealed partial record PostReplyRequest(Id<Channel> Channel, Id<Post> Thread, string Text, IReadOnlyList<Id<Platform.User>> Mentioned);
[Message] public sealed partial record PostView(Id<Post> Id, string Text);

// Follow false: stop following
[Message] public sealed partial record FollowThreadRequest(Id<Post> Thread, bool Follow);
[Message] public sealed partial record FollowState(bool Following);

[Message] public sealed partial record RequestApprovalRequest(Id<Platform.User> Owner, string Order, string Summary);
[Message] public sealed partial record ApproveRequest(Id<Approval> Id);
[Message] public sealed partial record DeclineRequest(Id<Approval> Id);
