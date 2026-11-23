// The context of one request: who calls, the request itself, calls to dependencies, the plugin's entities and declared
// transactions (Db.cs), the reads of dependencies' entities (Query), SQL sent as text (Sql), the workspace directory,
// the runs of the plugin's agents (Agents.cs), notifications (Notify.cs), background jobs (Jobs.cs), logging. Events
// answer `unavailable`. A job's run has a context of its own kind: `ctx.Job` is the run.
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Nodes;
using Celobus.Sdk.Ui;
using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging;

namespace Celobus.Sdk;

/// <summary>Sends syscalls to the platform and waits for their results (the session, or a test double).</summary>
internal interface ISyscalls
{
    Task<SyscallResult> Call(Syscall call, CancellationToken ct);

    /// <summary>Sends an <c>Invalidate</c> when the session is up; nothing is awaited.</summary>
    void Invalidate(string topic, string key);
}

/// <summary>The context of one request or one job's run, created by the SDK for every request and every attempt.</summary>
public sealed class Ctx
{
    private readonly ISyscalls _syscalls;

    internal Ctx(ISyscalls syscalls, PluginInfo plugin, RequestInfo request, Caller caller, ILogger log,
        CancellationToken cancelled, ulong tx = 0, string system = "", JobInfo? job = null)
    {
        Job = job ?? JobInfo.None;
        _syscalls = syscalls;
        Plugin = plugin;
        Request = request;
        Caller = caller;
        Log = log;
        Cancelled = cancelled;
        Tx = tx;
        System = system;
    }

    /// <summary>
    /// A context without the platform, for unit tests of handlers: the caller is the system (see
    /// <see cref="WithCaller"/>) and every call to the platform answers 503 <c>unavailable</c>.
    /// </summary>
    public static Ctx Test(string plugin, string version) => new(
        Detached.Instance,
        new PluginInfo(plugin, version),
        new RequestInfo(0, "", DateTimeOffset.UtcNow + TimeSpan.FromSeconds(30), null),
        new Caller(plugin, null),
        new JsonLogger("celobus.sdk", [new("plugin", plugin)]),
        CancellationToken.None);

    /// <summary>The same context with another caller (unit tests): <c>ctx.WithCaller(Caller.TestUser("u1"))</c>.</summary>
    public Ctx WithCaller(Caller caller) =>
        new(_syscalls, Plugin, Request, caller.For(Plugin.Id), Log, Cancelled, Tx, System, Job);

    /// <summary>
    /// The same context as a job's run (unit tests of job handlers): <c>ctx.Job</c> answers this run id and attempt,
    /// scheduled now.
    /// </summary>
    public Ctx WithRun(string runId, int attempt, int maxAttempts) =>
        new(_syscalls, Plugin, Request, Caller, Log, Cancelled, Tx, System,
            new JobInfo("", runId, attempt, maxAttempts, DateTimeOffset.UtcNow));

    /// <summary>No session: every syscall answers <c>unavailable</c> (<see cref="Test"/>).</summary>
    private sealed class Detached : ISyscalls
    {
        public static readonly Detached Instance = new();

        public Task<SyscallResult> Call(Syscall call, CancellationToken ct) =>
            Task.FromException<SyscallResult>(PluginError.Unavailable("the session with the platform is closed"));

        public void Invalidate(string topic, string key) { }
    }

    /// <summary>The declared transaction this context's repository calls are part of (0: none).</summary>
    internal ulong Tx { get; }

    /// <summary>The reason of a system context (<see cref="AsSystem"/>); empty for the caller's own.</summary>
    internal string System { get; }

    /// <summary>
    /// The same context with the system's rights on the plugin's own entities: its repository calls (and the
    /// transactions it begins) bypass the access rules within the scope the plugin declares for <c>authz:bypass</c>
    /// (<c>.Bypass&lt;Note&gt;(Perm.Read)</c>; outside it, 403 <c>capability_missing</c>), read masked columns, and are
    /// recorded in the workspace's audit with the caller, the request and <paramref name="reason"/>.
    /// </summary>
    public Ctx AsSystem(string reason) => new(_syscalls, Plugin, Request, Caller, Log, Cancelled, Tx, reason, Job);

    /// <summary>Who calls (resolved by the platform from the workspace's members, never by the plugin).</summary>
    public Caller Caller { get; }

    /// <summary>The request being served (in a job's run: the run's own request id, under the job's key).</summary>
    public RequestInfo Request { get; }

    /// <summary>
    /// The run a job's handler is in: its job, run id, attempt, the attempts it may take and the time it stands for.
    /// Outside a run (a request): an empty run id and attempt 0.
    /// </summary>
    public JobInfo Job { get; }

    /// <summary>This plugin: its id and version.</summary>
    public PluginInfo Plugin { get; }

    /// <summary>
    /// Fires when the request's deadline passes or the plugin stops; in a job's run, when the run is stopped (its
    /// timeout, a cancel, the plugin stopping, the run taken back): pass it to your own I/O and waits.
    /// </summary>
    public CancellationToken Cancelled { get; }

    /// <summary>
    /// Structured logs (JSON lines on stderr) with the request id, plugin and endpoint; in a job's run also kept with
    /// the run, by attempt (each line cut at 2 KiB; at most 1,000 lines and 1 MiB a second per process).
    /// </summary>
    public ILogger Log { get; }

    /// <summary>The workspace's members.</summary>
    public Members Members => new(this);

    /// <summary>The workspace's groups.</summary>
    public Groups Groups => new(this);

    /// <summary>The runs of this plugin's agents: start one, read it and its events, write to it, stop it.</summary>
    public Agents Agents => new(this);

    /// <summary>
    /// Calls an endpoint of a dependency (declared with <c>.DependsOn(plugin, endpoint)</c>) as this request's caller:
    /// <c>await ctx.Call&lt;Customer&gt;("customers", "GetCustomer", new { id })</c>. The input is serialized with the
    /// wire's JSON mapping; a dependency's error is rethrown as a <see cref="PluginError"/> with its status and code.
    /// </summary>
    public async Task<TRes> Call<TRes>(string plugin, string endpoint, object input)
    {
        var json = await CallRaw(plugin, endpoint, JsonSerializer.Serialize(input, input.GetType(), CelobusJson.Options));
        return JsonSerializer.Deserialize<TRes>(json, CelobusJson.Options)
               ?? throw PluginError.Unavailable($"{plugin}.{endpoint} answered null");
    }

    /// <summary>Calls an endpoint of a dependency with a JSON body; returns the JSON answer.</summary>
    public async Task<string> CallRaw(string plugin, string endpoint, string json)
    {
        var request = new Request
        {
            Method = "POST",
            Path = Names.Path(plugin, endpoint),
            Body = ByteString.CopyFromUtf8(json),
        };
        request.Headers.Add(new Header { Name = "content-type", Value = "application/json" });
        var r = await Syscall(new Syscall { Invoke = new Invoke { Plugin = plugin, Request = request, RequestId = Request.Id } });
        var response = r.InvokeResponse ?? throw PluginError.Unavailable($"{plugin}.{endpoint}: no answer");
        var body = response.Body.Span;
        if (response.Status is >= 200 and < 300) return Encoding.UTF8.GetString(body);
        throw PluginError.FromBody((int)response.Status, body)
              ?? new PluginError((int)response.Status, response.Status >= 500 ? "unavailable" : "invalid",
                  $"{plugin}.{endpoint} answered {response.Status}");
    }

    /// <summary>The repository of one of the plugin's entities; in a transaction (<c>tx.Db&lt;T&gt;()</c>) its calls are part of it.</summary>
    /// <exception cref="InvalidOperationException"><typeparamref name="T"/> is a dependency's entity (read it with <see cref="Query{T}"/>).</exception>
    public Db<T> Db<T>() where T : class
    {
        if (Schema.ForeignPlugin(typeof(T)) is { } plugin)
            throw new InvalidOperationException(
                $"{typeof(T).Name} is an entity of {plugin}: read it with ctx.Query<{typeof(T).Name}>(), not ctx.Db<{typeof(T).Name}>()");
        return new(this);
    }

    /// <summary>
    /// A typed read across entities from the root <typeparamref name="T"/>, an <see cref="IQueryable{T}"/>: the
    /// plugin's own entity, a dependency's exported one (the generated client's <c>[Entity(Plugin = "customers")]</c>,
    /// which keeps the repository's reads: <c>await ctx.Query&lt;Deps.Customers.Customer&gt;().Get(id)</c>), or the
    /// directory's (<c>Platform.User</c>, <c>Platform.Group</c>, <c>Platform.GroupMember</c>,
    /// <c>Platform.RoleAssignment</c>). A LINQ query over it (joins, where, group by, orderby, select, Take) is read
    /// with <c>ToListAsync()</c>, <c>FirstOrDefaultAsync()</c>, <c>CountAsync()</c> or <c>PageAsync(token)</c>; as the
    /// caller, every entity through its own rules; in a transaction its reads are part of it. In a system context the
    /// plugin's own entities within its <c>authz:bypass</c>, the directory as a member reads it, and a dependency's
    /// entity under the provider's rules for the caller (never more than the caller reads without it).
    /// </summary>
    public CelobusQuery<T> Query<T>() where T : class => new(this);

    /// <summary>
    /// SQL that reads (<c>select</c>, <c>with</c>, <c>values</c>), with positional parameters: <c>$1</c> is the first of
    /// <paramref name="args"/> (<c>ctx.Sql("select owner_id, count(*) as notes from hello_note group by 1 having
    /// count(*) &gt;= $1", 2)</c>). It reads every installed plugin's entities
    /// (<c>&lt;plugin&gt;_&lt;entity&gt;</c>: <c>chat_post</c>) and of the directory (<c>platform_user</c>,
    /// <c>platform_group</c>, <c>platform_group_member</c>, <c>platform_role_assignment</c>) as the caller: each entity's
    /// own rules and masks apply; never a catalog (<c>pg_class</c>, <c>information_schema</c>,
    /// <c>pg_stat_activity</c>) or another workspace: a relation other than these, or a function in
    /// <c>from</c> other than a few set-returning built-ins (<c>generate_series</c>, <c>unnest</c>, the JSON element and
    /// record functions, <c>regexp_matches</c>, <c>regexp_split_to_table</c>, <c>string_to_table</c>), is 403
    /// <c>permission_denied</c>. Each row is a JSON object keyed by column name as SQL names them (snake_case; a bigint
    /// is a JSON number). In a transaction (<c>tx.Sql(..)</c>) it is part of it and sees its writes. At most
    /// 1,000 rows (422 <c>query_rows_limit</c> beyond); its other refusals are those of <c>ctx.Db</c>. An encrypted field
    /// reads as its ciphertext: read it with <c>ctx.Db</c>.
    /// </summary>
    /// <param name="query">The SQL text, with <c>$1</c>, <c>$2</c>, … where the values go.</param>
    /// <param name="args">The values of <c>$1</c>, <c>$2</c>, …, as JSON (a <c>long</c> as its text, an <c>Id</c> as its
    /// key, a message as its object); each is a value, never SQL.</param>
    public Task<IReadOnlyList<JsonElement>> Sql(string query, params object?[] args) => RunSql(query, args, false);

    /// <summary>
    /// <see cref="Sql(string, object?[])"/> with each row read as <typeparamref name="T"/> by column name (a
    /// <c>[Message]</c> record: <c>select owner_id, count(*) as notes ..</c> fills <c>OwnerId</c> and <c>Notes</c>).
    /// </summary>
    public async Task<IReadOnlyList<T>> Sql<T>(string query, params object?[] args) =>
        Rows<T>(await RunSql(query, args, false));

    /// <summary>
    /// SQL over the tables of every plugin of the workspace (<c>chat_post</c>, <c>crm_customer</c>) past their access
    /// rules, with the capability <c>sql:unsafe</c>: a read, or with the capability <c>sql:write</c> also an
    /// <c>insert</c>, <c>update</c> or <c>delete</c> (its <c>returning</c> rows come back; none without). Never the
    /// platform's tables, the directory's (read it with <see cref="Sql(string, object?[])"/>) or a catalog (403). It
    /// runs in a system context of its own, never in a declared transaction, and each run is recorded in the
    /// workspace's audit; the tables keep their soft-deleted rows, and an encrypted field is its ciphertext.
    /// Parameters and rows as <see cref="Sql(string, object?[])"/>.
    /// </summary>
    public Task<IReadOnlyList<JsonElement>> UnsafeSql(string query, params object?[] args) => RunSql(query, args, true);

    /// <summary><see cref="UnsafeSql(string, object?[])"/> with each row read as <typeparamref name="T"/>.</summary>
    public async Task<IReadOnlyList<T>> UnsafeSql<T>(string query, params object?[] args) =>
        Rows<T>(await RunSql(query, args, true));

    private async Task<IReadOnlyList<JsonElement>> RunSql(string query, object?[]? args, bool @unsafe)
    {
        if (string.IsNullOrWhiteSpace(query)) throw PluginError.Internal();
        var r = await Syscall(new Syscall
        {
            Sql = new SqlRun
            {
                RequestId = Request.Id, Tx = Tx, Query = query, Unsafe = @unsafe,
                Params = ByteString.CopyFromUtf8(JsonSerializer.Serialize(args ?? [null], CelobusJson.Options)),
            },
        });
        var rows = (r.DbRows ?? throw PluginError.Unavailable("platform: unexpected syscall result")).Rows;
        return [.. rows.Select(row => JsonSerializer.Deserialize<JsonElement>(row.Span))];
    }

    private static IReadOnlyList<T> Rows<T>(IReadOnlyList<JsonElement> rows) =>
        [.. rows.Select(row => row.Deserialize<T>(CelobusJson.Options) ?? throw PluginError.Unavailable("platform: an empty row"))];

    /// <summary>
    /// A declared transaction: <paramref name="block"/> gets <c>tx</c>, a context whose repository calls are part of it;
    /// it commits when the block returns and rolls back when it throws. Inside a transaction it is nested and rolls back
    /// alone. The platform rolls back a transaction still open when the request ends or 5 s pass (504
    /// <c>transaction_timeout</c>).
    /// </summary>
    public async Task<TRes> Transaction<TRes>(Func<Ctx, Task<TRes>> block, TxOptions? options = null)
    {
        options ??= new TxOptions();
        var begun = await Syscall(new Syscall
        {
            TxBegin = new TxBegin
            {
                RequestId = Request.Id, Tx = Tx, ReadOnly = options.ReadOnly, AsSystem = System,
                Isolation = options.Isolation switch
                {
                    TxIsolation.RepeatableRead => V1.TxBegin.Types.Isolation.RepeatableRead,
                    TxIsolation.Serializable => V1.TxBegin.Types.Isolation.Serializable,
                    _ => V1.TxBegin.Types.Isolation.ReadCommitted,
                },
            },
        });
        var id = (begun.TxBegun ?? throw PluginError.Unavailable("platform: unexpected syscall result")).Tx;
        var tx = new Ctx(_syscalls, Plugin, Request, Caller, Log, Cancelled, id, System, Job);
        TRes value;
        try
        {
            value = await block(tx);
        }
        catch
        {
            try
            {
                await Syscall(new Syscall { TxRollback = new TxRollback { RequestId = Request.Id, Tx = id } });
            }
            catch (PluginError)
            {
                // the platform rolls it back anyway when the request ends
            }
            throw;
        }
        await Syscall(new Syscall { TxCommit = new TxCommit { RequestId = Request.Id, Tx = id } });
        return value;
    }

    /// <summary>A declared transaction whose block answers nothing.</summary>
    public Task Transaction(Func<Ctx, Task> block, TxOptions? options = null) =>
        Transaction(async tx =>
        {
            await block(tx);
            return true;
        }, options);

    /// <summary>Events: not available, <c>Emit</c> answers <c>unavailable</c>.</summary>
    public Events Events => new();

    /// <summary>
    /// The plugin's background jobs: <c>Enqueue</c> a run, <c>Cancel</c> one. In a declared transaction
    /// (<c>tx.Jobs</c>) an enqueued run exists only if the transaction commits.
    /// </summary>
    public Jobs Jobs => new(this);

    /// <summary>
    /// The plugin's notifications: <c>Send</c> a notice of a declared kind, <c>Ask</c> an
    /// approval or a decision and <c>Resolve</c> it, <c>Follow</c> / <c>Unfollow</c> a key, <c>Following(key)</c>. In a
    /// transaction (<c>tx.Notifications</c>) the writes are part of it and its members hear of them when it commits:
    /// send last in it.
    /// </summary>
    public Notifications Notifications => new(this);

    /// <summary>
    /// Tells the screens following <paramref name="topic"/> for <paramref name="key"/> to re-read (plugin UI). An
    /// endpoint that declares <c>e.Invalidates(..)</c> needs no call: the platform emits those after its 2xx answer. This
    /// is for the rest: a change made outside a declared endpoint, or a key the endpoint's response does not carry.
    /// Nothing is awaited, and a screen that misses it re-reads within its maximum data age.
    /// </summary>
    public void Invalidate<TKey>(Topic<TKey> topic, TKey key) where TKey : class =>
        _syscalls.Invalidate(topic.Name, CanonicalKey(JsonSerializer.SerializeToNode(key, CelobusJson.Options)));

    /// <summary>Every key of <paramref name="topic"/> re-reads.</summary>
    public void InvalidateAll<TKey>(Topic<TKey> topic) where TKey : class => _syscalls.Invalidate(topic.Name, "");

    private static readonly JsonSerializerOptions KeyJson = new() { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping };

    /// <summary>
    /// A topic key as the platform and the clients match it: the key message's fields by their snake_case names, sorted,
    /// values as strings (<c>{"id":"1048"}</c>); a key with no scalar fields is "" (every key). Takes the key in the
    /// wire's JSON mapping (camelCase); nested, array and null values are left out.
    /// </summary>
    internal static string CanonicalKey(JsonNode? json)
    {
        if (json is not JsonObject o) return "";
        var fields = new List<(string Name, string Value)>();
        foreach (var (k, v) in o)
        {
            if (v is not JsonValue value) continue;
            string? s = value.GetValueKind() switch
            {
                JsonValueKind.String => value.GetValue<string>(),
                JsonValueKind.Number => value.ToJsonString(),
                JsonValueKind.True => "true",
                JsonValueKind.False => "false",
                _ => null,
            };
            if (s is not null) fields.Add((Names.Snake(k), s));
        }
        if (fields.Count == 0) return "";
        fields.Sort((a, b) => string.CompareOrdinal(a.Name, b.Name));
        var m = new JsonObject();
        foreach (var (name, value) in fields) m[name] = value;
        return m.ToJsonString(KeyJson);
    }

    /// <summary>
    /// A syscall; an admission refusal (<c>call_undeclared: …</c>, <c>missing_role: …</c>) becomes 403 with its code,
    /// a refusal of an agent or a notification call (<c>unknown_agent: …</c>, <c>follow_limit: …</c>) its own status and code, any other
    /// error result <see cref="PluginError.Unavailable(string)"/>.
    /// </summary>
    internal async Task<SyscallResult> Syscall(Syscall call)
    {
        var r = await RawSyscall(call);
        if (!string.IsNullOrEmpty(r.Error)) throw SyscallFailed(r.Error, r.ErrorDetails);
        return r;
    }

    private static readonly HashSet<string> Denials = ["call_undeclared", "missing_role", "missing_scope", "reach", "not_a_member"];

    /// <summary>
    /// What the platform refuses an <c>agent_*</c> syscall with, and the status each code answers: an agent or a run the
    /// plugin does not have is 404, a run that ended the platform's <c>run_ended</c> conflict; of a start in a group,
    /// a group the plugin did not found is 404, an agent that is in the group already and a full group are conflicts.
    /// </summary>
    private static readonly Dictionary<string, int> AgentRefusals = new()
    {
        ["unknown_agent"] = 404, ["not_found"] = 404, ["run_ended"] = 409, ["bad_request"] = 400, ["forbidden"] = 403,
        ["unknown_group"] = 404, ["agent_in_group"] = 409, ["group_full"] = 409,
    };

    /// <summary>What the platform refuses a <c>notify_*</c> syscall with besides the db codes (<c>invalid</c>, <c>capability_missing</c>).</summary>
    private static readonly Dictionary<string, int> NotifyRefusals = new() { ["follow_limit"] = 409, ["rate_limited"] = 429 };

    /// <summary>
    /// What the platform refuses a job call with besides the db codes (<c>invalid</c>, <c>not_found</c>): a full queue,
    /// a payload over 64 KiB, and any call of a run that was stopped (<c>cancelled</c>).
    /// </summary>
    private static readonly Dictionary<string, int> JobRefusals = new() { ["queue_full"] = 429, ["payload_too_large"] = 413, ["cancelled"] = 503 };

    /// <summary>
    /// What the platform refuses a <c>Db*</c> or <c>Tx*</c> syscall with, and the status each code answers;
    /// <c>version_conflict</c> carries <c>details.current</c>, <c>unique_conflict</c> <c>details.column</c>.
    /// </summary>
    private static readonly Dictionary<string, int> DbRefusals = new()
    {
        ["not_found"] = 404, ["version_conflict"] = 409, ["unique_conflict"] = 409, ["transaction_conflict"] = 409,
        ["append_only"] = 400, ["invalid"] = 400, ["query_rows_limit"] = 422, ["bulk_limit"] = 422,
        ["transaction_timeout"] = 504, ["permission_denied"] = 403, ["guard_denied"] = 403,
        ["capability_missing"] = 403, ["row_locked"] = 409, ["query_invalid"] = 500, ["query_size_limit"] = 422,
        ["query_join_limit"] = 422, ["query_response_limit"] = 422,
    };

    internal static PluginError SyscallFailed(string error, DbErrorDetails? details = null)
    {
        var i = error.IndexOf(": ", StringComparison.Ordinal);
        if (i > 0 && Denials.Contains(error[..i])) return PluginError.Domain(error[..i], 403, error[(i + 2)..]);
        if (i > 0 && (AgentRefusals.TryGetValue(error[..i], out var status) || NotifyRefusals.TryGetValue(error[..i], out status)
                      || JobRefusals.TryGetValue(error[..i], out status)))
            return new PluginError(status, error[..i], error[(i + 2)..]);
        if (i > 0 && DbRefusals.TryGetValue(error[..i], out var db))
        {
            var code = error[..i];
            JsonObject? d = (code, details) switch
            {
                ("version_conflict", not null) => new JsonObject { ["current"] = details.CurrentVersion },
                ("unique_conflict", not null) => new JsonObject { ["column"] = details.Column },
                _ => null,
            };
            return new PluginError(db, code, error[(i + 2)..], d);
        }
        return PluginError.Unavailable($"platform: {error}");
    }

    internal Task<SyscallResult> RawSyscall(Syscall call) => _syscalls.Call(call, Cancelled);
}

internal static class EventsUnavailable
{
    public static PluginError Error() => PluginError.Unavailable("events are not available");
}

/// <summary>This plugin's id and version.</summary>
public sealed record PluginInfo(string Id, string Version)
{
    /// <summary>The jobs' keys by payload type (null: every payload's job is its name in snake_case).</summary>
    internal IReadOnlyDictionary<Type, string>? JobKeys { get; init; }

    /// <summary>The key of the job that takes <paramref name="payload"/>; 400 <c>invalid</c> when none does.</summary>
    internal string JobFor(Type payload)
    {
        if (JobKeys is null) return Jobs.Key(payload.Name);
        return JobKeys.TryGetValue(payload, out var key)
            ? key
            : throw PluginError.Invalid($"no job of the plugin takes {payload.Name}: declare one with Plugin.Job<..>()");
    }
}

/// <summary>The request being served.</summary>
public sealed class RequestInfo
{
    internal RequestInfo(ulong id, string endpoint, DateTimeOffset deadline, string? idempotencyKey)
    {
        Id = id;
        Endpoint = endpoint;
        Deadline = deadline;
        IdempotencyKey = idempotencyKey;
    }

    /// <summary>The platform's id of the request (dependency calls carry it).</summary>
    public ulong Id { get; }

    /// <summary>The endpoint's name.</summary>
    public string Endpoint { get; }

    /// <summary>When the endpoint's timeout passes.</summary>
    public DateTimeOffset Deadline { get; }

    /// <summary>The <c>Idempotency-Key</c> header, when given.</summary>
    public string? IdempotencyKey { get; }
}

/// <summary>What calls a request.</summary>
public enum CallerKind
{
    /// <summary>No caller (a system call).</summary>
    None,
    Person,
    Service,
}

/// <summary>Who calls a request, resolved by the platform from the workspace's members.</summary>
public sealed class Caller
{
    private readonly string _plugin;

    private Caller(string plugin, Id<Platform.User>? userId, CallerKind kind, string displayName,
        IReadOnlyList<string> roles, IReadOnlyList<Id<Platform.Group>> groups, IReadOnlyList<string> scopes)
    {
        _plugin = plugin;
        UserId = userId;
        Kind = kind;
        DisplayName = displayName;
        Roles = roles;
        Groups = groups;
        Scopes = scopes;
    }

    /// <summary>A caller for unit tests of handlers: a person with this id and no roles.</summary>
    public static Caller TestUser(string userId) =>
        new("", new Id<Platform.User>(userId), CallerKind.Person, "Test User", [], [], []);

    /// <summary>The same caller with these full role names added (<c>workspace.admin</c>, <c>&lt;plugin&gt;.&lt;role&gt;</c>); for tests.</summary>
    public Caller WithRoles(params string[] roles) =>
        new(_plugin, UserId, Kind, DisplayName, [.. Roles, .. roles], Groups, Scopes);

    /// <summary>The same caller with these full scope names added (<c>&lt;plugin&gt;:&lt;scope&gt;</c>); for tests.</summary>
    public Caller WithScopes(params string[] scopes) =>
        new(_plugin, UserId, Kind, DisplayName, Roles, Groups, [.. Scopes, .. scopes]);

    /// <summary>The same caller of this plugin (its roles and scopes read against it).</summary>
    internal Caller For(string plugin) => new(plugin, UserId, Kind, DisplayName, Roles, Groups, Scopes);

    internal Caller(string plugin, Principal? p)
    {
        _plugin = plugin;
        if (p is null) return;
        UserId = string.IsNullOrEmpty(p.UserId) ? null : new Id<Platform.User>(p.UserId);
        Kind = p.Kind switch
        {
            Principal.Types.Kind.Person => CallerKind.Person,
            Principal.Types.Kind.Service => CallerKind.Service,
            _ => CallerKind.None,
        };
        DisplayName = p.DisplayName;
        Roles = [.. p.Roles];
        Groups = [.. p.Groups.Select(g => new Id<Platform.Group>(g))];
        Scopes = [.. p.Scopes];
    }

    /// <summary>The user, or null without one.</summary>
    public Id<Platform.User>? UserId { get; }

    public CallerKind Kind { get; } = CallerKind.None;

    public string DisplayName { get; } = "";

    /// <summary>Effective roles: <c>workspace.*</c> and plugin roles <c>&lt;plugin&gt;.&lt;role&gt;</c>.</summary>
    public IReadOnlyList<string> Roles { get; } = [];

    public IReadOnlyList<Id<Platform.Group>> Groups { get; } = [];

    /// <summary>The token's scopes for this plugin, <c>&lt;plugin&gt;:&lt;scope&gt;</c>.</summary>
    public IReadOnlyList<string> Scopes { get; } = [];

    /// <summary>The user; 401 <c>unauthenticated</c> without one.</summary>
    public Id<Platform.User> User() => UserId ?? throw PluginError.Unauthenticated();

    /// <summary>Holds the role (a plugin role of this plugin, or a <c>workspace.*</c> role).</summary>
    public bool Has(Role role) => Roles.Contains(role.IsPlatform || role.Name.Contains('.') ? role.Name : $"{_plugin}.{role.Name}");

    /// <summary>The token carries this plugin's scope.</summary>
    public bool HasScope(Scope scope) => Scopes.Contains(scope.Name.Contains(':') ? scope.Name : $"{_plugin}:{scope.Name}");

    public override string ToString() => UserId is { } u ? $"{Kind.ToString().ToLowerInvariant()}:{u}" : "(nobody)";
}

/// <summary>A workspace member from the directory.</summary>
public sealed record Member
{
    public required Id<Platform.User> UserId { get; init; }
    public required CallerKind Kind { get; init; }
    public string DisplayName { get; init; } = "";
    public IReadOnlyList<string> Roles { get; init; } = [];
    public IReadOnlyList<Id<Platform.Group>> Groups { get; init; } = [];

    /// <summary>Empty unless the plugin declares the permission <c>members.email</c>.</summary>
    public string Email { get; init; } = "";

    public string AvatarUrl { get; init; } = "";

    /// <summary>Unix seconds, hour resolution.</summary>
    public long LastActive { get; init; }

    public string Locale { get; init; } = "";

    internal static Member From(V1.Member m) => new()
    {
        UserId = new Id<Platform.User>(m.UserId),
        Kind = m.Kind == Principal.Types.Kind.Service ? CallerKind.Service : CallerKind.Person,
        DisplayName = m.DisplayName,
        Roles = [.. m.Roles],
        Groups = [.. m.Groups.Select(g => new Id<Platform.Group>(g))],
        Email = m.Email,
        AvatarUrl = m.AvatarUrl,
        LastActive = m.LastActive,
        Locale = m.Locale,
    };
}

/// <summary>A group of the workspace.</summary>
public sealed record GroupInfo
{
    public required Id<Platform.Group> Id { get; init; }
    public string Name { get; init; } = "";
    public IReadOnlyList<Id<Platform.User>> Members { get; init; } = [];
}

/// <summary>The workspace's members (<c>ctx.Members</c>).</summary>
public readonly struct Members
{
    private const uint PageSize = 500;
    private readonly Ctx _ctx;

    internal Members(Ctx ctx) { _ctx = ctx; }

    /// <summary>All members (pages through the directory).</summary>
    public async Task<IReadOnlyList<Member>> List()
    {
        var all = new List<Member>();
        var token = "";
        while (true)
        {
            var r = await _ctx.Syscall(new Syscall { MembersList = new MembersList { PageToken = token, Limit = PageSize } });
            var page = r.MembersPage ?? throw PluginError.Unavailable("platform: unexpected syscall result");
            all.AddRange(page.Members.Select(Member.From));
            if (string.IsNullOrEmpty(page.NextPageToken)) return all;
            token = page.NextPageToken;
        }
    }

    /// <summary>The member, or null when the user is not a member of the workspace.</summary>
    public async Task<Member?> Get(Id<Platform.User> userId)
    {
        var r = await _ctx.RawSyscall(new Syscall { MemberGet = new MemberGet { UserId = userId.Value } });
        if (r.Error == "not found") return null;
        if (!string.IsNullOrEmpty(r.Error)) throw Ctx.SyscallFailed(r.Error);
        return Member.From(r.Member ?? throw PluginError.Unavailable("platform: unexpected syscall result"));
    }
}

/// <summary>The workspace's groups (<c>ctx.Groups</c>).</summary>
public readonly struct Groups
{
    private const uint PageSize = 500;
    private readonly Ctx _ctx;

    internal Groups(Ctx ctx) { _ctx = ctx; }

    /// <summary>All groups.</summary>
    public async Task<IReadOnlyList<GroupInfo>> List()
    {
        var all = new List<GroupInfo>();
        var token = "";
        while (true)
        {
            var r = await _ctx.Syscall(new Syscall { GroupsList = new GroupsList { PageToken = token, Limit = PageSize } });
            var page = r.GroupsPage ?? throw PluginError.Unavailable("platform: unexpected syscall result");
            all.AddRange(page.Groups.Select(g => new GroupInfo
            {
                Id = new Id<Platform.Group>(g.Id),
                Name = g.Name,
                Members = [.. g.Members.Select(m => new Id<Platform.User>(m))],
            }));
            if (string.IsNullOrEmpty(page.NextPageToken)) return all;
            token = page.NextPageToken;
        }
    }
}

/// <summary>Events: not available, <c>Emit</c> answers <c>unavailable</c>.</summary>
public sealed class Events
{
    internal Events() { }

    public Task Emit(object message) => Task.FromException(EventsUnavailable.Error());
}

