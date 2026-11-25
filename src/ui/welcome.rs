//! The welcome item of an empty chat (a message-list item): a centred column of header, description, the encryption line and a
//! row of pill buttons.
//!
//! - Fonts and colour tokens: header 26 px semibold, message-bodyText-primary-text; description 16 px,
//!   message-bodyText-secondary-text; security line 16 px underlined, label-success-text, bottom margin 2 x spacing; buttons
//!   L secondary pills, padding 2 x spacing, laid out left-to-right 5 apart, falling back to top-to-bottom when too narrow.
//! - Placement: the item starts 100 below the list top.
//! - Spacing 8, button height 32, icon 16, icon-text gap 8; the description / encryption line wrap width is MAX_W.
//! - Texts and buttons: "Say hi to start chatting 👋" with "This is where you two can get things done together. Start with a
//!   message or even a fun GIF." for a direct chat, with File / GIF / Schedule meeting (all enabled). Groups get no welcome item.
//! - The encryption line is not clickable; its tooltip is "Learn more about {{APP_NAME}} encryption".
use crate::*;
use egui::text::{LayoutJob, TextFormat};

/// Space between the list top and the item (pt).
const LIST_TOP: f32 = 100.0;
/// Header top padding: the header glyphs start 43.6 pt below the item top (egui's line box puts the glyph top ~0.6 under the
/// line top).
const PAD: f32 = 43.0;
/// Spacing (pt).
const SPACING: f32 = 8.0;
/// Wrap width of the description / encryption line, chosen so the lines break after "... Beginnen Sie" and
/// "... Verschlüsselung" / "... Klicken Sie" (with egui's metrics the widest kept line is 401 and the first excluded one 411).
const MAX_W: f32 = 406.0;
/// Button height (pt).
const BTN_H: f32 = 32.0;

/// Which welcome is shown (only the direct-chat one exists).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Welcome { Direct }

/// The welcome item for a chat: a direct chat with its other person present, not closed, whose message list has loaded and is
/// empty. A direct chat with nobody else left shows no welcome. Groups: none.
pub(crate) fn welcome_variant(direct: bool, closed: bool, has_other: bool, loaded_empty: bool) -> Option<Welcome> {
    (direct && !closed && has_other && loaded_empty).then_some(Welcome::Direct)
}

/// A text with emoji pictures: the emoji clusters become placeholders, returned with their char index.
fn emoji_job(text: &str, font: FontId, color: Color32, underline: bool, wrap: f32) -> (LayoutJob, Vec<(usize, String)>) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    job.halign = egui::Align::Center;
    let fmt = TextFormat { font_id: font, color, underline: if underline { Stroke::new(1.0_f32, color) } else { Stroke::NONE }, ..Default::default() };
    let mut marks = vec![];
    let (mut at, mut chars) = (0usize, 0usize);
    for (a, b) in super::emoji_text::clusters(text) {
        let plain = &text[at..a];
        job.append(plain, 0.0, fmt.clone());
        chars += plain.chars().count();
        match super::emoji_text::resolve(&text[a..b]) {
            Some(name) => { marks.push((chars, name)); job.append(super::emoji_text::PLACEHOLDER, 0.0, fmt.clone()); chars += super::emoji_text::PLACEHOLDER.chars().count(); }
            None => { job.append(&text[a..b], 0.0, fmt.clone()); chars += text[a..b].chars().count(); }
        }
        at = b;
    }
    job.append(&text[at..], 0.0, fmt);
    (job, marks)
}

/// One line of text with emoji pictures, left-aligned, vertically centred on `left_center`.
pub(crate) fn emoji_line(ui: &egui::Ui, icons: &mut icons::Icons, left_center: egui::Pos2, text: &str, font: FontId, color: Color32) {
    let (mut job, marks) = emoji_job(text, font, color, false, f32::INFINITY);
    job.halign = egui::Align::LEFT;
    let g = ui.fonts(|f| f.layout_job(job));
    let pos = pos2(left_center.x, left_center.y - g.size().y / 2.0);
    let spots = super::emoji_text::placeholder_rects(&g, &marks.iter().map(|m| m.0).collect::<Vec<_>>());
    ui.painter().galley(pos, g, color);
    for (spot, (_, name)) in spots.into_iter().zip(&marks) {
        if let Some(r) = spot { let r = r.translate(pos.to_vec2()); icons.emoji(ui, name, Rect::from_center_size(r.center(), Vec2::splat(r.height() * 0.8))); }
    }
}

impl App {
    /// The welcome item for `space` if there is one now (see `welcome_variant`).
    pub(crate) fn welcome_for(&self, space: &Space) -> Option<Welcome> {
        let loaded_empty = self.messages.get(&space.id).is_some_and(|m| m.is_empty());
        // Signed in: the other person must be among the participants; offline sample spaces carry no participants.
        let has_other = if self.client.is_some() { space.participants.iter().any(|p| p.id != self.me.id && (self.me.email.is_empty() || !p.email.eq_ignore_ascii_case(&self.me.email))) }
                        else { self.direct_person(space).is_some() };
        welcome_variant(space.direct, space.closed, has_other, loaded_empty)
    }

    /// Paints the welcome item into the message area `ui.max_rect()`.
    pub(crate) fn welcome_item(&mut self, ui: &mut egui::Ui, t: &Th, _w: Welcome, now: f64) {
        let area = ui.max_rect();
        let light = super::menu::light(t);
        let ink = if light { Color32::BLACK } else { Color32::WHITE };
        let (c_head, c_desc, c_link) = (alpha(ink, 0.95), alpha(ink, 0.70), if light { hex(0x185E46) } else { hex(0x3CC29A) });
        let cx = area.center().x;
        let mut y = area.top() + LIST_TOP + PAD;
        let put = |s: &mut Self, ui: &egui::Ui, text: &str, font: FontId, col: Color32, underline: bool, wrap: f32, y: &mut f32| -> Rect {
            let (job, marks) = emoji_job(text, font, col, underline, wrap);
            let g = ui.fonts(|f| f.layout_job(job));
            let pos = pos2(cx, *y);
            let rect = g.rect.translate(pos.to_vec2());
            let spots = super::emoji_text::placeholder_rects(&g, &marks.iter().map(|m| m.0).collect::<Vec<_>>());
            ui.painter().galley(pos, g.clone(), col);
            for (spot, (_, name)) in spots.into_iter().zip(&marks) {
                if let Some(r) = spot {
                    let side = r.height() * 0.8;
                    let r = r.translate(pos.to_vec2());
                    s.icons.emoji(ui, name, Rect::from_center_size(r.center(), Vec2::splat(side)));
                }
            }
            *y = rect.bottom();
            rect
        };
        put(self, ui, &crate::trq!("Say hi to start chatting 👋"), semibold(26.0), c_head, false, (area.width() - 32.0).max(100.0), &mut y);
        y += SPACING;
        let wrap = MAX_W.min(area.width() - 32.0).max(100.0);
        put(self, ui, &crate::trq!("This is where you two can get things done together. Start with a message or even a fun GIF."), FontId::proportional(16.0), c_desc, false, wrap, &mut y);
        y += SPACING;
        let sec = put(self, ui, &crate::trq!("🔒 This conversation is protected by strong encryption using a unique key. Click to learn more."), FontId::proportional(16.0), c_link, true, wrap, &mut y);
        ui.interact(sec, ui.id().with("welcome-security"), Sense::hover()).on_hover_text(crate::trq!("Learn more about {{APP_NAME}} encryption").into_owned()); // service encryption: {{APP_NAME}} = Webex (docs/naming.md)
        y += 2.0 * SPACING + SPACING;

        // Buttons: (icon, label, action)
        let btns: [(&'static str, String, u8); 3] = [
            ("attachment_16_em", tr!("WelcomeMessageItemDelegate", "Share a file").into_owned(), 0),
            ("gif_16", tr!("WelcomeMessageItemDelegate", "Send a GIF").into_owned(), 1),
            ("calendar-add-bold", tr!("WelcomeMessageItemDelegate", "Schedule a meeting").into_owned(), 2),
        ];
        let font = semibold(16.0);
        let widths: Vec<f32> = btns.iter().map(|b| ui.fonts(|f| f.layout_no_wrap(b.1.clone(), font.clone(), ink).size().x) + 16.0 + 8.0 + 2.0 * 2.0 * SPACING).collect();
        let total: f32 = widths.iter().sum::<f32>() + 5.0 * (btns.len() as f32 - 1.0);
        let row = area.width() > total;
        let mut x = cx - total / 2.0;
        let (txt, border, hov, prs) = (alpha(ink, 0.95), alpha(ink, 0.5), alpha(ink, 0.07), alpha(ink, 0.2)); // button-secondary-*
        for ((icon, label, act), w) in btns.iter().zip(&widths) {
            let r = if row { Rect::from_min_size(pos2(x, y), Vec2::new(*w, BTN_H)) } else { Rect::from_center_size(pos2(cx, y + BTN_H / 2.0), Vec2::new(*w, BTN_H)) };
            let resp = ui.interact(r, ui.id().with(("welcome-btn", *act)), Sense::click());
            let bg = if resp.is_pointer_button_down_on() { prs } else if resp.hovered() { hov } else { Color32::TRANSPARENT };
            ui.painter().rect_filled(r, BTN_H / 2.0, bg);
            ui.painter().rect_stroke(r, BTN_H / 2.0, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
            self.icons.paint(ui, icon, Rect::from_center_size(pos2(r.left() + 2.0 * SPACING + 8.0, r.center().y), Vec2::splat(16.0)), txt);
            ui.painter().text(pos2(r.left() + 2.0 * SPACING + 16.0 + 8.0, r.center().y), egui::Align2::LEFT_CENTER, label, font.clone(), txt);
            if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            if resp.clicked() {
                match act {
                    0 => if let Some(id) = self.selected.clone() { self.attach_pick(&id, now) }, // the composer's "Attach file"
                    1 => self.gif_picker_toggle(ui.ctx()),
                    _ => { // the in-app scheduler, the open chat's person invited
                        let sp = self.selected.as_ref().and_then(|id| self.spaces.iter().find(|s| &s.id == id)).cloned();
                        let inv: Vec<String> = sp.iter().flat_map(|s| s.participants.iter()).filter(|p| !p.email.is_empty() && !p.email.eq_ignore_ascii_case(&self.me.email)).map(|p| p.email.clone()).collect();
                        self.open_scheduler(&sp.map(|s| s.title).unwrap_or_default(), &inv);
                    }
                }
            }
            if row { x += w + 5.0; } else { y += BTN_H + SPACING; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn welcome_only_for_an_empty_direct_chat_with_its_person() {
        assert_eq!(welcome_variant(true, false, true, true), Some(Welcome::Direct)); // direct chat
        assert_eq!(welcome_variant(true, false, false, true), None); // nobody else left
        assert_eq!(welcome_variant(true, true, true, true), None); // closed
        assert_eq!(welcome_variant(true, false, true, false), None); // not loaded yet / has messages
        assert_eq!(welcome_variant(false, false, true, true), None); // groups: none
    }
    #[test]
    fn emoji_become_placeholders() {
        let (job, marks) = emoji_job("Hello 👋", FontId::proportional(16.0), Color32::WHITE, false, 100.0);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].0, 6);
        assert!(job.text.ends_with(super::super::emoji_text::PLACEHOLDER));
    }
}
