// Background jobs: a payload message and a handler, declared with Job<T> and Plugin.Job<TJob>(); started with
// `ctx.Jobs.Enqueue(payload)` (in a declared transaction `tx.Jobs`, so the run exists only if it commits) or by a
// schedule; cancelled with `ctx.Jobs.Cancel(..)`. In the handler, `ctx.Job` is the run and `ctx.Cancelled` fires when
// the run is stopped.
//
//   [Message(Kind = MessageKind.Job)]
//   public sealed record RemindUnpaid(int OlderThanDays = 3);
//
//   public sealed class RemindUnpaidJob : Job<RemindUnpaid>
//   {
//       public override void Configure(JobOptions j) => j.Cron("0 9 * * 1-5").AsSystem()
//           .Retry(Retry.Exponential(3, TimeSpan.FromMinutes(1))).Timeout(TimeSpan.FromMinutes(2));
//
//       public override async Task Run(Ctx ctx, RemindUnpaid p) { /* … */ }
//   }
//
//   Plugin.Create("orders").Job<RemindUnpaidJob>();
using System.Collections.Concurrent;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using Microsoft.Extensions.Logging;
using V1 = Celobus.V1;

namespace Celobus.Sdk;

/// <summary>
/// A background job: its payload message <typeparamref name="T"/> (<c>[Message(Kind = MessageKind.Job)]</c>) and its
/// handler. Its key is the payload's name in snake_case (<c>RemindUnpaid</c> → <c>remind_unpaid</c>;
/// <c>j.Name(..)</c> to choose another); its full name in logs and on screens is <c>&lt;plugin&gt;.&lt;key&gt;</c>.
/// Enqueueing picks the job by the payload's type, so no two jobs share a payload message.
/// <para>
/// Defaults: started only when enqueued (<c>j.Cron(..)</c> adds a schedule), as the caller who enqueued it
/// (<c>j.AsSystem()</c>: as the plugin's system principal), 3 retries 30 s apart backing off exponentially
/// (<c>j.Retry(..)</c>), a timeout of 5 minutes (1 s to 1 h, <c>j.Timeout(..)</c>), one run at a time (1 to 64,
/// <c>j.Concurrency(..)</c>).
/// </para>
/// One instance runs every attempt, concurrently: keep no per-run state in fields.
/// </summary>
public abstract class Job<T> : JobBase where T : class
{
    /// <summary>The job's options (name, description, schedule, principal, retries, timeout, concurrency).</summary>
    public virtual void Configure(JobOptions j) { }

    /// <summary>
    /// Runs one attempt. Returning ends the run as done; an exception fails the attempt, retried unless the error is
    /// permanent (<see cref="PluginError.IsPermanent"/>). Pass <c>ctx.Cancelled</c> to your own waits: it fires when
    /// the run is stopped (its timeout, a cancel, the plugin stopping).
    /// </summary>
    public abstract Task Run(Ctx ctx, T payload);

    internal override Type PayloadType => typeof(T);
    internal override void ConfigureCore(JobOptions j) => Configure(j);
    internal override Task RunCore(Ctx ctx, object payload) => Run(ctx, (T)payload);
}

/// <summary>The non-generic base of <see cref="Job{T}"/>; derive from that.</summary>
public abstract class JobBase
{
    private protected JobBase() { }

    internal abstract Type PayloadType { get; }
    internal abstract void ConfigureCore(JobOptions j);
    internal abstract Task RunCore(Ctx ctx, object payload);
}

/// <summary>The options of a job, set in its <c>Configure</c>.</summary>
public sealed class JobOptions
{
    internal string? NameValue { get; private set; }
    internal string Description { get; private set; } = "";
    internal string CronValue { get; private set; } = "";
    internal bool IsSystem { get; private set; }
    internal Retry? RetryValue { get; private set; }
    internal TimeSpan? TimeoutValue { get; private set; }
    internal int? ConcurrencyValue { get; private set; }

    /// <summary>The job's key, instead of the payload's name in snake_case: snake_case, at most 32 characters.</summary>
    public JobOptions Name(string name)
    {
        NameValue = name;
        return this;
    }

    /// <summary>What the job does, for the workspace's list of jobs: plain text or a key of the UI's text catalog.</summary>
    public JobOptions Describe(string description)
    {
        Description = description;
        return this;
    }

    /// <summary>
    /// A schedule: five fields in UTC, <c>minute hour day-of-month month day-of-week</c> (<c>0 9 * * 1-5</c>: weekdays
    /// at 09:00 UTC). Each occurrence starts a run with the payload's defaults; an occurrence that comes due while the
    /// previous scheduled run is still pending or running is not started.
    /// </summary>
    public JobOptions Cron(string cron)
    {
        CronValue = cron;
        return this;
    }

    /// <summary>Its runs run as the plugin's system principal, whoever enqueued them.</summary>
    public JobOptions AsSystem()
    {
        IsSystem = true;
        return this;
    }

    /// <summary>How failed attempts are retried (<see cref="Sdk.Retry"/>).</summary>
    public JobOptions Retry(Retry retry)
    {
        RetryValue = retry;
        return this;
    }

    /// <summary>How long an attempt may run (1 s to 1 h); past it the handler is stopped and the attempt fails as <c>timeout</c>.</summary>
    public JobOptions Timeout(TimeSpan timeout)
    {
        TimeoutValue = timeout;
        return this;
    }

    /// <summary>How many runs of the job run at once in the workspace (1 to 64).</summary>
    public JobOptions Concurrency(int n)
    {
        ConcurrencyValue = n;
        return this;
    }
}

/// <summary>How a retry's delay grows.</summary>
public enum Backoff
{
    /// <summary>Every retry waits the delay.</summary>
    Fixed,

    /// <summary>The delay doubles with each attempt (at most an hour).</summary>
    Exponential,
}

/// <summary>
/// How a job's failed attempts are retried: <c>Retry.Exponential(3, TimeSpan.FromMinutes(1))</c> waits 1, 2, 4 minutes
/// (each ±20 %, at most an hour); <c>Retry.Fixed(5, TimeSpan.FromSeconds(30))</c> waits 30 s each time;
/// <c>Retry.None</c> never retries. Unset: 3 retries, 30 s apart, exponential.
/// </summary>
public sealed record Retry
{
    private Retry(int retries, TimeSpan delay, Backoff backoff)
    {
        Retries = retries;
        Delay = delay;
        Backoff = backoff;
    }

    /// <summary>Attempts after the first (at most 20).</summary>
    public int Retries { get; }

    /// <summary>The delay before the first retry (at most an hour).</summary>
    public TimeSpan Delay { get; }

    public Backoff Backoff { get; }

    /// <summary><paramref name="retries"/> attempts after the first (at most 20), the delay doubling from <paramref name="delay"/> (at most an hour).</summary>
    public static Retry Exponential(int retries, TimeSpan delay) => new(retries, delay, Backoff.Exponential);

    /// <summary><paramref name="retries"/> attempts after the first (at most 20), <paramref name="delay"/> apart (at most an hour).</summary>
    public static Retry Fixed(int retries, TimeSpan delay) => new(retries, delay, Backoff.Fixed);

    /// <summary>One attempt only.</summary>
    public static Retry None { get; } = new(0, TimeSpan.Zero, Backoff.Fixed);
}

/// <summary>The run a job's handler is in: <c>ctx.Job</c>. Outside a run: an empty run id and attempt 0.</summary>
public sealed class JobInfo
{
    internal static JobInfo None { get; } = new("", "", 0, 0, DateTimeOffset.UnixEpoch);

    internal JobInfo(string job, string runId, int attempt, int maxAttempts, DateTimeOffset scheduledAt)
    {
        Job = job;
        RunId = runId;
        Attempt = attempt;
        MaxAttempts = maxAttempts;
        ScheduledAt = scheduledAt;
    }

    /// <summary>The job's key (<c>remind_unpaid</c>).</summary>
    public string Job { get; }

    /// <summary>The run's id, the same in every attempt: the idempotency key of the handler's outside effects.</summary>
    public string RunId { get; }

    /// <summary>1 on the first attempt.</summary>
    public int Attempt { get; }

    /// <summary>The attempts the run may take; <c>Attempt == MaxAttempts</c> on the last one.</summary>
    public int MaxAttempts { get; }

    /// <summary>The occurrence (or due time) the run stands for, the same in every attempt.</summary>
    public DateTimeOffset ScheduledAt { get; }

    /// <summary>Whether this is a job's run.</summary>
    public bool IsRun => RunId.Length > 0;

    /// <summary>Whether a failure of this attempt ends the run as failed.</summary>
    public bool IsLastAttempt => Attempt >= MaxAttempts;
}

/// <summary>The state of a run.</summary>
public enum RunState
{
    Pending,
    Running,
    Done,

    /// <summary>Failed for good: its retries used up, or it failed as permanent.</summary>
    Failed,
    Cancelled,

    /// <summary>A state this SDK does not know.</summary>
    Unknown,
}

/// <summary>
/// A run started by an enqueue: its id, and <c>Existing</c> when its singleton key was taken by an earlier run (whose
/// id and state these are; its payload is kept).
/// </summary>
public sealed record Enqueued(string RunId, bool Existing, RunState State);

/// <summary>The plugin's background jobs: <c>ctx.Jobs</c>, <c>tx.Jobs</c> in a declared transaction.</summary>
public sealed class Jobs
{
    /// <summary>The most a payload may weigh, as JSON (64 KiB).</summary>
    public const int MaxPayload = 64 << 10;

    private readonly Ctx _ctx;

    internal Jobs(Ctx ctx) { _ctx = ctx; }

    /// <summary>
    /// Starts a run of the job whose payload this is, as the caller of the request or run in progress (a job declared
    /// <c>AsSystem()</c> runs as the system); <c>.Delay(..)</c>, <c>.At(..)</c> and <c>.Singleton(..)</c> before
    /// <c>await</c>. In a declared transaction (<c>tx.Jobs</c>) the run is written in it and exists only if it commits;
    /// a singleton enqueue there may wait for a concurrent transaction enqueuing the same key. Errors: 400
    /// <c>invalid</c> (no job takes this payload, <c>At</c> more than a year ahead), 413 <c>payload_too_large</c> (over
    /// 64 KiB: put larger input in an entity and pass its id), 429 <c>queue_full</c> (10,000 pending runs of the plugin).
    /// </summary>
    public Enqueue Enqueue<T>(T payload) where T : class => new(_ctx, payload ?? throw new ArgumentNullException(nameof(payload)));

    /// <summary>
    /// Cancels one of the plugin's runs: a pending run never starts, a running one is stopped. False when it had already
    /// ended; 404 <c>not_found</c> when the plugin has no such run.
    /// </summary>
    public async Task<bool> Cancel(string runId)
    {
        var r = await _ctx.Syscall(new V1.Syscall { JobCancel = new V1.JobCancel { RunId = runId, RequestId = _ctx.Request.Id } });
        return (r.JobCancelled ?? throw PluginError.Unavailable("platform: unexpected syscall result")).Cancelled;
    }

    /// <inheritdoc cref="Cancel(string)"/>
    public Task<bool> Cancel(Enqueued run) => Cancel(run.RunId);

    /// <summary><c>RemindUnpaid</c> → <c>remind_unpaid</c>.</summary>
    internal static string Key(string message)
    {
        var sb = new StringBuilder(message.Length + 4);
        for (var i = 0; i < message.Length; i++)
        {
            var c = message[i];
            if (char.IsAsciiLetterUpper(c))
            {
                if (i > 0) sb.Append('_');
                sb.Append(char.ToLowerInvariant(c));
            }
            else sb.Append(c);
        }
        return sb.ToString();
    }
}

/// <summary>An enqueue in the making; <c>await</c> it.</summary>
public sealed class Enqueue
{
    private readonly Ctx _ctx;
    private readonly object _payload;
    private ulong? _delayMs;
    private long? _atUnixMs;
    private string _singleton = "";

    internal Enqueue(Ctx ctx, object payload)
    {
        _ctx = ctx;
        _payload = payload;
    }

    /// <summary>Due after <paramref name="delay"/> (default: now).</summary>
    public Enqueue Delay(TimeSpan delay)
    {
        _delayMs = delay <= TimeSpan.Zero ? 0 : (ulong)delay.TotalMilliseconds;
        _atUnixMs = null;
        return this;
    }

    /// <summary>Due at <paramref name="at"/> (at most a year ahead; a time past is now).</summary>
    public Enqueue At(DateTimeOffset at)
    {
        _atUnixMs = at.ToUnixTimeMilliseconds();
        _delayMs = null;
        return this;
    }

    /// <summary>
    /// At most one run per key and job while that run is kept (7 days after it ends, 30 if it failed): an enqueue with
    /// a key taken answers the earlier run (<c>Existing</c>) and keeps its payload.
    /// </summary>
    public Enqueue Singleton(string key)
    {
        _singleton = key;
        return this;
    }

    public TaskAwaiter<Enqueued> GetAwaiter() => Send().GetAwaiter();

    /// <summary>The enqueue as sent; refused locally when no job takes the payload or it is over 64 KiB.</summary>
    internal V1.JobEnqueue Message()
    {
        var type = _payload.GetType();
        var job = _ctx.Plugin.JobFor(type);
        var payload = JsonSerializer.Serialize(_payload, type, CelobusJson.Options);
        var size = Encoding.UTF8.GetByteCount(payload);
        if (size > Jobs.MaxPayload)
            throw new PluginError(413, "payload_too_large",
                $"the payload of {job} is {size} bytes, over 64 KiB: put larger input in an entity and pass its id");
        var m = new V1.JobEnqueue { Job = job, Payload = payload, Singleton = _singleton, RequestId = _ctx.Request.Id, Tx = _ctx.Tx };
        if (_delayMs is { } d) m.DelayMs = d;
        if (_atUnixMs is { } a) m.AtUnixMs = a;
        return m;
    }

    private async Task<Enqueued> Send()
    {
        var r = await _ctx.Syscall(new V1.Syscall { JobEnqueue = Message() });
        var e = r.JobEnqueued ?? throw PluginError.Unavailable("platform: unexpected syscall result");
        return new Enqueued(e.RunId, e.Existing, e.State switch
        {
            V1.RunState.Pending => RunState.Pending,
            V1.RunState.Running => RunState.Running,
            V1.RunState.Done => RunState.Done,
            V1.RunState.Failed => RunState.Failed,
            V1.RunState.Cancelled => RunState.Cancelled,
            _ => RunState.Unknown,
        });
    }
}

/// <summary>A declared job: its handler, key and options.</summary>
internal sealed record JobRegistration(JobBase Job, string Key, JobOptions Options)
{
    public Type Payload => Job.PayloadType;
}

/// <summary>
/// The runs of one session: each <c>JobRun</c> starts its handler on the job pool (apart from requests) and is answered
/// by one <c>JobDone</c>; a <c>JobAbort</c> stops it and answers at once with the reason's code.
/// </summary>
internal sealed class JobPool(
    IReadOnlyDictionary<string, JobRegistration> jobs,
    PluginInfo plugin,
    int slots,
    ISyscalls syscalls,
    Func<V1.PluginMessage, bool> send,
    JsonLogger log)
{
    private sealed class Active(string runId, uint attempt)
    {
        public string RunId { get; } = runId;
        public uint Attempt { get; } = attempt;
        public CancellationTokenSource Stop { get; } = new();
        public int Answered;
    }

    private readonly ConcurrentDictionary<string, Active> _runs = new(StringComparer.Ordinal);
    private int _taken;

    /// <summary>The runs not answered yet.</summary>
    public int Running => _runs.Count;

    /// <summary>Starts an attempt, or answers it at once when the plugin is stopping, has no such job or no free slot.</summary>
    public void Start(V1.JobRun run, bool draining)
    {
        if (draining)
        {
            Refuse(run, "shutdown", "the plugin is stopping");
            return;
        }
        if (!jobs.TryGetValue(run.Job, out var job))
        {
            Refuse(run, "unavailable", $"this version has no job {run.Job}");
            return;
        }
        if (Interlocked.Increment(ref _taken) > slots)
        {
            Interlocked.Decrement(ref _taken);
            Refuse(run, "unavailable", $"all {slots} job slots are taken");
            return;
        }
        var active = new Active(run.RunId, run.Attempt);
        if (!_runs.TryAdd(run.RunId, active))
        {
            Interlocked.Decrement(ref _taken);
            Refuse(run, "unavailable", $"run {run.RunId} is already running");
            return;
        }
        var timeout = TimeSpan.FromMilliseconds(run.TimeoutMs);
        var info = new JobInfo(run.Job, run.RunId, (int)run.Attempt, (int)run.MaxAttempts,
            DateTimeOffset.FromUnixTimeMilliseconds(run.ScheduledAtUnixMs));
        var runLog = log.With(new("request_id", run.RequestId), new("job", run.Job), new("run_id", run.RunId),
            new("attempt", run.Attempt));
        var ctx = new Ctx(syscalls, plugin, new RequestInfo(run.RequestId, run.Job, DateTimeOffset.UtcNow + timeout, null),
            new Caller(plugin.Id, run.Principal), runLog, active.Stop.Token, job: info);
        var target = new RunLogTarget(run.RequestId, send);
        _ = Task.Run(() => Execute(active, job, run.Payload, ctx, target, runLog));
    }

    /// <summary>Stops a run's handler and answers its attempt with the reason's code, not waiting for the handler.</summary>
    public void Abort(V1.JobAbort abort)
    {
        if (!_runs.TryGetValue(abort.RunId, out var active)) return;
        var code = AbortCode(abort.Reason);
        Answer(active, new PluginError(503, code, $"stopped: {code}"));
        try { active.Stop.Cancel(); }
        catch (AggregateException) { /* a callback of the handler's own failed: it is stopped anyway */ }
    }

    /// <summary>The session ended: every handler is stopped, and nothing is answered.</summary>
    public void StopAll()
    {
        foreach (var active in _runs.Values)
        {
            Interlocked.Exchange(ref active.Answered, 1);
            try { active.Stop.Cancel(); }
            catch (AggregateException) { }
        }
        _runs.Clear();
    }

    /// <summary>Resolves when every run is answered, or when <paramref name="ct"/> fires.</summary>
    public async Task WhenIdle(CancellationToken ct)
    {
        try
        {
            while (!_runs.IsEmpty) await Task.Delay(50, ct);
        }
        catch (OperationCanceledException) { }
    }

    internal static string AbortCode(V1.JobAbort.Types.Reason reason) => reason switch
    {
        V1.JobAbort.Types.Reason.Timeout => "timeout",
        V1.JobAbort.Types.Reason.Shutdown => "shutdown",
        V1.JobAbort.Types.Reason.LeaseLost => "lease_lost",
        V1.JobAbort.Types.Reason.AuthChanged => "auth_changed",
        V1.JobAbort.Types.Reason.Removed => "job_removed",
        _ => "cancelled",
    };

    private async Task Execute(Active active, JobRegistration job, string payloadJson, Ctx ctx, RunLogTarget target, ILogger runLog)
    {
        var started = DateTimeOffset.UtcNow;
        PluginError? error = null;
        try
        {
            var payload = Decode(job.Payload, payloadJson);
            await Invoke(job.Job, ctx, payload, target);
        }
        catch (PluginError e)
        {
            error = e;
        }
        catch (OperationCanceledException) when (active.Stop.IsCancellationRequested)
        {
            error = new PluginError(503, "cancelled", "stopped: cancelled");
        }
        catch (Exception e)
        {
            error = new PluginError(500, "internal", e.Message);
        }
        var elapsedMs = (long)(DateTimeOffset.UtcNow - started).TotalMilliseconds;
        if (Volatile.Read(ref active.Answered) == 0)
        {
            if (error is null) runLog.LogInformation("run done in {elapsed_ms} ms", elapsedMs);
            else runLog.LogWarning("run failed in {elapsed_ms} ms: {code}: {error} (permanent: {permanent})", elapsedMs,
                error.Code, error.Message, error.IsPermanent);
        }
        Answer(active, error);
    }

    // the run log follows the handler and what it awaits, and ends when it returns
    private static async Task Invoke(JobBase job, Ctx ctx, object payload, RunLogTarget target)
    {
        RunLog.Current = target;
        await job.RunCore(ctx, payload);
    }

    private static object Decode(Type type, string json)
    {
        try
        {
            return JsonSerializer.Deserialize(json, type, CelobusJson.Options) ?? throw new JsonException("the payload is null");
        }
        catch (Exception e) when (e is JsonException or FormatException or OverflowException or ArgumentException
                                      or InvalidOperationException)
        {
            throw new PluginError(400, "payload_invalid", $"the payload is not a {type.Name}: {e.Message}");
        }
    }

    /// <summary>Sends the attempt's one <c>JobDone</c> (a second answer is dropped) and frees its slot.</summary>
    private void Answer(Active active, PluginError? error)
    {
        if (Interlocked.Exchange(ref active.Answered, 1) != 0) return;
        if (!_runs.TryRemove(new KeyValuePair<string, Active>(active.RunId, active))) return; // the session ended
        Interlocked.Decrement(ref _taken);
        send(new V1.PluginMessage { JobDone = Done(active.RunId, active.Attempt, error) });
    }

    private void Refuse(V1.JobRun run, string code, string message) =>
        send(new V1.PluginMessage { JobDone = Done(run.RunId, run.Attempt, new PluginError(503, code, message)) });

    internal static V1.JobDone Done(string runId, uint attempt, PluginError? error)
    {
        var done = new V1.JobDone { RunId = runId, Attempt = attempt };
        if (error is null) done.Ok = new V1.JobOk();
        else
            done.Error = new V1.JobError
            {
                Code = error.Code,
                Message = RunLog.Cut(error.Message, 2048),
                Permanent = error.IsPermanent,
            };
        return done;
    }
}
