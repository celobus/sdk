// The context of one request: who calls, the request itself, and what the plugin may do through the platform
// (dependency calls, its entities and declared transactions, reads of its dependencies' entities, SQL, the
// directory, the runs of its agents, the plugin's files: `ctx.files`, files.ts; notifications: `ctx.notifications`,
// notify.ts; background jobs: `ctx.jobs` to enqueue and cancel runs, `ctx.job` for the run a job's handler is in).
// Events are not available: those members exist and reject with `unavailable`.
import { checkCall } from "./keys.js";
import { decodeMessage, encodeMessage, stringify, type FieldErrors } from "./codec.js";
import { Repository, TxRepository, type DbLink, type DbResult } from "./db.js";
import { RpcError, internal, invalid, unauthenticated, unavailable } from "./errors.js";
import { durationMs, type AgentEngine, type AgentTimeout, type Duration, type JobDef, type JobPayload, type JobPayloadType } from "./declare.js";
import type { ResolvedJob } from "./descriptor.js";
import type { Logger } from "./log.js";
import { endpointPath, snake } from "./names.js";
import { Notifications, linkWire, type NotifyLink } from "./notify.js";
import { ViewDef, entityNames, foreignPlugin, type EntityDef, type Infer, type IoValue, type MessageDef, type Shape } from "./schema.js";
import type { SettingsCache } from "./settings.js";
import { Analytics, startQuery, type Query, type Queryable } from "./query.js";
import { Lake, type LakeTableDef } from "./lake.js";
import { sqlParams, sqlRows, type SqlRow } from "./sql.js";
import type { TopicDef } from "./ui.js";
import { PlatformFiles, fileWire, type Files, type FilesResult } from "./files.js";
import type { File } from "./schema.js";

/** @internal What the platform answers a syscall (`SyscallResult`, protobuf field names in camelCase). */
export interface SyscallResult extends DbResult, FilesResult {
  id: string;
  error: string;
  errorDetails?: { currentVersion: string | number; column: string } | null;
  result?: string;
  invokeResponse?: { status: number; headers: { name: string; value: string }[]; body: Buffer } | null;
  membersPage?: { members: WireMember[]; nextPageToken: string } | null;
  member?: WireMember | null;
  groupsPage?: { groups: Group[]; nextPageToken: string } | null;
  agentRun?: WireRun | null;
  agentEvents?: {
    run?: WireRun | null;
    events?: { seq: string | number; at: string | number; type: string; json: Buffer }[];
    partial?: { rev: string | number; messages?: { messageId: string; text: string }[] } | null;
  } | null;
  agentRuns?: { runs?: WireRun[] } | null;
  agentInput?: { seq: string | number } | null;
  agentsAvailable?: { agents?: WireAvailableAgent[] } | null;
  notifyFollowState?: { following: boolean } | null;
  jobEnqueued?: { runId: string; existing: boolean; state: string } | null;
  jobCancelled?: { cancelled: boolean } | null;
}

/** `AvailableAgent` as the platform sends it. */
interface WireAvailableAgent {
  agent: string;
  plugin: string;
  name: string;
  title: string;
  description: string;
  icon: string;
  engine: string;
  session: boolean;
  pluginTitle: string;
}

/** `AgentRun` as the platform sends it (64-bit numbers as strings). */
interface WireRun {
  id: string;
  agent: string;
  plugin: string;
  engine: string;
  model: string;
  status: string;
  session: boolean;
  input: string;
  result: string;
  error?: { code: string; message: string } | null;
  startedBy?: { userId: string; displayName: string } | null;
  startedByPlugin: string;
  createdAt: string | number;
  updatedAt: string | number;
  deadlineAt: string | number;
  turn: number;
  lastSeq: string | number;
  pod: string;
  group: string;
  parent: string;
  parentAgent: string;
  onBehalfOf?: { userId: string; displayName: string } | null;
}

/** @internal A syscall that the platform answered with an error string (and the details some db refusals have). */
export class SyscallError extends Error {
  constructor(
    message: string,
    readonly details?: { currentVersion: string | number; column: string } | null,
  ) {
    super(message);
    this.name = "SyscallError";
  }
}

/** @internal The session as a context uses it. */
export interface SyscallSession {
  /** Sends a `Syscall` (without its id) and resolves with the result; rejects with `SyscallError` or when the session ends. */
  syscall(call: Record<string, unknown>): Promise<SyscallResult>;
  /** Sends an `Invalidate` when the session is up; nothing is awaited. */
  invalidate(topic: string, key: string): void;
  /** The plugin's jobs by their declarations (what `ctx.jobs.enqueue` takes); absent: none. */
  jobs?: ReadonlyMap<JobDef<any>, ResolvedJob>;
  /** Where `ctx.files` reads and writes instead of the platform (a test's files in memory); absent: the platform. */
  files?: Files;
  /** The plugin's settings as the platform sent them (or a test gave); absent: none. */
  settings?: SettingsCache;
}

/** @internal `Principal` as the platform sends it. */
export interface WirePrincipal {
  userId: string;
  kind: string;
  displayName: string;
  roles: string[];
  groups: string[];
  scopes: string[];
  agentRun?: string;
}

interface WireMember {
  userId: string;
  kind: string;
  displayName: string;
  roles: string[];
  groups: string[];
  email: string;
  avatarUrl: string;
  lastActive: string | number;
  locale: string;
}

/** A role a check can name: a role object of this plugin, or a full name (`workspace.admin`, `orders.clerk`). */
export type RoleLike = string | { readonly __role: true };

/** `user`: a person; `service`: a service account; `system`: a call without a caller. */
export type CallerKind = "user" | "service" | "system";

/** Who calls the request, resolved by the platform from the workspace's members. */
export class Caller {
  readonly userId: string | undefined;
  readonly kind: CallerKind;
  readonly displayName: string;
  /** Effective roles, full names: `workspace.*` (expanded) and `<plugin>.<role>`. */
  readonly roles: readonly string[];
  /** Group ids. */
  readonly groups: readonly string[];
  /** The token's scopes for this plugin, full names `<plugin>:<scope>`. */
  readonly scopes: readonly string[];
  /**
   * When the caller is an agent working on a run: that run's id. An agent calls the endpoints of the plugin whose
   * code started its run only from that run, so a handler may tie what the agent does to the record the run was
   * started for.
   */
  readonly agentRun: string | undefined;

  constructor(
    principal: WirePrincipal | null | undefined,
    private readonly pluginId: string,
    private readonly roleName: (role: RoleLike) => string | undefined = () => undefined,
  ) {
    this.userId = principal?.userId || undefined;
    this.kind = !principal ? "system" : principal.kind === "SERVICE" ? "service" : "user";
    this.displayName = principal?.displayName ?? "";
    this.roles = principal?.roles ?? [];
    this.groups = principal?.groups ?? [];
    this.scopes = principal?.scopes ?? [];
    this.agentRun = principal?.agentRun || undefined;
  }

  /** The calling user's id; throws 401 `unauthenticated` when the request has none. */
  user(): string {
    if (!this.userId) throw unauthenticated();
    return this.userId;
  }

  /** Holds the role: a role object of this plugin, its short name (`clerk`) or a full name (`workspace.admin`). */
  has(role: RoleLike): boolean {
    const name = typeof role === "string" ? role : this.roleName(role);
    if (!name) return false;
    return this.roles.includes(name.includes(".") ? name : `${this.pluginId}.${name}`);
  }

  /** The token allows the scope: short (`read`) or full (`orders:read`). */
  hasScope(scope: string): boolean {
    return this.scopes.includes(scope.includes(":") ? scope : `${this.pluginId}:${scope}`);
  }

  inGroup(groupId: string): boolean {
    return this.groups.includes(groupId);
  }
}

/** A workspace member from the directory. */
export interface Member {
  userId: string;
  kind: "user" | "service";
  displayName: string;
  roles: string[];
  groups: string[];
  /** empty unless the plugin declares the permission `members.email` */
  email: string;
  avatarUrl: string;
  /** unix seconds, hour resolution */
  lastActive: number;
  locale: string;
}

export interface Group {
  id: string;
  name: string;
  /** user ids */
  members: string[];
}

export interface RequestInfo {
  /** The request id the platform gave it (a decimal string). */
  readonly id: string;
  /** When the endpoint's timeout passes. */
  readonly deadline: Date;
  /** Aborted at the deadline, or when the session to the platform ends: pass it to your own I/O. */
  readonly signal: AbortSignal;
  /** The `Idempotency-Key` header, when the caller sent one. */
  readonly idempotencyKey?: string;
}

export interface PluginInfo {
  readonly id: string;
  readonly version: string;
}

/** How a declared transaction runs: read committed by default; not for a nested one (it rolls back alone). */
export interface TransactionOptions {
  isolation?: "read_committed" | "repeatable_read" | "serializable";
  /** Refuses writes. */
  readOnly?: boolean;
}

function notAvailable(what: string): Promise<never> {
  return Promise.reject(unavailable(`${what} is not available`));
}

function memberOf(m: WireMember): Member {
  return {
    userId: m.userId,
    kind: m.kind === "SERVICE" ? "service" : "user",
    displayName: m.displayName ?? "",
    roles: m.roles ?? [],
    groups: m.groups ?? [],
    email: m.email ?? "",
    avatarUrl: m.avatarUrl ?? "",
    lastActive: Number(m.lastActive ?? 0),
    locale: m.locale ?? "",
  };
}

const PAGE = 500;

/** What the platform refuses a job call with besides the db codes (`invalid`, `not_found`). */
const JOB_REFUSALS = new Map([
  ["queue_full", 429],
  ["payload_too_large", 413],
  // the run making the call was stopped (its abort)
  ["cancelled", 503],
]);

/** The admission refusals the platform answers a syscall with (`<code>: <message>`): 403 with that code. */
const DENIALS = new Set(["call_undeclared", "missing_role", "missing_scope", "reach", "not_a_member"]);

/**
 * What the platform refuses an `agent_*` syscall with, and the status each code answers: an agent or a run the plugin does
 * not have is 404, a run that ended the platform's `run_ended` conflict; a group the plugin did not found is 404, a
 * group that has the agent already or is full a conflict.
 */
const AGENT_REFUSALS = new Map([
  ["unknown_agent", 404],
  ["not_found", 404],
  ["run_ended", 409],
  ["unknown_group", 404],
  ["agent_in_group", 409],
  ["group_full", 409],
  ["bad_request", 400],
  ["forbidden", 403],
]);

/** What the platform refuses a file call with besides `not_found` and `bad_request`, and the status each code answers. */
const FILE_REFUSALS = new Map([
  ["file_gone", 410],
  ["file_too_large", 413],
  ["quota_exceeded", 507],
  ["too_much_unattached", 507],
  ["too_many_unattached", 429],
  ["file_size_mismatch", 409],
  ["file_not_uploaded", 409],
  ["file_unknown", 409],
  ["read_only", 409],
]);

/** What the platform refuses a `notify_*` syscall with besides the db codes (`invalid`, `capability_missing`). */
const NOTIFY_REFUSALS = new Map([
  ["follow_limit", 409],
  ["rate_limited", 429],
]);

/**
 * What the platform refuses a `Db*` or `Tx*` syscall with, and the status each code answers; `version_conflict` carries
 * `details.current`, `unique_conflict` `details.column`.
 */
const DB_REFUSALS = new Map([
  ["not_found", 404],
  ["version_conflict", 409],
  ["unique_conflict", 409],
  ["transaction_conflict", 409],
  ["append_only", 400],
  ["invalid", 400],
  ["query_rows_limit", 422],
  ["bulk_limit", 422],
  ["row_locked", 409],
  ["transaction_timeout", 504],
  ["permission_denied", 403],
  ["guard_denied", 403],
  ["capability_missing", 403],
  ["query_invalid", 500],
  ["query_size_limit", 422],
  ["query_join_limit", 422],
  ["query_response_limit", 422],
  ["analytic_time_limit", 422],
  ["analytic_memory_limit", 422],
  ["analytic_busy", 503],
  ["lake_rows_limit", 422],
  ["lake_time_limit", 422],
  ["lake_memory_limit", 422],
  ["lake_busy", 503],
  ["lake_model_rows_limit", 422],
  ["lake_partitions_limit", 422],
  ["lake_model_parse", 422],
  ["lake_model_changed", 409],
  ["not_loaded", 409],
  ["analytic_load_limit", 429],
  ["analytic_load_busy", 503],
  ["analytic_load_pending", 503],
]);

/**
 * A syscall error as the handler sees it: an admission refusal is 403 with its code, a refusal of an agent, a
 * notification or a db call its own status and code (with the details a db refusal has), anything else 503 `unavailable`.
 */
export function syscallFailed(e: unknown): unknown {
  if (!(e instanceof SyscallError)) return e;
  const i = e.message.indexOf(": ");
  const code = i > 0 ? e.message.slice(0, i) : "";
  const message = e.message.slice(i + 2);
  if (DENIALS.has(code)) return new RpcError(403, code, message);
  const status = AGENT_REFUSALS.get(code) ?? NOTIFY_REFUSALS.get(code) ?? JOB_REFUSALS.get(code) ?? FILE_REFUSALS.get(code);
  if (status !== undefined) return new RpcError(status, code, message);
  const db = DB_REFUSALS.get(code);
  if (db !== undefined) {
    const details: Record<string, unknown> = {};
    if (code === "version_conflict" && e.details) details.current = Number(e.details.currentVersion);
    if (code === "unique_conflict" && e.details) details.column = e.details.column;
    return new RpcError(db, code, message, details);
  }
  return unavailable(e.message);
}

/** Where a run is: `starting` → `running` ⇄ `idle` (a session between turns) → one of the four ends. */
export type AgentRunStatus = "starting" | "running" | "idle" | "succeeded" | "failed" | "cancelled" | "timed_out";

/** A run of one of the plugin's agents, or one its code started. Times are unix milliseconds. */
export interface AgentRun {
  id: string;
  /** The agent's name in its plugin (`plugin`). */
  agent: string;
  plugin: string;
  engine: string;
  /** `undefined`: the engine's own model. */
  model?: string;
  status: AgentRunStatus;
  /** The run is over: nothing more is written to it. */
  ended: boolean;
  /** The run waits for follow-up messages after every turn. */
  session: boolean;
  /** The task it was started with. */
  input: string;
  /** The result of the last finished turn. */
  result?: string;
  /** Why it failed or timed out (`start_timeout`, `lost`, `agent_failed`, …). */
  error?: { code: string; message: string };
  /** The caller of the request the run was started in (or of the HTTP call that started it). */
  startedBy: { userId: string; displayName: string };
  /** The plugin whose code started it; `undefined`: started over HTTP (the CLI, a client). */
  startedByPlugin?: string;
  createdAt: number;
  updatedAt: number;
  deadlineAt: number;
  turn: number;
  /** The sequence number of the run's last event. */
  lastSeq: number;
  /** Where the run executes, as the platform last reported it (an opaque name). */
  pod: string;
  /**
   * The group the run belongs to: the id of the group's first run (its own id for a run that founded one). The
   * agents of a group message each other by name.
   */
  group: string;
  /** The agent run that started this one; `undefined`: a person or a plugin's code did. */
  parent?: string;
  /** The agent of that run, `<plugin>.<name>`; `undefined` with `parent`. */
  parentAgent?: string;
  /** The person at the root: who started the run, or the run that started it. */
  onBehalfOf: { userId: string; displayName: string };
}

/**
 * One event of a run, as the platform keeps it: `type` is `message`, `tool`, `turn`, `status`, `progress` or
 * `error`, and the fields of that type come with it (`text`, `role`, `name`, `phase`, `summary`, …).
 */
export interface AgentEvent {
  seq: number;
  /** Unix milliseconds. */
  at: number;
  type: string;
  [field: string]: unknown;
}

/** A run with its events after a sequence number and the text being written. */
export interface AgentEvents {
  run: AgentRun;
  /** Oldest first. */
  events: AgentEvent[];
  /**
   * The messages being written right now: text, not events (the finished message arrives as an event and takes
   * their place). `rev` counts the changes (0: nothing was written yet); `text` is the messages joined by blank lines.
   */
  partial: { rev: number; messages: { messageId: string; text: string }[]; text: string };
}

/** How a run is started, when it differs from what the agent declares. */
export interface AgentRunOptions {
  engine?: AgentEngine;
  model?: string;
  /** The run is a session (it waits for follow-up messages after every turn), or is not. */
  session?: boolean;
  /** The longest the run takes, from a minute to six hours: `'90s'`, `'15m'`, `'2h'`. */
  timeout?: AgentTimeout;
  /**
   * The group the run joins: the `group` of a run this plugin's code started. Left out: the run founds a new group.
   */
  group?: string;
  /**
   * Tell the person the run works for, in their inbox, when it finishes (a run that fails or runs out of time always
   * tells them).
   */
  notify?: boolean;
  /**
   * Where the notification of the run's end opens: `notify.screen(..)` or `notify.overlay(..)` of this plugin's UI
   * (400 `bad_request` when the UI has no such target or params).
   */
  notifyLink?: NotifyLink;
  /**
   * A topic of this plugin (declared in `plugin({ topics })`) the platform invalidates, for every key, whenever the run
   * changes, as it does the topic the agent declares: how a plugin follows the runs it starts of another plugin's
   * agent. 400 `bad_request` when the plugin declares no such topic.
   */
  topic?: TopicDef<any> | string;
  /**
   * The key of `topic` to invalidate: only the screens that follow the topic for this key re-read (a conversation's
   * screens, not every screen of the plugin). Needs `topic` as a `TopicDef`; left out: every key.
   */
  topicKey?: Record<string, unknown>;
  /**
   * Files the run is given (the plugin's own: 409 `file_unknown` otherwise). They are kept while the run lives, its
   * input lists each one, and the agent downloads those it needs.
   */
  files?: readonly File[];
}

/** What a follow-up message to a run carries besides its text. */
export interface AgentSendOptions {
  /** Files given to the run, as `AgentRunOptions.files` are. */
  files?: readonly File[];
}

/** An agent of the workspace the caller of the request may start a run of (`ctx.agents.available()`). */
export interface AvailableAgent {
  /** `<plugin>.<name>`: what `run` takes. */
  agent: string;
  plugin: string;
  name: string;
  /** Its display name: the declared title, else its name. */
  title: string;
  /** What it is for, in a sentence (empty: not said). */
  description: string;
  /** Its glyph, a lucide icon name. */
  icon?: string;
  /** The engine its runs get in this workspace when a start names none: an owner's or admin's choice, else the one it declares, else its connection's; `undefined`: none can be resolved (it has no connection). */
  engine?: string;
  /** Its runs are sessions by its declaration. */
  session: boolean;
  /** Its plugin's title, else the plugin's id. */
  pluginTitle: string;
}

/**
 * The runs of the plugin's own agents and of the agents of other plugins its code starts (`ctx.agents`).
 */
export interface Agents {
  /**
   * Starts a run of the agent `agent` with the task `input`, as the agent declares its runs unless `options` say
   * otherwise: the plugin's own agent by its name in `plugin({ agents })` (in snake_case), or an agent of another
   * plugin of the workspace as `<plugin>.<name>` (403 `forbidden` when the caller does not reach that plugin or is an
   * agent's account). It is
   * started by this request's caller; outside a request the platform refuses (403 `forbidden`). 404 `unknown_agent` when
   * there is no such agent.
   * With `options.group` the run joins a group this plugin's code founded (the `group` of a run its code started);
   * without it the run founds a new group. 404 `unknown_group` when the platform no longer remembers the group's first
   * run or another plugin founded it, 409 `agent_in_group` when the agent has a live run in the group already, 409
   * `group_full` when the group has its eight live runs.
   */
  run(agent: string, input: string, options?: AgentRunOptions): Promise<AgentRun>;
  /**
   * The agents of the workspace this request's caller may start a run of: those of the plugins they reach, this
   * plugin's own among them, sorted by plugin and name. 403 `forbidden` outside a request of a signed-in caller and
   * for an agent's account.
   */
  available(): Promise<AvailableAgent[]>;
  /** The run; 404 `not_found` when it is neither a run of this plugin's agents nor one its code started. */
  get(runId: string): Promise<AgentRun>;
  /**
   * The run, its events after the sequence number `after` (default 0: from the first; oldest first, at most
   * `limit`, default all the run keeps) and the text being written. Answered at once: follow the agent's topic to
   * know when to read again.
   */
  events(runId: string, options?: { after?: number; limit?: number }): Promise<AgentEvents>;
  /**
   * The recent runs the plugin sees (of its agents, and the runs its code started), newest first: of one agent (its
   * name, or `<plugin>.<name>` of another plugin's) or of all, of one group (`group`: the `group` of a run) or of all,
   * at most `limit` (default 50).
   */
  list(options?: { agent?: string; limit?: number; group?: string }): Promise<AgentRun[]>;
  /**
   * Sends a follow-up message to a run that has not ended, as this request's caller; resolves with the sequence
   * number of the input. 409 `run_ended` when the run is over.
   */
  send(runId: string, text: string, options?: AgentSendOptions): Promise<number>;
  /** Stops the run now; a run that ended stays as it is. */
  cancel(runId: string): Promise<AgentRun>;
  /** Ends a session after the turn in progress. */
  close(runId: string): Promise<AgentRun>;
}

const ENDED = new Set(["succeeded", "failed", "cancelled", "timed_out"]);

/**
 * A change of a run the plugin sees, as the run-change hook tells it (`plugin({..}).onRunChange(handler)`): a run of
 * one of the plugin's agents, or one its code started. It names the run and where it stands; the handler reads the
 * rest with `ctx.agents`. Delivered at least once and coalesced (the latest state of a burst): handle it
 * idempotently, and read the run rather than trust the order of deliveries.
 */
export interface RunChange {
  runId: string;
  /** The agent's name in its plugin. */
  agent: string;
  /** The agent's plugin. */
  plugin: string;
  /** `unknown`: a status this SDK does not know. */
  status: AgentRunStatus | "unknown";
  turn: number;
  /** The sequence number of the run's last event. */
  lastSeq: number;
  /** The topic the start named (empty: none), and its canonical key (empty: every key). */
  topic: string;
  topicKey: string;
  /** The run is over: nothing more is written to it. */
  ended: boolean;
}

/** The run-change hook's handler: a system request (no caller); an error makes the platform call again. */
export type RunChangeHandler = (ctx: Ctx, change: RunChange) => void | Promise<void>;

const RUN_STATUSES = new Set(["starting", "running", "idle", "succeeded", "failed", "cancelled", "timed_out"]);

/** @internal Reads the hook's body (`{run_id, agent, plugin, status, turn, last_seq, topic, topic_key, ended}`). */
export function runChangeOf(body: unknown): RunChange {
  const v = (body !== null && typeof body === "object" ? body : {}) as Record<string, unknown>;
  const get = (snakeName: string, camel: string) => v[snakeName] ?? v[camel];
  const str = (snakeName: string, camel = snakeName) => {
    const x = get(snakeName, camel);
    return typeof x === "string" ? x : "";
  };
  const num = (snakeName: string, camel = snakeName) => {
    const x = get(snakeName, camel);
    const n = typeof x === "number" ? x : typeof x === "string" && /^\d+$/.test(x) ? Number(x) : 0;
    return Number.isSafeInteger(n) && n >= 0 ? n : 0;
  };
  const runId = str("run_id", "runId");
  if (!runId) throw invalid("run_id is required");
  const raw = str("status");
  const status = raw === "queued" ? "starting" : RUN_STATUSES.has(raw) ? (raw as AgentRunStatus) : "unknown";
  const ended = v.ended;
  return {
    runId,
    agent: str("agent"),
    plugin: str("plugin"),
    status,
    turn: num("turn"),
    lastSeq: num("last_seq", "lastSeq"),
    topic: str("topic"),
    topicKey: str("topic_key", "topicKey"),
    ended: typeof ended === "boolean" ? ended : ENDED.has(status),
  };
}

function runOf(r: WireRun | null | undefined, what: string): AgentRun {
  if (!r) throw unavailable(`the platform answered ${what} with something else`);
  // a run the platform names no person at the root of was started by that person
  const root = r.onBehalfOf ?? r.startedBy;
  return {
    id: r.id,
    agent: r.agent,
    plugin: r.plugin,
    engine: r.engine,
    ...(r.model ? { model: r.model } : {}),
    status: r.status as AgentRunStatus,
    ended: ENDED.has(r.status),
    session: !!r.session,
    input: r.input ?? "",
    ...(r.result ? { result: r.result } : {}),
    ...(r.error ? { error: { code: r.error.code, message: r.error.message } } : {}),
    startedBy: { userId: r.startedBy?.userId ?? "", displayName: r.startedBy?.displayName ?? "" },
    ...(r.startedByPlugin ? { startedByPlugin: r.startedByPlugin } : {}),
    createdAt: Number(r.createdAt ?? 0),
    updatedAt: Number(r.updatedAt ?? 0),
    deadlineAt: Number(r.deadlineAt ?? 0),
    turn: Number(r.turn ?? 0),
    lastSeq: Number(r.lastSeq ?? 0),
    pod: r.pod ?? "",
    group: r.group ?? "",
    ...(r.parent ? { parent: r.parent } : {}),
    ...(r.parentAgent ? { parentAgent: r.parentAgent } : {}),
    onBehalfOf: { userId: root?.userId ?? "", displayName: root?.displayName ?? "" },
  };
}

/** The canonical key of a start's `topicKey`, encoded as the topic's key message. */
function runTopicKey(topic: TopicDef<any> | string | undefined, key: Record<string, unknown>): string {
  if (typeof topic !== "object" || topic === null) {
    throw new TypeError("agents.run: topicKey needs `topic` as a topic definition (topic(name, key))");
  }
  return canonicalKey(encodeMessage(topic.keyMessage, key));
}

/** Whole seconds of `'90s'`, `'15m'`, `'2h'`; 0 (unset) for anything else. */
function runSeconds(timeout: string | undefined): number {
  const m = typeof timeout === "string" ? /^(\d+)(s|m|h)$/.exec(timeout) : null;
  return m ? Number(m[1]) * (m[2] === "s" ? 1 : m[2] === "m" ? 60 : 3600) : 0;
}

/**
 * @internal A topic key as the platform and the clients match it: the key message's fields by their snake_case names, sorted,
 * values as strings (`{"id":"1048"}`); a key with no scalar fields is "" (every key). Takes the key in the protobuf
 * JSON mapping (camelCase); nested, array and null values are left out.
 */
export function canonicalKey(json: Record<string, unknown>): string {
  const fields: [string, string][] = [];
  for (const [k, v] of Object.entries(json)) {
    if (typeof v === "string" || typeof v === "number" || typeof v === "boolean" || typeof v === "bigint") {
      fields.push([snake(k), String(v)]);
    }
  }
  if (fields.length === 0) return "";
  fields.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return JSON.stringify(Object.fromEntries(fields));
}

/** The run a job's handler is in (`ctx.job`); outside a run its `runId` is empty and `attempt` 0. */
export interface JobInfo {
  /** The job's name (`remind_unpaid`). */
  readonly job: string;
  /** The run's id, the same in every attempt: the idempotency key of the handler's outside effects. */
  readonly runId: string;
  /** 1 on the first attempt. */
  readonly attempt: number;
  /** The attempts the run may take; `attempt === maxAttempts` on the last one. */
  readonly maxAttempts: number;
  /** The occurrence (or due time) the run stands for, the same in every attempt. */
  readonly scheduledAt: Date;
  /** Whether this is a job's run. */
  readonly isRun: boolean;
  /** Whether a failure of this attempt ends the run as failed. */
  readonly isLastAttempt: boolean;
}

/** @internal `ctx.job` outside a run. */
export const NO_RUN: JobInfo = Object.freeze({
  job: "",
  runId: "",
  attempt: 0,
  maxAttempts: 0,
  scheduledAt: new Date(0),
  isRun: false,
  isLastAttempt: false,
});

/** The state of a run; `failed`: failed for good (its retries used up, or permanent); `unknown`: one this SDK does not know. */
export type RunState = "pending" | "running" | "done" | "failed" | "cancelled" | "unknown";

/**
 * A run started by an enqueue: `runId`, and `existing` when its singleton key was taken by an earlier run (whose id and
 * state these are; its payload is kept).
 */
export interface Enqueued {
  runId: string;
  existing: boolean;
  state: RunState;
}

export interface EnqueueOptions {
  /** Due after this long (default: now). */
  delay?: Duration;
  /** Due at this time, a `Date` or unix milliseconds (at most a year ahead; a time past is now). */
  at?: Date | number;
  /**
   * At most one run per key and job while that run is kept (7 days after it ends, 30 if it failed): an enqueue with a
   * key taken answers the earlier run (`existing`) and keeps its payload.
   */
  singleton?: string;
}

/** The plugin's background jobs: `ctx.jobs`, `tx.jobs` in a declared transaction. */
export interface Jobs {
  /**
   * Starts a run of `job` (one of `plugin({ jobs })`) with `payload`, as the caller of the request or run in progress (a
   * job declared `as: 'system'` runs as the system). In a declared transaction (`tx.jobs`) the run is written in it and
   * exists only if it commits; a singleton enqueue there may wait for a concurrent transaction enqueuing the same key.
   * Errors: 400 `invalid` (the job is not the plugin's, the payload does not fit its message, `at` more than a year
   * ahead), 413 `payload_too_large` (over 64 KiB as JSON: put larger input in an entity and pass its id), 429
   * `queue_full` (10,000 pending runs of the plugin).
   */
  enqueue<P extends JobPayloadType>(job: JobDef<P>, payload: JobPayload<P>, options?: EnqueueOptions): Promise<Enqueued>;
  /**
   * Cancels one of the plugin's runs (its id, or what `enqueue` answered): a pending run never starts, a running one is
   * stopped. `false` when it had already ended; 404 `not_found` when the plugin has no such run.
   */
  cancel(run: string | Enqueued): Promise<boolean>;
}

/** The most a job's payload may weigh, as JSON. */
const MAX_PAYLOAD = 64 << 10;

const RUN_STATES: Record<string, RunState> = {
  RUN_STATE_PENDING: "pending",
  RUN_STATE_RUNNING: "running",
  RUN_STATE_DONE: "done",
  RUN_STATE_FAILED: "failed",
  RUN_STATE_CANCELLED: "cancelled",
};

/** The context of one request, or of one attempt of a job's run. */
export class Ctx {
  readonly log: Logger;
  readonly members: {
    /** Every member of the workspace. */
    list(): Promise<Member[]>;
    /** The member, or `undefined` when there is none. */
    get(userId: string): Promise<Member | undefined>;
  };
  readonly groups: { list(): Promise<Group[]> };
  /** The runs of this plugin's agents: start one, read it and its events, write to it, stop it. */
  readonly agents: Agents;
  /** Not available: both reject with `unavailable`. */
  readonly events = {
    emit: (_message: unknown) => notAvailable("ctx.events"),
    emitMany: (_messages: unknown) => notAvailable("ctx.events"),
  };
  constructor(
    private readonly session: SyscallSession,
    readonly caller: Caller,
    readonly request: RequestInfo,
    readonly plugin: PluginInfo,
    log: Logger,
    /** @internal the declared transaction this context's repository calls are part of ("0": none) */
    private readonly tx: string = "0",
    /** @internal the reason of a system context ("": the caller's) */
    private readonly system: string = "",
    /** The run a job's handler is in; outside a run its `runId` is empty and `attempt` 0. */
    readonly job: JobInfo = NO_RUN,
  ) {
    this.log = log;
    this.members = {
      list: async () => {
        const out: Member[] = [];
        let pageToken = "";
        for (;;) {
          const r = await this.syscall({ membersList: { pageToken, limit: PAGE } });
          out.push(...(r.membersPage?.members ?? []).map(memberOf));
          pageToken = r.membersPage?.nextPageToken ?? "";
          if (!pageToken) return out;
        }
      },
      get: async (userId) => {
        try {
          const r = await this.session.syscall({ memberGet: { userId } });
          return r.member ? memberOf(r.member) : undefined;
        } catch (e) {
          if (e instanceof SyscallError && e.message === "not found") return undefined;
          throw syscallFailed(e);
        }
      },
    };
    this.groups = {
      list: async () => {
        const out: Group[] = [];
        let pageToken = "";
        for (;;) {
          const r = await this.syscall({ groupsList: { pageToken, limit: PAGE } });
          out.push(...(r.groupsPage?.groups ?? []).map((g) => ({ id: g.id, name: g.name, members: g.members ?? [] })));
          pageToken = r.groupsPage?.nextPageToken ?? "";
          if (!pageToken) return out;
        }
      },
    };
    this.agents = {
      run: async (agent, input, options = {}) => {
        checkCall("ctx.agents.run()", options);
        const start = {
          agent,
          input,
          engine: options.engine ?? "",
          model: options.model ?? "",
          // left out unless said: the agent's declaration decides
          ...(options.session === undefined ? {} : { session: options.session }),
          timeoutS: runSeconds(options.timeout),
          requestId: this.request.id,
          // left out unless said: the run founds a new group
          ...(options.group === undefined ? {} : { group: options.group }),
          notify: options.notify ?? false,
          ...(options.notifyLink ? { notifyLink: linkWire(options.notifyLink) } : {}),
          ...(options.topic === undefined ? {} : { topic: typeof options.topic === "string" ? options.topic : options.topic.name }),
          ...(options.topicKey === undefined ? {} : { topicKey: runTopicKey(options.topic, options.topicKey) }),
          files: (options.files ?? []).map(fileWire),
        };
        return runOf((await this.syscall({ agentRun: start })).agentRun, "agent_run");
      },
      available: async () => {
        const r = await this.syscall({ agentsAvailable: { requestId: this.request.id } });
        return (r.agentsAvailable?.agents ?? []).map((a) => ({
          agent: a.agent,
          plugin: a.plugin,
          name: a.name,
          title: a.title,
          description: a.description ?? "",
          ...(a.icon ? { icon: a.icon } : {}),
          ...(a.engine ? { engine: a.engine } : {}),
          session: !!a.session,
          pluginTitle: a.pluginTitle,
        }));
      },
      get: async (runId) => runOf((await this.syscall({ agentGet: { runId } })).agentRun, "agent_get"),
      events: async (runId, options = {}) => {
        checkCall("ctx.agents.events()", options);
        const page = (await this.syscall({ agentEvents: { runId, after: String(options.after ?? 0), limit: options.limit ?? 0 } }))
          .agentEvents;
        const messages = (page?.partial?.messages ?? []).map((m) => ({ messageId: m.messageId, text: m.text }));
        return {
          run: runOf(page?.run, "agent_events"),
          events: (page?.events ?? []).map((e) => {
            let fields: Record<string, unknown> = {};
            try {
              fields = JSON.parse(Buffer.from(e.json ?? []).toString("utf8") || "{}");
            } catch {
              // an event the platform could not write as JSON keeps its envelope
            }
            return { ...fields, seq: Number(e.seq), at: Number(e.at), type: e.type };
          }),
          partial: { rev: Number(page?.partial?.rev ?? 0), messages, text: messages.map((m) => m.text).join("\n\n") },
        };
      },
      list: async (options = {}) => {
        checkCall("ctx.agents.list()", options);
        const r = await this.syscall({ agentRuns: { agent: options.agent ?? "", limit: options.limit ?? 0, group: options.group ?? "" } });
        return (r.agentRuns?.runs ?? []).map((run) => runOf(run, "agent_runs"));
      },
      send: async (runId, text, options = {}) => {
        checkCall("ctx.agents.send()", options);
        const files = (options.files ?? []).map(fileWire);
        const r = await this.syscall({ agentSend: { runId, text, requestId: this.request.id, files } });
        return Number(r.agentInput?.seq ?? 0);
      },
      cancel: async (runId) => runOf((await this.syscall({ agentCancel: { runId } })).agentRun, "agent_cancel"),
      close: async (runId) => runOf((await this.syscall({ agentClose: { runId } })).agentRun, "agent_close"),
    };
  }

  private async syscall(call: Record<string, unknown>): Promise<SyscallResult> {
    try {
      return await this.session.syscall(call);
    } catch (e) {
      throw syscallFailed(e);
    }
  }

  /**
   * The plugin's background jobs: `enqueue` a run, `cancel` one. In a declared transaction (`tx.jobs`) an enqueue is
   * part of it.
   */
  get jobs(): Jobs {
    return {
      enqueue: async (job, payload, options = {}) => {
        checkCall("ctx.jobs.enqueue()", options);
        const found = this.session.jobs?.get(job);
        if (!found) throw invalid("ctx.jobs.enqueue: the job is not in plugin({ jobs })");
        let json: string;
        try {
          json = stringify(encodeMessage(found.payload, payload));
        } catch (e) {
          throw invalid(`the payload is not a ${found.payload.name}: ${(e as Error).message}`);
        }
        const size = Buffer.byteLength(json, "utf8");
        if (size > MAX_PAYLOAD) {
          throw new RpcError(413, "payload_too_large",
            `the payload of ${found.name} is ${size} bytes, over 64 KiB: put larger input in an entity and pass its id`);
        }
        const when: Record<string, string> = {};
        if (options.at !== undefined) {
          const ms = options.at instanceof Date ? options.at.getTime() : options.at;
          if (!Number.isFinite(ms)) throw invalid("ctx.jobs.enqueue: `at` is not a time");
          when.atUnixMs = String(Math.trunc(ms));
        } else if (options.delay !== undefined) {
          const ms = durationMs(options.delay);
          if (ms === undefined || ms < 0) throw invalid(`ctx.jobs.enqueue: ${JSON.stringify(options.delay)} is not a delay`);
          when.delayMs = String(Math.trunc(ms));
        }
        const r = await this.syscall({
          jobEnqueue: {
            job: found.name,
            payload: json,
            ...when,
            singleton: options.singleton ?? "",
            requestId: this.request.id,
            tx: this.tx,
          },
        });
        const e = r.jobEnqueued;
        if (!e) throw internal(new Error("the platform answered an enqueue with something else"));
        return { runId: e.runId, existing: !!e.existing, state: RUN_STATES[e.state] ?? "unknown" };
      },
      cancel: async (run) => {
        const runId = typeof run === "string" ? run : run.runId;
        const r = await this.syscall({ jobCancel: { runId, requestId: this.request.id } });
        if (!r.jobCancelled) throw internal(new Error("the platform answered a cancel with something else"));
        return !!r.jobCancelled.cancelled;
      },
    };
  }

  /**
   * The plugin's files: `open(file)` reads one as a stream (`.lines()`), `create(name, type)` writes a new one
   * (`write`, then `finish` gives its `File`), `head(file)` says what is known of one (`files.ts`).
   */
  get files(): Files {
    return (
      this.session.files ??
      new PlatformFiles({ syscall: (call) => this.syscall(call), requestId: this.request.id, signal: this.request.signal })
    );
  }

  /** Aborted at the deadline or when the session ends (the same as `ctx.request.signal`); in a job's run, when it must stop. */
  get signal(): AbortSignal {
    return this.request.signal;
  }

  /** The `Idempotency-Key` header, when the caller sent one (the same as `ctx.request.idempotencyKey`). */
  get idempotencyKey(): string | undefined {
    return this.request.idempotencyKey;
  }

  /**
   * Calls an endpoint of a dependency as this request's caller, with raw JSON (protobuf JSON mapping): `input`
   * is sent as is (a bigint as a string), the answer is the parsed body. A non-2xx answer throws an `RpcError`
   * with the dependency's status, code and message. The typed clients `celobus plugin add` writes (`src/deps/`) call it.
   */
  async call<T = unknown>(plugin: string, endpoint: string, input: unknown = {}): Promise<T> {
    const request = {
      method: "POST",
      path: endpointPath(plugin, endpoint),
      query: "",
      headers: [{ name: "content-type", value: "application/json" }],
      body: Buffer.from(stringify(input)),
    };
    const r = await this.syscall({ invoke: { plugin, request, requestId: this.request.id } });
    const resp = r.invokeResponse;
    if (!resp) throw unavailable(`${plugin}: no response`);
    const text = Buffer.from(resp.body ?? []).toString("utf8");
    let body: unknown = undefined;
    try {
      body = text ? JSON.parse(text) : {};
    } catch {
      body = undefined;
    }
    if (resp.status < 200 || resp.status >= 300) {
      const err = (body ?? {}) as { code?: string; message?: string; details?: Record<string, unknown> };
      throw new RpcError(
        resp.status,
        typeof err.code === "string" ? err.code : "dependency_error",
        typeof err.message === "string" ? err.message : `${plugin}.${endpoint} answered ${resp.status}`,
        err.details && typeof err.details === "object" ? err.details : {},
      );
    }
    if (body === undefined) throw unavailable(`${plugin}.${endpoint} answered a body that is not JSON`);
    return body as T;
  }

  /**
   * Tells the screens following `topic` for `key` to re-read (plugin UI). An endpoint that declares
   * `.invalidates(..)` needs no call: the platform emits those after its 2xx answer. This is for the rest: a change made
   * outside a declared endpoint, or a key the endpoint's response does not carry. Nothing is awaited, and a screen
   * that misses it re-reads within its maximum data age.
   */
  invalidate<K>(topic: TopicDef<K>, key: K): void {
    this.session.invalidate(topic.name, canonicalKey(encodeMessage(topic.keyMessage, key)));
  }

  /** Every key of `topic` re-reads. */
  invalidateAll<K>(topic: TopicDef<K>): void {
    this.session.invalidate(topic.name, "");
  }

  /** @internal */
  protected link(): DbLink {
    return {
      syscall: (call: Record<string, unknown>) => this.syscall(call),
      requestId: this.request.id,
      tx: this.tx,
      asSystem: this.system,
      deadline: this.request.deadline,
      inRun: this.job.isRun,
    };
  }

  /**
   * The plugin's notifications: `send` a notice of a declared kind, `ask` an approval or a
   * decision and `resolve` it, `follow` / `unfollow` a key, `following(key)`. In a transaction
   * (`tx.notifications`) the writes are part of it and its members hear of them when it commits: send last in it.
   */
  get notifications(): Notifications {
    return new Notifications({ syscall: (call) => this.syscall(call), requestId: this.request.id, tx: this.tx });
  }

  /**
   * The repository of one of the plugin's entities; in a transaction (`tx.db(order)`) its calls are part of it. A
   * dependency's entity is read with `ctx.query` (500 `internal` here).
   */
  db<E extends EntityDef<any>>(entity: E & (E extends EntityDef<any, true> ? never : E extends EntityDef<any, any, true> ? never : unknown)): Repository<E> {
    if ((entity as unknown) instanceof ViewDef) throw internal(new Error("ctx.db(): a view is read, never written: read it with ctx.query()"));
    if (entity.options.storage === "olap") {
      throw internal(new Error("ctx.db(): an entity stored in OLAP is neither read nor written so: its load fills it, ctx.analytics().query() reads it"));
    }
    const provider = foreignPlugin(entity);
    if (provider) {
      throw internal(new Error(`ctx.db(): ${provider}.${entity.options.name} is another plugin's entity; read it with ctx.query()`));
    }
    const name = entityNames.get(entity) ?? entity.options.name;
    if (!name) throw internal(new Error("ctx.db(): the entity is not in plugin({ entities })"));
    return new Repository(this.link(), entity, name);
  }

  /**
   * A typed read across entities, from the root `entity`: one of the plugin's, a dependency's exported one
   * (`ctx.query(customers.customer)`, generated by `celobus plugin add`; it keeps the repository's reads too: `get`,
   * `find`, `count`, `exists`, `all`, `aggregate`), or the directory's (`platform.user` from `@celobus/sdk/platform`).
   * Joins, `where`, `groupBy`, `having`, `select`, `orderBy`, `limit`; then `all`, `first`, `count`, `page`. In a
   * transaction it is part of it; in a system context it reads the plugin's own entities within its `authz:bypass`,
   * the directory as a member does, and a dependency's entity under the provider's rules for the caller (never more
   * than the caller reads without the system context).
   */
  query<E extends Queryable>(
    entity: E & (E extends LakeTableDef<any, any> ? never : E extends EntityDef<any, any, true> ? never : unknown),
  ): Query<E> {
    return startQuery(this.link(), entity);
  }

  /**
   * The analytic copy of the workspace's entities: `ctx.analytics().query(entity)` is `ctx.query(entity)` read there,
   * for reports and figures over many rows. Its answer may be slightly behind (usually under a second, never
   * promised) and says when its data is from (`allAsOf()`, `countAsOf()`, `asOf` on a page); a screen that must show
   * what was just saved reads with `ctx.query()`. A query runs at most 10 s (422 `analytic_time_limit`) within the
   * copy's memory (422 `analytic_memory_limit`); four of the workspace run at once, two of the plugin (503
   * `analytic_busy` after a short wait); 503 `unavailable` while the copy is being made, when its data is older than
   * the workspace's limit (15 minutes unless the workspace sets another; never a fallback to `ctx.query()`) or when the
   * workspace has none. Refused inside a transaction (400 `invalid`).
   */
  analytics(): Analytics {
    return new Analytics(this.link());
  }

  /**
   * The workspace's Data Lake (`lake.ts`): `ctx.lake().table(t)` writes one of the plugin's lake tables
   * (`replace(filter, rows)`, `append(rows, { key })`, `merge(rows)`), `ctx.lake().query(t)` reads lake tables with the
   * builder of `ctx.query()`. Outside a job's run an append names its write key; in a run (`JobCtx`) it has a default.
   */
  lake(): Lake<false> {
    return new Lake<false>(this.lakeLink());
  }

  /** @internal */
  protected lakeLink() {
    const request = this.request;
    return {
      db: this.link(),
      job: this.job,
      // one count per request or run, whichever context of it appends (`asSystem` keeps the request)
      nextAppend: () => {
        const n = (appends.get(request) ?? 0) + 1;
        appends.set(request, n);
        return n;
      },
    };
  }

  /**
   * SQL that reads (`select`, `with`, `values`), with positional parameters: `$1` is the first value after the query
   * (`ctx.sql('select owner_id, count(*) as notes from hello_note group by 1 having count(*) >= $1', 2)`). It reads the
   * entities of every installed plugin (`<plugin>_<entity>`: `chat_post`) and of the directory
   * (`platform_user`, `platform_group`, `platform_group_member`, `platform_role_assignment`) as the caller: each
   * entity's own rules and masks apply; never a catalog (`pg_class`, `information_schema`, `pg_stat_activity`) or
   * another workspace: a relation outside these, or a function in `from` other than a few
   * set-returning built-ins (`generate_series`, `unnest`, the JSON element and record functions, `regexp_matches`,
   * `regexp_split_to_table`, `string_to_table`), is 403 `permission_denied`. The rows are JSON objects keyed by column name as SQL names them (snake_case;
   * a bigint is a JSON number); `R` only types them (an interface of the columns), nothing is checked. In a transaction (`tx.sql(..)`) it runs inside
   * it and sees its writes. More than 1,000 rows is 422 `query_rows_limit`; the platform's refusals are those of
   * `ctx.db`. An encrypted field reads as its ciphertext: read it with `ctx.db`.
   *
   * The values follow the query as arguments, as `$1`, `$2`, … follow in the text: a string, number, bigint,
   * boolean, `null`, `Date`, or anything JSON (an object or array is its JSON text). Each is only ever a value,
   * never SQL.
   */
  sql<R = SqlRow>(query: string, ...params: unknown[]): Promise<R[]> {
    return this.runSql<R>(query, params, false);
  }

  /**
   * SQL over the tables of every plugin of the workspace (`chat_post`, `crm_customer`) past their access rules, with
   * the capability `sql:unsafe`: a read, or with the capability `sql:write` also an `insert`, `update` or `delete`
   * (its `returning` rows come back; none without). Never the platform's tables, the directory's (read it with
   * `ctx.sql`) or a catalog (403). It runs in a system context of its own, never in a declared transaction, and each
   * run is recorded in the workspace's audit; the tables keep their soft-deleted rows, and an encrypted field is its
   * ciphertext. Parameters and rows as `ctx.sql`.
   */
  unsafeSql<R = SqlRow>(query: string, ...params: unknown[]): Promise<R[]> {
    return this.runSql<R>(query, params, true);
  }

  private async runSql<R>(query: string, params: readonly unknown[], unsafe: boolean): Promise<R[]> {
    if (typeof query !== "string" || query.trim() === "") throw internal(new Error("ctx.sql(): the query is empty"));
    const r = await this.syscall({
      sql: { requestId: this.request.id, tx: this.tx, query, params: sqlParams(params), unsafe },
    });
    if (!r.dbRows) throw internal(new Error("the platform answered SQL with something else"));
    return sqlRows(r.dbRows.rows ?? []) as R[];
  }

  /**
   * The same context with the system's rights on the plugin's own entities: its repository calls (and the
   * transactions it begins) bypass the access rules within the scope the plugin declares for `authz:bypass`
   * (outside it, 403 `capability_missing`), read masked columns, and are recorded in the workspace's audit with the
   * caller, the request and `reason`.
   */
  /**
   * The plugin's settings (`plugin({ settings: settingsDef(schema) })`): what an owner or an admin saved, else the
   * declared defaults. Read from the process's memory (the platform sends each change), so it is cheap to call in every
   * handler; the value is frozen and shared. A required setting no one has set yet is a 409 `settings_required` naming
   * it (`details.fields`, `details.missing`): the plugin needs setup. A message other than the declared one is a 500.
   */
  settings<S extends Shape>(schema: MessageDef<S>): Infer<S> {
    const cache = this.session.settings;
    if (cache === undefined) {
      throw internal(new Error(`no settings: the plugin declares none (plugin({ settings: settingsDef(${schema.name}) })), or a test gave none (testCtx(plugin, { settings }))`));
    }
    return cache.get(schema);
  }

  asSystem(reason: string): this {
    const Kind = this.constructor as typeof Ctx;
    return new Kind(this.session, this.caller, this.request, this.plugin, this.log, this.tx, reason, this.job) as this;
  }

  /**
   * A declared transaction: `fn` gets `tx`, a context whose repository calls are part of it; it commits when `fn`
   * resolves and rolls back when it throws. Inside a transaction it is nested and rolls back alone. The platform rolls back a
   * transaction still open when the request ends or 5 s pass (504 `transaction_timeout`).
   */
  async transaction<T>(fn: (tx: TxCtx) => Promise<T>, options: TransactionOptions = {}): Promise<T> {
    checkCall("ctx.transaction()", options);
    const isolation = { read_committed: "READ_COMMITTED", repeatable_read: "REPEATABLE_READ", serializable: "SERIALIZABLE" }[
      options.isolation ?? "read_committed"
    ];
    const begun = await this.syscall({
      txBegin: { requestId: this.request.id, tx: this.tx, isolation, readOnly: !!options.readOnly, asSystem: this.system },
    });
    const id = begun.txBegun?.tx;
    if (!id) throw internal(new Error("the platform answered a begin with something else"));
    const tx = new TxCtx(this.session, this.caller, this.request, this.plugin, this.log, id, this.system, this.job);
    let value: T;
    try {
      value = await fn(tx);
    } catch (e) {
      await this.syscall({ txRollback: { requestId: this.request.id, tx: id } }).catch(() => {});
      throw e;
    }
    await this.syscall({ txCommit: { requestId: this.request.id, tx: id } });
    return value;
  }
}

/**
 * The context of an attempt of a job's run (a job's handler gets it): a `Ctx` whose lake appends have a default write
 * key, the run's id and the number of the append in the run (`<runId>#1`, `#2`, ..), the same in every attempt.
 */
export class JobCtx extends Ctx {
  override lake(): Lake<true> {
    return new Lake<true>(this.lakeLink());
  }
}

/** The lake appends of each request or run so far (a run's default write key is its id and the append's number). */
const appends = new WeakMap<object, number>();

/**
 * The context inside a declared transaction (`ctx.transaction(async (tx) => ..)`): a `Ctx` whose repositories may
 * lock the rows they read (`tx.db(e).get(id, { lock: 'update' })`, `tx.db(e).find(f).forUpdate().skipLocked()`).
 */
export class TxCtx extends Ctx {
  override db<E extends EntityDef<any>>(entity: E & (E extends EntityDef<any, true> ? never : E extends EntityDef<any, any, true> ? never : unknown)): TxRepository<E> {
    const repo = super.db(entity);
    return new TxRepository(this.link(), repo.entity, repo.name);
  }
}
