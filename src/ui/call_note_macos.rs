//! macOS: an incoming call also gets a Notification Center banner with "Accept" / "Decline" while Webox is not in front (the
//! toast window is there as on Windows; the banner reaches a user on another Space or in a full-screen app). Only inside
//! Webox.app (`notifications::bundled`); one banner per ringing, taken away when the ringing stops (`call_note_frame`).
use super::call::{state_text, CallActions, CallView};
use crate::cloud::locus::CallState;
use crate::platform::macos::notifications as un;
use crate::platform::notify::macos::{CALL_ANSWERS, CALL_ID};
use crate::*;
use std::sync::Mutex;

/// The pass the ringing was last seen in (None: no banner out).
static SEEN: Mutex<Option<u64>> = Mutex::new(None);

impl App {
    pub(crate) fn call_note(&mut self, ctx: &egui::Context, v: &CallView) -> CallActions {
        let mut a = CallActions::default();
        if self.shot.is_some() || !un::bundled() { return a; }
        let pass = ctx.cumulative_pass_nr();
        let mut seen = SEEN.lock().unwrap();
        if seen.is_none() && !self.window_active() {
            let body = [state_text(&CallState::Incoming, 0), v.email.clone()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join("\n");
            // Under "Do not disturb" / quiet hours the banner comes without its sound.
            un::send(CALL_ID, &v.name, &body, &[("answer", tr("Accept")), ("decline", tr("Decline"))], !self.calls_silenced());
            *seen = Some(pass);
        }
        if seen.is_some() { *seen = Some(pass); }
        drop(seen);
        for act in std::mem::take(&mut *CALL_ANSWERS.lock().unwrap()) {
            match act.as_str() {
                "decline" => a.hang = true,
                "answer" => { a.answer = true; self.show_main(ctx); }
                _ => self.show_main(ctx), // a click on the banner: the window with the call
            }
        }
        a
    }
}

/// Once per frame after the call windows: the ringing stopped (no `call_note` this pass) -> the banner goes.
pub(crate) fn call_note_frame(ctx: &egui::Context) {
    let mut seen = SEEN.lock().unwrap();
    if seen.is_some_and(|s| s < ctx.cumulative_pass_nr()) { *seen = None; un::remove(CALL_ID); }
}
