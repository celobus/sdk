// UI declarations (reviews/plugin-ui-v2/proposal.md): a plugin's sidebar, screens, overlays, home widgets and
// commands, built from the platform's catalog of primitives and bound to the plugin's own endpoints and topics. It
// is data in the descriptor (`ui`): every client renders it with its own components, and every read and action is
// the member's own call. Nothing here styles anything: tones, emphasis and density are meanings.
//
// Field handles come from the message definitions (`order.f.status`, see `MessageDef.f`), so a column, an argument
// or a condition over the wrong message, a wrong field name or a value of the wrong type is a type error:
//
//   const orders = s.query('orders', ui.call(listOrders, { status }).rows(listOrdersResponse.f.orders).paged().follow(ORDERS));
//   const cancel = s.rowAction('cancel', orders, (a, r) => a.confirm(ui.confirm.danger('Cancel this order?'))
//     .call(cancelOrder, { id: r.get(order.f.id) }).toast('Order cancelled').onError((e) => e.toastError()));
//   ui.table(orders).col(order.f.number, 'Order').money(order.f.total, 'Total', order.f.currency)
//     .rowButton((r) => ui.button('Cancel', cancel).danger().visible(r.get(order.f.status).eq('placed')));
//
// The builders make a tree that still holds what only the plugin knows when it resolves (endpoint names come from
// the keys of `plugin({ endpoints })`); `resolveUi` turns it into the descriptor's JSON (`crates/descriptor/src/ui.rs`
// is its schema) and collects the `ui.text(key, en)` texts into `texts.en`.
import type { RoleLike } from "./ctx.js";
import { Decimal } from "./decimal.js";
import type { EndpointDef } from "./declare.js";
import { snake } from "./names.js";
import { EntityDef, FieldRef, MessageDef, PlatformEntity, type AnyField, type Infer, type IoType, type IoValue, type Shape, type ValueOf } from "./schema.js";
import { VIZ_KINDS } from "./viz-kinds.js";

/** The catalog revision these builders write (2: charts and figures, `ui.viz`, and dashboards, `ui.board`; 3: the app-like primitives). */
export const CATALOG = 3;

export type Tone = "neutral" | "ok" | "attention" | "danger";
export type Gap = "tight" | "normal" | "loose";
export type Size = "small" | "medium" | "large";
export type NoticeTone = Tone;

// ---------------------------------------------------------------------------------------------- the tree

/** @internal What resolving needs from the plugin. */
export interface UiResolver {
  /** The endpoint's name, or an error at `path` (and `""`) when it is not one of the plugin's. */
  endpoint(def: EndpointDef<any, any>, path: string): string;
  /** `<plugin>.<entity>` of one of the plugin's entities. */
  entity(def: EntityDef<any>, path: string): string;
}

/** @internal A part of the document known only when the plugin resolves. */
export class Deferred {
  constructor(readonly resolve: (r: UiResolver, path: string) => unknown) {}
}

/** @internal Anything that becomes JSON in the document. */
export abstract class Built {
  /** @internal */
  abstract json(): unknown;
}

/** A catalog text: its key and its English (fallback) text, which goes into the document's `texts.en`. */
export class TextKey {
  constructor(
    readonly key: string,
    readonly en: string,
    readonly args: Record<string, unknown> = {},
  ) {}

  /** An argument of the text (`ui.text('orders.cancel', 'Cancel order {number}?').arg('number', r.get(order.f.number))`). */
  arg<T>(name: string, v: ValueLike<T>): TextKey {
    return new TextKey(this.key, this.en, { ...this.args, [name]: value(v) });
  }
}

function endpointRef(def: EndpointDef<any, any>): Deferred {
  return new Deferred((r, path) => r.endpoint(def, path));
}

/** A literal as the wire carries it: int64 and decimals as strings, timestamps in RFC 3339, bytes in base64. */
function encode(x: unknown): unknown {
  if (typeof x === "bigint") return x.toString();
  if (x instanceof Date) return x.toISOString();
  if (x instanceof Decimal) return x.toString();
  if (x instanceof Uint8Array) return Buffer.from(x).toString("base64");
  if (Array.isArray(x)) return x.map(encode);
  return x;
}

/** `[]` → `undefined`, so the key is left out. */
function some<T>(a: readonly T[]): T[] | undefined {
  return a.length > 0 ? [...a] : undefined;
}

/** `in_progress` → `In progress`. */
function labelOf(v: string): string {
  const s = v.replace(/_/g, " ");
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** The descriptor's type of a field (`string`, `enum:OrderStatus`, `message:Order`, `ref:platform.user`). */
function spelled(field: AnyField): unknown {
  const spec = field.spec;
  switch (spec.kind) {
    case "scalar":
      return spec.scalar;
    case "enum":
      return `enum:${spec.enumDef!.name}`;
    case "message":
      return `message:${spec.message!.name}`;
    case "array":
      return spelled(spec.inner!);
    case "ref": {
      const target = spec.ref!;
      if (target instanceof PlatformEntity) return `ref:${target.ref}`;
      if (typeof target === "string") return `ref:${target}`;
      return new Deferred((r, path) => {
        const e = target instanceof EntityDef ? target : target();
        return `ref:${r.entity(e, path)}`;
      });
    }
  }
}

function enumOptions(field: AnyField): { value: string; label: string }[] {
  const f = field.spec.kind === "array" ? field.spec.inner! : field;
  if (f.spec.kind !== "enum") return [];
  return f.spec.enumDef!.values.map((v) => ({ value: v, label: labelOf(v) }));
}

// ---------------------------------------------------------------------------------------------- values

/** A value of type `T` from somewhere: a literal, a param, state, a query field, a row field, the caller, … */
export class Val<T> extends Built {
  declare readonly __value: T;

  constructor(/** @internal */ readonly v: unknown) {
    super();
  }

  /** @internal */
  json(): unknown {
    return this.v;
  }

  eq(other: ValueLike<T>): Cond {
    return new Cond({ eq: [this.v, value(other)] });
  }

  ne(other: ValueLike<T>): Cond {
    return new Cond({ ne: [this.v, value(other)] });
  }

  gt(other: ValueLike<T>): Cond {
    return new Cond({ gt: [this.v, value(other)] });
  }

  lt(other: ValueLike<T>): Cond {
    return new Cond({ lt: [this.v, value(other)] });
  }

  /** Absent, `""` or `[]`. */
  empty(): Cond {
    return new Cond({ empty: this.v });
  }

  isIn(set: readonly T[]): Cond {
    return new Cond({ in: { value: this.v, set: set.map(encode) } });
  }
}

/** What may fill a slot of type `T`: a value of that type, a param or state of it, or a literal. */
export type ValueLike<T> = Val<T> | ParamRef<T> | StateRef<T> | T;

/** @internal The document value of a `ValueLike`. */
function value(x: unknown): unknown {
  if (x instanceof Val) return x.v;
  if (x instanceof ParamRef) return { param: x.name };
  if (x instanceof StateRef) return { state: x.name };
  return { lit: encode(x) };
}

/** A presentation condition (never access control): `val.eq(..)`, `.and(..)`, `ui.not(..)`. */
export class Cond extends Built {
  constructor(/** @internal */ readonly c: unknown) {
    super();
  }

  /** @internal */
  json(): unknown {
    return this.c;
  }

  and(other: Cond): Cond {
    return new Cond({ and: [this.c, other.c] });
  }

  or(other: Cond): Cond {
    return new Cond({ or: [this.c, other.c] });
  }
}

/** A text: a literal, a catalog text (`ui.text(key, en)`), or a value shown as text. */
export type Txt = string | TextKey | Val<any> | ParamRef<any> | StateRef<any>;

function txt(t: Txt): unknown {
  if (typeof t === "string" || t instanceof TextKey) return t;
  return value(t);
}

/** Request (or key, or params) fields set to values: `{ id: r.get(order.f.id), reason: 'late' }`. */
export type ArgsOf<M> = { [K in keyof M]?: ValueLike<NonNullable<M[K]>> };

interface ArgJson {
  field: string;
  value: unknown;
}

function args(a: object | undefined): ArgJson[] {
  return Object.entries(a ?? {})
    .filter(([, v]) => v !== undefined)
    .map(([k, v]) => ({ field: snake(k), value: value(v) }));
}

// ---------------------------------------------------------------------------------------------- topics

/**
 * A topic: `const ORDERS = topic('orders', t.message('AllOrders', {}))`. Endpoints declare that they invalidate it
 * (`.invalidates(ORDERS)`); queries follow it (`.follow(ORDERS)`, or one key: `.follow(ORDER.key({ id }))`).
 */
export class TopicDef<K> {
  declare readonly __key: (k: K) => K;

  constructor(
    readonly name: string,
    /** The key message (its fields name one record; none: the topic has one key). */
    readonly keyMessage: MessageDef<any>,
  ) {}

  /** The declaration with its defaults (no roles or scopes, invalidated by endpoint declarations). */
  decl(): TopicDecl {
    return new TopicDecl(this, [], [], undefined, false, "");
  }

  /** Members with any of these roles may follow it. */
  roles(roles: readonly RoleLike[]): TopicDecl {
    return this.decl().roles(roles);
  }

  scopes(scopes: readonly string[]): TopicDecl {
    return this.decl().scopes(scopes);
  }

  /** The read endpoint that decides whether a member may follow one key (it takes the key's fields). */
  authorize(endpoint: EndpointDef<any, any>): TopicDecl {
    return this.decl().authorize(endpoint);
  }

  /** Invalidated by code (`ctx.invalidate`) rather than by endpoint declarations. */
  manual(): TopicDecl {
    return this.decl().manual();
  }

  describe(text: string): TopicDecl {
    return this.decl().describe(text);
  }

  /** One key of the topic, for `.follow(..)`: `ORDER.key({ id })`. */
  key(key: ArgsOf<K>): FollowKey {
    return new FollowKey(this.name, args(key));
  }

  /** An invalidation of the key whose field `key` is the response's field `from`: `ORDER.by(orderKey.f.id, order.f.id)`. */
  by<T>(key: FieldRef<K, T>, from: FieldRef<any, T>): Invalidation {
    return new Invalidation(this.name, [[key.name, from.name]]);
  }
}

/** Declares a topic and its key message. */
export function topic<S extends Shape>(name: string, key: MessageDef<S>): TopicDef<Infer<S>> {
  return new TopicDef(name, key);
}

/** A topic as the plugin declares it (`plugin({ topics: [ORDERS.roles([clerk])] })`). */
export class TopicDecl {
  constructor(
    readonly topic: TopicDef<any>,
    /** @internal */ readonly roleList: readonly RoleLike[],
    /** @internal */ readonly scopeList: readonly string[],
    /** @internal */ readonly authorizer: EndpointDef<any, any> | undefined,
    /** @internal */ readonly isManual: boolean,
    /** @internal */ readonly description: string,
  ) {}

  roles(roles: readonly RoleLike[]): TopicDecl {
    return new TopicDecl(this.topic, [...roles], this.scopeList, this.authorizer, this.isManual, this.description);
  }

  scopes(scopes: readonly string[]): TopicDecl {
    return new TopicDecl(this.topic, this.roleList, [...scopes], this.authorizer, this.isManual, this.description);
  }

  authorize(endpoint: EndpointDef<any, any>): TopicDecl {
    return new TopicDecl(this.topic, this.roleList, this.scopeList, endpoint, this.isManual, this.description);
  }

  manual(): TopicDecl {
    return new TopicDecl(this.topic, this.roleList, this.scopeList, this.authorizer, true, this.description);
  }

  describe(text: string): TopicDecl {
    return new TopicDecl(this.topic, this.roleList, this.scopeList, this.authorizer, this.isManual, text);
  }
}

export type TopicLike = TopicDef<any> | TopicDecl;

/** @internal */
export function topicDecl(t: TopicLike): TopicDecl {
  return t instanceof TopicDecl ? t : t.decl();
}

/** An endpoint's declared invalidation: a topic, and the key taken from the response. */
export class Invalidation {
  constructor(
    readonly topic: string,
    /** @internal [key field, response field] */ readonly key: readonly (readonly [string, string])[],
  ) {}

  /** Another key field. */
  and<K, T>(key: FieldRef<K, T>, from: FieldRef<any, T>): Invalidation {
    return new Invalidation(this.topic, [...this.key, [key.name, from.name]]);
  }
}

export type InvalidationLike = TopicDef<any> | Invalidation;

/** @internal The descriptor's `invalidates` entry. */
export function invalidationJson(i: InvalidationLike): Record<string, unknown> {
  if (i instanceof TopicDef) return { topic: i.name };
  const out: Record<string, unknown> = { topic: i.topic };
  if (i.key.length > 0) out.key = i.key.map(([field, from]) => ({ field, from }));
  return out;
}

/** A followed topic with a key value. */
export class FollowKey {
  constructor(
    readonly topic: string,
    /** @internal */ readonly key: readonly ArgJson[],
  ) {}
}

export type FollowLike = TopicDef<any> | FollowKey;

// ---------------------------------------------------------------------------------------------- queries

interface QueryJson {
  name: string;
  endpoint: Deferred;
  args: ArgJson[];
  rows?: string;
  paged: boolean;
  feed?: { after: string; cursor: string; key: string };
  follow: { topic: string; key?: ArgJson[]; min_interval_ms?: number }[];
  poll?: number;
  max_age?: number;
  when?: Cond;
  older?: { token: string; arg: string };
  debounce_ms?: number;
  join?: { as: string; rows: string; key: string; by: string }[];
}

function copyQuery(q: QueryJson): QueryJson {
  return { ...q, args: [...q.args], follow: q.follow.map((f) => ({ ...f })), join: q.join ? [...q.join] : undefined };
}

function queryJson(q: QueryJson): Record<string, unknown> {
  return {
    name: q.name,
    endpoint: q.endpoint,
    args: some(q.args),
    rows: q.rows,
    paged: q.paged || undefined,
    feed: q.feed,
    follow: some(q.follow),
    poll: q.poll,
    max_age: q.max_age,
    when: q.when,
    older: q.older,
    debounce_ms: q.debounce_ms,
    join: q.join && some(q.join),
  };
}

/** A query definition: `ui.call(listOrders, { status }).rows(listOrdersResponse.f.orders).paged().follow(ORDERS)`. */
export class QueryDef<Req, Resp, Row> {
  declare readonly __types: (x: [Req, Resp, Row]) => void;

  constructor(/** @internal */ readonly q: QueryJson) {}

  /** The repeated field tables, lists and charts iterate. */
  rows<R>(f: FieldRef<Resp, R[]>): QueryDef<Req, Resp, R> {
    return new QueryDef({ ...copyQuery(this.q), rows: f.name });
  }

  /** `page_token`/`page_size` in the request, `next_page_token` in the response. */
  paged(): QueryDef<Req, Resp, Row> {
    return new QueryDef({ ...copyQuery(this.q), paged: true });
  }

  /** Re-run on the topic's invalidations (any key, or one key: `ORDER.key({ id })`), at most every `minIntervalMs`. */
  follow(t: FollowLike, minIntervalMs?: number): QueryDef<Req, Resp, Row> {
    const q = copyQuery(this.q);
    const f = t instanceof TopicDef ? { topic: t.name } : { topic: t.topic, key: some(t.key) };
    q.follow.push(minIntervalMs === undefined ? f : { ...f, min_interval_ms: minIntervalMs });
    return new QueryDef(q);
  }

  /** An append-only feed: `after` takes the last cursor, `cursor` gives it, rows are deduplicated by `key`. */
  feed(after: FieldRef<Req, unknown>, cursor: FieldRef<Resp, unknown>, key: FieldRef<Row, unknown>): QueryDef<Req, Resp, Row> {
    return new QueryDef({ ...copyQuery(this.q), feed: { after: after.name, cursor: cursor.name, key: key.name } });
  }

  /** Re-run every `seconds` (at least 15). */
  poll(seconds: number): QueryDef<Req, Resp, Row> {
    return new QueryDef({ ...copyQuery(this.q), poll: seconds });
  }

  /** The oldest the data may get while shown (seconds). */
  maxAge(seconds: number): QueryDef<Req, Resp, Row> {
    return new QueryDef({ ...copyQuery(this.q), max_age: seconds });
  }

  /** Runs only while `c` holds (a record panel reads once a row is selected). */
  when(c: Cond): QueryDef<Req, Resp, Row> {
    return new QueryDef({ ...copyQuery(this.q), when: c });
  }

  /**
   * The page before the rows (catalog 3): the same endpoint and args read again with the request field `arg` set to
   * the answer's `token`; a `stream` loads older pages with it.
   */
  older<A, B>(token: FieldRef<Resp, A>, arg: FieldRef<Req, B>): QueryDef<Req, Resp, Row> {
    return new QueryDef({ ...copyQuery(this.q), older: { token: token.name, arg: arg.name } });
  }

  /** Arguments that change wait this long (ms, at most 2000) before the read: a preview of what is typed (catalog 3). */
  debounceMs(ms: number): QueryDef<Req, Resp, Row> {
    return new QueryDef({ ...copyQuery(this.q), debounce_ms: ms });
  }

  /**
   * A lookup the rows' templates read as `<via>.<field>` (catalog 3): each row gets the item of the response's
   * repeated field `rows` whose `key` equals the row's `by`:
   * `.join(who, channelView.f.people, face.f.id, msg.f.by)` with `const who = ui.via('who', msg, face)`.
   */
  join<J, K>(via: Via<Row, J>, rows: FieldRef<Resp, J[]>, key: FieldRef<J, K>, by: FieldRef<Row, K>): QueryDef<Req, Resp, Row> {
    const q = copyQuery(this.q);
    q.join = [...(q.join ?? []), { as: via.name, rows: rows.name, key: key.name, by: by.name }];
    return new QueryDef(q);
  }
}

/** A call of one of the plugin's read-only `ui` endpoints with its arguments. */
function call<I extends IoType, O extends IoType>(
  endpoint: EndpointDef<I, O>,
  a?: ArgsOf<IoValue<I>>,
): QueryDef<IoValue<I>, IoValue<O>, IoValue<O>> {
  return new QueryDef({ name: "", endpoint: endpointRef(endpoint), args: args(a), paged: false, follow: [] });
}

/** A declared query of a screen, an overlay or the document: read its fields, iterate its rows. */
export class Q<Resp, Row> {
  declare readonly __types: (x: [Resp, Row]) => void;

  constructor(readonly name: string) {}

  /** A field of the response (one record). */
  field<T>(f: FieldRef<Resp, T>): Val<T> {
    return new Val<T>({ data: { query: this.name, field: f.name } });
  }

  /** A nested field by path (`customer.name`). */
  path<T = unknown>(path: string): Val<T> {
    return new Val<T>({ data: { query: this.name, field: path } });
  }
}

/** The current row of a table, list, chart or row menu. */
export class RowRef<R> {
  declare readonly __row: (r: R) => R;

  get<T>(f: FieldRef<R, T>): Val<T> {
    return new Val<T>({ row: f.name });
  }

  /** A row value by path: a nested field (`ui.path(..)`) or a field of a query's join (`who.f(face.f.name)`). */
  at<T>(p: RowPathLike<R, T>): Val<T> {
    return new Val<T>({ row: rowPath(p) });
  }

  /** The row as a list item (`r.view(r.get(order.f.customer)).subtitle(..)`): a table's compact view, a search result. */
  view(title: Txt): RowViewB<R> {
    return new RowViewB<R>(title);
  }
}

/** The values of the enclosing form. */
export class FormVals<Req> {
  declare readonly __request: (r: Req) => Req;

  get<T>(f: FieldRef<Req, T>): Val<T> {
    return new Val<T>({ form: f.name });
  }
}

// ---------------------------------------------------------------------------------------------- screens

/** A screen's key and its params message; `ui.screen(key, build)` builds it. */
export class ScreenKey<P> {
  declare readonly __params: (p: P) => P;

  constructor(
    readonly id: string,
    /** @internal */ readonly paramsMessage?: MessageDef<any>,
  ) {}
}

/** An overlay's key and its params message. */
export class OverlayKey<P> {
  declare readonly __params: (p: P) => P;

  constructor(
    readonly id: string,
    /** @internal */ readonly paramsMessage?: MessageDef<any>,
  ) {}
}

/** A screen or overlay param. */
export class ParamRef<T> {
  declare readonly __value: T;

  constructor(
    readonly name: string,
    /** @internal */ readonly field?: AnyField,
  ) {}

  val(): Val<T> {
    return new Val<T>({ param: this.name });
  }

  private input(control: string, label: Txt | undefined): InputB {
    return new InputB({ name: this.name, control, label: txt(label ?? labelOf(this.name)) });
  }

  /** A select in a filter bar: the enum's values (or `.option(..)`); the label defaults to the param's name. */
  select(label?: Txt): InputB {
    const i = this.input("select", label);
    if (this.field) i.i.options = enumOptions(this.field);
    return i;
  }

  /**
   * A filter that takes several values: the enum's values (or `.option(..)`). The param is a string: the chosen
   * values joined with commas, in the options' order (`unpaid,pending`); the endpoint splits it.
   */
  choices(label?: Txt): InputB {
    const i = this.input("multi_select", label);
    if (this.field) i.i.options = enumOptions(this.field);
    return i;
  }

  search(label?: Txt): InputB {
    return this.input("search", label);
  }

  date(label?: Txt): InputB {
    return this.input("date", label);
  }
}

/** A state value of a screen or overlay. */
export class StateRef<T> {
  declare readonly __value: T;

  constructor(readonly name: string) {}

  val(): Val<T> {
    return new Val<T>({ state: this.name });
  }
}

/** A declared action (a chain of steps). */
export class ActionRef {
  constructor(readonly name: string) {}
}

/** What a screen's or overlay's builder function returns: its layout (or `s.layout(..)` sets it). */
export type BuildResult = Node | ScreenBuilder<any> | void;
export type ScreenBuild<P> = (s: ScreenBuilder<P>) => BuildResult;

/** The builder a screen's or overlay's function gets. */
export class ScreenBuilder<P> {
  declare readonly __params: (p: P) => P;
  /** @internal */ readonly params: Record<string, unknown>[] = [];
  /** @internal */ readonly states: Record<string, unknown>[] = [];
  /** @internal */ readonly queries: Record<string, unknown>[] = [];
  /** @internal */ readonly actions: Record<string, unknown>[] = [];
  /** @internal */ readonly overlays: OverlayDef<any>[] = [];
  /** @internal */ readonly headerNodes: Node[] = [];
  /** @internal */ titleText?: unknown;
  /** @internal */ subtitleText?: unknown;
  /** @internal */ kickerText?: unknown;
  /** @internal */ sizeValue: Size = "medium";
  /** @internal */ layoutNode?: Node;
  private counter = 0;

  constructor(paramsMessage?: MessageDef<any>) {
    for (const [key, field] of Object.entries((paramsMessage?.fields ?? {}) as Shape)) {
      this.params.push({
        name: snake(key),
        type: spelled(field),
        required: !field.isOptional && field.spec.kind !== "array",
      });
    }
  }

  title(t: Txt): this {
    this.titleText = txt(t);
    return this;
  }

  subtitle(t: Txt): this {
    this.subtitleText = txt(t);
    return this;
  }

  /** A small line above an overlay's title (`COMMERCE · ORDERS`). */
  kicker(t: Txt): this {
    this.kickerText = txt(t);
    return this;
  }

  /** An overlay's size (default medium). */
  size(size: Size): this {
    this.sizeValue = size;
    return this;
  }

  /** A param of the params message (a filter, a record id). */
  param<T>(f: FieldRef<P, T>): ParamRef<T> {
    return new ParamRef<T>(f.name, f.field);
  }

  /**
   * A param with a default: a screen opened without it behaves as if the link had the default (the queries get it,
   * a `segmented` or a tab bound to it shows it selected), and the link stays without it. The param always has a
   * value, so the handle is of the plain type even when the field is optional:
   * `const range = s.paramOr(reportParams.f.range, '30')`.
   */
  paramOr<T>(f: FieldRef<P, T>, initial: NoInfer<NonNullable<T>>): ParamRef<NonNullable<T>> {
    const p = this.params.find((x) => x.name === f.name);
    if (p) {
      p.required = false;
      p.default = encode(initial);
    }
    return new ParamRef<NonNullable<T>>(f.name, f.field);
  }

  /** State of a type (a `t.*` field) with its initial value. */
  state<F extends AnyField>(name: string, type: F, initial?: ValueOf<F>): StateRef<ValueOf<F>> {
    this.states.push({ name, type: spelled(type), default: initial === undefined ? undefined : encode(initial) });
    return new StateRef(name);
  }

  /**
   * State that returns to its default when one of these params changes (catalog 3; a tab that starts over in another
   * conversation): `s.stateReset('tab', t.text(), 'notes', [channel])`.
   */
  stateReset<F extends AnyField>(name: string, type: F, initial: ValueOf<F>, params: readonly (ParamRef<any> | string)[]): StateRef<ValueOf<F>> {
    const r = this.state(name, type, initial);
    this.states[this.states.length - 1].reset = some(params.map((p) => (typeof p === "string" ? p : p.name)));
    return r;
  }

  query<Req, Resp, Row>(name: string, def: QueryDef<Req, Resp, Row>): Q<Resp, Row> {
    this.queries.push(queryJson({ ...copyQuery(def.q), name }));
    return new Q(name);
  }

  /** A chain of steps run by a button, a menu item or a row open. */
  action(name: string, build: (a: Chain) => unknown): ActionRef {
    const c = new Chain();
    build(c);
    return this.pushAction(name, c);
  }

  /** A chain that reads the row it is run on (`r.get(order.f.id)`). */
  rowAction<R>(name: string, _rows: Q<any, R>, build: (a: Chain, r: RowRef<R>) => unknown): ActionRef {
    const c = new Chain();
    build(c, new RowRef<R>());
    return this.pushAction(name, c);
  }

  private pushAction(name: string, c: Chain): ActionRef {
    this.actions.push({ name, steps: c.steps, on_error: some(c.errorSteps) });
    return new ActionRef(name);
  }

  /** Buttons and menus in the header (a screen) or the footer (a dialog). */
  header(nodes: readonly Node[]): this {
    this.headerNodes.push(...nodes);
    return this;
  }

  /** The same as `header`, as overlays name it. */
  buttons(nodes: readonly Node[]): this {
    return this.header(nodes);
  }

  /** An overlay this screen opens (`a.open(CANCEL, { id })`). */
  overlay(o: OverlayDef<any>): this {
    this.overlays.push(o);
    return this;
  }

  layout(node: Node): this {
    this.layoutNode = node;
    return this;
  }

  /** A form filling `endpoint`'s request; its submit runs the chain `submit` builds (`a.callForm(endpoint, ..)`). */
  form<I extends IoType, O extends IoType>(
    endpoint: EndpointDef<I, O>,
    build: (f: FormB<IoValue<I>>) => unknown,
    submitLabel: Txt,
    submit: (a: Chain, v: FormVals<IoValue<I>>) => unknown,
  ): N {
    const fb = new FormB<IoValue<I>>();
    build(fb);
    let alternate: { label: unknown; action: string } | undefined;
    if (fb.alternateButton) {
      this.counter += 1;
      const alt = this.pushAction(`alternate_${this.counter}`, fb.alternateButton.chain);
      alternate = { label: fb.alternateButton.label, action: alt.name };
    }
    this.counter += 1;
    const c = new Chain();
    submit(c, new FormVals());
    const a = this.pushAction(`submit_${this.counter}`, c);
    return new N("form", {
      endpoint: endpointRef(endpoint),
      sections: fb.sections
        .filter((s) => s.inputs.length > 0)
        .map((s) => ({ title: s.title, inputs: s.inputs.map((i) => i.i) })),
      submit: a.name,
      submit_label: txt(submitLabel),
      phone_submit_label: fb.phoneSubmitLabel,
      cancel_label: fb.cancelLabel,
      summary: fb.summaryValue,
      note: fb.noteText,
      alternate,
      notes: some(fb.notes),
    });
  }

  /** @internal */
  finish(id: string, result: BuildResult): Node {
    const layout = result instanceof Node ? result : this.layoutNode;
    if (!layout) throw new TypeError(`ui: ${id} has no layout (return a node or call s.layout(..))`);
    this.titleText ??= labelOf(id);
    return layout;
  }
}

/** A screen: `/w/{slug}/p/{plugin}/s/{id}?{params}`. */
export class ScreenDef<P> extends ScreenKey<P> {
  constructor(
    id: string,
    paramsMessage: MessageDef<any> | undefined,
    private readonly build: ScreenBuild<P>,
  ) {
    super(id, paramsMessage);
  }

  /** @internal */
  json(): Record<string, unknown> {
    const s = new ScreenBuilder<P>(this.paramsMessage);
    const layout = s.finish(this.id, this.build(s));
    return {
      id: this.id,
      title: s.titleText,
      subtitle: s.subtitleText,
      params: some(s.params),
      state: some(s.states),
      queries: some(s.queries),
      actions: some(s.actions),
      overlays: some(s.overlays.map((o) => o.json())),
      header: some(s.headerNodes),
      layout,
    };
  }
}

export type OverlayKind = "dialog" | "side_panel" | "sheet";

/** A dialog, side panel or sheet over a screen. */
export class OverlayDef<P> extends OverlayKey<P> {
  constructor(
    id: string,
    paramsMessage: MessageDef<any> | undefined,
    readonly kind: OverlayKind,
    private readonly build: ScreenBuild<P>,
  ) {
    super(id, paramsMessage);
  }

  /** @internal */
  json(): Record<string, unknown> {
    const s = new ScreenBuilder<P>(this.paramsMessage);
    const layout = s.finish(this.id, this.build(s));
    return {
      id: this.id,
      kind: this.kind,
      size: s.sizeValue,
      title: s.titleText,
      kicker: s.kickerText,
      params: some(s.params),
      state: some(s.states),
      queries: some(s.queries),
      actions: some(s.actions),
      buttons: some(s.headerNodes),
      layout,
    };
  }
}

function screenKey<S extends Shape>(id: string, params: MessageDef<S>): ScreenKey<Infer<S>>;
function screenKey(id: string): ScreenKey<{}>;
function screenKey(id: string, params?: MessageDef<any>): ScreenKey<any> {
  return new ScreenKey(id, params);
}

function overlayKey<S extends Shape>(id: string, params: MessageDef<S>): OverlayKey<Infer<S>>;
function overlayKey(id: string): OverlayKey<{}>;
function overlayKey(id: string, params?: MessageDef<any>): OverlayKey<any> {
  return new OverlayKey(id, params);
}

function screen<P>(key: ScreenKey<P>, build: ScreenBuild<P>): ScreenDef<P>;
function screen<S extends Shape>(id: string, params: MessageDef<S>, build: ScreenBuild<Infer<S>>): ScreenDef<Infer<S>>;
function screen(id: string, build: ScreenBuild<{}>): ScreenDef<{}>;
function screen(a: string | ScreenKey<any>, b: MessageDef<any> | ScreenBuild<any>, c?: ScreenBuild<any>): ScreenDef<any> {
  if (a instanceof ScreenKey) return new ScreenDef(a.id, a.paramsMessage, b as ScreenBuild<any>);
  return b instanceof MessageDef ? new ScreenDef(a, b, c!) : new ScreenDef(a, undefined, b);
}

function overlayOf(kind: OverlayKind) {
  function make<P>(key: OverlayKey<P>, build: ScreenBuild<P>): OverlayDef<P>;
  function make<S extends Shape>(id: string, params: MessageDef<S>, build: ScreenBuild<Infer<S>>): OverlayDef<Infer<S>>;
  function make(id: string, build: ScreenBuild<{}>): OverlayDef<{}>;
  function make(a: string | OverlayKey<any>, b: MessageDef<any> | ScreenBuild<any>, c?: ScreenBuild<any>): OverlayDef<any> {
    if (a instanceof OverlayKey) return new OverlayDef(a.id, a.paramsMessage, kind, b as ScreenBuild<any>);
    return b instanceof MessageDef ? new OverlayDef(a, b, kind, c!) : new OverlayDef(a, undefined, kind, b);
  }
  return make;
}

// ---------------------------------------------------------------------------------------------- actions

/** A confirmation before the next steps: `ui.confirm('Archive it?')`, `ui.confirm.danger('Delete it?')`. */
export class ConfirmB {
  /** @internal */ readonly c: Record<string, unknown>;

  constructor(title: Txt) {
    this.c = { title: txt(title) };
  }

  text(t: Txt): this {
    this.c.text = txt(t);
    return this;
  }

  confirmLabel(t: Txt): this {
    this.c.confirm_label = txt(t);
    return this;
  }

  danger(): this {
    this.c.danger = true;
    return this;
  }

  /** The member types this value to confirm. */
  typed<T>(v: ValueLike<T>): this {
    this.c.typed = value(v);
    return this;
  }

  cancelLabel(t: Txt): this {
    this.c.cancel_label = txt(t);
    return this;
  }

  /** What is affected, as a record line with an icon (`Order #1048 · Hartwell & Co · $2,480.00`). */
  summary(icon: string, t: Txt): this {
    this.c.summary = txt(t);
    this.c.summary_icon = icon;
    return this;
  }
}

const confirm = Object.assign((title: Txt) => new ConfirmB(title), {
  danger: (title: Txt) => new ConfirmB(title).danger(),
});

/** A chain of steps: `a.confirm(..).call(..).toast(..).close()`; `onError` for the failure branch. */
export class Chain {
  /** @internal */ readonly steps: Record<string, unknown>[] = [];
  /** @internal */ errorSteps: Record<string, unknown>[] = [];

  confirm(c: ConfirmB | Txt): this {
    this.steps.push({ confirm: c instanceof ConfirmB ? { ...c.c } : { title: txt(c) } });
    return this;
  }

  /** Calls a `ui` endpoint as the member. */
  call<I extends IoType, O extends IoType>(endpoint: EndpointDef<I, O>, a?: ArgsOf<IoValue<I>>): this {
    this.steps.push({ call: { endpoint: endpointRef(endpoint), args: some(args(a)) } });
    return this;
  }

  /** Calls with the enclosing form's values (and `a` over them). */
  callForm<I extends IoType, O extends IoType>(endpoint: EndpointDef<I, O>, a?: ArgsOf<IoValue<I>>): this {
    this.steps.push({ call: { endpoint: endpointRef(endpoint), args: some(args(a)), form: true } });
    return this;
  }

  /** The last call shows a pending row in `rows` until a re-read returns it. */
  pending(rows: Q<any, any>): this {
    const last = this.steps.at(-1);
    if (last && "call" in last) (last.call as Record<string, unknown>).pending = rows.name;
    return this;
  }

  toast(t: Txt, tone?: Tone): this {
    this.steps.push({ toast: { text: txt(t), tone } });
    return this;
  }

  /** In `onError`: the failing call's own message. */
  toastError(): this {
    this.steps.push({ toast: { tone: "danger", error: true } });
    return this;
  }

  /** The last toast gets a second line. */
  described(t: Txt): this {
    const last = this.steps.at(-1);
    if (last && "toast" in last) (last.toast as Record<string, unknown>).description = txt(t);
    return this;
  }

  /** The last toast gets a button running `action` (`Undo`). */
  withButton(label: Txt, action: ActionRef): this {
    const last = this.steps.at(-1);
    if (last && "toast" in last) (last.toast as Record<string, unknown>).action = { label: txt(label), action: action.name };
    return this;
  }

  open<P>(overlay: OverlayKey<P>, a?: ArgsOf<P>): this {
    this.steps.push({ open: { overlay: overlay.id, args: some(args(a)) } });
    return this;
  }

  close(): this {
    this.steps.push({ close: {} });
    return this;
  }

  navigate<P>(screen: ScreenKey<P>, a?: ArgsOf<P>): this {
    this.steps.push({ navigate: { screen: screen.id, args: some(args(a)) } });
    return this;
  }

  back(): this {
    this.steps.push({ back: {} });
    return this;
  }

  set<T>(state: StateRef<T>, v: NoInfer<ValueLike<T>>): this {
    this.steps.push({ set: { state: state.name, value: value(v) } });
    return this;
  }

  setParam<T>(param: ParamRef<T>, v: NoInfer<ValueLike<T>>): this {
    this.steps.push({ set: { param: param.name, value: value(v) } });
    return this;
  }

  /** Re-run these queries (none: every query of the screen). */
  refresh(...queries: (Q<any, any> | string)[]): this {
    this.steps.push({ refresh: { queries: some(queries.map((q) => (typeof q === "string" ? q : q.name))) } });
    return this;
  }

  openUrl(url: ValueLike<string>): this {
    this.steps.push({ open_url: { value: value(url) } });
    return this;
  }

  /** Puts the text into the composer of the item's stream (or the frame's primary composer), focused (catalog 3). */
  draft(t: Txt): this {
    this.steps.push({ draft: { text: txt(t) } });
    return this;
  }

  /** Adds `@<name> ` to the composer and sends `key` as its `reply` while the mention stays (catalog 3). */
  replyTo<K>(name: ValueLike<string>, key: Val<K>): this {
    this.steps.push({ reply_to: { name: value(name), key: key.v } });
    return this;
  }

  /** Scrolls the stream that holds an item with this key so the item is centred (catalog 3). */
  scrollTo<K>(key: Val<K>): this {
    this.steps.push({ scroll_to: { key: key.v } });
    return this;
  }

  /** Copies a value to the clipboard (catalog 3). */
  copy<T>(v: ValueLike<T>): this {
    this.steps.push({ copy: { value: value(v) } });
    return this;
  }

  /** Copies the screen's link with `#<key>` (catalog 3). */
  copyLink<T>(key: ValueLike<T>): this {
    this.steps.push({ copy: { link: value(key) } });
    return this;
  }

  /**
   * Puts a row into the loaded rows of a query (by the stream's key) until the query's next answer (catalog 3):
   * `a.call(postMessage, ..).merge(msgs, ui.result(posted.f.message))`.
   */
  merge<R>(q: Q<any, R>, row: Val<R>): this {
    this.steps.push({ merge: { query: q.name, row: row.v } });
    return this;
  }

  /** The steps run when a step fails. */
  onError(build: (e: Chain) => unknown): this {
    const c = new Chain();
    build(c);
    this.errorSteps = c.steps;
    return this;
  }
}

// ---------------------------------------------------------------------------------------------- nodes

/** A primitive with the common options: `.visible(cond)`, `.enabled(cond)`, `.fallback(node)`. */
export class Node extends Built {
  private visibleCond?: Cond;
  private enabledCond?: Cond;
  private fallbackNode?: Node;

  constructor(
    /** @internal */ readonly kind: string,
    /** @internal */ readonly body: Record<string, unknown>,
  ) {
    super();
  }

  visible(c: Cond): this {
    this.visibleCond = c;
    return this;
  }

  enabled(c: Cond): this {
    this.enabledCond = c;
    return this;
  }

  /** What an older client draws instead (display primitives only). */
  fallback(n: Node): this {
    this.fallbackNode = n;
    return this;
  }

  /** @internal */
  json(): unknown {
    return { [this.kind]: this.body, visible: this.visibleCond, enabled: this.enabledCond, fallback: this.fallbackNode };
  }
}

type Format = Record<string, Record<string, unknown>>;

const formats = {
  money: (currency?: string, code?: string): Format => ({ money: { currency, code } }),
  number: (decimals?: number): Format => ({ number: { decimals } }),
  percent: (decimals?: number): Format => ({ percent: { decimals } }),
  status: (tones: Record<string, Tone | undefined>): Format => ({ status: { tones: tonesJson(tones) } }),
  plain: (name: string): Format => ({ [name]: {} }),
};

function tonesJson(tones: Record<string, Tone | undefined>): Record<string, Tone> | undefined {
  const out = Object.fromEntries(Object.entries(tones).filter(([, v]) => v !== undefined)) as Record<string, Tone>;
  return Object.keys(out).length > 0 ? out : undefined;
}

/** A tone per value of an enum: `{ placed: 'attention', paid: 'ok' }`. */
export type Tones<T> = Partial<Record<Extract<T, string>, Tone>>;

/** The generic primitive; its options apply to the primitives that have them (as in the other SDKs). */
export class N extends Node {
  private setFormat(f: Format): this {
    if (["metric", "text", "chart"].includes(this.kind)) this.body.format = f;
    if (this.kind === "card") this.body.trailing_format = f;
    return this;
  }

  // --- structure
  /** The gap of a stack or row. */
  gap(gap: Gap): this {
    if (this.kind === "stack") this.body.gap = gap;
    return this;
  }

  /** A horizontal stack's children spread to both ends. */
  between(): this {
    if (this.kind === "stack") this.body.justify = "between";
    return this;
  }

  /** A value at the end of a card's title line (a total; its format is the card's `.moneyIn(..)` and the like). */
  trailing<T>(v: Val<T>): this {
    if (this.kind === "card") this.body.trailing = v;
    return this;
  }

  /** Relative widths of columns (`[2, 1]`). */
  ratios(r: readonly number[]): this {
    if (this.kind === "columns") this.body.ratios = some(r);
    return this;
  }

  /** A section's description. */
  description(t: Txt): this {
    if (this.kind === "section") this.body.description = txt(t);
    return this;
  }

  /** Buttons in a section's head. */
  actions(nodes: readonly Node[]): this {
    if (this.kind === "section") this.body.actions = some(nodes);
    return this;
  }

  /** A section's footer. */
  footer(nodes: readonly Node[]): this {
    if (this.kind === "section") this.body.footer = some(nodes);
    return this;
  }

  /** The state (or param) that holds the selected tab. */
  bind(to: StateRef<any> | ParamRef<any>): this {
    if (this.kind === "tabs") this.body.bind = to instanceof StateRef ? { state: to.name } : { param: to.name };
    return this;
  }

  // --- data
  /** A metric's change next to its value. */
  delta<T>(v: Val<T>): this {
    if (this.kind === "metric") this.body.delta = v;
    return this;
  }

  /** A metric's, meter's or progress' caption. */
  caption(t: Txt): this {
    if (["metric", "meter", "progress"].includes(this.kind)) this.body.caption = txt(t);
    return this;
  }

  /** A metric's caption tone and icon (`$33.9k overdue` in red with an alert icon). */
  captionTone(tone: Tone, icon: string): this {
    if (this.kind === "metric") {
      this.body.caption_tone = tone;
      this.body.caption_icon = icon;
    }
    return this;
  }

  /** A card's or fields' title. */
  titled(t: Txt): this {
    if (["card", "fields", "chart"].includes(this.kind)) this.body.title = txt(t);
    return this;
  }

  /** At most this many avatars of an avatar stack (default 5). */
  max(n: number): this {
    if (this.kind === "avatar_stack") this.body.max = n;
    return this;
  }

  // --- formats of a metric or a text
  moneyIn(code: string): this {
    return this.setFormat(formats.money(undefined, code));
  }

  number(decimals?: number): this {
    return this.setFormat(formats.number(decimals));
  }

  percent(decimals?: number): this {
    return this.setFormat(formats.percent(decimals));
  }

  date(): this {
    return this.setFormat(formats.plain("date"));
  }

  datetime(): this {
    return this.setFormat(formats.plain("datetime"));
  }

  relative(): this {
    return this.setFormat(formats.plain("relative"));
  }

  member(): this {
    return this.setFormat(formats.plain("member"));
  }

  status(tones: Record<string, Tone>): this {
    return this.setFormat(formats.status(tones));
  }

  /** Counts up, live, from a moment in ms (`0:48`, `1:02:05`; catalog 3). */
  elapsed(): this {
    return this.setFormat(formats.plain("elapsed"));
  }

  /** Counts down, live, to a moment in ms (`8 min`; catalog 3). */
  countdown(): this {
    return this.setFormat(formats.plain("countdown"));
  }

  /** A duration in ms (`1m 05s`; catalog 3). */
  span(): this {
    return this.setFormat(formats.plain("span"));
  }

  // --- a split as an app-like screen (catalog 3)
  /**
   * A split as an app-like screen: the viewport's height under the shell's header, each side scrolling itself (its
   * first `stream` takes the free height), the detail an in-page side panel shown while its `visible` holds.
   */
  fill(): this {
    if (this.kind === "split") this.body.fill = true;
    return this;
  }

  /** The split's panel width is dragged by its border: px from `min` to `max`, `initial` at first and on reset, the
   * column keeping at least `keep`. */
  resize(min: number, max: number, initial: number, keep?: number): this {
    if (this.kind === "split") this.body.resize = { min, max, default: initial, keep };
    return this;
  }

  /** The bool state the detail's header's expand button toggles: true, the detail fills the screen. */
  expand(state: StateRef<boolean>): this {
    if (this.kind === "split") this.body.expand = { state: state.name };
    return this;
  }

  /**
   * An icon button as a toggle (catalog 3): `pressed` says whether it is on; the icon and label given to
   * `ui.iconButton` are the pressed ones, these the others. Its action reads `ui.form.pressed()`.
   */
  toggle(pressed: ValueLike<boolean>, iconOff: string, labelOff: Txt): this {
    if (this.kind === "icon_button") Object.assign(this.body, { pressed: value(pressed), icon_off: iconOff, label_off: txt(labelOff) });
    return this;
  }

  // --- actions
  private emphasis(e: "primary" | "secondary" | "ghost" | "danger"): this {
    if (this.kind === "button") this.body.emphasis = e;
    return this;
  }

  primary(): this {
    return this.emphasis("primary");
  }

  ghost(): this {
    return this.emphasis("ghost");
  }

  danger(): this {
    return this.emphasis("danger");
  }

  /** The action a button runs. */
  run(action: ActionRef): this {
    if (this.kind === "button") this.body.action = action.name;
    return this;
  }

  /** A button's, menu's or empty state's icon. */
  icon(icon: string): this {
    if (["button", "menu", "empty"].includes(this.kind)) this.body.icon = icon;
    return this;
  }

  /** A notice's title, or a menu's label. */
  title(t: Txt): this {
    if (this.kind === "notice") this.body.title = txt(t);
    if (this.kind === "menu") this.body.label = txt(t);
    return this;
  }

  /** A notice's or an empty state's button. */
  withAction(b: Node): this {
    if (["notice", "empty"].includes(this.kind)) this.body.action = b;
    return this;
  }
}

function n(kind: string, body: Record<string, unknown>): N {
  return new N(kind, body);
}

// --- structure

function stack(children: readonly Node[]): N {
  return n("stack", { direction: "vertical", gap: "normal", children: [...children] });
}

/** `ui.row(children)`: children side by side (wrapping on a phone); `ui.row(field)`: a field of the current row. */
function row(children: readonly Node[]): N;
function row<T>(field: FieldRef<any, T>): Val<T>;
function row(x: readonly Node[] | FieldRef<any, unknown>): N | Val<unknown> {
  if (x instanceof FieldRef) return new Val({ row: x.name });
  return n("stack", { direction: "horizontal", gap: "normal", children: [...x] });
}

/** Columns that stack on a phone. */
function columns(children: readonly Node[]): N {
  return n("columns", { children: [...children] });
}

function section(title: Txt, children: readonly Node[]): N {
  return n("section", { title: txt(title), children: [...children] });
}

function card(children: readonly Node[]): N {
  return n("card", { children: [...children] });
}

function divider(): N {
  return n("divider", {});
}

function split(list: Node, detail: Node): N {
  return n("split", { list, detail });
}

/** One tab: its id, title and content. */
export class TabB extends Built {
  private badgeValue?: Val<any>;

  constructor(
    readonly id: string,
    private readonly title: Txt,
    private readonly children: readonly Node[],
  ) {
    super();
  }

  badge<T>(v: Val<T>): this {
    this.badgeValue = v;
    return this;
  }

  /** @internal */
  json(): unknown {
    return { id: this.id, title: txt(this.title), badge: this.badgeValue, children: [...this.children] };
  }
}

function tab(id: string, title: Txt, children: readonly Node[]): TabB {
  return new TabB(id, title, children);
}

function tabs(list: readonly TabB[]): N {
  return n("tabs", { tabs: [...list] });
}

// --- navigation

/**
 * A switch between a few options, bound to a state or to a param: a view switch,
 * `ui.segmented(view, [['list', 'List'], ['board', 'Board']])`, or the range of a report, in the link,
 * `ui.segmented(range, [['7', '7 days'], ['30', '30 days']])`. The options are values of the state's or the param's
 * type; give a param a default (`s.paramOr(..)`) so the switch shows it on a link without the param.
 */
function segmented<T>(to: StateRef<T> | ParamRef<T>, options: readonly (readonly [NoInfer<NonNullable<T>>, Txt])[]): N {
  return n("segmented", {
    bind: to instanceof StateRef ? { state: to.name } : { param: to.name },
    options: options.map(([v, l]) => ({ value: encode(v), label: txt(l) })),
  });
}

function link<P>(text: Txt, screen: ScreenKey<P>, a?: ArgsOf<P>): N {
  return n("link", { text: txt(text), target: { screen: screen.id, args: some(args(a)) } });
}

// --- data

/** A table over a query's rows; columns and row values are handles of the row message. */
export class TableB<R> extends Node {
  declare readonly __row: (r: R) => R;
  private readonly columns: Record<string, unknown>[] = [];
  private readonly rowActions: Node[] = [];
  private readonly rowTones: Record<string, unknown>[] = [];

  constructor(query: string) {
    super("table", { query });
    this.body.columns = this.columns;
  }

  private column(f: FieldRef<R, unknown>, title: Txt, format?: Format, align?: "start" | "end"): this {
    this.columns.push({ field: f.name, title: txt(title), format, align });
    return this;
  }

  col<T>(f: FieldRef<R, T>, title: Txt): this {
    return this.column(f, title);
  }

  /**
   * A column that sorts the list by the request's string field `by`: a click on its head passes `asc`, the next
   * `desc`, the third leaves the field out. One column sorts at a time.
   */
  sortable<T>(f: FieldRef<R, T>, title: Txt, by: string): this {
    this.column(f, title);
    this.columns[this.columns.length - 1].sort = snake(by);
    return this;
  }

  mono<T>(f: FieldRef<R, T>, title: Txt): this {
    return this.column(f, title, formats.plain("mono"));
  }

  number<T>(f: FieldRef<R, T>, title: Txt, decimals?: number): this {
    return this.column(f, title, formats.number(decimals), "end");
  }

  /** A decimal amount in the currency of the row's field `currency`. */
  money<T>(f: FieldRef<R, T>, title: Txt, currency: FieldRef<R, string>): this {
    return this.column(f, title, formats.money(currency.name), "end");
  }

  /** A decimal amount in a fixed currency (`USD`). */
  moneyIn<T>(f: FieldRef<R, T>, title: Txt, code: string): this {
    return this.column(f, title, formats.money(undefined, code), "end");
  }

  date<T>(f: FieldRef<R, T>, title: Txt): this {
    return this.column(f, title, formats.plain("date"));
  }

  datetime<T>(f: FieldRef<R, T>, title: Txt): this {
    return this.column(f, title, formats.plain("datetime"));
  }

  relative<T>(f: FieldRef<R, T>, title: Txt): this {
    return this.column(f, title, formats.plain("relative"));
  }

  member<T>(f: FieldRef<R, T>, title: Txt): this {
    return this.column(f, title, formats.plain("member"));
  }

  avatar<T>(f: FieldRef<R, T>, title: Txt): this {
    return this.column(f, title, formats.plain("avatar"));
  }

  tags<T>(f: FieldRef<R, T>, title: Txt): this {
    return this.column(f, title, formats.plain("tags"));
  }

  percent<T>(f: FieldRef<R, T>, title: Txt, decimals?: number): this {
    return this.column(f, title, formats.percent(decimals));
  }

  bool<T>(f: FieldRef<R, T>, title: Txt): this {
    return this.column(f, title, formats.plain("bool"));
  }

  /** An enum column drawn as a status with a tone per value. */
  status<T>(f: FieldRef<R, T>, title: Txt, tones: Tones<T>): this {
    return this.column(f, title, formats.status(tones));
  }

  /** The action run when a row is opened. */
  open(action: ActionRef): this {
    this.body.open = action.name;
    return this;
  }

  /** A button (or menu) on every row; a function gets the row (`r.get(order.f.status)`). */
  rowButton(node: Node | ((r: RowRef<R>) => Node)): this {
    this.rowActions.push(typeof node === "function" ? node(new RowRef<R>()) : node);
    this.body.row_actions = this.rowActions;
    return this;
  }

  /** The same as `rowButton`. */
  rowAction(node: Node | ((r: RowRef<R>) => Node)): this {
    return this.rowButton(node);
  }

  /** Buttons, search and filters above the table. */
  toolbar(nodes: readonly Node[]): this {
    this.body.toolbar = some(nodes);
    return this;
  }

  empty(t: Txt): this {
    this.body.empty = txt(t);
    return this;
  }

  /** The selected row's key goes into this state. */
  select<K>(state: StateRef<K>, key: FieldRef<R, K>): this {
    this.body.select = { state: state.name, key: key.name };
    return this;
  }

  /** The last column gets a second line from `f`. */
  secondLine<T>(f: FieldRef<R, T>): this {
    const c = this.columns.at(-1);
    if (c) c.subtitle = f.name;
    return this;
  }

  /** The last column gets an avatar: a picture URL from `f`, else the value's initials. */
  withAvatar<T>(f: FieldRef<R, T>): this {
    const c = this.columns.at(-1);
    if (c) c.avatar = f.name;
    return this;
  }

  /** Rows can be checked; `state` holds the checked keys. */
  checks<K>(state: StateRef<K[]>, key: FieldRef<R, K>): this {
    this.body.checks = { state: state.name, key: key.name };
    return this;
  }

  /** How many rows there are in all (`Showing 1–10 of 1,248`). */
  count<T>(v: Val<T>): this {
    this.body.count = v;
    return this;
  }

  /** The line over the compact rows (`412 of 1,248 orders · newest first`); the count alone when absent. */
  countLabel(t: Txt): this {
    this.body.count_label = txt(t);
    return this;
  }

  /** The row as a list item where a table does not fit (`(r) => r.view(r.get(order.f.customer)).subtitle(..)`). */
  compact(view: (r: RowRef<R>) => RowViewB<R>): this {
    this.body.compact = view(new RowRef<R>());
    return this;
  }

  /** The row whose key is in this param is drawn selected (a record panel next to the list). */
  selectedBy(param: ParamRef<any>, key: FieldRef<R, string>): this {
    this.body.select = { state: param.name, key: key.name };
    return this;
  }

  /** A tone for rows that match (first match wins). */
  rowTone(when: Cond | ((r: RowRef<R>) => Cond), tone: Tone): this {
    this.rowTones.push({ when: typeof when === "function" ? when(new RowRef<R>()) : when, tone });
    this.body.row_tone = this.rowTones;
    return this;
  }
}

function table<R>(q: Q<any, R>): TableB<R> {
  return new TableB<R>(q.name);
}

/** A list over a query's rows: a title per row, optionally a subtitle, meta, avatar, status. */
export class ListB<R> extends Node {
  declare readonly __row: (r: R) => R;
  private readonly rowActions: Node[] = [];

  constructor(query: string, title: string) {
    super("list", { query, title });
  }

  subtitle<T>(f: FieldRef<R, T>): this {
    this.body.subtitle = f.name;
    return this;
  }

  meta<T>(f: FieldRef<R, T>): this {
    this.body.meta = f.name;
    return this;
  }

  /** Meta shown as a relative time ("5 min ago"). */
  metaRelative<T>(f: FieldRef<R, T>): this {
    this.body.meta = f.name;
    this.body.meta_format = formats.plain("relative");
    return this;
  }

  avatar<T>(f: FieldRef<R, T>): this {
    this.body.avatar = f.name;
    return this;
  }

  icon(icon: string): this {
    this.body.icon = icon;
    return this;
  }

  /** A row field naming the row's icon. */
  iconField<T>(f: FieldRef<R, T>): this {
    this.body.icon_field = f.name;
    return this;
  }

  /** A quiet text before the meta (`×2`). */
  aside(t: Txt): this {
    this.body.aside = txt(t);
    return this;
  }

  /** The meta is money in a currency. */
  moneyIn(code: string): this {
    this.body.meta_format = formats.money(undefined, code);
    return this;
  }

  status<T>(f: FieldRef<R, T>, tones: Tones<T>): this {
    this.body.status = { field: f.name, tones: tonesJson(tones) };
    return this;
  }

  open(action: ActionRef): this {
    this.body.open = action.name;
    return this;
  }

  rowButton(node: Node | ((r: RowRef<R>) => Node)): this {
    this.rowActions.push(typeof node === "function" ? node(new RowRef<R>()) : node);
    this.body.row_actions = this.rowActions;
    return this;
  }

  empty(t: Txt): this {
    this.body.empty = txt(t);
    return this;
  }

  /** A filter box over the rows' titles with this placeholder; it filters the loaded rows, no request (catalog 3). */
  search(placeholder: Txt): this {
    this.body.search = txt(placeholder);
    return this;
  }

  /** A green dot after the title while this field is true (catalog 3). */
  online(f: FieldRef<R, boolean>): this {
    this.body.online = f.name;
    return this;
  }

  /** On a phone, the first `n` rows (1 to 100), then `Show all` (catalog 3). */
  limit(n: number): this {
    this.body.limit = n;
    return this;
  }

  /** On a phone, a label over the rows in a card (`People · 9`; catalog 3). */
  groupLabel(t: Txt): this {
    this.body.group_label = txt(t);
    return this;
  }
}

function list<R, T>(q: Q<any, R>, title: FieldRef<R, T>): ListB<R> {
  return new ListB<R>(q.name, title.name);
}

/** One label/value pair of `ui.fields` (`ui.item('Total', order.field(..)).moneyIn('USD')`). */
export class ItemB extends Built {
  private format?: Format;
  private target?: Record<string, unknown>;
  private avatarValue?: Val<any>;

  constructor(
    private readonly label: Txt,
    private readonly v: Val<any>,
  ) {
    super();
  }

  private setFormat(f: Format): this {
    this.format = f;
    return this;
  }

  moneyIn(code: string): this {
    return this.setFormat(formats.money(undefined, code));
  }

  number(decimals?: number): this {
    return this.setFormat(formats.number(decimals));
  }

  percent(decimals?: number): this {
    return this.setFormat(formats.percent(decimals));
  }

  date(): this {
    return this.setFormat(formats.plain("date"));
  }

  datetime(): this {
    return this.setFormat(formats.plain("datetime"));
  }

  relative(): this {
    return this.setFormat(formats.plain("relative"));
  }

  member(): this {
    return this.setFormat(formats.plain("member"));
  }

  bool(): this {
    return this.setFormat(formats.plain("bool"));
  }

  mono(): this {
    return this.setFormat(formats.plain("mono"));
  }

  status(tones: Record<string, Tone>): this {
    return this.setFormat(formats.status(tones));
  }

  /** The value links to a screen (a chip with an arrow). */
  link<P>(screen: ScreenKey<P>, a?: ArgsOf<P>): this {
    this.target = { screen: screen.id, args: some(args(a)) };
    return this;
  }

  /** The value drawn as a chip with initials. */
  chip(): this {
    return this.setFormat(formats.plain("avatar"));
  }

  /** The chip's picture URL or initials. */
  chipAvatar<T>(v: Val<T>): this {
    this.avatarValue = v;
    return this.setFormat(formats.plain("avatar"));
  }

  /** @internal */
  json(): unknown {
    return { label: txt(this.label), value: this.v, format: this.format, open: this.target, avatar: this.avatarValue };
  }
}

/** Label/value pairs of one record. */
function fields(items: readonly ItemB[]): N {
  return n("fields", { items: [...items] });
}

function item<T>(label: Txt, v: Val<T>): ItemB {
  return new ItemB(label, v);
}

function metric<T>(title: Txt, v: Val<T>): N {
  return n("metric", { title: txt(title), value: v });
}

/**
 * A chart over a query's rows: the first charts (catalog 1), kept for compatibility. New screens use `ui.viz`
 * (`ui.viz.bar.column()`, `ui.viz.line.line()`, …), which has every kind of the charts catalogue.
 */
export class ChartB<R> extends Node {
  declare readonly __row: (r: R) => R;
  private readonly seriesList: Record<string, unknown>[] = [];

  constructor(kind: "bar" | "line" | "area" | "pie", query: string, x: string) {
    super("chart", { kind, query, x });
    this.body.series = this.seriesList;
  }

  series<T>(f: FieldRef<R, T>, label: Txt): this {
    this.seriesList.push({ field: f.name, label: txt(label) });
    return this;
  }

  title(t: Txt): this {
    this.body.title = txt(t);
    return this;
  }

  moneyIn(code: string): this {
    this.body.format = formats.money(undefined, code);
    return this;
  }

  number(decimals?: number): this {
    this.body.format = formats.number(decimals);
    return this;
  }

  percent(decimals?: number): this {
    this.body.format = formats.percent(decimals);
    return this;
  }
}

function chartOf(kind: "bar" | "line" | "area" | "pie") {
  return <R, X>(q: Q<any, R>, x: FieldRef<R, X>): ChartB<R> => new ChartB<R>(kind, q.name, x.name);
}

// --- dashboards

/**
 * A dashboard (catalog revision 2): cards on one shared 12-column grid. The items flow left to right and wrap when a
 * row is full; every row is as tall as its tallest card, and a narrower screen widens the cards.
 *
 *   ui.board().quarter(revenue).quarter(orders).half(target).full(ui.heading('Sales')).span(8, line).third(donut)
 *
 * A chart is placed on the board itself (it draws its own card). A board is not put inside another board, and holds
 * at most 60 items. An older client (catalog 1) draws the board's `.fallback(..)`.
 */
export class BoardB extends Node {
  private readonly items: { span?: number; node: Node }[] = [];

  constructor() {
    super("board", {});
    this.body.children = this.items;
  }

  /** An item `columns` wide, of the grid's 12. */
  span(columns: number, n: Node): this {
    this.items.push({ span: columns === 12 ? undefined : columns, node: n });
    return this;
  }

  /** An item of the full width: a wide chart, a table, a heading between groups. */
  full(n: Node): this {
    return this.span(12, n);
  }

  /** An item of half the width (6 columns). */
  half(n: Node): this {
    return this.span(6, n);
  }

  /** An item of a third of the width (4 columns). */
  third(n: Node): this {
    return this.span(4, n);
  }

  /** An item of a quarter of the width (3 columns): a row of four metrics. */
  quarter(n: Node): this {
    return this.span(3, n);
  }
}

function board(): BoardB {
  return new BoardB();
}

// --- charts and figures

/** A family of the charts catalogue (`line`, `bar`, `figure`, …). */
export type VizFamily = keyof typeof VIZ_KINDS;
/** A kind of a family (`VizKind<'bar'>` is `'bar' | 'column' | …`). */
export type VizKind<F extends VizFamily> = (typeof VIZ_KINDS)[F][number];
/** How a column's cells (or a pivot's labels) are read. */
export type VizAs = "number" | "text" | "date";
/** Where a `viz` option's data comes from (one key: `column`, `series`, `pivot`, `rows` or `value`). */
export type VizSource = Record<string, any>;

/**
 * A chart or a figure of the charts catalogue (catalog revision 2): a kind of a family, its static options and the
 * data bound into them from the screen's queries.
 *
 *   ui.viz.line.line().title('Revenue').range('Last 30 days').option('fmt', 'money')
 *     .dates(sales, day.f.day).values(sales, day.f.total).open(ORDERS)
 *
 * The data methods add a bind each, in order; `.to(..)`, `.asText()`, `.asDate()` and `.textKeys(..)` change the
 * bind just added. A chart draws its own card: place it on a `ui.board()`, in columns or in a stack, not next to
 * other children of a card or a section. An older client (catalog 1) draws the node's `.fallback(..)`.
 */
export class VizB extends Node {
  private readonly binds: { to: string; from: VizSource }[] = [];

  constructor(family: string, kind: string) {
    super("viz", { family, kind });
  }

  title(t: Txt): this {
    this.body.title = txt(t);
    return this;
  }

  /** The muted text at the right of the head (`Last 30 days`). */
  range(t: Txt): this {
    this.body.range = txt(t);
    return this;
  }

  /** The height in px (16 to 1200); the kind's own height when not set. */
  height(px: number): this {
    this.body.height = px;
    return this;
  }

  /** A static option of the kind: `.option('fmt', 'money')`, `.option('bands', [50, 80])`. */
  option(name: string, value: unknown): this {
    return this.options({ [name]: value });
  }

  /** Several static options at once: `.options({ fmt: 'money', curve: 'smooth' })`. */
  options(options: Record<string, unknown>): this {
    this.body.options = { ...(this.body.options as Record<string, unknown> | undefined), ...options };
    return this;
  }

  /** A bind written by hand (the typed methods below cover the sources). */
  bind(to: string, from: VizSource): this {
    this.binds.push({ to, from });
    this.body.bind = this.binds;
    return this;
  }

  private last(source: string): Record<string, any> | undefined {
    return this.binds.at(-1)?.from[source];
  }

  /**
   * A row field of every row, as an array in the option `to`. Numbers and numeric strings become numbers;
   * `.asText()` keeps strings, `.asDate()` makes short date labels.
   */
  column<R, T>(to: string, q: Q<any, R>, f: FieldRef<R, T>): this {
    return this.bind(to, { column: { query: q.name, field: f.name } });
  }

  /** The `labels` (the x positions or the row names) from a row field, kept as text. */
  labels<R, T>(q: Q<any, R>, f: FieldRef<R, T>): this {
    return this.column("labels", q, f).asText();
  }

  /** The `labels` from a date or timestamp field, as short dates (`Nov 4`). */
  dates<R, T>(q: Q<any, R>, f: FieldRef<R, T>): this {
    return this.column("labels", q, f).asDate();
  }

  /** The `values` (one series) from a row field. */
  values<R, T>(q: Q<any, R>, f: FieldRef<R, T>): this {
    return this.column("values", q, f);
  }

  /**
   * One more series of the option `series`: the row field with its name in the legend. Calls one after another
   * over the same query (also after `.series(..)`) make one bind.
   */
  serie<R, T>(q: Q<any, R>, f: FieldRef<R, T>, label: Txt): this {
    const s = { field: f.name, label: txt(label) };
    const last = this.last("series");
    if (last && this.binds.at(-1)!.to === "series" && last.query === q.name) {
      last.fields.push(s);
      return this;
    }
    return this.bind("series", { series: { query: q.name, fields: [s] } });
  }

  /** The option `series`, one series per row field: `.series(q, [[day.f.online, 'Online'], [day.f.retail, 'Retail']])`. */
  series<R>(q: Q<any, R>, fields: readonly (readonly [FieldRef<R, any>, Txt])[]): this {
    return this.bind("series", { series: { query: q.name, fields: fields.map(([f, l]) => ({ field: f.name, label: txt(l) })) } });
  }

  /**
   * Long rows to wide: `labels` gets the distinct values of `x` in row order, the option `series` one series per
   * distinct value of `series`, with `value` in its cells (`.asDate()` makes the labels short dates).
   */
  pivot<R>(q: Q<any, R>, x: FieldRef<R, any>, series: FieldRef<R, any>, value: FieldRef<R, any>): this {
    return this.bind("series", { pivot: { query: q.name, x: x.name, series: series.name, value: value.name } });
  }

  /**
   * The option `rows`: an object per row with the keys given, each from a row field:
   * `.rows(top, { name: product.f.title, value: product.f.revenue })`. Numeric strings become numbers except under
   * the keys named by `.textKeys(..)`.
   */
  rows<R>(q: Q<any, R>, map: Record<string, FieldRef<R, any>>): this {
    return this.bind("rows", { rows: { query: q.name, map: Object.fromEntries(Object.entries(map).map(([k, f]) => [k, f.name])) } });
  }

  /** The keys of the `rows` bind just added that stay text (a product named `2024` is not a number). */
  textKeys(keys: readonly string[]): this {
    const rows = this.last("rows");
    if (rows && keys.length > 0) rows.text = [...(rows.text ?? []), ...keys];
    return this;
  }

  /**
   * One value in the option `to` (a name or a dotted path: `center.value`): a query's field
   * (`summary.field(summaryResponse.f.revenue)`), a state, a param or a literal.
   */
  value(to: string, v: ValueLike<unknown>): this {
    return this.bind(to, { value: value(v) });
  }

  /** Another option for the bind just added: `.rows(q, ..).to('parts')`, `.series(q, ..).to('groups')`. */
  to(to: string): this {
    const last = this.binds.at(-1);
    if (last) last.to = to;
    return this;
  }

  private cast(as: VizAs): this {
    const last = this.last("column") ?? this.last("pivot");
    if (last) last.as = as;
    return this;
  }

  /** The `column` just added keeps its cells as text (the labels of a `pivot` too). */
  asText(): this {
    return this.cast("text");
  }

  /** The `column` just added (or the labels of the `pivot`) holds ISO dates or datetimes: they become short labels. */
  asDate(): this {
    return this.cast("date");
  }

  /**
   * The state that receives what the member picks in the chart (a bar, a slice, a row): the picked value, or null
   * when nothing is picked. Queries and conditions of the screen read it like any state.
   */
  selectState(state: StateRef<any>): this {
    this.body.select = { state: state.name };
    return this;
  }

  /** The same, into a param of the screen (it is in the URL). */
  selectParam(param: ParamRef<any>): this {
    this.body.select = { param: param.name };
    return this;
  }

  /** The screen the card's link leads to, with its params like `ui.link` (a literal, a param, a state): `.open(ORDERS, { status: 'paid' })`. */
  open<P>(screen: ScreenKey<P>, a?: ArgsOf<P>): this {
    this.body.open = { screen: screen.id, args: some(args(a)) };
    return this;
  }
}

/** A constructor per kind of the charts catalogue, by family. */
export type VizCatalogue = { readonly [F in VizFamily]: { readonly [K in VizKind<F>]: () => VizB } };

/** A chart or figure by the names of its family and kind (a pair the catalogue has; `plugin check` says when not). */
function vizOf<F extends VizFamily>(family: F, kind: VizKind<F>): VizB;
function vizOf(family: string, kind: string): VizB;
function vizOf(family: string, kind: string): VizB {
  return new VizB(family, kind);
}

/** `ui.viz('line', 'line')`, or a constructor per kind: `ui.viz.line.line()`, `ui.viz.bar.column()`, `ui.viz.figure.metric()`. */
const viz: typeof vizOf & VizCatalogue = Object.assign(
  vizOf,
  Object.fromEntries(
    Object.entries(VIZ_KINDS).map(([family, kinds]) => [family, Object.fromEntries(kinds.map((kind) => [kind, () => new VizB(family, kind)]))]),
  ) as VizCatalogue,
);

/** A timeline over a query's rows: text, time and optionally the actor. */
export class TimelineB<R> extends Node {
  declare readonly __row: (r: R) => R;

  constructor(query: string, text: string, time: string) {
    super("timeline", { query, text, time });
  }

  actor<T>(f: FieldRef<R, T>): this {
    this.body.actor = f.name;
    return this;
  }

  /** The actor's picture URL (else initials). */
  avatar<T>(f: FieldRef<R, T>): this {
    this.body.avatar = f.name;
    return this;
  }

  /** A row field with an icon name for actors without a face. */
  icon<T>(f: FieldRef<R, T>): this {
    this.body.icon = f.name;
    return this;
  }

  /** Times as `2 min`, `40 min`, `2 h`. */
  shortTime(): this {
    this.body.time_format = { relative: { short: true } };
    return this;
  }

  tone<T>(f: FieldRef<R, T>, tones: Tones<T>): this {
    this.body.tone = { field: f.name, tones: tonesJson(tones) };
    return this;
  }
}

function timeline<R, A, B>(q: Q<any, R>, text: FieldRef<R, A>, time: FieldRef<R, B>): TimelineB<R> {
  return new TimelineB<R>(q.name, text.name, time.name);
}

function status<T>(v: Val<T>, tones: Tones<T> = {}): N {
  return n("status", { value: v, tones: tonesJson(tones) });
}

function tag(t: Txt): N {
  return n("tag", { text: txt(t) });
}

function avatar<T>(v: Val<T>): N {
  return n("avatar", { value: v });
}

function avatarStack<T>(v: Val<T>): N {
  return n("avatar_stack", { value: v, max: 5 });
}

function meter<A, B>(v: Val<A>, max: Val<B>): N {
  return n("meter", { value: v, max });
}

/** A fraction 0..1. */
function progress<T>(v: Val<T>): N {
  return n("progress", { value: v });
}

function textNode(t: Txt, style: "body" | "heading" | "caption" | "mono" | "label"): N {
  return n("text", { text: txt(t), style });
}

/** `ui.text(key, en)`: a catalog text; `ui.text(text)`: a body text primitive. */
function text(key: string, en: string, args?: Record<string, ValueLike<unknown>>): TextKey;
function text(t: Txt): N;
function text(a: Txt, en?: string, a2?: Record<string, ValueLike<unknown>>): TextKey | N {
  if (typeof a === "string" && en !== undefined) {
    return new TextKey(a, en, Object.fromEntries(Object.entries(a2 ?? {}).map(([k, v]) => [k, value(v)])));
  }
  return textNode(a, "body");
}

function heading(t: Txt): N {
  return textNode(t, "heading");
}

function caption(t: Txt): N {
  return textNode(t, "caption");
}

function mono(t: Txt): N {
  return textNode(t, "mono");
}

function copyField<T>(label: Txt, v: Val<T>): N {
  return n("copy_field", { value: v, label: txt(label) });
}

function empty(title: Txt, t?: Txt): N {
  return n("empty", { title: txt(title), text: t === undefined ? undefined : txt(t) });
}

function notice(tone: NoticeTone, t: Txt): N {
  return n("notice", { tone, text: txt(t) });
}

// --- actions

/** A button running an action (`ui.button('Cancel', cancel)` or `ui.button('Cancel').run(cancel)`). */
function button(label: Txt, action?: ActionRef): N {
  return n("button", { label: txt(label), action: action?.name ?? "", emphasis: "secondary" });
}

function iconButton(icon: string, label: Txt, action: ActionRef): N {
  return n("icon_button", { icon, label: txt(label), action: action.name });
}

/** A menu item running an action. */
export class MenuItemB extends Built {
  private readonly m: Record<string, unknown>;

  constructor(label: Txt, action: ActionRef) {
    super();
    this.m = { label: txt(label), action: action.name };
  }

  danger(): this {
    this.m.danger = true;
    return this;
  }

  icon(icon: string): this {
    this.m.icon = icon;
    return this;
  }

  /** Shown only when the condition holds. */
  when(c: Cond): this {
    this.m.visible = c;
    return this;
  }

  enabled(c: Cond): this {
    this.m.enabled = c;
    return this;
  }

  /** Starts a new group with this title. */
  section(t: Txt): this {
    this.m.section = txt(t);
    return this;
  }

  /** @internal */
  json(): unknown {
    return this.m;
  }
}

function menuItem(label: Txt, action: ActionRef): MenuItemB {
  return new MenuItemB(label, action);
}

function menu(items: readonly MenuItemB[]): N {
  return n("menu", { icon: "more-horizontal", items: [...items] });
}

// --- input

/** A form or filter input. */
export class InputB extends Built {
  constructor(/** @internal */ readonly i: Record<string, unknown>) {
    super();
  }

  required(): this {
    this.i.required = true;
    return this;
  }

  help(t: Txt): this {
    this.i.help = txt(t);
    return this;
  }

  placeholder(t: Txt): this {
    this.i.placeholder = txt(t);
    return this;
  }

  initial<T>(v: Val<T>): this {
    this.i.initial = v;
    return this;
  }

  /** A literal option of a select, multi-select or radio cards. */
  option(v: unknown, label: Txt): this {
    const options = (this.i.options as unknown[] | undefined) ?? [];
    options.push({ value: encode(v), label: txt(label) });
    this.i.options = options;
    return this;
  }

  /** `repeat`: rows at least / at most. */
  items(min?: number, max?: number): this {
    this.i.min_items = min;
    this.i.max_items = max;
    return this;
  }

  /** A filter chip's icon. */
  icon(icon: string): this {
    this.i.icon = icon;
    return this;
  }

  /** Shares its line with the next half input. */
  half(): this {
    this.i.width = "half";
    return this;
  }

  /** A number with − and + buttons. */
  stepper(): this {
    this.i.stepper = true;
    return this;
  }

  /** `repeat`: a value at the end of each row from the i-th row of `q` (a line total from a quote). */
  aside<R, V>(title: Txt, q: Q<any, R>, f: FieldRef<R, V>, moneyIn: string): this {
    this.i.aside = { title: txt(title), query: q.name, field: f.name, format: formats.money(undefined, moneyIn) };
    return this;
  }

  /** `repeat`: the add row's label. */
  addLabel(t: Txt): this {
    this.i.add_label = txt(t);
    return this;
  }

  /** @internal */
  json(): unknown {
    return this.i;
  }
}

/** Inputs that set params (a filter bar over a list screen). */
function filterBar(inputs: readonly InputB[]): N {
  return n("filter_bar", { inputs: [...inputs] });
}

/** The inputs of a form over `Req`. */
export class FormB<Req> {
  declare readonly __request: (r: Req) => Req;
  /** @internal */ readonly sections: { title?: unknown; inputs: InputB[] }[] = [{ inputs: [] }];
  /** @internal */ cancelLabel?: unknown;
  /** @internal */ phoneSubmitLabel?: unknown;
  /** @internal */ summaryValue?: Record<string, unknown>;
  /** @internal */ noteText?: unknown;
  /** @internal */ alternateButton?: { label: unknown; chain: Chain };
  /** @internal */ readonly notes: { text: unknown; visible?: Cond }[] = [];

  private input(f: FieldRef<Req, unknown>, control: string, label: Txt): InputB {
    const i = new InputB({ name: f.name, control, label: txt(label) });
    this.sections[this.sections.length - 1].inputs.push(i);
    return i;
  }

  /** Starts a titled section. */
  section(title: Txt): this {
    this.sections.push({ title: txt(title), inputs: [] });
    return this;
  }

  cancel(label: Txt): this {
    this.cancelLabel = txt(label);
    return this;
  }

  /** The submit button's label on a phone, where the form is a sheet (absent: the submit label). */
  phoneSubmit(label: Txt): this {
    this.phoneSubmitLabel = txt(label);
    return this;
  }

  text<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    return this.input(f, "text", label);
  }

  textarea<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    return this.input(f, "textarea", label);
  }

  number<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    return this.input(f, "number", label);
  }

  money<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    return this.input(f, "money", label);
  }

  date<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    return this.input(f, "date", label);
  }

  datetime<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    return this.input(f, "datetime", label);
  }

  checkbox(f: FieldRef<Req, boolean>, label: Txt): InputB {
    return this.input(f, "checkbox", label);
  }

  switch(f: FieldRef<Req, boolean>, label: Txt): InputB {
    return this.input(f, "switch", label);
  }

  member<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    return this.input(f, "member", label);
  }

  upload<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    return this.input(f, "upload", label);
  }

  search<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    return this.input(f, "search", label);
  }

  /** A select over an enum's values (or `.option(..)`). */
  select<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    const i = this.input(f, "select", label);
    i.i.options = some(enumOptions(f.field));
    return i;
  }

  /** A multi-select over a repeated enum's values (or `.option(..)`). */
  multiSelect<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    const i = this.input(f, "multi_select", label);
    i.i.options = some(enumOptions(f.field));
    return i;
  }

  /** Radio cards over an enum's values. */
  radio<T>(f: FieldRef<Req, T>, label: Txt): InputB {
    const i = this.input(f, "radio_cards", label);
    i.i.options = some(enumOptions(f.field));
    return i;
  }

  /** A select whose options come from a query's rows. */
  selectFrom<T, R, V, L>(f: FieldRef<Req, T>, label: Txt, q: Q<any, R>, v: FieldRef<R, V>, shown: FieldRef<R, L>): InputB {
    const i = this.input(f, "select", label);
    i.i.options_from = { query: q.name, value: v.name, label: shown.name };
    return i;
  }

  /** Rows of a repeated message field (`Items`: product, quantity per row). */
  repeat<T>(f: FieldRef<Req, T[]>, label: Txt, row: (r: RowForm<T>) => unknown): InputB {
    const rf = new RowForm<T>();
    row(rf);
    const i = this.input(f, "repeat", label);
    i.i.fields = some(rf.inputs);
    i.i.min_items = 1;
    return i;
  }

  /** A total under the inputs. */
  summary<T>(title: Txt, caption: Txt | undefined, v: Val<T>, moneyIn: string): this {
    this.summaryValue = {
      title: txt(title),
      caption: caption === undefined ? undefined : txt(caption),
      value: v,
      format: formats.money(undefined, moneyIn),
    };
    return this;
  }

  /** A line next to the buttons. */
  note(t: Txt): this {
    this.noteText = txt(t);
    return this;
  }

  /** A second button that sends the values another way (`Save draft`). */
  alternate(label: Txt, build: (a: Chain) => unknown): this {
    const c = new Chain();
    build(c);
    this.alternateButton = { label: txt(label), chain: c };
    return this;
  }

  /**
   * Chips over a directory (catalog 3): the rows of `q`, each chosen by `value` and drawn with `title`. The field gets
   * the chosen values joined with `, `; `ui.pickedCount(f)`, `ui.pickedAgents(f)` and `ui.pickedFirst(f)` read the
   * choice in conditions and texts, `ui.pickedQuery(f)` what is typed (in the `empty` text).
   */
  pick<T, R, V, L>(f: FieldRef<Req, T>, label: Txt, q: Q<any, R>, value: FieldRef<R, V>, title: FieldRef<R, L>): PickB<R> {
    const i = new PickB<R>({ name: f.name, control: "pick", label: txt(label), pick: { query: q.name, value: value.name, label: title.name } });
    this.sections[this.sections.length - 1].inputs.push(i);
    return i;
  }

  /** A line under the inputs while `c` (over the form's values) holds (catalog 3). */
  noteWhen(t: Txt, c: Cond): this {
    this.notes.push({ text: txt(t), visible: c });
    return this;
  }
}

/** A `pick` input: its directory's fields and texts (catalog 3). */
export class PickB<R> extends InputB {
  declare readonly __row: (r: R) => R;

  private pick(): Record<string, unknown> {
    return this.i.pick as Record<string, unknown>;
  }

  avatar<T>(f: FieldRef<R, T>): this {
    this.pick().avatar = f.name;
    return this;
  }

  /** A bool field: the row is an agent (drawn with its tile; counted by `ui.pickedAgents`). */
  agent(f: FieldRef<R, boolean>): this {
    this.pick().agent = f.name;
    return this;
  }

  /** The agent tile: the glyph and the engine. */
  tile<A, B>(icon: FieldRef<R, A>, engine: FieldRef<R, B>): this {
    Object.assign(this.pick(), { icon: icon.name, engine: engine.name });
    return this;
  }

  detail<T>(f: FieldRef<R, T>): this {
    this.pick().detail = f.name;
    return this;
  }

  /** The rows are grouped by this field. */
  section<T>(f: FieldRef<R, T>): this {
    this.pick().section = f.name;
    return this;
  }

  /** A list value of values not offered (the members already in). */
  exclude<T>(v: ValueLike<T>): this {
    this.pick().exclude = value(v);
    return this;
  }

  /** The line when nothing matches the typed text, and when every row is chosen. */
  empty(nothing: Txt, all: Txt): this {
    Object.assign(this.pick(), { empty: txt(nothing), empty_done: txt(all) });
    return this;
  }
}

/** The inputs of one row of a `repeat`. */
export class RowForm<T> {
  declare readonly __row: (r: T) => T;
  /** @internal */ readonly inputs: InputB[] = [];

  private push(f: FieldRef<T, unknown>, control: string, label: Txt): InputB {
    const i = new InputB({ name: f.name, control, label: txt(label) });
    this.inputs.push(i);
    return i;
  }

  /** A select from a query's rows. */
  selectFrom<V, R, A, L>(f: FieldRef<T, V>, label: Txt, q: Q<any, R>, value: FieldRef<R, A>, shown: FieldRef<R, L>): this {
    const i = this.push(f, "select", label);
    i.i.required = true;
    i.i.options_from = { query: q.name, value: value.name, label: shown.name };
    return this;
  }

  /** A number with − and + buttons, starting at `initial`. */
  stepper<V>(f: FieldRef<T, V>, label: Txt, initial: number): this {
    const i = this.push(f, "number", label);
    i.i.required = true;
    i.i.initial = { lit: initial };
    i.i.stepper = true;
    return this;
  }
}

// ---------------------------------------------------------------------------------------------- contributions

/** A sidebar entry: a screen with its params, an optional icon and badge. */
export class EntryB extends Built {
  private iconName?: string;
  private badgeValue?: unknown;
  private alertBadge = false;
  /** @internal */ badgeQuery?: QueryJson;

  constructor(
    private readonly screen: ScreenKey<any>,
    private readonly title: Txt,
    private readonly args: ArgJson[],
  ) {
    super();
  }

  icon(icon: string): this {
    this.iconName = icon;
    return this;
  }

  /** A count from a document-level query (`badge_<field>`), live through the topics it follows. */
  badge<Req, Resp, Row, T>(def: QueryDef<Req, Resp, Row>, count: FieldRef<Resp, T>): this {
    const q = copyQuery(def.q);
    q.name = `badge_${count.name}`;
    for (const f of q.follow) f.min_interval_ms ??= 2000;
    this.badgeQuery = q;
    this.badgeValue = { data: { query: q.name, field: count.name } };
    return this;
  }

  /** The badge asks for attention (a filled pill) instead of a quiet count. */
  alert(): this {
    this.alertBadge = true;
    return this;
  }

  /** @internal */ rowsJson?: Record<string, unknown>;

  /** Rows: each row's icon field (`hash`, an agent's `bot`). */
  rowIcon<R, T>(f: FieldRef<R, T>): this {
    if (this.rowsJson) this.rowsJson.icon = f.name;
    return this;
  }

  /** Rows: the unread count of each row (its label turns bold) and the viewer's mentions (an amber pill). */
  unread<R, T, M>(unread: FieldRef<R, T>, mentions: FieldRef<R, M>): this {
    if (this.rowsJson) Object.assign(this.rowsJson, { badge: unread.name, mentions: mentions.name });
    return this;
  }

  /** Rows: how many show before "Show all" (1 to 20). */
  limit(n: number): this {
    if (this.rowsJson) this.rowsJson.limit = n;
    return this;
  }

  /** Rows: an add row under them (`New channel`) that opens a document-level overlay. */
  add<P>(title: Txt, overlay: OverlayKey<P>): this {
    if (this.rowsJson) this.rowsJson.add = { title: txt(title), overlay: overlay.id };
    return this;
  }

  /** @internal */
  json(): unknown {
    if (this.rowsJson) return { title: txt(this.title), icon: this.iconName, rows: this.rowsJson };
    return {
      title: txt(this.title),
      icon: this.iconName,
      screen: this.screen.id,
      args: some(this.args),
      badge: this.badgeValue,
      badge_style: this.alertBadge ? "alert" : undefined,
    };
  }
}

function entry<P>(screen: ScreenKey<P>, title: Txt, a?: ArgsOf<P>): EntryB {
  return new EntryB(screen, title, args(a));
}

/** Entries from a query's rows (channels, direct messages): `ui.entryRows('Channels', def, channel.f.name, CHANNEL,
 * { c: ui.rowValue(channel.f.id) })`, five shown before "Show all". */
function entryRows<Req, Resp, R, L, P>(title: Txt, def: QueryDef<Req, Resp, R>, label: FieldRef<R, L>, screen: ScreenKey<P>, a?: ArgsOf<P>): EntryB {
  const e = new EntryB(screen, title, []);
  const q = copyQuery(def.q);
  q.name = `rows_${label.name}`;
  for (const f of q.follow) f.min_interval_ms ??= 2000;
  e.badgeQuery = q;
  e.rowsJson = { query: q.name, label: label.name, screen: screen.id, args: some(args(a)), limit: 5 };
  return e;
}

/** A sidebar group: `ui.nav('Orders', 'shopping-bag', [ui.entry(..)])`. */
export class NavB extends Built {
  private iconName?: string;
  /** @internal */ readonly entries: EntryB[];

  constructor(
    private readonly title: Txt,
    icon?: string,
    entries: readonly EntryB[] = [],
  ) {
    super();
    this.iconName = icon;
    this.entries = [...entries];
  }

  icon(icon: string): this {
    this.iconName = icon;
    return this;
  }

  entry(e: EntryB): this {
    this.entries.push(e);
    return this;
  }

  /** @internal */
  json(): unknown {
    return { title: txt(this.title), icon: this.iconName, entries: some(this.entries) };
  }
}

function nav(title: Txt, icon?: string, entries?: readonly EntryB[]): NavB {
  return new NavB(title, icon, entries);
}

/** A command-palette entry. */
export class CommandB extends Built {
  private iconName?: string;

  constructor(
    private readonly id: string,
    private readonly title: Txt,
    private readonly target: Record<string, unknown>,
  ) {
    super();
  }

  icon(icon: string): this {
    this.iconName = icon;
    return this;
  }

  /** @internal */
  json(): unknown {
    return { id: this.id, title: txt(this.title), icon: this.iconName, target: this.target };
  }
}

const command = {
  /**
   * Navigates to a screen, with params like a link: `ui.command.navigate('week', 'Sales: last 7 days', REPORT,
   * { range: '7' })`. A command runs from anywhere, so its args are literals.
   */
  navigate: <P>(id: string, title: Txt, screen: ScreenKey<P>, a?: ArgsOf<P>): CommandB =>
    new CommandB(id, title, { screen: screen.id, args: some(args(a)) }),
  /** Opens a document-level overlay (`ui.define({ overlays })`), with params (literals). */
  open: <P>(id: string, title: Txt, overlay: OverlayKey<P>, a?: ArgsOf<P>): CommandB =>
    new CommandB(id, title, { overlay: overlay.id, args: some(args(a)) }),
};

/** The builder a home widget's function gets: its queries are document-level (`home_<id>_<name>`). */
export class WidgetBuilder {
  /** @internal */ readonly queries: QueryJson[] = [];

  constructor(private readonly prefix: string) {}

  query<Req, Resp, Row>(name: string, def: QueryDef<Req, Resp, Row>): Q<Resp, Row> {
    const q = copyQuery(def.q);
    q.name = `${this.prefix}_${name}`;
    for (const f of q.follow) f.min_interval_ms ??= 2000;
    this.queries.push(q);
    return new Q(q.name);
  }
}

/** A workspace home widget. */
export class HomeB {
  private target?: Record<string, unknown>;

  constructor(
    readonly id: string,
    private readonly title: Txt,
    private readonly build: (w: WidgetBuilder) => Node,
  ) {}

  /** The screen the widget opens. */
  opens<P>(screen: ScreenKey<P>, a?: ArgsOf<P>): this {
    this.target = { screen: screen.id, args: some(args(a)) };
    return this;
  }

  /** @internal */
  json(): { widget: unknown; queries: QueryJson[] } {
    const w = new WidgetBuilder(`home_${this.id}`);
    const node = this.build(w);
    return { widget: { id: this.id, title: txt(this.title), node, open: this.target }, queries: w.queries };
  }
}

function home(id: string, title: Txt, build: (w: WidgetBuilder) => Node): HomeB {
  return new HomeB(id, title, build);
}

export interface UiOptions {
  /** Sidebar groups; their entries' badge queries become document-level queries. */
  nav?: readonly NavB[];
  /** The screen the plugin opens on; it takes no required param (`E_UI_LANDING`). Without it, the client opens the
   *  screen of the navigation's first entry that has one. */
  landing?: ScreenKey<any>;
  home?: readonly HomeB[];
  commands?: readonly CommandB[];
  screens?: readonly ScreenDef<any>[];
  /** Overlays opened from anywhere (a command, a nav "add" row). */
  overlays?: readonly OverlayDef<any>[];
  /** Records the command palette finds (`ui.search('Orders', findOrders, findRequest.f.text, findResponse.f.orders)`). */
  search?: readonly SearchB<any>[];
  /** Texts in other locales: `{ de: { 'nav.all': 'Alle Bestellungen' } }` (`en` comes from `ui.text(key, en)`). */
  texts?: Record<string, Record<string, string>>;
}

/** The plugin's UI, for `plugin({ ui })`. */
export class UiDoc {
  constructor(
    /** @internal */ readonly tree: Record<string, unknown>,
    /** @internal */ readonly texts: Record<string, Record<string, string>>,
  ) {}
}

/** The plugin's UI: `ui.define({ nav, screens, commands, home, overlays })`. Screens are built here, in order. */
function define(options: UiOptions): UiDoc {
  const queries: QueryJson[] = [];
  const addQuery = (q: QueryJson) => {
    if (!queries.some((x) => x.name === q.name)) queries.push(q);
  };
  for (const g of options.nav ?? []) for (const e of g.entries) if (e.badgeQuery) addQuery(e.badgeQuery);
  const home = (options.home ?? []).map((h) => {
    const { widget, queries: qs } = h.json();
    qs.forEach(addQuery);
    return widget;
  });
  const tree = {
    nav: some(options.nav ?? []),
    landing: options.landing?.id,
    home: some(home),
    commands: some(options.commands ?? []),
    queries: some(queries.map(queryJson)),
    screens: some((options.screens ?? []).map((s) => s.json())),
    overlays: some((options.overlays ?? []).map((o) => o.json())),
    search: some(options.search ?? []),
  };
  return new UiDoc(tree, Object.fromEntries(Object.entries(options.texts ?? {}).map(([k, v]) => [k, { ...v }])));
}

/** @internal The document as the descriptor carries it: deferred parts resolved, texts collected. */
export function resolveUi(doc: UiDoc, r: UiResolver): Record<string, unknown> {
  const en = new Map<string, string>();
  const walk = (v: unknown, path: string): unknown => {
    if (v instanceof Deferred) return walk(v.resolve(r, path), path);
    if (v instanceof Built) return walk(v.json(), path);
    if (v instanceof TextKey) {
      en.set(v.key, v.en);
      const args = walk(v.args, `${path}.args`) as Record<string, unknown>;
      return Object.keys(args).length > 0 ? { key: v.key, args } : { key: v.key };
    }
    if (Array.isArray(v)) return v.map((x, i) => walk(x, `${path}[${i}]`));
    if (v !== null && typeof v === "object") {
      const out: Record<string, unknown> = {};
      for (const [k, x] of Object.entries(v)) if (x !== undefined) out[k] = walk(x, `${path}.${k}`);
      return out;
    }
    return v;
  };
  const out = walk(doc.tree, "ui") as Record<string, unknown>;
  const texts: Record<string, Record<string, string>> = {};
  for (const [locale, catalog] of Object.entries(doc.texts)) texts[locale] = { ...catalog };
  if (en.size > 0) {
    const catalog = (texts.en ??= {});
    for (const [k, v] of en) if (!(k in catalog)) catalog[k] = v;
  }
  return { catalog: CATALOG, ...out, ...(Object.keys(texts).length > 0 ? { texts } : {}) };
}

// ---------------------------------------------------------------------------------------------- records

/** A row as a list item: `r.view(r.get(order.f.customer)).subtitle(..).meta(..)` (a compact table row, a search result). */
export class RowViewB<R> extends Built {
  declare readonly __row: (r: R) => R;
  private readonly v: Record<string, unknown>;

  constructor(title: Txt) {
    super();
    this.v = { title: txt(title) };
  }

  subtitle(t: Txt): this {
    this.v.subtitle = txt(t);
    return this;
  }

  meta<T>(v: Val<T>): this {
    this.v.meta = v;
    return this;
  }

  /** The meta is money in the row's currency field. */
  money<C>(currency: FieldRef<R, C>): this {
    this.v.meta_format = formats.money(currency.name);
    return this;
  }

  avatar<T>(f: FieldRef<R, T>): this {
    this.v.avatar = f.name;
    return this;
  }

  icon(icon: string): this {
    this.v.icon = icon;
    return this;
  }

  status<T>(f: FieldRef<R, T>, tones: Tones<T>): this {
    this.v.status = { field: f.name, tones: tonesJson(tones) };
    return this;
  }

  /** @internal */
  json(): unknown {
    return this.v;
  }
}

/** A row as a list item; inside `compact`/`item` callbacks `r.view(..)` infers the row type. */
function rowView<R>(title: Txt): RowViewB<R> {
  return new RowViewB<R>(title);
}

/** The head of a record: `ui.recordHeader(title).short(..).subtitle(..).tags([..]).actions([..]).menu([..])`. */
export class RecordHeaderB extends Node {
  constructor(title: Txt) {
    super("record_header", { title: txt(title) });
  }

  /** The top bar's title on a phone (`#1048`). */
  short(t: Txt): this {
    this.body.short = txt(t);
    return this;
  }

  subtitle(t: Txt): this {
    this.body.subtitle = txt(t);
    return this;
  }

  /** The heading and the line under it on a phone. */
  compact(title: Txt, subtitle: Txt): this {
    this.body.compact_title = txt(title);
    this.body.compact_subtitle = txt(subtitle);
    return this;
  }

  /** Status chips next to the title. */
  tags(nodes: readonly Node[]): this {
    this.body.tags = some(nodes);
    return this;
  }

  /** Buttons under the title. */
  actions(nodes: readonly Node[]): this {
    this.body.actions = some(nodes);
    return this;
  }

  /** More actions (`…`). */
  menu(items: readonly MenuItemB[]): this {
    this.body.menu = { label: "More actions", icon: "more-horizontal", items: [...items] };
    return this;
  }

  /** The action of the panel's expand button. */
  expand(action: ActionRef): this {
    this.body.expand = action.name;
    return this;
  }

  /** The action of the panel's close button. */
  close(action: ActionRef): this {
    this.body.close = action.name;
    return this;
  }
}

function recordHeader(title: Txt): RecordHeaderB {
  return new RecordHeaderB(title);
}

/** An agent's run on a record: who, its state, its steps, Stop and a message box. */
export class AgentRunB<Resp> extends Node {
  declare readonly __response: (r: Resp) => Resp;

  constructor(body: Record<string, unknown>) {
    super("agent_run", body);
  }

  /** The line under the agent's name. */
  subtitle<T>(f: FieldRef<Resp, T>): this {
    this.body.subtitle = f.name;
    return this;
  }

  /** A Stop button, shown while `when` holds. */
  stop(action: ActionRef, when: Cond): this {
    this.body.stop = action.name;
    this.body.stop_when = when;
    return this;
  }

  /** A message box; the action reads the text as `ui.formValue(..)` of the field `text`. */
  message(placeholder: Txt, action: ActionRef): this {
    this.body.message = { placeholder: txt(placeholder), action: action.name };
    return this;
  }

  /** What the run needs from the viewer, first and in amber: the request (a run field, empty when nothing is needed)
   * and one button; `due` is the deadline next to it (`by 11:30`). */
  needs<T, D>(text: FieldRef<Resp, T>, label: Txt, action: ActionRef, due?: FieldRef<Resp, D>): this {
    this.body.needs = { text: text.name, label: txt(label), action: action.name, ...(due ? { due: due.name } : {}) };
    return this;
  }

  /** Opens the run's session page; the card then shows the current step instead of the step list. */
  open(label: Txt, action: ActionRef): this {
    this.body.open = { label: txt(label), action: action.name };
    return this;
  }

  /** The changes the run made to this record: `changes('Changes to this order', run.f.changes, change.f.text)`. */
  changes<C, T>(title: Txt, field: FieldRef<Resp, C[]>, text: FieldRef<C, T>): this {
    this.body.changes = { title: txt(title), field: field.name, text: text.name };
    return this;
  }

  /** The agent's earlier sessions on this record: a run field with the line, hidden while empty, and its action. */
  earlier<T>(text: FieldRef<Resp, T>, action: ActionRef): this {
    this.body.earlier = { text: text.name, action: action.name };
    return this;
  }

  /** The run's full log in a panel (catalog 3): the head, every step, the note. */
  log(): this {
    this.body.log = true;
    return this;
  }

  /** The agent's tile in the log's head: response fields with the glyph and the engine (catalog 3). */
  tile<A, B>(icon: FieldRef<Resp, A>, engine: FieldRef<Resp, B>): this {
    this.body.icon = icon.name;
    this.body.engine = engine.name;
    return this;
  }

  /** The status's label in the log's head (catalog 3). */
  statusLabel<T>(f: FieldRef<Resp, T>): this {
    this.body.status_label = f.name;
    return this;
  }

  /** The current step's label in the log's head (catalog 3). */
  stepLabel<T>(f: FieldRef<Resp, T>): this {
    this.body.step_label = f.name;
    return this;
  }

  /** How long the run has worked, in the log's head (catalog 3). */
  elapsed<T>(f: FieldRef<Resp, T>): this {
    this.body.elapsed = f.name;
    return this;
  }

  /** The line under the steps while the run is not live (catalog 3). */
  note<T>(f: FieldRef<Resp, T>): this {
    this.body.note = f.name;
    return this;
  }

  /** While this holds the run is live (no note; catalog 3). */
  liveWhen(c: Cond): this {
    this.body.live_when = c;
    return this;
  }

  /** A step's detail line (a field of a step; catalog 3). */
  stepDetail<S, T>(f: FieldRef<S, T>): this {
    this.body.step_detail = f.name;
    return this;
  }

  /** A step's time (catalog 3). */
  stepTime<S, T>(f: FieldRef<S, T>): this {
    this.body.step_time = f.name;
    return this;
  }

  /** A step's call, drawn as a process row's (catalog 3). */
  stepTool<S, T>(f: FieldRef<S, T>): this {
    this.body.step_tool = f.name;
    return this;
  }

  /** A step's evidence (catalog 3). */
  stepEvidence<S, T>(f: FieldRef<S, T>): this {
    this.body.step_evidence = f.name;
    return this;
  }

  /** A step's lines (catalog 3). */
  stepLines<S, T>(f: FieldRef<S, T>): this {
    this.body.step_lines = f.name;
    return this;
  }
}

/** `ui.agentRun(run, runResponse.f.agent, runResponse.f.status, tones, runResponse.f.steps, step.f.text, step.f.state)`. */
function agentRun<Resp, A, E, S, B, C>(
  q: Q<Resp, any>,
  title: FieldRef<Resp, A>,
  status: FieldRef<Resp, E>,
  tones: Tones<E>,
  steps: FieldRef<Resp, S[]>,
  stepText: FieldRef<S, B>,
  stepState: FieldRef<S, C>,
): AgentRunB<Resp> {
  return new AgentRunB<Resp>({
    query: q.name,
    title: title.name,
    status: { field: status.name, tones: tonesJson(tones) },
    steps: steps.name,
    step_text: stepText.name,
    step_state: stepState.name,
  });
}

/** A conversation (a channel, a thread, a direct message): its messages oldest first, a card per message and a
 * message box. */
export class ConversationB<R> extends Node {
  declare readonly __row: (r: R) => R;

  constructor(query: string, author: string, text: string, time: string) {
    super("conversation", { query, author, text, time });
  }

  private cardBody(): Record<string, unknown> {
    return (this.body.card ??= { title: "" }) as Record<string, unknown>;
  }

  /** The author's picture URL (else their initials). */
  avatar<T>(f: FieldRef<R, T>): this {
    this.body.avatar = f.name;
    return this;
  }

  /** An icon name for authors without a face (an agent: `bot`). */
  icon<T>(f: FieldRef<R, T>): this {
    this.body.icon = f.name;
    return this;
  }

  /** A small chip after the author (`AGENT`). */
  badge<T>(f: FieldRef<R, T>): this {
    this.body.badge = f.name;
    return this;
  }

  /** A quiet line after the time (`via Commerce`). */
  via<T>(f: FieldRef<R, T>): this {
    this.body.via = f.name;
    return this;
  }

  /** A chip before the text (`To people`). */
  tag<T>(f: FieldRef<R, T>): this {
    this.body.tag = f.name;
    return this;
  }

  /** The receipt under a message (`Applied · step 3`), toned by a field. */
  receipt<T, E>(text: FieldRef<R, T>, tone: FieldRef<R, E>, tones: Tones<E>): this {
    this.body.receipt = { text: text.name, tone: { field: tone.name, tones: tonesJson(tones) } };
    return this;
  }

  /** A card under the message, shown while its title is not empty, with its text. */
  card<A, B>(title: FieldRef<R, A>, text: FieldRef<R, B>): this {
    Object.assign(this.cardBody(), { title: title.name, text: text.name });
    return this;
  }

  /** The card's band and its aside (`by 11:30`), toned by a field. */
  cardBand<A, B, E>(band: FieldRef<R, A>, aside: FieldRef<R, B>, tone: FieldRef<R, E>, tones: Tones<E>): this {
    Object.assign(this.cardBody(), { band: band.name, aside: aside.name, tone: { field: tone.name, tones: tonesJson(tones) } });
    return this;
  }

  /** The card's footer line (`14 replies · last 10:58`). */
  cardFooter<T>(f: FieldRef<R, T>): this {
    this.cardBody().footer = f.name;
    return this;
  }

  /** The card's button: its label is a row field (hidden while empty), the action runs on the row. */
  cardButton<T>(label: FieldRef<R, T>, action: ActionRef): this {
    Object.assign(this.cardBody(), { button: label.name, action: action.name });
    return this;
  }

  /** The message box; its action reads `ui.formValue(..)` of the fields `text` and, with addressees, `to`. */
  composer(placeholder: Txt, action: ActionRef): this {
    this.body.composer = { placeholder: txt(placeholder), action: action.name };
    return this;
  }

  /** An addressee of the message box; the first is the default. */
  to(value: string, label: Txt): this {
    const c = this.body.composer as Record<string, unknown> | undefined;
    if (c) ((c.to ??= []) as unknown[]).push({ value, label: txt(label) });
    return this;
  }

  empty(t: Txt): this {
    this.body.empty = txt(t);
    return this;
  }
}

/** `ui.conversation(messages, msg.f.author, msg.f.text, msg.f.at)`. */
function conversation<R, A, B, C>(q: Q<any, R>, author: FieldRef<R, A>, text: FieldRef<R, B>, time: FieldRef<R, C>): ConversationB<R> {
  return new ConversationB<R>(q.name, author.name, text.name, time.name);
}

/** A messenger screen (`ui.chat(channel, send)`): the open thread, a run's log and the members as side panels and
 * the actions on messages.
 * @deprecated Catalog 3 deprecates `chat` (`W_UI_DEPRECATED`); build the screen from a `fill` split with a
 * `ui.header`, `ui.stream`s of posts, a `ui.strip` and a `ui.composer`. */
export class ChatB extends Node {
  constructor(channel: string, send: string) {
    super("chat", { channel, actions: { send } });
  }

  private act(o: Record<string, string>): this {
    Object.assign(this.body.actions as Record<string, string>, o);
    return this;
  }

  /** The open thread (`root`, `replies`) and the reply action (`{form: text, to, also}`). */
  thread(q: Q<any, any>, reply: ActionRef): this {
    this.body.thread = q.name;
    return this.act({ reply: reply.name });
  }

  /** An agent run's log (`steps`), with Stop and a note to the agent (`{form: text}`). */
  run(q: Q<any, any>, stop: ActionRef, tell: ActionRef): this {
    this.body.run = q.name;
    return this.act({ stop: stop.name, tell: tell.name });
  }

  /** The members (`people`, `agents`) and the action that opens them. */
  members(q: Q<any, any>, open: ActionRef): this {
    this.body.members = q.name;
    return this.act({ members: open.name });
  }

  /** Opens a message's thread, and closes the side panel. */
  openThread(open: ActionRef, close: ActionRef): this {
    return this.act({ thread: open.name, close: close.name });
  }

  /**
   * The open thread's Following bell: the action follows the thread (`{form: follow}` true) or stops following it
   * (false); the thread query reads `following`, whether the reader follows it.
   */
  followThread(a: ActionRef): this {
    return this.act({ follow: a.name });
  }

  /** Opens a message's run log. */
  openRun(a: ActionRef): this {
    return this.act({ run: a.name });
  }

  /** Approve, Decline and Undo on a message's approval. */
  approval(approve: ActionRef, decline: ActionRef, undo: ActionRef): this {
    return this.act({ approve: approve.name, decline: decline.name, undo: undo.name });
  }

  /** A message card's button. */
  card(a: ActionRef): this {
    return this.act({ card: a.name });
  }

  /** Reactions on a message (`{form: emoji}`). */
  react(a: ActionRef): this {
    return this.act({ react: a.name });
  }

  /** The author's own changes to a message (edit `{form: text}`, delete), and pin or unpin it. */
  editing(edit: ActionRef, del: ActionRef, pin: ActionRef): this {
    return this.act({ edit: edit.name, delete: del.name, pin: pin.name });
  }

  /** Chooses an option of a message's decision (`{form: choice}`). */
  choose(a: ActionRef): this {
    return this.act({ choose: a.name });
  }

  /** The agent a direct message is with (`name`) and the action that pauses or resumes it. */
  about(q: Q<any, any>, pause: ActionRef): this {
    this.body.about = q.name;
    return this.act({ pause: pause.name });
  }

  /** Whom a new message can go to (`people`, `agents`) and the action that starts it (`{form: to, text}`). */
  directory(q: Q<any, any>, start: ActionRef): this {
    this.body.directory = q.name;
    return this.act({ start: start.name });
  }
}

/** @deprecated Catalog 3 deprecates `chat`: a `fill` split with `ui.header`, `ui.stream`, `ui.strip` and `ui.composer`. */
function chat(channel: Q<any, any>, send: ActionRef): ChatB {
  return new ChatB(channel.name, send.name);
}

/** Records the command palette finds: `ui.search('Orders', findOrders, findRequest.f.text, findResponse.f.orders)`. */
export class SearchB<R> extends Built {
  declare readonly __row: (r: R) => R;
  private itemView: RowViewB<R> = new RowViewB<R>("");
  private target: Record<string, unknown> = {};
  private readonly actions: Record<string, unknown>[] = [];

  constructor(
    private readonly title: Txt,
    private readonly endpoint: Deferred,
    private readonly text: string,
    private readonly rows: string,
  ) {
    super();
  }

  /** A result as a list item (`(r) => r.view(r.get(order.f.number)).subtitle(..)`). */
  item(view: (r: RowRef<R>) => RowViewB<R>): this {
    this.itemView = view(new RowRef<R>());
    return this;
  }

  /** Where a result leads (`(r) => ({ id: r.get(order.f.id) })`). */
  open<P>(screen: ScreenKey<P>, a?: (r: RowRef<R>) => ArgsOf<P>): this {
    this.target = { screen: screen.id, args: some(args(a?.(new RowRef<R>()))) };
    return this;
  }

  /** An action the palette offers on the top result: a document-level overlay with args from the result (`New order for {name}`). */
  action<P>(title: Txt, icon: string, overlay: OverlayKey<P>, a?: (r: RowRef<R>) => ArgsOf<P>): this {
    this.actions.push({ title: txt(title), icon, target: { overlay: overlay.id, args: some(args(a?.(new RowRef<R>()))) } });
    return this;
  }

  /** @internal */
  json(): unknown {
    return {
      title: txt(this.title),
      endpoint: this.endpoint,
      text: this.text,
      rows: this.rows,
      item: this.itemView,
      open: this.target,
      actions: some(this.actions),
    };
  }
}

/** A source of records for the command palette: a read `ui` endpoint called with the typed text in `text`. */
function search<I extends IoType, O extends IoType, R>(
  title: Txt,
  endpoint: EndpointDef<I, O>,
  text: FieldRef<IoValue<I>, string>,
  rows: FieldRef<IoValue<O>, R[]>,
): SearchB<R> {
  return new SearchB<R>(title, endpointRef(endpoint), text.name, rows.name);
}

/** A card with a title line (`3 items · from catalog`); `.trailing(v).moneyIn(..)` adds a value at its end. */
function titledCard(title: Txt, children: readonly Node[]): N {
  return n("card", { title: txt(title), children: [...children] });
}

/** A small uppercase label over a group (`ACTIVITY`). */
function label(t: Txt): N {
  return textNode(t, "label");
}

/** A menu opened by a labelled button (`View`). */
function menuButton(labelText: Txt, icon: string, items: readonly MenuItemB[]): N {
  return n("menu", { label: txt(labelText), icon, items: [...items] });
}

// ---------------------------------------------------------------------------------------------- catalog 3
//
// The app-like primitives: a `stream` of posts with a `process` slot, a
// `strip` of live items, a `composer`, a `header`, the `pick` input; the options of `split` (`.fill()`, `.resize()`,
// `.expand()`), `list`, `icon_button` (`.toggle()`), `agent_run` (`.log()`) and state (`s.stateReset`); the steps
// `draft`, `replyTo`, `scrollTo`, `copy`, `merge`; the query options `older`, `debounceMs`, `join`.
//
//   const who = ui.via('who', msg, face);
//   const msgs = s.query('channel', ui.call(getChannel, { id }).rows(channelView.f.messages)
//     .older(channelView.f.olderPageToken, getChannelRequest.f.pageToken).join(who, channelView.f.people, face.f.id, msg.f.by));
//   ui.stream(msgs).key(msg.f.id)
//     .post((p) => p.author(who.f(face.f.name)).avatar(who.f(face.f.avatar)).time(msg.f.time).markdown(msg.f.text)
//       .reactions(msg.f.reactions, reaction.f.emoji, reaction.f.count, react).quick(['👍', '🔥'])
//       .hover([ui.builtin.react(), ui.builtin.reply(), ui.postEntry(openThread, 'Reply in thread'), ui.builtin.more()]))
//     .days(msg.f.time).onSeen(markRead).id('messages')

/** The item type of a list, or the type itself: what a path continues into. */
export type Elem<T> = T extends readonly (infer E)[] ? E : T;

/**
 * A path into a row of `R` reading a `T`: a nested field (`ui.path(msg.f.thread, summary.f.count)` is
 * `thread.count`) or a field of a query's join (`who.f(face.f.name)` is `who.name`). Templates take it wherever they
 * take a field.
 */
export class PathOf<R, T> {
  declare readonly __row: (r: R) => R;
  declare readonly __value: T;

  constructor(readonly path: string) {}

  /** A field of the message this path leads to (or of its items). */
  dot<U>(f: FieldRef<Elem<T>, U>): PathOf<R, U> {
    return new PathOf<R, U>(`${this.path}.${f.name}`);
  }
}

/** A field or a path of a row of `R`. */
export type RowPathLike<R, T = any> = FieldRef<R, T> | PathOf<R, T>;

/** @internal The row path a field or a path names. */
function rowPath(p: RowPathLike<any, any>): string {
  return p instanceof PathOf ? p.path : p.name;
}

/** A nested field of a row: `ui.path(msg.f.thread, summary.f.count)` (`thread.count`); `.dot(..)` goes deeper. */
function path<R, T, U>(field: FieldRef<R, T>, next: FieldRef<Elem<T>, U>): PathOf<R, U> {
  return new PathOf<R, U>(`${field.name}.${next.name}`);
}

/** A lookup of a query's rows (`QueryDef.join(..)`): the rows of `R` read the item of `J` as `<name>.<field>`. */
export class Via<R, J> {
  declare readonly __types: (x: [R, J]) => [R, J];

  constructor(readonly name: string) {}

  /** A field of the looked-up item, as a path of the row. */
  f<T>(f: FieldRef<J, T>): PathOf<R, T> {
    return new PathOf<R, T>(`${this.name}.${f.name}`);
  }
}

/** `ui.via('who', msg, face)`: the rows of `msg` read a `face` as `who.<field>` (or `ui.via<Msg, Face>('who')`). */
function via<RS extends Shape, JS extends Shape>(name: string, row: MessageDef<RS>, item: MessageDef<JS>): Via<Infer<RS>, Infer<JS>>;
function via<R, J>(name: string): Via<R, J>;
function via(name: string): Via<any, any> {
  return new Via(name);
}

/** A row value by path (a nested or joined field), for conditions and texts: `ui.rowAt(who.f(face.f.agent)).eq(true)`. */
function rowAt<R, T>(p: RowPathLike<R, T>): Val<T> {
  return new Val<T>({ row: rowPath(p) });
}

function formVal<T>(name: string): Val<T> {
  return new Val<T>({ form: name });
}

/** The values a composer's `send`, a post's actions and a toggle give their chain (`{form: …}`; catalog 3). */
const form = {
  /** The text typed (a composer, a post's inline edit). */
  text: (): Val<string> => formVal("text"),
  /** The key of the item Reply was on (a composer with `.reply()`). */
  reply: (): Val<string> => formVal("reply"),
  /** A dictated recording: a data URL. */
  audio: (): Val<string> => formVal("audio"),
  /** The recording's duration. */
  duration: (): Val<string> => formVal("duration"),
  /** The recording's language. */
  language: (): Val<string> => formVal("language"),
  /** The recording's timed words (JSON). */
  words: (): Val<string> => formVal("words"),
  /** A composer's checkbox (`.option(name, label)`). */
  option: (name: string): Val<boolean> => formVal(name),
  /** The emoji of a reaction. */
  emoji: (): Val<string> => formVal("emoji"),
  /** The name an identity card's button is for. */
  to: (): Val<string> => formVal("to"),
  /** A toggle's new state. */
  pressed: (): Val<boolean> => formVal("pressed"),
};

/** Of a `pick` input: how many are chosen. */
function pickedCount<T>(f: FieldRef<any, T>): Val<bigint> {
  return formVal(`${f.name}.count`);
}

/** Of a `pick` input: how many of the chosen are agents. */
function pickedAgents<T>(f: FieldRef<any, T>): Val<bigint> {
  return formVal(`${f.name}.agents`);
}

/** Of a `pick` input: the first word of the first chosen label. */
function pickedFirst<T>(f: FieldRef<any, T>): Val<string> {
  return formVal(`${f.name}.first`);
}

/** Of a `pick` input: what is typed in its search (its `empty` text reads it: `Nobody matches “{q}”.`). */
function pickedQuery<T>(f: FieldRef<any, T>): Val<string> {
  return formVal(`${f.name}.query`);
}

/** A deep copy of plain objects and arrays; builders and values inside are shared. */
function copyPlain<T>(x: T): T {
  if (Array.isArray(x)) return x.map(copyPlain) as T;
  if (x !== null && typeof x === "object" && Object.getPrototypeOf(x) === Object.prototype) {
    return Object.fromEntries(Object.entries(x).map(([k, v]) => [k, copyPlain(v)])) as T;
  }
  return x;
}

// --- header

/** A piece of a header's subtitle: `ui.seg.text(..)`, `ui.seg.dotWaiting()`, `ui.seg.countdown(..).prefix('ends in ')`. */
export class SegB extends Built {
  constructor(/** @internal */ readonly s: Record<string, unknown>) {
    super();
  }

  /** A text before the value, left out with it. */
  prefix(t: Txt): this {
    this.s.prefix = txt(t);
    return this;
  }

  visible(c: Cond): this {
    this.s.visible = c;
    return this;
  }

  /** @internal */
  json(): unknown {
    return this.s;
  }
}

const seg = {
  /** A text; left out while its value is empty. */
  text: (t: Txt): SegB => new SegB({ text: txt(t) }),
  /** The amber dot of something at work. */
  dotLive: (): SegB => new SegB({ dot: "live" }),
  /** The hollow dot of something waiting. */
  dotWaiting: (): SegB => new SegB({ dot: "waiting" }),
  /** The line in the tone of something at work (amber), with no dot drawn. */
  toneLive: (): SegB => new SegB({ tone: "live" }),
  /** The line in the tone of something waiting, with no dot drawn. */
  toneWaiting: (): SegB => new SegB({ tone: "waiting" }),
  /** The time left until a moment in ms (left out while empty). */
  countdown: <T>(v: ValueLike<T>): SegB => new SegB({ value: value(v), format: formats.plain("countdown") }),
  /** The time since a moment in ms. */
  elapsed: <T>(v: ValueLike<T>): SegB => new SegB({ value: value(v), format: formats.plain("elapsed") }),
};

/** A tab of a header: `ui.headerTab('pinned', 'Pinned').badge(q.field(view.f.pinnedCount))`. */
export class HeaderTabB extends Built {
  private readonly t: Record<string, unknown>;

  constructor(id: string, title: Txt) {
    super();
    this.t = { id, title: txt(title) };
  }

  /** ` · N` after the title while above 0. */
  badge<T>(v: Val<T>): this {
    this.t.badge = v;
    return this;
  }

  /** Runs the action instead of selecting the tab. */
  action(a: ActionRef): this {
    this.t.action = a.name;
    return this;
  }

  /** @internal */
  json(): unknown {
    return this.t;
  }
}

function headerTab(id: string, title: Txt): HeaderTabB {
  return new HeaderTabB(id, title);
}

/**
 * A head of a screen's column or of a side panel (catalog 3): a leading tile or faces, the title with a glyph, a
 * subtitle line of segments, a faces pill, text pills, icon buttons, a `…` menu, tabs, and the panel's expand and
 * close. On a phone a column's header is the shell's top bar; a panel's header is a back bar.
 */
export class HeaderB extends Node {
  private readonly buttonList: Record<string, unknown>[] = [];

  constructor(title: Txt) {
    super("header", { title: txt(title) });
  }

  /** An icon name before the title. */
  titleIcon(v: ValueLike<string>): this {
    this.body.title_icon = value(v);
    return this;
  }

  /** The icon the breadcrumb and the phone's top bar put before the screen's title. */
  crumbIcon(v: ValueLike<string>): this {
    this.body.crumb_icon = value(v);
    return this;
  }

  /** An agent tile: the glyph and the engine (`claude`, `codex`, `pi`; empty: the ink cell). */
  leadingTile(icon: ValueLike<string>, engine: ValueLike<string>): this {
    this.body.leading = { tile: { icon: value(icon), engine: value(engine) } };
    return this;
  }

  /** A stack of faces (a list of `name`, `avatar`, `agent`, `icon`, `engine`). */
  leadingFaces<T>(v: Val<T>): this {
    this.body.leading = { faces: v };
    return this;
  }

  /** The leading's size in px. */
  leadingSize(px: number): this {
    const l = this.body.leading as Record<string, unknown> | undefined;
    if (l) l.size = px;
    return this;
  }

  /** One line of segments joined with ` · `; a segment with an empty value is left out. */
  subtitle(segs: readonly SegB[]): this {
    this.body.subtitle = some(segs);
    return this;
  }

  /** The subtitle line is a button running this action. */
  subtitleAction(a: ActionRef): this {
    this.body.subtitle_action = a.name;
    return this;
  }

  /** A muted text after the title (a panel's `# returns`). */
  context(t: Txt): this {
    this.body.context = txt(t);
    return this;
  }

  /** A faces pill: the faces, the label (`9 members`), the action. */
  faces<T>(faces: Val<T>, label: Txt, a: ActionRef): this {
    this.body.faces = { value: faces, label: txt(label), action: a.name };
    return this;
  }

  /** The faces pill's accessible name (absent: `{label}: open`). Call it after `faces`. */
  facesAriaLabel(t: Txt): this {
    const f = this.body.faces as Record<string, unknown> | undefined;
    if (f) f.aria_label = txt(t);
    return this;
  }

  /** The faces pill is shown while `c` holds. */
  facesWhen(c: Cond): this {
    const f = this.body.faces as Record<string, unknown> | undefined;
    if (f) f.visible = c;
    return this;
  }

  /** A text pill (at most 4). */
  button(label: Txt, a: ActionRef): this {
    this.buttonList.push({ label: txt(label), action: a.name });
    this.body.buttons = this.buttonList;
    return this;
  }

  /** The last pill is shown while `c` holds. */
  buttonWhen(c: Cond): this {
    const b = this.buttonList.at(-1);
    if (b) b.visible = c;
    return this;
  }

  /** `ui.iconButton` nodes (at most 4; a toggle bell). */
  icons(nodes: readonly Node[]): this {
    this.body.icons = some(nodes);
    return this;
  }

  /** The `…` menu. */
  menu(items: readonly MenuItemB[]): this {
    this.body.menu = { items: [...items] };
    return this;
  }

  /** The `…` button's and its menu's accessible label (absent: `More actions`). Call it after `menu`. */
  menuLabel(t: Txt): this {
    const menu = this.body.menu as Record<string, unknown> | undefined;
    if (menu) menu.label = txt(t);
    return this;
  }

  /** Tabs bound to a string state (or param) whose default is one of the ids. */
  tabs(bind: StateRef<string> | ParamRef<string>, items: readonly HeaderTabB[]): this {
    this.body.tabs = { bind: bind instanceof StateRef ? { state: bind.name } : { param: bind.name }, items: [...items] };
    return this;
  }

  /** The tab bar's accessible label (absent: the title). Call it after `tabs`. */
  tabsLabel(t: Txt): this {
    const tabs = this.body.tabs as Record<string, unknown> | undefined;
    if (tabs) tabs.label = txt(t);
    return this;
  }

  /** The enclosing split's expand button (desktop). */
  expand(): this {
    this.body.expand = true;
    return this;
  }

  /** The panel's close button (desktop) and back button (phone). */
  close(a: ActionRef): this {
    this.body.close = a.name;
    return this;
  }

  /** The phone back button's label. */
  back(t: Txt): this {
    this.body.back = txt(t);
    return this;
  }

  private phone(): Record<string, unknown> {
    return (this.body.phone ??= {}) as Record<string, unknown>;
  }

  /** The line under the phone's top-bar title. */
  phoneSub(t: Txt): this {
    this.phone().sub = txt(t);
    return this;
  }

  /** Top-bar icon buttons on a phone: the actions of the header's icons or faces. */
  phoneIcons(actions: readonly ActionRef[]): this {
    this.phone().icons = some(actions.map((a) => a.name));
    return this;
  }

  /** The tabs first in the phone's `…` menu. */
  menuTabs(): this {
    this.phone().menu_tabs = true;
    return this;
  }
}

function header(title: Txt): HeaderB {
  return new HeaderB(title);
}

// --- stream

/**
 * A live list of a query's rows (catalog 3), the newest at the bottom by default: older pages by the query's
 * `older`, day dividers, follow mode and jump to the latest, a `tail` query's rows merged after the rows by key, a
 * `head` row, an action when the newest row is seen, a post template per row:
 * `ui.stream(msgs).key(msg.f.id).post((p) => ..).days(msg.f.time)`.
 */
export class StreamB<R, Resp = any> extends Node {
  declare readonly __types: (x: [R, Resp]) => void;

  constructor(query: string) {
    super("stream", { query, key: "", item: { post: new PostB<R>() } });
  }

  /** The row field that identifies an item (pages, the tail, `merge` and `scrollTo` go by it). */
  key<K>(f: FieldRef<R, K>): this {
    this.body.key = f.name;
    return this;
  }

  /** The template of a row. */
  post(build: (p: PostB<R>) => PostB<R>): this {
    this.body.item = { post: build(new PostB<R>()) };
    return this;
  }

  /** A template built elsewhere (shared by two streams); the stream keeps it as it is now. */
  postWith(p: PostB<R>): this {
    this.body.item = { post: p.clone() };
    return this;
  }

  /** Rows whose `field` equals `when` are one centred muted line: the author, the text, the time. */
  kindsLine<K>(field: FieldRef<R, K>, when: string, author: RowPathLike<R>, text: RowPathLike<R>, time: RowPathLike<R>): this {
    this.body.kinds = { field: field.name, line: { when, author: rowPath(author), text: rowPath(text), time: rowPath(time) } };
    return this;
  }

  /**
   * Another query's rows (of the same row message) after the rows, by key: live items. A tail row gone from the next
   * tail answer stays until the main query has answered once more.
   */
  tail(q: Q<any, R>): this {
    this.body.tail = { query: q.name, hold: true };
    return this;
  }

  /** A row of the answer before the rows (drawn with the template, without its summary), and a rule under it. */
  head(row: FieldRef<Resp, R>, rule: Txt): this {
    this.body.head = { row: row.name, rule: txt(rule) };
    return this;
  }

  /** A list read from the top (no follow mode). */
  anchorTop(): this {
    this.body.anchor = "top";
    return this;
  }

  /** Older pages by a button only, with this label. */
  olderButton(label: Txt): this {
    this.body.older_load = "button";
    this.body.older_label = txt(label);
    return this;
  }

  /** The label of the button that loads older pages. */
  olderLabel(label: Txt): this {
    this.body.older_label = txt(label);
    return this;
  }

  /** A divider before each day (`Today`, `Yesterday`, the date) of this field's moments. */
  days(f: RowPathLike<R>): this {
    this.body.days = rowPath(f);
    return this;
  }

  /** A `New` divider after the item with this key. */
  unread<T>(key: ValueLike<T>): this {
    this.body.unread = value(key);
    return this;
  }

  /** The jump button's text when nothing new came (`Assistant is working`). */
  jumpWorking(t: Txt): this {
    this.body.jump_working = txt(t);
    return this;
  }

  empty(t: Txt): this {
    this.body.empty = txt(t);
    return this;
  }

  /** A muted line above the first item on a phone. */
  phoneIntro(t: Txt): this {
    this.body.phone_intro = txt(t);
    return this;
  }

  /** Runs the action with `{row: <the newest row>}` once the reader is at the end and the newest row changed. */
  onSeen(a: ActionRef): this {
    this.body.on_seen = { action: a.name };
    return this;
  }

  /** `onSeen` only while `c` holds (read in the newest row). */
  onSeenWhen(c: Cond): this {
    const o = this.body.on_seen as Record<string, unknown> | undefined;
    if (o) o.when = c;
    return this;
  }

  /** The name composers refer to (`ui.composer(send).forStream('messages')`). */
  id(id: string): this {
    this.body.id = id;
    return this;
  }

  /** The list's accessible name (`Messages`). */
  label(t: Txt): this {
    this.body.label = txt(t);
    return this;
  }
}

function stream<Resp, R>(q: Q<Resp, R>): StreamB<R, Resp> {
  return new StreamB<R, Resp>(q.name);
}

// --- post

/**
 * The template of a stream's row (catalog 3): who wrote it and when, its text and recording, marks and lines under
 * it, a slot for a run of work, reactions, a summary of what follows from it, and what can be done with it. Every
 * field is a row field or a path (`who.f(face.f.name)`, `ui.path(..)`).
 */
export class PostB<R> extends Built {
  declare readonly __row: (r: R) => R;

  constructor(/** @internal */ readonly p: Record<string, any> = { author: "", time: "" }) {
    super();
  }

  /** A copy to change apart (a second stream's template without its summary). */
  clone(): PostB<R> {
    return new PostB<R>(copyPlain(this.p));
  }

  author(f: RowPathLike<R>): this {
    this.p.author = rowPath(f);
    return this;
  }

  time(f: RowPathLike<R>): this {
    this.p.time = rowPath(f);
    return this;
  }

  private avatarBody(): Record<string, any> {
    return (this.p.avatar ??= {});
  }

  /** The picture's URL (else the author's initials). */
  avatar(f: RowPathLike<R>): this {
    this.avatarBody().url = rowPath(f);
    return this;
  }

  /** The agent tile: the glyph and the engine (drawn by default while `agent` is true or the glyph is not empty). */
  tile(icon: RowPathLike<R>, engine: RowPathLike<R>): this {
    this.avatarBody().tile = { icon: rowPath(icon), engine: rowPath(engine) };
    return this;
  }

  /** The tile is drawn while `c` holds. */
  tileWhen(c: Cond): this {
    const t = this.p.avatar?.tile;
    if (t) t.when = c;
    return this;
  }

  /** A tag after the name (`AGENT`) while `c` holds. */
  badge(t: Txt, c: Cond): this {
    this.p.badge = { text: txt(t), when: c };
    return this;
  }

  /** A quiet line after the time (`via Commerce`). */
  source(f: RowPathLike<R>): this {
    this.p.source = rowPath(f);
    return this;
  }

  /** The text as Markdown. */
  markdown(f: RowPathLike<R>): this {
    this.p.text = { field: rowPath(f) };
    return this;
  }

  /** The text as plain text with `@[Name]` and `#[x]` chips (and the words lit while the recording plays). */
  plain(f: RowPathLike<R>): this {
    this.p.text = { field: rowPath(f), format: "plain" };
    return this;
  }

  /** A bool field: the text is being written (a caret). */
  writing(f: RowPathLike<R, boolean>): this {
    if (this.p.text) this.p.text.writing = rowPath(f);
    return this;
  }

  /** A recording: its duration (s); `.audioMore(label, url, words)`. */
  audio(duration: RowPathLike<R>): this {
    this.p.audio = { duration: rowPath(duration) };
    return this;
  }

  /** The recording's label, URL (a data URL) and timed words (`word`, `start`, `end`). */
  audioMore(label: RowPathLike<R>, url: RowPathLike<R>, words: RowPathLike<R>): this {
    if (this.p.audio) Object.assign(this.p.audio, { label: rowPath(label), url: rowPath(url), words: rowPath(words) });
    return this;
  }

  /** A bool field: `(edited)` after the text. */
  edited(f: RowPathLike<R, boolean>): this {
    this.p.edited = rowPath(f);
    return this;
  }

  /** A bool field: pinned; `mark` true draws `Pinned` above the head. */
  pinned(f: RowPathLike<R, boolean>, mark: ValueLike<boolean>): this {
    this.p.pinned = { field: rowPath(f), mark: value(mark) };
    return this;
  }

  /** A bool field: the viewer's own row (no Reply to it). */
  mine(f: RowPathLike<R, boolean>): this {
    this.p.mine = rowPath(f);
    return this;
  }

  /** A line under the text; its tone by a field's value (`{ done: 'ok' }`). Drawn while the route's `to` is empty. */
  receipt(text: RowPathLike<R>, tone: RowPathLike<R>, tones: Record<string, Tone>): this {
    this.p.receipt = { text: rowPath(text), tone: { field: rowPath(tone), tones: tonesJson(tones) } };
    return this;
  }

  /** `→ <to> · <status>` with `why` as its tooltip; drawn while `to` is not empty. */
  route(to: RowPathLike<R>, status: RowPathLike<R>, why: RowPathLike<R>): this {
    this.p.route = { to: rowPath(to), status: rowPath(status), why: rowPath(why) };
    return this;
  }

  /** A correction inline for 30 s after the moment `from` (ms), then in the menu: its label, the action. */
  correction(label: RowPathLike<R>, a: ActionRef, from: RowPathLike<R>): this {
    this.p.correction = { label: rowPath(label), action: a.name, from: rowPath(from), window_ms: 30_000 };
    return this;
  }

  /** The correction's menu label, the words before it inline, its icon (`undo`, `arrow-right`). */
  correctionWords(menuLabel: RowPathLike<R>, lead: RowPathLike<R>, icon: RowPathLike<R>): this {
    if (this.p.correction) Object.assign(this.p.correction, { menu_label: rowPath(menuLabel), lead: rowPath(lead), icon: rowPath(icon) });
    return this;
  }

  /** How long the correction stays inline (ms, at most 120000). */
  correctionWindowMs(ms: number): this {
    if (this.p.correction) this.p.correction.window_ms = ms;
    return this;
  }

  /** Reaction chips: the repeated field, each one's emoji and count; a click runs `a` with `ui.form.emoji()`. */
  reactions<X, E, C>(field: RowPathLike<R, X[]>, emoji: FieldRef<X, E>, count: FieldRef<X, C>, a: ActionRef): this {
    this.p.reactions = { field: rowPath(field), emoji: emoji.name, count: count.name, action: a.name };
    return this;
  }

  /** The field of a reaction: the viewer's own (pressed). */
  reactionsMine<X>(f: FieldRef<X, boolean>): this {
    if (this.p.reactions) this.p.reactions.mine = f.name;
    return this;
  }

  /** The emoji the hover's React offers (at most 8). */
  quick(emoji: readonly string[]): this {
    if (this.p.reactions) this.p.reactions.quick = some(emoji);
    return this;
  }

  /** What follows from the item (replies): its count and the action that opens it. */
  summary(count: RowPathLike<R>, a: ActionRef): this {
    this.p.summary = { count: rowPath(count), action: a.name };
    return this;
  }

  /** The summary's label (`{count} replies`). */
  summaryLabel(t: Txt): this {
    if (this.p.summary) this.p.summary.label = txt(t);
    return this;
  }

  /** The summary's faces, last line, who works (since a moment in ms) and who waits. */
  summaryMore(faces: RowPathLike<R>, last: RowPathLike<R>, working: RowPathLike<R>, since: RowPathLike<R>, waiting: RowPathLike<R>): this {
    if (this.p.summary) {
      Object.assign(this.p.summary, {
        faces: rowPath(faces),
        last: rowPath(last),
        working: rowPath(working),
        since: rowPath(since),
        waiting: rowPath(waiting),
      });
    }
    return this;
  }

  /** No summary (a pinned list's posts). */
  noSummary(): this {
    delete this.p.summary;
    return this;
  }

  /** A run of work above the text: `at` is the field holding it; the block's fields are fields of what it holds. */
  process<T>(at: RowPathLike<R, T>, build: (w: ProcessB<Elem<T>>) => ProcessB<Elem<T>>): this {
    this.p.process = build(new ProcessB<Elem<T>>(rowPath(at))).p;
    return this;
  }

  /** The rail on hover (desktop), in order. */
  hover(entries: readonly PostEntryB[]): this {
    this.p.hover = some(entries);
    return this;
  }

  /** The `…` menu, in order. */
  menu(entries: readonly PostEntryB[]): this {
    this.p.menu = some(entries);
    return this;
  }

  /** The phone's long-press sheet, in order. */
  sheet(entries: readonly PostEntryB[]): this {
    this.p.sheet = some(entries);
    return this;
  }

  /** The inline editor of the text: `a` reads `ui.form.text()`; offered while `c` holds. */
  edit(a: ActionRef, c: Cond): this {
    this.p.edit = { action: a.name, when: c };
    return this;
  }

  /** While false: no rail, menu, sheet or inline correction. */
  actionsWhen(c: Cond): this {
    this.p.actions_when = c;
    return this;
  }

  /** The identity card a mention, the lead or the name opens. */
  card<F>(c: CardB<F>): this {
    this.p.card = c;
    return this;
  }

  /** @internal */
  json(): unknown {
    return this.p;
  }
}

/** A post template to share between streams: `ui.post(msg)` (or `ui.post<Msg>()`). */
function post<S extends Shape>(row: MessageDef<S>): PostB<Infer<S>>;
function post<R>(): PostB<R>;
function post(): PostB<any> {
  return new PostB();
}

/** An entry of a post's hover rail, menu or sheet: a built-in gesture (`ui.builtin.*`) or an action with its label. */
export class PostEntryB extends Built {
  constructor(/** @internal */ readonly e: Record<string, unknown>) {
    super();
  }

  icon(icon: string): this {
    this.e.icon = icon;
    return this;
  }

  /** Drawn as a text button (the hover's `Ask agent`). */
  textStyle(): this {
    this.e.style = "text";
    return this;
  }

  /** Drawn as an icon button (the label is its tooltip). */
  iconStyle(): this {
    this.e.style = "icon";
    return this;
  }

  danger(): this {
    this.e.danger = true;
    return this;
  }

  visible(c: Cond): this {
    this.e.visible = c;
    return this;
  }

  /** @internal */
  json(): unknown {
    return this.e;
  }
}

/** An entry running an action on the row: `ui.postEntry(openThread, 'Reply in thread').icon('message-square-text')`. */
function postEntry(a: ActionRef, label: Txt): PostEntryB {
  return new PostEntryB({ action: a.name, label: txt(label) });
}

const builtinEntry = (name: string) => () => new PostEntryB({ builtin: name });

/** The built-in entries of a post: client gestures that need no plugin action (or the post's own options). */
const builtin = {
  /** The quick reactions (needs `reactions`). */
  react: builtinEntry("react"),
  /** Reply to the author (needs a composer with `.reply()`). */
  reply: builtinEntry("reply"),
  copyText: builtinEntry("copy_text"),
  copyLink: builtinEntry("copy_link"),
  /** Play or pause the recording (needs `audio`). */
  play: builtinEntry("play"),
  /** The inline editor (needs `edit`). */
  edit: builtinEntry("edit"),
  /** The correction after its inline window (needs `correction`). */
  correction: builtinEntry("correction"),
  /** The `…` button that opens the menu (the hover rail only). */
  more: builtinEntry("more"),
  separator: builtinEntry("separator"),
};

/** An identity card: a name looked up (case-insensitively) in a query's rows. */
export class CardB<F> extends Built {
  declare readonly __row: (r: F) => F;
  private readonly c: Record<string, unknown>;

  constructor(query: string, name: string) {
    super();
    this.c = { query, name };
  }

  avatar<T>(f: FieldRef<F, T>): this {
    this.c.avatar = f.name;
    return this;
  }

  agent(f: FieldRef<F, boolean>): this {
    this.c.agent = f.name;
    return this;
  }

  tile<A, B>(icon: FieldRef<F, A>, engine: FieldRef<F, B>): this {
    Object.assign(this.c, { icon: icon.name, engine: engine.name });
    return this;
  }

  detail<T>(f: FieldRef<F, T>): this {
    this.c.detail = f.name;
    return this;
  }

  /** The card's button: `a` reads `ui.form.to()`. */
  action(a: ActionRef): this {
    this.c.action = a.name;
    return this;
  }

  /** What the card says of a name not in the rows. */
  unknown(t: Txt): this {
    this.c.unknown = txt(t);
    return this;
  }

  /** @internal */
  json(): unknown {
    return this.c;
  }
}

/** `ui.cardOf(people, face.f.name)`: the identity card over a query's rows. */
function cardOf<F, T>(q: Q<any, F>, name: FieldRef<F, T>): CardB<F> {
  return new CardB<F>(q.name, name.name);
}

// --- process

/**
 * A run of work (catalog 3): as a post's slot (`p.process(msg.f.work, (w) => w.state(work.f.state)..)`) or on its own
 * over a query's answer (`ui.process(runQ, (w) => ..)`). Live (`starting`, `working`), done, or ended (`stopped`,
 * `failed`, `timed_out`, `gone`); no block while `state` is empty. Its rows and calls have fixed field names.
 */
export class ProcessB<W> {
  declare readonly __work: (w: W) => W;
  /** @internal */ readonly p: Record<string, unknown> = { state: "" };

  constructor(private readonly at?: string) {}

  private path(f: FieldRef<W, unknown>): string {
    return this.at === undefined ? f.name : `${this.at}.${f.name}`;
  }

  /** `starting`, `working`, `done`, `stopped`, `failed`, `timed_out`, `gone`; no block while empty. */
  state<T>(f: FieldRef<W, T>): this {
    this.p.state = this.path(f);
    return this;
  }

  label<T>(f: FieldRef<W, T>): this {
    this.p.label = this.path(f);
    return this;
  }

  meta<T>(f: FieldRef<W, T>): this {
    this.p.meta = this.path(f);
    return this;
  }

  /** The moment (ms) the live block counts from. */
  started<T>(f: FieldRef<W, T>): this {
    this.p.started = this.path(f);
    return this;
  }

  /** The rows: `kind`, `title`, `detail`, `state`, `time`, `at`, `calls`, `tools`. */
  rows<T>(f: FieldRef<W, T>): this {
    this.p.rows = this.path(f);
    return this;
  }

  /** The number of steps (`View N steps`). */
  steps<T>(f: FieldRef<W, T>): this {
    this.p.steps = this.path(f);
    return this;
  }

  /** The text being written (with a caret). */
  saying<T>(f: FieldRef<W, T>): this {
    this.p.saying = this.path(f);
    return this;
  }

  /** Retry on an ended block, with its label from a field, while `c` holds. */
  retry<T>(a: ActionRef, label: FieldRef<W, T>, c: Cond): this {
    this.p.retry = { action: a.name, label: this.path(label), when: c };
    return this;
  }

  /** `Open full log`. */
  log(a: ActionRef): this {
    this.p.log = { action: a.name };
    return this;
  }

  /**
   * How a truncated call is read whole: `q` read once more with `a` and the request field `call` set to the call's
   * id; the answer's `field` replaces the call.
   */
  fetchCall<Req, Resp, C, F>(q: Q<Resp, any>, call: FieldRef<Req, C>, field: FieldRef<Resp, F>, a?: ArgsOf<Req>): this {
    this.p.fetch_call = { query: q.name, args: some(args(a)), call: call.name, field: field.name };
    return this;
  }
}

/** A run of work on its own, over a query's answer: `ui.process(runQ, (w) => w.state(run.f.state).rows(run.f.rows))`. */
function processNode<Resp, R>(q: Q<Resp, R>, build: (w: ProcessB<Resp>) => ProcessB<Resp>): N {
  return n("process", { query: q.name, ...build(new ProcessB<Resp>()).p });
}

// --- strip

/** A button of a strip's row: `ui.stripButton('Stop', stop).icon('square').stopStyle()`. */
export class StripButtonB extends Built {
  private readonly s: Record<string, unknown>;

  constructor(label: Txt, a: ActionRef) {
    super();
    this.s = { label: txt(label), action: a.name };
  }

  icon(icon: string): this {
    this.s.icon = icon;
    return this;
  }

  /** The red-outlined Stop with the filled square. */
  stopStyle(): this {
    this.s.style = "stop";
    return this;
  }

  /** A muted text button. */
  textStyle(): this {
    this.s.style = "text";
    return this;
  }

  visible(c: Cond): this {
    this.s.visible = c;
    return this;
  }

  /** @internal */
  json(): unknown {
    return this.s;
  }
}

function stripButton(label: Txt, a: ActionRef): StripButtonB {
  return new StripButtonB(label, a);
}

/**
 * A pinned list of live items (catalog 3): a tile, the title, a dot (live, waiting), `Starting` or the time since
 * `since`, a sub line, a jump to the item's place in a stream and buttons; hidden while there are no rows.
 */
export class StripB<R> extends Node {
  declare readonly __row: (r: R) => R;
  private readonly buttonList: StripButtonB[] = [];

  constructor(body: Record<string, unknown>) {
    super("strip", body);
  }

  /** Above the items (`2 live runs`); none while empty. */
  label(t: Txt): this {
    this.body.label = txt(t);
    return this;
  }

  tile<A, B>(icon: FieldRef<R, A>, engine: FieldRef<R, B>): this {
    this.body.tile = { icon: icon.name, engine: engine.name };
    return this;
  }

  /** Which values of the field are live, waiting and starting. */
  state<T>(f: FieldRef<R, T>, live: readonly string[], waiting: readonly string[] = [], starting: readonly string[] = []): this {
    this.body.state = { field: f.name, live: some(live), waiting: some(waiting), starting: some(starting) };
    return this;
  }

  /** The moment (ms) an item counts from. */
  since<T>(f: FieldRef<R, T>): this {
    this.body.since = f.name;
    return this;
  }

  /** The moment (ms) a waiting item counts down to. */
  until<T>(f: FieldRef<R, T>): this {
    this.body.until = f.name;
    return this;
  }

  /** The line under the title. */
  subLine<T>(f: FieldRef<R, T>): this {
    this.body.sub = f.name;
    return this;
  }

  /** After a waiting item's sub line, with `{left}` the countdown (`ends in {left}`). */
  countdownText(t: Txt): this {
    this.body.countdown_text = txt(t);
    return this;
  }

  /** The tile and the title run it (row context). */
  open(a: ActionRef): this {
    this.body.open = a.name;
    return this;
  }

  /** The jump to the item's place in a stream: the key in `f`, the labels by state. */
  jump<T>(f: FieldRef<R, T>, label: Txt, labelWaiting: Txt): this {
    this.body.jump = { field: f.name, label: txt(label), label_waiting: txt(labelWaiting) };
    return this;
  }

  button(b: StripButtonB): this {
    this.buttonList.push(b);
    this.body.buttons = this.buttonList;
    return this;
  }

  ariaLabel(t: Txt): this {
    this.body.aria_label = txt(t);
    return this;
  }
}

/** A strip over a repeated field of a query's answer, each item keyed by `key` and titled by `title`. */
function strip<Resp, R, K, T>(q: Q<Resp, any>, rows: FieldRef<Resp, R[]>, key: FieldRef<R, K>, title: FieldRef<R, T>): StripB<R> {
  return new StripB<R>({ query: q.name, rows: rows.name, key: key.name, title: title.name });
}

// --- composer

/** A tool of a composer besides the mention. */
export type ComposerTool = "add" | "emoji" | "attach";

/** The mention picker's directory: `ui.mentions(people, face.f.name).tile(face.f.icon, face.f.engine)`. */
export class MentionsB<F> extends Built {
  declare readonly __row: (r: F) => F;
  private readonly m: Record<string, unknown>;

  constructor(query: string, name: string) {
    super();
    this.m = { query, name };
  }

  avatar<T>(f: FieldRef<F, T>): this {
    this.m.avatar = f.name;
    return this;
  }

  agent(f: FieldRef<F, boolean>): this {
    this.m.agent = f.name;
    return this;
  }

  tile<A, B>(icon: FieldRef<F, A>, engine: FieldRef<F, B>): this {
    Object.assign(this.m, { icon: icon.name, engine: engine.name });
    return this;
  }

  detail<T>(f: FieldRef<F, T>): this {
    this.m.detail = f.name;
    return this;
  }

  /** The rows are grouped by this field on a desktop. */
  section<T>(f: FieldRef<F, T>): this {
    this.m.section = f.name;
    return this;
  }

  /** The picker's footer note. */
  note(t: Txt): this {
    this.m.note = txt(t);
    return this;
  }

  /** @internal */
  json(): unknown {
    return this.m;
  }
}

function mentions<F, T>(q: Q<any, F>, name: FieldRef<F, T>): MentionsB<F> {
  return new MentionsB<F>(q.name, name.name);
}

/**
 * A message box (catalog 3): `ui.composer(send).forStream('messages').primary().mentions(..).reply()`. `send` reads
 * `ui.form.text()`, `ui.form.reply()`, the recording (`audio`, `duration`, `language`, `words`) and each option
 * (`ui.form.option('also')`).
 */
export class ComposerB extends Node {
  private readonly optionList: Record<string, unknown>[] = [];
  private readonly captionList: Record<string, unknown>[] = [];

  constructor(send: string) {
    super("composer", { send });
  }

  /** The `id` of the stream it belongs to (jumps to its end on send; `draft` and `replyTo` from its items). */
  forStream(id: string): this {
    this.body.for = id;
    return this;
  }

  placeholder(t: Txt): this {
    this.body.placeholder = txt(t);
    return this;
  }

  /** Owns ⌥ Space and the steps of rows outside a stream; one per frame. */
  primary(): this {
    this.body.primary = true;
    return this;
  }

  /** The panel's box. */
  compact(): this {
    this.body.compact = true;
    return this;
  }

  /** The tools besides the mention; none when empty. */
  tools(tools: readonly ComposerTool[]): this {
    this.body.tools = some(tools);
    return this;
  }

  mentions<F>(m: MentionsB<F>): this {
    this.body.mentions = m;
    return this;
  }

  /** `#word` becomes `#[word]` on send. */
  tags(): this {
    this.body.tags = true;
    return this;
  }

  /** The Dictate button (shown when the workspace has a speech connection). */
  dictation(): this {
    this.body.dictation ??= {};
    return this;
  }

  /** Words for the recogniser besides the mentionable names. */
  vocabulary(t: Txt): this {
    ((this.body.dictation ??= {}) as Record<string, unknown>).vocabulary = txt(t);
    return this;
  }

  /** Takes `replyTo`: `ui.form.reply()` is the key. */
  reply(): this {
    this.body.reply = true;
    return this;
  }

  /** A checkbox under the box (`ui.form.option(name)`). */
  option(name: string, label: Txt): this {
    this.optionList.push({ name, label: txt(label) });
    this.body.options = this.optionList;
    return this;
  }

  /** The last option is shown while `c` holds. */
  optionWhen(c: Cond): this {
    const o = this.optionList.at(-1);
    if (o) o.visible = c;
    return this;
  }

  /** A caption line naming who will answer (`↳ …`); the first caption with a value is drawn. */
  captionRoute(t: Txt): this {
    this.captionList.push({ value: txt(t), icon: "corner-down-right", style: "route" });
    this.body.caption = this.captionList;
    return this;
  }

  /** A plain caption line. */
  captionDot(t: Txt): this {
    this.captionList.push({ value: txt(t), style: "dot" });
    this.body.caption = this.captionList;
    return this;
  }

  /** While `c` holds and the box is empty, Send is a labelled Stop running `a` (and Esc runs it). */
  stop(a: ActionRef, c: Cond): this {
    this.body.stop = { action: a.name, when: c };
    return this;
  }

  stopLabel(t: Txt): this {
    const s = this.body.stop as Record<string, unknown> | undefined;
    if (s) s.label = txt(t);
    return this;
  }

  /** The Stop button's accessible name (absent: its label). Call it after `stop`. */
  stopAriaLabel(t: Txt): this {
    const s = this.body.stop as Record<string, unknown> | undefined;
    if (s) s.aria_label = txt(t);
    return this;
  }

  /** While `c` holds, the reason stands where the box would be. */
  readOnly(c: Cond, reason: Txt): this {
    this.body.readonly = { when: c, reason: txt(reason) };
    return this;
  }

  /** The text is mirrored into this string state, for a query that reads what is typed. */
  draftState(s: StateRef<string>): this {
    this.body.draft_state = s.name;
    return this;
  }
}

function composer(send: ActionRef): ComposerB {
  return new ComposerB(send.name);
}

// ---------------------------------------------------------------------------------------------- the namespace

/** The UI builders. */
export const ui = {
  define,
  // texts and values
  text,
  lit: <T>(x: T): Val<T> => new Val<T>({ lit: encode(x) }),
  row,
  /** A field of the last call's result in the running chain. */
  result: <T>(f: FieldRef<any, T>): Val<T> => new Val<T>({ result: f.name }),
  /** The member looking at the screen. */
  callerId: (): Val<string> => new Val<string>({ caller: "user_id" }),
  callerName: (): Val<string> => new Val<string>({ caller: "display_name" }),
  /** In `onError`: what failed. */
  errorCode: (): Val<string> => new Val<string>({ error: "code" }),
  errorMessage: (): Val<string> => new Val<string>({ error: "message" }),
  errorOutcome: (): Val<"failed" | "unknown"> => new Val<"failed" | "unknown">({ error: "outcome" }),
  errorStep: (): Val<number> => new Val<number>({ error: "step" }),
  /** No value: clears an optional param or state (`a.setParam(order, ui.none())`). */
  none: <T = never>(): Val<T> => new Val<T>({ lit: null }),
  /** A field of the current row, where no row handle is at hand (a list's aside text). */
  rowValue: <T>(f: FieldRef<any, T>): Val<T> => new Val<T>({ row: f.name }),
  /** The form's current values, for queries of the same overlay (a quote that reads them as they change). */
  formValue: <T>(f: FieldRef<any, T>): Val<T> => new Val<T>({ form: f.name }),
  not: (c: Cond): Cond => new Cond({ not: c.c }),
  // queries, screens, overlays, actions
  call,
  screenKey,
  overlayKey,
  screen,
  dialog: overlayOf("dialog"),
  sidePanel: overlayOf("side_panel"),
  sheet: overlayOf("sheet"),
  confirm,
  // structure
  stack,
  columns,
  section,
  card,
  titledCard,
  tabs,
  tab,
  divider,
  split,
  board,
  // navigation
  segmented,
  link,
  // data
  table,
  list,
  fields,
  item,
  metric,
  barChart: chartOf("bar"),
  lineChart: chartOf("line"),
  areaChart: chartOf("area"),
  pieChart: chartOf("pie"),
  viz,
  timeline,
  status,
  tag,
  avatar,
  avatarStack,
  meter,
  progress,
  heading,
  caption,
  mono,
  copyField,
  label,
  empty,
  notice,
  // records
  recordHeader,
  agentRun,
  conversation,
  /** @deprecated Catalog 3 deprecates `chat` (`W_UI_DEPRECATED`): a `fill` split with `ui.header`, `ui.stream`,
   * `ui.strip` and `ui.composer`. */
  chat,
  rowView,
  search,
  // input and actions
  filterBar,
  button,
  iconButton,
  menu,
  menuButton,
  menuItem,
  // app-like screens (catalog 3)
  via,
  path,
  rowAt,
  /** The values a composer, a post's actions and a toggle give their chain. */
  form,
  pickedCount,
  pickedAgents,
  pickedFirst,
  pickedQuery,
  header,
  headerTab,
  seg,
  stream,
  post,
  postEntry,
  builtin,
  cardOf,
  process: processNode,
  strip,
  stripButton,
  composer,
  mentions,
  // contributions
  nav,
  entry,
  entryRows,
  command,
  home,
};
