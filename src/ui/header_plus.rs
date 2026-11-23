//! The + right of the search pill and its menu ("Nachricht senden", "Bereich erstellen", "Zu Webex einladen").
//!
//! - A click opens the global menu; tooltip "Eine Unterhaltung starten".
//! - The three entries open the "Neue Nachricht" pane (ui/new_message/), the create-space pane (`create_space.rs`) and the
//!   invite pane (ui/invite.rs).
//! - Texts: 'Send a message' / 'Create a space' / 'Invite to {{APP_NAME}}' (language pack, QObject context).
//! - Icons: chat-regular, chat-group-regular (two bubbles, no plus), participant-add-regular.
//! - Geometry (pt): balloon 284 wide, rows 40, padding 8, caret 15 high with its tip right under the button,
//!   icon 16 px centred 34 px from the balloon's left edge, label at 58 px.
use crate::*;
use super::menu;

/// The menu key (WEBOX_SHOT_MENU=plus opens it for a screenshot).
pub(crate) const KEY: &str = "plus";

const W: f32 = 284.0;
const ROW: f32 = 40.0;
const PAD: f32 = 8.0;
const CARET: f32 = 15.0;
const ICON: f32 = 16.0;
const ICON_CX: f32 = 34.0;
const TEXT_X: f32 = 58.0;
/// Label size (pt).
const FONT: f32 = 15.5;

/// The entries in menu order: (id, icon, English source text).
pub(crate) const ENTRIES: [(&str, &str, &str); 3] = [
    ("send", "chat-regular", "Send a message"),
    ("space", "chat-group-regular", "Create a space"),
    ("invite", "participant-add-regular", "Invite to {{APP_NAME}}"),
];

/// The label shown for an entry (German pack text, {{APP_NAME}} filled by the naming rule: "Invite to" keeps the service
/// name Webex, docs/naming.md).
pub(crate) fn label(src: &str) -> String { crate::i18n::brand("QObject", src, tr(src).into()).into_owned() }

/// The balloon body under a button `anchor`: centred on it, caret tip 2 px under it, body below the caret.
pub(crate) fn body_rect(anchor: Rect, screen: Rect) -> Rect {
    let h = PAD * 2.0 + ROW * ENTRIES.len() as f32;
    let x = (anchor.center().x - W / 2.0).clamp(screen.left() + 8.0, (screen.right() - W - 8.0).max(screen.left() + 8.0));
    Rect::from_min_size(pos2(x, anchor.bottom() + 2.0 + CARET), Vec2::new(W, h))
}

impl App {
    /// Draws the open menu under `anchor` (the + circle) and runs the picked entry.
    pub(crate) fn header_plus_menu(&mut self, ctx: &egui::Context, t: &Th, anchor: Rect, now: f64) {
        // --sample screenshots: WEBOX_SHOT_NEWMSG=<query>, WEBOX_SHOT_INVITE=<typed text> ("~" = empty; Windows drops empty variables).
        if self.shot.is_some() && self.frames == 3 {
            let val = |k: &str| std::env::var(k).ok().map(|v| if v == "~" { String::new() } else { v });
            if let Some(q) = val("WEBOX_SHOT_NEWMSG") {
                self.new_message_open(ctx);
                let mut s = super::new_message::state(ctx); s.query = q;
                // WEBOX_SHOT_NEWMSG_TO=<name part>,<name part> adds those sample people as chips; WEBOX_SHOT_NEWMSG_EDIT=<text> opens the
                // pencil's name field with that text ("~" = empty).
                for part in val("WEBOX_SHOT_NEWMSG_TO").unwrap_or_default().split(',').filter(|p| !p.is_empty()) {
                    if let Some(p) = self.people.iter().find(|p| p.name.to_lowercase().contains(&part.to_lowercase())) { s.to.push(p.clone()); }
                }
                if let Some(e) = val("WEBOX_SHOT_NEWMSG_EDIT") { s.editing = true; s.name_buf = e; s.focus = true; }
                super::new_message::set_state(ctx, s);
            }
            if let Some(e) = val("WEBOX_SHOT_INVITE") { self.invite_open(ctx); super::invite::shot_text(ctx, &e); }
        }
        if !menu::menu_is_open(ctx, KEY) { return; }
        let dark = !menu::light(t);
        let (bg, border, hover, pressed, text) = if dark {
            (Color32::BLACK, alpha(Color32::WHITE, 0.2), alpha(Color32::WHITE, 0.15), alpha(Color32::WHITE, 0.22), alpha(Color32::WHITE, 0.95))
        } else {
            (Color32::WHITE, alpha(Color32::BLACK, 0.2), alpha(Color32::BLACK, 0.07), alpha(Color32::BLACK, 0.14), alpha(Color32::BLACK, 0.95))
        };
        let body = body_rect(anchor, ctx.screen_rect());
        let outer = Rect::from_min_max(pos2(body.left(), anchor.bottom() + 2.0), body.max);
        let sel_id = egui::Id::new("header-plus-sel");
        let mut sel: Option<usize> = ctx.data(|d| d.get_temp(sel_id));
        let mut picked: Option<&str> = None;
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        let n = ENTRIES.len();
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) { sel = Some(sel.map_or(0, |s| (s + 1).min(n - 1))); }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) { sel = Some(sel.map_or(n - 1, |s| s.saturating_sub(1))); }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) { if let Some(s) = sel { picked = Some(ENTRIES[s].0); } }

        egui::Area::new(egui::Id::new("header-plus-menu")).fixed_pos(outer.min).order(egui::Order::Foreground).interactable(true).show(ctx, |ui| {
            ui.set_clip_rect(outer.expand(24.0));
            ui.allocate_exact_size(outer.size(), Sense::hover());
            let pt = ui.painter().clone();
            pt.add(egui::Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(if dark { 140 } else { 40 }) }.as_shape(body, 12.0));
            pt.rect_filled(body, 12.0, bg);
            pt.rect_stroke(body, 12.0, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
            // Caret: 45° sides, tip under the button centre.
            let cx = anchor.center().x;
            let (tip, base) = (pos2(cx, body.top() - CARET), body.top());
            pt.add(egui::Shape::convex_polygon(vec![pos2(cx - CARET, base + 1.0), pos2(cx + CARET, base + 1.0), tip], bg, Stroke::NONE));
            pt.line_segment([pos2(cx - CARET, base), tip], Stroke::new(1.0_f32, border));
            pt.line_segment([pos2(cx + CARET, base), tip], Stroke::new(1.0_f32, border));
            for (i, (id, icon, src)) in ENTRIES.iter().enumerate() {
                let row = Rect::from_min_size(pos2(body.left() + PAD, body.top() + PAD + ROW * i as f32), Vec2::new(W - 2.0 * PAD, ROW));
                let resp = ui.interact(row, egui::Id::new(("header-plus-item", i)), Sense::click());
                if resp.hovered() { sel = Some(i); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                if resp.is_pointer_button_down_on() { pt.rect_filled(row, 8.0, pressed); } else if sel == Some(i) { pt.rect_filled(row, 8.0, hover); }
                self.icons.paint(ui, icon, Rect::from_center_size(pos2(body.left() + ICON_CX, row.center().y), Vec2::splat(ICON)), text);
                pt.text(pos2(body.left() + TEXT_X, row.center().y), egui::Align2::LEFT_CENTER, label(src), FontId::proportional(FONT), text);
                if resp.clicked() { picked = Some(id); }
            }
        });
        // A click elsewhere closes the menu (the + itself toggles it).
        if let Some(pos) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None }) {
            if !outer.expand(2.0).contains(pos) && !anchor.contains(pos) { close = true; }
        }
        if picked.is_some() { close = true; }
        ctx.data_mut(|d| match sel { Some(s) => d.insert_temp(sel_id, s), None => d.remove::<usize>(sel_id) });
        if close { menu::menu_close(ctx, KEY); ctx.data_mut(|d| d.remove::<usize>(sel_id)); }
        match picked {
            // The create-space pane.
            Some("space") => self.act(Act::CreateSpace, ctx),
            // The "Neue Nachricht" pane (ui/new_message/).
            Some("send") => self.new_message_open(ctx),
            // The invite pane (ui/invite.rs).
            Some("invite") => self.invite_open(ctx),
            _ => {}
        }
        let _ = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_follow_the_original() {
        let ids: Vec<&str> = ENTRIES.iter().map(|e| e.0).collect();
        assert_eq!(ids, ["send", "space", "invite"]);
        assert_eq!(ENTRIES[1].1, "chat-group-regular");
        assert!(!label("Invite to {{APP_NAME}}").contains("{{"));
    }

    #[test]
    fn balloon_is_centred_under_the_button_and_kept_on_screen() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1272.0, 720.0));
        let b = body_rect(Rect::from_center_size(pos2(700.0, 26.0), Vec2::splat(26.0)), screen);
        assert_eq!(b.center().x, 700.0);
        assert_eq!(b.top(), 26.0 + 13.0 + 2.0 + CARET);
        assert_eq!(b.height(), 136.0);
        let b = body_rect(Rect::from_center_size(pos2(1260.0, 26.0), Vec2::splat(26.0)), screen);
        assert!(b.right() <= 1264.0);
    }
}
