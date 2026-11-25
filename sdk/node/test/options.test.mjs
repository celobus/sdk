// Options a declaration helper does not know are definition errors (E_OPTION_UNKNOWN), naming the key and the helper:
// a plugin written in plain JavaScript, or against a newer SDK, fails loudly instead of declaring something else.
import assert from "node:assert/strict";
import { test } from "node:test";
import { endpoint, entity, job, lakeTable, layer, notification, plugin, role, t, view } from "../dist/index.js";

const codes = (p) => p.check().filter((e) => e.code === "E_OPTION_UNKNOWN").map((e) => [e.path, e.message.split(":")[0]]);

test("an option a helper does not know is a definition error naming it", () => {
  const audit = layer({ title: "Audit", colour: "red" });
  const member = role({ title: "Member", rank: 1 });
  // as plain JavaScript or a spread from an untyped object would write it
  const daily = entity({ ...{ storage: "olap" }, ...{ olapKeys: ["day"] }, columns: { day: t.date(), ref: t.ref("crm.customer", { navv: "x" }) } });
  const loose = entity({ columns: { name: t.text() }, history: { layer: audit, keepAll: true }, indexes: [{ columns: ["name"], uniq: true }] });
  const raw = lakeTable({ layer: audit, columns: { day: t.date() }, exportModels: true });
  const v = view({ columns: { n: t.bigint() }, query: (q) => q.from(loose).columns({ n: loose.f.name }), cached: true });
  const hello = endpoint({ input: t.message("HelloRequest", {}, { descr: "x" }), output: {}, roles: [member], handler: () => ({}), retries: 2 });
  const nightly = job({ payload: {}, handler() {}, retry: { retries: 1, jitter: true }, cronn: "* * * * *" });
  const ping = notification("ping", { title: "Ping", sound: true });
  const p = plugin({
    id: "loose",
    version: "1.0.0",
    roles: { member },
    scopes: { export: { title: "Export", secret: true } },
    capabilities: { "push:send": { reason: "pings", why: "x" } },
    agents: { helper: { roles: [member], engin: "claude" } },
    dependencies: [{ plugin: "crm", calls: [], read: [] }],
    layers: { audit },
    entities: { daily, loose },
    views: { v },
    lakeTables: { raw },
    endpoints: { hello },
    jobs: { nightly },
    notifications: [ping],
    lakeModels: [],
  });
  const got = codes(p);
  const paths = got.map(([path]) => path);
  for (const path of [
    "plugin.lakeModels",
    "roles.member.rank",
    "scopes.export.secret",
    'capabilities."push:send".why',
    "agents.helper.engin",
    "dependencies[0].read",
    "entities.daily.olapKeys",
    "entities.daily.columns.ref.navv",
    "entities.loose.history.keepAll",
    "entities.loose.indexes[0].uniq",
    "views.v.cached",
    "layers.audit.colour",
    "lakeTables.raw.exportModels",
    "endpoints.hello.retries",
    "messages.HelloRequest.descr",
    "jobs.nightly.cronn",
    "jobs.nightly.retry.jitter",
    "notifications[0].sound",
  ]) {
    assert.ok(paths.includes(path), `${path} not in ${JSON.stringify(paths)}`);
  }
  assert.ok(got.some(([, m]) => m === "entity() has no option `olapKeys`"), JSON.stringify(got));
  // --describe refuses it as any definition error
  assert.throws(() => p.describe(), (e) => e.name === "DescribeErrors" && /E_OPTION_UNKNOWN/.test(e.message));
});

test("another plugin's entity knows only its own options", () => {
  const theirs = entity({ plugin: "crm", name: "customer", columns: { name: t.text() }, olapKey: ["name"] });
  const p = plugin({ id: "reader", version: "1.0.0", historyDependencies: [theirs] });
  assert.deepEqual(codes(p), [["historyDependencies[0].olapKey", "entity() of another plugin has no option `olapKey`"]]);
});

test("a plugin with known options only has no such error", () => {
  const p = plugin({ id: "tidy", version: "1.0.0", roles: { member: role({ title: "Member" }) } });
  assert.deepEqual(codes(p), []);
});

test("a t.ref inside t.array is checked too", () => {
  const p = plugin({ id: "arr", version: "1.0.0", entities: { tagged: entity({ columns: { refs: t.array(t.ref("crm.customer", { invers: "x" })) } }) } });
  assert.deepEqual(codes(p), [["entities.tagged.columns.refs.invers", "t.ref() has no option `invers`"]]);
});

test("a call given an option it does not know is refused before anything is sent", async () => {
  const { Caller, Ctx } = await import("../dist/ctx.js");
  const { lakeTable, layer, job } = await import("../dist/index.js");
  const mart = layer({ title: "Mart" });
  const stock = lakeTable({ layer: mart, name: "stock", columns: { day: t.date() }, partition: ["day"] });
  const nightly = job({ payload: {}, handler() {} });
  const member = role({ title: "Member" });
  const note = entity({ columns: { title: t.text() } });
  plugin({ id: "calls", version: "1.0.0", roles: { member }, layers: { mart }, lakeTables: { stock }, entities: { note }, jobs: { nightly } }).describe();
  const seen = [];
  const quiet = { debug() {}, info() {}, warn() {}, error() {}, with() { return quiet; } };
  const session = { invalidate() {}, async syscall(call) { seen.push(call); return {}; } };
  const request = { id: "1", deadline: new Date(Date.now() + 30_000), signal: new AbortController().signal };
  const ctx = new Ctx(session, new Caller(undefined, "calls", () => undefined), request, { id: "calls", version: "1.0.0" }, quiet);
  const refused = (re) => (e) => e.code === "internal" && re.test(String(e.cause?.message));
  // a misspelt partition would refresh the whole table
  await assert.rejects(ctx.lake().refresh(stock, { partiton: stock.f.day.eq("2026-01-01") }), refused(/refresh\(\) has no option `partiton`/));
  await assert.rejects(ctx.lake().table(stock).append([], { kye: "x" }), refused(/append\(\) has no option `kye`/));
  await assert.rejects(ctx.db(note).get("x", { withDeletd: true }), refused(/get\(\) has no option `withDeletd`/));
  await assert.rejects(ctx.transaction(async () => 1, { readonly: true }), refused(/transaction\(\) has no option `readonly`/));
  await assert.rejects(ctx.jobs.enqueue(nightly, {}, { singelton: "a" }), refused(/enqueue\(\) has no option `singelton`/));
  await assert.rejects(ctx.agents.run("helper", "hi", { sesion: true }), refused(/run\(\) has no option `sesion`/));
  await assert.rejects(ctx.agents.list({ agnt: "x" }), refused(/list\(\) has no option `agnt`/));
  await assert.rejects(ctx.agents.events("r", { afer: 1 }), refused(/events\(\) has no option `afer`/));
  await assert.rejects(ctx.notifications.follow({ id: "k" }, { reson: "manual" }), refused(/follow\(\) has no option `reson`/));
  assert.equal(seen.length, 0);
});
