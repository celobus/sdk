// The query builder: the tree it sends (the conformance query, as the three SDKs build it), how rows come back typed,
// the reads' modes and the refusals, against a fake link.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { startQuery } from "../dist/query.js";
import { Analytics, alias } from "../dist/query.js";
import * as platform from "../dist/platform.js";
import shop, { customer, order, orderLine, revenue } from "./fixtures/query.mjs";

shop.describe();

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

test("the conformance query is the shared tree", () => {
  const q = revenue(startQuery(link().link, orderLine));
  const expected = JSON.parse(readFileSync(new URL("../../conformance/query/revenue.json", import.meta.url), "utf8"));
  assert.deepEqual(canonical(q.tree()), expected);
});

test("the conformance query reads the analytic engine with the same tree", async () => {
  const { link: l, seen } = link(() => ({ dbRows: { rows: [], nextPageToken: "", asOf: "0", generation: "0" } }));
  const q = revenue(new Analytics(l).query(orderLine));
  const expected = JSON.parse(readFileSync(new URL("../../conformance/query/revenue.json", import.meta.url), "utf8"));
  assert.deepEqual(canonical(q.tree()), expected);
  await q.all();
  assert.equal(seen[0].query.engine, "ANALYTIC");
  assert.equal(seen[0].query.mode, "ALL");
  assert.deepEqual(canonical(seen[0].query.query), expected);
});

test("an analytic read names its engine and says when its data is from; the store's reads carry no time", async () => {
  const at = 1_795_000_000_123;
  const { link: l, seen } = link((call) => {
    const analytic = call.query.engine === "ANALYTIC";
    const stamp = { asOf: analytic ? String(at) : "0", generation: analytic ? "4" : "0" };
    if (call.query.mode === "COUNT") return { dbCount: { count: "1", ...stamp } };
    return { dbRows: { rows: [Buffer.from(JSON.stringify(["SO-1"]))], nextPageToken: "", ...stamp } };
  });
  const q = new Analytics(l).query(order).select({ n: order.f.number });
  const got = await q.allAsOf();
  assert.deepEqual(got.rows, [{ n: "SO-1" }]);
  assert.equal(got.asOf.getTime(), at);
  assert.equal(got.generation, 4n);
  assert.deepEqual(await q.all(), [{ n: "SO-1" }]);
  const counted = await q.countAsOf();
  assert.deepEqual([counted.rows, counted.asOf.getTime()], [1, at]);
  assert.equal(await q.count(), 1);
  const page = await q.page();
  assert.equal(page.asOf.getTime(), at);
  assert.equal(page.next, undefined);
  // the store's reads carry no time
  const plain = await startQuery(l, order).select({ n: order.f.number }).allAsOf();
  assert.deepEqual(plain, { rows: [{ n: "SO-1" }] });
  const storePage = await startQuery(l, order).select({ n: order.f.number }).page();
  assert.equal("asOf" in storePage, false);
  assert.deepEqual(seen.map((c) => c.query.engine), ["ANALYTIC", "ANALYTIC", "ANALYTIC", "ANALYTIC", "ANALYTIC", "STORE", "STORE"]);
});

test("an analytic query of the root alone is still a query, and never the repository's read", async () => {
  const row = { id: "0190f3c2-0000-7000-8000-000000000001", number: "SO-1", status: "paid", total: "1.00", createdBy: "0190f3c2-0000-7000-8000-000000000011",
    createdAt: "2026-11-12T10:00:00Z", updatedAt: "2026-11-12T10:00:00Z" };
  const { link: l, seen } = link((call) => {
    if (call.query?.mode === "COUNT") return { dbCount: { count: "1", asOf: "5", generation: "0" } };
    if (call.query) return { dbRows: { rows: [Buffer.from(JSON.stringify(row))], nextPageToken: "", asOf: "5", generation: "0" } };
    return { dbRows: { rows: [], nextPageToken: "" }, dbCount: { count: "0" } };
  });
  const q = new Analytics(l).query(order);
  const all = await q.allAsOf();
  assert.equal(all.rows[0].number, "SO-1");
  assert.equal(all.asOf.getTime(), 5);
  assert.equal(await q.count(), 1);
  assert.equal(await q.count(order.f.status.eq("paid")), 1);
  assert.deepEqual(seen.map((c) => Object.keys(c)[0]), ["query", "query", "query"]);
  // the repository's reads are of the workspace's data: refused on the analytic copy, before anything is sent
  const store = (e) => /read the workspace's data; use ctx\.query\(\)/.test(e.cause.message);
  assert.throws(() => q.get("1"), store);
  assert.throws(() => q.find(), store);
  await assert.rejects(q.exists(order.f.status.eq("paid")), store);
  assert.throws(() => q.aggregate, store);
  assert.equal(seen.length, 3);
});

test("the engine and the answer's time survive the session's encoding", async () => {
  const { celldDefinition } = await import("../dist/serve.js");
  const session = celldDefinition().service.Session;
  const { link: l, seen } = link(() => ({ dbRows: { rows: [], nextPageToken: "" } }));
  await new Analytics(l).query(order).select({ n: order.f.number }).all();
  const sent = session.requestDeserialize(session.requestSerialize({ syscall: { id: "1", ...seen[0] } }));
  assert.equal(sent.syscall.query.engine, "ANALYTIC");
  const answer = session.responseDeserialize(
    session.responseSerialize({ syscallResult: { id: "1", error: "", dbRows: { rows: [], asOf: "1795000000123", generation: "4" } } }),
  );
  assert.deepEqual([answer.syscallResult.dbRows.asOf, answer.syscallResult.dbRows.generation], ["1795000000123", "4"]);
  const store = session.responseDeserialize(session.responseSerialize({ syscallResult: { id: "1", error: "", dbCount: { count: "2" } } }));
  assert.equal(store.syscallResult.dbCount.asOf, "0");
});

test("the analytic engine's refusals come back typed", async () => {
  const { SyscallError, syscallFailed } = await import("../dist/ctx.js");
  const cases = [
    ["analytic_time_limit: the query ran past 10 s", 422],
    ["analytic_memory_limit: the query needs more memory than the copy has", 422],
    ["analytic_busy: four queries of the workspace are running", 503],
    ["unavailable: the analytic copy is being made", 503],
    ["lake_rows_limit: a write holds at most 1000000 rows", 422],
    ["lake_time_limit: the query took longer than 10 s", 422],
    ["lake_memory_limit: the query needs more memory than the lake's engine has", 422],
    ["lake_busy: other writes of this table ran for more than 60 s", 503],
  ];
  for (const [error, status] of cases) {
    const e = syscallFailed(new SyscallError(error));
    assert.equal(e.status, status, error);
    assert.equal(e.code, error.slice(0, error.indexOf(":")));
  }
});

test("rows come back typed, and every read says its mode", async () => {
  const clerk = "0190f3c2-0000-7000-8000-000000000011";
  const { link: l, seen } = link((call) => {
    if (call.query.mode === "COUNT") return { dbCount: { count: "2" } };
    return {
      dbRows: {
        rows: [
          Buffer.from(JSON.stringify([{ id: clerk, text: "Sarah", avatarUrl: "https://a" }, "Ada", "30.00", "3"])),
          Buffer.from(JSON.stringify([null, null, "0", "2"])),
        ],
        nextPageToken: "next",
      },
    };
  });
  const q = revenue(startQuery(l, orderLine));
  const rows = await q.all();
  assert.deepEqual(rows[0].clerk, { id: clerk, text: "Sarah", avatarUrl: "https://a" });
  assert.equal(rows[0].customer, "Ada");
  assert.equal(rows[0].revenue.toString(), "30.00");
  assert.equal(rows[0].revenue.constructor.name, "Decimal");
  assert.equal(rows[0].lines, 3);
  assert.deepEqual([rows[1].clerk, rows[1].customer], [undefined, undefined]);
  const page = await q.page("t1");
  assert.equal(page.next, "next");
  assert.ok(await q.first());
  assert.equal(await q.count(), 2);
  assert.deepEqual(
    seen.map((c) => [c.query.mode, c.query.pageToken, c.query.requestId]),
    [["ALL", "", "7"], ["PAGE", "t1", "7"], ["FIRST", "", "7"], ["COUNT", "", "7"]],
  );
  // where the query was composed: this test file
  assert.match(seen[0].query.source, /query\.test\.mjs:\d+$/);
});

test("joins go either way, and aliases name an entity twice", () => {
  const q = startQuery(link().link, order).join(orderLine, orderLine.f.orderId).select({ n: order.f.number, sku: orderLine.f.sku });
  assert.deepEqual(q.tree().sources[1], {
    entity: "order_line", alias: "order_line", join: "INNER",
    left: { source: "order", field: "id" }, right: { source: "order_line", field: "order_id" },
  });
  const owner = alias(platform.user, "owner").left();
  const two = startQuery(link().link, order)
    .join(platform.user, order.f.createdBy)
    .leftJoin(customer, order.f.customerId)
    .joinAs(owner, customer.f.ownerId)
    .select({ by: platform.user.f.displayName, owner: owner.f.displayName });
  const t = two.tree();
  assert.equal(t.sources[1].alias, "platform_user");
  assert.deepEqual(t.sources[3], {
    entity: "platform.user", alias: "owner", join: "LEFT",
    left: { source: "crm_customer", field: "owner_id" }, right: { source: "owner", field: "id" },
  });
  assert.deepEqual(t.select[1], { column: { source: "owner", field: "display_name" } });
  const on = startQuery(link().link, order).joinOn(customer, customer.f.ownerId.eqCol(order.f.createdBy)).select({ n: order.f.number });
  assert.deepEqual(on.tree().sources[1], {
    entity: "crm.customer", alias: "crm_customer", join: "INNER",
    left: { source: "order", field: "created_by" }, right: { source: "crm_customer", field: "owner_id" },
  });
});

test("a join the query cannot place is refused before anything is sent", async () => {
  const { link: l, seen } = link();
  // the query has no order to join a customer to
  await assert.rejects(startQuery(l, orderLine).join(customer, order.f.customerId).count(), (e) => e.status === 500 && e.code === "query_invalid");
  assert.deepEqual(seen, []);
});

test("a dependency's entity keeps the repository's reads when the query is its root alone", async () => {
  const { link: l, seen } = link((call) => (call.dbCount ? { dbCount: { count: "4" } } : { dbRows: { rows: [] } }));
  const q = startQuery(l, customer);
  assert.equal(await q.count(), 4);
  await q.all();
  await q.where(customer.f.name.eq("Ada")).all();
  assert.deepEqual(seen.map((c) => Object.keys(c)[0]), ["dbCount", "dbFind", "query"]);
  assert.deepEqual([seen[0].dbCount.dependency, seen[0].dbCount.entity], ["crm", "customer"]);
  assert.deepEqual(seen[2].query.query.filter, {
    compare: { left: { column: { source: "crm_customer", field: "name" } }, op: "EQ", values: [{ stringValue: "Ada" }] },
  });
});
