//! Typing indicators in the window (protocol: cloud/typing.rs).
//!
//! There is no "… schreibt" text: the typing person's avatar in the "Gesehen von" row at the end of the chat gets a round
//! overlay (`avatar-overlay`, radius height / 2) with three animated dots inside (width = avatar width - 4) while they type;
//! a typer who has not read the newest message joins that row. The dots pulse in a 12 x 80 ms brightness wave; the window
//! draws one frame per step while someone types and none otherwise.
//!
//! Sending (see cloud/typing.rs): every change of the composer text with text in it sends at most one start per 5 s; an
//! emptied composer resets the timer; nothing while editing a message.
use crate::*;
use crate::cloud::typing::{SEND_EVERY_MS, TYPER_EXPIRY_MS};
use std::cell::RefCell;

#[derive(Default)]
struct State {
    /// Space → (person, last start in Unix ms).
    typers: HashMap<String, Vec<(Person, i64)>>,
    /// Space → (composer text last seen, last start sent in Unix ms or 0).
    sent: HashMap<String, (String, i64)>,
}
thread_local! { static S: RefCell<State> = RefCell::new(State::default()); }

fn same(a: &Person, b: &Person) -> bool {
    (!a.id.is_empty() && a.id == b.id) || (!a.email.is_empty() && a.email.eq_ignore_ascii_case(&b.email))
}

/// Pure: the typers still shown at `now`.
pub(crate) fn live_typers(list: &[(Person, i64)], now: i64) -> Vec<Person> {
    list.iter().filter(|(_, t)| now - t < TYPER_EXPIRY_MS).map(|(p, _)| p.clone()).collect()
}

/// Pure: whether a composer change sends a start now. (`last_sent` 0 = none since the text was empty.)
pub(crate) fn send_due(text: &str, editing: bool, last_sent: i64, now: i64) -> bool {
    !editing && !text.trim().is_empty() && (last_sent == 0 || now - last_sent >= SEND_EVERY_MS)
}

/// A typing event from the cloud.
pub(crate) fn on_typing(space: String, person: Person, on: bool) {
    let now = super::chrome::now_ms();
    S.with(|s| {
        let mut s = s.borrow_mut();
        let list = s.typers.entry(space).or_default();
        list.retain(|(p, _)| !same(p, &person));
        if on { list.push((person, now)); }
    });
}

/// Their message arrived: they stopped typing it.
pub(crate) fn message_from(space: &str, email: &str) {
    if email.is_empty() { return }
    S.with(|s| { if let Some(l) = s.borrow_mut().typers.get_mut(space) { l.retain(|(p, _)| !p.email.eq_ignore_ascii_case(email)); } });
}

/// Who is typing in `space` now.
pub(crate) fn typers(space: &str) -> Vec<Person> {
    let now = super::chrome::now_ms();
    S.with(|s| s.borrow().typers.get(space).map(|l| live_typers(l, now)).unwrap_or_default())
}

/// Whether `p` is typing in `space`.
pub(crate) fn is_typing(space: &str, p: &Person) -> bool { typers(space).iter().any(|t| same(t, p)) }

impl App {
    /// Once per frame: send my typing for the open chat when due; keep the window drawing while someone types (the dots
    /// move, and a typer disappears on time although no event arrives).
    pub(crate) fn typing_frame(&mut self, ctx: &egui::Context) {
        let Some(id) = self.selected.clone() else { return };
        let now = super::chrome::now_ms();
        let text = self.drafts.get(&id).cloned().unwrap_or_default();
        let editing = matches!(self.compose, Compose::Edit { .. });
        let send = S.with(|s| {
            let mut s = s.borrow_mut();
            let e = s.sent.entry(id.clone()).or_insert_with(|| (text.clone(), 0));
            if e.0 == text { return false }
            e.0 = text.clone();
            if text.trim().is_empty() { e.1 = 0; return false } // emptied: the timer resets
            if send_due(&text, editing, e.1, now) { e.1 = now; true } else { false }
        });
        if send { if let Some(c) = &self.client { c.req(Req::Typing(id.clone())); } }
        self.typing_shot(&id);
        // Only while someone types here: one frame per animation step (a typer's expiry falls on one of them); idle otherwise.
        if !typers(&id).is_empty() { ctx.request_repaint_after(next_frame_in(anim_time(ctx))); }
    }

    /// --screenshot: `WEBOX_SHOT_TYPING=1` lets the open (sample) chat's partner type from frame 30 on.
    fn typing_shot(&self, id: &str) {
        if self.shot.is_none() || self.frames != 30 || std::env::var("WEBOX_SHOT_TYPING").is_err() { return }
        let s = self.spaces.iter().find(|s| s.id == id);
        let p = s.and_then(|s| self.direct_person(s)).or_else(|| self.messages.get(id).and_then(|l| l.iter().rev().find(|m| !m.mine && !m.email.is_empty()))
            .map(|m| Person { id: String::new(), name: m.author.clone(), email: m.email.clone(), status: String::new() }));
        if let Some(p) = p { on_typing(id.to_string(), p, true); }
    }
}

/// The animation: 12 frames of 80 ms, looping.
pub(crate) const FRAME_MS: f64 = 80.0;
/// Grey level of each dot per frame: a brightness wave running left to right.
const LEVELS: [[u8; 12]; 3] = [
    [128, 153, 178, 204, 230, 230, 230, 204, 178, 153, 128, 102],
    [170, 178, 102, 128, 153, 178, 204, 230, 230, 230, 204, 178],
    [230, 204, 178, 153, 128, 102, 128, 153, 178, 204, 230, 230],
];

/// The animation clock in s: egui's input time; `WEBOX_SHOT_TYPING_MS` pins it for screenshots.
pub(crate) fn anim_time(ctx: &egui::Context) -> f64 {
    static PIN: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
    PIN.get_or_init(|| std::env::var("WEBOX_SHOT_TYPING_MS").ok().and_then(|v| v.parse::<f64>().ok()).map(|ms| ms / 1000.0))
        .unwrap_or_else(|| ctx.input(|i| i.time))
}

/// Pure: the animation frame at `time` (s).
pub(crate) fn frame_at(time: f64) -> usize { ((time * 1000.0 / FRAME_MS).floor().max(0.0) as u64 % 12) as usize }

/// Pure: until the next frame starts (at least 1 ms).
pub(crate) fn next_frame_in(time: f64) -> std::time::Duration {
    let ms = FRAME_MS - (time * 1000.0).rem_euclid(FRAME_MS);
    std::time::Duration::from_secs_f64(ms.max(1.0) / 1000.0)
}

/// The overlay on a typing person's avatar (`r` = the avatar's 26 x 24 pt slot): `avatar-overlay` (#121212 at 40 %),
/// radius height / 2, and three dots in a 52:16 box of the slot's width - 4, centred.
pub(crate) fn draw_overlay(p: &egui::Painter, r: egui::Rect, time: f64) {
    p.rect_filled(r, r.height() / 2.0, egui::Color32::from_rgba_unmultiplied(0x12, 0x12, 0x12, 102));
    let w = r.width() - 4.0;
    let s = w / 52.0; // pt per pixel of the 52 x 16 box
    let left = r.center().x - w / 2.0;
    let f = frame_at(time);
    for (i, cx) in [9.0, 26.0, 42.0].into_iter().enumerate() {
        let g = LEVELS[i][f];
        p.circle_filled(egui::pos2(left + cx * s, r.center().y), 6.0 * s, egui::Color32::from_gray(g));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(email: &str) -> Person { Person { email: email.into(), ..Default::default() } }

    #[test]
    fn typers_expire_after_ten_seconds_without_a_new_start() {
        let list = vec![(p("a@example.com"), 1_000), (p("b@example.com"), 6_000)];
        assert_eq!(live_typers(&list, 10_999).len(), 2);
        assert_eq!(live_typers(&list, 11_000).len(), 1, "a's start is 10 s old");
        assert!(live_typers(&list, 16_000).is_empty());
    }

    #[test]
    fn send_at_most_every_five_seconds_while_text_is_typed() {
        assert!(send_due("H", false, 0, 1_000), "the first character sends at once");
        assert!(!send_due("Ha", false, 1_000, 5_999));
        assert!(send_due("Hal", false, 1_000, 6_000));
        assert!(!send_due("   ", false, 0, 9_000), "no text, nothing");
        assert!(!send_due("New", true, 0, 9_000), "not while editing a message");
    }

    #[test]
    fn animation_steps_every_80_ms_and_loops_after_12_frames() {
        assert_eq!(frame_at(0.0), 0);
        assert_eq!(frame_at(0.079), 0);
        assert_eq!(frame_at(0.080), 1);
        assert_eq!(frame_at(0.959), 11);
        assert_eq!(frame_at(0.960), 0);
        // The repaint lands on the next step, never later than one frame.
        assert_eq!(next_frame_in(0.0).as_millis(), 80);
        assert_eq!(next_frame_in(0.030).as_millis(), 50);
        assert!(next_frame_in(0.0799).as_millis() <= 1);
        for i in 0..1000 { assert!(next_frame_in(i as f64 * 0.0137) <= std::time::Duration::from_millis(80)); }
        // Each dot peaks once per loop, left to right.
        let peak = |d: usize| (0..12).find(|&f| LEVELS[d][f] == 230).unwrap();
        assert!(peak(0) < peak(1));
    }

    #[test]
    fn a_start_shows_at_once_and_expires_without_a_stop() {
        let t0 = std::time::Instant::now();
        on_typing("t-lat".into(), p("c@example.com"), true);
        assert!(is_typing("t-lat", &p("C@example.com")));
        assert!(t0.elapsed() < std::time::Duration::from_millis(5));
        S.with(|s| { for e in s.borrow_mut().typers.get_mut("t-lat").unwrap() { e.1 -= TYPER_EXPIRY_MS; } }); // the start is 10 s old
        assert!(typers("t-lat").is_empty(), "no stop arrived: gone after 10 s");
    }

    #[test]
    fn stop_and_message_remove_the_typer() {
        on_typing("t-sp".into(), p("a@example.com"), true);
        assert_eq!(typers("t-sp").len(), 1);
        message_from("t-sp", "A@example.com");
        assert!(typers("t-sp").is_empty());
        on_typing("t-sp".into(), p("a@example.com"), true);
        on_typing("t-sp".into(), p("a@example.com"), false);
        assert!(typers("t-sp").is_empty());
    }
}
