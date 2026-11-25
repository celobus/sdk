// Entities stored in OLAP: an entity whose rows are kept in the workspace's analytic copy only, filled by its load (a
// query over lake tables and histories) when the plugin asks for a snapshot of the copy that loads it.
//
//   export const storeMetric = entity({
//     storage: 'olap',
//     olapKey: ['day', 'store', 'metric'],                 // the row's id is made from these values
//     columns: { day: t.date(), store: t.ref(store), metric: t.text(), value: t.decimal().optional() },
//     access: (a) => { a.read(analyst); },
//     load: (l) => l.from(storeMetricsMart)
//       .where(storeMetricsMart.f.day.geCol(daysAgo(400)))
//       .columns({ day: storeMetricsMart.f.day, store: storeMetricsMart.f.storeId, metric: storeMetricsMart.f.metric,
//                  value: storeMetricsMart.f.value.parse('decimal') }),
//   });
//
//   // the scheduled job, last:
//   const ticket = await ctx.analytics().snapshot().load(storeMetric).request();   // answers at once
//   await ticket.wait();                                                            // optional: fails with the load's error
//   const rows = await ctx.analytics().query(storeMetric).where(..).all();
//
// Reading it. Only `ctx.analytics().query(e)` (and the plugin's analytic views and models that name it): `ctx.db(e)`
// and `ctx.query(e)` are type errors, and refused at run time. Before its first load a query of it is 409
// `not_loaded`, never an empty answer; so it is again after the analytic copy was made anew (its loaded rows are
// gone until the plugin asks again), a reinstall, a changed load or entity, or a withdrawn consent of a lake table's
// owner. So request loads from a scheduled job, not once at install.
//
// The request answers a `LoadTicket` at once; the snapshot is made in the background. A request while the plugin has one
// waiting is merged into it; snapshots with loads are at least 10 minutes apart (requests in between wait and share
// the next); a workspace makes at most 48 a day and a plugin's loads are in at most 12 of them (429
// `analytic_load_limit` past either, or past 30 requests a minute). `ticket.status()` asks now, `ticket.wait()` waits
// for its end: `done` once the plugin's analytic reads answer from the snapshot that loaded it (a read right after
// the wait sees the rows), `failed` with the load's error: 422 `analytic_load_rows_limit` (over 1,000,000 rows or
// 1 GiB), `analytic_load_time_limit` (over 50 s), `analytic_load_duplicate` (a key twice), `analytic_load_invalid` (a
// row its entity cannot hold), 503 `analytic_load_failed` (the snapshot could not be made). A plugin waits on at most
// 16 tickets at once (503 `analytic_load_busy`: retry).
import type { DbLink } from "./db.js";
import { RpcError, internal } from "./errors.js";
import { EntityDef, entityNames, foreignPlugin } from "./schema.js";

/** Where a ticket stands. */
export type LoadTicketState = "pending" | "building" | "done" | "failed";

/** One entity's load in a snapshot: its rows and when they were read from the lake, or why it failed (`code`). */
export interface Loaded {
  entity: string;
  rows: number;
  loadedAt?: Date;
  /** empty: loaded; else the failure's code (`analytic_load_rows_limit`, `analytic_load_duplicate`, ..) */
  code: string;
  error: string;
}

/** A ticket as the platform last answered it. */
export interface LoadTicketStatus {
  id: string;
  state: LoadTicketState;
  /** the snapshot that ran its loads (0 before) */
  generation: number;
  /** the entities it loads */
  entities: string[];
  loads: Loaded[];
  /** a failed ticket's first failure */
  code: string;
  error: string;
  requestedAt?: Date;
  finishedAt?: Date;
  /** a pending ticket: the earliest its snapshot may begin (snapshots with loads are spaced: the next slot, or now) */
  notBefore?: Date;
}

/** @internal An `AnalyticsTicket` as the session decodes it. */
interface WireTicket {
  ticket?: string;
  state?: string | number;
  generation?: string | number;
  entities?: string[];
  loads?: { entity?: string; rows?: string | number; loadedAt?: string | number; code?: string; error?: string }[];
  code?: string;
  error?: string;
  requestedAt?: string | number;
  finishedAt?: string | number;
  notBefore?: string | number;
}

const STATES: LoadTicketState[] = ["pending", "building", "done", "failed"];

function at(ms: string | number | undefined): Date | undefined {
  const n = Number(ms ?? 0);
  return n > 0 ? new Date(n) : undefined;
}

function statusOf(t: WireTicket): LoadTicketStatus {
  const state =
    typeof t.state === "number" ? STATES[t.state] : (String(t.state ?? "PENDING").toLowerCase() as LoadTicketState);
  const out: LoadTicketStatus = {
    id: t.ticket ?? "",
    state: STATES.includes(state as LoadTicketState) ? (state as LoadTicketState) : "pending",
    generation: Number(t.generation ?? 0),
    entities: [...(t.entities ?? [])],
    loads: (t.loads ?? []).map((l) => {
      const loaded: Loaded = { entity: l.entity ?? "", rows: Number(l.rows ?? 0), code: l.code ?? "", error: l.error ?? "" };
      const when = at(l.loadedAt);
      if (when) loaded.loadedAt = when;
      return loaded;
    }),
    code: t.code ?? "",
    error: t.error ?? "",
  };
  const requested = at(t.requestedAt);
  if (requested) out.requestedAt = requested;
  const finished = at(t.finishedAt);
  if (finished) out.finishedAt = finished;
  const notBefore = at(t.notBefore);
  if (notBefore) out.notBefore = notBefore;
  return out;
}

/**
 * A wait that the request's deadline ended before the ticket did: 503 `analytic_load_pending` (retry later; the ticket
 * goes on), its details the ticket's id, state and `not_before` (unix milliseconds).
 */
export function ticketPending(t: LoadTicketStatus): RpcError {
  return new RpcError(
    503,
    "analytic_load_pending",
    "the analytic data is still being prepared: the snapshot that loads it has not been made yet",
    { ticket: t.id, state: t.state, not_before: t.notBefore?.getTime() ?? 0 },
  );
}

/** How long before a request's deadline a ticket's wait gives up, to answer in time. */
const WAIT_MARGIN_MS = 2_000;

/** A failed ticket's failure as an error: its code (`analytic_load_failed` when it has none), and that code's status. */
export function ticketError(t: LoadTicketStatus): RpcError | undefined {
  if (t.state !== "failed") return undefined;
  const code = t.code || "analytic_load_failed";
  const status = ["analytic_load_rows_limit", "analytic_load_time_limit", "analytic_load_duplicate", "analytic_load_invalid"].includes(code)
    ? 422
    : ["permission_denied", "capability_missing"].includes(code)
      ? 403
      : ["analytic_load_failed", "unavailable"].includes(code)
        ? 503
        : 500;
  return new RpcError(status, code, t.error);
}

/** The longest a ticket's wait asks the platform to wait at once. */
const WAIT_MS = 30_000;

/**
 * A request for a snapshot with loads (`snapshot.request()`): `last` as answered, `status()` to ask again, `wait()` to
 * wait for its end. `LoadTicket.of(ctx, id)` follows one another call was given.
 */
export class LoadTicket {
  constructor(
    /** @internal */ private readonly link: DbLink,
    /** @internal */ private current: LoadTicketStatus,
  ) {}

  /** A ticket of the plugin by its id (one a request answered earlier, perhaps in another call). */
  static of(ctx: { analytics(): { snapshot(): Snapshot } }, id: string): LoadTicket {
    const link = ctx.analytics().snapshot().link;
    return new LoadTicket(link, { id, state: "pending", generation: 0, entities: [], loads: [], code: "", error: "" });
  }

  /** The ticket's id (another call may follow it: `LoadTicket.of(ctx, id)`). */
  get id(): string {
    return this.current.id;
  }

  /** The ticket as last answered. */
  get last(): LoadTicketStatus {
    return this.current;
  }

  private async ask(waitMs: number): Promise<LoadTicketStatus> {
    const r = (await this.link.syscall({
      analyticsLoadStatus: { requestId: this.link.requestId, ticket: this.current.id, waitMs },
    })) as { analyticsTicket?: WireTicket | null };
    if (!r.analyticsTicket) throw internal(new Error("the platform answered a ticket's state with something else"));
    this.current = statusOf(r.analyticsTicket);
    return this.current;
  }

  /** The ticket's state now. */
  status(): Promise<LoadTicketStatus> {
    return this.ask(0);
  }

  /**
   * Waits until the ticket ends: its state when every load ran, the failure's error otherwise (the other loads of the
   * snapshot stand). A request made within ten minutes of the last snapshot with loads waits for the next one: in an
   * endpoint, answer the ticket and let the caller ask later. In an endpoint the wait ends before the request's
   * deadline: 503 `analytic_load_pending` (`ticketPending`) when the ticket has not ended by then; in a job's run it
   * waits within the run's timeout.
   */
  async wait(): Promise<LoadTicketStatus> {
    for (;;) {
      let waitMs = WAIT_MS;
      if (!this.link.inRun && this.link.deadline) {
        // the whole seconds left before the request's deadline, less a margin to answer in
        const left = Math.floor((this.link.deadline.getTime() - Date.now() - WAIT_MARGIN_MS - 1) / 1000) * 1000;
        if (left < 1000) throw ticketPending(this.current);
        waitMs = Math.min(WAIT_MS, left);
      }
      const t = await this.ask(waitMs);
      if (t.state === "done") return t;
      if (t.state === "failed") throw ticketError(t)!;
    }
  }
}

/**
 * A snapshot of the workspace's analytic copy the plugin asks for (`ctx.analytics().snapshot()`): `.load(e)` names its
 * entities stored in OLAP whose loads it runs, `.request()` asks.
 */
export class Snapshot {
  private readonly entities: string[] = [];
  private error?: string;

  constructor(/** @internal */ readonly link: DbLink) {}

  /** The snapshot runs the load of `entity`, one of the plugin's entities stored in OLAP. */
  load(entity: EntityDef<any, false, true>): this {
    const e = entity as unknown;
    const name = e instanceof EntityDef ? (entityNames.get(e) ?? e.options.name) : undefined;
    if (!(e instanceof EntityDef) || foreignPlugin(e) || e.options.storage !== "olap" || !name) {
      this.error ??= `${name ?? "it"} is not one of the plugin's entities stored in OLAP: only those are loaded`;
      return this;
    }
    if (!this.entities.includes(name)) this.entities.push(name);
    return this;
  }

  /**
   * Asks for the snapshot: answers at once with its `LoadTicket` (a request while the plugin has one waiting is merged into
   * that one). Between two snapshots that load an entity its rows do not change, whatever happens in the lake.
   */
  async request(): Promise<LoadTicket> {
    if (this.error) throw new RpcError(400, "invalid", this.error);
    if (this.entities.length === 0) throw new RpcError(400, "invalid", "a snapshot names the entities it loads: .load(e)");
    const r = (await this.link.syscall({
      analyticsLoad: { requestId: this.link.requestId, entities: [...this.entities] },
    })) as { analyticsTicket?: WireTicket | null };
    if (!r.analyticsTicket) throw internal(new Error("the platform answered a snapshot's request with something else"));
    return new LoadTicket(this.link, statusOf(r.analyticsTicket));
  }
}
