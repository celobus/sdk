// The descriptor: the conformance plugin, naming, defaults and definition errors.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { DescribeErrors, endpoint, entity, plugin, role, t, topic, workspace } from "../dist/index.js";
import hello from "./fixtures/hello.mjs";

const here = dirname(fileURLToPath(import.meta.url));

test("the conformance plugin's descriptor is sdk/conformance/hello/ts/expected.json", () => {
  const expected = JSON.parse(readFileSync(join(here, "../../conformance/hello/ts/expected.json"), "utf8"));
  assert.deepEqual(hello.describe(), expected);
  assert.deepEqual(hello.check(), []);
});

test("column modifiers (unique, index, encrypted) are printed on the entity's fields only when set, not on its message's copy", () => {
  const sku = entity({
    columns: { code: t.text().unique(), shelf: t.text().optional().index(), supplierNote: t.text().optional().encrypted() },
  });
  const d = plugin({ id: "shop", version: "0.1.0", entities: { sku } }).describe();
  const fields = Object.fromEntries(d.entities[0].fields.map((f) => [f.name, f]));
  assert.equal(fields.code.unique, true);
  assert.equal(fields.shelf.index, true);
  assert.equal(fields.supplier_note.encrypted, true);
  assert.equal("encrypted" in fields.code, false);
  assert.equal("encrypted" in fields.shelf, false);
  assert.equal("index" in fields.supplier_note, false);
  for (const f of d.messages.find((m) => m.name === "Sku").fields) {
    assert.equal("unique" in f || "index" in f || "encrypted" in f, false, f.name);
  }
});

test("names come from keys: endpoints PascalCase, fields, roles and entities snake_case", () => {
  const reviewer = role({ title: "Reviewer", defaultFor: "member" });
  const orderLine = entity({ columns: { unitPrice: t.decimal(), productName: t.text() } });
  const d = plugin({
    id: "orders",
    version: "1.2.0-dev.1",
    roles: { leadReviewer: reviewer },
    scopes: { read: { title: "Read orders" }, exportAll: { title: "Export", sensitive: true } },
    agents: { nightlySync: { roles: [reviewer] } },
    permissions: ["members.email"],
    dependencies: ["customers"],
    entities: { orderLine },
    endpoints: {
      placeOrder: endpoint({
        input: { customerId: t.uuid(), lines: t.array(t.message("LineInput", { quantity: t.int() })) },
        output: { orderNumber: t.text() },
        roles: [reviewer, workspace.admin],
        scopes: ["write", "export_all"],
        idempotency: "required",
        timeout: "10s",
        handler: () => ({ orderNumber: "SO-1" }),
      }),
      listLines: endpoint({ input: {}, output: orderLine, readOnly: true, auth: "any", timeout: 500, handler: () => ({}) }),
    },
  }).describe();
  assert.equal(d.title, "Orders");
  assert.deepEqual(d.roles, [{ name: "lead_reviewer", title: "Reviewer", description: "", strict: false, default_for: "member" }]);
  assert.deepEqual(d.scopes.map((s) => [s.name, s.sensitive]), [["read", false], ["export_all", true]]);
  assert.deepEqual(d.agents, [{ name: "nightly_sync", roles: ["lead_reviewer"] }]);
  assert.deepEqual(d.dependencies, [{ plugin: "customers", calls: [] }]);
  assert.deepEqual(d.endpoints.map((e) => [e.name, e.input, e.output, e.roles, e.scopes, e.auth, e.idempotency, e.timeout_ms]), [
    ["PlaceOrder", "PlaceOrderRequest", "PlaceOrderReply", ["lead_reviewer", "workspace.admin"], ["write", "export_all"], "user", "required", 10000],
    ["ListLines", "ListLinesRequest", "OrderLine", [], ["read"], "any", "off", 500],
  ]);
  // first use while walking the endpoints (a nested message right after its parent), then the entities'
  assert.deepEqual(d.messages.map((m) => m.name), ["PlaceOrderRequest", "LineInput", "PlaceOrderReply", "ListLinesRequest", "OrderLine"]);
  assert.deepEqual(d.messages[0].fields.map((f) => [f.name, f.number, f.type, f.repeated]), [
    ["customer_id", 1, "uuid", false],
    ["lines", 2, "message:LineInput", true],
  ]);
  assert.deepEqual(d.entities, [{
    name: "order_line",
    message: "OrderLine",
    key: "uuid",
    label: "id",
    soft_delete: false,
    version: false,
    append_only: false,
    fields: [
      { name: "id", number: 1, type: "ref:orders.order_line", repeated: false, optional: false, description: "" },
      { name: "unit_price", number: 2, type: "decimal", repeated: false, optional: false, description: "" },
      { name: "product_name", number: 3, type: "string", repeated: false, optional: false, description: "" },
    ],
  }]);
});

test("definition errors are collected with codes and paths", () => {
  const stray = role({});
  const orphan = entity({ columns: { title: t.text() } });
  const p = plugin({
    id: "platform",
    version: "1.0",
    capabilities: { "net:anything": "x", "push:send": " " },
    dependencies: ["platform", "a", "a"],
    entities: { thing: entity({ columns: { id: t.text(), createdAt: t.timestamp() }, label: "nope" }) },
    endpoints: {
      do: endpoint({
        input: { owner_id: t.text(), other: t.ref(orphan) },
        output: orphan,
        roles: [stray, "billing.viewer"],
        scopes: ["admin"],
        timeout: "11m",
        handler: () => ({}),
      }),
      dupe: endpoint({ input: t.message("Clash", {}), output: t.message("Clash", {}), handler: () => ({}) }),
    },
  });
  const codes = p.check().map((e) => `${e.code} ${e.path}`);
  assert.deepEqual(codes, [
    "E_PLUGIN_ID_RESERVED id",
    "E_PLUGIN_VERSION version",
    "E_CAPABILITY_UNKNOWN capabilities.\"net:anything\"",
    "E_CAPABILITY_REASON capabilities.\"push:send\"",
    "E_DEP_SELF dependencies[0]",
    "E_DEP_DUP dependencies[2]",
    "E_COL_RESERVED entities.thing.columns.id",
    "E_COL_RESERVED entities.thing.columns.createdAt",
    "E_ENDPOINT_MESSAGE endpoints.do.output",
    "E_FIELD_NAME messages.DoRequest.owner_id",
    "E_FIELD_TYPE messages.DoRequest.other",
    "E_ENDPOINT_SCOPE_UNKNOWN endpoints.do.scopes[0]",
    "E_ENDPOINT_TIMEOUT_RANGE endpoints.do.timeout",
    "E_ENDPOINT_ROLE_UNKNOWN endpoints.do.roles[0]",
    "E_ENDPOINT_ROLE_UNKNOWN endpoints.do.roles[1]",
    "E_MSG_DUP endpoints.dupe.output",
    "E_ENT_LABEL entities.thing.label",
  ]);
  assert.throws(() => p.describe(), (e) => e instanceof DescribeErrors && /^error\[E_PLUGIN_ID_RESERVED\] at id: `platform` is reserved/.test(e.message));
});

test("an agent says how it runs; what it leaves out is left out", () => {
  const clerk = role({ title: "Clerk" });
  const p = plugin({
    id: "shop",
    version: "1.0.0",
    roles: { clerk },
    agents: {
      sync: { roles: [clerk] },
      priceHelper: {
        title: "Assistant",
        description: "Answers questions about prices.",
        engine: "pi",
        model: "claude-opus-5-5",
        session: true,
        timeout: "15m",
        skills: ["money"],
      },
    },
  });
  assert.deepEqual(p.describe().agents, [
    { name: "sync", roles: ["clerk"] },
    {
      name: "price_helper", roles: [], title: "Assistant", description: "Answers questions about prices.",
      engine: "pi", model: "claude-opus-5-5", session: true, timeout_s: 900, skills: ["money"],
    },
  ]);
  const bad = plugin({ id: "shop", version: "1.0.0", agents: { sync: { timeout: "soon", session: false } } });
  assert.deepEqual(bad.check().map((e) => `${e.code} ${e.path}`), ["E_AGENT_TIMEOUT_RANGE agents.sync.timeout"]);
});

test("an agent asks for base role admin; one that does not describes itself as before", () => {
  const p = plugin({
    id: "shop",
    version: "1.0.0",
    agents: { builder: { admin: true, workspace: ["plugins.install"] }, helper: { admin: false }, packer: {} },
  });
  const d = p.describe();
  assert.deepEqual(d.agents, [
    { name: "builder", roles: [], workspace: ["plugins.install"], admin: true },
    { name: "helper", roles: [] },
    { name: "packer", roles: [] },
  ]);
  assert.equal("admin" in d.agents[1], false, "admin: false is left out");
  assert.deepEqual(p.check(), []);
});

test("an agent names the agents it may bring into its group", () => {
  const p = plugin({
    id: "shop",
    version: "1.0.0",
    dependencies: ["crm"],
    agents: {
      // one of this plugin as its key is written, one of a dependency as `<plugin>.<agent>`
      helper: { delegates: ["orderHelper", "packer", "crm.scout", "crm.leadFinder"] },
      orderHelper: { delegates: [] },
      packer: {},
    },
  });
  const d = p.describe();
  assert.deepEqual(d.agents, [
    { name: "helper", roles: [], delegates: ["order_helper", "packer", "crm.scout", "crm.leadFinder"] },
    { name: "order_helper", roles: [] },
    { name: "packer", roles: [] },
  ]);
  assert.equal("delegates" in d.agents[1], false, "an empty list is left out");
  assert.equal("delegates" in d.agents[2], false, "an agent without delegates describes itself as before");
});

test("an agent names the topic its runs invalidate: one of the plugin's", () => {
  const clerk = role({ title: "Clerk" });
  const RUNS = topic("assistant_runs", t.message("AllAssistantRuns", {}));
  const p = plugin({
    id: "shop",
    version: "1.0.0",
    roles: { clerk },
    topics: [RUNS.roles([clerk])],
    agents: { assistant: { roles: [clerk], topic: RUNS }, sync: { topic: RUNS.roles([clerk]) } },
  });
  const d = p.describe();
  assert.deepEqual(d.agents, [
    { name: "assistant", roles: ["clerk"], topic: "assistant_runs" },
    { name: "sync", roles: [], topic: "assistant_runs" },
  ]);
  assert.equal(d.topics[0].name, "assistant_runs");
  // a topic the plugin does not declare, and something that is no topic
  const undeclared = plugin({ id: "shop", version: "1.0.0", agents: { assistant: { topic: RUNS }, sync: { topic: "assistant_runs" } } });
  assert.deepEqual(undeclared.check().map((e) => `${e.code} ${e.path}`), [
    "E_AGENT_TOPIC_UNKNOWN agents.assistant.topic",
    "E_SCHEMA agents.sync.topic",
  ]);
});

test("--describe prints the descriptor and exits 0; definition errors exit 2 on stderr", () => {
  const dir = mkdtempSync(join(tmpdir(), "celobus-sdk-"));
  const sdk = pathToFileURL(join(here, "../dist/index.js")).href;
  const good = join(dir, "good.mjs");
  writeFileSync(good, `import plugin from ${JSON.stringify(pathToFileURL(join(here, "fixtures/hello.mjs")).href)};\nawait plugin.serve();\n`);
  const ok = spawnSync(process.execPath, [good, "--describe"], { encoding: "utf8" });
  assert.equal(ok.status, 0, ok.stderr);
  assert.deepEqual(JSON.parse(ok.stdout), hello.describe());

  const bad = join(dir, "bad.mjs");
  writeFileSync(bad, `import { plugin } from ${JSON.stringify(sdk)};\nawait plugin({ id: "Bad_Id", version: "0.1.0" }).serve();\n`);
  const failed = spawnSync(process.execPath, [bad, "--describe"], { encoding: "utf8" });
  assert.equal(failed.status, 2);
  assert.equal(failed.stdout, "");
  assert.match(failed.stderr, /^error\[E_PLUGIN_ID\] at id: "Bad_Id" is not a plugin id/);
});

test("access rules: relations, grants, guards, masks, exports and the bypass scope", async () => {
  const { entity: ent, plugin: plug, role: r, t: tt, workspace: ws } = await import("../dist/index.js");
  const platform = await import("../dist/platform.js");
  const manager = r({ title: "Manager" });
  const auditor = r({ title: "Auditor", strict: true });
  const state = tt.enum("State", ["draft", "published"]);
  const book = ent({
    columns: { ownerId: tt.ref(platform.user), groupId: tt.ref(platform.group).optional(), readers: tt.array(tt.ref(platform.user)) },
    access: (a) => {
      const owner = a.relation("owner", a.column("ownerId"));
      a.read(owner, a.relation("team", a.group("groupId")), manager);
      a.create(owner);
      a.export(["read"]);
    },
  });
  const page = ent({
    columns: { bookId: tt.ref(() => book), authorId: tt.ref(platform.user), state, secret: tt.text().optional() },
    access: (a) => {
      const author = a.relation("author", a.column("authorId"));
      const inBook = a.relation("book", a.column("bookId"));
      a.read(author, inBook.via("read"));
      a.create(a.all(author, inBook.via("read")));
      a.update(author.when({ state: "draft" }), manager);
      a.delete(a.all(author, a.not(auditor)));
      a.permit("publish", manager);
      a.guard("state").to("published", a.holds("publish")).toAny(ws.admin);
      a.mask("secret", [manager]);
    },
  });
  const p = plug({
    id: "notes",
    version: "1.0.0",
    roles: { manager, auditor },
    capabilities: { "authz:bypass": { reason: "Archives pages.", on: [{ entity: page, ops: ["read", "update"] }] } },
    entities: { book, page },
  });
  assert.deepEqual(p.check(), []);
  const d = p.describe();
  const a = d.entities[1].access;
  assert.deepEqual(a.read, { any: [{ rel: "author" }, { via: { rel: "book", perm: "read" } }] });
  assert.deepEqual(a.update, {
    any: [{ when: { grant: { rel: "author" }, filter: [{ field: "state", op: "eq", value: "draft" }] } }, { role: "manager" }],
  });
  assert.deepEqual(a.delete, { all: [{ rel: "author" }, { not: { role: "auditor" } }] });
  assert.deepEqual(a.guards, [{ fields: ["state"], rules: [
    { value: "published", grant: { holds: "publish" } },
    { value: null, grant: { role: "workspace.admin" } },
  ] }]);
  assert.deepEqual(a.masks, [{ field: "secret", roles: ["manager"] }]);
  assert.deepEqual([a.export, a.admin, d.entities[0].access.export], [[], true, ["read"]]);
  assert.deepEqual(d.entities[0].access.relations.map((x) => x.kind), ["column", "group"]);
  assert.deepEqual(d.capabilities[0].on, [{ entity: "page", ops: ["read", "update"] }]);
  // without rules: none written
  const bare = plug({ id: "bare", version: "1.0.0", entities: { book: ent({ columns: { title: tt.text() } }) } }).describe();
  assert.equal(bare.entities[0].access, undefined);
});

test("when: equalities, `in`, `ne` and null tests on the row's own fields", async () => {
  const { entity: ent, plugin: plug, t: tt } = await import("../dist/index.js");
  const platform = await import("../dist/platform.js");
  const state = tt.enum("State", ["draft", "review", "published"]);
  const note = ent({
    columns: { ownerId: tt.ref(platform.user), state, closedAt: tt.timestamp().optional(), secret: tt.text().optional() },
    access: (a) => {
      const owner = a.relation("owner", a.column("ownerId"));
      a.read(owner.when({ state: ["draft", "review"], closedAt: null }));
      a.update(owner.when({ state: a.ne("published"), secret: a.isNotNull() }));
    },
  });
  const p = plug({ id: "notes", version: "1.0.0", entities: { note } });
  assert.deepEqual(p.check(), []);
  const a = p.describe().entities[0].access;
  assert.deepEqual(a.read.when.filter, [
    { field: "state", op: "in", value: ["draft", "review"] },
    { field: "closed_at", op: "is_null", value: null },
  ]);
  assert.deepEqual(a.update.when.filter, [
    { field: "state", op: "ne", value: "published" },
    { field: "secret", op: "is_not_null", value: null },
  ]);
});

test("a relation through the rows of a junction entity", async () => {
  const { entity: ent, plugin: plug, role: r, t: tt } = await import("../dist/index.js");
  const platform = await import("../dist/platform.js");
  const manager = r({ title: "Manager" });
  const shelfMember = ent({
    softDelete: true,
    columns: {
      shelfId: tt.ref(() => shelf),
      userId: tt.ref(platform.user).optional(),
      teamId: tt.ref(platform.group).optional(),
      keeper: tt.bool(),
    },
    access: (a) => {
      const inShelf = a.relation("shelf", a.column("shelfId"));
      a.read(inShelf.via("read"));
      a.create(inShelf.via("update"));
    },
  });
  const shelf = ent({
    columns: { title: tt.text() },
    access: (a) => {
      const member = a.relation("member", a.rows(shelfMember, "shelfId", "userId"));
      const team = a.relation("team", a.rows(shelfMember, "shelfId", "teamId"));
      const keeper = a.relation("keeper", a.rows(shelfMember, "shelfId", "userId").when({ keeper: true }));
      a.read(member, team, manager);
      a.update(keeper, manager);
    },
  });
  const p = plug({ id: "notes", version: "1.0.0", roles: { manager }, entities: { shelf, shelfMember } });
  assert.deepEqual(p.check(), []);
  const d = p.describe();
  assert.deepEqual(d.entities[0].access.relations, [
    { name: "member", kind: "rows", field: "user_id", rows: { entity: "shelf_member", target: "shelf_id", filter: [] } },
    { name: "team", kind: "rows", field: "team_id", rows: { entity: "shelf_member", target: "shelf_id", filter: [] } },
    { name: "keeper", kind: "rows", field: "user_id",
      rows: { entity: "shelf_member", target: "shelf_id", filter: [{ field: "keeper", op: "eq", value: true }] } },
  ]);
  assert.deepEqual(d.entities[0].access.read, { any: [{ rel: "member" }, { rel: "team" }, { role: "manager" }] });
  // a junction that is not one of the plugin's entities
  const stray = ent({ columns: { shelfId: tt.ref(() => shelf), userId: tt.ref(platform.user) } });
  const other = ent({
    columns: { title: tt.text() },
    access: (a) => a.read(a.relation("member", a.rows(stray, "shelfId", "userId"))),
  });
  const codes = plug({ id: "notes", version: "1.0.0", entities: { other } }).check().map((e) => e.code);
  assert.ok(codes.includes("E_ACCESS_ROWS_PLUGIN"), String(codes));
});

test("reads are written only when declared; SQL leaves nothing in the descriptor", () => {
  const bare = plugin({ id: "bare", version: "1.0.0", dependencies: ["customers", { plugin: "crm", calls: ["Get"], reads: [] }] }).describe();
  assert.equal("statements" in bare, false);
  assert.deepEqual(bare.dependencies, [{ plugin: "customers", calls: [] }, { plugin: "crm", calls: ["Get"] }]);
});

test("a dependency's reads are copied as the generated client gives them", async () => {
  const reader = (await import("./fixtures/reader.mjs")).default;
  assert.deepEqual(reader.check(), []);
  const d = reader.describe();
  assert.deepEqual(d.dependencies, [{
    plugin: "customers",
    calls: ["GetCustomer"],
    reads: [{ entity: "customer", fields: [
      { name: "id", type: "ref:customers.customer" },
      { name: "name", type: "string" },
      { name: "email", type: "optional string" },
    ] }],
  }]);
  // the dependency's entity is not this plugin's; a ref to it names its plugin
  assert.deepEqual(d.entities.map((e) => e.name), ["visit"]);
  assert.equal(d.messages.find((m) => m.name === "Visit").fields[1].type, "ref:customers.customer");
  assert.equal(d.messages.some((m) => m.name === "Customer"), false);
});

test("a foreign entity is no entity or output of this plugin", () => {
  const customer = entity({ plugin: "customers", name: "customer", columns: { name: t.text() } });
  const p = plugin({
    id: "stats",
    version: "1.0.0",
    entities: { customer },
    endpoints: { get: endpoint({ input: {}, output: customer, handler: () => ({}) }) },
  });
  assert.deepEqual(p.check().map((e) => `${e.code} ${e.path}`), [
    "E_SCHEMA entities.customer",
    "E_ENDPOINT_MESSAGE endpoints.get.output",
  ]);
});

test("searchable, nav and inverse are written only when declared; the SDK checks what it can see", () => {
  const parent = entity({ columns: { title: t.text().searchable(), body: t.text().optional().searchable("english") } });
  const child = entity({
    columns: {
      parentId: t.ref(() => parent, { nav: "parent", inverse: "children" }),
      otherId: t.ref(() => parent).optional(),
    },
  });
  const d = plugin({ id: "tree", version: "1.0.0", entities: { parent, child } }).describe();
  const field = (e, f) => d.entities.find((x) => x.name === e).fields.find((x) => x.name === f);
  assert.equal(field("parent", "title").searchable, "simple");
  assert.equal(field("parent", "body").searchable, "english");
  assert.deepEqual([field("child", "parent_id").nav, field("child", "parent_id").inverse], ["parent", "children"]);
  assert.ok(!("nav" in field("child", "other_id")) && !("searchable" in field("child", "other_id")));
  // the entity's message has the fields without the column modifiers
  assert.ok(!("nav" in d.messages.find((m) => m.name === "Child").fields[1]));

  const bad = entity({
    columns: {
      n: t.int().searchable(),
      s: t.text().searchable("klingon"),
      r: t.text(),
      ...Object.fromEntries(Array.from({ length: 8 }, (_, i) => [`x${i}`, t.text().searchable()])),
    },
  });
  const named = entity({ columns: { ownerId: t.ref(() => named, { nav: "Owner" }), label: t.text() } });
  const p = plugin({ id: "bad", version: "1.0.0", entities: { bad, named } });
  const codes = p.check().map((e) => `${e.code} ${e.path}`);
  assert.ok(codes.includes("E_SEARCH_TYPE entities.bad.columns.n"), codes.join("\n"));
  assert.ok(codes.includes("E_SEARCH_LANGUAGE entities.bad.columns.s.searchable"), codes.join("\n"));
  assert.ok(codes.includes("E_SEARCH_LIMIT entities.bad.columns.x6"), codes.join("\n"));
  assert.ok(codes.includes("E_REF_NAME entities.named.columns.ownerId.nav"), codes.join("\n"));
});

test("a ref's onDelete is written as on_delete and checked as the CLI checks it", () => {
  const parent = entity({ columns: { title: t.text() } });
  const ok = plugin({
    id: "shop",
    version: "1.0.0",
    entities: {
      parent,
      child: entity({ columns: {
        parentId: t.ref(() => parent, { onDelete: "cascade" }),
        otherId: t.ref(() => parent, { onDelete: "setNull" }).optional(),
        teamId: t.ref("platform.group", { onDelete: "setNull" }).optional(),
      } }),
    },
  });
  assert.deepEqual(ok.check(), []);
  const fields = ok.describe().entities.find((e) => e.name === "child").fields;
  assert.deepEqual(fields.map((f) => f.on_delete), [undefined, "cascade", "set_null", "set_null"]);
  const bad = plugin({
    id: "shop",
    version: "1.0.0",
    entities: {
      parent,
      log: entity({ appendOnly: true, columns: {
        parentId: t.ref(() => parent, { onDelete: "keep" }),
        otherId: t.ref(() => parent, { onDelete: "setNull" }),
        nextId: t.ref(() => parent, { onDelete: "setNull" }).optional(),
        customerId: t.ref("crm.customer", { onDelete: "cascade" }),
        title: t.text(),
      } }),
    },
  });
  assert.deepEqual(bad.check().map((e) => `${e.code} ${e.path}`), [
    "E_REF_ON_DELETE entities.log.columns.parentId.onDelete",
    "E_REF_ON_DELETE entities.log.columns.otherId.onDelete",
    "E_ENT_APPEND_ONLY_CASCADE entities.log.columns.nextId.onDelete",
    "E_REF_CROSS_ON_DELETE entities.log.columns.customerId.onDelete",
  ]);
});
