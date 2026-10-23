//! Civil calendar conversion and RFC 3339 formatting, ported from the `chr` branch.
//!
//! Windows timestamps wrap `FILETIME`; Unix timestamps wrap the platform's
//! `timespec`. Clock reads, calendar arithmetic, and formatting do not use the
//! Rust standard library.

use crate::values::FormatBuffer;
use core::fmt::Write;
#[cfg(any(windows, unix))]
use core::mem::MaybeUninit;

#[cfg(windows)]
const FILETIME_UNIX_EPOCH_SECONDS: u64 = 11_644_473_600;
#[cfg(windows)]
const FILETIME_UNIX_EPOCH_TICKS: u64 = FILETIME_UNIX_EPOCH_SECONDS * 10_000_000;

/// A native UTC wall-clock timestamp.
///
/// Other targets use an epoch placeholder for the no-op exporter, which never
/// serializes timestamps or reads a clock.
#[derive(Copy, Clone)]
pub(crate) struct Timestamp {
    #[cfg(windows)]
    value: windows_sys::Win32::Foundation::FILETIME,
    #[cfg(unix)]
    value: libc::timespec,
}

impl Timestamp {
    pub(crate) const UNIX_EPOCH: Self = Self {
        #[cfg(windows)]
        value: windows_sys::Win32::Foundation::FILETIME {
            dwLowDateTime: FILETIME_UNIX_EPOCH_TICKS as u32,
            dwHighDateTime: (FILETIME_UNIX_EPOCH_TICKS >> 32) as u32,
        },
        #[cfg(unix)]
        // SAFETY: Zero is valid for the integer fields and any ABI padding.
        value: unsafe { core::mem::zeroed() },
    };

    pub(crate) fn now() -> Self {
        #[cfg(windows)]
        {
            let mut value = MaybeUninit::uninit();
            // SAFETY: The API initializes the FILETIME pointed to by its argument.
            unsafe {
                windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime(
                    value.as_mut_ptr(),
                );
                Self {
                    value: value.assume_init(),
                }
            }
        }
        #[cfg(unix)]
        {
            let mut value = MaybeUninit::uninit();
            // SAFETY: The pointer is writable and has the platform's timespec layout.
            let result = unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, value.as_mut_ptr()) };
            assert_eq!(result, 0, "clock_gettime(CLOCK_REALTIME) failed");
            Self {
                // SAFETY: A successful clock_gettime call initialized the timespec.
                value: unsafe { value.assume_init() },
            }
        }
        #[cfg(not(any(windows, unix)))]
        {
            Self::UNIX_EPOCH
        }
    }

    fn unix_parts(self) -> (i64, u32) {
        #[cfg(windows)]
        {
            let ticks =
                (u64::from(self.value.dwHighDateTime) << 32) | u64::from(self.value.dwLowDateTime);
            let seconds = (ticks / 10_000_000) as i64 - FILETIME_UNIX_EPOCH_SECONDS as i64;
            let nanoseconds = (ticks % 10_000_000) as u32 * 100;
            (seconds, nanoseconds)
        }
        #[cfg(unix)]
        {
            // These casts also support ABIs with 32-bit time_t or c_long.
            #[allow(clippy::unnecessary_cast)]
            let seconds = self.value.tv_sec as i64;
            #[allow(clippy::unnecessary_cast)]
            let nanoseconds = self.value.tv_nsec as i64;
            let seconds = seconds
                .checked_add(nanoseconds.div_euclid(1_000_000_000))
                .expect("Timestamp exceeds supported Unix seconds");
            (seconds, nanoseconds.rem_euclid(1_000_000_000) as u32)
        }
        #[cfg(not(any(windows, unix)))]
        {
            (0, 0)
        }
    }

    pub(crate) fn unix_nanoseconds(self) -> i128 {
        let (seconds, nanoseconds) = self.unix_parts();
        i128::from(seconds) * 1_000_000_000 + i128::from(nanoseconds)
    }
}

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

impl From<Timestamp> for CivilTime {
    fn from(value: Timestamp) -> Self {
        let (secs, nanosecond) = value.unix_parts();
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

// Fits every four-digit year with nanoseconds; extreme years spill to the heap.
const RFC3339_CAPACITY: usize = 40;

/// Formats UTC with Chrono-compatible automatic fractional-second precision.
///
/// Formats into inline storage so per-event timestamps do not allocate.
pub(crate) fn to_rfc3339(ct: CivilTime) -> FormatBuffer<RFC3339_CAPACITY> {
    let mut out = FormatBuffer::new();
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
    out.write_str("+00:00")
        .expect("Writing a timestamp to String failed");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64, nanos: u32) -> CivilTime {
        CivilTime::from_unix(secs, nanos)
    }

    #[test]
    fn epoch_and_known_datetime() {
        assert_eq!(
            to_rfc3339(Timestamp::UNIX_EPOCH.into()).as_str(),
            "1970-01-01T00:00:00+00:00"
        );
        assert_eq!(
            to_rfc3339(at(1_704_164_645, 0)).as_str(),
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
            assert_eq!(to_rfc3339(at(secs, 0)).as_str(), expected);
        }
    }

    #[test]
    fn fractional_seconds_scaling() {
        for (nanos, expected) in [
            (123_000_000, "2024-01-02T03:04:05.123+00:00"),
            (123_456_000, "2024-01-02T03:04:05.123456+00:00"),
            (123_456_700, "2024-01-02T03:04:05.123456700+00:00"),
        ] {
            assert_eq!(to_rfc3339(at(1_704_164_645, nanos)).as_str(), expected);
        }
    }

    #[test]
    fn before_epoch_with_fractional_seconds() {
        for (seconds, nanoseconds, expected) in [
            (-1, 0, "1969-12-31T23:59:59+00:00"),
            (-1, 999_999_900, "1969-12-31T23:59:59.999999900+00:00"),
            (-2, 500_000_000, "1969-12-31T23:59:58.500+00:00"),
        ] {
            let ct = CivilTime::from_unix(seconds, nanoseconds);
            assert_eq!(to_rfc3339(ct).as_str(), expected);
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
            assert_eq!(to_rfc3339(CivilTime::from_unix(secs, 0)).as_str(), expected);
        }
    }

    #[cfg(windows)]
    fn filetime(ticks: u64) -> Timestamp {
        Timestamp {
            value: windows_sys::Win32::Foundation::FILETIME {
                dwLowDateTime: ticks as u32,
                dwHighDateTime: (ticks >> 32) as u32,
            },
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_filetime_epochs_and_subsecond_precision() {
        for (ticks, expected, nanoseconds) in [
            (0, "1601-01-01T00:00:00+00:00", -11_644_473_600_000_000_000),
            (FILETIME_UNIX_EPOCH_TICKS, "1970-01-01T00:00:00+00:00", 0),
            (
                FILETIME_UNIX_EPOCH_TICKS - 1,
                "1969-12-31T23:59:59.999999900+00:00",
                -100,
            ),
            (
                FILETIME_UNIX_EPOCH_TICKS + 1_704_164_645 * 10_000_000 + 1_234_567,
                "2024-01-02T03:04:05.123456700+00:00",
                1_704_164_645_123_456_700,
            ),
        ] {
            let timestamp = filetime(ticks);
            assert_eq!(to_rfc3339(timestamp.into()).as_str(), expected);
            assert_eq!(timestamp.unix_nanoseconds(), nanoseconds);
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_timespec_normalizes_subseconds() {
        for (seconds, nanoseconds, expected) in [
            (
                1_704_164_645,
                123_456_789,
                "2024-01-02T03:04:05.123456789+00:00",
            ),
            (
                1_704_164_644,
                1_500_000_000,
                "2024-01-02T03:04:05.500+00:00",
            ),
            (0, -1, "1969-12-31T23:59:59.999999999+00:00"),
        ] {
            let mut timestamp = Timestamp::UNIX_EPOCH;
            timestamp.value.tv_sec = seconds;
            timestamp.value.tv_nsec = nanoseconds;
            assert_eq!(to_rfc3339(timestamp.into()).as_str(), expected);
            assert_eq!(
                timestamp.unix_nanoseconds(),
                i128::from(seconds) * 1_000_000_000 + i128::from(nanoseconds),
            );
        }
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn native_clock_returns_a_valid_current_timestamp() {
        let timestamp = Timestamp::now();
        let (seconds, nanoseconds) = timestamp.unix_parts();
        assert!(
            seconds >= 1_704_067_200,
            "Native clock returned a date before 2024"
        );
        assert!(nanoseconds < 1_000_000_000);
        let civil = CivilTime::from(timestamp);
        assert_eq!(civil.unix_seconds(), seconds);
        assert_eq!(civil.nanosecond, nanoseconds);
    }
}
