//! Periodic refreshes.
//!
//! Running / upcoming meetings (`GET locus /loci?startingSoon=true&includePMRs=true`, `meetings()`): a GET at start and
//! then one fixed period per run, drawn between 2150 and 2320 s (about every 36 min).
//!
//! Calendar (`calendarEvents`): not polled. The Mercury events `calendar.meeting.create / update / update.minimal /
//! delete` re-read the calendar (mercury.rs).

/// The range of the loci period (s).
pub const LOCI_MIN_S: u64 = 2150;
pub const LOCI_MAX_S: u64 = 2320;

/// This run's loci period.
pub fn loci_period_s() -> u64 { use rand::Rng; rand::thread_rng().gen_range(LOCI_MIN_S..=LOCI_MAX_S) }

/// The calendar change events that re-read the calendar.
pub fn calendar_event(event: &str) -> bool {
    matches!(event, "calendar.meeting.create" | "calendar.meeting.update" | "calendar.meeting.update.minimal" | "calendar.meeting.delete")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loci_period_in_the_observed_range() {
        for _ in 0..100 { let p = loci_period_s(); assert!((LOCI_MIN_S..=LOCI_MAX_S).contains(&p)); }
        for seen in [2156, 2314, 2213, 2186] { assert!((LOCI_MIN_S..=LOCI_MAX_S).contains(&seen)); }
    }
    #[test]
    fn calendar_events() {
        assert!(calendar_event("calendar.meeting.update.minimal"));
        assert!(!calendar_event("calendar.free_busy"));
        assert!(!calendar_event("locus.sync_delta"));
    }
}
