// The SDK conformance plugin `shop`: a UI with a list screen (a compact row view, a count), a record screen
// (breadcrumbs, a record header, a titled card), a report screen of charts and figures on a `board` (`viz`: every bind source, a selection, a
// link, a fallback) under a range switch (a `Segmented` over a param with a default), a document-level dialog form (a
// note, an alternate button), a sidebar group with a live alert badge, two commands (one opens the report with args), a
// palette search source with an action and topics; an activity screen of catalog 3 (a fill split with a header, a
// stream of note posts with a packing job's process, a strip of live jobs, a composer, a side panel with a toggle and
// a searchable list) and an assign dialog with a pick input. Its `--describe` must equal ../descriptor.json, as the
// Rust and TypeScript versions' must. Only builds it, so the SDK's tests can load it without serving.
using System.Text.Json.Nodes;
using Celobus.Sdk;
using Celobus.Sdk.Ui;

namespace Shop;

public static class Topics
{
    public static readonly Topic<AllOrders> Orders = new("orders");
    public static readonly Topic<OrderKey> Order = new("order");
}

public static class Screens
{
    public static readonly ScreenKey<OrdersParams> Orders = new("orders");
    public static readonly ScreenKey<OrderParams> Order = new("order");
    public static readonly ScreenKey<ReportParams> Report = new("report");
    public static readonly OverlayKey<CancelParams> Cancel = new("cancel_form");
    public static readonly ScreenKey<ActivityParams> Activity = new("activity");
    public static readonly OverlayKey<AssignParams> Assign = new("assign");
}

public static class ShopPlugin
{
    public static readonly Role Clerk = new("clerk") { Title = "Order clerk" };

    public static Plugin Define() => Plugin.Create("shop")
        .Role(Clerk)
        .Agent("helper", a => a.Roles(Clerk).Title("Order helper").Describe("Answers questions about orders.").Icon("package")
            .Engine(AgentEngine.Codex).Model("gpt-5.5").Session().Timeout(TimeSpan.FromMinutes(15))
            .Skills("order-notes").Topic(Topics.Orders).Delegates("packer").Connection("models").SshKey())
        .Agent("packer", a => a.Describe("Says how an order is packed.").Admin())
        .Connection(Connection.Llm("models").Title("Order models").Describe("The model provider the order helper runs on."))
        .Connection(Connection.Stt("speech"))
        .Endpoint<ListOrders>()
        .Endpoint<GetOrder>()
        .Endpoint<CancelOrder>()
        .Endpoint<CountOpen>()
        .Endpoint<Sales>()
        .Endpoint<TopProducts>()
        .Endpoint<GetActivity>()
        .Endpoint<GetLive>()
        .Endpoint<GetWatchers>()
        .Endpoint<PreviewNote>()
        .Endpoint<GetJob>()
        .Endpoint<AddNote>()
        .Endpoint<NoteAction>()
        .Endpoint<Watch>()
        .Endpoint<Assign>()
        .Topic(Topics.Orders.Roles(Clerk))
        .Topic(Topics.Order.Decl().Authorize(GetOrder.Ref))
        .Ui(new PluginUi()
            .Nav(Nav.Group("Orders").Icon("shopping-bag").Entry(
                Entry.To(Screens.Orders, Ui.Text("nav.all", "All orders"))
                    .Badge(Query.Call(CountOpen.Ref).Follow(Topics.Orders), r => r.Count)
                    .Alert()))
            .Landing(Screens.Orders)
            .Command(Command.Navigate("find", "Find an order", Screens.Orders).Icon("search"))
            .Command(Command.Navigate("week", "Sales: last 7 days", Screens.Report, a => a.Arg(p => p.Range, "7")).Icon("bar-chart"))
            .Screen(OrdersScreen())
            .Screen(OrderScreen())
            .Screen(ReportScreen())
            .Screen(ActivityScreen())
            .Home("sales", "Sales", w =>
            {
                var days = w.Query("days", Query.Call(Sales.Ref).Rows(r => r.Days));
                return Viz.Line.Line()
                    .Dates(days, d => d.Day)
                    .Values(days, d => d.Total)
                    .Option("compact", true)
                    .Open(Screens.Report)
                    .Fallback(Ui.Metric("Revenue", days.Field(r => r.Revenue)));
            })
            .Overlay(CancelDialog())
            .Overlay(AssignDialog())
            .Search(Search.By("Orders", ListOrders.Ref, r => r.Text, r => r.Orders)
                .Item(r => r.View(r.Get(o => o.Number)).Subtitle(r.Get(o => o.Customer)))
                .Open(Screens.Order, (a, r) => a.Arg(x => x.Id, r.Get(o => o.Id)))
                .Action("Cancel order", "x-circle", Screens.Cancel, (a, r) => a.Arg(x => x.Id, r.Get(o => o.Id)))));

    private static Screen OrdersScreen() => Screen.Build(Screens.Orders, s =>
    {
        var status = s.Param(p => p.Status);
        var orders = s.Query("orders", Query.Call(ListOrders.Ref)
            .Arg(r => r.Status, status)
            .Rows(r => r.Orders)
            .Paged()
            .Follow(Topics.Orders));
        var open = s.RowAction("open", orders, (a, r) =>
            a.Navigate(Screens.Order, p => p.Arg(x => x.Id, r.Get(o => o.Id))));
        var cancel = s.RowAction("cancel", orders, (a, r) => a
            .Confirm(Confirm.Danger("Cancel this order?")
                .Text("The customer is told.")
                .CancelLabel("Keep it")
                .Summary("receipt", r.Get(o => o.Number)))
            .Call(CancelOrder.Ref, c => c.Arg(x => x.Id, r.Get(o => o.Id)))
            .Toast(Ui.Text("orders.cancelled", "Order cancelled"))
            .Described(r.Get(o => o.Customer))
            .WithButton("Open", open)
            .OnError(e => e.ToastError()));
        s.Title(Ui.Text("orders.title", "Orders"));
        return Ui.Stack(
            Ui.FilterBar(status.Select("Status")),
            Ui.Table(orders)
                .Mono(o => o.Number, "Order")
                .Col(o => o.Customer, "Customer")
                .WithAvatar(o => o.Customer)
                .SecondLine(o => o.Email)
                .Money(o => o.Total, "Total", o => o.Currency)
                .Status(o => o.Status, "Status", (OrderStatus.Placed, Tone.Attention), (OrderStatus.Paid, Tone.Ok))
                .Relative(o => o.PlacedAt, "Placed")
                .Open(open)
                .Count(orders.Field(r => r.Total))
                .CountLabel(Ui.Text("orders.count", "{total} orders · newest first").Arg("total", orders.Field(r => r.Total)))
                .Compact(r => r.View(r.Get(o => o.Customer))
                    .Subtitle(r.Get(o => o.Number))
                    .Meta(r.Get(o => o.Total))
                    .Money(o => o.Currency)
                    .Status(o => o.Status, (OrderStatus.Paid, Tone.Ok)))
                .RowButton(r => Ui.Button("Cancel", cancel)
                    .Danger()
                    .Visible(r.Get(o => o.Status).Eq(OrderStatus.Placed))));
    });

    /// <summary>
    /// Charts and figures on a board: a line over dates, a ranked bar that selects, a metric, a pivot and several series;
    /// over it the range switch, bound to a param with a default.
    /// </summary>
    private static Screen ReportScreen() => Screen.Build(Screens.Report, s =>
    {
        var product = s.Param(p => p.Product);
        var range = s.ParamOr(p => p.Range, "30");
        var channel = s.State("channel", "");
        var byDay = s.Query("sales", Query.Call(Sales.Ref)
            .Arg(r => r.Channel, channel)
            .Arg(r => r.Range, range)
            .Rows(r => r.Days));
        var top = s.Query("top", Query.Call(TopProducts.Ref).Rows(r => r.Products));
        s.Title("Report");
        var charts = Ui.Board()
            .Span(8, Viz.Line.Line()
                .Title(Ui.Text("report.revenue", "Revenue"))
                .Range("Last 30 days")
                .Options(new JsonObject { ["fmt"] = "money", ["curve"] = "smooth" })
                .Dates(byDay, d => d.Day)
                .Values(byDay, d => d.Total)
                .Height(280)
                .Open(Screens.Orders, a => a.Arg(p => p.Status, OrderStatus.Paid))
                .Fallback(Ui.Table(byDay).Date(d => d.Day, "Day").Col(d => d.Total, "Revenue")))
            .Third(Viz.Bar.Bar()
                .Title("Top products")
                .Rows(top, ("name", p => p.Name), ("value", p => p.Revenue), ("units", p => p.Units))
                .TextKeys("name")
                .Option("fmt", "money")
                .Option("limit", 5)
                .Option("field", "product")
                .SelectParam(product))
            .Quarter(Viz.Figure.Metric()
                .Title("Revenue")
                .Value("value", byDay.Field(r => r.Revenue))
                .Value("target", 120000)
                .Value("caption", "this month")
                .Value("filter.channel", channel))
            .Span(9, Viz.Bar.Column()
                .Title("By channel")
                .Pivot(byDay, d => d.Day, d => d.Channel, d => d.Total)
                .AsDate()
                .Option("stacked", true)
                .SelectState(channel))
            .Full(Ui.Heading("Channels"))
            .Full(Ui.Viz("line", "multiples")
                .Labels(byDay, d => d.Channel)
                .Series(byDay, (d => d.Online, "Online"), (d => d.Retail, Ui.Text("report.retail", "Retail")))
                .Serie(byDay, d => d.Total, "Total")
                .Column("notes", byDay, d => d.Channel)
                .AsText()
                .Rows(top, ("name", p => p.Name))
                .To("parts")
                .Visible(product.Empty()))
            .Fallback(Ui.Table(top).Col(p => p.Name, "Product"));
        return Ui.Stack(
            Ui.Segmented(range, ("7", Ui.Text("report.week", "7 days")), ("30", Ui.Text("report.month", "30 days"))),
            charts);
    });

    /// <summary>The cancel dialog, document-level: the order screen and the palette's search action open it.</summary>
    private static Overlay CancelDialog() => Overlay.Dialog(Screens.Cancel, o =>
    {
        var oid = o.Param(p => p.Id);
        o.Title("Cancel order");
        return o.Form(CancelOrder.Ref,
            f =>
            {
                f.Textarea(r => r.Reason, "Why").Required();
                f.Cancel("Keep it");
                f.Note("The customer gets an email.");
                f.Alternate("Cancel quietly", c => c.CallForm(CancelOrder.Ref, x => x.Arg(r => r.Id, oid)).Close());
            },
            "Cancel order",
            (a, _) => a.CallForm(CancelOrder.Ref, c => c.Arg(r => r.Id, oid)).Toast("Order cancelled").Close());
    });

    private static Screen OrderScreen() => Screen.Build(Screens.Order, s =>
    {
        var id = s.Param(p => p.Id);
        var order = s.Query("order", Query.Call(GetOrder.Ref)
            .Arg(r => r.Id, id)
            .Follow(Topics.Order.Key(k => k.Set(x => x.Id, id))));
        var edit = s.Action("cancel", a => a.Open(Screens.Cancel, p => p.Arg(x => x.Id, id)));
        var back = s.Action("back", a => a.Back());
        var remind = s.Action("remind", a => a.Toast("Reminder sent"));
        s.Title(order.Field(o => o.Number));
        return Ui.Stack(
            Ui.Breadcrumbs(
                Ui.Crumb("Orders", Screens.Orders),
                Ui.Crumb(Ui.Text("order.same_status", "Same status"), Screens.Orders,
                    a => a.Arg(p => p.Status, order.Field(o => o.Status))),
                Ui.Crumb(order.Field(o => o.Number))),
            Ui.RecordHeader(order.Field(o => o.Number))
                .Subtitle(order.Field(o => o.Customer))
                .Tags(Ui.Status(order.Field(o => o.Status), (OrderStatus.Paid, Tone.Ok)))
                .Actions(Ui.Button("Cancel order", edit)
                    .Danger()
                    .Visible(order.Field(o => o.Status).Eq(OrderStatus.Placed)))
                .Menu(Ui.MenuItem("Send a reminder", remind))
                .Close(back),
            Ui.Columns(
                    Ui.Fields(
                        Ui.Item("Customer", order.Field(o => o.Customer)),
                        Ui.Item("Placed", order.Field(o => o.PlacedAt)).Datetime()),
                    Ui.TitledCard("Payment", Ui.Status(order.Field(o => o.Status), (OrderStatus.Paid, Tone.Ok)))
                        .Trailing(order.Field(o => o.Total))
                        .MoneyIn("USD"))
                .Ratios(2, 1));
    });

    /// <summary>
    /// The activity of an order (catalog 3): notes and events read from the bottom, a packing job's work on a note, the
    /// live jobs, a box to write a note, and a side panel of watchers.
    /// </summary>
    private static Screen ActivityScreen() => Screen.Build(Screens.Activity, s =>
    {
        var order = s.Param(p => p.Order);
        var job = s.Param(p => p.Job);
        var tab = s.StateReset("tab", "notes", order);
        var wide = s.StateReset("wide", false, order);
        var draft = s.State("draft", "");
        var who = new Via<Note, Person>("who");
        var notes = s.Query("notes", Query.Call(GetActivity.Ref)
            .Arg(r => r.Order, order.OrEmpty())
            .Rows(v => v.Notes)
            .Older(v => v.OlderPageToken, r => r.PageToken)
            .Join(who, v => v.People, p => p.Id, n => n.By)
            .Follow(Topics.Orders));
        var live = s.Query("live", Query.Call(GetLive.Ref)
            .Arg(r => r.Order, order.OrEmpty())
            .Rows(v => v.Notes)
            .Join(who, v => v.People, p => p.Id, n => n.By));
        var people = s.Query("people", Query.Call(GetWatchers.Ref)
            .Arg(r => r.Order, order.OrEmpty())
            .Rows(v => v.People));
        var hint = s.Query("hint", Query.Call(PreviewNote.Ref)
            .Arg(r => r.Order, order.OrEmpty())
            .Arg(r => r.Text, draft)
            .DebounceMs(300)
            .When(!draft.Empty()));
        var jobQ = s.Query("job", Query.Call(GetJob.Ref)
            .Arg(r => r.Id, job.OrEmpty())
            .When(!job.Empty()));
        var add = s.Action("add", a => a
            .Call(AddNote.Ref, c => c
                .Arg(x => x.Order, order.OrEmpty())
                .Arg(x => x.Text, Form.Text())
                .Arg(x => x.Reply, Form.Reply())
                .Arg(x => x.Urgent, Form.Option("urgent")))
            .Merge(notes, Ui.Result((AddNoteResponse r) => r.Note)));
        ActionRef OnNote(string name, Val<string>? value) => s.RowAction(name, notes, (a, r) => a.Call(NoteAction.Ref, c =>
        {
            c.Arg(x => x.Note, r.Get(n => n.Id)).Arg(x => x.Action, name);
            if (value is not null && name == "react") c.Arg(x => x.Emoji, value);
            else if (value is not null) c.Arg(x => x.Text, value);
        }));
        var seen = OnNote("seen", null);
        var react = OnNote("react", Form.Emoji());
        var edit = OnNote("edit", Form.Text());
        var retry = OnNote("retry", null);
        var openJob = s.RowAction("open_job", notes, (a, r) => a.SetParam(job, r.Get<string?>(n => n.JobId)));
        var closeJob = s.Action("close_job", a => a.SetParam(job, Ui.None<string?>()));
        var quote = s.RowAction("quote", notes, (a, r) =>
            a.Draft(Ui.Text("activity.quote", "About {who}'s note: ").Arg("who", r.Get(who.F(p => p.Name)))));
        var answer = s.RowAction("answer", notes, (a, r) => a.ReplyTo(r.Get(who.F(p => p.Name)), r.Get(n => n.Id)));
        var copyLink = s.Action("copy_link", a => a.CopyLink(order));
        var copyTitle = s.Action("copy_title", a => a.Copy(notes.Field(v => v.Title)));
        var toggleWatch = s.Action("watch", a => a.Call(Watch.Ref, c => c
            .Arg(x => x.Order, order.OrEmpty())
            .Arg(x => x.Watch, Form.Pressed())));
        var mention = s.Action("mention", a => a.Toast(Ui.Text("activity.mention", "Write to {to}").Arg("to", Form.To())));
        var stop = s.Action("stop", a => a.Toast("Stopping"));
        var stopJob = s.Action("stop_job", a => a.Toast("Stopping the job"));
        var jump = s.Action("jump", a => a.ScrollTo(live.Field(v => v.Label)));
        var assign = s.Action("assign", a => a.Open(Screens.Assign, p => p.Arg(x => x.Order, order.OrEmpty())));
        s.Title(notes.Field(v => v.Title));
        var mine = Ui.RowValue((Note n) => n.Mine);
        var note = Ui.Post<Note>()
            .Author(who.F(p => p.Name))
            .Time(n => n.Time)
            .Avatar(who.F(p => p.Avatar))
            .Tile(who.F(p => p.Icon), who.F(p => p.Engine))
            .Badge("AGENT", Ui.RowValue(who.F(p => p.Agent)).Eq(true))
            .Markdown(n => n.Text)
            .Edited(n => n.Edited)
            .Pinned(n => n.Pinned, true)
            .Mine(n => n.Mine)
            .Receipt(n => n.Receipt, n => n.ReceiptTone, ("done", Tone.Ok))
            .Reactions(n => n.Reactions, x => x.Emoji, x => x.Count, react, mine: x => x.Mine)
            .Quick("👍", "✅")
            .Summary(n => n.Replies!.Count, openJob)
            .SummaryLabel(Ui.Text("activity.replies", "{n, plural, one {# reply} other {# replies}}")
                .Arg("n", Ui.RowValue((Note n) => n.Replies!.Count)))
            .Process(n => n.Job, w => w
                .State(j => j.State)
                .Label(j => j.Label)
                .Meta(j => j.Meta)
                .Started(j => j.Started)
                .Rows(j => j.Rows)
                .Steps(j => j.Steps)
                .Retry(retry, j => j.RetryLabel, mine.Eq(true))
                .Log(openJob)
                .FetchCall(jobQ, (GetJobRequest r) => r.Call, v => v.Call,
                    a => a.Arg(r => r.Id, Ui.RowValue((Note n) => n.JobId))))
            .Hover(Builtin.React(), Builtin.Reply(), Ui.Entry(quote, "Quote").TextStyle(), Builtin.More())
            .Menu(Builtin.CopyText(), Builtin.CopyLink(), Builtin.Edit(), Builtin.Separator(),
                Ui.Entry(answer, "Answer").Icon("reply").Danger())
            .Sheet(Builtin.Reply(), Builtin.CopyText())
            .Edit(edit, mine.Eq(true))
            .ActionsWhen(notes.Field(v => v.Closed).Ne(true))
            .Card(Ui.CardOf(people, p => p.Name)
                .Avatar(p => p.Avatar)
                .Agent(p => p.Agent)
                .Tile(p => p.Icon, p => p.Engine)
                .Detail(p => p.Role)
                .Action(mention)
                .Unknown("Not a watcher"));
        var column = Ui.Stack(
            Ui.Header(notes.Field(v => v.Title))
                .TitleIcon(Ui.Lit("package"))
                .LeadingTile(Ui.Lit("package"), Ui.Lit(""))
                .LeadingSize(40)
                .Subtitle(
                    Segment.Text(notes.Field(v => v.Status)),
                    Segment.LiveDot().Visible(live.Field(v => v.CanStop).Eq(true)),
                    Segment.Countdown(notes.Field(v => v.Ends)).Prefix("closes in "))
                .SubtitleAction(jump)
                .Faces(notes.Field(v => v.People), "Watchers", assign)
                .Button("Assign", assign)
                .ButtonWhen(notes.Field(v => v.Closed).Ne(true))
                .Menu(Ui.MenuItem("Copy link", copyLink), Ui.MenuItem("Copy title", copyTitle))
                .Tabs(tab, Ui.HeaderTab("notes", "Notes"), Ui.HeaderTab("pinned", "Pinned").Badge(notes.Field(v => v.Pinned)))
                .PhoneSub(notes.Field(v => v.Status))
                .PhoneIcons(assign)
                .MenuTabs(),
            Ui.Stream(notes)
                .Key(n => n.Id)
                .Post(note)
                .KindsLine(n => n.Kind, "event", who.F(p => p.Name), n => n.Text, n => n.Time)
                .Tail(live)
                .Days(n => n.Time)
                .JumpWorking(live.Field(v => v.Label))
                .Empty("No notes yet.")
                .PhoneIntro(notes.Field(v => v.Status))
                .OnSeen(seen)
                .OnSeenWhen(mine.Ne(true))
                .Id("notes")
                .Label("Notes")
                .Visible(tab.Eq("notes")),
            Ui.Stream(notes)
                .Key(n => n.Id)
                .AnchorTop()
                .OlderButton("Show more pinned notes")
                .Post(note.NoSummary())
                .Empty("Nothing pinned.")
                .Visible(tab.Eq("pinned")),
            Ui.Composer(add)
                .For("notes")
                .Primary()
                .Placeholder(notes.Field(v => v.Placeholder))
                .Tools(ComposerTool.Emoji)
                .Mentions(Ui.Mentions(people, p => p.Name)
                    .Avatar(p => p.Avatar)
                    .Agent(p => p.Agent)
                    .Tile(p => p.Icon, p => p.Engine)
                    .Detail(p => p.Role)
                    .Note("A mention tells them"))
                .Tags()
                .Dictation()
                .Vocabulary("DPD")
                .Reply()
                .Option("urgent", "Mark urgent")
                .OptionWhen(notes.Field(v => v.Closed).Ne(true))
                .CaptionRoute(hint.Field(v => v.Hint))
                .CaptionDot(live.Field(v => v.Hint))
                .Stop(stop, live.Field(v => v.CanStop).Eq(true))
                .StopLabel("Stop")
                .Readonly(notes.Field(v => v.Closed).Eq(true), "This order is closed.")
                .DraftState(draft));
        var panel = Ui.Stack(
                Ui.Header("Watchers")
                    .Context(notes.Field(v => v.Title))
                    .Icons(Ui.IconButton("eye", "Watching", toggleWatch).Toggle(notes.Field(v => v.Watching), "eye-off", "Watch"))
                    .Expand()
                    .Close(closeJob)
                    .Back("Activity"),
                Ui.Strip(live, v => v.Jobs, j => j.Job, j => j.Agent)
                    .Label(live.Field(v => v.Label))
                    .Tile(j => j.Icon, j => j.Engine)
                    .State(j => j.State, ["working", "starting"], ["waiting"], ["starting"])
                    .Since(j => j.Since)
                    .Until(j => j.Until)
                    .SubLine(j => j.Sub)
                    .CountdownText("ends in {left}")
                    .Open(openJob)
                    .Jump(j => j.Note, "Jump to the note", "Jump to its last note")
                    .Button(Ui.StripButton("Stop", stopJob)
                        .Icon("square")
                        .StopStyle()
                        .Visible(Ui.RowValue((Packing j) => j.State).Eq("working")))
                    .AriaLabel("Live jobs"),
                Ui.Process(jobQ, w => w.State(v => v.Title).Label(v => v.Title)),
                Ui.List(people, p => p.Name)
                    .Subtitle(p => p.Role)
                    .Search("Find a watcher")
                    .Online(p => p.Online)
                    .Limit(5)
                    .GroupLabel("Watchers"),
                Ui.Text(live.Field(v => v.Label)).Elapsed())
            .Visible(!job.Empty());
        return Ui.Split(column, panel)
            .Fill()
            .Resize(340, 820, 420, 400)
            .Expand(wide);
    });

    /// <summary>Assign people to an order: a pick over the watchers, a note while an agent is chosen.</summary>
    private static Overlay AssignDialog() => Overlay.Dialog(Screens.Assign, o =>
    {
        var oid = o.Param(p => p.Order);
        o.Title("Assign");
        var people = o.Query("directory", Query.Call(GetWatchers.Ref)
            .Arg(r => r.Order, oid.OrEmpty())
            .Rows(v => v.People));
        return o.Form(Assign.Ref,
            f =>
            {
                f.Pick(r => r.People, "People", people, p => p.Name, p => p.Name)
                    .Avatar(p => p.Avatar)
                    .Agent(p => p.Agent)
                    .Tile(p => p.Icon, p => p.Engine)
                    .Detail(p => p.Role)
                    .Exclude(Ui.Lit<string[]>([]))
                    .Empty("Nobody matches", "Everyone is assigned")
                    .Placeholder("Find people")
                    .Required();
                f.NoteWhen("An agent works on the order at once.", Form.PickedAgents((AssignRequest r) => r.People).Gt(0));
            },
            Ui.Text("assign.submit", "Assign {first}").Arg("first", Form.PickedFirst((AssignRequest r) => r.People)),
            (a, _) => a.CallForm(Assign.Ref, c => c.Arg(r => r.Order, oid.OrEmpty())).Close());
    });
}
