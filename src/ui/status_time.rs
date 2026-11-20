//! Time arithmetic of the status menu (pure, unit-tested): "Bitte nicht stören" durations, the custom status'
//! "Status löschen nach" end and the date/time picker of "Anpassen..." / "Benutzerdefiniert".
//!
//! DND submenu entries: 30 Minuten, 1 Stunde, 2 Stunden, 4 Stunden, Heute, Bis zur Deaktivierung, Anpassen...;
//! "Status löschen nach": 30 Minuten, 1 Stunde, 2 Stunden, 12 Stunden, 1 Tag (default), 7 Tage, 14 Tage, Benutzerdefiniert;
//! custom range: at least 30 minutes, at most 30 days. "Heute" = until local midnight; the picker is a day list + a
//! 30-minute time list.

/// Minutes of each "Status löschen nach" option; `None` = "Benutzerdefiniert" (picker). (The first label has a no-break
/// space, as the pack's "30\u{a0}Minuten".)
pub(crate) const CLEAR_AFTER: [(&str, Option<i64>); 8] = [
    ("30\u{a0}Minuten", Some(30)), ("1 Stunde", Some(60)), ("2 Stunden", Some(120)), ("12 Stunden", Some(720)),
    ("1 Tag", Some(1440)), ("7 Tage", Some(7 * 1440)), ("14 Tage", Some(14 * 1440)), ("Benutzerdefiniert", None),
];
/// "1 Tag" is preselected.
pub(crate) const CLEAR_DEFAULT: usize = 4;
/// Custom range: 30 minutes .. 30 days.
pub(crate) const MIN_MINUTES: i64 = 30;
pub(crate) const MAX_DAYS: i64 = 30;

/// The DND submenu: (id, label, what it means).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Dur { Minutes(i64), Today, NoEnd, Custom }
pub(crate) const DND: [(&str, &str, Dur); 7] = [
    ("30", "30 Minuten", Dur::Minutes(30)), ("60", "1 Stunde", Dur::Minutes(60)), ("120", "2 Stunden", Dur::Minutes(120)),
    ("240", "4 Stunden", Dur::Minutes(240)), ("today", "Heute", Dur::Today), ("always", "Bis zur Deaktivierung", Dur::NoEnd), ("custom", "Anpassen...", Dur::Custom),
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
        assert_eq!(labels, ["30\u{a0}Minuten", "1 Stunde", "2 Stunden", "12 Stunden", "1 Tag", "7 Tage", "14 Tage", "Benutzerdefiniert"]);
        assert_eq!(CLEAR_AFTER[CLEAR_DEFAULT].0, "1 Tag");
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
