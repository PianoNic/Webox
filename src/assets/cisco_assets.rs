//! Webex client files that are not part of the repository: translations, sounds, the splash and login
//! pictures, the Giphy marks and the shortcut and emoji tables (list: docs/cisco-assets.md).
//!
//! Order: embedded at build time from the git-ignored `assets/cisco/` (see `build.rs`), else the loose file of an
//! installed Webex app (Windows: `%LOCALAPPDATA%\CiscoSparkLauncher\dynamic_dependencies`, sounds only), else
//! `None`, and the caller falls back (English, silence, a plain background, the Webox tables).
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

mod built {
    include!(concat!(env!("OUT_DIR"), "/cisco_assets.rs"));
}

/// Whether this build has the app's files embedded (tests that compare against them skip otherwise).
#[cfg(test)]
pub fn embedded() -> bool { !built::CISCO.is_empty() }

/// The file at `rel` (relative to `assets/cisco/`, e.g. `sounds/Delight.wav`).
pub fn get(rel: &str) -> Option<&'static [u8]> {
    if let Ok(i) = built::CISCO.binary_search_by(|e| e.0.cmp(rel)) { return Some(built::CISCO[i].1); }
    static FROM_INSTALL: OnceLock<Mutex<HashMap<String, Option<&'static [u8]>>>> = OnceLock::new();
    let mut cache = FROM_INSTALL.get_or_init(Default::default).lock().ok()?;
    *cache.entry(rel.to_string()).or_insert_with(|| installed(rel).map(|b| &*Box::leak(b.into_boxed_slice())))
}

/// [`get`] as UTF-8 text.
pub fn text(rel: &str) -> Option<&'static str> { get(rel).and_then(|b| std::str::from_utf8(b).ok()) }

/// The same file as a loose file of an installed Webex app. Only the sounds are loose files there; translations are
/// read by `i18n::pack_dirs`.
fn installed(rel: &str) -> Option<Vec<u8>> {
    let name = rel.strip_prefix("sounds/")?;
    let base = std::path::Path::new(&std::env::var_os("LOCALAPPDATA")?).join("CiscoSparkLauncher").join("dynamic_dependencies");
    ["ringtones", "momentum_ringtones", "sounds"].iter().find_map(|group| std::fs::read(base.join(group).join(name)).ok())
}
