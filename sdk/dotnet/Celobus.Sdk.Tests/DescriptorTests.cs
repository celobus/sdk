using System.Text.Json.Nodes;
using Hello;

namespace Celobus.Sdk.Tests;

public class DescriptorTests
{
    [Fact]
    public void ConformancePluginMatchesItsFixture()
    {
        var plugin = HelloPlugin.Define();
        Assert.Empty(plugin.Check());
        var actual = JsonNode.Parse(plugin.DescriptorJson());
        var expected = JsonNode.Parse(System.IO.File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "conformance", "expected.json")));
        Assert.True(JsonNode.DeepEquals(expected, actual), actual!.ToJsonString());
    }

    [Fact]
    public void VersionComesFromTheCallingAssembly() =>
        Assert.Equal("0.1.0", JsonNode.Parse(HelloPlugin.Define().DescriptorJson())!["version"]!.GetValue<string>());

    [Fact]
    public void EndpointNameIsTheRequestTypeMinusRequest()
    {
        var d = JsonNode.Parse(TestPlugin.Define().Version("1.0.0").DescriptorJson())!;
        var endpoints = d["endpoints"]!.AsArray().Select(e => e!["name"]!.GetValue<string>()).ToList();
        Assert.Equal(["Fail", "Slow", "Gate", "PlaceOrder"], endpoints);
        Assert.Equal("/tests.v1.Tests/PlaceOrder", Names.Path("tests", "PlaceOrder"));
        Assert.Equal("Tests", d["title"]!.GetValue<string>());
    }

    [Fact]
    public void AnAgentSaysHowItRunsAndWhatItLeavesOutIsLeftOut()
    {
        var clerk = new Role("clerk");
        var d = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0")
            .Role(clerk)
            .Agent("sync", clerk)
            .Agent("builder", ["plugins.install"], clerk)
            .Agent("assistant", a => a.Title("Assistant").Describe("Answers questions about prices.")
                .Engine(AgentEngine.Pi).Model("claude-opus-5-5").Session().Timeout(TimeSpan.FromMinutes(15))
                .Skills("money"))
            .DescriptorJson())!;
        var expected = JsonNode.Parse("""
            [{"name":"sync","roles":["clerk"]},
             {"name":"builder","roles":["clerk"],"workspace":["plugins.install"]},
             {"name":"assistant","roles":[],"title":"Assistant","description":"Answers questions about prices.",
              "engine":"pi","model":"claude-opus-5-5","session":true,"timeout_s":900,"skills":["money"]}]
            """);
        Assert.True(JsonNode.DeepEquals(expected, d["agents"]), d["agents"]!.ToJsonString());
    }

    [Fact]
    public void AnAgentNamesTheAgentsItMayBringIntoItsGroup()
    {
        var clerk = new Role("clerk");
        var d = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0")
            .Role(clerk)
            .Agent("helper", a => a.Roles(clerk).Delegates("packer", "crm.scout"))
            .Agent("packer", a => a.Describe("Says how an order is packed."))
            .DescriptorJson())!;
        // written as given: an agent of the plugin by its name, a dependency's with the plugin; left out when none
        var expected = JsonNode.Parse("""
            [{"name":"helper","roles":["clerk"],"delegates":["packer","crm.scout"]},
             {"name":"packer","roles":[],"description":"Says how an order is packed."}]
            """);
        Assert.True(JsonNode.DeepEquals(expected, d["agents"]), d["agents"]!.ToJsonString());
    }

    [Fact]
    public void AnAgentAsksForBaseRoleAdmin()
    {
        var d = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0")
            .Agent("builder", a => a.Workspace("plugins.install").Admin())
            .Agent("packer", a => a.Describe("Says how an order is packed."))
            .DescriptorJson())!;
        // written only when asked for, so an agent that does not ask describes itself as before
        var expected = JsonNode.Parse("""
            [{"name":"builder","roles":[],"workspace":["plugins.install"],"admin":true},
             {"name":"packer","roles":[],"description":"Says how an order is packed."}]
            """);
        Assert.True(JsonNode.DeepEquals(expected, d["agents"]), d["agents"]!.ToJsonString());
    }

    [Fact]
    public void AnAgentNamesTheTopicItsRunsInvalidate()
    {
        var clerk = new Role("clerk");
        var d = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0")
            .Role(clerk)
            .Topic(LineTopics.Lines.Roles(clerk))
            .Agent("assistant", a => a.Roles(clerk).Topic(LineTopics.Lines))
            .DescriptorJson())!;
        var expected = JsonNode.Parse("""[{"name":"assistant","roles":["clerk"],"topic":"lines"}]""");
        Assert.True(JsonNode.DeepEquals(expected, d["agents"]), d["agents"]!.ToJsonString());
        Assert.Equal("lines", d["topics"]![0]!["name"]!.GetValue<string>());

        // a topic the plugin does not declare
        var errors = Plugin.Create("shop").Version("1.0.0")
            .Agent("assistant", a => a.Topic(LineTopics.Lines))
            .Check();
        Assert.Equal(["E_AGENT_TOPIC_UNKNOWN agents.assistant.topic"], errors.Select(e => $"{e.Code} {e.Path}"));
    }

    [Fact]
    public void FieldsAndMessagesFollowDeclarationAndFirstUse()
    {
        var d = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0")
            .Endpoint<PlaceOrder>()
            .Message<Props>()
            .Message<Sample>()
            .DescriptorJson())!;
        Assert.Equal(["PlaceOrderRequest", "PlaceOrderReply", "Props", "Sample", "Line"],
            d["messages"]!.AsArray().Select(m => m!["name"]!.GetValue<string>()));

        var props = d["messages"]![2]!;
        Assert.Equal("Init-only properties.", props["description"]!.GetValue<string>());
        Assert.Equal(["zeta", "alpha"], props["fields"]!.AsArray().Select(f => f!["name"]!.GetValue<string>()));
        Assert.Equal("The second field.", props["fields"]![1]!["description"]!.GetValue<string>());
        Assert.Equal("How many.", d["messages"]![0]!["fields"]![1]!["description"]!.GetValue<string>());

        var sample = d["messages"]![3]!["fields"]!.AsArray()
            .ToDictionary(f => f!["name"]!.GetValue<string>(), f => f!);
        Assert.Equal(Enumerable.Range(1, 18), sample.Values.Select(f => f["number"]!.GetValue<int>()));
        string Type(string name) => sample[name]["type"]!.GetValue<string>();
        Assert.Equal("int64", Type("big"));
        Assert.Equal("int32", Type("small"));
        Assert.Equal("decimal", Type("amount"));
        Assert.Equal("uuid", Type("uuid"));
        Assert.Equal("timestamp", Type("at"));
        Assert.Equal("date", Type("day"));
        Assert.Equal("bytes", Type("data"));
        Assert.Equal("enum:Status", Type("status"));
        Assert.Equal("message:Line", Type("lines"));
        Assert.True(sample["lines"]["repeated"]!.GetValue<bool>());
        Assert.False(sample["lines"]["optional"]!.GetValue<bool>());
        Assert.True(sample["single"]["optional"]!.GetValue<bool>());
        Assert.True(sample["maybe"]["optional"]!.GetValue<bool>());
        Assert.Equal("ref:shop.thing", Type("thing_id"));
        Assert.Equal("ref:platform.user", Type("owner"));
        Assert.True(sample["owner"]["optional"]!.GetValue<bool>());
        Assert.Equal("int64", Type("numbers"));
        Assert.True(sample["numbers"]["repeated"]!.GetValue<bool>());

        Assert.Equal(["Status"], d["enums"]!.AsArray().Select(e => e!["name"]!.GetValue<string>()));
        Assert.Equal(["placed", "on_hold"], d["enums"]![0]!["values"]!.AsArray().Select(v => v!.GetValue<string>()));
        // scopes: write by default
        Assert.Equal("write", d["endpoints"]![0]!["scopes"]![0]!.GetValue<string>());
    }

    [Fact]
    public void EntitiesAreDescribed()
    {
        var d = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0").Entity<Thing>().DescriptorJson())!;
        var thing = d["entities"]![0]!;
        Assert.Equal("thing", thing["name"]!.GetValue<string>());
        Assert.Equal("uuid", thing["key"]!.GetValue<string>());
        Assert.Equal("name", thing["label"]!.GetValue<string>());
        Assert.True(thing["soft_delete"]!.GetValue<bool>());
        Assert.Equal("Thing", d["messages"]![0]!["name"]!.GetValue<string>());
    }

    [Fact]
    public void ColumnModifiersArePrintedOnTheEntitysFieldsOnlyWhenSet()
    {
        var d = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0").Entity<Sku>().DescriptorJson())!;
        var fields = d["entities"]![0]!["fields"]!.AsArray();
        Assert.True(fields[1]!["unique"]!.GetValue<bool>());
        Assert.True(fields[2]!["index"]!.GetValue<bool>());
        Assert.True(fields[3]!["encrypted"]!.GetValue<bool>());
        Assert.Null(fields[1]!["encrypted"]);
        Assert.Null(fields[2]!["encrypted"]);
        Assert.Null(fields[3]!["index"]);
        foreach (var f in d["messages"]![0]!["fields"]!.AsArray())
            Assert.True(f!["unique"] is null && f["index"] is null && f["encrypted"] is null, f["name"]!.GetValue<string>());
    }

    [Fact]
    public void CollectsDefinitionErrors()
    {
        var errors = Plugin.Create("platform").Version("one")
            .Role(new Role("Clerk"))
            .Role(new Role("Clerk"))
            .Capability(Capability.PushSend, "")
            .Capability(new Capability("net:any"), "because")
            .DependsOn("platform")
            .Message<Unsupported>()
            .Endpoint<BadName>()
            .Endpoint<TooSlow>()
            .Check();
        var codes = errors.Select(e => e.Code).ToHashSet();
        Assert.Subset(codes, new HashSet<string>
        {
            "E_PLUGIN_ID_RESERVED", "E_PLUGIN_VERSION", "E_ROLE_NAME", "E_ROLE_DUP", "E_CAPABILITY_REASON",
            "E_CAPABILITY_UNKNOWN", "E_DEP_SELF", "E_FIELD_TYPE", "E_ENDPOINT_INPUT_NAME", "E_ENDPOINT_ROLE_UNKNOWN",
            "E_ENDPOINT_SCOPE_UNKNOWN", "E_ENDPOINT_TIMEOUT_RANGE",
        });
        var field = errors.First(e => e.Code == "E_FIELD_TYPE");
        Assert.Equal("error[E_FIELD_TYPE] at messages.Unsupported.fields.tags: `Dictionary<String, String>` is not a field type " +
                     "(a scalar, Id<T>, File, an enum, a [Message] type or a list of them)", field.ToString());
        Assert.Contains(errors, e => e.Code == "E_PLUGIN_ID" || e.Code == "E_PLUGIN_ID_RESERVED");
        Assert.Contains(Plugin.Create("Bad_Id").Version("1.0.0").Check(), e => e.Code == "E_PLUGIN_ID");
    }

    [Fact]
    public void ReadsAreWrittenOnlyWhenDeclared()
    {
        var bare = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0").DependsOn("customers", "GetCustomer").DescriptorJson())!;
        Assert.Null(bare["statements"]);
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse("""[{"plugin":"customers","calls":["GetCustomer"]}]"""), bare["dependencies"]));

        var d = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0")
            .DependsOn(Deps.Customers.Plugin, Deps.Customers.Calls)
            .Reads(Deps.Customers.Plugin, Deps.Customers.Reads)
            // reads of a plugin it does not call: an entry with no calls
            .Reads("catalog", [new DependencyRead("product", [new ReadField("id", "ref:catalog.product")])])
            .DescriptorJson())!;
        var dependencies = JsonNode.Parse("""
            [{"plugin":"customers","calls":["GetCustomer"],"reads":[{"entity":"customer","fields":[
                {"name":"id","type":"ref:customers.customer"},{"name":"name","type":"string"},
                {"name":"email","type":"optional string"}]}]},
             {"plugin":"catalog","calls":[],"reads":[{"entity":"product","fields":[{"name":"id","type":"ref:catalog.product"}]}]}]
            """);
        Assert.True(JsonNode.DeepEquals(dependencies, d["dependencies"]), d["dependencies"]!.ToJsonString());
        Assert.Null(d["statements"]); // SQL is sent as text: nothing of it is in the descriptor
        Assert.Empty(d["entities"]!.AsArray());

        // DependsOn after Reads fills the entry Reads made
        var late = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0")
            .Reads(Deps.Customers.Plugin, Deps.Customers.Reads)
            .DependsOn(Deps.Customers.Plugin, Deps.Customers.Calls)
            .DescriptorJson())!;
        Assert.Single(late["dependencies"]!.AsArray());
        Assert.Equal("GetCustomer", late["dependencies"]![0]!["calls"]![0]!.GetValue<string>());
    }

    [Fact]
    public void EntityMessagesComeAfterTheEndpointsAndBeforeTheExtraOnes()
    {
        var d = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0")
            .Message<Props>()
            .Entity<Thing>()
            .Endpoint<PlaceOrder>()
            .Message<Tally>()
            .DescriptorJson())!;
        Assert.Equal(["PlaceOrderRequest", "PlaceOrderReply", "Thing", "Props", "Tally"],
            d["messages"]!.AsArray().Select(m => m!["name"]!.GetValue<string>()));
    }

    [Fact]
    public void AnotherPluginsEntityIsNeverThisPluginsEntity()
    {
        var d = JsonNode.Parse(Plugin.Create("shop").Version("1.0.0").Endpoint<ReadCustomer>().DescriptorJson())!;
        // a message the endpoint answers with, not an entity of the plugin; its id refers to the provider's entity
        Assert.Empty(d["entities"]!.AsArray());
        var customer = d["messages"]!.AsArray().Single(m => m!["name"]!.GetValue<string>() == "Customer")!;
        Assert.Equal("ref:customers.customer", customer["fields"]![0]!["type"]!.GetValue<string>());
        Assert.Equal("ref:customers.customer", d["messages"]![0]!["fields"]![0]!["type"]!.GetValue<string>());

        var errors = Plugin.Create("shop").Version("1.0.0").Entity<Deps.Customers.Customer>().Check();
        Assert.Equal(["E_ENT_NAME entities.Customer"], errors.Select(e => $"{e.Code} {e.Path}"));
    }

    [Message] public sealed record ReadCustomerRequest(Id<Deps.Customers.Customer> Id);

    public sealed class ReadCustomer : Endpoint<ReadCustomerRequest, Deps.Customers.Customer>
    {
        public override async Task<Deps.Customers.Customer> Handle(Ctx ctx, ReadCustomerRequest req) =>
            await ctx.Query<Deps.Customers.Customer>().Get(req.Id);
    }

    [Theory]
    [InlineData("OwnerId", "owner_id")]
    [InlineData("HTTPStatus", "http_status")]
    [InlineData("Line2Total", "line2_total")]
    [InlineData("Id", "id")]
    [InlineData("OrderLine", "order_line")]
    public void SnakeCase(string pascal, string snake) => Assert.Equal(snake, Names.Snake(pascal));

    [Message] public sealed record Unsupported(Dictionary<string, string> Tags);
    [Message] public sealed record Bad(string X);
    [Message] public sealed record TooSlowRequest(string X);

    public sealed class BadName : Endpoint<Bad, Bad>
    {
        public override Task<Bad> Handle(Ctx ctx, Bad req) => Task.FromResult(req);
    }

    public sealed class TooSlow : Endpoint<TooSlowRequest, Bad>
    {
        public override void Configure(EndpointOptions e) =>
            e.Timeout(TimeSpan.FromMinutes(11)).Roles(new Role("ghost")).Scopes(new Scope("export"));

        public override Task<Bad> Handle(Ctx ctx, TooSlowRequest req) => Task.FromResult(new Bad(req.X));
    }
}
