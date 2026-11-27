//! "GIF Extras": a built-in, first-party Webox plugin. Listed on Settings > Plugins with a "Built-in" badge, off by default,
//! the switch saved in plugins.json and read every frame (live). While it is on:
//! - the GIF button opens the extended popup (`picker.rs`): tabs "GIFs | Stickers | Emojis", a home screen of tiles ("Favorites",
//!   "Trending GIFs" / "Trending stickers", Giphy's categories), stickers, favourites with a star on every tile; the setting
//!   "Compact size" (off by default) gives it the smaller 350 x 390 size of earlier versions (`picker/layout.rs`);
//! - Giphy GIFs in the chat and in the image viewer get a star chip that saves them to the favourites (`stars.rs`); uploaded GIFs get the
//!   same star, which keeps the GIF on this computer (`upload_stars.rs`, `upload_favs.rs`: gif-favorites/, entries of kind "upload");
//! - the message menu on any picture (PNG, JPEG, WebP, GIF) gets "Add to favorites" / "Remove from favorites": it keeps the
//!   original file the same way, and the picker's "Favorites" sends it again as an image attachment. The image viewer's star
//!   works for every uploaded picture too; the chat's star chip stays on GIFs.
//!
//! While it is off none of this runs: the picker is the standard GIF picker (src/ui/gif_picker), no star appears,
//! and the favourites (`favs.rs`, giphy_favorites.json, and the kept uploaded GIFs in gif-favorites/) are neither read nor written -
//! they stay on disk as they were.
//!
//! The core code reaches the plugin only through `crate::ui::gif_picker::ext::GifExtras` (registered by `register()`).
//! In-tree because dynamic plugins cannot draw egui UI safely across the C ABI.
use crate::ui::gif_picker::ext::{self, GifExtras, PopupOut};
use crate::*;

pub mod favs;
mod picker;
mod stars;
pub mod upload_favs;
mod upload_stars;
#[cfg(test)]
mod tests;


/// Key in the plugin list / plugins.json.
pub const KEY: &str = "builtin.gif-extras";
pub const NAME: &str = "GIF Extras";
pub const VERSION: &str = "1.0.0";
pub const DESCRIPTION: &str = "Stickers, categories and favorites for the GIF picker. Star a GIF in the chat, or save any picture from its menu, to keep it.";

/// The plugin object: the extended popup's state and the favourites (loaded on first use).
#[derive(Default)]
pub(crate) struct Plugin {
    picker: picker::State,
    favs: Option<favs::Favs>,
    /// A picture saved from the message menu, waiting for its original file.
    pending: Option<upload_stars::Pending>,
}

impl Plugin {
    /// The favourites, loaded on first use: next to the settings in live runs, in memory only in sample / screenshot runs.
    fn favs(&mut self, app: &App) -> &mut favs::Favs {
        self.favs.get_or_insert_with(|| favs::Favs::load((app.client.is_some() && app.shot.is_none()).then(system::config_dir)))
    }
}

/// Registers the plugin with the GIF picker's hook (App::new).
pub(crate) fn register() { ext::register(KEY, Box::new(Plugin::default())); }

/// Its settings, in the same schema format a native plugin returns (plugin-api's builder), drawn on its own page under
/// Settings > Plugins like any other plugin's. Read live with `app.plugins.setting_bool(KEY, ..)`.
pub fn settings_schema() -> String {
    collabhost_plugin_api::Schema::new()
        .section(&crate::trq!("GIF picker"), "")
        .toggle(COMPACT, &crate::trq!("Compact size"), &crate::trq!("The smaller GIF picker (350 × 390) of earlier Webox versions."), false)
        .toggle(SEND_INSTANTLY, &crate::trq!("Send favorites on click"), &crate::trq!("A picked GIF or saved picture is sent at once instead of being added to the message."), false)
        .section(&crate::trq!("Favorite stars"), &crate::trq!("A star saves a GIF to the favorites of the extended GIF picker."))
        .toggle("stars_chat", &crate::trq!("Star on GIFs in the chat"), &crate::trq!("Appears in the top right corner of a GIF while the mouse pointer is over it."), true)
        .toggle("stars_viewer", &crate::trq!("Star in the image viewer"), &crate::trq!("A star button next to the image viewer's buttons."), true)
        .toggle("stars_uploads", &crate::trq!("Uploads too"), &crate::trq!("Starred uploads are kept on this computer (folder gif-favorites): GIFs in the chat, any picture in the image viewer."), true)
        .section(&crate::trq!("Storage location"), "")
        .button("open_folder", &crate::trq!("Open favorites folder"), &crate::trq!("The folder with the kept uploaded GIFs and pictures."))
        .to_json()
}

/// A settings button of this plugin (the page calls this for built-in plugins).
pub(crate) fn settings_action(app: &mut App, id: &str) -> Option<String> {
    match id {
        "open_folder" => { let d = upload_stars::upload_fav_dir(app); let _ = std::fs::create_dir_all(&d); system::open_folder(&d); None }
        _ => None,
    }
}

fn on(app: &App, id: &str) -> bool { app.plugins.setting_bool(KEY, id, true) }

/// Setting id of "Send favorites on click" (default off): a picked GIF or kept picture goes out at once instead of waiting in the
/// composer.
pub const SEND_INSTANTLY: &str = "send_instantly";
/// Whether a picked GIF is sent at once: only while the plugin is on and its setting is set.
pub(crate) fn send_instantly(app: &App) -> bool { ext::active(&app.plugins, None) && app.plugins.setting_bool(KEY, SEND_INSTANTLY, false) }

/// Setting id of the compact popup size (default off).
pub const COMPACT: &str = "compact";
/// Whether the popup uses the compact size: the setting, or `shot` (`WEBOX_SHOT_GIF_COMPACT=0|1` in screenshot runs) when given.
pub(crate) fn compact(plugins: &crate::extras::plugins::Plugins, shot: Option<&str>) -> bool {
    match shot { Some(v) => v == "1", None => plugins.setting_bool(KEY, COMPACT, false) }
}
fn compact_now(app: &App) -> bool {
    let shot = if app.shot.is_some() { std::env::var("WEBOX_SHOT_GIF_COMPACT").ok() } else { None };
    compact(&app.plugins, shot.as_deref())
}

impl GifExtras for Plugin {
    fn open(&mut self, app: &mut App, ctx: &egui::Context) { self.picker_open(app, ctx); }
    fn popup(&mut self, app: &mut App, ui: &mut egui::Ui, t: &Th, rect: Rect, anchor: Rect, space: &str) -> PopupOut { let c = compact_now(app); self.picker_draw(app, ui, t, rect, anchor, space, c) }
    fn image_overlay(&mut self, app: &mut App, ui: &mut egui::Ui, r: Rect, a: &cloud::Attachment, salt: (&str, usize), ready: bool) {
        self.poll_pending(app, ui.ctx());
        if on(app, "stars_chat") && (a.giphy.is_some() || on(app, "stars_uploads")) { self.star_chip(app, ui, r, a, salt, ready); }
    }
    fn viewer_button(&mut self, app: &mut App, ui: &mut egui::Ui, cell: Rect, a: &cloud::Attachment) -> bool {
        self.poll_pending(app, ui.ctx());
        on(app, "stars_viewer") && (a.giphy.is_some() || on(app, "stars_uploads")) && self.viewer_star(app, ui, cell, a)
    }
    fn image_menu(&mut self, app: &mut App, ctx: &egui::Context, a: &cloud::Attachment) -> Option<bool> { self.picture_menu_state(app, ctx, a) }
    fn image_menu_pick(&mut self, app: &mut App, ctx: &egui::Context, a: &cloud::Attachment) { self.picture_menu_pick(app, ctx, a) }
}
