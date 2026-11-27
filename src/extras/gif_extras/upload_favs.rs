//! Favourites that were uploaded (not Giphy shares): GIFs starred in the chat and any picture (PNG, JPEG, WebP, GIF) saved with the
//! message menu's "Add to favorites". Their decrypted bytes are kept on this computer (`gif-favorites/<content hash>.<ext>` next to
//! the settings, with a small PNG thumbnail made once) and an entry of kind "upload" goes into the GIF-Extras plugin's favourites
//! store (`favs.rs`, same list as the Giphy favourites; `media_url` / `thumb_url` hold the two local paths). The same picture saved
//! twice is one entry (the id is its content hash). Removing one deletes both files. Picking one in the picker stages a copy (named
//! like the uploaded file) as a normal attachment, so it goes out with the encrypted upload on Send; nothing is shared from Giphy and
//! nothing leaves the computer before the user's own Send (or the click, with "Send favorites on click").
//!
//! Limits: `MAX_BYTES` per picture, `TOTAL_MAX_BYTES` for all kept files together.
//!
//! Self-contained (store + staging helpers); the UI hooks are `upload_stars.rs` and two calls in the plugin's popup (`picker.rs`).
//! Part of the GIF-Extras plugin: with the plugin off nothing here runs and the kept files stay as they are.
use crate::extras::giphy::Gif;
use super::favs::Fav;
use std::path::{Path, PathBuf};

/// Largest GIF / picture kept as a favourite.
pub const MAX_BYTES: usize = 20 * 1024 * 1024;
/// All kept files together (pictures and their thumbnails).
pub const TOTAL_MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Folder next to the settings.
pub const DIR: &str = "gif-favorites";
/// Longest side of the stored thumbnail (the picker's tiles are ~200 px wide).
const THUMB_SIDE: u32 = 240;
/// Room counted for a thumbnail when checking the total cap (they are far smaller).
const THUMB_ALLOWANCE: u64 = 256 * 1024;
/// Id prefix of an uploaded favourite (Giphy ids never contain ':').
pub const PREFIX: &str = "upload:";

pub fn is_upload_id(id: &str) -> bool { id.starts_with(PREFIX) }
pub fn is_upload(f: &Fav) -> bool { f.kind == "upload" }

/// The file extension of a picture that can be kept (by its content, not its name).
pub fn picture_ext(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Gif => Some("gif"), image::ImageFormat::Png => Some("png"), image::ImageFormat::Jpeg => Some("jpg"),
        image::ImageFormat::WebP => Some("webp"), _ => None,
    }
}

fn hex16(d: &[u8]) -> String { d.iter().take(16).map(|b| format!("{b:02x}")).collect() }

/// Stable id of a picture by its content: "upload:" + the first 32 hex digits of its SHA-256.
pub fn content_id(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{PREFIX}{}", hex16(&sha2::Sha256::digest(bytes)))
}

/// The `Fav::src` of an attachment key: a hash, so the store never holds the key (a content address) itself.
pub fn src_id(key: &str) -> String {
    use sha2::Digest;
    hex16(&sha2::Sha256::digest(key.as_bytes()))
}

/// Bytes on disk of the kept uploads in `favs` (pictures and thumbnails), without the entry `except`.
pub fn used_bytes<'a>(favs: impl Iterator<Item = &'a Fav>, except: &str) -> u64 {
    favs.filter(|f| f.id != except).flat_map(|f| [&f.media_url, &f.thumb_url]).filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum()
}

/// Keeps a GIF or picture as a favourite: writes `<dir>/<hash>.<ext>` and `<hash>.thumb.png` and returns the store entry.
/// `used`: what the other kept files take already (`used_bytes`). Refused: not a GIF / PNG / JPEG / WebP, larger than `MAX_BYTES`,
/// over `TOTAL_MAX_BYTES` together with the others, or undecodable.
pub fn star(dir: &Path, title: &str, bytes: &[u8], used: u64) -> Result<Fav, String> {
    if bytes.len() > MAX_BYTES { return Err(crate::trq!("Picture too large").into_owned()); }
    let ext = picture_ext(bytes).ok_or_else(|| crate::trq!("Not a supported picture").into_owned())?;
    let first = if ext == "gif" { first_frame(bytes) } else { crate::assets::photos::decode_net(bytes).ok().map(|i| i.into_rgba8()) }
        .ok_or_else(|| crate::trq!("Picture cannot be read").into_owned())?;
    if used + bytes.len() as u64 + THUMB_ALLOWANCE > TOTAL_MAX_BYTES {
        return Err(crate::trq!("The favorites folder is full (%1)", crate::app::files::size_text(TOTAL_MAX_BYTES)));
    }
    let (w, h) = (first.width(), first.height());
    let thumb = if w.max(h) > THUMB_SIDE { image::DynamicImage::ImageRgba8(first).thumbnail(THUMB_SIDE, THUMB_SIDE) } else { image::DynamicImage::ImageRgba8(first) };
    let id = content_id(bytes);
    let stem = id.trim_start_matches(PREFIX).to_string();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (file, png) = (dir.join(format!("{stem}.{ext}")), dir.join(format!("{stem}.thumb.png")));
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    thumb.save_with_format(&png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let title = match title.trim() { "" if ext == "gif" => "GIF".to_string(), "" => format!("Image.{ext}"), t => t.to_string() };
    Ok(Fav { id, title, thumb_url: png.to_string_lossy().into_owned(), media_url: file.to_string_lossy().into_owned(),
        thumb_w: thumb.width(), thumb_h: thumb.height(), media_w: w, media_h: h, kind: "upload".into(), src: String::new() })
}

/// Removes the two local files of an uploaded favourite (only files inside `dir`, so a hand-edited store can't delete elsewhere).
pub fn unstar(dir: &Path, f: &Fav) {
    for p in [&f.media_url, &f.thumb_url] {
        let p = Path::new(p);
        if p.parent().is_some_and(|d| same_dir(d, dir)) { let _ = std::fs::remove_file(p); }
    }
}

/// The favourite's file is still on disk (a deleted file hides the entry).
pub fn usable(f: &Fav) -> bool { !is_upload(f) || Path::new(&f.media_url).is_file() }

/// A copy for the composer, named like the uploaded file (the kept file's extension added when missing), in a fresh temporary
/// folder: staged and sent like any file the user attaches.
pub fn stage_copy(g: &Gif) -> Option<PathBuf> {
    let src = Path::new(&g.media_url);
    if !src.is_file() { return None; }
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("gif").to_lowercase();
    let mut name = crate::app::files::safe_name(&g.title);
    let lower = name.to_lowercase();
    if !(lower.ends_with(&format!(".{ext}")) || (ext == "jpg" && lower.ends_with(".jpeg"))) { name.push('.'); name.push_str(&ext); }
    let dir = crate::app::files::private_tmp("staged").ok()?.join(crate::jose::uuid());
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
    let d = crate::assets::photos::gif_decoder(bytes).ok()?; // canvas size checked, allocations limited
    d.into_frames().next()?.ok().map(|f| f.into_buffer())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) { (Ok(x), Ok(y)) => x == y, _ => a == b }
}

#[cfg(test)]
pub(super) mod tests {
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

    /// A 640 x 400 picture in `fmt` made here.
    pub(in crate::extras::gif_extras) fn encode(fmt: image::ImageFormat, seed: u8) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(640, 400, |x, y| image::Rgb([seed, (x % 256) as u8, (y % 256) as u8])));
        let mut out = std::io::Cursor::new(vec![]);
        img.write_to(&mut out, fmt).unwrap();
        out.into_inner()
    }

    #[test]
    fn star_writes_gif_and_thumbnail_and_unstar_removes_them() {
        let d = tmp("add");
        let b = gif(1);
        let f = star(&d, "  party.gif ", &b, 0).unwrap();
        assert!(is_upload(&f) && is_upload_id(&f.id) && f.id == content_id(&b));
        assert_eq!((f.title.as_str(), f.media_w, f.media_h), ("party.gif", 300, 200));
        assert!(f.thumb_w <= THUMB_SIDE && f.thumb_h <= THUMB_SIDE && f.thumb_w == 240);
        assert!(f.media_url.ends_with(".gif"));
        assert_eq!(std::fs::read(&f.media_url).unwrap(), b);
        assert_eq!(image::guess_format(&std::fs::read(&f.thumb_url).unwrap()).unwrap(), image::ImageFormat::Png);
        assert!(usable(&f));
        unstar(&d, &f);
        assert!(!Path::new(&f.media_url).exists() && !Path::new(&f.thumb_url).exists());
        assert!(!usable(&f), "a missing file hides the entry");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn pictures_are_kept_with_their_own_extension_and_a_thumbnail() {
        let d = tmp("pics");
        for (fmt, ext, seed) in [(image::ImageFormat::Png, "png", 1), (image::ImageFormat::Jpeg, "jpg", 2), (image::ImageFormat::WebP, "webp", 3)] {
            let b = encode(fmt, seed);
            let f = star(&d, "", &b, 0).unwrap();
            assert!(is_upload(&f) && f.id == content_id(&b));
            assert!(f.media_url.ends_with(&format!(".{ext}")), "{}", f.media_url);
            assert_eq!(std::fs::read(&f.media_url).unwrap(), b, "the original bytes, unchanged");
            assert_eq!((f.media_w, f.media_h, f.thumb_w, f.thumb_h), (640, 400, 240, 150), "thumbnail at the picture's aspect");
            assert_eq!(f.title, format!("Image.{ext}"));
            // Staged under the title with the kept file's extension.
            let p = stage_copy(&Fav { title: "holiday".into(), ..f.clone() }.to_gif()).unwrap();
            assert_eq!(p.file_name().unwrap().to_string_lossy(), format!("holiday.{ext}"));
            assert_eq!(std::fs::read(&p).unwrap(), b);
            let _ = std::fs::remove_dir_all(p.parent().unwrap());
        }
        // A JPEG named ".jpeg" keeps its name.
        let f = star(&d, "a.jpeg", &encode(image::ImageFormat::Jpeg, 9), 0).unwrap();
        let p = stage_copy(&f.to_gif()).unwrap();
        assert_eq!(p.file_name().unwrap(), "a.jpeg");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn same_content_same_id_other_content_other_id() {
        assert_eq!(content_id(&gif(1)), content_id(&gif(1)));
        assert_ne!(content_id(&gif(1)), content_id(&gif(2)));
        assert_eq!(content_id(b"x").len(), PREFIX.len() + 32);
    }

    #[test]
    fn src_ids_hash_the_key_and_used_bytes_count_the_files() {
        assert_eq!(src_id("k1"), src_id("k1"));
        assert_ne!(src_id("k1"), src_id("k2"));
        assert!(!src_id("https://files.example/abc").contains("example"));
        let d = tmp("used");
        let a = star(&d, "a.png", &encode(image::ImageFormat::Png, 1), 0).unwrap();
        let b = star(&d, "b.png", &encode(image::ImageFormat::Png, 2), 0).unwrap();
        let size = |f: &Fav| std::fs::metadata(&f.media_url).unwrap().len() + std::fs::metadata(&f.thumb_url).unwrap().len();
        assert_eq!(used_bytes([&a, &b].into_iter(), ""), size(&a) + size(&b));
        assert_eq!(used_bytes([&a, &b].into_iter(), &a.id), size(&b), "the entry being replaced does not count");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn refuses_too_big_unsupported_and_broken_pictures() {
        let d = tmp("cap");
        let mut big = gif(3);
        big.resize(MAX_BYTES + 1, 0);
        assert!(star(&d, "big.gif", &big, 0).is_err());
        assert!(star(&d, "x.png", b"\x89PNG\r\n\x1a\n0000", 0).is_err(), "a broken PNG");
        assert!(star(&d, "x.bmp", b"BM\x46\x00\x00\x00\x00\x00\x00\x00\x36\x00\x00\x00", 0).is_err(), "not a kept format");
        assert!(star(&d, "x.txt", b"hello", 0).is_err());
        assert!(star(&d, "broken.gif", b"GIF89a\x01", 0).is_err());
        assert!(star(&d, "bomb.gif", &[b"GIF89a".as_slice(), &[0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0x3b]].concat(), 0).is_err()); // 65535² canvas
        // The total cap: a picture that would take the folder over it is refused.
        assert!(star(&d, "x.png", &encode(image::ImageFormat::Png, 5), TOTAL_MAX_BYTES - 1000).is_err());
        assert!(!d.exists() || std::fs::read_dir(&d).unwrap().next().is_none(), "nothing written");
    }

    #[test]
    fn unstar_never_deletes_outside_its_folder() {
        let (d, other) = (tmp("own"), tmp("other"));
        std::fs::create_dir_all(&other).unwrap();
        let victim = other.join("keep.gif");
        std::fs::write(&victim, b"x").unwrap();
        let f = Fav { id: "upload:x".into(), title: "x".into(), thumb_url: victim.to_string_lossy().into(), media_url: victim.to_string_lossy().into(), thumb_w: 1, thumb_h: 1, media_w: 1, media_h: 1, kind: "upload".into(), src: String::new() };
        std::fs::create_dir_all(&d).unwrap();
        unstar(&d, &f);
        assert!(victim.exists());
        let _ = std::fs::remove_dir_all(other);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn staged_copy_is_named_like_the_file() {
        let d = tmp("stage");
        let f = star(&d, "Cat", &gif(4), 0).unwrap();
        let p = stage_copy(&f.to_gif()).unwrap();
        assert_eq!(p.file_name().unwrap(), "Cat.gif");
        assert_eq!(std::fs::read(&p).unwrap(), std::fs::read(&f.media_url).unwrap());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        let _ = std::fs::remove_dir_all(d);
    }
}
