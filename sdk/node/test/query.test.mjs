// The query builder: the tree it sends (the conformance query, as the three SDKs build it), how rows come back typed,
// the reads' modes and the refusals, against a fake link.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { startQuery } from "../dist/query.js";
import { alias } from "../dist/query.js";
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
