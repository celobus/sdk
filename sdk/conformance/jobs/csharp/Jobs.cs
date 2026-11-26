using System.Globalization;
using Celobus.Sdk;
using Microsoft.Extensions.Logging;
// the entity `Task` of this plugin takes the short name
using Done = System.Threading.Tasks.Task;

namespace Jobsdemo;

public sealed class ProcessTaskJob : Job<ProcessTask>
{
    public override void Configure(JobOptions j) => j.Describe("Marks a task processed");

    public override async Done Run(Ctx ctx, ProcessTask p)
    {
        var task = await ctx.Db<Task>().Get(p.Task);
        ctx.Log.LogInformation("processing task {Title}", task.Value.Title);
        await ctx.Db<Task>().Update(p.Task, Patch<Task>.Set(x => x.Status, "processed"));
    }
}

/// <summary>Reads the file line by line, never whole.</summary>
public sealed class ImportFileJob : Job<ImportFile>
{
    public override void Configure(JobOptions j) => j.Describe("Imports a file line by line");

    public override async Done Run(Ctx ctx, ImportFile p)
    {
        using var reader = new StreamReader(await ctx.Files.Open(p.File));
        var n = 0;
        while (await reader.ReadLineAsync(ctx.Cancelled) is { } line)
            if (line.Length > 0) n++;
        ctx.Log.LogInformation("imported {Lines} lines of {Name}", n, p.File.Name);
    }
}

public sealed class DailyReportJob : Job<DailyReport>
{
    public override void Configure(JobOptions j) => j.Describe("Writes the day's report");

    public override Done Run(Ctx ctx, DailyReport p)
    {
        ctx.Log.LogInformation("report for {Day} (run {RunId})", p.Day, ctx.Job.RunId);
        return Done.CompletedTask;
    }
}

public sealed class HeartbeatJob : Job<Heartbeat>
{
    public override void Configure(JobOptions j) =>
        j.Describe("Logs a heartbeat every minute").Cron("* * * * *").AsSystem().Retry(Retry.None);

    public override Done Run(Ctx ctx, Heartbeat p)
    {
        ctx.Log.LogInformation("heartbeat at {At}", ctx.Job.ScheduledAt.ToString("O", CultureInfo.InvariantCulture));
        ctx.Log.LogInformation("attempt {Attempt} of {MaxAttempts}", ctx.Job.Attempt, ctx.Job.MaxAttempts);
        ctx.Log.LogWarning("nothing to do");
        return Done.CompletedTask;
    }
}

public sealed class FlakyJob : Job<Flaky>
{
    public override void Configure(JobOptions j) =>
        j.Describe("Fails its first attempt").Retry(Retry.Fixed(2, TimeSpan.FromSeconds(5)));

    public override Done Run(Ctx ctx, Flaky p)
    {
        if (ctx.Job.Attempt == 1)
        {
            ctx.Log.LogWarning("first attempt: failing");
            throw PluginError.Unavailable("the first attempt always fails");
        }
        ctx.Log.LogInformation("attempt {Attempt}: done", ctx.Job.Attempt);
        return Done.CompletedTask;
    }
}

public sealed class DoomedJob : Job<Doomed>
{
    public override void Configure(JobOptions j) =>
        j.Describe("Fails for good").Retry(Retry.Exponential(3, TimeSpan.FromSeconds(10)));

    public override Done Run(Ctx ctx, Doomed p)
    {
        ctx.Log.LogError("giving up");
        throw PluginError.Permanent("this job always fails for good");
    }
}

public sealed class SlowJob : Job<Slow>
{
    public override void Configure(JobOptions j) =>
        j.Describe("Works in steps until it is stopped").Timeout(TimeSpan.FromSeconds(10)).Concurrency(2);

    public override async Done Run(Ctx ctx, Slow p)
    {
        var seconds = p.Seconds > 0 ? p.Seconds : 30;
        for (var step = 1; step <= seconds; step++)
        {
            // an abort stops the handler at this wait
            await Done.Delay(TimeSpan.FromSeconds(1), ctx.Cancelled);
            ctx.Log.LogInformation("step {Step} of {Seconds}", step, seconds);
        }
    }
}
