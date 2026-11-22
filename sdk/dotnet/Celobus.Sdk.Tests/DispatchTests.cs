using System.Text.Json.Nodes;
using Celobus.V1;
using Google.Protobuf;

namespace Celobus.Sdk.Tests;

/// <summary>Routing, decoding and error mapping of one request, without a session.</summary>
public class DispatchTests
{
    private static readonly Runtime Runtime = new(TestPlugin.Define().Version("1.0.0").Definition);

    private static readonly Principal Clerk = new()
    {
        UserId = "u-1",
        Kind = Principal.Types.Kind.Person,
        DisplayName = "Sarah Bennett",
        Roles = { "workspace.member", "tests.clerk" },
        Scopes = { "tests:write" },
    };

    private static async Task<(uint Status, JsonNode Body)> Send(Request req, ISyscalls? syscalls = null)
    {
        var r = await Runtime.Dispatch(req, syscalls ?? new FakeSyscalls(_ => new SyscallResult { Error = "no platform" }), CancellationToken.None);
        Assert.Equal("application/json", r.Headers.Single(h => h.Name == "content-type").Value);
        return (r.Status, JsonNode.Parse(r.Body.ToStringUtf8())!);
    }

    private static Task<(uint Status, JsonNode Body)> Fail(string kind, Principal? principal = null) =>
        Send(TestPlugin.Post("Fail", $$"""{"kind":"{{kind}}"}""", principal));

    [Theory]
    [InlineData("invalid", 400, "invalid", "bad request")]
    [InlineData("unauthenticated", 401, "unauthenticated", "a signed-in user is required")]
    [InlineData("forbidden", 403, "forbidden", "not yours")]
    [InlineData("not_found", 404, "not_found", "no such thing")]
    [InlineData("conflict", 409, "conflict", "taken")]
    [InlineData("unavailable", 503, "unavailable", "try later")]
    [InlineData("domain", 409, "not_cancellable", "the order is shipped")]
    [InlineData("db", 503, "unavailable", "platform: no platform")]
    [InlineData("crash", 500, "internal", "internal error")]
    [InlineData("null", 500, "internal", "internal error")]
    public async Task MapsErrors(string kind, int status, string code, string message)
    {
        var (s, body) = await Fail(kind);
        Assert.Equal((uint)status, s);
        Assert.Equal(code, body["code"]!.GetValue<string>());
        Assert.Equal(message, body["message"]!.GetValue<string>());
        Assert.NotNull(body["details"]);
    }

    [Fact]
    public async Task InvalidCarriesItsFields()
    {
        var (_, body) = await Fail("invalid");
        Assert.Equal("not a kind", body["details"]!["fields"]!["kind"]!.GetValue<string>());
    }

    [Fact]
    public async Task AnswersJson()
    {
        var (s, body) = await Send(TestPlugin.Post("PlaceOrder", """{"thingId":"9","quantity":2}"""));
        Assert.Equal(200u, s);
        Assert.Equal("""{"id":"9"}""", body.ToJsonString());
    }

    [Fact]
    public async Task EmptyBodyIsAnEmptyMessage()
    {
        var (s, body) = await Send(TestPlugin.Post("Fail", ""));
        Assert.Equal(200u, s);
        Assert.Equal("", body["text"]!.GetValue<string>());
    }

    [Fact]
    public async Task UnknownPathMethodAndMediaType()
    {
        Assert.Equal((404u, "unknown_endpoint"), Code(await Send(TestPlugin.Post("Nope", "{}"))));
        var get = TestPlugin.Post("Fail", "{}");
        get.Method = "GET";
        Assert.Equal((405u, "method_not_allowed"), Code(await Send(get)));
        var proto = TestPlugin.Post("Fail", "");
        proto.Headers[0].Value = "application/x-protobuf";
        Assert.Equal((415u, "unsupported_media_type"), Code(await Send(proto)));
        var bad = await Send(TestPlugin.Post("PlaceOrder", """{"quantity":"many"}"""));
        Assert.Equal((400u, "invalid"), Code(bad));
        Assert.NotNull(bad.Body["details"]!["fields"]!["quantity"]);
    }

    private static (uint, string) Code((uint Status, JsonNode Body) r) => (r.Status, r.Body["code"]!.GetValue<string>());

    [Fact]
    public async Task DeadlineCancelsTheHandler()
    {
        var (s, body) = await Send(TestPlugin.Post("Slow", """{"ms":5000}"""));
        Assert.Equal(504u, s);
        Assert.Equal("deadline_exceeded", body["code"]!.GetValue<string>());
    }

    [Fact]
    public async Task CallerComesFromThePrincipal()
    {
        var (_, body) = await Fail("whoami", Clerk);
        Assert.Equal("person:u-1 True False", body["text"]!.GetValue<string>());
        (_, body) = await Fail("whoami");
        Assert.Equal("(nobody) False False", body["text"]!.GetValue<string>());
    }

    [Fact]
    public async Task CallsADependencyAsTheCaller()
    {
        var syscalls = new FakeSyscalls(call =>
        {
            var resp = new Response { Status = 200, Body = ByteString.CopyFromUtf8("""{"text":"from other"}""") };
            return new SyscallResult { InvokeResponse = resp };
        });
        var req = TestPlugin.Post("Fail", """{"kind":"call_typed"}""");
        req.Id = 77;
        var (s, body) = await Send(req, syscalls);
        Assert.Equal(200u, s);
        Assert.Equal("from other", body["text"]!.GetValue<string>());
        var invoke = syscalls.Calls.Single().Invoke;
        Assert.Equal("other", invoke.Plugin);
        Assert.Equal(77u, invoke.RequestId);
        Assert.Equal("POST", invoke.Request.Method);
        Assert.Equal("/other.v1.Other/Get", invoke.Request.Path);
        Assert.Equal("""{"thingId":"5","count":"3"}""", invoke.Request.Body.ToStringUtf8());
    }

    [Fact]
    public async Task ForwardsADependencysError()
    {
        var syscalls = new FakeSyscalls(_ => new SyscallResult
        {
            InvokeResponse = new Response
            {
                Status = 404,
                Body = ByteString.CopyFromUtf8("""{"code":"not_found","message":"no customer","details":{}}"""),
            },
        });
        var (s, body) = await Send(TestPlugin.Post("Fail", """{"kind":"call"}"""), syscalls);
        Assert.Equal(404u, s);
        Assert.Equal("no customer", body["message"]!.GetValue<string>());
    }

    [Fact]
    public async Task SyscallErrorsAreUnavailable()
    {
        var (s, body) = await Fail("call");
        Assert.Equal(503u, s);
        Assert.Equal("platform: no platform", body["message"]!.GetValue<string>());
    }

    [Fact]
    public void DomainErrorsCheckCodeAndStatus()
    {
        Assert.Throws<ArgumentOutOfRangeException>(() => PluginError.Domain("late", 500, "no"));
        Assert.Throws<ArgumentException>(() => PluginError.Domain("Not-Valid", 409, "no"));
    }

    /// <summary>The run-change hook: a request of the platform, with no caller; one from a caller is refused as unknown.</summary>
    [Fact]
    public async Task TheRunChangeHookIsASystemRequest()
    {
        var seen = new List<RunChange>();
        var plugin = TestPlugin.Define().Version("1.0.0").OnRunChange((ctx, change) =>
        {
            Assert.Equal(CallerKind.None, ctx.Caller.Kind);
            if (change.Turn == 9) throw PluginError.Internal();
            lock (seen) seen.Add(change);
            return Task.CompletedTask;
        });
        Assert.Equal("""{"on_change":true}""", JsonNode.Parse(plugin.DescriptorJson())!["runs"]!.ToJsonString());
        Assert.Null(JsonNode.Parse(TestPlugin.Define().Version("1.0.0").DescriptorJson())!["runs"]);
        var runtime = new Runtime(plugin.Definition);
        async Task<(uint, JsonNode)> Hook(string body, Principal? principal)
        {
            var req = TestPlugin.Post("_RunChange", body, principal);
            Assert.Equal("/tests.v1.Tests/_RunChange", req.Path);
            var r = await runtime.Dispatch(req, new FakeSyscalls(_ => new SyscallResult { Error = "no platform" }), CancellationToken.None);
            return (r.Status, JsonNode.Parse(r.Body.ToStringUtf8())!);
        }
        var change = new JsonObject
        {
            ["run_id"] = "run-1", ["agent"] = "assistant", ["plugin"] = "crm", ["status"] = "idle", ["turn"] = 2,
            ["last_seq"] = 14, ["topic"] = "conversation", ["topic_key"] = """{"channel":"c-1"}""", ["ended"] = false,
        };

        // from a member (it came from outside): no such endpoint
        Assert.Equal((404u, "unknown_endpoint"), Code(await Hook(change.ToJsonString(), Clerk)));
        Assert.Empty(seen);
        // without the hook: no such endpoint either
        Assert.Equal((404u, "unknown_endpoint"), Code(await Send(TestPlugin.Post("_RunChange", change.ToJsonString()))));

        var (status, body) = await Hook(change.ToJsonString(), null);
        Assert.Equal((200u, "{}"), (status, body.ToJsonString()));
        Assert.Equal(new RunChange
        {
            RunId = "run-1", Agent = "assistant", Plugin = "crm", Status = AgentRunStatus.Idle, Turn = 2, LastSeq = 14,
            Topic = "conversation", TopicKey = """{"channel":"c-1"}""", Ended = false,
        }, seen.Single());

        // an error answers non-2xx: the platform calls again
        change["turn"] = 9;
        Assert.Equal(500u, (await Hook(change.ToJsonString(), null)).Item1);
        // no run id: invalid
        Assert.Equal((400u, "invalid"), Code(await Hook("{}", null)));
        // `ended` defaults to whether the status is an end
        await Hook("""{"run_id":"run-2","status":"succeeded"}""", null);
        Assert.True(seen[^1].Ended);
        Assert.Equal(AgentRunStatus.Succeeded, seen[^1].Status);
    }
}
