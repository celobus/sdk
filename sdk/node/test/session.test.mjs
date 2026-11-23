// The plugin session against a fake platform: a server on a temporary address implementing Celld.Session.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import * as grpc from "@grpc/grpc-js";
import { endpoint, plugin, t } from "../dist/index.js";
import { celldDefinition } from "../dist/serve.js";
import { sink } from "../dist/log.js";
import hello from "./fixtures/hello.mjs";

sink.write = () => {};

/**
 * A fake platform: records what the plugin sends, answers the hello plugin's greetings (its transaction, the create and
 * the count of `greeting`) from a list and the others with `answer` (a syscall → the members of its `SyscallResult`).
 */
async function fakePlatform(socket = join(mkdtempSync(join(tmpdir(), "celobus-")), "plugin.sock"), answer = () => undefined) {
  rmSync(socket, { force: true });
  const greetings = [];
  const seen = { register: undefined, syscalls: [], responses: new Map(), invalidations: [] };
  let session;
  let onSession;
  const sessionOpened = new Promise((resolve) => (onSession = resolve));
  const waiters = new Map();

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
          const result = { id: s.id, error: "" };
          if (s.txBegin) result.txBegun = { tx: "1" };
          else if (s.txCommit) result.ok = {};
          else if (s.dbCreate?.entity === "greeting") {
            const row = JSON.parse(s.dbCreate.rows[0].toString());
            greetings.push(row.name);
            const stored = { ...row, id: String(greetings.length), createdAt: "2026-11-12T10:00:00Z", updatedAt: "2026-11-12T10:00:00Z" };
            result.dbRows = { rows: [Buffer.from(JSON.stringify(stored))], nextPageToken: "" };
          } else if (s.dbCount?.entity === "greeting") {
            const name = s.dbCount.filter.condition.values[0].stringValue;
            result.dbCount = { count: String(greetings.filter((g) => g === name).length) };
          } else Object.assign(result, answer(s) ?? { error: "unsupported" });
          call.write({ syscallResult: result });
        } else if (msg.kind === "invalidate") {
          seen.invalidations.push({ ...msg.invalidate });
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
    socket,
    addr: `unix:${socket}`,
    greetings,
    seen,
    sessionOpened,
    /** Sends a request and resolves with the plugin's response. */
    request(id, path, body, principal = { userId: "u1", kind: "PERSON", displayName: "Sarah", roles: ["hello.clerk"], groups: [], scopes: ["hello:write"] }) {
      const answered = new Promise((resolve) => waiters.set(String(id), resolve));
      session.write({
        request: {
          id: String(id),
          method: "POST",
          path,
          query: "",
          headers: [{ name: "content-type", value: "application/json" }],
          body: Buffer.from(JSON.stringify(body)),
          principal,
        },
      });
      return answered;
    },
    close: () => new Promise((resolve) => server.tryShutdown(() => resolve())),
    kill: () => server.forceShutdown(),
  };
}

test("registers, serves requests and round-trips db syscalls over the session", async () => {
  const fake = await fakePlatform();
  const stop = new AbortController();
  const serving = hello.serve({ argv: ["node", "main.js"], addr: fake.addr, signals: false, stop: stop.signal });
  await fake.sessionOpened;

  assert.deepEqual(
    { ...fake.seen.register },
    { plugin: "hello", version: "0.1.0", runtime: "node", depends: ["greeter"], manifestDigest: "", token: "", jobSlots: 0 },
  );

  const first = await fake.request(7, "/hello.v1.Hello/Hello", { name: "Sarah" });
  assert.equal(first.status, 200);
  assert.deepEqual(JSON.parse(first.body.toString()), { text: "Hello, Sarah!", count: "1" });
  assert.deepEqual(fake.greetings, ["Sarah"]);
  assert.deepEqual(
    fake.seen.syscalls.map((s) => s.call),
    ["txBegin", "dbCreate", "dbCount", "txCommit"],
  );

  const second = await fake.request(8, "/hello.v1.Hello/Hello", { name: "Sarah", mood: "sad" });
  assert.deepEqual(JSON.parse(second.body.toString()), { text: "Cheer up, Sarah.", count: "2" });

  const note = await fake.request(9, "/hello.v1.Hello/GetNote", { id: "1" });
  assert.equal(note.status, 503);
  assert.equal(JSON.parse(note.body.toString()).code, "unavailable");

  const unknown = await fake.request(10, "/hello.v1.Hello/Nope", {});
  assert.equal(unknown.status, 404);

  stop.abort();
  await serving;
  await fake.close();
});

test("reconnects and registers again when the platform comes back", async () => {
  const first = await fakePlatform();
  const stop = new AbortController();
  const serving = hello.serve({ argv: ["node", "main.js"], addr: first.addr, signals: false, stop: stop.signal });
  await first.sessionOpened;
  first.kill();

  const second = await fakePlatform(first.socket);
  await second.sessionOpened;
  assert.equal(second.seen.register.plugin, "hello");
  const r = await second.request(1, "/hello.v1.Hello/Hello", { name: "Ann" });
  assert.deepEqual(JSON.parse(r.body.toString()), { text: "Hello, Ann!", count: "1" });

  stop.abort();
  await serving;
  await second.close();
});

test("ctx.invalidate sends an Invalidate on the session with the canonical key", async () => {
  const { endpoint, plugin, t, topic } = await import("../dist/index.js");
  const lineKey = t.message("LineKey", { orderId: t.text(), line: t.bigint() });
  const all = t.message("AllLines", {});
  const LINE = topic("line", lineKey);
  const LINES = topic("lines", all);
  const touch = endpoint({
    input: lineKey,
    output: all,
    handler(ctx, key) {
      ctx.invalidate(LINE, key);
      ctx.invalidateAll(LINES);
      return {};
    },
  });
  const p = plugin({ id: "lines", version: "0.1.0", topics: [LINE, LINES], endpoints: { touch } });
  const fake = await fakePlatform();
  const stop = new AbortController();
  const serving = p.serve({ argv: ["node", "main.js"], addr: fake.addr, signals: false, stop: stop.signal });
  await fake.sessionOpened;

  const r = await fake.request(1, "/lines.v1.Lines/Touch", { orderId: "1048", line: "2" });
  assert.equal(r.status, 200);
  assert.deepEqual(fake.seen.invalidations, [
    { topic: "line", key: '{"line":"2","order_id":"1048"}' },
    { topic: "lines", key: "" },
  ]);

  stop.abort();
  await serving;
  await fake.close();
});

test("canonical keys are snake_case, sorted, values as strings", async () => {
  const { canonicalKey } = await import("../dist/ctx.js");
  assert.equal(canonicalKey({ orderId: "1048", line: 2, open: true }), '{"line":"2","open":"true","order_id":"1048"}');
  assert.equal(canonicalKey({ id: "7", nested: { a: "1" }, tags: ["x"], gone: null }), '{"id":"7"}');
  assert.equal(canonicalKey({}), "");
});

test("admission refusals of a syscall are 403 with their code", async () => {
  const { SyscallError, syscallFailed } = await import("../dist/ctx.js");
  const e = syscallFailed(new SyscallError("call_undeclared: the calling plugin does not declare /c.v1.C/D among its calls"));
  assert.equal(e.status, 403);
  assert.equal(e.code, "call_undeclared");
  const other = syscallFailed(new SyscallError("membership_stale: the member table is not current"));
  assert.equal(other.status, 503);
});

test("ctx.agents starts, follows and stops the runs of the plugin's agents over the session", async () => {
  const { endpoint, plugin, t } = await import("../dist/index.js");
  const wireRun = (id, status, extra = {}) => ({
    id, agent: "assistant", plugin: "desk", engine: "claude", status, input: "what does tea cost?",
    startedBy: { userId: "u1", displayName: "Sarah" }, startedByPlugin: "desk",
    createdAt: "1000", updatedAt: "2000", deadlineAt: "1801000", turn: 1, lastSeq: "3", pod: "running", group: id, ...extra,
  });
  const event = (seq, fields) => ({ seq, at: 1500, type: fields.type, json: Buffer.from(JSON.stringify({ seq, at: 1500, ...fields })) });
  const answer = (s) => {
    if (s.agentRun) {
      if (s.agentRun.agent !== "assistant") return { error: `unknown_agent: the plugin desk declares no agent ${s.agentRun.agent}` };
      if (s.agentRun.requestId === "0") return { error: "forbidden: an agent run is started while the plugin handles a request" };
      return { agentRun: wireRun("run-1", "starting") };
    }
    if (s.agentGet) {
      if (s.agentGet.runId !== "run-1") return { error: "not_found: no such run" };
      return { agentRun: wireRun("run-1", "succeeded", { result: "Tea is $4.", model: "claude-opus-5-5" }) };
    }
    if (s.agentEvents) {
      return {
        agentEvents: {
          run: wireRun(s.agentEvents.runId, "running"),
          events: [
            event(2, { type: "tool", name: "shell", phase: "completed", summary: "celobus whoami" }),
            event(3, { type: "message", role: "assistant", text: "Let me look." }),
          ],
          partial: { rev: 4, messages: [{ messageId: "m1", text: "Tea" }, { messageId: "m2", text: "is $4" }] },
        },
      };
    }
    if (s.agentRuns) return { agentRuns: { runs: [wireRun("run-2", "running"), wireRun("run-1", "failed", { error: { code: "lost", message: "the agent stopped answering" } })] } };
    if (s.agentSend) return s.agentSend.text === "late" ? { error: "run_ended: the run has ended" } : { agentInput: { seq: 2 } };
    if (s.agentCancel) return { agentRun: wireRun(s.agentCancel.runId, "cancelled") };
    if (s.agentClose) return { agentRun: wireRun(s.agentClose.runId, "idle") };
    return undefined;
  };

  const out = {};
  const ask = endpoint({
    input: { question: t.text() },
    output: { runId: t.text() },
    async handler(ctx, { question }) {
      const a = ctx.agents;
      out.run = await a.run("assistant", question);
      await a.run("assistant", question, { engine: "pi", model: "claude-opus-5-5", session: false, timeout: "10m" });
      out.done = await a.get("run-1");
      out.page = await a.events("run-1", { after: 1, limit: 50 });
      out.runs = await a.list({ agent: "assistant", limit: 10 });
      await a.list();
      out.seq = await a.send("run-1", "and coffee?");
      out.cancelled = await a.cancel("run-1");
      out.closed = await a.close("run-1");
      out.errors = [];
      for (const failing of [() => a.run("nobody", "x"), () => a.get("run-9"), () => a.send("run-1", "late")]) {
        out.errors.push(await failing().then(() => "answered", (e) => `${e.status} ${e.code}: ${e.message}`));
      }
      return { runId: out.run.id };
    },
  });
  const p = plugin({ id: "desk", version: "0.1.0", agents: { assistant: {} }, endpoints: { ask } });
  const fake = await fakePlatform(undefined, answer);
  const stop = new AbortController();
  const serving = p.serve({ argv: ["node", "main.js"], addr: fake.addr, signals: false, stop: stop.signal });
  await fake.sessionOpened;

  const r = await fake.request(77, "/desk.v1.Desk/Ask", { question: "what does tea cost?" });
  assert.equal(r.status, 200, r.body.toString());
  assert.deepEqual(JSON.parse(r.body.toString()), { runId: "run-1" });

  // what the plugin's code got: typed runs, events with the fields of their type, the unfinished text
  assert.deepEqual(out.run, {
    id: "run-1", agent: "assistant", plugin: "desk", engine: "claude", status: "starting", ended: false, session: false,
    input: "what does tea cost?", startedBy: { userId: "u1", displayName: "Sarah" }, startedByPlugin: "desk",
    createdAt: 1000, updatedAt: 2000, deadlineAt: 1801000, turn: 1, lastSeq: 3, pod: "running",
    // it founded its group; the platform named nobody at the root, so that is who started it
    group: "run-1", onBehalfOf: { userId: "u1", displayName: "Sarah" },
  });
  assert.deepEqual(
    [out.done.status, out.done.ended, out.done.result, out.done.model, out.done.error],
    ["succeeded", true, "Tea is $4.", "claude-opus-5-5", undefined],
  );
  assert.equal(out.page.run.status, "running");
  assert.deepEqual(out.page.events, [
    { seq: 2, at: 1500, type: "tool", name: "shell", phase: "completed", summary: "celobus whoami" },
    { seq: 3, at: 1500, type: "message", role: "assistant", text: "Let me look." },
  ]);
  assert.deepEqual(out.page.partial, {
    rev: 4,
    messages: [{ messageId: "m1", text: "Tea" }, { messageId: "m2", text: "is $4" }],
    text: "Tea\n\nis $4",
  });
  assert.deepEqual(out.runs.map((x) => [x.id, x.status, x.error?.code]), [["run-2", "running", undefined], ["run-1", "failed", "lost"]]);
  assert.equal(out.seq, 2);
  assert.deepEqual([out.cancelled.status, out.cancelled.ended, out.closed.status], ["cancelled", true, "idle"]);
  assert.deepEqual(out.errors, [
    "404 unknown_agent: the plugin desk declares no agent nobody",
    "404 not_found: no such run",
    "409 run_ended: the run has ended",
  ]);

  // what went to the platform: the id of the request being handled with what acts for its caller
  const sent = fake.seen.syscalls;
  assert.deepEqual(sent.map((s) => s.call), [
    "agentRun", "agentRun", "agentGet", "agentEvents", "agentRuns", "agentRuns", "agentSend", "agentCancel", "agentClose",
    "agentRun", "agentGet", "agentSend",
  ]);
  const { _session: unsaid, ...first } = sent[0].agentRun;
  assert.equal(unsaid, undefined, "a start that does not say `session` leaves it to the agent's declaration");
  assert.deepEqual(first, { agent: "assistant", input: "what does tea cost?", engine: "", model: "", timeoutS: 0, requestId: "77", group: "", notify: false, notifyLink: null, topic: "", topicKey: "" });
  const { _session: said, ...second } = sent[1].agentRun;
  assert.equal(said, "session");
  assert.deepEqual(second, { agent: "assistant", input: "what does tea cost?", engine: "pi", model: "claude-opus-5-5", session: false, timeoutS: 600, requestId: "77", group: "", notify: false, notifyLink: null, topic: "", topicKey: "" });
  assert.deepEqual({ ...sent[3].agentEvents }, { runId: "run-1", after: "1", limit: 50 });
  assert.deepEqual({ ...sent[4].agentRuns }, { agent: "assistant", limit: 10, group: "" });
  assert.deepEqual({ ...sent[5].agentRuns }, { agent: "", limit: 0, group: "" });
  assert.deepEqual({ ...sent[6].agentSend }, { runId: "run-1", text: "and coffee?", requestId: "77" });

  stop.abort();
  await serving;
  await fake.close();
});

test("ctx.agents runs another plugin's agent on a topic of its own and lists the agents the caller may run", async () => {
  const { endpoint, plugin, t, topic } = await import("../dist/index.js");
  const CONVERSATION = topic("conversation", t.message("Conversation", { id: t.text() }));
  const answer = (s) => {
    if (s.agentRun) {
      return {
        agentRun: {
          id: "run-1", agent: "assistant", plugin: "crm", engine: "claude", status: "starting", input: s.agentRun.input,
          startedBy: { userId: "u1", displayName: "Sarah" }, startedByPlugin: "chat", createdAt: "1", updatedAt: "1",
          deadlineAt: "2", turn: 0, lastSeq: "1", pod: "requested", group: "run-1",
        },
      };
    }
    if (s.agentsAvailable) {
      return {
        agentsAvailable: {
          agents: [
            { agent: "chat.notes", plugin: "chat", name: "notes", title: "notes", description: "", icon: "", engine: "", session: false, pluginTitle: "Chat" },
            {
              agent: "crm.assistant", plugin: "crm", name: "assistant", title: "CRM assistant",
              description: "Answers about deals.", icon: "wand-sparkles", engine: "claude", session: true, pluginTitle: "CRM assistant",
            },
          ],
        },
      };
    }
    return undefined;
  };
  const out = {};
  const mention = endpoint({
    input: {},
    output: {},
    async handler(ctx) {
      out.agents = await ctx.agents.available();
      out.run = await ctx.agents.run("crm.assistant", "make a plugin", { topic: CONVERSATION, topicKey: { id: "c-7" }, session: true });
      return {};
    },
  });
  const p = plugin({ id: "chat", version: "0.1.0", topics: [CONVERSATION], endpoints: { mention } });
  const fake = await fakePlatform(undefined, answer);
  const stop = new AbortController();
  const serving = p.serve({ argv: ["node", "main.js"], addr: fake.addr, signals: false, stop: stop.signal });
  await fake.sessionOpened;

  const r = await fake.request(78, "/chat.v1.Chat/Mention", {});
  assert.equal(r.status, 200, r.body.toString());
  assert.deepEqual(out.agents, [
    { agent: "chat.notes", plugin: "chat", name: "notes", title: "notes", description: "", session: false, pluginTitle: "Chat" },
    {
      agent: "crm.assistant", plugin: "crm", name: "assistant", title: "CRM assistant",
      description: "Answers about deals.", icon: "wand-sparkles", engine: "claude", session: true, pluginTitle: "CRM assistant",
    },
  ]);
  assert.deepEqual([out.run.plugin, out.run.agent, out.run.startedByPlugin], ["crm", "assistant", "chat"]);
  const sent = fake.seen.syscalls;
  assert.deepEqual(sent.map((s) => s.call), ["agentsAvailable", "agentRun"]);
  assert.deepEqual({ ...sent[0].agentsAvailable }, { requestId: "78" });
  assert.equal(sent[1].agentRun.agent, "crm.assistant");
  assert.equal(sent[1].agentRun.topic, "conversation", "the topic goes by its name");
  assert.equal(sent[1].agentRun.topicKey, '{"id":"c-7"}', "its key as the platform matches it");

  stop.abort();
  await serving;
  await fake.close();
});

test("ctx.agents starts a run in a group and reads who is in it and for whom", async () => {
  const { endpoint, plugin, t } = await import("../dist/index.js");
  const wireRun = (id, agent, extra = {}) => ({
    id, agent, plugin: "desk", engine: "claude", status: "running", input: "pack order 7",
    startedBy: { userId: "u1", displayName: "Sarah" }, startedByPlugin: "desk",
    createdAt: "1000", updatedAt: "2000", deadlineAt: "1801000", turn: 1, lastSeq: "3", pod: "running", group: "run-1", ...extra,
  });
  // a run another agent of the group started: the agent is who started it, the person at the root is who it is for
  const brought = wireRun("run-3", "courier", {
    startedBy: { userId: "sa-packer", displayName: "Packer" }, startedByPlugin: "",
    parent: "run-2", parentAgent: "desk.packer", onBehalfOf: { userId: "u1", displayName: "Sarah" },
  });
  const answer = (s) => {
    if (s.agentRun) {
      const { agent, group } = s.agentRun;
      if (group === "run-0") return { error: "unknown_group: no group run-0 of the plugin desk" };
      if (group === "run-1" && agent === "assistant") return { error: "agent_in_group: the agent assistant has a run in the group run-1" };
      if (group === "run-9") return { error: "group_full: the group run-9 has its 8 runs" };
      return { agentRun: group ? wireRun("run-2", agent, { group }) : wireRun("run-1", agent) };
    }
    if (s.agentRuns) return { agentRuns: { runs: [brought, wireRun("run-2", "packer"), wireRun("run-1", "assistant")] } };
    return undefined;
  };

  const out = {};
  const pack = endpoint({
    input: { task: t.text() },
    output: { group: t.text() },
    async handler(ctx, { task }) {
      const a = ctx.agents;
      out.first = await a.run("assistant", task);
      out.joined = await a.run("packer", task, { group: out.first.group });
      out.runs = await a.list({ group: out.first.group });
      out.errors = [];
      for (const group of ["run-0", "run-1", "run-9"]) {
        out.errors.push(await a.run("assistant", task, { group }).then(() => "answered", (e) => `${e.status} ${e.code}: ${e.message}`));
      }
      return { group: out.first.group };
    },
  });
  const p = plugin({ id: "desk", version: "0.1.0", agents: { assistant: { delegates: ["packer"] }, packer: {} }, endpoints: { pack } });
  const fake = await fakePlatform(undefined, answer);
  const stop = new AbortController();
  const serving = p.serve({ argv: ["node", "main.js"], addr: fake.addr, signals: false, stop: stop.signal });
  await fake.sessionOpened;

  const r = await fake.request(78, "/desk.v1.Desk/Pack", { task: "pack order 7" });
  assert.equal(r.status, 200, r.body.toString());
  assert.deepEqual(JSON.parse(r.body.toString()), { group: "run-1" });

  // a run that founded its group is its own group; nothing started it but the plugin's code
  assert.deepEqual(
    [out.first.id, out.first.group, "parent" in out.first, "parentAgent" in out.first, out.first.onBehalfOf],
    ["run-1", "run-1", false, false, { userId: "u1", displayName: "Sarah" }],
  );
  assert.deepEqual([out.joined.id, out.joined.agent, out.joined.group, out.joined.parent], ["run-2", "packer", "run-1", undefined]);
  assert.deepEqual(out.runs[0], {
    id: "run-3", agent: "courier", plugin: "desk", engine: "claude", status: "running", ended: false, session: false,
    input: "pack order 7", startedBy: { userId: "sa-packer", displayName: "Packer" },
    createdAt: 1000, updatedAt: 2000, deadlineAt: 1801000, turn: 1, lastSeq: 3, pod: "running",
    group: "run-1", parent: "run-2", parentAgent: "desk.packer", onBehalfOf: { userId: "u1", displayName: "Sarah" },
  });
  assert.deepEqual(out.runs.map((x) => [x.id, x.group, x.parent, x.parentAgent]), [
    ["run-3", "run-1", "run-2", "desk.packer"],
    ["run-2", "run-1", undefined, undefined],
    ["run-1", "run-1", undefined, undefined],
  ]);
  assert.deepEqual(out.errors, [
    "404 unknown_group: no group run-0 of the plugin desk",
    "409 agent_in_group: the agent assistant has a run in the group run-1",
    "409 group_full: the group run-9 has its 8 runs",
  ]);

  // what went to the platform: the group a start joins and the one a listing is narrowed to
  const sent = fake.seen.syscalls;
  assert.deepEqual(sent.map((s) => s.call), ["agentRun", "agentRun", "agentRuns", "agentRun", "agentRun", "agentRun"]);
  assert.deepEqual([sent[0].agentRun.agent, sent[0].agentRun.group], ["assistant", ""]);
  assert.deepEqual([sent[1].agentRun.agent, sent[1].agentRun.group], ["packer", "run-1"]);
  assert.deepEqual({ ...sent[2].agentRuns }, { agent: "", limit: 0, group: "run-1" });

  stop.abort();
  await serving;
  await fake.close();
});

test("refusals of an agent call keep the platform's code and status", async () => {
  const { SyscallError, syscallFailed } = await import("../dist/ctx.js");
  const of = (message) => {
    const e = syscallFailed(new SyscallError(message));
    return [e.status, e.code, e.message];
  };
  assert.deepEqual(of("unknown_agent: the plugin shop declares no agent x"), [404, "unknown_agent", "the plugin shop declares no agent x"]);
  assert.deepEqual(of("not_found: no such run"), [404, "not_found", "no such run"]);
  assert.deepEqual(of("run_ended: the run has ended"), [409, "run_ended", "the run has ended"]);
  assert.deepEqual(of("bad_request: input is empty"), [400, "bad_request", "input is empty"]);
  assert.deepEqual(of("forbidden: not a member"), [403, "forbidden", "not a member"]);
  assert.deepEqual(of("unknown_group: no group run-0 of the plugin shop"), [404, "unknown_group", "no group run-0 of the plugin shop"]);
  assert.deepEqual(of("agent_in_group: the agent helper has a run in the group"), [409, "agent_in_group", "the agent helper has a run in the group"]);
  assert.deepEqual(of("group_full: the group has its 8 runs"), [409, "group_full", "the group has its 8 runs"]);
  assert.equal(of("store: the store is away")[0], 503);
});

/** A plugin whose `Wait` holds every request until `open()`; `started` counts the handlers that began. */
function gated() {
  let open;
  const gate = new Promise((resolve) => (open = resolve));
  const state = { started: 0, open: () => open() };
  state.plugin = plugin({
    id: "gate",
    version: "1.0.0",
    endpoints: {
      wait: endpoint({
        input: {},
        output: { ok: t.bool() },
        async handler() {
          state.started++;
          await gate;
          return { ok: true };
        },
      }),
    },
  });
  return state;
}

async function until(check) {
  while (!check()) await new Promise((resolve) => setTimeout(resolve, 5));
}

// crates/sdk/src/session.rs: 64 requests at once, 256 more wait, past them 503 `unavailable` with `retry-after: 1`.
test("64 requests run at once, 256 wait and the next is refused as busy", async () => {
  const fake = await fakePlatform();
  const g = gated();
  const stop = new AbortController();
  const serving = g.plugin.serve({ argv: ["node", "main.js"], addr: fake.addr, signals: false, stop: stop.signal });
  await fake.sessionOpened;

  const admitted = [];
  for (let id = 1; id <= 320; id++) admitted.push(fake.request(id, "/gate.v1.Gate/Wait", {}));
  await until(() => g.started === 64);
  const busy = await fake.request(321, "/gate.v1.Gate/Wait", {});
  assert.equal(busy.status, 503);
  assert.deepEqual(JSON.parse(busy.body.toString()), { code: "unavailable", message: "the plugin is busy", details: {} });
  assert.deepEqual(
    busy.headers.map((h) => ({ name: h.name, value: h.value })),
    [{ name: "content-type", value: "application/json" }, { name: "retry-after", value: "1" }],
  );
  assert.equal(g.started, 64);

  g.open();
  const answers = await Promise.all(admitted);
  assert.deepEqual(new Set(answers.map((r) => r.status)), new Set([200]));
  assert.equal(g.started, 320);
  assert.equal((await fake.request(322, "/gate.v1.Gate/Wait", {})).status, 200);

  stop.abort();
  await serving;
  await fake.close();
});

test("while draining, new requests are refused with retry-after and those in flight finish", async () => {
  const fake = await fakePlatform();
  const g = gated();
  const stop = new AbortController();
  const serving = g.plugin.serve({ argv: ["node", "main.js"], addr: fake.addr, signals: false, stop: stop.signal });
  await fake.sessionOpened;

  const inFlight = fake.request(1, "/gate.v1.Gate/Wait", {});
  await until(() => g.started === 1);
  stop.abort();
  const refused = await fake.request(2, "/gate.v1.Gate/Wait", {});
  assert.equal(refused.status, 503);
  assert.deepEqual(JSON.parse(refused.body.toString()), { code: "unavailable", message: "the plugin is stopping", details: {} });
  assert.equal(refused.headers.find((h) => h.name === "retry-after")?.value, "1");

  g.open();
  assert.equal((await inFlight).status, 200);
  await serving;
  await fake.close();
});
