using System.Text;
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

/// <summary>Keeps a document with its files and enqueues the import of its first file.</summary>
public sealed class AddDocument : Endpoint<AddDocumentRequest, AddDocumentReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Keeps a document with its files.").Roles(JobsdemoPlugin.Member);

    public override async Task<AddDocumentReply> Handle(Ctx ctx, AddDocumentRequest req)
    {
        var doc = (await ctx.Db<Document>().Create(new Document(default, req.Title, req.Files))).Value;
        var runId = req.Files.Count > 0 ? (await ctx.Jobs.Enqueue(new ImportFile(req.Files[0]))).RunId : "";
        return new AddDocumentReply(doc.Id, runId);
    }
}

/// <summary>Writes the day's export and answers it: the caller downloads it.</summary>
public sealed class ExportTasks : Endpoint<ExportTasksRequest, ExportTasksReply>
{
    public override void Configure(EndpointOptions e) =>
        e.Describe("Exports the day's tasks as a file.").Roles(JobsdemoPlugin.Member);

    public override async Task<ExportTasksReply> Handle(Ctx ctx, ExportTasksRequest req)
    {
        await using var w = await ctx.Files.Create($"tasks-{req.Day}.csv", "text/csv");
        await w.WriteAsync("day,title\n"u8.ToArray());
        await w.WriteAsync(Encoding.UTF8.GetBytes($"{req.Day},exported\n"));
        return new ExportTasksReply(await w.Finish());
    }
}
