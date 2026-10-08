//! Kernel time.
//!
//! Metamind has no clock abstraction and no timezone: a `Timestamp` is a
//! UTC instant with nanosecond precision, rendered as RFC3339 with a fixed
//! nine-digit fractional part so that lexicographic and chronological order
//! agree. System time and valid time are both `Timestamp`s.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A UTC instant, ordered by `(seconds, nanos)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    /// Seconds since the Unix epoch.
    pub seconds: u64,
    /// Nanoseconds within the second (`0..1_000_000_000`).
    pub nanos: u32,
}

/// A timestamp that could not be constructed or parsed.
#[derive(Debug, thiserror::Error)]
pub enum TimestampError {
    /// The instant is outside the representable range.
    #[error("timestamp out of range: {0}")]
    Range(String),
    /// The input is not a well-formed RFC3339 UTC timestamp.
    #[error("invalid RFC3339 timestamp: {0}")]
    Parse(String),
}

impl Timestamp {
    /// The Unix epoch.
    pub const EPOCH: Timestamp = Timestamp {
        seconds: 0,
        nanos: 0,
    };

    /// Build from a `SystemTime`. Instants before the epoch clamp to [`Self::EPOCH`].
    pub fn from_system_time(t: SystemTime) -> Self {
        match t.duration_since(UNIX_EPOCH) {
            Ok(d) => Timestamp {
                seconds: d.as_secs(),
                nanos: d.subsec_nanos(),
            },
            Err(_) => Timestamp::EPOCH,
        }
    }

    /// Build from seconds and nanoseconds.
    pub fn new(seconds: u64, nanos: u32) -> Result<Self, TimestampError> {
        if nanos >= 1_000_000_000 {
            return Err(TimestampError::Range(format!(
                "nanos must be < 1e9, got {nanos}"
            )));
        }
        Ok(Timestamp { seconds, nanos })
    }

    /// The current instant.
    pub fn now() -> Self {
        Timestamp::from_system_time(SystemTime::now())
    }

    /// Build from whole epoch seconds.
    pub fn from_epoch_seconds(seconds: u64) -> Self {
        Timestamp { seconds, nanos: 0 }
    }

    /// Nanoseconds since the epoch, for interval arithmetic.
    pub fn as_nanos(&self) -> u128 {
        u128::from(self.seconds) * 1_000_000_000 + u128::from(self.nanos)
    }

    /// Render as `YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ` (always UTC, always 9 digits).
    ///
    /// The phase-1 plan fixes this signature as `to_rfc3339(&self)`; on a `Copy`
    /// type clippy would rather take `self` by value, so the convention lint is
    /// allowed here to keep the published interface the plan specifies.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_rfc3339(&self) -> String {
        let (year, month, day) = civil_from_days((self.seconds / 86_400) as i64);
        let rem = self.seconds % 86_400;
        let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
        format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:09}Z",
            self.nanos
        )
    }

    /// Parse `YYYY-MM-DDTHH:MM:SS[.fraction][Z|+00:00]`.
    ///
    /// Only UTC is accepted: Metamind never stores a local time.
    pub fn from_rfc3339(s: &str) -> Result<Self, TimestampError> {
        let s = s.trim();
        let body = s
            .strip_suffix('Z')
            .or_else(|| s.strip_suffix("+00:00"))
            .or_else(|| s.strip_suffix("z"))
            .ok_or_else(|| TimestampError::Parse(format!("missing UTC offset in {s:?}")))?;

        let (date, time) = body
            .split_once(['T', 't'])
            .ok_or_else(|| TimestampError::Parse(format!("missing 'T' separator in {s:?}")))?;

        let mut d = date.split('-');
        let year: i64 = parse_part(d.next(), "year", s)?;
        let month: u32 = parse_part(d.next(), "month", s)?;
        let day: u32 = parse_part(d.next(), "day", s)?;
        if d.next().is_some() {
            return Err(TimestampError::Parse(format!(
                "trailing date fields in {s:?}"
            )));
        }
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return Err(TimestampError::Parse(format!("date out of range in {s:?}")));
        }

        let (hms, fraction) = match time.split_once('.') {
            Some((hms, frac)) => (hms, Some(frac)),
            None => (time, None),
        };
        let mut t = hms.split(':');
        let hour: u64 = parse_part(t.next(), "hour", s)?;
        let minute: u64 = parse_part(t.next(), "minute", s)?;
        let second: u64 = parse_part(t.next(), "second", s)?;
        if t.next().is_some() {
            return Err(TimestampError::Parse(format!(
                "trailing time fields in {s:?}"
            )));
        }
        if hour > 23 || minute > 59 || second > 59 {
            return Err(TimestampError::Parse(format!("time out of range in {s:?}")));
        }

        let nanos = match fraction {
            None => 0,
            Some(f) if f.len() > 9 => {
                return Err(TimestampError::Parse(format!(
                    "fractional seconds must have at most 9 digits in {s:?}"
                )))
            }
            Some(f) => {
                let digits: String = f.chars().take_while(char::is_ascii_digit).collect();
                if digits.is_empty() || digits.len() != f.len() {
                    return Err(TimestampError::Parse(format!(
                        "invalid fractional seconds in {s:?}"
                    )));
                }
                let mut padded = digits.clone();
                while padded.len() < 9 {
                    padded.push('0');
                }
                padded
                    .parse::<u32>()
                    .map_err(|_| TimestampError::Parse(format!("invalid fraction in {s:?}")))?
            }
        };

        let days = days_from_civil(year, month, day);
        let seconds = days
            .checked_mul(86_400)
            .and_then(|d| d.checked_add((hour * 3600 + minute * 60 + second) as i64))
            .ok_or_else(|| {
                TimestampError::Range(format!("{s:?} is outside the supported range"))
            })?;
        if seconds < 0 {
            return Err(TimestampError::Range(format!(
                "{s:?} is before the Unix epoch"
            )));
        }
        Timestamp::new(seconds as u64, nanos)
    }
}

fn parse_part<T: std::str::FromStr>(
    part: Option<&str>,
    name: &str,
    whole: &str,
) -> Result<T, TimestampError>
where
    T::Err: std::fmt::Display,
{
    part.ok_or_else(|| TimestampError::Parse(format!("missing {name} in {whole:?}")))?
        .parse::<T>()
        .map_err(|e| TimestampError::Parse(format!("invalid {name} in {whole:?}: {e}")))
}

/// Days since 1970-01-01 to `(year, month, day)`. Howard Hinnant's civil-from-days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `(year, month, day)` to days since 1970-01-01. Inverse of [`civil_from_days`].
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) as u64 + 2) / 5 + u64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

impl std::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_rfc3339())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Timestamp::from_rfc3339(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_renders_and_round_trips() {
        assert_eq!(
            Timestamp::EPOCH.to_rfc3339(),
            "1970-01-01T00:00:00.000000000Z"
        );
        assert_eq!(
            Timestamp::from_rfc3339("1970-01-01T00:00:00.000000000Z").unwrap(),
            Timestamp::EPOCH
        );
    }

    #[test]
    fn known_civil_vectors() {
        // 2001-09-09T01:46:40Z is exactly 1_000_000_000 epoch seconds.
        let t = Timestamp::from_epoch_seconds(1_000_000_000);
        assert_eq!(t.to_rfc3339(), "2001-09-09T01:46:40.000000000Z");
        assert_eq!(Timestamp::from_rfc3339("2001-09-09T01:46:40Z").unwrap(), t);
    }

    #[test]
    fn leap_day_is_handled() {
        let t = Timestamp::from_rfc3339("2024-02-29T12:34:56.123456789Z").unwrap();
        assert_eq!(t.to_rfc3339(), "2024-02-29T12:34:56.123456789Z");
    }

    #[test]
    fn ordering_matches_chronology() {
        let a = Timestamp::from_rfc3339("2024-01-01T00:00:00.000000001Z").unwrap();
        let b = Timestamp::from_rfc3339("2024-01-01T00:00:00.000000002Z").unwrap();
        assert!(a < b);
        assert!(b.to_rfc3339() > a.to_rfc3339());
    }

    #[test]
    fn rejects_non_utc_and_malformed() {
        for bad in [
            "2024-01-01T00:00:00+02:00",
            "2024-01-01 00:00:00Z",
            "not-a-timestamp",
            "2024-13-01T00:00:00Z",
            "2024-01-01T25:00:00Z",
            "1960-01-01T00:00:00Z",
        ] {
            assert!(Timestamp::from_rfc3339(bad).is_err(), "should reject {bad}");
        }
    }
}
