// Settings: what `--describe` writes, the cache `ctx.settings()` reads, the check hook and the test context
// (crates/sdk/src/settings_tests.rs: the same plugin, the same descriptor).
import assert from "node:assert/strict";
import { inspect } from "node:util";
import { test } from "node:test";
import { Decimal, Secret, endpoint, invalid, plugin, settingsDef, t, testCtx } from "../dist/index.js";
import * as platform from "../dist/platform.js";
import { dispatch } from "../dist/serve.js";
import { SettingsCache } from "../dist/settings.js";
import { createLogger } from "../dist/log.js";

const tone = t.enum("Tone", ["calm", "loud_and_clear"]);

const chatSettings = t.message(
  "ChatSettings",
  {
    threadWarmHours: t.int()
      .describe("After this many hours without messages a thread no longer sticks to the agent that answered in it.")
      .label("Thread stays with its agent, hours").default(2).min(1).max(48).section("Agents"),
    messageMaxChars: t.bigint().describe("The longest message a member may send.")
      .default(20000n).min(100).max(20000).message("From 100 to 20,000 characters"),
    webhookUrl: t.text().optional().label("Webhook URL").url().section("Integrations").placeholder("https://"),
    webhookToken: t.secret().optional().section("Integrations").maxLen(200),
    tone: t.enum(tone).default("calm"),
    watchers: t.array(t.ref(platform.user)).maxItems(3),
    greeting: t.text().multiline().pattern("^[A-Z]").minLen(1),
    price: t.decimal().optional(),
  },
  { description: "How the chat treats agents in threads.", kind: "settings" },
);

const ping = endpoint({
  input: {},
  output: { hours: t.int() },
  async handler(ctx) {
    return { hours: ctx.settings(chatSettings).threadWarmHours };
  },
});

const chat = (settings = settingsDef(chatSettings)) => plugin({ id: "chat", version: "1.0.0", endpoints: { ping }, settings });

const DERIVED = {
  message: "ChatSettings",
  on_change: false,
  form: {
    sections: [
      { title: "Agents", inputs: [
        { name: "thread_warm_hours", control: "number", label: "Thread stays with its agent, hours",
          help: "After this many hours without messages a thread no longer sticks to the agent that answered in it." },
      ] },
      { inputs: [
        { name: "message_max_chars", control: "number", label: "Message max chars", help: "The longest message a member may send." },
        { name: "tone", control: "select", label: "Tone",
          options: [{ value: "calm", label: "Calm" }, { value: "loud_and_clear", label: "Loud and clear" }] },
        { name: "watchers", control: "member", label: "Watchers" },
        { name: "greeting", control: "textarea", label: "Greeting", required: true },
        { name: "price", control: "number", label: "Price" },
      ] },
      { title: "Integrations", inputs: [
        { name: "webhook_url", control: "text", label: "Webhook URL", placeholder: "https://" },
        { name: "webhook_token", control: "secret", label: "Webhook token" },
      ] },
    ],
  },
};

const canon = (v) => JSON.parse(JSON.stringify(v));

test("describe writes the settings and their fields' options", () => {
  const d = chat().describe();
  const m = d.messages.find((x) => x.name === "ChatSettings");
  assert.equal(m.kind, "settings");
  // the settings message comes last
  assert.equal(d.messages.at(-1).name, "ChatSettings");
  assert.deepEqual(canon(m.fields[0]), {
    name: "thread_warm_hours", number: 1, type: "int32", repeated: false, optional: false,
    description: "After this many hours without messages a thread no longer sticks to the agent that answered in it.",
    label: "Thread stays with its agent, hours", default: 2, min: 1, max: 48, section: "Agents",
  });
  assert.deepEqual(canon(m.fields[1]), {
    name: "message_max_chars", number: 2, type: "int64", repeated: false, optional: false,
    description: "The longest message a member may send.",
    default: 20000, min: 100, max: 20000, message: "From 100 to 20,000 characters",
  });
  assert.equal(m.fields[2].format, "url");
  assert.equal(m.fields[2].placeholder, "https://");
  assert.deepEqual([m.fields[3].type, m.fields[3].max_len], ["secret", 200]);
  assert.deepEqual([m.fields[4].type, m.fields[4].default], ["enum:Tone", "calm"]);
  assert.deepEqual([m.fields[5].type, m.fields[5].repeated, m.fields[5].max_items], ["ref:platform.user", true, 3]);
  assert.deepEqual([m.fields[6].multiline, m.fields[6].pattern, m.fields[6].min_len], [true, "^[A-Z]", 1]);
  assert.deepEqual(canon(m.fields[7]), { name: "price", number: 8, type: "decimal", repeated: false, optional: true, description: "" });
  assert.deepEqual(canon(d.settings), DERIVED);
});

test("defaults, bounds and lists are written as each type spells them", () => {
  const s = t.message("S", {
    rate: t.double().default(0.5).min(-1.5).max(2),
    price: t.decimal().default(Decimal.from("12.50")).min("0.01").max("1000"),
    since: t.date().default("2026-01-01").min("2020-01-01"),
    on: t.bool().default(true),
    tags: t.array(t.text()).default(["a", "b"]).minItems(1),
    tones: t.array(t.enum(tone)).default(["calm"]),
    contact: t.text().optional().email(),
  }, { kind: "settings" });
  const d = plugin({ id: "xy", version: "1.0.0", settings: settingsDef(s) }).describe();
  const f = Object.fromEntries(d.messages.find((m) => m.name === "S").fields.map((x) => [x.name, x]));
  assert.deepEqual([f.rate.default, f.rate.min, f.rate.max], [0.5, -1.5, 2]);
  assert.deepEqual([f.price.default, f.price.min, f.price.max], ["12.50", "0.01", "1000"]);
  assert.deepEqual([f.since.default, f.since.min], ["2026-01-01", "2020-01-01"]);
  assert.equal(f.on.default, true);
  assert.deepEqual([f.tags.default, f.tags.min_items], [["a", "b"], 1]);
  assert.deepEqual(f.tones.default, ["calm"]);
  assert.equal(f.contact.format, "email");
  const inputs = d.settings.form.sections.flatMap((x) => x.inputs);
  assert.deepEqual(inputs.map((i) => i.control), ["number", "number", "date", "switch", "textarea", "multi_select", "text"]);
  assert.deepEqual(inputs[5].options, [{ value: "calm", label: "Calm" }, { value: "loud_and_clear", label: "Loud and clear" }]);
});

test("a form laid out by hand gets what its fields carry", () => {
  const p = chat(settingsDef(chatSettings, {
    onChange: () => {},
    form: (f) => {
      f.section("Messages");
      f.number(chatSettings.f.messageMaxChars, "Longest message");
      f.textarea(chatSettings.f.greeting, "Greeting").help("Shown first");
      f.section("Other");
      f.number(chatSettings.f.threadWarmHours, "Hours");
      f.radio(chatSettings.f.tone, "Tone");
      f.member(chatSettings.f.watchers, "Watchers");
      f.text(chatSettings.f.webhookUrl, "URL").required();
      f.secret(chatSettings.f.webhookToken, "Token");
      f.number(chatSettings.f.price, "Price");
    },
  }));
  const s = canon(p.describe().settings);
  assert.equal(s.on_change, true);
  assert.equal(s.form.sections.length, 2);
  assert.equal(s.form.sections[0].title, "Messages");
  assert.deepEqual(s.form.sections[0].inputs[0], {
    name: "message_max_chars", control: "number", label: "Longest message", help: "The longest message a member may send.",
  });
  assert.deepEqual(s.form.sections[0].inputs[1], { name: "greeting", control: "textarea", label: "Greeting", help: "Shown first", required: true });
  assert.deepEqual(s.form.sections[1].inputs[1].options[1], { value: "loud_and_clear", label: "Loud and clear" });
  // `required` follows the field, whatever the input says
  assert.deepEqual(s.form.sections[1].inputs[3], { name: "webhook_url", control: "text", label: "URL", placeholder: "https://" });
  assert.equal(s.form.sections[1].inputs[4].control, "secret");
});

test("describe refuses a form without every field, a field twice, or a control its type does not take", () => {
  const p = chat(settingsDef(chatSettings, {
    form: (f) => {
      f.number(chatSettings.f.threadWarmHours, "Hours");
      f.text(chatSettings.f.threadWarmHours, "Twice");
    },
  }));
  const errors = p.check();
  assert.ok(errors.some((e) => e.code === "E_SETTINGS_FORM_FIELD" && e.message.includes("greeting is not in the form")), JSON.stringify(errors));
  assert.ok(errors.some((e) => e.code === "E_SETTINGS_FORM_FIELD" && e.message.includes("twice")), JSON.stringify(errors));
  assert.ok(errors.some((e) => e.code === "E_SETTINGS_CONTROL"), JSON.stringify(errors));
});

test("the options of a setting belong to the settings message", () => {
  const codes = (p) => p.check().map((e) => [e.code, e.path]);
  // on an endpoint's message
  const req = t.message("Req", { n: t.int().min(1), k: t.secret() });
  const ep = endpoint({ input: req, output: {}, handler: () => ({}) });
  assert.deepEqual(codes(plugin({ id: "xy", version: "1.0.0", endpoints: { ep } })), [
    ["E_FIELD_VALIDATOR_SCOPE", "messages.Req.n"],
    ["E_SETTINGS_FIELD_TYPE", "messages.Req.k"],
  ]);
  // a settings message not of kind settings, an unknown option
  const plain = t.message("Plain", { n: t.int().optional() });
  assert.deepEqual(codes(plugin({ id: "xy", version: "1.0.0", settings: settingsDef(plain) })), [["E_SETTINGS_MESSAGE", "settings"]]);
  assert.deepEqual(codes(plugin({ id: "xy", version: "1.0.0", settings: settingsDef(chatSettings, { onChnage: () => {} }) })), [
    ["E_OPTION_UNKNOWN", "settings.onChnage"],
  ]);
});

test("describe refuses settings fields of a type or with options they do not take", () => {
  const s = t.message("Bad", {
    at: t.timestamp().optional(),
    n: t.int().optional().maxLen(3),
    name: t.text().optional().min(1),
    token: t.secret().default(new Secret("x")),
    many: t.array(t.int()),
    pair: t.int().optional().min(5).max(1),
    one: t.text().optional().maxItems(2),
    re: t.text().optional().pattern("("),
  }, { kind: "settings" });
  // (the fields of a type settings do not take are not in the derived form either)
  const got = plugin({ id: "xy", version: "1.0.0", settings: settingsDef(s) }).check()
    .filter((e) => e.code !== "E_SETTINGS_FORM_FIELD").map((e) => [e.code, e.path]);
  const at = (e) => Object.keys(s.fields).indexOf(e[1].split(".")[2]);
  const byField = (a, b) => at(a) - at(b);
  assert.deepEqual(got.sort(byField), [
    ["E_SETTINGS_FIELD_TYPE", "messages.Bad.at"],
    ["E_SETTINGS_VALIDATOR", "messages.Bad.n.max_len"],
    ["E_SETTINGS_VALIDATOR", "messages.Bad.name.min"],
    ["E_SETTINGS_DEFAULT", "messages.Bad.token.default"],
    ["E_SETTINGS_FIELD_TYPE", "messages.Bad.many"],
    ["E_SETTINGS_VALIDATOR", "messages.Bad.pair.min"],
    ["E_SETTINGS_VALIDATOR", "messages.Bad.one.min_items"],
    ["E_SETTINGS_PATTERN", "messages.Bad.re.pattern"],
  ]);
});

test("a secret stays out of logs", () => {
  const s = new Secret("sk-live-123");
  assert.equal(String(s), "Secret(••••)");
  assert.equal(`${s}`, "Secret(••••)");
  assert.equal(inspect(s), "Secret(••••)");
  assert.equal(JSON.stringify({ s }), '{"s":"Secret(••••)"}');
  assert.equal(s.expose(), "sk-live-123");
  assert.ok(!inspect({ webhookToken: s }, { depth: 5, showHidden: true }).includes("sk-live"));
});

test("the cache fills defaults and names what is missing", () => {
  const cache = new SettingsCache(chatSettings);
  // nothing sent yet: the defaults, and the required greeting is missing
  assert.throws(() => cache.get(chatSettings), (e) => {
    assert.deepEqual([e.status, e.code], [409, "settings_required"]);
    assert.deepEqual(e.details.missing, ["greeting"]);
    assert.deepEqual(e.details.fields, { greeting: "Required" });
    return true;
  });
  assert.equal(cache.update(3, Buffer.from('{"greeting": "Hi", "threadWarmHours": 6}'), true), true);
  const s = cache.get(chatSettings);
  assert.deepEqual([s.threadWarmHours, s.messageMaxChars, s.tone, s.watchers], [6, 20000n, "calm", []]);
  // read once, then shared
  assert.equal(cache.get(chatSettings), s);
  // an older change is stale; a newer one replaces them
  assert.equal(cache.update(2, Buffer.from('{"greeting": "Old"}'), false), false);
  assert.equal(cache.get(chatSettings).greeting, "Hi");
  assert.equal(cache.update(4, Buffer.from('{"greeting": "New", "messageMaxChars": "300", "webhookToken": "sk-1"}'), false), true);
  const n = cache.get(chatSettings);
  assert.deepEqual([n.greeting, n.messageMaxChars, n.webhookToken.expose()], ["New", 300n, "sk-1"]);
  assert.ok(n.webhookToken instanceof Secret);
  // a value of the wrong type is an error, never a guess
  assert.equal(cache.update(5, Buffer.from('{"greeting": "X", "threadWarmHours": "many"}'), false), true);
  assert.throws(() => cache.get(chatSettings), { status: 500, code: "internal" });
  // not JSON: kept the previous ones
  assert.equal(cache.update(6, Buffer.from("{"), false), false);
  // another message than the declared one
  const other = t.message("Other", { n: t.int().optional() }, { kind: "settings" });
  assert.throws(() => cache.get(other), { status: 500, code: "internal" });
  // a registration replaces them whatever its revision
  assert.equal(cache.update(0, Buffer.from('{"greeting": "Again"}'), true), true);
  assert.equal(cache.get(chatSettings).greeting, "Again");
  // empty bytes: no settings sent, the defaults
  assert.equal(cache.update(0, Buffer.alloc(0), true), true);
  assert.throws(() => cache.get(chatSettings), { status: 409 });
});

test("a test context has the settings it is given", async () => {
  const p = chat();
  assert.throws(() => testCtx(p).settings(chatSettings), { status: 500, code: "internal" });
  const ctx = testCtx(p, { settings: { greeting: "Hi", threadWarmHours: 4, webhookToken: new Secret("sk-1"), messageMaxChars: 500n } });
  const s = ctx.settings(chatSettings);
  assert.deepEqual([s.threadWarmHours, s.messageMaxChars, s.tone, s.webhookToken.expose()], [4, 500n, "calm", "sk-1"]);
  assert.equal(ctx.settings(chatSettings), s);
  assert.equal(ctx.asSystem("test").settings(chatSettings), s);
  assert.deepEqual(await ping.options.handler(ctx, {}), { hours: 4 });
  // a key that is not a field of the settings
  assert.throws(() => testCtx(p, { settings: { nope: 1 } }), /nope/);
  // a plugin without settings
  assert.throws(() => testCtx(plugin({ id: "xy", version: "1.0.0" }), { settings: {} }), /declares no settings/);
});

test("the check hook is a system request", async () => {
  const seen = [];
  const p = chat(settingsDef(chatSettings, {
    async onChange(ctx, change) {
      assert.equal(ctx.caller.kind, "system");
      if (change.current.webhookToken !== undefined && change.current.webhookUrl === undefined) {
        throw invalid("a token needs a URL").field("webhook_url", "Set the URL the token is for");
      }
      if (change.current.greeting === "Boom") {
        const { RpcError } = await import("../dist/errors.js");
        throw new RpcError(422, "unreachable", "The service did not answer");
      }
      seen.push([change.previous?.threadWarmHours, change.current.threadWarmHours]);
    },
  }));
  const m = { id: "chat", version: "1.0.0", depends: [], resolved: p.model(), log: createLogger({ plugin: "chat" }) };
  const routes = new Map(m.resolved.endpoints.map((ep) => [`/chat.v1.Chat/${ep.name}`, ep]));
  const path = "/chat.v1.Chat/_SettingsCheck";
  assert.ok(routes.has(path));
  const session = { syscall: () => Promise.reject(new Error("no")), invalidate() {}, jobs: new Map() };
  const send = async (body, who) => {
    const resp = await dispatch(m, routes, session, {
      id: "7", method: "POST", path, query: "", headers: [{ name: "content-type", value: "application/json" }],
      body: Buffer.from(JSON.stringify(body)), principal: who,
    });
    return { status: resp.status, body: JSON.parse(resp.body.toString()) };
  };
  const member = { userId: "u-1", kind: "PERSON", displayName: "A", roles: [], groups: [], scopes: [] };
  let r = await send({ previous: { greeting: "Hi" }, current: { greeting: "Hi", threadWarmHours: 8 } }, member);
  assert.deepEqual([r.status, r.body.code], [404, "unknown_endpoint"]);

  r = await send({ previous: { greeting: "Hi" }, current: { greeting: "Hi", threadWarmHours: 8 } }, null);
  assert.deepEqual(r, { status: 200, body: {} });
  assert.equal((await send({ previous: {}, current: { greeting: "Hi", threadWarmHours: 9 } }, null)).status, 200);
  assert.deepEqual(seen, [[2, 8], [undefined, 9]]);

  r = await send({ previous: null, current: { greeting: "Hi", webhookToken: "sk-1" } }, null);
  assert.equal(r.status, 400);
  assert.deepEqual(r.body.details.fields, { webhook_url: "Set the URL the token is for" });
  r = await send({ current: { greeting: "Boom" } }, null);
  assert.deepEqual([r.status, r.body.message], [422, "The service did not answer"]);
  // the plugin without on_change has no route
  assert.ok(![...chat().model().endpoints].some((ep) => ep.name === "_SettingsCheck"));
});
