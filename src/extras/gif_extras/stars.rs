//! The plugin's star chips: on a GIF shared from Giphy in the message list (top right of the picture while the pointer is over it)
//! and as a button in the image viewer's top-right row. Only reached through the GIF picker's hook while the plugin is on.
use super::Plugin;
use crate::extras::giphy::Gif;
use crate::*;

/// The picker's `Gif` for a GIF that was shared from Giphy (the favourites store keeps these): id and CDN address from the share, the
/// title is the file name without ".gif", the thumbnail is the CDN's 200 px still next to the media file (`200w_d.gif`, the name the
/// picker's own thumbnails have), sizes from the attachment. `None` for every other image.
pub(crate) fn giphy_to_gif(a: &cloud::Attachment) -> Option<Gif> {
    let g = a.giphy.as_ref().filter(|g| !g.id.is_empty() && g.url.starts_with("https://"))?;
    let bare = g.url.split(['?', '#']).next().unwrap_or("");
    let dir = bare.rsplit_once('/')?.0;
    let t = g.title.trim();
    let title = if t.len() > 4 && t[t.len() - 4..].eq_ignore_ascii_case(".gif") { &t[..t.len() - 4] } else { t };
    let (w, h) = (a.width.max(1), a.height.max(1));
    Some(Gif { id: g.id.clone(), title: title.to_string(), thumb_url: format!("{dir}/200w_d.gif"), thumb_w: 200, thumb_h: (200 * h / w).max(1), media_url: g.url.clone(), media_w: w, media_h: h })
}

fn tip(fav: bool) -> std::borrow::Cow<'static, str> { if fav { crate::tr!("AppPickerGridViewDelegate", "Remove from favorites") } else { crate::trq!("Add to favorites") } }

impl Plugin {
    /// The round star chip on a Giphy GIF in the chat (28 pt, 8 pt from the picture's top right corner).
    pub(super) fn star_chip(&mut self, app: &mut App, ui: &mut egui::Ui, r: Rect, a: &cloud::Attachment, salt: (&str, usize), ready: bool) {
        let Some(g) = giphy_to_gif(a) else { return self.upload_star_chip(app, ui, r, a, ready, salt) }; // an uploaded GIF: upload_stars.rs
        let forced = app.shot.is_some() && std::env::var("WEBOX_SHOT_HOVER_IMG").is_ok(); // screenshot checks only
        if !((crate::ui::widgets::hover_ok(ui, r) && ready) || forced) { return }
        let fav = self.favs(app).contains(&g.id, false);
        let cr = Rect::from_center_size(pos2(r.right() - 8.0 - 14.0, r.top() + 8.0 + 14.0), Vec2::splat(28.0));
        let seq = crate::ui::messages::cur_seq(ui.ctx());
        let sr = ui.interact(cr, egui::Id::new(("gif-fav", salt.0, salt.1, seq)), Sense::click());
        ui.painter().circle_filled(cr.center(), 14.0, Color32::from_black_alpha(if sr.hovered() { 215 } else { 150 }));
        app.icons.paint(ui, if fav { "favorite-filled" } else { "favorite-regular" }, Rect::from_center_size(cr.center(), Vec2::splat(16.0)), if fav { hex(0xF5B800) } else { Color32::WHITE });
        if sr.on_hover_text(tip(fav)).clicked() { self.favs(app).toggle(&g, false); }
    }

    /// The star button in the image viewer (same 36 pt round button as the viewer's own); whether it was drawn.
    pub(super) fn viewer_star(&mut self, app: &mut App, ui: &mut egui::Ui, cell: Rect, a: &cloud::Attachment) -> bool {
        let Some(g) = giphy_to_gif(a) else { return self.upload_viewer_star(app, ui, cell, a) };
        let fav = self.favs(app).contains(&g.id, false);
        let rs = ui.interact(cell, egui::Id::new(("lb-btn", "gif-fav", cell.min.x as i32)), Sense::click());
        if rs.hovered() { ui.painter().circle_filled(cell.center(), 16.0, Color32::from_white_alpha(28)); }
        app.icons.paint(ui, if fav { "favorite-filled" } else { "favorite-regular" }, Rect::from_center_size(cell.center(), Vec2::splat(18.0)), Color32::WHITE);
        if rs.on_hover_text(tip(fav)).clicked() { self.favs(app).toggle(&g, false); }
        true
    }
}
