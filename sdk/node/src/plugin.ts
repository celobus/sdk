// `plugin({...})`: the plugin as a value. `describe()` gives the descriptor, `check()` the definition errors,
// `serve()` runs it: with `--describe` it prints the descriptor and exits (2 on definition errors), otherwise it
// connects to the platform.
import * as fs from "node:fs";
import * as path from "node:path";
import type { RunChangeHandler } from "./ctx.js";
import type { PluginOptions } from "./declare.js";
import { DescribeErrors, formatError, resolve, type DescribeError, type Descriptor, type Resolved } from "./descriptor.js";
import { createLogger } from "./log.js";
import { runSession, type ServeOptions } from "./serve.js";

/** The version of the nearest package.json above the program (`process.argv[1]`) that is not the SDK's own. */
function packageVersion(): string | undefined {
  const main = process.argv[1];
  if (!main) return undefined;
  let dir: string;
  try {
    dir = path.dirname(fs.realpathSync(main));
  } catch {
    return undefined;
  }
  for (;;) {
    const file = path.join(dir, "package.json");
    if (fs.existsSync(file)) {
      try {
        const pkg = JSON.parse(fs.readFileSync(file, "utf8")) as { name?: string; version?: string };
        if (pkg.name !== "@celobus/sdk") return pkg.version;
      } catch {
        return undefined;
      }
    }
    const parent = path.dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

export class Plugin {
  private resolved?: Resolved;
  private runChange?: RunChangeHandler;

  constructor(readonly options: PluginOptions) {}

  /** @internal */
  model(): Resolved {
    const options = this.runChange ? { ...this.options, onRunChange: this.runChange } : this.options;
    this.resolved ??= resolve(options, this.options.version ?? packageVersion());
    return this.resolved;
  }

  /**
   * The run-change hook: the platform calls `handler` when a run the plugin sees
   * changes — a run of one of its agents, or one its code started — with where the run stands (`RunChange`). A
   * system request: no caller (`ctx.caller.kind` is `system`), so the handler writes with `ctx.asSystem(..)` and reads
   * the run with `ctx.agents`. The descriptor says `runs: {on_change: true}`.
   *
   * Coalesced per run (one call per 250 ms window, then a trailing one with the latest state) and delivered at least
   * once: an error, the handler's 30 s timeout or a restart of the plugin makes the platform call again. Handle it
   * idempotently.
   *
   *   export default plugin({ id: 'helpdesk', agents, endpoints }).onRunChange(async (ctx, change) => {
   *     if (change.ended) { const run = await ctx.agents.get(change.runId); ... }
   *   });
   */
  onRunChange(handler: RunChangeHandler): this {
    this.runChange = handler;
    this.resolved = undefined;
    return this;
  }

  /** The plugin's id. */
  get id(): string {
    return this.options.id;
  }

  /** The definition errors (empty when the plugin is well defined). */
  check(): DescribeError[] {
    return this.model().errors;
  }

  /** The descriptor; throws `DescribeErrors` when the definition has errors. */
  describe(): Descriptor {
    const { descriptor, errors } = this.model();
    if (errors.length > 0) throw new DescribeErrors(errors);
    return descriptor;
  }

  /**
   * With `--describe`: prints the descriptor on stdout and exits 0, or the definition errors on stderr and exits
   * 2, without connecting. Otherwise refuses to start on definition errors (exit 2), then serves until SIGTERM.
   */
  async serve(options: ServeOptions = {}): Promise<void> {
    const argv = options.argv ?? process.argv;
    const { descriptor, errors } = this.model();
    if (errors.length > 0) {
      process.stderr.write(errors.map(formatError).join("\n") + "\n", () => process.exit(2));
      return new Promise(() => {});
    }
    if (argv.includes("--describe")) {
      process.stdout.write(JSON.stringify(descriptor, null, 2) + "\n", () => process.exit(0));
      return new Promise(() => {});
    }
    const log = createLogger({ plugin: descriptor.id });
    return runSession(
      {
        id: descriptor.id,
        version: descriptor.version,
        depends: descriptor.dependencies.map((d) => d.plugin),
        resolved: this.model(),
        log,
      },
      options,
    );
  }
}

/**
 * Declares the plugin. Names come from the keys: `roles: { clerk }` → role `clerk`, `endpoints: { getNote }` →
 * endpoint `GetNote`, `entities: { orderLine }` → entity `order_line` (message `OrderLine`).
 */
export function plugin(options: PluginOptions): Plugin {
  return new Plugin(options);
}
