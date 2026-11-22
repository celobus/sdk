// A dependency's endpoint with its message types: what the clients `celobus plugin add` generates
// (`src/deps/<plugin>.ts`) export, `await catalog.getProduct(ctx, { id })`. The input is encoded and the answer
// decoded with the same codec as the plugin's own endpoints (bigint for int64, Date for timestamp, Decimal).
import { decodeMessage, encodeMessage, type FieldErrors } from "./codec.js";
import type { Ctx } from "./ctx.js";
import { internal } from "./errors.js";
import type { Infer, MessageDef, Shape } from "./schema.js";

/** Calls `endpoint` of `plugin` as the request's caller; a non-2xx answer throws an `RpcError` as `ctx.call`. */
export type Remote<I, O> = ((ctx: Ctx, input: I) => Promise<O>) & {
  readonly plugin: string;
  readonly endpoint: string;
};

export function remote<IS extends Shape, OS extends Shape>(
  plugin: string,
  endpoint: string,
  input: MessageDef<IS>,
  output: MessageDef<OS>,
): Remote<Infer<IS>, Infer<OS>> {
  const call = async (ctx: Ctx, value: Infer<IS>): Promise<Infer<OS>> => {
    const body = await ctx.call(plugin, endpoint, encodeMessage(input, value));
    const errors: FieldErrors = {};
    const out = decodeMessage(output, body, "", errors);
    const bad = Object.entries(errors);
    if (bad.length > 0) {
      const [path, why] = bad[0]!;
      throw internal(`${plugin}.${endpoint} answered a body this plugin cannot read: ${path}: ${why}`);
    }
    return out as Infer<OS>;
  };
  return Object.assign(call, { plugin, endpoint });
}
