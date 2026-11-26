// Serving: one session with the platform ($CELOBUS_ADDR). The plugin registers, the platform sends requests, the plugin
// answers each one (concurrently) and sends syscalls (entities, directory, calls to dependencies) whose results come
// back on the same stream. The runs of the plugin's jobs come on the same stream and run on a pool of their own
// (Jobs.cs). At most MaxAdmitted requests are handled at once and MaxQueued more wait; past them a request is answered
// 503 with retry-after. A lost session is reopened with backoff (its runs' handlers are stopped); on SIGTERM the plugin
// stops taking requests and runs, finishes the ones in flight and closes the session.
using System.Collections.Concurrent;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Channels;
using Celobus.V1;
using Google.Protobuf;
using Grpc.Core;
using Grpc.Net.Client;
using Microsoft.Extensions.Logging;

namespace Celobus.Sdk;

internal sealed class Runtime
{
    private const string JsonType = "application/json";
    /// <summary>How long in-flight requests and runs may finish after SIGTERM.</summary>
    private static readonly TimeSpan DrainGrace = TimeSpan.FromSeconds(10);

    /// <summary>Requests handled at once; more wait in a queue of <see cref="MaxQueued"/>, past it the answer is 503.</summary>
    internal const int MaxAdmitted = 64;
    internal const int MaxQueued = 256;

    /// <summary>The reserved method of the run-change hook (a leading <c>_</c>: the platform's own requests).</summary>
    internal const string RunChangeMethod = "_RunChange";
    private static readonly TimeSpan RunChangeTimeout = TimeSpan.FromSeconds(30);
    /// <summary>How long the settings check hook may take.</summary>
    private static readonly TimeSpan SettingsCheckTimeout = TimeSpan.FromSeconds(10);

    private readonly PluginDefinition _def;
    private readonly Dictionary<string, EndpointRegistration> _routes;
    private readonly string _runChangePath;
    private readonly string _settingsCheckPath;
    private readonly JsonLogger _log;
    private readonly PluginInfo _info;
    private readonly Dictionary<string, JobRegistration> _jobs;
    /// <summary>The handlers' slots, shared by the sessions.</summary>
    private readonly SemaphoreSlim _permits = new(MaxAdmitted);
    /// <summary>The requests taken and not answered yet: running or waiting for a slot.</summary>
    private int _taken;

    /// <summary>The requests taken and not answered yet (tests).</summary>
    internal int Taken => Volatile.Read(ref _taken);

    public Runtime(PluginDefinition def)
    {
        _def = def;
        _routes = def.Endpoints.ToDictionary(e => Names.Path(def.Id, e.Name), StringComparer.Ordinal);
        _runChangePath = Names.Path(def.Id, RunChangeMethod);
        _settingsCheckPath = Names.Path(def.Id, SettingsDescribe.CheckMethod);
        _log = new JsonLogger("celobus.sdk", [new("plugin", def.Id)]);
        _jobs = [];
        foreach (var j in def.Jobs) _jobs.TryAdd(j.Key, j);
        _info = new PluginInfo(def.Id, def.Version)
        {
            JobKeys = def.Jobs.GroupBy(j => j.Payload).ToDictionary(g => g.Key, g => g.First().Key),
            // the settings the platform sends at registration and on each change, kept across sessions
            SettingsCache = def.Settings is { } s ? new SettingsCache(s.Type) : null,
        };
    }

    /// <summary>The job pool's size, announced at registration (0: the plugin has no jobs).</summary>
    internal int JobSlots => _jobs.Count == 0 ? 0 : _def.JobSlots;

    /// <summary>The runs of one session.</summary>
    internal JobPool Pool(ISyscalls syscalls, Func<PluginMessage, bool> send) => new(_jobs, _info, JobSlots, syscalls, send, _log);

    /// <summary>Opens sessions until <paramref name="stop"/> fires.</summary>
    public async Task Run(string addr, CancellationToken stop)
    {
        using var channel = CreateChannel(addr);
        var client = new Celld.CelldClient(channel);
        var backoff = TimeSpan.FromMilliseconds(250);
        while (!stop.IsCancellationRequested)
        {
            try
            {
                if (await Session(client, stop)) backoff = TimeSpan.FromMilliseconds(250);
            }
            catch (Exception e) when (!stop.IsCancellationRequested)
            {
                _log.LogWarning("session with the platform ended: {reason}; reconnecting in {delay}", e.Message, backoff);
            }
            if (stop.IsCancellationRequested) break;
            try { await Task.Delay(backoff, stop); }
            catch (OperationCanceledException) { break; }
            backoff = TimeSpan.FromTicks(Math.Min(backoff.Ticks * 2, TimeSpan.FromSeconds(5).Ticks));
        }
        _log.LogInformation("stopped");
    }

    /// <summary>The channel to the platform at <c>$CELOBUS_ADDR</c>.</summary>
    internal static GrpcChannel CreateChannel(string addr)
    {
        if (!addr.StartsWith("unix:", StringComparison.Ordinal))
            throw new ArgumentException($"CELOBUS_ADDR must be unix:<path>, got {addr}");
        var path = addr.StartsWith("unix://", StringComparison.Ordinal) ? addr[7..] : addr[5..];
        return GrpcChannel.ForAddress("http://localhost", new GrpcChannelOptions
        {
            MaxReceiveMessageSize = 64 << 20,
            MaxSendMessageSize = 64 << 20,
            HttpHandler = new SocketsHttpHandler
            {
                ConnectCallback = async (_, ct) =>
                {
                    var socket = new Socket(AddressFamily.Unix, SocketType.Stream, ProtocolType.Unspecified);
                    try
                    {
                        await socket.ConnectAsync(new UnixDomainSocketEndPoint(path), ct);
                        return new NetworkStream(socket, ownsSocket: true);
                    }
                    catch
                    {
                        socket.Dispose();
                        throw;
                    }
                },
            },
        });
    }

    /// <summary>One session; returns whether the platform registered the plugin. Returns normally on <paramref name="stop"/>.</summary>
    private async Task<bool> Session(Celld.CelldClient client, CancellationToken stop)
    {
        using var sessionCts = new CancellationTokenSource();
        var sessionToken = sessionCts.Token;
        using var call = client.Session(cancellationToken: sessionToken);
        var outbox = Channel.CreateUnbounded<PluginMessage>(new UnboundedChannelOptions { SingleReader = true });
        var session = new Session(outbox.Writer);
        var pool = Pool(session, outbox.Writer.TryWrite);
        var inflight = new ConcurrentDictionary<ulong, Task>();
        var registered = false;
        var draining = false;

        // the only writer of the request stream
        var writer = Task.Run(async () =>
        {
            await foreach (var m in outbox.Reader.ReadAllAsync(sessionCts.Token))
                await call.RequestStream.WriteAsync(m, sessionCts.Token);
            await call.RequestStream.CompleteAsync();
        });

        var register = new Register
        {
            Plugin = _def.Id,
            Version = _def.Version,
            Runtime = "dotnet",
            ManifestDigest = "",
            Token = Environment.GetEnvironmentVariable("CELOBUS_PLUGIN_TOKEN") ?? "",
            JobSlots = (uint)JobSlots,
        };
        register.Depends.AddRange(_def.Dependencies.Select(d => d.Plugin).Distinct());
        outbox.Writer.TryWrite(new PluginMessage { Register = register });

        var reader = Task.Run(async () =>
        {
            await foreach (var msg in call.ResponseStream.ReadAllAsync(sessionCts.Token))
            {
                switch (msg.KindCase)
                {
                    case CelldMessage.KindOneofCase.Registered:
                        registered = true;
                        _log.LogInformation("registered as {instance}", msg.Registered.InstanceId);
                        if (_info.SettingsCache?.Update(msg.Registered.SettingsRev, msg.Registered.Settings.Span, registered: true) == false)
                            _log.LogWarning("the settings the platform sent are not JSON; kept the previous ones");
                        break;
                    case CelldMessage.KindOneofCase.SettingsChanged:
                        if (_info.SettingsCache?.Update(msg.SettingsChanged.Rev, msg.SettingsChanged.Json.Span, registered: false) == true)
                            _log.LogInformation("the settings changed (revision {rev})", msg.SettingsChanged.Rev);
                        break;
                    case CelldMessage.KindOneofCase.Request:
                        var req = msg.Request;
                        var busy = Interlocked.Increment(ref _taken) > MaxAdmitted + MaxQueued;
                        if (busy || Volatile.Read(ref draining))
                        {
                            Interlocked.Decrement(ref _taken);
                            var refused = Error(PluginError.Unavailable(busy ? "the plugin is busy" : "the plugin is stopping"));
                            refused.Id = req.Id;
                            refused.Headers.Add(new Header { Name = "retry-after", Value = "1" });
                            outbox.Writer.TryWrite(new PluginMessage { Response = refused });
                            break;
                        }
                        // in flight before its handler starts (or waits for a slot): a stop that comes at once still waits for it
                        var done = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
                        inflight[req.Id] = done.Task;
                        _ = Task.Run(async () =>
                        {
                            var admitted = false;
                            try
                            {
                                await _permits.WaitAsync(sessionToken);
                                admitted = true;
                                var resp = await Dispatch(req, session, sessionToken);
                                resp.Id = req.Id;
                                outbox.Writer.TryWrite(new PluginMessage { Response = resp });
                            }
                            catch (OperationCanceledException) when (!admitted)
                            {
                                // the session ended while it waited for a slot: there is no one to answer
                            }
                            finally
                            {
                                if (admitted) _permits.Release();
                                Interlocked.Decrement(ref _taken);
                                inflight.TryRemove(req.Id, out _);
                                done.TrySetResult();
                            }
                        });
                        break;
                    case CelldMessage.KindOneofCase.SyscallResult:
                        session.Complete(msg.SyscallResult);
                        break;
                    case CelldMessage.KindOneofCase.JobRun:
                        pool.Start(msg.JobRun, Volatile.Read(ref draining));
                        break;
                    case CelldMessage.KindOneofCase.JobAbort:
                        pool.Abort(msg.JobAbort);
                        break;
                }
            }
        });

        var stopped = new TaskCompletionSource();
        await using var onStop = stop.Register(() => stopped.TrySetResult());
        try
        {
            if (await Task.WhenAny(reader, stopped.Task) == stopped.Task)
            {
                Volatile.Write(ref draining, true);
                _log.LogInformation("stopping: finishing {count} requests and {runs} runs in flight", inflight.Count, pool.Running);
                using var grace = new CancellationTokenSource(DrainGrace);
                await Task.WhenAny(Task.WhenAll(Task.WhenAll(inflight.Values), pool.WhenIdle(grace.Token)), Task.Delay(DrainGrace));
                outbox.Writer.TryComplete();
                await Task.WhenAny(writer, Task.Delay(TimeSpan.FromSeconds(2)));
                // the last answers are sent; cancelling the call now could drop them before the platform reads them,
                // so give it a moment to end the session from its side
                await Task.WhenAny(reader, Task.Delay(TimeSpan.FromSeconds(2)));
                return registered;
            }
            await reader; // throws the reason the session ended
            throw new IOException("the platform closed the session");
        }
        finally
        {
            pool.StopAll();
            outbox.Writer.TryComplete();
            session.FailAll();
            await sessionCts.CancelAsync();
            await Task.WhenAll(Quiet(writer), Quiet(reader));
        }
    }

    private static async Task Quiet(Task t)
    {
        try { await t; }
        catch { /* the session is gone */ }
    }

    /// <summary>Serves one request: route, decode, run the handler, encode the answer or the error.</summary>
    internal async Task<Response> Dispatch(Request req, ISyscalls syscalls, CancellationToken session)
    {
        var path = req.Path;
        var q = path.IndexOf('?');
        if (q >= 0) path = path[..q];
        // the run-change hook is a request of the platform itself: it carries no caller, so one with a caller came from outside
        var hook = _def.OnRunChange is not null && path == _runChangePath && req.Principal is null ? _def.OnRunChange : null;
        // so is the settings check hook
        var check = _def.Settings?.OnChange is { } c && path == _settingsCheckPath && req.Principal is null ? c : null;
        EndpointRegistration? route = null;
        if (hook is null && check is null && !_routes.TryGetValue(path, out route))
            return Error(new PluginError(404, "unknown_endpoint", $"no endpoint {path}"));
        var name = route?.Name ?? (check is not null ? SettingsDescribe.CheckMethod : RunChangeMethod);
        if (!req.Method.Equals("POST", StringComparison.OrdinalIgnoreCase))
            return Error(new PluginError(405, "method_not_allowed", $"{name} takes POST, not {req.Method}"));
        var contentType = Header(req, "content-type");
        if (!string.IsNullOrEmpty(contentType) && !contentType.StartsWith(JsonType, StringComparison.OrdinalIgnoreCase))
            return Error(new PluginError(415, "unsupported_media_type", $"{contentType} is not supported; send {JsonType}"));

        // the handler: its answer's bytes, null when it returned null
        Func<Ctx, Task<byte[]?>> handle;
        TimeSpan timeout;
        if (check is not null)
        {
            JsonNode? body;
            try
            {
                body = req.Body.IsEmpty ? new JsonObject() : JsonNode.Parse(req.Body.Span);
            }
            catch (JsonException e)
            {
                return Error(PluginError.Invalid($"malformed settings change: {e.Message}"));
            }
            timeout = SettingsCheckTimeout;
            handle = async ctx =>
            {
                await check(ctx, body);
                return "{}"u8.ToArray();
            };
        }
        else if (route is null)
        {
            RunChange change;
            try
            {
                change = RunChange.FromJson(req.Body.Span);
            }
            catch (PluginError e)
            {
                return Error(e);
            }
            timeout = RunChangeTimeout;
            handle = async ctx =>
            {
                await hook!(ctx, change);
                return "{}"u8.ToArray();
            };
        }
        else
        {
            object input;
            try
            {
                var body = req.Body.IsEmpty ? "{}"u8 : req.Body.Span;
                input = JsonSerializer.Deserialize(body, route.Endpoint.RequestType, CelobusJson.Options)
                        ?? throw new JsonException("the body is null");
            }
            catch (Exception e) when (e is JsonException or FormatException or OverflowException or ArgumentException
                                          or InvalidOperationException)
            {
                if (e is FieldJsonException f)
                    return Error(PluginError.Invalid($"malformed {route.Endpoint.RequestType.Name}: {f.Message}").Field(f.FieldPath, f.Reason));
                return Error(PluginError.Invalid($"malformed {route.Endpoint.RequestType.Name}: {e.Message}"));
            }
            timeout = route.Options.TimeoutValue;
            handle = async ctx =>
            {
                var output = await route.Endpoint.HandleCore(ctx, input);
                return output is null
                    ? null
                    : JsonSerializer.SerializeToUtf8Bytes(output, route.Endpoint.ResponseType, CelobusJson.Options);
            };
        }

        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(session);
        deadline.CancelAfter(timeout);
        var request = new RequestInfo(req.Id, name, DateTimeOffset.UtcNow + timeout, Header(req, "idempotency-key"));
        var log = _log.With(new("request_id", req.Id), new("endpoint", name));
        var ctx = new Ctx(syscalls, _info, request, new Caller(_def.Id, req.Principal),
            log, deadline.Token);
        try
        {
            var bytes = await handle(ctx);
            if (bytes is null)
            {
                log.LogError("the handler returned null");
                return Error(PluginError.Internal());
            }
            return Answer(200, bytes);
        }
        catch (PluginError e)
        {
            if (e.Status >= 500) log.LogWarning("{code}: {message}", e.Code, e.Message);
            return Error(e);
        }
        catch (OperationCanceledException) when (deadline.IsCancellationRequested && !session.IsCancellationRequested)
        {
            log.LogWarning("deadline exceeded after {timeout}", timeout);
            return Error(new PluginError(504, "deadline_exceeded", $"{name} did not finish in {timeout.TotalSeconds:0.###} s"));
        }
        catch (OperationCanceledException) when (session.IsCancellationRequested)
        {
            return Error(PluginError.Unavailable("the session with the platform closed"));
        }
        catch (Exception e)
        {
            log.LogError(e, "the handler failed");
            return Error(PluginError.Internal());
        }
    }

    private static string? Header(Request req, string name) =>
        req.Headers.FirstOrDefault(h => h.Name.Equals(name, StringComparison.OrdinalIgnoreCase))?.Value;

    internal static Response Error(PluginError e) =>
        Answer(e.Status, Encoding.UTF8.GetBytes(e.ToJson().ToJsonString(CelobusJson.Options)));

    private static Response Answer(int status, byte[] body)
    {
        var r = new Response { Status = (uint)status, Body = ByteString.CopyFrom(body) };
        r.Headers.Add(new Header { Name = "content-type", Value = JsonType });
        return r;
    }
}

/// <summary>The syscalls in flight of one session.</summary>
internal sealed class Session(ChannelWriter<PluginMessage> outbox) : ISyscalls
{
    private readonly ConcurrentDictionary<ulong, TaskCompletionSource<SyscallResult>> _pending = new();
    private long _next;
    private volatile bool _closed;

    public async Task<SyscallResult> Call(Syscall call, CancellationToken ct)
    {
        call.Id = (ulong)Interlocked.Increment(ref _next);
        var tcs = new TaskCompletionSource<SyscallResult>(TaskCreationOptions.RunContinuationsAsynchronously);
        _pending[call.Id] = tcs;
        if (_closed || !outbox.TryWrite(new PluginMessage { Syscall = call }))
        {
            _pending.TryRemove(call.Id, out _);
            throw PluginError.Unavailable("the session with the platform is closed");
        }
        try
        {
            return await tcs.Task.WaitAsync(ct);
        }
        finally
        {
            _pending.TryRemove(call.Id, out _);
        }
    }

    public void Invalidate(string topic, string key)
    {
        if (!_closed) outbox.TryWrite(new PluginMessage { Invalidate = new global::Celobus.V1.Invalidate { Topic = topic, Key = key } });
    }

    public void Complete(SyscallResult r)
    {
        if (_pending.TryRemove(r.Id, out var tcs)) tcs.TrySetResult(r);
    }

    public void FailAll()
    {
        _closed = true;
        foreach (var id in _pending.Keys)
            if (_pending.TryRemove(id, out var tcs))
                tcs.TrySetException(PluginError.Unavailable("the session with the platform is closed"));
    }
}
