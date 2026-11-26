using Celobus.Sdk;

namespace Jobsdemo;

// Status: new, then processed. Its history keeps the changes of its status
[Entity, History("audit")]
public sealed partial record Task(Id<Task> Id, string Title, [History] string Status);

// A stage of the work: the title of a task's status (`new`, `processed`)
[Entity]
public sealed partial record Stage(Id<Stage> Id, string Name, string Title);

// An estimate of some work, in hours (a decimal)
[Entity]
public sealed partial record Estimate(Id<Estimate> Id, string Title, decimal Hours);

// The hours planned for some work: its history keeps every column but the list, the encrypted and the masked one
[Entity, History("audit", All = true)]
public sealed partial record Budget(Id<Budget> Id, string Title, decimal? Hours, List<string> Tags, [Encrypted] string? Secret, string? Notes);

// A document and the files it was given
[Entity]
public sealed partial record Document(Id<Document> Id, string Title, List<File> Files);

[View(Name = "recent_task"), Describe("The titled tasks of the last 30 days, a row each, with the title of their stage.")]
public sealed partial record RecentTask(DateOnly Day, string Title, string? Stage);

[View, Describe("The recent tasks by week and stage, where a stage had more than one.")]
public sealed partial record TasksByWeek(DateOnly Week, string? Stage, long Tasks);

[View, Describe("The estimates of more than 12.50 hours.")]
public sealed partial record LongEstimate(string Title, decimal Hours);

// Fail true: the request fails after the enqueue, and the task and its run roll back together
[Message] public sealed partial record CreateTaskRequest(string Title, bool Fail);
[Message] public sealed partial record CreateTaskReply(Id<Task> Task, string RunId);

// Day: 2026-11-20
[Message] public sealed partial record EnqueueReportRequest(string Day);
// Existing: the day's report was enqueued before
[Message] public sealed partial record EnqueueReportReply(string RunId, bool Existing);

[Message] public sealed partial record CancelRunRequest(string RunId);
// Cancelled false: the run had already ended
[Message] public sealed partial record CancelRunReply(bool Cancelled);

[Message]
public sealed partial record AddDocumentRequest(string Title, [MaxSize("1 GiB"), Accept("text/csv", "image/*")] List<File> Files);

// RunId: the import of its first file; empty: it has none
[Message] public sealed partial record AddDocumentReply(Id<Document> Document, string RunId);

// Day: 2026-11-20
[Message] public sealed partial record ExportTasksRequest(string Day);
[Message] public sealed partial record ExportTasksReply(File File);

[Message(Kind = MessageKind.Job)] public sealed partial record ProcessTask(Id<Task> Task);
[Message(Kind = MessageKind.Job)] public sealed partial record ImportFile([Accept("text/csv")] File File);
[Message(Kind = MessageKind.Job)] public sealed partial record DailyReport(string Day);
[Message(Kind = MessageKind.Job)] public sealed partial record Heartbeat;
[Message(Kind = MessageKind.Job)] public sealed partial record Flaky;
[Message(Kind = MessageKind.Job)] public sealed partial record Doomed;
// Seconds 0: 30, past the timeout
[Message(Kind = MessageKind.Job)] public sealed partial record Slow(int Seconds = 0);
