//! Favourite GIFs that were uploaded (not Giphy shares): starring one in the chat keeps its decrypted bytes on this computer
//! (`gif-favorites/<content hash>.gif` next to the settings, with a small PNG thumbnail made once) and adds an entry of kind
//! "upload" to the GIF-Extras plugin's favourites store (`favs.rs`, same list as the Giphy favourites; `media_url` / `thumb_url` hold the
//! two local paths). Unstarring deletes both files. Picking one in the picker stages a copy (named like the uploaded file) as a
//! normal attachment, so it goes out with the encrypted upload on Send; nothing is shared from Giphy and nothing leaves the
//! computer before the user's own Send.
//!
//! Self-contained (store + staging helpers); the UI hooks are `upload_stars.rs` and two calls in the plugin's popup (`picker.rs`).
//! Part of the GIF-Extras plugin: with the plugin off nothing here runs and the kept files stay as they are.
use crate::extras::giphy::Gif;
use super::favs::Fav;
use std::path::{Path, PathBuf};

/// Largest GIF kept as a favourite.
pub const MAX_BYTES: usize = 20 * 1024 * 1024;
/// Folder next to the settings.
pub const DIR: &str = "gif-favorites";
/// Longest side of the stored thumbnail (the picker's tiles are ~200 px wide).
const THUMB_SIDE: u32 = 240;
/// Id prefix of an uploaded favourite (Giphy ids never contain ':').
pub const PREFIX: &str = "upload:";

pub fn is_upload_id(id: &str) -> bool { id.starts_with(PREFIX) }
pub fn is_upload(f: &Fav) -> bool { f.kind == "upload" }

/// Stable id of a GIF by its content: "upload:" + the first 32 hex digits of its SHA-256.
pub fn content_id(bytes: &[u8]) -> String {
    use sha2::Digest;
    let h: String = sha2::Sha256::digest(bytes).iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("{PREFIX}{h}")
}

/// Keeps a GIF as a favourite: writes `<dir>/<hash>.gif` and `<hash>.thumb.png` and returns the store entry.
/// Refused: not a GIF, larger than `MAX_BYTES`, or undecodable.
pub fn star(dir: &Path, title: &str, bytes: &[u8]) -> Result<Fav, String> {
    if bytes.len() > MAX_BYTES { return Err("GIF zu groß".into()); }
    if image::guess_format(bytes).ok() != Some(image::ImageFormat::Gif) { return Err("kein GIF".into()); }
    let first = first_frame(bytes).ok_or("GIF nicht lesbar")?;
    let (w, h) = (first.width(), first.height());
    let thumb = if w.max(h) > THUMB_SIDE { image::DynamicImage::ImageRgba8(first).thumbnail(THUMB_SIDE, THUMB_SIDE) } else { image::DynamicImage::ImageRgba8(first) };
    let id = content_id(bytes);
    let stem = id.trim_start_matches(PREFIX).to_string();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (gif, png) = (dir.join(format!("{stem}.gif")), dir.join(format!("{stem}.thumb.png")));
    std::fs::write(&gif, bytes).map_err(|e| e.to_string())?;
    thumb.save_with_format(&png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let title = { let t = title.trim(); if t.is_empty() { "GIF".to_string() } else { t.to_string() } };
    Ok(Fav { id, title, thumb_url: png.to_string_lossy().into_owned(), media_url: gif.to_string_lossy().into_owned(),
        thumb_w: thumb.width(), thumb_h: thumb.height(), media_w: w, media_h: h, kind: "upload".into() })
}

/// Removes the two local files of an uploaded favourite (only files inside `dir`, so a hand-edited store can't delete elsewhere).
pub fn unstar(dir: &Path, f: &Fav) {
    for p in [&f.media_url, &f.thumb_url] {
        let p = Path::new(p);
        if p.parent().is_some_and(|d| same_dir(d, dir)) { let _ = std::fs::remove_file(p); }
    }
}

/// The favourite's GIF is still on disk (a deleted file hides the entry).
pub fn usable(f: &Fav) -> bool { !is_upload(f) || Path::new(&f.media_url).is_file() }

/// A copy for the composer, named like the uploaded file (".gif" added when missing), in a fresh temporary folder: staged and
/// sent like any file the user attaches.
pub fn stage_copy(g: &Gif) -> Option<PathBuf> {
    let src = Path::new(&g.media_url);
    if !src.is_file() { return None; }
    let mut name = crate::app::files::safe_name(&g.title);
    if !name.to_lowercase().ends_with(".gif") { name.push_str(".gif"); }
    let dir = std::env::temp_dir().join("webox-staged").join(crate::jose::uuid());
    std::fs::create_dir_all(&dir).ok()?;
    let dst = dir.join(name);
    std::fs::copy(src, &dst).ok()?;
    Some(dst)
}

/// Reads a favourite's thumbnail off the UI thread and hands it to the picker like a downloaded Giphy thumbnail.
pub fn load_thumb_async(tx: std::sync::mpsc::Sender<crate::extras::giphy::Event>, id: String, path: String) {
    std::thread::spawn(move || { let bytes = std::fs::read(&path).ok(); let _ = tx.send(crate::extras::giphy::Event::Thumb { id, bytes }); });
}

fn first_frame(bytes: &[u8]) -> Option<image::RgbaImage> {
    use image::AnimationDecoder;
    let d = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    d.into_frames().next()?.ok().map(|f| f.into_buffer())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) { (Ok(x), Ok(y)) => x == y, _ => a == b }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("webox-upfav-{name}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); d }

    /// A two-frame 300 x 200 GIF made here (no external file).
    fn gif(seed: u8) -> Vec<u8> {
        let mut out = vec![];
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            for i in 0..2u8 {
                let img = image::RgbaImage::from_pixel(300, 200, image::Rgba([seed, i * 100, 50, 255]));
                enc.encode_frame(image::Frame::new(img)).unwrap();
            }
        }
        out
    }

    #[test]
    fn star_writes_gif_and_thumbnail_and_unstar_removes_them() {
        let d = tmp("add");
        let b = gif(1);
        let f = star(&d, "  party.gif ", &b).unwrap();
        assert!(is_upload(&f) && is_upload_id(&f.id) && f.id == content_id(&b));
        assert_eq!((f.title.as_str(), f.media_w, f.media_h), ("party.gif", 300, 200));
        assert!(f.thumb_w <= THUMB_SIDE && f.thumb_h <= THUMB_SIDE && f.thumb_w == 240);
        assert_eq!(std::fs::read(&f.media_url).unwrap(), b);
        assert_eq!(image::guess_format(&std::fs::read(&f.thumb_url).unwrap()).unwrap(), image::ImageFormat::Png);
        assert!(usable(&f));
        unstar(&d, &f);
        assert!(!Path::new(&f.media_url).exists() && !Path::new(&f.thumb_url).exists());
        assert!(!usable(&f), "a missing file hides the entry");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn same_content_same_id_other_content_other_id() {
        assert_eq!(content_id(&gif(1)), content_id(&gif(1)));
        assert_ne!(content_id(&gif(1)), content_id(&gif(2)));
        assert_eq!(content_id(b"x").len(), PREFIX.len() + 32);
    }

    #[test]
    fn refuses_too_big_and_non_gifs() {
        let d = tmp("cap");
        let mut big = gif(3);
        big.resize(MAX_BYTES + 1, 0);
        assert!(star(&d, "big.gif", &big).is_err());
        assert!(star(&d, "x.png", b"\x89PNG\r\n\x1a\n0000").is_err());
        assert!(star(&d, "broken.gif", b"GIF89a\x01").is_err());
        assert!(!d.exists() || std::fs::read_dir(&d).unwrap().next().is_none(), "nothing written");
    }

    #[test]
    fn unstar_never_deletes_outside_its_folder() {
        let (d, other) = (tmp("own"), tmp("other"));
        std::fs::create_dir_all(&other).unwrap();
        let victim = other.join("keep.gif");
        std::fs::write(&victim, b"x").unwrap();
        let f = Fav { id: "upload:x".into(), title: "x".into(), thumb_url: victim.to_string_lossy().into(), media_url: victim.to_string_lossy().into(), thumb_w: 1, thumb_h: 1, media_w: 1, media_h: 1, kind: "upload".into() };
        std::fs::create_dir_all(&d).unwrap();
        unstar(&d, &f);
        assert!(victim.exists());
        let _ = std::fs::remove_dir_all(other);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn staged_copy_is_named_like_the_file() {
        let d = tmp("stage");
        let f = star(&d, "Katze", &gif(4)).unwrap();
        let p = stage_copy(&f.to_gif()).unwrap();
        assert_eq!(p.file_name().unwrap(), "Katze.gif");
        assert_eq!(std::fs::read(&p).unwrap(), std::fs::read(&f.media_url).unwrap());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        let _ = std::fs::remove_dir_all(d);
    }
}
