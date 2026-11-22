using Celobus.Sdk;

namespace Jobsdemo;

/// <summary>Creates a task and enqueues its processing in one transaction: both exist only if it commits.</summary>
public sealed class CreateTask : Endpoint<CreateTaskRequest, CreateTaskReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Creates a task and enqueues its processing.").Roles(JobsdemoPlugin.Member);

    public override Task<CreateTaskReply> Handle(Ctx ctx, CreateTaskRequest req) => ctx.Transaction(async tx =>
    {
        var task = (await tx.Db<Task>().Create(new Task(default, req.Title, "new"))).Value;
        var run = await tx.Jobs.Enqueue(new ProcessTask(task.Id));
        if (req.Fail) throw PluginError.Invalid("asked to fail: the task and its run are rolled back");
        return new CreateTaskReply(task.Id, run.RunId);
    });
}

/// <summary>Enqueues the day's report once: a second request for the same day answers the first run.</summary>
public sealed class EnqueueReport : Endpoint<EnqueueReportRequest, EnqueueReportReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Enqueues the day's report once.").Roles(JobsdemoPlugin.Member);

    public override async Task<EnqueueReportReply> Handle(Ctx ctx, EnqueueReportRequest req)
    {
        var run = await ctx.Jobs.Enqueue(new DailyReport(req.Day)).Singleton($"report:{req.Day}");
        return new EnqueueReportReply(run.RunId, run.Existing);
    }
}

public sealed class CancelRun : Endpoint<CancelRunRequest, CancelRunReply>
{
    public override void Configure(EndpointOptions e) => e.Describe("Cancels a run.").Roles(JobsdemoPlugin.Member);

    public override async Task<CancelRunReply> Handle(Ctx ctx, CancelRunRequest req) =>
        new(await ctx.Jobs.Cancel(req.RunId));
}
