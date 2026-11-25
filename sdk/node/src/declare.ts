// Declarations of identity and behavior: roles, endpoints, background jobs and the plugin's options. Names come from
// the keys these objects are registered under in `plugin({...})`, so a declaration carries no name of its own.
import { noted } from "./keys.js";
import type { Ctx, JobCtx, RoleLike, RunChangeHandler } from "./ctx.js";
import type { NotificationKind } from "./notify.js";
import type { EntityDef, EnumDef, Infer, IoType, IoValue, MessageDef, Shape, ViewDef } from "./schema.js";
import type { InvalidationLike, TopicLike, UiDoc } from "./ui.js";
import type { LakeTableDef, LayerDef } from "./lake.js";

export type DefaultFor = "none" | "member" | "admin";

export interface RoleOptions {
  title?: string;
  description?: string;
  /** Only explicit assignment: `workspace.admin` does not substitute for it. */
  strict?: boolean;
  /** Whom the role is given to by default when the plugin is installed. */
  defaultFor?: DefaultFor;
}

/** A role of the plugin; its name is its key in `plugin({ roles })`, in snake_case. */
export class RoleDef {
  readonly __role = true as const;
  constructor(readonly options: RoleOptions) {}
}

/** `export const manager = role({ title: 'Order manager', strict: true })`. */
export function role(options: RoleOptions = {}): RoleDef {
  return noted(new RoleDef(options), "role", options);
}

/** The platform's base roles, by their full names. */
export const workspace = {
  owner: "workspace.owner",
  admin: "workspace.admin",
  member: "workspace.member",
  guest: "workspace.guest",
} as const;

export type Auth = "user" | "service" | "any" | "public";
export type Idempotency = "off" | "optional" | "required";
/** Milliseconds, or a string: `'500ms'`, `'10s'`, `'2m'`. */
export type Timeout = number | `${number}ms` | `${number}s` | `${number}m`;

export interface EndpointOptions<I extends IoType, O extends IoType> {
  description?: string;
  /** A message (`t.message(...)`), an entity, or inline fields (the message `<Name>Request`). */
  input: I;
  /** A message, an entity (the handler returns a row), or inline fields (the message `<Name>Reply`). */
  output: O;
  /** Any of these roles (default: any member who passes the scopes). Checked by the platform, not the SDK. */
  roles?: readonly RoleLike[];
  /** All of these scopes; default `['write']`, or `['read']` when `readOnly`. */
  scopes?: readonly string[];
  readOnly?: boolean;
  /** Default `user`. */
  auth?: Auth;
  /** Default `off`. */
  idempotency?: Idempotency;
  /** Default 30 s, at most 600 s. */
  timeout?: Timeout;
  /** Screens may call it: queries (when it is `readOnly`) and actions. Other endpoints stay out of reach of a UI. */
  ui?: boolean;
  /**
   * After a 2xx answer the platform invalidates these topics (every key: `ORDERS`; or the key taken from the response:
   * `ORDER.by(orderKey.f.id, order.f.id)`); open screens following them re-read.
   */
  invalidates?: readonly InvalidationLike[];
  handler(ctx: Ctx, input: IoValue<I>): Promise<IoValue<O>> | IoValue<O>;
}

/** An endpoint; its name is its key in `plugin({ endpoints })` in PascalCase (`placeOrder` → `PlaceOrder`). */
export class EndpointDef<I extends IoType = IoType, O extends IoType = IoType> {
  /** @internal The declaration this one was derived from (`.ui()` and the rest return new ones); UI references
   * resolve through it, so `ui.call(listOrders)` names the endpoint registered as `listOrders.ui()`. */
  readonly root: EndpointDef<any, any>;

  constructor(
    readonly options: EndpointOptions<I, O>,
    root?: EndpointDef<any, any>,
  ) {
    this.root = root ?? this;
  }

  private with(extra: Partial<EndpointOptions<I, O>>): EndpointDef<I, O> {
    return new EndpointDef({ ...this.options, ...extra }, this.root);
  }

  /** Screens may call it (see `EndpointOptions.ui`). */
  ui(): EndpointDef<I, O> {
    return this.with({ ui: true });
  }

  /** The endpoint only reads: its scope is `read`, and UI queries may call it. */
  readOnly(): EndpointDef<I, O> {
    return this.with({ readOnly: true });
  }

  /** After a 2xx answer the platform invalidates the topic (see `EndpointOptions.invalidates`). */
  invalidates(...topics: InvalidationLike[]): EndpointDef<I, O> {
    return this.with({ invalidates: [...(this.options.invalidates ?? []), ...topics] });
  }
}

export function endpoint<I extends IoType, O extends IoType>(options: EndpointOptions<I, O>): EndpointDef<I, O> {
  return noted(new EndpointDef(options), "endpoint", options);
}

/**
 * @internal The name each endpoint has in its plugin's descriptor and its request message (an inline input's
 * generated one), set when the plugin is described: what an ask's button sends.
 */
export const describedEndpoints = new WeakMap<EndpointDef<any, any>, { name: string; input: MessageDef<any> | EntityDef<any> }>();

/** Milliseconds, or a string: `'500ms'`, `'10s'`, `'2m'`, `'1h'`. */
export type Duration = number | `${number}ms` | `${number}s` | `${number}m` | `${number}h`;

/** @internal Milliseconds of a duration, or `undefined` when it cannot be read. */
export function durationMs(d: unknown): number | undefined {
  if (typeof d === "number") return Number.isFinite(d) ? d : undefined;
  const m = typeof d === "string" ? /^(\d+(?:\.\d+)?)(ms|s|m|h)$/.exec(d) : null;
  return m ? Number(m[1]) * { ms: 1, s: 1000, m: 60_000, h: 3_600_000 }[m[2] as "ms" | "s" | "m" | "h"] : undefined;
}

/** A job's payload: a message (`t.message(...)`) or inline fields (the message named as the job, in PascalCase). */
export type JobPayloadType = MessageDef<any> | Shape;
/** The value of a job's payload in its handler and in `ctx.jobs.enqueue`. */
export type JobPayload<P> = P extends MessageDef<infer S> ? Infer<S> : P extends Shape ? Infer<P> : never;

/**
 * How a job's failed attempts are retried: `retries` attempts after the first (at most 20), the first `delay` after
 * the failure (at most an hour), then the same delay (`fixed`) or doubling (`exponential`, each ±20 %, at most an
 * hour). Unset members: 3 retries, 30 s, exponential. `{ retries: 0 }`: one attempt only.
 */
export interface JobRetry {
  retries?: number;
  delay?: Duration;
  backoff?: "fixed" | "exponential";
}

export interface JobOptions<P extends JobPayloadType> {
  /** The payload message, or inline fields (`{ olderThanDays: t.int() }`: the message `RemindUnpaid` of `remindUnpaid`). */
  payload: P;
  /** What the job does, for the workspace's list of jobs: plain text or a key of the UI's text catalog. */
  description?: string;
  /**
   * A schedule: five fields in UTC, `minute hour day-of-month month day-of-week` (`'0 9 * * 1-5'`: weekdays at 09:00
   * UTC). Each occurrence starts a run with the payload's defaults; an occurrence that comes due while the previous
   * scheduled run is still pending or running is not started. Unset: the job runs only when enqueued.
   */
  cron?: string;
  /** `'system'`: its runs run as the plugin's system principal, whoever enqueued them. Unset: as the enqueuer. */
  as?: "system";
  /** Unset: 3 retries, 30 s apart, backing off exponentially. */
  retry?: JobRetry;
  /** How long an attempt may run, 1 s to 1 h (default 5 minutes); past it `ctx.signal` aborts and the attempt fails as `timeout`. */
  timeout?: Duration;
  /** How many runs of the job run at once in the workspace, 1 to 64 (default 1). */
  concurrency?: number;
  /** The job's name instead of its key in snake_case: snake_case, at most 32 characters. */
  name?: string;
  /**
   * The run: `ctx.job` says which (run id, attempt), `ctx.signal` aborts when it must stop. Resolving ends the attempt
   * as done; throwing fails it, and the run is retried unless the error is permanent (`permanent(..)`, an error marked
   * `.asPermanent()`, or `invalid`, `not_found`, `permission_denied`, `payload_invalid`). Its lake appends have a
   * default write key (`JobCtx`).
   */
  handler(ctx: JobCtx, payload: JobPayload<P>): Promise<void> | void;
}

/**
 * A background job; its name is its key in `plugin({ jobs })` in snake_case (`remindUnpaid` → `remind_unpaid`), and
 * `ctx.jobs.enqueue(remindUnpaid, {..})` starts a run of it.
 */
export class JobDef<P extends JobPayloadType = JobPayloadType> {
  readonly __job = true as const;
  constructor(readonly options: JobOptions<P>) {}
}

/**
 * `export const remindUnpaid = job({ payload: { olderThanDays: t.int() }, cron: '0 9 * * 1-5', as: 'system',
 * async handler(ctx, p) { .. } })`, listed in `plugin({ jobs: { remindUnpaid } })`.
 */
export function job<P extends JobPayloadType>(options: JobOptions<P>): JobDef<P> {
  return noted(new JobDef(options), "job", options);
}

export interface ScopeOptions {
  /** Consent text. */
  title?: string;
  description?: string;
  sensitive?: boolean;
}

export interface Dependency {
  plugin: string;
  /** The endpoints this plugin calls (`['GetCustomer']`); the platform refuses others. */
  calls?: readonly string[];
  /**
   * The dependency's exported entities this plugin reads (`ctx.query`), with the fields it reads and their types,
   * as `celobus plugin add` generates them; the platform refuses other reads.
   */
  reads?: readonly DependencyRead[];
}

/** One entity of a dependency this plugin reads: its name there and the fields read, each with its descriptor type. */
export interface DependencyRead {
  entity: string;
  fields: readonly { name: string; type: string }[];
}

/** The coding-agent CLI that runs an agent. */
export type AgentEngine = "claude" | "codex" | "pi";

/** How long an agent's run may take: `'90s'`, `'15m'`, `'2h'`. */
export type AgentTimeout = `${number}s` | `${number}m` | `${number}h`;

/**
 * An agent of the plugin: a service account with the roles it holds, and how its runs are made. Its system
 * prompt, its skills and its extra MCP servers are files of the plugin: `agents/<name>/soul.md`,
 * `agents/<name>/skills/<skill>/SKILL.md`, `agents/<name>/mcp.json`.
 */
export interface AgentOptions {
  roles?: readonly RoleLike[];
  /**
   * The workspace permissions the agent asks for (`['plugins.install']`): with one, the agent does the workspace
   * operations that take it as a workspace admin would. Whoever installs the plugin sees the list.
   */
  workspace?: readonly string[];
  /**
   * The agents this one may bring into its group when they are not there: an agent of this plugin by its name, or
   * `<dependency>.<agent>`. Messaging an agent that is in the group already needs no entry. Whoever installs the
   * plugin sees the list.
   */
  delegates?: readonly string[];
  /**
   * The `llm` connection its runs use: a connection of this plugin by its key (`research`), or `<dependency>.<name>`
   * (`ai.anthropic`). An owner or admin may choose another one of the plugin or its dependencies in the agent's
   * settings. Whoever installs the plugin sees it.
   */
  connection?: string;
  /**
   * The agent has an SSH key of its own (ed25519, one per workspace), in `~/.ssh` of its runs: its public key is shown
   * in the workspace's settings, to add to the agent's account at GitHub or GitLab.
   */
  sshKey?: boolean;
  /**
   * The agent's service account asks for base role admin instead of member: it does whatever a workspace admin does,
   * not only the operations of its `workspace` permissions. Only an owner installs or upgrades to a version that asks
   * for it, and whoever installs the plugin sees it; an owner may still change the account's role. Never owner.
   */
  admin?: boolean;
  /** The agent's display name. */
  title?: string;
  /** What the agent is for, in a sentence. */
  description?: string;
  /**
   * The agent's own glyph, a lucide icon's name (`rocket`, `git-pull-request`): the client draws it over its engine's
   * mark wherever the agent is shown, so two agents on one engine are told apart.
   */
  icon?: string;
  /**
   * The engine of its runs, unless whoever starts one names another (default: the one its connection's protocol picks:
   * Claude Code for `anthropic`, Codex for `openai_responses`, Pi for `openai_chat`).
   */
  engine?: AgentEngine;
  /**
   * The model of its runs when its connection lists it, unless whoever starts a run or the workspace's settings name
   * another (default: the connection's default model).
   */
  model?: string;
  /** Its runs are sessions: they wait for follow-up messages after every turn. */
  session?: boolean;
  /** The longest a run takes, from a minute to six hours (default: 30 minutes). */
  timeout?: AgentTimeout;
  /** The shared skills of the plugin (`skills/<name>/`) the agent has besides its own. */
  skills?: readonly string[];
  /**
   * A topic of the plugin (one of `plugin({ topics })`) the platform invalidates, for every key, whenever a run of the
   * agent changes: its status, a new event, the text being written. Screens that follow it re-read the run through
   * the plugin's endpoints (`ctx.agents.get`, `ctx.agents.events`).
   */
  topic?: TopicLike;
}

/**
 * A connection the plugin declares: installing the plugin creates it, not set up, in the
 * workspace, and an owner or admin fills it in. `llm`: a language model provider for agents; `stt`: speech recognition
 * for dictation. The plugin's code never sees it.
 */
export interface ConnectionOptions {
  kind: "llm" | "stt";
  /** Its display name (at most 80 characters). */
  title?: string;
  /** What it is for, in a sentence (at most 500 characters). */
  description?: string;
}

/**
 * What a system context (`ctx.asSystem(reason)`) may do on an entity: the scope of `authz:bypass`
 * (`capabilities: { 'authz:bypass': { reason, on: [{ entity: note, ops: ['read'] }] } }`).
 */
export interface BypassScope {
  entity: EntityDef<any>;
  ops: readonly ("read" | "create" | "update" | "delete")[];
}

export interface PluginOptions {
  /** `^[a-z][a-z0-9]{1,15}$`, not reserved. */
  id: string;
  /** Semver; by default the `version` of the plugin's package.json. */
  version?: string;
  /** Default: the id, humanized. */
  title?: string;
  description?: string;
  roles?: Record<string, RoleDef>;
  /** Extra scopes and the titles of `read` and `write` (implicit). */
  scopes?: Record<string, ScopeOptions>;
  /** `{ 'push:send': 'Reminds clerks of their unpaid orders.' }`: a reason for each. */
  capabilities?: Record<string, string | { reason: string; on?: readonly BypassScope[] }>;
  /** Directory data beyond names: `['members.email']`. */
  permissions?: readonly string[];
  /**
   * The plugin's agents by name (`assistant: { roles: [manager], engine: 'claude' }`); a camelCase key is the
   * snake_case name (`orderHelper` is `order_helper`).
   */
  agents?: Record<string, AgentOptions>;
  /** The plugin's connections by name (`anthropic: { kind: 'llm', title: 'Shared Anthropic' }`); a camelCase key is the snake_case name. */
  connections?: Record<string, ConnectionOptions>;
  dependencies?: readonly (string | Dependency)[];
  entities?: Record<string, EntityDef<any>>;
  /**
   * The plugin's analytic views by key (`views: { sales, salesByMonth }`, each `view({..})`); a camelCase key is the
   * snake_case name (`sales_by_month`), its row message the PascalCase one (`SalesByMonth`).
   */
  views?: Record<string, ViewDef<any>>;
  /**
   * The layers of the plugin's part of the Data Lake by key (`layers: { raw, core }`, each `layer({..})`); a camelCase
   * key is the snake_case name.
   */
  layers?: Record<string, LayerDef>;
  /**
   * The plugin's lake tables by key (`lakeTables: { stockFile, stock }`, each `lakeTable({ layer, columns, .. })`); a
   * camelCase key is the snake_case name unless the table names itself.
   */
  lakeTables?: Record<string, LakeTableDef<any, false>>;
  /** Other plugins' exported lake tables this plugin reads (`lakeTable({ plugin: 'retail', name: 'stock', columns })`). */
  lakeDependencies?: readonly LakeTableDef<any, true>[];
  /**
   * Other plugins' entities whose exported histories this plugin reads (`ctx.lake().history(e)`; the entity as
   * declared with its owner and name, `entity({ plugin: 'helpdesk', name: 'ticket', columns })`). In the descriptor each is
   * named in its dependency's `lake`, after the lake tables of `lakeDependencies`.
   */
  historyDependencies?: readonly EntityDef<any, true>[];
  endpoints?: Record<string, EndpointDef<any, any>>;
  /**
   * The kinds of notification the plugin sends (`notification('mention', {..})`, `ctx.notifications`); they need the
   * capability `push:send`.
   */
  notifications?: readonly NotificationKind[];
  /**
   * The plugin's background jobs by key (`jobs: { remindUnpaid }`); a camelCase key is the snake_case name
   * (`remind_unpaid`).
   */
  jobs?: Record<string, JobDef<any>>;
  /** How many runs of its jobs one process runs at once, apart from its requests: 1 to 64 (default 16). */
  jobSlots?: number;
  /** Messages no endpoint or entity uses (they follow the others in the descriptor). */
  messages?: readonly MessageDef<any>[];
  /** Enums no message uses. */
  enums?: readonly EnumDef<any>[];
  /** Topics screens follow and endpoints invalidate (`[ORDERS.roles([clerk]), ORDER.authorize(getOrder)]`). */
  topics?: readonly TopicLike[] | Record<string, TopicLike>;
  /** The plugin's UI: `ui.define({ nav, screens, commands, ... })`. */
  ui?: UiDoc;
  /**
   * The run-change hook (or `plugin({..}).onRunChange(handler)`): the platform
   * calls it when a run the plugin sees changes. The descriptor then says `runs: {on_change: true}`.
   */
  onRunChange?: RunChangeHandler;
}
