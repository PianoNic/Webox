//! "GIF-Extras": a built-in, first-party Webox plugin. Listed on Settings > Plugins with a "Built-in" badge, off by default,
//! the switch saved in plugins.json and read every frame (live). While it is on:
//! - the GIF button opens the extended popup (`picker.rs`): tabs "GIFs | Sticker | Emojis", a home screen of tiles ("Favoriten",
//!   "Angesagte GIFs" / "Angesagte Sticker", Giphy's categories), stickers, favourites with a star on every tile; the setting
//!   "Kompakte Größe" (off by default) gives it the smaller 350 x 390 size of earlier versions (`picker/layout.rs`);
//! - Giphy GIFs in the chat and in the image viewer get a star chip that saves them to the favourites (`stars.rs`); uploaded GIFs get the
//!   same star, which keeps the GIF on this computer (`upload_stars.rs`, `upload_favs.rs`: gif-favorites/, entries of kind "upload").
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
pub const NAME: &str = "GIF-Extras";
pub const VERSION: &str = "1.0.0";
pub const DESCRIPTION: &str = "Sticker, Kategorien und Favoriten für die GIF-Auswahl. GIFs im Chat lassen sich mit einem Stern merken.";

/// The plugin object: the extended popup's state and the favourites (loaded on first use).
#[derive(Default)]
pub(crate) struct Plugin {
    picker: picker::State,
    favs: Option<favs::Favs>,
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
        .section("GIF-Auswahl", "")
        .toggle(COMPACT, "Kompakte Größe", "Die kleinere GIF-Auswahl (350 × 390) wie in früheren Webox-Versionen.", false)
        .section("Favoriten-Sterne", "Ein Stern speichert ein GIF in den Favoriten der erweiterten GIF-Auswahl.")
        .toggle("stars_chat", "Stern an GIFs im Chat", "Erscheint oben rechts auf einem GIF, solange der Mauszeiger darüber steht.", true)
        .toggle("stars_viewer", "Stern in der Bildansicht", "Ein Stern-Knopf neben den Knöpfen der Bildansicht.", true)
        .toggle("stars_uploads", "Auch hochgeladene GIFs", "Hochgeladene GIFs mit Stern werden auf diesem Computer behalten (Ordner gif-favorites).", true)
        .section("Speicherort", "")
        .button("open_folder", "Favoriten-Ordner öffnen", "Der Ordner mit den behaltenen hochgeladenen GIFs.")
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
        if on(app, "stars_chat") && (a.giphy.is_some() || on(app, "stars_uploads")) { self.star_chip(app, ui, r, a, salt, ready); }
    }
    fn viewer_button(&mut self, app: &mut App, ui: &mut egui::Ui, cell: Rect, a: &cloud::Attachment) -> bool {
        on(app, "stars_viewer") && (a.giphy.is_some() || on(app, "stars_uploads")) && self.viewer_star(app, ui, cell, a)
    }
}
