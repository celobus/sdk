// Typed reads across entities (`ctx.query(entity)`): joins, filters, groups and aggregates over the plugin's own
// entities, its dependencies' exported entities and the workspace directory, typed end to end. The SDK sends a tree of
// entities and columns by their names, operators and values (`celobus.v1.Query`), never SQL; the platform checks it
// against the installed descriptors and reads it as the caller, every entity under its own rules.
//
//   import { label, sum } from '@celobus/sdk/query';
//   const top = await ctx.query(orderLine)
//     .join(order, orderLine.f.orderId)
//     .leftJoin(customers.customer, order.f.customerId)
//     .where(order.f.status.eq('paid'))
//     .groupBy(customers.customer.f.id)
//     .select({ customer: customers.customer.f.name, revenue: sum(orderLine.f.lineTotal) })
//     .orderBy({ revenue: 'desc' })
//     .limit(20)
//     .all();                     // { customer: string | undefined; revenue: Decimal }[]
//
// The same builder reads the analytic copy of the workspace's entities: `ctx.analytics().query(entity)` takes the same
// joins, filters, groups and aggregates under the same access rules and masks, and answers what `ctx.query()` would
// answer for the data the copy holds. The copy follows the data, usually within a second, never with a promise: a
// caller does not see their own write in it at once, so a screen that shows what was just saved reads it with
// `ctx.query()`. Every analytic answer says when its data is from (`asOf`, from `allAsOf()`, `countAsOf()` and on a
// page); it is never waited for and never refused for being old. A query runs at most 10 s (422
// `analytic_time_limit`) within the copy's memory (422 `analytic_memory_limit`); four of the workspace run at once,
// two of one plugin (503 `analytic_busy` after a short wait); 503 `unavailable` while the copy is being made, when its
// data is older than the workspace's limit (15 minutes unless the workspace sets another) or when the workspace has
// none. It is refused inside a transaction (400 `invalid`); `search` is not available there. Text
// compares and orders by its bytes there. Decimals compute there with at most 20 digits before the point and 18 after:
// a query that compares, orders, groups or aggregates a column holding a longer value the caller may read (or binds
// one, or sums past them) is 500 `query_invalid`, naming the column, never rounded. A group by a decimal, and its
// `min` or `max`, answer the shortest text of the number (`1.5` for `1.5` and `1.50`).
//
//   const sales = await ctx.analytics().query(orderLine)
//     .join(order, orderLine.f.orderId)
//     .groupBy(order.f.status)
//     .select({ status: order.f.status, revenue: sum(orderLine.f.lineTotal) })
//     .allAsOf();                 // sales.rows, sales.asOf (a Date)
import { decodeFieldValue, decodeMessage, encodeFieldValue, type FieldErrors } from "./codec.js";
import { Decimal } from "./decimal.js";
import type { DbLink, DbResult, Find, GetOptions } from "./db.js";
import { ReadRepository } from "./db.js";
import { RpcError, internal } from "./errors.js";
import {
  Bucket,
  ColCmp,
  DaysAgo,
  EntityDef,
  FieldRef,
  Filter,
  Order,
  Parsed,
  PlatformEntity,
  QCond,
  QOrder,
  ViewDef,
  canonicalOf as canonicalName,
  entityNames,
  foreignPlugin,
  isHistorySource,
  isLakeSource,
  type AnyField,
  type ColumnLike,
  type Infer,
  type PlatformRow,
  type Row,
  type Shape,
  type Stored,
  type UntypedRef,
  type ValueOf,
  type ViewRow,
  type WColumn,
  type WExpr,
  type WTerm,
  type WireFilter,
} from "./schema.js";
import { snake } from "./names.js";
import { Snapshot } from "./olap.js";
import type { Field } from "./schema.js";
import type { LakeRow, LakeTableDef } from "./lake.js";
import { decodeHistoryRow, historyOf, type HistoryColumns, type HistoryDef, type HistoryRow } from "./history.js";

export { Bucket, DaysAgo, Parsed, QCond, QOrder, daysAgo } from "./schema.js";
import type { SumOf } from "./db.js";

/**
 * What a query names: an entity of the plugin's, a dependency's exported one, the directory's, a view of the plugin; a
 * lake table in a query of the lake (`ctx.lake().query(t)`).
 */
export type Queryable = EntityDef<any> | PlatformEntity<any> | ViewDef<any> | LakeTableDef<any, any> | HistoryDef<any>;

/** The row type of an entity (or a view), as its field handles name it. */
export type RowOf<E> = E extends EntityDef<any>
  ? Row<E>
  : E extends PlatformEntity<infer C>
    ? PlatformRow<C>
    : E extends ViewDef<any>
      ? ViewRow<E>
      : E extends LakeTableDef<any, any>
        ? LakeRow<E>
        : E extends HistoryDef<infer X>
          ? Row<X> | HistoryColumns<X>
          : never;

/** A ref a join follows between `T` and an entity of the query: a field of `T` (`orderLine.f.orderId` joins
 * `orderLine` to a query that has `order`), or a ref to `T` (`orderLine.f.orderId` joins `order`). */
export type JoinRefOf<T> = FieldRef<RowOf<T>, any, any, any> | FieldRef<any, any, T | UntypedRef, any>;

// --- the wire -----------------------------------------------------------------------------------------------------

/** @internal */ export type WAgg = { fn: string; column?: WColumn };
export type { WColumn, WExpr, WTerm };

/** The name of an entity (or a view) in a query: `order`, `crm.customer`, `platform.user`, `sales`. */
export function canonicalOf(e: object): string {
  try {
    return canonicalName(e);
  } catch (err) {
    throw internal(err);
  }
}

/** The alias of an entity (or a view) unless it is given one: its name with `_` for `.` (`crm_customer`). */
export function defaultAlias(e: object): string {
  return canonicalOf(e).replace(".", "_");
}

/** A filter parameter (`DbValue`), as the repository sends it. */
function dbValue(v: unknown): Record<string, unknown> {
  if (typeof v === "boolean") return { boolValue: v };
  if (typeof v === "number") return Number.isInteger(v) ? { intValue: String(v) } : { doubleValue: v };
  if (typeof v === "string") return { stringValue: v };
  return { stringValue: JSON.stringify(v) };
}

function wireColumn(c: FieldRef<any, any, any, any> | ColumnLike): WColumn {
  if (c instanceof FieldRef) {
    if (!c.owner) throw internal(new Error(`query: the column ${c.name} is not an entity's`));
    return { source: defaultAlias(c.owner), field: c.name };
  }
  try {
    return c.queryColumn();
  } catch (e) {
    throw internal(e);
  }
}

/** The right side of a comparison of columns: a column, or a relative day. */
function wireOperand(c: FieldRef<any, any, any, any> | ColumnLike | DaysAgo): WTerm {
  return c instanceof DaysAgo ? { daysAgo: c.days } : { column: wireColumn(c) };
}

function fromFilter(alias: string, f: WireFilter): WExpr {
  if ("and" in f) return { and: { items: f.and.items.map((i) => fromFilter(alias, i)) } };
  if ("or" in f) return { or: { items: f.or.items.map((i) => fromFilter(alias, i)) } };
  if ("not" in f) return { not: fromFilter(alias, f.not) };
  const c = f.condition;
  return { compare: { left: { column: { source: alias, field: c.field } }, op: c.op, values: c.values } };
}

// --- conditions ---------------------------------------------------------------------------------------------------

/** What a query's `where` and `having` take: a field handle's filter (`order.f.status.eq('paid')`) of an entity of the
 * query, a comparison of two columns, a `QCond`. */
export type CondLike = Filter<any> | ColCmp | QCond;

/** @internal */
export function exprOf(c: CondLike): WExpr | undefined {
  if (c instanceof QCond) return c.expr;
  if (c instanceof ColCmp) {
    return { compare: { left: { column: wireColumn(c.left) }, op: c.op, right: wireOperand(c.right) } };
  }
  if (c.node === undefined) return undefined;
  if (!c.source) throw internal(new Error("query: a filter that is not of an entity's field handles"));
  return fromFilter(defaultAlias(c.source), c.node);
}

/** All of the conditions (of any entities). */
export function and(...conds: CondLike[]): QCond {
  const items = conds.map(exprOf).filter((e): e is WExpr => e !== undefined);
  if (items.length === 0) return new QCond(undefined);
  return new QCond(items.length === 1 ? items[0] : { and: { items } });
}

/** Any of the conditions. */
export function or(...conds: CondLike[]): QCond {
  const items = conds.map(exprOf);
  if (items.some((e) => e === undefined)) return new QCond(undefined);
  return new QCond(items.length === 1 ? items[0] : { or: { items: items as WExpr[] } });
}

/** Not the condition. */
export function not(cond: CondLike): QCond {
  const e = exprOf(cond);
  return new QCond(e === undefined ? { or: { items: [] } } : { not: e });
}

/** A row of `entity` (or of an alias) for which the conditions hold exists; they may name the query's entities. */
export function exists(entity: Queryable | Alias<any, boolean>, ...conds: CondLike[]): QCond {
  const [name, alias] =
    entity instanceof Alias ? [canonicalOf(entity.entity), entity.name] : [canonicalOf(entity), defaultAlias(entity)];
  const filter = and(...conds).expr;
  return new QCond({ exists: { source: { entity: name, alias }, ...(filter ? { filter } : {}) } });
}

/** No row of `entity` for which the conditions hold exists. */
export function notExists(entity: Queryable | Alias<any, boolean>, ...conds: CondLike[]): QCond {
  return not(exists(entity, ...conds));
}

// --- aliases ------------------------------------------------------------------------------------------------------

/** A column of an aliased entity (`manager.f.displayName`): `T` its value, `O` whether it may be null. */
export class AliasCol<T, O extends boolean, R = unknown> implements ColumnLike {
  declare readonly __value: T;
  declare readonly __optional: O;
  declare readonly __target: R;

  constructor(
    /** @internal */ readonly alias: string,
    /** @internal */ readonly name: string,
    /** @internal */ readonly field: AnyField,
  ) {}

  /** @internal */
  readonly queryColumn = (): WColumn => ({ source: this.alias, field: this.name });

  private compare(op: string, values: readonly T[]): QCond {
    const inner = this.field.spec.kind === "array" ? this.field.spec.inner! : this.field;
    const encoded = values.map((v) => dbValue(encodeFieldValue(inner, v, this.name)));
    return new QCond({ compare: { left: { column: this.queryColumn() }, op, values: encoded } });
  }
  eq(v: T): QCond {
    return this.compare("EQ", [v]);
  }
  ne(v: T): QCond {
    return this.compare("NE", [v]);
  }
  lt(v: T): QCond {
    return this.compare("LT", [v]);
  }
  le(v: T): QCond {
    return this.compare("LE", [v]);
  }
  gt(v: T): QCond {
    return this.compare("GT", [v]);
  }
  ge(v: T): QCond {
    return this.compare("GE", [v]);
  }
  in(values: readonly T[]): QCond {
    return this.compare("IN", values);
  }
  isNull(): QCond {
    return this.compare("IS_NULL", []);
  }
  isNotNull(): QCond {
    return this.compare("IS_NOT_NULL", []);
  }
  eqCol(other: FieldRef<any, T, any, any> | AliasCol<T, any, any>): ColCmp {
    return new ColCmp(this, "EQ", other);
  }
  asc(): QOrder {
    return new QOrder({ term: { column: this.queryColumn() }, desc: false });
  }
  desc(): QOrder {
    return new QOrder({ term: { column: this.queryColumn() }, desc: true });
  }
}

type AliasCols<T, Left extends boolean> = {
  readonly [K in keyof FieldsOf<T>]: FieldsOf<T>[K] extends FieldRef<any, infer V, infer R, infer O>
    ? AliasCol<V, Left extends true ? true : O, R>
    : never;
};
type FieldsOf<T> = T extends { f: infer F } ? F : never;

/** An entity under a name of its own, for an entity joined twice: `alias(platform.user, 'manager')` (inner) or
 * `.left()`; join it with `joinAs`, name its columns with `f`. */
export class Alias<T extends Queryable, Left extends boolean = false> {
  /** The alias's columns (`manager.f.displayName`). */
  readonly f: AliasCols<T, Left>;

  constructor(
    /** @internal */ readonly entity: T,
    readonly name: string,
    /** @internal */ readonly isLeft: Left,
  ) {
    const out: Record<string, AliasCol<unknown, boolean>> = {};
    for (const [key, ref] of Object.entries(entity.f as Record<string, FieldRef<any, any, any, any>>)) {
      out[key] = new AliasCol(name, ref.name, ref.field);
    }
    this.f = out as never;
  }

  /** The alias joined left: its columns may be null. */
  left(): Alias<T, true> {
    return new Alias(this.entity, this.name, true);
  }
}

/** An alias of an entity (`[a-z][a-z0-9_]*`). */
export function alias<T extends Queryable>(entity: T, name: string): Alias<T, false> {
  return new Alias(entity, name, false);
}

// --- aggregates, labels, orders -----------------------------------------------------------------------------------

/** An aggregate whose value is an `O`: selected, ordered by, or compared in `having`. */
export class Agg<O> {
  declare readonly __out: O;

  constructor(
    /** @internal */ readonly wire: WAgg,
    /** @internal */ readonly read: (v: unknown) => O,
  ) {}

  private compare(op: string, v: unknown): QCond {
    // an aggregate's value is compared as the text of its number (an int64, a decimal, a double), as the other SDKs do
    return new QCond({ compare: { left: { aggregate: this.wire }, op, values: [{ stringValue: String(v) }] } });
  }
  eq(v: NonNullable<O>): QCond {
    return this.compare("EQ", v);
  }
  ne(v: NonNullable<O>): QCond {
    return this.compare("NE", v);
  }
  lt(v: NonNullable<O>): QCond {
    return this.compare("LT", v);
  }
  le(v: NonNullable<O>): QCond {
    return this.compare("LE", v);
  }
  gt(v: NonNullable<O>): QCond {
    return this.compare("GT", v);
  }
  ge(v: NonNullable<O>): QCond {
    return this.compare("GE", v);
  }
  asc(): QOrder {
    return new QOrder({ term: { aggregate: this.wire }, desc: false });
  }
  desc(): QOrder {
    return new QOrder({ term: { aggregate: this.wire }, desc: true });
  }
}

type AnyColumn = FieldRef<any, any, any, any> | AliasCol<any, any, any> | Bucket<any, any> | Parsed<any, any, any>;
type ValueOfColumn<C> = C extends FieldRef<any, infer T, any, any>
  ? T
  : C extends AliasCol<infer T, any, any>
    ? T
    : C extends Bucket<any, any>
      ? string
      : C extends Parsed<any, infer V, any>
        ? V
        : never;

function columnField(c: AnyColumn): AnyField {
  return c.field;
}

/** The rows (of a group). */
export function count(): Agg<number> {
  return new Agg({ fn: "COUNT" }, (v) => Number(v ?? 0));
}

/** The distinct values of a column (nulls not counted). */
export function countDistinct(c: AnyColumn): Agg<number> {
  return new Agg({ fn: "COUNT_DISTINCT", column: wireColumn(c) }, (v) => Number(v ?? 0));
}

function number(field: AnyField, v: unknown): unknown {
  switch (field.spec.scalar) {
    case "decimal":
      return Decimal.parse(String(v));
    case "double":
      return Number(v);
    default:
      return BigInt(String(v));
  }
}

/** The sum of a number column: int32 and int64 sum to a bigint, a decimal to a `Decimal`, a double to a number (0
 * when there is none). */
export function sum<C extends AnyColumn>(c: C): Agg<SumOf<ValueOfColumn<C>>> {
  const f = columnField(c);
  return new Agg({ fn: "SUM", column: wireColumn(c) }, (v) => number(f, v ?? "0") as SumOf<ValueOfColumn<C>>);
}

/** The average of a number column: a `Decimal` of integers and decimals, a number of doubles (`undefined` when every
 * value is null). */
export function avg<C extends AnyColumn>(c: C): Agg<AvgOf<ValueOfColumn<C>> | undefined> {
  const f = columnField(c);
  return new Agg({ fn: "AVG", column: wireColumn(c) }, (v) => {
    if (v === null || v === undefined) return undefined;
    return (f.spec.scalar === "double" ? Number(v) : Decimal.parse(String(v))) as AvgOf<ValueOfColumn<C>>;
  });
}

/** The average of a column of this type. */
export type AvgOf<V> = V extends Decimal ? Decimal : V extends bigint ? Decimal : V extends number ? Decimal | number : never;

function extreme<C extends AnyColumn>(fn: string, c: C): Agg<ValueOfColumn<C> | undefined> {
  const f = columnField(c);
  return new Agg({ fn, column: wireColumn(c) }, (v) => readValue(f, v) as ValueOfColumn<C> | undefined);
}

/** The smallest value of a column (`undefined` when there is none). */
export function min<C extends AnyColumn>(c: C): Agg<ValueOfColumn<C> | undefined> {
  return extreme("MIN", c);
}

/** The largest value of a column (`undefined` when there is none). */
export function max<C extends AnyColumn>(c: C): Agg<ValueOfColumn<C> | undefined> {
  return extreme("MAX", c);
}

/** A row's label as `label(ref)` selects it: a person's (or a group's) name and avatar, another row's label column. */
export interface Label {
  id: string;
  text: string;
  avatarUrl?: string;
}

/** A `label(..)` select item. */
export class LabelItem {
  constructor(/** @internal */ readonly column: WColumn) {}
}

/** The label of the row a ref names (or of the row of an `id`): for every reader of that row, a guest included;
 * `undefined` when the ref is empty or names a row the caller may not read. */
export function label(ref: FieldRef<any, string, any, any> | AliasCol<string, any, any>): LabelItem {
  return new LabelItem(wireColumn(ref));
}

// --- select -------------------------------------------------------------------------------------------------------

type ColumnIn<X> = X extends any ? FieldRef<X, any, any, any> | Bucket<X, any> | Parsed<X, any, any> : never;

/** An item `select` takes over the entities `X` of the query. */
export type SelectItem<X> = ColumnIn<X> | AliasCol<any, any, any> | Agg<any> | LabelItem;

/** The value of a select item: a left-joined entity's column (and an optional one) may be `undefined`. */
export type ItemValue<I, L> =
  I extends FieldRef<infer M, infer T, any, infer O>
    ? [M] extends [L]
      ? T | undefined
      : O extends true
        ? T | undefined
        : T
    : I extends Bucket<infer M, infer O>
      ? [M] extends [L]
        ? string | undefined
        : O extends true
          ? string | undefined
          : string
    : I extends Parsed<infer M, infer V, infer O>
      ? [M] extends [L]
        ? V | undefined
        : O extends true
          ? V | undefined
          : V
    : I extends AliasCol<infer T, infer O, any>
      ? O extends true
        ? T | undefined
        : T
      : I extends Agg<infer V>
        ? V
        : I extends LabelItem
          ? Label | undefined
          : never;

type CondIn<X> = (X extends any ? Filter<X> : never) | ColCmp | QCond;
type OrderIn<X> = (X extends any ? Order<X> : never) | QOrder;

function readValue(field: AnyField, raw: unknown): unknown {
  if (raw === null || raw === undefined) return undefined;
  const errors: FieldErrors = {};
  let value: unknown;
  if (field.spec.kind === "array") {
    value = Array.isArray(raw) ? raw.map((x, i) => decodeFieldValue(field.spec.inner!, x, `[${i}]`, errors)) : undefined;
  } else {
    value = decodeFieldValue(field, raw, "value", errors);
  }
  if (Object.keys(errors).length > 0) throw internal(new Error(`the platform sent a value this plugin cannot read: ${JSON.stringify(errors)}`));
  return value;
}

type Reader = (row: unknown[]) => Record<string, unknown>;

/** What the select items are on the wire, and how a row's values are read. */
function selection(items: Record<string, unknown>): { wire: Record<string, unknown>[]; read: Reader; terms: Map<string, WTerm> } {
  const keys = Object.keys(items);
  const wire: Record<string, unknown>[] = [];
  const reads: ((v: unknown) => unknown)[] = [];
  const terms = new Map<string, WTerm>();
  for (const k of keys) {
    const item = items[k];
    if (item instanceof FieldRef || item instanceof AliasCol || item instanceof Bucket || item instanceof Parsed) {
      const column = wireColumn(item);
      wire.push({ column });
      terms.set(k, { column });
      reads.push((v) => readValue(item.field, v));
    } else if (item instanceof Agg) {
      wire.push({ aggregate: item.wire });
      terms.set(k, { aggregate: item.wire });
      reads.push((v) => item.read(v));
    } else if (item instanceof LabelItem) {
      wire.push({ label: item.column });
      reads.push((v) => {
        if (v === null || typeof v !== "object") return undefined;
        const o = v as Record<string, unknown>;
        const out: Label = { id: String(o.id ?? ""), text: String(o.text ?? "") };
        if (typeof o.avatarUrl === "string") out.avatarUrl = o.avatarUrl;
        return out;
      });
    } else {
      throw internal(new Error(`query: select ${k} is not a column, an aggregate or a label`));
    }
  }
  const read: Reader = (row) => {
    const out: Record<string, unknown> = {};
    keys.forEach((k, i) => {
      out[k] = reads[i]!(row[i]);
    });
    return out;
  };
  return { wire, read, terms };
}

// --- the query ----------------------------------------------------------------------------------------------------

/** Which data a query reads: the workspace's (`ctx.query`), its analytic copy (`ctx.analytics().query`) or its lake
 * (`ctx.lake().query`). */
export type QueryEngine = "STORE" | "ANALYTIC" | "LAKE";

/** A page of a query's rows and the token of the next page (`undefined`: the last); from the analytic copy, when its
 * data is from. */
export interface QueryPage<T> {
  rows: T[];
  next?: string;
  /** When the analytic copy's data is from (`undefined` from `ctx.query()`). */
  asOf?: Date;
  /** Which copy answered (`undefined` from `ctx.query()`). */
  generation?: bigint;
}

/** An answer and, from the analytic copy (`ctx.analytics()`), when its data is from (`undefined` from `ctx.query()`). */
export interface AsOf<T> {
  rows: T;
  asOf?: Date;
  /** Which copy answered (`undefined` from `ctx.query()`). */
  generation?: bigint;
}

/** When an answer's data is from: unix milliseconds on the wire, 0 for an answer that is not analytic. */
function stamp(asOf: string | number | undefined, generation: string | number | undefined): { asOf?: Date; generation?: bigint } {
  const ms = Number(asOf ?? 0);
  if (!ms) return {};
  return { asOf: new Date(ms), generation: BigInt(generation ?? 0) };
}

/** The analytic copy of the workspace's entities (`ctx.analytics()`). */
export class Analytics {
  constructor(/** @internal */ private readonly link: DbLink) {}

  /** `ctx.query(entity)` on the analytic copy: the same builder, rules and answers, plus `asOf`. Its reads are `all`,
   * `allAsOf`, `first`, `count`, `countAsOf` and `page`: the repository's (`get`, `find`, `exists`, `aggregate`) read
   * only the workspace's data, through `ctx.query`. */
  query<E extends Queryable>(entity: E): Query<E> {
    return startQuery(this.link, entity, "ANALYTIC");
  }

  /**
   * A snapshot of the analytic copy to ask for, that loads some of the plugin's entities stored in OLAP
   * (`.load(e)..request()`, `olap.ts`).
   */
  snapshot(): Snapshot {
    return new Snapshot(this.link);
  }
}

interface Tree {
  sources: Record<string, unknown>[];
  filter?: WExpr;
  groupBy: WColumn[];
  having?: WExpr;
  select: Record<string, unknown>[];
  order: { term: WTerm; desc: boolean }[];
  limit: number;
}

/** Where the query was composed: the first frame of the stack outside the SDK (`src/report.ts:52`). */
function callerSource(): string {
  const sdk = new URL(".", import.meta.url).pathname;
  const lines = (new Error().stack ?? "").split("\n").slice(1);
  for (const line of lines) {
    const m = /\(?((?:file:\/\/)?[^()\s]+):(\d+):\d+\)?\s*$/.exec(line.trim());
    if (!m) continue;
    const file = m[1]!.replace(/^file:\/\//, "");
    if (file.startsWith(sdk) || file.startsWith("node:")) continue;
    return `${file}:${m[2]}`;
  }
  return "";
}

type Rows = { rows: unknown[]; next?: string; asOf?: Date; generation?: bigint };

class Sender {
  constructor(
    readonly link: DbLink,
    readonly tree: Tree,
    readonly error: string | undefined,
    readonly source: string,
    readonly engine: QueryEngine,
  ) {}

  async send(mode: string, token?: string): Promise<DbResult & { dbRows?: DbResult["dbRows"] }> {
    if (this.error) throw new RpcError(500, "query_invalid", this.error);
    const query: Record<string, unknown> = { sources: this.tree.sources, groupBy: this.tree.groupBy, select: this.tree.select, order: this.tree.order, limit: this.tree.limit };
    if (this.tree.filter) query.filter = this.tree.filter;
    if (this.tree.having) query.having = this.tree.having;
    return this.link.syscall({
      query: {
        requestId: this.link.requestId,
        tx: this.link.tx,
        query,
        mode,
        pageToken: token ?? "",
        asSystem: this.link.asSystem,
        source: this.source,
        engine: this.engine,
      },
    });
  }

  async rows(mode: string, token?: string): Promise<Rows> {
    const r = await this.send(mode, token);
    if (!r.dbRows) throw internal(new Error("the platform answered a query with something else"));
    const rows = (r.dbRows.rows ?? []).map((b) => {
      try {
        return JSON.parse(Buffer.from(b).toString("utf8")) as unknown;
      } catch (e) {
        throw internal(e);
      }
    });
    const next = r.dbRows.nextPageToken || undefined;
    const out: Rows = { rows, ...stamp(r.dbRows.asOf, r.dbRows.generation) };
    if (next) out.next = next;
    return out;
  }

  async count(): Promise<AsOf<number>> {
    const r = await this.send("COUNT");
    if (!r.dbCount) throw internal(new Error("the platform answered a count with something else"));
    return { rows: Number(r.dbCount.count), ...stamp(r.dbCount.asOf, r.dbCount.generation) };
  }
}

type StoredOf<R> = R extends EntityDef<any>
  ? Stored<R>
  : R extends ViewDef<any>
    ? ViewRow<R>
    : R extends LakeTableDef<any, any>
      ? LakeRow<R>
      : R extends HistoryDef<infer E>
        ? HistoryRow<E>
        : never;

/** A page of an answer's rows, with its next page's token and, from the analytic copy, its time. */
function pageOf<T>(r: Rows, rows: T[]): QueryPage<T> {
  const out: QueryPage<T> = { rows };
  if (r.next) out.next = r.next;
  if (r.asOf) Object.assign(out, { asOf: r.asOf, generation: r.generation });
  return out;
}

/** An answer with its time (none from the workspace's data). */
function asOfOf<T>(r: { asOf?: Date; generation?: bigint }, rows: T): AsOf<T> {
  return r.asOf ? { rows, asOf: r.asOf, generation: r.generation! } : { rows };
}

/**
 * A query over the root `Root`, the entities `J` (joined inner, the root among them) and `L` (joined left). Joins,
 * `where`, `groupBy`, `having`, `orderBy` and `limit` build it; `select` names its rows' values; `all`, `first`,
 * `count` and `page` read it. A dependency's entity keeps the repository's reads too (`get`, `find`, `exists`,
 * `aggregate`).
 */
export class Query<Root extends Queryable, J = RowOf<Root>, L = never> {
  /** @internal */ readonly tree: Tree;
  /** @internal */ error?: string;

  constructor(
    /** @internal */ readonly link: DbLink,
    /** @internal */ readonly root: Root,
    /** @internal */ readonly source: string,
    /** @internal the repository of an entity root (its reads) */
    readonly repo?: ReadRepository<any>,
    /** @internal which data it reads */
    readonly engine: QueryEngine = "STORE",
  ) {
    this.tree = { sources: [{ entity: canonicalOf(root), alias: defaultAlias(root) }], groupBy: [], select: [], order: [], limit: 0 };
  }

  private fail(message: string): void {
    this.error ??= message;
  }

  /** The alias of the one source of `entity` (its default alias first). */
  private sourceOf(entity: string): string | undefined {
    const all = this.tree.sources.filter((s) => s.entity === entity);
    const def = all.find((s) => s.alias === entity.replace(".", "_"));
    if (def) return String(def.alias);
    return all.length === 1 ? String(all[0]!.alias) : undefined;
  }

  private addJoin(entity: Queryable, alias: string, left: boolean, ref: FieldRef<any, any, any, any>): void {
    let name: string;
    let owner: string;
    let target: string;
    try {
      name = canonicalOf(entity);
      if (!ref.owner) throw new Error("the ref is not an entity's field");
      owner = canonicalOf(ref.owner);
      target = refTarget(ref);
    } catch (e) {
      this.fail(`join: ${(e as Error).message}`);
      return;
    }
    let l: WColumn;
    let r: WColumn;
    const from = this.sourceOf(owner);
    if (target === name && from !== undefined) {
      l = { source: from, field: ref.name };
      r = { source: alias, field: "id" };
    } else if (owner === name) {
      const to = this.sourceOf(target);
      if (to === undefined) {
        this.fail(`join of ${name}: the query has no ${target} (or more than one: join with joinOn)`);
        return;
      }
      l = { source: to, field: "id" };
      r = { source: alias, field: ref.name };
    } else {
      this.fail(`join of ${name}: the query has no ${owner} (or more than one: join with joinOn)`);
      return;
    }
    this.tree.sources.push({ entity: name, alias, join: left ? "LEFT" : "INNER", left: l, right: r });
  }

  private addJoinOn(entity: Queryable, alias: string, left: boolean, on: ColCmp): void {
    if (on.op !== "EQ" || on.right instanceof DaysAgo) return this.fail(`joinOn of ${alias}: two columns are compared with eqCol`);
    const a = wireColumn(on.left);
    const b = wireColumn(on.right);
    const [l, r] = b.source === alias ? [a, b] : a.source === alias ? [b, a] : [undefined, undefined];
    if (!l || !r) return this.fail(`joinOn of ${alias}: neither column is one of ${alias}`);
    this.tree.sources.push({ entity: canonicalOf(entity), alias, join: left ? "LEFT" : "INNER", left: l, right: r });
  }

  /** Joins `entity` along a ref between it and an entity of the query, either way. */
  join<T extends Queryable>(entity: T, ref: JoinRefOf<T>): Query<Root, J | RowOf<T>, L> {
    this.addJoin(entity, defaultAlias(entity), false, ref);
    return this as never;
  }

  /** As `join`, keeping the rows that have no `entity` (or none the caller may read): its columns may be undefined. */
  leftJoin<T extends Queryable>(entity: T, ref: JoinRefOf<T>): Query<Root, J, L | RowOf<T>> {
    this.addJoin(entity, defaultAlias(entity), true, ref);
    return this as never;
  }

  /**
   * Joins `entity` on two equal columns (`a.eqCol(b)`), one of `entity` and one of the query: one entity's keys, or two
   * plain columns of one type (`product.f.sku.eqCol(orderLine.f.sku)`, a bucket with a date; never a list, json or an
   * encrypted field).
   */
  joinOn<T extends Queryable>(entity: T, on: ColCmp): Query<Root, J | RowOf<T>, L> {
    this.addJoinOn(entity, defaultAlias(entity), false, on);
    return this as never;
  }

  leftJoinOn<T extends Queryable>(entity: T, on: ColCmp): Query<Root, J, L | RowOf<T>> {
    this.addJoinOn(entity, defaultAlias(entity), true, on);
    return this as never;
  }

  /** Joins an aliased entity (inner or left as the alias was made); its columns are the alias's (`a.f`). */
  joinAs<T extends Queryable>(a: Alias<T, boolean>, ref: JoinRefOf<T>): this {
    this.addJoin(a.entity, a.name, a.isLeft, ref);
    return this;
  }

  joinOnAs<T extends Queryable>(a: Alias<T, boolean>, on: ColCmp): this {
    this.addJoinOn(a.entity, a.name, a.isLeft, on);
    return this;
  }

  /** Keeps the rows for which every condition holds (several `where`s: all of them). */
  where(...conds: CondIn<J | L>[]): this {
    this.tree.filter = combine(this.tree.filter, conds);
    return this;
  }

  /** Groups the rows by these columns. */
  groupBy(...columns: (ColumnIn<J | L> | AliasCol<any, any, any>)[]): this {
    for (const c of columns) this.tree.groupBy.push(wireColumn(c));
    return this;
  }

  /** Keeps the groups for which every condition holds (aggregates and grouped columns). */
  having(...conds: CondIn<J | L>[]): this {
    this.tree.having = combine(this.tree.having, conds);
    return this;
  }

  /** Orders the rows (then by their keys). */
  orderBy(...orders: OrderIn<J | L>[]): this {
    for (const o of orders) this.tree.order.push(orderWire(o));
    return this;
  }

  /** At most `n` rows (a page: at most 100). */
  limit(n: number): this {
    this.tree.limit = n;
    return this;
  }

  /**
   * A history read (`ctx.lake().history(e)`) per day: one row per day each version was in force at the day's last
   * moment, in the workspace's time zone, with its `day` (`historyOf(e).f.day`).
   */
  eachDay(this: Query<HistoryDef<any>, J, L>): Query<Root, J, L> {
    const root = this.tree.sources[0]!;
    root.eachDay = true;
    return this as never;
  }

  /** The values of the rows, by name: columns, aggregates, labels. */
  select<S extends Record<string, SelectItem<J | L>>>(items: S): Selected<{ [K in keyof S]: ItemValue<S[K], L> }> {
    try {
      const s = selection(items);
      this.tree.select = s.wire;
      return new Selected(this.sender(), s.read, s.terms) as never;
    } catch (e) {
      this.fail((e as Error).message);
      return new Selected(this.sender(), () => ({}), new Map()) as never;
    }
  }

  /** @internal */
  sender(): Sender {
    return new Sender(this.link, this.tree, this.error, this.source, this.engine);
  }

  /** A plain read of the workspace's data, answered by the repository. */
  private plain(): boolean {
    const t = this.tree;
    return (
      this.engine === "STORE" &&
      this.repo !== undefined &&
      !this.error &&
      t.sources.length === 1 &&
      !t.filter &&
      t.groupBy.length === 0 &&
      !t.having &&
      t.order.length === 0 &&
      t.limit === 0
    );
  }

  /** The root's rows (at most 1,000: `query_rows_limit` past them). */
  async all(): Promise<StoredOf<Root>[]> {
    if (this.plain()) return (await this.repo!.all()) as never;
    return (await this.allAsOf()).rows;
  }

  /** `all`, with when its data is from. */
  async allAsOf(): Promise<AsOf<StoredOf<Root>[]>> {
    const r = await this.sender().rows("ALL");
    return asOfOf(r, r.rows.map((x) => this.decodeRoot(x)) as StoredOf<Root>[]);
  }

  /** The root's first row. */
  async first(): Promise<StoredOf<Root> | undefined> {
    const { rows } = await this.sender().rows("FIRST");
    return rows[0] === undefined ? undefined : (this.decodeRoot(rows[0]) as never);
  }

  /** A page of the root's rows (`limit`, at most 100) after the token of the previous page. */
  async page(token?: string): Promise<QueryPage<StoredOf<Root>>> {
    const r = await this.sender().rows("PAGE", token);
    return pageOf(r, r.rows.map((x) => this.decodeRoot(x)) as StoredOf<Root>[]);
  }

  /** How many rows (or groups) the query has; with a filter of the root, those it matches. */
  async count(filter?: Filter<RowOf<Root>>): Promise<number> {
    if (this.plain()) return this.repo!.count(filter as never);
    return (await this.countAsOf(filter)).rows;
  }

  /** `count`, with when its data is from. */
  async countAsOf(filter?: Filter<RowOf<Root>>): Promise<AsOf<number>> {
    if (!filter) return this.sender().count();
    // the filter counts these rows only: the query itself is not changed
    const tree = { ...this.tree, filter: combine(this.tree.filter, [filter]) };
    return new Sender(this.link, tree, this.error, this.source, this.engine).count();
  }

  private decodeRoot(raw: unknown): unknown {
    if (isHistorySource(this.root)) return decodeHistoryRow(this.root.entity, raw);
    if (this.root instanceof ViewDef || isLakeSource(this.root)) {
      const errors: FieldErrors = {};
      const row = decodeMessage(this.root, raw, "", errors);
      if (Object.keys(errors).length > 0) throw internal(new Error(`the platform sent a row this plugin cannot read: ${JSON.stringify(errors)}`));
      return row;
    }
    if (!(this.root instanceof EntityDef)) throw internal(new Error("query: a directory entity's rows are read with select"));
    const json = raw as Record<string, unknown>;
    const errors: FieldErrors = {};
    const row = decodeMessage(this.root, json, "", errors) as Record<string, unknown>;
    if (Object.keys(errors).length > 0) throw internal(new Error(`the platform sent a row this plugin cannot read: ${JSON.stringify(errors)}`));
    row.createdAt = new Date(String(json.createdAt));
    row.updatedAt = new Date(String(json.updatedAt));
    if (typeof json.deletedAt === "string") row.deletedAt = new Date(json.deletedAt);
    if (json.version !== undefined && json.version !== null) row.version = BigInt(String(json.version));
    return row;
  }

  private repository(): ReadRepository<any> {
    if (this.root instanceof ViewDef) throw internal(new Error("query: a view is read with all, first, count and page"));
    if (isLakeSource(this.root)) throw internal(new Error("query: a lake table is read with all, first, count and page"));
    if (isHistorySource(this.root)) throw internal(new Error("query: a history is read with all, first, count and page"));
    if (!this.repo) throw internal(new Error("query: the directory has no repository reads"));
    if (this.engine !== "STORE") {
      throw internal(new Error("ctx.analytics().query(): get, find, exists and aggregate read the workspace's data; use ctx.query()"));
    }
    return this.repo;
  }

  /** A row by id (the repository's read). */
  get(id: string, options: GetOptions = {}): Promise<StoredOf<Root>> {
    return this.repository().get(id, options) as never;
  }

  /** The rows matching `filter` (the repository's read): order, limit and page it. */
  find(filter?: Filter<RowOf<Root>>): Root extends EntityDef<any> ? Find<Root> : never {
    return this.repository().find(filter as never) as never;
  }

  async exists(filter: Filter<RowOf<Root>>): Promise<boolean> {
    return this.repository().exists(filter as never);
  }

  /** Groups of the rows (the repository's `aggregate`). */
  get aggregate(): Root extends EntityDef<any> ? ReadRepository<Root>["aggregate"] : never {
    const repo = this.repository();
    return repo.aggregate.bind(repo) as never;
  }
}

/** A query whose rows are `T` (`select`): `orderBy` (by a select item's name too) and `limit` still apply. */
export class Selected<T> {
  constructor(
    /** @internal */ readonly q: Sender,
    /** @internal */ readonly read: Reader,
    /** @internal */ readonly terms: Map<string, WTerm>,
  ) {}

  /** Orders the rows: by a select item (`{ revenue: 'desc' }`), a column or an aggregate; then by the keys. */
  orderBy(...orders: (QOrder | Order<any> | { [K in keyof T]?: "asc" | "desc" })[]): this {
    for (const o of orders) {
      if (o instanceof QOrder || o instanceof Order) {
        this.q.tree.order.push(orderWire(o));
        continue;
      }
      for (const [k, dir] of Object.entries(o as Record<string, string>)) {
        const term = this.terms.get(k);
        if (!term) throw internal(new Error(`query: orderBy ${k}: not a column or an aggregate of the select`));
        this.q.tree.order.push({ term, desc: dir === "desc" });
      }
    }
    return this;
  }

  /** At most `n` rows (a page: at most 100). */
  limit(n: number): this {
    this.q.tree.limit = n;
    return this;
  }

  private decode(rows: unknown[]): T[] {
    return rows.map((r) => {
      if (!Array.isArray(r)) throw internal(new Error("the platform sent a query row that is not a list"));
      return this.read(r) as T;
    });
  }

  /** Every row (at most 1,000, or `limit`: `query_rows_limit` past them). */
  async all(): Promise<T[]> {
    return (await this.allAsOf()).rows;
  }

  /** `all`, with when its data is from. */
  async allAsOf(): Promise<AsOf<T[]>> {
    const r = await this.q.rows("ALL");
    return asOfOf(r, this.decode(r.rows));
  }

  async first(): Promise<T | undefined> {
    return this.decode((await this.q.rows("FIRST")).rows)[0];
  }

  /** A page (`limit`, at most 100) after the token of the previous page: keyset over the order and the keys. */
  async page(token?: string): Promise<QueryPage<T>> {
    const r = await this.q.rows("PAGE", token);
    return pageOf(r, this.decode(r.rows));
  }

  /** How many rows (or groups). */
  async count(): Promise<number> {
    return (await this.q.count()).rows;
  }

  /** `count`, with when its data is from. */
  async countAsOf(): Promise<AsOf<number>> {
    return this.q.count();
  }

  /** The query tree the SDK sends (`celobus.v1.Query`), in the protobuf JSON mapping. */
  tree(): Record<string, unknown> {
    const t = this.q.tree;
    const out: Record<string, unknown> = { sources: t.sources };
    if (t.filter) out.filter = t.filter;
    if (t.groupBy.length) out.groupBy = t.groupBy;
    if (t.having) out.having = t.having;
    if (t.select.length) out.select = t.select;
    if (t.order.length) out.order = t.order;
    if (t.limit) out.limit = t.limit;
    return out;
  }
}

function combine(prev: WExpr | undefined, conds: CondLike[]): WExpr | undefined {
  const next = and(...conds).expr;
  if (!prev) return next;
  if (!next) return prev;
  return { and: { items: [prev, next] } };
}

function orderWire(o: QOrder | Order<any>): { term: WTerm; desc: boolean } {
  if (o instanceof QOrder) return o.wire;
  if (!o.source) throw internal(new Error(`query: the order by ${o.field} is not of an entity's field handle`));
  return { term: { column: { source: defaultAlias(o.source), field: o.field } }, desc: o.desc };
}

/** The entity a ref names, by its name in a query. */
function refTarget(ref: FieldRef<any, any, any, any>): string {
  const spec = ref.field.spec;
  if (spec.kind !== "ref" || !spec.ref) throw new Error(`${ref.name} is not a ref`);
  const t = spec.ref;
  if (typeof t === "string") return t;
  if (t instanceof PlatformEntity || t instanceof EntityDef) return canonicalOf(t);
  return canonicalOf(t());
}

/** @internal The start of a query (`ctx.query`). */
export function startQuery<E extends Queryable>(link: DbLink, entity: E, engine: QueryEngine = "STORE"): Query<E> {
  let repo: ReadRepository<any> | undefined;
  if (entity instanceof EntityDef) {
    if (entity.options.storage === "olap" && engine !== "ANALYTIC") {
      throw internal(new Error("ctx.query(): an entity stored in OLAP is read with ctx.analytics().query()"));
    }
    const provider = foreignPlugin(entity);
    const name = provider ? entity.options.name : (entityNames.get(entity) ?? entity.options.name);
    if (!name) throw internal(new Error("ctx.query(): the entity is not in plugin({ entities })"));
    repo = new ReadRepository(link, entity, name, provider ?? "");
  } else if (isLakeSource(entity)) {
    if (engine !== "LAKE") throw internal(new Error("ctx.query(): a lake table is read with ctx.lake().query()"));
  } else if (isHistorySource(entity)) {
    if (engine !== "LAKE") throw internal(new Error("ctx.query(): a history is read with ctx.lake().history()"));
  } else if (!(entity instanceof PlatformEntity) && !(entity instanceof ViewDef)) {
    throw internal(new Error("ctx.query(): not an entity, a view or a lake table"));
  }
  if (engine === "LAKE" && !isLakeSource(entity) && !isHistorySource(entity)) {
    throw internal(new Error("ctx.lake().query(): not a lake table"));
  }
  const q = new Query(link, entity, callerSource(), repo, engine);
  if (isHistorySource(entity)) q.tree.sources[0]!.history = true;
  return q;
}


// --- analytic views -----------------------------------------------------------------------------------------------

const NO_LINK: DbLink = {
  requestId: "0",
  tx: "0",
  asSystem: "",
  syscall: async () => {
    throw internal(new Error("a view's query is declared, not read: read the view with ctx.query(view)"));
  },
} as unknown as DbLink;

/** What a view's column may be for a field `F` of its row: a column of a source of that type (a bucket of a date), or,
 * grouped, an aggregate (`count()`, `sum(..)`). The platform checks the types exactly (`celobus plugin check`). */
export type ViewItemFor<F> =
  | FieldRef<any, NonNullable<ValueOf<F>>, any, any>
  | AliasCol<NonNullable<ValueOf<F>>, any, any>
  | (NonNullable<ValueOf<F>> extends string ? Bucket<any, any> : never)
  | Agg<any>;

/** A view's query as `columns` ends it: what the descriptor declares. */
export class ViewDecl<C extends Shape> {
  declare readonly __row: C;
  constructor(
    /** @internal */ readonly query: Query<any, any, any>,
    /** @internal */ readonly items: Record<string, unknown>,
  ) {}
}

/** The start of a view's query: `view({ query: (q) => q.from(orderLine).join(..).columns({..}) })`. */
export class ViewStart<C extends Shape> {
  declare readonly __row: C;

  /** The view's root: an entity of the plugin, a dependency's exported entity, the directory's, another view. */
  from<E extends Queryable>(entity: E): ViewQuery<C, RowOf<E>, never> {
    return new ViewQuery<C, RowOf<E>, never>(new Query(NO_LINK, entity, ""));
  }
}

/**
 * A view's query over the sources `J` (joined inner, the root among them) and `L` (joined left): joins, `where`,
 * `groupBy` and `having` as a query's, then `columns`, which gives each of the row's columns its value.
 */
export class ViewQuery<C extends Shape, J, L> {
  constructor(/** @internal */ readonly q: Query<any, any, any>) {}

  /** Joins `entity` along a ref between it and a source of the view, either way. */
  join<T extends Queryable>(entity: T, ref: JoinRefOf<T>): ViewQuery<C, J | RowOf<T>, L> {
    this.q.join(entity, ref);
    return this as never;
  }

  /** As `join`, keeping the rows that have no `entity` (or none the caller may read): its columns are null there. */
  leftJoin<T extends Queryable>(entity: T, ref: JoinRefOf<T>): ViewQuery<C, J, L | RowOf<T>> {
    this.q.leftJoin(entity, ref);
    return this as never;
  }

  /** Joins `entity` on two equal columns (`stage.f.name.eqCol(task.f.status)`): one entity's keys, or two plain
   * columns of one type. */
  joinOn<T extends Queryable>(entity: T, on: ColCmp): ViewQuery<C, J | RowOf<T>, L> {
    this.q.joinOn(entity, on);
    return this as never;
  }

  leftJoinOn<T extends Queryable>(entity: T, on: ColCmp): ViewQuery<C, J, L | RowOf<T>> {
    this.q.leftJoinOn(entity, on);
    return this as never;
  }

  /** Joins an aliased entity (an entity joined twice), inner or left as the alias was made. */
  joinAs<T extends Queryable>(a: Alias<T, boolean>, ref: JoinRefOf<T>): this {
    this.q.joinAs(a, ref);
    return this;
  }

  joinOnAs<T extends Queryable>(a: Alias<T, boolean>, on: ColCmp): this {
    this.q.joinOnAs(a, on);
    return this;
  }

  /** Keeps the rows for which every condition holds (several `where`s: all of them); `daysAgo(n)` is worked out
   * when the view is read. */
  where(...conds: CondIn<J | L>[]): this {
    this.q.where(...(conds as never[]));
    return this;
  }

  /** Groups the rows by these columns: each must be one of the view's columns. */
  groupBy(...columns: (ColumnIn<J | L> | AliasCol<any, any, any>)[]): this {
    this.q.groupBy(...(columns as never[]));
    return this;
  }

  /** Keeps the groups for which every condition holds. */
  having(...conds: CondIn<J | L>[]): this {
    this.q.having(...(conds as never[]));
    return this;
  }

  /** The value of each of the row's columns: a column of a source (`order.f.paidAt.day()`) or, in a grouped view, an
   * aggregate (`sum(orderLine.f.amount)`, `count()`). */
  columns(items: { [K in keyof C]: ViewItemFor<C[K]> }): ViewDecl<C> {
    return new ViewDecl<C>(this.q, items as Record<string, unknown>);
  }
}

/** @internal A view as the descriptor declares it (`views`). */
export interface DescribedView {
  name: string;
  message: string;
  description: string;
  sources: Record<string, unknown>[];
  filter?: unknown;
  group_by?: unknown[];
  having?: unknown;
  columns: { name: string; value: unknown }[];
}

function dColumn(c: WColumn): Record<string, unknown> {
  const out: Record<string, unknown> = { source: c.source, field: c.field };
  if (c.bucket) out.bucket = c.bucket.toLowerCase();
  // `PARSE_DECIMAL` on the wire, `decimal` in the descriptor
  if (c.parse) out.parse = c.parse.slice("PARSE_".length).toLowerCase();
  return out;
}

function dAggregate(a: WAgg): Record<string, unknown> {
  const out: Record<string, unknown> = { fn: a.fn.toLowerCase() };
  if (a.column) out.column = dColumn(a.column);
  return out;
}

function dTerm(t: WTerm): Record<string, unknown> {
  if (t.column) return { column: dColumn(t.column) };
  if (t.aggregate) return { aggregate: dAggregate(t.aggregate) };
  if (t.daysAgo !== undefined) return { days_ago: t.daysAgo };
  throw new Error("a term is missing");
}

function dValue(v: Record<string, unknown>): unknown {
  if ("stringValue" in v) return v.stringValue;
  if ("intValue" in v) return Number(v.intValue);
  if ("doubleValue" in v) return v.doubleValue;
  if ("boolValue" in v) return v.boolValue;
  return null;
}

function dExpr(e: WExpr): unknown {
  if ("and" in e) return { and: e.and.items.map(dExpr) };
  if ("or" in e) return { or: e.or.items.map(dExpr) };
  if ("not" in e) return { not: dExpr(e.not) };
  if ("compare" in e) {
    const c = e.compare;
    const out: Record<string, unknown> = { left: dTerm(c.left), op: c.op.toLowerCase() };
    if (c.values && c.values.length > 0) out.values = c.values.map(dValue);
    if (c.right) out.right = dTerm(c.right);
    return { compare: out };
  }
  const x = e.exists;
  const out: Record<string, unknown> = { entity: x.source.entity, alias: x.source.alias };
  if (x.filter) out.filter = dExpr(x.filter);
  return { exists: out };
}

/** The sources of a tree as the descriptor writes them (a history root with `history` and `each_day`). */
function dSources(sources: Record<string, unknown>[]): Record<string, unknown>[] {
  return sources.map((s) => {
    const src: Record<string, unknown> = { entity: s.entity, alias: s.alias };
    if (typeof s.join === "string") src.join = s.join.toLowerCase();
    if (s.left) src.left = dColumn(s.left as WColumn);
    if (s.right) src.right = dColumn(s.right as WColumn);
    if (s.history) src.history = true;
    if (s.eachDay) src.each_day = true;
    return src;
  });
}

/** @internal A view as the descriptor declares it, or the first mistake made building it. */
export function describeView(view: ViewDef<any>, name: string, message: string): DescribedView | string {
  let decl: unknown;
  try {
    decl = view.options.query(new ViewStart());
  } catch (e) {
    return (e instanceof RpcError && e.cause instanceof Error ? e.cause : (e as Error)).message;
  }
  if (!(decl instanceof ViewDecl)) return "the query does not end with .columns({..})";
  const q = decl.query;
  if (q.error) return q.error;
  const keys = Object.keys(view.columns);
  for (const k of Object.keys(decl.items)) {
    if (!keys.includes(k)) return `${k} is not a column of the view`;
  }
  const columns: DescribedView["columns"] = [];
  try {
    for (const k of keys) {
      const item = decl.items[k];
      let value: unknown;
      if (item instanceof FieldRef || item instanceof AliasCol || item instanceof Bucket) value = { column: dColumn(wireColumn(item)) };
      else if (item instanceof Agg) value = { aggregate: dAggregate(item.wire) };
      else if (item === undefined) return `the column ${snake(k)} is not given`;
      else return `the column ${snake(k)} is not a column of a source or an aggregate`;
      columns.push({ name: snake(k), value });
    }
    const t = q.tree;
    const out: DescribedView = {
      name,
      message,
      description: view.options.description ?? "",
      sources: dSources(t.sources),
      columns,
    };
    if (t.filter) out.filter = dExpr(t.filter);
    if (t.groupBy.length > 0) out.group_by = t.groupBy.map(dColumn);
    if (t.having) out.having = dExpr(t.having);
    return out;
  } catch (e) {
    return (e instanceof RpcError && e.cause instanceof Error ? e.cause : (e as Error)).message;
  }
}


// --- models of the lake -------------------------------------------------------------------------------------------

/** What a model's column may be for a field `F` of its table's row: a view's column, or a parsed text of a lake table. */
export type ModelItemFor<F> = ViewItemFor<F> | Parsed<any, NonNullable<ValueOf<F>>, any>;

/** A model's query as `columns` ends it: what the descriptor declares (`lake_models`). */
export class ModelDecl<C extends Shape> {
  declare readonly __row: C;
  constructor(
    /** @internal */ readonly query: Query<any, any, any>,
    /** @internal */ readonly items: Record<string, unknown>,
    /** @internal */ readonly unmasked: boolean,
  ) {}
}

/**
 * The start of a model's query (`lakeTable({ model: (m) => m.from(stockFile)..columns({..}) })`): its root is a lake
 * table (the plugin's own its system context reads, or another plugin's it declares and whose owner lets models copy
 * it), an entity or an analytic view of the plugin, or one of the plugin's entities' histories; never lake tables
 * with entities or views in one model.
 */
export class ModelStart<C extends Shape> {
  declare readonly __row: C;

  /** The model's root: a lake table, or an entity or an analytic view of the plugin. */
  from<E extends LakeTableDef<any, any> | EntityDef<any> | ViewDef<any>>(source: E): ModelQuery<C, RowOf<E>, never> {
    return new ModelQuery<C, RowOf<E>, never>(new Query(NO_LINK, source, ""));
  }

  /**
   * The model's root is the history of one of the plugin's own entities (`.eachDay()` for one row per day): its
   * columns are the entity's kept ones (`ticket.f.status`) and the history's own (`historyOf(ticket).f.day`). Such a
   * model reads the lake: it may join lake tables, never entities. It reads the history as the plugin's system
   * context: the history must say `systemRead`.
   */
  fromHistory<E extends EntityDef<any>>(entity: E): ModelQuery<C, Row<E> | HistoryColumns<E>, never> {
    const q = new Query(NO_LINK, historyOf(entity), "");
    q.tree.sources[0]!.history = true;
    return new ModelQuery(q);
  }
}

/**
 * A model's query over the sources `J` (joined inner, the root among them) and `L` (joined left): a view's verbs
 * (joins, `where`, `groupBy`, `having`), `eachDay` of a history root, `unmasked`, then `columns`, which gives each of
 * the table's columns its value.
 */
export class ModelQuery<C extends Shape, J, L> {
  /** @internal */ masked = false;
  constructor(/** @internal */ readonly q: Query<any, any, any>) {}

  /** Joins `source` along a ref between it and a source of the model, either way. */
  join<T extends Queryable>(source: T, ref: JoinRefOf<T>): ModelQuery<C, J | RowOf<T>, L> {
    this.q.join(source, ref);
    return this as never;
  }

  /** As `join`, keeping the rows that have no `source`: its columns are null there. */
  leftJoin<T extends Queryable>(source: T, ref: JoinRefOf<T>): ModelQuery<C, J, L | RowOf<T>> {
    this.q.leftJoin(source, ref);
    return this as never;
  }

  /** Joins `source` on two equal columns (`product.f.sku.eqCol(stock.f.sku)`). */
  joinOn<T extends Queryable>(source: T, on: ColCmp): ModelQuery<C, J | RowOf<T>, L> {
    this.q.joinOn(source, on);
    return this as never;
  }

  leftJoinOn<T extends Queryable>(source: T, on: ColCmp): ModelQuery<C, J, L | RowOf<T>> {
    this.q.leftJoinOn(source, on);
    return this as never;
  }

  /** Joins an aliased source (one joined twice), inner or left as the alias was made. */
  joinAs<T extends Queryable>(a: Alias<T, boolean>, ref: JoinRefOf<T>): this {
    this.q.joinAs(a, ref);
    return this;
  }

  joinOnAs<T extends Queryable>(a: Alias<T, boolean>, on: ColCmp): this {
    this.q.joinOnAs(a, on);
    return this;
  }

  /** Keeps the rows for which every condition holds (several `where`s: all of them). */
  where(...conds: CondIn<J | L>[]): this {
    this.q.where(...(conds as never[]));
    return this;
  }

  /** Groups the rows by these columns: each must be one of the model's columns. */
  groupBy(...columns: (ColumnIn<J | L> | AliasCol<any, any, any>)[]): this {
    this.q.groupBy(...(columns as never[]));
    return this;
  }

  /** Keeps the groups for which every condition holds. */
  having(...conds: CondIn<J | L>[]): this {
    this.q.having(...(conds as never[]));
    return this;
  }

  /** A history root per day: one row per day each version was in force at the day's end, with its `day`. */
  eachDay(): this {
    const root = this.q.tree.sources[0]!;
    if (!root.history) this.q.error ??= "eachDay: the model's root is not a history (m.fromHistory(entity))";
    root.eachDay = true;
    return this;
  }

  /**
   * The masked columns of the entities under the model go to the lake as they are: the lake has no masks, and whoever
   * reads the table reads every row of it.
   */
  unmasked(): this {
    this.masked = true;
    return this;
  }

  /** The value of each of the table's columns: a column of a source (a bucket, a parsed text) or, grouped, an
   * aggregate. */
  columns(items: { [K in keyof C]: ModelItemFor<C[K]> }): ModelDecl<C> {
    return new ModelDecl<C>(this.q, items as Record<string, unknown>, this.masked);
  }
}

// --- loads of entities stored in OLAP ------------------------------------------------------------------------

type OptionalKeys<S> = { [K in keyof S]: S[K] extends Field<any, true, any> ? K : never }[keyof S];
type RequiredKeys<S> = Exclude<keyof S, OptionalKeys<S>>;

/** A load's query as `columns` ends it: what the descriptor declares (`loads`). */
export class LoadDecl<C extends Shape> {
  declare readonly __row: C;
  constructor(
    /** @internal */ readonly query: Query<any, any, any>,
    /** @internal */ readonly items: Record<string, unknown>,
  ) {}
}

/**
 * The start of a load's query (`entity({ storage: 'olap', load: (l) => l.from(storeMetrics)..columns({..}) })`): its
 * root is a lake table (the plugin's own its system context reads, or another plugin's it declares and whose owner lets
 * models copy it) or one of the plugin's own entities' histories; never an entity or an analytic view (a view of the
 * plugin joins the loaded entity with those).
 */
export class LoadStart<C extends Shape> {
  declare readonly __row: C;

  /** The load's root: a lake table. */
  from<E extends LakeTableDef<any, any>>(source: E): LoadQuery<C, RowOf<E>, never> {
    return new LoadQuery<C, RowOf<E>, never>(new Query(NO_LINK, source, ""));
  }

  /** The load's root is the history of one of the plugin's own entities (`.eachDay()` for one row per day). */
  fromHistory<E extends EntityDef<any>>(entity: E): LoadQuery<C, Row<E> | HistoryColumns<E>, never> {
    const q = new Query(NO_LINK, historyOf(entity), "");
    q.tree.sources[0]!.history = true;
    return new LoadQuery(q);
  }
}

/**
 * A load's query over the sources `J` (joined inner, the root among them) and `L` (joined left): a view's verbs
 * (joins, `where`, `groupBy`, `having`), `eachDay` of a history root, then `columns`, which gives each of the
 * entity's columns its value.
 */
export class LoadQuery<C extends Shape, J, L> {
  constructor(/** @internal */ readonly q: Query<any, any, any>) {}

  /** Joins `source` along a ref between it and a source of the model, either way. */
  join<T extends Queryable>(source: T, ref: JoinRefOf<T>): LoadQuery<C, J | RowOf<T>, L> {
    this.q.join(source, ref);
    return this as never;
  }

  /** As `join`, keeping the rows that have no `source`: its columns are null there. */
  leftJoin<T extends Queryable>(source: T, ref: JoinRefOf<T>): LoadQuery<C, J, L | RowOf<T>> {
    this.q.leftJoin(source, ref);
    return this as never;
  }

  /** Joins `source` on two equal columns (`product.f.sku.eqCol(stock.f.sku)`). */
  joinOn<T extends Queryable>(source: T, on: ColCmp): LoadQuery<C, J | RowOf<T>, L> {
    this.q.joinOn(source, on);
    return this as never;
  }

  leftJoinOn<T extends Queryable>(source: T, on: ColCmp): LoadQuery<C, J, L | RowOf<T>> {
    this.q.leftJoinOn(source, on);
    return this as never;
  }

  /** Joins an aliased source (one joined twice), inner or left as the alias was made. */
  joinAs<T extends Queryable>(a: Alias<T, boolean>, ref: JoinRefOf<T>): this {
    this.q.joinAs(a, ref);
    return this;
  }

  joinOnAs<T extends Queryable>(a: Alias<T, boolean>, on: ColCmp): this {
    this.q.joinOnAs(a, on);
    return this;
  }

  /** Keeps the rows for which every condition holds (several `where`s: all of them). */
  where(...conds: CondIn<J | L>[]): this {
    this.q.where(...(conds as never[]));
    return this;
  }

  /** Groups the rows by these columns: each must be one of the model's columns. */
  groupBy(...columns: (ColumnIn<J | L> | AliasCol<any, any, any>)[]): this {
    this.q.groupBy(...(columns as never[]));
    return this;
  }

  /** Keeps the groups for which every condition holds. */
  having(...conds: CondIn<J | L>[]): this {
    this.q.having(...(conds as never[]));
    return this;
  }

  /** A history root per day: one row per day each version was in force at the day's end, with its `day`. */
  eachDay(): this {
    const root = this.q.tree.sources[0]!;
    if (!root.history) this.q.error ??= "eachDay: the model's root is not a history (m.fromHistory(entity))";
    root.eachDay = true;
    return this;
  }

  /** The value of each of the entity's columns (`id` aside: it is made from the key): a column of a source (a bucket, a
   * parsed text) or, grouped, an aggregate; an optional column may be left out and reads absent. */
  columns(items: { [K in RequiredKeys<C>]: ModelItemFor<C[K]> } & { [K in OptionalKeys<C>]?: ModelItemFor<C[K]> }): LoadDecl<C> {
    return new LoadDecl<C>(this.q, items as Record<string, unknown>);
  }
}

/** @internal A load as the descriptor declares it (`loads`). */
export interface DescribedLoad {
  entity: string;
  sources: Record<string, unknown>[];
  filter?: unknown;
  group_by?: unknown[];
  having?: unknown;
  columns: { name: string; value: unknown }[];
}

/**
 * @internal An entity's load as the descriptor declares it, or the first mistake made building it (`code`: a column
 * missing is `E_LOAD_COLUMN`, any other `E_LOAD_DEFINE`).
 */
export function describeLoad(
  entity: string,
  columns: Shape,
  load: (l: LoadStart<any>) => LoadDecl<any>,
): DescribedLoad | { code: string; message: string } {
  const fail = (message: string, code = "E_LOAD_DEFINE") => ({ code, message });
  let decl: unknown;
  try {
    decl = load(new LoadStart());
  } catch (e) {
    return fail((e instanceof RpcError && e.cause instanceof Error ? e.cause : (e as Error)).message);
  }
  if (!(decl instanceof LoadDecl)) return fail("the load does not end with .columns({..})");
  const q = decl.query;
  if (q.error) return fail(q.error);
  const keys = Object.keys(columns);
  for (const k of Object.keys(decl.items)) {
    if (!keys.includes(k)) return fail(`${k} is not a column of ${entity}`, "E_LOAD_COLUMN");
  }
  const out: DescribedLoad["columns"] = [];
  try {
    for (const k of keys) {
      const item = decl.items[k];
      let value: unknown;
      if (item instanceof FieldRef || item instanceof AliasCol || item instanceof Bucket || item instanceof Parsed) {
        value = { column: dColumn(wireColumn(item)) };
      } else if (item instanceof Agg) value = { aggregate: dAggregate(item.wire) };
      else if (item === undefined) {
        // an optional column may be left out: it reads absent
        if ((columns[k] as AnyField).isOptional) continue;
        return fail(`the column ${snake(k)} is not given (a required column of ${entity})`, "E_LOAD_COLUMN");
      } else return fail(`the column ${snake(k)} is not a column of a source or an aggregate`);
      out.push({ name: snake(k), value });
    }
    const t = q.tree;
    const d: DescribedLoad = { entity, sources: dSources(t.sources), columns: out };
    if (t.filter) d.filter = dExpr(t.filter);
    if (t.groupBy.length > 0) d.group_by = t.groupBy.map(dColumn);
    if (t.having) d.having = dExpr(t.having);
    return d;
  } catch (e) {
    return fail((e instanceof RpcError && e.cause instanceof Error ? e.cause : (e as Error)).message);
  }
}

/** @internal A model as the descriptor declares it (`lake_models`). */
export interface DescribedModel {
  table: string;
  sources: Record<string, unknown>[];
  filter?: unknown;
  group_by?: unknown[];
  having?: unknown;
  columns: { name: string; value: unknown }[];
  unmasked?: true;
}

/** @internal A lake table's model as the descriptor declares it, or the first mistake made building it. */
export function describeModel(
  table: string,
  columns: Shape,
  model: (m: ModelStart<any>) => ModelDecl<any>,
): DescribedModel | string {
  let decl: unknown;
  try {
    decl = model(new ModelStart());
  } catch (e) {
    return (e instanceof RpcError && e.cause instanceof Error ? e.cause : (e as Error)).message;
  }
  if (!(decl instanceof ModelDecl)) return "the model does not end with .columns({..})";
  const q = decl.query;
  if (q.error) return q.error;
  const keys = Object.keys(columns);
  for (const k of Object.keys(decl.items)) {
    if (!keys.includes(k)) return `${k} is not a column of ${table}`;
  }
  const out: DescribedModel["columns"] = [];
  try {
    for (const k of keys) {
      const item = decl.items[k];
      let value: unknown;
      if (item instanceof FieldRef || item instanceof AliasCol || item instanceof Bucket || item instanceof Parsed) {
        value = { column: dColumn(wireColumn(item)) };
      } else if (item instanceof Agg) value = { aggregate: dAggregate(item.wire) };
      else if (item === undefined) return `the column ${snake(k)} is not given`;
      else return `the column ${snake(k)} is not a column of a source or an aggregate`;
      out.push({ name: snake(k), value });
    }
    const t = q.tree;
    const d: DescribedModel = { table, sources: dSources(t.sources), columns: out };
    if (t.filter) d.filter = dExpr(t.filter);
    if (t.groupBy.length > 0) d.group_by = t.groupBy.map(dColumn);
    if (t.having) d.having = dExpr(t.having);
    if (decl.unmasked) d.unmasked = true;
    return d;
  } catch (e) {
    return (e instanceof RpcError && e.cause instanceof Error ? e.cause : (e as Error)).message;
  }
}
