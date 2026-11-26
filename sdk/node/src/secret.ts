// A setting's secret (`t.secret()`): the value is read with `expose()` only; printing, inspecting or serializing it
// shows `Secret(••••)`.
import { inspect } from "node:util";

const MASK = "Secret(••••)";

/**
 * A secret setting (an API key, a token): a string the plugin reads with `expose()`, kept encrypted by the platform and
 * never shown again once saved. `String(s)`, `JSON.stringify(s)` and `console.log(s)` print `Secret(••••)`, so it stays
 * out of logs and answers; on the wire it is a plain string.
 */
export class Secret {
  readonly #value: string;

  constructor(value: string) {
    this.#value = value;
  }

  /** The secret itself: pass it where it is needed, never into a log or an answer. */
  expose(): string {
    return this.#value;
  }

  toString(): string {
    return MASK;
  }

  toJSON(): string {
    return MASK;
  }

  [inspect.custom](): string {
    return MASK;
  }
}
