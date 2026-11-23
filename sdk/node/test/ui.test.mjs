// UI declarations: the shop conformance plugin, every builder's JSON, contributions, texts and definition errors.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { Decimal, VIZ_KINDS, endpoint, entity, plugin, role, t, topic, ui } from "../dist/index.js";
import shop, { cancelOrder, countOpen, getOrder, listOrders } from "./fixtures/shop.mjs";

const here = dirname(fileURLToPath(import.meta.url));

test("the shop conformance plugin's descriptor is sdk/conformance/shop/descriptor.json", () => {
  const expected = JSON.parse(readFileSync(join(here, "../../conformance/shop/descriptor.json"), "utf8"));
  assert.deepEqual(shop.describe(), expected);
  assert.deepEqual(shop.check(), []);
});

test(".ui(), .readOnly() and .invalidates() return new endpoints that UI references resolve through", () => {
  const ui1 = listOrders.ui();
  assert.notEqual(ui1, listOrders);
  assert.equal(listOrders.options.ui, undefined);
  assert.equal(ui1.root, listOrders);
  assert.equal(ui1.readOnly().invalidates().root, listOrders);
  const d = shop.describe();
  assert.deepEqual(
    d.endpoints.map((e) => [e.name, e.ui, e.read_only, e.invalidates?.length]),
    [
      ["ListOrders", true, true, undefined],
      ["GetOrder", true, true, undefined],
      ["CancelOrder", true, undefined, 2],
      ["CountOpen", true, true, undefined],
      ["Sales", true, true, undefined],
      ["TopProducts", true, true, undefined],
      ["GetActivity", true, true, undefined],
      ["GetLive", true, true, undefined],
      ["GetWatchers", true, true, undefined],
      ["PreviewNote", true, true, undefined],
      ["GetJob", true, true, undefined],
      ["AddNote", true, undefined, 1],
      ["NoteAction", true, undefined, undefined],
      ["Watch", true, undefined, undefined],
      ["Assign", true, undefined, undefined],
    ],
  );
  void [cancelOrder, countOpen, getOrder];
});

// A plugin around one screen, for looking at the JSON of its parts.
const status = t.enum("Status", ["open", "in_progress", "done"]);
const item = t.message("Item", {
  id: t.uuid(),
  title: t.text(),
  ownerId: t.text(),
  amount: t.decimal(),
  share: t.double(),
  status,
  tags: t.array(t.text()),
  dueOn: t.date(),
  updatedAt: t.timestamp(),
  done: t.bool(),
});
const listRequest = t.message("ListItemsRequest", { status: status.optional(), sortBy: t.text(), after: t.text() });
const listResponse = t.message("ListItemsResponse", { items: t.array(item), cursor: t.text(), total: t.bigint() });
const saveRequest = t.message("SaveItemRequest", {
  id: t.uuid(),
  title: t.text(),
  notes: t.text(),
  amount: t.decimal(),
  dueOn: t.date(),
  at: t.timestamp(),
  urgent: t.bool(),
  watch: t.bool(),
  ownerId: t.text(),
  status,
  labels: t.array(status),
  kind: status,
  file: t.bytes(),
  query: t.text(),
  parentId: t.uuid(),
});
const params = t.message("ItemsParams", { status: status.optional(), q: t.text().optional(), from: t.date().optional() });
const ITEMS = topic("items", t.message("ItemsKey", {}));
const listItems = endpoint({ input: listRequest, output: listResponse, readOnly: true, handler: () => ({}) });
const saveItem = endpoint({ input: saveRequest, output: item, idempotency: "required", handler: () => ({}) });
const EDIT = ui.overlayKey("edit", t.message("EditParams", { id: t.uuid() }));
const ITEMS_SCREEN = ui.screenKey("items", params);

function describeScreen(build, extra = {}) {
  return plugin({
    id: "items",
    version: "1.0.0",
    endpoints: { listItems: listItems.ui(), saveItem: saveItem.ui().invalidates(ITEMS) },
    topics: [ITEMS],
    ui: ui.define({ screens: [ui.screen(ITEMS_SCREEN, build)], ...extra }),
  }).describe();
}

test("screens: params from the message, state, queries and their options", () => {
  const d = describeScreen((s) => {
    const st = s.param(params.f.status);
    s.state("view", t.text(), "list");
    s.state("count", t.bigint(), 3n);
    s.query(
      "items",
      ui.call(listItems, { status: st, sortBy: "title" })
        .rows(listResponse.f.items)
        .feed(listRequest.f.after, listResponse.f.cursor, item.f.id)
        .follow(ITEMS, 500)
        .poll(30)
        .maxAge(60),
    );
    s.subtitle("All of them");
    return ui.divider();
  });
  const sc = d.ui.screens[0];
  assert.equal(d.ui.catalog, 3);
  assert.equal(sc.title, "Items");
  assert.equal(sc.subtitle, "All of them");
  assert.deepEqual(sc.params, [
    { name: "status", type: "enum:Status", required: false },
    { name: "q", type: "string", required: false },
    { name: "from", type: "date", required: false },
  ]);
  assert.deepEqual(sc.state, [
    { name: "view", type: "string", default: "list" },
    { name: "count", type: "int64", default: "3" },
  ]);
  assert.deepEqual(sc.queries, [{
    name: "items",
    endpoint: "ListItems",
    args: [{ field: "status", value: { param: "status" } }, { field: "sort_by", value: { lit: "title" } }],
    rows: "items",
    feed: { after: "after", cursor: "cursor", key: "id" },
    follow: [{ topic: "items", min_interval_ms: 500 }],
    poll: 30,
    max_age: 60,
  }]);
  assert.deepEqual(sc.layout, { divider: {} });
  assert.deepEqual(d.endpoints[1].invalidates, [{ topic: "items" }]);
  assert.equal(d.endpoints[1].read_only, undefined);
  assert.deepEqual(d.topics, [{ name: "items", key: "ItemsKey", description: "" }]);
});

test("report screens: a param with a default, a segmented over a param, a command, a viz link with args and breadcrumbs", () => {
  const grain = t.enum("Grain", ["day", "week"]);
  const trendParams = t.message("TrendParams", { range: t.text().optional(), grain: grain.optional(), top: t.int(), status: status.optional() });
  const TREND = ui.screenKey("trend", trendParams);
  const d = plugin({
    id: "items",
    version: "1.0.0",
    endpoints: { listItems: listItems.ui() },
    ui: ui.define({
      commands: [
        ui.command.navigate("trend", "Trend", TREND),
        ui.command.navigate("trend_week", "Trend: last 7 days", TREND, { range: "7", grain: "week" }).icon("chart-line"),
        ui.command.open("edit", "Edit", EDIT, { id: "0b0e7c1e-58a5-4b53-9d0c-3a1e2f6f7a10" }),
      ],
      overlays: [ui.dialog(EDIT, () => ui.divider())],
      screens: [
        ui.screen(TREND, (s) => {
          const range = s.paramOr(trendParams.f.range, "30");
          const by = s.paramOr(trendParams.f.grain, "day");
          const top = s.paramOr(trendParams.f.top, 5);
          const view = s.state("view", t.text(), "chart");
          const items = s.query("items", ui.call(listItems, { sortBy: range }).rows(listResponse.f.items));
          s.action("week", (a) => a.setParam(range, "7"));
          return ui.stack([
            ui.segmented(range, [["7", "7 days"], ["30", "30 days"]]),
            ui.segmented(by, [["day", "Days"], ["week", ui.text("trend.weeks", "Weeks")]]),
            ui.segmented(top, [[5, "Top 5"], [10, "Top 10"]]),
            ui.segmented(view, [["chart", "Chart"], ["table", "Table"]]),
            ui.segmented(s.param(trendParams.f.status), [["open", "Open"], ["done", "Done"]]),
            ui.viz.bar.bar().labels(items, item.f.title).open(TREND, { range: "7", status: "done", grain: by }),
            ui.link("Done", TREND, { status: "done", range }),
            ui.breadcrumbs([
              ui.crumb("Trend", TREND),
              ui.crumb(ui.text("trend.done", "Done"), TREND, { status: "done", grain: by }),
              ui.crumb("Week"),
            ]),
          ]);
        }),
      ],
    }),
  }).describe();
  const sc = d.ui.screens[0];
  // a default makes a param optional, whatever its field is
  assert.deepEqual(sc.params, [
    { name: "range", type: "string", required: false, default: "30" },
    { name: "grain", type: "enum:Grain", required: false, default: "day" },
    { name: "top", type: "int32", required: false, default: 5 },
    { name: "status", type: "enum:Status", required: false },
  ]);
  assert.deepEqual(sc.queries[0].args, [{ field: "sort_by", value: { param: "range" } }]);
  const nodes = sc.layout.stack.children;
  assert.deepEqual(nodes[0].segmented, { bind: { param: "range" }, options: [{ value: "7", label: "7 days" }, { value: "30", label: "30 days" }] });
  assert.deepEqual(nodes[1].segmented, { bind: { param: "grain" }, options: [{ value: "day", label: "Days" }, { value: "week", label: { key: "trend.weeks" } }] });
  assert.deepEqual(nodes[2].segmented, { bind: { param: "top" }, options: [{ value: 5, label: "Top 5" }, { value: 10, label: "Top 10" }] });
  assert.deepEqual(nodes[3].segmented.bind, { state: "view" });
  assert.deepEqual(nodes[4].segmented.bind, { param: "status" });
  assert.deepEqual(nodes[5].viz.open, {
    screen: "trend",
    args: [{ field: "range", value: { lit: "7" } }, { field: "status", value: { lit: "done" } }, { field: "grain", value: { param: "grain" } }],
  });
  assert.deepEqual(nodes[6].link.target, { screen: "trend", args: [{ field: "status", value: { lit: "done" } }, { field: "range", value: { param: "range" } }] });
  // a breadcrumb trail: crumbs that link to screens, with args like a link, and the last one a text
  assert.deepEqual(nodes[7].breadcrumbs, {
    items: [
      { text: "Trend", target: { screen: "trend" } },
      { text: { key: "trend.done" }, target: { screen: "trend", args: [{ field: "status", value: { lit: "done" } }, { field: "grain", value: { param: "grain" } }] } },
      { text: "Week" },
    ],
  });
  assert.deepEqual(d.ui.commands, [
    { id: "trend", title: "Trend", target: { screen: "trend" } },
    { id: "trend_week", title: "Trend: last 7 days", icon: "chart-line", target: { screen: "trend", args: [{ field: "range", value: { lit: "7" } }, { field: "grain", value: { lit: "week" } }] } },
    { id: "edit", title: "Edit", target: { overlay: "edit", args: [{ field: "id", value: { lit: "0b0e7c1e-58a5-4b53-9d0c-3a1e2f6f7a10" } }] } },
  ]);
});

test("actions: every step kind, conditions and values", () => {
  const d = describeScreen((s) => {
    const st = s.param(params.f.status);
    const view = s.state("view", t.text(), "list");
    const items = s.query("items", ui.call(listItems, {}).rows(listResponse.f.items));
    s.rowAction("all", items, (a, r) =>
      a
        .confirm(ui.confirm("Sure?").text("Really.").confirmLabel("Yes").typed(r.get(item.f.title)))
        .confirm("Plain?")
        .call(saveItem, { id: r.get(item.f.id), urgent: true, amount: Decimal.from("1.50"), at: new Date(0) })
        .pending(items)
        .toast("Saved", "ok")
        .open(EDIT, { id: ui.result(item.f.id) })
        .close()
        .navigate(ITEMS_SCREEN, { status: "done" })
        .back()
        .set(view, "board")
        .setParam(st, "open")
        .refresh()
        .refresh(items, "other")
        .openUrl("https://example.com")
        .onError((e) => e.toast(ui.errorMessage()).toastError()),
    );
    const v = ui.row(item.f.status);
    return ui.stack([
      ui.button("x").visible(v.eq("open").and(v.ne("done")).or(ui.not(v.empty()))),
      ui.text(ui.callerName()).enabled(v.isIn(["open", "in_progress"])),
      ui.text("t").visible(ui.row(item.f.amount).gt(ui.lit(Decimal.from("1"))).and(ui.row(item.f.amount).lt(Decimal.from("9")))),
      ui.text(ui.callerId()).fallback(ui.text(ui.errorCode())),
    ]);
  });
  const sc = d.ui.screens[0];
  assert.deepEqual(sc.actions[0].steps, [
    { confirm: { title: "Sure?", text: "Really.", confirm_label: "Yes", typed: { row: "title" } } },
    { confirm: { title: "Plain?" } },
    {
      call: {
        endpoint: "SaveItem",
        args: [
          { field: "id", value: { row: "id" } },
          { field: "urgent", value: { lit: true } },
          { field: "amount", value: { lit: "1.50" } },
          { field: "at", value: { lit: "1970-01-01T00:00:00.000Z" } },
        ],
        pending: "items",
      },
    },
    { toast: { text: "Saved", tone: "ok" } },
    { open: { overlay: "edit", args: [{ field: "id", value: { result: "id" } }] } },
    { close: {} },
    { navigate: { screen: "items", args: [{ field: "status", value: { lit: "done" } }] } },
    { back: {} },
    { set: { state: "view", value: { lit: "board" } } },
    { set: { param: "status", value: { lit: "open" } } },
    { refresh: {} },
    { refresh: { queries: ["items", "other"] } },
    { open_url: { value: { lit: "https://example.com" } } },
  ]);
  assert.deepEqual(sc.actions[0].on_error, [
    { toast: { text: { error: "message" } } },
    { toast: { tone: "danger", error: true } },
  ]);
  const [b, t1, t2, t3] = sc.layout.stack.children;
  assert.deepEqual(b.visible, {
    or: [
      { and: [{ eq: [{ row: "status" }, { lit: "open" }] }, { ne: [{ row: "status" }, { lit: "done" }] }] },
      { not: { empty: { row: "status" } } },
    ],
  });
  assert.deepEqual(t1, {
    text: { text: { caller: "display_name" }, style: "body" },
    enabled: { in: { value: { row: "status" }, set: ["open", "in_progress"] } },
  });
  assert.deepEqual(t2.visible, {
    and: [{ gt: [{ row: "amount" }, { lit: "1" }] }, { lt: [{ row: "amount" }, { lit: "9" }] }],
  });
  assert.deepEqual(t3.fallback, { text: { text: { error: "code" }, style: "body" } });
});

test("the catalog's primitives", () => {
  const d = describeScreen((s) => {
    const st = s.param(params.f.status);
    const q = s.param(params.f.q);
    const from = s.param(params.f.from);
    const view = s.state("view", t.text(), "list");
    const selected = s.state("selected", t.uuid());
    const items = s.query("items", ui.call(listItems, {}).rows(listResponse.f.items));
    const one = s.query("one", ui.call(listItems, {}));
    const go = s.action("go", (a) => a.back());
    return ui.stack([
      ui.row([ui.divider()]).gap("tight"),
      ui.columns([ui.divider()]),
      ui.section("S", [ui.divider()]).description("D").actions([ui.iconButton("plus", "Add", go)]).footer([ui.divider()]),
      ui.card([ui.divider()]).titled("C"),
      ui.tabs([ui.tab("a", "A", [ui.divider()]).badge(one.field(listResponse.f.total)), ui.tab("b", "B", [])]).bind(view),
      ui.split(ui.list(items, item.f.title), ui.divider()),
      ui.segmented(view, [["list", "List"], ["board", "Board"]]),
      ui.link("Open", ITEMS_SCREEN, { status: "open" }),
      ui
        .table(items)
        .col(item.f.title, "Title")
        .mono(item.f.id, "Id")
        .number(item.f.amount, "N", 2)
        .money(item.f.amount, "M", item.f.title)
        .moneyIn(item.f.amount, "USD", "USD")
        .date(item.f.dueOn, "Due")
        .datetime(item.f.updatedAt, "At")
        .relative(item.f.updatedAt, "Ago")
        .member(item.f.ownerId, "Owner")
        .avatar(item.f.ownerId, "Face")
        .tags(item.f.tags, "Tags")
        .percent(item.f.share, "Share")
        .bool(item.f.done, "Done")
        .status(item.f.status, "Status", { in_progress: "attention" })
        .sortable(item.f.title, "Sorted", "sortBy")
        .open(go)
        .rowButton(ui.menu([ui.menuItem("Delete", go).danger().icon("trash").when(ui.row(item.f.done).eq(false)).section("Danger")]))
        .toolbar([ui.filterBar([st.select(), q.search("Find").placeholder("…"), from.date("From").help("h"), q.choices("Any of").option("a", "A")])])
        .empty("Nothing")
        .select(selected, item.f.id)
        .rowTone((r) => r.get(item.f.status).eq("done"), "ok"),
      ui
        .list(items, item.f.title)
        .subtitle(item.f.ownerId)
        .metaRelative(item.f.updatedAt)
        .avatar(item.f.ownerId)
        .icon("file")
        .status(item.f.status, { done: "ok" })
        .open(go)
        .empty("None"),
      ui.fields([ui.item("Total", one.field(listResponse.f.total)).number(), ui.item("At", one.path("x.y")).relative()]).titled("F"),
      ui.metric("Count", one.field(listResponse.f.total)).delta(one.field(listResponse.f.total)).caption("cap").percent(1),
      ui.barChart(items, item.f.dueOn).series(item.f.amount, "Amount").title("Bars").moneyIn("EUR"),
      ui.lineChart(items, item.f.dueOn),
      ui.areaChart(items, item.f.dueOn),
      ui.pieChart(items, item.f.status),
      ui.timeline(items, item.f.title, item.f.updatedAt).actor(item.f.ownerId).tone(item.f.status, { done: "ok" }),
      ui.status(ui.row(item.f.status)),
      ui.tag("new"),
      ui.avatar(ui.callerId()),
      ui.avatarStack(ui.row(item.f.tags)).max(3),
      ui.meter(ui.row(item.f.share), ui.lit(1)).caption("m"),
      ui.progress(ui.row(item.f.share)),
      ui.heading("H"),
      ui.caption("c"),
      ui.mono("m"),
      ui.text("x").moneyIn("USD"),
      ui.copyField("Id", ui.row(item.f.id)),
      ui.empty("Nothing", "yet").icon("inbox").withAction(ui.button("Add").run(go).primary().icon("plus")),
      ui.notice("attention", "Careful").title("Note").withAction(ui.button("Go", go).ghost()),
    ]);
  });
  const c = d.ui.screens[0].layout.stack.children;
  const node = (i) => c[i];
  assert.deepEqual(node(0), { stack: { direction: "horizontal", gap: "tight", children: [{ divider: {} }] } });
  assert.deepEqual(node(1), { columns: { children: [{ divider: {} }] } });
  assert.deepEqual(node(2), {
    section: {
      title: "S",
      description: "D",
      actions: [{ icon_button: { icon: "plus", label: "Add", action: "go" } }],
      children: [{ divider: {} }],
      footer: [{ divider: {} }],
    },
  });
  assert.deepEqual(node(3), { card: { title: "C", children: [{ divider: {} }] } });
  assert.deepEqual(node(4), {
    tabs: {
      bind: { state: "view" },
      tabs: [
        { id: "a", title: "A", badge: { data: { query: "one", field: "total" } }, children: [{ divider: {} }] },
        { id: "b", title: "B", children: [] },
      ],
    },
  });
  assert.deepEqual(node(5), { split: { list: { list: { query: "items", title: "title" } }, detail: { divider: {} } } });
  assert.deepEqual(node(6), {
    segmented: { bind: { state: "view" }, options: [{ value: "list", label: "List" }, { value: "board", label: "Board" }] },
  });
  assert.deepEqual(node(7), { link: { text: "Open", target: { screen: "items", args: [{ field: "status", value: { lit: "open" } }] } } });
  const tbl = node(8).table;
  assert.deepEqual(tbl.columns.map((x) => [x.field, x.format, x.align, x.sort]), [
    ["title", undefined, undefined, undefined],
    ["id", { mono: {} }, undefined, undefined],
    ["amount", { number: { decimals: 2 } }, "end", undefined],
    ["amount", { money: { currency: "title" } }, "end", undefined],
    ["amount", { money: { code: "USD" } }, "end", undefined],
    ["due_on", { date: {} }, undefined, undefined],
    ["updated_at", { datetime: {} }, undefined, undefined],
    ["updated_at", { relative: {} }, undefined, undefined],
    ["owner_id", { member: {} }, undefined, undefined],
    ["owner_id", { avatar: {} }, undefined, undefined],
    ["tags", { tags: {} }, undefined, undefined],
    ["share", { percent: {} }, undefined, undefined],
    ["done", { bool: {} }, undefined, undefined],
    ["status", { status: { tones: { in_progress: "attention" } } }, undefined, undefined],
    ["title", undefined, undefined, "sort_by"],
  ]);
  assert.equal(tbl.open, "go");
  assert.deepEqual(tbl.row_actions, [{
    menu: {
      icon: "more-horizontal",
      items: [{ label: "Delete", action: "go", danger: true, icon: "trash", visible: { eq: [{ row: "done" }, { lit: false }] }, section: "Danger" }],
    },
  }]);
  assert.deepEqual(tbl.toolbar[0].filter_bar.inputs, [
    {
      name: "status",
      control: "select",
      label: "Status",
      options: [{ value: "open", label: "Open" }, { value: "in_progress", label: "In progress" }, { value: "done", label: "Done" }],
    },
    { name: "q", control: "search", label: "Find", placeholder: "…" },
    { name: "from", control: "date", label: "From", help: "h" },
    { name: "q", control: "multi_select", label: "Any of", options: [{ value: "a", label: "A" }] },
  ]);
  assert.equal(tbl.empty, "Nothing");
  assert.deepEqual(tbl.select, { state: "selected", key: "id" });
  assert.deepEqual(tbl.row_tone, [{ when: { eq: [{ row: "status" }, { lit: "done" }] }, tone: "ok" }]);
  assert.deepEqual(node(9), {
    list: {
      query: "items",
      title: "title",
      subtitle: "owner_id",
      meta: "updated_at",
      meta_format: { relative: {} },
      avatar: "owner_id",
      icon: "file",
      status: { field: "status", tones: { done: "ok" } },
      open: "go",
      empty: "None",
    },
  });
  assert.deepEqual(node(10), {
    fields: {
      title: "F",
      items: [
        { label: "Total", value: { data: { query: "one", field: "total" } }, format: { number: {} } },
        { label: "At", value: { data: { query: "one", field: "x.y" } }, format: { relative: {} } },
      ],
    },
  });
  assert.deepEqual(node(11), {
    metric: {
      title: "Count",
      value: { data: { query: "one", field: "total" } },
      delta: { data: { query: "one", field: "total" } },
      caption: "cap",
      format: { percent: { decimals: 1 } },
    },
  });
  assert.deepEqual(node(12), {
    chart: {
      kind: "bar",
      query: "items",
      x: "due_on",
      series: [{ field: "amount", label: "Amount" }],
      title: "Bars",
      format: { money: { code: "EUR" } },
    },
  });
  assert.deepEqual([node(13), node(14), node(15)].map((x) => x.chart.kind), ["line", "area", "pie"]);
  assert.deepEqual(node(16), {
    timeline: { query: "items", text: "title", time: "updated_at", actor: "owner_id", tone: { field: "status", tones: { done: "ok" } } },
  });
  assert.deepEqual(node(17), { status: { value: { row: "status" } } });
  assert.deepEqual(node(18), { tag: { text: "new" } });
  assert.deepEqual(node(19), { avatar: { value: { caller: "user_id" } } });
  assert.deepEqual(node(20), { avatar_stack: { value: { row: "tags" }, max: 3 } });
  assert.deepEqual(node(21), { meter: { value: { row: "share" }, max: { lit: 1 }, caption: "m" } });
  assert.deepEqual(node(22), { progress: { value: { row: "share" } } });
  assert.deepEqual([node(23), node(24), node(25)].map((x) => x.text.style), ["heading", "caption", "mono"]);
  assert.deepEqual(node(26), { text: { text: "x", style: "body", format: { money: { code: "USD" } } } });
  assert.deepEqual(node(27), { copy_field: { value: { row: "id" }, label: "Id" } });
  assert.deepEqual(node(28), {
    empty: {
      title: "Nothing",
      text: "yet",
      icon: "inbox",
      action: { button: { label: "Add", action: "go", emphasis: "primary", icon: "plus" } },
    },
  });
  assert.deepEqual(node(29), {
    notice: { tone: "attention", title: "Note", text: "Careful", action: { button: { label: "Go", action: "go", emphasis: "ghost" } } },
  });
});

test("charts and figures: viz kinds, options, every bind source, selection, link and fallback", () => {
  const d = describeScreen(
    (s) => {
      const q = s.param(params.f.q);
      const picked = s.state("picked", t.text(), "");
      const items = s.query("items", ui.call(listItems, {}).rows(listResponse.f.items));
      const column = (query, field, as) => ({ column: { query, field, ...(as ? { as } : {}) } });
      return ui.stack([
        // the contract's node: every source, in its order
        ui.viz.line
          .line()
          .title("Revenue")
          .range(ui.text("range.month", "Last 30 days"))
          .options({ fmt: "money" })
          .option("curve", "smooth")
          .dates(items, item.f.dueOn)
          .values(items, item.f.amount)
          .serie(items, item.f.amount, "Amount")
          .pivot(items, item.f.dueOn, item.f.status, item.f.amount)
          .asDate()
          .rows(items, { name: item.f.title, value: item.f.amount })
          .textKeys(["name"])
          .value("value", items.field(listResponse.f.total))
          .height(280)
          .selectState(picked)
          .open(ITEMS_SCREEN, { status: "open" }),
        // series of several fields, other destinations, a bind by hand, a fallback
        ui.viz.bar
          .column()
          .labels(items, item.f.title)
          .series(items, [[item.f.amount, "Amount"], [item.f.share, ui.text("viz.share", "Share")]])
          .serie(items, item.f.done, "Done")
          .column("targets", items, item.f.share)
          .column("notes", items, item.f.title)
          .asText()
          .rows(items, { name: item.f.title })
          .to("parts")
          .bind("extra", column("items", "share"))
          .selectParam(q)
          .fallback(ui.table(items).col(item.f.title, "Title")),
        // one value: a query's field, a state, a param, literals
        ui.viz.figure
          .metric()
          .value("value", items.field(listResponse.f.total))
          .value("center.value", picked)
          .value("q", q)
          .value("target", 12.5)
          .value("unit", "items")
          .value("big", 3n),
        ui.viz("meter", "meterList").visible(picked.val().empty()),
        ui.viz.meter.meterList(),
      ]);
    },
    {
      home: [
        ui.home("chart", "Chart", (w) => {
          const rows = w.query("rows", ui.call(listItems, {}).rows(listResponse.f.items));
          return ui.viz.part.donut().rows(rows, { name: item.f.title, value: item.f.amount }).to("parts").open(ITEMS_SCREEN);
        }),
      ],
    },
  );
  assert.equal(d.ui.catalog, 3);
  const c = d.ui.screens[0].layout.stack.children;
  assert.deepEqual(c[0], {
    viz: {
      family: "line",
      kind: "line",
      title: "Revenue",
      range: { key: "range.month" },
      options: { fmt: "money", curve: "smooth" },
      bind: [
        { to: "labels", from: { column: { query: "items", field: "due_on", as: "date" } } },
        { to: "values", from: { column: { query: "items", field: "amount" } } },
        { to: "series", from: { series: { query: "items", fields: [{ field: "amount", label: "Amount" }] } } },
        { to: "series", from: { pivot: { query: "items", x: "due_on", series: "status", value: "amount", as: "date" } } },
        { to: "rows", from: { rows: { query: "items", map: { name: "title", value: "amount" }, text: ["name"] } } },
        { to: "value", from: { value: { data: { query: "items", field: "total" } } } },
      ],
      height: 280,
      select: { state: "picked" },
      open: { screen: "items", args: [{ field: "status", value: { lit: "open" } }] },
    },
  });
  assert.deepEqual(c[1], {
    viz: {
      family: "bar",
      kind: "column",
      bind: [
        { to: "labels", from: { column: { query: "items", field: "title", as: "text" } } },
        {
          to: "series",
          from: {
            series: {
              query: "items",
              fields: [
                { field: "amount", label: "Amount" },
                { field: "share", label: { key: "viz.share" } },
                { field: "done", label: "Done" },
              ],
            },
          },
        },
        { to: "targets", from: { column: { query: "items", field: "share" } } },
        { to: "notes", from: { column: { query: "items", field: "title", as: "text" } } },
        { to: "parts", from: { rows: { query: "items", map: { name: "title" } } } },
        { to: "extra", from: { column: { query: "items", field: "share" } } },
      ],
      select: { param: "q" },
    },
    fallback: { table: { query: "items", columns: [{ field: "title", title: "Title" }] } },
  });
  assert.deepEqual(c[2].viz.bind, [
    { to: "value", from: { value: { data: { query: "items", field: "total" } } } },
    { to: "center.value", from: { value: { state: "picked" } } },
    { to: "q", from: { value: { param: "q" } } },
    { to: "target", from: { value: { lit: 12.5 } } },
    { to: "unit", from: { value: { lit: "items" } } },
    { to: "big", from: { value: { lit: "3" } } },
  ]);
  assert.deepEqual(Object.keys(c[2].viz).sort(), ["bind", "family", "kind"]);
  assert.deepEqual(c[3], { viz: { family: "meter", kind: "meterList" }, visible: { empty: { state: "picked" } } });
  assert.deepEqual(c[4], { viz: { family: "meter", kind: "meterList" } });
  assert.equal(d.ui.texts.en["range.month"], "Last 30 days");
  assert.equal(d.ui.texts.en["viz.share"], "Share");
  // a home widget binds its document-level query
  assert.deepEqual(d.ui.home[0].node, {
    viz: {
      family: "part",
      kind: "donut",
      bind: [{ to: "parts", from: { rows: { query: "home_chart_rows", map: { name: "title", value: "amount" } } } }],
      open: { screen: "items" },
    },
  });
  // a constructor per kind of the catalogue, from the table the validator reads
  const kinds = Object.entries(VIZ_KINDS).flatMap(([family, list]) => list.map((kind) => [family, kind]));
  assert.equal(Object.keys(VIZ_KINDS).length, 9);
  assert.equal(kinds.length, 82);
  for (const [family, kind] of kinds) {
    assert.deepEqual(ui.viz[family][kind]().json().viz, { family, kind });
  }
  assert.deepEqual(ui.viz.flow.funnel().json().viz, { family: "flow", kind: "funnel" });
  assert.deepEqual(ui.viz("bar", "funnel").json().viz, { family: "bar", kind: "funnel" });
});

test("dashboards: a board of spans on the 12-column grid, with a fallback", () => {
  const d = describeScreen((s) => {
    const items = s.query("items", ui.call(listItems, {}).rows(listResponse.f.items));
    const tile = (title) => ui.viz.figure.metric().title(title).value("value", items.field(listResponse.f.total));
    return ui
      .board()
      .quarter(tile("A"))
      .third(tile("B"))
      .half(tile("C"))
      .span(8, ui.viz.line.line().dates(items, item.f.dueOn).values(items, item.f.amount))
      .span(12, ui.heading("Groups"))
      .full(ui.table(items).col(item.f.title, "Title"))
      .fallback(ui.text("Open the dashboard in a newer app"));
  });
  const layout = d.ui.screens[0].layout;
  const tile = (title) => ({
    viz: { family: "figure", kind: "metric", title, bind: [{ to: "value", from: { value: { data: { query: "items", field: "total" } } } }] },
  });
  // the full width is the default span: it is left out
  assert.deepEqual(layout, {
    board: {
      children: [
        { span: 3, node: tile("A") },
        { span: 4, node: tile("B") },
        { span: 6, node: tile("C") },
        {
          span: 8,
          node: {
            viz: {
              family: "line",
              kind: "line",
              bind: [
                { to: "labels", from: { column: { query: "items", field: "due_on", as: "date" } } },
                { to: "values", from: { column: { query: "items", field: "amount" } } },
              ],
            },
          },
        },
        { node: { text: { text: "Groups", style: "heading" } } },
        { node: { table: { query: "items", columns: [{ field: "title", title: "Title" }] } } },
      ],
    },
    fallback: { text: { text: "Open the dashboard in a newer app", style: "body" } },
  });
  assert.deepEqual(ui.board().json(), { board: { children: [] }, visible: undefined, enabled: undefined, fallback: undefined });
});

test("forms: sections, controls, options from enums and queries, the submit chain", () => {
  const d = describeScreen((s) => {
    const items = s.query("items", ui.call(listItems, {}).rows(listResponse.f.items));
    return s.form(
      saveItem,
      (f) => {
        f.text(saveRequest.f.title, "Title").required().initial(ui.callerName());
        f.section("More");
        f.textarea(saveRequest.f.notes, "Notes");
        f.number(saveRequest.f.amount, "Number");
        f.money(saveRequest.f.amount, "Money");
        f.date(saveRequest.f.dueOn, "Due");
        f.datetime(saveRequest.f.at, "At");
        f.checkbox(saveRequest.f.urgent, "Urgent");
        f.switch(saveRequest.f.watch, "Watch");
        f.member(saveRequest.f.ownerId, "Owner");
        f.select(saveRequest.f.status, "Status");
        f.multiSelect(saveRequest.f.labels, "Labels");
        f.radio(saveRequest.f.kind, "Kind");
        f.upload(saveRequest.f.file, "File");
        f.search(saveRequest.f.query, "Search").option("a", "A");
        f.selectFrom(saveRequest.f.parentId, "Parent", items, item.f.id, item.f.title);
        f.cancel("Back");
      },
      ui.text("form.save", "Save"),
      (a, v) => a.callForm(saveItem, { title: v.get(saveRequest.f.title) }).pending(items).close(),
    );
  });
  const sc = d.ui.screens[0];
  const form = sc.layout.form;
  assert.equal(form.endpoint, "SaveItem");
  assert.equal(form.submit, "submit_1");
  assert.deepEqual(form.submit_label, { key: "form.save" });
  assert.equal(form.cancel_label, "Back");
  assert.deepEqual(form.sections[0], { inputs: [{ name: "title", control: "text", label: "Title", required: true, initial: { caller: "display_name" } }] });
  assert.equal(form.sections[1].title, "More");
  assert.deepEqual(form.sections[1].inputs.map((i) => [i.name, i.control]), [
    ["notes", "textarea"], ["amount", "number"], ["amount", "money"], ["due_on", "date"], ["at", "datetime"],
    ["urgent", "checkbox"], ["watch", "switch"], ["owner_id", "member"], ["status", "select"], ["labels", "multi_select"],
    ["kind", "radio_cards"], ["file", "upload"], ["query", "search"], ["parent_id", "select"],
  ]);
  const byName = (n, c) => form.sections[1].inputs.find((i) => i.name === n && i.control === c);
  assert.equal(byName("labels", "multi_select").options.length, 3);
  assert.deepEqual(byName("query", "search").options, [{ value: "a", label: "A" }]);
  assert.deepEqual(byName("parent_id", "select").options_from, { query: "items", value: "id", label: "title" });
  assert.deepEqual(sc.actions, [{
    name: "submit_1",
    steps: [
      { call: { endpoint: "SaveItem", args: [{ field: "title", value: { form: "title" } }], form: true, pending: "items" } },
      { close: {} },
    ],
  }]);
  assert.deepEqual(d.ui.texts, { en: { "form.save": "Save" } });
});

test("records: agent runs, row views, repeated form rows, quotes, and the options of catalog 1 nodes", () => {
  const stepMsg = t.message("RunStep", { text: t.text(), state: t.text() });
  const runState = t.enum("RunState", ["running", "done"]);
  const run = t.message("Run", { agent: t.text(), role: t.text(), state: runState, steps: t.array(stepMsg) });
  const runRequest = t.message("GetRunRequest", { id: t.uuid() });
  const line = t.message("Line", { productId: t.uuid(), quantity: t.int() });
  const quoteRequest = t.message("QuoteRequest", { lines: t.array(line) });
  const quoteLine = t.message("QuoteLine", { total: t.decimal() });
  const quote = t.message("Quote", { lines: t.array(quoteLine), total: t.decimal() });
  const getRun = endpoint({ input: runRequest, output: run, readOnly: true, handler: () => ({}) });
  const quoteOrder = endpoint({ input: quoteRequest, output: quote, readOnly: true, handler: () => ({}) });
  const placeOrder = endpoint({ input: quoteRequest, output: quote, handler: () => ({}) });
  const NEW = ui.overlayKey("new_order");
  const d = plugin({
    id: "items",
    version: "1.0.0",
    endpoints: { listItems: listItems.ui(), getRun: getRun.ui(), quoteOrder: quoteOrder.ui(), placeOrder: placeOrder.ui() },
    ui: ui.define({
      screens: [
        ui.screen(ITEMS_SCREEN, (s) => {
          const q = s.param(params.f.q);
          const checked = s.state("checked", t.array(t.uuid()));
          const items = s.query("items", ui.call(listItems, {}).rows(listResponse.f.items).when(q.val().empty()));
          const r = s.query("run", ui.call(getRun, { id: ui.lit("00000000-0000-0000-0000-000000000000") }));
          const go = s.action("go", (a) => a.setParam(q, ui.none()));
          return ui.stack([
            ui.row([ui.label("ACTIVITY"), ui.menuButton("View", "eye", [ui.menuItem("Go", go)])]).between(),
            ui.metric("Due", ui.lit(1)).caption("late").captionTone("danger", "alert"),
            ui.table(items).col(item.f.title, "Title").checks(checked, item.f.id).selectedBy(q, item.f.title),
            ui
              .list(items, item.f.title)
              .meta(item.f.amount)
              .moneyIn("USD")
              .iconField(item.f.status)
              .aside(ui.text("items.times", "×{n}").arg("n", ui.rowValue(item.f.share))),
            ui.timeline(items, item.f.title, item.f.updatedAt).avatar(item.f.ownerId).icon(item.f.status).shortTime(),
            ui.fields([
              ui.item("Owner", ui.lit("x")).chipAvatar(ui.lit("HW")).link(ITEMS_SCREEN, { status: "open" }),
              ui.item("Other", ui.lit("y")).chip(),
            ]),
            ui.filterBar([q.search("Find").icon("search")]),
            ui
              .agentRun(r, run.f.agent, run.f.state, { running: "attention" }, run.f.steps, stepMsg.f.text, stepMsg.f.state)
              .subtitle(run.f.role)
              .stop(go, r.field(run.f.state).eq("running"))
              .message("Ask", go)
              .needs(run.f.role, "Approve", go, run.f.agent)
              .open("Open session", go)
              .changes("Changes", run.f.steps, stepMsg.f.text)
              .earlier(run.f.role, go),
            ui.recordHeader("R").short("#1").compact("T", "S").expand(go),
            ui.table(items).col(item.f.title, "Title").compact(() => ui.rowView("Plain").icon("box").avatar(item.f.ownerId)),
            ui
              .conversation(items, item.f.title, item.f.title, item.f.updatedAt)
              .avatar(item.f.ownerId)
              .badge(item.f.title)
              .receipt(item.f.title, item.f.status, {})
              .card(item.f.title, item.f.title)
              .cardButton(item.f.title, go)
              .composer("Message", go)
              .to("agent", "Agent")
              .to("people", "People"),
            ui.chat(r, go).thread(r, go).run(r, go, go).members(r, go).openThread(go, go).followThread(go).openRun(go).approval(go, go, go).card(go).react(go)
              .editing(go, go, go).choose(go).about(r, go).directory(r, go),
          ]);
        }),
      ],
      overlays: [
        ui.dialog(NEW, (o) => {
          o.kicker("ORDERS");
          const qt = o.query("quote", ui.call(quoteOrder, { lines: ui.formValue(quoteRequest.f.lines) }).rows(quote.f.lines));
          const items = o.query("items", ui.call(listItems, {}).rows(listResponse.f.items));
          return o.form(
            placeOrder,
            (f) => {
              f.repeat(quoteRequest.f.lines, "Lines", (row) =>
                row.selectFrom(line.f.productId, "Product", items, item.f.id, item.f.title).stepper(line.f.quantity, "Qty", 1),
              )
                .aside("Total", qt, quoteLine.f.total, "USD")
                .addLabel("Add a line")
                .half();
              f.summary("Total", "with tax", qt.field(quote.f.total), "USD");
            },
            "Place",
            (a) => a.callForm(placeOrder).close(),
          );
        }),
      ],
    }),
  }).describe();
  const c = d.ui.screens[0].layout.stack.children;
  assert.deepEqual(d.ui.screens[0].queries[0].when, { empty: { param: "q" } });
  assert.deepEqual(d.ui.screens[0].actions[0].steps, [{ set: { param: "q", value: { lit: null } } }]);
  assert.deepEqual(c[0], {
    stack: {
      direction: "horizontal",
      gap: "normal",
      justify: "between",
      children: [
        { text: { text: "ACTIVITY", style: "label" } },
        { menu: { label: "View", icon: "eye", items: [{ label: "Go", action: "go" }] } },
      ],
    },
  });
  assert.deepEqual(c[1].metric, { title: "Due", value: { lit: 1 }, caption: "late", caption_tone: "danger", caption_icon: "alert" });
  assert.deepEqual(c[2].table.checks, { state: "checked", key: "id" });
  assert.deepEqual(c[2].table.select, { state: "q", key: "title" });
  assert.deepEqual(c[3].list, {
    query: "items",
    title: "title",
    meta: "amount",
    meta_format: { money: { code: "USD" } },
    icon_field: "status",
    aside: { key: "items.times", args: { n: { row: "share" } } },
  });
  assert.deepEqual(c[4].timeline, {
    query: "items",
    text: "title",
    time: "updated_at",
    avatar: "owner_id",
    icon: "status",
    time_format: { relative: { short: true } },
  });
  assert.deepEqual(c[5].fields.items, [
    {
      label: "Owner",
      value: { lit: "x" },
      format: { avatar: {} },
      avatar: { lit: "HW" },
      open: { screen: "items", args: [{ field: "status", value: { lit: "open" } }] },
    },
    { label: "Other", value: { lit: "y" }, format: { avatar: {} } },
  ]);
  assert.equal(c[6].filter_bar.inputs[0].icon, "search");
  assert.deepEqual(c[7].agent_run, {
    query: "run",
    title: "agent",
    subtitle: "role",
    status: { field: "state", tones: { running: "attention" } },
    steps: "steps",
    step_text: "text",
    step_state: "state",
    stop: "go",
    stop_when: { eq: [{ data: { query: "run", field: "state" } }, { lit: "running" }] },
    message: { placeholder: "Ask", action: "go" },
    needs: { text: "role", label: "Approve", action: "go", due: "agent" },
    open: { label: "Open session", action: "go" },
    changes: { title: "Changes", field: "steps", text: "text" },
    earlier: { text: "role", action: "go" },
  });
  assert.deepEqual(c[8].record_header, { title: "R", short: "#1", compact_title: "T", compact_subtitle: "S", expand: "go" });
  assert.deepEqual(c[9].table.compact, { title: "Plain", icon: "box", avatar: "owner_id" });
  assert.deepEqual(c[11].chat, {
    channel: "run", thread: "run", run: "run", members: "run", about: "run", directory: "run",
    actions: {
      send: "go", reply: "go", stop: "go", tell: "go", members: "go", thread: "go", close: "go", run: "go", approve: "go", decline: "go", undo: "go", card: "go", react: "go",
      edit: "go", delete: "go", pin: "go", choose: "go", pause: "go", start: "go", follow: "go",
    },
  });
  assert.deepEqual(JSON.parse(JSON.stringify(c[10].conversation)), {
    query: "items",
    author: "title",
    text: "title",
    time: "updated_at",
    avatar: "owner_id",
    badge: "title",
    receipt: { text: "title", tone: { field: "status" } },
    card: { title: "title", text: "title", button: "title", action: "go" },
    composer: { placeholder: "Message", action: "go", to: [{ value: "agent", label: "Agent" }, { value: "people", label: "People" }] },
  });
  const o = d.ui.overlays[0];
  assert.equal(o.kicker, "ORDERS");
  assert.deepEqual(o.queries[0].args, [{ field: "lines", value: { form: "lines" } }]);
  const form = o.layout.form;
  assert.deepEqual(form.sections[0].inputs[0], {
    name: "lines",
    control: "repeat",
    label: "Lines",
    min_items: 1,
    fields: [
      { name: "product_id", control: "select", label: "Product", required: true, options_from: { query: "items", value: "id", label: "title" } },
      { name: "quantity", control: "number", label: "Qty", required: true, initial: { lit: 1 }, stepper: true },
    ],
    aside: { title: "Total", query: "quote", field: "total", format: { money: { code: "USD" } } },
    add_label: "Add a line",
    width: "half",
  });
  assert.deepEqual(form.summary, {
    title: "Total",
    caption: "with tax",
    value: { data: { query: "quote", field: "total" } },
    format: { money: { code: "USD" } },
  });
  assert.equal(d.ui.texts.en["items.times"], "×{n}");
});

test("contributions: nav with badges lifted to document queries, commands, home widgets, overlays, texts", () => {
  const COUNT = t.message("CountReply", { open: t.bigint(), done: t.bigint() });
  const count = endpoint({ input: {}, output: COUNT, readOnly: true, handler: () => ({}) });
  const NEW = ui.overlayKey("new_item");
  const d = plugin({
    id: "items",
    version: "1.0.0",
    endpoints: { listItems: listItems.ui(), count: count.ui() },
    topics: { ITEMS },
    ui: ui.define({
      nav: [
        ui.nav("Items", "box", [
          ui.entry(ITEMS_SCREEN, ui.text("nav.open", "Open ({n})", { n: ui.lit(1) }), { status: "open" })
            .icon("circle")
            .badge(ui.call(count, {}).follow(ITEMS), COUNT.f.open),
          ui.entry(ITEMS_SCREEN, "Again").badge(ui.call(count, {}).follow(ITEMS, 100), COUNT.f.open),
        ]),
        ui.nav("Other").icon("dot").entry(ui.entry(ITEMS_SCREEN, "Done").badge(ui.call(count, {}), COUNT.f.done)),
        ui.nav("Rows").entry(
          ui
            .entryRows("Channels", ui.call(listItems, {}).rows(listResponse.f.items), item.f.title, ITEMS_SCREEN, { status: ui.rowValue(item.f.status) })
            .rowIcon(item.f.status)
            .unread(item.f.share, item.f.share)
            .limit(3)
            .add("New", NEW),
        ),
      ],
      home: [
        ui
          .home("summary", "Summary", (w) => {
            const c = w.query("count", ui.call(count, {}).follow(ITEMS));
            return ui.metric("Open", c.field(COUNT.f.open));
          })
          .opens(ITEMS_SCREEN, { status: "open" }),
      ],
      commands: [
        ui.command.navigate("find", "Find", ITEMS_SCREEN, { q: "x" }),
        ui.command.open("new", ui.text("cmd.new", "New item"), NEW).icon("plus"),
      ],
      screens: [ui.screen(ITEMS_SCREEN, () => ui.divider())],
      overlays: [ui.sidePanel(NEW, (o) => o.size("large").buttons([ui.divider()]).layout(ui.divider())), ui.sheet("peek", () => ui.divider())],
      texts: { de: { "nav.open": "Offen ({n})" }, en: { "cmd.new": "New" } },
    }),
  }).describe();
  assert.deepEqual(d.ui.nav, [
    {
      title: "Items",
      icon: "box",
      entries: [
        {
          title: { key: "nav.open", args: { n: { lit: 1 } } },
          icon: "circle",
          screen: "items",
          args: [{ field: "status", value: { lit: "open" } }],
          badge: { data: { query: "badge_open", field: "open" } },
        },
        { title: "Again", screen: "items", badge: { data: { query: "badge_open", field: "open" } } },
      ],
    },
    { title: "Other", icon: "dot", entries: [{ title: "Done", screen: "items", badge: { data: { query: "badge_done", field: "done" } } }] },
    {
      title: "Rows",
      entries: [{
        title: "Channels",
        rows: {
          query: "rows_title", label: "title", screen: "items", args: [{ field: "status", value: { row: "status" } }], limit: 3,
          icon: "status", badge: "share", mentions: "share", add: { title: "New", overlay: "new_item" },
        },
      }],
    },
  ]);
  // the first badge query of a name wins; follows get 2 s between re-runs unless they say otherwise
  assert.deepEqual(d.ui.queries, [
    { name: "badge_open", endpoint: "Count", follow: [{ topic: "items", min_interval_ms: 2000 }] },
    { name: "badge_done", endpoint: "Count" },
    { name: "rows_title", endpoint: "ListItems", rows: "items" },
    { name: "home_summary_count", endpoint: "Count", follow: [{ topic: "items", min_interval_ms: 2000 }] },
  ]);
  assert.deepEqual(d.ui.home, [{
    id: "summary",
    title: "Summary",
    node: { metric: { title: "Open", value: { data: { query: "home_summary_count", field: "open" } } } },
    open: { screen: "items", args: [{ field: "status", value: { lit: "open" } }] },
  }]);
  assert.deepEqual(d.ui.commands, [
    { id: "find", title: "Find", target: { screen: "items", args: [{ field: "q", value: { lit: "x" } }] } },
    { id: "new", title: { key: "cmd.new" }, icon: "plus", target: { overlay: "new_item" } },
  ]);
  assert.deepEqual(d.ui.overlays, [
    { id: "new_item", kind: "side_panel", size: "large", title: "New item", buttons: [{ divider: {} }], layout: { divider: {} } },
    { id: "peek", kind: "sheet", size: "medium", title: "Peek", layout: { divider: {} } },
  ]);
  // explicit texts win over the English of ui.text(key, en)
  assert.deepEqual(d.ui.texts, { de: { "nav.open": "Offen ({n})" }, en: { "cmd.new": "New", "nav.open": "Open ({n})" } });
});

test("topics: roles, scopes, authorize, manual, description; keyed invalidations", () => {
  const clerk = role({ title: "Clerk" });
  const KEY = t.message("ThingKey", { id: t.uuid(), rev: t.int() });
  const THING = topic("thing", KEY);
  const thing = t.message("Thing", { id: t.uuid(), rev: t.int() });
  const get = endpoint({ input: { id: t.uuid() }, output: thing, readOnly: true, handler: () => ({}) });
  const d = plugin({
    id: "things",
    version: "1.0.0",
    roles: { clerk },
    scopes: { audit: {} },
    endpoints: {
      get,
      put: endpoint({
        input: thing,
        output: thing,
        ui: true,
        invalidates: [THING.by(KEY.f.id, thing.f.id).and(KEY.f.rev, thing.f.rev)],
        handler: () => ({}),
      }),
    },
    topics: [THING.roles([clerk]).scopes(["audit"]).authorize(get).manual().describe("A thing changed.")],
  }).describe();
  assert.deepEqual(d.topics, [{
    name: "thing",
    key: "ThingKey",
    description: "A thing changed.",
    roles: ["clerk"],
    scopes: ["audit"],
    authorize: "Get",
    manual: true,
  }]);
  assert.deepEqual(d.endpoints[1].invalidates, [{ topic: "thing", key: [{ field: "id", from: "id" }, { field: "rev", from: "rev" }] }]);
  assert.equal(d.endpoints[1].ui, true);
  assert.equal(d.endpoints[0].ui, undefined);
  // the key message follows the endpoints' messages
  assert.deepEqual(d.messages.map((m) => m.name), ["GetRequest", "Thing", "ThingKey"]);
});

test("UI definition errors: endpoints not in the plugin, entity refs, undeclared roles", () => {
  const note = entity({ columns: { title: t.text() } });
  const orphan = endpoint({ input: {}, output: {}, readOnly: true, handler: () => ({}) });
  const NOTE = ui.screenKey("note", t.message("NoteParams", { id: t.ref(note) }));
  const p = plugin({
    id: "notes",
    version: "1.0.0",
    entities: { note },
    topics: [topic("notes", t.message("NotesKey", {})).roles([role({})]).authorize(orphan)],
    ui: ui.define({
      screens: [
        ui.screen(NOTE, (s) => {
          s.query("q", ui.call(orphan, {}));
          return ui.divider();
        }),
      ],
    }),
  });
  assert.deepEqual(p.check().map((e) => `${e.code} ${e.path}`), [
    "E_ENDPOINT_ROLE_UNKNOWN topics[0].roles[0]",
    "E_UI_ENDPOINT_UNKNOWN topics[0].authorize",
    "E_UI_ENDPOINT_UNKNOWN ui.screens[0].queries[0].endpoint",
  ]);
  const ok = plugin({ id: "notes", version: "1.0.0", entities: { note }, ui: ui.define({ screens: [ui.screen(NOTE, () => ui.divider())] }) });
  assert.deepEqual(ok.describe().ui.screens[0].params, [{ name: "id", type: "ref:notes.note", required: true }]);
  assert.throws(() => ui.define({ screens: [ui.screen("x", () => {})] }), /x has no layout/);
});
