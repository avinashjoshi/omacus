//! Dates and time zones. fend-core ships without a clock, so these are handled here:
//! `today`, `tomorrow + 2 weeks`, `now - 90 minutes`, `3pm PST in IST`,
//! `time in Tokyo`, `10:30 London to New York`, `3pm PST on Dec 1 in London`,
//! and counting days: `days until Dec 25`, `workdays in November`.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{
    DateTime, Datelike, Duration, Local, Months, NaiveDate, NaiveTime, TimeZone, Timelike, Utc,
    Weekday,
};
use chrono_tz::Tz;
use regex::Regex;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

re!(
    DATE_EXPR,
    r"(?i)^(today|now|tomorrow|yesterday)(?:\s*([+-])\s*(\d+)\s*(minute|min|hour|hr|day|week|month|year)s?)?$"
);
re!(
    ZONE_NOW,
    r"(?i)^(?:what\s+)?(?:time|now)\s+(?:in|at)\s+(.+?)\??$"
);
re!(CONVERT, r"(?i)^(.+?)\s+(?:in|to|as)\s+(.+)$");
// One "<number> <unit>" of a duration; bare m, w and d are left to fend
// (metres, watts, days are ambiguous on their own).
re!(
    DURATION_PART,
    r"(?i)^(\d+(?:\.\d+)?)\s*(weeks?|days?|hours?|hrs?|hr|h|minutes?|mins?|min)\b"
);
// "in 3 hours", "90 minutes from now", optionally "in <zone>".
re!(
    FROM_NOW,
    r"(?i)^(?:in\s+((?:\d+(?:\.\d+)?\s*(?:weeks?|days?|hours?|hrs?|hr|h|minutes?|mins?|min)\b\s*)+)|((?:\d+(?:\.\d+)?\s*(?:weeks?|days?|hours?|hrs?|hr|h|minutes?|mins?|min)\b\s*)+)from\s+now)(?:\s+(?:in|at)\s+(.+?))?$"
);
// "<time> + <duration>", optionally "in <zone>".
re!(
    SHIFT,
    r"(?i)^(.+?)\s*([+-])\s*((?:\d+(?:\.\d+)?\s*(?:weeks?|days?|hours?|hrs?|hr|h|minutes?|mins?|min)\b\s*)+)(?:\s+(?:in|to)\s+(.+))?$"
);
// A date on a time to convert: "on Dec 1", "1 December 2026", "2026-12-01",
// "tomorrow", "Friday", "next Monday".
re!(
    DATE_PART,
    r"(?ix)\b(?:on\s+)?(?:
        (today|tomorrow|yesterday)
      | (next\s+)?(monday|mon|tuesday|tues|tue|wednesday|wed|thursday|thurs|thu|friday|fri|saturday|sat|sunday|sun)
      | (jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\.?\s+(\d{1,2})(?:st|nd|rd|th)?(?:,?\s+(\d{4}))?
      | (\d{1,2})(?:st|nd|rd|th)?\s+(jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\.?(?:,?\s+(\d{4}))?
      | (\d{4})-(\d{2})-(\d{2})
    )\b"
);
// "days until Dec 25", "weeks since Jan 1", "until Christmas", "workdays till Friday".
re!(
    SPAN,
    r"(?i)^(?:how\s+(?:many|long)\s+)?(?:(days|weeks|months|workdays|work\s+days|weekdays|business\s+days)\s+)?(until|till|til|before|since)\s+(.+?)\??$"
);
// "days between Mar 1 and Jun 30", "workdays from Dec 1 to Dec 24".
re!(
    BETWEEN,
    r"(?i)^(?:how\s+many\s+)?(days|weeks|workdays|work\s+days|weekdays|business\s+days)\s+(?:between|from)\s+(.+?)\s+(?:and|to|until|till)\s+(.+?)\??$"
);
// "days in February", "workdays in November 2026".
re!(
    IN_MONTH,
    r"(?i)^(?:how\s+many\s+)?(days|workdays|work\s+days|weekdays|business\s+days)\s+in\s+(jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\.?(?:\s+(\d{4}))?\??$"
);
/// Holidays on the same date every year, for "days until Christmas".
const HOLIDAYS: &[(&str, u32, u32)] = &[
    ("christmas eve", 12, 24),
    ("christmas", 12, 25),
    ("xmas", 12, 25),
    ("new year's eve", 12, 31),
    ("new years eve", 12, 31),
    ("new year's day", 1, 1),
    ("new year's", 1, 1),
    ("new years", 1, 1),
    ("new year", 1, 1),
    ("halloween", 10, 31),
    ("valentine's day", 2, 14),
    ("valentines day", 2, 14),
    ("valentine's", 2, 14),
];
re!(
    CLOCK,
    r"(?i)^(?:(noon|midnight|now)|(\d{1,2})(?::(\d{2}))?\s*(am|pm|a\.m\.|p\.m\.)?)\s*(.*)$"
);

/// Common abbreviations, mapped to a region so daylight saving is handled
/// (`PST` in July means Pacific time, i.e. PDT).
const ABBREVIATIONS: &[(&str, &str)] = &[
    ("PST", "America/Los_Angeles"),
    ("PDT", "America/Los_Angeles"),
    ("PT", "America/Los_Angeles"),
    ("MST", "America/Denver"),
    ("MDT", "America/Denver"),
    ("MT", "America/Denver"),
    ("CST", "America/Chicago"),
    ("CDT", "America/Chicago"),
    ("CT", "America/Chicago"),
    ("EST", "America/New_York"),
    ("EDT", "America/New_York"),
    ("ET", "America/New_York"),
    ("AKST", "America/Anchorage"),
    ("HST", "Pacific/Honolulu"),
    ("UTC", "UTC"),
    ("GMT", "Etc/GMT"), // a fixed offset; London in summer is BST
    ("BST", "Europe/London"),
    ("WET", "Europe/Lisbon"),
    ("CET", "Europe/Paris"),
    ("CEST", "Europe/Paris"),
    ("EET", "Europe/Athens"),
    ("MSK", "Europe/Moscow"),
    ("IST", "Asia/Kolkata"),
    ("PKT", "Asia/Karachi"),
    ("NPT", "Asia/Kathmandu"),
    ("GST", "Asia/Dubai"),
    ("SGT", "Asia/Singapore"),
    ("HKT", "Asia/Hong_Kong"),
    ("JST", "Asia/Tokyo"),
    ("KST", "Asia/Seoul"),
    ("AEST", "Australia/Sydney"),
    ("AEDT", "Australia/Sydney"),
    ("NZST", "Pacific/Auckland"),
    ("NZDT", "Pacific/Auckland"),
];

const PLACES: &[(&str, &str)] = &[
    ("india", "Asia/Kolkata"),
    ("mumbai", "Asia/Kolkata"),
    ("delhi", "Asia/Kolkata"),
    ("new delhi", "Asia/Kolkata"),
    ("bangalore", "Asia/Kolkata"),
    ("bengaluru", "Asia/Kolkata"),
    ("chennai", "Asia/Kolkata"),
    ("hyderabad", "Asia/Kolkata"),
    ("pune", "Asia/Kolkata"),
    ("kolkata", "Asia/Kolkata"),
    ("sf", "America/Los_Angeles"),
    ("san francisco", "America/Los_Angeles"),
    ("seattle", "America/Los_Angeles"),
    ("california", "America/Los_Angeles"),
    ("nyc", "America/New_York"),
    ("boston", "America/New_York"),
    ("miami", "America/New_York"),
    ("washington", "America/New_York"),
    ("austin", "America/Chicago"),
    ("dallas", "America/Chicago"),
    ("houston", "America/Chicago"),
    ("uk", "Europe/London"),
    ("england", "Europe/London"),
    ("germany", "Europe/Berlin"),
    ("france", "Europe/Paris"),
    ("japan", "Asia/Tokyo"),
    ("china", "Asia/Shanghai"),
    ("beijing", "Asia/Shanghai"),
    ("uae", "Asia/Dubai"),
    ("dubai", "Asia/Dubai"),
    ("australia", "Australia/Sydney"),
    ("utc", "UTC"),
    ("local", "local"),
    ("here", "local"),
];

/// Every IANA zone by full name and by city ("new york", "tokyo").
static ZONES: LazyLock<HashMap<String, Tz>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    for tz in chrono_tz::TZ_VARIANTS {
        let name = tz.name();
        map.insert(name.to_ascii_lowercase(), tz);
        if let Some(city) = name.rsplit('/').next() {
            map.entry(city.replace('_', " ").to_ascii_lowercase())
                .or_insert(tz);
        }
    }
    map
});

#[derive(Clone, Copy)]
enum Zone {
    Local,
    Tz(Tz),
}

impl Zone {
    fn at(self, utc: DateTime<Utc>) -> (NaiveDate, String) {
        match self {
            Zone::Local => {
                let t = utc.with_timezone(&Local);
                (
                    t.date_naive(),
                    format!("{} {}", clock(t.time()), t.format("%Z")),
                )
            }
            Zone::Tz(tz) => {
                let t = utc.with_timezone(&tz);
                (
                    t.date_naive(),
                    format!(
                        "{} {}",
                        clock(t.time()),
                        label(tz, &t.format("%Z").to_string())
                    ),
                )
            }
        }
    }

    /// The UTC instant for a wall-clock time on `date` in this zone.
    fn to_utc(self, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
        let naive = date.and_time(time);
        match self {
            Zone::Local => Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|t| t.to_utc()),
            Zone::Tz(tz) => tz
                .from_local_datetime(&naive)
                .earliest()
                .map(|t| t.to_utc()),
        }
    }
}

fn zone(name: &str) -> Option<Zone> {
    let name = name.trim().trim_end_matches(" time");
    let upper = name.to_ascii_uppercase();
    let lower = name.to_ascii_lowercase();
    let iana = ABBREVIATIONS
        .iter()
        .find(|(a, _)| *a == upper)
        .or_else(|| PLACES.iter().find(|(p, _)| *p == lower))
        .map(|(_, z)| *z);
    match iana {
        Some("local") => Some(Zone::Local),
        Some(z) => z.parse().ok().map(Zone::Tz),
        None => ZONES.get(&lower).copied().map(Zone::Tz),
    }
}

/// The zone's abbreviation. Some zones only have a numeric one ("+04" for Dubai);
/// use the common name from ABBREVIATIONS for those when there is one.
fn label(tz: Tz, abbr: &str) -> String {
    if !abbr.starts_with(['+', '-']) {
        return abbr.to_string();
    }
    ABBREVIATIONS
        .iter()
        .find(|(_, z)| *z == tz.name())
        .map_or_else(|| format!("UTC{abbr}"), |(a, _)| (*a).to_string())
}

pub fn eval(line: &str) -> Option<String> {
    let now = Utc::now();
    eval_span(line, Local::now().date_naive())
        .or_else(|| eval_date(line))
        .or_else(|| eval_relative(line, now))
        .or_else(|| eval_zone(line, now))
}

/// Names offered by autocomplete.
pub fn completion_words() -> impl Iterator<Item = &'static str> {
    ABBREVIATIONS.iter().map(|(a, _)| *a)
}

fn eval_zone(line: &str, now: DateTime<Utc>) -> Option<String> {
    if let Some(c) = ZONE_NOW.captures(line) {
        return Some(zone(&c[1])?.at(now).1);
    }

    let c = CONVERT.captures(line)?;
    let target = zone(&c[2])?;
    let (instant, source, date) = instant(&c[1], now)?;
    let (from_date, _) = source.at(instant);
    let (to_date, shown) = target.at(instant);
    if date.is_some() {
        // A date was asked for, so say which date it is there.
        return Some(format!("{shown}, {}", day_label(to_date, source.at(now).0)));
    }
    Some(with_day_change(shown, from_date, to_date))
}

/// "(next day)" or "(prev day)" when a time lands on another date.
fn with_day_change(shown: String, from: NaiveDate, to: NaiveDate) -> String {
    match (to - from).num_days() {
        1 => format!("{shown} (next day)"),
        -1 => format!("{shown} (prev day)"),
        _ => shown,
    }
}

/// The moment a time like "Standup 9:30am PST on Dec 1" means: a label is
/// skipped, a date is optional, and no zone means local time.
fn instant(lhs: &str, now: DateTime<Utc>) -> Option<(DateTime<Utc>, Zone, Option<DateSpec>)> {
    let (lhs, date) = take_date(lhs.trim());
    // Allow a label first: "Sunset 6:12pm IST in Sydney".
    let t = std::iter::successors(Some(lhs.as_str()), |s| {
        s.split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim_start())
    })
    .find_map(|s| CLOCK.captures(s))?;
    let rest = t.get(5).map_or("", |m| m.as_str()).trim();
    let source = if rest.is_empty() {
        Zone::Local
    } else {
        zone(rest)?
    };

    if t.get(1)
        .is_some_and(|m| m.as_str().eq_ignore_ascii_case("now"))
    {
        return Some((now, source, date));
    }
    let time = match t.get(1).map(|m| m.as_str().to_ascii_lowercase()) {
        Some(w) if w == "noon" => NaiveTime::from_hms_opt(12, 0, 0)?,
        Some(_) => NaiveTime::from_hms_opt(0, 0, 0)?,
        None => {
            let mut hour: u32 = t[2].parse().ok()?;
            let minute: u32 = t.get(3).map_or(Ok(0), |m| m.as_str().parse()).ok()?;
            let meridiem = t.get(4).map(|m| m.as_str().to_ascii_lowercase());
            // A bare "15 in IST" is a number, not a time.
            if meridiem.is_none() && t.get(3).is_none() && rest.is_empty() {
                return None;
            }
            match meridiem.as_deref().map(|m| m.starts_with('p')) {
                Some(true) if hour < 12 => hour += 12,
                Some(false) if hour == 12 => hour = 0,
                _ => {}
            }
            NaiveTime::from_hms_opt(hour, minute, 0)?
        }
    };
    let today = source.at(now).0;
    let day = match date {
        Some(d) => d.resolve(today)?,
        None => today,
    };
    Some((source.to_utc(day, time)?, source, date))
}

/// A length of time: "45 min", "2 hours", "1 hour 30 minutes", "3 days".
fn duration(text: &str) -> Option<Duration> {
    let mut seconds = 0.0;
    let mut rest = text.trim();
    while !rest.is_empty() {
        let c = DURATION_PART.captures(rest)?;
        let n: f64 = c[1].parse().ok()?;
        let unit = c[2].to_ascii_lowercase();
        seconds += n * match unit.as_bytes()[0] {
            b'w' => 7.0 * 86400.0,
            b'd' => 86400.0,
            b'h' => 3600.0,
            _ => 60.0,
        };
        rest = rest[c.get(0)?.end()..].trim_start();
    }
    // Spans past chrono's range (about 290,000 years) aren't times anyone means.
    if !seconds.is_finite() || seconds.abs() > 1e13 {
        return None;
    }
    Duration::try_seconds(seconds.round() as i64)
}

/// "Fri 23 Oct", with the year when it isn't this one.
fn day_label(day: NaiveDate, today: NaiveDate) -> String {
    if day.year() == today.year() {
        day.format("%a %-d %b").to_string()
    } else {
        day.format("%a %-d %b %Y").to_string()
    }
}

/// "in 3 hours", "90 minutes from now", "in 2 days in Tokyo",
/// "3pm + 45 min", "9:30am PST + 2 hours in London".
fn eval_relative(line: &str, now: DateTime<Utc>) -> Option<String> {
    if let Some(c) = FROM_NOW.captures(line) {
        let span = duration(c.get(1).or(c.get(2))?.as_str())?;
        let target = match c.get(3) {
            Some(z) => zone(z.as_str())?,
            None => Zone::Local,
        };
        let then = now.checked_add_signed(span)?;
        let (today, _) = target.at(now);
        let (day, shown) = target.at(then);
        let shown = match target {
            Zone::Local => clock(then.with_timezone(&Local).time()),
            Zone::Tz(_) => shown,
        };
        return Some(if day == today {
            shown
        } else {
            format!("{shown}, {}", day_label(day, today))
        });
    }

    let c = SHIFT.captures(line)?;
    let (start, source, _) = instant(&c[1], now)?;
    let span = duration(&c[3])?;
    let then = if &c[2] == "-" {
        start.checked_sub_signed(span)?
    } else {
        start.checked_add_signed(span)?
    };
    let target = match c.get(4) {
        Some(z) => zone(z.as_str())?,
        None => source,
    };
    let (from_date, _) = source.at(start);
    let (to_date, shown) = target.at(then);
    // Moving more than a day: say which day, not just "next day".
    if (to_date - from_date).num_days().abs() > 1 {
        let shown = match (c.get(4), target) {
            (None, Zone::Local) => clock(then.with_timezone(&Local).time()),
            _ => shown,
        };
        return Some(format!("{shown}, {}", day_label(to_date, from_date)));
    }
    // Plain clock math in local time doesn't need a zone name.
    let shown = match (c.get(4), target) {
        (None, Zone::Local) => clock(then.with_timezone(&Local).time()),
        _ => shown,
    };
    Some(with_day_change(shown, from_date, to_date))
}

/// A date written on a time, resolved against "today" in the source zone.
#[derive(Clone, Copy)]
enum DateSpec {
    Offset(i64),
    Weekday {
        day: Weekday,
        next: bool,
    },
    MonthDay {
        month: u32,
        day: u32,
        year: Option<i32>,
    },
}

impl DateSpec {
    fn resolve(self, today: NaiveDate) -> Option<NaiveDate> {
        match self {
            DateSpec::Offset(n) => today.checked_add_signed(Duration::days(n)),
            DateSpec::Weekday { day, next } => {
                let ahead = (7 + day.num_days_from_monday() as i64
                    - today.weekday().num_days_from_monday() as i64)
                    % 7;
                // "next Friday" on a Friday is a week away; "Friday" is today.
                let ahead = if next && ahead == 0 { 7 } else { ahead };
                Some(today + Duration::days(ahead))
            }
            DateSpec::MonthDay {
                month,
                day,
                year: Some(y),
            } => NaiveDate::from_ymd_opt(y, month, day),
            DateSpec::MonthDay {
                month,
                day,
                year: None,
            } => {
                // Without a year, the next one: "Jan 5" in October is next January.
                // Dates up to two months back stay this year ("what time was it?").
                let this = NaiveDate::from_ymd_opt(today.year(), month, day)?;
                if this < today - Duration::days(60) {
                    NaiveDate::from_ymd_opt(today.year() + 1, month, day)
                } else {
                    Some(this)
                }
            }
        }
    }
}

/// Split a date phrase off the time part: "3pm PST on Dec 1" -> ("3pm PST", Dec 1).
fn take_date(lhs: &str) -> (String, Option<DateSpec>) {
    let Some(c) = DATE_PART.captures(lhs) else {
        return (lhs.to_string(), None);
    };
    let month = |m: &str| -> Option<u32> {
        let m = m.to_ascii_lowercase();
        [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ]
        .iter()
        .position(|n| m.starts_with(n))
        .map(|i| i as u32 + 1)
    };
    let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
    let spec = || -> Option<DateSpec> {
        Some(if let Some(w) = c.get(1) {
            DateSpec::Offset(match w.as_str().to_ascii_lowercase().as_str() {
                "tomorrow" => 1,
                "yesterday" => -1,
                _ => 0,
            })
        } else if let Some(w) = c.get(3) {
            let day = match &w.as_str().to_ascii_lowercase()[..3] {
                "mon" => Weekday::Mon,
                "tue" => Weekday::Tue,
                "wed" => Weekday::Wed,
                "thu" => Weekday::Thu,
                "fri" => Weekday::Fri,
                "sat" => Weekday::Sat,
                _ => Weekday::Sun,
            };
            DateSpec::Weekday {
                day,
                next: c.get(2).is_some(),
            }
        } else if let Some(m) = c.get(4) {
            DateSpec::MonthDay {
                month: month(m.as_str())?,
                day: num(5)?,
                year: num(6).map(|y| y as i32),
            }
        } else if let Some(m) = c.get(8) {
            DateSpec::MonthDay {
                month: month(m.as_str())?,
                day: num(7)?,
                year: num(9).map(|y| y as i32),
            }
        } else {
            DateSpec::MonthDay {
                month: num(11)?,
                day: num(12)?,
                year: num(10).map(|y| y as i32),
            }
        })
    };
    let Some(spec) = spec() else {
        return (lhs.to_string(), None);
    };
    let whole = c.get(0).unwrap();
    let rest = format!("{} {}", &lhs[..whole.start()], &lhs[whole.end()..]);
    (
        rest.split_whitespace().collect::<Vec<_>>().join(" "),
        Some(spec),
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Count {
    Days,
    Weeks,
    Months,
    Workdays,
}

fn count_unit(word: Option<&str>) -> Count {
    match word.map(|w| w.to_ascii_lowercase()) {
        Some(w) if w.starts_with("week") && w != "weekdays" => Count::Weeks,
        Some(w) if w.starts_with("month") => Count::Months,
        Some(w) if w.contains("day") && w != "days" => Count::Workdays,
        _ => Count::Days,
    }
}

/// A date on its own ("Dec 25", "Friday", "Christmas", "2026-12-01"), the next one
/// when `ahead`, the last one otherwise.
fn lone_date(text: &str, today: NaiveDate, ahead: bool) -> Option<NaiveDate> {
    let text = text.trim().trim_start_matches("the ").trim();
    let lower = text.to_ascii_lowercase();
    let (month, day, year) = if let Some(&(_, m, d)) = HOLIDAYS.iter().find(|(n, ..)| *n == lower) {
        (m, d, None)
    } else {
        let (rest, spec) = take_date(text);
        let spec = spec.filter(|_| rest.is_empty())?;
        match spec {
            DateSpec::MonthDay { month, day, year } => (month, day, year),
            DateSpec::Weekday { day, next } if !ahead => {
                // "since Friday": the last one.
                let back = (7 + today.weekday().num_days_from_monday() as i64
                    - day.num_days_from_monday() as i64)
                    % 7;
                let back = if next || back == 0 { back + 7 } else { back };
                return today.checked_sub_signed(Duration::days(back));
            }
            other => return other.resolve(today),
        }
    };
    if let Some(y) = year {
        return NaiveDate::from_ymd_opt(y, month, day);
    }
    let this = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    let year = match (ahead, this < today, this > today) {
        (true, true, _) => today.year() + 1,
        (false, _, true) => today.year() - 1,
        _ => today.year(),
    };
    NaiveDate::from_ymd_opt(year, month, day)
}

/// Mondays to Fridays from `from` up to, not including, `to`.
fn workdays(from: NaiveDate, to: NaiveDate) -> i64 {
    let (a, b) = if from <= to { (from, to) } else { (to, from) };
    let days = (b - a).num_days();
    let full = days / 7 * 5;
    let partial = (0..days % 7)
        .filter(|i| {
            let wd = (a.weekday().num_days_from_monday() as i64 + days / 7 * 7 + i) % 7;
            wd < 5
        })
        .count() as i64;
    full + partial
}

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The span from `from` to `to` in the unit asked for.
fn span(from: NaiveDate, to: NaiveDate, unit: Count) -> String {
    let days = (to - from).num_days().abs();
    match unit {
        Count::Days => plural(days, "day", "days"),
        Count::Workdays => {
            // Working days left before the date (or since it), not counting today.
            let (a, b) = if from <= to { (from, to) } else { (to, from) };
            plural(
                workdays(a.succ_opt().unwrap_or(a), b),
                "workday",
                "workdays",
            )
        }
        Count::Weeks => match (days / 7, days % 7) {
            (w, 0) => plural(w, "week", "weeks"),
            (w, d) => format!(
                "{}, {}",
                plural(w, "week", "weeks"),
                plural(d, "day", "days")
            ),
        },
        Count::Months => {
            let (a, b) = if from <= to { (from, to) } else { (to, from) };
            let mut months = (b.year() - a.year()) * 12 + b.month() as i32 - a.month() as i32;
            if b.day() < a.day() {
                months -= 1;
            }
            let anchor = a
                .checked_add_months(Months::new(months.max(0) as u32))
                .unwrap_or(a);
            let rest = (b - anchor).num_days();
            match (months, rest) {
                (m, 0) => plural(m as i64, "month", "months"),
                (0, d) => plural(d, "day", "days"),
                (m, d) => format!(
                    "{}, {}",
                    plural(m as i64, "month", "months"),
                    plural(d, "day", "days")
                ),
            }
        }
    }
}

fn eval_span(line: &str, today: NaiveDate) -> Option<String> {
    let line = line.trim();
    if let Some(c) = IN_MONTH.captures(line) {
        let month = [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ]
        .iter()
        .position(|m| c[2].to_ascii_lowercase().starts_with(m))? as u32
            + 1;
        let year = match c.get(3) {
            Some(y) => y.as_str().parse().ok()?,
            // A month well past is next year's ("workdays in March", asked in October).
            None if month + 2 < today.month() => today.year() + 1,
            None => today.year(),
        };
        let first = NaiveDate::from_ymd_opt(year, month, 1)?;
        let next = first.checked_add_months(Months::new(1))?;
        return Some(match count_unit(Some(&c[1])) {
            Count::Workdays => plural(workdays(first, next), "workday", "workdays"),
            _ => plural((next - first).num_days(), "day", "days"),
        });
    }
    if let Some(c) = BETWEEN.captures(line) {
        let a = lone_date(&c[2], today, true)?;
        let b = lone_date(&c[3], today, true)?;
        let names_year = |t: &str| {
            t.split(|ch: char| !ch.is_ascii_digit())
                .any(|n| n.len() == 4)
        };
        // "Dec 1 to Jan 5": the January after.
        let b = if b < a && !names_year(&c[3]) {
            b.with_year(a.year() + 1).filter(|d| *d > a).unwrap_or(b)
        } else {
            b
        };
        let unit = count_unit(Some(&c[1]));
        return Some(if unit == Count::Workdays {
            plural(workdays(a.min(b), a.max(b)), "workday", "workdays")
        } else {
            span(a, b, unit)
        });
    }
    let c = SPAN.captures(line)?;
    let ahead = !c[2].eq_ignore_ascii_case("since");
    let date = lone_date(&c[3], today, ahead)?;
    let unit = count_unit(c.get(1).map(|m| m.as_str()));
    if date == today {
        return Some("today".into());
    }
    let shown = span(today, date, unit);
    // The wrong side of today: "until" a date that's past says so.
    Some(match (ahead, date > today) {
        (true, false) => format!("{shown} ago"),
        (false, true) => format!("in {shown}"),
        _ => shown,
    })
}

fn clock(t: NaiveTime) -> String {
    let (pm, hour) = t.hour12();
    format!("{hour}:{:02} {}", t.minute(), if pm { "PM" } else { "AM" })
}

fn eval_date(line: &str) -> Option<String> {
    let c = DATE_EXPR.captures(line)?;
    let base = c[1].to_ascii_lowercase();
    let now = Local::now();
    let mut t = match base.as_str() {
        "tomorrow" => now.checked_add_signed(Duration::days(1))?,
        "yesterday" => now.checked_sub_signed(Duration::days(1))?,
        _ => now,
    };
    let mut show_time = base == "now";
    if let (Some(sign), Some(n), Some(unit)) = (c.get(2), c.get(3), c.get(4)) {
        let n: i64 = n.as_str().parse().ok()?;
        let n = if sign.as_str() == "-" { -n } else { n };
        let unit = unit.as_str().to_ascii_lowercase();
        let span = match unit.as_str() {
            "minute" | "min" => Duration::try_minutes(n),
            "hour" | "hr" => Duration::try_hours(n),
            "day" => Duration::try_days(n),
            "week" => Duration::try_weeks(n),
            _ => None,
        };
        t = match unit.as_str() {
            "minute" | "min" | "hour" | "hr" | "day" | "week" => t.checked_add_signed(span?)?,
            "month" | "year" => {
                let per = if unit == "year" { 12 } else { 1 };
                let months = u32::try_from(n.unsigned_abs() * per).ok()?;
                if n < 0 {
                    t.checked_sub_months(Months::new(months))?
                } else {
                    t.checked_add_months(Months::new(months))?
                }
            }
            _ => return None,
        };
        show_time |= matches!(unit.as_str(), "minute" | "min" | "hour" | "hr");
    }
    Some(if show_time {
        t.format("%a, %-d %b %Y, %-I:%M %p").to_string()
    } else {
        t.format("%A, %-d %B %Y").to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 16 Oct 2026, 12:00 UTC: US is on daylight time, India is UTC+5:30.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap()
    }

    fn conv(line: &str) -> Option<String> {
        eval_zone(line, now())
    }

    #[test]
    fn converts_between_zones() {
        assert!(conv("3pm PST in IST").unwrap().starts_with("3:30 AM IST"));
        assert!(conv("3pm PST in IST").unwrap().ends_with("(next day)"));
        assert_eq!(conv("10:30 London to New York").unwrap(), "5:30 AM EDT");
        assert_eq!(
            conv("Sunset 6:12pm IST in Sydney").unwrap(),
            "11:42 PM AEDT"
        );
        assert_eq!(conv("Standup 9:30am PST in IST").unwrap(), "10:00 PM IST");
        assert_eq!(conv("noon UTC in tokyo").unwrap(), "9:00 PM JST");
        assert_eq!(conv("9am IST in EST").unwrap(), "11:30 PM EDT (prev day)");
        assert_eq!(conv("7am Singapore in Dubai").unwrap(), "3:00 AM GST");
        assert_eq!(conv("noon UTC in Asia/Kabul").unwrap(), "4:30 PM UTC+0430");
    }

    #[test]
    fn counts_days() {
        // Friday 16 Oct 2026.
        let today = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let span = |l: &str| eval_span(l, today);
        assert_eq!(span("days until Dec 25").as_deref(), Some("70 days"));
        assert_eq!(span("until Christmas").as_deref(), Some("70 days"));
        assert_eq!(
            span("how many days until Christmas?").as_deref(),
            Some("70 days")
        );
        assert_eq!(span("weeks until Dec 25").as_deref(), Some("10 weeks"));
        assert_eq!(span("weeks until New Year's").as_deref(), Some("11 weeks"));
        assert_eq!(
            span("months until Dec 25").as_deref(),
            Some("2 months, 9 days")
        );
        assert_eq!(span("days since Jan 1").as_deref(), Some("288 days"));
        assert_eq!(
            span("days until Oct 1 2026").as_deref(),
            Some("15 days ago")
        );
        assert_eq!(span("days until today").as_deref(), Some("today"));
        assert_eq!(span("days until Friday").as_deref(), Some("today"));
        assert_eq!(span("days until next Friday").as_deref(), Some("7 days"));
        assert_eq!(span("days since Monday").as_deref(), Some("4 days"));
        // Mon 19 to Fri 23, then the 26th: 6 working days before the 27th.
        assert_eq!(span("workdays until Oct 27").as_deref(), Some("6 workdays"));
        assert_eq!(
            span("days between Mar 1 2026 and Jun 30 2026").as_deref(),
            Some("121 days")
        );
        assert_eq!(
            span("workdays from Nov 2 2026 to Nov 9 2026").as_deref(),
            Some("5 workdays")
        );
        assert_eq!(
            span("days from Dec 20 to Jan 5").as_deref(),
            Some("16 days")
        );
        assert_eq!(span("days in February 2028").as_deref(), Some("29 days"));
        assert_eq!(span("workdays in November").as_deref(), Some("21 workdays"));
        assert_eq!(span("days until the meeting"), None);
    }

    #[test]
    fn converts_on_a_date() {
        // 16 Oct 2026 is a Friday. On 1 Dec, California and London are on standard time.
        assert_eq!(
            conv("3pm PST on Dec 1 in London").unwrap(),
            "11:00 PM GMT, Tue 1 Dec"
        );
        assert_eq!(
            conv("Dec 1 3pm PST in London").unwrap(),
            "11:00 PM GMT, Tue 1 Dec"
        );
        assert_eq!(
            conv("9am PST tomorrow in Tokyo").unwrap(),
            "1:00 AM JST, Sun 18 Oct"
        );
        assert_eq!(
            conv("on Friday 9am EST in London").unwrap(),
            "2:00 PM BST, Fri 16 Oct"
        );
        assert_eq!(
            conv("next Friday 9am EST in London").unwrap(),
            "2:00 PM BST, Fri 23 Oct"
        );
        assert_eq!(
            conv("Launch 2026-11-02 10am ET in Berlin").unwrap(),
            "4:00 PM CET, Mon 2 Nov"
        );
        assert_eq!(
            conv("Jan 5 9am PST in Tokyo").unwrap(),
            "2:00 AM JST, Wed 6 Jan 2027"
        );
        assert_eq!(
            conv("1 December 2026 noon UTC in NYC").unwrap(),
            "7:00 AM EST, Tue 1 Dec"
        );
        // Words that only start like a day name are labels.
        assert_eq!(conv("Wedding 4pm CET in New York").unwrap(), "10:00 AM EDT");
        assert_eq!(
            conv("Sunday brunch 11am PST in London").unwrap(),
            "7:00 PM BST, Sun 18 Oct"
        );
    }

    #[test]
    fn relative_times() {
        let rel = |line: &str| eval_relative(line, now());
        // now() is 12:00 UTC on Fri 16 Oct.
        assert_eq!(rel("in 90 minutes in Tokyo").unwrap(), "10:30 PM JST");
        assert_eq!(rel("3 hours from now in UTC").unwrap(), "3:00 PM UTC");
        assert_eq!(rel("in 3 days in UTC").unwrap(), "12:00 PM UTC, Mon 19 Oct");
        assert_eq!(rel("in 1 hour 30 min in London").unwrap(), "2:30 PM BST");
        assert_eq!(rel("3pm UTC + 45 min").unwrap(), "3:45 PM UTC");
        assert_eq!(rel("11pm UTC + 2 hours").unwrap(), "1:00 AM UTC (next day)");
        assert_eq!(
            rel("9:30am PST + 2 hours in London").unwrap(),
            "7:30 PM BST"
        );
        assert_eq!(rel("Meeting 3pm UTC - 15 min").unwrap(), "2:45 PM UTC");
        // In local time, plain clock math has no zone name.
        assert!(rel("3pm + 45 min").unwrap().starts_with("3:45 PM"));
        assert!(rel("in 3 hours").unwrap().ends_with('M'));
        // Not a time: left to the math engine.
        assert_eq!(rel("3 + 45 min"), None);
        assert_eq!(rel("$5 + 3 hours"), None);
        assert_eq!(rel("in 3 m"), None);
    }

    #[test]
    fn huge_spans_are_not_times() {
        let rel = |line: &str| eval_relative(line, now());
        assert_eq!(rel("in 99999999999 h"), None);
        assert_eq!(rel("3pm UTC + 99999999999 hours"), None);
        assert_eq!(eval_date("today + 99999999999999 days"), None);
        assert_eq!(eval_date("now + 999999999999 hours"), None);
    }

    #[test]
    fn long_shifts_say_which_day() {
        let rel = |line: &str| eval_relative(line, now());
        assert_eq!(rel("3pm UTC + 3 days").unwrap(), "3:00 PM UTC, Mon 19 Oct");
        assert_eq!(
            rel("9am PST + 2 days in London").unwrap(),
            "5:00 PM BST, Sun 18 Oct"
        );
        assert_eq!(
            rel("in 100 weeks in UTC").unwrap(),
            "12:00 PM UTC, Fri 15 Sep 2028"
        );
    }

    #[test]
    fn gmt_is_a_fixed_offset() {
        assert_eq!(conv("3pm GMT in UTC").unwrap(), "3:00 PM UTC");
    }

    #[test]
    fn current_time_in_zone() {
        assert_eq!(conv("time in Tokyo").unwrap(), "9:00 PM JST");
        assert_eq!(conv("now in India").unwrap(), "5:30 PM IST");
    }

    #[test]
    fn leaves_math_alone() {
        assert_eq!(conv("3 km in miles"), None);
        assert_eq!(conv("255 in hex"), None);
        assert_eq!(conv("15 in IST"), None);
        assert_eq!(conv("$30 in EUR"), None);
    }
}
