// A plugin whose one endpoint runs a repository operation by name and answers what it got (JSON, bigints and
// Decimals as strings): the shapes of search, aggregates, include, bulk writes and row locks on the session.
import { endpoint, entity, plugin, t } from "../../dist/index.js";
import * as platform from "../../dist/platform.js";

export const item = entity({
  key: "string",
  columns: {
    name: t.text().searchable("english"),
    sku: t.text().unique(),
    qty: t.int(),
    price: t.decimal(),
    weight: t.double(),
    ownerId: t.ref(platform.user),
    parentId: t.ref(() => item, { nav: "parent", inverse: "children" }).optional(),
  },
});

const ops = {
  search: (ctx) => ctx.db(item).find(item.f.name.search('"red apple" -green').and(item.f.sku.contains("50%_"))).all(),
  countBy: (ctx) => ctx.db(item).aggregate({ where: item.f.qty.gt(0), groupBy: "ownerId", count: true }),
  countAll: (ctx) => ctx.db(item).aggregate({ count: true, withDeleted: true }),
  sumQty: (ctx) => ctx.db(item).aggregate({ groupBy: "parentId", sum: "qty" }),
  sumPrice: async (ctx) =>
    (await ctx.db(item).aggregate({ sum: "price" })).map((g) => ({ sum: g.sum.toString(), decimal: g.sum.constructor.name === "Decimal" })),
  sumWeight: (ctx) => ctx.db(item).aggregate({ sum: "weight" }),
  include: (ctx) => ctx.db(item).find().include("parent", "children", "parentId").all(),
  getInclude: (ctx) => ctx.db(item).get("A-1", { include: ["children"] }),
  updateWhere: (ctx) => ctx.db(item).updateWhere(item.f.qty.eq(0), { qty: 5, parentId: null }),
  upsert: (ctx) => ctx.db(item).upsert("sku", { id: "A-2", name: "pear", sku: "P", qty: 1, price: "1.5", weight: 2, ownerId: OWNER }),
  lockGet: (ctx) => ctx.transaction(async (tx) => tx.db(item).get("A-1", { lock: "update", wait: "nowait" })),
  lockFind: (ctx) => ctx.transaction(async (tx) => tx.db(item).find(item.f.qty.gt(0)).forShare().skipLocked().limit(3).page()),
  lockOutside: (ctx) => ctx.db(item).get("A-1", { lock: "update" }),
  lockOrphanWait: (ctx) => ctx.transaction(async (tx) => tx.db(item).find().nowait().all()),
};

export const OWNER = "0190f3c2-7a1b-7c3d-8e4f-0123456789ab";

const replacer = (_k, v) => (typeof v === "bigint" ? `${v}n` : v);

export const run = endpoint({
  input: t.message("RunRequest", { op: t.text() }),
  output: t.message("RunReply", { out: t.text() }),
  async handler(ctx, { op }) {
    const value = await ops[op](ctx);
    return { out: JSON.stringify(value ?? null, replacer) };
  },
});

export default plugin({ id: "repo", version: "0.1.0", entities: { item }, endpoints: { run } });
