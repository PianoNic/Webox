//! Typing indicators in the window (protocol: cloud/typing.rs).
//!
//! There is no "… schreibt" text: the typing person's avatar in the "Gesehen von" row at the end of the chat gets a round
//! overlay (`avatar-overlay`, radius height / 2) with three animated dots inside (width = avatar width - 4) while they type;
//! a typer who has not read the newest message joins that row.
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
        if !typers(&id).is_empty() { ctx.request_repaint_after(std::time::Duration::from_millis(120)); }
    }
}

/// The overlay on a typing person's avatar (`r` = the avatar's rect).
pub(crate) fn draw_overlay(p: &egui::Painter, r: egui::Rect, time: f64) {
    p.circle_filled(r.center(), r.width() / 2.0, egui::Color32::from_black_alpha(150));
    let w = r.width() - 4.0; // avatar width - 4
    let step = w / 4.0;
    for i in 0..3 {
        // A dot rises in turn (period 1.2 s).
        let phase = ((time * 1000.0 / 400.0) as i64 % 3) as usize;
        let lift = if phase == i { 2.0 } else { 0.0 };
        p.circle_filled(egui::pos2(r.left() + 2.0 + step * (i as f32 + 1.0), r.center().y - lift), 1.6, egui::Color32::WHITE);
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
        assert!(!send_due("Neu", true, 0, 9_000), "not while editing a message");
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
