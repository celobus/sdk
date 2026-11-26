// Plugin settings: what an owner or an admin of the workspace sets for the installed plugin (a limit, a webhook, an API
// key). The plugin declares them as one message of kind `settings` — its fields' options (`.label()`, `.default()`,
// `.min()`, ..) give the form and the validators — and reads them with `ctx.settings(schema)`:
//
//   const chatSettings = t.message('ChatSettings', {
//     threadWarmHours: t.int().label('Thread stays with its agent, hours').default(2).min(1).max(48),
//     webhookUrl: t.text().optional().label('Webhook URL').url().section('Integrations'),
//     webhookToken: t.secret().optional().section('Integrations'),
//   }, { kind: 'settings' });
//   export default plugin({ id: 'chat', settings: settingsDef(chatSettings, { onChange: check }), .. });
//   // in a handler
//   const s = ctx.settings(chatSettings);   // s.threadWarmHours: number, s.webhookToken?: Secret
import { decodeMessage, encodeFieldValue, messageFields, type FieldErrors } from "./codec.js";
import type { Ctx } from "./ctx.js";
import { RpcError, internal } from "./errors.js";
import { noted } from "./keys.js";
import { MessageDef, type AnyField, type Infer, type Shape } from "./schema.js";
import type { FormB } from "./ui.js";

/** A change of the settings the plugin checks before it is saved (`settingsDef(.., { onChange })`). */
export interface SettingsChange<V> {
  /** The settings before the change; `undefined` when they did not read (a required value was never set). */
  previous?: V;
  /** The settings to be saved, as `ctx.settings(..)` would read them. */
  current: V;
}

/** The options of `settingsDef(schema, options)`. */
export interface SettingsOptions<S extends Shape> {
  /**
   * Checks a change before it is saved, for what validators cannot say (fields that go together, an outside service
   * that must answer). It runs as a system request (no caller: it reads the plugin's own data and calls out) within 10
   * seconds. Returning lets the change be saved; throwing refuses it: `invalid(..).field('webhook_url', "Didn't answer")`
   * shows each message under its field (by its name or its key), any other error its message.
   */
  onChange?: (ctx: Ctx, change: SettingsChange<Infer<S>>) => void | Promise<void>;
  /**
   * Lays the form out by hand: `(f) => { f.section('Agents'); f.number(chatSettings.f.threadWarmHours, 'Hours'); .. }`,
   * every field once. Each input still gets its field's help (its description), placeholder, whether it is required and
   * an enum's options, unless it says its own. Without it the form follows the message: its fields in order, in
   * sections by their `section`; an enum is a select, a list of enum values a multi-select, a bool a switch, a number
   * a number, a `multiline` text a text area, a list of texts a text area (one per line), a secret a secret input, a
   * ref to `platform.user` a member.
   */
  form?: (f: FormB<Infer<S>>) => unknown;
}

/** The plugin's settings, as `settingsDef(..)` declares them for `plugin({ settings })`. */
export class SettingsDef<S extends Shape = Shape> {
  constructor(
    /** The message of kind `settings` whose fields the settings are. */
    readonly schema: MessageDef<S>,
    readonly options: SettingsOptions<S> = {},
  ) {}
}

/**
 * The plugin's settings for `plugin({ settings })`: their message (`t.message(.., { kind: 'settings' })`), with an
 * optional check of each change (`onChange`) and an optional form laid out by hand (`form`).
 */
export function settingsDef<S extends Shape>(schema: MessageDef<S>, options: SettingsOptions<S> = {}): SettingsDef<S> {
  return noted(new SettingsDef(schema, options), "settings", options);
}

/** @internal A field's declared default in the plugin's JSON mapping (int64 as a string), `undefined` without one. */
export function defaultJson(field: AnyField, path: string): unknown {
  if (!field.settings.hasDefault) return undefined;
  const v = field.settings.default;
  if (field.spec.kind === "array") {
    if (!Array.isArray(v)) return undefined;
    return v.map((x, i) => encodeFieldValue(field.spec.inner!, x, `${path}[${i}]`));
  }
  return encodeFieldValue(field, v, path);
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/**
 * Reads settings JSON (the plugin's JSON mapping) as the message: the declared defaults fill what is absent; a required
 * field with no value is a 409 `settings_required` naming it; a value that does not read is a 500.
 */
export function readSettings<S extends Shape>(schema: MessageDef<S>, json: unknown): Infer<S> {
  if (schema.kind !== "settings") {
    throw internal(new Error(`${schema.name} is not a settings message: declare it with t.message(.., { kind: 'settings' })`));
  }
  if (json !== null && json !== undefined && !isObject(json)) throw internal(new Error("the settings are not a JSON object"));
  const obj: Record<string, unknown> = { ...(json ?? {}) };
  const missing: string[] = [];
  for (const f of messageFields(schema)) {
    const present = [f.json, f.name].some((k) => obj[k] !== undefined && obj[k] !== null);
    if (present) continue;
    if (f.field.settings.hasDefault) {
      try {
        obj[f.json] = defaultJson(f.field, f.key);
      } catch (e) {
        throw internal(e);
      }
    } else if (!f.field.isOptional && f.field.spec.kind !== "array") {
      missing.push(f.name);
    }
  }
  if (missing.length > 0) {
    const fields = Object.fromEntries(missing.map((m) => [m, "Required"]));
    throw new RpcError(
      409,
      "settings_required",
      `The plugin needs setup: an owner or an admin sets ${missing.join(", ")} in its settings`,
      { fields, missing },
    );
  }
  const errors: FieldErrors = {};
  const value = decodeMessage(schema, obj, "", errors);
  const problems = Object.entries(errors).map(([p, m]) => `${p}: ${m}`);
  if (problems.length > 0) throw internal(new Error(`the settings do not read as ${schema.name}: ${problems.join("; ")}`));
  return Object.freeze(value) as Infer<S>;
}

/** @internal The check hook's body (`{previous, current}`) as a `SettingsChange`. */
export function settingsChangeOf<S extends Shape>(schema: MessageDef<S>, body: unknown): SettingsChange<Infer<S>> {
  const b = isObject(body) ? body : {};
  const current = readSettings(schema, b.current ?? null);
  let previous: Infer<S> | undefined;
  if (isObject(b.previous)) {
    try {
      previous = readSettings(schema, b.previous);
    } catch {
      previous = undefined;
    }
  }
  return previous === undefined ? { current } : { previous, current };
}

/**
 * @internal The settings a process holds: the latest JSON the platform sent (or a test gave), and the value last read
 * from it.
 */
export class SettingsCache {
  private rev = 0n;
  private json: Record<string, unknown> | undefined;
  private read: { schema: MessageDef<any>; value: unknown } | undefined;

  constructor(
    /** the settings message the plugin declares (`undefined`: none) */
    readonly declared: MessageDef<any> | undefined,
    json?: Record<string, unknown>,
  ) {
    this.json = json;
  }

  /**
   * Takes settings from the platform: at registration always, after it only a revision not older than the held one.
   * Empty bytes are no settings. Whether they were taken.
   */
  update(rev: bigint | number | string, bytes: Uint8Array | string | null | undefined, registered: boolean): boolean {
    const r = BigInt(rev ?? 0);
    const text = typeof bytes === "string" ? bytes : Buffer.from(bytes ?? []).toString("utf8");
    let json: Record<string, unknown> | undefined;
    if (text.trim() !== "") {
      try {
        const v = JSON.parse(text) as unknown;
        json = isObject(v) ? v : {};
      } catch {
        return false;
      }
    }
    if (!registered && r < this.rev) return false;
    this.rev = r;
    this.json = json;
    this.read = undefined;
    return true;
  }

  /** The settings read as `schema`: the defaults when the platform sent none. */
  get<S extends Shape>(schema: MessageDef<S>): Infer<S> {
    if (this.declared === undefined) {
      throw internal(new Error(`the plugin declares no settings: plugin({ settings: settingsDef(${schema.name}) })`));
    }
    if (this.declared !== schema) {
      throw internal(new Error(`${schema.name} is not the plugin's settings: plugin({ settings }) declares ${this.declared.name}`));
    }
    if (this.read?.schema === schema) return this.read.value as Infer<S>;
    const value = readSettings(schema, this.json ?? {});
    this.read = { schema, value };
    return value;
  }
}
