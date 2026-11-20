//! Star chip for uploaded GIFs (the Giphy ones have theirs in `stars.rs`): in the chat on hover and in the image viewer's toolbar.
//! Starring keeps the GIF's decrypted bytes on this computer (`upload_favs`) and lists it in the plugin popup's "Favoriten"; picking
//! it there stages a copy as a normal attachment. Part of the GIF-Extras plugin: only reached through the GIF picker's hook while
//! the plugin is on.
use super::{upload_favs, Plugin};
use crate::cloud::Attachment;
use crate::*;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Attachment key -> content id of its GIF (hashed once; None = not a GIF we can keep).
    static IDS: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// An uploaded (not Giphy) GIF attachment.
pub(super) fn uploaded_gif(a: &Attachment) -> bool { a.giphy.is_none() && (a.mime == "image/gif" || a.name.to_lowercase().ends_with(".gif")) }

/// Where the kept GIFs live: next to the settings when signed in; screenshot / sample runs use a temporary folder.
pub(super) fn upload_fav_dir(app: &App) -> PathBuf {
    if app.client.is_some() && app.shot.is_none() { system::config_dir().join(upload_favs::DIR) } else { std::env::temp_dir().join("webox-sample-gif-favorites") }
}

/// The decrypted GIF bytes of an attachment (the original file when it has been loaded, else what the chat shows).
fn upload_gif_bytes(app: &App, a: &Attachment) -> Option<Vec<u8>> {
    let b = app.photos.image_bytes(&format!("full:{}", a.key)).or_else(|| app.photos.image_bytes(&a.key))?;
    (image::guess_format(b).ok() == Some(image::ImageFormat::Gif)).then(|| b.to_vec())
}

fn upload_gif_id(app: &App, a: &Attachment) -> Option<String> {
    if !uploaded_gif(a) { return None; }
    if let Some(v) = IDS.with(|m| m.borrow().get(&a.key).cloned()) { return v; }
    let bytes = upload_gif_bytes(app, a)?; // not loaded yet: ask again later
    let id = (bytes.len() <= upload_favs::MAX_BYTES).then(|| upload_favs::content_id(&bytes));
    IDS.with(|m| m.borrow_mut().insert(a.key.clone(), id.clone()));
    id
}

fn tip(fav: bool) -> &'static str { if fav { "Aus Favoriten entfernen" } else { "Zu Favoriten hinzufügen" } }

impl Plugin {
    /// Some(starred?) for an uploaded GIF that can be kept, None otherwise.
    fn upload_fav_state(&mut self, app: &App, a: &Attachment) -> Option<bool> {
        let id = upload_gif_id(app, a)?;
        Some(self.favs(app).get(&id).is_some())
    }

    /// Star / unstar an uploaded GIF.
    fn upload_fav_toggle(&mut self, app: &mut App, a: &Attachment, now: f64) {
        let Some(id) = upload_gif_id(app, a) else { return };
        let dir = upload_fav_dir(app);
        if let Some(f) = self.favs(app).remove(&id) { upload_favs::unstar(&dir, &f); return; }
        let Some(bytes) = upload_gif_bytes(app, a) else { return };
        match upload_favs::star(&dir, &a.name, &bytes) {
            Ok(f) => self.favs(app).insert(f),
            Err(e) => app.toast(format!("Zu Favoriten hinzufügen: {e}"), now),
        }
    }

    /// The round star chip at the picture's top right while the pointer is over it (the same chip the Giphy GIFs get).
    pub(super) fn upload_star_chip(&mut self, app: &mut App, ui: &mut egui::Ui, r: Rect, a: &Attachment, ready: bool, salt: (&str, usize)) {
        if !uploaded_gif(a) { return; }
        let forced = app.shot.is_some() && std::env::var("WEBOX_SHOT_HOVER_IMG").is_ok(); // screenshot checks only
        if !((crate::ui::widgets::hover_ok(ui, r) && ready) || forced) { return; }
        let Some(fav) = self.upload_fav_state(app, a) else { return };
        let cr = Rect::from_center_size(pos2(r.right() - 8.0 - 14.0, r.top() + 8.0 + 14.0), Vec2::splat(28.0));
        let sr = ui.interact(cr, egui::Id::new(("upload-gif-fav", salt.0, salt.1)), Sense::click());
        ui.painter().circle_filled(cr.center(), 14.0, Color32::from_black_alpha(if sr.hovered() { 215 } else { 150 }));
        app.icons.paint(ui, if fav { "favorite-filled" } else { "favorite-regular" }, Rect::from_center_size(cr.center(), Vec2::splat(16.0)), if fav { hex(0xF5B800) } else { Color32::WHITE });
        if sr.on_hover_text(tip(fav)).clicked() {
            let now = ui.input(|i| i.time);
            self.upload_fav_toggle(app, a, now);
        }
    }

    /// The viewer's star for an uploaded GIF; whether it was drawn.
    pub(super) fn upload_viewer_star(&mut self, app: &mut App, ui: &mut egui::Ui, cell: Rect, a: &Attachment) -> bool {
        let Some(fav) = self.upload_fav_state(app, a) else { return false };
        let rs = ui.interact(cell, egui::Id::new(("lb-btn", "upload-gif-fav", cell.min.x as i32)), Sense::click());
        if rs.hovered() { ui.painter().circle_filled(cell.center(), 16.0, Color32::from_white_alpha(28)); }
        app.icons.paint(ui, if fav { "favorite-filled" } else { "favorite-regular" }, Rect::from_center_size(cell.center(), Vec2::splat(18.0)), Color32::WHITE);
        if rs.on_hover_text(tip(fav)).clicked() { let now = ui.input(|i| i.time); self.upload_fav_toggle(app, a, now); }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_uploaded_gifs_get_the_star() {
        let a = |mime: &str, name: &str, giphy: bool| Attachment { name: name.into(), mime: mime.into(), giphy: giphy.then(Default::default), ..Default::default() };
        assert!(uploaded_gif(&a("image/gif", "x.gif", false)));
        assert!(uploaded_gif(&a("", "Party.GIF", false)));
        assert!(!uploaded_gif(&a("image/gif", "x", true)), "Giphy shares have their own star");
        assert!(!uploaded_gif(&a("image/png", "x.png", false)));
    }
}
