// jobsdemo's part of the Data Lake, the same in the three SDKs: two layers, a table of tasks as an outside tracker
// sends them (partitioned by the day of the load; its message is named apart from the table), a table of the tasks of
// a day by stage (partitioned and keyed, read by members and by the plugin's system context, exported), and another
// plugin's exported table it reads. `LoadTasks` writes them in each mode, `TaskDays` reads one back grouped, and the
// job `task_batch` appends with the run's default keys. Five tables are computed by models, one of each kind of source:
// the plugin's own table (a text parsed), another plugin's table (grouped by week), an entity (a masked column copied as
// it is), an analytic view, and the history of tasks per day; `RefreshTables` refreshes them, whole or by partition, and
// `TaskTrail` reads the history of tasks. What each SDK sends for them is ../lake.json.
using Celobus.Sdk;
using Celobus.Sdk.Linq;
using Done = System.Threading.Tasks.Task;

namespace Jobsdemo;

// Hours: as the tracker wrote it
[LakeTable(Layer = "raw", Name = "task_load", Partition = [nameof(LoadedOn)]), Describe("Tasks as an outside tracker sends them, a load a day.")]
public sealed partial record TaskLoadRow(DateOnly LoadedOn, string Title, string Hours);

[LakeTable(Layer = "mart", Partition = [nameof(Day)], Key = [nameof(Day), nameof(Stage)]), Describe("The tasks of a day by stage.")]
public sealed partial record TaskDay(DateOnly Day, string Stage, long Tasks, decimal Hours, string? Note);

/// <summary>The tracker's issues by day, a table the plugin <c>tracker</c> exports.</summary>
[LakeTable(Plugin = "tracker", Name = "issue_day")]
public sealed partial record TrackerIssueDay(DateOnly Day, long Issues);

[Entity(Storage = EntityStorage.Olap, OlapKey = [nameof(Day), nameof(Stage)]),
 Describe("The tasks of a day by stage with tasks, kept in the analytic copy only: loaded from `task_day`.")]
public sealed partial record StageDay(Id<StageDay> Id, DateOnly Day, string Stage, long Tasks, decimal? Hours);

// Step: request, status or wait; Ticket: status and wait: the ticket asked about
[Message] public sealed partial record LoadStageDaysRequest(string Step, string Ticket);
// State: pending, building, done or failed; Entities: the entities it loads, Rows: the rows of its first load (0
// before); Code: a failed ticket's first failure
[Message]
public sealed partial record LoadStageDaysReply(string Ticket, string State, long Generation, List<string> Entities, long Rows,
    string Code);

[Message] public sealed partial record StageDaysRequest;
// Rows: the rows the analytic copy answered
[Message] public sealed partial record StageDaysReply(int Rows);

/// <summary>The tracker's issues, whose history the plugin <c>tracker</c> exports.</summary>
[Entity(Plugin = "tracker")]
public sealed partial record Issue(Id<Issue> Id, string Title);

[LakeTable(Layer = "mart", Partition = [nameof(Day)]), Describe("The hours of the loaded tasks, as numbers: computed from `task_load`.")]
public sealed partial record TaskHours(DateOnly Day, string Title, decimal Hours);

[LakeTable(Layer = "mart", Partition = [nameof(Week)]), Describe("The tracker's issues by week: computed from its exported `issue_day`.")]
public sealed partial record IssueWeek(DateOnly Week, long Issues);

[LakeTable(Layer = "mart"), Describe("The budgets with their notes: computed from the entity, its masked column as it is.")]
public sealed partial record BudgetCopy(Id<Budget> Id, string Title, string? Notes);

[LakeTable(Layer = "mart"), Describe("The long estimates: computed from the analytic view.")]
public sealed partial record LongEstimateCopy(string Title, decimal Hours);

[LakeTable(Layer = "mart", Partition = [nameof(Day)]), Describe("The tasks not processed at the end of each day, by status: computed from the history of tasks.")]
public sealed partial record OpenByDay(DateOnly Day, string? Status, long Tasks);

// Table: task_hours, issue_week, budget_copy, long_estimate_copy or open_by_day; Day: only the conformance day's partition
// Every: task_hours: a refresh of partitions naming none (refused)
[Message] public sealed partial record RefreshTablesRequest(string Table, bool Day, bool Every);
// AsOf: when the data read is from, unix milliseconds (0: a model over lake tables); StalePartitions: the partitions
// still holding an earlier declaration's rows
[Message] public sealed partial record RefreshTablesReply(long Rows, long AsOf, long StalePartitions);

// EachDay: one row per day instead of one per change
[Message] public sealed partial record TaskTrailRequest(bool EachDay);
// the first row's change, its author's kind and plugin, its day ("" for none) and status ("" for none); when its version
// was made and ended, unix milliseconds (0: none)
[Message]
public sealed partial record TaskTrailReply(int Rows, string Change, string ChangedByKind, string ChangedByPlugin, string Day,
    string Status, long ValidFrom, long ValidTo);

// Mode: replace, append or merge; Width: the x's after each title; Key: append's write key (none: refused outside a run)
[Message] public sealed partial record LoadTasksRequest(string Mode, int Rows, int Width, string Key);
[Message] public sealed partial record LoadTasksReply(long Rows, bool Skipped);

[Message] public sealed partial record TaskDaysRequest;
// Groups: the groups the lake answered
[Message] public sealed partial record TaskDaysReply(int Groups);

[Message(Kind = MessageKind.Job)] public sealed partial record TaskBatch(int Rows = 0);

public static class LakeRows
{
    public static readonly Layer Raw = new("raw") { Title = "As it arrived" };
    public static readonly Layer Mart = new("mart") { Title = "For reports" };
    public static readonly Layer Audit = new("audit") { Title = "History of records" };

    /// <summary>The days of <c>task_day</c> with tasks.</summary>
    public static readonly IQueryable StageDayLoad =
        from d in LakeModel.From<TaskDay>()
        where d.Tasks > 0
        select new { d.Day, d.Stage, d.Tasks, d.Hours };

    /// <summary>The loads' tasks with a title, their hours parsed.</summary>
    public static readonly IQueryable<TaskHours> TaskHoursModel =
        from l in LakeModel.From<TaskLoadRow>()
        where l.Title != ""
        select new TaskHours(l.LoadedOn, l.Title, Query.Parse<decimal>(l.Hours));

    /// <summary>The tracker's issues summed by week.</summary>
    public static readonly IQueryable<IssueWeek> IssueWeekModel =
        from d in LakeModel.From<TrackerIssueDay>()
        group d by Query.Week(d.Day) into g
        select new IssueWeek(g.Key, g.Sum(x => x.Issues));

    /// <summary>The budgets, their masked notes as they are (declared unmasked).</summary>
    public static readonly IQueryable<BudgetCopy> BudgetCopyModel =
        from b in LakeModel.From<Budget>()
        select new BudgetCopy(b.Id, b.Title, b.Notes);

    /// <summary>The rows of the view <c>long_estimate</c>.</summary>
    public static readonly IQueryable<LongEstimateCopy> LongEstimateCopyModel =
        from e in LakeModel.From<LongEstimate>()
        select new LongEstimateCopy(e.Title, e.Hours);

    /// <summary>The tasks not processed at the end of each day, counted by status.</summary>
    public static readonly IQueryable<OpenByDay> OpenByDayModel =
        from h in LakeModel.History<Task>().EachDay()
        where h.Row.Status != "processed"
        group h by new { h.Day, h.Row.Status } into g
        select new OpenByDay(g.Key.Day!.Value, g.Key.Status, g.LongCount());

    /// <summary>The day every conformance row is of.</summary>
    public static readonly DateOnly Day = new(2026, 11, 20);

    /// <summary><paramref name="n"/> rows of <c>task_load</c>: <c>t&lt;i&gt;</c> followed by <paramref name="width"/> x's, 1.5 hours.</summary>
    public static IEnumerable<TaskLoadRow> Loads(int n, int width)
    {
        var pad = new string('x', Math.Max(0, width));
        for (var i = 0; i < n; i++) yield return new TaskLoadRow(Day, $"t{i}{pad}", "1.5");
    }

    /// <summary><paramref name="n"/> rows of <c>task_day</c>: stage <c>s&lt;i&gt;</c>, <c>i</c> tasks, 2.25 hours, no note.</summary>
    public static IEnumerable<TaskDay> Days(int n)
    {
        for (var i = 0; i < n; i++) yield return new TaskDay(Day, $"s{i}", i, 2.25m, null);
    }

    /// <summary>The conformance lake query: the tasks and hours of November 2026 with tasks, by month and stage.</summary>
    public static IQueryable<TaskDaysRow> TaskDays(Ctx ctx) =>
        (from d in ctx.Lake().Query<TaskDay>()
         where Query.Between(d.Day, new DateOnly(2026, 11, 1), new DateOnly(2026, 11, 30)) && d.Tasks > 0
         group d by new { Month = Query.Month(d.Day), d.Stage } into g
         orderby g.Key.Stage
         select new TaskDaysRow(g.Key.Month, g.Key.Stage, g.Sum(x => x.Tasks), g.Sum(x => x.Hours)))
        .Take(50);

    /// <summary>A group of the conformance lake query.</summary>
    public sealed record TaskDaysRow(DateOnly Month, string Stage, long Tasks, decimal Hours);
}

/// <summary>Writes generated rows in one write: replace and append into <c>task_load</c>, merge into <c>task_day</c>.</summary>
public sealed class LoadTasks : Endpoint<LoadTasksRequest, LoadTasksReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Writes generated rows to the lake.").Roles(JobsdemoPlugin.Member);

    public override async Task<LoadTasksReply> Handle(Ctx ctx, LoadTasksRequest req)
    {
        var lake = ctx.Lake();
        var written = req.Mode switch
        {
            "replace" => await lake.Table<TaskLoadRow>()
                .ReplaceAsync(Filter<TaskLoadRow>.Eq(x => x.LoadedOn, LakeRows.Day), LakeRows.Loads(req.Rows, req.Width)),
            "append" => await lake.Table<TaskLoadRow>().AppendAsync(LakeRows.Loads(req.Rows, req.Width), req.Key),
            "merge" => await lake.Table<TaskDay>().MergeAsync(LakeRows.Days(req.Rows)),
            _ => throw PluginError.Invalid($"{req.Mode} is not replace, append or merge"),
        };
        return new LoadTasksReply(written.Rows, written.Skipped);
    }
}

public sealed class TaskDays : Endpoint<TaskDaysRequest, TaskDaysReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Reads the tasks of the month back from the lake.").Roles(JobsdemoPlugin.Member);

    public override async Task<TaskDaysReply> Handle(Ctx ctx, TaskDaysRequest req) =>
        new((await LakeRows.TaskDays(ctx).ToListAsync()).Count);
}

/// <summary>
/// Appends a batch three times: with the run's default key, with a key of its own, and from the system context with the
/// next default key (<c>&lt;run&gt;#1</c>, <c>batch-&lt;run&gt;</c>, <c>&lt;run&gt;#3</c>): an attempt run again appends
/// nothing twice.
/// </summary>
public sealed class TaskBatchJob : Job<TaskBatch>
{
    public override void Configure(JobOptions j) => j.Describe("Appends a batch of tasks to the lake");

    public override async Done Run(Ctx ctx, TaskBatch p)
    {
        var table = ctx.Lake().Table<TaskLoadRow>();
        await table.AppendAsync(LakeRows.Loads(p.Rows, 0));
        await table.AppendAsync(LakeRows.Loads(p.Rows, 0), $"batch-{ctx.Job.RunId}");
        await ctx.AsSystem("the batch's last part").Lake().Table<TaskLoadRow>().AppendAsync(LakeRows.Loads(p.Rows, 0));
    }
}

/// <summary>Refreshes one of the computed tables, whole or the conformance day's partition.</summary>
public sealed class RefreshTables : Endpoint<RefreshTablesRequest, RefreshTablesReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Refreshes a table computed by a model.").Roles(JobsdemoPlugin.Member);

    public override async Task<RefreshTablesReply> Handle(Ctx ctx, RefreshTablesRequest req)
    {
        var lake = ctx.Lake();
        var done = (req.Table, req.Day) switch
        {
            ("task_hours", _) when req.Every => await lake.RefreshAsync(Filter<TaskHours>.All),
            ("task_hours", false) => await lake.RefreshAsync<TaskHours>(),
            ("task_hours", true) => await lake.RefreshAsync(Filter<TaskHours>.Eq(x => x.Day, LakeRows.Day)),
            ("issue_week", false) => await lake.RefreshAsync<IssueWeek>(),
            ("issue_week", true) => await lake.RefreshAsync(Filter<IssueWeek>.Eq(x => x.Week, LakeRows.Day)),
            ("budget_copy", _) => await lake.RefreshAsync<BudgetCopy>(),
            ("long_estimate_copy", _) => await lake.RefreshAsync<LongEstimateCopy>(),
            ("open_by_day", false) => await lake.RefreshAsync<OpenByDay>(),
            ("open_by_day", true) => await lake.RefreshAsync(Filter<OpenByDay>.Eq(x => x.Day, LakeRows.Day)),
            _ => throw PluginError.Invalid($"{req.Table} has no model"),
        };
        return new RefreshTablesReply(done.Rows, done.AsOf?.ToUnixTimeMilliseconds() ?? 0, done.StalePartitions);
    }
}

/// <summary>
/// The conformance history read: the changes of tasks whose status is not empty, by when they were made (or, per day,
/// by day), at most 20.
/// </summary>
public sealed class TaskTrail : Endpoint<TaskTrailRequest, TaskTrailReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Reads the history of tasks.").Roles(JobsdemoPlugin.Member);

    public override async Task<TaskTrailReply> Handle(Ctx ctx, TaskTrailRequest req)
    {
        var rows = req.EachDay
            ? await (from h in ctx.Lake().History<Task>().EachDay() where h.Row.Status != "" orderby h.Day select h).Take(20).ToListAsync()
            : await (from h in ctx.Lake().History<Task>() where h.Row.Status != "" orderby h.ValidFrom select h).Take(20).ToListAsync();
        if (rows.Count == 0) return new TaskTrailReply(0, "", "", "", "", "", 0, 0);
        var first = rows[0];
        return new TaskTrailReply(rows.Count, first.Change, first.ChangedByKind, first.ChangedByPlugin ?? "",
            first.Day?.ToString("yyyy-MM-dd", System.Globalization.CultureInfo.InvariantCulture) ?? "",
            first.Has(t => t.Status) ? first.Row.Status : "",
            first.ValidFrom.ToUnixTimeMilliseconds(), first.ValidTo?.ToUnixTimeMilliseconds() ?? 0);
    }
}

/// <summary>Asks for a snapshot that loads <c>stage_day</c>, asks how its ticket stands, or waits for its end.</summary>
public sealed class LoadStageDays : Endpoint<LoadStageDaysRequest, LoadStageDaysReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Asks for the load of the days by stage, or follows its ticket.").Roles(JobsdemoPlugin.Member);

    private static LoadStageDaysReply Reply(LoadTicketStatus t) =>
        new(t.Id, t.State.ToString().ToLowerInvariant(), t.Generation, [.. t.Entities], t.Loads.FirstOrDefault()?.Rows ?? 0, t.Code);

    public override async Task<LoadStageDaysReply> Handle(Ctx ctx, LoadStageDaysRequest req) => req.Step switch
    {
        "request" => Reply((await ctx.Analytics().Snapshot().Load<StageDay>().RequestAsync()).Last),
        "status" => Reply(await LoadTicket.Of(ctx, req.Ticket).StatusAsync()),
        "wait" => Reply(await LoadTicket.Of(ctx, req.Ticket).WaitAsync()),
        _ => throw PluginError.Invalid($"{req.Step} is not request, status or wait"),
    };
}

/// <summary>Reads the loaded days of <c>stage_day</c> from the analytic copy.</summary>
public sealed class StageDays : Endpoint<StageDaysRequest, StageDaysReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Reads the loaded days by stage.").Roles(JobsdemoPlugin.Member);

    public override async Task<StageDaysReply> Handle(Ctx ctx, StageDaysRequest req) =>
        new((await (from s in ctx.Analytics().Query<StageDay>()
                    where s.Tasks > 0
                    orderby s.Day
                    select new { s.Day, s.Stage, s.Tasks, s.Hours }).ToListAsync()).Count);
}
