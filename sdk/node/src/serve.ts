// The plugin session (`Celld.Session` of proto/plugin.proto) and request dispatch.
//
// The plugin connects to the platform at $CELOBUS_ADDR, sends `Register`, then serves `Request`s concurrently
// (at most `maxConcurrency` handlers at once, up to 256 more wait in order, past them 503 `unavailable` with
// `retry-after: 1`) and sends `Syscall`s for its context. When
// the session drops, pending syscalls fail with `unavailable`, in-flight handlers are aborted (their answers
// could not be delivered), and the plugin reconnects with backoff. SIGTERM (and SIGINT) drain: new requests get
// 503 `unavailable` (with `retry-after: 1`), in-flight ones finish (up to `drainMs`), then the session closes and the process exits.
//
// Background jobs: the plugin announces its job pool (`jobSlots`) when it registers, and runs each `JobRun` on that
// pool, apart from the requests; the platform never hands it more runs than it has slots. A run's handler gets a
// context whose `job` is the run and whose `signal` aborts on `JobAbort` (and when the session ends); the attempt is
// answered with `JobDone` (at once on an abort). A drain waits for the runs too, and a run that arrives while
// draining is refused as `shutdown`.
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import * as grpc from "@grpc/grpc-js";
import * as loader from "@grpc/proto-loader";
import { decodeMessage, encodeMessage, OutputError, type FieldErrors } from "./codec.js";
import { Caller, Ctx, JobCtx, SyscallError, type SyscallResult, type SyscallSession, type WirePrincipal } from "./ctx.js";
import type { ResolvedEndpoint, Resolved } from "./descriptor.js";
import { DomainError, InvalidError, RpcError, toRpcError, unavailable } from "./errors.js";
import { cutUtf8, runLogger, type Logger, type LogLevel } from "./log.js";
import { endpointPath } from "./names.js";
import { SettingsCache } from "./settings.js";

export interface Header {
  name: string;
  value: string;
}

/** `Request` as the platform sends it. */
export interface WireRequest {
  id: string;
  method: string;
  path: string;
  query: string;
  headers: Header[];
  body: Buffer;
  principal?: WirePrincipal | null;
}

export interface WireResponse {
  status: number;
  headers: Header[];
  body: Buffer;
}

export interface ServeOptions {
  /** Default `process.argv`; `--describe` prints the descriptor and exits. */
  argv?: readonly string[];
  /** Default `$CELOBUS_ADDR`, which the platform sets. */
  addr?: string;
  /** Handlers running at once; default 64. Up to 256 more requests wait in order; past them 503 `unavailable`. */
  maxConcurrency?: number;
  /** Drain SIGTERM/SIGINT and exit; default true. */
  signals?: boolean;
  /** Stops serving (drains, closes the session, resolves `serve()`), for tests and embedding. */
  stop?: AbortSignal;
  /** How long a drain waits for in-flight requests; default 10 s. */
  drainMs?: number;
}

/** @internal What dispatch needs from the plugin. */
export interface ServeModel {
  id: string;
  version: string;
  depends: string[];
  resolved: Resolved;
  log: Logger;
}

function protoPath(): string {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const candidates = [
    process.env.CELOBUS_PROTO,
    path.join(here, "..", "proto", "plugin.proto"), // the package (copied at build)
    path.join(here, "..", "..", "..", "proto", "plugin.proto"), // the platform checkout
  ].filter((p): p is string => !!p);
  const found = candidates.find((p) => fs.existsSync(p));
  if (!found) throw new Error("plugin.proto not found; set CELOBUS_PROTO");
  return found;
}

let celldService: grpc.ServiceClientConstructor | undefined;

/** @internal The `celobus.v1.Celld` service, loaded from plugin.proto. */
export function celldDefinition(): grpc.ServiceClientConstructor {
  if (!celldService) {
    const def = loader.loadSync(protoPath(), { keepCase: false, longs: String, enums: String, defaults: true, oneofs: true });
    const pkg = grpc.loadPackageDefinition(def) as any;
    celldService = pkg.celobus.v1.Celld as grpc.ServiceClientConstructor;
  }
  return celldService;
}

function json(status: number, body: unknown): WireResponse {
  return {
    status,
    headers: [{ name: "content-type", value: "application/json" }],
    body: Buffer.from(JSON.stringify(body)),
  };
}

function errorResponse(e: RpcError): WireResponse {
  return json(e.status, e.toJSON());
}

/** Requests that wait for a handler at most; past them the answer is 503 `unavailable`. */
const MAX_QUEUED = 256;

/** 503 `unavailable` with `retry-after: 1`: the plugin cannot take the request now. */
function refused(why: string): WireResponse {
  const resp = errorResponse(unavailable(why));
  resp.headers.push({ name: "retry-after", value: "1" });
  return resp;
}

function header(req: WireRequest, name: string): string | undefined {
  return req.headers?.find((h) => h.name.toLowerCase() === name)?.value;
}

/** @internal Serves one request: route, decode, run the handler under its deadline, encode, map errors. */
export async function dispatch(
  model: ServeModel,
  routes: Map<string, ResolvedEndpoint>,
  session: SyscallSession,
  req: WireRequest,
  sessionSignal?: AbortSignal,
): Promise<WireResponse> {
  const found = routes.get(req.path);
  // the platform's own requests (the run-change hook) carry no caller: one with a caller came from outside
  const ep = found?.system && req.principal ? undefined : found;
  if (!ep) return errorResponse(new RpcError(404, "unknown_endpoint", `no endpoint ${req.path}`));
  if ((req.method || "POST").toUpperCase() !== "POST") {
    return errorResponse(new RpcError(405, "method_not_allowed", `${req.method} is not allowed; use POST`));
  }
  const contentType = header(req, "content-type");
  if (contentType && !/^application\/json\s*(;|$)/i.test(contentType)) {
    return errorResponse(new RpcError(415, "unsupported_media_type", `${contentType} is not supported; send application/json`));
  }
  const log = model.log.with({ endpoint: ep.name, request_id: req.id });

  let body: unknown;
  try {
    const text = Buffer.from(req.body ?? []).toString("utf8");
    body = text.trim() ? JSON.parse(text) : {};
  } catch {
    return errorResponse(new InvalidError("the body is not JSON"));
  }
  if (ep.decode) {
    try {
      body = ep.decode(body);
    } catch (e) {
      return errorResponse(toRpcError(e));
    }
  }
  const fieldErrors: FieldErrors = {};
  const input = ep.decode ? body : decodeMessage(ep.input, body, "", fieldErrors);
  if (Object.keys(fieldErrors).length > 0) {
    const e = new InvalidError("the request is invalid");
    for (const [p, m] of Object.entries(fieldErrors)) e.field(p, m);
    return errorResponse(e);
  }

  const controller = new AbortController();
  const deadline = new Date(Date.now() + ep.timeoutMs);
  const timer = setTimeout(
    () => controller.abort(new RpcError(504, "deadline_exceeded", `${ep.name} did not answer within ${ep.timeoutMs} ms`)),
    ep.timeoutMs,
  );
  const onSessionEnd = () => controller.abort(unavailable("the session to the platform ended"));
  if (sessionSignal?.aborted) onSessionEnd();
  else sessionSignal?.addEventListener("abort", onSessionEnd, { once: true });
  const aborted = new Promise<never>((_, reject) => {
    if (controller.signal.aborted) reject(controller.signal.reason);
    controller.signal.addEventListener("abort", () => reject(controller.signal.reason), { once: true });
  });
  aborted.catch(() => {});

  const ctx = new Ctx(
    session,
    new Caller(req.principal, model.id, model.resolved.roleName),
    { id: String(req.id), deadline, signal: controller.signal, idempotencyKey: header(req, "idempotency-key") },
    { id: model.id, version: model.version },
    log,
  );
  try {
    const output = await Promise.race([Promise.resolve().then(() => ep.handler(ctx, input as never)), aborted]);
    return json(200, encodeMessage(ep.output, output));
  } catch (e) {
    const err = toRpcError(e);
    if (err.status >= 500) {
      const cause = e instanceof RpcError ? e.cause : e;
      log.error(e instanceof OutputError ? "the handler's output does not fit its type" : "the request failed", {
        status: err.status,
        code: err.code,
        error: cause ?? err,
      });
    }
    return errorResponse(err);
  } finally {
    clearTimeout(timer);
    sessionSignal?.removeEventListener("abort", onSessionEnd);
  }
}

/** `JobRun` as the platform sends it (64-bit numbers as strings). */
export interface WireJobRun {
  runId: string;
  job: string;
  attempt: number;
  maxAttempts: number;
  payload: string;
  principal?: WirePrincipal | null;
  scheduledAtUnixMs: string | number;
  timeoutMs: number;
  requestId: string | number;
}

/** The error code of an abort's reason (an unknown reason is a cancel). */
const ABORT_CODES: Record<string, string> = {
  TIMEOUT: "timeout",
  CANCELLED: "cancelled",
  SHUTDOWN: "shutdown",
  LEASE_LOST: "lease_lost",
  AUTH_CHANGED: "auth_changed",
  REMOVED: "job_removed",
};

/** The longest error message a `JobDone` carries, in UTF-8 bytes. */
const MAX_JOB_ERROR = 2048;

/** A run's attempt stopped: `code` is what its `JobDone` says. */
class RunStopped extends RpcError {
  constructor(code: string) {
    super(503, code, `stopped: ${code}`);
  }
}

/** The `JobError` of what a run's handler threw. */
function jobError(e: unknown): { code: string; message: string; permanent: boolean } {
  const err = e instanceof DomainError ? e.error() : e;
  if (err instanceof RpcError) return { code: err.code, message: cutUtf8(err.message, MAX_JOB_ERROR), permanent: err.permanent };
  const message = err instanceof Error ? err.message : String(err);
  return { code: "internal", message: cutUtf8(message, MAX_JOB_ERROR), permanent: false };
}

const LEVELS: Record<LogLevel, string> = { debug: "DEBUG", info: "INFO", warn: "WARN", error: "ERROR" };

interface Link {
  stream: grpc.ClientDuplexStream<any, any>;
  pending: Map<string, { resolve: (r: SyscallResult) => void; reject: (e: Error) => void }>;
  closed: boolean;
  /** aborted when this session ends */
  ended: AbortController;
}

/** @internal Runs the session until stopped. */
export function runSession(model: ServeModel, options: ServeOptions = {}): Promise<void> {
  const addr = options.addr ?? process.env.CELOBUS_ADDR;
  if (!addr) throw new Error("CELOBUS_ADDR is not set (the platform sets it)");
  const maxConcurrency = Math.max(1, options.maxConcurrency ?? 64);
  const drainMs = options.drainMs ?? 10_000;
  const routes = new Map(model.resolved.endpoints.map((ep) => [endpointPath(model.id, ep.name), ep]));
  const log = model.log;

  const Celld = celldDefinition();
  const client = new Celld(addr, grpc.credentials.createInsecure(), {
    "grpc.max_receive_message_length": 64 << 20,
    "grpc.max_send_message_length": 64 << 20,
  });

  let link: Link | undefined;
  let nextSyscall = 1;
  let backoff = 250;
  let stopping = false;
  let stopped = false;
  let active = 0;
  const waiting: (() => void)[] = [];
  let idle: (() => void) | undefined;
  let finish!: () => void;
  const done = new Promise<void>((resolve) => (finish = resolve));

  const jobs = model.resolved.jobs ?? new Map();
  // the settings the platform sends at registration and on each change, kept across reconnects
  const settings = model.resolved.settings !== undefined ? new SettingsCache(model.resolved.settings) : undefined;
  const jobSlots = jobs.size > 0 ? model.resolved.jobSlots : 0;
  /** The runs in flight, by run id: what stops each and the session it came on. */
  const runs = new Map<string, { stop: AbortController; link: Link }>();

  const session: SyscallSession = {
    jobs: new Map([...jobs.values()].map((j) => [j.def, j])),
    settings,
    syscall(call) {
      const current = link;
      if (!current || current.closed) return Promise.reject(unavailable("the session to the platform is down"));
      const id = String(nextSyscall++);
      return new Promise((resolve, reject) => {
        current.pending.set(id, { resolve, reject });
        current.stream.write({ syscall: { id, ...call } });
      });
    },
    invalidate(topic, key) {
      const current = link;
      if (current && !current.closed) current.stream.write({ invalidate: { topic, key } });
    },
  };

  const acquire = (): Promise<void> => {
    if (active < maxConcurrency) {
      active++;
      return Promise.resolve();
    }
    return new Promise((resolve) => waiting.push(() => (active++, resolve())));
  };
  const busy = () => active > 0 || waiting.length > 0 || runs.size > 0;
  const release = () => {
    active--;
    waiting.shift()?.();
    if (!busy()) idle?.();
  };

  const onRequest = async (current: Link, req: WireRequest) => {
    let resp: WireResponse;
    if (waiting.length >= MAX_QUEUED) {
      resp = refused("the plugin is busy");
    } else if (stopping) {
      resp = refused("the plugin is stopping");
    } else {
      await acquire();
      try {
        resp = await dispatch(model, routes, session, req, current.ended.signal);
      } finally {
        release();
      }
    }
    if (!current.closed) current.stream.write({ response: { id: req.id, ...resp } });
  };

  const send = (current: Link, msg: Record<string, unknown>) => {
    if (!current.closed) current.stream.write(msg);
  };

  const onJobRun = async (current: Link, run: WireJobRun) => {
    const runId = String(run.runId);
    const attempt = Number(run.attempt ?? 0);
    const done = (outcome: Record<string, unknown>) => send(current, { jobDone: { runId, attempt, ...outcome } });
    const refuse = (code: string, message: string) => done({ error: { code, message, permanent: false } });
    if (stopping) return refuse("shutdown", "the plugin is stopping");
    const job = jobs.get(run.job);
    if (!job) return refuse("unavailable", `this version has no job ${run.job}`);
    if (runs.size >= jobSlots || runs.has(runId)) return refuse("unavailable", `all ${jobSlots} job slots are taken`);

    const stop = new AbortController();
    runs.set(runId, { stop, link: current });
    const requestId = String(run.requestId ?? "0");
    const timeoutMs = Number(run.timeoutMs ?? 0);
    const aborted = new Promise<never>((_, reject) => {
      stop.signal.addEventListener("abort", () => reject(stop.signal.reason), { once: true });
    });
    aborted.catch(() => {});
    const stderr = log.with({ job: job.name, run_id: runId, attempt });
    const ctxLog = runLogger(stderr, (level, message) =>
      send(current, { pluginLog: { requestId, level: LEVELS[level], message, atUnixMs: String(Date.now()) } }),
    );
    const maxAttempts = Number(run.maxAttempts ?? 0);
    const ctx = new JobCtx(
      session,
      new Caller(run.principal, model.id, model.resolved.roleName),
      { id: requestId, deadline: new Date(Date.now() + timeoutMs), signal: stop.signal },
      { id: model.id, version: model.version },
      ctxLog,
      "0",
      "",
      Object.freeze({
        job: job.name,
        runId,
        attempt,
        maxAttempts,
        scheduledAt: new Date(Number(run.scheduledAtUnixMs ?? 0)),
        isRun: true,
        isLastAttempt: attempt >= maxAttempts,
      }),
    );
    const started = Date.now();
    let outcome: Record<string, unknown>;
    try {
      let json: unknown;
      try {
        json = run.payload ? JSON.parse(run.payload) : {};
      } catch (e) {
        throw new RpcError(400, "payload_invalid", `the payload is not JSON: ${(e as Error).message}`);
      }
      const errors: FieldErrors = {};
      const payload = decodeMessage(job.payload, json, "", errors);
      const problems = Object.entries(errors).map(([p, m]) => `${p}: ${m}`);
      if (problems.length > 0) {
        throw new RpcError(400, "payload_invalid", `the payload is not a ${job.payload.name}: ${problems.join("; ")}`);
      }
      await Promise.race([Promise.resolve().then(() => job.handler(ctx, payload as never)), aborted]);
      outcome = { ok: {} };
      log.info("run done", { job: job.name, run_id: runId, attempt, elapsed_ms: Date.now() - started });
    } catch (e) {
      const error = stop.signal.aborted && stop.signal.reason instanceof RunStopped ? jobError(stop.signal.reason) : jobError(e);
      outcome = { error };
      log.warn("run failed", { job: job.name, run_id: runId, attempt, elapsed_ms: Date.now() - started, ...error });
    } finally {
      runs.delete(runId);
    }
    done(outcome);
    if (!busy()) idle?.();
  };

  const close = (current: Link, why: string) => {
    if (current.closed) return;
    current.closed = true;
    for (const p of current.pending.values()) p.reject(new SyscallError("the session to the platform ended"));
    current.pending.clear();
    current.ended.abort();
    // the runs of this session stop: their answers could not be delivered
    for (const run of runs.values()) if (run.link === current) run.stop.abort(new RunStopped("cancelled"));
    if (link === current) link = undefined;
    if (stopped) return;
    log.warn("session ended; reconnecting", { reason: why, retry_ms: backoff });
    setTimeout(start, backoff);
    backoff = Math.min(backoff * 2, 5_000);
  };

  function start() {
    if (stopped) return;
    const stream = (client as any).session() as grpc.ClientDuplexStream<any, any>;
    const current: Link = { stream, pending: new Map(), closed: false, ended: new AbortController() };
    link = current;
    stream.on("data", (msg: any) => {
      switch (msg.kind) {
        case "registered":
          backoff = 250;
          log.info("registered", { instance_id: msg.registered.instanceId, version: model.version });
          if (settings && !settings.update(msg.registered.settingsRev ?? 0, msg.registered.settings, true)) {
            log.warn("the settings the platform sent are not JSON; kept the previous ones");
          }
          break;
        case "settingsChanged": {
          const rev = msg.settingsChanged.rev ?? 0;
          if (settings?.update(rev, msg.settingsChanged.json, false)) log.info("the settings changed", { rev: String(rev) });
          break;
        }
        case "request":
          void onRequest(current, msg.request as WireRequest);
          break;
        case "jobRun":
          void onJobRun(current, msg.jobRun as WireJobRun);
          break;
        case "jobAbort": {
          const reason = String(msg.jobAbort.reason ?? "");
          runs.get(String(msg.jobAbort.runId))?.stop.abort(new RunStopped(ABORT_CODES[reason] ?? "cancelled"));
          break;
        }
        case "syscallResult": {
          const r = msg.syscallResult as SyscallResult;
          const p = current.pending.get(String(r.id));
          if (!p) break;
          current.pending.delete(String(r.id));
          if (r.error) p.reject(new SyscallError(r.error, r.errorDetails));
          else p.resolve(r);
          break;
        }
      }
    });
    stream.on("error", (e: Error) => close(current, e.message));
    stream.on("end", () => close(current, "end"));
    stream.write({
      register: {
        plugin: model.id,
        version: model.version,
        runtime: "node",
        depends: model.depends,
        manifestDigest: "",
        token: process.env.CELOBUS_PLUGIN_TOKEN ?? "",
        jobSlots,
      },
    });
  }

  const stop = async (exit: boolean) => {
    if (stopping) return;
    stopping = true;
    log.info("stopping", { in_flight: active + waiting.length, runs: runs.size });
    if (busy()) {
      await Promise.race([
        new Promise<void>((resolve) => (idle = resolve)),
        new Promise<void>((resolve) => setTimeout(resolve, drainMs).unref()),
      ]);
    }
    stopped = true;
    if (link) {
      const current = link;
      current.stream.end();
      close(current, "stopped");
    }
    client.close();
    finish();
    if (exit) process.exit(0);
  };

  if (options.signals ?? true) {
    process.once("SIGTERM", () => void stop(true));
    process.once("SIGINT", () => void stop(true));
  }
  if (options.stop) {
    if (options.stop.aborted) void stop(false);
    else options.stop.addEventListener("abort", () => void stop(false), { once: true });
  }
  log.info("connecting", { addr, plugin: model.id, version: model.version });
  start();
  return done;
}
