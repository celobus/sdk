// The builders of catalog 3 (an app-like screen: a header, a stream of posts with a run of work, a strip of live runs,
// a composer, a side panel, a pick dialog): the JSON each one writes (the shape of crates/descriptor/src/ui.rs).
using System.Text.Json.Nodes;
using Celobus.Sdk.Ui;
using U = Celobus.Sdk.Ui.Ui;

namespace Celobus.Sdk.Tests;

[Message] public sealed record AppFace(string Id, string Name, string Avatar, bool Agent, string Icon, string Engine, string Detail, string Section, bool Online);
[Message] public sealed record AppReaction(string Emoji, int Count, bool Mine);
[Message] public sealed record AppCall(string Call, string Detail, string State, string Output, bool Truncated);
[Message] public sealed record AppWorkRow(string Kind, string Title, string Detail, IReadOnlyList<AppCall> Tools);
[Message] public sealed record AppWork(string State, string Label, string Meta, long Started, IReadOnlyList<AppWorkRow> Rows, int Steps, string Saying, string RetryLabel);
[Message] public sealed record AppSummary(int Count, IReadOnlyList<AppFace> Faces, string Last, string Working, long Since, string Waiting);

[Message]
public sealed record AppMsg(
    string Id, string By, string Time, string Text, bool Writing, string Kind, bool Edited, bool Pinned, bool Mine,
    string Receipt, string ReceiptTone, string ReceiptTo, string Why, string FixLabel, IReadOnlyList<AppReaction> Reactions,
    AppSummary? Thread, AppWork? Work, string Run, string Duration);

[Message] public sealed record AppLiveRun(string Run, string Agent, string Icon, string Engine, string State, long Since, long Until, string Sub, string Block);
[Message]
public sealed record ChannelView(
    string Title, string Icon, IReadOnlyList<AppFace> Faces, IReadOnlyList<AppMsg> Messages, IReadOnlyList<AppFace> People,
    string OlderPageToken, int PinnedCount, bool Readonly, string Placeholder, bool Following, long Ends, AppMsg? First);
[Message] public sealed record LiveRunsView(IReadOnlyList<AppMsg> Blocks, IReadOnlyList<AppFace> People, IReadOnlyList<AppLiveRun> Runs, string Label, bool CanStop, string Hint);
[Message] public sealed record AppRunView(string Title, Status Status, string Note, AppCall? Call, IReadOnlyList<AppWorkRow> Steps);
[Message] public sealed record GetChannelRequest(string Id, string PageToken, string Text);
[Message] public sealed record GetAppRunRequest(string Id, string Call);
[Message] public sealed record PostMessageRequest(string Channel, string Text, string Reply, bool Also, string Audio);
[Message] public sealed record PostedMessage(AppMsg? Message);
[Message] public sealed record StartChatRequest(string To, string Text);
[Message] public sealed record ChannelParams(string? C, string? Run);

public sealed class GetChannel : Endpoint<GetChannelRequest, ChannelView>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();
    public override Task<ChannelView> Handle(Ctx ctx, GetChannelRequest req) => throw new NotSupportedException();
}

public sealed class GetLiveRuns : Endpoint<GetChannelRequest, LiveRunsView>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();
    public override Task<LiveRunsView> Handle(Ctx ctx, GetChannelRequest req) => throw new NotSupportedException();
}

public sealed class GetAppRun : Endpoint<GetAppRunRequest, AppRunView>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();
    public override Task<AppRunView> Handle(Ctx ctx, GetAppRunRequest req) => throw new NotSupportedException();
}

public sealed class PostMessage : Endpoint<PostMessageRequest, PostedMessage>
{
    public override void Configure(EndpointOptions e) => e.Ui();
    public override Task<PostedMessage> Handle(Ctx ctx, PostMessageRequest req) => throw new NotSupportedException();
}

public sealed class StartChat : Endpoint<StartChatRequest, PostedMessage>
{
    public override void Configure(EndpointOptions e) => e.Ui();
    public override Task<PostedMessage> Handle(Ctx ctx, StartChatRequest req) => throw new NotSupportedException();
}

public class UiAppTests
{
    private static readonly ScreenKey<ChannelParams> Channel = new("channel");
    private static readonly OverlayKey<ChannelParams> NewMessage = new("new_message");

    private static JsonNode J(object x) => UiJson.Of(x, new UiWriter())!;

    private static void Same(string expected, JsonNode? actual) =>
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse(expected), actual), actual?.ToJsonString());

    private static JsonNode ScreenJson(Func<ScreenBuilder<ChannelParams>, Node> build) => J(Screen.Build(Channel, build).Obj);

    [Fact]
    public void RowPathsReachNestedAndJoinedFields()
    {
        var who = new Via<AppMsg, AppFace>("who");
        Same("""{"row":"who.name"}""", J(new Row<AppMsg>().Get(who.F(f => f.Name))));
        Same("""{"row":"thread.count"}""", J(U.RowValue((AppMsg m) => m.Thread!.Count)));
        Same("""{"eq":[{"row":"who.agent"},{"lit":true}]}""", J(U.RowValue(who.F(f => f.Agent)).Eq(true)));
        // a joined path goes wherever a row field does
        Same("""{"list":{"query":"q","title":"who.name"}}""", J(U.List(new Q<ChannelView, AppMsg>("q"), who.F(f => f.Name))));
    }

    [Fact]
    public void QueryOptionsStateResetAndSteps()
    {
        var j = ScreenJson(s =>
        {
            var c = s.Param(p => p.C);
            var run = s.Param(p => p.Run);
            s.StateReset("tab", "messages", c);
            s.StateReset("wide", false, c, run);
            var draft = s.State("draft", "");
            var who = new Via<AppMsg, AppFace>("who");
            var channel = s.Query("channel", Query.Call(GetChannel.Ref)
                .Arg(r => r.Id, c.OrEmpty())
                .Rows(v => v.Messages)
                .Older(v => v.OlderPageToken, r => r.PageToken)
                .Join(who, v => v.People, f => f.Id, m => m.By));
            s.Query("hint", Query.Call(GetChannel.Ref).Arg(r => r.Text, draft).DebounceMs(300).When(!draft.Empty()));
            s.Action("send", a => a
                .Call(PostMessage.Ref, x => x
                    .Arg(r => r.Channel, c.OrEmpty())
                    .Arg(r => r.Text, Form.Text())
                    .Arg(r => r.Reply, Form.Reply())
                    .Arg(r => r.Audio, Form.Audio())
                    .Arg(r => r.Also, Form.Option("also")))
                .Merge(channel, U.Result((PostedMessage p) => p.Message)));
            s.RowAction("ask", channel, (a, r) => a.Draft(U.Text("ask", "@{who} about this: ").Arg("who", r.Get(who.F(f => f.Name)))));
            s.RowAction("reply", channel, (a, r) => a.ReplyTo(r.Get(who.F(f => f.Name)), r.Get(m => m.Id)));
            s.Action("links", a => a.CopyLink(c).Copy(channel.Field(v => v.Title)).ScrollTo(channel.Field(v => v.Title)));
            return U.Divider();
        });
        Same("""
            [{"name":"tab","type":"string","default":"messages","reset":["c"]},
             {"name":"wide","type":"bool","default":false,"reset":["c","run"]},
             {"name":"draft","type":"string","default":""}]
            """, j["state"]);
        Same("""
            {"name":"channel","endpoint":"GetChannel","args":[{"field":"id","value":{"param":"c"}}],"rows":"messages",
             "older":{"token":"older_page_token","arg":"page_token"},
             "join":[{"as":"who","rows":"people","key":"id","by":"by"}]}
            """, j["queries"]![0]);
        Same("""
            {"name":"hint","endpoint":"GetChannel","args":[{"field":"text","value":{"state":"draft"}}],
             "when":{"not":{"empty":{"state":"draft"}}},"debounce_ms":300}
            """, j["queries"]![1]);
        Same("""
            [{"call":{"endpoint":"PostMessage","args":[{"field":"channel","value":{"param":"c"}},
               {"field":"text","value":{"form":"text"}},{"field":"reply","value":{"form":"reply"}},
               {"field":"audio","value":{"form":"audio"}},{"field":"also","value":{"form":"also"}}]}},
             {"merge":{"query":"channel","row":{"result":"message"}}}]
            """, j["actions"]![0]!["steps"]);
        Same("""[{"draft":{"text":{"key":"ask","args":{"who":{"row":"who.name"}}}}}]""", j["actions"]![1]!["steps"]);
        Same("""[{"reply_to":{"name":{"row":"who.name"},"key":{"row":"id"}}}]""", j["actions"]![2]!["steps"]);
        Same("""
            [{"copy":{"link":{"param":"c"}}},{"copy":{"value":{"data":{"query":"channel","field":"title"}}}},
             {"scroll_to":{"key":{"data":{"query":"channel","field":"title"}}}}]
            """, j["actions"]![3]!["steps"]);
    }

    [Fact]
    public void FormValues()
    {
        Same("""{"form":"text"}""", J(Form.Text()));
        Same("""{"form":"reply"}""", J(Form.Reply()));
        Same("""{"form":"audio"}""", J(Form.Audio()));
        Same("""{"form":"duration"}""", J(Form.Duration()));
        Same("""{"form":"language"}""", J(Form.Language()));
        Same("""{"form":"words"}""", J(Form.Words()));
        Same("""{"form":"also"}""", J(Form.Option("also")));
        Same("""{"form":"emoji"}""", J(Form.Emoji()));
        Same("""{"form":"to"}""", J(Form.To()));
        Same("""{"form":"pressed"}""", J(Form.Pressed()));
        Same("""{"gt":[{"form":"to.count"},{"lit":"0"}]}""", J(Form.PickedCount((StartChatRequest r) => r.To).Gt(0)));
        Same("""{"form":"to.agents"}""", J(Form.PickedAgents((StartChatRequest r) => r.To)));
        Same("""{"form":"to.first"}""", J(Form.PickedFirst((StartChatRequest r) => r.To)));
    }

    [Fact]
    public void FormatsAndOptionsOfEarlierNodes()
    {
        var q = new Q<AppRunView, AppRunView>("run");
        Same("""{"text":{"text":"x","style":"body","format":{"elapsed":{}}}}""", J(U.Text("x").Elapsed()));
        Same("""{"countdown":{}}""", J(U.Metric("Ends", q.Field(r => r.Title)).Countdown())["metric"]!["format"]);
        Same("""{"span":{}}""", J(U.Item("Took", q.Field(r => r.Title)).Span())["format"]);
        Same("""
            {"split":{"list":{"divider":{}},"detail":{"divider":{}},"fill":true,
              "resize":{"min":340,"max":820,"default":420,"keep":400},"expand":{"state":"wide"}}}
            """, J(U.Split(U.Divider(), U.Divider()).Fill().Resize(340, 820, 420, 400).Expand(new StateRef<bool>("wide"))));
        Same("""{"min":240,"max":600,"default":300}""",
            J(U.Split(U.Divider(), U.Divider()).Resize(240, 600, 300))["split"]!["resize"]);
        Same("""
            {"icon_button":{"icon":"bell","label":"Following","action":"go","pressed":{"data":{"query":"run","field":"title"}},
              "icon_off":"bell-off","label_off":"Follow"}}
            """, J(U.IconButton("bell", "Following", new ActionRef("go")).Toggle(q.Path<bool>("title"), "bell-off", "Follow")));
        Same("""
            {"list":{"query":"people","title":"name","search":"Find","online":"online","limit":5,"group_label":"People"}}
            """, J(U.List(new Q<ChannelView, AppFace>("people"), f => f.Name).Search("Find").Online(f => f.Online).Limit(5).GroupLabel("People")));
        var log = J(U.AgentRun(q, r => r.Title, r => r.Status, Array.Empty<(Status, Tone)>(), r => r.Steps, s => s.Title, s => s.Kind)
            .Log()
            .Tile(r => r.Title, r => r.Note)
            .StatusLabel(r => r.Title)
            .StepLabel(r => r.Note)
            .Elapsed(r => r.Title)
            .Note(r => r.Note)
            .LiveWhen(q.Field(r => r.Status).Eq(Status.Placed))
            .StepDetail((AppWorkRow s) => s.Detail)
            .StepTime((AppWorkRow s) => s.Kind)
            .StepTool((AppWorkRow s) => s.Tools)
            .StepEvidence((AppWorkRow s) => s.Detail)
            .StepLines((AppWorkRow s) => s.Title))["agent_run"]!;
        Same("""
            {"query":"run","title":"title","status":{"field":"status"},"steps":"steps","step_text":"title","step_state":"kind",
             "log":true,"icon":"title","engine":"note","status_label":"title","step_label":"note","elapsed":"title","note":"note",
             "live_when":{"eq":[{"data":{"query":"run","field":"status"}},{"lit":"placed"}]},
             "step_detail":"detail","step_time":"kind","step_tool":"tools","step_evidence":"detail","step_lines":"title"}
            """, log);
    }

    [Fact]
    public void Header()
    {
        var q = new Q<ChannelView, AppMsg>("channel");
        var tab = new StateRef<string>("tab");
        var go = new ActionRef("go");
        var h = U.Header(q.Field(v => v.Title))
            .TitleIcon(q.Field(v => v.Icon))
            .CrumbIcon(U.Lit("hash"))
            .LeadingTile(U.Lit("bot"), U.Lit("claude"))
            .LeadingSize(40)
            .Subtitle(
                Segment.Text("Agents"),
                Segment.LiveDot().Visible(q.Field(v => v.Following).Eq(true)),
                Segment.WaitingDot(),
                Segment.Countdown(q.Field(v => v.Ends)).Prefix("ends in "),
                Segment.Elapsed(q.Field(v => v.Ends)),
                Segment.LiveTone().Visible(q.Field(v => v.Following).Eq(false)),
                Segment.WaitingTone())
            .SubtitleAction(go)
            .Context("# returns")
            .Faces(q.Field(v => v.Faces), "9 members", go)
            .FacesWhen(q.Field(v => v.Readonly).Ne(true))
            .FacesAriaLabel("9 members: open the members")
            .Button("Join", go)
            .ButtonWhen(q.Field(v => v.Readonly).Eq(true))
            .Icons(U.IconButton("bell", "Following", go))
            .Menu(U.MenuItem("Copy link", go))
            .MenuLabel("Channel actions")
            .Tabs(tab, U.HeaderTab("messages", "Messages"), U.HeaderTab("pinned", "Pinned").Badge(q.Field(v => v.PinnedCount)).Action(go))
            .TabsLabel("Channel")
            .Expand()
            .Close(go)
            .Back("Channels")
            .PhoneSub("2 live")
            .PhoneIcons(go)
            .MenuTabs();
        Same("""
            {"header":{"title":{"data":{"query":"channel","field":"title"}},
              "title_icon":{"data":{"query":"channel","field":"icon"}},"crumb_icon":{"lit":"hash"},
              "leading":{"tile":{"icon":{"lit":"bot"},"engine":{"lit":"claude"}},"size":40},
              "subtitle":[{"text":"Agents"},{"dot":"live","visible":{"eq":[{"data":{"query":"channel","field":"following"}},{"lit":true}]}},
                {"dot":"waiting"},{"value":{"data":{"query":"channel","field":"ends"}},"format":{"countdown":{}},"prefix":"ends in "},
                {"value":{"data":{"query":"channel","field":"ends"}},"format":{"elapsed":{}}},
                {"tone":"live","visible":{"eq":[{"data":{"query":"channel","field":"following"}},{"lit":false}]}},{"tone":"waiting"}],
              "subtitle_action":"go","context":"# returns",
              "faces":{"value":{"data":{"query":"channel","field":"faces"}},"label":"9 members","action":"go",
                "visible":{"ne":[{"data":{"query":"channel","field":"readonly"}},{"lit":true}]},"aria_label":"9 members: open the members"},
              "buttons":[{"label":"Join","action":"go","visible":{"eq":[{"data":{"query":"channel","field":"readonly"}},{"lit":true}]}}],
              "icons":[{"icon_button":{"icon":"bell","label":"Following","action":"go"}}],
              "menu":{"items":[{"label":"Copy link","action":"go"}],"label":"Channel actions"},
              "tabs":{"bind":{"state":"tab"},"items":[{"id":"messages","title":"Messages"},
                {"id":"pinned","title":"Pinned","badge":{"data":{"query":"channel","field":"pinned_count"}},"action":"go"}],"label":"Channel"},
              "expand":true,"close":"go","back":"Channels","phone":{"sub":"2 live","icons":["go"],"menu_tabs":true}}}
            """, J(h));
        Same("""{"header":{"title":"Members"}}""", J(U.Header("Members")));
        Same("""{"faces":{"lit":[]}}""", J(U.Header("x").LeadingFaces(U.Lit<string[]>([])))["header"]!["leading"]);
    }

    [Fact]
    public void StreamsAndPosts()
    {
        var channel = new Q<ChannelView, AppMsg>("channel");
        var live = new Q<LiveRunsView, AppMsg>("live");
        var people = new Q<ChannelView, AppFace>("people");
        var run = new Q<AppRunView, AppRunView>("run");
        var who = new Via<AppMsg, AppFace>("who");
        var go = new ActionRef("go");
        var mine = U.RowValue((AppMsg m) => m.Mine).Eq(true);
        var post = U.Post<AppMsg>()
            .Author(who.F(f => f.Name))
            .Time(m => m.Time)
            .Avatar(who.F(f => f.Avatar))
            .Tile(who.F(f => f.Icon), who.F(f => f.Engine))
            .TileWhen(U.RowValue(who.F(f => f.Agent)).Eq(true))
            .Badge("AGENT", U.RowValue(who.F(f => f.Agent)).Eq(true))
            .Source(m => m.Kind)
            .Plain(m => m.Text)
            .Writing(m => m.Writing)
            .Audio(m => m.Duration)
            .AudioMore(m => m.Kind, m => m.Text, m => m.Why)
            .Edited(m => m.Edited)
            .Pinned(m => m.Pinned, false)
            .Mine(m => m.Mine)
            .Receipt(m => m.Receipt, m => m.ReceiptTone, ("failed", Tone.Danger), ("done", Tone.Ok))
            .Route(m => m.ReceiptTo, m => m.ReceiptTone, m => m.Why)
            .Correction(m => m.FixLabel, go, m => m.Time)
            .CorrectionWords(m => m.FixLabel, m => m.Why, m => m.Kind)
            .CorrectionWindowMs(60000)
            .Reactions(m => m.Reactions, r => r.Emoji, r => r.Count, go)
            .ReactionsMine((AppReaction r) => r.Mine)
            .Quick("👍", "🔥")
            .Summary(m => m.Thread!.Count, go)
            .SummaryLabel("{count} replies")
            .SummaryMore(m => m.Thread!.Faces, m => m.Thread!.Last, m => m.Thread!.Working, m => m.Thread!.Since, m => m.Thread!.Waiting)
            .Process(m => m.Work, w => w
                .State(x => x.State).Label(x => x.Label).Meta(x => x.Meta).Started(x => x.Started).Rows(x => x.Rows)
                .Steps(x => x.Steps).Saying(x => x.Saying).Retry(go, x => x.RetryLabel, mine).Log(go)
                .FetchCall(run, (GetAppRunRequest r) => r.Call, v => v.Call, a => a.Arg(r => r.Id, U.RowValue((AppMsg m) => m.Run))))
            .Hover(Builtin.React(), Builtin.Reply(), U.Entry(go, "Ask agent").TextStyle(), Builtin.More())
            .Menu(Builtin.CopyText(), Builtin.CopyLink(), Builtin.Play(), Builtin.Edit(), Builtin.Correction(), Builtin.Separator(),
                U.Entry(go, "Delete").Icon("trash").IconStyle().Danger().Visible(mine))
            .Sheet(Builtin.Reply())
            .Edit(go, mine)
            .ActionsWhen(channel.Field(v => v.Readonly).Ne(true))
            .Card(U.CardOf(people, f => f.Name).Avatar(f => f.Avatar).Agent(f => f.Agent).Tile(f => f.Icon, f => f.Engine)
                .Detail(f => f.Detail).Action(go).Unknown("Not in this channel"));
        var stream = U.Stream(channel)
            .Key(m => m.Id)
            .Post(post)
            .KindsLine(m => m.Kind, "event", who.F(f => f.Name), m => m.Text, m => m.Time)
            .Tail(live)
            .Head(v => v.First, "Start of the thread")
            .Days(m => m.Time)
            .Unread(channel.Field(v => v.Title))
            .JumpWorking("Assistant is working")
            .Empty("No messages yet.")
            .PhoneIntro("Agents answer here")
            .OnSeen(go)
            .OnSeenWhen(mine)
            .Id("messages")
            .Label("Messages");
        // the template was copied: changing it afterwards leaves the stream as it was
        post.NoSummary();
        var j = J(stream)["stream"]!;
        Same("""
            {"author":"who.name","time":"time","avatar":{"url":"who.avatar","tile":{"icon":"who.icon","engine":"who.engine",
               "when":{"eq":[{"row":"who.agent"},{"lit":true}]}}},
             "badge":{"text":"AGENT","when":{"eq":[{"row":"who.agent"},{"lit":true}]}},"source":"kind",
             "text":{"field":"text","format":"plain","writing":"writing"},
             "audio":{"duration":"duration","label":"kind","url":"text","words":"why"},"edited":"edited",
             "pinned":{"field":"pinned","mark":{"lit":false}},"mine":"mine",
             "receipt":{"text":"receipt","tone":{"field":"receipt_tone","tones":{"done":"ok","failed":"danger"}}},
             "route":{"to":"receipt_to","status":"receipt_tone","why":"why"},
             "correction":{"label":"fix_label","menu_label":"fix_label","lead":"why","icon":"kind","action":"go","from":"time","window_ms":60000},
             "reactions":{"field":"reactions","emoji":"emoji","count":"count","mine":"mine","action":"go","quick":["👍","🔥"]},
             "summary":{"count":"thread.count","action":"go","label":"{count} replies","faces":"thread.faces","last":"thread.last",
               "working":"thread.working","since":"thread.since","waiting":"thread.waiting"},
             "process":{"state":"work.state","label":"work.label","meta":"work.meta","started":"work.started","rows":"work.rows",
               "steps":"work.steps","saying":"work.saying",
               "retry":{"action":"go","label":"work.retry_label","when":{"eq":[{"row":"mine"},{"lit":true}]}},"log":{"action":"go"},
               "fetch_call":{"query":"run","args":[{"field":"id","value":{"row":"run"}}],"call":"call","field":"call"}},
             "hover":[{"builtin":"react"},{"builtin":"reply"},{"action":"go","label":"Ask agent","style":"text"},{"builtin":"more"}],
             "menu":[{"builtin":"copy_text"},{"builtin":"copy_link"},{"builtin":"play"},{"builtin":"edit"},{"builtin":"correction"},
               {"builtin":"separator"},{"action":"go","label":"Delete","icon":"trash","style":"icon","danger":true,
               "visible":{"eq":[{"row":"mine"},{"lit":true}]}}],
             "sheet":[{"builtin":"reply"}],
             "edit":{"action":"go","when":{"eq":[{"row":"mine"},{"lit":true}]}},
             "actions_when":{"ne":[{"data":{"query":"channel","field":"readonly"}},{"lit":true}]},
             "card":{"query":"people","name":"name","avatar":"avatar","agent":"agent","icon":"icon","engine":"engine","detail":"detail",
               "action":"go","unknown":"Not in this channel"}}
            """, j["item"]!["post"]);
        j.AsObject().Remove("item");
        Same("""
            {"query":"channel","key":"id","kinds":{"field":"kind","line":{"when":"event","author":"who.name","text":"text","time":"time"}},
             "tail":{"query":"live","hold":true},"head":{"row":"first","rule":"Start of the thread"},"days":"time",
             "unread":{"data":{"query":"channel","field":"title"}},"jump_working":"Assistant is working","empty":"No messages yet.",
             "phone_intro":"Agents answer here","on_seen":{"action":"go","when":{"eq":[{"row":"mine"},{"lit":true}]}},
             "id":"messages","label":"Messages"}
            """, j);

        // the defaults: a post without fields, the anchor and older load left out; a top list with a button
        Same("""{"stream":{"query":"channel","key":"","item":{"post":{"author":"","time":""}}}}""", J(U.Stream(channel)));
        Same("""
            {"stream":{"query":"channel","key":"id","item":{"post":{"author":"by","time":"time","text":{"field":"text"},"summary":{"count":"thread.count","action":"go"}}},
              "anchor":"top","older_load":"button","older_label":"Show more"}}
            """, J(U.Stream(channel).Key(m => m.Id).AnchorTop().OlderButton("Show more")
                .Post(p => p.Author(m => m.By).Time(m => m.Time).Markdown(m => m.Text).Summary(m => m.Thread!.Count, go))));
        Same("""{"older_label":"Earlier"}""", new JsonObject { ["older_label"] = J(U.Stream(channel).OlderLabel("Earlier"))["stream"]!["older_label"]!.DeepClone() });
    }

    [Fact]
    public void ProcessStripAndComposer()
    {
        var run = new Q<AppRunView, AppRunView>("run");
        var live = new Q<LiveRunsView, AppMsg>("live");
        var people = new Q<ChannelView, AppFace>("people");
        var go = new ActionRef("go");
        Same("""{"process":{"query":"run","state":"status","label":"title"}}""",
            J(U.Process(run, w => w.State(r => r.Status).Label(r => r.Title))));

        Same("""
            {"strip":{"query":"live","rows":"runs","key":"run","title":"agent","label":{"data":{"query":"live","field":"label"}},
              "tile":{"icon":"icon","engine":"engine"},
              "state":{"field":"state","live":["working"],"waiting":["waiting"],"starting":["starting"]},
              "since":"since","until":"until","sub":"sub","countdown_text":"ends in {left}","open":"go",
              "jump":{"field":"block","label":"Jump","label_waiting":"Jump to its question"},
              "buttons":[{"label":"Stop","action":"go","icon":"square","style":"stop","visible":{"eq":[{"row":"state"},{"lit":"working"}]}},
                {"label":"Open","action":"go","style":"text"}],
              "aria_label":"Live runs"}}
            """, J(U.Strip(live, v => v.Runs, r => r.Run, r => r.Agent)
                .Label(live.Field(v => v.Label))
                .Tile(r => r.Icon, r => r.Engine)
                .State(r => r.State, ["working"], ["waiting"], ["starting"])
                .Since(r => r.Since)
                .Until(r => r.Until)
                .SubLine(r => r.Sub)
                .CountdownText("ends in {left}")
                .Open(go)
                .Jump(r => r.Block, "Jump", "Jump to its question")
                .Button(U.StripButton("Stop", go).Icon("square").StopStyle().Visible(U.RowValue((AppLiveRun r) => r.State).Eq("working")))
                .Button(U.StripButton("Open", go).TextStyle())
                .AriaLabel("Live runs")));
        Same("""{"strip":{"query":"live","key":"id","title":"by","state":{"field":"kind"}}}""",
            J(U.Strip(live, m => m.Id, m => m.By).State(m => m.Kind, [], [], [])));

        Same("""
            {"composer":{"send":"go","for":"messages","placeholder":"Message #returns","primary":true,"compact":true,
              "tools":["add","emoji","attach"],
              "mentions":{"query":"people","name":"name","avatar":"avatar","agent":"agent","icon":"icon","engine":"engine",
                "detail":"detail","section":"section","note":"A mention tells them"},
              "tags":true,"dictation":{"vocabulary":"DPD"},"reply":true,
              "options":[{"name":"also","label":"Also send to the channel","visible":{"ne":[{"state":"tab"},{"lit":"x"}]}}],
              "caption":[{"value":"Answers: Assistant","icon":"corner-down-right","style":"route"},{"value":"Shift+Enter: a new line","style":"dot"}],
              "stop":{"action":"go","when":{"eq":[{"data":{"query":"live","field":"can_stop"}},{"lit":true}]},"label":"Stop","aria_label":"Stop the agent"},
              "readonly":{"when":{"eq":[{"state":"ro"},{"lit":true}]},"reason":"Read only"},"draft_state":"draft"}}
            """, J(U.Composer(go)
                .For("messages")
                .Placeholder("Message #returns")
                .Primary()
                .Compact()
                .Tools(ComposerTool.Add, ComposerTool.Emoji, ComposerTool.Attach)
                .Mentions(U.Mentions(people, f => f.Name).Avatar(f => f.Avatar).Agent(f => f.Agent).Tile(f => f.Icon, f => f.Engine)
                    .Detail(f => f.Detail).Section(f => f.Section).Note("A mention tells them"))
                .Tags()
                .Vocabulary("DPD")
                .Reply()
                .Option("also", "Also send to the channel")
                .OptionWhen(new StateRef<string>("tab").Ne("x"))
                .CaptionRoute("Answers: Assistant")
                .CaptionDot("Shift+Enter: a new line")
                .Stop(go, live.Field(v => v.CanStop).Eq(true))
                .StopLabel("Stop")
                .StopAriaLabel("Stop the agent")
                .Readonly(new StateRef<bool>("ro").Eq(true), "Read only")
                .DraftState(new StateRef<string>("draft"))));
        Same("""{"composer":{"send":"go","dictation":{}}}""", J(U.Composer(go).Dictation()));
        Same("""{"query":"channel","rows":"people","name":"name"}""",
            J(U.Composer(go).Mentions(U.Mentions(new Q<ChannelView, AppMsg>("channel"), v => v.People, f => f.Name)))["composer"]!["mentions"]);
    }

    [Fact]
    public void PickDialog()
    {
        var o = J(Overlay.Dialog(NewMessage, s =>
        {
            s.Title("New message");
            var people = s.Query("directory", Query.Call(GetChannel.Ref).Rows(v => v.People));
            var channel = s.Query("channel", Query.Call(GetChannel.Ref));
            return s.Form(StartChat.Ref,
                f =>
                {
                    f.Pick(r => r.To, "To", people, x => x.Name, x => x.Name)
                        .Avatar(x => x.Avatar)
                        .Agent(x => x.Agent)
                        .Tile(x => x.Icon, x => x.Engine)
                        .Detail(x => x.Detail)
                        .Section(x => x.Section)
                        .Exclude(U.Lit<string[]>(["u-1"]))
                        .Empty(U.Text("nobody", "Nobody matches “{q}”.").Arg("q", Form.PickedQuery((StartChatRequest r) => r.To)), "Everyone is here")
                        .Placeholder("Find people")
                        .Required();
                    f.PhoneSubmit("Start conversation");
                    f.Pick(r => r.Text, "Also", channel, v => v.Faces, x => x.Id, x => x.Name);
                    f.NoteWhen("Agents act only on messages that mention them.", Form.PickedAgents((StartChatRequest r) => r.To).Gt(0));
                },
                U.Text("start_with", "Start with {first}").Arg("first", Form.PickedFirst((StartChatRequest r) => r.To)),
                (a, _) => a.CallForm(StartChat.Ref).Close());
        }).Obj)["layout"]!["form"]!;
        Same("""
            [{"inputs":[
              {"name":"to","control":"pick","label":"To","placeholder":"Find people","required":true,
               "pick":{"query":"directory","value":"name","label":"name","avatar":"avatar","agent":"agent","icon":"icon",
                 "engine":"engine","detail":"detail","section":"section","exclude":{"lit":["u-1"]},
                 "empty":{"key":"nobody","args":{"q":{"form":"to.query"}}},
                 "empty_done":"Everyone is here"}},
              {"name":"text","control":"pick","label":"Also","pick":{"query":"channel","rows":"faces","value":"id","label":"name"}}]}]
            """, o["sections"]);
        Same("""
            [{"text":"Agents act only on messages that mention them.","visible":{"gt":[{"form":"to.agents"},{"lit":"0"}]}}]
            """, o["notes"]);
        Same("""{"key":"start_with","args":{"first":{"form":"to.first"}}}""", o["submit_label"]);
        Assert.Equal("Start conversation", o["phone_submit_label"]!.GetValue<string>());
        // a form without notes writes none
        var plain = J(Overlay.Dialog(NewMessage, s => s.Form(StartChat.Ref, f => f.Text(r => r.Text, "Text"), "Send",
            (a, _) => a.CallForm(StartChat.Ref))).Obj)["layout"]!["form"]!.AsObject();
        Assert.False(plain.ContainsKey("notes"));
    }
}
