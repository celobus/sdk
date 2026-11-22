// The repository and transactions over the session: what the SDK sends for each call and how it reads the platform's
// answers and refusals, against the fake platform of the session tests.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import * as grpc from "@grpc/grpc-js";
import { celldDefinition } from "../dist/serve.js";
import { sink } from "../dist/log.js";
import hello from "./fixtures/hello.mjs";
import reader, { VISIT_STATS, VISITS_SINCE } from "./fixtures/reader.mjs";
import repo, { OWNER as REPO_OWNER } from "./fixtures/repo.mjs";

sink.write = () => {};

/** A fake platform whose syscalls `answer` answers; the syscalls it saw. */
async function fakePlatform(answer) {
  const socket = join(mkdtempSync(join(tmpdir(), "celobus-")), "plugin.sock");
  rmSync(socket, { force: true });
  const seen = [];
  let session;
  let onSession;
  const opened = new Promise((resolve) => (onSession = resolve));
  const waiters = new Map();
  const server = new grpc.Server();
  server.addService(celldDefinition().service, {
    Session(call) {
      session = call;
      call.on("data", (msg) => {
        if (msg.kind === "register") {
          call.write({ registered: { instanceId: "test-1" } });
          onSession();
        } else if (msg.kind === "syscall") {
          seen.push(msg.syscall);
          call.write({ syscallResult: { id: msg.syscall.id, error: "", ...answer(msg.syscall) } });
        } else if (msg.kind === "response") {
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
    opened,
    request(id, path, body, plugin = "hello") {
      path = path.replace("/hello.v1.Hello/", `/${plugin}.v1.${plugin[0].toUpperCase()}${plugin.slice(1)}/`);
      const answered = new Promise((resolve) => waiters.set(String(id), resolve));
      session.write({
        request: {
          id: String(id), method: "POST", path, query: "",
          headers: [{ name: "content-type", value: "application/json" }],
          body: Buffer.from(JSON.stringify(body)),
          principal: { userId: "u1", kind: "PERSON", displayName: "Sarah", roles: [], groups: [], scopes: ["hello:write"] },
        },
      });
      return answered;
    },
    close: () => new Promise((resolve) => server.tryShutdown(() => resolve())),
  };
}

const OWNER = "0190f3c2-7a1b-7c3d-8e4f-0123456789ab";
const stored = (id, title, version) =>
  Buffer.from(JSON.stringify({ id: String(id), title, ownerId: OWNER, version: String(version),
    createdAt: "2026-11-12T10:00:00.000000Z", updatedAt: "2026-11-12T10:00:01.000000Z" }));
const rows = (...r) => ({ dbRows: { rows: r, nextPageToken: "" } });

async function serving(answer, run, plug = hello) {
  const fake = await fakePlatform(answer);
  const stop = new AbortController();
  const done = plug.serve({ argv: ["node", "main.js"], addr: fake.addr, signals: false, stop: stop.signal });
  await fake.opened;
  try {
    await run(fake);
  } finally {
    stop.abort();
    await done;
    await fake.close();
  }
}

test("a transaction creates, reads back and commits; the rows come back with their system columns", async () => {
  await serving(
    (s) => {
      if (s.txBegin) return { txBegun: { tx: "5" } };
      if (s.dbCreate) return rows(stored(1, "a", 1));
      if (s.dbFind) return rows(stored(1, "a", 1));
      return { ok: {} };
    },
    async (fake) => {
      const r = await fake.request(3, "/hello.v1.Hello/SaveNote", { title: "a", ownerId: OWNER });
      assert.equal(r.status, 200, r.body.toString());
      assert.deepEqual(JSON.parse(r.body.toString()), { id: "1", title: "a", ownerId: OWNER });
      const calls = fake.seen.map((s) => s.call);
      assert.deepEqual(calls, ["txBegin", "dbCreate", "dbFind", "txCommit"]);
      const [begin, create, find, commit] = fake.seen;
      assert.deepEqual(
        { requestId: begin.txBegin.requestId, tx: begin.txBegin.tx, isolation: begin.txBegin.isolation },
        { requestId: "3", tx: "0", isolation: "READ_COMMITTED" },
      );
      assert.equal(create.dbCreate.tx, "5");
      assert.equal(create.dbCreate.entity, "note");
      assert.deepEqual(JSON.parse(create.dbCreate.rows[0].toString()), { id: "0", title: "a", ownerId: OWNER });
      assert.equal(find.dbFind.tx, "5");
      assert.equal(find.dbFind.limit, 1);
      assert.deepEqual(find.dbFind.order, [{ field: "created_at", desc: true }]);
      const and = find.dbFind.filter.and.items;
      assert.deepEqual(and.map((c) => [c.condition.field, c.condition.op, c.condition.values[0].stringValue]), [
        ["owner_id", "EQ", OWNER],
        ["title", "EQ", "a"],
      ]);
      assert.equal(commit.txCommit.tx, "5");
    },
  );
});

test("a failed block rolls back; a db refusal keeps its status, code and details", async () => {
  await serving(
    (s) => {
      if (s.txBegin) return { txBegun: { tx: "6" } };
      if (s.dbUpdate) return { error: "unique_conflict: another row has this title", errorDetails: { currentVersion: "0", column: "title" } };
      if (s.dbGet) return rows(stored(1, "a", 3));
      if (s.dbDelete) return { error: "version_conflict: note 1 is at version 4", errorDetails: { currentVersion: "4", column: "" } };
      return { ok: {} };
    },
    async (fake) => {
      const r = await fake.request(4, "/hello.v1.Hello/SaveNote", { id: "1", title: "b", ownerId: OWNER });
      assert.equal(r.status, 409);
      const body = JSON.parse(r.body.toString());
      assert.equal(body.code, "unique_conflict");
      assert.deepEqual(body.details, { column: "title" });
      assert.deepEqual(fake.seen.map((s) => s.call), ["txBegin", "dbUpdate", "txRollback"]);
      assert.deepEqual(JSON.parse(fake.seen[1].dbUpdate.patch.toString()), { title: "b", body: null });

      const d = await fake.request(5, "/hello.v1.Hello/DeleteNote", { id: "1" });
      assert.equal(d.status, 409);
      const err = JSON.parse(d.body.toString());
      assert.deepEqual([err.code, err.details], ["version_conflict", { current: 4 }]);
      const del = fake.seen.find((s) => s.dbDelete).dbDelete;
      assert.deepEqual([del.id, del.expectedVersion, del.force], ["1", "3", false]);
    },
  );
});

test("filters and orders from field handles", async () => {
  const { and, not, or } = await import("../dist/index.js");
  const { note } = await import("./fixtures/hello.mjs");
  const f = or(note.f.title.in(["a", "b"]), not(note.f.body.isNull()), and());
  assert.deepEqual(f.node, undefined, "or with every row is every row");
  const g = and(note.f.title.ge("a"), not(note.f.body.isNull()));
  assert.deepEqual(g.node, {
    and: {
      items: [
        { condition: { field: "title", op: "GE", values: [{ stringValue: "a" }] } },
        { not: { condition: { field: "body", op: "IS_NULL", values: [] } } },
      ],
    },
  });
  const o = note.f.version.desc();
  assert.deepEqual([o.field, o.desc], ["version", true]);
});

test("a system context names its reason on its calls; the access rules' refusals are 403s", async () => {
  await serving(
    (s) => {
      if (s.dbCount) return { dbCount: { count: "7" } };
      if (s.dbFind) return { error: "permission_denied: you may not read these notes" };
      return { ok: {} };
    },
    async (fake) => {
      const r = await fake.request(6, "/hello.v1.Hello/CountAllNotes", { ownerId: OWNER });
      assert.equal(r.status, 200, r.body.toString());
      assert.deepEqual(JSON.parse(r.body.toString()), { count: "7" });
      const count = fake.seen.find((s) => s.dbCount).dbCount;
      assert.equal(count.asSystem, "count every note");
      assert.equal(count.filter.condition.field, "owner_id");

      const l = await fake.request(7, "/hello.v1.Hello/ListNotes", { limit: 5 });
      assert.equal(l.status, 403);
      assert.equal(JSON.parse(l.body.toString()).code, "permission_denied");
      const find = fake.seen.find((s) => s.dbFind).dbFind;
      assert.equal(find.asSystem, "", "the caller's own context");
      assert.equal(find.limit, 5);
    },
  );
});

const CUSTOMER = "0190f3c2-0000-7000-8000-000000000001";
const customerRow = (name, extra = {}) =>
  Buffer.from(JSON.stringify({ id: CUSTOMER, name, createdAt: "2026-11-12T10:00:00Z", updatedAt: "2026-11-12T10:00:01Z", version: "3", ...extra }));
const read = (fake) => (id, endpoint, body = {}) => fake.request(id, `/hello.v1.Hello/${endpoint}`, body, "reader");

test("ctx.query reads a dependency's entity: dependency and entity on every call, filters, order, page, count", async () => {
  await serving(
    (s) => {
      if (s.dbFind?.all) return rows(customerRow("Ann"), customerRow("Bob"));
      if (s.dbFind?.limit === 1) return rows();
      if (s.dbFind) return { dbRows: { rows: [customerRow("Ann"), customerRow("Al")], nextPageToken: "p2" } };
      if (s.dbCount) return { dbCount: { count: "12" } };
      if (s.dbGet) return rows(customerRow("Ann", { deletedAt: "2026-11-11T09:00:00Z" }));
      return { ok: {} };
    },
    async (fake) => {
      const r = await read(fake)(10, "FindCustomers", { name: "Ann", pageToken: "p1" });
      assert.equal(r.status, 200, r.body.toString());
      assert.deepEqual(JSON.parse(r.body.toString()), {
        names: ["Ann", "Al"], next: "p2", count: 12, exists: false, deleted: "2026-11-11T09:00:00.000Z", version: "3",
      });
      assert.deepEqual(fake.seen.map((s) => s.call), ["dbFind", "dbCount", "dbFind", "dbGet"]);
      const [find, count, exists, get] = fake.seen;
      for (const [call, body] of [["dbFind", find.dbFind], ["dbCount", count.dbCount], ["dbFind", exists.dbFind], ["dbGet", get.dbGet]]) {
        assert.equal(body.dependency, "customers", call);
        assert.equal(body.entity, "customer", call);
        assert.equal(body.requestId, "10", call);
        assert.equal(body.tx, "0", call);
      }
      const either = find.dbFind.filter.or.items.map((c) => [c.condition.field, c.condition.op, c.condition.values[0]?.stringValue]);
      assert.deepEqual(either, [["name", "EQ", "Ann"], ["email", "IS_NULL", undefined]]);
      assert.deepEqual(find.dbFind.order, [{ field: "name", desc: true }]);
      assert.deepEqual([find.dbFind.limit, find.dbFind.pageToken, find.dbFind.withDeleted, find.dbFind.all], [20, "p1", true, false]);
      assert.equal(count.dbCount.filter.condition.op, "GE");
      assert.equal(exists.dbFind.limit, 1);
      assert.deepEqual([get.dbGet.id, get.dbGet.withDeleted], [CUSTOMER, true]);

      const all = await read(fake)(11, "AllCustomers");
      assert.equal(all.status, 200, all.body.toString());
      assert.deepEqual(JSON.parse(all.body.toString()), { names: ["Ann", "Bob"] });
      const every = fake.seen.at(-1).dbFind;
      assert.deepEqual([every.all, every.dependency, every.entity], [true, "customers", "customer"]);
    },
    reader,
  );
});

test("ctx.query of an own entity reads it as its root; ctx.db of a dependency's is refused before anything is sent", async () => {
  await serving(
    () => ({ dbCount: { count: "1" } }),
    async (fake) => {
      const r = await read(fake)(13, "DbForeign");
      assert.equal(r.status, 500, r.body.toString());
      assert.equal(JSON.parse(r.body.toString()).code, "internal");
      assert.deepEqual(fake.seen, []);
      const own = await read(fake)(12, "QueryOwn");
      assert.equal(own.status, 200, own.body.toString());
      assert.deepEqual(JSON.parse(own.body.toString()), { n: 1 });
      assert.deepEqual([fake.seen[0].dbCount.entity, fake.seen[0].dbCount.dependency ?? ""], ["visit", ""]);
    },
    reader,
  );
});

test("own repositories send no dependency", async () => {
  await serving(
    (s) => (s.dbCount ? { dbCount: { count: "7" } } : { ok: {} }),
    async (fake) => {
      const r = await fake.request(14, "/hello.v1.Hello/CountAllNotes", {});
      assert.equal(r.status, 200, r.body.toString());
      assert.equal(fake.seen[0].dbCount.dependency, "");
    },
  );
});

const tally = (owner, notes) => Buffer.from(JSON.stringify({ owner_id: owner, notes }));
const NOTES_BY_OWNER =
  "select owner_id, count(*) as notes from hello_note group by owner_id having count(*) >= $1 order by owner_id";

test("ctx.sql sends the query and its params as a JSON array, and answers the rows as JSON objects by column", async () => {
  await serving(
    (s) => (s.sql ? rows(tally(OWNER, 3), tally("0190f3c2-7a1b-7c3d-8e4f-0123456789ac", 2)) : { ok: {} }),
    async (fake) => {
      const r = await fake.request(20, "/hello.v1.Hello/NotesByOwner", { minNotes: "2" });
      assert.equal(r.status, 200, r.body.toString());
      assert.deepEqual(JSON.parse(r.body.toString()), {
        owners: [{ ownerId: OWNER, notes: "3" }, { ownerId: "0190f3c2-7a1b-7c3d-8e4f-0123456789ac", notes: "2" }],
      });
      assert.deepEqual(fake.seen.map((s) => s.call), ["sql"]);
      const call = fake.seen[0].sql;
      assert.deepEqual([call.requestId, call.tx, call.query, call.unsafe], ["20", "0", NOTES_BY_OWNER, false]);
      assert.deepEqual(JSON.parse(call.params.toString()), ["2"], "a bigint is its decimal text");
    },
  );
});

test("ctx.sql without params sends []; inside a transaction it carries the transaction's id", async () => {
  await serving(
    (s) => {
      if (s.txBegin) return { txBegun: { tx: "9" } };
      if (s.sql) return rows(Buffer.from(JSON.stringify({ day: "2026-11-12", visits: 4 })));
      return { ok: {} };
    },
    async (fake) => {
      const r = await read(fake)(21, "Stats");
      assert.equal(r.status, 200, r.body.toString());
      assert.deepEqual(JSON.parse(r.body.toString()), { rows: [{ day: "2026-11-12", visits: "4" }] });
      const plain = fake.seen[0].sql;
      assert.deepEqual([plain.query, plain.tx, plain.params.toString(), plain.unsafe], [VISIT_STATS, "0", "[]", false]);

      const t = await read(fake)(22, "StatsSince", { from: "2026-11-01", min: "2" });
      assert.equal(t.status, 200, t.body.toString());
      assert.deepEqual(fake.seen.slice(1).map((s) => s.call), ["txBegin", "sql", "txCommit"]);
      const inTx = fake.seen[2].sql;
      assert.deepEqual([inTx.query, inTx.tx, inTx.requestId], [VISITS_SINCE, "9", "22"]);
      assert.deepEqual(JSON.parse(inTx.params.toString()), ["2026-11-01", "2"]);
    },
    reader,
  );
});

test("ctx.unsafeSql sends unsafe and answers the returning rows", async () => {
  await serving(
    (s) => (s.sql ? rows(Buffer.from(JSON.stringify({ day: "2026-10-30", visits: 1 }))) : { ok: {} }),
    async (fake) => {
      const r = await read(fake)(26, "Purge", { from: "2026-11-01", min: "0" });
      assert.equal(r.status, 200, r.body.toString());
      assert.deepEqual(JSON.parse(r.body.toString()), { rows: [{ day: "2026-10-30", visits: "1" }] });
      const call = fake.seen[0].sql;
      assert.deepEqual([call.requestId, call.tx, call.unsafe], ["26", "0", true]);
      assert.match(call.query, /^delete from reader_visit where at < \$1 returning/);
      assert.deepEqual(JSON.parse(call.params.toString()), ["2026-11-01"]);
    },
    reader,
  );
});

test("ctx.sql surfaces the platform's refusals as the repository does: query_rows_limit is 422, invalid 400, permission_denied 403", async () => {
  await serving(
    () => ({ error: "query_rows_limit: the query returned more than 1000 rows" }),
    async (fake) => {
      const r = await fake.request(23, "/hello.v1.Hello/NotesByOwner", { minNotes: "0" });
      assert.equal(r.status, 422);
      const body = JSON.parse(r.body.toString());
      assert.deepEqual([body.code, body.message], ["query_rows_limit", "the query returned more than 1000 rows"]);
    },
  );
  await serving(
    () => ({ error: "invalid: the query: syntax error at or near \"selec\"" }),
    async (fake) => {
      const r = await read(fake)(28, "Stats");
      assert.equal(r.status, 400, r.body.toString());
      const body = JSON.parse(r.body.toString());
      assert.deepEqual([body.code, body.message], ["invalid", 'the query: syntax error at or near "selec"']);
    },
    reader,
  );
  await serving(
    () => ({ error: "permission_denied: reader_secret is not one of the plugin's tables" }),
    async (fake) => {
      const r = await read(fake)(27, "Purge", { from: "2026-11-01", min: "0" });
      assert.equal(r.status, 403, r.body.toString());
      assert.equal(JSON.parse(r.body.toString()).code, "permission_denied");
    },
    reader,
  );
});

test("ctx.sql binds every JSON value in order: null for undefined, a Date as its ISO text, an object as its JSON", async () => {
  const { sqlParams } = await import("../dist/sql.js");
  const at = new Date("2026-11-12T10:00:00.000Z");
  assert.equal(
    sqlParams(["a", 1, 2n, true, null, undefined, at, { k: [1] }]).toString(),
    '["a",1,"2",true,null,null,"2026-11-12T10:00:00.000Z",{"k":[1]}]',
  );
});

// --- search, aggregates, include, bulk writes, row locks ---------------------------------------------------------

const op = (fake) => async (id, name) => {
  const r = await fake.request(id, "/hello.v1.Hello/Run", { op: name }, "repo");
  const body = JSON.parse(r.body.toString());
  return { status: r.status, body, out: r.status === 200 ? JSON.parse(body.out) : undefined };
};
const json = (v) => Buffer.from(JSON.stringify(v));
const item = (id, extra = {}) =>
  json({ id, name: "apple", sku: "A", qty: 1, price: "2.50", weight: 1.5, ownerId: REPO_OWNER,
    createdAt: "2026-11-12T10:00:00.000000Z", updatedAt: "2026-11-12T10:00:01.000000Z", ...extra });

test("search and contains are conditions of their own on text columns", async () => {
  await serving(
    () => rows(),
    async (fake) => {
      const r = await op(fake)(30, "search");
      assert.equal(r.status, 200, JSON.stringify(r.body));
      const and = fake.seen[0].dbFind.filter.and.items.map((i) => i.condition);
      assert.deepEqual(and.map((c) => [c.field, c.op, c.values[0].stringValue]), [
        ["name", "SEARCH", '"red apple" -green'],
        ["sku", "CONTAINS", "50%_"],
      ]);
    },
    repo,
  );
});

test("aggregates send their group, measure and field, and read keys and values with the column's codec", async () => {
  const groups = (...g) => ({ dbGroups: { groups: g.map(([k, v]) => ({ key: json(k), value: json(v) })) } });
  await serving(
    (s) => {
      const a = s.dbAggregate;
      if (a.groupBy === "owner_id") return groups([REPO_OWNER, "3"], [null, "1"]);
      if (a.field === "qty") return groups(["A-1", "12"]);
      if (a.field === "price") return groups([null, "7.25"]);
      if (a.field === "weight") return groups([null, 2.5]);
      return groups([null, "9"]);
    },
    async (fake) => {
      const run = op(fake);
      const by = await run(31, "countBy");
      assert.deepEqual(by.out, [{ ownerId: REPO_OWNER, count: 3 }, { count: 1 }]);
      const a = fake.seen[0].dbAggregate;
      assert.deepEqual([a.entity, a.groupBy, a.measure, a.field, a.withDeleted], ["item", "owner_id", "COUNT", "", false]);
      assert.equal(a.filter.condition.op, "GT");
      assert.deepEqual((await run(32, "countAll")).out, [{ count: 9 }]);
      assert.equal(fake.seen[1].dbAggregate.withDeleted, true);
      assert.equal(fake.seen[1].dbAggregate.groupBy, "");
      assert.deepEqual((await run(33, "sumQty")).out, [{ parentId: "A-1", sum: "12n" }]);
      assert.deepEqual([fake.seen[2].dbAggregate.measure, fake.seen[2].dbAggregate.groupBy], ["SUM", "parent_id"]);
      assert.deepEqual((await run(34, "sumPrice")).out, [{ sum: "7.25", decimal: true }]);
      assert.deepEqual((await run(35, "sumWeight")).out, [{ sum: 2.5 }]);
    },
    repo,
  );
});

test("include sends snake_case names and decodes each row's navigations: a row, or the rows of an inverse", async () => {
  await serving(
    (s) => {
      if (s.dbFind) {
        return {
          dbRows: {
            rows: [item("A-2", { parentId: "A-1" }), item("A-1")],
            nextPageToken: "",
            included: [json({ parent: JSON.parse(item("A-1").toString()), parent_id: JSON.parse(item("A-1").toString()), children: [] }), json({ children: [JSON.parse(item("A-2", { parentId: "A-1" }).toString())] })],
          },
        };
      }
      return { dbRows: { rows: [item("A-1")], nextPageToken: "", included: [json({})] } };
    },
    async (fake) => {
      const run = op(fake);
      const r = await run(36, "include");
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(fake.seen[0].dbFind.include, ["parent", "children", "parent_id"]);
      const [a2, a1] = r.out;
      assert.equal(a2.included.parent.id, "A-1");
      assert.equal(a2.included.parentId.id, "A-1");
      assert.equal(a2.included.parent.createdAt, "2026-11-12T10:00:00.000Z", "decoded as the target's row");
      assert.deepEqual(a2.included.children, []);
      assert.equal(a1.included.parent, undefined);
      assert.deepEqual(a1.included.children.map((c) => [c.id, c.parentId]), [["A-2", "A-1"]]);
      const g = await run(37, "getInclude");
      assert.deepEqual(fake.seen[1].dbGet.include, ["children"]);
      assert.deepEqual(g.out.included, { children: [] }, "an inverse with nothing is an empty list");
    },
    repo,
  );
});

test("updateWhere sends its filter and patch and answers the rows changed; upsert names its unique column", async () => {
  await serving(
    (s) => (s.dbUpdateWhere ? { dbCount: { count: "4" } } : rows(item("A-2"))),
    async (fake) => {
      const run = op(fake);
      assert.equal((await run(38, "updateWhere")).out, 4);
      const u = fake.seen[0].dbUpdateWhere;
      assert.equal(u.entity, "item");
      assert.equal(u.filter.condition.field, "qty");
      assert.deepEqual(JSON.parse(u.patch.toString()), { qty: 5, parentId: null });
      assert.equal((await run(39, "upsert")).out.id, "A-2");
      const up = fake.seen[1].dbUpsert;
      assert.equal(up.on, "sku");
      assert.deepEqual(JSON.parse(up.row.toString()).sku, "P");
    },
    repo,
  );
});

test("row locks go only inside a transaction; row_locked is 409", async () => {
  await serving(
    (s) => {
      if (s.txBegin) return { txBegun: { tx: "8" } };
      if (s.dbGet) return { error: "row_locked: item A-1 is locked by another transaction" };
      if (s.dbFind) return rows(item("A-1"));
      return { ok: {} };
    },
    async (fake) => {
      const run = op(fake);
      const locked = await run(40, "lockGet");
      assert.equal(locked.status, 409);
      assert.equal(locked.body.code, "row_locked");
      const get = fake.seen[1].dbGet;
      assert.deepEqual([get.tx, get.lock.strength, get.lock.wait], ["8", "FOR_UPDATE", "NOWAIT"]);
      assert.deepEqual(fake.seen.map((s) => s.call), ["txBegin", "dbGet", "txRollback"]);

      const page = await run(41, "lockFind");
      assert.equal(page.status, 200, JSON.stringify(page.body));
      const find = fake.seen[4].dbFind;
      assert.deepEqual([find.tx, find.limit, find.lock.strength, find.lock.wait], ["8", 3, "FOR_SHARE", "SKIP_LOCKED"]);

      const before = fake.seen.length;
      const outside = await run(42, "lockOutside");
      assert.equal(outside.status, 400);
      assert.equal(outside.body.code, "invalid");
      assert.equal(fake.seen.length, before, "nothing is sent");
      const orphan = await run(43, "lockOrphanWait");
      assert.equal(orphan.status, 400, "nowait without a lock is refused");
    },
    repo,
  );
});
