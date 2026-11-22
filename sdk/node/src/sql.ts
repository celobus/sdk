// SQL sent as text (`ctx.sql`, `ctx.unsafeSql`): the query with positional parameters
// (`$1` is the first value), its values as a JSON array, the rows back as JSON objects keyed by column name.
import { internal } from "./errors.js";

/**
 * One row of `ctx.sql`: its columns by the names SQL gives them (`owner_id`, `notes` for
 * `select owner_id, count(*) as notes ..`). A bigint is a JSON number (exact up to 2^53), a timestamp its text.
 */
export type SqlRow = Record<string, unknown>;

/**
 * The values of `$1`, `$2`, … as the JSON array the platform reads: a `bigint` is its decimal text and a `Date` its ISO
 * text (a value carries no type of its own, so `'42'` and `42` are the same),
 * `undefined` is null, an object or array its JSON.
 */
export function sqlParams(params: readonly unknown[]): Buffer {
  try {
    const text = JSON.stringify(params.map((v) => (v === undefined ? null : v)), (_key, v: unknown) =>
      typeof v === "bigint" ? v.toString() : v,
    );
    return Buffer.from(text, "utf8");
  } catch (e) {
    throw internal(new Error(`ctx.sql(): a parameter is not JSON: ${(e as Error).message}`));
  }
}

/** The rows of a `DbRows` answer, each a JSON object. */
export function sqlRows(rows: readonly (Buffer | Uint8Array)[]): SqlRow[] {
  return rows.map((raw) => {
    let row: unknown;
    try {
      row = JSON.parse(Buffer.from(raw).toString("utf8"));
    } catch (e) {
      throw internal(new Error(`the platform sent a row that is not JSON: ${(e as Error).message}`));
    }
    if (row === null || typeof row !== "object" || Array.isArray(row)) {
      throw internal(new Error("the platform sent a row that is not a JSON object"));
    }
    return row as SqlRow;
  });
}
