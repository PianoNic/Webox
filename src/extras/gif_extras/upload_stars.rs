//! Keeping uploaded pictures (the Giphy GIFs have their star in `stars.rs`):
//! - the star chip on uploaded GIFs in the chat (on hover) and the star in the image viewer's toolbar (any uploaded picture, once
//!   its original has loaded there);
//! - the message menu's "Add to favorites" / "Remove from favorites" on any picture (PNG, JPEG, WebP, GIF). It keeps the original
//!   file: when the original isn't loaded yet it is asked for and the picture is kept when it arrives (after `FULL_WAIT` without
//!   it, what the chat shows is kept instead).
//!
//! Keeping stores the decrypted bytes on this computer (`upload_favs`) and lists the picture in the plugin popup's "Favorites";
//! picking it there stages a copy as a normal attachment. Part of the GIF Extras plugin: only reached through the GIF picker's
//! hook while the plugin is on.
use super::{upload_favs, Plugin};
use crate::cloud::Attachment;
use crate::*;
use std::cell::RefCell;
use std::collections::HashMap;

/// How long a menu save waits for the original file before keeping what the chat shows.
const FULL_WAIT: f64 = 15.0;

thread_local! {
    /// Source ("full:<key>" or "<key>") -> content id of its bytes (hashed once; None = nothing we can keep).
    static IDS: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// A menu save waiting for the original file: the picture, when it was asked for, and whether this asked for the original
/// (then it is freed again afterwards).
#[derive(Clone)]
pub(super) struct Pending { a: Attachment, since: f64, asked: bool }

/// An uploaded (not Giphy) GIF attachment.
pub(super) fn uploaded_gif(a: &Attachment) -> bool { a.giphy.is_none() && (a.mime == "image/gif" || a.name.to_lowercase().ends_with(".gif")) }

/// An uploaded (not Giphy) picture of a kind that can be kept: GIF, PNG, JPEG, WebP.
pub(super) fn uploaded_picture(a: &Attachment) -> bool {
    if a.giphy.is_some() { return false; }
    let n = a.name.to_lowercase();
    matches!(a.mime.as_str(), "image/gif" | "image/png" | "image/jpeg" | "image/jpg" | "image/webp")
        || [".gif", ".png", ".jpg", ".jpeg", ".webp"].iter().any(|e| n.ends_with(e))
}

/// Where the kept pictures live: next to the settings when signed in; screenshot / sample runs use a temporary folder.
pub(super) fn upload_fav_dir(app: &App) -> PathBuf {
    if app.client.is_some() && app.shot.is_none() { system::config_dir().join(upload_favs::DIR) } else { std::env::temp_dir().join("webox-sample-gif-favorites") }
}

/// The bytes to keep and where they come from: the original file once it has loaded; else what the chat shows, for a GIF (the
/// GIF itself) or when `shown_ok` (a 512 px thumbnail for other pictures).
fn keep_source<'a>(app: &'a App, a: &Attachment, shown_ok: bool) -> Option<(String, &'a [u8])> {
    let full = format!("full:{}", a.key);
    if let Some(b) = app.photos.image_bytes(&full) { return Some((full, b)); }
    let b = app.photos.image_bytes(&a.key)?;
    let gif = uploaded_gif(a) && image::guess_format(b).ok() == Some(image::ImageFormat::Gif);
    (gif || shown_ok).then(|| (a.key.clone(), b))
}

/// The content id the picture would be kept under, once its bytes are here.
fn picture_id(app: &App, a: &Attachment) -> Option<String> {
    if !uploaded_picture(a) { return None; }
    let (src, bytes) = keep_source(app, a, false)?; // not loaded yet: ask again later
    if let Some(v) = IDS.with(|m| m.borrow().get(&src).cloned()) { return v; }
    let id = (bytes.len() <= upload_favs::MAX_BYTES && upload_favs::picture_ext(bytes).is_some()).then(|| upload_favs::content_id(bytes));
    IDS.with(|m| m.borrow_mut().insert(src, id.clone()));
    id
}

fn tip(fav: bool) -> std::borrow::Cow<'static, str> { if fav { crate::tr!("AppPickerGridViewDelegate", "Remove from favorites") } else { crate::trq!("Add to favorites") } }

impl Plugin {
    /// Some(starred?) for an uploaded picture whose bytes are here and can be kept, None otherwise.
    fn upload_fav_state(&mut self, app: &App, a: &Attachment) -> Option<bool> {
        let id = picture_id(app, a)?;
        Some(self.favs(app).get(&id).is_some() || self.favs(app).by_src(&upload_favs::src_id(&a.key)).is_some())
    }

    /// The id of the kept picture `a` is (by content once its bytes are here, else by the attachment it was saved from).
    fn saved_id(&mut self, app: &App, a: &Attachment) -> Option<String> {
        if let Some(id) = picture_id(app, a).filter(|id| self.favs(app).get(id).is_some()) { return Some(id); }
        self.favs(app).by_src(&upload_favs::src_id(&a.key)).map(|f| f.id.clone())
    }

    /// Removes a kept picture and its files.
    fn remove_upload(&mut self, app: &App, id: &str) {
        let dir = upload_fav_dir(app);
        if let Some(f) = self.favs(app).remove(id) { upload_favs::unstar(&dir, &f); }
    }

    /// Keeps picture `a` (the original when loaded, see `keep_source`); the same picture again is the same entry, moved to the front.
    pub(super) fn keep_picture(&mut self, app: &mut App, a: &Attachment, shown_ok: bool, now: f64) {
        let Some(bytes) = keep_source(app, a, shown_ok).map(|(_, b)| b.to_vec()) else { return };
        let dir = upload_fav_dir(app);
        let id = upload_favs::content_id(&bytes);
        let used = upload_favs::used_bytes(self.favs(app).uploads(), &id);
        match upload_favs::star(&dir, &a.name, &bytes, used) {
            Ok(mut f) => {
                f.src = upload_favs::src_id(&a.key);
                for d in self.favs(app).insert(f) { if upload_favs::is_upload(&d) { upload_favs::unstar(&dir, &d); } }
            }
            Err(e) => app.toast(format!("{}: {e}", crate::trq!("Add to favorites")), now),
        }
    }

    /// Star / unstar an uploaded picture whose bytes are here.
    fn upload_fav_toggle(&mut self, app: &mut App, a: &Attachment, now: f64) {
        if let Some(id) = self.saved_id(app, a) { self.remove_upload(app, &id); return; }
        self.keep_picture(app, a, false, now);
    }

    /// The message menu on picture `a`: Some(saved?) for a picture that can be kept (or a Giphy GIF), None for anything else.
    pub(super) fn picture_menu_state(&mut self, app: &mut App, ctx: &egui::Context, a: &Attachment) -> Option<bool> {
        if let Some(g) = super::stars::giphy_to_gif(a) { return Some(self.favs(app).contains(&g.id, false)); }
        if !uploaded_picture(a) { return None; }
        self.poll_pending(app, ctx);
        Some(self.saved_id(app, a).is_some() || self.pending.as_ref().is_some_and(|p| p.a.key == a.key))
    }

    /// "Add to favorites" / "Remove from favorites" picked in the message menu on picture `a`.
    pub(super) fn picture_menu_pick(&mut self, app: &mut App, ctx: &egui::Context, a: &Attachment) {
        let now = ctx.input(|i| i.time);
        if let Some(g) = super::stars::giphy_to_gif(a) { self.favs(app).toggle(&g, false); return; }
        if !uploaded_picture(a) { return; }
        if self.pending.as_ref().is_some_and(|p| p.a.key == a.key) { self.pending = None; return; } // removed before it was kept
        if let Some(id) = self.saved_id(app, a) { self.remove_upload(app, &id); return; }
        let full = app.photos.image_bytes(&format!("full:{}", a.key)).is_some();
        if full || app.client.is_none() { self.keep_picture(app, a, true, now); return; } // sample runs: only what the chat shows exists
        // The original first (the chat shows a 512 px thumbnail): kept once it arrives (`poll_pending`).
        let asked = !app.photos.full_requested(&a.key);
        app.request_full_image(&a.key);
        self.pending = Some(Pending { a: a.clone(), since: now, asked });
        ctx.request_repaint();
    }

    /// Finishes a waiting menu save once the original has arrived (or after `FULL_WAIT` with what the chat shows), then frees the
    /// original again when only this asked for it and the viewer doesn't show it. Called from the plugin's hooks every frame.
    pub(super) fn poll_pending(&mut self, app: &mut App, ctx: &egui::Context) {
        let Some(p) = self.pending.clone() else { return };
        let now = ctx.input(|i| i.time);
        let full = app.photos.image_bytes(&format!("full:{}", p.a.key)).is_some();
        if !full && now - p.since < FULL_WAIT { ctx.request_repaint_after(std::time::Duration::from_millis(500)); return; }
        self.pending = None;
        self.keep_picture(app, &p.a, true, now);
        let viewing = ctx.data(|d| d.get_temp::<String>(egui::Id::new("lightbox"))).as_deref() == Some(p.a.key.as_str());
        if p.asked && !viewing { app.release_full_image(&p.a.key); }
    }

    /// The round star chip at the picture's top right while the pointer is over it (the same chip the Giphy GIFs get). GIFs only.
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

    /// The viewer's star for an uploaded picture (once its bytes are here; the viewer loads the original); whether it was drawn.
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
    fn a(mime: &str, name: &str, giphy: bool) -> Attachment { Attachment { name: name.into(), mime: mime.into(), giphy: giphy.then(Default::default), ..Default::default() } }

    #[test]
    fn only_uploaded_gifs_get_the_star() {
        assert!(uploaded_gif(&a("image/gif", "x.gif", false)));
        assert!(uploaded_gif(&a("", "Party.GIF", false)));
        assert!(!uploaded_gif(&a("image/gif", "x", true)), "Giphy shares have their own star");
        assert!(!uploaded_gif(&a("image/png", "x.png", false)));
    }

    #[test]
    fn every_uploaded_picture_can_be_kept() {
        for (m, n) in [("image/png", "x.png"), ("image/jpeg", "x"), ("", "Photo.JPG"), ("image/webp", "x.webp"), ("image/gif", "x.gif"), ("", "a.jpeg")] {
            assert!(uploaded_picture(&a(m, n, false)), "{m} {n}");
        }
        assert!(!uploaded_picture(&a("image/gif", "x.gif", true)), "a Giphy share is a Giphy favourite");
        assert!(!uploaded_picture(&a("image/svg+xml", "x.svg", false)));
        assert!(!uploaded_picture(&a("application/pdf", "x.pdf", false)));
    }
}
