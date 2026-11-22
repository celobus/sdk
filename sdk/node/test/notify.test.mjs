// Notifications: the kinds in the descriptor, what the SDK sends for each call, and how it reads the platform's refusals,
// against syscalls answered in memory.
import assert from "node:assert/strict";
import { test } from "node:test";
import { Caller, Ctx, endpoint, entity, notification, notify, plugin, role, t, ui } from "../dist/index.js";
import { SyscallError } from "../dist/ctx.js";
import { createLogger, sink } from "../dist/log.js";
import * as platform from "../dist/platform.js";

sink.write = () => {};

const member = role({ title: "Member" });
const mention = notification("mention", { title: "Mentions", description: "Someone wrote @you", digest: true });
const reply = notification("thread_reply", { title: "Replies in threads you follow", tab: "watching" });
const post = entity({ columns: { text: t.text() }, access: (a) => a.read(member) });
const channelParams = t.message("ChannelParams", { c: t.text(), thread: t.text().optional() });
const CHANNEL = ui.screenKey("channel", channelParams);
const PANEL = ui.overlayKey("panel", channelParams);
const approveRun = endpoint({
  input: t.message("ApproveRunRequest", { id: t.text(), owner: t.ref(platform.user) }),
  output: { ok: t.bool() },
  ui: true,
  handler: () => ({ ok: true }),
});
const chat = plugin({
  id: "chat",
  version: "1.0.0",
  roles: { member },
  capabilities: { "push:send": "Tells people when they are mentioned." },
  notifications: [mention, reply],
  entities: { post },
  endpoints: { approveRun },
});

const SARAH = "0193a4c2-0000-7000-8000-0000000000aa";
const POST = "0193a4c2-0000-7000-8000-000000000001";

/** A context of request 77 whose syscalls `answer` answers (or throws); the calls it saw. */
function context(answer = () => ({})) {
  const seen = [];
  const session = {
    async syscall(call) {
      seen.push(call);
      return { id: "1", error: "", ...answer(call) };
    },
    invalidate() {},
  };
  const request = { id: "77", deadline: new Date(Date.now() + 30_000), signal: new AbortController().signal };
  const ctx = new Ctx(session, new Caller(null, "chat"), request, { id: "chat", version: "1.0.0" }, createLogger());
  return { ctx, seen };
}

test("kinds are described only when declared, and need push:send", () => {
  assert.deepEqual(chat.describe().notifications, [
    { name: "mention", title: "Mentions", description: "Someone wrote @you", tab: "for_you", digest: true },
    { name: "thread_reply", title: "Replies in threads you follow", description: "", tab: "watching", digest: false },
  ]);
  assert.equal(plugin({ id: "quiet", version: "1.0.0" }).describe().notifications, undefined);
  const bad = plugin({ id: "chat", version: "1.0.0", notifications: [mention, mention, notification("Bad", { title: "Bad" })] });
  assert.deepEqual(bad.check().map((e) => e.code), ["E_NOTIFY_CAPABILITY", "E_NOTIFY_KIND_NAME", "E_NOTIFY_KIND_NAME"]);
});

test("a send carries the notice and the request; in a transaction it is part of it", async () => {
  chat.describe(); // the plugin is described before it serves: its entities and endpoints have their names
  const { ctx, seen } = context((c) => (c.txBegin ? { txBegun: { tx: "5" } } : {}));
  const thread = notify.row(post, POST);
  assert.equal(thread.key, `post:${POST}`);
  await ctx.notifications.send(mention, {
    to: [SARAH], key: thread, title: "mentioned you in #fulfilment", body: "@Sarah can we hold #1042",
    source: { label: "#fulfilment", icon: "hash" }, link: notify.screen(CHANNEL, { c: "1", thread: "2" }),
  });
  await ctx.transaction(async (tx) => {
    await tx.notifications.send(reply, { followers: thread, except: [SARAH], title: "replied", once: "x", noActor: true });
  });
  const s = seen[0].notifySend;
  assert.equal(s.requestId, "77");
  assert.deepEqual([s.tx, s.kind, s.followers, s.to, s.key, s.subject], ["0", "mention", false, [SARAH], `post:${POST}`, `post/${POST}`]);
  assert.deepEqual(s.actor, { caller: true });
  assert.deepEqual(s.source, { label: "#fulfilment", icon: "hash" });
  assert.deepEqual([s.link.screen, s.link.overlay, JSON.parse(s.link.args)], ["channel", "", { c: "1", thread: "2" }]);
  assert.equal(s.ask, undefined);
  // in a transaction: on it, to the followers of the key but Sarah, as nobody
  const f = seen[2].notifySend;
  assert.deepEqual([f.tx, f.followers, f.once, f.to, f.except, f.key], ["5", true, "x", [], [SARAH], `post:${POST}`]);
  assert.deepEqual(f.actor, { none: true });
  assert.ok(seen[3].txCommit);
});

test("an ask names its buttons by their endpoints, with their requests", async () => {
  chat.describe();
  const { ctx, seen } = context();
  await ctx.notifications.ask({
    kind: "approval", key: notify.key("approval:1"), to: [SARAH], agent: "fulfilment", title: "needs your approval",
    link: notify.overlay(PANEL, { c: "9" }), review: "Review 5 changes",
    buttons: [
      notify.button("Approve", approveRun, { id: "1", owner: SARAH }, { primary: true, icon: "check" }),
      notify.button("Decline", approveRun, { id: "2", owner: SARAH }),
    ],
  });
  await ctx.notifications.resolve(notify.key("approval:1"), "Approved by Sarah Bennett");
  const s = seen[0].notifySend;
  assert.deepEqual([s.kind, s.key, s.subject, s.to], ["", "approval:1", "", [SARAH]]);
  assert.deepEqual(s.actor, { agent: "fulfilment" });
  assert.deepEqual([s.link.screen, s.link.overlay], ["", "panel"]);
  assert.equal(s.ask.kind, "APPROVAL");
  assert.equal(s.ask.review, "Review 5 changes");
  assert.deepEqual(s.ask.buttons.map((b) => [b.label, b.endpoint, JSON.parse(b.body), b.primary, b.icon]), [
    ["Approve", "ApproveRun", { id: "1", owner: SARAH }, true, "check"],
    ["Decline", "ApproveRun", { id: "2", owner: SARAH }, false, ""],
  ]);
  assert.deepEqual(seen[1].notifyResolve, { requestId: "77", tx: "0", key: "approval:1", outcome: "Approved by Sarah Bennett" });
});

test("follow, unfollow and the caller's state", async () => {
  chat.describe();
  const { ctx, seen } = context((c) => (c.notifyFollow?.op === "GET" ? { notifyFollowState: { following: true } } : {}));
  const thread = notify.row(post, POST);
  await ctx.notifications.follow(thread, { reason: "replied", noun: "thread" });
  await ctx.notifications.follow(thread, { reason: "replied", users: [SARAH] });
  await ctx.notifications.unfollow(thread);
  await ctx.notifications.follow(thread);
  assert.equal(await ctx.notifications.following(thread), true);
  const f = seen.map((c) => c.notifyFollow);
  assert.deepEqual([f[0].op, f[0].reason, f[0].noun, f[0].users, f[0].subject], ["FOLLOW", "REPLIED", "thread", [], `post/${POST}`]);
  assert.deepEqual([f[1].reason, f[1].users], ["REPLIED", [SARAH]]);
  assert.deepEqual([f[2].op, f[2].reason, f[2].users], ["UNFOLLOW", "REASON_UNSPECIFIED", []]);
  assert.deepEqual([f[3].op, f[3].reason, f[3].users], ["FOLLOW", "MANUAL", []]);
  assert.deepEqual([f[4].op, f[4].users], ["GET", []]);
});

test("the platform's refusals: invalid, capability_missing, follow_limit, rate_limited", async () => {
  const refusals = [
    ["invalid: kind `nope` is not declared", 400, "invalid"],
    ["capability_missing: the plugin does not declare push:send", 403, "capability_missing"],
    ["follow_limit: the key has 2000 followers", 409, "follow_limit"],
    ["rate_limited: too many sends", 429, "rate_limited"],
  ];
  for (const [error, status, code] of refusals) {
    const { ctx } = context(() => {
      throw new SyscallError(error);
    });
    await assert.rejects(ctx.notifications.send(mention, { to: [SARAH], title: "hi" }), (e) => e.status === status && e.code === code);
  }
});

test("an agent run's start asks to be told when it finishes, and where that opens", async () => {
  const { ctx, seen } = context(() => ({ agentRun: { id: "run-1", agent: "assistant", status: "starting" } }));
  await ctx.agents.run("assistant", "pack order 7", { notify: true, notifyLink: notify.screen(CHANNEL, { c: "1" }) });
  await ctx.agents.run("assistant", "pack order 8");
  const [told, quiet] = seen.map((c) => c.agentRun);
  assert.equal(told.notify, true);
  assert.deepEqual([told.notifyLink.screen, told.notifyLink.overlay, JSON.parse(told.notifyLink.args)], ["channel", "", { c: "1" }]);
  // unsaid: only a failure tells, and it opens the run
  assert.deepEqual([quiet.notify, quiet.notifyLink], [false, undefined]);
});
