// Files: `t.file()` in the descriptor (with `maxSize`, `accept`), its JSON, the kit's file primitives (upload, attach,
// post files, file columns), sizes, and `ctx.files` in a test context (files kept in memory).
import assert from "node:assert/strict";
import { test } from "node:test";
import { MemoryFiles, endpoint, entity, job, plugin, t, testCtx, ui } from "../dist/index.js";
import { parseSize } from "../dist/schema.js";
import { decodeMessage, encodeMessage } from "../dist/codec.js";

const codes = (p) => p.check().map((e) => `${e.code} ${e.path}`);

test("sizes read as the Rust SDK reads them: bytes, binary and decimal units, fractions dropped below a byte", () => {
  const cases = [
    ["1048576", 1_048_576], ["1 GiB", 2 ** 30], ["500MB", 500_000_000], ["1.5 GiB", 1.5 * 2 ** 30], ["10 mib", 10 * 2 ** 20],
    ["2 TiB", 2 * 2 ** 40], ["3 kb", 3000], ["7 B", 7], ["1.005 MB", 1_005_000], ["0.5 KiB", 512], ["1.0000001 KB", 1000],
    [" 12 ", 12],
  ];
  for (const [s, n] of cases) assert.equal(parseSize(s), n, s);
  for (const s of ["", "GiB", "1 PB", "1.", ".5 MB", "1.2.3 MB", "-1 MB", "1 GiBs", "1e3"]) assert.equal(parseSize(s), undefined, s);
});

const report = entity({
  columns: {
    title: t.text(),
    scans: t.array(t.file({ maxSize: "1 GiB", accept: ["image/*", "application/pdf"] })),
    cover: t.file().optional(),
    signed: t.file(),
  },
});
const importCsv = endpoint({
  input: t.message("ImportCsvRequest", { file: t.file({ accept: ["text/csv"], maxSize: 1_048_576 }) }),
  output: t.message("Imported", { rows: t.bigint() }),
  handler: () => ({ rows: 0n }),
});

test("file fields are described as `file`, with their options on the entity and its message", () => {
  const p = plugin({ id: "reports", version: "0.1.0", entities: { report }, endpoints: { importCsv } });
  const d = p.describe();
  const fields = d.entities[0].fields;
  assert.deepEqual(fields[2], {
    name: "scans", number: 3, type: "file", repeated: true, optional: false, description: "", max_size: 2 ** 30,
    accept: ["image/*", "application/pdf"],
  });
  assert.deepEqual([fields[3].type, fields[3].optional, fields[3].max_size, fields[3].accept], ["file", true, undefined, undefined]);
  assert.deepEqual([fields[4].type, fields[4].optional, fields[4].repeated], ["file", false, false]);
  assert.equal(d.messages.find((m) => m.name === "Report").fields[2].max_size, 2 ** 30);
  const imp = d.messages.find((m) => m.name === "ImportCsvRequest").fields[0];
  assert.deepEqual([imp.accept, imp.max_size], [["text/csv"], 1_048_576]);
  assert.deepEqual(codes(p), []);
});

test("a size that is not one, and an option t.file() does not know, are definition errors", () => {
  const bad = entity({ columns: { a: t.file({ maxSize: "lots" }), b: t.file({ maxSize: 0 }), c: t.file({ maxSize: 1.5 }), d: t.file({ limit: 3 }) } });
  assert.deepEqual(codes(plugin({ id: "bad", version: "0.1.0", entities: { bad } })), [
    "E_FILE_OPTION entities.bad.columns.a.maxSize",
    "E_FILE_OPTION entities.bad.columns.b.maxSize",
    "E_FILE_OPTION entities.bad.columns.c.maxSize",
    "E_OPTION_UNKNOWN entities.bad.columns.d.limit",
  ]);
});

test("a scheduled job's payload with a required file has no default", () => {
  const nightly = job({ payload: { file: t.file() }, cron: "0 3 * * *", handler() {} });
  assert.deepEqual(codes(plugin({ id: "jobs", version: "0.1.0", jobs: { nightly } })), ["E_JOB_SCHEDULE_PAYLOAD jobs.nightly.payload"]);
  const optional = job({ payload: { file: t.file().optional(), files: t.array(t.file()) }, cron: "0 3 * * *", handler() {} });
  assert.deepEqual(codes(plugin({ id: "jobs", version: "0.1.0", jobs: { optional } })), []);
});

test("a file reads and writes its JSON: the size a string, a number read too, the id required", () => {
  const msg = t.message("Upload", { file: t.file(), more: t.array(t.file()), maybe: t.file().optional() });
  const json = { id: "0193", name: "ads.csv", type: "text/csv", size: "2100000" };
  const value = { id: "0193", name: "ads.csv", type: "text/csv", size: 2_100_000 };
  assert.deepEqual(encodeMessage(msg, { file: value, more: [value] }), { file: json, more: [json] });
  let errors = {};
  assert.deepEqual(decodeMessage(msg, { file: { ...json, size: 2_100_000 }, more: [json] }, "", errors), { file: value, more: [value] });
  assert.deepEqual(errors, {});
  // name, type and size may be left out
  assert.deepEqual(decodeMessage(msg, { file: { id: "x" } }, "", (errors = {})).file, { id: "x", name: "", type: "", size: 0 });
  // a file without an id, a missing required file, a size that is not bytes
  decodeMessage(msg, { file: { name: "x" }, more: [{ id: "y", size: "-1" }] }, "", (errors = {}));
  assert.deepEqual(errors, { "file.id": "expected a file id", "more[0].size": "expected a number of bytes" });
  decodeMessage(msg, {}, "", (errors = {}));
  assert.deepEqual(errors, { file: "required" });
  assert.throws(() => encodeMessage(msg, { file: { name: "x" } }), /expected a File/);
});

test("the kit's file primitives: an upload with its field's options, a file column and meta, post files, attach", () => {
  const upload = t.message("UploadRequest", {
    note: t.text(),
    scans: t.array(t.file({ maxSize: "10 MiB", accept: ["image/*", "application/pdf"] })),
    cover: t.file(),
  });
  const row = t.message("Doc", { id: t.text(), file: t.file(), files: t.array(t.file()), text: t.text(), by: t.text(), time: t.bigint() });
  const docs = t.message("Docs", { rows: t.array(row) });
  const send = t.message("SendRequest", { text: t.text(), files: t.array(t.file({ maxSize: "2 GiB" })) });
  const add = endpoint({ input: upload, output: row, handler: () => ({}) });
  const list = endpoint({ input: {}, output: docs, readOnly: true, handler: () => ({ rows: [] }) });
  const post = endpoint({ input: send, output: row, handler: () => ({}) });
  const SCREEN = ui.screenKey("docs", t.message("DocsParams", {}));
  const d = plugin({
    id: "docs",
    version: "0.1.0",
    endpoints: { add: add.ui(), list: list.ui(), post: post.ui() },
    ui: ui.define({
      screens: [
        ui.screen(SCREEN, (s) => {
          const q = s.query("docs", ui.call(list, {}).rows(docs.f.rows));
          const sendIt = s.action("send", (a) => a.call(post, { text: ui.form.text() }));
          return ui.stack([
            s.form(add, (f) => {
              f.text(upload.f.note, "Note");
              f.upload(upload.f.scans, "Scans");
              f.upload(upload.f.cover, "Cover");
            }, "Add", (a) => a.callForm(add, {})),
            ui.table(q).col(row.f.text, "Text").file(row.f.file, "File"),
            ui.list(q, row.f.text).metaFile(row.f.file),
            ui.stream(q).key(row.f.id).post((p) => p.author(row.f.by).time(row.f.time).markdown(row.f.text).files(row.f.files)),
            ui.composer(sendIt).tools(["emoji"]).attach(send.f.files).attach(send.f.files),
          ]);
        }),
      ],
    }),
  }).describe();
  const [form, table, listNode, stream, composer] = d.ui.screens[0].layout.stack.children;
  assert.deepEqual(form.form.sections[0].inputs, [
    { name: "note", control: "text", label: "Note" },
    { name: "scans", control: "upload", label: "Scans", multiple: true, max_size: 10 * 2 ** 20, accept: ["image/*", "application/pdf"] },
    { name: "cover", control: "upload", label: "Cover" },
  ]);
  assert.deepEqual(table.table.columns[1], { field: "file", title: "File", format: { file: {} } });
  assert.deepEqual([listNode.list.meta, listNode.list.meta_format], ["file", { file: {} }]);
  assert.equal(stream.stream.item.post.files, "files");
  assert.deepEqual([composer.composer.tools, composer.composer.files], [["emoji", "attach"], "files"]);
});

test("in a test context ctx.files reads, writes and looks up files kept in memory", async () => {
  const files = new MemoryFiles();
  const csv = files.add("ads.csv", "text/csv", "day,clicks\r\n2026-11-01,3\n2026-11-02,5");
  const importIt = job({
    payload: { file: t.file({ accept: ["text/csv"] }) },
    async handler(ctx, { file }) {
      const lines = [];
      for await (const line of (await ctx.files.open(file)).lines()) lines.push(line);
      const w = await ctx.files.create("clicks.csv", "text/csv", { size: 2 });
      await w.write("8\n");
      out.made = await w.finish();
      out.lines = lines;
      out.head = await ctx.files.head(file);
      const tail = await ctx.files.open(file, { offset: 12 });
      out.tail = Buffer.from(await new Response(tail).arrayBuffer()).toString();
      out.file = tail.file;
    },
  });
  const out = {};
  const p = plugin({ id: "ads", version: "0.1.0", jobs: { importIt } });
  await importIt.options.handler(testCtx(p, { files, run: { runId: "r1", attempt: 1, maxAttempts: 1 } }), { file: csv });
  assert.deepEqual(out.lines, ["day,clicks", "2026-11-01,3", "2026-11-02,5"]);
  assert.deepEqual(out.made, { id: "file-2", name: "clicks.csv", type: "text/csv", size: 2 });
  assert.equal(files.text(out.made), "8\n");
  assert.deepEqual(out.head.file, csv);
  assert.equal(out.tail, "2026-11-01,3\n2026-11-02,5");
  assert.deepEqual(out.file, csv);
  assert.deepEqual(files.list().map((f) => f.name), ["ads.csv", "clicks.csv"]);

  const ctx = testCtx(p);
  await assert.rejects(ctx.files.open(csv), { status: 404, code: "not_found" });
  const w = await ctx.files.create("x.txt", "", { size: 1 });
  await assert.rejects(w.write("ab"), { status: 409, code: "file_size_mismatch" });
  await assert.rejects(ctx.files.create("y", "text/plain", { bytes: 1 }), { status: 500, code: "internal" });
  const empty = await (await ctx.files.create("empty.txt", "")).finish();
  assert.deepEqual([empty.type, empty.size], ["application/octet-stream", 0]);
});

test("a writer over the platform: a declared size goes with the create, a failed part fails the rest and the finish", async () => {
  const http = await import("node:http");
  const { PlatformFiles } = await import("../dist/files.js");
  const puts = [];
  const store = http.createServer((req, res) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const n = Number(req.url.split("/").pop());
      puts.push([n, Buffer.concat(chunks).toString()]);
      if (n === 2) return res.writeHead(500).end();
      res.writeHead(200, { etag: `"e${n}"` }).end();
    });
  });
  await new Promise((resolve) => store.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${store.address().port}`;
  const later = String(Date.now() + 3_600_000);
  const calls = [];
  const link = {
    requestId: "9",
    signal: new AbortController().signal,
    async syscall(call) {
      calls.push(call);
      const page = (first) => ({ id: "f1", partSize: "4", parts: 3, first, urls: [1, 2, 3].map((n) => `${base}/put/${n}`).slice(first - 1), expiresAtMs: later });
      if (call.filesCreate) return { filesUpload: page(1) };
      if (call.filesParts) return { filesUpload: page(call.filesParts.from) };
      return { file: { id: "f1", name: "a.bin", contentType: "", size: "10" } };
    },
  };
  const w = await new PlatformFiles(link).create("a.bin", "", { size: 10 });
  assert.deepEqual(calls[0], { filesCreate: { name: "a.bin", contentType: "", size: "10", requestId: "9" } });
  const reused = new Uint8Array(4);
  reused.set([97, 98, 99, 100]);
  const first = w.write(reused);
  reused.fill(120); // the caller reuses its array: what was written stays
  await first;
  await assert.rejects(w.write("efgh"), { status: 503, code: "unavailable" });
  await assert.rejects(w.write("ij"), { status: 503, code: "unavailable" });
  await assert.rejects(w.finish(), { status: 503, code: "unavailable" });
  assert.equal(calls.some((c) => c.filesFinish), false, "a file with a part missing is never finished");
  assert.deepEqual(puts.map(([n, body]) => `${n}:${body}`), ["1:abcd", "2:efgh", "2:efgh", "2:efgh"]);
  store.close();
});
