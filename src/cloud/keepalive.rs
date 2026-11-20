//! My own "Active" keepalive. Only the decision is here (pure, tested); the job loop sends the request
//! (`Session::set_presence("active", 600)`, the same `POST apheleia /events` the user's own menu choice sends).
//!
//! Behaviour (matches the Webex desktop client):
//! * Active timer: a one-shot timer of `rand(420, 540)` s; it stops the inactive timer. When it fires it checks for input
//!   within `delay - 300` s.
//! * Inactive timer: a 5000 ms timer (stops the active one); when it fires it checks for input within 60 s.
//! * The check: input within the window → set presence Active with ttl 600 and start the active timer; otherwise start
//!   the inactive timer. Nothing is sent when the user is idle: the last Active simply expires on the server (its TTL),
//!   which turns the status to inactive for everyone else.
//! * Self-presence updates: Active starts the active timer if it is not running, Inactive starts the inactive timer if it
//!   is not running, **every other state (DND, meeting, call, presenting, OOO, busy, away …) stops both timers** - a status
//!   the user chose is never overridden.
//!
//! Input idle: on Windows the system-wide last input (`GetLastInputInfo`: any window, also while minimised; a locked screen
//! has no input). Elsewhere the last input the window itself saw. With the screen locked there is no input, so the idle
//! rule already stops the keepalive.
//!
//! Extra guard: the status menu sends "Abwesend" as event type `inactive`. So a manual choice other than "Verfügbar" also
//! stops the keepalive for its TTL (or until the user picks "Verfügbar" again); otherwise the inactive poll would turn a
//! chosen "Abwesend" back into Active.
use std::sync::Mutex;

/// The active timer's random delay (s), inclusive.
pub const ACTIVE_MIN_S: u64 = 420;
pub const ACTIVE_MAX_S: u64 = 540;
/// Seconds subtracted from the active timer's delay to get its input window.
pub const ACTIVE_WINDOW_LESS_S: u64 = 300;
/// The inactive timer's period and input window.
pub const INACTIVE_POLL_MS: i64 = 5000;
pub const INACTIVE_WINDOW_S: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// No timer (unknown self state, or a state other than Active / Inactive).
    Off,
    /// The one-shot active timer: at `due` check for input within `window_s`.
    Active { due: i64, window_s: u64 },
    /// The 5 s inactive poll.
    Inactive { due: i64 },
}

#[derive(Clone, Debug)]
pub struct Keepalive {
    pub mode: Mode,
    /// The last self status seen ("active", "inactive", "DoNotDisturb", …).
    last: String,
    /// A manual status choice other than "Verfügbar" holds the keepalive until then (Unix ms; i64::MAX = no end).
    manual_until: Option<i64>,
}

impl Default for Keepalive { fn default() -> Self { Keepalive { mode: Mode::Off, last: String::new(), manual_until: None } } }

impl Keepalive {
    fn held(&self, now: i64) -> bool { self.manual_until.is_some_and(|u| now < u) }

    fn start_active(&mut self, now: i64, delay_s: u64) {
        self.mode = Mode::Active { due: now + delay_s as i64 * 1000, window_s: delay_s.saturating_sub(ACTIVE_WINDOW_LESS_S) };
    }

    /// My presence state as the services report it (snapshot or live update, or the answer to my own write).
    pub fn on_self_status(&mut self, status: &str, now: i64, delay_s: u64) {
        self.last = status.to_string();
        if self.held(now) { self.mode = Mode::Off; return }
        match (status, self.mode) {
            ("active", Mode::Active { .. }) | ("inactive", Mode::Inactive { .. }) => {} // the timer is already running
            ("active", _) => self.start_active(now, delay_s),
            ("inactive", _) => self.mode = Mode::Inactive { due: now + INACTIVE_POLL_MS },
            _ => self.mode = Mode::Off,
        }
    }

    /// The user picked a status in the menu (`ttl` in s, -1 = no end).
    pub fn on_manual(&mut self, status: &str, ttl: i64, now: i64) {
        if status == "active" { self.manual_until = None; return }
        self.manual_until = Some(if ttl > 0 { now + ttl * 1000 } else { i64::MAX });
        self.mode = Mode::Off;
    }

    /// One tick: `true` = send Active with ttl 600 now. `idle_s` = seconds since the last input; `delay_s` = a fresh random
    /// delay for the next active timer.
    pub fn tick(&mut self, now: i64, idle_s: u64, delay_s: u64) -> bool {
        if let Some(u) = self.manual_until {
            if now < u { self.mode = Mode::Off; return false }
            // The chosen status ran out: carry on from the last known state.
            self.manual_until = None;
            let last = self.last.clone();
            self.on_self_status(&last, now, delay_s);
        }
        match self.mode {
            Mode::Off => false,
            Mode::Active { due, window_s } if now >= due => self.check(now, idle_s, window_s, delay_s),
            Mode::Inactive { due } if now >= due => self.check(now, idle_s, INACTIVE_WINDOW_S, delay_s),
            _ => false,
        }
    }

    /// Checks for input within `window_s` and starts the matching timer.
    fn check(&mut self, now: i64, idle_s: u64, window_s: u64, delay_s: u64) -> bool {
        if idle_s < window_s { self.start_active(now, delay_s); true } else { self.mode = Mode::Inactive { due: now + INACTIVE_POLL_MS }; false }
    }
}

/// A random active-timer delay.
pub fn random_delay() -> u64 { use rand::Rng; rand::thread_rng().gen_range(ACTIVE_MIN_S..=ACTIVE_MAX_S) }

/// My status as the UI learnt it (`Resp::Presence` for me), handed to the job loop.
static SELF_STATUS: Mutex<Option<String>> = Mutex::new(None);
/// Called by the UI for every presence entry that is mine.
pub fn note_self_status(status: &str) { if let Ok(mut s) = SELF_STATUS.lock() { *s = Some(status.to_string()); } }
pub(super) fn take_self_status() -> Option<String> { SELF_STATUS.lock().ok().and_then(|mut s| s.take()) }

/// The last input the window itself saw (Unix ms), for systems without a system-wide idle clock.
static LAST_INPUT_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
/// Called by the UI on any keyboard / pointer input.
pub fn note_input(now_ms: i64) { LAST_INPUT_MS.store(now_ms, std::sync::atomic::Ordering::Relaxed); }

/// Seconds since the last user input (u64::MAX = none seen).
pub fn input_idle_s(now_ms: i64) -> u64 {
    if let Some(ms) = system_idle_ms() { return ms / 1000 }
    match LAST_INPUT_MS.load(std::sync::atomic::Ordering::Relaxed) { 0 => u64::MAX, t => ((now_ms - t).max(0) / 1000) as u64 }
}

/// System-wide input idle (`GetLastInputInfo`).
#[cfg(windows)]
fn system_idle_ms() -> Option<u64> {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: plain Win32 call filling a correctly sized struct.
    if !unsafe { GetLastInputInfo(&mut li) }.as_bool() { return None }
    Some(unsafe { GetTickCount() }.wrapping_sub(li.dwTime) as u64)
}
#[cfg(not(windows))]
fn system_idle_ms() -> Option<u64> { None }

#[cfg(test)]
mod tests {
    use super::*;
    const T0: i64 = 1_800_000_000_000;

    #[test]
    fn active_timer_sends_while_the_user_works() {
        let mut k = Keepalive::default();
        k.on_self_status("active", T0, 480);
        assert_eq!(k.mode, Mode::Active { due: T0 + 480_000, window_s: 180 });
        assert!(!k.tick(T0 + 479_000, 0, 500), "not before the timer");
        assert!(k.tick(T0 + 480_000, 179, 500), "input within delay-300 s → Active");
        assert_eq!(k.mode, Mode::Active { due: T0 + 980_000, window_s: 200 });
        // The answer to my own write (self = active) leaves the running timer alone.
        k.on_self_status("active", T0 + 981_000, 420);
        assert_eq!(k.mode, Mode::Active { due: T0 + 980_000, window_s: 200 });
    }

    #[test]
    fn idle_user_gets_no_keepalive_then_a_quick_one_on_return() {
        let mut k = Keepalive::default();
        k.on_self_status("active", T0, 420);
        assert!(!k.tick(T0 + 420_000, 120, 450), "no input within 120 s: nothing is sent, the TTL lapses");
        assert_eq!(k.mode, Mode::Inactive { due: T0 + 425_000 });
        assert!(!k.tick(T0 + 425_000, 300, 450));
        assert_eq!(k.mode, Mode::Inactive { due: T0 + 430_000 }, "polls every 5 s");
        assert!(k.tick(T0 + 430_000, 59, 450), "input within 60 s → Active at once");
        assert_eq!(k.mode, Mode::Active { due: T0 + 880_000, window_s: 150 });
    }

    #[test]
    fn inactive_self_state_starts_the_poll() {
        let mut k = Keepalive::default();
        k.on_self_status("inactive", T0, 500);
        assert_eq!(k.mode, Mode::Inactive { due: T0 + 5000 });
        assert!(k.tick(T0 + 5000, 3, 500));
    }

    #[test]
    fn other_states_stop_the_timers() {
        for s in ["DoNotDisturb", "meeting", "call", "presenting", "OutOfOffice", "busy", "away", ""] {
            let mut k = Keepalive::default();
            k.on_self_status("active", T0, 420);
            k.on_self_status(s, T0 + 1000, 420);
            assert_eq!(k.mode, Mode::Off, "{s}");
            assert!(!k.tick(T0 + 10_000_000, 0, 420), "{s}: never overridden");
        }
    }

    #[test]
    fn manual_choice_holds_the_keepalive() {
        let mut k = Keepalive::default();
        k.on_self_status("active", T0, 420);
        // "Abwesend" from the menu (sent as event type inactive, no end).
        k.on_manual("inactive", -1, T0 + 1000);
        k.on_self_status("inactive", T0 + 2000, 420);
        assert_eq!(k.mode, Mode::Off);
        assert!(!k.tick(T0 + 100_000_000, 0, 420), "a chosen Abwesend is not turned back into Active");
        // "Verfügbar" again: the next self update restarts the timer.
        k.on_manual("active", 600, T0 + 200_000_000);
        k.on_self_status("active", T0 + 200_000_100, 420);
        assert!(matches!(k.mode, Mode::Active { .. }));
    }

    #[test]
    fn manual_status_with_an_end_resumes_after_it() {
        let mut k = Keepalive::default();
        k.on_self_status("active", T0, 420);
        k.on_manual("DoNotDisturb", 60, T0);
        assert!(!k.tick(T0 + 59_000, 0, 420));
        // The DND ran out; the last self state known is still "active" (the DND update never reached the keepalive).
        assert!(!k.tick(T0 + 60_000, 0, 420), "restarts the timer, sends at its end");
        assert_eq!(k.mode, Mode::Active { due: T0 + 480_000, window_s: 120 });
    }

    #[test]
    fn delay_is_in_the_originals_range() {
        for _ in 0..200 { let d = random_delay(); assert!((ACTIVE_MIN_S..=ACTIVE_MAX_S).contains(&d)); }
    }
}
