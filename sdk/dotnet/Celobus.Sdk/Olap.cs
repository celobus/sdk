// Entities stored in OLAP: an entity whose rows are kept in the workspace's analytic copy only, filled by its load (a
// query over lake tables and histories) when the plugin asks for a snapshot of the copy that loads it.
//
//   [Entity(Storage = EntityStorage.Olap, OlapKey = [nameof(Day), nameof(Store), nameof(Metric)])]
//   public sealed record StoreMetric(Id<StoreMetric> Id, DateOnly Day, Id<Store> Store, string Metric, decimal? Value);
//
//   Plugin.Create("retail")…
//       .Entity<StoreMetric>().Access<StoreMetric>(a => a.Read(Analyst))
//       .Load<StoreMetric>(from m in LakeModel.From<StoreMetricsMart>()
//                          where m.Day >= Query.DaysAgo(400)
//                          select new { m.Day, Store = m.StoreId, m.Metric, Value = Query.Parse<decimal>(m.Value) });
//
//   // the scheduled job, last:
//   var ticket = await ctx.Analytics().Snapshot().Load<StoreMetric>().RequestAsync();   // answers at once
//   await ticket.WaitAsync();                                   // optional: throws the load's error
//   var rows = await ctx.Analytics().Query<StoreMetric>().Where(..).ToListAsync();
//
// Reading it. Only ctx.Analytics().Query<T>() (and the plugin's analytic views and models that name it):
// ctx.Db<T>() and ctx.Query<T>() are refused (CELOBUS061 at compile time). Before its first load a query of it is 409
// `not_loaded`, never an empty answer; so it is again after the analytic copy was made anew (its loaded rows are gone
// until the plugin asks again), a reinstall, a changed load or entity, or a withdrawn consent of a lake table's owner.
// So request loads from a scheduled job, not once at install.
//
// The request answers a LoadTicket at once; the snapshot is made in the background. A request while the plugin has one
// waiting is merged into it; snapshots with loads are at least 10 minutes apart (requests in between wait and share
// the next); a workspace makes at most 48 a day and a plugin's loads are in at most 12 of them (429
// `analytic_load_limit` past either, or past 30 requests a minute). StatusAsync() asks now, WaitAsync() waits for its
// end: Done once the plugin's analytic reads answer from the snapshot that loaded it (a read right after the wait sees
// the rows), Failed with the load's error: 422 `analytic_load_rows_limit` (over 1,000,000 rows or 1 GiB),
// `analytic_load_time_limit` (over 50 s), `analytic_load_duplicate` (a key twice), `analytic_load_invalid` (a row its
// entity cannot hold), 503 `analytic_load_failed` (the snapshot could not be made). A plugin waits on at most 16
// tickets at once (503 `analytic_load_busy`: retry).
using Celobus.V1;

namespace Celobus.Sdk;

/// <summary>Where a ticket stands.</summary>
public enum LoadTicketState
{
    /// <summary>Waiting for the next snapshot.</summary>
    Pending,

    /// <summary>The snapshot that runs its loads is being made or adopted.</summary>
    Building,

    /// <summary>The snapshot that loaded every entity it names is the workspace's, and the plugin's analytic reads answer from it.</summary>
    Done,

    /// <summary>A load failed (the others of the snapshot stand), or the snapshot could not be made.</summary>
    Failed,
}

/// <summary>One entity's load in a snapshot: its rows and when they were read from the lake, or why it failed (<see cref="Code"/>).</summary>
public sealed record Loaded(string Entity, long Rows, DateTimeOffset? LoadedAt, string Code, string Error);

/// <summary>A ticket as the platform last answered it.</summary>
public sealed record LoadTicketStatus(
    string Id,
    LoadTicketState State,
    long Generation,
    IReadOnlyList<string> Entities,
    IReadOnlyList<Loaded> Loads,
    string Code,
    string Error,
    DateTimeOffset? RequestedAt,
    DateTimeOffset? FinishedAt,
    DateTimeOffset? NotBefore = null)
{
    private static DateTimeOffset? At(long ms) => ms > 0 ? DateTimeOffset.FromUnixTimeMilliseconds(ms) : null;

    internal static LoadTicketStatus Of(AnalyticsTicket t) => new(
        t.Ticket,
        t.State switch
        {
            AnalyticsTicket.Types.State.Building => LoadTicketState.Building,
            AnalyticsTicket.Types.State.Done => LoadTicketState.Done,
            AnalyticsTicket.Types.State.Failed => LoadTicketState.Failed,
            _ => LoadTicketState.Pending,
        },
        (long)t.Generation,
        [.. t.Entities],
        [.. t.Loads.Select(l => new Loaded(l.Entity, (long)l.Rows, At(l.LoadedAt), l.Code, l.Error))],
        t.Code,
        t.Error,
        At(t.RequestedAt),
        At(t.FinishedAt),
        At(t.NotBefore));

    /// <summary>
    /// A wait that the request's deadline ended before the ticket did: 503 <c>analytic_load_pending</c> (retry later;
    /// the ticket goes on), its details the ticket's id, state and <c>not_before</c> (unix milliseconds).
    /// </summary>
    public PluginError Pending() => new(503, "analytic_load_pending",
        "the analytic data is still being prepared: the snapshot that loads it has not been made yet",
        new System.Text.Json.Nodes.JsonObject
        {
            ["ticket"] = Id,
            ["state"] = State.ToString().ToLowerInvariant(),
            ["not_before"] = NotBefore?.ToUnixTimeMilliseconds() ?? 0,
        });

    /// <summary>A failed ticket's failure as an error: its code (<c>analytic_load_failed</c> when it has none), and that code's status.</summary>
    public PluginError? Failure()
    {
        if (State != LoadTicketState.Failed) return null;
        var code = Code.Length == 0 ? "analytic_load_failed" : Code;
        var status = code switch
        {
            "analytic_load_rows_limit" or "analytic_load_time_limit" or "analytic_load_duplicate" or "analytic_load_invalid" => 422,
            "permission_denied" or "capability_missing" => 403,
            "analytic_load_failed" or "unavailable" => 503,
            _ => 500,
        };
        return new PluginError(status, code, Error);
    }
}

/// <summary>
/// A request for a snapshot with loads (<see cref="Snapshot.RequestAsync"/>): <see cref="Last"/> as answered,
/// <see cref="StatusAsync"/> to ask again, <see cref="WaitAsync"/> to wait for its end. <see cref="Of"/> follows one
/// another call was given.
/// </summary>
public sealed class LoadTicket
{
    private readonly Ctx _ctx;

    internal LoadTicket(Ctx ctx, LoadTicketStatus status)
    {
        _ctx = ctx;
        Last = status;
    }

    /// <summary>A ticket of the plugin by its id (one a request answered earlier, perhaps in another call).</summary>
    public static LoadTicket Of(Ctx ctx, string id) => new(ctx, new LoadTicketStatus(id, LoadTicketState.Pending, 0, [], [], "", "", null, null));

    /// <summary>The ticket's id (another call may follow it: <see cref="Of"/>).</summary>
    public string Id => Last.Id;

    /// <summary>The ticket as last answered.</summary>
    public LoadTicketStatus Last { get; private set; }

    private async Task<LoadTicketStatus> Ask(uint waitMs)
    {
        var r = await _ctx.Syscall(new Syscall
        {
            AnalyticsLoadStatus = new AnalyticsLoadStatus { RequestId = _ctx.Request.Id, Ticket = Last.Id, WaitMs = waitMs },
        });
        Last = LoadTicketStatus.Of(r.AnalyticsTicket ?? throw PluginError.Unavailable("platform: a ticket's state was answered with something else"));
        return Last;
    }

    /// <summary>The ticket's state now.</summary>
    public Task<LoadTicketStatus> StatusAsync() => Ask(0);

    /// <summary>How long before a request's deadline a ticket's wait gives up, to answer in time.</summary>
    private static readonly TimeSpan WaitMargin = TimeSpan.FromSeconds(2);

    /// <summary>
    /// Waits until the ticket ends: its state when every load ran, the failure's <see cref="PluginError"/> otherwise
    /// (the other loads of the snapshot stand). A request made within ten minutes of the last snapshot with loads waits
    /// for the next one: in an endpoint, answer the ticket and let the caller ask later. In an endpoint the wait ends
    /// before the request's deadline: 503 <c>analytic_load_pending</c> (<see cref="LoadTicketStatus.Pending"/>) when
    /// the ticket has not ended by then; in a job's run it waits within the run's timeout.
    /// </summary>
    public async Task<LoadTicketStatus> WaitAsync()
    {
        while (true)
        {
            var waitMs = 30_000u;
            if (!_ctx.Job.IsRun)
            {
                // the whole seconds left before the request's deadline, less a margin to answer in
                var left = Math.Floor(((_ctx.Request.Deadline - DateTimeOffset.UtcNow - WaitMargin).TotalMilliseconds - 1) / 1000) * 1000;
                if (left < 1000) throw Last.Pending();
                waitMs = (uint)Math.Min(30_000, left);
            }
            var t = await Ask(waitMs);
            if (t.State == LoadTicketState.Done) return t;
            if (t.State == LoadTicketState.Failed) throw t.Failure()!;
        }
    }
}

/// <summary>
/// A snapshot of the workspace's analytic copy the plugin asks for (<c>ctx.Analytics().Snapshot()</c>):
/// <see cref="Load{T}"/> names its entities stored in OLAP whose loads it runs, <see cref="RequestAsync"/> asks.
/// </summary>
public sealed class Snapshot
{
    private readonly Ctx _ctx;
    private readonly List<string> _entities = [];
    private string? _error;

    internal Snapshot(Ctx ctx) => _ctx = ctx;

    /// <summary>The snapshot runs the load of <typeparamref name="T"/>, one of the plugin's entities stored in OLAP.</summary>
    public Snapshot Load<T>() where T : class
    {
        var name = Names.Snake(typeof(T).Name);
        if (!Schema.IsOlap(typeof(T)) || Schema.ForeignPlugin(typeof(T)) is not null)
            _error ??= $"{name} is not one of the plugin's entities stored in OLAP: only those are loaded";
        else if (!_entities.Contains(name)) _entities.Add(name);
        return this;
    }

    /// <summary>
    /// Asks for the snapshot: answers at once with its <see cref="LoadTicket"/> (a request while the plugin has one waiting
    /// is merged into that one). Between two snapshots that load an entity its rows do not change, whatever happens in
    /// the lake.
    /// </summary>
    public async Task<LoadTicket> RequestAsync()
    {
        if (_error is not null) throw PluginError.Invalid(_error);
        if (_entities.Count == 0) throw PluginError.Invalid("a snapshot names the entities it loads: .Load<T>()");
        var r = await _ctx.Syscall(new Syscall
        {
            AnalyticsLoad = new AnalyticsLoad { RequestId = _ctx.Request.Id, Entities = { _entities } },
        });
        return new LoadTicket(_ctx, LoadTicketStatus.Of(r.AnalyticsTicket ?? throw PluginError.Unavailable("platform: a snapshot's request was answered with something else")));
    }
}
