// The Data Lake: what a plugin declares of its layers and lake tables, what a write sends (chunks, keys, filters) and
// what it refuses before sending, and what a lake query sends, against a fake platform (the same cases as the Rust
// SDK's).
import assert from "node:assert/strict";
import { test } from "node:test";
import { Caller, Ctx, JobCtx, NO_RUN, SyscallError } from "../dist/ctx.js";
import { Decimal, Filter, lakeTable, layer, plugin, role, t } from "../dist/index.js";
import { CHUNK_BYTES, WRITE_ROWS, chunks } from "../dist/lake.js";
import { sum } from "../dist/query.js";

const analyst = role({ title: "Analyst" });
const raw = layer({ title: "As it arrived" });
const core = layer({ title: "Cleaned" });

const stockFile = lakeTable({
  layer: raw,
  name: "stock_file",
  message: "StockFileRow",
  description: "Stock as the ERP sends it.",
  columns: { loadedOn: t.date(), sku: t.text(), onHand: t.text() },
  partition: ["loadedOn"],
});

const stock = lakeTable({
  layer: core,
  columns: { day: t.date(), sku: t.text(), onHand: t.decimal(), reserved: t.bigint().optional() },
  partition: ["day"],
  key: ["day", "sku"],
  read: [analyst],
  systemRead: true,
  export: true,
});

/** Another plugin's exported table. */
const delivery = lakeTable({ plugin: "erp", name: "deliveries", columns: { day: t.date(), sku: t.text() } });

const retail = plugin({
  id: "retail",
  version: "1.0.0",
  roles: { analyst },
  layers: { raw, core },
  lakeTables: { stockFile, stock },
  lakeDependencies: [delivery],
});

test("a plugin declares its layers and lake tables", () => {
  const d = retail.describe();
  assert.deepEqual(d.layers, [
    { name: "raw", title: "As it arrived" },
    { name: "core", title: "Cleaned" },
  ]);
  assert.deepEqual(d.lake_tables, [
    { name: "stock_file", layer: "raw", message: "StockFileRow", description: "Stock as the ERP sends it.", partition: ["loaded_on"] },
    { name: "stock", layer: "core", message: "Stock", partition: ["day"], key: ["day", "sku"], read: ["analyst"], system_read: true, export: true },
  ]);
  assert.deepEqual(d.dependencies, [{ plugin: "erp", calls: [], lake: ["deliveries"] }]);
  // the rows are messages of the plugin, in table order; the other plugin's is not
  assert.deepEqual(d.messages.map((m) => m.name), ["StockFileRow", "Stock"]);
  assert.deepEqual(d.messages[1].fields.map((f) => [f.name, f.type, f.optional]), [
    ["day", "date", false],
    ["sku", "string", false],
    ["on_hand", "decimal", false],
    ["reserved", "int64", true],
  ]);
  // nothing of the lake for a plugin without it
  const plain = plugin({ id: "plain", version: "1.0.0" }).describe();
  assert.equal("layers" in plain || "lake_tables" in plain, false);
});

test("mistakes of the lake declarations are reported", () => {
  const codes = (p) => p.check().map((e) => e.code);
  assert.ok(codes(plugin({ id: "retail", version: "1.0.0", roles: { analyst }, layers: { raw }, lakeTables: { stock } })).includes("E_LAKE_TABLE_LAYER"));
  assert.ok(codes(plugin({ id: "retail", version: "1.0.0", lakeTables: { delivery } })).includes("E_LAKE_DEFINE"));
  assert.ok(codes(plugin({ id: "retail", version: "1.0.0", lakeDependencies: [stock] })).includes("E_LAKE_DEFINE"));
  assert.ok(codes(plugin({ id: "retail", version: "1.0.0", layers: { core }, lakeTables: { stock } })).includes("E_LAKE_ROLE"));
  const odd = lakeTable({ layer: core, columns: { day: t.date() }, partition: ["nope"], key: ["nope"] });
  const c = codes(plugin({ id: "retail", version: "1.0.0", layers: { core }, lakeTables: { stock_: odd } }));
  assert.ok(c.includes("E_LAKE_TABLE_NAME") && c.includes("E_LAKE_PARTITION") && c.includes("E_LAKE_KEY"), c.join());
});

/** A context of request 77 (or a run's) whose lake writes and queries a fake platform answers; the calls it saw. */
function lakeCtx(run) {
  const seen = [];
  let rows = 0;
  const session = {
    invalidate() {},
    async syscall(call) {
      seen.push(call);
      if (call.lakeWrite) {
        const w = call.lakeWrite;
        if (w.write === "0") rows = 0;
        rows += w.rows.length;
        return { lakeWritten: { write: "41", rows: String(rows), committed: w.last && w.key !== "seen", skipped: w.last && w.key === "seen" } };
      }
      if (call.query) return { dbRows: { rows: [Buffer.from('{"day":"2026-01-01","sku":"a","onHand":"1.5"}')], nextPageToken: "" } };
      throw new SyscallError("unavailable: not here");
    },
  };
  const request = { id: "77", deadline: new Date(Date.now() + 30_000), signal: new AbortController().signal };
  const job = run ? Object.freeze({ ...NO_RUN, job: "nightly", runId: run.runId, attempt: run.attempt, maxAttempts: 3, isRun: true }) : NO_RUN;
  const Kind = run ? JobCtx : Ctx;
  const ctx = new Kind(session, new Caller(undefined, "retail", () => undefined), request, { id: "retail", version: "1.0.0" }, quiet, "0", "", job);
  return { ctx, seen };
}
/** A 500 `internal` whose cause says `pattern` (what the plugin's log shows). */
const internalOf = (pattern) => (e) => e.code === "internal" && pattern.test(String(e.cause?.message));
const quiet = { debug() {}, info() {}, warn() {}, error() {}, with() { return quiet; } };
retail.describe();

test("a write goes in chunks and commits with the last", async () => {
  const { ctx, seen } = lakeCtx();
  const rows = Array.from({ length: 25_001 }, (_, i) => ({ loadedOn: "2026-01-01", sku: `s${i}`, onHand: "1" }));
  const out = await ctx.lake().table(stockFile).replace(stockFile.f.loadedOn.eq("2026-01-01"), rows);
  assert.deepEqual(out, { rows: 25_001, skipped: false });
  const writes = seen.map((c) => c.lakeWrite);
  assert.deepEqual(writes.map((w) => [w.rows.length, w.write, w.last]), [
    [10_000, "0", false],
    [10_000, "41", false],
    [5_001, "41", true],
  ]);
  assert.equal(writes[0].table, "stock_file");
  assert.equal(writes[0].mode, "REPLACE");
  assert.deepEqual(writes[0].filter, { condition: { field: "loaded_on", op: "EQ", values: [{ stringValue: "2026-01-01" }] } });
  assert.equal(writes[1].filter, undefined);
  assert.equal(writes[0].requestId, "77");
  assert.deepEqual(JSON.parse(writes[0].rows[0].toString()), { loadedOn: "2026-01-01", sku: "s0", onHand: "1" });
  // a table replaced whole sends no filter
  await ctx.lake().table(stockFile).replace(Filter.all(), []);
  assert.equal(seen.at(-1).lakeWrite.filter, undefined);
  assert.equal(seen.at(-1).lakeWrite.last, true);
});

test("an append carries its key, and a run's default one", async () => {
  const { ctx, seen } = lakeCtx();
  const row = { day: "2026-01-01", sku: "a", onHand: Decimal.from("1.5") };
  assert.equal((await ctx.lake().table(stock).append([row], { key: "seen" })).skipped, true);
  // outside a run without a key: refused before anything is sent (and a type error: the cast says so)
  const before = seen.length;
  await assert.rejects(ctx.lake().table(stock).append([row], /** @type {any} */ ({})), { status: 400, code: "invalid" });
  await assert.rejects(ctx.lake().table(stock).append([row], { key: "  " }), { code: "invalid" });
  assert.equal(seen.length, before);
  // in a run: its id and the append's number, the same in every attempt; a given key still takes a number, and the
  // system context counts on
  const keys = [];
  for (const attempt of [1, 2]) {
    const run = lakeCtx({ runId: "run-7", attempt });
    await run.ctx.lake().table(stock).append([row]);
    await run.ctx.lake().table(stock).append([row], { key: "mine" });
    await run.ctx.asSystem("the rest").lake().table(stock).append([row]);
    keys.push(...run.seen.map((c) => c.lakeWrite.key));
  }
  assert.deepEqual(keys, ["run-7#1", "mine", "run-7#3", "run-7#1", "mine", "run-7#3"]);
  await ctx.lake().table(stock).merge([row]);
  assert.deepEqual([seen.at(-1).lakeWrite.mode, seen.at(-1).lakeWrite.key], ["MERGE", ""]);
});

test("a lake query reads lake tables by name, on the lake", async () => {
  const { ctx, seen } = lakeCtx();
  const rows = await ctx.lake().query(stock).where(stock.f.sku.eq("a")).all();
  assert.equal(rows[0].onHand.toString(), "1.5");
  await ctx.lake().query(delivery).select({ day: delivery.f.day, n: sum(delivery.f.day) }).all().catch(() => {});
  const reads = seen.map((c) => c.query);
  assert.equal(reads[0].engine, "LAKE");
  assert.deepEqual(reads[0].query.sources[0], { entity: "stock", alias: "stock" });
  assert.deepEqual(reads[1].query.sources[0], { entity: "erp.deliveries", alias: "erp_deliveries" });
  // never read as an entity or from another engine
  assert.throws(() => ctx.query(stock), internalOf(/ctx.lake\(\).query/));
  assert.throws(() => ctx.analytics().query(stock), internalOf(/ctx.lake\(\).query/));
  await assert.rejects(async () => ctx.lake().query(stock).get("x"), internalOf(/all, first, count and page/));
});

test("another plugin's table is not written", async () => {
  const { ctx } = lakeCtx();
  await assert.rejects(ctx.lake().table(delivery).merge([{ day: "2026-01-01", sku: "a" }]), internalOf(/another plugin/));
});

test("rows past a million are refused before anything is sent, and chunks keep under 4 MiB", async () => {
  const many = { *[Symbol.iterator]() { for (let i = 0; i <= WRITE_ROWS; i++) yield { loadedOn: "2026-01-01", sku: "", onHand: "" }; } };
  assert.throws(() => chunks(stockFile, many), { status: 422, code: "lake_rows_limit" });
  const big = "x".repeat(1 << 20);
  const sized = chunks(stockFile, Array.from({ length: 9 }, () => ({ loadedOn: "2026-01-01", sku: big, onHand: "" })));
  assert.deepEqual(sized.map((c) => c.length), [3, 3, 3]);
  assert.throws(() => chunks(stockFile, [{ loadedOn: "2026-01-01", sku: "x".repeat(CHUNK_BYTES), onHand: "" }]), { code: "bulk_limit" });
  const { ctx, seen } = lakeCtx();
  await assert.rejects(ctx.lake().table(stockFile).merge(many), { code: "lake_rows_limit" });
  assert.equal(seen.length, 0);
});

test("a row is its JSON as the other SDKs write it", () => {
  // what decides the chunks: the bytes of each row, with no character escaped that the others write as it is
  const [[row]] = chunks(stockFile, [{ loadedOn: "2026-01-01", sku: "é'<&+\"\\ 😀 \u2028", onHand: "" }]);
  assert.equal(row.toString(), '{"loadedOn":"2026-01-01","sku":"é\'<&+\\"\\\\ 😀 \u2028","onHand":""}');
});

test("the lake's refusals carry their status", async () => {
  for (const [answer, status] of [
    ["lake_busy: other writes of this table ran for more than 60 s; retry", 503],
    ["lake_rows_limit: a write holds at most 1000000 rows", 422],
    ["lake_time_limit: the query took longer than 10 s", 422],
    ["lake_memory_limit: the query needs more memory", 422],
  ]) {
    const session = { invalidate() {}, syscall: async () => { throw new SyscallError(answer); } };
    const request = { id: "1", deadline: new Date(Date.now() + 30_000), signal: new AbortController().signal };
    const ctx = new Ctx(session, new Caller(undefined, "retail", () => undefined), request, { id: "retail", version: "1.0.0" }, quiet);
    await assert.rejects(ctx.lake().table(stock).merge([{ day: "2026-01-01", sku: "a", onHand: Decimal.from("1") }]), {
      status,
      code: answer.split(":")[0],
    });
  }
});
