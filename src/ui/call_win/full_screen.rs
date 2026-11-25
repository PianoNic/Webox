//! Full screen for the call window (the official MeetingViewButtons "Vollbild" / "Vollbildmodus beenden"): the title bar
//! button, F11 and the registry's `FullScreen` (Alt+Eingabe) toggle it, Esc leaves it. The OS does the work (winit
//! borderless full screen on the window's monitor); nothing is drawn or repainted for it.
use super::*;

impl App {
    /// Each call-window frame: applies a pending toggle and the window's own keys. Returns whether the window is in full screen.
    pub(super) fn call_full_screen(&mut self, vctx: &egui::Context) -> bool {
        let full = vctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let (f11, esc) = vctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::F11), full && i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)));
        if std::mem::take(&mut self.call.win.full_toggle) | f11 | esc { vctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full)); }
        full
    }
}
