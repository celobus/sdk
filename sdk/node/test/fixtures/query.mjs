// The conformance query (sdk/conformance/query/revenue.json) as a TypeScript plugin builds it: the paid lines of
// orders over 10.50 placed by a clerk, per clerk and customer, with the clerk's label, the customer's name, the
// revenue and the lines.
import { Decimal, entity, plugin, t } from "../../dist/index.js";
import * as platform from "../../dist/platform.js";
import { count, exists, label, sum } from "../../dist/query.js";

export const customer = entity({
  plugin: "crm",
  name: "customer",
  key: "uuid",
  columns: { name: t.text(), ownerId: t.ref(platform.user) },
});

export const orderStatus = t.enum("OrderStatus", ["placed", "paid"]);

export const order = entity({
  columns: {
    number: t.text(),
    status: t.enum(orderStatus),
    total: t.decimal(),
    createdBy: t.ref(platform.user),
    customerId: t.ref(customer).optional(),
  },
});

export const orderLine = entity({
  columns: { orderId: t.ref(order), sku: t.text(), amount: t.decimal() },
});

export default plugin({
  id: "shop",
  version: "1.0.0",
  dependencies: [
    {
      plugin: "crm",
      calls: [],
      reads: [
        {
          entity: "customer",
          fields: [
            { name: "id", type: "ref:crm.customer" },
            { name: "name", type: "string" },
            { name: "owner_id", type: "ref:platform.user" },
          ],
        },
      ],
    },
  ],
  entities: { order, orderLine },
});

const ra = platform.roleAssignment.f;

/** The conformance query from `ctx.query(orderLine)`. */
export function revenue(q) {
  return q
    .join(order, orderLine.f.orderId)
    .leftJoin(customer, order.f.customerId)
    .where(order.f.status.eq("paid").and(order.f.total.ge(Decimal.parse("10.50"))))
    .where(exists(platform.roleAssignment, ra.userId.eqCol(order.f.createdBy), ra.role.eq("shop.clerk")))
    .groupBy(order.f.createdBy, customer.f.id, customer.f.name)
    .having(count().gt(1))
    .select({ clerk: label(order.f.createdBy), customer: customer.f.name, revenue: sum(orderLine.f.amount), lines: count() })
    .orderBy({ revenue: "desc" })
    .limit(20);
}
