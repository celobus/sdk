// `testCtx(plugin, {..})`: a context without the platform, for unit tests of handlers and of jobs' handlers.
import { Caller, Ctx, JobCtx, NO_RUN, type CallerKind, type JobInfo, type RoleLike, type SyscallSession } from "./ctx.js";
import { unavailable } from "./errors.js";
import { MemoryFiles } from "./files.js";
import type { Logger } from "./log.js";
import type { Plugin } from "./plugin.js";
import { encodeFieldValue, messageFields } from "./codec.js";
import type { MessageDef } from "./schema.js";
import { SettingsCache } from "./settings.js";

/** The settings a test gives, in the plugin's JSON mapping. */
function testSettings(schema: MessageDef<any> | undefined, values: Record<string, unknown>): SettingsCache {
  if (schema === undefined) throw new Error("testCtx: the plugin declares no settings (plugin({ settings: settingsDef(..) }))");
  const fields = messageFields(schema);
  const json: Record<string, unknown> = {};
  for (const [key, v] of Object.entries(values)) {
    const f = fields.find((x) => x.key === key);
    if (f === undefined) throw new Error(`testCtx: settings: ${key} is not a field of ${schema.name}`);
    if (v === undefined || v === null) continue;
    json[f.json] = f.field.spec.kind === "array"
      ? (v as unknown[]).map((x, i) => encodeFieldValue(f.field.spec.inner!, x, `${key}[${i}]`))
      : encodeFieldValue(f.field, v, key);
  }
  return new SettingsCache(schema, json);
}

/** Who calls in a test context: a person (or a service account) with this id. */
export interface TestCaller {
  userId: string;
  /** Default `user`. */
  kind?: Exclude<CallerKind, "system">;
  /** Default `Test User`. */
  displayName?: string;
  /** The plugin's role objects, their short names (`clerk`) or full names (`workspace.admin`). */
  roles?: readonly RoleLike[];
  /** Group ids. */
  groups?: readonly string[];
  /** Short (`read`) or full (`orders:read`) scope names. */
  scopes?: readonly string[];
}

/** The run a job's handler is in, in a test context; it was scheduled now. */
export interface TestRun {
  runId: string;
  /** 1 is the first attempt. */
  attempt: number;
  maxAttempts: number;
  /** Default empty. */
  job?: string;
}

export interface TestCtxOptions {
  /** Default: no caller (`ctx.caller.kind` is `system`). */
  caller?: TestCaller;
  /** Default: no run (`ctx.job.isRun` is false). */
  run?: TestRun;
  /** `ctx.idempotencyKey`; default none. */
  idempotencyKey?: string;
  /** Sets `ctx.request.deadline`; default 30 s from now. */
  timeoutMs?: number;
  /** `ctx.signal`; default one that never aborts. */
  signal?: AbortSignal;
  /** `ctx.log`; default one that drops every line. */
  log?: Logger;
  /**
   * Where `ctx.files` reads and writes: give one to put files in before (`files.add(..)`) and read what the handler
   * wrote after (`files.text(file)`); default a new, empty one.
   */
  files?: MemoryFiles;
  /**
   * The plugin's settings `ctx.settings(..)` reads, by their keys as declared (`{ threadWarmHours: 4, webhookToken: new
   * Secret('sk-1') }`): the declared defaults fill the rest. Default: none (`ctx.settings(..)` is a 500).
   */
  settings?: Record<string, unknown>;
}

const quiet: Logger = {
  debug: () => {},
  info: () => {},
  warn: () => {},
  error: () => {},
  with: () => quiet,
};

/**
 * A context of `plugin` without the platform, for unit tests of handlers: by default the caller is the system, the
 * request id `0` and its deadline 30 s away. `ctx.files` works on files kept in memory (`options.files`). Every call
 * that needs the platform (`ctx.db`, `ctx.query`, `ctx.sql`, `ctx.transaction`, `ctx.call` and the dependency clients,
 * `ctx.members`, `ctx.groups`, `ctx.agents`, `ctx.jobs`, `ctx.notifications`) rejects with 503 `unavailable`;
 * `ctx.invalidate` does nothing. Put the logic worth testing in functions the handler calls with what it read.
 *
 *   const ctx = testCtx(crm, { caller: { userId: 'u1', roles: [sales] } });
 *   await assert.rejects(handler(ctx, input), { code: 'forbidden' });
 *   const run = testCtx(crm, { run: { runId: 'run-1', attempt: 1, maxAttempts: 4 } });   // a JobCtx
 *   const files = new MemoryFiles(); const csv = files.add('a.csv', 'text/csv', 'a,b\n1,2\n');
 *   await importCsv.options.handler(testCtx(crm, { files }), { file: csv });
 */
export function testCtx(plugin: Plugin, options: TestCtxOptions & { run: TestRun }): JobCtx;
export function testCtx(plugin: Plugin, options?: TestCtxOptions): Ctx;
export function testCtx(plugin: Plugin, options: TestCtxOptions = {}): Ctx {
  const model = plugin.model();
  const id = model.descriptor.id;
  const session: SyscallSession = {
    syscall: () => Promise.reject(unavailable("the session with the platform is closed")),
    invalidate: () => {},
    jobs: new Map([...model.jobs.values()].map((j) => [j.def, j])),
    files: options.files ?? new MemoryFiles(),
  };
  if (options.settings !== undefined) session.settings = testSettings(model.settings, options.settings);
  const c = options.caller;
  const principal = c && {
    userId: c.userId,
    kind: c.kind === "service" ? "SERVICE" : "PERSON",
    displayName: c.displayName ?? "Test User",
    roles: (c.roles ?? []).map((r) => {
      const name = model.roleName(r);
      if (name === undefined) throw new Error("testCtx: a role of the caller is not in plugin({ roles })");
      return name.includes(".") ? name : `${id}.${name}`;
    }),
    groups: [...(c.groups ?? [])],
    scopes: (c.scopes ?? []).map((s) => (s.includes(":") ? s : `${id}:${s}`)),
  };
  const run = options.run;
  const job: JobInfo = run
    ? Object.freeze({
        job: run.job ?? "",
        runId: run.runId,
        attempt: run.attempt,
        maxAttempts: run.maxAttempts,
        scheduledAt: new Date(),
        isRun: true,
        isLastAttempt: run.attempt >= run.maxAttempts,
      })
    : NO_RUN;
  return new (run ? JobCtx : Ctx)(
    session,
    new Caller(principal, id, model.roleName),
    {
      id: "0",
      deadline: new Date(Date.now() + (options.timeoutMs ?? 30_000)),
      signal: options.signal ?? new AbortController().signal,
      idempotencyKey: options.idempotencyKey,
    },
    { id, version: model.descriptor.version },
    options.log ?? quiet,
    "0",
    "",
    job,
  );
}
