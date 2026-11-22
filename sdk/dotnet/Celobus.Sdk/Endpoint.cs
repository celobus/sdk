namespace Celobus.Sdk;

/// <summary>Who may call an endpoint: a signed-in user (the default), a service account, either, or anyone.</summary>
public enum AuthKind
{
    User,
    Service,
    Any,
    Public,
}

/// <summary>Whether an endpoint takes an <c>Idempotency-Key</c>.</summary>
public enum IdempotencyMode
{
    Off,
    Optional,
    Required,
}

/// <summary>
/// An endpoint: a unary RPC of the plugin's service, <c>POST /&lt;id&gt;.v1.&lt;Service&gt;/&lt;Name&gt;</c>. Its name is
/// the request type's name minus <c>Request</c> (<c>PlaceOrderRequest</c> → <c>PlaceOrder</c>).
/// <code>
/// public sealed class Hello : Endpoint&lt;HelloRequest, HelloReply&gt;
/// {
///     public override void Configure(EndpointOptions e) => e.Roles(Roles.Clerk).Describe("Greets.");
///     public override Task&lt;HelloReply&gt; Handle(Ctx ctx, HelloRequest req) => Task.FromResult(new HelloReply($"Hello, {req.Name}!"));
/// }
/// </code>
/// One instance serves every request, concurrently: keep no per-request state in fields.
/// </summary>
public abstract class Endpoint<TReq, TRes> : EndpointBase where TReq : class where TRes : class
{
    /// <summary>The endpoint as UI declarations name it: <c>Query.Call(ListOrders.Ref)</c>, <c>a.Call(CancelOrder.Ref, ..)</c>.</summary>
    public static EndpointRef<TReq, TRes> Ref { get; } = new();

    /// <summary>The endpoint's options (description, roles, scopes, auth, idempotency, timeout).</summary>
    public virtual void Configure(EndpointOptions e) { }

    /// <summary>Serves one request.</summary>
    public abstract Task<TRes> Handle(Ctx ctx, TReq req);

    internal override Type RequestType => typeof(TReq);
    internal override Type ResponseType => typeof(TRes);
    internal override void ConfigureCore(EndpointOptions e) => Configure(e);
    internal override async Task<object?> HandleCore(Ctx ctx, object req) => await Handle(ctx, (TReq)req);
}

/// <summary>
/// An endpoint by its request and response types (<c>ListOrders.Ref</c>), for UI queries and steps, and the topic that
/// an endpoint authorizes. Its name is the request type's name minus <c>Request</c>.
/// </summary>
public sealed class EndpointRef<TReq, TRes> where TReq : class where TRes : class
{
    internal EndpointRef() { }

    public string Name { get; } = EndpointName.Of(typeof(TReq));
}

internal static class EndpointName
{
    /// <summary><c>PlaceOrderRequest</c> → <c>PlaceOrder</c>.</summary>
    public static string Of(Type request)
    {
        var name = request.Name;
        return name.EndsWith("Request", StringComparison.Ordinal) && name.Length > "Request".Length
            ? name[..^"Request".Length]
            : name;
    }
}

/// <summary>The non-generic base of <see cref="Endpoint{TReq, TRes}"/>; derive from that.</summary>
public abstract class EndpointBase
{
    private protected EndpointBase() { }

    internal abstract Type RequestType { get; }
    internal abstract Type ResponseType { get; }
    internal abstract void ConfigureCore(EndpointOptions e);
    internal abstract Task<object?> HandleCore(Ctx ctx, object req);
}

/// <summary>The options of an endpoint, set in its <c>Configure</c>.</summary>
public sealed class EndpointOptions
{
    internal string Description { get; private set; } = "";
    internal List<Role> RoleList { get; } = [];
    internal List<string>? ScopeList { get; private set; }
    internal bool IsReadOnly { get; private set; }
    internal AuthKind AuthKind { get; private set; } = AuthKind.User;
    internal IdempotencyMode IdempotencyMode { get; private set; } = IdempotencyMode.Off;
    internal TimeSpan TimeoutValue { get; private set; } = TimeSpan.FromSeconds(30);
    internal bool IsUi { get; private set; }
    internal List<Ui.Invalidation> InvalidationList { get; } = [];

    /// <summary>The scopes the endpoint requires: the declared ones, else <c>read</c> when read-only, else <c>write</c>.</summary>
    internal IReadOnlyList<string> EffectiveScopes => ScopeList ?? [IsReadOnly ? Scope.Read.Name : Scope.Write.Name];

    /// <summary>Display text and the MCP tool description.</summary>
    public EndpointOptions Describe(string text)
    {
        Description = text;
        return this;
    }

    /// <summary>Any of these roles admits the caller (a plugin role or <c>Role.WorkspaceAdmin</c>); none: any member.</summary>
    public EndpointOptions Roles(params Role[] roles)
    {
        RoleList.AddRange(roles);
        return this;
    }

    /// <summary>All of these scopes are required (default <c>write</c>, or <c>read</c> with <see cref="ReadOnly"/>).</summary>
    public EndpointOptions Scopes(params Scope[] scopes)
    {
        ScopeList = [.. ScopeList ?? [], .. scopes.Select(s => s.Name)];
        return this;
    }

    /// <summary>The endpoint only reads: it requires the <c>read</c> scope instead of <c>write</c>.</summary>
    public EndpointOptions ReadOnly()
    {
        IsReadOnly = true;
        return this;
    }

    /// <summary>Who may call it (default <see cref="AuthKind.User"/>).</summary>
    public EndpointOptions Auth(AuthKind auth)
    {
        AuthKind = auth;
        return this;
    }

    /// <summary>Whether it takes an <c>Idempotency-Key</c> (default <see cref="IdempotencyMode.Off"/>).</summary>
    public EndpointOptions Idempotency(IdempotencyMode mode)
    {
        IdempotencyMode = mode;
        return this;
    }

    /// <summary>
    /// Screens may call it: queries (when it is <see cref="ReadOnly"/>) and actions. Other endpoints stay out of reach of
    /// a UI document.
    /// </summary>
    public EndpointOptions Ui()
    {
        IsUi = true;
        return this;
    }

    /// <summary>
    /// After a 2xx answer the platform invalidates the topic: every key (<c>.Invalidates(Topics.Orders)</c>), or the key
    /// taken from the response (<c>.Invalidates(Topics.Order.By(k =&gt; k.Id, (Order o) =&gt; o.Id))</c>); open screens
    /// following it re-read.
    /// </summary>
    public EndpointOptions Invalidates(Ui.Invalidation topic)
    {
        InvalidationList.Add(topic);
        return this;
    }

    /// <summary>The handler's deadline (default 30 s, at most 600 s); <c>ctx.Cancelled</c> fires when it passes.</summary>
    public EndpointOptions Timeout(TimeSpan timeout)
    {
        TimeoutValue = timeout;
        return this;
    }
}

/// <summary>A registered endpoint: its instance, name and options.</summary>
internal sealed class EndpointRegistration(EndpointBase endpoint, string name, EndpointOptions options)
{
    public EndpointBase Endpoint { get; } = endpoint;
    public string Name { get; } = name;
    public EndpointOptions Options { get; } = options;
}
