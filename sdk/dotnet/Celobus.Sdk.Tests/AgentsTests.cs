using Celobus.V1;
using Google.Protobuf;
using Microsoft.Extensions.Logging.Abstractions;

namespace Celobus.Sdk.Tests;

/// <summary><c>ctx.Agents</c>: the runs of the plugin's agents, against syscalls answered in memory.</summary>
public class AgentsTests
{
    private static V1.AgentRun WireRun(string id, string status) => new()
    {
        Id = id,
        Agent = "assistant",
        Plugin = "tests",
        Engine = "claude",
        Status = status,
        Input = "what does tea cost?",
        Result = status == "succeeded" ? "Tea is $4." : "",
        Error = status == "failed" ? new V1.AgentRunError { Code = "lost", Message = "the agent stopped answering" } : null,
        StartedBy = new AgentRunActor { UserId = "u-1", DisplayName = "Sarah Bennett" },
        StartedByPlugin = "tests",
        CreatedAt = 1_000,
        UpdatedAt = 2_000,
        DeadlineAt = 1_801_000,
        Turn = 1,
        LastSeq = 3,
        Pod = "running",
        Group = id,
    };

    /// <summary>The context of request 77 over these syscalls.</summary>
    private static Ctx Context(FakeSyscalls syscalls) => new(
        syscalls, new PluginInfo("tests", "1.0.0"),
        new RequestInfo(77, "Ask", DateTimeOffset.UtcNow.AddSeconds(30), null),
        new Caller("tests", null), NullLogger.Instance, CancellationToken.None);

    private static readonly FakeSyscalls Cell = new(call => call.CallCase switch
    {
        Syscall.CallOneofCase.AgentRun => new SyscallResult { AgentRun = WireRun("run-1", "starting") },
        Syscall.CallOneofCase.AgentGet => new SyscallResult { AgentRun = WireRun(call.AgentGet.RunId, "succeeded") },
        Syscall.CallOneofCase.AgentCancel => new SyscallResult { AgentRun = WireRun(call.AgentCancel.RunId, "cancelled") },
        Syscall.CallOneofCase.AgentClose => new SyscallResult { AgentRun = WireRun(call.AgentClose.RunId, "idle") },
        Syscall.CallOneofCase.AgentSend => new SyscallResult { AgentInput = new AgentRunInput { Seq = 2 } },
        Syscall.CallOneofCase.AgentRuns => new SyscallResult
        {
            AgentRuns = new AgentRunsPage { Runs = { WireRun("run-2", "running"), WireRun("run-1", "failed") } },
        },
        Syscall.CallOneofCase.AgentEvents => new SyscallResult
        {
            AgentEvents = new AgentRunEventsPage
            {
                Run = WireRun(call.AgentEvents.RunId, "running"),
                Events =
                {
                    new AgentRunEvent
                    {
                        Seq = 3, At = 1_500, Type = "tool",
                        Json = ByteString.CopyFromUtf8("""{"seq":3,"at":1500,"type":"tool","name":"shell","phase":"completed","summary":"celobus whoami"}"""),
                    },
                },
                Partial = new AgentRunPartial
                {
                    Rev = 4,
                    Messages =
                    {
                        new AgentRunPartialMessage { MessageId = "m1", Text = "Tea" },
                        new AgentRunPartialMessage { MessageId = "m2", Text = "is $4" },
                    },
                },
            },
        },
        _ => new SyscallResult { Error = "unsupported" },
    });

    [Fact]
    public async Task StartsARunAsTheRequestsCaller()
    {
        var syscalls = new FakeSyscalls(_ => new SyscallResult { AgentRun = WireRun("run-1", "starting") });
        var agents = Context(syscalls).Agents;

        var run = await agents.Run("assistant", "what does tea cost?");
        Assert.Equal(("run-1", AgentRunStatus.Starting, false), (run.Id, run.Status, run.Ended));
        Assert.Equal(("assistant", "tests", "claude"), (run.Agent, run.Plugin, run.Engine));
        Assert.Equal(new AgentActor(new Id<Platform.User>("u-1"), "Sarah Bennett"), run.StartedBy);
        Assert.Equal("tests", run.StartedByPlugin);
        Assert.Null(run.Model);
        Assert.Null(run.Result);
        Assert.Null(run.Error);
        Assert.Equal(DateTimeOffset.FromUnixTimeMilliseconds(1_000), run.CreatedAt);
        Assert.Equal((1, 3L, "running"), (run.Turn, run.LastSeq, run.Pod));
        // a run a person started founds its group; with no one named at the root, it is who started it
        Assert.Equal(("run-1", null, null), (run.Group, run.Parent, run.ParentAgent));
        Assert.Equal(run.StartedBy, run.OnBehalfOf);

        await agents.Run("assistant", "and coffee?", new AgentRunOptions
        {
            Engine = AgentEngine.Pi, Model = "claude-opus-5-5", Session = false, Timeout = TimeSpan.FromMinutes(10),
        });

        // what went to the platform: the request's id; what a start does not say is left to the agent's declaration
        var first = syscalls.Calls[0].AgentRun;
        Assert.Equal(("assistant", "what does tea cost?", 77ul), (first.Agent, first.Input, first.RequestId));
        Assert.Equal(("", "", false, 0u, ""), (first.Engine, first.Model, first.HasSession, first.TimeoutS, first.Group));
        Assert.Equal("", first.Topic);
        var second = syscalls.Calls[1].AgentRun;
        Assert.Equal(("pi", "claude-opus-5-5", 600u), (second.Engine, second.Model, second.TimeoutS));
        Assert.True(second.HasSession);
        Assert.False(second.Session);
        Assert.Equal("", second.Group);
        // unsaid: only a failure tells, and it opens the run
        Assert.Equal((false, null), (second.Notify, second.NotifyLink));
    }

    [Fact]
    public async Task AStartAsksToBeToldWhenTheRunFinishes()
    {
        var syscalls = new FakeSyscalls(_ => new SyscallResult { AgentRun = WireRun("run-1", "starting") });
        await Context(syscalls).Agents.Run("assistant", "pack order 7", new AgentRunOptions
        {
            Notify = true,
            NotifyLink = NotifyLink.Screen(RunScreen, new RunParams { Run = "run-1" }),
        });
        var start = syscalls.Calls[0].AgentRun;
        Assert.True(start.Notify);
        Assert.Equal(("run", "", """{"run":"run-1"}"""), (start.NotifyLink.Screen, start.NotifyLink.Overlay, start.NotifyLink.Args));
    }

    private sealed class RunParams { public string Run { get; init; } = ""; }

    private static readonly Ui.ScreenKey<RunParams> RunScreen = new("run");

    [Fact]
    public async Task ARunJoinsAGroupAndSaysWhoBroughtItIn()
    {
        var syscalls = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.AgentRuns => new SyscallResult { AgentRuns = new AgentRunsPage { Runs = { WireRun("run-1", "running") } } },
            Syscall.CallOneofCase.AgentGet => new SyscallResult
            {
                // a run an agent started: the wire names its parent and the person at the root
                AgentRun = new V1.AgentRun(WireRun(call.AgentGet.RunId, "running"))
                {
                    Group = "run-1",
                    Parent = "run-1",
                    ParentAgent = "tests.assistant",
                    StartedBy = new AgentRunActor { UserId = "sa-7", DisplayName = "assistant" },
                    OnBehalfOf = new AgentRunActor { UserId = "u-1", DisplayName = "Sarah Bennett" },
                },
            },
            _ => new SyscallResult { AgentRun = new V1.AgentRun(WireRun("run-2", "starting")) { Group = call.AgentRun.Group } },
        });
        var agents = Context(syscalls).Agents;

        var joined = await agents.Run("packer", "how is order 7 packed?", new AgentRunOptions { Group = "run-1" });
        Assert.Equal(("run-2", "run-1"), (joined.Id, joined.Group));
        Assert.Equal(("packer", "run-1"), (syscalls.Calls[0].AgentRun.Agent, syscalls.Calls[0].AgentRun.Group));

        var child = await agents.Get("run-3");
        Assert.Equal(("run-1", "run-1", "tests.assistant"), (child.Group, child.Parent, child.ParentAgent));
        Assert.Equal(new AgentActor(new Id<Platform.User>("sa-7"), "assistant"), child.StartedBy);
        Assert.Equal(new AgentActor(new Id<Platform.User>("u-1"), "Sarah Bennett"), child.OnBehalfOf);

        await agents.List(group: "run-1");
        var listed = syscalls.Calls[2].AgentRuns;
        Assert.Equal(("", 0u, "run-1"), (listed.Agent, listed.Limit, listed.Group));
        await agents.List("packer", 5);
        listed = syscalls.Calls[3].AgentRuns;
        Assert.Equal(("packer", 5u, ""), (listed.Agent, listed.Limit, listed.Group));
    }

    [Fact]
    public async Task RunsAnotherPluginsAgentOnATopicOfItsOwnAndListsTheAgentsTheCallerMayRun()
    {
        var syscalls = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.AgentsAvailable => new SyscallResult
            {
                AgentsAvailable = new AvailableAgents
                {
                    Agents =
                    {
                        new V1.AvailableAgent { Agent = "chat.notes", Plugin = "chat", Name = "notes", Title = "notes", PluginTitle = "Chat" },
                        new V1.AvailableAgent
                        {
                            Agent = "crm.assistant", Plugin = "crm", Name = "assistant", Title = "CRM assistant",
                            Description = "Makes plugins.", Icon = "wand-sparkles", Engine = "claude", Session = true,
                            PluginTitle = "CRM",
                        },
                    },
                },
            },
            _ => new SyscallResult
            {
                AgentRun = new V1.AgentRun(WireRun("run-1", "starting")) { Agent = "assistant", Plugin = "crm" },
            },
        });
        var agents = Context(syscalls).Agents;

        var available = await agents.Available();
        Assert.Equal(77ul, syscalls.Calls[0].AgentsAvailable.RequestId);
        Assert.Equal(["chat.notes", "crm.assistant"], available.Select(a => a.Agent));
        Assert.Equal((null, null, false, "Chat"), (available[0].Icon, available[0].Engine, available[0].Session, available[0].PluginTitle));
        Assert.Equal(("CRM assistant", "wand-sparkles", "claude", true), (available[1].Title, available[1].Icon, available[1].Engine, available[1].Session));

        var run = await agents.Run(available[1].Agent, "make a plugin", new AgentRunOptions
        {
            Topic = Conversation.Name,
            TopicKey = Agents.TopicKey(Conversation, new ConversationKey { Id = "c-7" }),
        });
        Assert.Equal(("crm", "assistant"), (run.Plugin, run.Agent));
        var start = syscalls.Calls[1].AgentRun;
        Assert.Equal(("crm.assistant", "conversation", """{"id":"c-7"}"""), (start.Agent, start.Topic, start.TopicKey));
    }

    private sealed class ConversationKey { public string Id { get; init; } = ""; }

    private static readonly Ui.Topic<ConversationKey> Conversation = new("conversation");

    [Fact]
    public async Task ReadsARunItsEventsAndTheUnfinishedText()
    {
        var agents = Context(Cell).Agents;

        var done = await agents.Get("run-1");
        Assert.Equal((AgentRunStatus.Succeeded, true, "Tea is $4."), (done.Status, done.Ended, done.Result));

        var page = await agents.Events("run-1", after: 2, limit: 50);
        Assert.Equal(AgentRunStatus.Running, page.Run.Status);
        var tool = Assert.Single(page.Events);
        Assert.Equal((3L, DateTimeOffset.FromUnixTimeMilliseconds(1_500), "tool"), (tool.Seq, tool.At, tool.Type));
        // an event keeps the fields of its type
        Assert.Equal(("shell", "completed", "celobus whoami"), (tool.Str("name"), tool.Str("phase"), tool.Str("summary")));
        Assert.Null(tool.Text);
        Assert.Null(tool.Str("seq"));
        Assert.Equal(3, tool.Data["seq"]!.GetValue<int>());
        Assert.Equal(4, page.Partial.Rev);
        Assert.Equal([new AgentPartialMessage("m1", "Tea"), new AgentPartialMessage("m2", "is $4")], page.Partial.Messages);
        Assert.Equal("Tea\n\nis $4", page.Partial.Text);
        var asked = Cell.Calls.Last(c => c.CallCase == Syscall.CallOneofCase.AgentEvents).AgentEvents;
        Assert.Equal(("run-1", 2ul, 50u), (asked.RunId, asked.After, asked.Limit));

        var runs = await agents.List("assistant", 10);
        Assert.Equal(["run-2", "run-1"], runs.Select(r => r.Id));
        Assert.Equal((AgentRunStatus.Failed, new AgentRunError("lost", "the agent stopped answering")), (runs[1].Status, runs[1].Error));
        var listed = Cell.Calls.Last(c => c.CallCase == Syscall.CallOneofCase.AgentRuns).AgentRuns;
        Assert.Equal(("assistant", 10u), (listed.Agent, listed.Limit));
        await agents.List();
        listed = Cell.Calls.Last(c => c.CallCase == Syscall.CallOneofCase.AgentRuns).AgentRuns;
        Assert.Equal(("", 0u), (listed.Agent, listed.Limit));
    }

    [Fact]
    public async Task SendsCancelsAndCloses()
    {
        var syscalls = new FakeSyscalls(call => call.CallCase switch
        {
            Syscall.CallOneofCase.AgentSend => new SyscallResult { AgentInput = new AgentRunInput { Seq = 2 } },
            Syscall.CallOneofCase.AgentCancel => new SyscallResult { AgentRun = WireRun(call.AgentCancel.RunId, "cancelled") },
            _ => new SyscallResult { AgentRun = WireRun(call.AgentClose.RunId, "idle") },
        });
        var agents = Context(syscalls).Agents;

        Assert.Equal(2, await agents.Send("run-1", "and coffee?"));
        var sent = syscalls.Calls[0].AgentSend;
        Assert.Equal(("run-1", "and coffee?", 77ul), (sent.RunId, sent.Text, sent.RequestId));
        var cancelled = await agents.Cancel("run-1");
        Assert.Equal((AgentRunStatus.Cancelled, true), (cancelled.Status, cancelled.Ended));
        Assert.Equal(AgentRunStatus.Idle, (await agents.Close("run-1")).Status);
        Assert.Equal("run-1", syscalls.Calls[2].AgentClose.RunId);
    }

    [Theory]
    [InlineData("unknown_agent: the plugin tests declares no agent nobody", 404, "unknown_agent", "the plugin tests declares no agent nobody")]
    [InlineData("not_found: no such run", 404, "not_found", "no such run")]
    [InlineData("run_ended: the run has ended", 409, "run_ended", "the run has ended")]
    [InlineData("bad_request: input is empty", 400, "bad_request", "input is empty")]
    [InlineData("unknown_group: the plugin tests founded no group run-9", 404, "unknown_group", "the plugin tests founded no group run-9")]
    [InlineData("agent_in_group: the agent assistant has a live run in the group", 409, "agent_in_group", "the agent assistant has a live run in the group")]
    [InlineData("group_full: the group has its eight live runs", 409, "group_full", "the group has its eight live runs")]
    [InlineData("forbidden: an agent run is started while the plugin handles a request", 403, "forbidden", "an agent run is started while the plugin handles a request")]
    [InlineData("store: the store is away", 503, "unavailable", "platform: store: the store is away")]
    public async Task RefusalsKeepThePlatformsCode(string error, int status, string code, string message)
    {
        var agents = Context(new FakeSyscalls(_ => new SyscallResult { Error = error })).Agents;
        var e = await Assert.ThrowsAsync<PluginError>(() => agents.Run("assistant", "x"));
        Assert.Equal((status, code, message), (e.Status, e.Code, e.Message));
        e = await Assert.ThrowsAsync<PluginError>(() => agents.Send("run-1", "more"));
        Assert.Equal((status, code), (e.Status, e.Code));
    }
}
