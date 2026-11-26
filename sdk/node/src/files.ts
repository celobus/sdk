// Files (`ctx.files`): read, write and look up the plugin's files. A `File` value (`t.file()`) names a file; its bytes
// move directly between the workspace's object store and the plugin over signed URLs the platform hands out, never
// through the plugin's session. `open` streams a file (resuming from where it stopped when a URL expires or the
// connection drops), `create` writes a new one part by part (one part buffered at a time), `head` says what is known of
// one. A plugin reaches only its own files: one it was given (in a request, a call's answer, a job payload, a row) or
// one it made.
import { checkCall } from "./keys.js";
import { RpcError, internal, unavailable } from "./errors.js";
import type { File } from "./schema.js";

/** How `ctx.files.open` reads. */
export interface OpenOptions {
  /** The byte to start at (default 0). */
  offset?: number;
}

/** How `ctx.files.create` writes. */
export interface CreateOptions {
  /**
   * The file's size in bytes when it is known: the space is reserved at once, and writing more or fewer bytes fails
   * (409 `file_size_mismatch`). Left out: the space is reserved part by part as the bytes are written.
   */
  size?: number;
}

/** What is known of a file (`ctx.files.head`). */
export interface FileInfo {
  file: File;
  /** The bytes' entity tag: equal for equal bytes of the same file. */
  etag: string;
  createdAt: Date;
  /** The user who uploaded or made it; `undefined`: the plugin itself. */
  createdBy?: string;
}

/**
 * A file's bytes as a web `ReadableStream` (`for await (const chunk of stream)`), with its lines (`stream.lines()`).
 */
export type FileStream = ReadableStream<Uint8Array> & {
  /** The file read. */
  readonly file: File;
  /**
   * Its lines as UTF-8 text, without the line ends (`\n` or `\r\n`); a last line without an end is a line too.
   * `for await (const line of (await ctx.files.open(file)).lines()) { .. }`
   */
  lines(): AsyncIterableIterator<string>;
};

/** A file being written (`ctx.files.create`): `write` the bytes in order, then `finish`. */
export interface FileWriter {
  /** The new file's id. */
  readonly id: string;
  /**
   * Adds bytes (a string is written as UTF-8). Resolves once they are buffered or sent; writes are kept in the order
   * they are called. 507 `quota_exceeded` when the workspace's storage is full.
   */
  write(chunk: string | Uint8Array): Promise<void>;
  /**
   * Sends what is left and ends the file: the `File` to keep in a row, a job payload or an answer. A file nothing holds
   * is collected a day after it was made; inside a request its caller may download it until then (an export).
   */
  finish(): Promise<File>;
}

/**
 * The plugin's files (`ctx.files`). There is no delete: a file goes when nothing holds it (remove it from the record).
 * Errors: 404 `not_found` (not a file of this plugin), 410 `file_gone` (it was collected), 413 `file_too_large`, 507
 * `quota_exceeded`, 409 `file_size_mismatch`, 409 `read_only` (`create` in a read-only endpoint), 503 `unavailable`.
 */
export interface Files {
  /**
   * Opens one of the plugin's files for reading: its bytes as a stream, from `offset` when given. A long read survives
   * the expiry of its URL: it resumes where it stopped.
   */
  open(file: File, options?: OpenOptions): Promise<FileStream>;
  /**
   * Starts writing a new file named `name` (at most 255 bytes; `/`, `\`, control characters and leading dots are
   * removed) of the MIME type `type` (empty: `application/octet-stream`).
   */
  create(name: string, type: string, options?: CreateOptions): Promise<FileWriter>;
  /** What is known of one of the plugin's files. */
  head(file: File): Promise<FileInfo>;
}

/** @internal What the platform answers the file calls (protobuf field names in camelCase, 64-bit numbers as strings). */
export interface WireFile {
  id: string;
  name: string;
  contentType: string;
  size: string | number;
}
interface WireUpload {
  id: string;
  partSize: string | number;
  parts: number;
  first: number;
  urls?: string[];
  done?: number[];
  expiresAtMs: string | number;
}
/** @internal */
export interface FilesResult {
  filesUpload?: WireUpload | null;
  file?: WireFile | null;
  filesRead?: { file?: WireFile | null; url: string; expiresAtMs: string | number } | null;
  fileInfo?: { file?: WireFile | null; etag: string; createdAtMs: string | number; createdBy: string } | null;
}

/** @internal A `File` as the platform sends it. */
export function fileOf(w: WireFile | null | undefined): File {
  if (!w) throw internal(new Error("the platform answered a file call without the file"));
  return { id: w.id, name: w.name ?? "", type: w.contentType ?? "", size: Number(w.size ?? 0) };
}

/** @internal A `File` as the platform takes it. */
export function fileWire(f: File): WireFile {
  if (typeof f?.id !== "string" || f.id === "") throw internal(new Error("not a File: it has no id"));
  return { id: f.id, name: f.name ?? "", contentType: f.type ?? "", size: String(f.size ?? 0) };
}

/** @internal What the platform's files need of a context. */
export interface FilesLink {
  syscall(call: Record<string, unknown>): Promise<FilesResult>;
  requestId: string;
  signal: AbortSignal;
}

/** How long before its expiry a URL is renewed rather than used. */
const RENEW_MS = 30_000;
/** Attempts of one part or one stretch of a read without progress. */
const ATTEMPTS = 3;

function idOf(file: File, call: string): string {
  if (typeof file?.id !== "string" || file.id === "") throw internal(new Error(`${call}: not a File (it has no id)`));
  return file.id;
}

function sizeOf(n: number | undefined, what: string): number | undefined {
  if (n === undefined) return undefined;
  if (!Number.isSafeInteger(n) || n < 0) throw internal(new Error(`${what}: ${n} is not a number of bytes`));
  return n;
}

/** @internal `ctx.files` over the platform. */
export class PlatformFiles implements Files {
  constructor(private readonly link: FilesLink) {}

  async open(file: File, options: OpenOptions = {}): Promise<FileStream> {
    checkCall("ctx.files.open()", options);
    const id = idOf(file, "ctx.files.open()");
    const start = sizeOf(options.offset, "ctx.files.open()") ?? 0;
    const read = async (offset: number) => {
      const call = { filesOpen: { id, offset: String(offset), requestId: this.link.requestId } };
      const r = (await this.link.syscall(call)).filesRead;
      if (!r) throw internal(new Error("the platform answered an open with something else"));
      return { file: fileOf(r.file), url: r.url, expiresAt: Number(r.expiresAtMs) };
    };
    const first = await read(start);
    const signal = this.link.signal;
    let { url, expiresAt } = first;
    let pos = start;
    let skip = 0; // bytes to drop of an answer that started before `pos`
    let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
    const renew = async () => {
      ({ url, expiresAt } = await read(pos));
    };
    const stream = new ReadableStream<Uint8Array>({
      async pull(controller) {
        for (let attempt = 1; ; attempt++) {
          try {
            if (reader === undefined) {
              if (pos >= first.file.size) return controller.close();
              if (Date.now() >= expiresAt - RENEW_MS) await renew();
              const res = await fetch(url, { headers: pos > 0 ? { range: `bytes=${pos}-` } : {}, signal });
              if (!res.ok || !res.body) {
                await res.body?.cancel();
                throw new HttpStatus(res.status);
              }
              skip = pos > 0 && res.status !== 206 ? pos : 0;
              reader = res.body.getReader();
            }
            let chunk: Uint8Array | undefined;
            while (chunk === undefined) {
              const { done, value } = await reader.read();
              if (done) {
                reader = undefined;
                if (pos >= first.file.size) return controller.close();
                throw new HttpStatus(0); // the answer ended early: read on from where it stopped
              }
              const drop = Math.min(skip, value.length);
              skip -= drop;
              if (drop < value.length) chunk = value.subarray(drop);
            }
            pos += chunk.length;
            controller.enqueue(chunk);
            return;
          } catch (e) {
            reader?.cancel().catch(() => {});
            reader = undefined;
            if (signal.aborted) throw e;
            if (e instanceof RpcError) throw e;
            if (attempt >= ATTEMPTS) {
              const why = e instanceof HttpStatus ? (e.status ? `answered ${e.status}` : "ended early") : (e as Error).message;
              throw unavailable(`reading ${first.file.name || id}: the object store ${why}`);
            }
            await renew();
          }
        }
      },
      cancel() {
        return reader?.cancel();
      },
    });
    return fileStream(stream, first.file);
  }

  async create(name: string, type: string, options: CreateOptions = {}): Promise<FileWriter> {
    checkCall("ctx.files.create()", options);
    const size = sizeOf(options.size, "ctx.files.create()");
    const r = await this.link.syscall({
      filesCreate: {
        name,
        contentType: type,
        ...(size === undefined ? {} : { size: String(size) }),
        requestId: this.link.requestId,
      },
    });
    if (!r.filesUpload) throw internal(new Error("the platform answered a create with something else"));
    return new PlatformWriter(this.link, r.filesUpload, size, name);
  }

  async head(file: File): Promise<FileInfo> {
    const id = idOf(file, "ctx.files.head()");
    const r = (await this.link.syscall({ filesHead: { id, requestId: this.link.requestId } })).fileInfo;
    if (!r) throw internal(new Error("the platform answered a head with something else"));
    return {
      file: fileOf(r.file),
      etag: r.etag ?? "",
      createdAt: new Date(Number(r.createdAtMs ?? 0)),
      ...(r.createdBy ? { createdBy: r.createdBy } : {}),
    };
  }
}

/** An answer of the object store that is not the bytes asked for (0: the answer ended early). */
class HttpStatus extends Error {
  constructor(readonly status: number) {
    super(`HTTP ${status}`);
  }
}

/** A writer over the platform: parts of `partSize` bytes, numbered from 1, each sent to its signed URL. */
class PlatformWriter implements FileWriter {
  readonly id: string;
  private readonly partSize: number;
  private page: WireUpload;
  private expiresAt: number;
  private buffer: Uint8Array[] = [];
  private buffered = 0;
  private written = 0;
  private part = 1;
  private readonly etags: string[] = [];
  private queue: Promise<void> = Promise.resolve();
  private finished = false;
  /** The first failure: a part may be missing, so every later write and the finish fail with it. */
  private failed: { error: unknown } | undefined;

  constructor(
    private readonly link: FilesLink,
    upload: WireUpload,
    private readonly size: number | undefined,
    private readonly name: string,
  ) {
    this.id = upload.id;
    this.page = upload;
    this.expiresAt = Number(upload.expiresAtMs);
    this.partSize = Number(upload.partSize);
    if (!(this.partSize > 0)) throw internal(new Error("the platform answered a create without a part size"));
  }

  write(chunk: string | Uint8Array): Promise<void> {
    // a copy, taken now: the caller may reuse its array as soon as this returns
    const own = chunk instanceof Uint8Array ? Uint8Array.from(chunk) : chunk;
    const next = this.queue.then(() => this.guard(() => this.add(own)));
    this.queue = next.catch(() => {});
    return next;
  }

  async finish(): Promise<File> {
    await this.queue;
    return this.guard(() => this.end());
  }

  /** Runs `step` unless a step failed before; latches its failure. */
  private async guard<T>(step: () => Promise<T>): Promise<T> {
    if (this.failed) throw this.failed.error;
    try {
      return await step();
    } catch (e) {
      this.failed ??= { error: e };
      throw e;
    }
  }

  private async end(): Promise<File> {
    if (this.finished) throw internal(new Error(`ctx.files: ${this.name} is finished already`));
    this.finished = true;
    if (this.size !== undefined && this.written !== this.size) {
      throw new RpcError(409, "file_size_mismatch", `${this.name}: ${this.written} bytes written, ${this.size} declared`);
    }
    // the last part (an empty file is one empty part)
    if (this.buffered > 0 || this.part === 1) await this.send(this.take(this.buffered));
    const known = this.etags.length === this.part - 1 && this.etags.every((e) => e !== "");
    const r = await this.link.syscall({
      filesFinish: { id: this.id, etags: known ? this.etags : [], requestId: this.link.requestId },
    });
    return fileOf(r.file);
  }

  private async add(chunk: string | Uint8Array): Promise<void> {
    if (this.finished) throw internal(new Error(`ctx.files: ${this.name} is finished`));
    const bytes = typeof chunk === "string" ? Buffer.from(chunk, "utf8") : chunk;
    if (!(bytes instanceof Uint8Array)) throw internal(new Error("ctx.files: write takes a string or a Uint8Array"));
    if (this.size !== undefined && this.written + bytes.length > this.size) {
      throw new RpcError(409, "file_size_mismatch", `${this.name}: more than the ${this.size} bytes declared`);
    }
    this.written += bytes.length;
    if (bytes.length > 0) {
      this.buffer.push(bytes);
      this.buffered += bytes.length;
    }
    while (this.buffered >= this.partSize) await this.send(this.take(this.partSize));
  }

  /** The first `n` buffered bytes, as one part. */
  private take(n: number): Uint8Array<ArrayBuffer> {
    const out = Buffer.allocUnsafe(n);
    let at = 0;
    while (at < n) {
      const head = this.buffer[0];
      const used = Math.min(head.length, n - at);
      out.set(head.subarray(0, used), at);
      at += used;
      if (used === head.length) this.buffer.shift();
      else this.buffer[0] = head.subarray(used);
    }
    this.buffered -= n;
    return out;
  }

  /** Sends the next part to its URL, asking for a fresh one when it expired or was refused. */
  private async send(body: Uint8Array<ArrayBuffer>): Promise<void> {
    const n = this.part;
    for (let attempt = 1; ; attempt++) {
      const url = await this.urlOf(n, attempt > 1);
      let status = 0;
      try {
        const res = await fetch(url, { method: "PUT", body, signal: this.link.signal });
        await res.body?.cancel();
        if (res.ok) {
          this.etags[n - 1] = res.headers.get("etag") ?? "";
          this.part += 1;
          return;
        }
        status = res.status;
      } catch (e) {
        if (this.link.signal.aborted) throw e;
      }
      if (attempt >= ATTEMPTS) {
        throw unavailable(`writing ${this.name}: the object store ${status ? `answered ${status}` : "could not be reached"}`);
      }
    }
  }

  /** The URL of part `n`: from the page in hand while it is valid, else from the next page. */
  private async urlOf(n: number, fresh: boolean): Promise<string> {
    const inPage = () => {
      const i = n - this.page.first;
      const urls = this.page.urls ?? [];
      return i >= 0 && i < urls.length ? urls[i] : undefined;
    };
    let url = inPage();
    if (fresh || url === undefined || Date.now() >= this.expiresAt - RENEW_MS) {
      const r = await this.link.syscall({ filesParts: { id: this.id, from: n, requestId: this.link.requestId } });
      if (!r.filesUpload) throw internal(new Error("the platform answered a parts call with something else"));
      this.page = r.filesUpload;
      this.expiresAt = Number(r.filesUpload.expiresAtMs);
      url = inPage();
    }
    if (url === undefined) throw internal(new Error(`the platform gave no URL for part ${n} of ${this.name}`));
    return url;
  }
}

/** @internal Gives a stream of a file's bytes its `file` and `lines()`. */
export function fileStream(stream: ReadableStream<Uint8Array>, file: File): FileStream {
  return Object.assign(stream, { file, lines: () => lines(stream) });
}

async function* lines(stream: ReadableStream<Uint8Array>): AsyncGenerator<string> {
  const decoder = new TextDecoder("utf-8");
  let rest = "";
  const line = (s: string) => (s.endsWith("\r") ? s.slice(0, -1) : s);
  for await (const chunk of stream as unknown as AsyncIterable<Uint8Array>) {
    rest += decoder.decode(chunk, { stream: true });
    const parts = rest.split("\n");
    rest = parts.pop()!;
    for (const p of parts) yield line(p);
  }
  rest += decoder.decode();
  if (rest !== "") yield line(rest);
}

/**
 * Files kept in memory, for tests (`testCtx(plugin, { files })`): `ctx.files` reads, writes and looks them up here.
 * `add` puts a file in (what a test's handler is given), `bytes` and `text` read what a handler wrote.
 */
export class MemoryFiles implements Files {
  private readonly store = new Map<string, { file: File; bytes: Uint8Array; createdAt: Date }>();
  private next = 0;

  /** Puts a file in: the `File` to give a handler. */
  add(name: string, type: string, content: string | Uint8Array): File {
    const bytes = typeof content === "string" ? Buffer.from(content, "utf8") : new Uint8Array(content);
    this.next += 1;
    const file: File = { id: `file-${this.next}`, name, type: type || "application/octet-stream", size: bytes.length };
    this.store.set(file.id, { file, bytes, createdAt: new Date() });
    return { ...file };
  }

  /** The bytes of a file; `undefined` when there is none. */
  bytes(file: File): Uint8Array | undefined {
    return this.store.get(file.id)?.bytes;
  }

  /** The bytes of a file as UTF-8 text; `undefined` when there is none. */
  text(file: File): string | undefined {
    const b = this.bytes(file);
    return b === undefined ? undefined : Buffer.from(b.buffer, b.byteOffset, b.byteLength).toString("utf8");
  }

  /** Every file, in the order they were made. */
  list(): File[] {
    return [...this.store.values()].map((x) => ({ ...x.file }));
  }

  private found(file: File, call: string) {
    const f = this.store.get(idOf(file, call));
    if (!f) throw new RpcError(404, "not_found", `${call}: no file ${file.id}`);
    return f;
  }

  async open(file: File, options: OpenOptions = {}): Promise<FileStream> {
    checkCall("ctx.files.open()", options);
    const f = this.found(file, "ctx.files.open()");
    const from = Math.min(sizeOf(options.offset, "ctx.files.open()") ?? 0, f.bytes.length);
    const bytes = f.bytes.subarray(from);
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        if (bytes.length > 0) controller.enqueue(new Uint8Array(bytes));
        controller.close();
      },
    });
    return fileStream(stream, { ...f.file });
  }

  async create(name: string, type: string, options: CreateOptions = {}): Promise<FileWriter> {
    checkCall("ctx.files.create()", options);
    const size = sizeOf(options.size, "ctx.files.create()");
    if (name.trim() === "") throw new RpcError(400, "bad_request", "ctx.files.create(): the name is empty");
    const chunks: Uint8Array[] = [];
    let written = 0;
    let finished = false;
    this.next += 1;
    const id = `file-${this.next}`;
    return {
      id,
      write: async (chunk) => {
        if (finished) throw internal(new Error(`ctx.files: ${name} is finished`));
        const bytes = typeof chunk === "string" ? Buffer.from(chunk, "utf8") : new Uint8Array(chunk);
        if (size !== undefined && written + bytes.length > size) {
          throw new RpcError(409, "file_size_mismatch", `${name}: more than the ${size} bytes declared`);
        }
        chunks.push(bytes);
        written += bytes.length;
      },
      finish: async () => {
        if (finished) throw internal(new Error(`ctx.files: ${name} is finished already`));
        finished = true;
        if (size !== undefined && written !== size) {
          throw new RpcError(409, "file_size_mismatch", `${name}: ${written} bytes written, ${size} declared`);
        }
        const bytes = new Uint8Array(Buffer.concat(chunks));
        const file: File = { id, name, type: type || "application/octet-stream", size: bytes.length };
        this.store.set(id, { file, bytes, createdAt: new Date() });
        return { ...file };
      },
    };
  }

  async head(file: File): Promise<FileInfo> {
    const f = this.found(file, "ctx.files.head()");
    return { file: { ...f.file }, etag: `"${f.file.id}"`, createdAt: f.createdAt };
  }
}
