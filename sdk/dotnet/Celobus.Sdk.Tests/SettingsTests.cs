using System.ComponentModel;
using System.Text.Json.Nodes;
using Celobus.Sdk.Tests;
using Celobus.V1;

// its own namespace: the tests' fixtures have a `Secret` entity of their own
namespace Celobus.Sdk.SettingTests;

[Message]
public enum ChatTone { Calm, LoudAndClear }

/// <summary>How the chat treats agents in threads.</summary>
[Message(Kind = MessageKind.Settings)]
[Describe("How the chat treats agents in threads.")]
public sealed record ChatSettings
{
    [Describe("After this many hours without messages a thread no longer sticks to the agent that answered in it.")]
    [Label("Thread stays with its agent, hours"), Range(1, 48), Section("Agents")]
    public int ThreadWarmHours { get; init; } = 2;

    [Describe("The longest message a member may send.")]
    [Range(100, 20000), ValidationMessage("From 100 to 20,000 characters")]
    public long MessageMaxChars { get; init; } = 20000;

    [Label("Webhook URL"), Url, Section("Integrations"), Placeholder("https://")]
    public string? WebhookUrl { get; init; }

    [Section("Integrations"), MaxLength(200)]
    public Secret? WebhookToken { get; init; }

    [DefaultValue(ChatTone.Calm)]
    public ChatTone Tone { get; init; }

    [MaxItems(3)]
    public List<Id<Platform.User>> Watchers { get; init; } = [];

    [Multiline, Pattern("^[A-Z]"), MinLength(1)]
    public required string Greeting { get; init; }

    public decimal? Price { get; init; }
}

[Message(Kind = MessageKind.Settings)]
public sealed record OtherSettings
{
    public int? N { get; init; }
}

[Message]
public sealed record ScopedRequest
{
    [Range(1, 5)]
    public int N { get; init; }

    public Secret? K { get; init; }
}

[Message]
public sealed record ScopedReply;

public sealed class Scoped : Endpoint<ScopedRequest, ScopedReply>
{
    public override Task<ScopedReply> Handle(Ctx ctx, ScopedRequest req) => Task.FromResult(new ScopedReply());
}

[Message(Kind = MessageKind.Settings)]
public sealed record BadSettings
{
    public DateTimeOffset? At { get; init; }

    [MaxLength(3)]
    public int? N { get; init; }

    [Range(1, 2)]
    public string? Name { get; init; }

    [DefaultValue("x")]
    public Secret? Token { get; init; }

    public List<int> Many { get; init; } = [];

    [Range(5, 1)]
    public int? Pair { get; init; }

    [MaxItems(2)]
    public string? One { get; init; }
}

[Message(Kind = MessageKind.Settings)]
public sealed record SpelledSettings
{
    [Range(-1.5, 2)]
    public double Rate { get; init; } = 0.5;

    [DefaultValue(typeof(decimal), "12.50"), Range("0.01", "1000")]
    public decimal Price { get; init; }

    [Range(Min = "2020-01-01")]
    public DateOnly Since { get; init; } = new(2026, 1, 1);

    public bool On { get; init; } = true;

    [MinItems(1)]
    public List<string> Tags { get; init; } = ["a", "b"];

    public List<ChatTone> Tones { get; init; } = [ChatTone.Calm];

    [Email]
    public string? Contact { get; init; }
}

public class SettingsTests
{
    private static Plugin Chat(Action<SettingsOptions<ChatSettings>>? configure = null) =>
        Plugin.Create("chat").Version("1.0.0").Settings(configure);

    private static JsonObject Describe(Plugin p)
    {
        Assert.Empty(p.Check());
        return JsonNode.Parse(p.DescriptorJson())!.AsObject();
    }

    private static JsonNode Message(JsonObject d, string name) =>
        d["messages"]!.AsArray().Single(m => (string?)m!["name"] == name)!;

    [Fact]
    public void DescribeWritesTheSettingsAndTheirFieldsOptions()
    {
        var d = Describe(Chat());
        var m = Message(d, "ChatSettings");
        Assert.Equal("settings", (string?)m["kind"]);
        Assert.Equal("ChatSettings", (string?)d["messages"]!.AsArray()[^1]!["name"]);
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""
            { "name": "thread_warm_hours", "number": 1, "type": "int32", "repeated": false, "optional": false,
              "description": "After this many hours without messages a thread no longer sticks to the agent that answered in it.",
              "label": "Thread stays with its agent, hours", "default": 2, "min": 1, "max": 48, "section": "Agents" }
            """), m["fields"]![0]), m["fields"]![0]!.ToJsonString());
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""
            { "name": "message_max_chars", "number": 2, "type": "int64", "repeated": false, "optional": false,
              "description": "The longest message a member may send.",
              "default": 20000, "min": 100, "max": 20000, "message": "From 100 to 20,000 characters" }
            """), m["fields"]![1]), m["fields"]![1]!.ToJsonString());
        Assert.Equal("url", (string?)m["fields"]![2]!["format"]);
        Assert.Equal(("secret", 200), ((string?)m["fields"]![3]!["type"], (int)m["fields"]![3]!["max_len"]!));
        Assert.Equal(("enum:ChatTone", "calm"), ((string?)m["fields"]![4]!["type"], (string?)m["fields"]![4]!["default"]));
        Assert.Equal(("ref:platform.user", true, 3),
            ((string?)m["fields"]![5]!["type"], (bool)m["fields"]![5]!["repeated"]!, (int)m["fields"]![5]!["max_items"]!));
        Assert.Equal((true, "^[A-Z]", 1),
            ((bool)m["fields"]![6]!["multiline"]!, (string?)m["fields"]![6]!["pattern"], (int)m["fields"]![6]!["min_len"]!));
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""
            { "name": "price", "number": 8, "type": "decimal", "repeated": false, "optional": true, "description": "" }
            """), m["fields"]![7]));
        var expected = JsonNode.Parse("""
            { "message": "ChatSettings", "on_change": false, "form": { "sections": [
              { "title": "Agents", "inputs": [
                { "name": "thread_warm_hours", "control": "number", "label": "Thread stays with its agent, hours",
                  "help": "After this many hours without messages a thread no longer sticks to the agent that answered in it." } ] },
              { "inputs": [
                { "name": "message_max_chars", "control": "number", "label": "Message max chars",
                  "help": "The longest message a member may send." },
                { "name": "tone", "control": "select", "label": "Tone",
                  "options": [{ "value": "calm", "label": "Calm" }, { "value": "loud_and_clear", "label": "Loud and clear" }] },
                { "name": "watchers", "control": "member", "label": "Watchers" },
                { "name": "greeting", "control": "textarea", "label": "Greeting", "required": true },
                { "name": "price", "control": "number", "label": "Price" } ] },
              { "title": "Integrations", "inputs": [
                { "name": "webhook_url", "control": "text", "label": "Webhook URL", "placeholder": "https://" },
                { "name": "webhook_token", "control": "secret", "label": "Webhook token" } ] }
            ] } }
            """);
        Assert.True(JsonNode.DeepEquals(expected, d["settings"]), d["settings"]!.ToJsonString());
    }

    [Fact]
    public void DefaultsAndBoundsAreWrittenAsEachTypeSpellsThem()
    {
        var d = Describe(Plugin.Create("xy").Version("1.0.0").Settings<SpelledSettings>());
        var f = Message(d, "SpelledSettings")["fields"]!.AsArray().ToDictionary(x => (string)x!["name"]!, x => x!);
        Assert.Equal("""[0.5,-1.5,2]""", new JsonArray(f["rate"]["default"]!.DeepClone(), f["rate"]["min"]!.DeepClone(), f["rate"]["max"]!.DeepClone()).ToJsonString());
        Assert.Equal(("12.50", "0.01", "1000"), ((string?)f["price"]["default"], (string?)f["price"]["min"], (string?)f["price"]["max"]));
        Assert.Equal(("2026-01-01", "2020-01-01"), ((string?)f["since"]["default"], (string?)f["since"]["min"]));
        Assert.Null(f["since"]["max"]);
        Assert.True((bool)f["on"]["default"]!);
        Assert.Equal(("""["a","b"]""", 1), (f["tags"]["default"]!.ToJsonString(), (int)f["tags"]["min_items"]!));
        Assert.Equal("""["calm"]""", f["tones"]["default"]!.ToJsonString());
        Assert.Equal("email", (string?)f["contact"]["format"]);
        var inputs = d["settings"]!["form"]!["sections"]!.AsArray().SelectMany(s => s!["inputs"]!.AsArray()).ToList();
        Assert.Equal(["number", "number", "date", "switch", "textarea", "multi_select", "text"], inputs.Select(i => (string)i!["control"]!));
        Assert.Equal("""[{"value":"calm","label":"Calm"},{"value":"loud_and_clear","label":"Loud and clear"}]""", inputs[5]!["options"]!.ToJsonString());
    }

    [Fact]
    public void AFormLaidOutByHandGetsWhatItsFieldsCarry()
    {
        var s = Describe(Chat(o => o
            .OnChange((_, _) => Task.CompletedTask)
            .Form(f =>
            {
                f.Section("Messages");
                f.Number(x => x.MessageMaxChars, "Longest message");
                f.Textarea(x => x.Greeting, "Greeting").Help("Shown first");
                f.Section("Other");
                f.Number(x => x.ThreadWarmHours, "Hours");
                f.Radio(x => x.Tone, "Tone");
                f.Member(x => x.Watchers, "Watchers");
                f.Text(x => x.WebhookUrl, "URL").Required();
                f.Secret(x => x.WebhookToken, "Token");
                f.Number(x => x.Price, "Price");
            })))["settings"]!;
        Assert.True((bool)s["on_change"]!);
        var sections = s["form"]!["sections"]!.AsArray();
        Assert.Equal(2, sections.Count);
        Assert.Equal("Messages", (string?)sections[0]!["title"]);
        Assert.Equal("""{"name":"message_max_chars","control":"number","label":"Longest message","help":"The longest message a member may send."}""",
            Sorted(sections[0]!["inputs"]![0]!));
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""{"name":"greeting","control":"textarea","label":"Greeting","help":"Shown first","required":true}"""),
            sections[0]!["inputs"]![1]));
        Assert.Equal("""{"value":"loud_and_clear","label":"Loud and clear"}""", sections[1]!["inputs"]![1]!["options"]![1]!.ToJsonString());
        // `required` follows the field, whatever the input says
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""{"name":"webhook_url","control":"text","label":"URL","placeholder":"https://"}"""),
            sections[1]!["inputs"]![3]), sections[1]!["inputs"]![3]!.ToJsonString());
        Assert.Equal("secret", (string?)sections[1]!["inputs"]![4]!["control"]);
    }

    private static string Sorted(JsonNode n) =>
        new JsonObject(n.AsObject().OrderBy(p => p.Key switch { "name" => 0, "control" => 1, "label" => 2, _ => 3 })
            .Select(p => KeyValuePair.Create(p.Key, p.Value?.DeepClone()))).ToJsonString();

    [Fact]
    public void DescribeRefusesAFormWithoutEveryField()
    {
        var errors = Chat(o => o.Form(f =>
        {
            f.Number(x => x.ThreadWarmHours, "Hours");
            f.Text(x => x.ThreadWarmHours, "Twice");
        })).Check();
        Assert.Contains(errors, e => e.Code == "E_SETTINGS_FORM_FIELD" && e.Message.Contains("greeting is not in the form"));
        Assert.Contains(errors, e => e.Code == "E_SETTINGS_FORM_FIELD" && e.Message.Contains("twice"));
        Assert.Contains(errors, e => e.Code == "E_SETTINGS_CONTROL");
    }

    [Fact]
    public void TheOptionsOfASettingBelongToTheSettingsMessage()
    {
        var errors = Plugin.Create("xy").Version("1.0.0").Endpoint<Scoped>().Check();
        Assert.Equal(
            [("E_FIELD_VALIDATOR_SCOPE", "messages.ScopedRequest.fields.n"), ("E_SETTINGS_FIELD_TYPE", "messages.ScopedRequest.fields.k")],
            errors.Select(e => (e.Code, e.Path)));
        Assert.Equal([("E_SETTINGS_MESSAGE", "settings")],
            Plugin.Create("xy").Version("1.0.0").Settings<ScopedReply>().Check().Select(e => (e.Code, e.Path)));
    }

    private static readonly string[] BadFields = ["at", "n", "name", "token", "many", "pair", "one"];

    // (the fields of a type settings do not take are not in the derived form either)
    [Fact]
    public void DescribeRefusesSettingsFieldsOfATypeOrWithOptionsTheyDoNotTake()
    {
        var errors = Plugin.Create("xy").Version("1.0.0").Settings<BadSettings>().Check()
            .Where(e => e.Code != "E_SETTINGS_FORM_FIELD").Select(e => (e.Code, e.Path))
            .OrderBy(e => Array.IndexOf(BadFields, e.Path.Split('.')[3])).ToList();
        Assert.Equal([
            ("E_SETTINGS_FIELD_TYPE", "messages.BadSettings.fields.at"),
            ("E_SETTINGS_VALIDATOR", "messages.BadSettings.fields.n.max_len"),
            ("E_SETTINGS_VALIDATOR", "messages.BadSettings.fields.name.min"),
            ("E_SETTINGS_DEFAULT", "messages.BadSettings.fields.token.default"),
            ("E_SETTINGS_FIELD_TYPE", "messages.BadSettings.fields.many"),
            ("E_SETTINGS_VALIDATOR", "messages.BadSettings.fields.pair.min"),
            ("E_SETTINGS_VALIDATOR", "messages.BadSettings.fields.one.min_items"),
        ], errors);
    }

    [Fact]
    public void ASecretStaysOutOfLogs()
    {
        var s = new Secret("sk-live-123");
        Assert.Equal("Secret(••••)", s.ToString());
        Assert.Equal("Secret(••••)", $"{s}");
        Assert.Equal("sk-live-123", s.Expose());
        var settings = new ChatSettings { Greeting = "Hi", WebhookToken = s };
        Assert.DoesNotContain("sk-live", settings.ToString());
        Assert.Equal("sk-live-123", (string?)JsonNode.Parse(CelobusJson.Serialize(settings))!["webhookToken"]);
        Assert.Equal(new Secret("a"), new Secret("a"));
    }

    [Fact]
    public void TheCacheFillsDefaultsAndNamesWhatIsMissing()
    {
        var cache = new SettingsCache(typeof(ChatSettings));
        // nothing sent yet: the defaults, and the required greeting is missing
        var e = Assert.Throws<PluginError>(() => cache.Get<ChatSettings>());
        Assert.Equal((409, "settings_required"), (e.Status, e.Code));
        Assert.Equal("""["greeting"]""", e.Details!["missing"]!.ToJsonString());
        Assert.Equal("Required", e.Fields["greeting"]);

        Assert.True(cache.Update(3, """{"greeting": "Hi", "threadWarmHours": 6}"""u8.ToArray(), registered: true));
        var s = cache.Get<ChatSettings>();
        Assert.Equal((6, 20000L, ChatTone.Calm), (s.ThreadWarmHours, s.MessageMaxChars, s.Tone));
        Assert.Same(s, cache.Get<ChatSettings>());
        // an older change is stale; a newer one replaces them
        Assert.False(cache.Update(2, """{"greeting": "Old"}"""u8.ToArray(), registered: false));
        Assert.Equal("Hi", cache.Get<ChatSettings>().Greeting);
        Assert.True(cache.Update(4, """{"greeting": "New", "messageMaxChars": "300", "webhookToken": "sk-1"}"""u8.ToArray(), registered: false));
        s = cache.Get<ChatSettings>();
        Assert.Equal(("New", 300L, "sk-1"), (s.Greeting, s.MessageMaxChars, s.WebhookToken!.Expose()));
        // a value of the wrong type is an error, never a guess
        Assert.True(cache.Update(5, """{"greeting": "X", "threadWarmHours": "many"}"""u8.ToArray(), registered: false));
        Assert.Equal(500, Assert.Throws<PluginError>(() => cache.Get<ChatSettings>()).Status);
        // not JSON: kept the previous ones
        Assert.False(cache.Update(6, "{"u8.ToArray(), registered: false));
        // another message than the declared one
        Assert.Equal(500, Assert.Throws<PluginError>(() => cache.Get<OtherSettings>()).Status);
        // a registration replaces them whatever its revision
        Assert.True(cache.Update(0, """{"greeting": "Again"}"""u8.ToArray(), registered: true));
        Assert.Equal("Again", cache.Get<ChatSettings>().Greeting);
        // empty bytes: no settings sent, the defaults
        Assert.True(cache.Update(0, [], registered: true));
        Assert.Equal(409, Assert.Throws<PluginError>(() => cache.Get<ChatSettings>()).Status);
    }

    [Fact]
    public void ATestContextHasTheSettingsItIsGiven()
    {
        var ctx = Ctx.Test("chat", "1.0.0");
        Assert.Equal(500, Assert.Throws<PluginError>(() => ctx.Settings<ChatSettings>()).Status);
        var given = new ChatSettings { Greeting = "Hi", ThreadWarmHours = 4, WebhookToken = new Secret("sk-1") };
        ctx = ctx.WithSettings(given);
        var s = ctx.Settings<ChatSettings>();
        Assert.Equal((4, 20000L, ChatTone.Calm, "sk-1"), (s.ThreadWarmHours, s.MessageMaxChars, s.Tone, s.WebhookToken!.Expose()));
        Assert.Same(s, ctx.Settings<ChatSettings>());
        Assert.Equal(4, ctx.WithCaller(Caller.TestUser("u-1")).Settings<ChatSettings>().ThreadWarmHours);
        Assert.Equal(4, ctx.AsSystem("test").Settings<ChatSettings>().ThreadWarmHours);
    }

    [Fact]
    public async Task TheCheckHookIsASystemRequest()
    {
        var seen = new List<(int?, int)>();
        var plugin = Chat(o => o.OnChange((ctx, change) =>
        {
            Assert.Equal(CallerKind.None, ctx.Caller.Kind);
            if (change.Current.WebhookToken is not null && change.Current.WebhookUrl is null)
                throw PluginError.Invalid("a token needs a URL").Field("webhook_url", "Set the URL the token is for");
            if (change.Current.Greeting == "Boom") throw new PluginError(422, "unreachable", "The service did not answer");
            lock (seen) seen.Add((change.Previous?.ThreadWarmHours, change.Current.ThreadWarmHours));
            return Task.CompletedTask;
        }));
        var runtime = new Runtime(plugin.Definition);
        async Task<(uint Status, JsonNode Body)> Hook(string body, Principal? principal)
        {
            var req = new Request
            {
                Id = 7, Method = "POST", Path = "/chat.v1.Chat/_SettingsCheck",
                Body = Google.Protobuf.ByteString.CopyFromUtf8(body), Principal = principal,
            };
            req.Headers.Add(new Header { Name = "content-type", Value = "application/json" });
            var r = await runtime.Dispatch(req, new FakeSyscalls(_ => new SyscallResult { Error = "no platform" }), CancellationToken.None);
            return (r.Status, JsonNode.Parse(r.Body.ToStringUtf8())!);
        }
        var member = new Principal { UserId = "u-1", Kind = Principal.Types.Kind.Person };
        var (status, body) = await Hook("""{"previous":{"greeting":"Hi"},"current":{"greeting":"Hi","threadWarmHours":8}}""", member);
        Assert.Equal((404u, "unknown_endpoint"), (status, (string?)body["code"]));

        (status, body) = await Hook("""{"previous":{"greeting":"Hi"},"current":{"greeting":"Hi","threadWarmHours":8}}""", null);
        Assert.Equal((200u, "{}"), (status, body.ToJsonString()));
        Assert.Equal(200u, (await Hook("""{"previous":{},"current":{"greeting":"Hi","threadWarmHours":9}}""", null)).Status);
        Assert.Equal([(2, 8), (null, 9)], seen);

        (status, body) = await Hook("""{"previous":null,"current":{"greeting":"Hi","webhookToken":"sk-1"}}""", null);
        Assert.Equal(400u, status);
        Assert.Equal("""{"webhook_url":"Set the URL the token is for"}""", body["details"]!["fields"]!.ToJsonString());
        (status, body) = await Hook("""{"current":{"greeting":"Boom"}}""", null);
        Assert.Equal((422u, "The service did not answer"), (status, (string?)body["message"]));
        // without the hook: no such endpoint
        var plain = new Runtime(Chat().Definition);
        var r = await plain.Dispatch(new Request { Id = 1, Method = "POST", Path = "/chat.v1.Chat/_SettingsCheck" },
            new FakeSyscalls(_ => new SyscallResult()), CancellationToken.None);
        Assert.Equal(404u, r.Status);
    }
}
