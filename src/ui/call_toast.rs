//! The incoming-call toast as its own frameless, always-on-top, non-activating OS window at the bottom-right of the work
//! area: content width 320, fade-out 400 ms, Esc closes, tab cycle answer -> answer on computer -> decline -> message ->
//! avatar -> mute -> close. The window margin (`toast_pos::EDGE_GAP`), button diameters and content height are Webox's own.
//! Like the official `CallDelegate`: the bell is "Anruf stummschalten" (Benachrichtigungston an / aus), the close button is
//! "Anruf ignorieren" (toast and local ring go, the call is not declined). The message button opens the quick replies
//! ("Anruf ablehnen und mit Textnachricht antworten"). Under "Bitte nicht stören" / Ruhezeiten the toast shows without ringing.
use super::call::{state_text, CallActions, CallView, QUICK_REPLIES};
use super::toast_pos::toast_anchor;
use crate::cloud::locus::CallState;
use crate::*;

/// Toast content width (px).
pub(crate) const CONTENT_W: f32 = 320.0;
/// 8 top margin + 32 title row + 16 + 40 avatar/text row + 16 + 40 round buttons + 12 + 40 answer-on-computer pill + 16 bottom.
pub(crate) const CONTENT_H: f32 = 220.0;
/// Row height of the quick-reply list (5 rows fit below the title row).
const REPLY_ROW_H: f32 = 32.0;
/// Fade-out duration (s).
const FADE_SECS: f64 = 0.4;
pub(crate) const TOAST_TITLE: &str = "Webox - Eingehender Anruf";

/// Round toolkit-style button drawn into an explicit rect (so the tab order can differ from the paint order); registered with
/// `Sense::click()` so egui's Tab navigation, Enter / Space and the focus ring work.
pub(crate) fn round_at(ui: &mut egui::Ui, ic: &mut icons::Icons, id: &str, center: egui::Pos2, d: f32, icon: &'static str, fill: Color32, icon_col: Color32, ring: Color32) -> egui::Response {
    let r = Rect::from_center_size(center, Vec2::splat(d));
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let f = if resp.is_pointer_button_down_on() { fill.gamma_multiply(0.7) } else if resp.hovered() { fill.gamma_multiply(0.85) } else { fill };
    ui.painter().circle_filled(center, d / 2.0, f);
    ic.paint(ui, icon, Rect::from_center_size(center, Vec2::splat(d * 0.5)), icon_col);
    if resp.has_focus() { ui.painter().circle_stroke(center, d / 2.0 + 1.5, Stroke::new(2.0_f32, ring)); }
    if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
    resp
}

/// A ghost (transparent, hover-filled) round button, for the toast title row and the window headers.
pub(crate) fn ghost_at(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, id: &str, center: egui::Pos2, d: f32, icon: &'static str, col: Color32) -> egui::Response {
    let r = Rect::from_center_size(center, Vec2::splat(d));
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    if resp.hovered() || resp.has_focus() { ui.painter().circle_filled(center, d / 2.0, if resp.is_pointer_button_down_on() { t.pill } else { t.hover }); }
    ic.paint(ui, icon, Rect::from_center_size(center, Vec2::splat(d * 0.55)), col);
    if resp.has_focus() { ui.painter().circle_stroke(center, d / 2.0, Stroke::new(2.0_f32, t.focus)); }
    if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
    resp
}

/// The answer button's icon and tooltip: answered with the camera on, or audio only.
pub(crate) fn answer_look(video: bool) -> (&'static str, &'static str) {
    if video { ("video-regular", "Antworten (Video an)") } else { ("handset-regular", "Antworten") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_answer_button_shows_how_the_call_is_answered() {
        assert_eq!(super::answer_look(true).0, "video-regular", "Anrufe mit eingeschalteter Videokamera annehmen");
        assert_eq!(super::answer_look(false), ("handset-regular", "Antworten"));
    }
}

impl App {
    /// Shows (or fades out) the toast window; returns what the user pressed. Called once per frame while a call rings.
    pub(crate) fn incoming_toast_window(&mut self, ctx: &egui::Context, t: &Th, now: f64, v: &CallView) -> CallActions {
        // Ignored: no notification banner either (`call_note_frame` takes it away when `call_note` isn't called).
        let mut actions = if self.call.ignored { CallActions::default() } else { self.call_note(ctx, v) };
        let w = &mut self.call.win;
        let fade = w.toast_closed.map(|s| 1.0 - ((now - s) / FADE_SECS) as f32);
        if fade.is_some_and(|f| f <= 0.0) { return actions; } // faded out: window gone; the call rings on my other devices
        let note = std::mem::take(&mut actions);
        // Exactly the card: no shadow, so no shadow padding.
        let size = Vec2::new(CONTENT_W, CONTENT_H);
        let pos = *w.toast_pos.get_or_insert_with(|| toast_anchor(ctx, size, 0));
        let b = egui::ViewportBuilder::default().with_title(TOAST_TITLE).with_decorations(false).with_always_on_top().with_resizable(false)
            .with_inner_size(size).with_position(pos).with_taskbar(false).with_transparent(true).with_active(false);
        // No fade without a compositor: it would darken towards black instead of showing what's behind.
        let opacity = if crate::platform::tray::composited() { fade.unwrap_or(1.0).clamp(0.0, 1.0) } else { 1.0 };
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("webox-incoming-toast"), b, |vctx, class| {
            if class != egui::ViewportClass::Immediate { return; }
            actions = self.toast_content(vctx, t, v, opacity);
            self.viewport_shot(vctx, "toast");
        });
        // Per-pixel alpha around the card (see tray::see_through).
        if self.call.win.toast_hwnd.is_none() {
            use crate::platform::tray;
            self.call.win.toast_hwnd = tray::find_window(TOAST_TITLE);
            if let Some(h) = self.call.win.toast_hwnd {
                tray::see_through(h);
            }
        }
        if fade.is_some() { ctx.request_repaint(); }
        // "Anruf ignorieren": the toast fades out and the local ring stops; the call is not declined.
        if actions.dismiss { self.call.win.toast_closed = Some(now); self.call.ignored = true; self.call.reply_open = false; }
        if let Some(text) = actions.reply.take() { self.decline_with_reply(&v.email, &text, now); }
        actions.answer |= note.answer; actions.hang |= note.hang;
        actions
    }

    /// The call notification (Wayland only, see `CallNote`): sent once per ringing; its buttons act like the toast's.
    #[cfg(target_os = "linux")]
    pub(crate) fn call_note(&mut self, ctx: &egui::Context, v: &CallView) -> CallActions {
        let mut a = CallActions::default();
        if crate::platform::linux::session() == crate::platform::linux::Session::Wayland && self.shot.is_none() {
            let pass = ctx.cumulative_pass_nr();
            let urgency = if self.calls_silenced() { 1 } else { 2 };
            let action = {
                let mut g = CALL_NOTE.lock().unwrap();
                let n = g.get_or_insert_with(|| {
                    let body = [state_text(&CallState::Incoming, 0), v.email.clone()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join("\n");
                    let acts = [("default", "Öffnen"), ("answer", "Annehmen"), ("decline", "Ablehnen")];
                    // Critical rings through the desktop's own do-not-disturb; under Bitte nicht stören / Ruhezeiten it is a normal one.
                    CallNote { sent: crate::platform::linux::notifications::send(ctx, &v.name, &body, &acts, urgency), seen: pass, action: None }
                });
                n.seen = pass;
                n.action.take()
            };
            match action {
                Some((k, _)) if k == "decline" => a.hang = true,
                Some((k, token)) => { a.answer = k == "answer"; self.notify.raise_token = token; self.show_main(ctx); } // its own activation token raises the window
                None => {}
            }
        }
        a
    }
    /// No call notification on Windows (the toast window is placed and kept on top there); macOS: `ui/call_note_macos.rs`.
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn call_note(&mut self, _ctx: &egui::Context, _v: &CallView) -> CallActions { CallActions::default() }

    fn toast_content(&mut self, vctx: &egui::Context, t: &Th, v: &CallView, opacity: f32) -> CallActions {
        let mut a = CallActions::default();
        let replies = self.call.reply_open;
        if vctx.input(|i| i.key_pressed(Key::Escape)) { if replies { self.call.reply_open = false; } else { a.dismiss = true; } } // Esc: back, or ignore
        let silenced = self.calls_silenced();
        let muted = self.call.ring_muted || silenced;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(vctx, |ui| {
            ui.set_opacity(opacity);
            let full = ui.max_rect();
            // The window is the card, without a drop shadow.
            let body = full;
            // modal-primary-background (#000000 dark / #FFFFFF light) with modal-primary-border (#FFFFFF33 / #00000033).
            let bg = t.tk("modal-primary-background");
            ui.painter().rect_filled(body, 0.0, bg);
            ui.painter().rect_stroke(body, 0.0, Stroke::new(1.0_f32, t.sep), egui::StrokeKind::Inside);
            let o = body.min;
            let at = |x: f32, y: f32| o + Vec2::new(x, y);
            let ring = t.focus;
            // Bottom rows first by position, but registered in the tab order.
            let mid_y = 8.0 + 32.0 + 16.0 + 20.0; // centre of the avatar row
            let row_y = 8.0 + 32.0 + 16.0 + 40.0 + 16.0 + 20.0; // centre of the round buttons
            let pill = Rect::from_min_size(at(16.0, row_y + 20.0 + 12.0), Vec2::new(CONTENT_W - 32.0, 40.0));
            if replies {
                // Quick replies, then the custom message (declines and opens the chat) and Abbrechen (back to the buttons).
                let mut y = 8.0 + 32.0 + 8.0;
                let rows = QUICK_REPLIES.iter().map(|r| (*r, 0u8)).chain([("Schreiben Sie eine benutzerdefinierte Nachricht.", 1), ("Abbrechen", 2)]);
                for (i, (text, kind)) in rows.enumerate() {
                    let r = Rect::from_min_size(at(8.0, y), Vec2::new(CONTENT_W - 16.0, REPLY_ROW_H));
                    let resp = ui.interact(r, ui.id().with(("reply", i)), Sense::click());
                    if resp.hovered() || resp.has_focus() { ui.painter().rect_filled(r, 6.0, if resp.is_pointer_button_down_on() { t.pill } else { t.hover }); }
                    if resp.has_focus() { ui.painter().rect_stroke(r, 6.0, Stroke::new(2.0_f32, ring), egui::StrokeKind::Inside); }
                    if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    crate::ui::chrome::fit_text(ui.painter(), r.min.x + 8.0, r.center().y - 9.0, text, FontId::proportional(14.0), if kind == 0 { t.text1 } else { t.text2 }, r.width() - 16.0);
                    if resp.clicked() { match kind { 0 => a.reply = Some(text.to_string()), 1 => a.message = true, _ => self.call.reply_open = false } }
                    y += REPLY_ROW_H;
                }
            } else {
            // 1. Answer (round). Its icon says how the call is answered (the toast model's `answerIcon`): the camera while
            // "Anrufe mit eingeschalteter Videokamera annehmen" is on, else the handset.
            let video = self.answer_with_video();
            let (icon, tip) = answer_look(video);
            if round_at(ui, &mut self.icons, "answer", at(CONTENT_W / 2.0 + 52.0, row_y), 40.0, icon, t.green, Color32::WHITE, ring).on_hover_text(tip).clicked() { a.answer = true; }
            // 2. Answer on this computer (pill, secondary because the round answer button is shown too).
            {
                let resp = ui.interact(pill, ui.id().with("answer-pc"), Sense::click());
                let fill = if resp.hovered() { t.pill } else { t.selected };
                ui.painter().rect_filled(pill, 20.0, fill);
                ui.painter().text(pill.center(), egui::Align2::CENTER_CENTER, "Auf meinem Computer annehmen", semibold(14.0), t.text1);
                if resp.has_focus() { ui.painter().rect_stroke(pill.expand(1.5), 21.5, Stroke::new(2.0_f32, ring), egui::StrokeKind::Inside); }
                if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                if resp.clicked() { a.answer = true; }
            }
            // 3. Decline (round).
            if round_at(ui, &mut self.icons, "decline", at(CONTENT_W / 2.0, row_y), 40.0, "handset-down-slash-regular", t.badge, Color32::WHITE, ring).on_hover_text("Ablehnen").clicked() { a.hang = true; }
            // 4. Decline with a text (round, secondary): opens the quick replies.
            if round_at(ui, &mut self.icons, "message", at(CONTENT_W / 2.0 - 52.0, row_y), 40.0, "chat-regular", t.selected, t.text1, ring).on_hover_text("Anruf ablehnen und mit Textnachricht antworten").clicked() { self.call.reply_open = true; }
            // 5. Avatar (click -> contact card).
            {
                let ar = Rect::from_center_size(at(32.0, mid_y - 4.0), Vec2::splat(32.0));
                let resp = ui.interact(ar, ui.id().with("avatar"), Sense::click());
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(ar));
                avatar_photo(&mut c, &mut self.icons, t, &v.name, 32.0, false, None, None, v.photo);
                if resp.has_focus() { ui.painter().circle_stroke(ar.center(), 17.5, Stroke::new(2.0_f32, ring)); }
                if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                if resp.on_hover_text("Visitenkarte anzeigen").clicked() { a.contact = true; }
            }
            }
            // 6. "Anruf stummschalten" (notification sound on / off; off while Bitte nicht stören / Ruhezeiten), 7. "Anruf ignorieren".
            let mute_icon = if muted { "alert-muted-bold" } else { "alert-bold" };
            let mute_tip = if silenced { "Bitte nicht stören" } else if muted { "Benachrichtigungston, aus" } else { "Benachrichtigungston, an" };
            if ghost_at(ui, &mut self.icons, t, "mute", at(CONTENT_W - 16.0 - 32.0 - 8.0 - 16.0, 8.0 + 16.0), 32.0, mute_icon, if muted { t.badge } else { t.text1 }).on_hover_text(mute_tip).clicked() && !silenced { a.toggle_ring = true; }
            if ghost_at(ui, &mut self.icons, t, "close", at(CONTENT_W - 16.0 - 16.0, 8.0 + 16.0), 32.0, "cancel-regular", t.text1).on_hover_text("Anruf ignorieren").clicked() { a.dismiss = true; }
            // Static content.
            let p = ui.painter().clone();
            p.text(at(16.0, 8.0 + 16.0), egui::Align2::LEFT_CENTER, state_text(&CallState::Incoming, 0), semibold(14.0), t.text1);
            if !replies {
                crate::ui::chrome::fit_text(&p, 16.0 + o.x + 44.0, o.y + mid_y - 12.0, &v.name, semibold(15.0), t.text1, CONTENT_W - 32.0 - 44.0);
                if !v.email.is_empty() { crate::ui::chrome::fit_text(&p, 16.0 + o.x + 44.0, o.y + mid_y + 6.0, &v.email, FontId::proportional(13.0), t.text2, CONTENT_W - 32.0 - 44.0); }
            }
        });
        vctx.request_repaint_after(std::time::Duration::from_millis(100));
        a
    }
}

/// Wayland: the incoming call also goes to the desktop's notification server (critical, "Annehmen" / "Ablehnen"), since the toast
/// window can neither be placed nor kept on top there. One notification per ringing; it closes when the ringing stops.
#[cfg(target_os = "linux")]
/// `action`: the key of a pressed button and its activation token.
struct CallNote { sent: crate::platform::linux::notifications::Sent, seen: u64, action: Option<(String, Option<String>)> }
#[cfg(target_os = "linux")]
static CALL_NOTE: std::sync::Mutex<Option<CallNote>> = std::sync::Mutex::new(None);

/// An action of the call notification (from `native_toasts`): true when `id` is that notification.
#[cfg(target_os = "linux")]
pub(crate) fn call_note_invoked(id: u32, key: &str, token: Option<String>) -> bool {
    let mut g = CALL_NOTE.lock().unwrap();
    let Some(n) = g.as_mut().filter(|n| n.sent.get() == Some(&Some(id))) else { return false };
    n.action = Some((key.to_string(), token));
    true
}

/// Once per frame after the call windows: the ringing stopped (no `incoming_toast_window` this pass) -> the notification goes.
#[cfg(target_os = "linux")]
pub(crate) fn call_note_frame(ctx: &egui::Context) {
    let mut g = CALL_NOTE.lock().unwrap();
    let Some(n) = g.as_ref().filter(|n| n.seen < ctx.cumulative_pass_nr()) else { return };
    match n.sent.get() {
        None => ctx.request_repaint_after(std::time::Duration::from_millis(100)), // still being sent: close it once it has an id
        Some(id) => { if let Some(id) = id { crate::platform::linux::notifications::close(*id); } *g = None; }
    }
}
