// Messages and a small plugin used by the tests.
using Celobus.Sdk;
using Celobus.V1;
using Platform = Celobus.Sdk.Platform;

namespace Celobus.Sdk.Tests;

[Message]
public enum Status { Placed, OnHold }

[Message] public sealed record Line(string Sku, int Quantity);

[Entity(SoftDelete = true)]
public sealed record Thing(Id<Thing> Id, string Name);

// column modifiers: printed on the entity's fields only
[Entity]
public sealed record Sku(Id<Sku> Id, [Unique] string Code, [Index] string? Shelf, [Encrypted] string? SupplierNote);

[Message]
public sealed record Sample(
    string Text, long Big, int Small, double Ratio, bool Flag, decimal Amount, Guid Uuid, DateTimeOffset At,
    DateOnly Day, byte[] Data, Status Status, Status? Maybe, IReadOnlyList<Line> Lines, Line? Single,
    Id<Thing> ThingId, Id<Platform.User>? Owner, string? Note, long[] Numbers);

/// <summary>A message without a primary constructor: fields in declaration order.</summary>
[Message, Describe("Init-only properties.")]
public sealed class Props
{
    public required string Zeta { get; init; }

    [Describe("The second field.")]
    public int Alpha { get; set; }

    public string Computed => Zeta + Alpha; // get-only: not a field
}

[Message] public sealed record PlaceOrderRequest(Id<Thing> ThingId, [property: Describe("How many.")] int Quantity);
[Message] public sealed record PlaceOrderReply(Id<Thing> Id);

public sealed class PlaceOrder : Endpoint<PlaceOrderRequest, PlaceOrderReply>
{
    public override Task<PlaceOrderReply> Handle(Ctx ctx, PlaceOrderRequest req) => Task.FromResult(new PlaceOrderReply(req.ThingId));
}

[Message] public sealed record FailRequest(string Kind);
[Message] public sealed record FailReply(string Text);

/// <summary>Fails the way the request asks.</summary>
public sealed class Fail : Endpoint<FailRequest, FailReply>
{
    public override async Task<FailReply> Handle(Ctx ctx, FailRequest req)
    {
        switch (req.Kind)
        {
            case "invalid": throw PluginError.Invalid("bad request").Field("kind", "not a kind");
            case "unauthenticated": ctx.Caller.User(); break;
            case "forbidden": throw PluginError.Forbidden("not yours");
            case "not_found": throw PluginError.NotFound("no such thing");
            case "conflict": throw PluginError.Conflict("taken");
            case "unavailable": throw PluginError.Unavailable("try later");
            case "domain": throw PluginError.Domain("not_cancellable", 409, "the order is shipped");
            case "db": await ctx.Db<Thing>().Get(new Id<Thing>("1")); break;
            case "crash": throw new InvalidOperationException("a secret detail");
            case "null": return null!;
            case "whoami": return new FailReply($"{ctx.Caller} {ctx.Caller.Has(Roles.Clerk)} {ctx.Caller.Has(Role.WorkspaceAdmin)}");
            case "call": return new FailReply(await ctx.CallRaw("other", "Get", """{"id":"1"}"""));
            case "call_typed": return await ctx.Call<FailReply>("other", "Get", new { ThingId = new Id<Thing>(5), Count = 3L });
        }
        return new FailReply(req.Kind);
    }
}

[Message] public sealed record SlowRequest(int Ms);

public sealed class Slow : Endpoint<SlowRequest, FailReply>
{
    public override void Configure(EndpointOptions e) => e.Timeout(TimeSpan.FromMilliseconds(200));

    /// <summary>Completed when a request of <c>ms</c> starts (a test waits for its request to be in flight).</summary>
    public static readonly System.Collections.Concurrent.ConcurrentDictionary<int, TaskCompletionSource> Started = new();

    public override async Task<FailReply> Handle(Ctx ctx, SlowRequest req)
    {
        Started.GetOrAdd(req.Ms, _ => new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously)).TrySetResult();
        await Task.Delay(req.Ms, ctx.Cancelled);
        return new FailReply("done");
    }
}

[Message] public sealed record GateRequest(int N);

/// <summary>Runs until its test opens <see cref="Open"/>: a request that is surely in flight, with time to spare.</summary>
public sealed class Gate : Endpoint<GateRequest, FailReply>
{
    public override void Configure(EndpointOptions e) => e.Timeout(TimeSpan.FromSeconds(30));

    public static readonly System.Collections.Concurrent.ConcurrentDictionary<int, TaskCompletionSource> Started = new();
    public static readonly System.Collections.Concurrent.ConcurrentDictionary<int, TaskCompletionSource> Open = new();

    public static TaskCompletionSource Of(System.Collections.Concurrent.ConcurrentDictionary<int, TaskCompletionSource> d, int n) =>
        d.GetOrAdd(n, _ => new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously));

    public override async Task<FailReply> Handle(Ctx ctx, GateRequest req)
    {
        Of(Started, req.N).TrySetResult();
        await Of(Open, req.N).Task.WaitAsync(ctx.Cancelled);
        return new FailReply("done");
    }
}

[Message] public sealed record LineKey(string OrderId, long Line);
[Message] public sealed record AllLines();
[Message] public sealed record TouchRequest(string OrderId, long Line);

public static class LineTopics
{
    public static readonly Ui.Topic<LineKey> Line = new("line");
    public static readonly Ui.Topic<AllLines> Lines = new("lines");
}

/// <summary>Invalidates the line it is given, and every line.</summary>
public sealed class Touch : Endpoint<TouchRequest, AllLines>
{
    public override Task<AllLines> Handle(Ctx ctx, TouchRequest req)
    {
        ctx.Invalidate(LineTopics.Line, new LineKey(req.OrderId, req.Line));
        ctx.InvalidateAll(LineTopics.Lines);
        return Task.FromResult(new AllLines());
    }
}

public static class Roles
{
    public static readonly Role Clerk = new("clerk") { Title = "Clerk" };
}

public static class TestPlugin
{
    public static Plugin Define() => Plugin.Create("tests")
        .Role(Roles.Clerk)
        .Endpoint<Fail>()
        .Endpoint<Slow>()
        .Endpoint<Gate>()
        .Endpoint<PlaceOrder>();

    /// <summary>A plugin with only <see cref="Touch"/>.</summary>
    public static Plugin Lines() => Plugin.Create("lines").Version("1.0.0").Endpoint<Touch>();

    internal static Request Post(string endpoint, string json, Principal? principal = null)
    {
        var r = new Request
        {
            Id = 1,
            Method = "POST",
            Path = $"/tests.v1.Tests/{endpoint}",
            Body = Google.Protobuf.ByteString.CopyFromUtf8(json),
            Principal = principal,
        };
        r.Headers.Add(new Header { Name = "content-type", Value = "application/json" });
        return r;
    }
}

/// <summary>Syscalls answered in memory.</summary>
internal sealed class FakeSyscalls(Func<Syscall, SyscallResult> answer) : ISyscalls
{
    public List<Syscall> Calls { get; } = [];

    public Task<SyscallResult> Call(Syscall call, CancellationToken ct)
    {
        lock (Calls) Calls.Add(call);
        var r = answer(call);
        r.Id = call.Id;
        return Task.FromResult(r);
    }

    public List<(string Topic, string Key)> Invalidations { get; } = [];

    public void Invalidate(string topic, string key)
    {
        lock (Invalidations) Invalidations.Add((topic, key));
    }
}
