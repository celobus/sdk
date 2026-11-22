// The schedule of a job: a cron expression of five fields, in UTC, checked as the platform checks it.
//
// `minute hour day-of-month month day-of-week`, each a `*`, a number, a range `a-b`, a step `*/n`, `a-b/n` or `a/n`
// (from `a` to the field's end), or a comma-separated list of these. Months may be named (`jan`–`dec`) and weekdays
// too (`sun`–`sat`; `0` and `7` are Sunday), in any case. When both the day of the month and the day of the week are
// restricted (neither starts with `*`), a day matches either of them, as in the classic cron; otherwise both must
// match. There are no seconds, no years, no `L`, `W`, `#`, `?` and no `@daily` shorthands.

interface Spec {
  what: string;
  min: number;
  max: number;
  names: readonly string[];
  /** the value of the first name */
  namesFrom: number;
}

const MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

const SPECS: readonly Spec[] = [
  { what: "minute", min: 0, max: 59, names: [], namesFrom: 0 },
  { what: "hour", min: 0, max: 23, names: [], namesFrom: 0 },
  { what: "day of the month", min: 1, max: 31, names: [], namesFrom: 0 },
  { what: "month", min: 1, max: 12, names: MONTHS, namesFrom: 1 },
  { what: "day of the week", min: 0, max: 7, names: WEEKDAYS, namesFrom: 0 },
];

class CronError extends Error {}

const UNSIGNED = /^\+?\d+$/;

function value(s: string, spec: Spec): number {
  const i = spec.names.indexOf(s.toLowerCase());
  if (i >= 0) return spec.namesFrom + i;
  if (!UNSIGNED.test(s)) throw new CronError(`${JSON.stringify(s)} is not a ${spec.what}`);
  const n = Number(s);
  if (n < spec.min || n > spec.max) throw new CronError(`${n} is out of ${spec.min}..=${spec.max} for the ${spec.what}`);
  return n;
}

function itemSet(item: string, spec: Spec): Set<number> {
  let range = item;
  let step: number | undefined;
  const slash = item.indexOf("/");
  if (slash >= 0) {
    range = item.slice(0, slash);
    const st = item.slice(slash + 1);
    if (!UNSIGNED.test(st) || !Number.isSafeInteger(Number(st))) throw new CronError(`${JSON.stringify(st)} is not a step`);
    step = Number(st);
    if (step === 0) throw new CronError("a step of 0");
  }
  let lo: number;
  let hi: number;
  const dash = range.indexOf("-");
  if (range === "*") {
    [lo, hi] = [spec.min, spec.max];
  } else if (dash >= 0) {
    [lo, hi] = [value(range.slice(0, dash), spec), value(range.slice(dash + 1), spec)];
    if (lo > hi) throw new CronError(`the range ${JSON.stringify(range)} runs backwards`);
  } else {
    lo = value(range, spec);
    // `a/n` runs from `a` to the field's end
    hi = step !== undefined ? spec.max : lo;
  }
  const out = new Set<number>();
  for (let v = lo; v <= hi; v += step ?? 1) out.add(v);
  return out;
}

function fieldSet(s: string, spec: Spec): Set<number> {
  if (s === "") throw new CronError(`the ${spec.what} field is empty`);
  const out = new Set<number>();
  for (const item of s.split(",")) {
    try {
      for (const v of itemSet(item, spec)) out.add(v);
    } catch (e) {
      throw new CronError(`${(e as Error).message} in the ${spec.what} field (${JSON.stringify(s)})`);
    }
  }
  return out;
}

/** The most days a month has (29 for February). */
function maxDay(month: number): number {
  return month === 2 ? 29 : [4, 6, 9, 11].includes(month) ? 30 : 31;
}

/** @internal What is wrong with a cron expression, in words; `undefined` when it parses. */
export function cronError(s: string): string | undefined {
  const parts = s.split(/\s+/).filter(Boolean);
  if (parts.length !== 5) {
    return `a cron has 5 fields (minute hour day-of-month month day-of-week), this one has ${parts.length}`;
  }
  try {
    const sets = parts.map((p, i) => fieldSet(p, SPECS[i]!));
    const [, , days, months] = sets as [Set<number>, Set<number>, Set<number>, Set<number>, Set<number>];
    // a day of the month alone that no chosen month has (`0 0 30 2 *`) never fires
    if (parts[4]!.startsWith("*") || parts[2]!.startsWith("*")) {
      let possible = false;
      for (let m = 1; m <= 12 && !possible; m++) {
        if (!months.has(m)) continue;
        for (let d = 1; d <= maxDay(m); d++) if (days.has(d)) possible = true;
      }
      if (!possible) return "no chosen month has the chosen day of the month";
    }
    return undefined;
  } catch (e) {
    if (e instanceof CronError) return e.message;
    throw e;
  }
}
