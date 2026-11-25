//! Optional extras: the Giphy search (`giphy`), the built-in "GIF Extras" plugin (`gif_extras`: extended picker, stickers,
//! favourites of Giphy and uploaded GIFs, stars), the plugin host (`plugins`), plugin settings (`plugin_settings`) and plugin
//! repositories (`plugin_repo`).
pub mod gif_extras;
pub mod giphy;
pub mod plugins;
pub mod plugin_repo;
pub mod plugin_settings;

/// A built-in (in-tree, first-party) plugin for the plugin list.
pub struct Builtin {
    pub key: &'static str, pub name: &'static str, pub version: &'static str, pub description: &'static str,
    /// Its settings schema (the same JSON a native plugin returns, built with plugin-api's `Schema`), if it has settings.
    pub settings: Option<fn() -> String>,
}

/// The built-in plugins.
pub fn builtin_plugins() -> Vec<Builtin> {
    vec![Builtin { key: gif_extras::KEY, name: gif_extras::NAME, version: gif_extras::VERSION, description: gif_extras::DESCRIPTION, settings: Some(gif_extras::settings_schema) }]
}