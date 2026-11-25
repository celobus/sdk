// An entity's history in the workspace's Data Lake: every change of the columns the entity keeps, each its own row with
// when it was made and by whom, kept by the platform without a time limit in a layer the plugin declares.
//
//   export const audit = layer({ title: 'History of records' });
//   export const ticket = entity({
//     softDelete: true,
//     columns: { title: t.text(), status: t.enum(TicketStatus).history(), assignee: t.ref(platform.user).optional().history() },
//     history: { layer: audit, read: [analyst], systemRead: true },  // or { layer: audit, all: true, unmasked: true }
//   });
//   // plugin({ layers: { audit }, entities: { ticket }, .. })
//
//   const h = historyOf(ticket);
//   const trail = await ctx.lake().history(ticket)
//     .where(ticket.f.id.eq(id))
//     .orderBy(h.f.validFrom.asc())
//     .all();                     // HistoryRow<typeof ticket>[]: r.change, r.changedBy, r.validTo, r.status (or undefined)
//   const perDay = await ctx.lake().history(ticket).eachDay()
//     .groupBy(h.f.day)
//     .select({ day: h.f.day, open: count() })
//     .all();
//
// What is kept. The columns marked `.history()`, or with `history: { layer, all: true }` every column (the ones a later
// version adds too) but the encrypted ones, the lists and messages and, unless `unmasked: true` is said too, the
// masked ones (the lake has no masks; `celobus plugin check` lists what `all` left out). A kept column is a scalar, an
// enum or a ref.
//
// A row is a change: the entity's `id`, the kept columns as they were after it (each may be absent: a row written
// before the column was kept has none), `validFrom` (when it was made), `validTo` (the row's next change; none for
// the current version; equal to `validFrom` on a delete), `change` (`baseline`, `insert`, `update`, `delete`,
// `restore`) and who made it: `changedBy` (the person or service account, none for a system context or a job without
// one), `changedByKind` (`person`, `service`, `job`, `system`, `platform`, `anonymous`) and `changedByPlugin` (the
// plugin whose call made it). Two changes are never merged; an update that changes no kept column is none; a soft
// delete is a `delete`, undoing it a `restore`. When the history begins (the install of a version that keeps it, or an
// upgrade that keeps a new column) every row is written as it is then, as `baseline`; on a large table in the
// background, and until it is whole a read of the history is refused (`unavailable`, with how far it is). Changes
// reach the lake within seconds, in order.
//
// Per day. `.eachDay()` reads one row per day each version was in force at the day's last moment, in the workspace's
// time zone, with its `day`: from the day of `validFrom` to the day before that of `validTo`, to today for the
// current version (a version that began and ended within one day, and a delete, have none). So counting the rows of
// a day counts what was there at its end.
//
// Who reads it. The holders of the roles of `history.read`, every row of it, every version, without the entity's
// access rules or masks; the plugin's own system context only with `history.systemRead`; another plugin only when the
// owner exports it (`history.export`) and that plugin declares it (`plugin({ historyDependencies: [ticket] })`). A
// history is read in the lake only, with the lake's limits; a model reads it too (`m.fromHistory(ticket)`). Nothing
// erases one row's history (a delete leaves every past version), and `changedBy` holds people's ids.
import { decodeFieldValue, messageFields, type FieldErrors } from "./codec.js";
import { internal } from "./errors.js";
import { EntityDef, Field, fieldRefs, type FieldRefs, type Row } from "./schema.js";

/** What a change was. */
export type HistoryChange = "baseline" | "insert" | "update" | "delete" | "restore";

/** Who made a change: a person, a service account, a job, a system context, the platform itself, no one signed in. */
export type ChangedByKind = "person" | "service" | "job" | "system" | "platform" | "anonymous";

/** The columns every history has besides the entity's `id` and its kept columns (`historyOf(e).f`). */
export interface HistoryColumns<E> {
  /** @internal whose history */
  readonly __historyOf?: E;
  validFrom: Date;
  validTo?: Date;
  change: HistoryChange;
  changedBy?: string;
  changedByKind: ChangedByKind;
  changedByPlugin?: string;
  /** read per day (`eachDay()`): the day */
  day: string;
}

/**
 * A change of a row of `E` as a history read answers it: the entity's `id`, the history's own columns, and the kept
 * columns as they were after the change, each possibly absent (a row written before the column was kept has none).
 */
export type HistoryRow<E> = E extends EntityDef<any>
  ? {
      id: string;
      validFrom: Date;
      /** The row's next change; none for the current version; on a delete, the delete's own moment. */
      validTo?: Date;
      change: HistoryChange;
      /** The person or service account the change was made for; none for a system context or a job without one. */
      changedBy?: string;
      changedByKind: ChangedByKind;
      /** The plugin whose call made it; none for the platform's own. */
      changedByPlugin?: string;
      /** Read per day (`eachDay()`): the day. */
      day?: string;
    } & { [K in Exclude<keyof Row<E>, "id">]?: Row<E>[K] }
  : never;

const HISTORY_SHAPE = {
  validFrom: new Field<Date, false>({ kind: "scalar", scalar: "timestamp" }),
  validTo: new Field<Date, false>({ kind: "scalar", scalar: "timestamp" }).optional(),
  change: new Field<HistoryChange, false>({ kind: "scalar", scalar: "string" }),
  changedBy: new Field<string, false>({ kind: "ref", ref: "platform.user" }).optional(),
  changedByKind: new Field<ChangedByKind, false>({ kind: "scalar", scalar: "string" }),
  changedByPlugin: new Field<string, false>({ kind: "scalar", scalar: "string" }).optional(),
  day: new Field<string, false>({ kind: "scalar", scalar: "date" }),
};

/**
 * The history of an entity (`historyOf(ticket)`): the handles of the history's own columns (`h.f.validFrom`,
 * `h.f.validTo`, `h.f.change`, `h.f.changedBy`, `h.f.changedByKind`, `h.f.changedByPlugin`, `h.f.day`), which a
 * history read or a model over it filters, groups, selects and orders by, beside the entity's kept columns
 * (`ticket.f.status`).
 */
export class HistoryDef<E extends EntityDef<any>> {
  readonly __history = true as const;
  readonly f: FieldRefs<HistoryColumns<E>, typeof HISTORY_SHAPE>;

  constructor(readonly entity: E) {
    // the handles' owner is the history, named as its entity: a query names both by the entity's alias
    this.f = fieldRefs(HISTORY_SHAPE, this) as never;
  }
}

const histories = new WeakMap<object, HistoryDef<any>>();

/** The history of `entity`: the handles of its own columns (`historyOf(ticket).f.validFrom`). */
export function historyOf<E extends EntityDef<any>>(entity: E): HistoryDef<E> {
  let h = histories.get(entity);
  if (!h) {
    h = new HistoryDef(entity);
    histories.set(entity, h);
  }
  return h as HistoryDef<E>;
}

/** @internal A row of a history as the platform sends it (protobuf JSON names), as `HistoryRow<E>`. */
export function decodeHistoryRow(entity: EntityDef<any>, raw: unknown): Record<string, unknown> {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    throw internal(new Error("the platform sent a history row that is not an object"));
  }
  const json = raw as Record<string, unknown>;
  const errors: FieldErrors = {};
  const out: Record<string, unknown> = {};
  for (const f of messageFields(entity)) {
    const value = decodeFieldValue(f.field, json[f.json], f.json, errors);
    if (value !== undefined) out[f.key] = value;
  }
  for (const [key, field] of Object.entries(HISTORY_SHAPE)) {
    const value = decodeFieldValue(field, json[key], key, errors);
    if (value !== undefined) out[key] = value;
  }
  for (const key of ["validFrom", "change", "changedByKind"]) {
    if (out[key] === undefined) errors[key] ??= "missing";
  }
  if (Object.keys(errors).length > 0) {
    throw internal(new Error(`the platform sent a history row this plugin cannot read: ${JSON.stringify(errors)}`));
  }
  return out;
}
