// The schedule of a job: a cron expression of five fields, in UTC.
//
// `minute hour day-of-month month day-of-week`, each a `*`, a number, a range `a-b`, a step `*/n`, `a-b/n` or `a/n`
// (from `a` to the field's end), or a comma-separated list of these. Months may be named (`jan`–`dec`) and weekdays
// too (`sun`–`sat`; `0` and `7` are Sunday), in any case. When both the day of the month and the day of the week are
// restricted, a day matches either of them, as in the classic cron. There are no seconds, no years, no `L`, `W`, `#`,
// `?` and no `@daily` shorthands. The SDK checks a job's cron with the same grammar `celobus plugin check` and
// installing use.
using System.Globalization;

namespace Celobus.Sdk;

internal static class Cron
{
    private sealed record Spec(string What, int Min, int Max, string[] Names, int NamesFrom);

    private static readonly string[] Months = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    private static readonly string[] Weekdays = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

    private static readonly Spec[] Specs =
    [
        new("minute", 0, 59, [], 0),
        new("hour", 0, 23, [], 0),
        new("day of the month", 1, 31, [], 0),
        new("month", 1, 12, Months, 1),
        new("day of the week", 0, 7, Weekdays, 0),
    ];

    /// <summary>Null when <paramref name="s"/> is a valid schedule; else what is wrong, in words.</summary>
    public static string? Check(string s)
    {
        var parts = s.Split([' ', '\t', '\n', '\r'], StringSplitOptions.RemoveEmptyEntries);
        if (parts.Length != 5)
            return $"a cron has 5 fields (minute hour day-of-month month day-of-week), this one has {parts.Length}";
        var masks = new ulong[5];
        for (var i = 0; i < 5; i++)
        {
            var error = Field(parts[i], Specs[i], out masks[i]);
            if (error is not null) return error;
        }
        // a day of the month alone that no chosen month has (`0 0 30 2 *`) never fires
        if (parts[2].StartsWith('*') || parts[4].StartsWith('*'))
        {
            var possible = false;
            for (var m = 1; m <= 12 && !possible; m++)
                if ((masks[3] & (1UL << m)) != 0)
                    for (var d = 1; d <= MaxDay(m) && !possible; d++)
                        possible = (masks[2] & (1UL << d)) != 0;
            if (!possible) return "no chosen month has the chosen day of the month";
        }
        return null;
    }

    private static int MaxDay(int month) => month switch { 2 => 29, 4 or 6 or 9 or 11 => 30, _ => 31 };

    private static string? Field(string s, Spec spec, out ulong mask)
    {
        mask = 0;
        foreach (var item in s.Split(','))
        {
            var error = Item(item, spec, out var m);
            if (error is not null) return $"{error} in the {spec.What} field (\"{s}\")";
            mask |= m;
        }
        return null;
    }

    private static string? Item(string item, Spec spec, out ulong mask)
    {
        mask = 0;
        var range = item;
        int? step = null;
        var slash = item.IndexOf('/');
        if (slash >= 0)
        {
            var st = item[(slash + 1)..];
            if (!uint.TryParse(st, NumberStyles.None, CultureInfo.InvariantCulture, out var n) || n > int.MaxValue)
                return $"\"{st}\" is not a step";
            if (n == 0) return "a step of 0";
            range = item[..slash];
            step = (int)n;
        }
        int lo, hi;
        string? error;
        if (range == "*") (lo, hi) = (spec.Min, spec.Max);
        else if (range.IndexOf('-') is var dash and >= 0)
        {
            if ((error = Value(range[..dash], spec, out lo)) is not null) return error;
            if ((error = Value(range[(dash + 1)..], spec, out hi)) is not null) return error;
            if (lo > hi) return $"the range \"{range}\" runs backwards";
        }
        else
        {
            if ((error = Value(range, spec, out lo)) is not null) return error;
            // `a/n` runs from `a` to the field's end
            hi = step is null ? lo : spec.Max;
        }
        for (var v = lo; v <= hi; v += step ?? 1) mask |= 1UL << v;
        return null;
    }

    private static string? Value(string s, Spec spec, out int value)
    {
        var i = Array.IndexOf(spec.Names, s.ToLowerInvariant());
        if (i >= 0)
        {
            value = spec.NamesFrom + i;
            return null;
        }
        if (!uint.TryParse(s, NumberStyles.None, CultureInfo.InvariantCulture, out var n))
        {
            value = 0;
            return $"\"{s}\" is not a {spec.What}";
        }
        if (n < spec.Min || n > spec.Max)
        {
            value = 0;
            return $"{n} is out of {spec.Min}..={spec.Max} for the {spec.What}";
        }
        value = (int)n;
        return null;
    }
}
