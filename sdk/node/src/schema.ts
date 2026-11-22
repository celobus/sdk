// Declarations of data: field builders (`t.*`), messages, enums and entities, with the TypeScript types they
// infer. Builders are immutable: `.optional()` and `.describe()` return a new field, so one enum or message may
// be used in several places with different modifiers.
import { encodeFieldValue } from "./codec.js";
import type { Decimal } from "./decimal.js";
import { snake } from "./names.js";

/** The descriptor's scalar types. */
export type ScalarType =
  | "string"
  | "bool"
  | "int32"
  | "int64"
  | "double"
  | "decimal"
  | "uuid"
  | "timestamp"
  | "date"
  | "bytes"
  | "json";

export type KeyType = "uuid" | "i64" | "string";

/**
 * What a ref points at: an entity of this plugin (or a function returning it, for an entity declared later),
 * the directory (`platform.user`, `platform.group` from `@celobus/sdk/platform`), or the canonical name of
 * another plugin's entity (`'customers.customer'`) until dependencies have generated clients.
 */
export type RefTarget = EntityDef<any> | (() => EntityDef<any>) | PlatformEntity | `${string}.${string}`;

/** @internal What a field holds. */
export interface FieldSpec {
  kind: "scalar" | "message" | "enum" | "ref" | "array";
  scalar?: ScalarType;
  message?: MessageDef<Shape>;
  enumDef?: EnumDef<string>;
  ref?: RefTarget;
  inner?: AnyField;
}

/** @internal The column modifiers of an entity's field. */
export interface ColumnSpec {
  unique?: boolean;
  index?: boolean;
  encrypted?: boolean;
  /** the text search language of a `searchable` column */
  searchable?: string;
  /** a ref's forward navigation name (default: the column's name without `_id`) */
  nav?: string;
  /** a ref's navigation on its target: the rows that point at a row of it */
  inverse?: string;
  /** what a ref does when its target is deleted for good */
  onDelete?: OnDelete;
}

/**
 * What a ref does when the row it names is deleted for good (`t.ref(target, { onDelete })`): within the plugin
 * `restrict` (the default: the target is not deleted while rows refer to it), `cascade` (they are deleted with it),
 * `setNull` (an optional ref is cleared), `noAction` (as `restrict`, checked at the end of the statement); to
 * another plugin's entity or the directory `keep` (the default: the id stays) or, onto the directory, `setNull`.
 * A soft delete of the target does nothing: the rows go (or are cleared) when it is removed for good.
 */
export type OnDelete = "restrict" | "cascade" | "setNull" | "noAction" | "keep";

/** The options of a ref (`t.ref(target, { nav, inverse, onDelete })`). */
export interface RefOptions {
  /** The forward navigation's name (`include('customer')`); by default the column's name without `Id`/`_id`. */
  nav?: string;
  /** A navigation on the target (an entity of this plugin): the rows of this entity that point at a row of it. */
  inverse?: string;
  /** What the ref does when its target is deleted for good ({@link OnDelete}). */
  onDelete?: OnDelete;
}

/**
 * What a ref field's target is at the type level when it is not an entity value (a thunk, another plugin's entity
 * named by a string): a join along it is checked by the platform alone.
 */
export declare const untypedRef: unique symbol;
export type UntypedRef = typeof untypedRef;

/** A field of a message or an entity: its type `T`, whether it may be absent (`O`), a ref's target (`R`). */
export class Field<T, O extends boolean = false, R = unknown> {
  declare readonly __type: T;
  declare readonly __optional: O;
  declare readonly __target: R;

  constructor(
    /** @internal */ readonly spec: FieldSpec,
    /** @internal */ readonly isOptional: boolean = false,
    /** @internal */ readonly description: string = "",
    /** @internal */ readonly column: ColumnSpec = {},
  ) {}

  /** The field may be absent (`undefined` in TypeScript, omitted on the wire). Fields are required otherwise. */
  optional(): Field<T, true, R> {
    return new Field<T, true, R>(this.spec, true, this.description, this.column);
  }

  /** The field's description (descriptor, documentation, MCP). */
  describe(text: string): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, text, this.column);
  }

  /** An entity column whose values are unique (among the rows that are not deleted, on a soft-delete entity). */
  unique(): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, this.description, { ...this.column, unique: true });
  }

  /** An entity column with an index of its own (refs have one without it). */
  index(): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, this.description, { ...this.column, index: true });
  }

  /**
   * A text column searched by full text (`note.f.title.search('words')`), in a PostgreSQL text search language
   * (`simple` by default: no stemming).
   */
  searchable(language = "simple"): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, this.description, { ...this.column, searchable: language });
  }

  /**
   * An entity column the platform keeps encrypted at rest with the workspace's key (never unique, indexed, filtered,
   * sorted, a ref, the id, the label or read by declared SQL).
   */
  encrypted(): Field<T, O> {
    return new Field<T, O>(this.spec, this.isOptional, this.description, { ...this.column, encrypted: true });
  }

  /** @internal A use of this field with its own identity (messages and entities copy their fields). */
  use(): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, this.description, this.column);
  }
}

export type AnyField = Field<any, boolean, any>;
export type Shape = { [key: string]: AnyField };

/** The TypeScript type of a field's value. */
export type ValueOf<F> = F extends Field<infer T, any, any> ? T : never;

/** A ref field's target (`UntypedRef` for a thunk or a name; `unknown` for any other field). */
export type TargetOf<F> = F extends Field<any, any, infer R> ? R : unknown;

/** Whether a field may be absent. */
export type OptionalOf<F> = F extends Field<any, infer O, any> ? O : false;

type Simplify<T> = { [K in keyof T]: T[K] } & {};
type OptionalKeys<S> = { [K in keyof S]: S[K] extends Field<any, true, any> ? K : never }[keyof S];
type RequiredKeys<S> = Exclude<keyof S, OptionalKeys<S>>;

/** The object type of a message with these fields: required fields present, optional ones may be absent. */
export type Infer<S extends Shape> = Simplify<
  { [K in RequiredKeys<S>]: ValueOf<S[K]> } & { [K in OptionalKeys<S>]?: ValueOf<S[K]> | undefined }
>;

/**
 * A typed handle of a message's field, for UI declarations: `order.f.status` is a `FieldRef<Order, OrderStatus>`
 * (`M` is the message's value type, `T` the field's value type without `undefined`). A handle of another message
 * does not fit where a field of `M` is expected, so a wrong field name or message is a type error.
 */
export class FieldRef<M, T, R = unknown, O extends boolean = boolean> {
  declare readonly __message: (m: M) => M;
  declare readonly __value: T;
  /** a ref's target (`UntypedRef`: not known to the types) */
  declare readonly __target: R;
  /** whether the column may be null */
  declare readonly __optional: O;

  constructor(
    /** The descriptor's (snake_case) field name. */
    readonly name: string,
    /** @internal The field's declaration. */
    readonly field: AnyField,
    /** @internal The entity whose field this is (a query names its alias). */
    readonly owner?: object,
  ) {}

  private compare(op: string, values: readonly T[]): Filter<M> {
    const inner = this.field.spec.kind === "array" ? this.field.spec.inner! : this.field;
    return new Filter<M>(
      { condition: { field: this.name, op, values: values.map((v) => dbValue(encodeFieldValue(inner, v, this.name))) } },
      this.owner,
    );
  }

  /** @internal A comparison with another column (`eqCol` and the others). */
  private cmpCol(op: string, other: FieldRef<any, any, any, any> | ColumnLike): ColCmp {
    return new ColCmp(this, op, other);
  }

  /**
   * In a query (`ctx.query(..)`): this column equals another of the same type (a key and a ref to its entity, two
   * refs to one entity, two strings); also what `joinOn` joins on.
   */
  eqCol(other: ColumnOf<T, R>): ColCmp {
    return this.cmpCol("EQ", other);
  }
  /** Not equal (a null is not equal to a value). */
  neCol(other: ColumnOf<T, R>): ColCmp {
    return this.cmpCol("NE", other);
  }
  ltCol(other: ColumnOf<T, R>): ColCmp {
    return this.cmpCol("LT", other);
  }
  leCol(other: ColumnOf<T, R>): ColCmp {
    return this.cmpCol("LE", other);
  }
  gtCol(other: ColumnOf<T, R>): ColCmp {
    return this.cmpCol("GT", other);
  }
  geCol(other: ColumnOf<T, R>): ColCmp {
    return this.cmpCol("GE", other);
  }

  /** Rows whose field equals `v` (`ctx.db(order).find(order.f.status.eq('paid'))`). */
  eq(v: T): Filter<M> {
    return this.compare("EQ", [v]);
  }
  /** Not equal (a null is not equal to a value). */
  ne(v: T): Filter<M> {
    return this.compare("NE", [v]);
  }
  lt(v: T): Filter<M> {
    return this.compare("LT", [v]);
  }
  le(v: T): Filter<M> {
    return this.compare("LE", [v]);
  }
  gt(v: T): Filter<M> {
    return this.compare("GT", [v]);
  }
  ge(v: T): Filter<M> {
    return this.compare("GE", [v]);
  }
  /** Any of the values. */
  in(values: readonly T[]): Filter<M> {
    return this.compare("IN", values);
  }
  isNull(): Filter<M> {
    return this.compare("IS_NULL", []);
  }
  isNotNull(): Filter<M> {
    return this.compare("IS_NOT_NULL", []);
  }
  /**
   * Full-text search of a `searchable` column: the words as a web search reads them (`"a phrase"`, `or`, `-word`),
   * in the column's language. The platform refuses it (400 `invalid`) on a column that is not searchable.
   */
  search(this: FieldRef<M, string, any, any>, words: string): Filter<M> {
    return new Filter<M>({ condition: { field: this.name, op: "SEARCH", values: [{ stringValue: words }] } }, this.owner);
  }
  /** A case-insensitive substring of a text column, taken literally (`%` and `_` are no wildcards). */
  contains(this: FieldRef<M, string, any, any>, part: string): Filter<M> {
    return new Filter<M>({ condition: { field: this.name, op: "CONTAINS", values: [{ stringValue: part }] } }, this.owner);
  }
  asc(): Order<M> {
    return new Order<M>(this.name, false, this.owner);
  }
  desc(): Order<M> {
    return new Order<M>(this.name, true, this.owner);
  }
}

/** @internal A column of a query that is not a field handle: an alias's column (`query.ts`). */
export interface ColumnLike {
  /** @internal */ readonly queryColumn: () => { source: string; field: string };
  /** @internal */ readonly field: AnyField;
}

/** A column a column of type `T` (a ref to `R`) compares with: a field handle or an alias's column of that type. */
export type ColumnOf<T, R> =
  | FieldRef<any, T, R extends UntypedRef ? any : R | UntypedRef, any>
  | (ColumnLike & { readonly __value: T });

/** Two columns compared (`a.eqCol(b)`): a query's condition, and what `joinOn` joins on. */
export class ColCmp {
  constructor(
    /** @internal */ readonly left: FieldRef<any, any, any, any> | ColumnLike,
    /** @internal */ readonly op: string,
    /** @internal */ readonly right: FieldRef<any, any, any, any> | ColumnLike,
  ) {}
}

/** @internal A filter tree as the session carries it (`DbFilter`). */
export type WireFilter =
  | { and: { items: WireFilter[] } }
  | { or: { items: WireFilter[] } }
  | { not: WireFilter }
  | { condition: { field: string; op: string; values: Record<string, unknown>[] } };

/** A filter parameter (`DbValue`): the platform converts it to the column's type. */
function dbValue(v: unknown): Record<string, unknown> {
  if (typeof v === "boolean") return { boolValue: v };
  if (typeof v === "number") return Number.isInteger(v) ? { intValue: String(v) } : { doubleValue: v };
  if (typeof v === "string") return { stringValue: v };
  return { stringValue: JSON.stringify(v) };
}

/** A condition on the rows of `M`, from field handles; combined with `and`, `or`, `not`. */
export class Filter<M> {
  declare readonly __row: (m: M) => M;

  constructor(
    /** @internal */ readonly node: WireFilter | undefined,
    /** @internal The entity whose columns it tests (a query names its alias). */
    readonly source?: object,
  ) {}

  /** Every row. */
  static all<M>(): Filter<M> {
    return new Filter<M>(undefined);
  }

  and(...others: Filter<M>[]): Filter<M> {
    return and(this, ...others);
  }

  or(...others: Filter<M>[]): Filter<M> {
    return or(this, ...others);
  }

  not(): Filter<M> {
    return not(this);
  }
}

/** All of the conditions. */
export function and<M>(...filters: Filter<M>[]): Filter<M> {
  const items = filters.map((f) => f.node).filter((n): n is WireFilter => n !== undefined);
  const source = filters.find((f) => f.source)?.source;
  if (items.length === 0) return new Filter<M>(undefined, source);
  return new Filter<M>(items.length === 1 ? items[0] : { and: { items } }, source);
}

/** Any of the conditions. */
export function or<M>(...filters: Filter<M>[]): Filter<M> {
  const source = filters.find((f) => f.source)?.source;
  if (filters.some((f) => f.node === undefined)) return new Filter<M>(undefined, source);
  const items = filters.map((f) => f.node as WireFilter);
  return new Filter<M>(items.length === 1 ? items[0] : { or: { items } }, source);
}

/** Not the condition. */
export function not<M>(filter: Filter<M>): Filter<M> {
  return new Filter<M>(filter.node === undefined ? { or: { items: [] } } : { not: filter.node }, filter.source);
}

/** A column the rows are ordered by. */
export class Order<M> {
  declare readonly __row: (m: M) => M;

  constructor(
    readonly field: string,
    readonly desc: boolean,
    /** @internal The entity whose column it is (a query names its alias). */
    readonly source?: object,
  ) {}
}

/** The field handles of a message with these fields. */
export type FieldRefs<M, S extends Shape> = {
  readonly [K in keyof S & string]: FieldRef<M, ValueOf<S[K]>, TargetOf<S[K]>, OptionalOf<S[K]>>;
};

function fieldRefs(shape: Shape, owner?: object): Record<string, FieldRef<unknown, unknown>> {
  const out: Record<string, FieldRef<unknown, unknown>> = {};
  for (const [key, field] of Object.entries(shape)) out[key] = new FieldRef(snake(key), field, owner);
  return out;
}

/** @internal The name each entity has in its plugin's descriptor, set when the plugin is described. */
export const entityNames = new WeakMap<object, string>();

/** @internal A navigation of an entity: the target it brings (unknown for the directory or a named ref). */
export type NavInfo =
  | { kind: "forward"; target: EntityDef<any> | undefined }
  | { kind: "inverse"; source: EntityDef<any> };

/** @internal Each entity's navigations by their (snake_case) names, set when the plugin is described. */
export const navigations = new WeakMap<object, Map<string, NavInfo>>();

/** Maps fields to the descriptor path they were declared under, for `invalid(..).field(handle, ..)`. */
export const fieldKeys = new WeakMap<AnyField, string>();

function copyFields<S extends Shape>(shape: S): S {
  const out: Shape = {};
  for (const [key, field] of Object.entries(shape)) {
    if (!(field instanceof Field)) throw new TypeError(`field ${key}: not a t.* builder`);
    const copy = field.use();
    fieldKeys.set(copy, key);
    out[key] = copy;
  }
  return out as S;
}

/** A named message (`t.message('HelloRequest', {...})`), usable as an endpoint's input or output or as a field. */
export class MessageDef<S extends Shape> extends Field<Infer<S>, false> {
  /** The fields as declared, each a handle for `invalid(..).field(message.fields.name, ..)`. */
  readonly fields: S;
  /** Typed field handles for UI declarations (`order.f.status`). */
  readonly f: FieldRefs<Infer<S>, S>;

  constructor(
    readonly name: string,
    shape: S,
    /** the message's own description */
    readonly messageDescription: string = "",
    /**
     * Fields at their protobuf default (`""`, `0`, `false`, an empty list, a message whose JSON is `{}`) are left out
     * of its JSON; an enum and an optional field with a value are still written (`t.message(.., { omitDefaults: true })`).
     */
    readonly omitDefaults: boolean = false,
  ) {
    const spec: FieldSpec = { kind: "message" };
    super(spec);
    spec.message = this as unknown as MessageDef<Shape>;
    this.fields = copyFields(shape);
    this.f = fieldRefs(this.fields) as FieldRefs<Infer<S>, S>;
  }
}

/** A named enum (`t.enum('OrderStatus', ['placed', 'paid'])`); its values are strings on the wire and in code. */
export class EnumDef<V extends string> extends Field<V, false> {
  constructor(
    readonly name: string,
    readonly values: readonly V[],
  ) {
    const spec: FieldSpec = { kind: "enum" };
    super(spec);
    spec.enumDef = this as unknown as EnumDef<string>;
  }
}

/**
 * An entity of the directory: `platform.user`, `platform.group` (refs name them), and for queries
 * `platform.groupMember`, `platform.roleAssignment` (`@celobus/sdk/platform`). `f` are its columns' handles.
 */
export class PlatformEntity<C extends Shape = Shape> {
  /** Typed handles of the columns a query names (`platform.user.f.displayName`). */
  readonly f: FieldRefs<PlatformRow<C>, C>;
  /** @internal */ readonly columns: C;

  constructor(
    readonly ref: "platform.user" | "platform.group" | "platform.group_member" | "platform.role_assignment",
    columns: C = {} as C,
  ) {
    this.columns = copyFields(columns);
    this.f = fieldRefs(this.columns, this) as never;
  }
}

/** A row of a directory entity, as a query reads it. */
export type PlatformRow<C extends Shape> = Simplify<Infer<C>>;

export interface EntityOptions<C extends Shape> {
  /**
   * Another plugin's entity, which this plugin reads with `ctx.query(..)` (what `celobus plugin add` generates for a
   * dependency's exported entity, with `name`); left out for an entity of this plugin.
   */
  plugin?: string;
  /** The entity's name; by default its key in `plugin({ entities })`, in snake_case. */
  name?: string;
  description?: string;
  /** Who makes the primary key `id`: `uuid` (default) or `i64` by the store, `string` by the author. */
  key?: KeyType;
  softDelete?: boolean;
  version?: boolean;
  appendOnly?: boolean;
  /** The column that names a row; by default `name`, `title`, `number`, else `id`. */
  label?: (keyof C & string) | "id";
  /** The columns; `id` is implied by `key`. */
  columns: C;
  /**
   * Indexes over several columns, each its columns in order (`'createdAt desc'` for a descending one; system columns
   * may be named): `[['ownerId', 'createdAt desc']]`, or `{ columns: [...], unique: true }` for a unique one.
   */
  indexes?: readonly (readonly IndexColumn<C>[] | { columns: readonly IndexColumn<C>[]; unique?: boolean })[];
  /**
   * Who reads, creates, changes and deletes the rows (`./access.ts`); without it no one but a system context does.
   */
  access?: (a: import("./access.js").AccessBuilder) => void;
}

type SystemColumn = "id" | "createdAt" | "updatedAt" | "deletedAt" | "version";
/** A column of an index: a column's key or a system column, optionally followed by ` desc` or ` asc`. */
export type IndexColumn<C> = (keyof C & string) | SystemColumn | `${(keyof C & string) | SystemColumn} ${"asc" | "desc"}`;

/**
 * A dependency's entity this plugin reads (`ctx.query(customers.customer)`), as `celobus plugin add` generates it:
 * the provider's plugin id, the entity's name there, its key and system columns, and the columns this plugin reads.
 */
export interface ForeignEntityOptions<C extends Shape> {
  /** The provider's plugin id. */
  plugin: string;
  /** The entity's name in the provider (snake_case). */
  name: string;
  description?: string;
  key?: KeyType;
  softDelete?: boolean;
  version?: boolean;
  columns: C;
}

/**
 * An entity: one of the plugin's (`ctx.db(e)`), or a dependency's it reads (`ctx.query(e)`; `F` is `true`).
 */
export class EntityDef<C extends Shape, F extends boolean = boolean> {
  /** @internal `true`: another plugin's entity (`ctx.query`), never this plugin's (`ctx.db`). */
  declare readonly __foreign: F;
  readonly columns: C;
  readonly key: KeyType;
  /**
   * Typed field handles of the entity's message for UI declarations and filters (`note.f.id`, `note.f.title`), and of
   * its system columns (`note.f.createdAt`; `deletedAt` and `version` when it has them).
   */
  readonly f: FieldRefs<Row<EntityDef<C>>, C & { id: Field<string, false, EntityDef<C, F>> } & SystemFields>;

  constructor(readonly options: EntityOptions<C>) {
    this.columns = copyFields(options.columns);
    this.key = options.key ?? "uuid";
    const system: Shape = { createdAt: scalar<Date>("timestamp"), updatedAt: scalar<Date>("timestamp") };
    if (options.softDelete) system.deletedAt = scalar<Date>("timestamp").optional();
    if (options.version) system.version = scalar<bigint>("int64");
    this.f = fieldRefs({ id: new Field<string, false>({ kind: "ref", ref: this }), ...this.columns, ...system }, this) as never;
  }
}

type SystemFields = {
  createdAt: Field<Date, false>;
  updatedAt: Field<Date, false>;
  deletedAt: Field<Date, true>;
  version: Field<bigint, false>;
};

/**
 * Declares an entity: `entity({ key: 'i64', version: true, label: 'title', columns: { title: t.text() } })`; with
 * `plugin` and `name`, a dependency's entity this plugin reads (generated by `celobus plugin add`).
 */
export function entity<C extends Shape>(options: ForeignEntityOptions<C>): EntityDef<C, true>;
export function entity<C extends Shape>(options: EntityOptions<C> & { plugin?: undefined }): EntityDef<C, false>;
export function entity<C extends Shape>(options: EntityOptions<C>): EntityDef<C> {
  return new EntityDef(options);
}

/** @internal The provider of a dependency's entity; `undefined` for an entity of this plugin. */
export function foreignPlugin(e: EntityDef<any>): string | undefined {
  return e.options.plugin || undefined;
}

/** A row of an entity, as endpoints answer it: `id` and the columns. */
export type Row<E> = E extends EntityDef<infer C> ? Simplify<{ id: string } & Infer<C>> : never;

/** A row as the repository returns it: the row and its system columns. */
export type Stored<E> = Simplify<
  Row<E> & {
    createdAt: Date;
    updatedAt: Date;
    /** When a soft-delete entity's row was deleted (reads `withDeleted` see such rows). */
    deletedAt?: Date;
    /** On a versioned entity: what `expectVersion` checks. */
    version?: bigint;
    /** What `include(..)` brought, by the names it was given (absent when the read included nothing). */
    included?: Included;
  }
>;

/** A row an `include` brought: the target's stored row (`id`, its columns, the system columns). */
export interface IncludedRow {
  id: string;
  createdAt: Date;
  updatedAt: Date;
  deletedAt?: Date;
  version?: bigint;
  [column: string]: unknown;
}

/**
 * What an `include` brought, by the name it was given: a forward navigation's row (`undefined` when the ref is
 * empty, or names a row the caller may not read) or an inverse navigation's rows (in id order).
 */
export interface Included {
  readonly [nav: string]: IncludedRow | IncludedRow[] | undefined;
}

/** What `create` takes: the columns, and `id` only for a `string` key (the store makes the others). */
export type Insert<E> = E extends EntityDef<infer C> ? Simplify<{ id?: string } & Infer<C>> : never;

/** What `update` takes: the columns it changes; `null` clears an optional one. */
export type Patch<E> = E extends EntityDef<infer C> ? { [K in keyof Infer<C>]?: Infer<C>[K] | null } : never;

function scalar<T>(type: ScalarType): Field<T, false> {
  return new Field<T, false>({ kind: "scalar", scalar: type });
}

/** Field builders. Each maps to one descriptor type; the comment says the TypeScript and the wire type. */
export const t = {
  /** string */
  text: () => scalar<string>("string"),
  /** string (the length is not checked) */
  varchar: (_length?: number) => scalar<string>("string"),
  /** boolean */
  bool: () => scalar<boolean>("bool"),
  /** int32: a JS number, an integer */
  int: () => scalar<number>("int32"),
  /** int64: a JS bigint; a string on the wire */
  bigint: () => scalar<bigint>("int64"),
  /** double: a JS number */
  double: () => scalar<number>("double"),
  /** decimal: the SDK's `Decimal`; a string on the wire (precision and scale are not checked) */
  decimal: (_precision?: number, _scale?: number) => scalar<Decimal>("decimal"),
  /** uuid: a string in canonical form */
  uuid: () => scalar<string>("uuid"),
  /** timestamp: a JS `Date`; an RFC 3339 string on the wire */
  timestamp: () => scalar<Date>("timestamp"),
  /** date: a `YYYY-MM-DD` string */
  date: () => scalar<string>("date"),
  /** bytes: a `Uint8Array`; base64 on the wire */
  bytes: () => scalar<Uint8Array>("bytes"),
  /** json: any JSON value, typed as `T`; the value itself on the wire */
  json: <T = unknown>() => scalar<T>("json"),
  /** repeated: an array of the inner field's type (never absent: an empty array) */
  array: <F extends AnyField>(inner: F): Field<ValueOf<F>[], false> =>
    new Field<ValueOf<F>[], false>({ kind: "array", inner }),
  /**
   * A reference to a row: its id as a string, whatever the target's key type (an `i64` key is its decimal
   * string, as on the wire).
   */
  ref: refBuilder,
  /**
   * A named message; also usable as a field (a nested message). `omitDefaults`: its fields at their protobuf default
   * (`""`, `0`, `false`, an empty list, a message whose JSON is `{}`) are left out of the JSON it is written as (an
   * enum, and an optional field that has a value, are still written), which trims the views a screen reads often.
   */
  message: <S extends Shape>(name: string, fields: S, options?: { description?: string; omitDefaults?: boolean }): MessageDef<S> =>
    new MessageDef(name, fields, options?.description ?? "", options?.omitDefaults ?? false),
  /** `t.enum('Name', [...values])` declares an enum; `t.enum(def)` is a field of it. */
  enum: enumBuilder,
};

/**
 * A ref to an entity value (`t.ref(customer)`, `t.ref(platform.user)`) knows its target at the type level, so a query
 * joins along it only to that entity; a ref by a thunk or a name does not.
 */
function refBuilder<E extends EntityDef<any> | PlatformEntity<any>>(target: E, options?: RefOptions): Field<string, false, E>;
function refBuilder(target: RefTarget, options?: RefOptions): Field<string, false, UntypedRef>;
function refBuilder(target: RefTarget, options: RefOptions = {}): Field<string, false, any> {
  const column: ColumnSpec = {};
  if (options.nav) column.nav = options.nav;
  if (options.inverse) column.inverse = options.inverse;
  if (options.onDelete) column.onDelete = options.onDelete;
  return new Field<string, false>({ kind: "ref", ref: target }, false, "", column);
}

function enumBuilder<const V extends readonly string[]>(name: string, values: V): EnumDef<V[number]>;
function enumBuilder<V extends string>(def: EnumDef<V>): Field<V, false>;
function enumBuilder(nameOrDef: string | EnumDef<string>, values?: readonly string[]): AnyField {
  if (nameOrDef instanceof EnumDef) return nameOrDef.use();
  return new EnumDef(nameOrDef, [...(values ?? [])]);
}

/** The value type of an endpoint's input or output declaration. */
export type IoType = MessageDef<any> | EntityDef<any> | Shape;
export type IoValue<I> =
  I extends MessageDef<infer S> ? Infer<S> : I extends EntityDef<any> ? Row<I> : I extends Shape ? Infer<I> : never;
