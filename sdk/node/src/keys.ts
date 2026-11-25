// The options each declaration helper knows, and the ones it was given that it does not: a plugin written in plain
// JavaScript (or against a newer SDK) that passes an option this SDK does not know would otherwise declare something
// else than it says, silently. A helper notes the unknown keys of its options on what it returns; `--describe` and
// `plugin.check()` report them as definition errors (`E_OPTION_UNKNOWN`), naming the key and the helper.
//
// Each list is an object literal typed by its options' interface: a key the interface gains and the list lacks, or a
// key the list has that the interface does not, is a type error here, so the lists follow the interfaces.
import type { AgentOptions, BypassScope, ConnectionOptions, Dependency, DependencyRead, EndpointOptions, JobOptions, JobRetry, PluginOptions, RoleOptions, ScopeOptions } from "./declare.js";
import type { LakeTableOptions, LayerOptions } from "./lake.js";
import type { NotificationOptions } from "./notify.js";
import type { EntityOptions, ForeignEntityOptions, HistoryOptions, RefOptions, Shape, ViewOptions } from "./schema.js";
import type { UiOptions } from "./ui.js";
import type { AgentRunOptions, EnqueueOptions, TransactionOptions } from "./ctx.js";
import type { TxGetOptions } from "./db.js";
import type { AppendOptions, RefreshOptions } from "./lake.js";
import type { Ask, Follow, Notice } from "./notify.js";
import { internal } from "./errors.js";

/** Every key of `T`, each once: the object literal must name them all and nothing else. */
const all = <T>(keys: { [K in keyof Required<T>]: true }): readonly string[] => Object.keys(keys);

export const KEYS = {
  plugin: all<PluginOptions>({
    id: true, version: true, title: true, description: true, roles: true, scopes: true, capabilities: true,
    permissions: true, agents: true, connections: true, dependencies: true, entities: true, views: true, layers: true,
    lakeTables: true, lakeDependencies: true, historyDependencies: true, endpoints: true, notifications: true, jobs: true,
    jobSlots: true, messages: true, enums: true, topics: true, ui: true, onRunChange: true,
  }),
  role: all<RoleOptions>({ title: true, description: true, strict: true, defaultFor: true }),
  scope: all<ScopeOptions>({ title: true, description: true, sensitive: true }),
  capability: all<{ reason: string; on?: readonly BypassScope[] }>({ reason: true, on: true }),
  bypass: all<BypassScope>({ entity: true, ops: true }),
  agent: all<AgentOptions>({
    roles: true, workspace: true, delegates: true, connection: true, sshKey: true, admin: true, title: true,
    description: true, icon: true, engine: true, model: true, session: true, timeout: true, skills: true, topic: true,
  }),
  connection: all<ConnectionOptions>({ kind: true, title: true, description: true }),
  dependency: all<Dependency>({ plugin: true, calls: true, reads: true }),
  dependencyRead: all<DependencyRead>({ entity: true, fields: true }),
  entity: all<EntityOptions<Shape>>({
    plugin: true, name: true, description: true, key: true, softDelete: true, version: true, appendOnly: true,
    label: true, columns: true, indexes: true, access: true, storage: true, olapKey: true, load: true, history: true,
  }),
  foreignEntity: all<ForeignEntityOptions<Shape>>({
    plugin: true, name: true, description: true, key: true, softDelete: true, version: true, storage: true, columns: true,
  }),
  index: all<{ columns: unknown; unique?: boolean }>({ columns: true, unique: true }),
  history: all<HistoryOptions>({ layer: true, all: true, unmasked: true, read: true, systemRead: true, export: true }),
  view: all<ViewOptions<Shape>>({ name: true, description: true, columns: true, query: true }),
  layer: all<LayerOptions>({ title: true }),
  lakeTable: all<LakeTableOptions<Shape>>({
    layer: true, plugin: true, name: true, message: true, description: true, columns: true, partition: true, key: true,
    read: true, systemRead: true, export: true, exportToModels: true, model: true,
  }),
  endpoint: all<EndpointOptions<any, any>>({
    description: true, input: true, output: true, roles: true, scopes: true, readOnly: true, auth: true,
    idempotency: true, timeout: true, ui: true, invalidates: true, handler: true,
  }),
  job: all<JobOptions<any>>({
    payload: true, description: true, cron: true, as: true, retry: true, timeout: true, concurrency: true, name: true,
    handler: true,
  }),
  retry: all<JobRetry>({ retries: true, delay: true, backoff: true }),
  notification: all<NotificationOptions>({ title: true, description: true, tab: true, digest: true }),
  ref: all<RefOptions>({ nav: true, inverse: true, onDelete: true }),
  message: all<{ description?: string; omitDefaults?: boolean }>({ description: true, omitDefaults: true }),
  ui: all<UiOptions>({ nav: true, landing: true, home: true, commands: true, screens: true, overlays: true, search: true, texts: true }),
} as const;

export type Helper = keyof typeof KEYS;

/** The helpers' names as authors call them. */
const CALLS: Record<Helper, string> = {
  plugin: "plugin()", role: "role()", scope: "a scope of plugin({ scopes })", capability: "a capability of plugin({ capabilities })",
  bypass: "a bypass scope (capabilities['authz:bypass'].on)", agent: "an agent of plugin({ agents })",
  connection: "a connection of plugin({ connections })", dependency: "a dependency of plugin({ dependencies })",
  dependencyRead: "a dependency's reads", entity: "entity()", foreignEntity: "entity() of another plugin",
  index: "an index of entity({ indexes })", history: "entity({ history })", view: "view()", layer: "layer()",
  lakeTable: "lakeTable()", endpoint: "endpoint()", job: "job()", retry: "job({ retry })", notification: "notification()",
  ref: "t.ref()", message: "t.message()", ui: "ui.define()",
};

/** The keys of `value` that `helper` does not know (none when it is not an object). */
export function unknownKeys(helper: Helper, value: unknown): string[] {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return [];
  const known = KEYS[helper];
  return Object.keys(value).filter((k) => !known.includes(k));
}

/** A problem of an unknown option, as the descriptor reports it. */
export interface OptionProblem {
  key: string;
  message: string;
  help: string;
}

/** The problems of `value`'s keys for `helper`. */
export function optionProblems(helper: Helper, value: unknown): OptionProblem[] {
  return unknownKeys(helper, value).map((key) => ({
    key,
    message: `${CALLS[helper]} has no option \`${key}\`: this SDK does not know it, and would declare something else than it says`,
    help: `known: ${KEYS[helper].join(", ")} (a newer feature needs a newer @celobus/sdk)`,
  }));
}

/**
 * The options each call of a context takes (`ctx.lake().refresh(t, { partition })`, `ctx.transaction(fn, { .. })`, ..).
 * A key a call does not know refuses the call before anything is sent (500 `internal`, naming the key and the call):
 * a misspelt `partiton` would otherwise refresh the whole table.
 */
export const CALL_KEYS = {
  "ctx.lake().refresh()": all<RefreshOptions<Shape>>({ partition: true }),
  "ctx.lake().table(t).append()": all<AppendOptions>({ key: true }),
  "get()": all<TxGetOptions>({ withDeleted: true, include: true, lock: true, wait: true }),
  "ctx.transaction()": all<TransactionOptions>({ isolation: true, readOnly: true }),
  "ctx.jobs.enqueue()": all<EnqueueOptions>({ delay: true, at: true, singleton: true }),
  "ctx.agents.run()": all<AgentRunOptions>({
    engine: true, model: true, session: true, timeout: true, group: true, notify: true, notifyLink: true, topic: true, topicKey: true,
  }),
  "ctx.agents.events()": all<{ after?: number; limit?: number }>({ after: true, limit: true }),
  "ctx.agents.list()": all<{ agent?: string; limit?: number; group?: string }>({ agent: true, limit: true, group: true }),
  "ctx.notifications.send()": all<Notice>({
    agent: true, noActor: true, to: true, followers: true, except: true, key: true, title: true, body: true, source: true,
    link: true, once: true,
  }),
  "ctx.notifications.ask()": all<Ask>({
    agent: true, noActor: true, kind: true, key: true, to: true, title: true, body: true, source: true, link: true,
    review: true, buttons: true,
  }),
  "ctx.notifications.follow()": all<Follow>({ reason: true, noun: true, users: true }),
  "notify.button()": all<{ primary?: boolean; icon?: string }>({ primary: true, icon: true }),
} as const;

/** Refuses a call given an option it does not know: throws, naming the key and the call. */
export function checkCall(call: keyof typeof CALL_KEYS, value: unknown): void {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return;
  const known = CALL_KEYS[call];
  const key = Object.keys(value).find((k) => !known.includes(k));
  if (key !== undefined) {
    throw internal(
      new Error(`${call} has no option \`${key}\`: this SDK does not know it, and would do something else than asked (known: ${known.join(", ")})`),
    );
  }
}

/** @internal The unknown options a helper was given, noted on what it returned. */
export const givenUnknown = new WeakMap<object, OptionProblem[]>();

/** @internal Notes on `def` the unknown keys of `options` for `helper`; returns `def`. */
export function noted<D extends object>(def: D, helper: Helper, options: unknown): D {
  const problems = optionProblems(helper, options);
  if (problems.length > 0) givenUnknown.set(def, problems);
  return def;
}
