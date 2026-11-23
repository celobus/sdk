// testCtx: a context without the platform (crates/sdk/src/ctx.rs, Ctx::test, with_caller, with_run).
import assert from "node:assert/strict";
import { test } from "node:test";
import { endpoint, entity, job, plugin, role, t, testCtx } from "../dist/index.js";

const sales = role({ title: "Sales" });
const deal = entity({ key: "i64", columns: { title: t.text() } });
const remind = job({ payload: { days: t.int() }, async handler() {} });
const crm = plugin({
  id: "crm",
  version: "0.1.0",
  roles: { sales },
  entities: { deal },
  jobs: { remind },
  endpoints: {
    whoAmI: endpoint({
      input: {},
      output: { id: t.text(), sales: t.bool() },
      async handler(ctx) {
        return { id: ctx.caller.user(), sales: ctx.caller.has(sales) };
      },
    }),
  },
});

const unavailable = { status: 503, code: "unavailable" };

test("by default the caller is the system, the request id 0 and the deadline 30 s away", () => {
  const ctx = testCtx(crm);
  assert.equal(ctx.caller.kind, "system");
  assert.equal(ctx.caller.userId, undefined);
  assert.throws(() => ctx.caller.user(), { status: 401, code: "unauthenticated" });
  assert.deepEqual({ ...ctx.plugin }, { id: "crm", version: "0.1.0" });
  assert.equal(ctx.request.id, "0");
  const left = ctx.request.deadline.getTime() - Date.now();
  assert.ok(left > 29_000 && left <= 30_000, `${left} ms left`);
  assert.equal(ctx.signal.aborted, false);
  assert.equal(ctx.idempotencyKey, undefined);
  assert.equal(ctx.job.isRun, false);
  assert.equal(ctx.job.runId, "");
});

test("a test caller has the roles, groups and scopes it is given, as full names", async () => {
  const ctx = testCtx(crm, { caller: { userId: "u1", roles: [sales, "workspace.member"], groups: ["g1"], scopes: ["read"] } });
  assert.equal(ctx.caller.kind, "user");
  assert.equal(ctx.caller.displayName, "Test User");
  assert.deepEqual(ctx.caller.roles, ["crm.sales", "workspace.member"]);
  assert.equal(ctx.caller.has(sales), true);
  assert.equal(ctx.caller.has("sales"), true);
  assert.equal(ctx.caller.has("workspace.admin"), false);
  assert.equal(ctx.caller.inGroup("g1"), true);
  assert.equal(ctx.caller.hasScope("crm:read"), true);
  assert.equal(testCtx(crm, { caller: { userId: "u1", roles: ["sales"] } }).caller.has(sales), true);
  assert.equal(testCtx(crm, { caller: { userId: "bot", kind: "service", displayName: "Bot" } }).caller.kind, "service");
  assert.throws(() => testCtx(crm, { caller: { userId: "u1", roles: [role({ title: "Other" })] } }), /not in plugin/);

  const handler = crm.options.endpoints.whoAmI.options.handler;
  assert.deepEqual(await handler(ctx, {}), { id: "u1", sales: true });
  await assert.rejects(handler(testCtx(crm), {}), { status: 401 });
});

test("a test run is the run a job's handler is in, scheduled now", () => {
  const ctx = testCtx(crm, { run: { runId: "run-1", attempt: 1, maxAttempts: 4 } });
  assert.equal(ctx.job.isRun, true);
  assert.equal(ctx.job.runId, "run-1");
  assert.equal(ctx.job.attempt, 1);
  assert.equal(ctx.job.maxAttempts, 4);
  assert.equal(ctx.job.job, "");
  assert.equal(ctx.job.isLastAttempt, false);
  assert.ok(Math.abs(ctx.job.scheduledAt.getTime() - Date.now()) < 1_000);
  assert.equal(testCtx(crm, { run: { runId: "run-1", attempt: 4, maxAttempts: 4 } }).job.isLastAttempt, true);
  assert.equal(ctx.caller.kind, "system");
});

test("the options set the idempotency key, the deadline and the signal", () => {
  const stop = new AbortController();
  const ctx = testCtx(crm, { idempotencyKey: "k1", timeoutMs: 1_000, signal: stop.signal });
  assert.equal(ctx.idempotencyKey, "k1");
  assert.equal(ctx.request.idempotencyKey, "k1");
  assert.ok(ctx.request.deadline.getTime() - Date.now() <= 1_000);
  stop.abort();
  assert.equal(ctx.signal.aborted, true);
  assert.equal(ctx.asSystem("test").idempotencyKey, "k1");
});

test("every call to the platform answers 503 unavailable", async () => {
  const ctx = testCtx(crm, { caller: { userId: "u1" } });
  await assert.rejects(ctx.db(deal).get(1n), unavailable);
  await assert.rejects(ctx.db(deal).create({ title: "A" }), unavailable);
  await assert.rejects(ctx.query(deal).all(), unavailable);
  await assert.rejects(ctx.sql("select 1"), unavailable);
  await assert.rejects(ctx.transaction(async () => 1), unavailable);
  await assert.rejects(ctx.call("catalog", "GetProduct", {}), unavailable);
  await assert.rejects(ctx.members.list(), unavailable);
  await assert.rejects(ctx.members.get("u2"), unavailable);
  await assert.rejects(ctx.groups.list(), unavailable);
  await assert.rejects(ctx.agents.list(), unavailable);
  await assert.rejects(ctx.jobs.enqueue(remind, { days: 7 }), unavailable);
  await assert.rejects(ctx.jobs.cancel("run-1"), unavailable);
  await assert.rejects(ctx.notifications.following("k"), unavailable);
  ctx.invalidateAll({ name: "deals" });
});
