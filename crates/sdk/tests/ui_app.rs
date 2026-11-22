//! An app-like screen of catalog 3 (a messenger: a header, a stream of posts with a run of work, a strip of live
//! runs, a composer, a side panel, a pick dialog) described by the Rust SDK validates with the platform's own
//! validator.

use celobus_sdk::prelude::*;
use celobus_sdk::ui::*;

#[derive(Message, Clone, Debug, Default)]
pub struct Face {
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub agent: bool,
    pub icon: String,
    pub engine: String,
    pub detail: String,
    pub section: String,
    pub online: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Reaction {
    pub emoji: String,
    pub count: i32,
    pub mine: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Call {
    pub call: String,
    pub detail: String,
    pub state: String,
    pub output: String,
    pub truncated: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct WorkRow {
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub tools: Vec<Call>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Work {
    pub state: String,
    pub label: String,
    pub meta: String,
    pub started: i64,
    pub rows: Vec<WorkRow>,
    pub steps: i32,
    pub saying: String,
    pub retry_label: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Summary {
    pub count: i32,
    pub faces: Vec<Face>,
    pub last: String,
    pub working: String,
    pub since: i64,
    pub waiting: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Msg {
    pub id: String,
    pub by: String,
    pub time: String,
    pub text: String,
    pub writing: bool,
    pub kind: String,
    pub edited: bool,
    pub pinned: bool,
    pub mine: bool,
    pub receipt: String,
    pub receipt_tone: String,
    pub receipt_to: String,
    pub why: String,
    pub fix_label: String,
    pub reactions: Vec<Reaction>,
    pub thread: Option<Summary>,
    pub work: Option<Work>,
    pub run: String,
    pub can_retry: bool,
    pub duration: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Run {
    pub run: String,
    pub agent: String,
    pub icon: String,
    pub engine: String,
    pub state: String,
    pub since: i64,
    pub until: i64,
    pub sub: String,
    pub block: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ChannelView {
    pub title: String,
    pub icon: String,
    pub description: String,
    pub members: String,
    pub faces: Vec<Face>,
    pub messages: Vec<Msg>,
    pub people: Vec<Face>,
    pub older_page_token: String,
    pub pinned_count: i32,
    pub readonly: bool,
    pub placeholder: String,
    pub following: bool,
    pub ends: i64,
}

#[derive(Message, Clone, Debug, Default)]
pub struct LiveView {
    pub blocks: Vec<Msg>,
    pub people: Vec<Face>,
    pub runs: Vec<Run>,
    pub label: String,
    pub can_stop: bool,
    pub hint: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct PeopleView {
    pub people: Vec<Face>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct RunView {
    pub title: String,
    pub status: RunStatus,
    pub note: String,
    pub call: Option<Call>,
    pub steps: Vec<WorkRow>,
}

#[derive(Message, Clone, Debug, Default, PartialEq)]
pub enum RunStatus {
    #[default]
    Running,
    Done,
}

#[derive(Message, Clone, Debug, Default)]
pub struct GetChannelRequest {
    pub id: String,
    pub page_token: String,
    pub tab: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct GetLiveRequest {
    pub id: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct GetPeopleRequest {
    pub id: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct GetRunRequest {
    pub id: String,
    pub call: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct PreviewRequest {
    pub channel: String,
    pub text: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct PostRequest {
    pub channel: String,
    pub text: String,
    pub reply: String,
    pub also: bool,
    pub audio: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Posted {
    pub message: Option<Msg>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct MessageRequest {
    pub message: String,
    pub text: String,
    pub emoji: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct FollowRequest {
    pub id: String,
    pub follow: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct StartRequest {
    pub to: String,
    pub text: String,
}

#[derive(Message, Clone, Debug, Default)]
pub struct Done {
    pub ok: bool,
}

#[derive(Message, Clone, Debug, Default)]
pub struct ChannelParams {
    pub c: Option<String>,
    pub run: Option<String>,
}

#[derive(Message, Clone, Debug, Default)]
pub struct NoParams {}

pub const CHANNEL: ScreenKey<ChannelParams> = ScreenKey::new("channel");
pub const RUNS: Topic<NoParams> = Topic::new("runs");
pub const NEW_MESSAGE: OverlayKey<NoParams> = OverlayKey::new("new_message");

async fn get_channel(_: Ctx, _: GetChannelRequest) -> Result<ChannelView> {
    Ok(ChannelView::default())
}
async fn get_live(_: Ctx, _: GetLiveRequest) -> Result<LiveView> {
    Ok(LiveView::default())
}
async fn get_people(_: Ctx, _: GetPeopleRequest) -> Result<PeopleView> {
    Ok(PeopleView::default())
}
async fn get_run(_: Ctx, _: GetRunRequest) -> Result<RunView> {
    Ok(RunView::default())
}
async fn preview(_: Ctx, _: PreviewRequest) -> Result<LiveView> {
    Ok(LiveView::default())
}
async fn post_message(_: Ctx, _: PostRequest) -> Result<Posted> {
    Ok(Posted::default())
}
async fn on_message(_: Ctx, _: MessageRequest) -> Result<Done> {
    Ok(Done::default())
}
async fn follow(_: Ctx, _: FollowRequest) -> Result<Done> {
    Ok(Done::default())
}
async fn start(_: Ctx, _: StartRequest) -> Result<Done> {
    Ok(Done::default())
}

fn new_message() -> Overlay {
    Overlay::dialog(NEW_MESSAGE, |s| {
        s.title("New message");
        let people = s.query(
            "directory",
            Query::call(get_people).rows(PeopleView::people),
        );
        s.form(
            start,
            |f| {
                f.pick(StartRequest::to, "To", &people, Face::name, Face::name)
                    .avatar(Face::avatar)
                    .agent(Face::agent)
                    .tile(Face::icon, Face::engine)
                    .detail(Face::detail)
                    .section(Face::section)
                    .empty(
                        t("nobody", "Nobody matches “{q}”.")
                            .arg("q", picked_query(StartRequest::to)),
                        "Everyone is here",
                    )
                    .placeholder("Find people");
                f.phone_submit("Start conversation");
                f.note_when(
                    "Agents act only on messages that mention them.",
                    picked_agents(StartRequest::to).gt(0),
                );
            },
            t("start_with", "Start with {first}").arg("first", picked_first(StartRequest::to)),
            |a, _| a.call_form(start, |c| c).close(),
        )
    })
}

fn channel_screen() -> Screen {
    Screen::build(CHANNEL, |s| {
        let c = s.param(ChannelParams::c);
        let run = s.param(ChannelParams::run);
        let tab = s.state_reset("tab", "messages".to_string(), &[c.name()]);
        let wide = s.state_reset("wide", false, &[c.name()]);
        let draft = s.state("draft", String::new());
        let who = Via::<Msg, Face>::new("who");

        let channel = s.query(
            "channel",
            Query::call(get_channel)
                .arg(GetChannelRequest::id, c.or_empty())
                .rows(ChannelView::messages)
                .older(ChannelView::older_page_token, GetChannelRequest::page_token)
                .join(&who, ChannelView::people, Face::id, Msg::by),
        );
        let live = s.query(
            "live",
            Query::call(get_live)
                .arg(GetLiveRequest::id, c.or_empty())
                .follow_every(RUNS, 1000)
                .rows(LiveView::blocks)
                .join(&who, LiveView::people, Face::id, Msg::by),
        );
        let people = s.query(
            "people",
            Query::call(get_people)
                .arg(GetPeopleRequest::id, c.or_empty())
                .rows(PeopleView::people),
        );
        let hint = s.query(
            "hint",
            Query::call(preview)
                .arg(PreviewRequest::channel, c.or_empty())
                .arg(PreviewRequest::text, draft.val())
                .debounce_ms(300)
                .when(not(draft.val().empty())),
        );
        let run_q = s.query(
            "run",
            Query::call(get_run)
                .arg(GetRunRequest::id, run.or_empty())
                .when(not(run.val().empty())),
        );

        let send = s.action("send", |a| {
            a.call(post_message, |x| {
                x.arg(PostRequest::channel, c.or_empty())
                    .arg(PostRequest::text, form::text())
                    .arg(PostRequest::reply, form::reply())
                    .arg(PostRequest::audio, form::audio())
                    .arg(PostRequest::also, form::option("also"))
            })
            .merge(&channel, result(Posted::message))
        });
        let on = |s: &mut S<ChannelParams>, name: &str| {
            s.row_action(name, &channel, |a, r| {
                a.call(on_message, |x| {
                    x.arg(MessageRequest::message, r.get(Msg::id))
                })
            })
        };
        let mark_read = on(s, "mark_read");
        let retry = on(s, "retry");
        let correct = on(s, "correct");
        let open_thread = on(s, "open_thread");
        let react = s.row_action("react", &channel, |a, r| {
            a.call(on_message, |x| {
                x.arg(MessageRequest::message, r.get(Msg::id))
                    .arg(MessageRequest::emoji, form::emoji())
            })
        });
        let edit = s.row_action("edit", &channel, |a, r| {
            a.call(on_message, |x| {
                x.arg(MessageRequest::message, r.get(Msg::id))
                    .arg(MessageRequest::text, form::text())
            })
        });
        let open_run = s.row_action("open_run", &channel, |a, r| {
            a.set_param(run, r.get(Msg::run))
        });
        let close_run = s.action("close_run", |a| a.set_param(run, none()));
        let ask = s.row_action("ask", &channel, |a, r| {
            a.draft(t("ask", "@{who} about this: ").arg("who", r.at(who.f(Face::name))))
        });
        let reply = s.row_action("reply", &channel, |a, r| {
            a.reply_to(r.at(who.f(Face::name)), r.get(Msg::id))
        });
        let copy_link = s.action("copy_link", |a| a.copy_link(c.val()));
        let follow_a = s.action("follow", |a| {
            a.call(follow, |x| {
                x.arg(FollowRequest::id, c.or_empty())
                    .arg(FollowRequest::follow, form::pressed())
            })
        });
        let message_person = s.action("message_person", |a| {
            a.call(start, |x| x.arg(StartRequest::to, form::to()))
        });
        let stop_dm = s.action("stop_dm", |a| a.call(on_message, |x| x));
        let new_msg = s.action("new_message", |a| a.open(NEW_MESSAGE, |p| p));
        let stop_run = s.action("stop_run", |a| a.call(on_message, |x| x));

        let msg = post::<Msg>()
            .author(who.f(Face::name))
            .time(Msg::time)
            .avatar(who.f(Face::avatar))
            .tile(who.f(Face::icon), who.f(Face::engine))
            .tile_when(row_at(who.f(Face::agent)).eq(true))
            .badge("AGENT", row_at(who.f(Face::agent)).eq(true))
            .markdown(Msg::text)
            .writing(Msg::writing)
            .audio(Msg::duration)
            .edited(Msg::edited)
            .pinned(Msg::pinned, lit(true))
            .mine(Msg::mine)
            .receipt(Msg::receipt, Msg::receipt_tone, [("done", Tone::Ok)])
            .route(Msg::receipt_to, Msg::receipt, Msg::why)
            .correction(Msg::fix_label, &correct, Msg::time)
            .reactions(Msg::reactions, Reaction::emoji, Reaction::count, &react)
            .reactions_mine(Reaction::mine)
            .quick(["👍", "🔥", "👀", "✅", "🙏"])
            .summary(Msg::thread.then(Summary::count), &open_thread)
            .summary_label(
                t("replies", "{n, plural, one {# reply} other {# replies}}")
                    .arg("n", row_at(Msg::thread.then(Summary::count))),
            )
            .summary_more(
                Msg::thread.then(Summary::faces),
                Msg::thread.then(Summary::last),
                Msg::thread.then(Summary::working),
                Msg::thread.then(Summary::since),
                Msg::thread.then(Summary::waiting),
            )
            .process(Msg::work, |w: ProcessB<Work>| {
                w.state(Work::state)
                    .label(Work::label)
                    .meta(Work::meta)
                    .started(Work::started)
                    .rows(Work::rows)
                    .steps(Work::steps)
                    .saying(Work::saying)
                    .retry(
                        &retry,
                        Work::retry_label,
                        row_value(Msg::can_retry).eq(true),
                    )
                    .log(&open_run)
                    .fetch_call(&run_q, GetRunRequest::call, RunView::call, |a| {
                        a.arg(GetRunRequest::id, row_value(Msg::run))
                    })
            })
            .hover([
                builtin::react(),
                builtin::reply(),
                entry(&ask, "Ask agent")
                    .text_style()
                    .visible(row_at(who.f(Face::agent)).ne(true)),
                builtin::more(),
            ])
            .menu([
                builtin::play(),
                builtin::copy_text(),
                builtin::copy_link(),
                builtin::correction(),
                builtin::edit(),
                builtin::separator(),
                entry(&reply, "Reply").icon("reply"),
            ])
            .sheet([builtin::reply(), builtin::copy_text()])
            .edit(&edit, row_value(Msg::mine).eq(true))
            .actions_when(channel.field(ChannelView::readonly).ne(true))
            .card(
                card_of(&people, Face::name)
                    .avatar(Face::avatar)
                    .agent(Face::agent)
                    .tile(Face::icon, Face::engine)
                    .detail(Face::detail)
                    .action(&message_person)
                    .unknown("Not in this workspace's directory"),
            );

        let column = stack([
            header(channel.field(ChannelView::title))
                .title_icon(channel.field(ChannelView::icon))
                .crumb_icon(channel.field(ChannelView::icon))
                .leading_faces(channel.field(ChannelView::faces))
                .subtitle([
                    seg_text(channel.field(ChannelView::description)),
                    seg_dot_waiting().visible(live.field(LiveView::can_stop).eq(true)),
                    seg_tone_live().visible(live.field(LiveView::can_stop).eq(false)),
                    seg_countdown(channel.field(ChannelView::ends)).prefix("ends in "),
                ])
                .faces(
                    channel.field(ChannelView::faces),
                    channel.field(ChannelView::members),
                    &new_msg,
                )
                .menu([menu_item("Copy link", &copy_link)])
                .menu_label("Conversation actions")
                .faces_aria_label("2 members: open the members")
                .tabs(
                    &tab,
                    [
                        header_tab("messages", "Messages"),
                        header_tab("pinned", "Pinned")
                            .badge(channel.field(ChannelView::pinned_count)),
                    ],
                )
                .tabs_label("Channel")
                .phone_sub(channel.field(ChannelView::members))
                .phone_icons([&new_msg])
                .menu_tabs()
                .into(),
            stream(&channel)
                .key(Msg::id)
                .post_with(msg.clone())
                .kinds_line(Msg::kind, "event", who.f(Face::name), Msg::text, Msg::time)
                .tail(&live)
                .days(Msg::time)
                .jump_working(live.field(LiveView::label))
                .empty("No messages yet.")
                .on_seen(&mark_read)
                .id("messages")
                .visible(tab.val().eq("messages")),
            stream(&channel)
                .key(Msg::id)
                .anchor_top()
                .older_button("Show more pinned messages")
                .post_with(msg.no_summary())
                .visible(tab.val().eq("pinned")),
            composer(&send)
                .for_stream("messages")
                .primary()
                .placeholder(channel.field(ChannelView::placeholder))
                .mentions(
                    mentions(&people, Face::name)
                        .avatar(Face::avatar)
                        .agent(Face::agent)
                        .tile(Face::icon, Face::engine)
                        .detail(Face::detail)
                        .section(Face::section)
                        .note("A mention always reaches the agent"),
                )
                .tags()
                .dictation()
                .reply()
                .draft_state(&draft)
                .option("also", "Also send to the channel")
                .caption_route(hint.field(LiveView::hint))
                .caption_dot(live.field(LiveView::hint))
                .stop(&stop_dm, live.field(LiveView::can_stop).eq(true))
                .stop_label("Stop")
                .stop_aria_label("Stop the agent")
                .readonly(
                    channel.field(ChannelView::readonly).eq(true),
                    channel.field(ChannelView::placeholder),
                )
                .into(),
        ]);
        let panel = stack([
            header("Run")
                .icons([icon_button("bell", "Following", &follow_a).toggle(
                    channel.field(ChannelView::following),
                    "bell-off",
                    "Follow",
                )])
                .expand()
                .close(&close_run)
                .back("Back")
                .into(),
            strip(&live, LiveView::runs, Run::run, Run::agent)
                .label(live.field(LiveView::label))
                .tile(Run::icon, Run::engine)
                .state(
                    Run::state,
                    &["working", "starting"],
                    &["waiting"],
                    &["starting"],
                )
                .since(Run::since)
                .until(Run::until)
                .sub_line(Run::sub)
                .countdown_text("ends in {left}")
                .jump(Run::block, "Jump to turn", "Jump to its last answer")
                .button(
                    strip_button("Stop", &stop_run)
                        .icon("square")
                        .stop_style()
                        .visible(row_value(Run::state).eq("working")),
                )
                .into(),
            agent_run(
                &run_q,
                RunView::title,
                RunView::status,
                [(RunStatus::Done, Tone::Ok)],
                RunView::steps,
                WorkRow::title,
                WorkRow::kind,
            )
            .log()
            .note(RunView::note)
            .step_detail(WorkRow::detail)
            .into(),
            list(&people, Face::name)
                .search("Find people")
                .online(Face::online)
                .limit(5)
                .group_label("People")
                .into(),
        ])
        .visible(not(run.val().empty()));
        split(column, panel)
            .fill()
            .resize(340, 820, 420, 400)
            .expand(&wide)
    })
}

fn plugin() -> Plugin {
    Plugin::new("talk", "1.0.0")
        .topic(RUNS.decl().manual())
        .endpoint(Endpoint::new(get_channel).ui().read_only())
        .endpoint(Endpoint::new(get_live).ui().read_only())
        .endpoint(Endpoint::new(get_people).ui().read_only())
        .endpoint(Endpoint::new(get_run).ui().read_only())
        .endpoint(Endpoint::new(preview).ui().read_only())
        .endpoint(
            Endpoint::new(post_message)
                .ui()
                .idempotency(Idempotency::Optional),
        )
        .endpoint(
            Endpoint::new(on_message)
                .ui()
                .idempotency(Idempotency::Optional),
        )
        .endpoint(
            Endpoint::new(follow)
                .ui()
                .idempotency(Idempotency::Optional),
        )
        .endpoint(Endpoint::new(start).ui().idempotency(Idempotency::Optional))
        .ui(Ui::new().screen(channel_screen()).overlay(new_message()))
}

#[test]
fn an_app_like_screen_describes_and_validates() {
    let json = plugin().describe_json().expect("describes");
    let d = celobus_descriptor::parse(json.as_bytes()).expect("parses");
    let errors = celobus_descriptor::validate(&d);
    assert!(errors.is_empty(), "{errors:#?}\n{json}");
    assert_eq!(celobus_descriptor::ui_check::ui_warnings(&d), vec![]);
    let ui = d.ui.as_ref().unwrap();
    assert_eq!(ui.catalog, 3);
    let r = celobus_descriptor::ui_check::requirements(ui);
    assert_eq!(r["channel"].compat(3), "full");
    assert_eq!(r["channel"].compat(2), "unsupported");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let screen = &v["ui"]["screens"][0];
    assert_eq!(screen["queries"][0]["join"][0]["as"], "who");
    assert_eq!(screen["queries"][0]["older"]["arg"], "page_token");
    assert_eq!(screen["queries"][1]["follow"][0]["min_interval_ms"], 1000);
    assert_eq!(screen["state"][0]["reset"][0], "c");
    let split = &screen["layout"]["split"];
    assert_eq!(split["fill"], true);
    assert_eq!(split["expand"]["state"], "wide");
    let col = &split["list"]["stack"]["children"];
    let post = &col[1]["stream"]["item"]["post"];
    assert_eq!(post["author"], "who.name");
    assert_eq!(post["summary"]["count"], "thread.count");
    assert_eq!(post["process"]["state"], "work.state");
    assert_eq!(post["process"]["fetch_call"]["call"], "call");
    assert_eq!(post["hover"][2]["style"], "text");
    assert!(col[2]["stream"]["item"]["post"]["summary"].is_null());
    assert_eq!(col[3]["composer"]["for"], "messages");
    assert_eq!(col[3]["composer"]["caption"][0]["style"], "route");
    let panel = &split["detail"]["stack"]["children"];
    assert_eq!(
        panel[0]["header"]["icons"][0]["icon_button"]["icon_off"],
        "bell-off"
    );
    assert_eq!(panel[1]["strip"]["rows"], "runs");
    assert_eq!(panel[2]["agent_run"]["log"], true);
    assert_eq!(panel[3]["list"]["limit"], 5);
    let send = &screen["actions"][0]["steps"];
    assert_eq!(
        send[1]["merge"]["row"],
        serde_json::json!({ "result": "message" })
    );
    assert_eq!(ui.texts["en"]["start_with"], "Start with {first}");
    let head = &col[0]["header"];
    assert_eq!(head["subtitle"][2]["tone"], "live");
    assert_eq!(head["tabs"]["label"], "Channel");
    assert_eq!(head["menu"]["label"], "Conversation actions");
    let stop = &v["ui"]["screens"][0].to_string();
    assert!(stop.contains(r#""aria_label":"Stop the agent""#));
    assert_eq!(head["faces"]["aria_label"], "2 members: open the members");
    let form = &v["ui"]["overlays"][0]["layout"]["form"];
    assert_eq!(form["phone_submit_label"], "Start conversation");
    assert_eq!(
        form["sections"][0]["inputs"][0]["pick"]["empty"]["args"]["q"],
        serde_json::json!({ "form": "to.query" })
    );
}
