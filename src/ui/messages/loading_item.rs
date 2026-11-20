//! The message list's loading item: a row 50 pt high with the animated "Loading.gif" centred in it, scaled to 40 pt wide
//! (the gif is 144 x 90 px with 6 frames of 700 ms, so 40 x 25), or - when loading failed - a "Wiederholen" pill that asks
//! again. The list is bottom-to-top, so a list holding only this row shows it at the bottom, above the list's bottom margin.
//! No delay before it appears. The same row is shown at the top while older messages load. Without the gif
//! (assets/cisco/images/Loading.gif missing) three pulsing dots of the same size stand in.
use super::*;

/// Height of the loading row (pt).
pub(super) const LOADING_ROW_H: f32 = 50.0;
/// The gif's width (pt), height from its aspect (144 x 90).
const GIF_W: f32 = 40.0;
const GIF_H: f32 = 25.0;
const GIF_KEY: &str = "cisco:Loading.gif";

/// What the message area shows for a space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListLoad {
    /// Messages asked for, nothing arrived yet: the loading item.
    Loading,
    /// The first page could not be loaded: the retry item.
    Failed,
    /// Loaded, no messages: the welcome view (ui/welcome.rs) where there is one, else an empty list.
    LoadedEmpty,
    Loaded,
}

/// `loaded` = the number of messages once they arrived (None before), `failed` = the request for them failed.
pub(crate) fn list_load(loaded: Option<usize>, failed: bool) -> ListLoad {
    match loaded {
        Some(0) => ListLoad::LoadedEmpty,
        Some(_) => ListLoad::Loaded,
        None if failed => ListLoad::Failed,
        None => ListLoad::Loading,
    }
}

impl App {
    /// The load state of a space's message list (see `list_load`). Screenshot checks: WEBOX_SHOT_MSGLOAD=loading|failed.
    pub(crate) fn list_load_state(&self, id: &str) -> ListLoad {
        if self.shot.is_some() {
            match std::env::var("WEBOX_SHOT_MSGLOAD").as_deref() { Ok("loading") => return ListLoad::Loading, Ok("failed") => return ListLoad::Failed, _ => {} }
        }
        let failed = self.photos.history.get(id).is_some_and(|h| h.first_failed);
        list_load(self.messages.get(id).map(|m| m.len()), failed)
    }

    /// The message area while the first page loads or after it failed: one loading item at the bottom of the list.
    pub(crate) fn list_loading_view(&mut self, ui: &mut egui::Ui, id: &str, state: ListLoad) {
        let area = ui.max_rect();
        let row = Rect::from_min_max(pos2(area.left(), area.bottom() - list_bottom() - LOADING_ROW_H), pos2(area.right(), area.bottom() - list_bottom()));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(row));
        if self.loading_item(&mut child, state == ListLoad::Failed) {
            // Retry: ask again.
            if let Some(h) = self.photos.history.get_mut(id) { h.first_failed = false; }
            if let Some(c) = &self.client { c.req(crate::cloud::Req::Messages(id.to_string())); }
        }
    }

    /// One loading row in `ui` (full width, LOADING_ROW_H high). Returns true when Retry was clicked.
    pub(super) fn loading_item(&mut self, ui: &mut egui::Ui, retry: bool) -> bool {
        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), LOADING_ROW_H), Sense::hover());
        let ctx = ui.ctx().clone();
        if retry {
            let c = mc_for(ui);
            let label = crate::tr!("UTLoadingItemDelegate", "Retry").into_owned();
            let g = ui.painter().layout_no_wrap(label, FontId::new(14.0, egui::FontFamily::Name("semibold".into())), c.0);
            let pill = Rect::from_center_size(r.center(), Vec2::new(g.size().x + 24.0, 28.0));
            let resp = ui.interact(pill, Id::new("msg-loading-retry"), Sense::click());
            if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); ui.painter().rect_filled(pill, 14.0, c.2); }
            ui.painter().rect_stroke(pill, 14.0, Stroke::new(1.0, c.1), StrokeKind::Inside);
            ui.painter().galley(pill.center() - g.size() / 2.0, g, c.0);
            return resp.clicked();
        }
        let gif = Rect::from_center_size(r.center(), Vec2::new(GIF_W, GIF_H));
        let now = ctx.input(|i| i.time);
        if self.photos.image_bytes(GIF_KEY).is_none() {
            if let Some(b) = crate::assets::cisco_assets::get("images/Loading.gif") { self.photos.add_images(vec![(GIF_KEY.to_string(), b.to_vec())]); }
        }
        match self.photos.image_frame(&ctx, GIF_KEY, now) {
            Some((tex, _)) => { ui.painter().image(tex, gif, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
            None => {
                // Fallback: three dots.
                for k in 0..3 {
                    let a = (((now * 1.4) as f32 - k as f32 / 3.0).rem_euclid(1.0) * 0.8 + 0.2).clamp(0.2, 1.0);
                    ui.painter().circle_filled(pos2(gif.center().x + (k as f32 - 1.0) * 12.0, gif.center().y), 4.0, wa(mc_for(ui).0, a));
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        false
    }
}

/// (text, border, hover fill) for the retry pill from the theme of `ui` (button text primary / outline, as the other
/// outline pills of the list).
fn mc_for(ui: &egui::Ui) -> (Color32, Color32, Color32) {
    let dark = ui.visuals().dark_mode;
    let f = if dark { Color32::WHITE } else { Color32::BLACK };
    (wa(f, 0.95), wa(f, 0.5), wa(f, 0.07))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_states() {
        assert_eq!(list_load(None, false), ListLoad::Loading);
        assert_eq!(list_load(None, true), ListLoad::Failed);
        assert_eq!(list_load(Some(0), false), ListLoad::LoadedEmpty); // finished and empty: welcome view, not the indicator
        assert_eq!(list_load(Some(0), true), ListLoad::LoadedEmpty); // a later failure doesn't hide what arrived
        assert_eq!(list_load(Some(5), false), ListLoad::Loaded);
        assert_eq!(list_load(Some(5), true), ListLoad::Loaded);
    }
}
