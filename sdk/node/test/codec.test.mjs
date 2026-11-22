// The JSON mapping (protobuf JSON) and the Decimal type.
import assert from "node:assert/strict";
import { test } from "node:test";
import { Decimal, entity, t } from "../dist/index.js";
import { decodeMessage, encodeMessage, OutputError } from "../dist/codec.js";
import * as platform from "../dist/platform.js";

const status = t.enum("Status", ["placed", "paid"]);
const line = t.message("Line", { productId: t.text(), quantity: t.int() });
const all = t.message("All", {
  text: t.text(),
  flag: t.bool(),
  small: t.int(),
  big: t.bigint(),
  ratio: t.double(),
  amount: t.decimal(12, 2),
  uid: t.uuid(),
  at: t.timestamp(),
  day: t.date(),
  raw: t.bytes(),
  doc: t.json(),
  status,
  lines: t.array(line),
  tags: t.array(t.text()),
  ownerId: t.ref(platform.user),
  note: t.text().optional(),
});

const decode = (msg, json) => {
  const errors = {};
  const value = decodeMessage(msg, json, "", errors);
  return { value, errors };
};

test("decodes every type from protobuf JSON", () => {
  const { value, errors } = decode(all, {
    text: "a",
    flag: true,
    small: 7,
    big: "9007199254740993",
    ratio: "NaN",
    amount: "12.50",
    uid: "0193a5b2-7c1e-7d2a-9f00-000000000001",
    at: "2026-11-08T10:00:00Z",
    day: "2026-02-28",
    raw: Buffer.from("hi").toString("base64"),
    doc: { any: ["thing"] },
    status: "paid",
    lines: [{ productId: "CHAI", quantity: "2" }],
    tags: null,
    owner_id: "u1", // the descriptor (snake_case) name is accepted too
    unknown: 1, // ignored
  });
  assert.deepEqual(errors, {});
  assert.equal(value.big, 9007199254740993n);
  assert.ok(Number.isNaN(value.ratio));
  assert.ok(value.amount instanceof Decimal);
  assert.equal(value.amount.toString(), "12.50");
  assert.ok(value.at instanceof Date);
  assert.equal(value.at.toISOString(), "2026-11-08T10:00:00.000Z");
  assert.equal(Buffer.from(value.raw).toString(), "hi");
  assert.deepEqual(value.doc, { any: ["thing"] });
  assert.deepEqual(value.lines, [{ productId: "CHAI", quantity: 2 }]);
  assert.deepEqual(value.tags, []);
  assert.equal(value.ownerId, "u1");
  assert.equal("note" in value, false);
});

test("absent fields read as proto3 defaults; absent timestamps and dates are invalid", () => {
  const { value, errors } = decode(all, {});
  assert.equal(value.text, "");
  assert.equal(value.flag, false);
  assert.equal(value.small, 0);
  assert.equal(value.big, 0n);
  assert.equal(value.amount.toString(), "0");
  assert.equal(value.status, "placed");
  assert.deepEqual(value.lines, []);
  assert.deepEqual(errors, { at: "required", day: "required" });
});

test("reports each bad field by its path", () => {
  const { errors } = decode(all, {
    text: 1,
    small: 2 ** 31,
    big: "1.5",
    uid: "nope",
    day: "2026-02-30",
    status: "lost",
    lines: [{ productId: "A", quantity: 1 }, { productId: 5, quantity: 1.5 }],
    at: "2026-11-08T10:00:00Z",
  });
  assert.deepEqual(errors, {
    text: "expected a string",
    small: "expected a 32-bit integer",
    big: "expected a 64-bit integer",
    uid: "expected a uuid",
    day: "expected a date (YYYY-MM-DD)",
    status: "expected one of placed, paid",
    "lines[1].productId": "expected a string",
    "lines[1].quantity": "expected a 32-bit integer",
  });
  assert.deepEqual(decode(all, []).errors, { $: "expected an object" });
});

test("encodes lowerCamelCase names, int64 as strings, enums by name", () => {
  const out = encodeMessage(all, {
    text: "a",
    flag: false,
    small: 0,
    big: 12n,
    ratio: Infinity,
    amount: Decimal.from("1.10"),
    uid: "0193a5b2-7c1e-7d2a-9f00-000000000001",
    at: new Date("2026-11-08T10:00:00Z"),
    day: "2026-11-08",
    raw: new Uint8Array([104, 105]),
    doc: null,
    status: "paid",
    lines: [{ productId: "CHAI", quantity: 2 }],
    ownerId: "u1",
  });
  assert.deepEqual(out, {
    text: "a",
    flag: false,
    small: 0,
    big: "12",
    ratio: "Infinity",
    amount: "1.10",
    uid: "0193a5b2-7c1e-7d2a-9f00-000000000001",
    at: "2026-11-08T10:00:00.000Z",
    day: "2026-11-08",
    raw: "aGk=",
    doc: null,
    status: "paid",
    lines: [{ productId: "CHAI", quantity: 2 }],
    tags: [],
    ownerId: "u1",
  });
  assert.throws(() => encodeMessage(line, { productId: "A" }), (e) => e instanceof OutputError && e.path === "quantity");
  assert.throws(() => encodeMessage(all, { ...out, status: "lost" }), OutputError);
});

test("an entity's message is its id then its columns; an i64 id is a string", () => {
  const note = entity({ key: "i64", columns: { title: t.text() } });
  assert.deepEqual(encodeMessage(note, { id: 42n, title: "T" }), { id: "42", title: "T" });
  const { value, errors } = decode(note, { id: 42, title: "T" });
  assert.deepEqual(errors, {});
  assert.deepEqual(value, { id: "42", title: "T" });
  assert.deepEqual(decode(note, { id: "x" }).errors, { id: "expected an integer id" });
});

test("Decimal is exact and rounds half away from zero", () => {
  assert.equal(Decimal.from("0.1").add("0.2").toString(), "0.3");
  assert.equal(Decimal.sum(["1.10", "2.205", 3]).toString(), "6.305");
  assert.equal(Decimal.from("19.99").mul(3).toString(), "59.97");
  assert.equal(Decimal.HUNDRED.sub(15).toString(), "85");
  assert.equal(Decimal.from("10").div(4).toString(), "2.5");
  assert.equal(Decimal.from("1").div(3).toString(), "0.3333333333333333333333333333");
  assert.equal(Decimal.from("2").div(3).round(2).toString(), "0.67");
  assert.equal(Decimal.from("2.345").round(2).toString(), "2.35");
  assert.equal(Decimal.from("-2.345").round(2).toString(), "-2.35");
  assert.equal(Decimal.from("2.344").round(2).toString(), "2.34");
  assert.equal(Decimal.from("2.5").round().toString(), "3");
  assert.equal(Decimal.from("-2.5").round().toString(), "-3");
  assert.equal(Decimal.from("1.5").round(3).toString(), "1.500");
  assert.equal(Decimal.from("1e3").toString(), "1000");
  assert.equal(Decimal.from("-0.05").toString(), "-0.05");
  assert.equal(Decimal.from("99999999999999999999999999.99").add("0.01").toString(), "100000000000000000000000000.00");
  // PlaceOrder's total: subtotal × (100 − discount) / 100, to cents
  const total = Decimal.from("123.45").mul(Decimal.HUNDRED.sub(7)).div(Decimal.HUNDRED).round(2);
  assert.equal(total.toString(), "114.81");
  assert.ok(Decimal.from("1.0").eq("1"));
  assert.ok(Decimal.from("1.01").gt(1));
  assert.throws(() => Decimal.from("abc"), SyntaxError);
  assert.throws(() => Decimal.ONE.div(0), RangeError);
  assert.equal(JSON.stringify({ a: Decimal.from("1.20") }), '{"a":"1.20"}');
});

// `omitDefaults` (crates/sdk/tests/json.rs, omit_defaults_leaves_out_empty_fields): a trimmed view's empty fields are
// left out; an enum, an optional field with a value and a message that does not trim are written.
const face = t.message("Face", { id: t.text(), agent: t.bool() }, { omitDefaults: true });
const trimmed = t.message(
  "Trimmed",
  {
    text: t.text(),
    count: t.int(),
    big: t.bigint(),
    weight: t.double(),
    flag: t.bool(),
    tags: t.array(t.text()),
    face,
    faces: t.array(face),
    maybe: t.int().optional(),
    status,
    line: t.message("TrimLine", { productId: t.text(), quantity: t.int(), unitPrice: t.decimal() }),
    raw: t.bytes(),
  },
  { omitDefaults: true },
);

test("omitDefaults leaves out fields at their protobuf default", () => {
  const empty = {
    text: "", count: 0, big: 0n, weight: 0, flag: false, tags: [], face: { id: "", agent: false }, faces: [], maybe: 0, status: "placed",
    line: { productId: "", quantity: 0, unitPrice: Decimal.from("0") }, raw: new Uint8Array(),
  };
  assert.deepEqual(encodeMessage(trimmed, empty), {
    status: "placed",
    maybe: 0,
    line: { productId: "", quantity: 0, unitPrice: "0" },
  });
  // what is left out reads back as the defaults
  const back = decode(trimmed, encodeMessage(trimmed, empty));
  assert.deepEqual(back.errors, {});
  assert.deepEqual({ ...back.value, raw: [...back.value.raw] }, { ...empty, raw: [] });

  const full = { ...empty, text: "hi", count: 2, big: 3n, weight: 0.5, flag: true, tags: ["a"], face: { id: "u-1", agent: false }, faces: [{ id: "", agent: false }] };
  assert.deepEqual(encodeMessage(trimmed, full), {
    text: "hi", count: 2, big: "3", weight: 0.5, flag: true, tags: ["a"], face: { id: "u-1" }, faces: [{}], status: "placed", maybe: 0,
    line: { productId: "", quantity: 0, unitPrice: "0" },
  });
  // an absent optional field stays out; arrays may be left out too
  const { maybe: _m, tags: _t, faces: _f, ...rest } = empty;
  assert.deepEqual(encodeMessage(trimmed, rest), { status: "placed", line: { productId: "", quantity: 0, unitPrice: "0" } });
  // a message without the option writes every field
  assert.deepEqual(encodeMessage(line, { productId: "", quantity: 0 }), { productId: "", quantity: 0 });
});
