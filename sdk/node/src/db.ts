// The repository of the plugin's own entities (`ctx.db(entity)`). The platform stores the rows; the SDK
// sends it typed requests: rows in the entity's protobuf JSON,
// filters as a tree of conditions on field handles (`note.f.title.eq('a')`, combined with `and`, `or`, `not`),
// orders (`note.f.title.desc()`). Reads return `Stored<E>`: the row and its system columns. A dependency's entity
// is read through the same calls with `dependency` set (`ctx.query(customer)`), and has no writes.
import { decodeFieldValue, decodeMessage, encodeFieldValue, encodeMessage, messageFields, type FieldErrors } from "./codec.js";
import { Decimal } from "./decimal.js";
import { RpcError, internal } from "./errors.js";
import { snake } from "./names.js";
import type { AnyField, EntityDef, Filter, Included, IncludedRow, Insert, Order, Patch, Row, Stored } from "./schema.js";
import { Filter as FilterClass, navigations } from "./schema.js";

/** @internal What the repository needs of its context: a syscall, the request it is part of, its transaction. */
export interface DbLink {
  syscall(call: Record<string, unknown>): Promise<DbResult>;
  requestId: string;
  tx: string;
  /** the reason of a system context (`ctx.asSystem`); "" for the caller's */
  asSystem: string;
}

/** @internal The members of `SyscallResult` the repository reads. */
export interface DbResult {
  dbRows?: { rows?: Buffer[]; nextPageToken?: string; included?: Buffer[] } | null;
  dbCount?: { count: string | number } | null;
  dbGroups?: { groups?: { key: Buffer; value: Buffer }[] } | null;
  txBegun?: { tx: string } | null;
}

/** How a lock read waits for a row another transaction holds. */
export type LockWait = "wait" | "nowait" | "skipLocked";

/** A row lock of a read in a transaction: `update` (`FOR UPDATE`) or `share` (`FOR SHARE`). */
export type LockStrength = "update" | "share";

/** @internal The lock a read asks for, and the check that it is asked for inside a transaction. */
function wireLock(link: DbLink, strength: LockStrength | undefined, wait: LockWait | undefined): Record<string, unknown> {
  if (!strength) {
    if (wait && wait !== "wait") throw lockRefused("nowait() and skipLocked() go with forUpdate() or forShare()");
    return {};
  }
  if (link.tx === "0") {
    throw lockRefused("a row lock is taken inside a transaction: ctx.transaction(async (tx) => tx.db(e).get(id, { lock }))");
  }
  return {
    lock: {
      strength: strength === "update" ? "FOR_UPDATE" : "FOR_SHARE",
      wait: wait === "nowait" ? "NOWAIT" : wait === "skipLocked" ? "SKIP_LOCKED" : "WAIT",
    },
  };
}

function lockRefused(message: string): RpcError {
  return new RpcError(400, "invalid", message);
}

/** The names an `include` sends (snake_case) and the keys its rows come back under (as given). */
function includeNames(names: readonly string[]): { wire: string[]; keys: string[] } {
  return { wire: names.map((n) => snake(n)), keys: [...names] };
}

/** A page of rows and the token of the next one (`undefined`: the last page). */
export interface Page<R> {
  rows: R[];
  next?: string;
}

function decodeRow<E extends EntityDef<any>>(entity: E, raw: Buffer): Stored<E> {
  let json: Record<string, unknown>;
  try {
    json = JSON.parse(Buffer.from(raw).toString("utf8"));
  } catch (e) {
    throw internal(e);
  }
  const errors: FieldErrors = {};
  const row = decodeMessage(entity, json, "", errors) as Record<string, unknown>;
  if (Object.keys(errors).length > 0) throw internal(new Error(`the platform sent a row this plugin cannot read: ${JSON.stringify(errors)}`));
  row.createdAt = new Date(String(json.createdAt));
  row.updatedAt = new Date(String(json.updatedAt));
  if (typeof json.deletedAt === "string") row.deletedAt = new Date(json.deletedAt);
  if (json.version !== undefined && json.version !== null) row.version = BigInt(String(json.version));
  return row as Stored<E>;
}

/** A row an include brought: decoded as its entity's when the SDK knows it, else its JSON as is. */
function decodeAny(entity: EntityDef<any> | undefined, json: unknown): IncludedRow {
  if (!entity) return json as IncludedRow;
  return decodeRow(entity, Buffer.from(JSON.stringify(json))) as unknown as IncludedRow;
}

/** The target an included name brings, from the navigations the plugin's description recorded. */
function includedEntity(entity: EntityDef<any>, name: string): EntityDef<any> | undefined {
  const nav = navigations.get(entity)?.get(name);
  if (!nav) return undefined;
  if (nav.kind === "inverse") return nav.source;
  return nav.target;
}

function decodeIncluded(entity: EntityDef<any>, raw: Buffer | undefined, names: { wire: string[]; keys: string[] }): Included {
  let json: Record<string, unknown> = {};
  if (raw && raw.length > 0) {
    try {
      json = JSON.parse(Buffer.from(raw).toString("utf8"));
    } catch (e) {
      throw internal(e);
    }
  }
  const out: Record<string, IncludedRow | IncludedRow[] | undefined> = {};
  names.wire.forEach((wire, i) => {
    const key = names.keys[i]!;
    const v = json[wire];
    const target = includedEntity(entity, wire);
    if (Array.isArray(v)) out[key] = v.map((x) => decodeAny(target, x));
    else if (v !== undefined && v !== null) out[key] = decodeAny(target, v);
    else out[key] = navigations.get(entity)?.get(wire)?.kind === "inverse" ? [] : undefined;
  });
  return out;
}

function rowsOf<E extends EntityDef<any>>(
  entity: E,
  r: DbResult,
  include?: { wire: string[]; keys: string[] },
): { rows: Stored<E>[]; next: string } {
  if (!r.dbRows) throw internal(new Error("the platform answered a db call with something else"));
  const included = r.dbRows.included ?? [];
  const rows = (r.dbRows.rows ?? []).map((b, i) => {
    const row = decodeRow(entity, b);
    if (include && include.wire.length > 0) (row as Record<string, unknown>).included = decodeIncluded(entity, included[i], include);
    return row;
  });
  return { rows, next: r.dbRows.nextPageToken ?? "" };
}

function oneOf<E extends EntityDef<any>>(entity: E, r: DbResult, include?: { wire: string[]; keys: string[] }): Stored<E> {
  const row = rowsOf(entity, r, include).rows[0];
  if (!row) throw internal(new Error("the platform answered without the row"));
  return row;
}

/** The id the store replaces on create (`uuid` and `i64` keys); a `string` key is the row's own. */
function placeholder(entity: EntityDef<any>): string {
  return entity.key === "i64" ? "0" : entity.key === "uuid" ? "00000000-0000-0000-0000-000000000000" : "";
}

/** A call that goes when it is awaited, after options such as `expectVersion`. */
export class Pending<T> implements PromiseLike<T> {
  constructor(private readonly run: () => Promise<T>) {}

  then<A = T, B = never>(onOk?: ((v: T) => A | PromiseLike<A>) | null, onErr?: ((e: unknown) => B | PromiseLike<B>) | null): Promise<A | B> {
    return this.run().then(onOk, onErr);
  }
}

/** An update: `await db.update(id, patch).expectVersion(v)`. */
export class Update<E extends EntityDef<any>> extends Pending<Stored<E>> {
  private expected?: bigint;

  constructor(send: (expected?: bigint) => Promise<Stored<E>>) {
    super(() => send(this.expected));
  }

  /** Refuses the update (409 `version_conflict`) unless the row is at this version. */
  expectVersion(v: bigint | number | undefined): this {
    this.expected = v === undefined ? undefined : BigInt(v);
    return this;
  }
}

/** A delete: `await db.delete(id).expectVersion(v)`. */
export class Delete extends Pending<void> {
  private expected?: bigint;

  constructor(send: (expected?: bigint) => Promise<void>) {
    super(() => send(this.expected));
  }

  expectVersion(v: bigint | number | undefined): this {
    this.expected = v === undefined ? undefined : BigInt(v);
    return this;
  }
}

/** A query being built: `db.find(filter).orderBy(..).limit(..)`, read with `all()`, `first()` or `page()`. */
export class Find<E extends EntityDef<any>> {
  private readonly order: { field: string; desc: boolean }[] = [];
  private size = 0;
  private token = "";
  private deleted = false;
  private readonly navs: string[] = [];
  /** @internal set by the lock methods of `TxFind` */
  protected lockStrength?: LockStrength;
  /** @internal */
  protected lockWait?: LockWait;

  constructor(
    private readonly repo: ReadRepository<E>,
    private readonly filter: Filter<Row<E>>,
  ) {}

  /**
   * Brings each row's navigations with it, read under their targets' own rules: a forward one (a ref's `nav`, by
   * default the column's name without `Id`, or the ref column itself) brings the row it names, an inverse one the
   * rows that point at this one. They come back in `row.included`, under the names given here; the rows they bring
   * count toward the 1,000 of a read (422 `query_rows_limit`).
   */
  include(...navs: string[]): this {
    this.navs.push(...navs);
    return this;
  }

  /** Orders by a column; called again, by the next one (ties end by id). */
  orderBy(o: Order<Row<E>>): this {
    this.order.push({ field: o.field, desc: o.desc });
    return this;
  }

  /** Rows per page (100 at most; 100 by default). */
  limit(n: number): this {
    this.size = n;
    return this;
  }

  /** The page after the one whose `next` this is. */
  pageToken(token: string): this {
    this.token = token;
    return this;
  }

  /** Soft-deleted rows too. */
  withDeleted(): this {
    this.deleted = true;
    return this;
  }

  private send(all: boolean, limit: number): Promise<DbResult> {
    const lock = wireLock(this.repo.link, this.lockStrength, this.lockWait);
    return this.repo.send({
      dbFind: {
        ...this.repo.head(),
        ...(this.filter.node ? { filter: this.filter.node } : {}),
        order: this.order,
        limit,
        pageToken: this.token,
        withDeleted: this.deleted,
        all,
        include: includeNames(this.navs).wire,
        ...lock,
      },
    });
  }

  /** Every matching row: more than 1,000 is 422 `query_rows_limit` (page through them instead). */
  async all(): Promise<Stored<E>[]> {
    return rowsOf(this.repo.entity, await this.send(true, 0), includeNames(this.navs)).rows;
  }

  async first(): Promise<Stored<E> | undefined> {
    return rowsOf(this.repo.entity, await this.send(false, 1), includeNames(this.navs)).rows[0];
  }

  async page(): Promise<Page<Stored<E>>> {
    const { rows, next } = rowsOf(this.repo.entity, await this.send(false, this.size), includeNames(this.navs));
    return next ? { rows, next } : { rows };
  }
}

/**
 * A query in a transaction (`tx.db(e).find(f)`), which may lock the rows it reads until the transaction ends:
 * `forUpdate()` or `forShare()`, then `nowait()` (409 `row_locked` at once instead of waiting) or `skipLocked()`
 * (leaves out the rows another transaction holds: a work queue). Waiting counts against the transaction's 5 s.
 */
export class TxFind<E extends EntityDef<any>> extends Find<E> {
  /** Locks the rows read against changes (`FOR UPDATE`). */
  forUpdate(): this {
    this.lockStrength = "update";
    return this;
  }

  /** Locks the rows read against changes by others, letting them lock for share too (`FOR SHARE`). */
  forShare(): this {
    this.lockStrength = "share";
    return this;
  }

  /** Fails at once with 409 `row_locked` when a row is held by another transaction. */
  nowait(): this {
    this.lockWait = "nowait";
    return this;
  }

  /** Leaves out the rows another transaction holds. */
  skipLocked(): this {
    this.lockWait = "skipLocked";
    return this;
  }
}

/**
 * The reads of an entity: of a dependency's (`ctx.query(customer)`, which has no writes), and of the plugin's own
 * (`Repository`, which adds them).
 */
export class ReadRepository<E extends EntityDef<any>> {
  constructor(
    /** @internal */ readonly link: DbLink,
    /** @internal */ readonly entity: E,
    /** @internal */ readonly name: string,
    /** @internal the provider whose entity this reads; "" for the plugin's own */
    readonly dependency: string = "",
  ) {}

  /** @internal */
  head(): { requestId: string; tx: string; entity: string; asSystem: string; dependency?: string } {
    const head = { requestId: this.link.requestId, tx: this.link.tx, entity: this.name, asSystem: this.link.asSystem };
    return this.dependency ? { ...head, dependency: this.dependency } : head;
  }

  /** @internal */
  send(call: Record<string, unknown>): Promise<DbResult> {
    return this.link.syscall(call);
  }

  /**
   * A row by id; 404 `not_found` when there is none, or it is soft-deleted (`withDeleted` reads those too).
   * `include` brings its navigations (`Find.include`).
   */
  async get(id: string, options: GetOptions = {}): Promise<Stored<E>> {
    // a lock outside a transaction (past the types, from JavaScript) is refused by `wireLock` before anything is sent
    const { lock, wait } = options as TxGetOptions;
    return this.getting(id, options, lock, wait);
  }

  /** @internal */
  protected async getting(id: string, options: GetOptions, strength: LockStrength | undefined, wait: LockWait | undefined): Promise<Stored<E>> {
    const names = includeNames(options.include ?? []);
    const lock = wireLock(this.link, strength, wait);
    const r = await this.send({ dbGet: { ...this.head(), id, withDeleted: !!options.withDeleted, include: names.wire, ...lock } });
    return oneOf(this.entity, r, names);
  }

  /**
   * Groups of the rows `where` matches (every row without it; soft-deleted ones only `withDeleted`), read under the
   * entity's rules as any read: by the values of `groupBy` (none: one group of every row), each with its `count` of
   * rows or the `sum` of a number column (int32 and int64 sum to a bigint, a decimal to a `Decimal`, a double to a
   * number; a group of none sums to 0). In the order of the keys (nulls, `undefined` here, last); more than 1,000
   * groups is 422 `query_rows_limit`.
   */
  async aggregate<K extends ColumnKey<E> | undefined = undefined>(
    query: AggregateQuery<E, K> & { count: true },
  ): Promise<(GroupKey<E, K> & { count: number })[]>;
  async aggregate<K extends ColumnKey<E> | undefined = undefined, S extends ColumnKey<E> = ColumnKey<E>>(
    query: AggregateQuery<E, K> & { sum: S },
  ): Promise<(GroupKey<E, K> & { sum: SumOf<StoredValue<E, S>> })[]>;
  async aggregate(query: AggregateQuery<E, any> & { count?: true; sum?: string }): Promise<Record<string, unknown>[]> {
    const handles = this.entity.f as Record<string, { name: string; field: AnyField } | undefined>;
    const handle = (key: string, what: string) => {
      const h = handles[key];
      if (!h) throw internal(new Error(`aggregate: ${key} is not a ${what} of ${this.name}`));
      return h;
    };
    const group = query.groupBy === undefined ? undefined : handle(String(query.groupBy), "column");
    const summed = query.sum === undefined ? undefined : handle(String(query.sum), "column");
    if ((summed === undefined) === (query.count === undefined)) {
      throw internal(new Error("aggregate: give it count: true or sum: '<column>'"));
    }
    const filter = query.where?.node;
    const r = await this.send({
      dbAggregate: {
        ...this.head(),
        ...(filter ? { filter } : {}),
        withDeleted: !!query.withDeleted,
        groupBy: group?.name ?? "",
        measure: summed ? "SUM" : "COUNT",
        field: summed?.name ?? "",
      },
    });
    if (!r.dbGroups) throw internal(new Error("the platform answered an aggregate with something else"));
    return (r.dbGroups.groups ?? []).map((g) => {
      const errors: FieldErrors = {};
      const parse = (b: Buffer): unknown => {
        try {
          return JSON.parse(Buffer.from(b).toString("utf8"));
        } catch (e) {
          throw internal(e);
        }
      };
      const out: Record<string, unknown> = {};
      if (group && query.groupBy !== undefined) {
        const key = decodeFieldValue(group.field, parse(g.key), String(query.groupBy), errors);
        if (Object.keys(errors).length > 0) throw internal(new Error(`the platform sent a group key this plugin cannot read: ${JSON.stringify(errors)}`));
        out[String(query.groupBy)] = key;
      }
      const value = parse(g.value);
      if (summed) out.sum = sumValue(summed.field, value);
      else out.count = Number(value);
      return out;
    });
  }

  /** The rows matching `filter` (every row without one). */
  find(filter: Filter<Row<E>> = FilterClass.all<Row<E>>()): Find<E> {
    return new Find(this, filter);
  }

  async count(filter: Filter<Row<E>> = FilterClass.all<Row<E>>()): Promise<number> {
    const r = await this.send({ dbCount: { ...this.head(), ...(filter.node ? { filter: filter.node } : {}) } });
    if (!r.dbCount) throw internal(new Error("the platform answered a count with something else"));
    return Number(r.dbCount.count);
  }

  async exists(filter: Filter<Row<E>>): Promise<boolean> {
    return (await this.find(filter).first()) !== undefined;
  }

  /** Every row: more than 1,000 is 422 `query_rows_limit` (page through them with `find().page()` instead). */
  async all(): Promise<Stored<E>[]> {
    return this.find().all();
  }
}

/** The options of `get`. */
export interface GetOptions {
  /** A soft-deleted row too. */
  withDeleted?: boolean;
  /** Navigations to bring with the row (`Find.include`). */
  include?: readonly string[];
}

/** The options of `get` in a transaction: a row lock besides. */
export interface TxGetOptions extends GetOptions {
  /** Locks the row until the transaction ends. */
  lock?: LockStrength;
  /** How the lock waits (`wait` by default; `nowait`: 409 `row_locked` at once). */
  wait?: LockWait;
}

/** A column of the entity's row (a TypeScript key), a system column included. */
export type ColumnKey<E> = keyof Stored<E> & string;
type StoredValue<E, K> = K extends keyof Stored<E> ? Exclude<Stored<E>[K], undefined> : never;

/** What an aggregate reads: which rows (`where`, `withDeleted`) and the column it groups by. */
export interface AggregateQuery<E extends EntityDef<any>, K> {
  where?: Filter<Row<E>>;
  withDeleted?: boolean;
  groupBy?: K;
}

/** A group's key: the `groupBy` column's value (`undefined` for the rows without one). */
export type GroupKey<E, K> = K extends string ? { [P in K]: StoredValue<E, P> | undefined } : {};

/** The sum of a column of this type. */
export type SumOf<V> = V extends Decimal ? Decimal : V extends bigint ? bigint : V extends number ? number | bigint : never;

/** A sum as the field's type: int32 and int64 a bigint, a decimal a `Decimal`, a double a number. */
function sumValue(field: AnyField, v: unknown): unknown {
  switch (field.spec.scalar) {
    case "decimal":
      return Decimal.parse(String(v ?? "0"));
    case "double":
      return Number(v ?? 0);
    default:
      return BigInt(String(v ?? "0"));
  }
}

/** The repository of an entity of the plugin: `ctx.db(order)` (in a transaction, `tx.db(order)`). */
export class Repository<E extends EntityDef<any>> extends ReadRepository<E> {
  constructor(link: DbLink, entity: E, name: string) {
    super(link, entity, name);
  }

  /** A new row: the store makes a `uuid` or `i64` key; a `string` key is the row's `id`. */
  async create(row: Insert<E>): Promise<Stored<E>> {
    const [created] = await this.createMany([row]);
    if (!created) throw internal(new Error("the platform answered a create without the row"));
    return created;
  }

  /** New rows, in one statement (at most 1,000). */
  async createMany(rows: Insert<E>[]): Promise<Stored<E>[]> {
    const wire = rows.map((r) => {
      const value = { ...(r as Record<string, unknown>) };
      if (value.id === undefined) value.id = placeholder(this.entity);
      return Buffer.from(JSON.stringify(encodeMessage(this.entity, value)));
    });
    return rowsOf(this.entity, await this.send({ dbCreate: { ...this.head(), rows: wire } })).rows;
  }

  /** Changes the fields `patch` names (`null` clears an optional one); `.expectVersion(v)` checks the row's version. */
  update(id: string, patch: Patch<E>): Update<E> {
    return new Update<E>(async (expected) => {
      const json = this.patchJson(patch, "update");
      const r = await this.send({
        dbUpdate: {
          ...this.head(),
          id,
          patch: Buffer.from(JSON.stringify(json)),
          ...(expected === undefined ? {} : { expectedVersion: expected.toString() }),
        },
      });
      return oneOf(this.entity, r);
    });
  }

  /** Deletes a row (marks it, on a soft-delete entity); `.expectVersion(v)` checks its version. */
  delete(id: string): Delete {
    return this.deleting(id, false);
  }

  /** Removes a row of a soft-delete entity for good. */
  deleteForce(id: string): Delete {
    return this.deleting(id, true);
  }

  /**
   * Changes the fields `patch` names on every row `where` matches that the caller may read (`Filter.all()` for
   * every row), in one statement: each row passes the `update` rule and the guards as `update` does (one that does
   * not refuses them all: 403); `version` and `updatedAt` move on each. More than 1,000 rows is 422 `bulk_limit`,
   * and nothing is written. Answers how many rows changed.
   */
  async updateWhere(where: Filter<Row<E>>, patch: Patch<E>): Promise<number> {
    const json = this.patchJson(patch, "updateWhere");
    const r = await this.send({
      dbUpdateWhere: {
        ...this.head(),
        ...(where.node ? { filter: where.node } : {}),
        patch: Buffer.from(JSON.stringify(json)),
      },
    });
    if (!r.dbCount) throw internal(new Error("the platform answered an updateWhere with something else"));
    return Number(r.dbCount.count);
  }

  /**
   * Creates `row`, or changes the row with the same value of the unique column `on` to it (`id` only for a
   * `string` key): the new row passes the `create` rule, the changed one the `update` rule and the guards.
   */
  async upsert(on: (keyof Row<E> & string), row: Insert<E>): Promise<Stored<E>> {
    const value = { ...(row as Record<string, unknown>) };
    if (value.id === undefined) value.id = placeholder(this.entity);
    const wire = Buffer.from(JSON.stringify(encodeMessage(this.entity, value)));
    return oneOf(this.entity, await this.send({ dbUpsert: { ...this.head(), on: snake(on), row: wire } }));
  }

  /** @internal A patch as the row's JSON writes it. */
  protected patchJson(patch: Patch<E>, what: string): Record<string, unknown> {
    const fields = new Map(messageFields(this.entity).map((f) => [f.key, f]));
    const json: Record<string, unknown> = {};
    for (const [key, v] of Object.entries(patch as Record<string, unknown>)) {
      const f = fields.get(key);
      if (!f || key === "id") throw internal(new Error(`${what}: ${key} is not a column of ${this.name}`));
      json[f.json] = v === null ? null : encodeColumn(f.field, v, key);
    }
    return json;
  }

  /** Brings a soft-deleted row back. */
  async restore(id: string): Promise<Stored<E>> {
    return oneOf(this.entity, await this.send({ dbRestore: { ...this.head(), id } }));
  }

  private deleting(id: string, force: boolean): Delete {
    return new Delete(async (expected) => {
      await this.send({
        dbDelete: { ...this.head(), id, force, ...(expected === undefined ? {} : { expectedVersion: expected.toString() }) },
      });
    });
  }
}

/**
 * The repository of an entity in a transaction (`tx.db(order)`): reads may lock their rows until it ends, with
 * `get(id, { lock: 'update' })` and `find(f).forUpdate()`; outside a transaction they are refused (400 `invalid`).
 */
export class TxRepository<E extends EntityDef<any>> extends Repository<E> {
  override async get(id: string, options: TxGetOptions = {}): Promise<Stored<E>> {
    return this.getting(id, options, options.lock, options.wait);
  }

  override find(filter: Filter<Row<E>> = FilterClass.all<Row<E>>()): TxFind<E> {
    return new TxFind(this, filter);
  }
}

/** One column's value as the row's JSON writes it. */
function encodeColumn(field: AnyField, v: unknown, path: string): unknown {
  if (field.spec.kind === "array") {
    if (!Array.isArray(v)) throw internal(new Error(`update: ${path} is a list`));
    return v.map((x, i) => encodeFieldValue(field.spec.inner!, x, `${path}[${i}]`));
  }
  return encodeFieldValue(field, v, path);
}
