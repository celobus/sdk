using Celobus.Sdk;

namespace Jobsdemo;

// Status: new, then processed
[Entity]
public sealed partial record Task(Id<Task> Id, string Title, string Status);

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

[Message(Kind = MessageKind.Job)] public sealed partial record ProcessTask(Id<Task> Task);
[Message(Kind = MessageKind.Job)] public sealed partial record DailyReport(string Day);
[Message(Kind = MessageKind.Job)] public sealed partial record Heartbeat;
[Message(Kind = MessageKind.Job)] public sealed partial record Flaky;
[Message(Kind = MessageKind.Job)] public sealed partial record Doomed;
// Seconds 0: 30, past the timeout
[Message(Kind = MessageKind.Job)] public sealed partial record Slow(int Seconds = 0);
