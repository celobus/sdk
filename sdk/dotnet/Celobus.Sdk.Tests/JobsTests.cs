using System.Text.Json.Nodes;
using System.Threading.Channels;
using Celobus.V1;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using J = Jobsdemo;

namespace Celobus.Sdk.Tests;

[Message(Kind = MessageKind.Job)] public sealed record RemindUnpaid(int OlderThanDays = 3);
[Message(Kind = MessageKind.Job)] public sealed record Reindex;
[Message(Kind = MessageKind.Job)] public sealed record PostReport(DateOnly Day);
[Message(Kind = MessageKind.Job)] public sealed record Big(string Text);
[Message] public sealed record Unused(string X);

/// <summary>Runs as the test asks through its payload's days: 3 answers ok and logs, others fail as named.</summary>
public sealed class RemindUnpaidJob : Job<RemindUnpaid>
{
    public override void Configure(JobOptions j) => j.Describe("Remind unpaid orders").Cron("0 9 * * 1-5").AsSystem()
        .Retry(Retry.Exponential(3, TimeSpan.FromMinutes(1))).Timeout(TimeSpan.FromMinutes(2));

    public override async Task Run(Ctx ctx, RemindUnpaid p)
    {
        switch (p.OlderThanDays)
        {
            case 3:
                Assert.True(ctx.Job.IsRun);
                Assert.Equal((900ul, "remind_unpaid", "run-0", 2, 4), (ctx.Request.Id, ctx.Job.Job, ctx.Job.RunId, ctx.Job.Attempt, ctx.Job.MaxAttempts));
                Assert.Equal(1_800_000_000_000, ctx.Job.ScheduledAt.ToUnixTimeMilliseconds());
                Assert.False(ctx.Job.IsLastAttempt);
                Assert.Equal("u-7", ctx.Caller.User().Value);
                ctx.Log.LogInformation("reminding {Days} days", p.OlderThanDays);
                await Task.Yield();
                ctx.Log.LogDebug("below the level: not kept");
                return;
            case 1: throw PluginError.Unavailable("the mail server is away");
            case 2: throw PluginError.Permanent("customer has no e-mail");
            case 4: throw PluginError.Conflict("taken").AsPermanent();
            case 5: throw PluginError.NotFound("no order");
            case 6: throw new InvalidOperationException("a bug");
            case 7: throw PluginError.Unavailable(new string('é', 2000));
            case 8:
                await Task.Delay(Timeout.Infinite, ctx.Cancelled);
                return;
            case 9:
                // ignores its signal for a second: the attempt is answered anyway
                await Task.Delay(1000);
                return;
        }
    }
}

public sealed class ReindexJob : Job<Reindex>
{
    public override void Configure(JobOptions j) => j.Name("reindex_all");

    public override Task Run(Ctx ctx, Reindex p) => Task.CompletedTask;
}

public sealed class PostReportJob : Job<PostReport>
{
    public override Task Run(Ctx ctx, PostReport p) => Task.CompletedTask;
}

public sealed class BigJob : Job<Big>
{
    public override Task Run(Ctx ctx, Big p) => Task.CompletedTask;
}

/// <summary>A second job of the payload <see cref="Reindex"/> (refused: E_JOB_PAYLOAD_DUP).</summary>
public sealed class ReindexAgainJob : Job<Reindex>
{
    public override void Configure(JobOptions j) => j.Name("reindex_again");

    public override Task Run(Ctx ctx, Reindex p) => Task.CompletedTask;
}

/// <summary>
/// Background jobs: the descriptor's <c>jobs</c> and their rules, enqueue and cancel on the wire, a run from its
/// <c>JobRun</c> to its <c>JobDone</c>, the run log, and the conformance plugin <c>jobsdemo</c>.
/// </summary>
public class JobsTests
{
    private static Plugin Orders() => Plugin.Create("orders").Version("1.0.0")
        .Message<Unused>()
        .Job<RemindUnpaidJob>()
        .Job<ReindexJob>()
        .Job<BigJob>();

    private static List<string> Codes(Plugin p) => [.. p.Check().Select(e => e.Code)];

    // --- the descriptor

    [Fact]
    public void JobsdemoMatchesItsFixture()
    {
        var plugin = J.JobsdemoPlugin.Define();
        Assert.Empty(plugin.Check());
        var actual = JsonNode.Parse(plugin.DescriptorJson());
        var expected = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "conformance", "jobs.json")));
        Assert.True(JsonNode.DeepEquals(expected, actual), actual!.ToJsonString());
    }

    [Fact]
    public void AJobDeclaresItsEntryAndItsPayload()
    {
        var d = JsonNode.Parse(Orders().DescriptorJson())!;
        Assert.Empty(Orders().Check());
        var jobs = d["jobs"]!.AsArray();
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""
            {"name":"remind_unpaid","payload":"RemindUnpaid","description":"Remind unpaid orders","cron":"0 9 * * 1-5",
             "as_system":true,"retry":{"retries":3,"delay_ms":60000,"backoff":"exponential"},"timeout_ms":120000}
            """), jobs[0]), jobs[0]!.ToJsonString());
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""{"name":"reindex_all","payload":"Reindex"}"""), jobs[1]));
        // the payloads come after the extra messages, kind job
        var messages = d["messages"]!.AsArray().Select(m => (m!["name"]!.GetValue<string>(), m["kind"]!.GetValue<string>())).ToList();
        Assert.Equal([("Unused", "message"), ("RemindUnpaid", "job"), ("Reindex", "job"), ("Big", "job")], messages);

        Assert.Null(JsonNode.Parse(TestPlugin.Define().Version("1.0.0").DescriptorJson())!["jobs"]);
    }

    [Fact]
    public void TheKeyIsThePayloadInSnakeCase()
    {
        Assert.Equal("remind_unpaid", Jobs.Key("RemindUnpaid"));
        Assert.Equal("h_t_t_p_ping", Jobs.Key("HTTPPing"));
        Assert.Equal("slow", Jobs.Key("Slow"));
    }

    [Fact]
    public void TheJobsRulesAreCheckedAtBuild()
    {
        Assert.Equal(["E_JOB_CRON"], Codes(Plugin.Create("orders").Version("1.0.0").Job<CronOf61>()));
        Assert.Equal(["E_JOB_PAYLOAD_DUP"], Codes(Plugin.Create("orders").Version("1.0.0").Job<ReindexJob>().Job<ReindexAgainJob>()));
        Assert.Equal(["E_JOB_DUP", "E_JOB_PAYLOAD_DUP"], Codes(Plugin.Create("orders").Version("1.0.0").Job<ReindexJob>().Job<ReindexJob>()));
        Assert.Equal(["E_JOB_NAME"], Codes(Plugin.Create("orders").Version("1.0.0").Job<BadName>()));
        Assert.Equal(["E_JOB_SCHEDULE_PAYLOAD"], Codes(Plugin.Create("orders").Version("1.0.0").Job<ScheduledReport>()));
        Assert.Empty(Codes(Plugin.Create("orders").Version("1.0.0").Job<PostReportJob>())); // enqueued only: fine
        Assert.Equal(["E_JOB_RETRY", "E_JOB_RETRY", "E_JOB_TIMEOUT", "E_JOB_CONCURRENCY"],
            Codes(Plugin.Create("orders").Version("1.0.0").Job<OutOfBounds>()));
        Assert.Equal(["E_JOB_SLOTS"], Codes(Orders().JobSlots(65)));
        Assert.Equal(["E_JOB_SLOTS"], Codes(Orders().JobSlots(0)));
        Assert.Empty(Codes(Orders().JobSlots(64)));

        var cron = Plugin.Create("orders").Version("1.0.0").Job<CronOf61>().Check().Single();
        Assert.Equal("jobs[reindex].cron", cron.Path);
        Assert.Contains("61 is out of 0..=59 for the minute", cron.Message);
    }

    private sealed class CronOf61 : Job<Reindex>
    {
        public override void Configure(JobOptions j) => j.Cron("61 * * * *");
        public override Task Run(Ctx ctx, Reindex p) => Task.CompletedTask;
    }

    private sealed class BadName : Job<Reindex>
    {
        public override void Configure(JobOptions j) => j.Name("Reindex-All");
        public override Task Run(Ctx ctx, Reindex p) => Task.CompletedTask;
    }

    private sealed class ScheduledReport : Job<PostReport>
    {
        public override void Configure(JobOptions j) => j.Cron("0 6 * * *");
        public override Task Run(Ctx ctx, PostReport p) => Task.CompletedTask;
    }

    private sealed class OutOfBounds : Job<Reindex>
    {
        public override void Configure(JobOptions j) => j.Retry(Retry.Fixed(21, TimeSpan.FromHours(2)))
            .Timeout(TimeSpan.FromMilliseconds(500)).Concurrency(65);
        public override Task Run(Ctx ctx, Reindex p) => Task.CompletedTask;
    }

    [Theory]
    [InlineData("* * * * *")]
    [InlineData("0 9 * * 1-5")]
    [InlineData("*/15 0-6,22-23 1,15 jan-mar MON-fri")]
    [InlineData("5/10 * * * 7")]
    [InlineData("0 0 29 2 *")]
    [InlineData("0  9\t* * *")]
    public void ValidSchedules(string cron) => Assert.Null(Cron.Check(cron));

    [Theory]
    [InlineData("* * * *", "a cron has 5 fields")]
    [InlineData("@daily", "a cron has 5 fields")]
    [InlineData("* 24 * * *", "24 is out of 0..=23 for the hour")]
    [InlineData("* * 0 * *", "0 is out of 1..=31 for the day of the month")]
    [InlineData("*/0 * * * *", "a step of 0")]
    [InlineData("10-5 * * * *", "runs backwards")]
    [InlineData("* * * foo *", "\"foo\" is not a month")]
    [InlineData("0 0 30 2 *", "no chosen month has the chosen day of the month")]
    [InlineData("0 0 L * *", "\"L\" is not a day of the month")]
    [InlineData("* * * * 8", "8 is out of 0..=7")]
    public void InvalidSchedules(string cron, string why) => Assert.Contains(why, Cron.Check(cron));

    [Fact]
    public void PermanenceIsTheFlagOrAnInputClassError()
    {
        Assert.True(PluginError.Permanent("no mail").IsPermanent);
        Assert.Equal((422, "permanent"), (PluginError.Permanent("no mail").Status, PluginError.Permanent("no mail").Code));
        Assert.True(PluginError.Unavailable("x").AsPermanent().IsPermanent);
        foreach (var e in new[] { PluginError.Invalid("x"), PluginError.NotFound("x"), new PluginError(403, "permission_denied", "x"),
                     new PluginError(400, "payload_invalid", "x") })
            Assert.True(e.IsPermanent, e.Code);
        foreach (var e in new[] { PluginError.Unavailable("x"), PluginError.Conflict("x"), PluginError.Forbidden(), new PluginError(500, "internal", "x") })
            Assert.False(e.IsPermanent, e.Code);
    }

    // --- enqueue and cancel

    private static Ctx Context(FakeSyscalls syscalls, Plugin plugin) => new(
        syscalls, new PluginInfo("orders", "1.0.0") { JobKeys = plugin.Definition.Jobs.ToDictionary(j => j.Payload, j => j.Key) },
        new RequestInfo(77, "PlaceOrder", DateTimeOffset.UtcNow.AddSeconds(30), null),
        new Caller("orders", new Principal { UserId = "u-1", Kind = Principal.Types.Kind.Person }),
        NullLogger.Instance, CancellationToken.None);

    private static FakeSyscalls Platform() => new(call => call.CallCase switch
    {
        Syscall.CallOneofCase.TxBegin => new SyscallResult { TxBegun = new TxBegun { Tx = 5 } },
        Syscall.CallOneofCase.JobEnqueue when call.JobEnqueue.Job == "big" =>
            new SyscallResult { Error = "queue_full: 10,000 pending runs" },
        Syscall.CallOneofCase.JobEnqueue when call.JobEnqueue.Singleton != "" =>
            new SyscallResult { JobEnqueued = new JobEnqueued { RunId = "run-1", Existing = true, State = V1.RunState.Done } },
        Syscall.CallOneofCase.JobEnqueue =>
            new SyscallResult { JobEnqueued = new JobEnqueued { RunId = $"run-{call.JobEnqueue.Job}", State = (V1.RunState)9 } },
        Syscall.CallOneofCase.JobCancel when call.JobCancel.RunId == "gone" => new SyscallResult { Error = "not_found: no run gone" },
        Syscall.CallOneofCase.JobCancel when call.JobCancel.RunId == "aborted" =>
            new SyscallResult { Error = "cancelled: the run making the call was stopped" },
        Syscall.CallOneofCase.JobCancel => new SyscallResult { JobCancelled = new JobCancelled { Cancelled = true } },
        _ => new SyscallResult { Ok = new Empty() },
    });

    [Fact]
    public async Task EnqueueAndCancelOnTheWire()
    {
        var syscalls = Platform();
        var ctx = Context(syscalls, Orders());

        var run = await ctx.Jobs.Enqueue(new RemindUnpaid(7)).Delay(TimeSpan.FromHours(1));
        Assert.Equal(new Enqueued("run-remind_unpaid", false, Sdk.RunState.Unknown), run);
        var again = await ctx.Jobs.Enqueue(new RemindUnpaid(3)).At(DateTimeOffset.FromUnixTimeMilliseconds(1_800_000_000_000))
            .Singleton("report:2026-11-20");
        Assert.Equal(new Enqueued("run-1", true, Sdk.RunState.Done), again);
        await ctx.Transaction(async tx => { await tx.Jobs.Enqueue(new Reindex()); });
        Assert.True(await ctx.Jobs.Cancel(run));

        var e = await Assert.ThrowsAsync<PluginError>(() => ctx.Jobs.Cancel("gone"));
        Assert.Equal((404, "not_found"), (e.Status, e.Code));
        e = await Assert.ThrowsAsync<PluginError>(() => ctx.Jobs.Cancel("aborted"));
        Assert.Equal((503, "cancelled"), (e.Status, e.Code));
        e = await Assert.ThrowsAsync<PluginError>(async () => await ctx.Jobs.Enqueue(new Big("x")));
        Assert.Equal((429, "queue_full"), (e.Status, e.Code));

        var enqueues = syscalls.Calls.Where(c => c.CallCase == Syscall.CallOneofCase.JobEnqueue).Select(c => c.JobEnqueue).ToList();
        Assert.Equal(("remind_unpaid", """{"olderThanDays":7}""", 3_600_000ul, 77ul, 0ul),
            (enqueues[0].Job, enqueues[0].Payload, enqueues[0].DelayMs, enqueues[0].RequestId, enqueues[0].Tx));
        Assert.Equal((JobEnqueue.WhenOneofCase.AtUnixMs, 1_800_000_000_000, "report:2026-11-20"),
            (enqueues[1].WhenCase, enqueues[1].AtUnixMs, enqueues[1].Singleton));
        Assert.Equal(("reindex_all", 5ul, JobEnqueue.WhenOneofCase.None), (enqueues[2].Job, enqueues[2].Tx, enqueues[2].WhenCase));
        Assert.Contains(syscalls.Calls, c => c.JobCancel is { RunId: "run-remind_unpaid", RequestId: 77 });
    }

    [Fact]
    public async Task EnqueueChecksTheJobAndThePayloadFirst()
    {
        var syscalls = Platform();
        var ctx = Context(syscalls, Orders());
        var e = await Assert.ThrowsAsync<PluginError>(async () => await ctx.Jobs.Enqueue(new PostReport(new DateOnly(2026, 11, 20))));
        Assert.Equal((400, "invalid"), (e.Status, e.Code));
        e = await Assert.ThrowsAsync<PluginError>(async () => await ctx.Jobs.Enqueue(new Big(new string('x', 70_000))));
        Assert.Equal((413, "payload_too_large"), (e.Status, e.Code));
        Assert.Empty(syscalls.Calls);
    }

    // --- runs

    /// <summary>A job pool of a plugin, its messages to the platform in a channel.</summary>
    private sealed class Pool
    {
        public Channel<PluginMessage> Sent { get; } = Channel.CreateUnbounded<PluginMessage>();
        public JobPool Jobs { get; }

        public Pool(Plugin plugin) =>
            Jobs = new Runtime(plugin.Definition).Pool(new FakeSyscalls(_ => new SyscallResult { Ok = new Empty() }), Sent.Writer.TryWrite);

        public void Start(string job, string runId, string payload, bool draining = false) => Jobs.Start(new JobRun
        {
            RunId = runId, Job = job, Attempt = 2, MaxAttempts = 4, Payload = payload,
            Principal = new Principal { UserId = "u-7", Kind = Principal.Types.Kind.Person },
            ScheduledAtUnixMs = 1_800_000_000_000, TimeoutMs = 120_000, RequestId = 900,
        }, draining);

        /// <summary>The next JobDone, and the run's log lines sent before it.</summary>
        public async Task<(JobDone Done, List<PluginLog> Lines)> Done()
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(10));
            var lines = new List<PluginLog>();
            while (true)
            {
                var m = await Sent.Reader.ReadAsync(timeout.Token);
                if (m.PluginLog is { } l) lines.Add(l);
                if (m.JobDone is { } d) return (d, lines);
            }
        }
    }

    private static (string Code, bool Permanent) Outcome(JobDone d) => d.OutcomeCase == JobDone.OutcomeOneofCase.Ok
        ? ("ok", false)
        : (d.Error.Code, d.Error.Permanent);

    [Fact]
    public async Task ARunAnswersItsOutcome()
    {
        var pool = new Pool(Orders());
        pool.Start("remind_unpaid", "run-0", """{"olderThanDays":3}""");
        var (d, lines) = await pool.Done();
        Assert.Equal(("run-0", 2u), (d.RunId, d.Attempt));
        Assert.Equal(("ok", false), Outcome(d));
        var line = Assert.Single(lines);
        Assert.Equal((900ul, PluginLog.Types.Level.Info, "reminding 3 days"), (line.RequestId, line.Level, line.Message));
        Assert.True(line.AtUnixMs > 0);

        foreach (var (days, code, permanent) in new[]
                 {
                     (1, "unavailable", false), (2, "permanent", true), (4, "conflict", true), (5, "not_found", true),
                     (6, "internal", false),
                 })
        {
            pool.Start("remind_unpaid", $"run-{days}", $$"""{"olderThanDays":{{days}}}""");
            (d, _) = await pool.Done();
            Assert.Equal((code, permanent), Outcome(d));
            if (days == 6) Assert.Equal("a bug", d.Error.Message);
        }

        pool.Start("remind_unpaid", "run-7", """{"olderThanDays":7}""");
        (d, _) = await pool.Done();
        Assert.Equal(2048, System.Text.Encoding.UTF8.GetByteCount(d.Error.Message)); // cut at 2 KiB

        foreach (var payload in new[] { "not json", """{"olderThanDays":"many"}""", "" })
        {
            pool.Start("remind_unpaid", "run-p", payload);
            (d, _) = await pool.Done();
            Assert.Equal(("payload_invalid", true), Outcome(d));
            Assert.StartsWith("the payload is not a RemindUnpaid", d.Error.Message);
        }

        pool.Start("nightly", "run-n", "{}");
        (d, _) = await pool.Done();
        Assert.Equal(("unavailable", false), Outcome(d));
        Assert.Equal("this version has no job nightly", d.Error.Message);

        pool.Start("reindex_all", "run-s", "{}", draining: true);
        (d, _) = await pool.Done();
        Assert.Equal(("shutdown", false), Outcome(d));
        Assert.Equal(0, pool.Jobs.Running);
    }

    [Theory]
    [InlineData(1, "timeout")]
    [InlineData(2, "cancelled")]
    [InlineData(3, "shutdown")]
    [InlineData(4, "lease_lost")]
    [InlineData(5, "auth_changed")]
    [InlineData(6, "job_removed")]
    [InlineData(42, "cancelled")]
    public async Task AnAbortAnswersAtOnceWithItsReason(int reason, string code)
    {
        var pool = new Pool(Orders());
        // 9 ignores its signal for 1 s: the answer does not wait for it
        pool.Start("remind_unpaid", "run-a", """{"olderThanDays":9}""");
        for (var i = 0; pool.Jobs.Running == 0 && i < 500; i++) await Task.Delay(10); // the run is under way
        var at = DateTimeOffset.UtcNow;
        pool.Jobs.Abort(new JobAbort { RunId = "run-a", Reason = (JobAbort.Types.Reason)reason });
        var (d, _) = await pool.Done();
        Assert.True(DateTimeOffset.UtcNow - at < TimeSpan.FromMilliseconds(700));
        Assert.Equal((code, false), Outcome(d));
        Assert.Equal($"stopped: {code}", d.Error.Message);
        Assert.Equal(0, pool.Jobs.Running);
        // the handler's own end later sends nothing more
        await Task.Delay(1100);
        Assert.False(pool.Sent.Reader.TryRead(out _));
    }

    [Fact]
    public async Task AnAbortFiresTheRunsSignal()
    {
        var pool = new Pool(Orders());
        pool.Start("remind_unpaid", "run-c", """{"olderThanDays":8}""");
        await Task.Delay(20);
        Assert.Equal(1, pool.Jobs.Running);
        pool.Jobs.Abort(new JobAbort { RunId = "run-c", Reason = JobAbort.Types.Reason.Timeout });
        var (d, _) = await pool.Done();
        Assert.Equal(("timeout", false), Outcome(d));
        await Task.Delay(100);
        Assert.False(pool.Sent.Reader.TryRead(out _));
    }

    [Fact]
    public async Task ARunPastTheSlotsIsRefused()
    {
        var pool = new Pool(Orders().JobSlots(1));
        pool.Start("remind_unpaid", "run-1", """{"olderThanDays":8}""");
        pool.Start("reindex_all", "run-2", "{}");
        var (d, _) = await pool.Done();
        Assert.Equal(("run-2", "unavailable", false), (d.RunId, d.Error.Code, d.Error.Permanent));
        Assert.Equal("all 1 job slots are taken", d.Error.Message);
        // an answered run frees its slot
        pool.Jobs.Abort(new JobAbort { RunId = "run-1", Reason = JobAbort.Types.Reason.Cancelled });
        await pool.Done();
        pool.Start("reindex_all", "run-3", "{}");
        (d, _) = await pool.Done();
        Assert.Equal(("run-3", "ok"), (d.RunId, Outcome(d).Code));
    }

    [Fact]
    public void TheJobPoolIsAnnouncedOnlyWithJobs()
    {
        Assert.Equal(16, new Runtime(Orders().Definition).JobSlots);
        Assert.Equal(4, new Runtime(Orders().JobSlots(4).Definition).JobSlots);
        Assert.Equal(0, new Runtime(TestPlugin.Define().JobSlots(4).Definition).JobSlots);
    }

    [Fact]
    public void AHandlerOutsideARunSeesNoRun()
    {
        var ctx = Context(Platform(), Orders());
        Assert.False(ctx.Job.IsRun);
        Assert.Equal(("", 0), (ctx.Job.RunId, ctx.Job.Attempt));
    }

    // --- the run log

    [Fact]
    public void LinesAreTheMessageThenTheOtherFields()
    {
        var state = new List<KeyValuePair<string, object?>>
        {
            new("Order", "SO-1"), new("n", 3), new("{OriginalFormat}", "reminded {Order}"),
        };
        Assert.Equal("reminded SO-1 n=3", RunLog.Line(state, null, "reminded SO-1"));
        Assert.Equal("no mail error=\"InvalidOperationException: down\"",
            RunLog.Line<object?>(null, new InvalidOperationException("down"), "no mail"));
        var cut = RunLog.Cut(new string('é', 2000), RunLog.MaxLine);
        Assert.Equal((1024, true), (cut.Length, cut.All(c => c == 'é')));
        Assert.Equal("ab", RunLog.Cut("ab😀", 5));
        // never in the middle of a character
        Assert.Equal(2047, System.Text.Encoding.UTF8.GetByteCount(RunLog.Cut("a" + new string('é', 2000), RunLog.MaxLine)));
    }

    [Fact]
    public void TheLimitIsPerSecond()
    {
        var l = new Limiter();
        for (var i = 0; i < Limiter.LinesPerSecond; i++) Assert.True(l.Admit(10_000, 10));
        Assert.False(l.Admit(10_999, 10));
        Assert.True(l.Admit(11_000, 10));
        l = new Limiter();
        Assert.True(l.Admit(0, Limiter.BytesPerSecond - 1));
        Assert.False(l.Admit(0, 2));
        Assert.True(l.Admit(0, 1));
    }

    [Fact]
    public async Task OnlyARunsLinesAreSent()
    {
        var sent = new List<PluginMessage>();
        var log = new JsonLogger("t", []);
        log.LogInformation("a request's line");
        await Task.Run(async () =>
        {
            RunLog.Current = new RunLogTarget(5, m => { lock (sent) sent.Add(m); return true; });
            log.LogWarning("in the run");
            await Task.Yield();
            log.LogError("still in it");
        });
        log.LogInformation("after it");
        Assert.Equal([(5ul, PluginLog.Types.Level.Warn, "in the run"), (5ul, PluginLog.Types.Level.Error, "still in it")],
            sent.Select(m => (m.PluginLog.RequestId, m.PluginLog.Level, m.PluginLog.Message)));
    }
}
