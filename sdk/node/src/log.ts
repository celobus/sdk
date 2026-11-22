// Structured logs: one JSON object per line on stderr. What a job's handler logs with `ctx.log` while it runs is also
// kept with the run, by attempt, and shown with it (the run log): its message, then its fields as `key=value`. Lines
// are cut at 2 KiB; a process sends at most 1,000 lines and 1 MiB a second, and drops the rest. Lines logged while
// handling a request are not kept this way.

export type LogLevel = "debug" | "info" | "warn" | "error";
export type LogFields = Record<string, unknown>;

export interface Logger {
  debug(message: string, fields?: LogFields): void;
  info(message: string, fields?: LogFields): void;
  warn(message: string, fields?: LogFields): void;
  error(message: string, fields?: LogFields): void;
  /** A logger that adds these fields to every line. */
  with(fields: LogFields): Logger;
}

function plain(value: unknown): unknown {
  if (typeof value === "bigint") return value.toString();
  if (value instanceof Error) return { name: value.name, message: value.message, stack: value.stack };
  return value;
}

/** @internal Where log lines go; tests replace it. */
export const sink = { write: (line: string) => void process.stderr.write(line) };

export function createLogger(base: LogFields = {}): Logger {
  const write = (level: LogLevel, msg: string, fields?: LogFields) => {
    const line = { ts: new Date().toISOString(), level, msg, ...base, ...fields };
    sink.write(JSON.stringify(line, (_k, v) => plain(v)) + "\n");
  };
  return {
    debug: (m, f) => write("debug", m, f),
    info: (m, f) => write("info", m, f),
    warn: (m, f) => write("warn", m, f),
    error: (m, f) => write("error", m, f),
    with: (fields) => createLogger({ ...base, ...fields }),
  };
}

/** The longest run log line, in UTF-8 bytes. */
export const MAX_LINE = 2048;
/** Run log lines and bytes a process sends a second. */
export const LINES_PER_SECOND = 1_000;
export const BYTES_PER_SECOND = 1 << 20;

/** @internal At most `max` UTF-8 bytes of `s`, cut at a character boundary. */
export function cutUtf8(s: string, max: number): string {
  const bytes = Buffer.from(s, "utf8");
  if (bytes.length <= max) return s;
  let end = max;
  while (end > 0 && (bytes[end]! & 0xc0) === 0x80) end--;
  return bytes.subarray(0, end).toString("utf8");
}

/** @internal A fixed one-second window of lines and bytes. */
export class Limiter {
  private window: number | undefined;
  private lines = 0;
  private bytes = 0;

  /** Whether a line of `bytes` may go at `now` (ms). */
  admit(now: number, bytes: number): boolean {
    if (this.window === undefined || now - this.window >= 1000 || now < this.window) {
      this.window = now;
      this.lines = 0;
      this.bytes = 0;
    }
    if (this.lines >= LINES_PER_SECOND || this.bytes + bytes > BYTES_PER_SECOND) return false;
    this.lines += 1;
    this.bytes += bytes;
    return true;
  }
}

/** @internal The process's run log limit. */
export const runLogLimit = new Limiter();

function fieldValue(v: unknown): string {
  if (typeof v === "string") return JSON.stringify(v);
  if (typeof v === "number" || typeof v === "boolean" || typeof v === "bigint" || v === undefined) return String(v);
  if (v instanceof Error) return JSON.stringify(v.message);
  try {
    return JSON.stringify(v, (_k, x) => (typeof x === "bigint" ? x.toString() : x)) ?? String(v);
  } catch {
    return String(v);
  }
}

/** @internal A run log line: the message, then the fields as `key=value`, cut at 2 KiB. */
export function runLine(message: string, fields: LogFields = {}): string {
  const parts = Object.entries(fields).map(([k, v]) => `${k}=${fieldValue(v)}`);
  const out = parts.length === 0 ? message : message ? `${message} ${parts.join(" ")}` : parts.join(" ");
  return cutUtf8(out, MAX_LINE);
}

/**
 * @internal A run's logger: every line goes to `inner` (stderr) and, within the process's limit, to `send` as a run log
 * line with the fields of the call and of `.with(..)` made on it (not the fields `inner` adds).
 */
export function runLogger(inner: Logger, send: (level: LogLevel, message: string) => void, fields: LogFields = {}): Logger {
  const write = (level: LogLevel) => (msg: string, f?: LogFields) => {
    inner[level](msg, f);
    const line = runLine(msg, { ...fields, ...f });
    if (runLogLimit.admit(Date.now(), Buffer.byteLength(line, "utf8"))) send(level, line);
  };
  return {
    debug: write("debug"),
    info: write("info"),
    warn: write("warn"),
    error: write("error"),
    with: (extra) => runLogger(inner.with(extra), send, { ...fields, ...extra }),
  };
}
