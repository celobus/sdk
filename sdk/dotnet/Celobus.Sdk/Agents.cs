// The runs of the plugin's own agents, from its code: an endpoint
// starts a run of an agent the plugin declares and answers its id; the screen reads the run through another endpoint
// of the plugin and re-reads when the agent's topic is invalidated (AgentOptions.Topic). The plugin also starts the
// agents of the workspace's other plugins its caller reaches, by <plugin>.<name> (Agents.Available lists them), and
// follows those runs on a topic of its own (AgentRunOptions.Topic).
using System.Text.Json;
using System.Text.Json.Nodes;
using Celobus.V1;

namespace Celobus.Sdk;

/// <summary>
/// Where a run is: <c>Starting</c> (the run is being prepared) → <c>Running</c> ⇄ <c>Idle</c> (a session between turns) →
/// one of the four ends.
/// </summary>
public enum AgentRunStatus
{
    /// <summary>A status this SDK does not know (a newer platform).</summary>
    Unknown,
    Starting,
    Running,
    Idle,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

/// <summary>
/// A change of a run the plugin sees, as the run-change hook tells it (<see cref="Plugin.OnRunChange"/>): a run of one
/// of the plugin's agents, or one its code started. It names the run and where it stands; the handler reads the rest
/// with <c>ctx.Agents</c>. Delivered at least once and coalesced (the latest state of a burst): handle it idempotently,
/// and read the run rather than trust the order of deliveries.
/// </summary>
public sealed record RunChange
{
    public required string RunId { get; init; }

    /// <summary>The agent's name in its plugin.</summary>
    public string Agent { get; init; } = "";

    /// <summary>The agent's plugin.</summary>
    public string Plugin { get; init; } = "";

    public AgentRunStatus Status { get; init; }

    public int Turn { get; init; }

    /// <summary>The sequence number of the run's last event.</summary>
    public long LastSeq { get; init; }

    /// <summary>The topic the start named (<c>AgentRunOptions.Topic</c>; empty: none).</summary>
    public string Topic { get; init; } = "";

    /// <summary>The topic's canonical key (empty: every key).</summary>
    public string TopicKey { get; init; } = "";

    /// <summary>The run is over: nothing more is written to it.</summary>
    public bool Ended { get; init; }

    /// <summary>Reads the hook's body (<c>{run_id, agent, plugin, status, turn, last_seq, topic, topic_key, ended}</c>).</summary>
    internal static RunChange FromJson(ReadOnlySpan<byte> body)
    {
        JsonElement v;
        try
        {
            var reader = new Utf8JsonReader(body.IsEmpty ? "{}"u8 : body);
            v = JsonElement.ParseValue(ref reader);
        }
        catch (JsonException x)
        {
            throw PluginError.Invalid($"malformed RunChange: {x.Message}");
        }
        if (v.ValueKind != JsonValueKind.Object) throw PluginError.Invalid("malformed RunChange: expected an object");
        string S(string k) => v.TryGetProperty(k, out var x) && x.ValueKind == JsonValueKind.String ? x.GetString()! : "";
        ulong N(string k) => v.TryGetProperty(k, out var x) && x.ValueKind == JsonValueKind.Number && x.TryGetUInt64(out var n) ? n : 0;
        var runId = S("run_id");
        if (runId.Length == 0) throw PluginError.Invalid("run_id is required");
        var status = AgentRun.StatusOf(S("status"));
        var ended = v.TryGetProperty("ended", out var e) && e.ValueKind is JsonValueKind.True or JsonValueKind.False
            ? e.GetBoolean()
            : status is AgentRunStatus.Succeeded or AgentRunStatus.Failed or AgentRunStatus.Cancelled or AgentRunStatus.TimedOut;
        return new RunChange
        {
            RunId = runId,
            Agent = S("agent"),
            Plugin = S("plugin"),
            Status = status,
            Turn = (int)Math.Min(N("turn"), int.MaxValue),
            LastSeq = (long)Math.Min(N("last_seq"), long.MaxValue),
            Topic = S("topic"),
            TopicKey = S("topic_key"),
            Ended = ended,
        };
    }
}

/// <summary>Who started a run, or sent it a message.</summary>
public sealed record AgentActor(Id<Platform.User> UserId, string DisplayName);

/// <summary>Why a run failed or timed out (<c>start_timeout</c>, <c>lost</c>, <c>agent_failed</c>, …).</summary>
public sealed record AgentRunError(string Code, string Message);

/// <summary>A run of one of the plugin's agents, or one its code started. Times are UTC.</summary>
public sealed record AgentRun
{
    public required string Id { get; init; }

    /// <summary>The agent's name in its plugin (<see cref="Plugin"/>).</summary>
    public required string Agent { get; init; }

    public required string Plugin { get; init; }
    public required string Engine { get; init; }

    /// <summary>Null: the engine's own model.</summary>
    public string? Model { get; init; }

    public required AgentRunStatus Status { get; init; }

    /// <summary>The run waits for follow-up messages after every turn.</summary>
    public bool Session { get; init; }

    /// <summary>The task it was started with.</summary>
    public string Input { get; init; } = "";

    /// <summary>The result of the last finished turn.</summary>
    public string? Result { get; init; }

    public AgentRunError? Error { get; init; }

    /// <summary>The caller of the request the run was started in (or of the HTTP call that started it).</summary>
    public required AgentActor StartedBy { get; init; }

    /// <summary>The plugin whose code started it; null: started over HTTP (the CLI, a client).</summary>
    public string? StartedByPlugin { get; init; }

    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }
    public DateTimeOffset DeadlineAt { get; init; }
    public int Turn { get; init; }

    /// <summary>The sequence number of the run's last event.</summary>
    public long LastSeq { get; init; }

    /// <summary>Where the run executes, an opaque name as the platform last reported it.</summary>
    public string Pod { get; init; } = "";

    /// <summary>
    /// The group the run belongs to: the id of the group's first run (its own id for a run that founded one). The
    /// agents of a group message each other by name; an agent never leaves its group.
    /// </summary>
    public string Group { get; init; } = "";

    /// <summary>The agent run that started this one; null: a person or a plugin's code did.</summary>
    public string? Parent { get; init; }

    /// <summary>The agent of <see cref="Parent"/>, <c>&lt;plugin&gt;.&lt;name&gt;</c>; null with it.</summary>
    public string? ParentAgent { get; init; }

    /// <summary>
    /// The person at the root: who started the run, or the run that started it (<see cref="StartedBy"/> when the
    /// platform names none).
    /// </summary>
    public required AgentActor OnBehalfOf { get; init; }

    /// <summary>The run is over: nothing more is written to it.</summary>
    public bool Ended => Status is AgentRunStatus.Succeeded or AgentRunStatus.Failed or AgentRunStatus.Cancelled
        or AgentRunStatus.TimedOut;

    /// <summary>The platform's name of a status (<c>timed_out</c>) → the status.</summary>
    internal static AgentRunStatus StatusOf(string status) => status switch
    {
        "starting" or "queued" => AgentRunStatus.Starting,
        "running" => AgentRunStatus.Running,
        "idle" => AgentRunStatus.Idle,
        "succeeded" => AgentRunStatus.Succeeded,
        "failed" => AgentRunStatus.Failed,
        "cancelled" => AgentRunStatus.Cancelled,
        "timed_out" => AgentRunStatus.TimedOut,
        _ => AgentRunStatus.Unknown,
    };

    private static AgentActor Actor(AgentRunActor? a) => new(new Id<Platform.User>(a?.UserId ?? ""), a?.DisplayName ?? "");

    internal static AgentRun From(V1.AgentRun r) => new()
    {
        Id = r.Id,
        Agent = r.Agent,
        Plugin = r.Plugin,
        Engine = r.Engine,
        Model = r.Model.Length > 0 ? r.Model : null,
        Status = StatusOf(r.Status),
        Session = r.Session,
        Input = r.Input,
        Result = r.Result.Length > 0 ? r.Result : null,
        Error = r.Error is { } e ? new AgentRunError(e.Code, e.Message) : null,
        StartedBy = Actor(r.StartedBy),
        StartedByPlugin = r.StartedByPlugin.Length > 0 ? r.StartedByPlugin : null,
        CreatedAt = DateTimeOffset.FromUnixTimeMilliseconds(r.CreatedAt),
        UpdatedAt = DateTimeOffset.FromUnixTimeMilliseconds(r.UpdatedAt),
        DeadlineAt = DateTimeOffset.FromUnixTimeMilliseconds(r.DeadlineAt),
        Turn = (int)r.Turn,
        LastSeq = (long)r.LastSeq,
        Pod = r.Pod,
        Group = r.Group,
        Parent = r.Parent.Length > 0 ? r.Parent : null,
        ParentAgent = r.ParentAgent.Length > 0 ? r.ParentAgent : null,
        OnBehalfOf = Actor(r.OnBehalfOf ?? r.StartedBy),
    };
}

/// <summary>
/// One event of a run: <see cref="Type"/> is <c>message</c>, <c>tool</c>, <c>turn</c>, <c>status</c>,
/// <c>progress</c> or <c>error</c>, and <see cref="Data"/> the whole event as the platform keeps it, with the fields
/// of its type.
/// </summary>
public sealed record AgentEvent(long Seq, DateTimeOffset At, string Type, JsonObject Data)
{
    /// <summary>A string field of the event (<c>text</c>, <c>role</c>, <c>name</c>, <c>phase</c>, <c>summary</c>, …), or null.</summary>
    public string? Str(string field) =>
        Data[field] is JsonValue v && v.GetValueKind() == JsonValueKind.String ? v.GetValue<string>() : null;

    /// <summary>The text of a <c>message</c>, <c>status</c> or <c>progress</c> event.</summary>
    public string? Text => Str("text");
}

/// <summary>The text of one message while the agent writes it.</summary>
public sealed record AgentPartialMessage(string MessageId, string Text);

/// <summary>
/// The messages being written right now: text, not events (the finished message arrives as an event and takes their
/// place). <see cref="Rev"/> counts the changes (0: nothing was written yet).
/// </summary>
public sealed record AgentPartial(long Rev, IReadOnlyList<AgentPartialMessage> Messages)
{
    /// <summary>The unfinished text, the messages joined by blank lines.</summary>
    public string Text => string.Join("\n\n", Messages.Select(m => m.Text));
}

/// <summary>A run with its events after a sequence number (oldest first) and its unfinished text.</summary>
public sealed record AgentEvents(AgentRun Run, IReadOnlyList<AgentEvent> Events, AgentPartial Partial);

/// <summary>How a run is started, when it differs from what the agent declares.</summary>
public sealed record AgentRunOptions
{
    public AgentEngine? Engine { get; init; }
    public string? Model { get; init; }

    /// <summary>The run is a session (it waits for follow-up messages after every turn), or is not.</summary>
    public bool? Session { get; init; }

    /// <summary>The longest the run takes, from a minute to six hours.</summary>
    public TimeSpan? Timeout { get; init; }

    /// <summary>
    /// The group the run joins: one this plugin's code founded, that is, the <see cref="AgentRun.Group"/> of a run
    /// its code started. Null: the run founds a new group.
    /// </summary>
    public string? Group { get; init; }

    /// <summary>
    /// Tell the person the run works for, in their inbox, when it finishes (a run that fails or runs out of time
    /// always tells them).
    /// </summary>
    public bool Notify { get; init; }

    /// <summary>
    /// Where the notification of the run's end opens: <see cref="Sdk.NotifyLink.Screen{TParams}"/> or
    /// <see cref="Sdk.NotifyLink.Overlay{TParams}"/> of this plugin's UI (400 <c>bad_request</c> when the UI has no
    /// such target or params).
    /// </summary>
    public NotifyLink? NotifyLink { get; init; }

    /// <summary>
    /// The name of a topic of this plugin (declared with <c>Plugin.Topic</c>; <c>MyTopic.Name</c>) the platform
    /// invalidates, for every key, whenever the run changes, as it does the topic the agent declares: how a plugin
    /// follows the runs it starts of another plugin's agent. 400 <c>bad_request</c> when the plugin declares no such
    /// topic.
    /// </summary>
    public string? Topic { get; init; }

    /// <summary>
    /// The key of <see cref="Topic"/> to invalidate, as the platform matches it: only the screens that follow the topic
    /// for this key re-read. Made with <see cref="Agents.TopicKey{TKey}"/>; null: every key.
    /// </summary>
    public string? TopicKey { get; init; }
}

/// <summary>
/// An agent of the workspace the caller of the request may start a run of (<see cref="Agents.Available"/>), as its
/// installed package describes it.
/// </summary>
public sealed record AvailableAgent
{
    /// <summary><c>&lt;plugin&gt;.&lt;name&gt;</c>: what <see cref="Agents.Run"/> takes.</summary>
    public required string Agent { get; init; }

    public required string Plugin { get; init; }
    public required string Name { get; init; }

    /// <summary>Its display name: the declared title, else its name.</summary>
    public required string Title { get; init; }

    /// <summary>What it is for, in a sentence (empty: not said).</summary>
    public string Description { get; init; } = "";

    /// <summary>Its glyph, a lucide icon name; null: none.</summary>
    public string? Icon { get; init; }

    /// <summary>The engine its runs get in this workspace when a start names none: an owner's or admin's choice, else the one it declares, else its connection's; null: none can be resolved (it has no connection).</summary>
    public string? Engine { get; init; }

    /// <summary>Its runs are sessions by its declaration.</summary>
    public bool Session { get; init; }

    /// <summary>Its plugin's title, else the plugin's id.</summary>
    public required string PluginTitle { get; init; }

    internal static AvailableAgent From(V1.AvailableAgent a) => new()
    {
        Agent = a.Agent,
        Plugin = a.Plugin,
        Name = a.Name,
        Title = a.Title,
        Description = a.Description,
        Icon = a.Icon.Length > 0 ? a.Icon : null,
        Engine = a.Engine.Length > 0 ? a.Engine : null,
        Session = a.Session,
        PluginTitle = a.PluginTitle,
    };
}

/// <summary>
/// The runs of the plugin's agents (<c>ctx.Agents</c>). A plugin reaches the runs of its own agents, all of them,
/// whoever started them, and the runs its code started of other plugins' agents: its endpoints decide whom to show
/// which run. A run is started, and a message sent, by the caller of the request being handled; outside a request
/// both are refused with 403 <c>forbidden</c>.
/// </summary>
public readonly struct Agents
{
    private readonly Ctx _ctx;

    internal Agents(Ctx ctx) { _ctx = ctx; }

    private static PluginError Unexpected() => PluginError.Unavailable("platform: unexpected syscall result");

    private async Task<AgentRun> RunOf(Syscall call) =>
        AgentRun.From((await _ctx.Syscall(call)).AgentRun ?? throw Unexpected());

    /// <summary>
    /// Starts a run of the agent <paramref name="agent"/> with the task <paramref name="input"/>, as the agent
    /// declares its runs unless <paramref name="options"/> say otherwise: the plugin's own agent by its name, or an
    /// agent of another plugin of the workspace as <c>&lt;plugin&gt;.&lt;name&gt;</c> (403 <c>forbidden</c> when the
    /// caller does not reach that plugin or is an agent's account). It is started by this request's caller. 404 <c>unknown_agent</c> when
    /// there is no such agent. With
    /// <see cref="AgentRunOptions.Group"/> the run joins a group this plugin's code founded (the
    /// <see cref="AgentRun.Group"/> of a run its code started) instead of founding one: 404 <c>unknown_group</c> when
    /// the platform no longer remembers the group's first run or another plugin founded it, 409 <c>agent_in_group</c>
    /// when the agent has a live run in the group already, 409 <c>group_full</c> when the group has its eight live
    /// runs.
    /// </summary>
    public Task<AgentRun> Run(string agent, string input, AgentRunOptions? options = null)
    {
        var start = new AgentRunStart
        {
            Agent = agent,
            Input = input,
            Engine = options?.Engine?.ToString().ToLowerInvariant() ?? "",
            Model = options?.Model ?? "",
            TimeoutS = options?.Timeout is { } t ? (uint)Math.Clamp(t.TotalSeconds, 0, uint.MaxValue) : 0,
            RequestId = _ctx.Request.Id,
        };
        // left unset unless said: the agent's declaration decides
        if (options?.Session is { } session) start.Session = session;
        // empty: the run founds a new group
        if (options?.Group is { } group) start.Group = group;
        start.Notify = options?.Notify ?? false;
        if (options?.NotifyLink is { } link) start.NotifyLink = link.Wire();
        if (options?.Topic is { } topic) start.Topic = topic;
        if (options?.TopicKey is { } topicKey) start.TopicKey = topicKey;
        return RunOf(new Syscall { AgentRun = start });
    }

    /// <summary>
    /// The key of <paramref name="topic"/> as the platform matches it, for <see cref="AgentRunOptions.TopicKey"/>:
    /// <c>new AgentRunOptions { Topic = Conversation.Name, TopicKey = Agents.TopicKey(Conversation, key) }</c>.
    /// </summary>
    public static string TopicKey<TKey>(Ui.Topic<TKey> topic, TKey key) where TKey : class =>
        Ctx.CanonicalKey(JsonSerializer.SerializeToNode(key, CelobusJson.Options));

    /// <summary>
    /// The agents of the workspace this request's caller may start a run of: those of the plugins they reach, this
    /// plugin's own among them, sorted by plugin and name. 403 <c>forbidden</c> outside a request of a signed-in
    /// caller and for an agent's account.
    /// </summary>
    public async Task<IReadOnlyList<AvailableAgent>> Available()
    {
        var call = new Syscall { AgentsAvailable = new AgentsAvailable { RequestId = _ctx.Request.Id } };
        var page = (await _ctx.Syscall(call)).AgentsAvailable ?? throw Unexpected();
        return [.. page.Agents.Select(AvailableAgent.From)];
    }

    /// <summary>
    /// The run; 404 <c>not_found</c> when it is neither a run of this plugin's agents nor one its code started.
    /// </summary>
    public Task<AgentRun> Get(string runId) => RunOf(new Syscall { AgentGet = new AgentRunGet { RunId = runId } });

    /// <summary>
    /// The run, its events after the sequence number <paramref name="after"/> (0: from the first; oldest first, at
    /// most <paramref name="limit"/>, 0: all the run keeps) and the text being written. Answered at once: follow the
    /// agent's topic to know when to read again.
    /// </summary>
    public async Task<AgentEvents> Events(string runId, long after = 0, int limit = 0)
    {
        var call = new Syscall { AgentEvents = new AgentRunEvents { RunId = runId, After = (ulong)Math.Max(after, 0), Limit = (uint)Math.Max(limit, 0) } };
        var page = (await _ctx.Syscall(call)).AgentEvents ?? throw Unexpected();
        var events = page.Events.Select(e =>
        {
            JsonObject data;
            try { data = JsonNode.Parse(e.Json.Span) as JsonObject ?? []; }
            catch (JsonException) { data = []; }
            return new AgentEvent((long)e.Seq, DateTimeOffset.FromUnixTimeMilliseconds(e.At), e.Type, data);
        }).ToList();
        var partial = new AgentPartial((long)(page.Partial?.Rev ?? 0),
            [.. (page.Partial?.Messages ?? []).Select(m => new AgentPartialMessage(m.MessageId, m.Text))]);
        return new AgentEvents(AgentRun.From(page.Run ?? throw Unexpected()), events, partial);
    }

    /// <summary>
    /// The recent runs the plugin sees (of its agents, and the runs its code started), newest first: of one agent
    /// (its name, or <c>&lt;plugin&gt;.&lt;name&gt;</c> of another plugin's) or (null) of all, at most
    /// <paramref name="limit"/> (0: 50; the platform keeps 200), of one group (<see cref="AgentRun.Group"/>) when
    /// <paramref name="group"/> names it.
    /// </summary>
    public async Task<IReadOnlyList<AgentRun>> List(string? agent = null, int limit = 0, string? group = null)
    {
        var call = new Syscall
        {
            AgentRuns = new AgentRunList { Agent = agent ?? "", Limit = (uint)Math.Max(limit, 0), Group = group ?? "" },
        };
        var page = (await _ctx.Syscall(call)).AgentRuns ?? throw Unexpected();
        return [.. page.Runs.Select(AgentRun.From)];
    }

    /// <summary>
    /// Sends a follow-up message to a run that has not ended, as this request's caller; the sequence number of the
    /// input. 409 <c>run_ended</c> when the run is over.
    /// </summary>
    public async Task<long> Send(string runId, string text)
    {
        var call = new Syscall { AgentSend = new AgentRunSend { RunId = runId, Text = text, RequestId = _ctx.Request.Id } };
        return (long)((await _ctx.Syscall(call)).AgentInput ?? throw Unexpected()).Seq;
    }

    /// <summary>Stops the run now; a run that ended stays as it is.</summary>
    public Task<AgentRun> Cancel(string runId) => RunOf(new Syscall { AgentCancel = new AgentRunCancel { RunId = runId } });

    /// <summary>Ends a session after the turn in progress.</summary>
    public Task<AgentRun> Close(string runId) => RunOf(new Syscall { AgentClose = new AgentRunClose { RunId = runId } });
}
