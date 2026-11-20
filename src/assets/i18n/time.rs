//! Civil time helpers without a time crate: days, parts, ISO parsing, the local UTC offset.
use super::*;

// Civil time helpers (no chrono/time crate).

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of [`days_from_civil`]: (year, month, day).
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    /// 0 = Monday.
    pub weekday: i64,
}

/// Break a unix timestamp (seconds, already shifted into the wanted zone) into calendar parts.
pub fn parts(secs: i64) -> Parts {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (year, month, day) = civil_from_days(days);
    Parts { year, month, day, hour: rem / 3600, minute: rem % 3600 / 60, second: rem % 60, weekday: (days + 3).rem_euclid(7) }
}

/// Parse an ISO-8601 timestamp (`2026-11-13T15:48:23.123Z`, `+01:00` offsets, space separator, date only)
/// into unix milliseconds (UTC). Returns None when malformed.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    let num = |a: usize, n: usize| -> Option<i64> { s.get(a..a + n)?.parse::<i64>().ok() };
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let (mut h, mut mi, mut sec, mut ms) = (0, 0, 0, 0);
    let mut offset = 0i64;
    if b.len() > 10 {
        if b[10] != b'T' && b[10] != b' ' {
            return None;
        }
        h = num(11, 2)?;
        if b.get(13) != Some(&b':') {
            return None;
        }
        mi = num(14, 2)?;
        let mut p = 16;
        if b.get(p) == Some(&b':') {
            sec = num(p + 1, 2)?;
            p += 3;
            if b.get(p) == Some(&b'.') {
                p += 1;
                let st = p;
                while p < b.len() && b[p].is_ascii_digit() {
                    p += 1;
                }
                let frac = &s[st..p];
                ms = format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()?;
            }
        }
        match b.get(p) {
            None | Some(b'Z') | Some(b'z') => {}
            Some(&c) if c == b'+' || c == b'-' => {
                let oh = num(p + 1, 2)?;
                let om = if b.get(p + 3) == Some(&b':') { num(p + 4, 2)? } else { num(p + 3, 2).unwrap_or(0) };
                offset = (oh * 3600 + om * 60) * if c == b'-' { -1 } else { 1 };
            }
            _ => return None,
        }
    }
    let secs = days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + sec - offset;
    Some(secs * 1000 + ms)
}

/// Offset of the local time zone from UTC in seconds at the given instant (DST aware).
pub fn local_offset_secs(utc_secs: i64) -> i32 {
    static FORCED: OnceLock<Option<i32>> = OnceLock::new();
    if let Some(m) = *FORCED.get_or_init(|| std::env::var("WEBOX_TZ_OFFSET_MIN").ok().and_then(|v| v.trim().parse::<i32>().ok()).map(|m| m * 60)) {
        return m;
    }
    os_local_offset(utc_secs)
}

#[cfg(windows)]
pub(super) fn os_local_offset(utc_secs: i64) -> i32 {
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;
    let p = parts(utc_secs);
    if !(1601..=30827).contains(&p.year) {
        return 0;
    }
    let st = SYSTEMTIME {
        wYear: p.year as u16,
        wMonth: p.month as u16,
        wDayOfWeek: 0,
        wDay: p.day as u16,
        wHour: p.hour as u16,
        wMinute: p.minute as u16,
        wSecond: p.second as u16,
        wMilliseconds: 0,
    };
    let mut out = SYSTEMTIME::default();
    // SAFETY: valid pointers to stack SYSTEMTIMEs; None = the current time zone.
    if unsafe { SystemTimeToTzSpecificLocalTime(None, &st, &mut out) }.is_err() {
        return 0;
    }
    let local = days_from_civil(out.wYear as i64, out.wMonth as i64, out.wDay as i64) * 86400
        + out.wHour as i64 * 3600
        + out.wMinute as i64 * 60
        + out.wSecond as i64;
    (local - utc_secs) as i32
}

#[cfg(all(unix, not(windows)))]
pub(super) fn os_local_offset(utc_secs: i64) -> i32 {
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: std::os::raw::c_long,
        zone: *const std::os::raw::c_char,
    }
    extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: libc call with valid pointers; struct layout matches glibc/musl/macOS `struct tm`.
    let r = unsafe { localtime_r(&utc_secs, &mut tm) };
    if r.is_null() {
        0
    } else {
        tm.gmtoff as i32
    }
}

#[cfg(not(any(windows, unix)))]
pub(super) fn os_local_offset(_utc_secs: i64) -> i32 {
    0
}
