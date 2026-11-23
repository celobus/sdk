using System.Collections.Concurrent;
using System.Text.Json.Nodes;
using System.Threading.Channels;
using Celobus.V1;
using Google.Protobuf;
using Grpc.Core;
using Hello;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Server.Kestrel.Core;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;

namespace Celobus.Sdk.Tests;

/// <summary>
/// A fake platform: a server on a temporary local address. It answers Register and the hello plugin's greetings (the
/// transaction, the create and the count of <c>greeting</c>) itself, passes every other
/// plugin message to the test (<see cref="FromPlugin"/>), and sends what the test writes to <see cref="ToPlugin"/>.
/// </summary>
internal sealed class FakeCelld : Celld.CelldBase
{
    public Channel<PluginMessage> FromPlugin { get; } = Channel.CreateUnbounded<PluginMessage>();
    public Channel<CelldMessage> ToPlugin { get; } = Channel.CreateUnbounded<CelldMessage>();
    /// <summary>The names greeted: the rows of <c>greeting</c>.</summary>
    public ConcurrentQueue<string> Greetings { get; } = new();
    public int Sessions;
    private TaskCompletionSource _drop = new(TaskCreationOptions.RunContinuationsAsynchronously);

    /// <summary>Ends the current session from the server's side.</summary>
    public void Drop() => _drop.TrySetResult();

    public override async Task Session(IAsyncStreamReader<PluginMessage> requestStream,
        IServerStreamWriter<CelldMessage> responseStream, ServerCallContext context)
    {
        var n = Interlocked.Increment(ref Sessions);
        _drop = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var drop = _drop.Task;
        using var end = CancellationTokenSource.CreateLinkedTokenSource(context.CancellationToken);
        var send = Channel.CreateUnbounded<CelldMessage>();
        // the only writer of the response stream: syscall results and the test's messages
        var pump = Task.Run(async () =>
        {
            var fromTest = ToPlugin.Reader.ReadAllAsync(end.Token).GetAsyncEnumerator();
            var mine = send.Reader.ReadAllAsync(end.Token).GetAsyncEnumerator();
            Task<bool>? a = null, b = null;
            while (true)
            {
                a ??= fromTest.MoveNextAsync().AsTask();
                b ??= mine.MoveNextAsync().AsTask();
                var done = await Task.WhenAny(a, b);
                if (!await done) return;
                await responseStream.WriteAsync(done == a ? fromTest.Current : mine.Current);
                if (done == a) a = null; else b = null;
            }
        });

        var read = Task.Run(async () =>
        {
            await foreach (var m in requestStream.ReadAllAsync(end.Token))
            {
                switch (m.KindCase)
                {
                    case PluginMessage.KindOneofCase.Register:
                        await send.Writer.WriteAsync(new CelldMessage { Registered = new Registered { InstanceId = $"i-{n}" } });
                        break;
                    case PluginMessage.KindOneofCase.Syscall when m.Syscall.CallCase is Syscall.CallOneofCase.TxBegin:
                        await send.Writer.WriteAsync(Result(m.Syscall.Id, r => r.TxBegun = new TxBegun { Tx = 1 }));
                        continue;
                    case PluginMessage.KindOneofCase.Syscall when m.Syscall.CallCase is Syscall.CallOneofCase.TxCommit:
                        await send.Writer.WriteAsync(Result(m.Syscall.Id, r => r.Ok = new Empty()));
                        continue;
                    case PluginMessage.KindOneofCase.Syscall when m.Syscall.CallCase is Syscall.CallOneofCase.DbCreate
                        && m.Syscall.DbCreate.Entity == "greeting":
                        var name = System.Text.Json.Nodes.JsonNode.Parse(m.Syscall.DbCreate.Rows[0].ToStringUtf8())!["name"]!.GetValue<string>();
                        Greetings.Enqueue(name);
                        var row = ByteString.CopyFromUtf8(
                            $$"""{"id":"{{Greetings.Count}}","name":"{{name}}","createdAt":"2026-11-12T10:00:00Z","updatedAt":"2026-11-12T10:00:00Z"}""");
                        await send.Writer.WriteAsync(Result(m.Syscall.Id, r => r.DbRows = new DbRows { Rows = { row } }));
                        continue;
                    case PluginMessage.KindOneofCase.Syscall when m.Syscall.CallCase is Syscall.CallOneofCase.DbCount
                        && m.Syscall.DbCount.Entity == "greeting":
                        var of = m.Syscall.DbCount.Filter.Condition.Values[0].StringValue;
                        await send.Writer.WriteAsync(Result(m.Syscall.Id,
                            r => r.DbCount = new DbCountResult { Count = (ulong)Greetings.Count(g => g == of) }));
                        continue;
                }
                await FromPlugin.Writer.WriteAsync(m);
            }
        });

        await Task.WhenAny(read, drop);
        await end.CancelAsync();
        try { await Task.WhenAll(pump, read); }
        catch (OperationCanceledException) { }
    }

    private static CelldMessage Result(ulong id, Action<SyscallResult> fill)
    {
        var r = new SyscallResult { Id = id };
        fill(r);
        return new CelldMessage { SyscallResult = r };
    }

    /// <summary>The next plugin message of this kind (others are skipped).</summary>
    public async Task<PluginMessage> Next(PluginMessage.KindOneofCase kind)
    {
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        while (true)
        {
            var m = await FromPlugin.Reader.ReadAsync(timeout.Token);
            if (m.KindCase == kind) return m;
        }
    }
}

public sealed class SessionTests : IAsyncLifetime
{
    private readonly string _socket = Path.Combine(Path.GetTempPath(), $"celld-{Guid.NewGuid():N}"[..20] + ".sock");
    private readonly FakeCelld _celld = new();
    private WebApplication? _app;

    public async Task InitializeAsync()
    {
        var builder = WebApplication.CreateSlimBuilder();
        builder.Logging.ClearProviders();
        builder.WebHost.ConfigureKestrel(k => k.ListenUnixSocket(_socket, l => l.Protocols = HttpProtocols.Http2));
        builder.Services.AddGrpc();
        builder.Services.AddSingleton(_celld);
        _app = builder.Build();
        _app.MapGrpcService<FakeCelld>();
        await _app.StartAsync();
    }

    public async Task DisposeAsync()
    {
        if (_app != null) await _app.DisposeAsync();
        File.Delete(_socket);
    }

    private static Request Hello(ulong id, string json)
    {
        var r = new Request
        {
            Id = id,
            Method = "POST",
            Path = "/hello.v1.Hello/Hello",
            Body = ByteString.CopyFromUtf8(json),
            Principal = new Principal { UserId = "u-1", Kind = Principal.Types.Kind.Person, Roles = { "hello.clerk" } },
        };
        r.Headers.Add(new Header { Name = "content-type", Value = "application/json" });
        return r;
    }

    private async Task<Response> Answer(ulong id)
    {
        while (true)
        {
            var m = await _celld.Next(PluginMessage.KindOneofCase.Response);
            if (m.Response.Id == id) return m.Response;
        }
    }

    [Fact]
    public async Task RegistersServesAndCountsInItsEntity()
    {
        using var stop = new CancellationTokenSource();
        var serving = new Runtime(HelloPlugin.Define().Definition).Run($"unix:{_socket}", stop.Token);

        var register = (await _celld.Next(PluginMessage.KindOneofCase.Register)).Register;
        Assert.Equal("hello", register.Plugin);
        Assert.Equal("0.1.0", register.Version);
        Assert.Equal("dotnet", register.Runtime);
        Assert.Equal(["greeter"], register.Depends);
        Assert.Equal("", register.ManifestDigest);
        Assert.Equal(0u, register.JobSlots); // no jobs: no job pool

        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = Hello(7, """{"name":"Sarah"}""") });
        var first = await Answer(7);
        Assert.Equal(200u, first.Status);
        Assert.Equal("""{"text":"Hello, Sarah!","count":"1"}""", first.Body.ToStringUtf8());
        Assert.Equal(["Sarah"], _celld.Greetings);

        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = Hello(8, """{"name":"Sarah","mood":"sad"}""") });
        var second = await Answer(8);
        Assert.Equal("""{"text":"Cheer up, Sarah.","count":"2"}""", second.Body.ToStringUtf8());

        await stop.CancelAsync();
        await serving.WaitAsync(TimeSpan.FromSeconds(10));
    }

    [Fact]
    public async Task ReconnectsWhenTheSessionEnds()
    {
        using var stop = new CancellationTokenSource();
        var serving = new Runtime(HelloPlugin.Define().Definition).Run($"unix:{_socket}", stop.Token);
        await _celld.Next(PluginMessage.KindOneofCase.Register);
        _celld.Drop();
        await _celld.Next(PluginMessage.KindOneofCase.Register);
        Assert.Equal(2, _celld.Sessions);

        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = Hello(9, """{"name":"Sam"}""") });
        Assert.Equal(200u, (await Answer(9)).Status);

        await stop.CancelAsync();
        await serving.WaitAsync(TimeSpan.FromSeconds(10));
    }

    [Fact]
    public async Task StopFinishesRequestsInFlight()
    {
        using var stop = new CancellationTokenSource();
        var serving = new Runtime(TestPlugin.Define().Version("1.0.0").Definition).Run($"unix:{_socket}", stop.Token);
        await _celld.Next(PluginMessage.KindOneofCase.Register);

        var gate = TestPlugin.Post("Gate", """{"n":1}""");
        gate.Id = 3;
        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = gate });
        await Gate.Of(Gate.Started, 1).Task.WaitAsync(TimeSpan.FromSeconds(10)); // in flight before the stop
        await stop.CancelAsync();
        Gate.Of(Gate.Open, 1).TrySetResult(); // it finishes only after the stop

        var answer = await Answer(3);
        Assert.Equal(200u, answer.Status);
        Assert.Equal("""{"text":"done"}""", answer.Body.ToStringUtf8());
        await serving.WaitAsync(TimeSpan.FromSeconds(10));
    }

    private static Request HoldRequest(ulong id)
    {
        var r = new Request
        {
            Id = id,
            Method = "POST",
            Path = "/crowd.v1.Crowd/Hold",
            Body = ByteString.CopyFromUtf8($$"""{"n":{{id}}}"""),
        };
        r.Headers.Add(new Header { Name = "content-type", Value = "application/json" });
        return r;
    }

    [Fact]
    public async Task RunsAtMost64RequestsQueues256AndRefusesTheRest()
    {
        using var stop = new CancellationTokenSource();
        var serving = new Runtime(TestPlugin.Crowd().Definition).Run($"unix:{_socket}", stop.Token);
        await _celld.Next(PluginMessage.KindOneofCase.Register);

        const int taken = Runtime.MaxAdmitted + Runtime.MaxQueued;
        Assert.Equal((64, 256), (Runtime.MaxAdmitted, Runtime.MaxQueued));
        for (ulong id = 1; id <= taken + 1; id++)
            await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = HoldRequest(id) });

        // the one past the queue is refused at once, with a hint when to come back
        var refused = await Answer(taken + 1);
        Assert.Equal(503u, refused.Status);
        var error = JsonNode.Parse(refused.Body.ToStringUtf8())!;
        Assert.Equal(("unavailable", "the plugin is busy"), (error["code"]!.GetValue<string>(), error["message"]!.GetValue<string>()));
        Assert.Contains(refused.Headers, h => h is { Name: "retry-after", Value: "1" });

        using (var wait = new CancellationTokenSource(TimeSpan.FromSeconds(10)))
            while (Volatile.Read(ref Hold.Running) < Runtime.MaxAdmitted) await Task.Delay(10, wait.Token);
        await Task.Delay(200); // no more start while the slots are taken
        Assert.Equal(Runtime.MaxAdmitted, Volatile.Read(ref Hold.Running));

        Hold.Release.TrySetResult();
        var answered = new HashSet<ulong>();
        while (answered.Count < taken)
        {
            var r = (await _celld.Next(PluginMessage.KindOneofCase.Response)).Response;
            Assert.Equal(200u, r.Status);
            answered.Add(r.Id);
        }
        Assert.Equal(Runtime.MaxAdmitted, Hold.MaxRunning);

        // the slots and the queue are free again
        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = HoldRequest(1000) });
        Assert.Equal(200u, (await Answer(1000)).Status);

        await stop.CancelAsync();
        await serving.WaitAsync(TimeSpan.FromSeconds(10));
    }

    [Fact]
    public async Task ALostSessionFreesTheSlotsAndTheQueue()
    {
        using var stop = new CancellationTokenSource();
        var runtime = new Runtime(TestPlugin.Define().Version("1.0.0").Definition);
        var serving = runtime.Run($"unix:{_socket}", stop.Token);
        await _celld.Next(PluginMessage.KindOneofCase.Register);

        const int taken = Runtime.MaxAdmitted + Runtime.MaxQueued;
        for (var n = 0; n < taken; n++)
        {
            var gate = TestPlugin.Post("Gate", $$"""{"n":{{100 + n}}}""");
            gate.Id = (ulong)(100 + n);
            await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = gate });
        }
        using (var wait = new CancellationTokenSource(TimeSpan.FromSeconds(10)))
            while (runtime.Taken < taken) await Task.Delay(10, wait.Token);

        _celld.Drop(); // the handlers are stopped, the queued requests dropped
        await _celld.Next(PluginMessage.KindOneofCase.Register);
        using (var wait = new CancellationTokenSource(TimeSpan.FromSeconds(10)))
            while (runtime.Taken > 0) await Task.Delay(10, wait.Token);

        var next = TestPlugin.Post("Gate", """{"n":99}""");
        next.Id = 99;
        Gate.Of(Gate.Open, 99).TrySetResult();
        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = next });
        Assert.Equal(200u, (await Answer(99)).Status);

        await stop.CancelAsync();
        await serving.WaitAsync(TimeSpan.FromSeconds(10));
    }

    [Fact]
    public async Task StopRefusesNewRequestsWithRetryAfter()
    {
        using var stop = new CancellationTokenSource();
        var serving = new Runtime(TestPlugin.Define().Version("1.0.0").Definition).Run($"unix:{_socket}", stop.Token);
        await _celld.Next(PluginMessage.KindOneofCase.Register);

        var gate = TestPlugin.Post("Gate", """{"n":21}""");
        gate.Id = 21;
        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = gate });
        await Gate.Of(Gate.Started, 21).Task.WaitAsync(TimeSpan.FromSeconds(10)); // keeps the session draining
        await stop.CancelAsync();
        await Task.Delay(500); // the plugin is draining

        var late = TestPlugin.Post("Gate", """{"n":22}""");
        late.Id = 22;
        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = late });
        var refused = await Answer(22);
        Assert.Equal(503u, refused.Status);
        var error = JsonNode.Parse(refused.Body.ToStringUtf8())!;
        Assert.Equal(("unavailable", "the plugin is stopping"), (error["code"]!.GetValue<string>(), error["message"]!.GetValue<string>()));
        Assert.Contains(refused.Headers, h => h is { Name: "retry-after", Value: "1" });

        Gate.Of(Gate.Open, 21).TrySetResult();
        Assert.Equal(200u, (await Answer(21)).Status);
        await serving.WaitAsync(TimeSpan.FromSeconds(10));
    }

    [Fact]
    public async Task InvalidateSendsTheCanonicalKeyOnTheSession()
    {
        using var stop = new CancellationTokenSource();
        var serving = new Runtime(TestPlugin.Lines().Definition).Run($"unix:{_socket}", stop.Token);
        await _celld.Next(PluginMessage.KindOneofCase.Register);

        var touch = new Request
        {
            Id = 4,
            Method = "POST",
            Path = "/lines.v1.Lines/Touch",
            Body = ByteString.CopyFromUtf8("""{"orderId":"1048","line":"2"}"""),
        };
        touch.Headers.Add(new Header { Name = "content-type", Value = "application/json" });
        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { Request = touch });

        var one = (await _celld.Next(PluginMessage.KindOneofCase.Invalidate)).Invalidate;
        Assert.Equal(("line", """{"line":"2","order_id":"1048"}"""), (one.Topic, one.Key));
        var all = (await _celld.Next(PluginMessage.KindOneofCase.Invalidate)).Invalidate;
        Assert.Equal(("lines", ""), (all.Topic, all.Key));
        Assert.Equal(200u, (await Answer(4)).Status);

        await stop.CancelAsync();
        await serving.WaitAsync(TimeSpan.FromSeconds(10));
    }

    private static JobRun Run(string job, string runId, string payload) => new()
    {
        RunId = runId, Job = job, Attempt = 1, MaxAttempts = 1, Payload = payload, TimeoutMs = 10_000, RequestId = 31,
        ScheduledAtUnixMs = 1_800_000_000_000,
    };

    [Fact]
    public async Task AnnouncesItsJobSlotsAndAnswersARunWithItsLog()
    {
        using var stop = new CancellationTokenSource();
        var serving = new Runtime(Jobsdemo.JobsdemoPlugin.Define().Definition).Run($"unix:{_socket}", stop.Token);
        Assert.Equal(16u, (await _celld.Next(PluginMessage.KindOneofCase.Register)).Register.JobSlots);

        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { JobRun = Run("daily_report", "run-r", """{"day":"2026-11-20"}""") });
        var line = (await _celld.Next(PluginMessage.KindOneofCase.PluginLog)).PluginLog;
        Assert.Equal((31ul, PluginLog.Types.Level.Info, "report for 2026-11-20 (run run-r)"), (line.RequestId, line.Level, line.Message));
        var done = (await _celld.Next(PluginMessage.KindOneofCase.JobDone)).JobDone;
        Assert.Equal(("run-r", 1u, JobDone.OutcomeOneofCase.Ok), (done.RunId, done.Attempt, done.OutcomeCase));

        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { JobRun = Run("doomed", "run-d", "{}") });
        done = (await _celld.Next(PluginMessage.KindOneofCase.JobDone)).JobDone;
        Assert.Equal(("permanent", true, "this job always fails for good"), (done.Error.Code, done.Error.Permanent, done.Error.Message));

        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { JobRun = Run("slow", "run-s", "{}") });
        await _celld.Next(PluginMessage.KindOneofCase.PluginLog); // step 1 of 30
        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { JobAbort = new JobAbort { RunId = "run-s", Reason = JobAbort.Types.Reason.Timeout } });
        done = (await _celld.Next(PluginMessage.KindOneofCase.JobDone)).JobDone;
        Assert.Equal(("run-s", "timeout", false), (done.RunId, done.Error.Code, done.Error.Permanent));

        await stop.CancelAsync();
        await serving.WaitAsync(TimeSpan.FromSeconds(10));
    }

    [Fact]
    public async Task StopFinishesRunsInFlightAndTakesNoMore()
    {
        using var stop = new CancellationTokenSource();
        var serving = new Runtime(Jobsdemo.JobsdemoPlugin.Define().Definition).Run($"unix:{_socket}", stop.Token);
        await _celld.Next(PluginMessage.KindOneofCase.Register);

        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { JobRun = Run("slow", "run-1", """{"seconds":2}""") });
        await Task.Delay(200);
        await stop.CancelAsync();
        await Task.Delay(500); // the plugin is draining
        await _celld.ToPlugin.Writer.WriteAsync(new CelldMessage { JobRun = Run("heartbeat", "run-2", "{}") });

        var done = new Dictionary<string, JobDone>();
        for (var i = 0; i < 2; i++)
        {
            var d = (await _celld.Next(PluginMessage.KindOneofCase.JobDone)).JobDone;
            done[d.RunId] = d;
        }
        Assert.Equal("shutdown", done["run-2"].Error?.Code);
        Assert.Equal(JobDone.OutcomeOneofCase.Ok, done["run-1"].OutcomeCase);
        await serving.WaitAsync(TimeSpan.FromSeconds(10));
    }

    [Fact]
    public void CanonicalKeysAreSnakeSortedStrings()
    {
        Assert.Equal("""{"line":"2","open":"true","order_id":"1048"}""",
            Ctx.CanonicalKey(JsonNode.Parse("""{"orderId":"1048","line":2,"open":true}""")));
        Assert.Equal("""{"id":"7"}""", Ctx.CanonicalKey(JsonNode.Parse("""{"id":"7","nested":{"a":"1"},"tags":["x"],"gone":null}""")));
        Assert.Equal("", Ctx.CanonicalKey(JsonNode.Parse("{}")));
    }
}
