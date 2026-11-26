// The descriptor (schema 1): what `--describe` prints, the same JSON
// in every SDK. Every member of the schema is emitted, with its default when not declared (empty strings, false,
// empty arrays), so all SDKs give the same shape.
//
// Order rules (arrays keep this order; object keys are sorted by the CLI when it canonicalizes):
// - roles, scopes, capabilities, agents, entities, endpoints: as declared (the key order of `plugin({...})`);
// - messages: in order of first use while walking the endpoints as declared (each endpoint's input, then its
//   output; a message's nested messages right after it, depth first, in field order), then the entities'
//   messages in entity order, then the extra `messages`, then the jobs' payloads (in job order), then the topics'
//   key messages, then the views' rows (in view order), then the lake tables' rows (in table order), then the
//   settings message;
// - dependencies and their reads: as declared;
// - enums: in order of first use while walking those messages' fields, then the extra `enums`;
// - fields: declaration order; an entity's message has `id` first, then its columns. `number` is the 1-based
//   position.
import { messageFields } from "./codec.js";
import { runChangeOf, type RoleLike } from "./ctx.js";
import { describeAccess } from "./access.js";
import { cronError } from "./cron.js";
import { EndpointDef, JobDef, RoleDef, describedEndpoints, durationMs, type EndpointOptions, type JobOptions, type PluginOptions } from "./declare.js";
import { NotificationKind } from "./notify.js";
import { jsonName, humanize, pascal, snake } from "./names.js";
import { EntityDef, EnumDef, Field, MessageDef, PlatformEntity, ViewDef, entityNames as describedNames, foreignPlugin, navigations, viewNames, type AnyField, type IoType, type Shape } from "./schema.js";
import { describeLoad, describeModel, describeView, type DescribedLoad, type DescribedModel, type DescribedView } from "./query.js";
import { LakeTableDef, LayerDef, lakeNames } from "./lake.js";
import { givenUnknown, optionProblems, type Helper } from "./keys.js";
import { TopicDecl, TopicDef, UiDoc, invalidationJson, resolveUi, settingsFormJson, topicDecl, type TopicLike } from "./ui.js";
import { SettingsDef, settingsChangeOf } from "./settings.js";
import { completeForm, deriveForm, settingsErrors, settingsKeys, type DescribedSettings, type SettingsKeys } from "./settings-describe.js";

/** A layer's or a lake table's name: snake_case, at most 32 characters. */
const LAKE_NAME = /^[a-z][a-z0-9_]{0,31}$/;

/** The columns every history has besides the entity's `id` and its kept ones: no kept column takes their names. */
const HISTORY_COLUMNS = ["valid_from", "valid_to", "change", "changed_by", "changed_by_kind", "changed_by_plugin", "day", "seq"];

/** The scalar types a lake column holds. */
const LAKE_SCALARS = ["string", "bool", "int32", "int64", "double", "decimal", "uuid", "timestamp", "date", "bytes", "json"];

/** Whether a field is of a type a lake column holds: a single scalar, enum or ref (never a list or a message). */
function lakeType(f: DescriptorField): boolean {
  return !f.repeated && (LAKE_SCALARS.includes(f.type) || f.type.startsWith("enum:") || f.type.startsWith("ref:"));
}

export interface DescriptorField extends SettingsKeys {
  name: string;
  number: number;
  type: string;
  repeated: boolean;
  optional: boolean;
  description: string;
  /** Entity columns only, each written only when declared. */
  unique?: true;
  index?: true;
  encrypted?: true;
  searchable?: string;
  nav?: string;
  inverse?: string;
  on_delete?: string;
  /** The entity's history keeps the column's changes; entity columns only, written only when set. */
  history?: true;
  /** A file field's largest file in bytes and its accepted types; each written only when declared. */
  max_size?: number;
  accept?: string[];
}

/** `on_delete` of a ref within the plugin, and of one to another plugin or the directory. */
const ON_DELETE_OWN = ["restrict", "cascade", "set_null", "no_action"];
const ON_DELETE_CROSS = ["keep", "set_null"];

/** The languages of a `searchable` column: PostgreSQL's text search configurations. */
const SEARCH_LANGUAGES = [
  "simple", "arabic", "armenian", "basque", "catalan", "danish", "dutch", "english", "estonian", "finnish", "french",
  "german", "greek", "hindi", "hungarian", "indonesian", "irish", "italian", "lithuanian", "nepali", "norwegian",
  "portuguese", "romanian", "russian", "serbian", "spanish", "swedish", "tamil", "turkish", "yiddish",
];
const MAX_SEARCHABLE = 8;
const NAV_NAME = /^[a-z][a-z0-9_]{0,62}$/;

export interface Descriptor {
  schema: 1;
  id: string;
  version: string;
  title: string;
  description: string;
  roles: { name: string; title: string; description: string; strict: boolean; default_for: string }[];
  scopes: { name: string; title: string; description: string; sensitive: boolean }[];
  /** `on`: the scope of `authz:bypass`, written only when declared. */
  capabilities: { name: string; reason: string; on?: { entity: string; ops: string[] }[] }[];
  permissions: string[];
  agents: {
    name: string;
    roles: string[];
    workspace?: string[];
    delegates?: string[];
    connection?: string;
    ssh_key?: true;
    admin?: true;
    title?: string;
    description?: string;
    icon?: string;
    engine?: string;
    model?: string;
    session?: true;
    timeout_s?: number;
    skills?: string[];
    topic?: string;
  }[];
  /** Written only when declared. */
  connections?: { name: string; kind: string; title?: string; description?: string }[];
  /** `reads`: written only when declared. */
  dependencies: {
    plugin: string;
    calls: string[];
    reads?: { entity: string; fields: { name: string; type: string }[] }[];
    /** The dependency's exported lake tables read; written only when declared. */
    lake?: string[];
  }[];
  enums: { name: string; values: string[] }[];
  /** `kind`: `job` for a job's payload, `settings` for the settings message. */
  messages: { name: string; kind: "message" | "job" | "settings"; description: string; fields: DescriptorField[] }[];
  entities: {
    name: string;
    message: string;
    key: string;
    label: string;
    soft_delete: boolean;
    version: boolean;
    append_only: boolean;
    fields: DescriptorField[];
    /** Written only when declared. */
    indexes?: { columns: { name: string; desc?: true }[]; unique?: true }[];
    /** The access rules; written only when declared. */
    access?: Record<string, unknown>;
    /** The history in the Data Lake; written only when declared, each optional member only when set. */
    history?: DescribedHistory;
    /** `olap`: its rows are kept in the analytic copy only; written only when declared. */
    storage?: "olap";
    /** The fields that key a row of an entity stored in OLAP; written only when declared. */
    olap_key?: string[];
  }[];
  endpoints: {
    name: string;
    input: string;
    output: string;
    description: string;
    auth: string;
    roles: string[];
    scopes: string[];
    idempotency: string;
    timeout_ms: number;
    /** `ui` endpoints only (written when true). */
    read_only?: boolean;
    /** Written when true. */
    ui?: boolean;
    /** Written when the endpoint invalidates topics. */
    invalidates?: Record<string, unknown>[];
  }[];
  /** Written only when declared, so plugins without them describe as before. */
  topics?: {
    name: string;
    key: string;
    description: string;
    roles?: string[];
    scopes?: string[];
    authorize?: string;
    manual?: boolean;
  }[];
  /** The UI document (`ui.define(..)`), written only when declared. */
  ui?: Record<string, unknown>;
  /** The notification kinds; written only when declared, every member of each. */
  notifications?: { name: string; title: string; description: string; tab: string; digest: boolean }[];
  /** What the plugin asks of agent runs; written only when it handles the run-change hook. */
  runs?: { on_change: true };
  /** The background jobs; written only when declared, each optional member only when declared. */
  jobs?: {
    name: string;
    payload: string;
    description?: string;
    cron?: string;
    as_system?: true;
    retry?: { retries: number; delay_ms: number; backoff: "fixed" | "exponential" };
    timeout_ms?: number;
    concurrency?: number;
  }[];
  /** The analytic views; written only when declared. */
  views?: DescribedView[];
  /** The layers of the Data Lake; written only when declared, `title` only when set. */
  layers?: { name: string; title?: string }[];
  /** The lake tables; written only when declared, each optional member only when set. */
  lake_tables?: DescribedLakeTable[];
  /** The models of lake tables; written only when declared. */
  lake_models?: DescribedModel[];
  /** The loads of the entities stored in OLAP; written only when declared. */
  loads?: DescribedLoad[];
  /** The settings: their message, whether the plugin checks a change, and their form; written only when declared. */
  settings?: DescribedSettings;
}

/** An entity's history as the descriptor has it. */
export interface DescribedHistory {
  layer: string;
  all?: true;
  unmasked?: true;
  read?: string[];
  system_read?: true;
  export?: true;
}

/** A lake table as the descriptor has it. */
export interface DescribedLakeTable {
  name: string;
  layer: string;
  message: string;
  description?: string;
  partition?: string[];
  key?: string[];
  read?: string[];
  system_read?: true;
  export?: true;
  export_models?: true;
}

/** A definition error: `error[E_CODE] at <path>: <message>`. */
export interface DescribeError {
  code: string;
  path: string;
  message: string;
  help?: string;
}

export function formatError(e: DescribeError): string {
  return `error[${e.code}] at ${e.path}: ${e.message}` + (e.help ? `\n  help: ${e.help}` : "");
}

/** Thrown by `plugin.describe()` when the definition has errors. */
export class DescribeErrors extends Error {
  constructor(readonly errors: DescribeError[]) {
    super(errors.map(formatError).join("\n"));
    this.name = "DescribeErrors";
  }
}

/** An endpoint ready to serve. */
export interface ResolvedEndpoint {
  key: string;
  name: string;
  input: MessageDef<any> | EntityDef<any>;
  output: MessageDef<any> | EntityDef<any>;
  timeoutMs: number;
  handler: EndpointOptions<IoType, IoType>["handler"];
  /** A request of the platform itself (the run-change hook): refused (404) when it carries a caller. */
  system?: boolean;
  /** Reads the body instead of the input message (throws an `RpcError`). */
  decode?: (body: unknown) => unknown;
}

/** The reserved method of the run-change hook: `/<id>.v1.<Service>/_RunChange`. */
export const RUN_CHANGE_METHOD = "_RunChange";
/** How long the run-change hook may take (ms). */
const RUN_CHANGE_TIMEOUT_MS = 30_000;
const NO_FIELDS = new MessageDef("RunChangeAnswer", {});

/** The reserved method of the settings check hook: `/<id>.v1.<Service>/_SettingsCheck`. */
export const SETTINGS_CHECK_METHOD = "_SettingsCheck";
/** How long the settings check hook may take (ms). */
const SETTINGS_CHECK_TIMEOUT_MS = 10_000;

/** A job ready to run. */
export interface ResolvedJob {
  def: JobDef<any>;
  name: string;
  payload: MessageDef<any>;
  handler: JobOptions<any>["handler"];
}

/** The runs a process starts at once by default, and the most it may announce. */
export const DEFAULT_JOB_SLOTS = 16;
export const MAX_JOB_SLOTS = 64;
const JOB_NAME = /^[a-z][a-z0-9_]{0,31}$/;
const JOB_MAX_RETRIES = 20;
const JOB_MAX_DELAY_MS = 3_600_000;

export interface Resolved {
  descriptor: Descriptor;
  errors: DescribeError[];
  endpoints: ResolvedEndpoint[];
  /** The jobs by name. */
  jobs: Map<string, ResolvedJob>;
  /** The job pool's size, announced at registration (0: no jobs). */
  jobSlots: number;
  roleName: (role: RoleLike) => string | undefined;
  /** The settings message the plugin declares (`plugin({ settings })`). */
  settings?: MessageDef<any>;
}

export const RESERVED_IDS = [
  "platform", "celobus", "core", "system", "sys", "workspace", "admin", "api", "app", "auth", "ui", "sdk", "cell",
  "public", "internal", "mcp",
];
export const CAPABILITIES = [
  "authz:bypass", "identity:manage", "push:send", "sql:write", "sql:unsafe", "blobs:use", "mcp:tools",
];
const WORKSPACE_ROLES = ["workspace.owner", "workspace.admin", "workspace.member", "workspace.guest"];
const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const TYPE_NAME = /^[A-Z][A-Za-z0-9]*$/;
const RESERVED_COLUMNS = ["id", "created_at", "updated_at", "deleted_at", "version"];
const MAX_ERRORS = 100;

/** Milliseconds of a timeout option, or `undefined` when it cannot be read. */
export function timeoutMs(timeout: unknown): number | undefined {
  if (timeout === undefined) return 30_000;
  if (typeof timeout === "number") return Number.isFinite(timeout) ? timeout : undefined;
  const m = typeof timeout === "string" ? /^(\d+(?:\.\d+)?)(ms|s|m)$/.exec(timeout) : null;
  if (!m) return undefined;
  return Number(m[1]) * (m[2] === "ms" ? 1 : m[2] === "s" ? 1000 : 60_000);
}

/** Whole seconds of an agent's timeout (`'90s'`, `'15m'`, `'2h'`), or `undefined` when it cannot be read. */
export function agentTimeoutSeconds(timeout: unknown): number | undefined {
  const m = typeof timeout === "string" ? /^(\d+)(s|m|h)$/.exec(timeout) : null;
  if (!m) return undefined;
  return Number(m[1]) * (m[2] === "s" ? 1 : m[2] === "m" ? 60 : 3600);
}

function isShape(v: unknown): v is Shape {
  return (
    typeof v === "object" &&
    v !== null &&
    !(v instanceof Field) &&
    !(v instanceof EntityDef) &&
    Object.values(v).every((f) => f instanceof Field)
  );
}

/** Builds the descriptor and the serving model; definition errors are collected, not thrown. */
export function resolve(options: PluginOptions, version: string | undefined): Resolved {
  const errors: DescribeError[] = [];
  const err = (code: string, path: string, message: string, help?: string) => {
    if (errors.length < MAX_ERRORS) errors.push(help ? { code, path, message, help } : { code, path, message });
  };
  const id = options.id;
  // options this SDK does not know: given to a helper (noted on what it returned) or within the plugin's own
  const optionsOf = (helper: Helper, value: unknown, path: string) => {
    for (const p of optionProblems(helper, value)) err("E_OPTION_UNKNOWN", `${path}.${p.key}`, p.message, p.help);
  };
  const noted = (def: unknown, path: string) => {
    if (typeof def !== "object" || def === null) return;
    for (const p of givenUnknown.get(def) ?? []) err("E_OPTION_UNKNOWN", `${path}.${p.key}`, p.message, p.help);
  };
  optionsOf("plugin", options, "plugin");

  // --- plugin -------------------------------------------------------------------------------------
  if (typeof id !== "string" || !/^[a-z][a-z0-9]{1,15}$/.test(id)) {
    err("E_PLUGIN_ID", "id", `${JSON.stringify(id)} is not a plugin id`, "2–16 characters: a lowercase letter, then lowercase letters and digits");
  } else if (RESERVED_IDS.includes(id) || id.startsWith("pg")) {
    err("E_PLUGIN_ID_RESERVED", "id", `\`${id}\` is reserved`, `reserved ids: ${RESERVED_IDS.join(", ")}, pg*`);
  }
  if (!version || !SEMVER.test(version)) {
    err("E_PLUGIN_VERSION", "version", version ? `${JSON.stringify(version)} is not semver` : "no version",
      "set `version` in plugin({...}) or in the plugin's package.json");
  }

  // --- roles --------------------------------------------------------------------------------------
  const roleNames = new Map<RoleDef, string>();
  const roles: Descriptor["roles"] = [];
  for (const [key, r] of Object.entries(options.roles ?? {})) {
    const path = `roles.${key}`;
    if (!(r instanceof RoleDef)) {
      err("E_SCHEMA", path, "not a role (use role({...}))");
      continue;
    }
    noted(r, path);
    const name = snake(key);
    if (!/^[a-z][a-z0-9_]{1,31}$/.test(name)) err("E_ROLE_NAME", path, `\`${name}\` is not a role name`, "2–32 characters: a-z, 0-9, _");
    if (roles.some((x) => x.name === name) || roleNames.has(r)) err("E_ROLE_DUP", path, `role \`${name}\` is declared twice`);
    const defaultFor = r.options.defaultFor ?? "none";
    if (!["none", "member", "admin"].includes(defaultFor)) err("E_SCHEMA", `${path}.defaultFor`, `\`${defaultFor}\` is not none, member or admin`);
    roleNames.set(r, name);
    roles.push({
      name,
      title: r.options.title ?? "",
      description: r.options.description ?? "",
      strict: r.options.strict ?? false,
      default_for: defaultFor,
    });
  }
  const roleName = (r: RoleLike): string | undefined => (r instanceof RoleDef ? roleNames.get(r) : typeof r === "string" ? r : undefined);
  const roleRefs = (list: readonly RoleLike[] | undefined, path: string): string[] =>
    (list ?? []).flatMap((r, i) => {
      const name = r instanceof RoleDef ? roleNames.get(r) : typeof r === "string" ? r : undefined;
      const known = name !== undefined && (WORKSPACE_ROLES.includes(name) || roles.some((x) => x.name === name));
      if (!known) {
        err("E_ENDPOINT_ROLE_UNKNOWN", `${path}[${i}]`, name ? `role \`${name}\` is not declared` : "the role is not in plugin({ roles })",
          "declare it in plugin({ roles }) or use workspace.owner|admin|member|guest");
        return [];
      }
      return [name!];
    });

  // --- scopes, capabilities, permissions, agents, dependencies ------------------------------------
  const scopes: Descriptor["scopes"] = [];
  for (const [key, s] of Object.entries(options.scopes ?? {})) {
    const name = snake(key);
    const path = `scopes.${key}`;
    optionsOf("scope", s, path);
    if (!/^[a-z][a-z0-9_]{0,31}$/.test(name)) err("E_SCOPE_NAME", path, `\`${name}\` is not a scope name`);
    if (scopes.some((x) => x.name === name)) err("E_SCOPE_DUP", path, `scope \`${name}\` is declared twice`);
    scopes.push({ name, title: s?.title ?? "", description: s?.description ?? "", sensitive: s?.sensitive ?? false });
  }
  const scopeNames = new Set(["read", "write", ...scopes.map((s) => s.name)]);

  const capabilities: Descriptor["capabilities"] = [];
  for (const [name, c] of Object.entries(options.capabilities ?? {})) {
    const path = `capabilities.${JSON.stringify(name)}`;
    const reason = typeof c === "string" ? c : c?.reason ?? "";
    if (typeof c !== "string") {
      optionsOf("capability", c, path);
      (c?.on ?? []).forEach((b, i) => optionsOf("bypass", b, `${path}.on[${i}]`));
    }
    if (!CAPABILITIES.includes(name)) err("E_CAPABILITY_UNKNOWN", path, `\`${name}\` is not a capability`, `capabilities: ${CAPABILITIES.join(", ")}`);
    if (!reason.trim()) err("E_CAPABILITY_REASON", path, "a capability needs a reason");
    capabilities.push({ name, reason });
  }

  const permissions = [...(options.permissions ?? [])];

  const agents: Descriptor["agents"] = Object.entries(options.agents ?? {}).map(([key, a]) => {
    optionsOf("agent", a, `agents.${key}`);
    const seconds = a?.timeout === undefined ? undefined : agentTimeoutSeconds(a.timeout);
    if (a?.timeout !== undefined && seconds === undefined)
      err("E_AGENT_TIMEOUT_RANGE", `agents.${key}.timeout`, `${JSON.stringify(a.timeout)} is not a time`, "whole seconds as '90s', '15m' or '2h'");
    return {
      name: snake(key),
      roles: roleRefs(a?.roles, `agents.${key}.roles`),
      // each left out when not declared, so a plugin without them describes itself as before
      ...(a?.workspace?.length ? { workspace: [...a.workspace] } : {}),
      // an agent of this plugin is named as its key is (`orderHelper` is `order_helper`); `<dependency>.<agent>` as given
      ...(a?.delegates?.length ? { delegates: a.delegates.map((d) => (typeof d === "string" && !d.includes(".") ? snake(d) : d)) } : {}),
      // a connection of this plugin is named as its key is; `<dependency>.<name>` as given
      ...(a?.connection ? { connection: a.connection.includes(".") ? a.connection : snake(a.connection) } : {}),
      ...(a?.sshKey ? { ssh_key: true as const } : {}),
      ...(a?.admin ? { admin: true as const } : {}),
      ...(a?.title ? { title: a.title } : {}),
      ...(a?.description ? { description: a.description } : {}),
      ...(a?.icon ? { icon: a.icon } : {}),
      ...(a?.engine ? { engine: a.engine } : {}),
      ...(a?.model ? { model: a.model } : {}),
      ...(a?.session ? { session: true as const } : {}),
      ...(seconds !== undefined ? { timeout_s: seconds } : {}),
      ...(a?.skills?.length ? { skills: [...a.skills] } : {}),
      ...(a?.topic instanceof TopicDef || a?.topic instanceof TopicDecl ? { topic: topicDecl(a.topic).topic.name } : {}),
    };
  });

  for (const [key, c] of Object.entries(options.connections ?? {})) optionsOf("connection", c, `connections.${key}`);
  const connections: NonNullable<Descriptor["connections"]> = Object.entries(options.connections ?? {}).map(([key, c]) => ({
    name: snake(key),
    kind: c?.kind ?? "",
    ...(c?.title ? { title: c.title } : {}),
    ...(c?.description ? { description: c.description } : {}),
  }));

  const dependencies: Descriptor["dependencies"] = [];
  (options.dependencies ?? []).forEach((d, i) => {
    const dep: Descriptor["dependencies"][number] =
      typeof d === "string" ? { plugin: d, calls: [] } : { plugin: d.plugin, calls: [...(d.calls ?? [])] };
    // the reads as the generated client gives them, written only when there are any
    if (typeof d !== "string" && d.reads?.length) {
      dep.reads = d.reads.map((r) => ({ entity: r.entity, fields: (r.fields ?? []).map((f) => ({ name: f.name, type: f.type })) }));
    }
    const path = `dependencies[${i}]`;
    if (typeof d !== "string") {
      optionsOf("dependency", d, path);
      (d.reads ?? []).forEach((r, j) => optionsOf("dependencyRead", r, `${path}.reads[${j}]`));
    }
    if (dep.plugin === id) err("E_DEP_SELF", path, "a plugin cannot depend on itself");
    if (dependencies.some((x) => x.plugin === dep.plugin)) err("E_DEP_DUP", path, `\`${dep.plugin}\` is listed twice`);
    dependencies.push(dep);
  });

  // --- entities: names first, so refs can name them -----------------------------------------------
  const entityNames = new Map<EntityDef<any>, string>();
  const entityList: [string, EntityDef<any>][] = [];
  for (const [key, e] of Object.entries(options.entities ?? {})) {
    const path = `entities.${key}`;
    if (!(e instanceof EntityDef)) {
      err("E_SCHEMA", path, "not an entity (use entity({...}))");
      continue;
    }
    noted(e, path);
    if (e.options.history !== undefined) optionsOf("history", e.options.history, `${path}.history`);
    (e.options.indexes ?? []).forEach((ix: unknown, i) => {
      if (!Array.isArray(ix)) optionsOf("index", ix, `${path}.indexes[${i}]`);
    });
    if (foreignPlugin(e)) {
      err("E_SCHEMA", path, `\`${foreignPlugin(e)}.${e.options.name ?? ""}\` is another plugin's entity`,
        "read it with ctx.query() and list its reads in the dependency");
      continue;
    }
    const name = e.options.name ?? snake(key);
    if (!/^[a-z][a-z0-9_]{0,62}$/.test(name)) err("E_ENT_NAME", path, `\`${name}\` is not an entity name`);
    if (name.endsWith("_with_deleted")) err("E_ENT_NAME_RESERVED", path, "an entity name may not end in `_with_deleted`");
    if ([...entityNames.values()].includes(name) || entityNames.has(e)) err("E_ENT_DUP", path, `entity \`${name}\` is declared twice`);
    if (!["uuid", "i64", "string"].includes(e.key)) err("E_ENT_KEY", `${path}.key`, `\`${e.key}\` is not uuid, i64 or string`);
    for (const col of Object.keys(e.columns)) {
      if (RESERVED_COLUMNS.includes(snake(col))) err("E_COL_RESERVED", `${path}.columns.${col}`, `\`${snake(col)}\` is a system column`,
        "`id` is implied by `key`; timestamps and `version` are the platform's");
    }
    entityNames.set(e, name);
    describedNames.set(e, name);
    entityList.push([key, e]);
  }

  // --- views: names first, so queries and endpoints can name them ---------------------------------
  const viewList: [string, string, ViewDef<any>][] = []; // [key, name, view]
  for (const [key, v] of Object.entries(options.views ?? {})) {
    const path = `views.${key}`;
    if (!(v instanceof ViewDef)) {
      err("E_SCHEMA", path, "not a view (use view({...}))");
      continue;
    }
    noted(v, path);
    const name = v.options.name ?? snake(key);
    if (!/^[a-z][a-z0-9_]{0,62}$/.test(name)) err("E_VIEW_NAME", path, `\`${name}\` is not a view name`, "snake_case, at most 63 characters");
    if (viewList.some(([, n, x]) => n === name || x === v)) {
      err("E_VIEW_DUP", path, `view \`${name}\` is declared twice`);
    }
    if ([...entityNames.values()].includes(name)) err("E_VIEW_CLASH", path, `\`${name}\` is the name of an entity of the plugin`);
    viewNames.set(v, name);
    viewList.push([key, name, v]);
  }

  // --- messages and enums -------------------------------------------------------------------------
  const messages: Descriptor["messages"] = [];
  const enums: Descriptor["enums"] = [];
  const typeOwners = new Map<string, object>(); // message and enum names share the protobuf namespace
  const visited = new Map<object, Descriptor["messages"][number]>();

  const claimType = (name: string, owner: object, path: string, code: "E_MSG_DUP" | "E_ENUM_DUP"): boolean => {
    if (!TYPE_NAME.test(name)) err("E_MSG_NAME", path, `\`${name}\` is not a PascalCase type name`);
    const existing = typeOwners.get(name);
    if (existing && existing !== owner) {
      err(code, path, `two different declarations are named \`${name}\``);
      return false;
    }
    typeOwners.set(name, owner);
    return true;
  };

  const visitEnum = (def: EnumDef<string>, path: string): string => {
    if (typeOwners.get(def.name) === def) return def.name;
    if (!claimType(def.name, def, path, "E_ENUM_DUP")) return def.name;
    const values = [...def.values];
    if (values.length === 0) err("E_SCHEMA", `enums.${def.name}`, "an enum needs at least one value");
    if (new Set(values).size !== values.length) err("E_SCHEMA", `enums.${def.name}`, "an enum value is listed twice");
    enums.push({ name: def.name, values });
    return def.name;
  };

  const refType = (field: AnyField, path: string): string => {
    const target = field.spec.ref!;
    if (target instanceof PlatformEntity) return `ref:${target.ref}`;
    if (typeof target === "string") {
      if (!/^[a-z][a-z0-9]*\.[a-z][a-z0-9_]*$/.test(target)) err("E_FIELD_TYPE", path, `\`${target}\` is not <plugin>.<entity>`);
      return `ref:${target}`;
    }
    let entity: unknown = target;
    if (!(target instanceof EntityDef)) {
      try {
        entity = target();
      } catch (e) {
        err("E_FIELD_TYPE", path, `the ref's target could not be read: ${(e as Error).message}`);
        return "ref:";
      }
    }
    if (entity instanceof EntityDef && foreignPlugin(entity)) return `ref:${foreignPlugin(entity)}.${entity.options.name ?? ""}`;
    const name = entity instanceof EntityDef ? entityNames.get(entity) : undefined;
    if (name === undefined) {
      err("E_FIELD_TYPE", path, "the ref's entity is not in plugin({ entities })");
      return "ref:";
    }
    return `ref:${id}.${name}`;
  };

  const fieldType = (field: AnyField, path: string): string => {
    switch (field.spec.kind) {
      case "scalar":
        return field.spec.scalar!;
      case "enum":
        return `enum:${visitEnum(field.spec.enumDef!, path)}`;
      case "message":
        return `message:${visitMessage(field.spec.message!, path)}`;
      case "ref":
        return refType(field, path);
      case "array":
        err("E_FIELD_TYPE", path, "an array of arrays is not a field type", "wrap the inner array in a message");
        return "";
    }
  };

  const describeFields = (source: MessageDef<any> | EntityDef<any> | ViewDef<any> | LakeTableDef<any, any>, path: string): DescriptorField[] => {
    const seen = new Set<string>();
    return messageFields(source).map((f, i) => {
      const fpath = `${path}.${f.key}`;
      // the options of a `t.ref`, also of one inside `t.array`
      const inner = f.field.spec.kind === "array" ? f.field.spec.inner : undefined;
      for (const p of [...(f.field.column.unknownOptions ?? []), ...(inner?.column.unknownOptions ?? [])]) {
        err("E_OPTION_UNKNOWN", `${fpath}.${p.key}`, p.message, p.help);
      }
      if (!/^[a-z][a-zA-Z0-9]*$/.test(f.key) || jsonName(f.name) !== f.key) {
        err("E_FIELD_NAME", fpath, `\`${f.key}\` is not a lowerCamelCase field name`);
      }
      if (seen.has(f.name)) err("E_FIELD_DUP", fpath, `field \`${f.name}\` is declared twice`);
      seen.add(f.name);
      let field = f.field;
      let repeated = false;
      if (field.spec.kind === "array") {
        repeated = true;
        field = field.spec.inner!;
        if (!(field instanceof Field)) err("E_FIELD_TYPE", fpath, "t.array() needs a t.* field");
        else if (field.isOptional) err("E_FIELD_TYPE", fpath, "an array's elements cannot be optional");
      }
      const out: DescriptorField = {
        name: f.name,
        number: i + 1,
        type: field instanceof Field ? fieldType(field, fpath) : "",
        repeated,
        optional: f.field.isOptional,
        description: f.field.description,
      };
      const file = field instanceof Field ? field.spec.file : undefined;
      if (file?.badSize !== undefined) {
        err("E_FILE_OPTION", `${fpath}.maxSize`, `${JSON.stringify(file.badSize)} is not a size`,
          "bytes (1048576) or a number and a unit ('1 GiB', '500 MB'; KiB, MiB, GiB, TiB, KB, MB, GB, TB)");
      }
      if (file?.maxSize !== undefined) out.max_size = file.maxSize;
      if (file?.accept !== undefined) out.accept = [...file.accept];
      // a setting's options (`.label()`, `.default()`, `.min()`, ..), each written only when declared
      Object.assign(out, settingsKeys(f.field, out.type, repeated, fpath, err));
      if (repeated && field instanceof Field && Object.keys(field.settings).length > 0) {
        err("E_SETTINGS_VALIDATOR", fpath, "the options of a list setting go on the list: t.array(t.text()).maxItems(3), not on its item");
      }
      return out;
    });
  };

  // Registers a message (an entity's message, a view's row) at its first use and returns its name.
  function visitMessage(source: MessageDef<any> | EntityDef<any> | ViewDef<any> | LakeTableDef<any, any>, path: string): string {
    if (source instanceof LakeTableDef) return visitLakeRow(source, path);
    const isEntity = source instanceof EntityDef;
    const isView = source instanceof ViewDef;
    const name = isEntity ? pascal(entityNames.get(source) ?? "") : isView ? pascal(viewNames.get(source) ?? "") : source.name;
    if (visited.has(source)) return name;
    if (!claimType(name, source, path, "E_MSG_DUP")) return name;
    const desc: Descriptor["messages"][number] = {
      name,
      kind: source instanceof MessageDef && source.kind === "settings" ? "settings" : "message",
      description: isEntity || isView ? source.options.description ?? "" : source.messageDescription,
      fields: [],
    };
    visited.set(source, desc);
    messages.push(desc);
    if (source instanceof MessageDef) noted(source, `messages.${name}`);
    desc.fields = describeFields(source, isEntity ? `entities.${entityNames.get(source)}.columns` : isView ? `views.${viewNames.get(source)}.columns` : `messages.${name}`);
    return name;
  }

  // A lake table's row: its message, named apart from the table (`TaskLoadRow` of `task_load`).
  function visitLakeRow(source: LakeTableDef<any, any>, path: string): string {
    const name = source.messageName();
    if (visited.has(source)) return name;
    if (!claimType(name, source, path, "E_MSG_DUP")) return name;
    const desc: Descriptor["messages"][number] = { name, kind: "message", description: source.options.description ?? "", fields: [] };
    visited.set(source, desc);
    messages.push(desc);
    desc.fields = describeFields(source, `${path}.columns`);
    return name;
  }

  // --- endpoints ----------------------------------------------------------------------------------
  const endpoints: Descriptor["endpoints"] = [];
  const endpointNames = new Map<EndpointDef<any, any>, string>();
  const resolvedEndpoints: ResolvedEndpoint[] = [];
  const io = (value: unknown, generated: string, path: string): MessageDef<any> | EntityDef<any> | undefined => {
    if (value instanceof MessageDef) return value;
    if (value instanceof EntityDef) {
      if (entityNames.has(value)) return value;
      err("E_ENDPOINT_MESSAGE", path, "the entity is not in plugin({ entities })");
      return undefined;
    }
    if (isShape(value)) return new MessageDef(generated, value);
    err("E_ENDPOINT_MESSAGE", path, "not a message, an entity or an object of t.* fields");
    return undefined;
  };
  for (const [key, ep] of Object.entries(options.endpoints ?? {})) {
    const path = `endpoints.${key}`;
    if (!(ep instanceof EndpointDef)) {
      err("E_SCHEMA", path, "not an endpoint (use endpoint({...}))");
      continue;
    }
    noted(ep.root, path);
    const o = ep.options;
    const name = pascal(key);
    if (!TYPE_NAME.test(name)) err("E_ENDPOINT_NAME", path, `\`${name}\` is not an endpoint name`);
    if (endpoints.some((x) => x.name === name)) err("E_ENDPOINT_DUP", path, `endpoint \`${name}\` is declared twice`);
    const input = io(o.input, `${name}Request`, `${path}.input`);
    const output = io(o.output, `${name}Reply`, `${path}.output`);
    const inputName = input ? visitMessage(input, `${path}.input`) : "";
    const outputName = output ? visitMessage(output, `${path}.output`) : "";
    const scopeList = [...(o.scopes ?? (o.readOnly ? ["read"] : ["write"]))];
    scopeList.forEach((s, i) => {
      if (!scopeNames.has(s)) err("E_ENDPOINT_SCOPE_UNKNOWN", `${path}.scopes[${i}]`, `scope \`${s}\` is not declared`);
    });
    const auth = o.auth ?? "user";
    if (!["user", "service", "any", "public"].includes(auth)) err("E_SCHEMA", `${path}.auth`, `\`${auth}\` is not user, service, any or public`);
    const idempotency = o.idempotency ?? "off";
    if (!["off", "optional", "required"].includes(idempotency)) err("E_SCHEMA", `${path}.idempotency`, `\`${idempotency}\` is not off, optional or required`);
    const ms = timeoutMs(o.timeout);
    if (ms === undefined || !Number.isInteger(ms) || ms < 1 || ms > 600_000) {
      err("E_ENDPOINT_TIMEOUT_RANGE", `${path}.timeout`, `${JSON.stringify(o.timeout)} is not 1 ms … 600 s`, "milliseconds, or '500ms', '10s', '2m'");
    }
    if (typeof o.handler !== "function") err("E_SCHEMA", `${path}.handler`, "an endpoint needs a handler");
    const decl: Descriptor["endpoints"][number] = {
      name,
      input: inputName,
      output: outputName,
      description: o.description ?? "",
      auth,
      roles: roleRefs(o.roles, `${path}.roles`),
      scopes: scopeList,
      idempotency,
      timeout_ms: ms ?? 0,
    };
    if (o.ui) {
      if (o.readOnly) decl.read_only = true;
      decl.ui = true;
    }
    if (o.invalidates && o.invalidates.length > 0) decl.invalidates = o.invalidates.map(invalidationJson);
    endpoints.push(decl);
    if (!endpointNames.has(ep.root)) endpointNames.set(ep.root, name);
    if (input && !describedEndpoints.has(ep.root)) describedEndpoints.set(ep.root, { name, input });
    if (input && output) {
      resolvedEndpoints.push({ key, name, input, output, timeoutMs: ms ?? 30_000, handler: o.handler as ResolvedEndpoint["handler"] });
    }
  }

  // --- the navigations `include` reads: each entity's forward ones, and the inverse ones declared onto it --------
  const entityByName = new Map(entityList.map(([, e]) => [entityNames.get(e)!, e]));
  for (const [, e] of entityList) navigations.set(e, new Map());
  const targetOf = (field: AnyField): EntityDef<any> | undefined => {
    const t = field.spec.ref;
    if (t === undefined || t instanceof PlatformEntity || typeof t === "string") return undefined;
    try {
      const resolved = t instanceof EntityDef ? t : t();
      return resolved instanceof EntityDef ? resolved : undefined;
    } catch {
      return undefined;
    }
  };
  const recordNavigations = (e: EntityDef<any>, fields: DescriptorField[]): void => {
    for (const f of fields) {
      if (f.name === "id" || f.repeated || !f.type.startsWith("ref:")) continue;
      const col = Object.entries(e.columns).find(([k]) => snake(k) === f.name)?.[1] as AnyField | undefined;
      if (!col) continue;
      const target = targetOf(col);
      const own = navigations.get(e)!;
      const nav = f.nav ?? (f.name.endsWith("_id") && f.name.length > 3 ? f.name.slice(0, -3) : f.name);
      own.set(f.name, { kind: "forward", target });
      own.set(nav, { kind: "forward", target });
      if (f.inverse && f.type.startsWith(`ref:${id}.`)) {
        const onto = entityByName.get(f.type.slice(`ref:${id}.`.length));
        if (onto) navigations.get(onto)?.set(f.inverse, { kind: "inverse", source: e });
      }
    }
  };

  // --- entities' messages, then the extra messages and enums --------------------------------------
  const entities: Descriptor["entities"] = entityList.map(([key, e]) => {
    const name = entityNames.get(e)!;
    const path = `entities.${key}`;
    const message = visitMessage(e, path);
    const columns = Object.keys(e.columns).map(snake);
    let label: string;
    if (e.options.label !== undefined) {
      label = snake(e.options.label);
      if (label !== "id" && !columns.includes(label)) err("E_ENT_LABEL", `${path}.label`, `\`${label}\` is not a column`);
    } else {
      label = ["name", "title", "number"].find((c) => columns.includes(c)) ?? "id";
    }
    // the entity's fields are its message's, with the column modifiers (the message's copy has none)
    let searchable = 0;
    const fields = (visited.get(e)?.fields ?? []).map((f) => {
      const entry = Object.entries(e.columns).find(([k]) => snake(k) === f.name);
      const col = entry?.[1] as AnyField | undefined;
      const cpath = `${path}.columns.${entry?.[0] ?? f.name}`;
      const out: DescriptorField = { ...f };
      if (col?.column.unique) out.unique = true;
      if (col?.column.index) out.index = true;
      if (col?.column.encrypted) out.encrypted = true;
      if (col?.column.searchable !== undefined) {
        out.searchable = col.column.searchable;
        searchable += 1;
        if (f.type !== "string" || f.repeated) {
          err("E_SEARCH_TYPE", cpath, `${f.name} is not a text column: only a single string column is searchable`);
        }
        if (!SEARCH_LANGUAGES.includes(out.searchable)) {
          err("E_SEARCH_LANGUAGE", `${cpath}.searchable`, `${JSON.stringify(out.searchable)} is not a text search language`,
            `one of ${SEARCH_LANGUAGES.join(", ")}`);
        }
        if (searchable === MAX_SEARCHABLE + 1) {
          err("E_SEARCH_LIMIT", cpath, `${name} has more than ${MAX_SEARCHABLE} searchable columns`);
        }
      }
      for (const what of ["nav", "inverse"] as const) {
        const v = col?.column[what];
        if (v === undefined) continue;
        out[what] = v;
        if (!f.type.startsWith("ref:") || f.repeated) {
          err("E_REF_NAME", `${cpath}.${what}`, `${f.name} is not a ref column: only a single ref has a ${what}`);
        } else if (!NAV_NAME.test(v)) {
          err("E_REF_NAME", `${cpath}.${what}`, `${JSON.stringify(v)} is not a navigation name`, "snake_case, at most 63 characters");
        }
      }
      const onDelete = col?.column.onDelete;
      if (onDelete !== undefined) {
        const v = snake(onDelete);
        out.on_delete = v;
        const dpath = `${cpath}.onDelete`;
        const target = f.type.startsWith("ref:") ? f.type.slice(4).split(".")[0] : undefined;
        if (target === undefined || f.repeated) {
          err("E_REF_ON_DELETE", dpath, `${f.name} is not a ref column: only a single ref has an onDelete`);
        } else if (target === id && !ON_DELETE_OWN.includes(v)) {
          err("E_REF_ON_DELETE", dpath, `${JSON.stringify(onDelete)} is not an onDelete of a ref within the plugin`,
            "one of restrict, cascade, setNull, noAction");
        } else if (target !== id && (!ON_DELETE_CROSS.includes(v) || (v === "set_null" && target !== "platform"))) {
          err("E_REF_CROSS_ON_DELETE", dpath, `${JSON.stringify(onDelete)} is not an onDelete of a ref to ${f.type.slice(4)}`,
            target === "platform" ? "keep or setNull" : "a ref to another plugin keeps the id (keep)");
        } else if (v === "set_null" && !f.optional) {
          err("E_REF_ON_DELETE", dpath, `${f.name} is required: setNull clears an optional ref`);
        } else if (v === "set_null" && e.options.appendOnly) {
          err("E_ENT_APPEND_ONLY_CASCADE", dpath, `${name} is append-only: setNull would change its rows`);
        }
      }
      return out;
    });
    recordNavigations(e, fields);
    const decl: Descriptor["entities"][number] = {
      name,
      message,
      key: e.key,
      label,
      soft_delete: e.options.softDelete ?? false,
      version: e.options.version ?? false,
      append_only: e.options.appendOnly ?? false,
      fields,
    };
    const indexes = (e.options.indexes ?? []).map((ix: unknown) => {
      const { columns, unique } = Array.isArray(ix)
        ? { columns: ix as string[], unique: false }
        : (ix as { columns: string[]; unique?: boolean });
      const out: NonNullable<Descriptor["entities"][number]["indexes"]>[number] = {
        columns: columns.map((c) => {
          const [col, dir] = c.trim().split(/\s+/);
          const column: { name: string; desc?: true } = { name: snake(col ?? "") };
          if (dir === "desc") column.desc = true;
          return column;
        }),
      };
      if (unique) out.unique = true;
      return out;
    });
    if (indexes.length > 0) decl.indexes = indexes;
    // stored in OLAP: its key's fields by their descriptor names
    const storage = (e.options as { storage?: unknown }).storage;
    if (storage !== undefined) {
      if (storage === "olap") decl.storage = "olap";
      else err("E_OLAP_STORAGE", `${path}.storage`, `${JSON.stringify(storage)} is not a storage`, "storage: 'olap' keeps the rows in the analytic copy only");
    }
    const olapKey = e.options.olapKey ?? [];
    if (olapKey.length > 0) {
      if (storage !== "olap") err("E_OLAP_KEY", `${path}.olapKey`, "olapKey keys an entity stored in OLAP: declare storage: 'olap'");
      decl.olap_key = olapKey.map((k) => {
        if (!(k in e.columns)) err("E_OLAP_KEY", `${path}.olapKey`, `\`${k}\` is not a column of ${name}`);
        return snake(k);
      });
    }
    if (storage === "olap" && e.options.load === undefined) {
      err("E_OLAP_LOAD", `${path}.load`, `${name} is stored in OLAP: its load fills it`, "load: (l) => l.from(lakeTable)..columns({..})");
    }
    if (storage !== "olap" && e.options.load !== undefined) {
      err("E_LOAD_ENTITY", `${path}.load`, `${name} is not stored in OLAP: only such an entity has a load`, "storage: 'olap'");
    }
    if (e.options.access) {
      decl.access = describeAccess(e.options.access, {
        role: roleName,
        entity: (x) => entityNames.get(x) ?? (foreignPlugin(x) ? `${foreignPlugin(x)}.${x.options.name ?? ""}` : undefined),
        error: (message, code) => err(code ?? "E_ACCESS_ROLE_UNKNOWN", `${path}.access`, message),
      });
    }
    return decl;
  });
  // the scope of `authz:bypass`: entities by their names
  for (const [name, c] of Object.entries(options.capabilities ?? {})) {
    const on = typeof c === "string" ? undefined : c?.on;
    if (!on?.length) continue;
    const path = `capabilities.${JSON.stringify(name)}.on`;
    if (name !== "authz:bypass") err("E_CAPABILITY_SCOPE", path, `${name} has no scope`);
    const decl = capabilities.find((x) => x.name === name);
    if (decl) {
      decl.on = on.map((s, i) => {
        const entity = entityNames.get(s.entity);
        if (entity === undefined) err("E_CAPABILITY_SCOPE", `${path}[${i}]`, "the entity is not in plugin({ entities })");
        return { entity: entity ?? "", ops: [...s.ops] };
      });
    }
  }
  (options.messages ?? []).forEach((m, i) => {
    if (m instanceof MessageDef) visitMessage(m, `messages[${i}]`);
    else err("E_SCHEMA", `messages[${i}]`, "not a message (use t.message(...))");
  });
  // --- jobs: their payloads follow the extra messages ----------------------------------------------
  const jobs: NonNullable<Descriptor["jobs"]> = [];
  const resolvedJobs = new Map<string, ResolvedJob>();
  const jobPayloads = new Map<string, string>(); // payload message → the job that has it
  const scheduled: [string, string][] = []; // [path, payload] of the jobs with a schedule
  for (const [key, j] of Object.entries(options.jobs ?? {})) {
    const path = `jobs.${key}`;
    if (!(j instanceof JobDef)) {
      err("E_SCHEMA", path, "not a job (use job({...}))");
      continue;
    }
    noted(j, path);
    if (j.options.retry !== undefined) optionsOf("retry", j.options.retry, `${path}.retry`);
    const o = j.options;
    const name = o.name ?? snake(key);
    if (typeof name !== "string" || !JOB_NAME.test(name)) {
      err("E_JOB_NAME", `${path}.name`, `${JSON.stringify(name)} is not a job name`, "snake_case, at most 32 characters: remind_unpaid");
    }
    if (jobs.some((x) => x.name === name)) err("E_JOB_DUP", path, `job \`${name}\` is declared twice`);
    let payload: MessageDef<any> | undefined;
    if (o.payload instanceof MessageDef) payload = o.payload;
    else if (isShape(o.payload)) payload = new MessageDef(pascal(String(name)), o.payload);
    else err("E_JOB_PAYLOAD", `${path}.payload`, "not a message or an object of t.* fields");
    const payloadName = payload ? visitMessage(payload, `${path}.payload`) : "";
    const desc = payload ? visited.get(payload) : undefined;
    if (desc) desc.kind = "job";
    if (payload) {
      const other = jobPayloads.get(payloadName);
      if (other !== undefined) {
        err("E_JOB_PAYLOAD_DUP", `${path}.payload`, `message \`${payloadName}\` is the payload of job \`${other}\``,
          "a job is enqueued by its declaration: give each job a message of its own");
      } else jobPayloads.set(payloadName, String(name));
    }
    const decl: NonNullable<Descriptor["jobs"]>[number] = { name, payload: payloadName };
    if (o.description) decl.description = o.description;
    if (o.cron !== undefined && o.cron !== "") {
      const problem = typeof o.cron === "string" ? cronError(o.cron) : "not a string";
      if (problem !== undefined) {
        err("E_JOB_CRON", `${path}.cron`, `${JSON.stringify(o.cron)}: ${problem}`,
          "five fields in UTC, minute hour day-of-month month day-of-week: 0 9 * * 1-5");
      }
      decl.cron = String(o.cron);
      if (payload) scheduled.push([`${path}.payload`, payloadName]);
    }
    if (o.as !== undefined) {
      if (o.as === "system") decl.as_system = true;
      else err("E_SCHEMA", `${path}.as`, `${JSON.stringify(o.as)} is not 'system'`);
    }
    if (o.retry !== undefined) {
      const r = o.retry;
      const retries = r.retries ?? 3;
      const delay = r.delay === undefined ? 30_000 : durationMs(r.delay);
      const backoff = r.backoff ?? "exponential";
      if (!Number.isInteger(retries) || retries < 0 || retries > JOB_MAX_RETRIES) {
        err("E_JOB_RETRY", `${path}.retry.retries`, `${JSON.stringify(r.retries)} retries; a job has 0 to ${JOB_MAX_RETRIES}`);
      }
      if (delay === undefined || !Number.isInteger(delay) || delay < 0 || delay > JOB_MAX_DELAY_MS) {
        err("E_JOB_RETRY", `${path}.retry.delay`, `a retry delay of ${JSON.stringify(r.delay)}; whole milliseconds, at most one hour`,
          "milliseconds, or '500ms', '30s', '5m', '1h'");
      }
      if (backoff !== "fixed" && backoff !== "exponential") {
        err("E_JOB_RETRY", `${path}.retry.backoff`, `${JSON.stringify(backoff)} is not fixed or exponential`);
      }
      decl.retry = { retries, delay_ms: delay ?? 0, backoff };
    }
    if (o.timeout !== undefined) {
      const ms = durationMs(o.timeout);
      if (ms === undefined || !Number.isInteger(ms) || ms < 1000 || ms > 3_600_000) {
        err("E_JOB_TIMEOUT", `${path}.timeout`, `a timeout of ${JSON.stringify(o.timeout)}; a run's timeout is 1 s to 1 h`,
          "milliseconds, or '10s', '2m', '1h'");
      }
      decl.timeout_ms = ms ?? 0;
    }
    if (o.concurrency !== undefined) {
      if (!Number.isInteger(o.concurrency) || o.concurrency < 1 || o.concurrency > 64) {
        err("E_JOB_CONCURRENCY", `${path}.concurrency`, `a concurrency of ${o.concurrency}; a job runs 1 to 64 runs at once`);
      }
      decl.concurrency = o.concurrency;
    }
    if (typeof o.handler !== "function") err("E_SCHEMA", `${path}.handler`, "a job needs a handler");
    jobs.push(decl);
    if (payload && !resolvedJobs.has(name)) resolvedJobs.set(name, { def: j, name, payload, handler: o.handler });
  }
  const jobSlots = options.jobSlots ?? DEFAULT_JOB_SLOTS;
  if (!Number.isInteger(jobSlots) || jobSlots < 1 || jobSlots > MAX_JOB_SLOTS) {
    err("E_JOB_SLOTS", "jobSlots", `${jobSlots} job slots; a process runs 1 to ${MAX_JOB_SLOTS} runs at once`);
  }

  // --- topics: their key messages follow the jobs' payloads -----------------------------------------
  const endpointName = (def: unknown, path: string): string => {
    const name = def instanceof EndpointDef ? endpointNames.get(def.root) : undefined;
    if (name === undefined) err("E_UI_ENDPOINT_UNKNOWN", path, "the endpoint is not in plugin({ endpoints })");
    return name ?? "";
  };
  const topicList: [string, TopicLike][] = Array.isArray(options.topics)
    ? (options.topics as readonly TopicLike[]).map((t, i) => [`topics[${i}]`, t])
    : Object.entries(options.topics ?? {}).map(([k, t]) => [`topics.${k}`, t]);
  const topics: NonNullable<Descriptor["topics"]> = [];
  for (const [path, t] of topicList) {
    if (!(t instanceof TopicDef || t instanceof TopicDecl) || !(topicDecl(t).topic.keyMessage instanceof MessageDef)) {
      err("E_SCHEMA", path, "not a topic (use topic(name, t.message(..)))");
      continue;
    }
    const d = topicDecl(t);
    const decl: NonNullable<Descriptor["topics"]>[number] = {
      name: d.topic.name,
      key: visitMessage(d.topic.keyMessage, `${path}.key`),
      description: d.description,
    };
    const roleList = roleRefs(d.roleList, `${path}.roles`);
    if (roleList.length > 0) decl.roles = roleList;
    d.scopeList.forEach((s, i) => {
      if (!scopeNames.has(s)) err("E_ENDPOINT_SCOPE_UNKNOWN", `${path}.scopes[${i}]`, `scope \`${s}\` is not declared`);
    });
    if (d.scopeList.length > 0) decl.scopes = [...d.scopeList];
    if (d.authorizer) decl.authorize = endpointName(d.authorizer, `${path}.authorize`);
    if (d.isManual) decl.manual = true;
    topics.push(decl);
  }
  // the topic an agent's runs invalidate is one of the plugin's
  for (const [key, a] of Object.entries(options.agents ?? {})) {
    if (a?.topic === undefined) continue;
    const named = a.topic instanceof TopicDef || a.topic instanceof TopicDecl ? topicDecl(a.topic).topic.name : undefined;
    if (named === undefined) err("E_SCHEMA", `agents.${key}.topic`, "not a topic (use topic(name, t.message(..)))");
    else if (!topics.some((t) => t.name === named))
      err("E_AGENT_TOPIC_UNKNOWN", `agents.${key}.topic`, `topic \`${named}\` is not declared`, "list it in plugin({ topics })");
  }

  // --- views: their rows follow the topics' keys; their queries are built now that every name is known ----------
  const views: DescribedView[] = [];
  for (const [key, name, v] of viewList) {
    const message = visitMessage(v, `views.${key}`);
    const d = describeView(v, name, message);
    if (typeof d === "string") err("E_VIEW_DEFINE", `views.${key}`, d);
    else views.push(d);
  }

  // --- the lake: layers, the tables (their rows follow the views'), other plugins' tables read -----------------
  const layers: NonNullable<Descriptor["layers"]> = [];
  const layerNames = new Map<LayerDef, string>();
  for (const [key, l] of Object.entries(options.layers ?? {})) {
    const path = `layers.${key}`;
    if (!(l instanceof LayerDef)) {
      err("E_SCHEMA", path, "not a layer (use layer({...}))");
      continue;
    }
    noted(l, path);
    const name = snake(key);
    if (!LAKE_NAME.test(name)) err("E_LAKE_LAYER_NAME", path, `\`${name}\` is not a layer name`, "snake_case, at most 32 characters");
    if (layers.some((x) => x.name === name)) err("E_LAKE_LAYER_DUP", path, `layer \`${name}\` is declared twice`);
    layerNames.set(l, name);
    layers.push(l.options.title ? { name, title: l.options.title } : { name });
  }
  const lakeTables: DescribedLakeTable[] = [];
  const viewNameList = viewList.map(([, n]) => n);
  for (const [key, tbl] of Object.entries(options.lakeTables ?? {})) {
    const path = `lakeTables.${key}`;
    if (!(tbl instanceof LakeTableDef)) {
      err("E_SCHEMA", path, "not a lake table (use lakeTable({...}))");
      continue;
    }
    noted(tbl, path);
    if (tbl.options.plugin) {
      err("E_LAKE_DEFINE", path, `\`${tbl.sourceName()}\` is another plugin's lake table`, "list it in plugin({ lakeDependencies })");
      continue;
    }
    const name = tbl.options.name ?? snake(key);
    if (!LAKE_NAME.test(name) || name.includes("__") || name.endsWith("_")) {
      err("E_LAKE_TABLE_NAME", path, `\`${name}\` is not a lake table name`, "snake_case, at most 32 characters, no `__`, not ending in `_`");
    }
    if (lakeTables.some((x) => x.name === name)) err("E_LAKE_TABLE_DUP", path, `lake table \`${name}\` is declared twice`);
    if ([...entityNames.values()].includes(name) || viewNameList.includes(name)) {
      err("E_LAKE_TABLE_CLASH", path, `\`${name}\` is the name of an entity or a view of the plugin`);
    }
    lakeNames.set(tbl, name);
    const layer = tbl.options.layer instanceof LayerDef ? layerNames.get(tbl.options.layer) : undefined;
    if (layer === undefined) err("E_LAKE_TABLE_LAYER", `${path}.layer`, "the layer is not in plugin({ layers })");
    const columns = (list: readonly string[] | undefined, what: "partition" | "key", code: string): string[] =>
      (list ?? []).flatMap((c) => {
        if (!(c in tbl.columns)) {
          err(code, `${path}.${what}`, `\`${c}\` is not a column of ${name}`);
          return [];
        }
        return [snake(c)];
      });
    const out: DescribedLakeTable = { name, layer: layer ?? "", message: visitMessage(tbl, path) };
    if (tbl.options.description) out.description = tbl.options.description;
    const partition = columns(tbl.options.partition, "partition", "E_LAKE_PARTITION");
    if (partition.length) out.partition = partition;
    const keyColumns = columns(tbl.options.key, "key", "E_LAKE_KEY");
    if (keyColumns.length) out.key = keyColumns;
    const read: string[] = [];
    (tbl.options.read ?? []).forEach((r, i) => {
      const role = r instanceof RoleDef ? roleNames.get(r) : undefined;
      if (role === undefined) {
        err("E_LAKE_ROLE", `${path}.read[${i}]`, "the role is not in plugin({ roles })", "a lake table is read by the plugin's own roles");
      } else if (!read.includes(role)) read.push(role); // each once, where first named (a history's alike)
    });
    if (read.length) out.read = read;
    if (tbl.options.systemRead) out.system_read = true;
    if (tbl.options.export) out.export = true;
    if (tbl.options.exportToModels) out.export_models = true;
    lakeTables.push(out);
  }
  // the models, in the order of their tables, once every table has its name
  const lakeModels: DescribedModel[] = [];
  for (const [key, tbl] of Object.entries(options.lakeTables ?? {})) {
    if (!(tbl instanceof LakeTableDef) || tbl.options.plugin || tbl.options.model === undefined) continue;
    const name = lakeNames.get(tbl) ?? "";
    if (typeof tbl.options.model !== "function") {
      err("E_LAKE_MODEL_DEFINE", `lakeTables.${key}.model`, "not a model (model: (m) => m.from(..)..columns({..}))");
      continue;
    }
    const d = describeModel(name, tbl.columns, tbl.options.model as never);
    if (typeof d === "string") err("E_LAKE_MODEL_DEFINE", `lakeTables.${key}.model`, d);
    else lakeModels.push(d);
  }
  (options.lakeDependencies ?? []).forEach((tbl, i) => {
    const path = `lakeDependencies[${i}]`;
    if (!(tbl instanceof LakeTableDef) || !tbl.options.plugin || !tbl.options.name) {
      err("E_LAKE_DEFINE", path, "not another plugin's lake table", "lakeTable({ plugin: 'retail', name: 'stock', columns })");
      return;
    }
    noted(tbl, path);
    let dep = dependencies.find((d) => d.plugin === tbl.options.plugin);
    if (!dep) {
      dep = { plugin: tbl.options.plugin, calls: [] };
      dependencies.push(dep);
    }
    dep.lake ??= [];
    if (!dep.lake.includes(tbl.options.name)) dep.lake.push(tbl.options.name);
  });
  (options.historyDependencies ?? []).forEach((e, i) => {
    const path = `historyDependencies[${i}]`;
    const owner = e instanceof EntityDef ? foreignPlugin(e) : undefined;
    if (!owner || !e.options.name) {
      err("E_LAKE_DEFINE", path, "not another plugin's entity", "entity({ plugin: 'helpdesk', name: 'ticket', columns }): another plugin's entity, by its owner and name");
      return;
    }
    noted(e, path);
    let dep = dependencies.find((d) => d.plugin === owner);
    if (!dep) {
      dep = { plugin: owner, calls: [] };
      dependencies.push(dep);
    }
    dep.lake ??= [];
    if (!dep.lake.includes(e.options.name)) dep.lake.push(e.options.name);
  });

  // --- the loads of the entities stored in OLAP, in entity order, once every lake table has its name ----------
  const loads: DescribedLoad[] = [];
  for (const [key, e] of entityList) {
    if (e.options.storage !== "olap" || e.options.load === undefined) continue;
    if (typeof e.options.load !== "function") {
      err("E_LOAD_DEFINE", `entities.${key}.load`, "not a load (load: (l) => l.from(..)..columns({..}))");
      continue;
    }
    const d = describeLoad(entityNames.get(e) ?? "", e.columns, e.options.load as never);
    if ("code" in d) err(d.code, `entities.${key}.load`, d.message);
    else loads.push(d);
  }

  // --- the entities' histories, now that the layers have their names ------------------------------
  for (const [key, e] of entityList) {
    const decl = entities.find((x) => x.name === entityNames.get(e));
    if (!decl) continue;
    const path = `entities.${key}`;
    const kept = (k: string) => (e.columns[k] as AnyField | undefined)?.column.history === true;
    for (const f of decl.fields) {
      const k = Object.keys(e.columns).find((c) => snake(c) === f.name);
      if (f.name !== "id" && k !== undefined && kept(k)) f.history = true;
    }
    const h = e.options.history;
    if (h === undefined) {
      const f = decl.fields.find((x) => x.history);
      if (f) err("E_HISTORY_LAYER", `${path}.columns.${f.name}`, `${decl.name}.${f.name} is kept, but the entity keeps no history`, "history: { layer } on the entity");
      continue;
    }
    const layer = h.layer instanceof LayerDef ? layerNames.get(h.layer) : undefined;
    if (layer === undefined) err("E_HISTORY_LAYER", `${path}.history.layer`, "the layer is not in plugin({ layers })");
    const history: DescribedHistory = { layer: layer ?? "" };
    if (h.all) history.all = true;
    if (h.unmasked) history.unmasked = true;
    const read: string[] = [];
    (h.read ?? []).forEach((r, i) => {
      const role = r instanceof RoleDef ? roleNames.get(r) : undefined;
      if (role === undefined) {
        err("E_HISTORY_ROLE", `${path}.history.read[${i}]`, "the role is not in plugin({ roles })", "a history is read by the plugin's own roles");
      } else if (!read.includes(role)) read.push(role); // each once, where first named (a lake table's alike)
    });
    if (read.length) history.read = read;
    if (h.systemRead) history.system_read = true;
    if (h.export) history.export = true;
    decl.history = history;
    // `all`: the flag on every column it covers, by the platform's rule (the encrypted ones, the lists and messages,
    // the history's own column names and, without `unmasked`, the masked ones left out)
    if (h.all) {
      const masked = new Set(((decl.access?.masks ?? []) as { field: string }[]).map((m) => m.field));
      for (const f of decl.fields) {
        if (f.name === "id" || f.encrypted || HISTORY_COLUMNS.includes(f.name) || !lakeType(f)) continue;
        if (!h.unmasked && masked.has(f.name)) continue;
        f.history = true;
      }
    }
  }

  // --- the settings message comes last (after the lake tables' rows), with the enums its fields use -------------
  const settingsDef = options.settings;
  let settingsSchema: MessageDef<any> | undefined;
  if (settingsDef !== undefined) {
    if (!(settingsDef instanceof SettingsDef) || !(settingsDef.schema instanceof MessageDef)) {
      err("E_SCHEMA", "settings", "not settings (use settingsDef(message, {..}))");
    } else {
      noted(settingsDef, "settings");
      settingsSchema = settingsDef.schema;
      visitMessage(settingsSchema, "settings");
      if (settingsSchema.kind !== "settings") {
        err("E_SETTINGS_MESSAGE", "settings", `${settingsSchema.name} is not a message of kind settings`,
          "declare it with t.message(.., { kind: 'settings' })");
      }
    }
  }

  (options.enums ?? []).forEach((en, i) => {
    if (en instanceof EnumDef) visitEnum(en, `enums[${i}]`);
    else err("E_SCHEMA", `enums[${i}]`, "not an enum (use t.enum(...))");
  });

  const descriptor: Descriptor = {
    schema: 1,
    id,
    version: version ?? "",
    title: options.title ?? (typeof id === "string" ? humanize(id) : ""),
    description: options.description ?? "",
    roles,
    scopes,
    capabilities,
    permissions,
    agents,
    dependencies,
    enums,
    messages,
    entities,
    endpoints,
  };
  if (connections.length > 0) descriptor.connections = connections;
  if (jobs.length > 0) descriptor.jobs = jobs;
  // a scheduled job runs with its payload's defaults: a required timestamp, date or file has none
  for (const [path, payload] of scheduled) {
    const field = requiredWithoutDefault(descriptor, payload);
    if (field !== undefined) {
      err("E_JOB_SCHEDULE_PAYLOAD", path, `a scheduled job runs with its payload's defaults, and ${field} has none`,
        "make the field optional, or start the job by enqueueing it instead of a schedule");
    }
  }
  if (topics.length > 0) descriptor.topics = topics;
  if (views.length > 0) descriptor.views = views;
  if (layers.length > 0) descriptor.layers = layers;
  if (lakeTables.length > 0) descriptor.lake_tables = lakeTables;
  if (lakeModels.length > 0) descriptor.lake_models = lakeModels;
  if (loads.length > 0) descriptor.loads = loads;
  // --- notification kinds: written only when declared; they need push:send
  const kinds = options.notifications ?? [];
  if (kinds.length > 0 && !capabilities.some((c) => c.name === "push:send")) {
    err("E_NOTIFY_CAPABILITY", "notifications", "notification kinds need the capability push:send");
  }
  const notifications: NonNullable<Descriptor["notifications"]> = [];
  kinds.forEach((k: unknown, i) => {
    const path = `notifications[${i}]`;
    if (!(k instanceof NotificationKind)) {
      err("E_SCHEMA", path, "not a notification kind (use notification(name, {..}))");
      return;
    }
    noted(k, path);
    if (typeof k.name !== "string" || !/^[a-z][a-z0-9_]{0,31}$/.test(k.name)) {
      err("E_NOTIFY_KIND_NAME", path, `\`${k.name}\` must match ^[a-z][a-z0-9_]{0,31}$`);
    } else if (notifications.some((x) => x.name === k.name)) {
      err("E_NOTIFY_KIND_NAME", path, `notification kind \`${k.name}\` is declared twice`);
    }
    notifications.push({
      name: k.name,
      title: k.options.title ?? "",
      description: k.options.description ?? "",
      tab: k.options.tab ?? "for_you",
      digest: !!k.options.digest,
    });
  });
  if (notifications.length > 0) descriptor.notifications = notifications;
  // --- the run-change hook: a system route, and `runs` in the descriptor
  if (options.onRunChange !== undefined) {
    const handler = options.onRunChange;
    if (typeof handler !== "function") {
      err("E_SCHEMA", "onRunChange", "not a function (ctx, change) => ...");
    } else {
      descriptor.runs = { on_change: true };
      resolvedEndpoints.push({
        key: "onRunChange",
        name: RUN_CHANGE_METHOD,
        input: NO_FIELDS,
        output: NO_FIELDS,
        timeoutMs: RUN_CHANGE_TIMEOUT_MS,
        system: true,
        decode: runChangeOf,
        handler: (async (ctx: Parameters<typeof handler>[0], change: Parameters<typeof handler>[1]) => {
          await handler(ctx, change);
          return {};
        }) as unknown as ResolvedEndpoint["handler"],
      });
    }
  }
  // --- the settings: the form (laid out by the plugin and completed, or derived), the check hook's route
  if (settingsSchema !== undefined && settingsDef instanceof SettingsDef) {
    const fields = messages.find((m) => m.name === settingsSchema!.name)?.fields ?? [];
    const { onChange, form } = settingsDef.options;
    let sections;
    if (form === undefined) sections = deriveForm(fields, enums);
    else if (typeof form !== "function") {
      err("E_SCHEMA", "settings.form", "not a form ((f) => { f.number(..); .. })");
      sections = deriveForm(fields, enums);
    } else sections = completeForm(settingsFormJson(form as never), fields, enums);
    descriptor.settings = { message: settingsSchema.name, on_change: onChange !== undefined, form: { sections } };
    if (onChange !== undefined) {
      if (typeof onChange !== "function") {
        err("E_SCHEMA", "settings.onChange", "not a function (ctx, change) => ...");
      } else {
        const schema = settingsSchema;
        resolvedEndpoints.push({
          key: "settings.onChange",
          name: SETTINGS_CHECK_METHOD,
          input: NO_FIELDS,
          output: NO_FIELDS,
          timeoutMs: SETTINGS_CHECK_TIMEOUT_MS,
          system: true,
          decode: (body) => settingsChangeOf(schema, body),
          handler: (async (ctx: Parameters<typeof onChange>[0], change: Parameters<typeof onChange>[1]) => {
            await onChange(ctx, change);
            return {};
          }) as unknown as ResolvedEndpoint["handler"],
        });
      }
    }
  }
  // the options of a setting and the secret type belong to the settings message; its fields and form
  settingsErrors(messages, enums, descriptor.settings, err);
  if (options.ui !== undefined) {
    if (options.ui instanceof UiDoc) {
      noted(options.ui, "ui");
      descriptor.ui = resolveUi(options.ui, {
        endpoint: endpointName,
        entity: (e, path) => {
          const name = entityNames.get(e);
          if (name === undefined) err("E_FIELD_TYPE", path, "the ref's entity is not in plugin({ entities })");
          return `${id}.${name ?? ""}`;
        },
      });
      // upload inputs carry their file field's options, so a screen checks a file before uploading it
      fillUploads(descriptor.ui, descriptor);
    } else {
      err("E_SCHEMA", "ui", "not a UI document (use ui.define({...}))");
    }
  }
  return {
    descriptor,
    errors,
    endpoints: resolvedEndpoints,
    jobs: resolvedJobs,
    jobSlots: resolvedJobs.size > 0 ? jobSlots : 0,
    roleName,
    ...(settingsSchema !== undefined ? { settings: settingsSchema } : {}),
  };
}

/**
 * Copies each `upload` input's file field options into it (`multiple` for a list of files, `max_size`, `accept`), from
 * the request of its form's endpoint, inputs of a `repeat` included.
 */
function fillUploads(ui: unknown, d: Descriptor): void {
  if (Array.isArray(ui)) {
    for (const v of ui) fillUploads(v, d);
    return;
  }
  if (ui === null || typeof ui !== "object") return;
  const m = ui as Record<string, unknown>;
  const form = m.form as Record<string, unknown> | undefined;
  if (form !== null && typeof form === "object" && !Array.isArray(form)) {
    const input = d.endpoints.find((e) => e.name === form.endpoint)?.input;
    if (input !== undefined && Array.isArray(form.sections)) {
      for (const s of form.sections as Record<string, unknown>[]) {
        if (Array.isArray(s?.inputs)) fillUploadInputs(s.inputs, input, d);
      }
    }
  }
  for (const v of Object.values(m)) fillUploads(v, d);
}

function fillUploadInputs(inputs: Record<string, unknown>[], msg: string, d: Descriptor): void {
  for (const i of inputs) {
    const f = typeof i?.name === "string" ? fieldAt(d, msg, i.name) : undefined;
    if (f === undefined) continue;
    if (i.control === "upload" && f.type === "file") {
      if (f.repeated) i.multiple = true;
      if (f.max_size !== undefined) i.max_size = f.max_size;
      if (f.accept !== undefined && f.accept.length > 0) i.accept = [...f.accept];
    }
    if (f.type.startsWith("message:") && Array.isArray(i.fields)) {
      fillUploadInputs(i.fields as Record<string, unknown>[], f.type.slice("message:".length), d);
    }
  }
}

/** A field of a message by its path (`customer.name`), through nested messages. */
function fieldAt(d: Descriptor, msg: string, path: string): DescriptorField | undefined {
  const dot = path.indexOf(".");
  const head = dot < 0 ? path : path.slice(0, dot);
  const f = d.messages.find((m) => m.name === msg)?.fields.find((x) => x.name === head);
  if (f === undefined || dot < 0) return f;
  return f.type.startsWith("message:") ? fieldAt(d, f.type.slice("message:".length), path.slice(dot + 1)) : undefined;
}

/**
 * The first field of message `name` that a payload read from `{}` cannot fill (`RemindUnpaid.since`): a single
 * `timestamp`, `date` or `file` that is not optional, directly or in a message field that is not optional.
 */
function requiredWithoutDefault(d: Descriptor, name: string, seen: string[] = []): string | undefined {
  if (seen.includes(name)) return undefined;
  seen.push(name);
  const fields = d.messages.find((m) => m.name === name)?.fields ?? d.entities.find((e) => e.message === name)?.fields;
  for (const f of fields ?? []) {
    if (f.optional || f.repeated) continue;
    if (f.type === "timestamp" || f.type === "date" || f.type === "file") return `${name}.${f.name}`;
    if (f.type.startsWith("message:")) {
      const inner = f.type.slice("message:".length);
      if (!d.messages.some((m) => m.name === inner)) continue;
      const found = requiredWithoutDefault(d, inner, seen);
      if (found !== undefined) return found;
    }
  }
  return undefined;
}
