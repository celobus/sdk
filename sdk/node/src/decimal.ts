// Decimal: exact base-10 numbers (a bigint coefficient and a scale), the SDK's type of `t.decimal()`.
//
// Addition, subtraction and multiplication are exact. Division keeps up to 28 fractional digits, rounded half
// away from zero, without trailing zeros. `round(n)` is half away from zero, as in every celobus SDK. On the wire
// a decimal is a string (`"12.50"`): the scale is kept, so `12.50` stays `12.50`.

export type DecimalLike = Decimal | string | number | bigint;

const PATTERN = /^([+-])?(\d*)(?:\.(\d*))?(?:[eE]([+-]?\d+))?$/;
const DIV_SCALE = 28;

function pow10(n: number): bigint {
  return 10n ** BigInt(n);
}

/** Divides and rounds half away from zero. */
function divRound(n: bigint, d: bigint): bigint {
  const negative = n < 0n !== d < 0n;
  const an = n < 0n ? -n : n;
  const ad = d < 0n ? -d : d;
  let q = an / ad;
  if ((an % ad) * 2n >= ad) q += 1n;
  return negative ? -q : q;
}

export class Decimal {
  static readonly ZERO = new Decimal(0n, 0);
  static readonly ONE = new Decimal(1n, 0);
  static readonly HUNDRED = new Decimal(100n, 0);

  private constructor(
    /** the value is `coefficient × 10^-scale` */
    readonly coefficient: bigint,
    readonly scale: number,
  ) {}

  /** A decimal from a string (`"12.50"`, `"-1e3"`), an integer, a finite number or a bigint. */
  static from(value: DecimalLike): Decimal {
    if (value instanceof Decimal) return value;
    if (typeof value === "bigint") return new Decimal(value, 0);
    if (typeof value === "number") {
      if (!Number.isFinite(value)) throw new RangeError(`not a finite number: ${value}`);
      if (Number.isInteger(value) && Number.isSafeInteger(value)) return new Decimal(BigInt(value), 0);
      return Decimal.parse(String(value));
    }
    if (typeof value === "string") return Decimal.parse(value);
    throw new TypeError(`not a decimal: ${String(value)}`);
  }

  /** Parses a decimal string; throws a `SyntaxError` for anything else. */
  static parse(text: string): Decimal {
    const m = PATTERN.exec(text.trim());
    if (!m || (!m[2] && !m[3])) throw new SyntaxError(`not a decimal: ${JSON.stringify(text)}`);
    const [, sign, int = "", frac = "", exp = "0"] = m;
    let coefficient = BigInt((int || "0") + frac);
    if (sign === "-") coefficient = -coefficient;
    let scale = frac.length - Number(exp);
    if (scale < 0) {
      coefficient *= pow10(-scale);
      scale = 0;
    }
    return new Decimal(coefficient, scale);
  }

  /** The sum of the values (zero for none). */
  static sum(values: Iterable<DecimalLike>): Decimal {
    let total = Decimal.ZERO;
    for (const v of values) total = total.add(v);
    return total;
  }

  private static align(a: Decimal, b: Decimal): [bigint, bigint, number] {
    const scale = Math.max(a.scale, b.scale);
    return [a.coefficient * pow10(scale - a.scale), b.coefficient * pow10(scale - b.scale), scale];
  }

  add(other: DecimalLike): Decimal {
    const [a, b, scale] = Decimal.align(this, Decimal.from(other));
    return new Decimal(a + b, scale);
  }

  sub(other: DecimalLike): Decimal {
    const [a, b, scale] = Decimal.align(this, Decimal.from(other));
    return new Decimal(a - b, scale);
  }

  mul(other: DecimalLike): Decimal {
    const o = Decimal.from(other);
    return new Decimal(this.coefficient * o.coefficient, this.scale + o.scale);
  }

  /** Division to 28 fractional digits, half away from zero, trailing zeros dropped; a zero divisor throws. */
  div(other: DecimalLike): Decimal {
    const o = Decimal.from(other);
    if (o.coefficient === 0n) throw new RangeError("division by zero");
    // this / o = (c1 / c2) × 10^(s2 - s1); computed at DIV_SCALE fractional digits
    const shift = DIV_SCALE + o.scale - this.scale;
    const n = shift >= 0 ? this.coefficient * pow10(shift) : this.coefficient;
    const d = shift >= 0 ? o.coefficient : o.coefficient * pow10(-shift);
    return new Decimal(divRound(n, d), DIV_SCALE).normalize(Math.max(this.scale, 0));
  }

  /** Rounds to `digits` fractional digits, half away from zero (`2.345` → `2.35`, `-2.345` → `-2.35`). */
  round(digits = 0): Decimal {
    if (!Number.isInteger(digits) || digits < 0) throw new RangeError(`bad number of digits: ${digits}`);
    if (digits >= this.scale) return new Decimal(this.coefficient * pow10(digits - this.scale), digits);
    return new Decimal(divRound(this.coefficient, pow10(this.scale - digits)), digits);
  }

  /** Drops trailing fractional zeros, keeping at least `minScale` digits. */
  normalize(minScale = 0): Decimal {
    let { coefficient, scale } = this;
    while (scale > minScale && coefficient % 10n === 0n) {
      coefficient /= 10n;
      scale -= 1;
    }
    return new Decimal(coefficient, scale);
  }

  neg(): Decimal {
    return new Decimal(-this.coefficient, this.scale);
  }

  abs(): Decimal {
    return this.coefficient < 0n ? this.neg() : this;
  }

  /** -1, 0 or 1. */
  cmp(other: DecimalLike): number {
    const [a, b] = Decimal.align(this, Decimal.from(other));
    return a < b ? -1 : a > b ? 1 : 0;
  }

  eq(other: DecimalLike): boolean {
    return this.cmp(other) === 0;
  }

  lt(other: DecimalLike): boolean {
    return this.cmp(other) < 0;
  }

  gt(other: DecimalLike): boolean {
    return this.cmp(other) > 0;
  }

  isZero(): boolean {
    return this.coefficient === 0n;
  }

  /** The nearest JavaScript number (may lose precision). */
  toNumber(): number {
    return Number(this.toString());
  }

  /** Plain notation with the scale kept: `12.50`, `-0.05`, `3`. */
  toString(): string {
    const negative = this.coefficient < 0n;
    const digits = (negative ? -this.coefficient : this.coefficient).toString();
    if (this.scale === 0) return (negative ? "-" : "") + digits;
    const padded = digits.padStart(this.scale + 1, "0");
    const int = padded.slice(0, padded.length - this.scale);
    const frac = padded.slice(padded.length - this.scale);
    return `${negative ? "-" : ""}${int}.${frac}`;
  }

  toJSON(): string {
    return this.toString();
  }
}
