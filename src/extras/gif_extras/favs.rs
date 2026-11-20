//! Favourite GIFs / stickers of the GIF-Extras plugin (loaded only while the plugin is on; with it off the file is never read or
//! written): saved locally only (JSON file `giphy_favorites.json` in the app's settings folder,
//! %APPDATA%\WebOx on Windows), newest first, no network sync. Entries keep the thumbnail and the shared file's URL and sizes so the
//! "Favoriten" list works without asking Giphy again. Capped at `MAX` (oldest dropped). In `--sample` / screenshot runs the list lives
//! in memory only (`path = None`), so nothing is written.
use crate::extras::giphy::Gif;
use serde_json::{json, Value};
use std::path::PathBuf;

pub const MAX: usize = 200;
const FILE: &str = "giphy_favorites.json";

#[derive(Clone, Debug, PartialEq)]
pub struct Fav {
    pub id: String, pub title: String, pub thumb_url: String, pub media_url: String,
    pub thumb_w: u32, pub thumb_h: u32, pub media_w: u32, pub media_h: u32,
    /// "gif", "sticker" or "upload" (an uploaded GIF kept on this computer, see `upload_favs`: `media_url` / `thumb_url` are local paths).
    pub kind: String,
}

impl Fav {
    pub fn from_gif(g: &Gif, sticker: bool) -> Fav {
        Fav { id: g.id.clone(), title: g.title.clone(), thumb_url: g.thumb_url.clone(), media_url: g.media_url.clone(), thumb_w: g.thumb_w, thumb_h: g.thumb_h,
              media_w: g.media_w, media_h: g.media_h, kind: if sticker { "sticker" } else { "gif" }.into() }
    }
    pub fn to_gif(&self) -> Gif {
        Gif { id: self.id.clone(), title: self.title.clone(), thumb_url: self.thumb_url.clone(), thumb_w: self.thumb_w, thumb_h: self.thumb_h, media_url: self.media_url.clone(), media_w: self.media_w, media_h: self.media_h }
    }
    pub fn is_sticker(&self) -> bool { self.kind == "sticker" }
    fn to_json(&self) -> Value {
        json!({ "id": self.id, "title": self.title, "thumb_url": self.thumb_url, "media_url": self.media_url,
                "thumb_w": self.thumb_w, "thumb_h": self.thumb_h, "media_w": self.media_w, "media_h": self.media_h, "kind": self.kind })
    }
    /// Entries without an id or the two URLs are dropped (a hand-edited file cannot inject anything else).
    fn from_json(v: &Value) -> Option<Fav> {
        let st = |k: &str| v[k].as_str().map(str::to_string);
        let n = |k: &str| v[k].as_u64().unwrap_or(0) as u32;
        let (id, thumb_url, media_url) = (st("id").filter(|s| !s.is_empty())?, st("thumb_url")?, st("media_url")?);
        Some(Fav { id, title: st("title").unwrap_or_default(), thumb_url, media_url, thumb_w: n("thumb_w"), thumb_h: n("thumb_h"), media_w: n("media_w"), media_h: n("media_h"),
                   kind: match st("kind").as_deref() { Some("sticker") => "sticker".into(), Some("upload") => "upload".into(), _ => "gif".into() } })
    }
}

#[derive(Default, Clone, Debug)]
pub struct Favs { items: Vec<Fav>, path: Option<PathBuf> }

impl Favs {
    /// Loads `dir/giphy_favorites.json` (missing or broken file = empty list). `dir = None`: memory only.
    pub fn load(dir: Option<PathBuf>) -> Favs {
        let path = dir.map(|d| d.join(FILE));
        let items = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v.as_array().map(|a| a.iter().filter_map(Fav::from_json).collect::<Vec<_>>())).unwrap_or_default();
        let mut f = Favs { items, path };
        f.items.truncate(MAX);
        f
    }
    pub fn contains(&self, id: &str, sticker: bool) -> bool { self.items.iter().any(|f| f.id == id && f.is_sticker() == sticker) }
    /// Favourites of one kind, newest first (uploaded GIFs count as GIFs; one whose local file is gone is left out).
    pub fn list(&self, sticker: bool) -> Vec<&Fav> { self.items.iter().filter(|f| f.is_sticker() == sticker && super::upload_favs::usable(f)).collect() }
    pub fn latest(&self, sticker: bool) -> Option<&Fav> { self.items.iter().find(|f| f.is_sticker() == sticker && super::upload_favs::usable(f)) }
    /// An entry by id (any kind).
    pub fn get(&self, id: &str) -> Option<&Fav> { self.items.iter().find(|f| f.id == id) }
    /// Adds an entry at the front (replacing one with the same id). Saves the file.
    pub fn insert(&mut self, f: Fav) { self.items.retain(|x| x.id != f.id); self.items.insert(0, f); self.items.truncate(MAX); self.save(); }
    /// Removes the entry with this id (any kind) and returns it. Saves the file.
    pub fn remove(&mut self, id: &str) -> Option<Fav> { let i = self.items.iter().position(|f| f.id == id)?; let f = self.items.remove(i); self.save(); Some(f) }
    #[allow(dead_code)]
    pub fn len(&self) -> usize { self.items.len() }
    /// Adds (to the front) or removes the entry; returns whether it is a favourite now. Saves the file.
    pub fn toggle(&mut self, g: &Gif, sticker: bool) -> bool {
        let now = if let Some(i) = self.items.iter().position(|f| f.id == g.id && f.is_sticker() == sticker) { self.items.remove(i); false }
                  else { self.items.insert(0, Fav::from_gif(g, sticker)); self.items.truncate(MAX); true };
        self.save();
        now
    }
    fn save(&self) {
        let Some(p) = &self.path else { return };
        if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
        if let Ok(s) = serde_json::to_string_pretty(&Value::Array(self.items.iter().map(Fav::to_json).collect())) { let _ = std::fs::write(p, s); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: &str) -> Gif { Gif { id: id.into(), title: format!("t {id}"), thumb_url: format!("https://media.giphy.com/{id}/200w_d.gif"), thumb_w: 200, thumb_h: 100,
        media_url: format!("https://media.giphy.com/{id}/giphy.gif"), media_w: 480, media_h: 240 } }

    fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("webox-favs-test-{name}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); d }

    #[test]
    fn toggle_adds_newest_first_and_removes() {
        let mut f = Favs::load(None);
        assert!(f.toggle(&g("a"), false));
        assert!(f.toggle(&g("b"), false));
        assert_eq!(f.list(false).iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(f.latest(false).unwrap().id, "b");
        assert!(!f.toggle(&g("a"), false), "second toggle removes");
        assert!(!f.contains("a", false) && f.contains("b", false));
    }

    #[test]
    fn kinds_are_separate() {
        let mut f = Favs::load(None);
        f.toggle(&g("x"), false);
        f.toggle(&g("x"), true);
        assert_eq!((f.list(false).len(), f.list(true).len()), (1, 1));
        assert!(f.latest(true).unwrap().is_sticker());
    }

    #[test]
    fn saved_to_disk_and_reloaded() {
        let d = tmp("rt");
        let mut f = Favs::load(Some(d.clone()));
        f.toggle(&g("a"), false);
        f.toggle(&g("s"), true);
        let back = Favs::load(Some(d.clone()));
        assert_eq!(back.len(), 2);
        let fav = back.latest(true).unwrap();
        assert_eq!((fav.id.as_str(), fav.kind.as_str(), fav.media_w), ("s", "sticker", 480));
        assert_eq!(fav.to_gif(), g("s"));
        let raw = std::fs::read_to_string(d.join("giphy_favorites.json")).unwrap();
        assert!(raw.contains("\"thumb_url\"") && raw.contains("\"media_url\"") && raw.contains("\"kind\": \"sticker\""));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn cap_and_bad_file() {
        let mut f = Favs::load(None);
        for i in 0..(MAX + 5) { f.toggle(&g(&i.to_string()), false); }
        assert_eq!(f.len(), MAX);
        assert_eq!(f.latest(false).unwrap().id, (MAX + 4).to_string(), "newest kept, oldest dropped");
        let d = tmp("bad");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(FILE), "{ not json").unwrap();
        assert_eq!(Favs::load(Some(d.clone())).len(), 0, "a broken file is treated as empty");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn memory_only_writes_nothing() {
        let mut f = Favs::load(None);
        f.toggle(&g("a"), false);
        assert!(f.path.is_none());
    }
}
