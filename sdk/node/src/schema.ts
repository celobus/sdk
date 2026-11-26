// Declarations of data: field builders (`t.*`), messages, enums and entities, with the TypeScript types they
// infer. Builders are immutable: `.optional()` and `.describe()` return a new field, so one enum or message may
// be used in several places with different modifiers.
import { noted, optionProblems } from "./keys.js";
import { encodeFieldValue } from "./codec.js";
import type { Decimal } from "./decimal.js";
import type { Secret } from "./secret.js";
import { snake } from "./names.js";
import type { RoleLike } from "./ctx.js";
import type { LayerDef } from "./lake.js";

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
  | "json"
  | "file"
  | "secret";

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
  /** a file field's options (`t.file({ maxSize, accept })`) */
  file?: FileSpec;
}

/** @internal The options of a file field as declared: the size in bytes, or why it is not one. */
export interface FileSpec {
  maxSize?: number;
  accept?: string[];
  /** the declared `maxSize` that is not a size */
  badSize?: string;
}

/**
 * A file: a typed value of a message field, an entity column or a job payload (`t.file()`, `t.array(t.file())`). It
 * names the file, never holds its bytes: read and write them with `ctx.files`. In JSON `{"id", "name", "type",
 * "size"}`, `size` a string like every 64-bit integer (a number is read too).
 *
 * A file comes from a caller (an endpoint's file field, uploaded by a screen, the CLI or another client within the
 * field's `maxSize` and `accept`), from another plugin (in a call or its answer), or from the plugin's own code
 * (`ctx.files.create`). It lives while a row, a pending job, a schedule or a live agent run of the plugin holds it; a
 * file nothing holds is collected after a grace period. A file in an endpoint's answer reaches the client with a
 * short-lived URL to read it.
 */
export interface File {
  /** The file's id. */
  id: string;
  /** Its name as uploaded or made (`report.csv`). */
  name: string;
  /** Its MIME type (`text/csv`); `application/octet-stream` when unknown. */
  type: string;
  /** Its size in bytes. */
  size: number;
}

/** The options of a file field (`t.file({ maxSize: '1 GiB', accept: ['text/csv', 'image/*'] })`). */
export interface FileOptions {
  /**
   * The largest file the field takes: bytes (`1048576`), or a number and a unit (`'1 GiB'`, `'500 MB'`; `KiB`, `MiB`,
   * `GiB`, `TiB` binary, `KB`, `MB`, `GB`, `TB` decimal). Left out: the workspace's limit.
   */
  maxSize?: number | string;
  /** The MIME types (`text/csv`) or patterns (`image/*`) it accepts. Left out: any type. */
  accept?: readonly string[];
}

const SIZE_UNITS: Record<string, bigint> = {
  "": 1n, b: 1n, kb: 1_000n, mb: 1_000_000n, gb: 1_000_000_000n, tb: 1_000_000_000_000n,
  kib: 1n << 10n, mib: 1n << 20n, gib: 1n << 30n, tib: 1n << 40n,
};

/**
 * A size as file fields take it: bytes (`1048576`), or a number and a unit, binary (`KiB`, `MiB`, `GiB`, `TiB`: powers
 * of 1024) or decimal (`B`, `KB`, `MB`, `GB`, `TB`: powers of 1000), a space between them optional, the unit's case
 * ignored (`'1 GiB'`, `'500MB'`, `'1.5 GiB'`). A fraction of a byte is dropped. `undefined` when it is not a size.
 * @internal
 */
export function parseSize(s: string): number | undefined {
  const text = s.trim();
  const m = /^([0-9.]*)(.*)$/s.exec(text)!;
  const num = m[1];
  const unit = m[2].trim().toLowerCase();
  if (num === "" || num.startsWith(".") || num.endsWith(".") || num.split(".").length > 2) return undefined;
  const mult = Object.hasOwn(SIZE_UNITS, unit) ? SIZE_UNITS[unit] : undefined;
  if (mult === undefined) return undefined;
  const [whole, frac = ""] = num.split(".");
  let bytes = BigInt(whole) * mult;
  if (frac !== "") {
    const digits = frac.slice(0, 18);
    bytes += (BigInt(digits) * mult) / 10n ** BigInt(digits.length);
  }
  return bytes <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(bytes) : undefined;
}

function fileSpec(options: FileOptions): FileSpec {
  const out: FileSpec = {};
  const size = options.maxSize;
  if (size !== undefined) {
    const n = typeof size === "string" ? parseSize(size) : Number.isSafeInteger(size) ? size : undefined;
    if (n === undefined || n <= 0) out.badSize = String(size);
    else out.maxSize = n;
  }
  if (options.accept !== undefined && options.accept.length > 0) out.accept = [...options.accept];
  return out;
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
  /** the entity's history keeps the column's changes */
  history?: boolean;
  /** @internal the options of `t.ref(target, options)` it does not know */
  unknownOptions?: import("./keys.js").OptionProblem[];
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
    /** @internal The options of a settings field (`label`, `default`, the validators, ..), as declared. */
    readonly settings: SettingsSpec = {},
  ) {}

  /** @internal The same field with these settings options added. */
  private setting(more: SettingsSpec): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, this.description, this.column, { ...this.settings, ...more });
  }

  /** A setting's label in the settings form; by default its name in words (`threadWarmHours` is `Thread warm hours`). */
  label(text: string): Field<T, O, R> {
    return this.setting({ label: text });
  }

  /**
   * A setting's default: what `ctx.settings(..)` reads until an owner or an admin sets another value (a setting with
   * one is never required). Not for a secret.
   */
  default(value: T): Field<T, O, R> {
    return this.setting({ default: value, hasDefault: true });
  }

  /** The section of the settings form the setting is in (the settings without one share an untitled section). */
  section(title: string): Field<T, O, R> {
    return this.setting({ section: title });
  }

  /** A hint shown in a setting's empty input. */
  placeholder(text: string): Field<T, O, R> {
    return this.setting({ placeholder: text });
  }

  /** A text setting filled in a text area. */
  multiline(): Field<T, O, R> {
    return this.setting({ multiline: true });
  }

  /** A setting's lowest value: a number (a bigint for an int64), a decimal or a date (`YYYY-MM-DD`). */
  min(value: SettingBound): Field<T, O, R> {
    return this.setting({ min: value, hasMin: true });
  }

  /** A setting's highest value: a number (a bigint for an int64), a decimal or a date (`YYYY-MM-DD`). */
  max(value: SettingBound): Field<T, O, R> {
    return this.setting({ max: value, hasMax: true });
  }

  /** The fewest characters (Unicode code points) of a text or a secret setting. */
  minLen(n: number): Field<T, O, R> {
    return this.setting({ minLen: n });
  }

  /** The most characters (Unicode code points) of a text or a secret setting. */
  maxLen(n: number): Field<T, O, R> {
    return this.setting({ maxLen: n });
  }

  /**
   * A regular expression a text setting matches somewhere (anchor it with `^` and `$`): literals, classes, `\d`,
   * `\w`, groups, alternation and repetition. A `RegExp`'s source is taken, its flags are not.
   */
  pattern(re: string | RegExp): Field<T, O, R> {
    return this.setting({ pattern: typeof re === "string" ? re : re.source });
  }

  /** A text setting that is an absolute `http` or `https` URL. */
  url(): Field<T, O, R> {
    return this.setting({ format: "url" });
  }

  /** A text setting that is an email address. */
  email(): Field<T, O, R> {
    return this.setting({ format: "email" });
  }

  /** The fewest items of a list setting. */
  minItems(n: number): Field<T, O, R> {
    return this.setting({ minItems: n });
  }

  /** The most items of a list setting. */
  maxItems(n: number): Field<T, O, R> {
    return this.setting({ maxItems: n });
  }

  /** The text shown under a setting when any of its validators refuses a value, instead of the platform's own. */
  message(text: string): Field<T, O, R> {
    return this.setting({ message: text });
  }

  /** The field may be absent (`undefined` in TypeScript, omitted on the wire). Fields are required otherwise. */
  optional(): Field<T, true, R> {
    return new Field<T, true, R>(this.spec, true, this.description, this.column, this.settings);
  }

  /** The field's description (descriptor, documentation, MCP). */
  describe(text: string): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, text, this.column, this.settings);
  }

  /** An entity column whose values are unique (among the rows that are not deleted, on a soft-delete entity). */
  unique(): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, this.description, { ...this.column, unique: true }, this.settings);
  }

  /** An entity column with an index of its own (refs have one without it). */
  index(): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, this.description, { ...this.column, index: true }, this.settings);
  }

  /**
   * A text column searched by full text (`note.f.title.search('words')`), in a PostgreSQL text search language
   * (`simple` by default: no stemming).
   */
  searchable(language = "simple"): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, this.description, { ...this.column, searchable: language }, this.settings);
  }

  /**
   * An entity column the platform keeps encrypted at rest with the workspace's key (never unique, indexed, filtered,
   * sorted, a ref, the id, the label or read by declared SQL).
   */
  encrypted(): Field<T, O> {
    return new Field<T, O>(this.spec, this.isOptional, this.description, { ...this.column, encrypted: true }, this.settings);
  }

  /**
   * An entity column whose changes the entity's history keeps (`entity({ history: { layer } })`): each change a row of
   * the history with the column as it was after it. A kept column is a scalar, an enum or a ref; never encrypted, and
   * masked only when the history says `unmasked`.
   */
  history(): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, this.description, { ...this.column, history: true }, this.settings);
  }

  /** @internal A use of this field with its own identity (messages and entities copy their fields). */
  use(): Field<T, O, R> {
    return new Field<T, O, R>(this.spec, this.isOptional, this.description, this.column, this.settings);
  }
}

/** A bound of a setting (`min`, `max`): a number, a bigint (an int64), a decimal, or a date or a decimal as text. */
export type SettingBound = number | bigint | string | Decimal;

/** @internal The options of a settings field as declared (each present only when given). */
export interface SettingsSpec {
  label?: string;
  default?: unknown;
  hasDefault?: boolean;
  section?: string;
  placeholder?: string;
  multiline?: boolean;
  min?: SettingBound;
  hasMin?: boolean;
  max?: SettingBound;
  hasMax?: boolean;
  minLen?: number;
  maxLen?: number;
  pattern?: string;
  format?: "url" | "email";
  minItems?: number;
  maxItems?: number;
  message?: string;
}

/** @internal Whether a field carries any option of a setting. */
export function hasSettingsOptions(f: AnyField): boolean {
  return Object.keys(f.settings).length > 0;
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
  private cmpCol(op: string, other: FieldRef<any, any, any, any> | ColumnLike | DaysAgo): ColCmp {
    return new ColCmp(this, op, other);
  }

  /**
   * In a query (`ctx.query(..)`) or a view: this column equals another of the same type (a key and a ref to its
   * entity, two refs to one entity, two plain columns such as two strings); also what `joinOn` joins on. A date or
   * timestamp column also compares with `daysAgo(n)`.
   */
  eqCol(other: ColumnOf<T, R> | DayOf<T>): ColCmp {
    return this.cmpCol("EQ", other);
  }
  /** Not equal (a null is not equal to a value). */
  neCol(other: ColumnOf<T, R> | DayOf<T>): ColCmp {
    return this.cmpCol("NE", other);
  }
  ltCol(other: ColumnOf<T, R> | DayOf<T>): ColCmp {
    return this.cmpCol("LT", other);
  }
  leCol(other: ColumnOf<T, R> | DayOf<T>): ColCmp {
    return this.cmpCol("LE", other);
  }
  gtCol(other: ColumnOf<T, R> | DayOf<T>): ColCmp {
    return this.cmpCol("GT", other);
  }
  geCol(other: ColumnOf<T, R> | DayOf<T>): ColCmp {
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

  /** Between `from` and `to`, both included (`ge(from)` and `le(to)`). */
  between(from: T, to: T): Filter<M> {
    return and(this.ge(from), this.le(to));
  }

  private bucket(unit: BucketUnit): Bucket<M, O> {
    const scalar = this.field.spec.scalar;
    if (this.field.spec.kind !== "scalar" || (scalar !== "date" && scalar !== "timestamp")) {
      throw new TypeError(`${this.name}.${unit.toLowerCase()}(): ${this.name} is not a date or a timestamp`);
    }
    return new Bucket<M, O>(this, unit);
  }
  /**
   * The day (in the workspace's time zone) of a timestamp or a date column: a `date` (`YYYY-MM-DD`) wherever a column
   * stands in a query or a view (filtered, grouped, selected, ordered, joined).
   */
  day(this: FieldRef<M, Date | string, R, O>): Bucket<M, O> {
    return this.bucket("DAY");
  }
  /** The first day of the week the moment or day falls in, from the workspace's first day of the week. */
  week(this: FieldRef<M, Date | string, R, O>): Bucket<M, O> {
    return this.bucket("WEEK");
  }
  /** The first day of the month. */
  month(this: FieldRef<M, Date | string, R, O>): Bucket<M, O> {
    return this.bucket("MONTH");
  }
  /** The first day of the quarter. */
  quarter(this: FieldRef<M, Date | string, R, O>): Bucket<M, O> {
    return this.bucket("QUARTER");
  }
  /** The first day of the year. */
  year(this: FieldRef<M, Date | string, R, O>): Bucket<M, O> {
    return this.bucket("YEAR");
  }

  /**
   * The column's text, without the spaces around it, read as another type (a lake table's text column, in a model or
   * a lake query only): `stockFile.f.onHand.parse('decimal')`. A text that does not read so fails the query (a
   * model's refresh: 422 `lake_model_parse`, naming the column; nothing is written). A decimal reads only within the
   * lake's digits (20 before the point, 18 after), never rounded; an integer only as digits; a bool as `true` or
   * `false`; a date as `YYYY-MM-DD`; a timestamp as RFC 3339 with its offset. An absent text stays absent.
   */
  parse<K extends ParseKind>(this: FieldRef<M, string, R, O>, kind: K): Parsed<M, ParseValue[K], O> {
    return new Parsed<M, ParseValue[K], O>(this, kind);
  }
}

/** @internal A column of a query as the session carries it: a source's alias, a field, its bucket. */
export type WColumn = { source: string; field: string; bucket?: BucketUnit; parse?: ParseWire };

/** @internal A column of a query that is not a field handle: an alias's column, a bucket (`query.ts`). */
export interface ColumnLike {
  /** @internal */ readonly queryColumn: () => WColumn;
  /** @internal */ readonly field: AnyField;
}

/** A column a column of type `T` (a ref to `R`) compares with: a field handle or an alias's column of that type. */
export type ColumnOf<T, R> =
  | FieldRef<any, T, R extends UntypedRef ? any : R | UntypedRef, any>
  | (ColumnLike & { readonly __value: T });

/** What a date or timestamp column also compares with: a day relative to the day the query runs. */
export type DayOf<T> = [T] extends [Date] ? DaysAgo : [T] extends [string] ? DaysAgo : never;

/** Two columns compared (`a.eqCol(b)`), or a column and `daysAgo(n)`: a query's condition, and what `joinOn` joins on. */
export class ColCmp {
  constructor(
    /** @internal */ readonly left: FieldRef<any, any, any, any> | ColumnLike,
    /** @internal */ readonly op: string,
    /** @internal */ readonly right: FieldRef<any, any, any, any> | ColumnLike | DaysAgo,
  ) {}
}

/**
 * A day relative to the day the query runs: today in the workspace's time zone less `days` days, worked out when the
 * query runs (so a view of "the last 30 days" moves with the calendar). Compared with a date column or a bucket as
 * that day, with a timestamp column as the first moment of that day: `order.f.paidAt.geCol(daysAgo(30))`.
 */
export class DaysAgo {
  constructor(/** how many days before today */ readonly days: number) {
    if (!Number.isInteger(days) || days < -2_147_483_648 || days > 2_147_483_647) {
      throw new TypeError(`daysAgo(${days}): a whole number of days`);
    }
  }
}

/** `daysAgo(n)`: today in the workspace's time zone less `n` days, worked out when the query runs. */
export function daysAgo(n: number): DaysAgo {
  return new DaysAgo(n);
}

// --- query conditions and orders beyond one entity's filter ----------------------------------------------------

/** @internal A query's term as the session carries it: a column, an aggregate, a relative day. */
export type WTerm = { column?: WColumn; aggregate?: { fn: string; column?: WColumn }; daysAgo?: number };
/** @internal A query's condition as the session carries it. */
export type WExpr =
  | { and: { items: WExpr[] } }
  | { or: { items: WExpr[] } }
  | { not: WExpr }
  | { compare: { left: WTerm; op: string; values?: Record<string, unknown>[]; right?: WTerm } }
  | { exists: { source: { entity: string; alias: string }; filter?: WExpr } };

/** A condition of a query beyond one entity's filter: `exists`, a column against a column, an aggregate's comparison,
 * a bucket's, and the `and`, `or`, `not` of conditions of any entities. */
export class QCond {
  constructor(/** @internal */ readonly expr: WExpr | undefined) {}
}

/** An order of a query: a field handle's (`order.f.total.desc()`), an alias column's, a bucket's, an aggregate's. */
export class QOrder {
  constructor(/** @internal */ readonly wire: { term: WTerm; desc: boolean }) {}
}

/** The periods a bucket names. */
export type BucketUnit = "DAY" | "WEEK" | "MONTH" | "QUARTER" | "YEAR";

/**
 * The period a date or timestamp column falls in (`order.f.paidAt.month()`): the first day of its day, week, month,
 * quarter or year in the workspace's time zone, a `date` (`YYYY-MM-DD`; `undefined` where the column is null). It
 * stands wherever a column does in a query or a view: compared (`eq`, `ge`, `between`, `in`, `geCol(daysAgo(30))`),
 * grouped, selected, ordered, joined.
 */
export class Bucket<M, O extends boolean = boolean> implements ColumnLike {
  declare readonly __message: (m: M) => M;
  declare readonly __value: string;
  declare readonly __optional: O;
  /** @internal what its value is read as: a date */
  readonly field: AnyField;

  constructor(
    /** @internal */ readonly ref: FieldRef<M, any, any, O>,
    /** @internal */ readonly unit: BucketUnit,
  ) {
    const date = new Field<string, false>({ kind: "scalar", scalar: "date" });
    this.field = ref.field.isOptional ? date.optional() : date;
  }

  /** @internal */
  readonly queryColumn = (): WColumn => {
    if (!this.ref.owner) throw new TypeError(`query: the column ${this.ref.name} is not an entity's or a view's`);
    return { source: defaultAlias(this.ref.owner), field: this.ref.name, bucket: this.unit };
  };

  private compare(op: string, values: readonly string[]): QCond {
    const encoded = values.map((v) => dbValue(encodeFieldValue(this.field, v, this.ref.name)));
    return new QCond({ compare: { left: { column: this.queryColumn() }, op, values: encoded } });
  }
  eq(v: string): QCond {
    return this.compare("EQ", [v]);
  }
  ne(v: string): QCond {
    return this.compare("NE", [v]);
  }
  lt(v: string): QCond {
    return this.compare("LT", [v]);
  }
  le(v: string): QCond {
    return this.compare("LE", [v]);
  }
  gt(v: string): QCond {
    return this.compare("GT", [v]);
  }
  ge(v: string): QCond {
    return this.compare("GE", [v]);
  }
  /** Between `from` and `to` (`YYYY-MM-DD`), both included. */
  between(from: string, to: string): QCond {
    return new QCond({ and: { items: [this.ge(from).expr!, this.le(to).expr!] } });
  }
  /** Any of the days. */
  in(values: readonly string[]): QCond {
    return this.compare("IN", values);
  }
  isNull(): QCond {
    return this.compare("IS_NULL", []);
  }
  isNotNull(): QCond {
    return this.compare("IS_NOT_NULL", []);
  }
  /** Equals another date column or bucket (also what `joinOn` joins on), or `daysAgo(n)`. */
  eqCol(other: ColumnOf<string, unknown> | DaysAgo): ColCmp {
    return new ColCmp(this, "EQ", other);
  }
  neCol(other: ColumnOf<string, unknown> | DaysAgo): ColCmp {
    return new ColCmp(this, "NE", other);
  }
  ltCol(other: ColumnOf<string, unknown> | DaysAgo): ColCmp {
    return new ColCmp(this, "LT", other);
  }
  leCol(other: ColumnOf<string, unknown> | DaysAgo): ColCmp {
    return new ColCmp(this, "LE", other);
  }
  gtCol(other: ColumnOf<string, unknown> | DaysAgo): ColCmp {
    return new ColCmp(this, "GT", other);
  }
  geCol(other: ColumnOf<string, unknown> | DaysAgo): ColCmp {
    return new ColCmp(this, "GE", other);
  }
  asc(): QOrder {
    return new QOrder({ term: { column: this.queryColumn() }, desc: false });
  }
  desc(): QOrder {
    return new QOrder({ term: { column: this.queryColumn() }, desc: true });
  }
}

/** The types a lake table's text column is parsed as (`f.parse('decimal')`). */
export type ParseKind = "decimal" | "int32" | "int64" | "double" | "bool" | "date" | "timestamp" | "uuid";

/** What a parsed text is in TypeScript, by the type it is parsed as. */
export interface ParseValue {
  decimal: Decimal;
  int32: number;
  int64: bigint;
  double: number;
  bool: boolean;
  date: string;
  timestamp: Date;
  uuid: string;
}

/** @internal A parse on the wire (`QueryColumn.parse`). */
export type ParseWire = `PARSE_${Uppercase<ParseKind>}`;

const PARSE_SCALARS: Record<ParseKind, ScalarType> = {
  decimal: "decimal",
  int32: "int32",
  int64: "int64",
  double: "double",
  bool: "bool",
  date: "date",
  timestamp: "timestamp",
  uuid: "uuid",
};

/**
 * A lake table's text column read as another type (`stockFile.f.onHand.parse('decimal')`): a column of that type
 * wherever a column stands in a model of the lake or a lake query (compared, grouped, selected, ordered, a model's
 * column).
 */
export class Parsed<M, V, O extends boolean = boolean> implements ColumnLike {
  declare readonly __message: (m: M) => M;
  declare readonly __value: V;
  declare readonly __optional: O;
  /** @internal what its value is read as */
  readonly field: AnyField;

  constructor(
    /** @internal */ readonly ref: FieldRef<M, any, any, O>,
    /** @internal */ readonly kind: ParseKind,
  ) {
    const scalar = PARSE_SCALARS[kind];
    if (scalar === undefined) throw new TypeError(`parse: ${JSON.stringify(kind)} is not decimal, int32, int64, double, bool, date, timestamp or uuid`);
    const f = new Field<V, false>({ kind: "scalar", scalar });
    this.field = ref.field.isOptional ? f.optional() : f;
  }

  /** @internal */
  readonly queryColumn = (): WColumn => {
    if (!this.ref.owner) throw new TypeError(`query: the column ${this.ref.name} is not a lake table's`);
    const spec = this.ref.field.spec;
    if (spec.kind !== "scalar" || spec.scalar !== "string") throw new TypeError(`parse: ${this.ref.name} is not a text column`);
    return { source: defaultAlias(this.ref.owner), field: this.ref.name, parse: `PARSE_${this.kind.toUpperCase()}` as ParseWire };
  };

  private compare(op: string, values: readonly V[]): QCond {
    const encoded = values.map((v) => dbValue(encodeFieldValue(this.field, v, this.ref.name)));
    return new QCond({ compare: { left: { column: this.queryColumn() }, op, values: encoded } });
  }
  eq(v: V): QCond {
    return this.compare("EQ", [v]);
  }
  ne(v: V): QCond {
    return this.compare("NE", [v]);
  }
  lt(v: V): QCond {
    return this.compare("LT", [v]);
  }
  le(v: V): QCond {
    return this.compare("LE", [v]);
  }
  gt(v: V): QCond {
    return this.compare("GT", [v]);
  }
  ge(v: V): QCond {
    return this.compare("GE", [v]);
  }
  /** Between `from` and `to`, both included. */
  between(from: V, to: V): QCond {
    return new QCond({ and: { items: [this.ge(from).expr!, this.le(to).expr!] } });
  }
  in(values: readonly V[]): QCond {
    return this.compare("IN", values);
  }
  isNull(): QCond {
    return this.compare("IS_NULL", []);
  }
  isNotNull(): QCond {
    return this.compare("IS_NOT_NULL", []);
  }
  asc(): QOrder {
    return new QOrder({ term: { column: this.queryColumn() }, desc: false });
  }
  desc(): QOrder {
    return new QOrder({ term: { column: this.queryColumn() }, desc: true });
  }
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

/** @internal */
export function fieldRefs(shape: Shape, owner?: object): Record<string, FieldRef<unknown, unknown>> {
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

/** @internal */
export function copyFields<S extends Shape>(shape: S): S {
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
    /** `settings`: the plugin's settings (`t.message(.., { kind: 'settings' })`, `settingsDef(..)`). */
    readonly kind: MessageKind = "message",
  ) {
    const spec: FieldSpec = { kind: "message" };
    super(spec);
    spec.message = this as unknown as MessageDef<Shape>;
    this.fields = copyFields(shape);
    this.f = fieldRefs(this.fields) as FieldRefs<Infer<S>, S>;
  }
}

/** What a message is: a plain one, or the plugin's settings (`t.message(.., { kind: 'settings' })`). */
export type MessageKind = "message" | "settings";

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
  /**
   * `'olap'`: the entity's rows are kept in the analytic copy only (gigabytes of facts computed in the lake, read by
   * reports and joined with the plugin's other entities under the same rules). Its `load` fills it when the plugin asks
   * (`ctx.analytics().snapshot().load(e).request()`); `ctx.analytics().query(e)` reads it; `ctx.db(e)` and
   * `ctx.query(e)` are type errors (and refused at run time). Never written; no history, `softDelete`, `version` nor
   * `appendOnly`. Before its first load (and after the analytic copy was made anew, a reinstall, a changed load or
   * entity) a query of it is 409 `not_loaded`.
   */
  storage?: "olap";
  /**
   * With `storage: 'olap'`: the columns that key a row (`['day', 'store', 'metric']`: required plain values), unique
   * among the rows a load gives; the row's `id` is a uuid made from their values.
   */
  olapKey?: readonly (keyof C & string)[];
  /**
   * With `storage: 'olap'`: how the platform fills it from the lake, a view's query over the plugin's lake tables and
   * entities' histories (each read by its system context) and other plugins' lake tables exported to models, whose
   * columns are the entity's (`id` aside; an optional one may be left out):
   * `(l) => l.from(storeMetrics).where(..).columns({ day: storeMetrics.f.day, .. })`.
   */
  load?: (l: import("./query.js").LoadStart<C>) => import("./query.js").LoadDecl<C>;
  /**
   * The entity keeps its history in the workspace's Data Lake (`ctx.lake().history(e)`): every change of the columns
   * marked `.history()` (or, with `all`, of every column), each its own row with when it was made and by whom.
   */
  history?: HistoryOptions;
}

/**
 * How an entity keeps its history: the layer of the lake it goes to, which columns, who reads it. Whoever reads it
 * reads every row of it, every version, without the entity's access rules or masks.
 */
export interface HistoryOptions {
  /** A layer of the plugin (one of `plugin({ layers })`). */
  layer: LayerDef;
  /**
   * Every column is kept, the ones a later version adds too, but the encrypted ones, the lists and messages and,
   * unless `unmasked` is said too, the masked ones (`celobus plugin check` lists what it left out).
   */
  all?: boolean;
  /** Masked columns may be kept: their values go to the lake as they are (the lake has no masks). */
  unmasked?: boolean;
  /** The plugin's roles whose holders read the history, every row of it. None: no member reads it. */
  read?: readonly RoleLike[];
  /** The plugin's own system context (`ctx.asSystem(..)`) reads it too (a model over it needs this). */
  systemRead?: boolean;
  /** Other plugins that declare it (`historyDependencies`) read it, for callers holding one of its read roles. */
  export?: boolean;
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
  /** `'olap'`: the provider keeps its rows in the analytic copy only; `ctx.analytics().query(e)` reads it. */
  storage?: "olap";
  columns: C;
}

/**
 * An entity: one of the plugin's (`ctx.db(e)`), or a dependency's it reads (`ctx.query(e)`; `F` is `true`).
 */
export class EntityDef<C extends Shape, F extends boolean = boolean, O extends boolean = boolean> {
  /** @internal `true`: another plugin's entity (`ctx.query`), never this plugin's (`ctx.db`). */
  declare readonly __foreign: F;
  /** @internal `true`: stored in OLAP (`storage: 'olap'`), read with `ctx.analytics()` only. */
  declare readonly __olap: O;
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

/**
 * A view's query, as `view({ query })` builds it (`query.ts`): `(q) => q.from(orderLine).join(..).where(..).columns({..})`.
 */
export interface ViewOptions<C extends Shape> {
  /** The view's name (snake_case); by default its key in `plugin({ views })`, in snake_case. */
  name?: string;
  /** What a row is (an agent of the plugin reads it to pick the view); also its row message's description. */
  description?: string;
  /** The row's columns, each of the type its value has (`t.date()` for a bucket, `t.bigint()` for a count). */
  columns: C;
  /**
   * The view's query, run when the plugin is described: its root (`q.from(entity)`), joins, `where`, `groupBy`,
   * `having`, and its `columns`, each a column of a source (a bucket too) or, grouped, an aggregate.
   */
  query: (q: import("./query.js").ViewStart<C>) => import("./query.js").ViewDecl<C>;
}

/**
 * An analytic view of the plugin (`view({..})`): a named, typed query over its entities (a dependency's exported
 * entities, the directory, its other views) whose rows are its `columns`. The platform creates it in both engines;
 * `ctx.query(v)` reads it from the workspace's data, `ctx.analytics().query(v)` from the analytic copy. A view has no
 * rules of its own: every row under it passes its own entity's access rules and masks for the caller. It is read,
 * never written (`ctx.db(v)` is refused).
 */
export class ViewDef<C extends Shape = Shape> {
  readonly columns: C;
  /** Typed handles of the view's columns (`sales.f.day`, `sales.f.day.month()`). */
  readonly f: FieldRefs<Simplify<Infer<C>>, C>;

  constructor(readonly options: ViewOptions<C>) {
    this.columns = copyFields(options.columns);
    this.f = fieldRefs(this.columns, this) as never;
  }
}

/** Declares an analytic view: `view({ description, columns: { day: t.date(), .. }, query: (q) => q.from(..)..columns({..}) })`. */
export function view<C extends Shape>(options: ViewOptions<C>): ViewDef<C> {
  return noted(new ViewDef(options), "view", options);
}

/** @internal The name each view has in its plugin's descriptor, set when the plugin is described. */
export const viewNames = new WeakMap<object, string>();

/** A row of a view: its columns. */
export type ViewRow<V> = V extends ViewDef<infer C> ? Simplify<Infer<C>> : never;

/**
 * @internal A lake table (`lakeTable({..})`, `lake.ts`) as a query names it: its name in the plugin (`stock`), or
 * `<plugin>.<table>` for another plugin's.
 */
export interface LakeSource {
  readonly __lake: true;
  readonly columns: Shape;
  sourceName(): string;
}

/** @internal Whether `e` is a lake table. */
export function isLakeSource(e: unknown): e is LakeSource {
  return typeof e === "object" && e !== null && (e as { __lake?: unknown }).__lake === true;
}

/** @internal An entity's history (`historyOf(e)`, `history.ts`) as a query names it: the entity's own name. */
export interface HistorySource {
  readonly __history: true;
  readonly entity: EntityDef<any>;
}

/** @internal Whether `e` is an entity's history. */
export function isHistorySource(e: unknown): e is HistorySource {
  return typeof e === "object" && e !== null && (e as { __history?: unknown }).__history === true;
}

/** The name of an entity (or a view, or a lake table) in a query: `order`, `crm.customer`, `platform.user`, `sales`. */
export function canonicalOf(e: object): string {
  if (isLakeSource(e)) return e.sourceName();
  if (isHistorySource(e)) return canonicalOf(e.entity);
  if (e instanceof PlatformEntity) return e.ref;
  if (e instanceof EntityDef) {
    const provider = foreignPlugin(e);
    if (provider) return `${provider}.${e.options.name}`;
    const name = entityNames.get(e) ?? e.options.name;
    if (!name) throw new TypeError("ctx.query(): the entity is not in plugin({ entities })");
    return name;
  }
  if (e instanceof ViewDef) {
    const name = viewNames.get(e) ?? e.options.name;
    if (!name) throw new TypeError("ctx.query(): the view is not in plugin({ views })");
    return name;
  }
  throw new TypeError("ctx.query(): not an entity, a view or a lake table");
}

/** The alias of an entity (or a view) unless it is given one: its name with `_` for `.` (`crm_customer`). */
export function defaultAlias(e: object): string {
  return canonicalOf(e).replace(".", "_");
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
export function entity<C extends Shape>(options: ForeignEntityOptions<C> & { storage: "olap" }): EntityDef<C, true, true>;
export function entity<C extends Shape>(options: ForeignEntityOptions<C>): EntityDef<C, true, false>;
export function entity<C extends Shape>(options: EntityOptions<C> & { plugin?: undefined; storage: "olap" }): EntityDef<C, false, true>;
export function entity<C extends Shape>(options: EntityOptions<C> & { plugin?: undefined }): EntityDef<C, false, false>;
export function entity<C extends Shape>(options: EntityOptions<C>): EntityDef<C> {
  return noted(new EntityDef(options), options.plugin ? "foreignEntity" : "entity", options);
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

/** The options of `t.message(name, fields, options)`. */
export interface MessageOptions {
  description?: string;
  /**
   * Fields at their protobuf default are left out of the message's JSON (an enum, and an optional field with a value,
   * are still written).
   */
  omitDefaults?: boolean;
  /**
   * `settings`: the plugin's settings, what an owner or an admin sets for it (`plugin({ settings: settingsDef(m) })`);
   * only its fields take the options of a setting (`.label()`, `.default()`, `.min()`, ..) and `t.secret()`.
   */
  kind?: MessageKind;
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
  /**
   * file: a `File` (`{ id, name, type, size }`), never its bytes: `ctx.files` reads and writes them. `maxSize` is the
   * largest file the field takes (`'1 GiB'`, `'500 MB'`, or bytes), `accept` the MIME types or patterns it takes
   * (`['text/csv', 'image/*']`); a screen's upload checks a file against them before sending it. A list of files is
   * `t.array(t.file({..}))`.
   */
  file: (options: FileOptions = {}): Field<File, false> => {
    const column: ColumnSpec = {};
    const unknown = optionProblems("file", options);
    if (unknown.length > 0) column.unknownOptions = unknown;
    return new Field<File, false>({ kind: "scalar", scalar: "file", file: fileSpec(options) }, false, "", column);
  },
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
  message: <S extends Shape>(name: string, fields: S, options?: MessageOptions): MessageDef<S> =>
    noted(
      new MessageDef(name, fields, options?.description ?? "", options?.omitDefaults ?? false, options?.kind === "settings" ? "settings" : "message"),
      "message",
      options,
    ),
  /**
   * secret: a setting's secret (an API key, a token), a `Secret` read with `.expose()`; a string on the wire. Only a
   * field of the settings message (`kind: 'settings'`) is one; it has no default.
   */
  secret: () => scalar<Secret>("secret"),
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
  // the options it does not know go with the column, whatever modifiers follow
  const unknown = optionProblems("ref", options);
  if (unknown.length > 0) column.unknownOptions = unknown;
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
