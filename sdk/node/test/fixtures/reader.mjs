// A plugin that reads a dependency's entity (`ctx.query`) and sends SQL (`ctx.sql`, `ctx.unsafeSql`), with the
// dependency's client as `celobus plugin add` generates it.
import { endpoint, entity, plugin, t } from "../../dist/index.js";

// --- what `celobus plugin add customers` generates -------------------------------------------------
export const PLUGIN = "customers";
export const customer = entity({
  plugin: PLUGIN,
  name: "customer",
  key: "uuid",
  softDelete: true,
  version: true,
  columns: {
    name: t.text(),
    email: t.text().optional(),
  },
});
export const dependency = {
  plugin: PLUGIN,
  calls: ["GetCustomer"],
  reads: [{ entity: "customer", fields: [{ name: "id", type: "ref:customers.customer" }, { name: "name", type: "string" }, { name: "email", type: "optional string" }] }],
};

// --- the plugin -------------------------------------------------------------------------------------
export const visit = entity({ columns: { customerId: t.ref(customer), at: t.timestamp() } });

export const findRequest = t.message("FindRequest", { name: t.text(), pageToken: t.text() });
export const customerList = t.message("CustomerList", {
  names: t.array(t.text()),
  next: t.text(),
  count: t.int(),
  exists: t.bool(),
  deleted: t.text(),
  version: t.bigint(),
});
export const statsRow = t.message("StatsRow", { day: t.date(), visits: t.bigint() });
export const statsReply = t.message("StatsReply", { rows: t.array(statsRow) });
export const sinceRequest = t.message("SinceRequest", { from: t.date(), min: t.bigint() });

export const VISIT_STATS = "select at::date as day, count(*) as visits from reader_visit group by 1";
export const VISITS_SINCE =
  "select at::date as day, count(*) as visits from reader_visit where at >= $1 group by 1 having count(*) >= $2";
const statsRows = (rows) => rows.map((r) => ({ day: r.day, visits: BigInt(r.visits) }));

export const findCustomers = endpoint({
  input: findRequest,
  output: customerList,
  readOnly: true,
  async handler(ctx, { name, pageToken }) {
    const customers = ctx.query(customer);
    const page = await customers
      .find(customer.f.name.eq(name).or(customer.f.email.isNull()))
      .orderBy(customer.f.name.desc())
      .limit(20)
      .pageToken(pageToken)
      .withDeleted()
      .page();
    const count = await customers.count(customer.f.name.ge("a"));
    const exists = await customers.exists(customer.f.email.eq("a@b.c"));
    const one = await customers.get("0190f3c2-0000-7000-8000-000000000001", { withDeleted: true });
    return {
      names: page.rows.map((c) => c.name),
      next: page.next ?? "",
      count,
      exists,
      deleted: one.deletedAt?.toISOString() ?? "",
      version: one.version ?? 0n,
    };
  },
});

export const allCustomers = endpoint({
  input: {},
  output: { names: t.array(t.text()) },
  readOnly: true,
  async handler(ctx) {
    return { names: (await ctx.query(customer).all()).map((c) => c.name) };
  },
});

export const queryOwn = endpoint({
  input: {},
  output: { n: t.int() },
  readOnly: true,
  async handler(ctx) {
    return { n: await ctx.query(visit).count() };
  },
});

export const dbForeign = endpoint({
  input: {},
  output: { n: t.int() },
  readOnly: true,
  async handler(ctx) {
    return { n: await ctx.db(customer).count() };
  },
});

export const stats = endpoint({
  input: {},
  output: statsReply,
  readOnly: true,
  async handler(ctx) {
    return { rows: statsRows(await ctx.sql(VISIT_STATS)) };
  },
});

export const statsSince = endpoint({
  input: sinceRequest,
  output: statsReply,
  readOnly: true,
  async handler(ctx, req) {
    return ctx.transaction(async (tx) => ({ rows: statsRows(await tx.sql(VISITS_SINCE, req.from, req.min)) }), { readOnly: true });
  },
});

export const purge = endpoint({
  input: sinceRequest,
  output: statsReply,
  async handler(ctx, req) {
    const gone = await ctx.unsafeSql(
      "delete from reader_visit where at < $1 returning at::date as day, 1 as visits",
      req.from,
    );
    return { rows: statsRows(gone) };
  },
});

export default plugin({
  id: "reader",
  version: "0.1.0",
  capabilities: { "sql:unsafe": "Purges old visits.", "sql:write": "Purges old visits." },
  dependencies: [dependency],
  entities: { visit },
  endpoints: { findCustomers, allCustomers, queryOwn, dbForeign, stats, statsSince, purge },
});
