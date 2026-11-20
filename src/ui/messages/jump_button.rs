//! "Scroll to the latest messages" button: a round XS button with "arrow-down_12", tooltip "Klicken, um zu den neuesten
//! Nachrichten zu scrollen", centred horizontally over the message list, its bottom 20 pt above the list's bottom. Shown
//! while the list is not at its newest end, opacity animated over 300 ms. A click jumps (no animation) to the newest end.
//! The same request comes from the shortcut "Zum Ende springen" (Shift+PgDown).
//! Colours: button-message-fill-background normal #1170CF / hovered #0353A8 / pressed #063A75, icon button-message-fill-text
//! #FFFFFF @ 0.95 (both themes); 24 pt diameter. No new-message count or text on it: the "Neue Nachrichten" /
//! "Neue Antworten" banners are separate buttons above it.
//! It is drawn on the list's own layer, so the wheel over it still scrolls the list.
use super::*;

const DIAMETER: f32 = 24.0;
const BOTTOM_MARGIN: f32 = 20.0;
const FADE_S: f32 = 0.3;
/// Request key: the list jumps to its newest end and pins there (button, shortcut).
pub(crate) const JUMP_KEY: &str = "msg-jump-bottom";

/// Shown while the list can scroll and is not at its end.
pub(crate) fn jump_button_visible(off: f32, max: f32) -> bool { max > 1.0 && off < max - 1.0 }

/// Asks the open message list to jump to its newest message (shortcut "Zum Ende springen").
pub(crate) fn request_jump_to_bottom(ctx: &egui::Context) { ctx.data_mut(|d| d.insert_temp(Id::new(JUMP_KEY), true)); }

impl App {
    /// Draws the button over `list` (the list's own layer); a click asks for the jump.
    pub(super) fn jump_button(&mut self, ui: &mut egui::Ui, list: Rect, off: f32, max: f32) {
        let ctx = ui.ctx().clone();
        let a = ctx.animate_bool_with_time(Id::new("msg-jump-fade"), jump_button_visible(off, max), FADE_S);
        if a <= 0.0 { return; }
        let r = Rect::from_center_size(pos2(list.center().x, list.bottom() - BOTTOM_MARGIN - DIAMETER / 2.0), Vec2::splat(DIAMETER));
        let resp = ui.interact(r, Id::new("msg-jump-bottom-btn"), Sense::click());
        let bg = if resp.is_pointer_button_down_on() { hex(0x063A75) } else if resp.hovered() { hex(0x0353A8) } else { hex(0x1170CF) };
        ui.painter().circle_filled(r.center(), DIAMETER / 2.0, bg.gamma_multiply(a));
        self.icons.paint(ui, "arrow-down_12", Rect::from_center_size(r.center(), Vec2::splat(12.0)), wa(Color32::WHITE, 0.95 * a));
        if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
        if resp.on_hover_text(crate::tr!("MessagesView", "Click to scroll to the latest messages").into_owned()).clicked() { request_jump_to_bottom(&ctx); ctx.request_repaint(); }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shown_unless_at_the_newest_end() {
        use super::jump_button_visible as v;
        assert!(!v(1828.0, 1828.0)); // at the end
        assert!(!v(1827.5, 1828.0)); // within a pixel of it
        assert!(v(1700.0, 1828.0)); // scrolled up
        assert!(v(0.0, 1828.0));
        assert!(!v(0.0, 0.0)); // nothing to scroll
    }
}
