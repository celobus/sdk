// Background jobs: their descriptor and definition errors, the cron check, permanent errors, and the session against a
// fake platform (Register.jobSlots, enqueue and cancel on the wire, a run from JobRun to JobDone, aborts, the run log).
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { setTimeout as sleep } from "node:timers/promises";
import * as grpc from "@grpc/grpc-js";
import { endpoint, entity, invalid, job, notFound, permanent, plugin, RpcError, role, t, topic, unavailable } from "../dist/index.js";
import { cronError } from "../dist/cron.js";
import { Limiter, LINES_PER_SECOND, BYTES_PER_SECOND, runLine, sink } from "../dist/log.js";
import { celldDefinition } from "../dist/serve.js";

sink.write = () => {};

const codes = (p) => p.check().map((e) => `${e.code} ${e.path}`);

// --- the descriptor -------------------------------------------------------------------------------

test("a job is described under its key in snake_case, its inline payload a job message named in PascalCase", () => {
  const note = t.message("Note", { text: t.text() });
  const reportKey = t.message("ReportKey", { day: t.text() });
  const report = topic("report", reportKey);
  const remindUnpaid = job({
    payload: { olderThanDays: t.int() },
    description: "Remind unpaid orders",
    cron: "0 9 * * 1-5",
    as: "system",
    retry: { retries: 3, delay: "1m", backoff: "exponential" },
    timeout: "2m",
    concurrency: 4,
    handler() {},
  });
  const reindex = job({ payload: t.message("Reindex", {}), handler() {} });
  const renamed = job({ name: "nightly", payload: {}, retry: { retries: 0 }, handler() {} });
  const d = plugin({
    id: "orders",
    version: "0.1.0",
    messages: [note],
    topics: [report],
    jobs: { remindUnpaid, reindex, renamed },
  }).describe();
  assert.deepEqual(d.jobs, [
    {
      name: "remind_unpaid",
      payload: "RemindUnpaid",
      description: "Remind unpaid orders",
      cron: "0 9 * * 1-5",
      as_system: true,
      retry: { retries: 3, delay_ms: 60_000, backoff: "exponential" },
      timeout_ms: 120_000,
      concurrency: 4,
    },
    { name: "reindex", payload: "Reindex" },
    // a declared retry writes all three members, the defaults filled
    { name: "nightly", payload: "Nightly", retry: { retries: 0, delay_ms: 30_000, backoff: "exponential" } },
  ]);
  // the extra messages, then the jobs' payloads, then the topics' keys
  assert.deepEqual(
    d.messages.map((m) => `${m.name}:${m.kind}`),
    ["Note:message", "RemindUnpaid:job", "Reindex:job", "Nightly:job", "ReportKey:message"],
  );
  assert.deepEqual(d.messages[1].fields, [
    { name: "older_than_days", number: 1, type: "int32", repeated: false, optional: false, description: "" },
  ]);
});

test("a plugin without jobs has no `jobs` member", () => {
  const d = plugin({ id: "plain", version: "0.1.0" }).describe();
  assert.equal("jobs" in d, false);
});

test("job definition errors carry the descriptor's codes", () => {
  const shared = t.message("Shared", {});
  const h = () => {};
  const p = plugin({
    id: "orders",
    version: "0.1.0",
    jobs: {
      Bad: job({ payload: {}, handler: h }),
      tooLongANameForAJobOfThisPluginXyz: job({ payload: {}, handler: h }),
      one: job({ payload: shared, handler: h }),
      two: job({ payload: shared, handler: h }),
      dup: job({ name: "one", payload: {}, handler: h }),
      cron: job({ payload: {}, cron: "61 * * * *", handler: h }),
      since: job({ payload: { since: t.timestamp(), note: t.text() }, cron: "0 0 * * *", handler: h }),
      sinceOk: job({ payload: { since: t.timestamp().optional() }, cron: "0 0 * * *", handler: h }),
      retries: job({ payload: {}, retry: { retries: 21 }, handler: h }),
      delay: job({ payload: {}, retry: { delay: "2h" }, handler: h }),
      quick: job({ payload: {}, timeout: "500ms", handler: h }),
      long: job({ payload: {}, timeout: "2h", handler: h }),
      wide: job({ payload: {}, concurrency: 65, handler: h }),
      none: job({ payload: {}, concurrency: 0, handler: h }),
    },
    jobSlots: 65,
  });
  assert.deepEqual(codes(p), [
    "E_JOB_NAME jobs.Bad.name",
    "E_JOB_NAME jobs.tooLongANameForAJobOfThisPluginXyz.name",
    "E_JOB_PAYLOAD_DUP jobs.two.payload",
    "E_JOB_DUP jobs.dup",
    "E_JOB_CRON jobs.cron.cron",
    "E_JOB_RETRY jobs.retries.retry.retries",
    "E_JOB_RETRY jobs.delay.retry.delay",
    "E_JOB_TIMEOUT jobs.quick.timeout",
    "E_JOB_TIMEOUT jobs.long.timeout",
    "E_JOB_CONCURRENCY jobs.wide.concurrency",
    "E_JOB_CONCURRENCY jobs.none.concurrency",
    "E_JOB_SLOTS jobSlots",
    "E_JOB_SCHEDULE_PAYLOAD jobs.since.payload",
  ]);
  assert.match(p.check().find((e) => e.code === "E_JOB_SCHEDULE_PAYLOAD").message, /Since\.since has none/);
  assert.deepEqual(codes(plugin({ id: "orders", version: "0.1.0", jobSlots: 0 })), ["E_JOB_SLOTS jobSlots"]);
});

test("the cron check follows the platform's grammar", () => {
  for (const ok of ["* * * * *", "0 9 * * 1-5", "*/15 0-23/2 1,15 jan-mar MON-fri", "5/10 * * * 7", "0 0 29 2 *", "0 0 31 2 1"]) {
    assert.equal(cronError(ok), undefined, ok);
  }
  const bad = {
    "* * * *": /5 fields/,
    "60 * * * *": /out of 0\.\.=59 for the minute/,
    "* * 0 * *": /day of the month/,
    "*/0 * * * *": /a step of 0/,
    "5-1 * * * *": /runs backwards/,
    "* * * foo *": /is not a month/,
    "0 0 30 2 *": /no chosen month has the chosen day/,
    "@daily": /5 fields/,
    "* * * * 8": /out of 0\.\.=7/,
    "1,,2 * * * *": /in the minute field/,
  };
  for (const [cron, re] of Object.entries(bad)) assert.match(cronError(cron) ?? "", re, cron);
});

test("permanent errors: permanent(..), .asPermanent() and the input-class codes", () => {
  const p = permanent("no e-mail");
  assert.equal(p.status, 422);
  assert.equal(p.code, "permanent");
  assert.equal(p.permanent, true);
  assert.equal(unavailable("down").permanent, false);
  assert.equal(unavailable("down").asPermanent().permanent, true);
  assert.equal(invalid("bad").permanent, true);
  assert.equal(notFound().permanent, true);
  assert.equal(new RpcError(403, "permission_denied", "no").permanent, true);
  assert.equal(new RpcError(409, "conflict", "later").permanent, false);
  assert.deepEqual(p.toJSON(), { code: "permanent", message: "no e-mail", details: {} });
});

test("run log lines: the message, then the fields as key=value, cut at 2 KiB at a character boundary", () => {
  assert.equal(runLine("reminded", { order: "SO-1", n: 3, ok: true }), 'reminded order="SO-1" n=3 ok=true');
  assert.equal(runLine("plain"), "plain");
  const long = runLine("é".repeat(2000));
  assert.ok(Buffer.byteLength(long) <= 2048 && Buffer.byteLength(long) >= 2047);
  assert.ok([...long].every((c) => c === "é"));
});

test("the run log limit is a fixed one-second window of 1,000 lines and 1 MiB", () => {
  let l = new Limiter();
  for (let i = 0; i < LINES_PER_SECOND; i++) assert.ok(l.admit(1000, 10));
  assert.equal(l.admit(1999, 10), false);
  assert.ok(l.admit(2000, 10));
  l = new Limiter();
  assert.ok(l.admit(0, BYTES_PER_SECOND - 1));
  assert.equal(l.admit(0, 2), false);
  assert.ok(l.admit(0, 1));
});

// --- the session ----------------------------------------------------------------------------------

/** A fake platform: records what the plugin sends; answers syscalls with `answer` (a syscall → its result's members). */
async function fakePlatform(answer = () => undefined) {
  const socket = join(mkdtempSync(join(tmpdir(), "celobus-")), "plugin.sock");
  rmSync(socket, { force: true });
  const seen = { register: undefined, syscalls: [], done: [], logs: [], responses: new Map() };
  const waiters = new Map();
  const doneWaiters = new Map();
  let session;
  let onSession;
  const sessionOpened = new Promise((resolve) => (onSession = resolve));
  const server = new grpc.Server();
  server.addService(celldDefinition().service, {
    Session(call) {
      session = call;
      call.on("data", (msg) => {
        if (msg.kind === "register") {
          seen.register = msg.register;
          call.write({ registered: { instanceId: "test-1" } });
          onSession();
        } else if (msg.kind === "syscall") {
          const s = msg.syscall;
          seen.syscalls.push(s);
          call.write({ syscallResult: { id: s.id, error: "", ...(answer(s) ?? { error: "unsupported" }) } });
        } else if (msg.kind === "jobDone") {
          seen.done.push(msg.jobDone);
          doneWaiters.get(msg.jobDone.runId)?.(msg.jobDone);
        } else if (msg.kind === "pluginLog") {
          seen.logs.push(msg.pluginLog);
        } else if (msg.kind === "response") {
          seen.responses.set(msg.response.id, msg.response);
          waiters.get(msg.response.id)?.(msg.response);
        }
      });
      call.on("end", () => call.end());
    },
  });
  await new Promise((resolve, reject) =>
    server.bindAsync(`unix:${socket}`, grpc.ServerCredentials.createInsecure(), (e) => (e ? reject(e) : resolve())),
  );
  return {
    addr: `unix:${socket}`,
    seen,
    sessionOpened,
    request(id, path, body) {
      const answered = new Promise((resolve) => waiters.set(String(id), resolve));
      session.write({
        request: {
          id: String(id),
          method: "POST",
          path,
          query: "",
          headers: [{ name: "content-type", value: "application/json" }],
          body: Buffer.from(JSON.stringify(body)),
          principal: { userId: "u1", kind: "PERSON", displayName: "Sarah", roles: [], groups: [], scopes: [] },
        },
      });
      return answered.then((r) => ({ status: r.status, body: JSON.parse(r.body.toString()) }));
    },
    /** Delivers a run; resolves with its JobDone. */
    run(runId, jobName, payload, extra = {}) {
      const done = new Promise((resolve) => doneWaiters.set(runId, resolve));
      session.write({
        jobRun: {
          runId,
          job: jobName,
          attempt: 1,
          maxAttempts: 4,
          payload: typeof payload === "string" ? payload : JSON.stringify(payload),
          scheduledAtUnixMs: "1795000000000",
          timeoutMs: 60_000,
          requestId: "900",
          ...extra,
        },
      });
      return done;
    },
    abort(runId, reason) {
      session.write({ jobAbort: { runId, reason } });
    },
    close: () => new Promise((resolve) => server.tryShutdown(() => resolve())),
  };
}

const gate = () => {
  let open;
  const p = new Promise((resolve) => (open = resolve));
  return { p, open };
};

/** A plugin of jobs that tests drive: what each run saw goes to `runs`. */
function jobsPlugin(jobSlots) {
  const seen = [];
  const hold = gate();
  const note = entity({ columns: { text: t.text() } });
  const remindUnpaid = job({
    payload: { olderThanDays: t.int(), since: t.timestamp().optional() },
    async handler(ctx, p) {
      seen.push({ job: ctx.job, payload: p, caller: ctx.caller.kind });
      ctx.log.info("reminding", { days: p.olderThanDays });
      ctx.log.with({ order: "SO-1" }).warn("no mail");
    },
  });
  const retryable = job({ payload: {}, handler: () => Promise.reject(unavailable("try again")) });
  const doomed = job({ payload: {}, handler: () => Promise.reject(permanent("for good")) });
  const marked = job({ payload: {}, handler: () => Promise.reject(new RpcError(409, "busy", "marked").asPermanent()) });
  const missing = job({ payload: {}, handler: () => Promise.reject(notFound("gone")) });
  const crash = job({ payload: {}, handler: () => Promise.reject(new TypeError("x is not a function")) });
  const wordy = job({ payload: {}, handler: () => Promise.reject(unavailable("é".repeat(3000))) });
  const chatty = job({
    payload: {},
    handler(ctx) {
      for (let i = 0; i < 1500; i++) ctx.log.debug(`line ${i}`);
    },
  });
  const waits = job({
    payload: {},
    async handler(ctx) {
      seen.push({ waiting: true });
      await new Promise((_, reject) => ctx.signal.addEventListener("abort", () => reject(ctx.signal.reason)));
    },
  });
  const blocks = job({ payload: {}, handler: () => hold.p });
  const enqueuer = job({
    payload: {},
    async handler(ctx) {
      await ctx.jobs.enqueue(remindUnpaid, { olderThanDays: 1 });
    },
  });
  const enqueue = endpoint({
    input: t.message("EnqueueRequest", { days: t.int(), inTx: t.bool(), singleton: t.text() }),
    output: t.message("EnqueueReply", { runId: t.text(), existing: t.bool(), state: t.text(), cancelled: t.bool() }),
    async handler(ctx, req) {
      const options = req.singleton ? { singleton: req.singleton, delay: "1h" } : { at: new Date(1795000000000) };
      const run = req.inTx
        ? await ctx.transaction((tx) => tx.jobs.enqueue(remindUnpaid, { olderThanDays: req.days }, options))
        : await ctx.jobs.enqueue(remindUnpaid, { olderThanDays: req.days }, options);
      return { ...run, cancelled: await ctx.jobs.cancel(run) };
    },
  });
  const stray = job({ payload: {}, handler() {} });
  const bulky = job({ payload: { blob: t.text() }, handler() {} });
  const misuse = endpoint({
    input: t.message("MisuseRequest", { what: t.text() }),
    output: t.message("MisuseReply", {}),
    async handler(ctx, req) {
      if (req.what === "stray") await ctx.jobs.enqueue(stray, {});
      else await ctx.jobs.enqueue(bulky, { blob: "x".repeat(70_000) });
      return {};
    },
  });
  const p = plugin({
    id: "orders",
    version: "0.1.0",
    entities: { note },
    endpoints: { enqueue, misuse },
    jobs: { remindUnpaid, bulky, retryable, doomed, marked, missing, crash, wordy, chatty, waits, blocks, enqueuer },
    ...(jobSlots ? { jobSlots } : {}),
  });
  return { plugin: p, seen, hold };
}

async function serving(p, answer) {
  const fake = await fakePlatform(answer);
  const stop = new AbortController();
  const served = p.serve({ argv: ["node", "main.js"], addr: fake.addr, signals: false, stop: stop.signal, drainMs: 2000 });
  await fake.sessionOpened;
  return {
    fake,
    stop: async () => {
      stop.abort();
      await served;
      await fake.close();
    },
  };
}

const enqueued = (s) =>
  s.jobEnqueue ? { jobEnqueued: { runId: "r-1", existing: s.jobEnqueue.singleton !== "", state: "RUN_STATE_PENDING" } } : undefined;

test("Register announces the job pool: 16 by default, the declared size, 0 without jobs", async () => {
  for (const [slots, expected] of [[undefined, 16], [3, 3]]) {
    const { fake, stop } = await serving(jobsPlugin(slots).plugin);
    assert.equal(fake.seen.register.jobSlots, expected);
    await stop();
  }
});

test("enqueue and cancel on the wire, alone and in a declared transaction", async () => {
  const { plugin: p } = jobsPlugin();
  const { fake, stop } = await serving(p, (s) => {
    if (s.txBegin) return { txBegun: { tx: "7" } };
    if (s.txCommit) return { ok: {} };
    if (s.jobCancel) return { jobCancelled: { cancelled: true } };
    return enqueued(s);
  });
  const alone = await fake.request(1, "/orders.v1.Orders/Enqueue", { days: 7, inTx: false, singleton: "" });
  assert.equal(alone.status, 200);
  assert.deepEqual(alone.body, { runId: "r-1", existing: false, state: "pending", cancelled: true });
  const [e1, c1] = fake.seen.syscalls.filter((s) => s.jobEnqueue || s.jobCancel);
  assert.deepEqual(
    { ...e1.jobEnqueue },
    { job: "remind_unpaid", payload: '{"olderThanDays":7}', atUnixMs: "1795000000000", when: "atUnixMs", singleton: "", requestId: "1", tx: "0" },
  );
  assert.deepEqual({ ...c1.jobCancel }, { runId: "r-1", requestId: "1" });

  const inTx = await fake.request(2, "/orders.v1.Orders/Enqueue", { days: 3, inTx: true, singleton: "report:2026-11-20" });
  assert.deepEqual(inTx.body, { runId: "r-1", existing: true, state: "pending", cancelled: true });
  const e2 = fake.seen.syscalls.filter((s) => s.jobEnqueue)[1].jobEnqueue;
  assert.equal(e2.tx, "7");
  assert.equal(e2.delayMs, "3600000");
  assert.equal(e2.singleton, "report:2026-11-20");
  await stop();
});

test("enqueue refusals keep their codes; a job not in the plugin and a payload over 64 KiB are refused locally", async () => {
  const { plugin: p } = jobsPlugin();
  let refusal = "queue_full: 10000 runs are pending";
  const { fake, stop } = await serving(p, (s) => (s.jobEnqueue ? { error: refusal } : undefined));
  const cases = [
    ["queue_full: 10000 runs are pending", 429, "queue_full"],
    ["payload_too_large: over 64 KiB", 413, "payload_too_large"],
    ["invalid: at is more than a year ahead", 400, "invalid"],
    ["cancelled: the run was stopped", 503, "cancelled"],
  ];
  let id = 10;
  for (const [error, status, code] of cases) {
    refusal = error;
    const r = await fake.request(id++, "/orders.v1.Orders/Enqueue", { days: 1, inTx: false, singleton: "" });
    assert.deepEqual([r.status, r.body.code], [status, code], error);
  }
  const before = fake.seen.syscalls.length;
  const stray = await fake.request(id++, "/orders.v1.Orders/Misuse", { what: "stray" });
  assert.deepEqual([stray.status, stray.body.code], [400, "invalid"]);
  const big = await fake.request(id++, "/orders.v1.Orders/Misuse", { what: "big" });
  assert.deepEqual([big.status, big.body.code], [413, "payload_too_large"]);
  assert.equal(fake.seen.syscalls.length, before);
  await stop();
});

test("a run: its handler gets the payload, ctx.job and the run's principal, and the attempt is answered ok", async () => {
  const { plugin: p, seen } = jobsPlugin();
  const { fake, stop } = await serving(p);
  const done = await fake.run("run-1", "remind_unpaid", { olderThanDays: 7 }, { attempt: 4 });
  assert.equal(done.outcome, "ok");
  assert.equal(done.attempt, 4);
  const [{ job: info, payload, caller }] = seen;
  assert.deepEqual(payload, { olderThanDays: 7 });
  assert.equal(caller, "system");
  assert.deepEqual(
    { ...info, scheduledAt: info.scheduledAt.getTime() },
    { job: "remind_unpaid", runId: "run-1", attempt: 4, maxAttempts: 4, scheduledAt: 1795000000000, isRun: true, isLastAttempt: true },
  );
  // what the handler logged, with the run's request id
  assert.deepEqual(
    fake.seen.logs.map((l) => [l.requestId, l.level, l.message]),
    [
      ["900", "INFO", "reminding days=7"],
      ["900", "WARN", 'no mail order="SO-1"'],
    ],
  );
  assert.ok(Number(fake.seen.logs[0].atUnixMs) > 0);
  await stop();
});

test("failed attempts: retryable, permanent, marked permanent, input-class, a thrown non-RpcError, payload_invalid", async () => {
  const { plugin: p } = jobsPlugin();
  const { fake, stop } = await serving(p);
  const err = async (job, payload = {}) => {
    const d = await fake.run(`run-${job}-${Math.random()}`, job, payload);
    assert.equal(d.outcome, "error", job);
    return { ...d.error };
  };
  assert.deepEqual(await err("retryable"), { code: "unavailable", message: "try again", permanent: false });
  assert.deepEqual(await err("doomed"), { code: "permanent", message: "for good", permanent: true });
  assert.deepEqual(await err("marked"), { code: "busy", message: "marked", permanent: true });
  assert.deepEqual(await err("missing"), { code: "not_found", message: "gone", permanent: true });
  assert.deepEqual(await err("crash"), { code: "internal", message: "x is not a function", permanent: false });
  const wordy = await err("wordy");
  assert.ok(Buffer.byteLength(wordy.message) <= 2048 && Buffer.byteLength(wordy.message) >= 2047);
  const bad = await err("remind_unpaid", { olderThanDays: "seven" });
  assert.equal(bad.code, "payload_invalid");
  assert.equal(bad.permanent, true);
  assert.match(bad.message, /RemindUnpaid/);
  const notJson = await err("remind_unpaid", "{oops");
  assert.equal(notJson.code, "payload_invalid");
  await stop();
});

test("an abort stops the handler and answers its code at once; an unknown reason is a cancel", async () => {
  const { plugin: p } = jobsPlugin();
  const { fake, stop } = await serving(p);
  const reasons = { TIMEOUT: "timeout", CANCELLED: "cancelled", SHUTDOWN: "shutdown", LEASE_LOST: "lease_lost", AUTH_CHANGED: "auth_changed", REMOVED: "job_removed", REASON_UNSPECIFIED: "cancelled" };
  for (const [reason, code] of Object.entries(reasons)) {
    const runId = `abort-${reason}`;
    const done = fake.run(runId, "waits", {});
    await sleep(20);
    fake.abort(runId, reason);
    const d = await done;
    assert.equal(d.outcome, "error");
    assert.deepEqual({ ...d.error }, { code, message: `stopped: ${code}`, permanent: false }, reason);
  }
  await stop();
});

test("a run of a job the version does not have, or past the free slots, is refused as unavailable", async () => {
  const { plugin: p, hold } = jobsPlugin(1);
  const { fake, stop } = await serving(p);
  const unknown = await fake.run("u-1", "nope", {});
  assert.deepEqual({ ...unknown.error }, { code: "unavailable", message: "this version has no job nope", permanent: false });
  const first = fake.run("b-1", "blocks", {});
  await sleep(20);
  const second = await fake.run("b-2", "blocks", {});
  assert.deepEqual({ ...second.error }, { code: "unavailable", message: "all 1 job slots are taken", permanent: false });
  hold.open();
  assert.equal((await first).outcome, "ok");
  await stop();
});

test("a handler's enqueue carries the run's request id", async () => {
  const { plugin: p } = jobsPlugin();
  const { fake, stop } = await serving(p, enqueued);
  assert.equal((await fake.run("e-1", "enqueuer", {})).outcome, "ok");
  assert.equal(fake.seen.syscalls[0].jobEnqueue.requestId, "900");
  await stop();
});

test("a run sends at most 1,000 log lines a second", async () => {
  const { plugin: p } = jobsPlugin();
  const { fake, stop } = await serving(p);
  await sleep(1100); // the earlier tests' lines are in another window
  assert.equal((await fake.run("c-1", "chatty", {})).outcome, "ok");
  await sleep(50);
  assert.equal(fake.seen.logs.length, LINES_PER_SECOND);
  assert.deepEqual(fake.seen.logs.at(-1).message, `line ${LINES_PER_SECOND - 1}`);
  await stop();
});

test("request lines are not sent as run logs", async () => {
  const quiet = endpoint({
    input: t.message("QuietRequest", {}),
    output: t.message("QuietReply", {}),
    handler(ctx) {
      ctx.log.info("a request's line");
      return {};
    },
  });
  const p = plugin({ id: "orders", version: "0.1.0", endpoints: { quiet }, jobs: { tick: job({ payload: {}, handler() {} }) } });
  const { fake, stop } = await serving(p);
  assert.equal((await fake.request(1, "/orders.v1.Orders/Quiet", {})).status, 200);
  await sleep(20);
  assert.deepEqual(fake.seen.logs, []);
  await stop();
});

test("a drain waits for the runs in flight and refuses new ones as shutdown", async () => {
  const { plugin: p, hold } = jobsPlugin();
  const fake = await fakePlatform();
  const stopper = new AbortController();
  const served = p.serve({ argv: ["node", "main.js"], addr: fake.addr, signals: false, stop: stopper.signal, drainMs: 5000 });
  await fake.sessionOpened;
  const running = fake.run("d-1", "blocks", {});
  await sleep(20);
  stopper.abort();
  let finished = false;
  void served.then(() => (finished = true));
  const refused = await fake.run("d-2", "remind_unpaid", { olderThanDays: 1 });
  assert.deepEqual({ ...refused.error }, { code: "shutdown", message: "the plugin is stopping", permanent: false });
  await sleep(50);
  assert.equal(finished, false);
  hold.open();
  assert.equal((await running).outcome, "ok");
  await served;
  await fake.close();
});
