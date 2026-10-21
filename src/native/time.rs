//! Civil calendar conversion and RFC 3339 formatting, ported from the `chr` branch.
//!
//! Calendar arithmetic and formatting use only `core` and `alloc`. Reading the
//! clock remains the caller's responsibility; `SystemTime` is a conversion input.

extern crate alloc;
use alloc::string::String;
use core::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

/// A timestamp broken down into UTC civil calendar fields.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CivilTime {
    pub(crate) year: i64,
    pub(crate) month: u8,
    pub(crate) day: u8,
    pub(crate) hour: u8,
    pub(crate) minute: u8,
    pub(crate) second: u8,
    pub(crate) nanosecond: u32,
}

impl CivilTime {
    /// Converts Unix seconds and a positive sub-second nanosecond component.
    fn from_unix(secs: i64, nanosecond: u32) -> Self {
        assert!(
            nanosecond < 1_000_000_000,
            "Nanosecond component out of range"
        );
        let days = secs.div_euclid(86_400);
        let secs_of_day = secs.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: (secs_of_day / 3_600) as u8,
            minute: ((secs_of_day % 3_600) / 60) as u8,
            second: (secs_of_day % 60) as u8,
            nanosecond,
        }
    }

    /// Returns Unix seconds, rounding down to the start of the current second.
    #[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
    pub(crate) fn unix_seconds(&self) -> i64 {
        let days = days_from_civil(self.year, self.month, self.day);
        days * 86_400 + self.hour as i64 * 3_600 + self.minute as i64 * 60 + self.second as i64
    }
}

impl From<SystemTime> for CivilTime {
    fn from(value: SystemTime) -> Self {
        let (secs, nanosecond) = match value.duration_since(UNIX_EPOCH) {
            Ok(dur) => (
                i64::try_from(dur.as_secs()).expect("Timestamp exceeds supported Unix seconds"),
                dur.subsec_nanos(),
            ),
            Err(e) => {
                let dur = e.duration();
                let nanos = dur.subsec_nanos();
                let secs = -i128::from(dur.as_secs()) - i128::from(nanos != 0);
                (
                    i64::try_from(secs).expect("Timestamp precedes supported Unix seconds"),
                    if nanos == 0 { 0 } else { 1_000_000_000 - nanos },
                )
            }
        };
        Self::from_unix(secs, nanosecond)
    }
}

// Howard Hinnant's civil_from_days and days_from_civil algorithms.
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = y + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let m = month as i64;
    let d = day as i64;
    let y = if m <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Formats UTC with Chrono-compatible automatic fractional-second precision.
pub(crate) fn to_rfc3339(ct: CivilTime) -> String {
    let mut out = String::with_capacity(35);
    if (0..=9_999).contains(&ct.year) {
        write!(out, "{:04}", ct.year).expect("Writing a year to String failed");
    } else {
        write!(out, "{:+05}", ct.year).expect("Writing a year to String failed");
    }
    write!(
        out,
        "-{:02}-{:02}T{:02}:{:02}:{:02}",
        ct.month, ct.day, ct.hour, ct.minute, ct.second
    )
    .expect("Writing a timestamp to String failed");

    if ct.nanosecond != 0 {
        if ct.nanosecond % 1_000_000 == 0 {
            write!(out, ".{:03}", ct.nanosecond / 1_000_000)
        } else if ct.nanosecond % 1_000 == 0 {
            write!(out, ".{:06}", ct.nanosecond / 1_000)
        } else {
            write!(out, ".{:09}", ct.nanosecond)
        }
        .expect("Writing fractional seconds to String failed");
    }
    out.push_str("+00:00");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    fn at(secs: u64, nanos: u32) -> CivilTime {
        CivilTime::from(UNIX_EPOCH + Duration::new(secs, nanos))
    }

    #[test]
    fn epoch_and_known_datetime() {
        assert_eq!(to_rfc3339(UNIX_EPOCH.into()), "1970-01-01T00:00:00+00:00");
        assert_eq!(
            to_rfc3339(at(1_704_164_645, 0)),
            "2024-01-02T03:04:05+00:00"
        );
    }

    #[test]
    fn leap_days_and_century_boundaries() {
        for (secs, expected) in [
            (951_782_400, "2000-02-29T00:00:00+00:00"),
            (1_582_977_600, "2020-02-29T12:00:00+00:00"),
            (4_107_542_400, "2100-03-01T00:00:00+00:00"),
        ] {
            assert_eq!(to_rfc3339(at(secs, 0)), expected);
        }
    }

    #[test]
    fn fractional_seconds_scaling() {
        for (nanos, expected) in [
            (123_000_000, "2024-01-02T03:04:05.123+00:00"),
            (123_456_000, "2024-01-02T03:04:05.123456+00:00"),
            (123_456_700, "2024-01-02T03:04:05.123456700+00:00"),
        ] {
            assert_eq!(to_rfc3339(at(1_704_164_645, nanos)), expected);
        }
    }

    #[test]
    fn before_epoch_with_fractional_seconds() {
        for (duration, expected, seconds) in [
            (Duration::from_secs(1), "1969-12-31T23:59:59+00:00", -1),
            (
                Duration::from_nanos(100),
                "1969-12-31T23:59:59.999999900+00:00",
                -1,
            ),
            (
                Duration::new(1, 500_000_000),
                "1969-12-31T23:59:58.500+00:00",
                -2,
            ),
        ] {
            let ct = CivilTime::from(UNIX_EPOCH - duration);
            assert_eq!(to_rfc3339(ct), expected);
            assert_eq!(ct.unix_seconds(), seconds);
        }
    }

    #[test]
    fn unix_seconds_round_trips() {
        for secs in [
            0,
            1,
            -1,
            86_400,
            1_582_977_600,
            1_704_164_645,
            -62_135_596_800,
        ] {
            let ct = CivilTime::from_unix(secs, 999_999_999);
            assert_eq!(ct.unix_seconds(), secs);
        }
    }

    #[test]
    fn extended_year_formatting() {
        for (year, expected) in [
            (-1, "-0001-01-01T00:00:00+00:00"),
            (0, "0000-01-01T00:00:00+00:00"),
            (10_000, "+10000-01-01T00:00:00+00:00"),
        ] {
            let secs = days_from_civil(year, 1, 1) * 86_400;
            assert_eq!(to_rfc3339(CivilTime::from_unix(secs, 0)), expected);
        }
    }
}
