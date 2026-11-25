//! Time arithmetic of the status menu (pure, unit-tested): "Do not disturb" durations, the custom status'
//! "Clear status after" end and the date/time picker of "Custom…".
//!
//! DND submenu entries: 30 minutes, 1 hour, 2 hours, 4 hours, Today, Until turned off, Custom…;
//! "Clear status after": 30 minutes, 1 hour, 2 hours, 12 hours, 1 day (default), 7 days, 14 days, Custom;
//! custom range: at least 30 minutes, at most 30 days. "Today" = until local midnight; the picker is a day list + a
//! 30-minute time list. The labels are English pack sources; they are translated where they are shown.

/// Minutes of each "Clear status after" option; `None` = "Custom" (picker).
pub(crate) const CLEAR_AFTER: [(&str, Option<i64>); 8] = [
    ("30 minutes", Some(30)), ("1 hour", Some(60)), ("2 hours", Some(120)), ("12 hours", Some(720)),
    ("1 day", Some(1440)), ("7 days", Some(7 * 1440)), ("14 days", Some(14 * 1440)), ("Custom", None),
];
/// "1 day" is preselected.
pub(crate) const CLEAR_DEFAULT: usize = 4;
/// Custom range: 30 minutes .. 30 days.
pub(crate) const MIN_MINUTES: i64 = 30;
pub(crate) const MAX_DAYS: i64 = 30;

/// The DND submenu: (id, label, what it means).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Dur { Minutes(i64), Today, NoEnd, Custom }
pub(crate) const DND: [(&str, &str, Dur); 7] = [
    ("30", "30 minutes", Dur::Minutes(30)), ("60", "1 hour", Dur::Minutes(60)), ("120", "2 hours", Dur::Minutes(120)),
    ("240", "4 hours", Dur::Minutes(240)), ("today", "Today", Dur::Today), ("always", "Until turned off", Dur::NoEnd), ("custom", "Custom\u{2026}", Dur::Custom),
];

const DAY_MS: i64 = 86_400_000;

/// Unix ms of the next local midnight after `now_ms` (`offset_secs` = the local zone's offset at that time).
pub(crate) fn end_of_day_ms(now_ms: i64, offset_secs: i32) -> i64 {
    let local = now_ms + offset_secs as i64 * 1000;
    (local.div_euclid(DAY_MS) + 1) * DAY_MS - offset_secs as i64 * 1000
}

/// Seconds from `now_ms` to `until_ms` (at least 1).
pub(crate) fn ttl_secs(now_ms: i64, until_ms: i64) -> i64 { ((until_ms - now_ms) / 1000).max(1) }

/// The picker's choice: day `d` (0 = today) at the `slot`-th half hour, local → Unix ms.
pub(crate) fn picked_ms(now_ms: i64, offset_secs: i32, day: usize, slot: usize) -> i64 {
    let local_midnight = end_of_day_ms(now_ms, offset_secs) - DAY_MS;
    local_midnight + day as i64 * DAY_MS + slot as i64 * 30 * 60_000
}

/// A picked end is allowed when it is 30 minutes .. 30 days ahead.
pub(crate) fn picked_ok(now_ms: i64, until_ms: i64) -> bool {
    until_ms - now_ms >= MIN_MINUTES * 60_000 && until_ms - now_ms <= MAX_DAYS * DAY_MS
}

/// The first half-hour slot that is at least 30 minutes after `now` (used as the picker's start value), as (day, slot).
pub(crate) fn first_slot(now_ms: i64, offset_secs: i32) -> (usize, usize) {
    let target = now_ms + MIN_MINUTES * 60_000;
    let local_midnight = end_of_day_ms(now_ms, offset_secs) - DAY_MS;
    let mins = (target - local_midnight + 30 * 60_000 - 1).div_euclid(30 * 60_000);
    ((mins / 48) as usize, (mins % 48) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(iso: &str) -> i64 { crate::i18n::parse_iso_ms(iso).unwrap() }

    #[test]
    fn today_ends_at_local_midnight() {
        // 10:00 UTC in UTC+1 → next local midnight = 23:00 UTC.
        assert_eq!(end_of_day_ms(ms("2026-11-16T10:00:00.000Z"), 3600), ms("2026-11-16T23:00:00.000Z"));
        // 23:30 UTC in UTC+1 is already 00:30 of the next local day → its end is the day after.
        assert_eq!(end_of_day_ms(ms("2026-11-16T23:30:00.000Z"), 3600), ms("2026-11-17T23:00:00.000Z"));
        assert_eq!(end_of_day_ms(ms("2026-11-16T10:00:00.000Z"), 0), ms("2026-11-17T00:00:00.000Z"));
    }

    #[test]
    fn options_are_the_official_list() {
        let labels: Vec<&str> = CLEAR_AFTER.iter().map(|o| o.0).collect();
        assert_eq!(labels, ["30 minutes", "1 hour", "2 hours", "12 hours", "1 day", "7 days", "14 days", "Custom"]);
        assert_eq!(CLEAR_AFTER[CLEAR_DEFAULT].0, "1 day");
    }

    #[test]
    fn picker_range_and_start() {
        let now = ms("2026-11-16T10:10:00.000Z");
        // UTC+1: local 11:10 → first slot ≥ 11:40 is 12:00 = slot 24 of today.
        assert_eq!(first_slot(now, 3600), (0, 24));
        assert_eq!(picked_ms(now, 3600, 0, 24), ms("2026-11-16T11:00:00.000Z"));
        assert!(picked_ok(now, picked_ms(now, 3600, 0, 24)));
        assert!(!picked_ok(now, now + 10 * 60_000)); // under 30 minutes
        assert!(!picked_ok(now, now + 31 * DAY_MS)); // over 30 days
        assert_eq!(ttl_secs(now, now + 1800_000), 1800);
        // Late evening: the first slot moves to tomorrow.
        assert_eq!(first_slot(ms("2026-11-16T22:50:00.000Z"), 3600), (1, 1)); // local 23:50 → 00:30 tomorrow
    }
}
