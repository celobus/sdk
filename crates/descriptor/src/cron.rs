//! The schedule of a job: a cron expression of five fields, in UTC.
//!
//! `minute hour day-of-month month day-of-week`, each a `*`, a number, a range `a-b`, a step `*/n`, `a-b/n` or
//! `a/n` (from `a` to the field's end), or a comma-separated list of these. Months may be named (`jan`–`dec`) and
//! weekdays too (`sun`–`sat`; `0` and `7` are Sunday), in any case. When both the day of the month and the day of the
//! week are restricted (neither starts with `*`), a day matches either of them, as in the classic cron; otherwise both
//! must match. There are no seconds, no years, no `L`, `W`, `#`, `?` and no `@daily` shorthands.
//!
//! The same parser checks a plugin's descriptor and computes the next occurrence of an installed schedule, so a
//! cron that passes `celobus plugin check` is accepted at install.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc};

/// A parsed cron expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    source: String,
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    /// the day-of-month field starts with `*`
    days_any: bool,
    /// the day-of-week field starts with `*`
    weekdays_any: bool,
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// How far ahead [`Cron::next_after`] looks: past it there is no occurrence (eight years cover every 29 February).
const HORIZON_YEARS: i32 = 8;

struct Spec {
    what: &'static str,
    min: u32,
    max: u32,
    names: &'static [&'static str],
    /// the value of the first name
    names_from: u32,
}

const SPECS: [Spec; 5] = [
    Spec {
        what: "minute",
        min: 0,
        max: 59,
        names: &[],
        names_from: 0,
    },
    Spec {
        what: "hour",
        min: 0,
        max: 23,
        names: &[],
        names_from: 0,
    },
    Spec {
        what: "day of the month",
        min: 1,
        max: 31,
        names: &[],
        names_from: 0,
    },
    Spec {
        what: "month",
        min: 1,
        max: 12,
        names: &MONTHS,
        names_from: 1,
    },
    Spec {
        what: "day of the week",
        min: 0,
        max: 7,
        names: &WEEKDAYS,
        names_from: 0,
    },
];

impl Cron {
    /// Parses a five-field expression; the error says what is wrong in words.
    pub fn parse(s: &str) -> Result<Cron, String> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(format!(
                "a cron has 5 fields (minute hour day-of-month month day-of-week), this one has {}",
                parts.len()
            ));
        }
        let mut masks = [0u64; 5];
        for (i, (part, spec)) in parts.iter().zip(SPECS.iter()).enumerate() {
            masks[i] = field(part, spec)?;
        }
        // Sunday is 0 and 7
        let mut weekdays = masks[4];
        if weekdays & (1 << 7) != 0 {
            weekdays = (weekdays | 1) & !(1 << 7);
        }
        let cron = Cron {
            source: parts.join(" "),
            minutes: masks[0],
            hours: masks[1],
            days: masks[2],
            months: masks[3],
            weekdays,
            days_any: parts[2].starts_with('*'),
            weekdays_any: parts[4].starts_with('*'),
        };
        // a day of the month alone that no chosen month has (`0 0 30 2 *`) never fires
        if cron.weekdays_any || cron.days_any {
            let possible = (1..=12u32).any(|m| {
                cron.months & (1 << m) != 0 && (1..=max_day(m)).any(|d| cron.days & (1 << d) != 0)
            });
            if !possible {
                return Err("no chosen month has the chosen day of the month".into());
            }
        }
        Ok(cron)
    }

    /// The expression as parsed, its fields separated by one space.
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// The first occurrence strictly after `t`, at a whole minute; `None` when there is none within eight years.
    pub fn next_after(&self, t: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let start = t.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        let end_year = start.year() + HORIZON_YEARS;
        let mut c = start;
        while c.year() <= end_year {
            if !bit(self.months, c.month()) {
                let (y, m) = if c.month() == 12 {
                    (c.year() + 1, 1)
                } else {
                    (c.year(), c.month() + 1)
                };
                c = Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0).single()?;
                continue;
            }
            if !self.day_matches(c.date_naive()) {
                let next = c.date_naive().succ_opt()?;
                c = Utc.from_utc_datetime(&next.and_hms_opt(0, 0, 0)?);
                continue;
            }
            if !bit(self.hours, c.hour()) {
                c = c.with_minute(0)? + Duration::hours(1);
                continue;
            }
            if !bit(self.minutes, c.minute()) {
                c += Duration::minutes(1);
                continue;
            }
            return Some(c);
        }
        None
    }

    fn day_matches(&self, d: NaiveDate) -> bool {
        let dom = bit(self.days, d.day());
        let dow = bit(self.weekdays, d.weekday().num_days_from_sunday());
        if self.days_any || self.weekdays_any {
            dom && dow
        } else {
            dom || dow
        }
    }
}

impl std::fmt::Display for Cron {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.source)
    }
}

impl std::str::FromStr for Cron {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Cron::parse(s)
    }
}

fn bit(mask: u64, n: u32) -> bool {
    mask & (1 << n) != 0
}

/// The most days a month has (29 for February).
fn max_day(month: u32) -> u32 {
    match month {
        2 => 29,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// One field's set of values, as a bit mask.
fn field(s: &str, spec: &Spec) -> Result<u64, String> {
    if s.is_empty() {
        return Err(format!("the {} field is empty", spec.what));
    }
    let mut mask = 0u64;
    for item in s.split(',') {
        mask |=
            item_mask(item, spec).map_err(|e| format!("{e} in the {} field ({s:?})", spec.what))?;
    }
    Ok(mask)
}

fn item_mask(item: &str, spec: &Spec) -> Result<u64, String> {
    let (range, step) = match item.split_once('/') {
        Some((r, st)) => {
            let n: u32 = st.parse().map_err(|_| format!("{st:?} is not a step"))?;
            if n == 0 {
                return Err("a step of 0".into());
            }
            (r, Some(n))
        }
        None => (item, None),
    };
    let (lo, hi) = if range == "*" {
        (spec.min, spec.max)
    } else if let Some((a, b)) = range.split_once('-') {
        let (a, b) = (value(a, spec)?, value(b, spec)?);
        if a > b {
            return Err(format!("the range {range:?} runs backwards"));
        }
        (a, b)
    } else {
        let a = value(range, spec)?;
        // `a/n` runs from `a` to the field's end
        (a, if step.is_some() { spec.max } else { a })
    };
    let mut mask = 0u64;
    let mut v = lo;
    while v <= hi {
        mask |= 1 << v;
        v += step.unwrap_or(1);
    }
    Ok(mask)
}

fn value(s: &str, spec: &Spec) -> Result<u32, String> {
    let lower = s.to_ascii_lowercase();
    if let Some(i) = spec.names.iter().position(|n| *n == lower) {
        return Ok(spec.names_from + i as u32);
    }
    let n: u32 = s
        .parse()
        .map_err(|_| format!("{s:?} is not a {}", spec.what))?;
    if !(spec.min..=spec.max).contains(&n) {
        return Err(format!(
            "{n} is out of {}..={} for the {}",
            spec.min, spec.max, spec.what
        ));
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn next(cron: &str, after: &str) -> String {
        Cron::parse(cron)
            .unwrap()
            .next_after(at(after))
            .unwrap()
            .to_rfc3339()
    }

    #[test]
    fn every_minute_is_strictly_after() {
        assert_eq!(
            next("* * * * *", "2026-11-20T10:00:00Z"),
            "2026-11-20T10:01:00+00:00"
        );
        assert_eq!(
            next("* * * * *", "2026-11-20T10:00:59.900Z"),
            "2026-11-20T10:01:00+00:00"
        );
    }

    #[test]
    fn hourly_at_quarter_past() {
        assert_eq!(
            next("15 * * * *", "2026-11-20T10:14:00Z"),
            "2026-11-20T10:15:00+00:00"
        );
        assert_eq!(
            next("15 * * * *", "2026-11-20T10:15:00Z"),
            "2026-11-20T11:15:00+00:00"
        );
        assert_eq!(
            next("15 * * * *", "2026-12-31T23:20:00Z"),
            "2027-01-01T00:15:00+00:00"
        );
    }

    #[test]
    fn weekdays_at_nine() {
        // 2026-11-20 is a Friday
        assert_eq!(
            next("0 9 * * 1-5", "2026-11-20T08:00:00Z"),
            "2026-11-20T09:00:00+00:00"
        );
        assert_eq!(
            next("0 9 * * 1-5", "2026-11-20T09:00:00Z"),
            "2026-11-23T09:00:00+00:00"
        );
        assert_eq!(
            next("0 9 * * mon-fri", "2026-11-21T12:00:00Z"),
            "2026-11-23T09:00:00+00:00"
        );
        // Sunday as 0, 7 and its name
        for sunday in ["0", "7", "SUN"] {
            assert_eq!(
                next(&format!("30 6 * * {sunday}"), "2026-11-20T00:00:00Z"),
                "2026-11-22T06:30:00+00:00"
            );
        }
    }

    #[test]
    fn month_boundaries() {
        assert_eq!(
            next("0 0 1 * *", "2026-11-20T00:00:00Z"),
            "2026-12-01T00:00:00+00:00"
        );
        assert_eq!(
            next("0 0 1 * *", "2026-12-15T00:00:00Z"),
            "2027-01-01T00:00:00+00:00"
        );
        // the 31st skips the months without one
        assert_eq!(
            next("0 12 31 * *", "2026-11-01T00:00:00Z"),
            "2026-12-31T12:00:00+00:00"
        );
        assert_eq!(
            next("0 12 31 * *", "2027-01-31T12:00:00Z"),
            "2027-03-31T12:00:00+00:00"
        );
        // 29 February waits for a leap year
        assert_eq!(
            next("0 0 29 2 *", "2026-03-01T00:00:00Z"),
            "2028-02-29T00:00:00+00:00"
        );
        assert_eq!(
            next("0 0 1 jan,jul *", "2026-07-01T00:00:00Z"),
            "2027-01-01T00:00:00+00:00"
        );
    }

    #[test]
    fn steps_ranges_and_lists() {
        assert_eq!(
            next("*/15 * * * *", "2026-11-20T10:16:00Z"),
            "2026-11-20T10:30:00+00:00"
        );
        assert_eq!(
            next("5/20 * * * *", "2026-11-20T10:46:00Z"),
            "2026-11-20T11:05:00+00:00"
        );
        assert_eq!(
            next("0 8-18/2 * * *", "2026-11-20T18:01:00Z"),
            "2026-11-21T08:00:00+00:00"
        );
        assert_eq!(
            next("0 0,12 * * *", "2026-11-20T00:00:00Z"),
            "2026-11-20T12:00:00+00:00"
        );
    }

    #[test]
    fn day_of_month_or_day_of_week() {
        // both restricted: the 1st or a Monday
        assert_eq!(
            next("0 0 1 * 1", "2026-11-20T00:00:00Z"),
            "2026-11-23T00:00:00+00:00"
        );
        assert_eq!(
            next("0 0 1 * 1", "2026-11-24T00:00:00Z"),
            "2026-11-30T00:00:00+00:00"
        );
        assert_eq!(
            next("0 0 1 * 1", "2026-11-30T00:00:00Z"),
            "2026-12-01T00:00:00+00:00"
        );
        // a day of the week with `*/1` days of the month: both must match
        assert_eq!(
            next("0 0 */1 * 1", "2026-11-20T00:00:00Z"),
            "2026-11-23T00:00:00+00:00"
        );
    }

    #[test]
    fn refuses_what_is_not_a_cron() {
        for bad in [
            "",
            "* * * *",
            "* * * * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * 32 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "5-1 * * * *",
            "a * * * *",
            "1,,2 * * * *",
            "@daily",
            "0 0 L * *",
            "0 0 30 2 *",
            "0 0 31 4,6 *",
        ] {
            assert!(Cron::parse(bad).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn keeps_its_source() {
        let c = Cron::parse(" 0  9 * *   1-5 ").unwrap();
        assert_eq!(c.as_str(), "0 9 * * 1-5");
        assert_eq!(c.to_string(), "0 9 * * 1-5");
        // 30 February never comes, but a weekday does
        assert!(Cron::parse("0 0 30 2 1").is_ok());
    }
}
