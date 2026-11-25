//! Editing a message in a normal space: an overlay over the conversation, painted all over with `wx-overlay-background`
//! (#000000 @ 0.75 in both themes). At its top, full width, a holder with the `gradient-secondary` background, as high as
//! the edit widget needs: a 56 pt header "Edit message" (centred, `text-primary`, 14 px semibold) and below it a
//! second input box in edit mode whose send button reads "Update". Cancel and a sent edit (announced as
//! "Message edited") hide it. The space's own input box stays where it is, under the dark overlay, with its own draft.
//! The overlay covers the area below the tab bar.
use crate::*;

/// Height of the "Edit message" header (pt).
const TITLE_H: f32 = 56.0;

impl App {
    /// Draws the edit overlay over `area` (message list + composer); `own_h` = the height of the space's own composer at the
    /// bottom, which stays visible (and inactive) under the dark overlay.
    pub(crate) fn edit_overlay(&mut self, ui: &mut egui::Ui, t: &Th, space: &Space, area: Rect, own_h: f32, now: f64) {
        let ctx = ui.ctx().clone();
        // The space's own input box with its own draft, under the overlay (drawn, not interactive: the overlay takes the input).
        let own = Rect::from_min_max(pos2(area.left(), area.bottom() - own_h), area.max);
        ui.painter().rect_filled(own, 0.0, t.composer);
        let draft = self.drafts.get(&crate::app::actions::edit_stash_key(&space.id)).cloned().unwrap_or_default();
        let (txt, col) = match draft.lines().find(|l| !l.trim().is_empty()) { Some(l) => (l.to_string(), t.text1), None => (crate::trq!("Write a message to {{0}}", &space.title), t.text2) };
        ui.painter().text(pos2(own.left() + 20.0, own.bottom() - 34.0), egui::Align2::LEFT_CENTER, txt, FontId::proportional(16.0), col);
        // wx-overlay-background over everything; it takes the clicks and the wheel.
        egui::Area::new(egui::Id::new("edit-overlay-scrim")).order(egui::Order::Middle).fixed_pos(area.min).interactable(true).show(&ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(191)); // #000000 @ 0.75
        });
        // The holder at the top: title + the input box in edit mode.
        let edit_h = self.composer_height(&ctx);
        let panel = Rect::from_min_size(area.min, Vec2::new(area.width(), (TITLE_H + edit_h).min(area.height())));
        let shown = egui::Area::new(egui::Id::new("edit-overlay-panel")).order(egui::Order::Middle).fixed_pos(panel.min).interactable(true).show(&ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(panel.size(), Sense::hover());
            super::widgets::vgrad(ui.painter(), r, 0.0, t.card, t.card_b); // gradient-secondary
            let title = Rect::from_min_size(r.min, Vec2::new(r.width(), TITLE_H));
            ui.painter().text(title.center(), egui::Align2::CENTER_CENTER, crate::ui::widgets::tr("Edit message"), super::widgets::semibold(14.0), t.text1); // text-primary
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(r.left(), title.bottom()), r.max)));
            self.composer(&mut child, t, space, now);
        });
        ctx.move_to_top(shown.response.layer_id); // above the scrim
    }
}
