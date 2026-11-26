// The settings in the descriptor: the options of a settings field as the descriptor spells them, the form (derived
// from the fields, or laid out by the plugin and completed from them) and the definition checks of settings. The same
// rules every SDK follows, so each prints the same descriptor; `celobus plugin check` checks the rest (a default
// against its own validators, a pattern both regular expression engines read alike).
import { Decimal } from "./decimal.js";
import type { DescriptorField } from "./descriptor.js";
import type { AnyField } from "./schema.js";

/** The most fields the settings message has, and the most secrets among them. */
const MAX_SETTINGS_FIELDS = 64;
const MAX_SECRETS = 16;
/** The largest default, as JSON. */
const MAX_VALUE_BYTES = 64 << 10;

/** The descriptor's `settings`. */
export interface DescribedSettings {
  message: string;
  on_change: boolean;
  form: { sections: FormSection[] };
}

export interface FormSection {
  title?: unknown;
  inputs: Record<string, unknown>[];
}

/** The keys a settings field may carry in the descriptor, each written only when declared. */
export interface SettingsKeys {
  label?: string;
  default?: unknown;
  section?: string;
  placeholder?: string;
  multiline?: true;
  min?: number | string;
  max?: number | string;
  min_len?: number;
  max_len?: number;
  pattern?: string;
  format?: "url" | "email";
  min_items?: number;
  max_items?: number;
  message?: string;
}

type Err = (code: string, path: string, message: string, help?: string) => void;

/** One value of a scalar type as the descriptor spells it (numbers as numbers, decimals and dates as text). */
function spell(type: string, v: unknown): unknown {
  switch (type) {
    case "int32":
    case "int64":
      if (typeof v === "bigint") return v >= BigInt(Number.MIN_SAFE_INTEGER) && v <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(v) : undefined;
      return typeof v === "number" && Number.isSafeInteger(v) ? v : undefined;
    case "double":
      if (typeof v === "bigint") return Number(v);
      return typeof v === "number" && Number.isFinite(v) ? v : undefined;
    case "decimal":
      if (v instanceof Decimal) return v.toString();
      if (typeof v === "string") return v;
      if (typeof v === "number" && Number.isFinite(v)) return String(v);
      return typeof v === "bigint" ? v.toString() : undefined;
    case "bool":
      return typeof v === "boolean" ? v : undefined;
    default:
      // string, date, secret, an enum's value, a member's id
      return typeof v === "string" ? v : undefined;
  }
}

function count(n: unknown): number | undefined {
  return typeof n === "number" && Number.isInteger(n) && n >= 0 && n <= 4_294_967_295 ? n : undefined;
}

/**
 * The options of a settings field (`field`, the declared one; `type` and `repeated` as the descriptor has them) as the
 * descriptor spells them; a value that is not spelled as its type is reported.
 */
export function settingsKeys(field: AnyField, type: string, repeated: boolean, path: string, err: Err): SettingsKeys {
  const s = field.settings;
  const out: SettingsKeys = {};
  if (s.label !== undefined) out.label = s.label;
  if (s.hasDefault) {
    const v = s.default;
    const d = repeated ? (Array.isArray(v) ? v.map((x) => spell(type, x)) : undefined) : spell(type, v);
    if (d === undefined || (Array.isArray(d) && d.includes(undefined))) {
      err("E_SETTINGS_DEFAULT", `${path}.default`, `the default is not a value of ${repeated ? `a list of ${type}` : type}`);
    } else out.default = d;
  }
  if (s.section !== undefined) out.section = s.section;
  if (s.placeholder !== undefined) out.placeholder = s.placeholder;
  if (s.multiline) out.multiline = true;
  for (const [what, has, v] of [["min", s.hasMin, s.min], ["max", s.hasMax, s.max]] as const) {
    if (!has) continue;
    const b = spell(type, v);
    if (typeof b === "number" || typeof b === "string") out[what] = b;
    else out[what] = (typeof v === "bigint" ? Number(v) : v) as number;
  }
  for (const [what, key, v] of [
    ["minLen", "min_len", s.minLen], ["maxLen", "max_len", s.maxLen], ["minItems", "min_items", s.minItems], ["maxItems", "max_items", s.maxItems],
  ] as const) {
    if (v === undefined) continue;
    const n = count(v);
    if (n === undefined) err("E_SETTINGS_VALIDATOR", `${path}.${key}`, `${what}(${String(v)}): a count from 0 to 4294967295`);
    else out[key] = n;
  }
  if (s.pattern !== undefined) out.pattern = s.pattern;
  if (s.format !== undefined) out.format = s.format;
  if (s.message !== undefined) out.message = s.message;
  return out;
}

// ------------------------------------------------------------------------------------------------ kinds and controls

type Kind = "string" | "bool" | "int32" | "int64" | "double" | "decimal" | "date" | "secret" | "enum" | "user";

/** The kind of a settings field; `undefined` for a type settings do not take. */
function kindOf(f: DescriptorField, enums: { name: string; values: string[] }[]): Kind | undefined {
  let k: Kind;
  if (["string", "bool", "int32", "int64", "double", "decimal", "date", "secret"].includes(f.type)) k = f.type as Kind;
  else if (f.type.startsWith("enum:") && enums.some((e) => e.name === f.type.slice(5))) k = "enum";
  else if (f.type === "ref:platform.user") k = "user";
  else return undefined;
  if (f.repeated && (f.optional || !["string", "enum", "user"].includes(k))) return undefined;
  return k;
}

/** The controls a settings field may be filled with; the first is the one a derived form uses. */
function controls(k: Kind, repeated: boolean): string[] {
  switch (k) {
    case "string":
      return repeated ? ["textarea"] : ["text", "textarea"];
    case "secret":
      return ["secret"];
    case "bool":
      return ["switch", "checkbox"];
    case "date":
      return ["date"];
    case "enum":
      return repeated ? ["multi_select"] : ["select", "radio_cards"];
    case "user":
      return ["member"];
    default:
      return ["number"];
  }
}

/** Whether a field needs a value before the plugin works: neither optional nor a list, and without a default. */
function isRequired(f: DescriptorField & SettingsKeys): boolean {
  return !f.optional && !f.repeated && f.default === undefined;
}

/** `thread_warm_hours` → `Thread warm hours`. */
function humanize(name: string): string {
  const s = name.replace(/_/g, " ");
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function enumOptions(f: DescriptorField, enums: { name: string; values: string[] }[]): { value: string; label: string }[] {
  if (!f.type.startsWith("enum:")) return [];
  return (enums.find((e) => e.name === f.type.slice(5))?.values ?? []).map((v) => ({ value: v, label: humanize(v) }));
}

/** What every input of a settings form carries from its field: `required`, and its help, placeholder, options. */
function fillInput(input: Record<string, unknown>, f: DescriptorField & SettingsKeys, enums: { name: string; values: string[] }[]): void {
  if (isRequired(f)) input.required = true;
  else delete input.required;
  if (input.help === undefined && f.description !== "") input.help = f.description;
  if (input.placeholder === undefined && f.placeholder !== undefined) input.placeholder = f.placeholder;
  const options = input.options;
  if ((options === undefined || (Array.isArray(options) && options.length === 0)) && ["select", "radio_cards", "multi_select"].includes(input.control as string)) {
    const o = enumOptions(f, enums);
    if (o.length > 0) input.options = o;
    else delete input.options;
  }
}

/**
 * The form of a settings message the plugin lays out no form for: its fields in declaration order, in sections by
 * their `section` (in the order each first appears; the fields without one in an untitled section, where the first of
 * them appears).
 */
export function deriveForm(fields: (DescriptorField & SettingsKeys)[], enums: { name: string; values: string[] }[]): FormSection[] {
  const sections: FormSection[] = [];
  const keys: (string | undefined)[] = [];
  for (const f of fields) {
    const k = kindOf(f, enums);
    if (k === undefined) continue;
    const control = f.multiline && k === "string" && !f.repeated ? "textarea" : controls(k, f.repeated)[0];
    const input: Record<string, unknown> = { name: f.name, control, label: f.label ?? humanize(f.name) };
    fillInput(input, f, enums);
    let at = keys.indexOf(f.section);
    if (at < 0) {
      keys.push(f.section);
      sections.push(f.section !== undefined ? { title: f.section, inputs: [] } : { inputs: [] });
      at = sections.length - 1;
    }
    sections[at].inputs.push(input);
  }
  return sections;
}

/** A form the plugin laid out, its inputs filled from their fields as `deriveForm` fills them. */
export function completeForm(sections: FormSection[], fields: (DescriptorField & SettingsKeys)[], enums: { name: string; values: string[] }[]): FormSection[] {
  for (const s of sections) {
    for (const i of s.inputs) {
      const f = fields.find((x) => x.name === i.name);
      if (f) fillInput(i, f, enums);
    }
  }
  return sections;
}

// ------------------------------------------------------------------------------------------------ checks

const NUMERIC: Kind[] = ["int32", "int64", "double", "decimal", "date"];
const DATE = /^\d{4}-\d{2}-\d{2}$/;

function compare(k: Kind, a: unknown, b: unknown): number | undefined {
  try {
    if (k === "decimal") return Decimal.from(a as string).cmp(Decimal.from(b as string));
    if (k === "date") return typeof a === "string" && typeof b === "string" ? (a < b ? -1 : a > b ? 1 : 0) : undefined;
    return typeof a === "number" && typeof b === "number" ? a - b : undefined;
  } catch {
    return undefined;
  }
}

/** Whether a bound is spelled as its kind is (numbers as numbers; decimals and dates as text). */
function boundOk(k: Kind, v: unknown): boolean {
  if (k === "date") return typeof v === "string" && DATE.test(v);
  if (k === "decimal") {
    if (typeof v !== "string") return false;
    try {
      Decimal.from(v);
      return true;
    } catch {
      return false;
    }
  }
  if (k === "int32" || k === "int64") return typeof v === "number" && Number.isInteger(v);
  return typeof v === "number";
}

/** A settings field's own checks: its type, its validators against its type, its default, its pattern. */
function fieldErrors(f: DescriptorField & SettingsKeys, enums: { name: string; values: string[] }[], path: string, err: Err): void {
  const k = kindOf(f, enums);
  if (k === undefined) {
    err("E_SETTINGS_FIELD_TYPE", path,
      `${f.name} is ${f.repeated ? `a list of ${f.type}` : f.type}: settings take string, bool, int32, int64, double, decimal, date, secret, an enum or a member, optional or not, and lists of strings, enums or members`);
    return;
  }
  const bad = (what: string, why: string) => err("E_SETTINGS_VALIDATOR", `${path}.${what}`, why);
  const list = f.repeated;
  if ((f.min !== undefined || f.max !== undefined) && !(NUMERIC.includes(k) && !list)) {
    bad("min", `min and max bound a number, a decimal or a date; ${f.name} is not one`);
  } else if (NUMERIC.includes(k)) {
    for (const [what, b] of [["min", f.min], ["max", f.max]] as const) {
      if (b !== undefined && !boundOk(k, b)) bad(what, `${what} ${JSON.stringify(b)} is not a value of ${f.type}`);
    }
    if (f.min !== undefined && f.max !== undefined && (compare(k, f.min, f.max) ?? 0) > 0) bad("min", `min ${f.min} is above max ${f.max}`);
  }
  const stringy = k === "string" || k === "secret";
  if ((f.min_len !== undefined || f.max_len !== undefined) && !stringy) {
    bad(f.min_len !== undefined ? "min_len" : "max_len", `min_len and max_len bound a string or a secret; ${f.name} is not one`);
  }
  if (f.min_len !== undefined && f.max_len !== undefined && f.min_len > f.max_len) bad("min_len", `min_len ${f.min_len} is above max_len ${f.max_len}`);
  if (f.pattern !== undefined && k !== "string") bad("pattern", `a pattern checks a string; ${f.name} is not one`);
  else if (f.pattern !== undefined) {
    try {
      new RegExp(f.pattern, "u");
    } catch (e) {
      err("E_SETTINGS_PATTERN", `${path}.pattern`, `${JSON.stringify(f.pattern)}: ${(e as Error).message}`,
        "use literals, classes, \\d, \\w, groups, alternation and repetition; anchor with ^ and $");
    }
  }
  if (f.format !== undefined && k !== "string") bad("format", `url and email check a string; ${f.name} is not one`);
  if ((f.min_items !== undefined || f.max_items !== undefined) && !list) {
    bad("min_items", `min_items and max_items bound a list; ${f.name} is not one`);
  }
  if (f.min_items !== undefined && f.max_items !== undefined && f.min_items > f.max_items) {
    bad("min_items", `min_items ${f.min_items} is above max_items ${f.max_items}`);
  }
  if (f.multiline && !(k === "string" && !list)) bad("multiline", `multiline belongs to a string; ${f.name} is not one`);
  if (f.default !== undefined) {
    if (k === "secret") err("E_SETTINGS_DEFAULT", `${path}.default`, "a secret has no default: an owner or an admin sets it");
    else if (JSON.stringify(f.default).length > MAX_VALUE_BYTES) err("E_SETTINGS_LIMIT", `${path}.default`, "the default is larger than 64 KiB");
    else if (k === "enum") {
      const values = enums.find((e) => e.name === f.type.slice(5))?.values ?? [];
      const given = (list ? f.default : [f.default]) as unknown[];
      const wrong = given.find((v) => !values.includes(v as string));
      if (wrong !== undefined) err("E_SETTINGS_DEFAULT", `${path}.default`, `the default ${JSON.stringify(wrong)} is not a value of ${f.type}`);
    }
  }
}

/** The keys a settings input may not carry. */
const NOT_ON_SETTINGS = ["options_from", "pick", "fields", "aside", "initial"];

/**
 * The definition checks of settings: the options of a setting and the secret type belong to the settings message's
 * fields (`messages`: every message, the settings one named `settingsMessage`); the settings message's fields and form.
 */
export function settingsErrors(
  messages: { name: string; kind: string; fields: (DescriptorField & SettingsKeys)[] }[],
  enums: { name: string; values: string[] }[],
  settings: DescribedSettings | undefined,
  err: Err,
): void {
  for (const m of messages) {
    if (settings !== undefined && m.name === settings.message) continue;
    for (const f of m.fields) {
      const path = `messages.${m.name}.${f.name}`;
      const { label, default: d, section, placeholder, multiline, min, max, min_len, max_len, pattern, format, min_items, max_items, message } = f;
      if ([label, d, section, placeholder, multiline, min, max, min_len, max_len, pattern, format, min_items, max_items, message].some((x) => x !== undefined)) {
        err("E_FIELD_VALIDATOR_SCOPE", path,
          "label, default, section, placeholder, multiline, validators and message belong to the fields of the settings message",
          "declare it with t.message(.., { kind: 'settings' }) and plugin({ settings }), or check an endpoint's input in its handler");
      }
      if (f.type === "secret") {
        err("E_SETTINGS_FIELD_TYPE", path, "secret is the type of a settings field only",
          "keep the value in the plugin's settings, or use t.text().encrypted() on an entity");
      }
    }
  }
  if (settings === undefined) return;
  const m = messages.find((x) => x.name === settings.message);
  if (m === undefined) return;
  const base = `messages.${m.name}`;
  if (m.fields.length > MAX_SETTINGS_FIELDS) err("E_SETTINGS_LIMIT", base, `${m.fields.length} fields; settings have at most ${MAX_SETTINGS_FIELDS}`);
  const secrets = m.fields.filter((f) => f.type === "secret").length;
  if (secrets > MAX_SECRETS) err("E_SETTINGS_LIMIT", base, `${secrets} secrets; settings have at most ${MAX_SECRETS}`);
  for (const f of m.fields) fieldErrors(f, enums, `${base}.${f.name}`, err);

  // the form: every field once, each with a control its type takes
  const seen: string[] = [];
  settings.form.sections.forEach((s, si) => {
    s.inputs.forEach((i, ii) => {
      const p = `settings.form.sections[${si}].inputs[${ii}]`;
      const name = String(i.name);
      const f = m.fields.find((x) => x.name === name);
      if (f === undefined) {
        err("E_SETTINGS_FORM_FIELD", p, `${m.name} has no field ${name}`);
        return;
      }
      if (seen.includes(name)) err("E_SETTINGS_FORM_FIELD", p, `${name} is in the form twice`);
      seen.push(name);
      if (NOT_ON_SETTINGS.some((k) => i[k] !== undefined)) {
        err("E_SETTINGS_CONTROL", p, "a settings input takes no options_from, pick, fields, aside or initial (a field's default is its initial value)");
      }
      const k = kindOf(f, enums);
      if (k === undefined) return;
      const allowed = controls(k, f.repeated);
      if (!allowed.includes(String(i.control))) {
        err("E_SETTINGS_CONTROL", `${p}.control`, `${name} (${f.repeated ? `a list of ${f.type}` : f.type}) is filled with ${allowed.join(" or ")}, not ${String(i.control)}`);
      }
      const options = Array.isArray(i.options) ? (i.options as { value: unknown }[]) : [];
      if (k === "enum") {
        const values = enums.find((e) => e.name === f.type.slice(5))?.values ?? [];
        for (const o of options) {
          if (!values.includes(o.value as string)) err("E_SETTINGS_CONTROL", `${p}.options`, `option ${JSON.stringify(o.value)} is not a value of ${f.type}`);
        }
      } else if (options.length > 0) {
        err("E_SETTINGS_CONTROL", `${p}.options`, `${name} is not an enum: its input takes no options`);
      }
    });
  });
  for (const f of m.fields) {
    if (!seen.includes(f.name)) {
      err("E_SETTINGS_FORM_FIELD", "settings.form", `${f.name} is not in the form`, "a settings form has every field of the settings message once");
    }
  }
}
