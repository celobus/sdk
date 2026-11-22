// The SDK conformance plugin `jobsdemo`: background jobs as one small plugin, the same in Rust, TypeScript and C#.
// A task is created and its processing enqueued in one transaction (a request that asks to fail rolls both back);
// a daily report is enqueued once per day (a singleton); a heartbeat runs every minute and logs a few lines; a flaky
// job fails its first attempt and succeeds on the second; a doomed job fails for good; a slow job works in steps of
// a second and stops when it is aborted (its timeout is 10 s). Its `--describe` must equal ../descriptor.json.
using Celobus.Sdk;

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
        .Endpoint<CreateTask>()
        .Endpoint<EnqueueReport>()
        .Endpoint<CancelRun>()
        .Job<ProcessTaskJob>()
        .Job<DailyReportJob>()
        .Job<HeartbeatJob>()
        .Job<FlakyJob>()
        .Job<DoomedJob>()
        .Job<SlowJob>();
}
