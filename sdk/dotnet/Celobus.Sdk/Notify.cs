// Notifications: the kinds a plugin declares, and what it sends.
//
// A plugin only sends: the platform stores each notification in the member's inbox, tracks read and done, keeps
// follows and preferences, hides what the member can no longer read, keeps their screens live and mails a daily
// digest of what stays unread. Sending needs the capability `push:send`, declared with its reason.
//
//   public static readonly NotificationKind Mention = new("mention") { Title = "Mentions", Digest = true };
//   Plugin.Create("chat").Capability(Capability.PushSend, "Tells people when they are mentioned.").Notification(Mention);
//
//   await tx.Notifications.Send(Mention, new Notice { To = mentioned, Key = NotifyKey.Row(post.Id), Title = "mentioned you" });
//
// The API is `ctx.Notifications` (`Notifications`): `Send` a notice of a kind, `Ask` an approval or a decision with
// buttons and `Resolve` it, `Follow` a key, `Unfollow` it, ask whether the caller is `Following` it.
using System.Text.Json;
using Celobus.Sdk.Ui;

namespace Celobus.Sdk;

/// <summary>The inbox tab a kind arrives in.</summary>
public enum NotificationTab
{
    /// <summary>Addressed to the member (a mention, a direct message, an assignment).</summary>
    ForYou,

    /// <summary>About something the member follows (a reply in a thread).</summary>
    Watching,
}

/// <summary>
/// A kind of notification the plugin declares (<c>.Notification(Kinds.Mention)</c>): what a member turns on or off in
/// their settings. Its name is snake_case, at most 32 characters; the title is 1 to 40 characters, the description at
/// most 120. The digest leaves it out unless <see cref="Digest"/>.
/// </summary>
public sealed class NotificationKind(string name)
{
    public string Name { get; } = name;

    /// <summary>The settings row's title ("Mentions").</summary>
    public string Title { get; init; } = "";

    /// <summary>The settings row's second line.</summary>
    public string Description { get; init; } = "";

    public NotificationTab Tab { get; init; } = NotificationTab.ForYou;

    /// <summary>The daily digest includes it unless the member turns that off.</summary>
    public bool Digest { get; init; }
}

/// <summary>
/// What a notification is about: the plugin's name for it (<c>post:0193…</c>, at most 200 bytes), optionally one of
/// its entity rows (the subject). Unread notifications of one kind and key group into one row ("3 replies"); members
/// follow keys; an ask's key is its identity. A member who can no longer read the subject no longer sees it.
/// </summary>
public sealed record NotifyKey
{
    /// <summary>A key of the plugin's own (no subject).</summary>
    public NotifyKey(string key)
    {
        Key = key;
    }

    public string Key { get; }

    /// <summary><c>&lt;entity&gt;/&lt;id&gt;</c>, or empty.</summary>
    public string Subject { get; private init; } = "";

    /// <summary>The key of one of the plugin's entity rows, <c>&lt;entity&gt;:&lt;id&gt;</c>, with the row as its subject.</summary>
    public static NotifyKey Row<T>(Id<T> id) where T : class =>
        new($"{Names.Snake(typeof(T).Name)}:{id.Value}") { Subject = SubjectOf(id) };

    /// <summary>The same key tied to one of the plugin's entity rows.</summary>
    public NotifyKey WithSubject<T>(Id<T> id) where T : class => this with { Subject = SubjectOf(id) };

    private static string SubjectOf<T>(Id<T> id) where T : class => $"{Names.Snake(typeof(T).Name)}/{id.Value}";

    public override string ToString() => Key;
}

/// <summary>Where a notification opens: one of the plugin's screens or document-level overlays, with its params.</summary>
public sealed class NotifyLink
{
    private NotifyLink(string screen, string overlay, string args)
    {
        ScreenId = screen;
        OverlayId = overlay;
        Args = args;
    }

    internal string ScreenId { get; }
    internal string OverlayId { get; }
    internal string Args { get; }

    public static NotifyLink Screen<TParams>(ScreenKey<TParams> screen, TParams @params) where TParams : class =>
        new(screen.Id, "", JsonSerializer.Serialize(@params, CelobusJson.Options));

    /// <summary>A document-level overlay (a side panel, shown beside the inbox's list).</summary>
    public static NotifyLink Overlay<TParams>(OverlayKey<TParams> overlay, TParams @params) where TParams : class =>
        new("", overlay.Id, JsonSerializer.Serialize(@params, CelobusJson.Options));

    internal V1.NotifyLink Wire() => new() { Screen = ScreenId, Overlay = OverlayId, Args = Args };
}

/// <summary>Where a notification happened: a label ("#fulfilment") and an icon of the kit ("hash").</summary>
public sealed record NotifySource(string Label, string Icon);

/// <summary>A notification of one of the plugin's kinds: to members (<see cref="To"/>), to the followers of a key (<see cref="Followers"/>), or both.</summary>
public sealed class Notice
{
    /// <summary>These members, at most 100.</summary>
    public IEnumerable<Id<Platform.User>> To { get; init; } = [];

    /// <summary>Also the active followers of this key (its key too, unless <see cref="Key"/> names another).</summary>
    public NotifyKey? Followers { get; init; }

    /// <summary>Not these members (those already told in this request).</summary>
    public IEnumerable<Id<Platform.User>> Except { get; init; } = [];

    /// <summary>What it is about: unread notifications of its kind and key group into one row.</summary>
    public NotifyKey? Key { get; init; }

    /// <summary>The phrase after the actor ("mentioned you in #fulfilment"), at most 140 characters.</summary>
    public string Title { get; init; } = "";

    /// <summary>A snippet, at most 500 characters of plain text.</summary>
    public string Body { get; init; } = "";

    public NotifySource? Source { get; init; }

    public NotifyLink? Link { get; init; }

    /// <summary>Shown as one of the plugin's agents, by name, instead of the caller.</summary>
    public string? Agent { get; init; }

    /// <summary>Shown as the plugin itself, without an actor.</summary>
    public bool NoActor { get; init; }

    /// <summary>A member gets at most one notification of the plugin with this value (at most 120 bytes); never grouped.</summary>
    public string Once { get; init; } = "";
}

/// <summary>An approval or a decision.</summary>
public enum AskKind
{
    Approval,
    Decision,
}

/// <summary>
/// A button of an ask: it calls one of the plugin's <c>ui</c> endpoints (<c>Approve.Ref</c>) with this request, as the
/// member who presses it. The endpoint checks the member and resolves the ask.
/// </summary>
public sealed record NotifyButton
{
    private NotifyButton(string label, string endpoint, string body, bool primary)
    {
        Label = label;
        Endpoint = endpoint;
        Body = body;
        Primary = primary;
    }

    public string Label { get; }
    internal string Endpoint { get; }
    internal string Body { get; }

    /// <summary>The row's main button.</summary>
    public bool Primary { get; init; }

    /// <summary>An icon of the kit by name (<c>with { Icon = "check" }</c>).</summary>
    public string Icon { get; init; } = "";

    public static NotifyButton Of<TReq, TRes>(string label, EndpointRef<TReq, TRes> endpoint, TReq request)
        where TReq : class where TRes : class =>
        new(label, endpoint.Name, JsonSerializer.Serialize(request, CelobusJson.Options), false);

    /// <summary>The row's main button.</summary>
    public static NotifyButton PrimaryOf<TReq, TRes>(string label, EndpointRef<TReq, TRes> endpoint, TReq request)
        where TReq : class where TRes : class =>
        Of(label, endpoint, request) with { Primary = true };

    internal V1.NotifyButton Wire() => new() { Label = Label, Endpoint = Endpoint, Body = Body, Primary = Primary, Icon = Icon };
}

/// <summary>
/// An approval or a decision a member answers in their inbox (NEEDS YOU): 1 to 3 buttons that call the plugin's own
/// endpoints, and a review link. It stays until it is resolved (<c>Resolve</c>) or the member marks it done; asking
/// again refreshes the open one of its key.
/// </summary>
public sealed class Ask(AskKind kind, NotifyKey key)
{
    public AskKind Kind { get; } = kind;

    /// <summary>Its identity.</summary>
    public NotifyKey Key { get; } = key;

    public IEnumerable<Id<Platform.User>> To { get; init; } = [];
    public string Title { get; init; } = "";
    public string Body { get; init; } = "";
    public NotifySource? Source { get; init; }

    /// <summary>What the review link opens.</summary>
    public NotifyLink? Link { get; init; }

    /// <summary>The review link's label ("Review 5 changes").</summary>
    public string Review { get; init; } = "";

    public IEnumerable<NotifyButton> Buttons { get; init; } = [];

    /// <summary>Shown as one of the plugin's agents, by name, instead of the caller.</summary>
    public string? Agent { get; init; }

    public bool NoActor { get; init; }
}

/// <summary>How members come to follow a key.</summary>
public enum FollowReason
{
    /// <summary>The member asked to (a Follow button); resumes a follow they stopped.</summary>
    Manual,

    /// <summary>They replied: only members whose setting "follow threads I reply in" is on and who never stopped following the key.</summary>
    Replied,

    /// <summary>They edited: as <see cref="Replied"/>, with the setting "follow records I edit".</summary>
    Edited,
}

/// <summary>A follow of a key: why, what the inbox calls it, and whom (default: the caller).</summary>
public sealed class Follow
{
    public FollowReason Reason { get; init; } = FollowReason.Manual;

    /// <summary>What the inbox calls the key ("thread": "Stop following this thread"), at most 24 characters.</summary>
    public string Noun { get; init; } = "";

    /// <summary>
    /// These members (at most 100) instead of the caller: with <see cref="FollowReason.Replied"/> or
    /// <see cref="FollowReason.Edited"/> only (a manual follow is the caller's own; naming anyone else is <c>invalid</c>).
    /// </summary>
    public IEnumerable<Id<Platform.User>> Users { get; init; } = [];
}

/// <summary>
/// The plugin's notifications (<c>ctx.Notifications</c>; <c>tx.Notifications</c> writes in the transaction, and its
/// members hear of it when it commits). A notification never undoes the transaction: a call the platform or a limit
/// refuses in it is let go (logged) and returns normally. Send late in a transaction: a send holds the unread groups
/// it grew until the commit. Needs the capability <c>push:send</c> (403 <c>capability_missing</c>); a send the
/// platform refuses is 400 <c>invalid</c>; sends over 50 a second are let go; a manual follow of a key with 2,000
/// followers, outside a transaction, is 409 <c>follow_limit</c>. A send answers nothing about who received it.
/// </summary>
public readonly struct Notifications
{
    private readonly Ctx _ctx;

    internal Notifications(Ctx ctx) { _ctx = ctx; }

    private static V1.NotifyActor Actor(string? agent, bool none) => agent is { Length: > 0 }
        ? new V1.NotifyActor { Agent = agent }
        : none ? new V1.NotifyActor { None = true } : new V1.NotifyActor { Caller = true };

    private V1.NotifySend Wire(string kind, NotifyKey? key, IEnumerable<Id<Platform.User>> to, string title, string body,
        NotifySource? source, NotifyLink? link, string? agent, bool noActor) => new()
    {
        RequestId = _ctx.Request.Id,
        Tx = _ctx.Tx,
        Kind = kind,
        To = { to.Select(u => u.Value) },
        Key = key?.Key ?? "",
        Subject = key?.Subject ?? "",
        Actor = Actor(agent, noActor),
        Title = title,
        Body = body,
        Source = source is null ? null : new V1.NotifySource { Label = source.Label, Icon = source.Icon },
        Link = link?.Wire(),
    };

    /// <summary>Sends a notification of one of the plugin's kinds.</summary>
    public async Task Send(NotificationKind kind, Notice notice)
    {
        var send = Wire(kind.Name, notice.Key ?? notice.Followers, notice.To, notice.Title, notice.Body, notice.Source,
            notice.Link, notice.Agent, notice.NoActor);
        send.Followers = notice.Followers is not null;
        send.Except.AddRange(notice.Except.Select(u => u.Value));
        send.Once = notice.Once;
        await _ctx.Syscall(new V1.Syscall { NotifySend = send });
    }

    /// <summary>Asks members for an approval or a decision.</summary>
    public async Task Ask(Ask ask)
    {
        var send = Wire("", ask.Key, ask.To, ask.Title, ask.Body, ask.Source, ask.Link, ask.Agent, ask.NoActor);
        send.Ask = new V1.NotifyAsk
        {
            Kind = ask.Kind == AskKind.Decision ? V1.NotifyAsk.Types.Kind.Decision : V1.NotifyAsk.Types.Kind.Approval,
            Buttons = { ask.Buttons.Select(b => b.Wire()) },
            Review = ask.Review,
        };
        await _ctx.Syscall(new V1.Syscall { NotifySend = send });
    }

    /// <summary>
    /// Resolves every open ask of <paramref name="key"/>, for every member it asked: <paramref name="outcome"/>
    /// ("Approved by Sarah Bennett", at most 80 characters) is what the inbox shows.
    /// </summary>
    public async Task Resolve(NotifyKey key, string outcome) =>
        await _ctx.Syscall(new V1.Syscall
        {
            NotifyResolve = new V1.NotifyResolve { RequestId = _ctx.Request.Id, Tx = _ctx.Tx, Key = key.Key, Outcome = outcome },
        });

    private V1.NotifyFollow FollowWire(NotifyKey key, V1.NotifyFollow.Types.Op op, IEnumerable<Id<Platform.User>>? users,
        string noun = "", V1.NotifyFollow.Types.Reason reason = V1.NotifyFollow.Types.Reason.Unspecified) => new()
    {
        RequestId = _ctx.Request.Id,
        Tx = _ctx.Tx,
        Key = key.Key,
        Subject = key.Subject,
        Noun = noun,
        Users = { (users ?? []).Select(u => u.Value) },
        Reason = reason,
        Op = op,
    };

    /// <summary>Makes the caller (or, replied or edited, <see cref="Celobus.Sdk.Follow.Users"/>) follow <paramref name="key"/>.</summary>
    public async Task Follow(NotifyKey key, Follow? follow = null)
    {
        follow ??= new Follow();
        var reason = follow.Reason switch
        {
            FollowReason.Replied => V1.NotifyFollow.Types.Reason.Replied,
            FollowReason.Edited => V1.NotifyFollow.Types.Reason.Edited,
            _ => V1.NotifyFollow.Types.Reason.Manual,
        };
        await _ctx.Syscall(new V1.Syscall
        {
            NotifyFollow = FollowWire(key, V1.NotifyFollow.Types.Op.Follow, follow.Users, follow.Noun, reason),
        });
    }

    /// <summary>The caller stops following <paramref name="key"/>; an automatic follow does not resume it.</summary>
    public async Task Unfollow(NotifyKey key) =>
        await _ctx.Syscall(new V1.Syscall { NotifyFollow = FollowWire(key, V1.NotifyFollow.Types.Op.Unfollow, null) });

    /// <summary>Whether the caller follows <paramref name="key"/> (for a "Following" button): the one thing a plugin may read of notifications.</summary>
    public async Task<bool> Following(NotifyKey key)
    {
        var r = await _ctx.Syscall(new V1.Syscall { NotifyFollow = FollowWire(key, V1.NotifyFollow.Types.Op.Get, null) });
        return (r.NotifyFollowState ?? throw PluginError.Unavailable("the platform answered a follow with something else")).Following;
    }
}
