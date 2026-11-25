// Models of the lake and entities' histories: what a plugin declares (a model of each kind of source, a parsed text,
// `exportToModels`, a history kept per column or `all`), what a refresh and a history read send and answer, against a
// fake platform (the same cases as the Rust SDK's lake_model_tests.rs and history_tests.rs).
import assert from "node:assert/strict";
import { test } from "node:test";
import { Caller, Ctx, SyscallError } from "../dist/ctx.js";
import { Decimal, Filter, entity, historyOf, lakeTable, layer, plugin, role, t } from "../dist/index.js";
import { count, countDistinct, sum } from "../dist/query.js";
import * as platform from "../dist/platform.js";

const analyst = role({ title: "Analyst" });
const agent = role({ title: "Agent" });
const raw = layer({ title: "As it arrived" });
const core = layer({ title: "Cleaned" });
const mart = layer({ title: "For reports" });
const audit = layer({ title: "History of records" });

const stockFile = lakeTable({
  layer: raw,
  name: "stock_file",
  message: "StockFileRow",
  columns: { loadedOn: t.date(), sku: t.text(), onHand: t.text() },
  partition: ["loadedOn"],
  systemRead: true,
});

const stock = lakeTable({
  layer: core,
  columns: { day: t.date(), sku: t.text(), onHand: t.decimal() },
  partition: ["day"],
  systemRead: true,
  // the columns in the order of the table's fields, whatever the order given
  model: (m) =>
    m
      .from(stockFile)
      .where(stockFile.f.sku.ne(""))
      .columns({ onHand: stockFile.f.onHand.parse("decimal"), day: stockFile.f.loadedOn, sku: stockFile.f.sku }),
});

const stockByMonth = lakeTable({
  layer: mart,
  name: "stock_by_month",
  columns: { month: t.date(), skus: t.bigint(), onHand: t.decimal() },
  partition: ["month"],
  read: [analyst],
  exportToModels: true,
  model: (m) =>
    m
      .from(stock)
      .groupBy(stock.f.day.month())
      .columns({ month: stock.f.day.month(), skus: countDistinct(stock.f.sku), onHand: sum(stock.f.onHand) }),
});

const retail = plugin({
  id: "retail",
  version: "1.0.0",
  roles: { analyst },
  layers: { raw, core, mart },
  lakeTables: { stockFile, stock, stockByMonth },
});

test("a plugin declares its models", () => {
  const d = retail.describe();
  const col = (source, field) => ({ column: { source, field } });
  assert.deepEqual(d.lake_models, [
    {
      table: "stock",
      sources: [{ entity: "stock_file", alias: "stock_file" }],
      filter: { compare: { left: col("stock_file", "sku"), op: "ne", values: [""] } },
      columns: [
        { name: "day", value: col("stock_file", "loaded_on") },
        { name: "sku", value: col("stock_file", "sku") },
        { name: "on_hand", value: { column: { source: "stock_file", field: "on_hand", parse: "decimal" } } },
      ],
    },
    {
      table: "stock_by_month",
      sources: [{ entity: "stock", alias: "stock" }],
      group_by: [{ source: "stock", field: "day", bucket: "month" }],
      columns: [
        { name: "month", value: { column: { source: "stock", field: "day", bucket: "month" } } },
        { name: "skus", value: { aggregate: { fn: "count_distinct", column: { source: "stock", field: "sku" } } } },
        { name: "on_hand", value: { aggregate: { fn: "sum", column: { source: "stock", field: "on_hand" } } } },
      ],
    },
  ]);
  // the table other plugins' models may copy, apart from export
  const byMonth = d.lake_tables.find((x) => x.name === "stock_by_month");
  assert.equal(byMonth.export_models, true);
  assert.equal("export" in byMonth, false);
  const plain = plugin({ id: "plain", version: "1.0.0" }).describe();
  assert.equal("lake_models" in plain, false);
});

test("mistakes of a model are reported", () => {
  const codes = (p) => p.check().map((e) => e.code);
  // a column left without a value
  const short = lakeTable({ layer: core, name: "stock", columns: { day: t.date(), sku: t.text() }, model: (m) => m.from(stockFile).columns(/** @type {any} */ ({ day: stockFile.f.loadedOn })) });
  assert.ok(codes(plugin({ id: "retail", version: "1.0.0", layers: { raw, core }, lakeTables: { stockFile, short } })).includes("E_LAKE_MODEL_DEFINE"));
  // a model that does not end with columns
  const open = lakeTable({ layer: core, name: "stock", columns: { day: t.date() }, model: /** @type {any} */ ((m) => m.from(stockFile)) });
  assert.ok(codes(plugin({ id: "retail", version: "1.0.0", layers: { raw, core }, lakeTables: { stockFile, open } })).includes("E_LAKE_MODEL_DEFINE"));
  // a day per day of what is not a history
  const notHistory = lakeTable({ layer: core, name: "stock", columns: { day: t.date() }, model: (m) => m.from(stockFile).eachDay().columns({ day: stockFile.f.loadedOn }) });
  const c = retailCheck(notHistory);
  assert.ok(c.some((e) => e.code === "E_LAKE_MODEL_DEFINE" && /not a history/.test(e.message)), JSON.stringify(c));
});

function retailCheck(table) {
  return plugin({ id: "retail", version: "1.0.0", layers: { raw, core }, lakeTables: { stockFile, table } }).check();
}

const quiet = { debug() {}, info() {}, warn() {}, error() {}, with() { return quiet; } };

/** A context of request 77 whose calls `answer` answers; the calls it saw. */
function ctxWith(answer) {
  const seen = [];
  const session = {
    invalidate() {},
    async syscall(call) {
      seen.push(call);
      return answer(call);
    },
  };
  const request = { id: "77", deadline: new Date(Date.now() + 30_000), signal: new AbortController().signal };
  const ctx = new Ctx(session, new Caller(undefined, "retail", () => undefined), request, { id: "retail", version: "1.0.0" }, quiet);
  return { ctx, seen };
}

test("a refresh sends its table and partitions and reads what it did", async () => {
  const { ctx, seen } = ctxWith((call) => ({
    lakeRefreshed: { rows: "12", asOf: call.lakeRefresh.table === "stock" ? "0" : "1767225600000", stalePartitions: call.lakeRefresh.filter ? "3" : "0" },
  }));
  assert.deepEqual(await ctx.lake().refresh(stock), { rows: 12, stalePartitions: 0 });
  const out = await ctx.lake().refresh(stockByMonth, { partition: stockByMonth.f.month.eq("2026-01-01") });
  assert.equal(out.asOf.toISOString(), "2026-01-01T00:00:00.000Z");
  assert.equal(out.stalePartitions, 3);
  // a filter that names every partition is still a refresh of partitions
  await ctx.lake().refresh(stock, { partition: Filter.all() });
  const calls = seen.map((c) => c.lakeRefresh);
  assert.deepEqual(calls[0], { requestId: "77", table: "stock" });
  assert.deepEqual(calls[1].filter, { condition: { field: "month", op: "EQ", values: [{ stringValue: "2026-01-01" }] } });
  assert.deepEqual(calls[2].filter, {});
});

test("a refresh's refusals carry their status", async () => {
  for (const [answer, status] of [
    ["lake_model_rows_limit: the model computes more than 1000000 rows", 422],
    ["lake_model_parse: f.on_hand holds a text that is not a decimal", 422],
    ["lake_model_changed: the model of stock changed", 409],
    ["lake_partitions_limit: too many partitions in one write: 1200, at most 1000: write fewer day at a time", 422],
    ["capability_missing: the model reads product", 403],
    ["unavailable: the analytic copy is loading", 503],
  ]) {
    const { ctx } = ctxWith(() => {
      throw new SyscallError(answer);
    });
    await assert.rejects(ctx.lake().refresh(stock), { status, code: answer.split(":")[0] });
  }
  // another plugin's table is not refreshed
  const theirs = lakeTable({ plugin: "erp", name: "deliveries", columns: { day: t.date() } });
  const { ctx } = ctxWith(() => ({}));
  await assert.rejects(ctx.lake().refresh(/** @type {any} */ (theirs)), (e) => e.code === "internal");
});

// --- histories ------------------------------------------------------------------------------------------------

const ticketStatus = t.enum("TicketStatus", ["open", "closed"]);

const ticket = entity({
  softDelete: true,
  columns: {
    title: t.text(),
    status: t.enum(ticketStatus).history(),
    assignee: t.ref(platform.user).optional().history(),
    body: t.text(),
  },
  history: { layer: audit, read: [analyst], systemRead: true, export: true },
});

/** Every column kept, but the list, the encrypted one and the masked one. */
const contract = entity({
  columns: {
    title: t.text(),
    amount: t.decimal().optional(),
    tags: t.array(t.text()),
    secret: t.text().optional().encrypted(),
    notes: t.text().optional(),
  },
  access: (a) => {
    a.read(agent);
    a.mask("notes", [agent]);
  },
  history: { layer: audit, all: true },
});

const h = historyOf(ticket);

/** Open tickets at the end of each day. */
const openByDay = lakeTable({
  layer: mart,
  name: "open_by_day",
  columns: { day: t.date(), assignee: t.ref(platform.user).optional(), open: t.bigint() },
  partition: ["day"],
  model: (m) =>
    m
      .fromHistory(ticket)
      .eachDay()
      .where(ticket.f.status.ne("closed"))
      .groupBy(h.f.day, ticket.f.assignee)
      .columns({ day: h.f.day, assignee: ticket.f.assignee, open: count() }),
});

/** Another plugin's entity whose history this plugin reads. */
const theirTicket = entity({ plugin: "support", name: "case", columns: { state: t.text() } });

const helpdesk = plugin({
  id: "helpdesk",
  version: "1.0.0",
  roles: { analyst, agent },
  layers: { audit, mart },
  entities: { ticket, contract },
  lakeTables: { openByDay },
  historyDependencies: [theirTicket],
});

test("an entity declares its history and its kept columns", () => {
  const d = helpdesk.describe();
  const [t1, t2] = d.entities;
  assert.deepEqual(t1.history, { layer: "audit", read: ["analyst"], system_read: true, export: true });
  assert.deepEqual(t1.fields.filter((f) => f.history).map((f) => f.name), ["status", "assignee"]);
  // `all`: a flag on every column it covers, none on the list, the encrypted and the masked one
  assert.deepEqual(t2.history, { layer: "audit", all: true });
  assert.deepEqual(t2.fields.filter((f) => f.history).map((f) => f.name), ["title", "amount"]);
  // the message of the row carries no history flag
  const msg = d.messages.find((m) => m.name === "Ticket");
  assert.ok(msg.fields.every((f) => !("history" in f)));
  // the model over the history per day
  assert.deepEqual(d.lake_models[0].sources, [{ entity: "ticket", alias: "ticket", history: true, each_day: true }]);
  assert.deepEqual(d.lake_models[0].group_by, [
    { source: "ticket", field: "day" },
    { source: "ticket", field: "assignee" },
  ]);
  // another plugin's history read: named in its dependency's lake
  assert.deepEqual(d.dependencies, [{ plugin: "support", calls: [], lake: ["case"] }]);
  // unmasked keeps the masked column too
  const all = entity({ columns: { notes: t.text().optional() }, access: (a) => a.mask("notes", [agent]), history: { layer: audit, all: true, unmasked: true } });
  const e = plugin({ id: "xx", version: "1.0.0", roles: { agent }, layers: { audit }, entities: { all } }).describe().entities[0];
  assert.deepEqual(e.fields.filter((f) => f.history).map((f) => f.name), ["notes"]);
  assert.deepEqual(e.history, { layer: "audit", all: true, unmasked: true });
});

test("mistakes of a history are reported", () => {
  const codes = (p) => p.check().map((x) => x.code);
  // its layer is not declared
  assert.ok(codes(plugin({ id: "helpdesk", version: "1.0.0", roles: { analyst }, entities: { ticket } })).includes("E_HISTORY_LAYER"));
  // a kept column on an entity that keeps none
  const plain = entity({ columns: { name: t.text().history() } });
  assert.ok(codes(plugin({ id: "helpdesk", version: "1.0.0", entities: { plain } })).includes("E_HISTORY_LAYER"));
  // a role the plugin does not declare
  assert.ok(codes(plugin({ id: "helpdesk", version: "1.0.0", layers: { audit }, entities: { ticket } })).includes("E_HISTORY_ROLE"));
  // not another plugin's entity
  assert.ok(codes(plugin({ id: "helpdesk", version: "1.0.0", historyDependencies: [/** @type {any} */ (ticket)] })).includes("E_LAKE_DEFINE"));
});

const ROW = {
  id: "0190f3c2-0000-7000-8000-000000000101",
  status: "open",
  validFrom: "2026-11-01T10:00:00Z",
  validTo: null,
  change: "insert",
  changedBy: "0190f3c2-0000-7000-8000-0000000000a1",
  changedByKind: "person",
  changedByPlugin: "helpdesk",
};

test("a history read names the history and answers its rows", async () => {
  helpdesk.describe();
  const { ctx, seen } = ctxWith(() => ({ dbRows: { rows: [Buffer.from(JSON.stringify(ROW))], nextPageToken: "" } }));
  const rows = await ctx.lake().history(ticket).where(ticket.f.id.eq(ROW.id)).orderBy(h.f.validFrom.asc()).all();
  assert.equal(rows.length, 1);
  const r = rows[0];
  assert.deepEqual([r.change, r.changedByKind, r.changedByPlugin, r.changedBy], ["insert", "person", "helpdesk", ROW.changedBy]);
  assert.equal(r.validFrom.toISOString(), "2026-11-01T10:00:00.000Z");
  assert.equal(r.validTo, undefined);
  assert.equal(r.day, undefined);
  assert.equal(r.status, "open");
  // a kept column the row has no value of is absent
  assert.equal(r.assignee, undefined);
  assert.equal("title" in r, false);
  const q = seen[0].query;
  assert.equal(q.engine, "LAKE");
  assert.deepEqual(q.query.sources[0], { entity: "ticket", alias: "ticket", history: true });
  assert.deepEqual(q.query.order, [{ term: { column: { source: "ticket", field: "valid_from" } }, desc: false }]);
  // per day, counted by day
  const perDay = ctxWith(() => ({ dbRows: { rows: [Buffer.from('["2026-11-02","3"]')], nextPageToken: "" } }));
  const days = await perDay.ctx.lake().history(ticket).eachDay().groupBy(h.f.day).select({ day: h.f.day, n: count() }).all();
  assert.deepEqual(days, [{ day: "2026-11-02", n: 3 }]);
  const tree = perDay.seen[0].query.query;
  assert.deepEqual(tree.sources[0], { entity: "ticket", alias: "ticket", history: true, eachDay: true });
  assert.deepEqual(tree.groupBy, [{ source: "ticket", field: "day" }]);
  // another plugin's history, by its qualified name
  const theirs = ctxWith(() => ({ dbRows: { rows: [], nextPageToken: "" } }));
  await theirs.ctx.lake().history(theirTicket).all();
  assert.deepEqual(theirs.seen[0].query.query.sources[0], { entity: "support.case", alias: "support_case", history: true });
  // a history is read in the lake only, and with the query's reads only
  assert.throws(() => ctx.query(/** @type {any} */ (h)), (e) => e.code === "internal");
  await assert.rejects(async () => ctx.lake().history(ticket).get("x"), (e) => e.code === "internal");
});

test("a row the platform sends without its history columns is refused", async () => {
  const { ctx } = ctxWith(() => ({ dbRows: { rows: [Buffer.from('{"id":"x"}')], nextPageToken: "" } }));
  await assert.rejects(ctx.lake().history(ticket).all(), (e) => e.code === "internal" && /history row/.test(String(e.cause?.message)));
});

test("a parsed text in a lake query: its wire name, its value's type", async () => {
  retail.describe();
  const { ctx, seen } = ctxWith(() => ({ dbRows: { rows: [Buffer.from('["12.50"]')], nextPageToken: "" } }));
  const rows = await ctx
    .lake()
    .query(stockFile)
    .where(stockFile.f.onHand.parse("decimal").gt(Decimal.from("10")))
    .select({ onHand: stockFile.f.onHand.parse("decimal") })
    .all();
  assert.ok(rows[0].onHand instanceof Decimal);
  assert.equal(rows[0].onHand.toString(), "12.50");
  const q = seen[0].query.query;
  assert.deepEqual(q.select, [{ column: { source: "stock_file", field: "on_hand", parse: "PARSE_DECIMAL" } }]);
  assert.deepEqual(q.filter.compare.left, { column: { source: "stock_file", field: "on_hand", parse: "PARSE_DECIMAL" } });
  assert.deepEqual(q.filter.compare.values, [{ stringValue: "10" }]);
  // a parse of anything but a text, or to an unknown type
  assert.throws(() => stock.f.sku.parse(/** @type {any} */ ("money")), /not decimal/);
});

test("a parse, a history root and a refresh survive the session's encoding", async () => {
  retail.describe();
  helpdesk.describe();
  const { celldDefinition } = await import("../dist/serve.js");
  const session = celldDefinition().service.Session;
  const encoded = (call) => session.requestDeserialize(session.requestSerialize({ syscall: { id: "1", ...call } })).syscall;
  const { ctx, seen } = ctxWith((call) => (call.query ? { dbRows: { rows: [], nextPageToken: "" } } : { lakeRefreshed: { rows: "1", asOf: "0" } }));
  await ctx.lake().query(stockFile).where(stockFile.f.onHand.parse("decimal").gt(Decimal.from("10"))).select({ n: stockFile.f.onHand.parse("int64") }).all();
  await ctx.lake().history(ticket).eachDay().all();
  await ctx.lake().refresh(stock, { partition: Filter.all() });
  await ctx.lake().refresh(stock, { partition: stock.f.day.eq("2026-01-01") });
  await ctx.lake().refresh(stock);
  const q = encoded(seen[0]).query.query;
  assert.equal(q.filter.compare.left.column.parse, "PARSE_DECIMAL");
  assert.equal(q.select[0].column.parse, "PARSE_INT64");
  const h = encoded(seen[1]).query.query.sources[0];
  assert.deepEqual([h.entity, h.history, h.eachDay], ["ticket", true, true]);
  // a partition filter that names every partition stays set (empty); none is the whole table
  const [all, day, whole] = seen.slice(2).map((c) => encoded(c).lakeRefresh);
  assert.ok(all.filter !== null && all.filter !== undefined, JSON.stringify(all));
  assert.equal(day.filter.condition.field, "day");
  assert.ok(whole.filter === null || whole.filter === undefined, JSON.stringify(whole));
  assert.equal(whole.table, "stock");
});

test("a history row's author is an id of the directory, whatever its form", async () => {
  helpdesk.describe();
  const row = { ...ROW, changedBy: "u-42", changedByKind: "service" };
  const { ctx, seen } = ctxWith(() => ({ dbRows: { rows: [Buffer.from(JSON.stringify(row))], nextPageToken: "" } }));
  const [r] = await ctx.lake().history(ticket).where(h.f.changedBy.eq("u-42")).all();
  assert.equal(r.changedBy, "u-42");
  assert.deepEqual(seen[0].query.query.filter.compare.values, [{ stringValue: "u-42" }]);
});

test("read roles are listed once each, where first named, a history's and a lake table's alike", () => {
  const t1 = entity({ columns: { status: t.text().history() }, history: { layer: audit, read: [analyst, agent, analyst] } });
  const t2 = lakeTable({ layer: mart, name: "t2", columns: { day: t.date() }, read: [agent, analyst, agent] });
  const d = plugin({ id: "roles", version: "1.0.0", roles: { analyst, agent }, layers: { audit, mart }, entities: { t1 }, lakeTables: { t2 } }).describe();
  assert.deepEqual(d.entities[0].history.read, ["analyst", "agent"]);
  assert.deepEqual(d.lake_tables[0].read, ["agent", "analyst"]);
});
