// Topics (channels the plugin announces changes on: invalidations only, never data), their declarations, endpoint
// invalidations, and the request/key/params arguments that queries, steps and links set.
using System.Linq.Expressions;
using System.Text.Json.Nodes;

namespace Celobus.Sdk.Ui;

/// <summary>
/// A topic: <c>public static readonly Topic&lt;OrderKey&gt; Order = new("order");</c>. Endpoints declare that they
/// invalidate it (<c>e.Invalidates(Order)</c>); queries follow it; the plugin declares it (<c>.Topic(Order.Roles(..))</c>).
/// An unkeyed topic has an empty key message.
/// </summary>
public sealed class Topic<TKey>(string name) where TKey : class
{
    public string Name { get; } = name;

    /// <summary>The declaration (roles, scopes, the authorizing endpoint).</summary>
    public TopicDecl Decl() => new(Name, typeof(TKey));

    /// <summary>Any of these roles may follow it.</summary>
    public TopicDecl Roles(params Role[] roles) => Decl().Roles(roles);

    /// <summary>One key of the topic: <c>Order.Key(k =&gt; k.Set(x =&gt; x.Id, id))</c>.</summary>
    public FollowKey Key(Action<Args<TKey>> key)
    {
        var a = new Args<TKey>();
        key(a);
        return new FollowKey(Name, a.List);
    }

    /// <summary>The invalidated key's field <paramref name="key"/> taken from the response field <paramref name="from"/>: <c>Order.By(k =&gt; k.Id, (Order o) =&gt; o.Id)</c>.</summary>
    public Invalidation<TKey> By<TRes, T>(Expression<Func<TKey, T>> key, Expression<Func<TRes, T>> from) =>
        new Invalidation<TKey>(Name).And(key, from);

    public static implicit operator TopicDecl(Topic<TKey> t) => t.Decl();

    public static implicit operator Invalidation(Topic<TKey> t) => new(t.Name);

    public static implicit operator FollowKey(Topic<TKey> t) => new(t.Name, []);
}

/// <summary>A topic as the plugin declares it (<c>.Topic(..)</c>).</summary>
public sealed class TopicDecl
{
    internal string Name { get; }
    internal Type KeyType { get; }
    private readonly List<string> _roles = [];
    private readonly List<string> _scopes = [];
    private string? _authorize;
    private bool _manual;
    private string _description = "";

    internal TopicDecl(string name, Type keyType)
    {
        Name = name;
        KeyType = keyType;
    }

    public TopicDecl Roles(params Role[] roles)
    {
        _roles.Clear();
        _roles.AddRange(roles.Select(r => r.Name));
        return this;
    }

    public TopicDecl Scopes(params Scope[] scopes)
    {
        _scopes.Clear();
        _scopes.AddRange(scopes.Select(s => s.Name));
        return this;
    }

    /// <summary>The read endpoint that decides whether a member may follow one key (it takes the key's fields).</summary>
    public TopicDecl Authorize<TReq, TRes>(EndpointRef<TReq, TRes> endpoint) where TReq : class where TRes : class
    {
        _authorize = endpoint.Name;
        return this;
    }

    /// <summary>Invalidated by code rather than by endpoint declarations.</summary>
    public TopicDecl Manual()
    {
        _manual = true;
        return this;
    }

    public TopicDecl Describe(string description)
    {
        _description = description;
        return this;
    }

    internal JsonObject ToJson()
    {
        var o = new JsonObject { ["name"] = Name, ["key"] = KeyType.Name, ["description"] = _description };
        if (_roles.Count > 0) o["roles"] = new JsonArray([.. _roles.Select(r => (JsonNode)r)]);
        if (_scopes.Count > 0) o["scopes"] = new JsonArray([.. _scopes.Select(s => (JsonNode)s)]);
        if (_authorize is not null) o["authorize"] = _authorize;
        if (_manual) o["manual"] = true;
        return o;
    }
}

/// <summary>An endpoint's declared invalidation: a topic, and the key taken from the response (none: every key).</summary>
public class Invalidation
{
    internal string Topic { get; }
    internal List<(string Field, string From)> Key { get; } = [];

    internal Invalidation(string topic) { Topic = topic; }

    internal JsonObject ToJson()
    {
        var o = new JsonObject { ["topic"] = Topic };
        if (Key.Count > 0)
            o["key"] = new JsonArray([.. Key.Select(k => (JsonNode)new JsonObject { ["field"] = k.Field, ["from"] = k.From })]);
        return o;
    }
}

/// <summary>A keyed invalidation of a <see cref="Topic{TKey}"/>; <c>.And(..)</c> adds a key field.</summary>
public sealed class Invalidation<TKey> : Invalidation
{
    internal Invalidation(string topic) : base(topic) { }

    /// <summary>Another key field, taken from a response field.</summary>
    public Invalidation<TKey> And<TRes, T>(Expression<Func<TKey, T>> key, Expression<Func<TRes, T>> from)
    {
        Key.Add((FieldPath.Of(key), FieldPath.Of(from)));
        return this;
    }
}

/// <summary>A followed topic, with a key value (<c>Order.Key(..)</c>) or any key (the topic itself).</summary>
public sealed class FollowKey
{
    internal string Topic { get; }
    internal IReadOnlyList<JsonObject> Key { get; }

    internal FollowKey(string topic, IReadOnlyList<JsonObject> key)
    {
        Topic = topic;
        Key = key;
    }
}

/// <summary>Request (or key, or params) fields set to values: <c>c =&gt; c.Arg(r =&gt; r.Id, row.Get(o =&gt; o.Id))</c>.</summary>
public sealed class Args<TMsg>
{
    internal List<JsonObject> List { get; } = [];

    internal Args() { }

    /// <summary>Sets a field to a value (a literal converts); an optional field also takes a value that is never absent (a param with a default).</summary>
    public Args<TMsg> Arg<T>(Expression<Func<TMsg, T?>> field, Val<T> value)
    {
        List.Add(Of(FieldPath.Of(field), Val<T>.Json(value)));
        return this;
    }

    /// <summary>Sets an optional field to a value of its inner type.</summary>
    public Args<TMsg> Arg<T>(Expression<Func<TMsg, T?>> field, IVal<T> value) where T : struct
    {
        List.Add(Of(FieldPath.Of(field), Val<T>.Json(value)));
        return this;
    }

    /// <summary>The same as <c>Arg</c>, for a key.</summary>
    public Args<TMsg> Set<T>(Expression<Func<TMsg, T?>> field, Val<T> value) => Arg(field, value);

    /// <summary>The same as <c>Arg</c>, for an optional key field.</summary>
    public Args<TMsg> Set<T>(Expression<Func<TMsg, T?>> field, IVal<T> value) where T : struct => Arg(field, value);

    internal static JsonObject Of(string field, JsonNode value) => new() { ["field"] = field, ["value"] = value };

    internal static List<JsonObject> Build(Action<Args<TMsg>>? args)
    {
        var a = new Args<TMsg>();
        args?.Invoke(a);
        return a.List;
    }
}

/// <summary>A screen's id and its params message: <c>public static readonly ScreenKey&lt;OrderParams&gt; Order = new("order");</c>.</summary>
public sealed class ScreenKey<TParams>(string id) where TParams : class
{
    public string Id { get; } = id;
}

/// <summary>An overlay's id and its params message.</summary>
public sealed class OverlayKey<TParams>(string id) where TParams : class
{
    public string Id { get; } = id;
}

/// <summary>Where a step, link or command goes: a screen or an overlay with its params.</summary>
internal static class Target
{
    public static JsonObject Of(string? screen, string? overlay, List<JsonObject> args)
    {
        var o = new JsonObject();
        if (screen is not null) o["screen"] = screen;
        if (overlay is not null) o["overlay"] = overlay;
        if (args.Count > 0) o["args"] = new JsonArray([.. args]);
        return o;
    }
}
