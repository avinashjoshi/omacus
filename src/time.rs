//! Dates and time zones. fend-core ships without a clock, so these are handled here:
//! `today`, `tomorrow + 2 weeks`, `now - 90 minutes`, `3pm PST in IST`,
//! `time in Tokyo`, `10:30 London to New York`.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, Duration, Local, Months, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
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
    ("GMT", "Europe/London"),
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
                    format!("{} {}", clock(t.time()), t.format("%Z")),
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

pub fn eval(line: &str) -> Option<String> {
    eval_date(line).or_else(|| eval_zone(line, Utc::now()))
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
    // Allow a label first: "Sunset 6:12pm IST in Sydney".
    let lhs = c[1].trim();
    let t = std::iter::successors(Some(lhs), |s| {
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

    let instant = if t
        .get(1)
        .is_some_and(|m| m.as_str().eq_ignore_ascii_case("now"))
    {
        now
    } else {
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
        source.to_utc(source.at(now).0, time)?
    };

    let (from_date, _) = source.at(instant);
    let (to_date, shown) = target.at(instant);
    Some(match (to_date - from_date).num_days() {
        1 => format!("{shown} (next day)"),
        -1 => format!("{shown} (prev day)"),
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
        "tomorrow" => now + Duration::days(1),
        "yesterday" => now - Duration::days(1),
        _ => now,
    };
    let mut show_time = base == "now";
    if let (Some(sign), Some(n), Some(unit)) = (c.get(2), c.get(3), c.get(4)) {
        let n: i64 = n.as_str().parse().ok()?;
        let n = if sign.as_str() == "-" { -n } else { n };
        let unit = unit.as_str().to_ascii_lowercase();
        t = match unit.as_str() {
            "minute" | "min" => t + Duration::minutes(n),
            "hour" | "hr" => t + Duration::hours(n),
            "day" => t + Duration::days(n),
            "week" => t + Duration::weeks(n),
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
