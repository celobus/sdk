// The UI builders: the JSON each one writes (the shape of crates/descriptor/src/ui.rs), the shop conformance plugin
// against its fixture, and the descriptor fields that are written only when declared.
using System.Text.Json.Nodes;
using Celobus.Sdk.Ui;
using Shop;
using U = Celobus.Sdk.Ui.Ui;

namespace Celobus.Sdk.Tests;

[Message] public sealed record Customer(string Name, string Email);

[Message]
public sealed record Invoice(
    Guid Id, string Number, decimal Amount, string Currency, Status State, long Count, DateTimeOffset At,
    Customer Owner, IReadOnlyList<string> Labels, double Share, Guid? Parent);

[Message] public sealed record ListInvoicesRequest(Status? State, string Search, string PageToken, int PageSize, string After);
[Message] public sealed record ListInvoicesResponse(IReadOnlyList<Invoice> Invoices, string NextPageToken, string Cursor);
[Message] public sealed record PayInvoiceRequest(Guid Id, Status? State, long Count, bool Notify, Guid? Parent);
[Message] public sealed record InvoiceKey(Guid Id);
[Message] public sealed record NoKey;
[Message] public sealed record InvoicesParams(Status? State, string? Search);
[Message] public sealed record InvoiceParams(Guid Id, IReadOnlyList<string> Tags);
[Message] public sealed record TrendParams(string? Range, Status? Grain, int Top, Status? State);

[Message] public sealed record RunStep(string Text, string State);
[Message] public sealed record Run(string Agent, string Role, Status State, IReadOnlyList<RunStep> Steps);
[Message] public sealed record GetRunRequest(Guid Id);
[Message] public sealed record QuoteLine(Guid ProductId, int Quantity);
[Message] public sealed record QuoteRequest(IReadOnlyList<QuoteLine> Lines);
[Message] public sealed record QuotedLine(decimal Total);
[Message] public sealed record Quote(IReadOnlyList<QuotedLine> Lines, decimal Total);

public sealed class GetRun : Endpoint<GetRunRequest, Run>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();
    public override Task<Run> Handle(Ctx ctx, GetRunRequest req) => throw new NotSupportedException();
}

public sealed class QuoteOrder : Endpoint<QuoteRequest, Quote>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();
    public override Task<Quote> Handle(Ctx ctx, QuoteRequest req) => throw new NotSupportedException();
}

public sealed class ListInvoices : Endpoint<ListInvoicesRequest, ListInvoicesResponse>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();
    public override Task<ListInvoicesResponse> Handle(Ctx ctx, ListInvoicesRequest req) => throw new NotSupportedException();
}

public sealed class PayInvoice : Endpoint<PayInvoiceRequest, Invoice>
{
    public override void Configure(EndpointOptions e) => e.Ui()
        .Invalidates(UiTests.Invoices)
        .Invalidates(UiTests.InvoiceTopic.By(k => k.Id, (Invoice i) => i.Id));
    public override Task<Invoice> Handle(Ctx ctx, PayInvoiceRequest req) => throw new NotSupportedException();
}

public class UiTests
{
    public static readonly Topic<NoKey> Invoices = new("invoices");
    public static readonly Topic<InvoiceKey> InvoiceTopic = new("invoice");
    private static readonly ScreenKey<InvoicesParams> InvoicesScreen = new("invoices");
    private static readonly ScreenKey<InvoiceParams> InvoiceScreen = new("invoice_detail");
    private static readonly OverlayKey<InvoiceParams> PayDialog = new("pay");

    private static JsonNode J(object x) => UiJson.Of(x, new UiWriter())!;

    private static JsonNode P(string json) => JsonNode.Parse(json)!;

    private static void Same(string expected, JsonNode? actual) =>
        Assert.True(JsonNode.DeepEquals(P(expected), actual), actual?.ToJsonString());

    /// <summary>A screen with the given layout and the pieces <paramref name="build"/> declares; its JSON.</summary>
    private static JsonNode ScreenJson(Func<ScreenBuilder<InvoicesParams>, Node> build) =>
        J(Screen.Build(InvoicesScreen, build).Obj);

    [Fact]
    public void ShopPluginMatchesItsFixture()
    {
        var plugin = ShopPlugin.Define();
        Assert.Empty(plugin.Check());
        var actual = JsonNode.Parse(plugin.DescriptorJson());
        var expected = JsonNode.Parse(System.IO.File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "conformance", "shop.json")));
        Assert.True(JsonNode.DeepEquals(expected, actual), actual!.ToJsonString());
    }

    [Fact]
    public void UiFieldsAreWrittenOnlyWhenDeclared()
    {
        var d = JsonNode.Parse(Plugin.Create("plain").Version("1.0.0").Endpoint<PlaceOrder>().DescriptorJson())!.AsObject();
        Assert.False(d.ContainsKey("topics"));
        Assert.False(d.ContainsKey("ui"));
        var e = d["endpoints"]![0]!.AsObject();
        Assert.False(e.ContainsKey("ui") || e.ContainsKey("read_only") || e.ContainsKey("invalidates"));

        // read_only is a UI fact: a read-only endpoint outside the UI describes as before
        var hello = JsonNode.Parse(Hello.HelloPlugin.Define().DescriptorJson())!;
        Assert.False(hello["endpoints"]![1]!.AsObject().ContainsKey("read_only"));
    }

    [Fact]
    public void EndpointsTopicsAndTheirKeysAreDescribed()
    {
        var d = JsonNode.Parse(Plugin.Create("billing").Version("1.0.0")
            .Endpoint<ListInvoices>()
            .Endpoint<PayInvoice>()
            .Message<Line>()
            .Topic(Invoices.Decl().Scopes(Scope.Read).Manual().Describe("Any invoice."))
            .Topic(InvoiceTopic.Roles(Role.WorkspaceAdmin).Authorize(ListInvoices.Ref))
            .DescriptorJson())!;
        Same("""
            {"auth":"user","description":"","idempotency":"off","input":"ListInvoicesRequest","name":"ListInvoices",
            "output":"ListInvoicesResponse","read_only":true,"roles":[],"scopes":["read"],"timeout_ms":30000,"ui":true}
            """,
            d["endpoints"]![0]);
        Same("""[{"topic":"invoices"},{"topic":"invoice","key":[{"field":"id","from":"id"}]}]""", d["endpoints"]![1]!["invalidates"]);
        Assert.False(d["endpoints"]![1]!.AsObject().ContainsKey("read_only"));
        Same("""
            [{"name":"invoices","key":"NoKey","description":"Any invoice.","scopes":["read"],"manual":true},
            {"name":"invoice","key":"InvoiceKey","description":"","roles":["workspace.admin"],"authorize":"ListInvoices"}]
            """,
            d["topics"]);
        // the topics' keys come after the extra messages
        var names = d["messages"]!.AsArray().Select(m => m!["name"]!.GetValue<string>()).ToList();
        Assert.Equal(["Line", "NoKey", "InvoiceKey"], names[^3..]);
        Assert.Equal("ListInvoices", ListInvoices.Ref.Name);
    }

    [Fact]
    public void ValuesAndLiterals()
    {
        Same("""{"lit":"42"}""", J(U.Lit(42L)));
        Same("""{"lit":42}""", J(U.Lit(42)));
        Same("""{"lit":"1.50"}""", J(U.Lit(1.50m)));
        Same("""{"lit":"on_hold"}""", J(U.Lit(Status.OnHold)));
        Same("""{"lit":true}""", J(U.Lit(true)));
        Same("""{"lit":"00000000-0000-0000-0000-000000000001"}""", J(U.Lit(Guid.Parse("00000000-0000-0000-0000-000000000001"))));
        Same("""{"caller":"user_id"}""", J(U.CallerId()));
        Same("""{"caller":"display_name"}""", J(U.CallerName()));
        Same("""{"error":"code"}""", J(U.ErrorCode()));
        Same("""{"error":"message"}""", J(U.ErrorMessage()));
        Same("""{"result":"owner.name"}""", J(U.Result((Invoice i) => i.Owner.Name)));
        Same("""{"row":"parent"}""", J(new Row<Invoice>().Get(i => i.Parent!.Value)));
        Same("""{"form":"notify"}""", J(new FormVals<PayInvoiceRequest>().Get(r => r.Notify)));
        Same("""{"data":{"query":"q","field":"owner.email"}}""", J(new Q<Invoice, Invoice>("q").Field(i => i.Owner.Email)));
        Same("""{"data":{"query":"q","field":"a.b"}}""", J(new Q<Invoice, Invoice>("q").Path<string>("a.b")));
        Assert.Throws<ArgumentException>(() => new Row<Invoice>().Get(i => i.Number.Length + 1));
    }

    [Fact]
    public void Conditions()
    {
        var row = new Row<Invoice>();
        var state = row.Get(i => i.State);
        Same("""{"eq":[{"row":"state"},{"lit":"placed"}]}""", J(state.Eq(Status.Placed)));
        Same("""{"ne":[{"row":"state"},{"lit":"on_hold"}]}""", J(state.Ne(Status.OnHold)));
        Same("""{"gt":[{"row":"count"},{"lit":"3"}]}""", J(row.Get(i => i.Count).Gt(3)));
        Same("""{"lt":[{"row":"share"},{"lit":0.5}]}""", J(row.Get(i => i.Share).Lt(0.5)));
        Same("""{"empty":{"row":"number"}}""", J(row.Get(i => i.Number).Empty()));
        Same("""{"in":{"value":{"row":"state"},"set":["placed","on_hold"]}}""", J(state.In(Status.Placed, Status.OnHold)));
        Same("""{"and":[{"empty":{"row":"number"}},{"not":{"empty":{"row":"currency"}}}]}""",
            J(row.Get(i => i.Number).Empty() & !row.Get(i => i.Currency).Empty()));
        Same("""{"or":[{"empty":{"row":"number"}},{"empty":{"row":"currency"}}]}""",
            J(row.Get(i => i.Number).Empty().Or(row.Get(i => i.Currency).Empty())));
        // an optional value compares with a value of its inner type
        Same("""{"eq":[{"row":"parent"},{"lit":null}]}""", J(row.Get(i => i.Parent).Eq(null!)));
    }

    [Fact]
    public void ParamsStateAndQueries()
    {
        var j = ScreenJson(s =>
        {
            var state = s.Param(p => p.State);
            var search = s.Param(p => p.Search);
            var view = s.State("view", "list");
            s.State("page", 2L);
            s.Query("invoices", Query.Call(ListInvoices.Ref)
                .Arg(r => r.State, state)
                .Arg(r => r.Search, search!)
                .Arg(r => r.PageSize, 50)
                .Rows(r => r.Invoices)
                .Feed(r => r.After, r => r.Cursor, i => i.Id)
                .Follow(Invoices)
                .Follow(InvoiceTopic.Key(k => k.Set(x => x.Id, Guid.Empty)))
                .Poll(30)
                .MaxAge(60));
            s.Query("placed", Query.Call(ListInvoices.Ref).Arg(r => r.State, Status.Placed).Paged());
            s.Query("count", Query.Call(PayInvoice.Ref).Arg(r => r.State, state.OrEmpty()));
            return U.Segmented(view, ("list", "List"), ("board", "Board"));
        });
        Same("""[{"name":"state","type":"enum:Status","required":false},{"name":"search","type":"string","required":false}]""", j["params"]);
        Same("""[{"name":"view","type":"string","default":"list"},{"name":"page","type":"int64","default":"2"}]""", j["state"]);
        Same("""
            [{"name":"invoices","endpoint":"ListInvoices",
              "args":[{"field":"state","value":{"param":"state"}},{"field":"search","value":{"param":"search"}},
                      {"field":"page_size","value":{"lit":50}}],
              "rows":"invoices","feed":{"after":"after","cursor":"cursor","key":"id"},
              "follow":[{"topic":"invoices"},
                        {"topic":"invoice","key":[{"field":"id","value":{"lit":"00000000-0000-0000-0000-000000000000"}}]}],
              "poll":30,"max_age":60},
             {"name":"placed","endpoint":"ListInvoices","args":[{"field":"state","value":{"lit":"placed"}}],"paged":true},
             {"name":"count","endpoint":"PayInvoice","args":[{"field":"state","value":{"param":"state"}}]}]
            """
                , j["queries"]);
                Same(
                """{"segmented":{"bind":{"state":"view"},"options":[{"value":"list","label":"List"},{"value":"board","label":"Board"}]}}""",
            j["layout"]);
        // no title: the id, humanized
        Assert.Equal("Invoices", j["title"]!.GetValue<string>());
        Assert.False(j.AsObject().ContainsKey("actions"));
    }

    [Fact]
    public void ReportScreensDefaultsSwitchesBreadcrumbsAndCommandsWithArgs()
    {
        var trend = new ScreenKey<TrendParams>("trend");
        var j = J(Screen.Build(trend, s =>
        {
            // a param with a default always has a value: its handle is of the plain type
            ParamRef<string> range = s.ParamOr(p => p.Range, "30");
            ParamRef<Status> grain = s.ParamOr(p => p.Grain, Status.Placed);
            ParamRef<int> top = s.ParamOr(p => p.Top, 5);
            var state = s.Param(p => p.State);
            var view = s.State("view", "chart");
            var invoices = s.Query("invoices", Query.Call(ListInvoices.Ref).Arg(r => r.Search, range).Rows(r => r.Invoices));
            s.Action("week", a => a.SetParam(range, "7"));
            return U.Stack(
                U.Segmented(range, ("7", "7 days"), ("30", "30 days")),
                U.Segmented(grain, (Status.Placed, "Placed"), (Status.OnHold, U.Text("trend.hold", "On hold"))),
                U.Segmented(top, (5, "Top 5"), (10, "Top 10")),
                U.Segmented(state, (Status.Placed, "Placed"), (Status.OnHold, "On hold")),
                U.Segmented(view, ("chart", "Chart"), ("table", "Table")),
                Viz.Bar.Bar().Labels(invoices, i => i.Number)
                    .Open(trend, a => a.Arg(p => p.Range, "7").Arg(p => p.State, Status.OnHold).Arg(p => p.Grain, grain)),
                U.Link("On hold", trend, a => a.Arg(p => p.State, Status.OnHold).Arg(p => p.Range, range)),
                U.Breadcrumbs(
                    U.Crumb("Trend", trend),
                    U.Crumb(U.Text("trend.hold", "On hold"), trend, a => a.Arg(p => p.State, Status.OnHold).Arg(p => p.Grain, grain)),
                    U.Crumb("Week")));
        }).Obj);
        // a default makes a param optional, whatever its field is
        Same("""
            [{"name":"range","type":"string","required":false,"default":"30"},
             {"name":"grain","type":"enum:Status","required":false,"default":"placed"},
             {"name":"top","type":"int32","required":false,"default":5},
             {"name":"state","type":"enum:Status","required":false}]
            """, j["params"]);
        Same("""[{"field":"search","value":{"param":"range"}}]""", j["queries"]![0]!["args"]);
        var nodes = j["layout"]!["stack"]!["children"]!;
        Same("""{"bind":{"param":"range"},"options":[{"value":"7","label":"7 days"},{"value":"30","label":"30 days"}]}""",
            nodes[0]!["segmented"]);
        Same("""{"bind":{"param":"grain"},"options":[{"value":"placed","label":"Placed"},{"value":"on_hold","label":{"key":"trend.hold"}}]}""",
            nodes[1]!["segmented"]);
        Same("""{"bind":{"param":"top"},"options":[{"value":5,"label":"Top 5"},{"value":10,"label":"Top 10"}]}""", nodes[2]!["segmented"]);
        Same("""{"bind":{"param":"state"},"options":[{"value":"placed","label":"Placed"},{"value":"on_hold","label":"On hold"}]}""",
            nodes[3]!["segmented"]);
        Same("""{"state":"view"}""", nodes[4]!["segmented"]!["bind"]);
        Same("""
            {"screen":"trend","args":[{"field":"range","value":{"lit":"7"}},{"field":"state","value":{"lit":"on_hold"}},
                                      {"field":"grain","value":{"param":"grain"}}]}
            """, nodes[5]!["viz"]!["open"]);
        Same("""{"screen":"trend","args":[{"field":"state","value":{"lit":"on_hold"}},{"field":"range","value":{"param":"range"}}]}""",
            nodes[6]!["link"]!["target"]);
        // a breadcrumb trail: crumbs that link to screens, with args like a link, and the last one a text
        Same("""
            {"items":[{"text":"Trend","target":{"screen":"trend"}},
                     {"text":{"key":"trend.hold"},"target":{"screen":"trend","args":[
                        {"field":"state","value":{"lit":"on_hold"}},{"field":"grain","value":{"param":"grain"}}]}},
                     {"text":"Week"}]}
            """, nodes[7]!["breadcrumbs"]);

        var ui = new PluginUi()
            .Command(Command.Navigate("trend", "Trend", trend))
            .Command(Command.Navigate("trend_week", "Trend: last 7 days", trend,
                a => a.Arg(p => p.Range, "7").Arg(p => p.Grain, Status.OnHold)).Icon("chart-line"))
            .Command(Command.Open("pay", "Pay an invoice", PayDialog, a => a.Arg(p => p.Id, Guid.Empty)))
            .Build();
        Same("""
            [{"id":"trend","title":"Trend","target":{"screen":"trend"}},
             {"id":"trend_week","title":"Trend: last 7 days","icon":"chart-line","target":{"screen":"trend","args":[
                {"field":"range","value":{"lit":"7"}},{"field":"grain","value":{"lit":"on_hold"}}]}},
             {"id":"pay","title":"Pay an invoice","target":{"overlay":"pay","args":[
                {"field":"id","value":{"lit":"00000000-0000-0000-0000-000000000000"}}]}}]
            """, ui["commands"]);
    }

    [Fact]
    public void ActionChains()
    {
        var j = ScreenJson(s =>
        {
            var view = s.State("view", "list");
            var state = s.Param(p => p.State);
            var q = s.Query("invoices", Query.Call(ListInvoices.Ref).Rows(r => r.Invoices));
            s.RowAction("pay", q, (a, r) => a
                .Confirm(new Confirm("Pay it?").ConfirmLabel("Pay").Typed(r.Get(i => i.Number)))
                .Call(PayInvoice.Ref, c => c
                    .Arg(x => x.Id, r.Get(i => i.Id))
                    .Arg(x => x.Parent, r.Get(i => i.Id))
                    .Arg(x => x.State, Status.Placed)
                    .Arg(x => x.Count, 2L))
                .Pending(q)
                .Toast("Paid", Tone.Ok)
                .Navigate(InvoiceScreen, p => p.Arg(x => x.Id, U.Result((Invoice i) => i.Id)))
                .OnError(e => e.Toast(U.ErrorMessage()).Refresh("invoices").Back()));
            s.Action("misc", a => a
                .Open(PayDialog)
                .Close()
                .Set(view, "board")
                .SetParam(state, Status.OnHold)
                .Refresh()
                .OpenUrl("https://example.com"));
            return U.Divider();
        });
        Same("""
            [{"name":"pay",
              "steps":[
                {"confirm":{"title":"Pay it?","confirm_label":"Pay","typed":{"row":"number"}}},
                {"call":{"endpoint":"PayInvoice","pending":"invoices","args":[
                   {"field":"id","value":{"row":"id"}},{"field":"parent","value":{"row":"id"}},
                   {"field":"state","value":{"lit":"placed"}},{"field":"count","value":{"lit":"2"}}]}},
                {"toast":{"text":"Paid","tone":"ok"}},
                {"navigate":{"screen":"invoice_detail","args":[{"field":"id","value":{"result":"id"}}]}}],
              "on_error":[{"toast":{"text":{"error":"message"}}},{"refresh":{"queries":["invoices"]}},{"back":{}}]},
             {"name":"misc",
              "steps":[{"open":{"overlay":"pay"}},{"close":{}},{"set":{"state":"view","value":{"lit":"board"}}},
                       {"set":{"param":"state","value":{"lit":"on_hold"}}},{"refresh":{}},
                       {"open_url":{"value":{"lit":"https://example.com"}}}]}]
            """
                , j["actions"]);
                Same(
                """{"divider":{}}""", j["layout"]);
    }

    [Fact]
    public void TableColumnsAndOptions()
    {
        var j = ScreenJson(s =>
        {
            var q = s.Query("invoices", Query.Call(ListInvoices.Ref).Rows(r => r.Invoices));
            var selected = s.State("selected", Guid.Empty);
            var open = s.Action("open", a => a.Back());
            return U.Table(q)
                .Col(i => i.Owner.Name, "Owner")
                .Mono(i => i.Number, "Number")
                .Number(i => i.Count, "Count")
                .Money(i => i.Amount, "Amount", i => i.Currency)
                .MoneyIn(i => i.Amount, "USD", "USD")
                .Date(i => i.At, "Date")
                .Datetime(i => i.At, "When")
                .Relative(i => i.At, "Ago")
                .Member(i => i.Owner.Email, "Member")
                .Avatar(i => i.Owner.Name, "Avatar")
                .Tags(i => i.Labels, "Labels")
                .Percent(i => i.Share, "Share")
                .Status(i => i.State, "State", (Status.OnHold, Tone.Attention))
                .Open(open)
                .Toolbar(U.Button("Refresh", open))
                .Empty("No invoices")
                .Select(selected, i => i.Id)
                .RowTone(r => r.Get(i => i.State).Eq(Status.OnHold), Tone.Danger)
                .RowButton(_ => U.Menu(U.MenuItem("Open", open).Icon("eye").Danger().When(U.Not(new Row<Invoice>().Get(i => i.Number).Empty()))));
        });
        Same("""
            {"table":{"query":"invoices",
              "columns":[
                {"field":"owner.name","title":"Owner"},
                {"field":"number","title":"Number","format":{"mono":{}}},
                {"field":"count","title":"Count","format":{"number":{}},"align":"end"},
                {"field":"amount","title":"Amount","format":{"money":{"currency":"currency"}},"align":"end"},
                {"field":"amount","title":"USD","format":{"money":{"code":"USD"}},"align":"end"},
                {"field":"at","title":"Date","format":{"date":{}}},
                {"field":"at","title":"When","format":{"datetime":{}}},
                {"field":"at","title":"Ago","format":{"relative":{}}},
                {"field":"owner.email","title":"Member","format":{"member":{}}},
                {"field":"owner.name","title":"Avatar","format":{"avatar":{}}},
                {"field":"labels","title":"Labels","format":{"tags":{}}},
                {"field":"share","title":"Share","format":{"percent":{}}},
                {"field":"state","title":"State","format":{"status":{"tones":{"on_hold":"attention"}}}}],
              "open":"open",
              "toolbar":[{"button":{"label":"Refresh","action":"open","emphasis":"secondary"}}],
              "empty":"No invoices",
              "select":{"state":"selected","key":"id"},
              "row_tone":[{"when":{"eq":[{"row":"state"},{"lit":"on_hold"}]},"tone":"danger"}],
              "row_actions":[{"menu":{"icon":"more-horizontal","items":[
                 {"label":"Open","action":"open","icon":"eye","danger":true,"visible":{"not":{"empty":{"row":"number"}}}}]}}]}}
            """
                , j["layout"]);
                }
                [Fact]
                public void StructureAndDisplayPrimitives()
                {
                var j = ScreenJson(s =>
                {
                var q = s.Query("invoices", Query.Call(ListInvoices.Ref).Rows(r => r.Invoices));
                var one = new Q<Invoice, Invoice>("one");
                var go = s.Action("go", a => a.Back());
                return U.Stack(
                U.Row(U.Tag("New"), U.Avatar(one.Field(i => i.Owner.Name)), U.AvatarStack(one.Field(i => i.Labels))).Gap(Gap.Tight),
                U.Columns(U.Card(U.Text("Body")).Title("Card"), U.Divider()).Ratios(1, 2),
                U.Section("Totals",
                U.Metric("Amount", one.Field(i => i.Amount)).MoneyIn("EUR").Delta(one.Field(i => i.Share)).Caption("vs last month"),
                U.Meter(one.Field(i => i.Count), U.Lit(10L)).Caption("of 10"),
                U.Progress(one.Field(i => i.Share)))
                .Description("This month")
                .Actions(U.IconButton("plus", "Add", go))
                .Footer(U.Link("All", InvoiceScreen, p => p.Arg(x => x.Id, one.Field(i => i.Id)))),
                U.Tabs(U.Tab("a", "A", U.Heading("H")).Badge(one.Field(i => i.Count)), U.Tab("b", "B", U.Caption("C"), U.Mono("M")))
                    .BindParam(s.Param(p => p.Search)),
                U.Split(
                U.List(q, i => i.Number).Subtitle(i => i.Owner.Name).MetaRelative(i => i.At).Avatar(i => i.Owner.Email)
                .Icon("file").Status(i => i.State, (Status.Placed, Tone.Neutral)).Open(go).Empty("None"),
                U.Fields(U.Item("At", one.Field(i => i.At)).Relative(), U.Item("State", one.Field(i => i.State)).Status(("placed", Tone.Ok)))
                .Title("Details")),
                U.BarChart(q, i => i.Number).Series(i => i.Amount, "Amount").Title("By number").MoneyIn("USD"),
                U.LineChart(q, i => i.At).Series(i => i.Count, "Count"),
                U.Timeline(q, i => i.Number, i => i.At).Actor(i => i.Owner.Email).Tone(i => i.State, (Status.OnHold, Tone.Attention)),
                U.Status(one.Field(i => i.State), (Status.Placed, Tone.Ok)),
                U.CopyField("Id", one.Field(i => i.Id)),
                U.Text(one.Field(i => i.Amount)).Number(),
                U.Empty("Nothing", "Nothing yet").Icon("inbox").WithAction(U.Button("Add", go).Primary().Icon("plus")),
                U.Notice(Tone.Attention, "Careful").Title("Note").WithAction(U.Button("Fix", go).Ghost()))
                .Visible(U.CallerId().Ne(""))
                .Enabled(one.Field(i => i.Count).Gt(0L))
                .Fallback(U.Text("Update the app"));
                });
                Same(
                """
            {"stack":{"direction":"vertical","gap":"normal","children":[
              {"stack":{"direction":"horizontal","gap":"tight","children":[
                {"tag":{"text":"New"}},{"avatar":{"value":{"data":{"query":"one","field":"owner.name"}}}},
                {"avatar_stack":{"value":{"data":{"query":"one","field":"labels"}},"max":5}}]}},
              {"columns":{"ratios":[1,2],"children":[
                {"card":{"title":"Card","children":[{"text":{"text":"Body","style":"body"}}]}},{"divider":{}}]}},
              {"section":{"title":"Totals","description":"This month",
                "actions":[{"icon_button":{"icon":"plus","label":"Add","action":"go"}}],
                "children":[
                  {"metric":{"title":"Amount","value":{"data":{"query":"one","field":"amount"}},"format":{"money":{"code":"EUR"}},
                             "delta":{"data":{"query":"one","field":"share"}},"caption":"vs last month"}},
                  {"meter":{"value":{"data":{"query":"one","field":"count"}},"max":{"lit":"10"},"caption":"of 10"}},
                  {"progress":{"value":{"data":{"query":"one","field":"share"}}}}],
                "footer":[{"link":{"text":"All","target":{"screen":"invoice_detail",
                  "args":[{"field":"id","value":{"data":{"query":"one","field":"id"}}}]}}}]}},
              {"tabs":{"bind":{"param":"search"},"tabs":[
                {"id":"a","title":"A","badge":{"data":{"query":"one","field":"count"}},"children":[{"text":{"text":"H","style":"heading"}}]},
                {"id":"b","title":"B","children":[{"text":{"text":"C","style":"caption"}},{"text":{"text":"M","style":"mono"}}]}]}},
              {"split":{
                "list":{"list":{"query":"invoices","title":"number","subtitle":"owner.name","meta":"at","meta_format":{"relative":{}},
                  "avatar":"owner.email","icon":"file","status":{"field":"state","tones":{"placed":"neutral"}},"open":"go","empty":"None"}},
                "detail":{"fields":{"title":"Details","items":[
                  {"label":"At","value":{"data":{"query":"one","field":"at"}},"format":{"relative":{}}},
                  {"label":"State","value":{"data":{"query":"one","field":"state"}},"format":{"status":{"tones":{"placed":"ok"}}}}]}}}},
              {"chart":{"kind":"bar","query":"invoices","x":"number","series":[{"field":"amount","label":"Amount"}],
                "title":"By number","format":{"money":{"code":"USD"}}}},
              {"chart":{"kind":"line","query":"invoices","x":"at","series":[{"field":"count","label":"Count"}]}},
              {"timeline":{"query":"invoices","text":"number","time":"at","actor":"owner.email",
                "tone":{"field":"state","tones":{"on_hold":"attention"}}}},
              {"status":{"value":{"data":{"query":"one","field":"state"}},"tones":{"placed":"ok"}}},
              {"copy_field":{"label":"Id","value":{"data":{"query":"one","field":"id"}}}},
              {"text":{"text":{"data":{"query":"one","field":"amount"}},"style":"body","format":{"number":{}}}},
              {"empty":{"title":"Nothing","text":"Nothing yet","icon":"inbox",
                "action":{"button":{"label":"Add","action":"go","emphasis":"primary","icon":"plus"}}}},
              {"notice":{"tone":"attention","title":"Note","text":"Careful",
                "action":{"button":{"label":"Fix","action":"go","emphasis":"ghost"}}}}]},
             "visible":{"ne":[{"caller":"user_id"},{"lit":""}]},
             "enabled":{"gt":[{"data":{"query":"one","field":"count"}},{"lit":"0"}]},
             "fallback":{"text":{"text":"Update the app","style":"body"}}}
            """
                , j["layout"]);
                }
    [Fact]
    public void ChartsAndFigures()
    {
        var j = ScreenJson(s =>
        {
            var search = s.Param(p => p.Search);
            var picked = s.State("picked", "");
            var q = s.Query("invoices", Query.Call(ListInvoices.Ref).Rows(r => r.Invoices));
            return U.Stack(
                // the contract's node: every source, in its order
                Viz.Line.Line()
                    .Title("Revenue")
                    .Range(U.Text("range.month", "Last 30 days"))
                    .Options(new JsonObject { ["fmt"] = "money" })
                    .Option("curve", "smooth")
                    .Dates(q, i => i.At)
                    .Values(q, i => i.Amount)
                    .Serie(q, i => i.Amount, "Amount")
                    .Pivot(q, i => i.At, i => i.State, i => i.Amount)
                    .AsDate()
                    .Rows(q, ("name", i => i.Number), ("value", i => i.Amount))
                    .TextKeys("name")
                    .Value("value", q.Field(r => r.Cursor))
                    .Height(280)
                    .SelectState(picked)
                    .Open(InvoicesScreen, a => a.Arg(p => p.State, Status.Placed)),
                // series of fields of different types, other destinations, a bind by hand, a fallback
                Viz.Bar.Column()
                    .Labels(q, i => i.Owner.Name)
                    .Series(q, (i => i.Amount, "Amount"), (i => i.Share, U.Text("viz.share", "Share")))
                    .Serie(q, i => i.Count, "Count")
                    .Column("targets", q, i => i.Share)
                    .Column("notes", q, i => i.Number)
                    .AsText()
                    .Rows(q, ("name", i => i.Number))
                    .To("parts")
                    .Bind("extra", P("""{"column":{"query":"invoices","field":"share"}}""").AsObject())
                    .SelectParam(search)
                    .Fallback(U.Table(q).Col(i => i.Number, "Number")),
                // one value: a query's field, a state, a param, literals
                Viz.Figure.Metric()
                    .Value("value", q.Field(r => r.Cursor))
                    .Value("center.value", picked)
                    .Value("search", search)
                    .Value("target", 12.5)
                    .Value("unit", "invoices"),
                U.Viz("meter", "meterList").Visible(picked.Empty()),
                Viz.Meter.MeterList());
        });
        var c = j["layout"]!["stack"]!["children"]!;
        Same(
            """
            {"viz":{"family":"line","kind":"line","title":"Revenue","range":{"key":"range.month"},
              "options":{"fmt":"money","curve":"smooth"},
              "bind":[
                {"to":"labels","from":{"column":{"query":"invoices","field":"at","as":"date"}}},
                {"to":"values","from":{"column":{"query":"invoices","field":"amount"}}},
                {"to":"series","from":{"series":{"query":"invoices","fields":[{"field":"amount","label":"Amount"}]}}},
                {"to":"series","from":{"pivot":{"query":"invoices","x":"at","series":"state","value":"amount","as":"date"}}},
                {"to":"rows","from":{"rows":{"query":"invoices","map":{"name":"number","value":"amount"},"text":["name"]}}},
                {"to":"value","from":{"value":{"data":{"query":"invoices","field":"cursor"}}}}],
              "height":280,
              "select":{"state":"picked"},
              "open":{"screen":"invoices","args":[{"field":"state","value":{"lit":"placed"}}]}}}
            """, c[0]);
        Same(
            """
            {"viz":{"family":"bar","kind":"column",
              "bind":[
                {"to":"labels","from":{"column":{"query":"invoices","field":"owner.name","as":"text"}}},
                {"to":"series","from":{"series":{"query":"invoices","fields":[
                  {"field":"amount","label":"Amount"},{"field":"share","label":{"key":"viz.share"}},
                  {"field":"count","label":"Count"}]}}},
                {"to":"targets","from":{"column":{"query":"invoices","field":"share"}}},
                {"to":"notes","from":{"column":{"query":"invoices","field":"number","as":"text"}}},
                {"to":"parts","from":{"rows":{"query":"invoices","map":{"name":"number"}}}},
                {"to":"extra","from":{"column":{"query":"invoices","field":"share"}}}],
              "select":{"param":"search"}},
             "fallback":{"table":{"query":"invoices","columns":[{"field":"number","title":"Number"}]}}}
            """, c[1]);
        Same(
            """
            {"viz":{"family":"figure","kind":"metric",
              "bind":[
                {"to":"value","from":{"value":{"data":{"query":"invoices","field":"cursor"}}}},
                {"to":"center.value","from":{"value":{"state":"picked"}}},
                {"to":"search","from":{"value":{"param":"search"}}},
                {"to":"target","from":{"value":{"lit":12.5}}},
                {"to":"unit","from":{"value":{"lit":"invoices"}}}]}}
            """, c[2]);
        Same("""{"viz":{"family":"meter","kind":"meterList"},"visible":{"empty":{"state":"picked"}}}""", c[3]);
        Same("""{"viz":{"family":"meter","kind":"meterList"}}""", c[4]);

        // a constructor per kind of the catalogue, from the table the validator reads
        Assert.Equal(10, Viz.Kinds.Count);
        Assert.Equal(83, Viz.Kinds.Values.Sum(k => k.Length));
        foreach (var (family, kinds) in Viz.Kinds)
        {
            var group = typeof(Viz).GetProperty(char.ToUpperInvariant(family[0]) + family[1..])!.GetValue(null)!;
            foreach (var kind in kinds)
            {
                var node = (VizNode)group.GetType().GetMethod(char.ToUpperInvariant(kind[0]) + kind[1..])!.Invoke(group, null)!;
                Assert.True(JsonNode.DeepEquals(new JsonObject { ["viz"] = new JsonObject { ["family"] = family, ["kind"] = kind } }, J(node)));
            }
        }
        Assert.Equal(83, typeof(Viz).GetProperties().Sum(p => p.PropertyType.GetMethods().Count(m => m.ReturnType == typeof(VizNode))));
        Same("""{"viz":{"family":"flow","kind":"funnel"}}""", J(Viz.Flow.Funnel()));
        Same("""{"viz":{"family":"bar","kind":"funnel"}}""", J(Viz.Bar.Funnel()));

        // a home widget binds its document-level query; a catalog text of a series registers itself
        var ui = new PluginUi().Home("chart", "Chart", w =>
        {
            var rows = w.Query("rows", Query.Call(ListInvoices.Ref).Rows(r => r.Invoices));
            return Viz.Part.Donut().Rows(rows, ("name", i => i.Number), ("value", i => i.Amount)).To("parts")
                .Serie(rows, i => i.Share, U.Text("viz.share", "Share"))
                .Open(InvoicesScreen);
        }).Build();
        Same(
            """
            {"viz":{"family":"part","kind":"donut",
              "bind":[
                {"to":"parts","from":{"rows":{"query":"home_chart_rows","map":{"name":"number","value":"amount"}}}},
                {"to":"series","from":{"series":{"query":"home_chart_rows","fields":[{"field":"share","label":{"key":"viz.share"}}]}}}],
              "open":{"screen":"invoices"}}}
            """, ui["home"]![0]!["node"]);
        Same("""{"en":{"viz.share":"Share"}}""", ui["texts"]);
        Assert.Equal(4, ui["catalog"]!.GetValue<int>());
    }

    [Fact]
    public void TimeTravelAndMaps()
    {
        TimeSpec<Invoice>? noAt = null;
        var j = ScreenJson(s =>
        {
            var search = s.Param(p => p.Search);
            var then = s.State("then", "");
            var width = s.State("width", 1);
            var upto = s.State("upto", 0);
            var picked = s.State("picked", "");
            var q = s.Query("invoices", Query.Call(ListInvoices.Ref).Arg(r => r.Search, search).Rows(r => r.Invoices));
            noAt = Time.Flow(q, i => i.Number, i => i.Currency);
            return U.Board()
                .Half(Viz.Bar.Bar()
                    .Rows(q, ("name", i => i.Number), ("value", i => i.Amount))
                    .Time(Time.Flow(q, i => i.Number, i => i.Currency)
                        .At(search)
                        .Track(i => i.Amount)
                        .Window(width)
                        .Compare(then)
                        .CompareBy(TimeCompare.Previous)
                        .Upto(upto)
                        .Control(TimeControl.Brush)
                        .Grain(TimeGrain.Quarter)))
                .Half(Viz.Geo.Map()
                    .Title("Deals")
                    .Connection("models.osm")
                    .Layer(MapLayer.Points(q).Lat(i => i.Share).Lon(i => i.Share).Value(i => i.Amount).Id(i => i.Id)
                        .Title("Deals").Option("size", true).SelectState(picked))
                    .Layer(MapLayer.Flows(q).From(i => i.Share, i => i.Share).To(i => i.Share, i => i.Share))
                    .Layer(MapLayer.Area().SelectParam(search)));
        });
        var items = j["layout"]!["board"]!["children"]!;
        Same(
            """
            {"read":"flow","control":"brush",
             "periods":{"query":"invoices","value":"number","label":"currency","track":"amount"},
             "at":{"param":"search"},"window":{"state":"width"},"then":{"state":"then"},
             "compare":"previous","upto":{"state":"upto"},"grain":"quarter"}
            """, items[0]!["node"]!["viz"]!["time"]);
        Same(
            """
            {"viz":{"family":"geo","kind":"map","title":"Deals","connection":"models.osm","layers":[
               {"type":"points","query":"invoices","fields":{"lat":"share","lon":"share","value":"amount","id":"id"},
                "title":"Deals","options":{"size":true},"select":{"state":"picked"}},
               {"type":"flows","query":"invoices",
                "fields":{"from_lat":"share","from_lon":"share","to_lat":"share","to_lon":"share"}},
               {"type":"area","select":{"param":"search"}}]}}
            """, items[1]!["node"]);
        // time travel names the shown period
        Assert.Throws<InvalidOperationException>(() => Viz.Bar.Bar().Time(noAt!));
    }

    [Fact]
    public void Dashboards()
    {
        var j = ScreenJson(s =>
        {
            var q = s.Query("invoices", Query.Call(ListInvoices.Ref).Rows(r => r.Invoices));
            VizNode Tile(string title) => Viz.Figure.Metric().Title(title).Value("value", q.Field(r => r.Cursor));
            return U.Board()
                .Quarter(Tile("A"))
                .Third(Tile("B"))
                .Half(Tile("C"))
                .Span(8, Viz.Line.Line().Dates(q, i => i.At).Values(q, i => i.Amount))
                .Span(12, U.Heading("Groups"))
                .Full(U.Table(q).Col(i => i.Number, "Number"))
                .Fallback(U.Text("Open the dashboard in a newer app"));
        });
        // the full width is the default span: it is left out
        Same(
            """
            {"board":{"children":[
               {"span":3,"node":{"viz":{"family":"figure","kind":"metric","title":"A",
                 "bind":[{"to":"value","from":{"value":{"data":{"query":"invoices","field":"cursor"}}}}]}}},
               {"span":4,"node":{"viz":{"family":"figure","kind":"metric","title":"B",
                 "bind":[{"to":"value","from":{"value":{"data":{"query":"invoices","field":"cursor"}}}}]}}},
               {"span":6,"node":{"viz":{"family":"figure","kind":"metric","title":"C",
                 "bind":[{"to":"value","from":{"value":{"data":{"query":"invoices","field":"cursor"}}}}]}}},
               {"span":8,"node":{"viz":{"family":"line","kind":"line","bind":[
                 {"to":"labels","from":{"column":{"query":"invoices","field":"at","as":"date"}}},
                 {"to":"values","from":{"column":{"query":"invoices","field":"amount"}}}]}}},
               {"node":{"text":{"text":"Groups","style":"heading"}}},
               {"node":{"table":{"query":"invoices","columns":[{"field":"number","title":"Number"}]}}}]},
             "fallback":{"text":{"text":"Open the dashboard in a newer app","style":"body"}}}
            """, j["layout"]);
        Same("""{"board":{"children":[]}}""", J(U.Board()));
    }

                [Fact]
                public void FormsFiltersAndOverlays()
                {
                var j = ScreenJson(s =>
                {
                var state = s.Param(p => p.State);
                var search = s.Param(p => p.Search);
                var q = s.Query("invoices", Query.Call(ListInvoices.Ref).Rows(r => r.Invoices));
                s.Overlay(Overlay.SidePanel(PayDialog, o =>
                {
                var id = o.Param(p => p.Id);
                o.Size(Size.Large).Subtitle("Pay one invoice");
                var close = o.Action("close", a => a.Close());
                o.Header(U.Button("Close", close));
                return o.Form(PayInvoice.Ref,
                f =>
                {
                f.Select(r => r.State, "State").Required().Help("Pick one");
                f.Section("More");
                f.Number(r => r.Count, "Count").Placeholder("1").Initial(U.Lit(1L));
                f.Checkbox(r => r.Notify, "Notify");
                f.Switch(r => r.Notify, "Notify again");
                f.Radio(r => r.State, "State again");
                f.SelectFrom(r => r.Parent, "Parent", q, i => i.Id, i => i.Number);
                f.Text(r => r.Id, "Id").Option("x", "X");
                },
                "Pay",
                (a, v) => a.CallForm(PayInvoice.Ref, c => c.Arg(r => r.Id, id).Arg(r => r.Count, v.Get(r => r.Count))).Close());
                }));
                return U.FilterBar(state.Select("State"), search.Search("Search").Placeholder("Number"), search.Date("Day"),
                    search.Choices("Any of").Option("a", "A").Option("b", "B"));
                });
                Same(
                """
            {"filter_bar":{"inputs":[
              {"name":"state","control":"select","label":"State",
               "options":[{"value":"placed","label":"Placed"},{"value":"on_hold","label":"On hold"}]},
              {"name":"search","control":"search","label":"Search","placeholder":"Number"},
              {"name":"search","control":"date","label":"Day"},
              {"name":"search","control":"multi_select","label":"Any of",
               "options":[{"value":"a","label":"A"},{"value":"b","label":"B"}]}]}}
            """
                , j["layout"]);
                Same(
                """
            [{"id":"pay","kind":"side_panel","size":"large","title":"Pay","subtitle":"Pay one invoice",
              "params":[{"name":"id","type":"uuid","required":true},{"name":"tags","type":"string","required":false}],
              "actions":[
                {"name":"close","steps":[{"close":{}}]},
                {"name":"submit_1","steps":[
                  {"call":{"endpoint":"PayInvoice","form":true,"args":[
                    {"field":"id","value":{"param":"id"}},{"field":"count","value":{"form":"count"}}]}},
                  {"close":{}}]}],
              "buttons":[{"button":{"label":"Close","action":"close","emphasis":"secondary"}}],
              "layout":{"form":{"endpoint":"PayInvoice","submit":"submit_1","submit_label":"Pay","sections":[
                {"inputs":[{"name":"state","control":"select","label":"State","required":true,"help":"Pick one",
                  "options":[{"value":"placed","label":"Placed"},{"value":"on_hold","label":"On hold"}]}]},
                {"title":"More","inputs":[
                  {"name":"count","control":"number","label":"Count","placeholder":"1","initial":{"lit":"1"}},
                  {"name":"notify","control":"checkbox","label":"Notify"},
                  {"name":"notify","control":"switch","label":"Notify again"},
                  {"name":"state","control":"radio_cards","label":"State again",
                   "options":[{"value":"placed","label":"Placed"},{"value":"on_hold","label":"On hold"}]},
                  {"name":"parent","control":"select","label":"Parent",
                   "options_from":{"query":"invoices","value":"id","label":"number"}},
                  {"name":"id","control":"text","label":"Id","options":[{"value":"x","label":"X"}]}]}]}}}]
            """
                , j["overlays"]);
                }
    [Fact]
    public void RecordsAgentRunsAndTheOptionsOfCatalogOneNodes()
    {
        var j = ScreenJson(s =>
        {
            var search = s.Param(p => p.Search);
            var check = s.State("checked", new List<Guid>());
            var q = s.Query("invoices", Query.Call(ListInvoices.Ref).Rows(r => r.Invoices).When(search.Empty()));
            var run = s.Query("run", Query.Call(GetRun.Ref).Arg(r => r.Id, Guid.Empty));
            var go = s.Action("go", a => a.SetParam(search, U.None<string?>()));
            s.Overlay(Overlay.Dialog(PayDialog, o =>
            {
                o.Kicker("BILLING");
                var quote = o.Query("quote", Query.Call(QuoteOrder.Ref)
                    .Arg(r => r.Lines, U.FormValue((QuoteRequest r) => r.Lines))
                    .Rows(r => r.Lines));
                return o.Form(QuoteOrder.Ref,
                    f =>
                    {
                        f.Repeat(r => r.Lines, "Lines", row => row
                                .SelectFrom(l => l.ProductId, "Product", q, i => i.Id, i => i.Number)
                                .Stepper(l => l.Quantity, "Qty", 1))
                            .Aside("Total", quote, l => l.Total, "USD")
                            .AddLabel("Add a line")
                            .Half();
                        f.Summary("Total", "with tax", quote.Field(r => r.Total), "USD");
                    },
                    "Place",
                    (a, _) => a.CallForm(QuoteOrder.Ref).Close());
            }));
            return U.Stack(
                U.Row(U.Label("ACTIVITY"), U.MenuButton("View", "eye", U.MenuItem("Go", go))).Between(),
                U.Metric("Due", U.Lit(1)).Caption("late").CaptionTone(Tone.Danger, "alert"),
                U.Table(q).Col(i => i.Number, "Number").Sortable((ListInvoicesRequest r) => r.After).Checks(check, i => i.Id).SelectedBy(search, i => i.Number)
                    .Compact(_ => U.RowView<Invoice>("Plain").Icon("box").Avatar(i => i.Number)),
                U.List(q, i => i.Number).Meta(i => i.Amount).MoneyIn("USD").IconField(i => i.State)
                    .Aside(U.Text("items.times", "×{n}").Arg("n", U.RowValue((Invoice i) => i.Share))),
                U.Timeline(q, i => i.Number, i => i.At).Avatar(i => i.Owner.Email).Icon(i => i.State).ShortTime(),
                U.Fields(
                    U.Item("Owner", U.Lit("x")).ChipAvatar(U.Lit("HW")).Link(InvoicesScreen, p => p.Arg(x => x.State, Status.Placed)),
                    U.Item("Other", U.Lit("y")).Chip()),
                U.FilterBar(search.Search("Find").Icon("search")),
                U.AgentRun(run, r => r.Agent, r => r.State, [(Status.OnHold, Tone.Attention)], r => r.Steps, x => x.Text, x => x.State)
                    .Subtitle(r => r.Role)
                    .Stop(go, run.Field(r => r.State).Eq(Status.OnHold))
                    .Message("Ask", go)
                    .Needs(r => r.Role, "Approve", go, r => r.Agent)
                    .Open("Open session", go)
                    .Changes("Changes", r => r.Steps, x => x.Text)
                    .Earlier(r => r.Role, go),
                U.RecordHeader("R").Short("#1").Compact("T", "S").Expand(go),
                U.Conversation(q, i => i.Number, i => i.Number, i => i.At)
                    .Badge(i => i.Number)
                    .Receipt(i => i.Number, i => i.State, (Status.OnHold, Tone.Attention))
                    .Card(i => i.Number, i => i.Number)
                    .CardButton(i => i.Number, go)
                    .Composer("Message", go)
                    .To("agent", "Agent"),
#pragma warning disable CS0618 // Ui.Chat is deprecated in catalog 3 and still written
                U.Chat(run, go).Thread(run, go).Run(run, go, go).Members(run, go).OpenThread(go, go).FollowThread(go).OpenRun(go)
                    .Approval(go, go, go).Card(go).React(go).Editing(go, go, go).Choose(go).About(run, go).Directory(run, go));
#pragma warning restore CS0618
        });
        Same("""{"empty":{"param":"search"}}""", j["queries"]![0]!["when"]);
        Same("""[{"set":{"param":"search","value":{"lit":null}}}]""", j["actions"]![0]!["steps"]);
        var c = j["layout"]!["stack"]!["children"]!;
        Same(
            """
            {"stack":{"direction":"horizontal","gap":"normal","justify":"between","children":[
              {"text":{"text":"ACTIVITY","style":"label"}},
              {"menu":{"label":"View","icon":"eye","items":[{"label":"Go","action":"go"}]}}]}}
            """, c[0]);
        Same("""{"title":"Due","value":{"lit":1},"caption":"late","caption_tone":"danger","caption_icon":"alert"}""", c[1]!["metric"]);
        Same(
            """
            {"query":"invoices","columns":[{"field":"number","title":"Number","sort":"after"}],
             "checks":{"state":"checked","key":"id"},"select":{"state":"search","key":"number"},
             "compact":{"title":"Plain","icon":"box","avatar":"number"}}
            """, c[2]!["table"]);
        Same(
            """
            {"query":"invoices","title":"number","meta":"amount","meta_format":{"money":{"code":"USD"}},
             "icon_field":"state","aside":{"key":"items.times","args":{"n":{"row":"share"}}}}
            """, c[3]!["list"]);
        Same(
            """
            {"query":"invoices","text":"number","time":"at","avatar":"owner.email","icon":"state",
             "time_format":{"relative":{"short":true}}}
            """, c[4]!["timeline"]);
        Same(
            """
            [{"label":"Owner","value":{"lit":"x"},"format":{"avatar":{}},"avatar":{"lit":"HW"},
              "open":{"screen":"invoices","args":[{"field":"state","value":{"lit":"placed"}}]}},
             {"label":"Other","value":{"lit":"y"},"format":{"avatar":{}}}]
            """, c[5]!["fields"]!["items"]);
        Assert.Equal("search", c[6]!["filter_bar"]!["inputs"]![0]!["icon"]!.GetValue<string>());
        Same(
            """
            {"query":"run","title":"agent","subtitle":"role","status":{"field":"state","tones":{"on_hold":"attention"}},
             "steps":"steps","step_text":"text","step_state":"state","stop":"go",
             "stop_when":{"eq":[{"data":{"query":"run","field":"state"}},{"lit":"on_hold"}]},
             "message":{"placeholder":"Ask","action":"go"},
             "needs":{"text":"role","label":"Approve","action":"go","due":"agent"},
             "open":{"label":"Open session","action":"go"},
             "changes":{"title":"Changes","field":"steps","text":"text"},
             "earlier":{"text":"role","action":"go"}}
            """, c[7]!["agent_run"]);
        Same("""{"title":"R","short":"#1","compact_title":"T","compact_subtitle":"S","expand":"go"}""", c[8]!["record_header"]);
        Same(
            """
            {"query":"invoices","author":"number","text":"number","time":"at","badge":"number",
             "receipt":{"text":"number","tone":{"field":"state","tones":{"on_hold":"attention"}}},
             "card":{"title":"number","text":"number","button":"number","action":"go"},
             "composer":{"placeholder":"Message","action":"go","to":[{"value":"agent","label":"Agent"}]}}
            """, c[9]!["conversation"]);
        Same(
            """
            {"channel":"run","actions":{"send":"go","reply":"go","stop":"go","tell":"go","members":"go","thread":"go",
             "close":"go","run":"go","approve":"go","decline":"go","undo":"go","card":"go","react":"go","edit":"go",
             "delete":"go","pin":"go","choose":"go","pause":"go","start":"go","follow":"go"},
             "thread":"run","run":"run","members":"run","about":"run","directory":"run"}
            """, c[10]!["chat"]);
        var o = j["overlays"]![0]!;
        Assert.Equal("BILLING", o["kicker"]!.GetValue<string>());
        Same("""[{"field":"lines","value":{"form":"lines"}}]""", o["queries"]![0]!["args"]);
        var form = o["layout"]!["form"]!;
        Same(
            """
            {"name":"lines","control":"repeat","label":"Lines","min_items":1,
             "fields":[
               {"name":"product_id","control":"select","label":"Product","required":true,
                "options_from":{"query":"invoices","value":"id","label":"number"}},
               {"name":"quantity","control":"number","label":"Qty","required":true,"initial":{"lit":1},"stepper":true}],
             "aside":{"title":"Total","query":"quote","field":"total","format":{"money":{"code":"USD"}}},
             "add_label":"Add a line","width":"half"}
            """, form["sections"]![0]!["inputs"]![0]);
        Same(
            """
            {"title":"Total","caption":"with tax","value":{"data":{"query":"quote","field":"total"}},
             "format":{"money":{"code":"USD"}}}
            """, form["summary"]);
    }

                [Fact]
                public void ContributionsAndTexts()
                {
                var count = Query.Call(ListInvoices.Ref).Follow(Invoices);
                var ui = new PluginUi()
                .Nav(Nav.Group(U.Text("nav.billing", "Billing")).Icon("receipt")
                .Entry(Entry.To(InvoicesScreen, "Open", p => p.Arg(x => x.State, Status.Placed))
                .Icon("inbox")
                .Badge(count, r => r.NextPageToken))
                .Entry(Entry.To(InvoicesScreen, "Again").Badge(count, r => r.NextPageToken)))
                .Command(Command.Open("pay", U.Text("cmd.pay", "Pay an invoice"), PayDialog))
                .Overlay(Overlay.Dialog(PayDialog, o => U.Text(U.Text("pay.body", "Pay it"))))
                .Home("due", "Due", w =>
                {
                var q = w.Query("list", Query.Call(ListInvoices.Ref).Rows(r => r.Invoices).Follow(Invoices));
                return U.List(q, i => i.Number);
                })
                .Text("de", "nav.billing", "Abrechnung")
                .Text("en", "cmd.pay", "Pay one");
                var j = ui.Build();
                Assert.Equal(4, j["catalog"]!.GetValue<int>());
                Same(
                """
            [{"title":{"key":"nav.billing"},"icon":"receipt","entries":[
              {"title":"Open","icon":"inbox","screen":"invoices","args":[{"field":"state","value":{"lit":"placed"}}],
               "badge":{"data":{"query":"badge_next_page_token","field":"next_page_token"}}},
              {"title":"Again","screen":"invoices","badge":{"data":{"query":"badge_next_page_token","field":"next_page_token"}}}]}]
            """
                , j["nav"]);
                // the badge query is lifted once; nav and home queries re-run at most every 2 s
                Same(
                """
            [{"name":"badge_next_page_token","endpoint":"ListInvoices","follow":[{"topic":"invoices","min_interval_ms":2000}]},
             {"name":"home_due_list","endpoint":"ListInvoices","rows":"invoices","follow":[{"topic":"invoices","min_interval_ms":2000}]}]
            """
                , j["queries"]);
                Same(
                """[{"id":"pay","title":{"key":"cmd.pay"},"target":{"overlay":"pay"}}]""", j["commands"]);
        Same("""[{"id":"due","title":"Due","node":{"list":{"query":"home_due_list","title":"number"}}}]""", j["home"]);
        Assert.Equal("dialog", j["overlays"]![0]!["kind"]!.GetValue<string>());
        // en: the catalog texts used, unless declared explicitly
        Same("""{"de":{"nav.billing":"Abrechnung"},"en":{"cmd.pay":"Pay one","nav.billing":"Billing","pay.body":"Pay it"}}""", j["texts"]);
        Assert.False(j.ContainsKey("screens"));

        // building twice gives the same document
        Assert.True(JsonNode.DeepEquals(j, ui.Build()));
        Same("""{"catalog":4}""", new PluginUi().Build());
    }
}
