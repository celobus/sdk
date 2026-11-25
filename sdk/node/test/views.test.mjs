// Analytic views: their descriptor, the conformance query over one (with a bucket and a relative day), how a view's
// rows come back, what a view refuses, and how buckets and relative days survive the session's encoding.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { daysAgo, entity, plugin, t, view } from "../dist/index.js";
import { Analytics, count, startQuery, sum } from "../dist/query.js";
import views, { estimates, longEstimate, recentTask, stage, task, tasksByWeek, weekly } from "./fixtures/views.mjs";

const described = views.describe();
const e = longEstimate.f;

/** A link whose syscalls `answer` answers; the calls it saw. */
function link(answer = () => ({})) {
  const seen = [];
  return {
    seen,
    link: { requestId: "7", tx: "0", asSystem: "", syscall: async (call) => (seen.push(call), answer(call)) },
  };
}

/** The tree as protobuf JSON writes it: default values (a COUNT, the root's join, false, 0, empty lists) left out. */
function canonical(v, key = "") {
  if (Array.isArray(v)) return v.length ? v.map((x) => canonical(x)) : undefined;
  if (v && typeof v === "object") {
    const out = {};
    for (const [k, x] of Object.entries(v)) {
      const c = canonical(x, k);
      if (c !== undefined) out[k] = c;
    }
    return out;
  }
  if (v === false || v === 0 || v === "" || (key === "fn" && v === "COUNT") || (key === "join" && v === "ROOT")) return undefined;
  return v;
}

const sorted = (v) => JSON.parse(JSON.stringify(v, (_, x) => (x && typeof x === "object" && !Array.isArray(x) ? Object.fromEntries(Object.entries(x).sort()) : x)));

test("the jobsdemo views are the conformance descriptor's", () => {
  const expected = JSON.parse(readFileSync(new URL("../../conformance/jobs/descriptor.json", import.meta.url), "utf8"));
  assert.deepEqual(sorted(described.views), expected.views);
  const rows = described.messages.slice(-3);
  const names = rows.map((m) => m.name);
  assert.deepEqual(sorted(rows), expected.messages.filter((m) => names.includes(m.name)));
});

test("the conformance view query is the shared tree", async () => {
  const { link: l, seen } = link(() => ({ dbRows: { rows: [], nextPageToken: "", asOf: "0", generation: "0" } }));
  const q = weekly(new Analytics(l).query(recentTask));
  const expected = JSON.parse(readFileSync(new URL("../../conformance/query/weekly.json", import.meta.url), "utf8"));
  assert.deepEqual(canonical(q.tree()), expected);
  await q.all();
  assert.equal(seen[0].query.engine, "ANALYTIC");
  assert.deepEqual(canonical(seen[0].query.query), expected);
});

test("the conformance decimal query is the shared tree", async () => {
  const { link: l, seen } = link(() => ({ dbRows: { rows: [], nextPageToken: "", asOf: "0", generation: "0" } }));
  const q = estimates(new Analytics(l).query(longEstimate));
  const expected = JSON.parse(readFileSync(new URL("../../conformance/query/estimates.json", import.meta.url), "utf8"));
  assert.deepEqual(canonical(q.tree()), expected);
  await q.all();
  assert.deepEqual(canonical(seen[0].query.query), expected);
  // a decimal is written as its text, also from a plain number, in a query and in a view's declaration
  const loose = new Analytics(l).query(longEstimate).where(e.hours.lt(1.5)).select({ title: e.title }).tree();
  assert.deepEqual(loose.filter.compare.values, [{ stringValue: "1.5" }]);
  const declared = described.views.find((v) => v.name === "long_estimate");
  assert.deepEqual(declared.filter.compare.values, ["12.50"]);
});

test("a view's rows come back as its own type, on either engine", async () => {
  const { link: l, seen } = link((call) => ({
    dbRows: {
      rows: [Buffer.from(JSON.stringify({ day: "2026-11-02", title: "Write", stage: null }))],
      nextPageToken: "",
      asOf: call.query.engine === "ANALYTIC" ? "9" : "0",
      generation: "1",
    },
  }));
  const rows = await startQuery(l, recentTask).all();
  assert.deepEqual(rows, [{ day: "2026-11-02", title: "Write" }]);
  const a = await new Analytics(l).query(recentTask).where(recentTask.f.title.eq("Write")).allAsOf();
  assert.deepEqual(a.rows, [{ day: "2026-11-02", title: "Write" }]);
  assert.equal(a.asOf.getTime(), 9);
  assert.deepEqual(seen.map((c) => [c.query.engine, c.query.query.sources]), [
    ["STORE", [{ entity: "recent_task", alias: "recent_task" }]],
    ["ANALYTIC", [{ entity: "recent_task", alias: "recent_task" }]],
  ]);
  // grouped by a bucket, read as a date
  const { link: l2 } = link(() => ({ dbRows: { rows: [Buffer.from(JSON.stringify(["2026-11-01", null, "3"]))], nextPageToken: "" } }));
  const months = await startQuery(l2, recentTask)
    .groupBy(recentTask.f.day.month(), recentTask.f.stage)
    .select({ month: recentTask.f.day.month(), stage: recentTask.f.stage, tasks: count() })
    .all();
  assert.deepEqual(months, [{ month: "2026-11-01", stage: undefined, tasks: 3 }]);
});

test("a view is read, never written, and has no repository reads", async () => {
  const { Ctx } = await import("../dist/ctx.js");
  const { link: l, seen } = link();
  const q = startQuery(l, recentTask);
  assert.throws(() => q.get("1"), (e) => /a view is read with/.test(e.cause.message));
  assert.throws(() => q.find(), (e) => /a view is read with/.test(e.cause.message));
  assert.equal(seen.length, 0);
  assert.equal(typeof Ctx.prototype.db, "function");
  assert.throws(() => Ctx.prototype.db.call({}, recentTask), (e) => /a view is read, never written/.test(e.cause.message));
});

test("entity handles bucket, compare with a relative day, and join on plain columns", () => {
  const q = startQuery(link().link, task)
    .leftJoinOn(stage, stage.f.name.eqCol(task.f.status))
    .where(task.f.createdAt.geCol(daysAgo(30)), task.f.createdAt.between(new Date("2026-01-01T00:00:00Z"), new Date("2026-02-01T00:00:00Z")))
    .groupBy(task.f.createdAt.quarter())
    .select({ quarter: task.f.createdAt.quarter(), n: count() })
    .orderBy({ quarter: "desc" });
  const tree = canonical(q.tree());
  assert.deepEqual(tree.sources[1], {
    entity: "stage", alias: "stage", join: "LEFT",
    left: { source: "task", field: "status" }, right: { source: "stage", field: "name" },
  });
  assert.deepEqual(tree.filter.and.items[0], {
    compare: { left: { column: { source: "task", field: "created_at" } }, op: "GE", right: { daysAgo: 30 } },
  });
  assert.equal(tree.filter.and.items[1].and.items.length, 2);
  assert.deepEqual(tree.groupBy, [{ source: "task", field: "created_at", bucket: "QUARTER" }]);
  assert.deepEqual(tree.order, [{ term: { column: { source: "task", field: "created_at", bucket: "QUARTER" } }, desc: true }]);
  // a bucket of what is not a date or a timestamp is refused where it is made
  assert.throws(() => task.f.title.month(), /not a date or a timestamp/);
  // joinOn takes two columns, never a relative day
  const bad = startQuery(link().link, task).joinOn(stage, task.f.createdAt.eqCol(daysAgo(1)));
  assert.match(bad.error, /two columns are compared with eqCol/);
});

test("a view's mistakes are definition errors", () => {
  const thing = entity({ columns: { n: t.int(), at: t.timestamp() } });
  const bad = view({
    columns: { n: t.int(), total: t.bigint() },
    query: (q) => q.from(thing).columns({ n: thing.f.n, total: sum(thing.f.n), extra: thing.f.at }),
  });
  const unbuilt = view({ columns: { n: t.int() }, query: (q) => q.from(thing).columns({ n: thing.f.n.month() }) });
  const clash = view({ columns: { n: t.int() }, query: (q) => q.from(thing).columns({ n: thing.f.n }) });
  const errors = plugin({ id: "bad", version: "1.0.0", entities: { thing }, views: { bad, unbuilt, thing: clash } }).check();
  assert.deepEqual(errors.map((e) => [e.code, e.path]), [
    ["E_VIEW_CLASH", "views.thing"],
    ["E_VIEW_DEFINE", "views.bad"],
    ["E_VIEW_DEFINE", "views.unbuilt"],
    ["E_MSG_DUP", "views.thing"],
  ]);
  assert.match(errors[1].message, /extra is not a column of the view/);
  assert.match(errors[2].message, /not a date or a timestamp/);
});

test("buckets and relative days survive the session's encoding", async () => {
  const { celldDefinition } = await import("../dist/serve.js");
  const session = celldDefinition().service.Session;
  const { link: l, seen } = link(() => ({ dbRows: { rows: [], nextPageToken: "" } }));
  await weekly(new Analytics(l).query(recentTask)).all();
  const sent = session.requestDeserialize(session.requestSerialize({ syscall: { id: "1", ...seen[0] } })).syscall.query.query;
  assert.deepEqual(sent.filter.and.items[0].compare.right.daysAgo, 7);
  assert.deepEqual(sent.filter.and.items[1].and.items[0].compare.left.column.bucket, "MONTH");
  assert.deepEqual(sent.groupBy.map((c) => c.bucket), ["MONTH", "NONE"]);
  assert.deepEqual(sent.select[0].column.bucket, "MONTH");
});
