// Runtime errors of a handler, mapped to an HTTP status and the platform error body
// `{ "code": "...", "message": "...", "details": {...} }`. Anything else a handler throws is 500 `internal`,
// with its message hidden from the caller and written to the log. In a job's handler an error fails the attempt, and
// the run is retried unless the error is permanent (`permanent(..)`, `.asPermanent()`, or an input-class code).
import { fieldKeys, type AnyField } from "./schema.js";

export type ErrorDetails = Record<string, unknown>;

/** The codes of input-class errors: a job's attempt failing with one of them is not retried. */
const PERMANENT_CODES = new Set(["invalid", "not_found", "permission_denied", "payload_invalid"]);

/** An error with a status, a stable code and details: what the caller receives. */
export class RpcError extends Error {
  private marked = false;

  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly details: ErrorDetails = {},
    options?: { cause?: unknown },
  ) {
    super(message, options);
    this.name = "RpcError";
  }

  /** The same error, marked permanent: a job's run that fails with it is not retried. */
  asPermanent(): this {
    this.marked = true;
    return this;
  }

  /**
   * Whether a job's run that fails with this error is not retried: marked permanent, or an input-class error
   * (`invalid`, `not_found`, `permission_denied`, `payload_invalid`).
   */
  get permanent(): boolean {
    return this.marked || PERMANENT_CODES.has(this.code);
  }

  /** The response body. */
  toJSON(): { code: string; message: string; details: ErrorDetails } {
    return { code: this.code, message: this.message, details: this.details };
  }
}

/** 400 `invalid`, with the offending fields in `details.fields` (`{ "<path>": "<message>" }`). */
export class InvalidError extends RpcError {
  readonly fields: Record<string, string> = {};

  constructor(message: string) {
    super(400, "invalid", message);
    this.details.fields = this.fields;
  }

  /** Names a field: a path (`lines[0].quantity`) or a message's field handle (`placeOrderRequest.fields.lines`). */
  field(path: string | AnyField, message: string): this {
    const key = typeof path === "string" ? path : fieldKeys.get(path);
    if (key === undefined) throw new TypeError("invalid(..).field: not a field of a message");
    this.fields[key] = message;
    return this;
  }
}

export function invalid(message = "the request is invalid"): InvalidError {
  return new InvalidError(message);
}

export function unauthenticated(message = "the request has no user"): RpcError {
  return new RpcError(401, "unauthenticated", message);
}

export function forbidden(message = "forbidden", reason?: string): RpcError {
  return new RpcError(403, "forbidden", message, reason ? { reason } : {});
}

export function notFound(message = "not found", field?: string): RpcError {
  return new RpcError(404, "not_found", message, field ? { field } : {});
}

export function conflict(message = "conflict"): RpcError {
  return new RpcError(409, "conflict", message);
}

/** 503 `unavailable`: a feature the platform does not offer, or an outside failure the plugin wraps. */
export function unavailable(cause?: unknown): RpcError {
  const message =
    cause === undefined ? "unavailable" : typeof cause === "string" ? cause : (cause as Error)?.message ?? String(cause);
  return new RpcError(503, "unavailable", message, {}, { cause });
}

/** 422 `permanent`: in a job, the attempt fails and the run is not retried (bad input that a retry cannot fix). */
export function permanent(message: string): RpcError {
  return new RpcError(422, "permanent", message).asPermanent();
}

/** @internal 500 `internal`; the message the caller sees is fixed. */
export function internal(cause?: unknown): RpcError {
  return new RpcError(500, "internal", "internal error", {}, { cause });
}

const KINDS = { invalid: 400, forbidden: 403, not_found: 404, conflict: 409, unprocessable: 422 } as const;
export type DomainErrorKind = keyof typeof KINDS;

/** A domain error of the plugin: a code of its own and a status in 400–422. */
export class DomainError {
  readonly status: number;

  constructor(
    readonly code: string,
    status: number | DomainErrorKind,
    readonly description: string = "",
  ) {
    this.status = typeof status === "number" ? status : KINDS[status];
    if (!/^[a-z][a-z0-9_]*$/.test(code)) throw new TypeError(`domain error code ${JSON.stringify(code)}: use snake_case`);
    if (!Number.isInteger(this.status) || this.status < 400 || this.status > 422) {
      throw new TypeError(`domain error ${code}: status ${status} is not in 400–422`);
    }
  }

  /** The error to throw: `throw notCancellable.error(\`order ${n} is shipped\`)`. */
  error(message?: string, details: ErrorDetails = {}): RpcError {
    return new RpcError(this.status, this.code, message ?? (this.description || this.code), details);
  }
}

/** `domainError('not_cancellable', 409)` or `domainError('not_cancellable', 'conflict', 'The order is no longer placed.')`. */
export function domainError(code: string, status: number | DomainErrorKind = 400, description = ""): DomainError {
  return new DomainError(code, status, description);
}

/** The body and status of any thrown value. */
export function toRpcError(e: unknown): RpcError {
  if (e instanceof RpcError) return e;
  if (e instanceof DomainError) return e.error();
  return internal(e);
}
