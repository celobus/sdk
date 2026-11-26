// UI catalog 3: the app-like primitives (header, stream of posts, process, strip, composer), the pick input and form
// notes, the new options of split, list, icon_button, agent_run and state, the new steps, query options and formats.
import assert from "node:assert/strict";
import { test } from "node:test";
import { endpoint, plugin, t, topic, ui } from "../dist/index.js";

const face = t.message("Face", { id: t.text(), name: t.text(), avatar: t.text(), agent: t.bool(), icon: t.text(), engine: t.text(), role: t.text(), online: t.bool() });
const reaction = t.message("Reaction", { emoji: t.text(), count: t.int(), mine: t.bool() });
const call = t.message("Call", { call: t.text(), detail: t.text() });
const work = t.message("Work", { state: t.text(), label: t.text(), started: t.bigint(), rows: t.array(call), saying: t.text(), retryLabel: t.text() });
const summary = t.message("Summary", { count: t.int(), faces: t.array(face), last: t.text(), working: t.text(), since: t.bigint(), waiting: t.text() });
const msg = t.message("Msg", {
  id: t.text(),
  by: t.text(),
  time: t.bigint(),
  text: t.text(),
  kind: t.text(),
  writing: t.bool(),
  duration: t.int(),
  audioLabel: t.text(),
  audioUrl: t.text(),
  words: t.text(),
  source: t.text(),
  to: t.text(),
  status: t.text(),
  why: t.text(),
  fix: t.text(),
  fixMenu: t.text(),
  fixLead: t.text(),
  fixIcon: t.text(),
  reactions: t.array(reaction),
  thread: summary.optional(),
  work: work.optional(),
  mine: t.bool(),
});
const run = t.message("Run", { run: t.text(), agent: t.text(), state: t.text(), since: t.bigint(), until: t.bigint(), task: t.text(), block: t.text() });
const step = t.message("Step", { text: t.text(), state: t.text(), detail: t.text(), time: t.text(), tool: t.text(), evidence: t.text(), lines: t.text() });
const channelRequest = t.message("GetChannelRequest", { id: t.text(), pageToken: t.text(), call: t.text(), draft: t.text() });
const channelView = t.message("ChannelView", {
  title: t.text(),
  ends: t.bigint(),
  messages: t.array(msg),
  people: t.array(face),
  root: msg.optional(),
  runs: t.array(run),
  olderPageToken: t.text(),
  unread: t.text(),
  following: t.bool(),
  working: t.text(),
  status: t.text(),
  icon: t.text(),
  engine: t.text(),
  steps: t.array(step),
  elapsed: t.bigint(),
  note: t.text(),
  callView: call.optional(),
});
const postRequest = t.message("PostRequest", { channel: t.text(), text: t.text(), reply: t.text(), also: t.bool(), audio: t.text(), duration: t.text(), language: t.text(), words: t.text() });
const posted = t.message("Posted", { message: msg.optional() });
const inviteRequest = t.message("InviteRequest", { channel: t.text(), people: t.text() });
const params = t.message("ChannelParams", { c: t.text().optional(), run: t.text().optional() });
const CHANNEL = topic("channel", t.message("ChannelKey", {}));

const getChannel = endpoint({ input: channelRequest, output: channelView, readOnly: true, handler: () => ({}) });
const postMessage = endpoint({ input: postRequest, output: posted, handler: () => ({}) });
const invite = endpoint({ input: inviteRequest, output: posted, handler: () => ({}) });
const CHANNEL_SCREEN = ui.screenKey("channel", params);

function describeScreen(build, extra = {}) {
  return plugin({
    id: "talk",
    version: "1.0.0",
    endpoints: { getChannel: getChannel.ui(), postMessage: postMessage.ui().invalidates(CHANNEL), invite: invite.ui() },
    topics: [CHANNEL],
    ui: ui.define({ screens: [ui.screen(CHANNEL_SCREEN, build)], ...extra }),
  }).describe();
}

const who = ui.via("who", msg, face);

test("queries: older pages, debounce and joins; state that resets with params", () => {
  const d = describeScreen((s) => {
    const c = s.param(params.f.c);
    s.stateReset("tab", t.text(), "messages", [c, "run"]);
    const draft = s.state("draft", t.text(), "");
    s.query(
      "channel",
      ui.call(getChannel, { id: c, draft })
        .rows(channelView.f.messages)
        .older(channelView.f.olderPageToken, channelRequest.f.pageToken)
        .debounceMs(300)
        .join(who, channelView.f.people, face.f.id, msg.f.by)
        .join(ui.via("author", msg, face), channelView.f.people, face.f.name, msg.f.by)
        .follow(CHANNEL),
    );
    return ui.divider();
  });
  const sc = d.ui.screens[0];
  assert.deepEqual(sc.state, [
    { name: "tab", type: "string", default: "messages", reset: ["c", "run"] },
    { name: "draft", type: "string", default: "" },
  ]);
  assert.deepEqual(sc.queries, [{
    name: "channel",
    endpoint: "GetChannel",
    args: [{ field: "id", value: { param: "c" } }, { field: "draft", value: { state: "draft" } }],
    rows: "messages",
    follow: [{ topic: "channel" }],
    older: { token: "older_page_token", arg: "page_token" },
    debounce_ms: 300,
    join: [
      { as: "who", rows: "people", key: "id", by: "by" },
      { as: "author", rows: "people", key: "name", by: "by" },
    ],
  }]);
});

test("paths: nested fields, joined fields, row values by path", () => {
  assert.equal(ui.path(msg.f.thread, summary.f.count).path, "thread.count");
  assert.equal(ui.path(msg.f.thread, summary.f.faces).dot(face.f.name).path, "thread.faces.name");
  assert.equal(ui.path(msg.f.work, work.f.rows).dot(call.f.detail).path, "work.rows.detail");
  assert.equal(who.f(face.f.name).path, "who.name");
  assert.deepEqual(ui.rowAt(who.f(face.f.agent)).eq(true).json(), { eq: [{ row: "who.agent" }, { lit: true }] });
  assert.deepEqual(ui.rowAt(msg.f.text).json(), { row: "text" });
});

test("steps: draft, reply_to, scroll_to, copy, merge; form values and pick values", () => {
  const d = describeScreen((s) => {
    const c = s.param(params.f.c);
    const msgs = s.query("channel", ui.call(getChannel, { id: c }).rows(channelView.f.messages));
    s.rowAction("quote", msgs, (a, r) => a.draft(ui.text("q", "About {who}: ").arg("who", r.at(who.f(face.f.name)))));
    s.rowAction("answer", msgs, (a, r) => a.replyTo(r.at(who.f(face.f.name)), r.get(msg.f.id)).scrollTo(r.get(msg.f.id)));
    s.action("links", (a) => a.copy(msgs.field(channelView.f.title)).copy("literal").copyLink(c).copyLink(msgs.field(channelView.f.unread)));
    s.action("send", (a) =>
      a.call(postMessage, {
        channel: c,
        text: ui.form.text(),
        reply: ui.form.reply(),
        also: ui.form.option("also"),
        audio: ui.form.audio(),
        duration: ui.form.duration(),
        language: ui.form.language(),
        words: ui.form.words(),
      }).merge(msgs, ui.result(posted.f.message)),
    );
    s.action("values", (a) => a.toast(ui.text("v", "{e} {t} {p}").arg("e", ui.form.emoji()).arg("t", ui.form.to()).arg("p", ui.form.pressed())));
    return ui.divider();
  });
  const steps = Object.fromEntries(d.ui.screens[0].actions.map((a) => [a.name, a.steps]));
  assert.deepEqual(steps.quote, [{ draft: { text: { key: "q", args: { who: { row: "who.name" } } } } }]);
  assert.deepEqual(steps.answer, [
    { reply_to: { name: { row: "who.name" }, key: { row: "id" } } },
    { scroll_to: { key: { row: "id" } } },
  ]);
  assert.deepEqual(steps.links, [
    { copy: { value: { data: { query: "channel", field: "title" } } } },
    { copy: { value: { lit: "literal" } } },
    { copy: { link: { param: "c" } } },
    { copy: { link: { data: { query: "channel", field: "unread" } } } },
  ]);
  assert.deepEqual(steps.send[0].call.args.map((x) => x.value), [
    { param: "c" }, { form: "text" }, { form: "reply" }, { form: "also" }, { form: "audio" }, { form: "duration" },
    { form: "language" }, { form: "words" },
  ]);
  assert.deepEqual(steps.send[1], { merge: { query: "channel", row: { result: "message" } } });
  assert.deepEqual(steps.values[0].toast.text.args, { e: { form: "emoji" }, t: { form: "to" }, p: { form: "pressed" } });
  assert.deepEqual(ui.pickedCount(inviteRequest.f.people).gt(1n).json(), { gt: [{ form: "people.count" }, { lit: "1" }] });
  assert.deepEqual(ui.pickedAgents(inviteRequest.f.people).json(), { form: "people.agents" });
  assert.deepEqual(ui.pickedFirst(inviteRequest.f.people).json(), { form: "people.first" });
  assert.deepEqual(d.ui.catalog, 4);
});

test("options of catalog 1 nodes: split fill, resize and expand; list; toggle; formats; agent_run's log", () => {
  const d = describeScreen((s) => {
    const c = s.param(params.f.c);
    const wide = s.state("wide", t.bool(), false);
    const q = s.query("channel", ui.call(getChannel, { id: c }));
    const people = s.query("people", ui.call(getChannel, { id: c }).rows(channelView.f.people));
    const follow = s.action("follow", (a) => a.toast("ok"));
    return ui.split(
      ui.stack([
        ui.list(people, face.f.name).search("Find someone").online(face.f.online).limit(5).groupLabel("People"),
        ui.iconButton("bell", "Following", follow).toggle(q.field(channelView.f.following), "bell-off", "Follow"),
        ui.text(q.field(channelView.f.ends)).countdown(),
        ui.metric("Running", q.field(channelView.f.elapsed)).elapsed(),
        ui.caption(q.field(channelView.f.elapsed)).span(),
      ]),
      ui.agentRun(q, channelView.f.title, channelView.f.status, { working: "attention" }, channelView.f.steps, step.f.text, step.f.state)
        .log()
        .tile(channelView.f.icon, channelView.f.engine)
        .statusLabel(channelView.f.status)
        .stepLabel(channelView.f.working)
        .elapsed(channelView.f.elapsed)
        .note(channelView.f.note)
        .liveWhen(q.field(channelView.f.status).eq("working"))
        .stepDetail(step.f.detail)
        .stepTime(step.f.time)
        .stepTool(step.f.tool)
        .stepEvidence(step.f.evidence)
        .stepLines(step.f.lines),
    ).fill().resize(340, 820, 420, 400).expand(wide);
  });
  const split = d.ui.screens[0].layout.split;
  assert.equal(split.fill, true);
  assert.deepEqual(split.resize, { min: 340, max: 820, default: 420, keep: 400 });
  assert.deepEqual(split.expand, { state: "wide" });
  const [list, toggle, countdown, elapsed, span] = split.list.stack.children;
  assert.deepEqual(list.list, { query: "people", title: "name", search: "Find someone", online: "online", limit: 5, group_label: "People" });
  assert.deepEqual(toggle.icon_button, {
    icon: "bell",
    label: "Following",
    action: "follow",
    pressed: { data: { query: "channel", field: "following" } },
    icon_off: "bell-off",
    label_off: "Follow",
  });
  assert.deepEqual(countdown.text.format, { countdown: {} });
  assert.deepEqual(elapsed.metric.format, { elapsed: {} });
  assert.deepEqual(span.text.format, { span: {} });
  assert.deepEqual(split.detail.agent_run, {
    query: "channel",
    title: "title",
    status: { field: "status", tones: { working: "attention" } },
    steps: "steps",
    step_text: "text",
    step_state: "state",
    log: true,
    icon: "icon",
    engine: "engine",
    status_label: "status",
    step_label: "working",
    elapsed: "elapsed",
    note: "note",
    live_when: { eq: [{ data: { query: "channel", field: "status" } }, { lit: "working" }] },
    step_detail: "detail",
    step_time: "time",
    step_tool: "tool",
    step_evidence: "evidence",
    step_lines: "lines",
  });
  // a split without the options stays as it was
  assert.deepEqual(describeScreen(() => ui.split(ui.divider(), ui.divider())).ui.screens[0].layout, {
    split: { list: { divider: {} }, detail: { divider: {} } },
  });
});

test("header: leading, subtitle segments, faces, pills, icons, menu, tabs, expand, close, phone", () => {
  const d = describeScreen((s) => {
    const c = s.param(params.f.c);
    const tab = s.state("tab", t.text(), "messages");
    const q = s.query("channel", ui.call(getChannel, { id: c }));
    const open = s.action("open", (a) => a.toast("open"));
    const close = s.action("close", (a) => a.close());
    return ui.stack([
      ui.header(q.field(channelView.f.title))
        .titleIcon("hash")
        .crumbIcon(q.field(channelView.f.icon))
        .leadingFaces(q.field(channelView.f.people))
        .leadingSize(36)
        .subtitle([
          ui.seg.text("Session open"),
          ui.seg.dotWaiting(),
          ui.seg.countdown(q.field(channelView.f.ends)).prefix("ends in ").visible(q.field(channelView.f.ends).gt(0n)),
          ui.seg.dotLive(),
          ui.seg.elapsed(q.field(channelView.f.elapsed)),
          ui.seg.toneLive(),
          ui.seg.toneWaiting(),
        ])
        .subtitleAction(open)
        .context("# returns")
        .faces(q.field(channelView.f.people), "9 members", open)
        .facesWhen(q.field(channelView.f.following).eq(true))
        .facesAriaLabel("9 members: open the members")
        .button("End session", close)
        .buttonWhen(q.field(channelView.f.ends).gt(0n))
        .button("Files", open)
        .icons([ui.iconButton("search", "Search", open)])
        .menu([ui.menuItem("Copy link", open)])
        .menuLabel("Channel actions")
        .tabs(tab, [ui.headerTab("messages", "Messages"), ui.headerTab("files", "Files").badge(q.field(channelView.f.ends)).action(open)])
        .tabsLabel("Channel")
        .expand()
        .close(close)
        .back("Channel")
        .phoneSub("9 members")
        .phoneIcons([open])
        .menuTabs(),
      ui.header("Plain").leadingTile(q.field(channelView.f.icon), q.field(channelView.f.engine)).visible(tab.val().eq("files")),
    ]);
  });
  const [full, plain] = d.ui.screens[0].layout.stack.children;
  const data = (field) => ({ data: { query: "channel", field } });
  assert.deepEqual(full.header, {
    title: data("title"),
    title_icon: { lit: "hash" },
    crumb_icon: data("icon"),
    leading: { faces: data("people"), size: 36 },
    subtitle: [
      { text: "Session open" },
      { dot: "waiting" },
      { value: data("ends"), format: { countdown: {} }, prefix: "ends in ", visible: { gt: [data("ends"), { lit: "0" }] } },
      { dot: "live" },
      { value: data("elapsed"), format: { elapsed: {} } },
      { tone: "live" },
      { tone: "waiting" },
    ],
    subtitle_action: "open",
    context: "# returns",
    faces: { value: data("people"), label: "9 members", action: "open", visible: { eq: [data("following"), { lit: true }] }, aria_label: "9 members: open the members" },
    buttons: [
      { label: "End session", action: "close", visible: { gt: [data("ends"), { lit: "0" }] } },
      { label: "Files", action: "open" },
    ],
    icons: [{ icon_button: { icon: "search", label: "Search", action: "open" } }],
    menu: { items: [{ label: "Copy link", action: "open" }], label: "Channel actions" },
    tabs: {
      bind: { state: "tab" },
      items: [{ id: "messages", title: "Messages" }, { id: "files", title: "Files", badge: data("ends"), action: "open" }],
      label: "Channel",
    },
    expand: true,
    close: "close",
    back: "Channel",
    phone: { sub: "9 members", icons: ["open"], menu_tabs: true },
  });
  assert.deepEqual(plain, {
    header: { title: "Plain", leading: { tile: { icon: data("icon"), engine: data("engine") } } },
    visible: { eq: [{ state: "tab" }, { lit: "files" }] },
  });
});

test("stream and post: every option of the template, a shared template, kinds, tail, head", () => {
  const d = describeScreen((s) => {
    const c = s.param(params.f.c);
    const msgs = s.query("channel", ui.call(getChannel, { id: c }).rows(channelView.f.messages).join(who, channelView.f.people, face.f.id, msg.f.by));
    const live = s.query("live", ui.call(getChannel, { id: c }).rows(channelView.f.messages));
    const people = s.query("people", ui.call(getChannel, { id: c }).rows(channelView.f.people));
    const act = (name) => s.rowAction(name, msgs, (a) => a.toast(name));
    const [react, thread, fix, edit, retry, log, ask, seen, card] = ["react", "thread", "fix", "edit", "retry", "log", "ask", "seen", "card"].map(act);
    const mine = ui.rowValue(msg.f.mine).eq(true);
    const full = ui.post(msg)
      .author(who.f(face.f.name))
      .time(msg.f.time)
      .avatar(who.f(face.f.avatar))
      .tile(who.f(face.f.icon), who.f(face.f.engine))
      .tileWhen(ui.rowAt(who.f(face.f.agent)).eq(true))
      .badge("AGENT", ui.rowAt(who.f(face.f.agent)).eq(true))
      .source(msg.f.source)
      .plain(msg.f.text)
      .writing(msg.f.writing)
      .audio(msg.f.duration)
      .audioMore(msg.f.audioLabel, msg.f.audioUrl, msg.f.words)
      .route(msg.f.to, msg.f.status, msg.f.why)
      .correction(msg.f.fix, fix, msg.f.time)
      .correctionWords(msg.f.fixMenu, msg.f.fixLead, msg.f.fixIcon)
      .correctionWindowMs(60000)
      .reactions(msg.f.reactions, reaction.f.emoji, reaction.f.count, react)
      .summary(ui.path(msg.f.thread, summary.f.count), thread)
      .summaryLabel("{count} replies")
      .summaryMore(
        ui.path(msg.f.thread, summary.f.faces),
        ui.path(msg.f.thread, summary.f.last),
        ui.path(msg.f.thread, summary.f.working),
        ui.path(msg.f.thread, summary.f.since),
        ui.path(msg.f.thread, summary.f.waiting),
      )
      .process(msg.f.work, (w) => w.state(work.f.state).label(work.f.label).started(work.f.started).rows(work.f.rows).saying(work.f.saying).retry(retry, work.f.retryLabel, mine).log(log))
      .hover([ui.builtin.react(), ui.postEntry(ask, "Ask agent").textStyle(), ui.builtin.more()])
      .menu([
        ui.builtin.reply(), ui.builtin.copyText(), ui.builtin.copyLink(), ui.builtin.play(), ui.builtin.edit(),
        ui.builtin.correction(), ui.builtin.separator(), ui.postEntry(fix, "Delete").icon("trash").iconStyle().danger().visible(mine),
      ])
      .sheet([ui.builtin.reply()])
      .edit(edit, mine)
      .actionsWhen(mine)
      .card(ui.cardOf(people, face.f.name).avatar(face.f.avatar).agent(face.f.agent).tile(face.f.icon, face.f.engine).detail(face.f.role).action(card).unknown("Not here"));
    const shared = ui.post(msg).author(msg.f.by).time(msg.f.time).markdown(msg.f.text).summary(ui.path(msg.f.thread, summary.f.count), thread);
    return ui.stack([
      ui.stream(msgs).key(msg.f.id).postWith(full).head(channelView.f.root, "Start of the thread").unread(msgs.field(channelView.f.unread))
        .olderLabel("Load earlier").label("Messages"),
      ui.stream(msgs).key(msg.f.id).postWith(shared).kindsLine(msg.f.kind, "event", who.f(face.f.name), msg.f.text, msg.f.time).tail(live)
        .onSeen(seen),
      ui.stream(msgs).key(msg.f.id).postWith(shared.clone().noSummary()).anchorTop().olderButton("More"),
      ui.stream(live).post((p) => p.author(msg.f.by).time(msg.f.time)),
    ]);
  });
  const [first, second, third, fourth] = d.ui.screens[0].layout.stack.children.map((c) => c.stream);
  const mine = { eq: [{ row: "mine" }, { lit: true }] };
  assert.deepEqual(first, {
    query: "channel",
    key: "id",
    head: { row: "root", rule: "Start of the thread" },
    unread: { data: { query: "channel", field: "unread" } },
    older_label: "Load earlier",
    label: "Messages",
    item: {
      post: {
        author: "who.name",
        time: "time",
        avatar: { url: "who.avatar", tile: { icon: "who.icon", engine: "who.engine", when: { eq: [{ row: "who.agent" }, { lit: true }] } } },
        badge: { text: "AGENT", when: { eq: [{ row: "who.agent" }, { lit: true }] } },
        source: "source",
        text: { field: "text", format: "plain", writing: "writing" },
        audio: { duration: "duration", label: "audio_label", url: "audio_url", words: "words" },
        route: { to: "to", status: "status", why: "why" },
        correction: { label: "fix", action: "fix", from: "time", window_ms: 60000, menu_label: "fix_menu", lead: "fix_lead", icon: "fix_icon" },
        reactions: { field: "reactions", emoji: "emoji", count: "count", action: "react" },
        summary: {
          count: "thread.count",
          action: "thread",
          label: "{count} replies",
          faces: "thread.faces",
          last: "thread.last",
          working: "thread.working",
          since: "thread.since",
          waiting: "thread.waiting",
        },
        process: {
          state: "work.state",
          label: "work.label",
          started: "work.started",
          rows: "work.rows",
          saying: "work.saying",
          retry: { action: "retry", label: "work.retry_label", when: mine },
          log: { action: "log" },
        },
        hover: [{ builtin: "react" }, { action: "ask", label: "Ask agent", style: "text" }, { builtin: "more" }],
        menu: [
          { builtin: "reply" }, { builtin: "copy_text" }, { builtin: "copy_link" }, { builtin: "play" }, { builtin: "edit" },
          { builtin: "correction" }, { builtin: "separator" },
          { action: "fix", label: "Delete", icon: "trash", style: "icon", danger: true, visible: mine },
        ],
        sheet: [{ builtin: "reply" }],
        edit: { action: "edit", when: mine },
        actions_when: mine,
        card: { query: "people", name: "name", avatar: "avatar", agent: "agent", icon: "icon", engine: "engine", detail: "role", action: "card", unknown: "Not here" },
      },
    },
  });
  assert.deepEqual(second, {
    query: "channel",
    key: "id",
    item: { post: { author: "by", time: "time", text: { field: "text" }, summary: { count: "thread.count", action: "thread" } } },
    kinds: { field: "kind", line: { when: "event", author: "who.name", text: "text", time: "time" } },
    tail: { query: "live", hold: true },
    on_seen: { action: "seen" },
  });
  // the clone without its summary leaves the shared template as it was
  assert.deepEqual(third, {
    query: "channel",
    key: "id",
    item: { post: { author: "by", time: "time", text: { field: "text" } } },
    anchor: "top",
    older_load: "button",
    older_label: "More",
  });
  // a stream without a key or a post still writes both (the checker asks for them)
  assert.deepEqual(fourth, { query: "live", key: "", item: { post: { author: "by", time: "time" } } });
});

test("process, strip and composer", () => {
  const d = describeScreen((s) => {
    const c = s.param(params.f.c);
    const draft = s.state("draft", t.text(), "");
    const q = s.query("channel", ui.call(getChannel, { id: c }));
    const people = s.query("people", ui.call(getChannel, { id: c }).rows(channelView.f.people));
    const a = (name) => s.action(name, (x) => x.toast(name));
    const [stop, end, open, send, halt] = ["stop", "end", "open", "send", "halt"].map(a);
    return ui.stack([
      ui.process(q, (w) => w.state(channelView.f.status).label(channelView.f.title).rows(channelView.f.steps)
        .fetchCall(q, channelRequest.f.call, channelView.f.callView, { id: c })),
      ui.strip(q, channelView.f.runs, run.f.run, run.f.agent)
        .state(run.f.state, ["working"], ["waiting"])
        .since(run.f.since)
        .until(run.f.until)
        .subLine(run.f.task)
        .countdownText("ends in {left}")
        .open(open)
        .jump(run.f.block, "Jump to turn", "Jump to its last answer")
        .button(ui.stripButton("Stop", stop).icon("square").stopStyle())
        .button(ui.stripButton("End session", end).textStyle().visible(ui.rowValue(run.f.state).eq("waiting"))),
      ui.composer(send)
        .forStream("messages")
        .placeholder("Message #returns")
        .compact()
        .tools(["add", "emoji", "attach"])
        .mentions(ui.mentions(people, face.f.name).avatar(face.f.avatar).agent(face.f.agent).tile(face.f.icon, face.f.engine)
          .detail(face.f.role).section(face.f.role).note("Agents answer in a thread"))
        .vocabulary("Celobus")
        .option("also", "Also send to #returns")
        .captionRoute(q.field(channelView.f.working))
        .stop(halt, q.field(channelView.f.status).eq("working"))
        .stopAriaLabel("Stop the agent")
        .readOnly(q.field(channelView.f.following).eq(false), "Read only")
        .draftState(draft),
      ui.composer(send).tools([]),
    ]);
  });
  const [process, strip, composer, bare] = d.ui.screens[0].layout.stack.children;
  assert.deepEqual(process.process, {
    query: "channel",
    state: "status",
    label: "title",
    rows: "steps",
    fetch_call: { query: "channel", args: [{ field: "id", value: { param: "c" } }], call: "call", field: "call_view" },
  });
  assert.deepEqual(strip.strip, {
    query: "channel",
    rows: "runs",
    key: "run",
    title: "agent",
    state: { field: "state", live: ["working"], waiting: ["waiting"] },
    since: "since",
    until: "until",
    sub: "task",
    countdown_text: "ends in {left}",
    open: "open",
    jump: { field: "block", label: "Jump to turn", label_waiting: "Jump to its last answer" },
    buttons: [
      { label: "Stop", action: "stop", icon: "square", style: "stop" },
      { label: "End session", action: "end", style: "text", visible: { eq: [{ row: "state" }, { lit: "waiting" }] } },
    ],
  });
  assert.deepEqual(composer.composer, {
    send: "send",
    for: "messages",
    placeholder: "Message #returns",
    compact: true,
    tools: ["add", "emoji", "attach"],
    mentions: {
      query: "people", name: "name", avatar: "avatar", agent: "agent", icon: "icon", engine: "engine", detail: "role", section: "role",
      note: "Agents answer in a thread",
    },
    dictation: { vocabulary: "Celobus" },
    options: [{ name: "also", label: "Also send to #returns" }],
    caption: [{ value: { data: { query: "channel", field: "working" } }, icon: "corner-down-right", style: "route" }],
    stop: { action: "halt", when: { eq: [{ data: { query: "channel", field: "status" } }, { lit: "working" }] }, aria_label: "Stop the agent" },
    readonly: { when: { eq: [{ data: { query: "channel", field: "following" } }, { lit: false }] }, reason: "Read only" },
    draft_state: "draft",
  });
  assert.deepEqual(bare.composer, { send: "send" });
});

test("forms: a pick input over a directory, notes under the inputs", () => {
  const inviteParams = t.message("InviteParams", { c: t.text().optional() });
  const INVITE = ui.overlayKey("invite", inviteParams);
  const d = describeScreen(() => ui.divider(), {
    overlays: [ui.dialog(INVITE, (o) => {
      const c = o.param(inviteParams.f.c);
      const people = o.query("people", ui.call(getChannel, { id: c }).rows(channelView.f.people));
      return o.form(
        invite,
        (f) => {
          f.pick(inviteRequest.f.people, "People", people, face.f.id, face.f.name)
            .avatar(face.f.avatar)
            .agent(face.f.agent)
            .tile(face.f.icon, face.f.engine)
            .detail(face.f.role)
            .section(face.f.role)
            .exclude(ui.lit(["u-1"]))
            .empty(ui.text("invite.none", "Nobody matches “{q}”.").arg("q", ui.pickedQuery(inviteRequest.f.people)), "Everyone is in")
            .help("Who joins");
          f.phoneSubmit("Invite");
          f.noteWhen("Agents join at once.", ui.pickedAgents(inviteRequest.f.people).gt(0n));
          f.noteWhen("Two or more.", ui.pickedCount(inviteRequest.f.people).gt(1n));
        },
        ui.text("invite.submit", "Invite {first}").arg("first", ui.pickedFirst(inviteRequest.f.people)),
        (a) => a.callForm(invite, { channel: c }).close(),
      );
    })],
  });
  const form = d.ui.overlays[0].layout.form;
  assert.deepEqual(form.sections, [{
    inputs: [{
      name: "people",
      control: "pick",
      label: "People",
      help: "Who joins",
      pick: {
        query: "people", value: "id", label: "name", avatar: "avatar", agent: "agent", icon: "icon", engine: "engine", detail: "role",
        section: "role", exclude: { lit: ["u-1"] }, empty: { key: "invite.none", args: { q: { form: "people.query" } } },
        empty_done: "Everyone is in",
      },
    }],
  }]);
  assert.deepEqual(form.notes, [
    { text: "Agents join at once.", visible: { gt: [{ form: "people.agents" }, { lit: "0" }] } },
    { text: "Two or more.", visible: { gt: [{ form: "people.count" }, { lit: "1" }] } },
  ]);
  assert.deepEqual(form.submit_label, { key: "invite.submit", args: { first: { form: "people.first" } } });
  assert.deepEqual(form.phone_submit_label, "Invite");
  assert.deepEqual(d.ui.texts.en["invite.submit"], "Invite {first}");
});
