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
import { decodeFieldValue, decodeMessage, encodeFieldValue, type FieldErrors } from "./codec.js";
import { Decimal } from "./decimal.js";
import type { DbLink, DbResult, Find, GetOptions } from "./db.js";
import { ReadRepository } from "./db.js";
import { RpcError, internal } from "./errors.js";
import {
  ColCmp,
  EntityDef,
  FieldRef,
  Filter,
  Order,
  PlatformEntity,
  entityNames,
  foreignPlugin,
  type AnyField,
  type ColumnLike,
  type PlatformRow,
  type Row,
  type Stored,
  type UntypedRef,
  type WireFilter,
} from "./schema.js";
import type { SumOf } from "./db.js";

/** An entity a query names: one of the plugin's, a dependency's exported one, the directory's. */
export type Queryable = EntityDef<any> | PlatformEntity<any>;

/** The row type of an entity, as its field handles name it. */
export type RowOf<E> = E extends EntityDef<any> ? Row<E> : E extends PlatformEntity<infer C> ? PlatformRow<C> : never;

/** A ref a join follows between `T` and an entity of the query: a field of `T` (`orderLine.f.orderId` joins
 * `orderLine` to a query that has `order`), or a ref to `T` (`orderLine.f.orderId` joins `order`). */
export type JoinRefOf<T> = FieldRef<RowOf<T>, any, any, any> | FieldRef<any, any, T | UntypedRef, any>;

// --- the wire -----------------------------------------------------------------------------------------------------

/** @internal */ export type WColumn = { source: string; field: string };
/** @internal */ export type WAgg = { fn: string; column?: WColumn };
/** @internal */ export type WTerm = { column?: WColumn; aggregate?: WAgg };
/** @internal */ export type WExpr =
  | { and: { items: WExpr[] } }
  | { or: { items: WExpr[] } }
  | { not: WExpr }
  | { compare: { left: WTerm; op: string; values?: Record<string, unknown>[]; right?: WTerm } }
  | { exists: { source: { entity: string; alias: string }; filter?: WExpr } };

/** The name of an entity in a query: `order`, `crm.customer`, `platform.user`. */
export function canonicalOf(e: object): string {
  if (e instanceof PlatformEntity) return e.ref;
  if (e instanceof EntityDef) {
    const provider = foreignPlugin(e);
    if (provider) return `${provider}.${e.options.name}`;
    const name = entityNames.get(e) ?? e.options.name;
    if (!name) throw internal(new Error("ctx.query(): the entity is not in plugin({ entities })"));
    return name;
  }
  throw internal(new Error("ctx.query(): not an entity"));
}

/** The alias of an entity unless it is given one: its name with `_` for `.` (`crm_customer`). */
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
  return c.queryColumn();
}

function fromFilter(alias: string, f: WireFilter): WExpr {
  if ("and" in f) return { and: { items: f.and.items.map((i) => fromFilter(alias, i)) } };
  if ("or" in f) return { or: { items: f.or.items.map((i) => fromFilter(alias, i)) } };
  if ("not" in f) return { not: fromFilter(alias, f.not) };
  const c = f.condition;
  return { compare: { left: { column: { source: alias, field: c.field } }, op: c.op, values: c.values } };
}

// --- conditions ---------------------------------------------------------------------------------------------------

/** A condition of a query beyond one entity's filter: `exists`, a column against a column, an aggregate's comparison,
 * and the `and`, `or`, `not` of conditions of any entities. */
export class QCond {
  constructor(/** @internal */ readonly expr: WExpr | undefined) {}
}

/** What a query's `where` and `having` take: a field handle's filter (`order.f.status.eq('paid')`) of an entity of the
 * query, a comparison of two columns, a `QCond`. */
export type CondLike = Filter<any> | ColCmp | QCond;

/** @internal */
export function exprOf(c: CondLike): WExpr | undefined {
  if (c instanceof QCond) return c.expr;
  if (c instanceof ColCmp) {
    return { compare: { left: { column: wireColumn(c.left) }, op: c.op, right: { column: wireColumn(c.right) } } };
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

/** An order of a query: a field handle's (`order.f.total.desc()`), an alias column's, an aggregate's. */
export class QOrder {
  constructor(/** @internal */ readonly wire: { term: WTerm; desc: boolean }) {}
}

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

type AnyColumn = FieldRef<any, any, any, any> | AliasCol<any, any, any>;
type ValueOfColumn<C> = C extends FieldRef<any, infer T, any, any> ? T : C extends AliasCol<infer T, any, any> ? T : never;

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

type ColumnIn<X> = X extends any ? FieldRef<X, any, any, any> : never;

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
    if (item instanceof FieldRef || item instanceof AliasCol) {
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

/** A page of a query's rows and the token of the next page (`undefined`: the last). */
export interface QueryPage<T> {
  rows: T[];
  next?: string;
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

class Sender {
  constructor(
    readonly link: DbLink,
    readonly tree: Tree,
    readonly error: string | undefined,
    readonly source: string,
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
      },
    });
  }

  async rows(mode: string, token?: string): Promise<{ rows: unknown[]; next?: string }> {
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
    return next ? { rows, next } : { rows };
  }

  async count(): Promise<number> {
    const r = await this.send("COUNT");
    if (!r.dbCount) throw internal(new Error("the platform answered a count with something else"));
    return Number(r.dbCount.count);
  }
}

type StoredOf<R> = R extends EntityDef<any> ? Stored<R> : never;

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
    if (on.op !== "EQ") return this.fail(`joinOn of ${alias}: the columns are compared with eqCol`);
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

  /** Joins `entity` on two columns of one entity's keys (`a.eqCol(b)`): one of `entity`, one of the query. */
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
    return new Sender(this.link, this.tree, this.error, this.source);
  }

  private plain(): boolean {
    const t = this.tree;
    return (
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
    const { rows } = await this.sender().rows("ALL");
    return rows.map((r) => this.decodeRoot(r)) as never;
  }

  /** The root's first row. */
  async first(): Promise<StoredOf<Root> | undefined> {
    const { rows } = await this.sender().rows("FIRST");
    return rows[0] === undefined ? undefined : (this.decodeRoot(rows[0]) as never);
  }

  /** A page of the root's rows (`limit`, at most 100) after the token of the previous page. */
  async page(token?: string): Promise<QueryPage<StoredOf<Root>>> {
    const { rows, next } = await this.sender().rows("PAGE", token);
    const out: QueryPage<StoredOf<Root>> = { rows: rows.map((r) => this.decodeRoot(r)) as never };
    if (next) out.next = next;
    return out;
  }

  /** How many rows (or groups) the query has; with a filter of the root, those it matches. */
  async count(filter?: Filter<RowOf<Root>>): Promise<number> {
    if (this.plain()) return this.repo!.count(filter as never);
    if (!filter) return this.sender().count();
    // the filter counts these rows only: the query itself is not changed
    const tree = { ...this.tree, filter: combine(this.tree.filter, [filter]) };
    return new Sender(this.link, tree, this.error, this.source).count();
  }

  private decodeRoot(raw: unknown): unknown {
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
    if (!this.repo) throw internal(new Error("query: the directory has no repository reads"));
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
    return this.decode((await this.q.rows("ALL")).rows);
  }

  async first(): Promise<T | undefined> {
    return this.decode((await this.q.rows("FIRST")).rows)[0];
  }

  /** A page (`limit`, at most 100) after the token of the previous page: keyset over the order and the keys. */
  async page(token?: string): Promise<QueryPage<T>> {
    const { rows, next } = await this.q.rows("PAGE", token);
    const out: QueryPage<T> = { rows: this.decode(rows) };
    if (next) out.next = next;
    return out;
  }

  /** How many rows (or groups). */
  async count(): Promise<number> {
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
export function startQuery<E extends Queryable>(link: DbLink, entity: E): Query<E> {
  let repo: ReadRepository<any> | undefined;
  if (entity instanceof EntityDef) {
    const provider = foreignPlugin(entity);
    const name = provider ? entity.options.name : (entityNames.get(entity) ?? entity.options.name);
    if (!name) throw internal(new Error("ctx.query(): the entity is not in plugin({ entities })"));
    repo = new ReadRepository(link, entity, name, provider ?? "");
  } else if (!(entity instanceof PlatformEntity)) {
    throw internal(new Error("ctx.query(): not an entity"));
  }
  return new Query(link, entity, callerSource(), repo);
}

