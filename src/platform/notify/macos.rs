//! macOS, once per frame: the app menu (Webox / Bearbeiten / Fenster) and its commands, the toasts as Notification Center
//! banners (`platform::macos::notifications`: under Webox's own name inside Webox.app, osascript without a bundle) and what the
//! user did with them (a click opens the space, a button acts like the toast's button). The banner of an incoming call
//! (Annehmen / Ablehnen) is `ui/call_note_macos.rs`. `WEBOX_NATIVE_TOASTS=0` keeps the app's own toast window instead.
use super::*;
use crate::platform::macos::appkit::{self, MenuCmd};
use crate::platform::macos::notifications as un;
use std::sync::Mutex;

/// Banners sent (by id), for their answers: a toast's space and buttons.
static SENT: Mutex<Vec<(String, Toast)>> = Mutex::new(Vec::new());
/// Answers to the incoming-call banner (`ui/call_note_macos.rs`).
pub(crate) static CALL_ANSWERS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The id of the incoming-call banner.
pub(crate) const CALL_ID: &str = "webox-call";

impl App {
    pub(super) fn mac_frame(&mut self, ctx: &egui::Context, now: f64) {
        static STARTED: std::sync::Once = std::sync::Once::new();
        let shot = self.shot.is_some();
        STARTED.call_once(|| { appkit::install_main_menu(ctx); if !shot { un::init(ctx); } });

        for c in appkit::take_menu() {
            match c {
                MenuCmd::About => self.pick_tray("about", ctx, now),
                MenuCmd::Settings => { self.show_main(ctx); self.set_tab(Tab::Settings); }
                MenuCmd::Quit => self.pick_tray("quit", ctx, now),
                // Cmd+W: like the window's X (into the menu bar when "close to tray" is on, else quit).
                MenuCmd::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                MenuCmd::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                MenuCmd::Zoom => { let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false)); ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max)); }
                _ => {} // edit commands: App::raw_input_hook
            }
        }

        if !shot && !std::env::var("WEBOX_NATIVE_TOASTS").is_ok_and(|v| v == "0") {
            for ts in std::mem::take(&mut self.notify.toasts) {
                if ts.dismissed { continue; }
                let lead = ts.lead.as_ref().map(|l| l.0.as_str()).unwrap_or("");
                let body = [ts.kicker.as_str(), lead, ts.body.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("\n");
                let id = if ts.key.is_empty() { format!("webox-{}", ts.id) } else { format!("webox-{}", ts.key) };
                let buttons: Vec<(&str, &str)> = ts.buttons.iter().map(|(b, l, _)| (*b, l.as_str())).collect();
                if un::send(&id, &ts.title, &body, &buttons, false) {
                    let mut sent = SENT.lock().unwrap();
                    sent.retain(|(i, _)| *i != id);
                    sent.push((id, ts));
                    let n = sent.len();
                    if n > 50 { sent.drain(..n - 50); }
                }
            }
        }

        crate::ui::call_note_macos::call_note_frame(ctx);
        for a in un::take() {
            if a.id == CALL_ID { CALL_ANSWERS.lock().unwrap().push(a.action); continue; }
            let Some(ts) = SENT.lock().unwrap().iter().find(|(i, _)| *i == a.id).map(|(_, t)| t.clone()) else {
                self.notify.out.push(Out::Open(None)); // from before a restart: just the window
                continue;
            };
            match ts.buttons.iter().find(|(b, _, _)| *b == a.action) {
                Some((b, _, _)) => self.notify.out.push(Out::Button(ts.clone(), b)),
                None => self.notify.out.push(Out::Open(ts.space.clone())),
            }
        }
    }
}
