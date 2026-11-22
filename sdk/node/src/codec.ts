// The JSON codec: request bodies to handler input and handler output to response bodies, in the protobuf JSON
// mapping. Field names are the protobuf JSON names (lowerCamelCase: the TypeScript keys); the snake_case
// descriptor names are accepted on input too. int64 is a string (a number is accepted on input), enums are their
// value names, `decimal` is a string, `timestamp` an RFC 3339 string, `date` `YYYY-MM-DD`, `bytes` base64,
// `json` the value itself. Unknown fields are ignored.
//
// Absent fields follow proto3: a required scalar, enum, ref or repeated field that is absent reads as its
// default (`""`, `0`, `0n`, `false`, the first enum value, `[]`), a required message as the message read from `{}`
// (what an `omitDefaults` message leaves out of an empty one); a required timestamp or date that is absent is
// invalid. Output emits every field that has a value, defaults included, except in an `omitDefaults` message.
import { Decimal } from "./decimal.js";
import { jsonName, snake } from "./names.js";
import { EntityDef, Field, MessageDef, PlatformEntity, type AnyField, type KeyType, type RefTarget } from "./schema.js";

/** One field of a message as the codec sees it. */
export interface CodecField {
  key: string; // the TypeScript key
  name: string; // the descriptor name (snake_case)
  json: string; // the protobuf JSON name
  field: AnyField;
}

const models = new WeakMap<object, CodecField[]>();

function modelOf(key: string, field: AnyField): CodecField {
  const name = snake(key);
  return { key, name, json: jsonName(name), field };
}

/** The fields of a message, or of an entity's message (`id` first, then the columns). */
export function messageFields(source: MessageDef<any> | EntityDef<any>): CodecField[] {
  let fields = models.get(source);
  if (!fields) {
    if (source instanceof EntityDef) {
      const id = new Field<string, false>({ kind: "ref", ref: source });
      // a column named `id` is a definition error (E_COL_RESERVED); the implied id stands
      const columns = Object.entries(source.columns).filter(([k]) => k !== "id");
      fields = [modelOf("id", id), ...columns.map(([k, f]) => modelOf(k, f as AnyField))];
    } else {
      fields = Object.entries(source.fields).map(([k, f]) => modelOf(k, f as AnyField));
    }
    models.set(source, fields);
  }
  return fields;
}

/** The key type of a ref's target; `undefined` for the directory and other plugins' entities. */
export function refKey(target: RefTarget): KeyType | undefined {
  if (target instanceof EntityDef) return target.key;
  if (target instanceof PlatformEntity || typeof target === "string") return undefined;
  const resolved = target();
  return resolved instanceof EntityDef ? resolved.key : undefined;
}

/** Collects input errors by path, as `details.fields` of a 400 `invalid`. */
export type FieldErrors = Record<string, string>;

const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
const DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const INT = /^-?\d+$/;
const BASE64 = /^[A-Za-z0-9+/_-]*={0,2}$/;
const INT64_MIN = -(2n ** 63n);
const INT64_MAX = 2n ** 63n - 1n;

const join = (path: string, key: string) => (path ? `${path}.${key}` : key);

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** Decodes a JSON object into a message's TypeScript value; problems go to `errors`. */
export function decodeMessage(
  source: MessageDef<any> | EntityDef<any>,
  json: unknown,
  path: string,
  errors: FieldErrors,
): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  if (!isObject(json)) {
    errors[path || "$"] = "expected an object";
    return out;
  }
  for (const f of messageFields(source)) {
    const raw = f.json in json ? json[f.json] : f.name in json ? json[f.name] : json[f.key];
    const value = decodeField(f.field, raw, join(path, f.json), errors);
    if (value !== undefined) out[f.key] = value;
  }
  return out;
}

/** One value of a (single) field: `undefined` for a JSON null or nothing. */
export function decodeFieldValue(field: AnyField, raw: unknown, path: string, errors: FieldErrors): unknown {
  if (raw === undefined || raw === null) return undefined;
  return decodeValue(field, raw, path, errors);
}

function decodeField(field: AnyField, raw: unknown, path: string, errors: FieldErrors): unknown {
  const { spec } = field;
  if (spec.kind === "array") {
    if (raw === undefined || raw === null) return [];
    if (!Array.isArray(raw)) {
      errors[path] = "expected an array";
      return undefined;
    }
    return raw.map((item, i) => decodeValue(spec.inner!, item, `${path}[${i}]`, errors));
  }
  if (raw === undefined || raw === null) {
    if (field.isOptional) return undefined;
    return defaultValue(field, path, errors);
  }
  return decodeValue(field, raw, path, errors);
}

function defaultValue(field: AnyField, path: string, errors: FieldErrors): unknown {
  const { spec } = field;
  switch (spec.kind) {
    case "enum":
      return spec.enumDef!.values[0];
    case "ref":
      return refKey(spec.ref!) === "i64" ? "0" : "";
    case "message":
      return decodeMessage(spec.message!, {}, path, errors);
    case "scalar":
      switch (spec.scalar) {
        case "string":
        case "uuid":
          return "";
        case "bool":
          return false;
        case "int32":
        case "double":
          return 0;
        case "int64":
          return 0n;
        case "decimal":
          return Decimal.ZERO;
        case "bytes":
          return new Uint8Array();
        case "json":
          return null;
        default:
          errors[path] = "required";
          return undefined;
      }
  }
  return undefined;
}

function decodeValue(field: AnyField, raw: unknown, path: string, errors: FieldErrors): unknown {
  const { spec } = field;
  const fail = (message: string) => {
    errors[path] = message;
    return undefined;
  };
  switch (spec.kind) {
    case "array":
      return fail("nested arrays are not supported");
    case "message":
      return decodeMessage(spec.message!, raw, path, errors);
    case "enum": {
      const values = spec.enumDef!.values;
      return typeof raw === "string" && values.includes(raw) ? raw : fail(`expected one of ${values.join(", ")}`);
    }
    case "ref": {
      const key = refKey(spec.ref!);
      if (key === "i64") {
        if (typeof raw === "number" && Number.isSafeInteger(raw)) return String(raw);
        return typeof raw === "string" && INT.test(raw) ? raw : fail("expected an integer id");
      }
      if (typeof raw !== "string") return fail("expected a string id");
      if (key === "uuid" && !UUID.test(raw)) return fail("expected a uuid");
      return raw;
    }
    case "scalar":
      break;
  }
  switch (spec.scalar!) {
    case "string":
      return typeof raw === "string" ? raw : fail("expected a string");
    case "uuid":
      return typeof raw === "string" && UUID.test(raw) ? raw : fail("expected a uuid");
    case "bool":
      return typeof raw === "boolean" ? raw : fail("expected true or false");
    case "int32": {
      const n = typeof raw === "string" && INT.test(raw) ? Number(raw) : raw;
      return typeof n === "number" && Number.isInteger(n) && n >= -(2 ** 31) && n < 2 ** 31
        ? n
        : fail("expected a 32-bit integer");
    }
    case "int64": {
      let n: bigint | undefined;
      if (typeof raw === "string" && INT.test(raw)) n = BigInt(raw);
      else if (typeof raw === "number" && Number.isSafeInteger(raw)) n = BigInt(raw);
      return n !== undefined && n >= INT64_MIN && n <= INT64_MAX ? n : fail("expected a 64-bit integer");
    }
    case "double": {
      if (typeof raw === "number") return raw;
      if (raw === "NaN") return NaN;
      if (raw === "Infinity") return Infinity;
      if (raw === "-Infinity") return -Infinity;
      const n = typeof raw === "string" && raw.trim() !== "" ? Number(raw) : NaN;
      return Number.isNaN(n) ? fail("expected a number") : n;
    }
    case "decimal":
      if (typeof raw !== "string" && typeof raw !== "number") return fail("expected a decimal string");
      try {
        return Decimal.from(raw);
      } catch {
        return fail("expected a decimal string");
      }
    case "timestamp": {
      if (typeof raw !== "string") return fail("expected an RFC 3339 timestamp");
      const d = new Date(raw);
      return Number.isNaN(d.getTime()) ? fail("expected an RFC 3339 timestamp") : d;
    }
    case "date":
      return typeof raw === "string" && validDate(raw) ? raw : fail("expected a date (YYYY-MM-DD)");
    case "bytes":
      return typeof raw === "string" && BASE64.test(raw) ? Buffer.from(raw, "base64") : fail("expected base64");
    case "json":
      return raw;
  }
}

function validDate(s: string): boolean {
  const m = DATE.exec(s);
  if (!m) return false;
  const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])));
  return d.getUTCFullYear() === Number(m[1]) && d.getUTCMonth() === Number(m[2]) - 1 && d.getUTCDate() === Number(m[3]);
}

/** A handler answered something that does not fit its output type: a 500 for the caller, logged with the path. */
export class OutputError extends Error {
  constructor(
    readonly path: string,
    message: string,
  ) {
    super(`output ${path}: ${message}`);
    this.name = "OutputError";
  }
}

/** Encodes a handler's output value as a JSON object. */
export function encodeMessage(source: MessageDef<any> | EntityDef<any>, value: unknown, path = ""): Record<string, unknown> {
  if (!isObject(value)) throw new OutputError(path || "$", "expected an object");
  const out: Record<string, unknown> = {};
  // `omitDefaults`: a field at its protobuf default is left out (an optional one is written whenever it has a value)
  const trim = source instanceof MessageDef && source.omitDefaults;
  for (const f of messageFields(source)) {
    const v = value[f.key];
    const p = join(path, f.key);
    if (f.field.spec.kind === "array") {
      if (v === undefined || v === null) {
        if (!trim) out[f.json] = [];
      } else if (!Array.isArray(v)) throw new OutputError(p, "expected an array");
      else if (!(trim && v.length === 0)) out[f.json] = v.map((item, i) => encodeValue(f.field.spec.inner!, item, `${p}[${i}]`));
      continue;
    }
    if (v === null && f.field.spec.scalar === "json") {
      out[f.json] = null;
      continue;
    }
    if (v === undefined || v === null) {
      if (f.field.isOptional) continue;
      throw new OutputError(p, "a required field is missing");
    }
    const encoded = encodeValue(f.field, v, p);
    if (trim && !f.field.isOptional && isDefault(f.field, encoded)) continue;
    out[f.json] = encoded;
  }
  return out;
}

/**
 * The protobuf default of a field's JSON that an `omitDefaults` message leaves out: `""` of a string, `0` of a number,
 * `false`, empty bytes, a message whose JSON is `{}`. Never an enum (its first value is written), a uuid, a timestamp,
 * a date, a decimal, a json value or a ref.
 */
function isDefault(field: AnyField, json: unknown): boolean {
  const { spec } = field;
  if (spec.kind === "message") return isObject(json) && Object.keys(json).length === 0;
  if (spec.kind !== "scalar") return false;
  switch (spec.scalar) {
    case "string":
    case "bytes":
      return json === "";
    case "bool":
      return json === false;
    case "int32":
    case "double":
      return json === 0;
    case "int64":
      return json === "0" || json === "-0";
    default:
      return false;
  }
}

function encodeValue(field: AnyField, v: unknown, path: string): unknown {
  const { spec } = field;
  const fail = (message: string): never => {
    throw new OutputError(path, message);
  };
  switch (spec.kind) {
    case "array":
      return fail("nested arrays are not supported");
    case "message":
      return encodeMessage(spec.message!, v, path);
    case "enum":
      return typeof v === "string" && spec.enumDef!.values.includes(v) ? v : fail(`not a value of ${spec.enumDef!.name}`);
    case "ref":
      if (typeof v === "string") return v;
      if (typeof v === "bigint" || (typeof v === "number" && Number.isSafeInteger(v))) return String(v);
      return fail("expected an id");
    case "scalar":
      break;
  }
  switch (spec.scalar!) {
    case "string":
    case "uuid":
      return typeof v === "string" ? v : fail("expected a string");
    case "bool":
      return typeof v === "boolean" ? v : fail("expected a boolean");
    case "int32":
      return typeof v === "number" && Number.isInteger(v) ? v : fail("expected an integer");
    case "int64":
      if (typeof v === "bigint") return v.toString();
      if (typeof v === "number" && Number.isSafeInteger(v)) return String(v);
      return typeof v === "string" && INT.test(v) ? v : fail("expected a bigint");
    case "double":
      if (typeof v !== "number") return fail("expected a number");
      return Number.isFinite(v) ? v : Number.isNaN(v) ? "NaN" : v > 0 ? "Infinity" : "-Infinity";
    case "decimal":
      try {
        return Decimal.from(v as Decimal).toString();
      } catch {
        return fail("expected a Decimal");
      }
    case "timestamp": {
      const d = v instanceof Date ? v : typeof v === "string" ? new Date(v) : undefined;
      return d && !Number.isNaN(d.getTime()) ? d.toISOString() : fail("expected a Date");
    }
    case "date":
      if (v instanceof Date && !Number.isNaN(v.getTime())) return v.toISOString().slice(0, 10);
      return typeof v === "string" && validDate(v) ? v : fail("expected a YYYY-MM-DD date");
    case "bytes":
      return v instanceof Uint8Array ? Buffer.from(v.buffer, v.byteOffset, v.byteLength).toString("base64") : fail("expected a Uint8Array");
    case "json":
      return v;
  }
}

/** @internal One value of a field in the protobuf JSON mapping (for filters and patches). */
export function encodeFieldValue(field: AnyField, v: unknown, path: string): unknown {
  return encodeValue(field, v, path);
}

/** JSON.stringify that writes a bigint as a string (for raw calls and logs). */
export function stringify(value: unknown): string {
  return JSON.stringify(value, (_k, v) => (typeof v === "bigint" ? v.toString() : v));
}
