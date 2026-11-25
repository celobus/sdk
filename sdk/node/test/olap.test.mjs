// Entities stored in OLAP: what a plugin declares (`storage: 'olap'`, `olapKey`, `load`), what a request for a
// snapshot and its ticket send and read, and what refuses such an entity, against a fake platform (the same cases as
// the Rust SDK's load_tests.rs).
import assert from "node:assert/strict";
import { test } from "node:test";
import { Caller, Ctx, SyscallError } from "../dist/ctx.js";
import { LoadTicket, daysAgo, entity, lakeTable, layer, plugin, role, t } from "../dist/index.js";

const analyst = role({ title: "Analyst" });
const mart = layer({ title: "For reports" });

const store = entity({ columns: { name: t.text() }, access: (a) => a.read(analyst) });

const metrics = lakeTable({
  layer: mart,
  name: "metrics",
  message: "MetricRow",
  columns: { day: t.date(), storeId: t.ref(store), metric: t.text(), value: t.text() },
  partition: ["day"],
  systemRead: true,
});

/** One metric of one store per day. */
const storeMetric = entity({
  storage: "olap",
  olapKey: ["day", "store", "metric"],
  columns: { day: t.date(), store: t.ref(store), metric: t.text(), value: t.decimal().optional() },
  access: (a) => {
    a.read(analyst);
    a.mask("value", [analyst]);
  },
  // the columns in the order of the entity's fields, whatever the order given
  load: (l) =>
    l
      .from(metrics)
      .where(metrics.f.day.geCol(daysAgo(400)))
      .columns({ value: metrics.f.value.parse("decimal"), day: metrics.f.day, store: metrics.f.storeId, metric: metrics.f.metric }),
});

const retail = plugin({
  id: "retail",
  version: "1.0.0",
  roles: { analyst },
  layers: { mart },
  lakeTables: { metrics },
  entities: { store, storeMetric },
});

test("a plugin declares an entity stored in OLAP and its load", () => {
  const d = retail.describe();
  const e = d.entities.find((x) => x.name === "store_metric");
  assert.equal(e.storage, "olap");
  assert.deepEqual(e.olap_key, ["day", "store", "metric"]);
  const plain = d.entities.find((x) => x.name === "store");
  assert.ok(!("storage" in plain) && !("olap_key" in plain));
  const col = (source, field) => ({ column: { source, field } });
  assert.deepEqual(d.loads, [
    {
      entity: "store_metric",
      sources: [{ entity: "metrics", alias: "metrics" }],
      filter: { compare: { left: col("metrics", "day"), op: "ge", right: { days_ago: 400 } } },
      columns: [
        { name: "day", value: col("metrics", "day") },
        { name: "store", value: col("metrics", "store_id") },
        { name: "metric", value: col("metrics", "metric") },
        { name: "value", value: { column: { source: "metrics", field: "value", parse: "decimal" } } },
      ],
    },
  ]);
  assert.equal("loads" in plugin({ id: "plain", version: "1.0.0" }).describe(), false);
});

test("an optional column may be left out of a load; a required one may not", () => {
  const sparse = entity({
    storage: "olap",
    olapKey: ["day"],
    columns: { day: t.date(), note: t.text().optional() },
    load: (l) => l.from(metrics).columns({ day: metrics.f.day }),
  });
  const p = plugin({ id: "retail", version: "1.0.0", roles: { analyst }, layers: { mart }, lakeTables: { metrics }, entities: { store, sparse } });
  assert.deepEqual(p.describe().loads[0].columns, [{ name: "day", value: { column: { source: "metrics", field: "day" } } }]);
  const short = entity({
    storage: "olap",
    olapKey: ["day"],
    columns: { day: t.date(), metric: t.text() },
    load: (l) => l.from(metrics).columns(/** @type {any} */ ({ day: metrics.f.day })),
  });
  const codes = plugin({ id: "retail", version: "1.0.0", roles: { analyst }, layers: { mart }, lakeTables: { metrics }, entities: { store, short } }).check().map((x) => x.code);
  assert.ok(codes.includes("E_LOAD_COLUMN"), codes.join());
});

test("mistakes of an entity stored in OLAP are reported", () => {
  const codes = (entities) => plugin({ id: "retail", version: "1.0.0", roles: { analyst }, layers: { mart }, lakeTables: { metrics }, entities: { store, ...entities } }).check().map((x) => x.code);
  // stored in OLAP without its load
  assert.ok(codes({ none: entity({ storage: "olap", olapKey: ["day"], columns: { day: t.date() } }) }).includes("E_OLAP_LOAD"));
  // a load of an ordinary entity
  const ordinary = entity({ columns: { day: t.date() }, load: (l) => l.from(metrics).columns({ day: metrics.f.day }) });
  assert.ok(codes({ ordinary }).includes("E_LOAD_ENTITY"));
  // a key of an ordinary entity, a key that is not a column
  assert.ok(codes({ keyed: entity({ olapKey: ["day"], columns: { day: t.date() } }) }).includes("E_OLAP_KEY"));
  const bad = entity({ storage: "olap", olapKey: [/** @type {any} */ ("nope")], columns: { day: t.date() }, load: (l) => l.from(metrics).columns({ day: metrics.f.day }) });
  assert.ok(codes({ bad }).includes("E_OLAP_KEY"));
  // another storage
  assert.ok(codes({ odd: entity(/** @type {any} */ ({ storage: "disk", columns: { day: t.date() } })) }).includes("E_OLAP_STORAGE"));
});

const quiet = { debug() {}, info() {}, warn() {}, error() {}, with() { return quiet; } };

function ctxWith(answer, deadlineMs = 60_000) {
  const seen = [];
  const session = { invalidate() {}, async syscall(call) { seen.push(call); return answer(call, seen); } };
  const request = { id: "77", deadline: new Date(Date.now() + deadlineMs), signal: new AbortController().signal };
  return { ctx: new Ctx(session, new Caller(undefined, "retail", () => undefined), request, { id: "retail", version: "1.0.0" }, quiet), seen };
}

test("a request answers a ticket and wait follows it to its end", async () => {
  retail.describe();
  let asked = 0;
  const { ctx, seen } = ctxWith((call) => {
    if (call.analyticsLoad) return { analyticsTicket: { ticket: "t1", state: "PENDING", entities: call.analyticsLoad.entities, requestedAt: "1795000000000" } };
    asked += 1;
    return {
      analyticsTicket: {
        ticket: "t1",
        state: asked >= 2 ? "DONE" : "BUILDING",
        generation: "7",
        loads: [{ entity: "store_metric", rows: "3", loadedAt: "1795000000000", code: "", error: "" }],
      },
    };
  });
  const ticket = await ctx.analytics().snapshot().load(storeMetric).load(storeMetric).request();
  assert.deepEqual([ticket.id, ticket.last.state, ticket.last.entities], ["t1", "pending", ["store_metric"]]);
  assert.equal(ticket.last.requestedAt.toISOString(), "2026-11-18T11:06:40.000Z");
  const done = await ticket.wait();
  assert.deepEqual([done.state, done.generation, done.loads[0].rows], ["done", 7, 3]);
  assert.ok(done.loads[0].loadedAt instanceof Date);
  assert.deepEqual(seen[0], { analyticsLoad: { requestId: "77", entities: ["store_metric"] } });
  assert.deepEqual(seen.slice(1).map((c) => c.analyticsLoadStatus), [
    { requestId: "77", ticket: "t1", waitMs: 30_000 },
    { requestId: "77", ticket: "t1", waitMs: 30_000 },
  ]);
  // a ticket another call was given, asked now
  const later = LoadTicket.of(ctx, "t1");
  assert.equal((await later.status()).state, "done");
  assert.equal(seen.at(-1).analyticsLoadStatus.waitMs, 0);
});

test("a failed ticket is its load's error, and refusals carry their status", async () => {
  retail.describe();
  for (const [code, status] of [
    ["analytic_load_rows_limit", 422],
    ["analytic_load_time_limit", 422],
    ["analytic_load_duplicate", 422],
    ["analytic_load_invalid", 422],
    ["analytic_load_failed", 503],
    ["", 503],
  ]) {
    const { ctx } = ctxWith(() => ({ analyticsTicket: { ticket: "t2", state: "FAILED", code, error: "the load failed" } }));
    await assert.rejects(LoadTicket.of(ctx, "t2").wait(), { status, code: code || "analytic_load_failed" });
  }
  for (const [answer, status] of [
    ["analytic_load_limit: past the bound", 429],
    ["analytic_load_busy: 16 waits", 503],
    ["not_loaded: no load yet", 409],
    ["not_found: no such ticket", 404],
  ]) {
    const { ctx } = ctxWith(() => {
      throw new SyscallError(answer);
    });
    await assert.rejects(ctx.analytics().snapshot().load(storeMetric).request(), { status, code: answer.split(":")[0] });
  }
  // an ordinary entity, or none, is no load: refused before anything is sent
  const { ctx, seen } = ctxWith(() => ({}));
  await assert.rejects(ctx.analytics().snapshot().load(/** @type {any} */ (store)).request(), { status: 400, code: "invalid" });
  await assert.rejects(ctx.analytics().snapshot().request(), { status: 400, code: "invalid" });
  assert.equal(seen.length, 0);
});

test("an entity stored in OLAP is read with ctx.analytics() only", async () => {
  retail.describe();
  const { ctx, seen } = ctxWith(() => ({ dbRows: { rows: [], nextPageToken: "" } }));
  await ctx.analytics().query(storeMetric).where(storeMetric.f.metric.eq("footfall")).all();
  assert.equal(seen[0].query.engine, "ANALYTIC");
  assert.throws(() => ctx.db(/** @type {any} */ (storeMetric)), (e) => e.code === "internal" && /OLAP/.test(e.cause.message));
  assert.throws(() => ctx.query(/** @type {any} */ (storeMetric)), (e) => e.code === "internal" && /OLAP/.test(e.cause.message));
  assert.throws(() => ctx.asSystem("x").db(/** @type {any} */ (storeMetric)), (e) => e.code === "internal");
});

test("an entity stored in OLAP's own rows: its columns and the moment they were loaded", async () => {
  retail.describe();
  const row = { id: "0190f3c2-0000-7000-8000-000000000101", day: "2026-11-20", store: "0190f3c2-0000-7000-8000-000000000201", metric: "footfall", value: "12.5", createdAt: "2026-11-21T02:00:00Z", updatedAt: "2026-11-21T02:00:00Z" };
  const { ctx } = ctxWith(() => ({ dbRows: { rows: [Buffer.from(JSON.stringify(row))], nextPageToken: "", asOf: "1795140000000", generation: "7" } }));
  const r = await ctx.analytics().query(storeMetric).allAsOf();
  assert.equal(r.rows[0].metric, "footfall");
  assert.equal(r.rows[0].value.toString(), "12.5");
  assert.equal(r.rows[0].createdAt.toISOString(), "2026-11-21T02:00:00.000Z");
  assert.equal(r.generation, 7n);
});

test("a wait in a request ends before its deadline with the ticket pending", async () => {
  retail.describe();
  const { ctx, seen } = ctxWith(async (call) => {
    if (call.analyticsLoadStatus) await new Promise((r) => setTimeout(r, Math.min(call.analyticsLoadStatus.waitMs, 500)));
    return { analyticsTicket: { ticket: "t3", state: "PENDING", notBefore: "1795000600000" } };
  }, 4_000);
  const ticket = await ctx.analytics().snapshot().load(storeMetric).request();
  assert.equal(ticket.last.notBefore.getTime(), 1795000600000);
  const began = Date.now();
  await assert.rejects(ticket.wait(), (e) => {
    assert.deepEqual([e.status, e.code], [503, "analytic_load_pending"]);
    assert.deepEqual(e.details, { ticket: "t3", state: "pending", not_before: 1795000600000 });
    return true;
  });
  assert.ok(Date.now() - began < 3_000, "ended before the deadline");
  // every ask waited no longer than what was left
  for (const c of seen.slice(1)) assert.ok(c.analyticsLoadStatus.waitMs <= 2_000, JSON.stringify(c));
});
