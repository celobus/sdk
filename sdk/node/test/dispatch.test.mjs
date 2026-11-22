// Request dispatch and error mapping, against an in-memory session.
import assert from "node:assert/strict";
import { test } from "node:test";
import { conflict, domainError, endpoint, entity, forbidden, invalid, notFound, plugin, t, unauthenticated, unavailable } from "../dist/index.js";
import { dispatch } from "../dist/serve.js";
import { SyscallError } from "../dist/ctx.js";
import { sink } from "../dist/log.js";
import { createLogger } from "../dist/log.js";

const logged = [];
sink.write = (line) => logged.push(JSON.parse(line));

const notCancellable = domainError("not_cancellable", 409, "The order is no longer placed.");
const request = t.message("CancelRequest", { id: t.text(), qty: t.int() });
const record = entity({ key: "i64", columns: { title: t.text() } });

const p = plugin({
  id: "orders",
  version: "1.0.0",
  dependencies: ["customers"],
  entities: { record },
  endpoints: {
    cancel: endpoint({
      input: request,
      output: { ok: t.bool() },
      timeout: 200,
      async handler(ctx, input) {
        switch (input.id) {
          case "invalid": throw invalid("bad lines").field(request.fields.qty, "too many").field("lines[0]", "no product");
          case "unauthenticated": ctx.caller.user(); break;
          case "forbidden": throw forbidden();
          case "missing": throw notFound("no order 7");
          case "conflict": throw conflict("busy");
          case "domain": throw notCancellable.error("order 7 is shipped");
          case "unavailable": throw unavailable(new Error("upstream down"));
          case "crash": throw new Error("secret detail");
          case "db": return ctx.db(record).get("1");
          case "slow": await new Promise((r) => setTimeout(r, 1000)); break;
          case "badOutput": return { ok: "yes" };
          case "caller": return { ok: ctx.caller.has("clerk") && ctx.caller.has("workspace.member") && ctx.caller.hasScope("read") && ctx.caller.kind === "user" };
          case "call": return { ok: (await ctx.call("customers", "GetCustomer", { id: 5n })).found };
        }
        return { ok: true };
      },
    }),
  },
});

const model = { id: "orders", version: "1.0.0", depends: ["customers"], resolved: p.model(), log: createLogger({ plugin: "orders" }) };
const routes = new Map(model.resolved.endpoints.map((ep) => [`/orders.v1.Orders/${ep.name}`, ep]));
const syscalls = [];
const session = {
  async syscall(call) {
    syscalls.push(call);
    if (call.dbGet) throw new SyscallError("not_found: no record 1");
    if (call.invoke) {
      return { id: "1", error: "", invokeResponse: { status: 200, headers: [], body: Buffer.from('{"found":true}') } };
    }
    throw new Error("unexpected syscall");
  },
};

const principal = { userId: "u1", kind: "PERSON", displayName: "Sarah", roles: ["orders.clerk", "workspace.member"], groups: [], scopes: ["orders:read"] };

async function call(body, { path = "/orders.v1.Orders/Cancel", method = "POST", contentType = "application/json", who = principal } = {}) {
  const headers = contentType ? [{ name: "Content-Type", value: contentType }] : [];
  const resp = await dispatch(model, routes, session, {
    id: "42", method, path, query: "", headers, body: Buffer.from(typeof body === "string" ? body : JSON.stringify(body)), principal: who,
  });
  return { status: resp.status, body: JSON.parse(resp.body.toString()), headers: resp.headers };
}

test("a request is decoded, handled and answered as JSON", async () => {
  const r = await call({ id: "ok", qty: "3" });
  assert.equal(r.status, 200);
  assert.deepEqual(r.body, { ok: true });
  assert.deepEqual(r.headers, [{ name: "content-type", value: "application/json" }]);
  assert.equal((await call({ id: "caller" })).body.ok, true);
  assert.equal((await call("", { contentType: "" })).status, 200); // no body, no content type
});

test("errors map to statuses and the platform error body", async () => {
  const cases = [
    ["invalid", 400, { code: "invalid", message: "bad lines", details: { fields: { qty: "too many", "lines[0]": "no product" } } }],
    ["forbidden", 403, { code: "forbidden", message: "forbidden", details: {} }],
    ["missing", 404, { code: "not_found", message: "no order 7", details: {} }],
    ["conflict", 409, { code: "conflict", message: "busy", details: {} }],
    ["domain", 409, { code: "not_cancellable", message: "order 7 is shipped", details: {} }],
    ["unavailable", 503, { code: "unavailable", message: "upstream down", details: {} }],
    ["crash", 500, { code: "internal", message: "internal error", details: {} }],
    ["db", 404, { code: "not_found", message: "no record 1", details: {} }],
    ["slow", 504, { code: "deadline_exceeded", message: "Cancel did not answer within 200 ms", details: {} }],
    ["badOutput", 500, { code: "internal", message: "internal error", details: {} }],
  ];
  for (const [id, status, body] of cases) {
    const r = await call({ id });
    assert.equal(r.status, status, id);
    assert.deepEqual(r.body, body, id);
  }
  assert.equal((await call({ id: "unauthenticated" }, { who: null })).status, 401);
  // the hidden message of an internal error is in the log, with the request
  const line = logged.find((l) => l.error?.message === "secret detail");
  assert.equal(line.level, "error");
  assert.equal(line.endpoint, "Cancel");
  assert.equal(line.request_id, "42");
  assert.ok(logged.some((l) => /output does not fit/.test(l.msg) && /ok/.test(l.error.message)));
});

test("transport errors: unknown endpoint, method, media type, malformed input", async () => {
  assert.deepEqual((await call({}, { path: "/orders.v1.Orders/Nope" })).body.code, "unknown_endpoint");
  assert.equal((await call({}, { path: "/orders.v1.Orders/Nope" })).status, 404);
  assert.equal((await call({}, { method: "GET" })).status, 405);
  const proto = await call({}, { contentType: "application/x-protobuf" });
  assert.equal(proto.status, 415);
  assert.equal(proto.body.code, "unsupported_media_type");
  const notJson = await call("{oops");
  assert.equal(notJson.status, 400);
  assert.equal(notJson.body.code, "invalid");
  const wrong = await call({ id: 1, qty: 1.5 });
  assert.equal(wrong.status, 400);
  assert.deepEqual(wrong.body.details.fields, { id: "expected a string", qty: "expected a 32-bit integer" });
});

test("ctx.call invokes the dependency with the current request id and raw JSON", async () => {
  syscalls.length = 0;
  const r = await call({ id: "call" });
  assert.deepEqual(r.body, { ok: true });
  const { invoke } = syscalls[0];
  assert.equal(invoke.plugin, "customers");
  assert.equal(invoke.requestId, "42");
  assert.equal(invoke.request.method, "POST");
  assert.equal(invoke.request.path, "/customers.v1.Customers/GetCustomer");
  assert.equal(invoke.request.body.toString(), '{"id":"5"}');
});

// The run-change hook (crates/sdk/src/session_tests.rs, the_run_change_hook_is_a_system_request): a request of the
// platform, with no caller; one from a caller is refused as unknown.
test("the run-change hook is a system request", async () => {
  const seen = [];
  const hooked = plugin({ id: "echo", version: "1.0.0" }).onRunChange(async (ctx, change) => {
    assert.equal(ctx.caller.kind, "system");
    if (change.turn === 9) throw new Error("try again");
    seen.push(change);
  });
  assert.deepEqual(hooked.describe().runs, { on_change: true });
  assert.equal(plugin({ id: "echo", version: "1.0.0" }).describe().runs, undefined);
  assert.deepEqual(plugin({ id: "echo", version: "1.0.0", onRunChange: () => {} }).describe().runs, { on_change: true });
  const m = { id: "echo", version: "1.0.0", depends: [], resolved: hooked.model(), log: createLogger({ plugin: "echo" }) };
  const hookRoutes = new Map(m.resolved.endpoints.map((ep) => [`/echo.v1.Echo/${ep.name}`, ep]));
  const path = "/echo.v1.Echo/_RunChange";
  assert.ok(hookRoutes.has(path));
  const change = {
    run_id: "run-1", agent: "assistant", plugin: "crm", status: "idle", turn: 2, last_seq: 14, topic: "conversation",
    topic_key: '{"channel":"c-1"}', ended: false,
  };
  const send = async (body, who) => {
    const resp = await dispatch(m, hookRoutes, session, {
      id: "7", method: "POST", path, query: "", headers: [{ name: "content-type", value: "application/json" }],
      body: Buffer.from(JSON.stringify(body)), principal: who,
    });
    return { status: resp.status, body: JSON.parse(resp.body.toString()) };
  };

  // from a member (from outside): no such endpoint
  const fromMember = await send(change, principal);
  assert.deepEqual([fromMember.status, fromMember.body.code], [404, "unknown_endpoint"]);
  assert.equal(seen.length, 0);

  assert.deepEqual(await send(change, null), { status: 200, body: {} });
  assert.deepEqual(seen, [{
    runId: "run-1", agent: "assistant", plugin: "crm", status: "idle", turn: 2, lastSeq: 14, topic: "conversation",
    topicKey: '{"channel":"c-1"}', ended: false,
  }]);
  // an error answers non-2xx: the platform calls again
  assert.equal((await send({ ...change, turn: 9 }, null)).status, 500);
  // no run id: invalid
  assert.equal((await send({}, undefined)).status, 400);
  // `ended` defaults to the status; `queued` is starting; a status this SDK does not know is `unknown`
  await send({ run_id: "run-2", status: "succeeded" }, null);
  await send({ run_id: "run-3", status: "queued" }, null);
  await send({ run_id: "run-4", status: "paused" }, null);
  assert.deepEqual(seen.slice(1).map((c) => [c.runId, c.status, c.ended]), [
    ["run-2", "succeeded", true], ["run-3", "starting", false], ["run-4", "unknown", false],
  ]);
});
