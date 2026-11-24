//! Image paste on native Wayland. arboard (`ui/attach`) only speaks X11, so on Wayland it reads the clipboard through
//! XWayland, which misses what native Wayland apps copied (a Spectacle screenshot, an image from Firefox). Here the clipboard
//! is read through the data-control protocols (ext-data-control-v1 / wlr-data-control: KDE Plasma, wlroots, Hyprland) with
//! wl-clipboard-rs. GNOME offers neither; there, and on any connection error, the caller falls back to the XWayland path.
//! Only reading: copying stays with arboard.
use std::io::Read;
use wl_clipboard_rs::paste::{self, ClipboardType, Error, MimeType, Seat};

/// The image types the composer takes, best first: PNG goes through as offered, the others (the image crate's enabled
/// decoders) are turned into PNG, since a pasted image is staged as `<guid>.PNG`.
const IMAGE_TYPES: [&str; 5] = ["image/png", "image/webp", "image/jpeg", "image/jpg", "image/gif"];

/// The offered type to ask for: the best of `IMAGE_TYPES`, spelled as the source offered it.
pub(crate) fn pick_image_type<'a>(offered: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let offered: Vec<&str> = offered.into_iter().collect();
    IMAGE_TYPES.iter().find_map(|t| offered.iter().copied().find(|o| o.trim().eq_ignore_ascii_case(t)))
}

/// Whether a failed read means "can't ask this way" (no data-control protocol as on GNOME, no seat, no connection): then the
/// X11 path gets its try. An empty clipboard or one without a matching type is an answer, not a failure.
pub(crate) fn falls_back(e: &Error) -> bool { !matches!(e, Error::ClipboardEmpty | Error::NoMimeType) }

/// The clipboard's bytes as PNG: a PNG as it is (no decode / re-encode), anything else the image crate reads re-encoded.
pub(crate) fn to_png(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if image::guess_format(&bytes).ok()? == image::ImageFormat::Png { return Some(bytes); }
    let img = image::load_from_memory(&bytes).ok()?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// The clipboard's image as PNG through data-control, off the UI thread (it waits for the source app to write).
/// `None`: the compositor can't be asked this way, use the X11 path. `Some(None)`: it answered, and there is no image.
pub(crate) fn image_png() -> Option<Option<Vec<u8>>> {
    let answer = |e: Error| if falls_back(&e) { None } else { Some(None) };
    let types = match paste::get_mime_types(ClipboardType::Regular, Seat::Unspecified) { Ok(t) => t, Err(e) => return answer(e) };
    let Some(mime) = pick_image_type(types.iter().map(String::as_str)) else { return Some(None) };
    let (mut pipe, _) = match paste::get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Specific(mime)) {
        Ok(p) => p,
        Err(e) => return answer(e),
    };
    let mut bytes = Vec::new();
    if pipe.read_to_end(&mut bytes).is_err() { return Some(None); }
    Some(to_png(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_best_offered_image_type() {
        // Spectacle / Firefox offer PNG among others: PNG wins whatever the order.
        assert_eq!(pick_image_type(["text/html", "image/jpeg", "image/png", "text/plain"]), Some("image/png"));
        assert_eq!(pick_image_type(["image/gif", "image/jpeg"]), Some("image/jpeg"));
        assert_eq!(pick_image_type(["image/gif", "image/webp"]), Some("image/webp"));
        assert_eq!(pick_image_type(["image/jpg"]), Some("image/jpg"));
        // asked for as the source spelled it
        assert_eq!(pick_image_type(["IMAGE/PNG"]), Some("IMAGE/PNG"));
        // no image: text, files, image types without a decoder here
        assert_eq!(pick_image_type(["text/plain;charset=utf-8", "UTF8_STRING"]), None);
        assert_eq!(pick_image_type(["text/uri-list", "image/bmp", "image/tiff"]), None);
        assert_eq!(pick_image_type([] as [&str; 0]), None);
    }

    #[test]
    fn falls_back_only_without_the_protocol() {
        // GNOME (no data-control) and connection trouble: try XWayland as before
        assert!(falls_back(&Error::MissingProtocol { name: "ext_data_control_manager_v1", version: 1 }));
        assert!(falls_back(&Error::NoSeats));
        assert!(falls_back(&Error::SeatNotFound));
        // an answer: nothing (image) on the clipboard
        assert!(!falls_back(&Error::ClipboardEmpty));
        assert!(!falls_back(&Error::NoMimeType));
    }

    fn encoded(format: image::ImageFormat) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3, 2, image::Rgb([10, 20, 30]))); // RGB: the JPEG encoder takes no alpha
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    #[test]
    fn png_passes_through_others_become_png() {
        let png = encoded(image::ImageFormat::Png);
        assert_eq!(to_png(png.clone()), Some(png));
        for f in [image::ImageFormat::Jpeg, image::ImageFormat::Gif] {
            let out = to_png(encoded(f)).expect("converted");
            assert_eq!(image::guess_format(&out).unwrap(), image::ImageFormat::Png);
            assert_eq!(image::load_from_memory(&out).unwrap().width(), 3);
        }
        assert_eq!(to_png(b"not an image".to_vec()), None);
    }
}
