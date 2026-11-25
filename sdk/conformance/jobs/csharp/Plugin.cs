// The SDK conformance plugin `jobsdemo`: background jobs as one small plugin, the same in Rust, TypeScript and C#.
// A task is created and its processing enqueued in one transaction (a request that asks to fail rolls both back);
// a daily report is enqueued once per day (a singleton); a heartbeat runs every minute and logs a few lines; a flaky
// job fails its first attempt and succeeds on the second; a doomed job fails for good; a slow job works in steps of
// a second and stops when it is aborted (its timeout is 10 s). Its `--describe` must equal ../descriptor.json.
using Celobus.Sdk;
using Celobus.Sdk.Linq;

namespace Jobsdemo;

public static class JobsdemoPlugin
{
    public static readonly Role Member = new("member") { Title = "Member", DefaultFor = RoleDefault.Member };

    public static Plugin Define() => Plugin.Create("jobsdemo")
        .Title("Jobs demo")
        .Describe("The SDK conformance plugin of background jobs.")
        .Role(Member)
        .Entity<Task>()
        .Access<Task>(a =>
        {
            a.Read(Member);
            a.Create(Member);
            a.Update(Member);
        })
        .Entity<Stage>()
        .Access<Stage>(a => a.Read(Member))
        .Entity<Estimate>()
        .Access<Estimate>(a => a.Read(Member))
        .Entity<Budget>()
        .Access<Budget>(a =>
        {
            a.Read(Member);
            a.Mask(b => b.Notes, Member);
        })
        .Capability(Capability.AuthzBypass, "The lake's models copy the estimates and the budgets")
        .Bypass<Estimate>(Perm.Read)
        .Bypass<Budget>(Perm.Read)
        .Endpoint<CreateTask>()
        .Endpoint<EnqueueReport>()
        .Endpoint<CancelRun>()
        .Endpoint<LoadTasks>()
        .Endpoint<TaskDays>()
        .Endpoint<RefreshTables>()
        .Endpoint<TaskTrail>()
        .Endpoint<LoadStageDays>()
        .Endpoint<StageDays>()
        .Job<ProcessTaskJob>()
        .Job<DailyReportJob>()
        .Job<HeartbeatJob>()
        .Job<FlakyJob>()
        .Job<DoomedJob>()
        .Job<SlowJob>()
        .Job<TaskBatchJob>()
        .View(RecentTasks)
        .View(TasksByWeek)
        .View(LongEstimates)
        .Layer(LakeRows.Raw)
        .Layer(LakeRows.Mart)
        .Layer(LakeRows.Audit)
        .LakeTable<TaskLoadRow>()
        .LakeReadAsSystem<TaskLoadRow>()
        .LakeTable<TaskDay>()
        .LakeRead<TaskDay>(Member, Member)
        .LakeReadAsSystem<TaskDay>()
        .LakeExport<TaskDay>()
        .LakeExportToModels<TaskDay>()
        .LakeTable<TaskHours>()
        .LakeRead<TaskHours>(Member)
        .Model(LakeRows.TaskHoursModel)
        .LakeTable<IssueWeek>()
        .Model(LakeRows.IssueWeekModel)
        .LakeTable<BudgetCopy>()
        .Model(LakeRows.BudgetCopyModel, unmasked: true)
        .LakeTable<LongEstimateCopy>()
        .Model(LakeRows.LongEstimateCopyModel)
        .LakeTable<OpenByDay>()
        .LakeRead<OpenByDay>(Member)
        .Model(LakeRows.OpenByDayModel)
        .LakeDependency<TrackerIssueDay>()
        .HistoryDependency<Issue>()
        .HistoryRead<Task>(Member)
        .HistoryRead<Task>(Member)
        .HistoryReadAsSystem<Task>()
        .HistoryExport<Task>()
        .HistoryRead<Budget>(Member)
        .Entity<StageDay>()
        .Access<StageDay>(a => a.Read(Member))
        .Load<StageDay>(LakeRows.StageDayLoad);

    /// <summary>A task's day, its title and its stage's title (the stage joined on its name, a plain column).</summary>
    public static readonly IQueryable<RecentTask> RecentTasks =
        from t in Views.From<Task>()
        join s in Views.From<Stage>() on t.Status equals s.Name into stages
        from s in stages.DefaultIfEmpty()
        where Query.CreatedAt(t) >= Query.DaysAgo(30) && t.Title != ""
        select new RecentTask(Query.Day(Query.CreatedAt(t)), t.Title, s!.Title);

    /// <summary>The recent tasks counted by week and stage: a view of a view, grouped.</summary>
    public static readonly IQueryable<TasksByWeek> TasksByWeek =
        from r in Views.From<RecentTask>()
        group r by new { Week = Query.Week(r.Day), r.Stage } into g
        where g.LongCount() > 1
        select new TasksByWeek(g.Key.Week, g.Key.Stage, g.LongCount());

    /// <summary>The estimates over 12.50 hours: a decimal column compared with a decimal (a string in the
    /// declaration).</summary>
    public static readonly IQueryable<LongEstimate> LongEstimates =
        from e in Views.From<Estimate>()
        where e.Hours > 12.50m
        select new LongEstimate(e.Title, e.Hours);
}
