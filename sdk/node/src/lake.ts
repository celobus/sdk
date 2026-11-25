// The workspace's Data Lake: tables of rows kept for analysis and for BI tools, grouped in layers the plugin names
// (`raw`, `core`, `mart`: the platform names none). A plugin declares its layers and its lake tables, writes them from
// its code (usually a job: `replace`, `append`, `merge`) and reads them with the query builder of its entities.
//
//   export const analyst = role({ title: 'Analyst' });
//   export const raw = layer({ title: 'As it arrived' });
//   export const core = layer({ title: 'Cleaned' });
//
//   /** Stock as the ERP sends it. */
//   export const stockFile = lakeTable({
//     layer: raw, message: 'StockFileRow', description: 'Stock as the ERP sends it.',
//     columns: { loadedOn: t.date(), sku: t.text(), warehouse: t.text(), onHand: t.text() },
//     partition: ['loadedOn'],
//   });
//   export const stock = lakeTable({
//     layer: core,
//     columns: { day: t.date(), sku: t.text(), warehouse: t.text(), onHand: t.decimal(), reserved: t.bigint().optional() },
//     partition: ['day'], key: ['day', 'sku', 'warehouse'],
//     read: [analyst], systemRead: true, export: true,
//   });
//   // plugin({ id: 'retail', roles: { analyst }, layers: { raw, core }, lakeTables: { stockFile, stock }, .. })
//
//   await ctx.lake().table(stock).merge(rows);                                   // in a job's run, or anywhere
//   await ctx.lake().table(stockFile).replace(stockFile.f.loadedOn.eq(day), rows);
//   await ctx.lake().table(stockFile).append(rows, { key: `erp-${day}` });       // the key: required outside a run
//   const low = await ctx.lake().query(stock).where(stock.f.onHand.lt(min)).orderBy(stock.f.onHand.asc()).limit(50).all();
//
// Writes. A plugin writes only its own tables. Each write is one commit: its rows are all there, or none, and a reader
// never sees half of it. A write holds at most 1,000,000 rows (422 `lake_rows_limit`, refused before anything is
// sent; larger loads go partition by partition) in at most 1,000 partitions (422 `lake_partitions_limit`: write fewer
// partitions at a time); the SDK sends them in chunks of at most 10,000 rows and 4 MiB (a row
// over 4 MiB is 422 `bulk_limit`). The writes of one table run one at a time across the workspace (one that waits
// more than 60 s is 503 `lake_busy`); a plugin commits at most 120 writes a minute (`lake_busy` past them: write fewer,
// larger ones) and stages at most two at once; a commit runs at most 50 s (422 `lake_time_limit`, nothing written).
//  * `replace(filter, rows)` swaps the partitions the filter names for `rows`: the filter compares partition columns
//    only, and a row it does not select refuses the write (400 `invalid`). A table without partition columns is
//    replaced whole (`Filter.all()`). Run again, it leaves the same table.
//  * `append(rows, { key })` adds the rows once per write key: a write whose key the table has had within 32 days
//    (longer than a failed run can be retried) adds nothing (`skipped`). In a job's run the key is, by default, the run's id and the number of the append in the
//    run, so an attempt that runs again appends nothing twice; elsewhere the key is required (a type error, and 400
//    `invalid` before anything is sent): a write may answer an error although it was written, and only its key makes
//    running it again safe.
//  * `merge(rows)` writes each row over the row with the same declared key, or adds it; two rows of one write with the
//    same key refuse it. Only on a table with a key.
// A row the table cannot hold refuses the whole write (400 `invalid`): a decimal with more than 20 digits before the
// point or 18 after, a text, bytes or JSON value over 1 MiB.
//
// Reads. `ctx.lake().query(table)` takes the joins, filters, groups, aggregates, time buckets, orders, limits and pages
// of `ctx.query()`, over lake tables only (the plugin's own, or another plugin's it declares in `lakeDependencies`).
// Whoever holds one of a table's read roles reads every row of it; there are no row rules (anyone else: 403
// `permission_denied`). The plugin's system context (`ctx.asSystem(..)`) reads only its own tables declared
// `systemRead` (403 `capability_missing` otherwise). A query runs at most 10 s (422 `lake_time_limit`), within the
// lake's memory (422 `lake_memory_limit`), four of the workspace at once (503 `lake_busy` after a short wait); at most
// 1,000 rows or pages of 100. Never inside a transaction. Decimals read back without trailing zeros (`1.5`). 503
// `unavailable` when the workspace has no lake.
//
// Models. A lake table may be computed by the platform from other tables, when the plugin asks: its `model` is a
// view's query whose row is the table's, over lake tables (the plugin's own its system context reads, or another
// plugin's it declares and whose owner lets models copy it: `exportToModels`), or over the plugin's own entities and
// analytic views, or over the history of one of its entities (never lake tables with entities or views).
// `ctx.lake().refresh(t)` runs it and replaces the whole table with its rows (`{ partition: filter }`: only the
// partitions the filter names), in one commit; the rows never pass through the plugin. A refresh holds at most
// 1,000,000 rows (422 `lake_model_rows_limit`: refresh by partition); a text it parses that does not read as its type
// fails it, naming the column (422 `lake_model_parse`); after the model's declaration changed, a refresh of
// partitions answers how many partitions still hold the earlier declaration's rows (`stalePartitions`): the table mixes
// them until it is 0. A model over entities or views reads them as
// the plugin's system context (its `authz:bypass` of `read` must cover every entity under it: 403
// `capability_missing`), from the workspace's analytic copy, and the read is audited: 503 `unavailable` while the copy
// is not ready or is older than what the table was last computed from (it waits up to 10 s for the copy to catch up:
// retry later); `asOf` says when its data is from.
//
//   export const stock = lakeTable({
//     layer: core, columns: { day: t.date(), sku: t.text(), onHand: t.decimal() }, partition: ['day'],
//     model: (m) => m.from(stockFile)
//       .where(stockFile.f.sku.ne(''))
//       .columns({ day: stockFile.f.loadedOn, sku: stockFile.f.sku, onHand: stockFile.f.onHand.parse('decimal') }),
//   });
//   const r = await ctx.lake().refresh(stock, { partition: stock.f.day.eq(day) });   // r.rows, r.asOf
import { checkCall, noted } from "./keys.js";
import { stringify, encodeMessage, messageFields } from "./codec.js";
import type { DbLink } from "./db.js";
import type { JobInfo, RoleLike } from "./ctx.js";
import { RpcError, internal } from "./errors.js";
import { pascal } from "./names.js";
import { startQuery, type ModelDecl, type ModelStart, type Query } from "./query.js";
import { historyOf, type HistoryDef } from "./history.js";
import { EntityDef, Filter, copyFields, fieldRefs, type FieldRefs, type Infer, type Shape } from "./schema.js";

/** The rows of one write, of one chunk of it, and the bytes of a chunk's rows. */
export const WRITE_ROWS = 1_000_000;
export const CHUNK_ROWS = 10_000;
export const CHUNK_BYTES = 4 << 20;

export interface LayerOptions {
  /** What the layer holds (`As it arrived`). */
  title?: string;
}

/** A layer of the lake; its name is its key in `plugin({ layers })`, in snake_case (at most 32 characters). */
export class LayerDef {
  readonly __layer = true as const;
  constructor(readonly options: LayerOptions) {}
}

/** `export const raw = layer({ title: 'As it arrived' })`, listed in `plugin({ layers: { raw } })`. */
export function layer(options: LayerOptions = {}): LayerDef {
  return noted(new LayerDef(options), "layer", options);
}

export interface LakeTableOptions<C extends Shape> {
  /** The layer the table belongs to (one of `plugin({ layers })`); none for another plugin's table. */
  layer?: LayerDef;
  /** Another plugin's exported table this plugin reads (with `name`), listed in `plugin({ lakeDependencies })`. */
  plugin?: string;
  /** The table's name (snake_case, at most 32 characters); by default its key in `plugin({ lakeTables })`, in snake_case. */
  name?: string;
  /** The row message's name (PascalCase); by default the table's name in PascalCase. */
  message?: string;
  /** What a row is. */
  description?: string;
  /** The columns: scalars, enums and refs, never a list, a message or an encrypted field. */
  columns: C;
  /** The partition columns, in order (at most 4): what a `replace` swaps. Required `string`, `bool`, `int`, `bigint`, `uuid`, `date` or enum columns. */
  partition?: readonly (keyof C & string)[];
  /** The key columns (at most 8): what a `merge` writes over. Partition-like columns, a `timestamp` or a ref. */
  key?: readonly (keyof C & string)[];
  /** The plugin's roles whose holders read the table, every row of it. None: no member reads it. */
  read?: readonly RoleLike[];
  /** The plugin's own system context (`ctx.asSystem(..)`) reads it too. */
  systemRead?: boolean;
  /** Other plugins that declare it (`lakeDependencies`) read it, for callers holding one of its read roles. */
  export?: boolean;
  /**
   * Other plugins that declare it (`lakeDependencies`) may copy it into their own lake tables with a model, where their
   * own roles decide who reads the copy (every row of it). Apart from `export`; a later version that drops it stops
   * further refreshes of the copies (the rows already copied stay).
   */
  exportToModels?: boolean;
  /**
   * How the platform computes the table, when the plugin asks (`ctx.lake().refresh(t)`): a view's query whose columns
   * are the table's, each given once (`m.from(stockFile).where(..).columns({..})`; `.unmasked()` to copy masked
   * columns as they are).
   */
  model?: (m: ModelStart<C>) => ModelDecl<C>;
}

/** @internal The name each own lake table has in its plugin's descriptor, set when the plugin is described. */
export const lakeNames = new WeakMap<object, string>();

/**
 * A lake table: one of the plugin's (`F` is `false`; written with `ctx.lake().table(t)`), or another plugin's it reads
 * (`F` is `true`). Read with `ctx.lake().query(t)`; never with `ctx.db` or `ctx.query`.
 */
export class LakeTableDef<C extends Shape = Shape, F extends boolean = boolean> {
  readonly __lake = true as const;
  /** @internal `true`: another plugin's table. */
  declare readonly __foreign: F;
  readonly columns: C;
  /** Typed handles of the table's columns (`stock.f.day`, `stock.f.day.month()`). */
  readonly f: FieldRefs<LakeRow<LakeTableDef<C, F>>, C>;

  constructor(readonly options: LakeTableOptions<C>) {
    this.columns = copyFields(options.columns);
    // the handles' owner is the table itself: a query names it by its name
    this.f = fieldRefs(this.columns, this) as never;
  }

  /** @internal The table's name in a query: its own name, or `<plugin>.<name>`. */
  sourceName(): string {
    if (this.options.plugin) return `${this.options.plugin}.${this.options.name ?? ""}`;
    const name = lakeNames.get(this) ?? this.options.name;
    if (!name) throw new TypeError("ctx.lake(): the lake table is not in plugin({ lakeTables })");
    return name;
  }

  /** @internal The row message's name. */
  messageName(): string {
    return this.options.message ?? pascal(lakeNames.get(this) ?? this.options.name ?? "");
  }
}

/**
 * Declares a lake table: `lakeTable({ layer: raw, columns: { day: t.date(), sku: t.text() }, partition: ['day'] })`,
 * listed in `plugin({ lakeTables: { stock } })`; with `plugin` and `name`, another plugin's exported table this plugin
 * reads, listed in `plugin({ lakeDependencies: [retailStock] })`.
 */
export function lakeTable<C extends Shape>(options: LakeTableOptions<C> & { plugin: string; name: string }): LakeTableDef<C, true>;
export function lakeTable<C extends Shape>(options: LakeTableOptions<C> & { plugin?: undefined; layer: LayerDef }): LakeTableDef<C, false>;
export function lakeTable<C extends Shape>(options: LakeTableOptions<C>): LakeTableDef<C> {
  return noted(new LakeTableDef(options), "lakeTable", options);
}

/** A row of a lake table: its columns. */
export type LakeRow<T> = T extends LakeTableDef<infer C, any> ? Infer<C> : never;

/** What a write did: the rows it carried, and whether an append was skipped (its key was written before). */
export interface Written {
  rows: number;
  skipped: boolean;
}

export interface AppendOptions {
  /**
   * The write key (at most 512 bytes): a write whose key the table has had within 32 days adds nothing. In a job's
   * run, by default, the run's id and the number of the append in the run.
   */
  key?: string;
}

/** @internal What the lake needs of its context. */
export interface LakeLink {
  db: DbLink;
  job: JobInfo;
  /** The number of the context's next append (1 first), counted per request or run. */
  nextAppend(): number;
}

/**
 * The workspace's Data Lake (`ctx.lake()`): the writes of the plugin's lake tables and their queries. `InRun`: a job's
 * run, where an append has a default key.
 */
export class Lake<InRun extends boolean = boolean> {
  constructor(/** @internal */ private readonly link: LakeLink) {}

  /** The writes of one of the plugin's lake tables. */
  table<C extends Shape>(table: LakeTableDef<C, false>): LakeWriter<C, InRun> {
    return new LakeWriter(this.link, table);
  }

  /**
   * A query of lake tables from the root `table` (the plugin's own, or another plugin's it declares): the builder of
   * `ctx.query()`. Rows are the table's (`all`, `first`, `count`, `page`), or what `select` names.
   */
  query<T extends LakeTableDef<any, any>>(table: T): Query<T> {
    return startQuery(this.link.db, table, "LAKE");
  }

  /**
   * A query of the history of `entity` (`history.ts`): its changes, filtered, grouped and ordered by the entity's
   * kept columns (`ticket.f.status`) and the history's own (`historyOf(ticket).f.validFrom`); `.eachDay()` reads it per
   * day. Its rows without `select` are `HistoryRow<E>`. The plugin's own entity, or another plugin's it declares in
   * `historyDependencies`.
   */
  history<E extends EntityDef<any>>(entity: E): Query<HistoryDef<E>> {
    return startQuery(this.link.db, historyOf(entity), "LAKE");
  }

  /**
   * Runs the model of one of the plugin's lake tables: the platform computes its rows and replaces the whole table
   * with them, or with `{ partition }` (partition columns only, as `replace`'s filter) only the partitions it names,
   * in one commit.
   */
  async refresh<C extends Shape>(table: LakeTableDef<C, false>, options: RefreshOptions<C> = {}): Promise<Refreshed> {
    checkCall("ctx.lake().refresh()", options);
    if (table.options.plugin) {
      throw internal(new Error(`${table.sourceName()} is another plugin's lake table: a plugin refreshes only its own`));
    }
    const refresh: Record<string, unknown> = { requestId: this.link.db.requestId, table: table.sourceName() };
    // a partition filter is sent even when it names every partition: none is the whole table
    if (options.partition !== undefined) refresh.filter = options.partition.node ?? {};
    const r = await this.link.db.syscall({ lakeRefresh: refresh });
    const out = r.lakeRefreshed;
    if (!out) throw internal(new Error("the platform answered a lake refresh with something else"));
    const asOf = Number(out.asOf ?? 0);
    const done: Refreshed = { rows: Number(out.rows ?? 0), stalePartitions: Number(out.stalePartitions ?? 0) };
    if (asOf > 0) done.asOf = new Date(asOf);
    return done;
  }
}

export interface RefreshOptions<C extends Shape> {
  /**
   * Only the partitions this filter names (partition columns only, as `replace`'s). A filter that names no partition
   * (`Filter.all()`) is sent as such and refused (400 `invalid`): a whole refresh is the one without `partition`.
   */
  partition?: Filter<Infer<C>>;
}

/**
 * What a refresh did: the rows it wrote, and when the data of the analytic copy a model over entities or views read is
 * from (none for a model over lake tables; for a model over a history, the moment the lake had the history up to).
 */
export interface Refreshed {
  rows: number;
  asOf?: Date;
  /**
   * After a refresh of partitions: how many partitions the table still holds that an earlier declaration of the model
   * computed (0: every row is this declaration's). The table mixes the two declarations' rows until it is 0, and its
   * readers see it so: a plugin that must not show a mixed table refreshes every partition first.
   */
  stalePartitions: number;
}

type AppendArgs<InRun extends boolean> = InRun extends true
  ? [options?: AppendOptions]
  : [options: AppendOptions & { key: string }];

/** The writes of a lake table (`ctx.lake().table(t)`). */
export class LakeWriter<C extends Shape, InRun extends boolean = boolean> {
  constructor(
    /** @internal */ private readonly link: LakeLink,
    /** @internal */ private readonly table: LakeTableDef<C, false>,
  ) {}

  /**
   * Swaps the partitions `filter` names (partition columns only) for `rows`; a table without partition columns is
   * replaced whole with `Filter.all()`.
   */
  replace(filter: Filter<Infer<C>>, rows: Iterable<Infer<C>>): Promise<Written> {
    return send(this.link, this.table, "REPLACE", filter.node, "", rows);
  }

  /** Writes each row over the row with the same key, or adds it. */
  merge(rows: Iterable<Infer<C>>): Promise<Written> {
    return send(this.link, this.table, "MERGE", undefined, "", rows);
  }

  /**
   * Adds the rows, once per write key: `{ key }`, or in a job's run by default the run's id and the number of the
   * append in the run. Outside a run an append without a key is refused (400 `invalid`) before anything is sent.
   */
  append(rows: Iterable<Infer<C>>, ...args: AppendArgs<InRun>): Promise<Written> {
    const options = (args[0] ?? {}) as AppendOptions;
    try {
      checkCall("ctx.lake().table(t).append()", options);
    } catch (e) {
      return Promise.reject(e);
    }
    // the number is taken in a run whether or not a key is given, so the defaults stay the same in every attempt
    const fallback = this.link.job.isRun ? `${this.link.job.runId}#${this.link.nextAppend()}` : "";
    const key = options.key ?? fallback;
    if (key.trim() === "") {
      return Promise.reject(
        new RpcError(
          400,
          "invalid",
          `an append of ${this.table.sourceName()} outside a job's run needs { key }: a write run again after an error appends once only under its key`,
        ),
      );
    }
    return send(this.link, this.table, "APPEND", undefined, key, rows);
  }
}

/** @internal The rows as chunks of at most `CHUNK_ROWS` rows and `CHUNK_BYTES` bytes, each row its JSON. */
export function chunks(table: LakeTableDef<any, any>, rows: Iterable<unknown>): Buffer[][] {
  const out: Buffer[][] = [[]];
  let size = 0;
  let n = 0;
  for (const r of rows) {
    const b = Buffer.from(stringify(encodeMessage(table, r)));
    if (b.length > CHUNK_BYTES) {
      throw new RpcError(422, "bulk_limit", `a row of ${table.messageName()} is over ${CHUNK_BYTES >> 20} MiB`);
    }
    n += 1;
    if (n > WRITE_ROWS) {
      throw new RpcError(422, "lake_rows_limit", `a write holds at most ${WRITE_ROWS} rows: write the rest in another write`);
    }
    const last = out[out.length - 1]!;
    if (last.length === CHUNK_ROWS || size + b.length > CHUNK_BYTES) {
      out.push([]);
      size = 0;
    }
    size += b.length;
    out[out.length - 1]!.push(b);
  }
  return out;
}

async function send(
  link: LakeLink,
  table: LakeTableDef<any, any>,
  mode: "APPEND" | "REPLACE" | "MERGE",
  filter: unknown,
  key: string,
  rows: Iterable<unknown>,
): Promise<Written> {
  if (table.options.plugin) {
    throw internal(new Error(`${table.sourceName()} is another plugin's lake table: a plugin writes only its own`));
  }
  let parts: Buffer[][];
  try {
    parts = chunks(table, rows);
  } catch (e) {
    if (e instanceof RpcError) throw e;
    throw internal(e);
  }
  const name = table.sourceName();
  let write = "0";
  for (let i = 0; i < parts.length; i++) {
    const first = write === "0";
    const r = await link.db.syscall({
      lakeWrite: {
        requestId: link.db.requestId,
        table: name,
        mode,
        ...(first && filter ? { filter } : {}),
        key: first ? key : "",
        rows: parts[i],
        write,
        last: i + 1 === parts.length,
      },
    });
    const out = r.lakeWritten;
    if (!out) throw internal(new Error("the platform answered a lake write with something else"));
    write = String(out.write);
    if (out.committed || out.skipped) return { rows: Number(out.rows), skipped: !!out.skipped };
  }
  throw internal(new Error("the platform did not end the lake write"));
}

/** @internal The descriptor's names of a table's columns, by their keys. */
export function columnNames(table: LakeTableDef<any, any>): Map<string, string> {
  return new Map(messageFields(table).map((f) => [f.key, f.name]));
}
