//! Minimal, `no_std`-friendly date/time helpers.
//!
//! These replace the small slice of functionality previously provided by the
//! `chrono` crate: decomposing a [`SystemTime`] into civil calendar fields and
//! formatting it as an RFC 3339 string. The implementation relies only on
//! integer arithmetic and `core` formatting, so it does not pull in any
//! `std`-only dependencies beyond [`SystemTime`] itself.
//!
//! In addition to [`SystemTime`], [`CivilTime`] can be built from the native
//! platform time representations used by each backend: [`SYSTEMTIME`] and
//! [`FILETIME`] for the Windows/ETW path, and [`timespec`] for the
//! Linux/`user_events` path.

use core::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

/// A `SystemTime` broken down into UTC civil calendar fields.
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
    /// Builds a `CivilTime` from a count of whole seconds since the Unix epoch
    /// (which may be negative) and a positive sub-second nanosecond component in
    /// `[0, 1_000_000_000)`.
    fn from_unix(secs: i64, nanosecond: u32) -> Self {
        let days = secs.div_euclid(86_400);
        let secs_of_day = secs.rem_euclid(86_400);

        let hour = (secs_of_day / 3_600) as u8;
        let minute = ((secs_of_day % 3_600) / 60) as u8;
        let second = (secs_of_day % 60) as u8;

        let (year, month, day) = civil_from_days(days);

        CivilTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
            nanosecond,
        }
    }

    /// Returns the number of whole seconds between the Unix epoch
    /// (1970-01-01T00:00:00 UTC) and this civil time, negative for times before
    /// the epoch. This is the inverse of [`from_unix`](Self::from_unix); any
    /// sub-second [`nanosecond`](Self#structfield.nanosecond) component is
    /// truncated.
    #[allow(dead_code)]
    pub(crate) fn unix_seconds(&self) -> i64 {
        let days = days_from_civil(self.year, self.month, self.day);
        days * 86_400
            + self.hour as i64 * 3_600
            + self.minute as i64 * 60
            + self.second as i64
    }
}

impl From<SystemTime> for CivilTime {
    fn from(value: SystemTime) -> Self {
        // Seconds (and sub-second nanoseconds) relative to the Unix epoch,
        // handling times both before and after the epoch.
        let (secs, nanosecond) = match value.duration_since(UNIX_EPOCH) {
            Ok(dur) => (dur.as_secs() as i64, dur.subsec_nanos()),
            Err(e) => {
                let dur = e.duration();
                let nanos = dur.subsec_nanos();
                if nanos == 0 {
                    (-(dur.as_secs() as i64), 0)
                } else {
                    // Borrow a second so the nanosecond component stays positive.
                    (-(dur.as_secs() as i64) - 1, 1_000_000_000 - nanos)
                }
            }
        };

        Self::from_unix(secs, nanosecond)
    }
}

/// Win32 `SYSTEMTIME`: an already broken-down calendar time (assumed UTC here).
///
/// Layout matches the Win32 `SYSTEMTIME` structure so a value obtained from the
/// OS can be converted directly.
#[cfg(target_os = "windows")]
#[repr(C)]
#[allow(non_snake_case, dead_code)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SYSTEMTIME {
    pub wYear: u16,
    pub wMonth: u16,
    pub wDayOfWeek: u16,
    pub wDay: u16,
    pub wHour: u16,
    pub wMinute: u16,
    pub wSecond: u16,
    pub wMilliseconds: u16,
}

#[cfg(target_os = "windows")]
impl From<SYSTEMTIME> for CivilTime {
    fn from(value: SYSTEMTIME) -> Self {
        CivilTime {
            year: value.wYear as i64,
            month: value.wMonth as u8,
            day: value.wDay as u8,
            hour: value.wHour as u8,
            minute: value.wMinute as u8,
            second: value.wSecond as u8,
            nanosecond: value.wMilliseconds as u32 * 1_000_000,
        }
    }
}

/// Win32 `FILETIME`: the number of 100-nanosecond intervals since
/// 1601-01-01T00:00:00 UTC. Layout matches the Win32 `FILETIME` structure.
#[cfg(target_os = "windows")]
#[repr(C)]
#[allow(non_snake_case, dead_code)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FILETIME {
    pub dwLowDateTime: u32,
    pub dwHighDateTime: u32,
}

/// Whole seconds between the `FILETIME` epoch (1601-01-01) and the Unix epoch
/// (1970-01-01).
#[cfg(target_os = "windows")]
const FILETIME_UNIX_EPOCH_DELTA_SECS: i64 = 11_644_473_600;

#[cfg(target_os = "windows")]
impl From<FILETIME> for CivilTime {
    fn from(value: FILETIME) -> Self {
        let ticks = ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64;
        let secs_since_1601 = (ticks / 10_000_000) as i64;
        let sub_second_100ns = (ticks % 10_000_000) as u32;
        Self::from_unix(
            secs_since_1601 - FILETIME_UNIX_EPOCH_DELTA_SECS,
            sub_second_100ns * 100,
        )
    }
}

/// The C `long` type on Linux, which is the basis for both `time_t` and
/// `c_long` in the classic `struct timespec` layout: 64 bits wide on LP64
/// targets and 32 bits wide on 32-bit (ILP32) targets.
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
#[allow(non_camel_case_types)]
type c_long = i64;
#[cfg(all(target_os = "linux", not(target_pointer_width = "64")))]
#[allow(non_camel_case_types)]
type c_long = i32;

/// POSIX `timespec`: seconds and nanoseconds since the Unix epoch. This mirrors
/// the C `struct timespec` used on the Linux/`user_events` path. The field width
/// follows the target: 64-bit on LP64 platforms and 32-bit on 32-bit platforms.
#[cfg(target_os = "linux")]
#[repr(C)]
#[allow(non_camel_case_types, dead_code)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct timespec {
    pub tv_sec: c_long,
    pub tv_nsec: c_long,
}

#[cfg(target_os = "linux")]
impl From<timespec> for CivilTime {
    fn from(value: timespec) -> Self {
        // Widen to i64 so the arithmetic is identical regardless of the target's
        // native `long` width, then normalise so the nanosecond component lands
        // in [0, 1e9), carrying any overflow (or borrow) into the seconds.
        let tv_sec = value.tv_sec as i64;
        let tv_nsec = value.tv_nsec as i64;
        let secs = tv_sec + tv_nsec.div_euclid(1_000_000_000);
        let nanosecond = tv_nsec.rem_euclid(1_000_000_000) as u32;
        Self::from_unix(secs, nanosecond)
    }
}

/// Converts a count of days since the Unix epoch (1970-01-01) into a
/// `(year, month, day)` civil date.
///
/// Based on Howard Hinnant's `civil_from_days` algorithm, valid for a very wide
/// range of dates.
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8; // [1, 12]
    let year = y + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// Converts a `(year, month, day)` civil date into a count of days since the
/// Unix epoch (1970-01-01). Inverse of [`civil_from_days`], also from Howard
/// Hinnant's algorithms.
#[allow(dead_code)]
fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let m = month as i64;
    let d = day as i64;
    let y = if m <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Formats a [`SystemTime`] as an RFC 3339 / ISO 8601 string in UTC, e.g.
/// `2024-01-02T03:04:05.123456789+00:00`.
///
/// The fractional-seconds component uses the same auto-scaling rule as
/// `chrono`'s `to_rfc3339`: it is omitted when zero, otherwise rendered with 3,
/// 6, or 9 digits depending on the available precision.
pub(crate) fn to_rfc3339(ct: CivilTime) -> String {

    let mut out = String::with_capacity(35);
    let _ = write!(
        out,
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        ct.year, ct.month, ct.day, ct.hour, ct.minute, ct.second
    );

    if ct.nanosecond != 0 {
        if ct.nanosecond % 1_000_000 == 0 {
            let _ = write!(out, ".{:03}", ct.nanosecond / 1_000_000);
        } else if ct.nanosecond % 1_000 == 0 {
            let _ = write!(out, ".{:06}", ct.nanosecond / 1_000);
        } else {
            let _ = write!(out, ".{:09}", ct.nanosecond);
        }
    }

    out.push_str("+00:00");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64, nanos: u32) -> CivilTime {
        CivilTime::from(UNIX_EPOCH + Duration::new(secs, nanos))
    }

    #[test]
    fn epoch() {
        let ct = CivilTime::from(UNIX_EPOCH);
        assert_eq!(
            (ct.year, ct.month, ct.day, ct.hour, ct.minute, ct.second),
            (1970, 1, 1, 0, 0, 0)
        );
        assert_eq!(to_rfc3339(CivilTime::from(UNIX_EPOCH)), "1970-01-01T00:00:00+00:00");
    }

    #[test]
    fn known_datetime() {
        // 2024-01-02T03:04:05Z == 1704164645 seconds since the epoch.
        let t = at(1_704_164_645, 0);
        let ct = CivilTime::from(t);
        assert_eq!(
            (ct.year, ct.month, ct.day, ct.hour, ct.minute, ct.second),
            (2024, 1, 2, 3, 4, 5)
        );
        assert_eq!(to_rfc3339(ct), "2024-01-02T03:04:05+00:00");
    }

    #[test]
    fn leap_day() {
        // 2020-02-29T12:00:00Z == 1582977600 seconds since the epoch.
        let t = at(1_582_977_600, 0);
        let ct = CivilTime::from(t);
        assert_eq!((ct.year, ct.month, ct.day), (2020, 2, 29));
    }

    #[test]
    fn fractional_seconds_scaling() {
        assert_eq!(
            to_rfc3339(at(1_704_164_645, 123_000_000)),
            "2024-01-02T03:04:05.123+00:00"
        );
        assert_eq!(
            to_rfc3339(at(1_704_164_645, 123_456_000)),
            "2024-01-02T03:04:05.123456+00:00"
        );
        assert_eq!(
            to_rfc3339(at(1_704_164_645, 123_456_700)),
            "2024-01-02T03:04:05.123456700+00:00"
        );
    }

    #[test]
    fn before_epoch() {
        // 1969-12-31T23:59:59Z == -1 second relative to the epoch.
        let t = UNIX_EPOCH - Duration::new(1, 0);
        let ct = CivilTime::from(t);
        assert_eq!(
            (ct.year, ct.month, ct.day, ct.hour, ct.minute, ct.second),
            (1969, 12, 31, 23, 59, 59)
        );
    }

    #[test]
    fn unix_seconds_known_values() {
        assert_eq!(CivilTime::from(UNIX_EPOCH).unix_seconds(), 0);
        assert_eq!(at(1_704_164_645, 0).unix_seconds(), 1_704_164_645);
        // Sub-second components are truncated, not rounded.
        assert_eq!(at(1_704_164_645, 999_999_999).unix_seconds(), 1_704_164_645);
    }

    #[test]
    fn unix_seconds_before_epoch() {
        let ct = CivilTime::from(UNIX_EPOCH - Duration::new(1, 0));
        assert_eq!(ct.unix_seconds(), -1);
    }

    #[test]
    fn unix_seconds_round_trips() {
        for secs in [0i64, 1, -1, 86_400, 1_582_977_600, 1_704_164_645, -62_135_596_800] {
            let ct = CivilTime::from_unix(secs, 0);
            assert_eq!(ct.unix_seconds(), secs);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn from_systemtime() {
        let st = SYSTEMTIME {
            wYear: 2024,
            wMonth: 1,
            wDayOfWeek: 2,
            wDay: 2,
            wHour: 3,
            wMinute: 4,
            wSecond: 5,
            wMilliseconds: 123,
        };
        let ct = CivilTime::from(st);
        assert_eq!(
            (ct.year, ct.month, ct.day, ct.hour, ct.minute, ct.second),
            (2024, 1, 2, 3, 4, 5)
        );
        assert_eq!(ct.nanosecond, 123_000_000);
    }

    #[cfg(target_os = "windows")]
    fn filetime_from_ticks(ticks: u64) -> FILETIME {
        FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn from_filetime_epoch() {
        // The Unix epoch is 11_644_473_600 seconds after the FILETIME epoch.
        let ticks = 11_644_473_600u64 * 10_000_000;
        let ct = CivilTime::from(filetime_from_ticks(ticks));
        assert_eq!(
            (ct.year, ct.month, ct.day, ct.hour, ct.minute, ct.second),
            (1970, 1, 1, 0, 0, 0)
        );
        assert_eq!(ct.nanosecond, 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn from_filetime_known_datetime() {
        // 2024-01-02T03:04:05.123456700Z.
        let secs_since_1601 = 1_704_164_645u64 + 11_644_473_600;
        let ticks = secs_since_1601 * 10_000_000 + 1_234_567;
        let ct = CivilTime::from(filetime_from_ticks(ticks));
        assert_eq!(
            (ct.year, ct.month, ct.day, ct.hour, ct.minute, ct.second),
            (2024, 1, 2, 3, 4, 5)
        );
        assert_eq!(ct.nanosecond, 123_456_700);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn from_timespec() {
        let ts = timespec {
            tv_sec: 1_704_164_645,
            tv_nsec: 123_456_789,
        };
        let ct = CivilTime::from(ts);
        assert_eq!(
            (ct.year, ct.month, ct.day, ct.hour, ct.minute, ct.second),
            (2024, 1, 2, 3, 4, 5)
        );
        assert_eq!(ct.nanosecond, 123_456_789);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn from_timespec_normalises_overflow() {
        // 1.5 seconds of nanoseconds should carry into the seconds component.
        let ts = timespec {
            tv_sec: 1_704_164_644,
            tv_nsec: 1_500_000_000,
        };
        let ct = CivilTime::from(ts);
        assert_eq!(ct.second, 5);
        assert_eq!(ct.nanosecond, 500_000_000);
    }
}
