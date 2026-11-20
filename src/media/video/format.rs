//! What kind of file a video is (by its first bytes, then its name / MIME type), whether this platform can play it, and the
//! player's time text.
//!
//! Detection: the container's signature (ISO BMFF `ftyp` and its brand, EBML DocType for Matroska / WebM, RIFF `AVI `, the ASF
//! header, MPEG-TS sync bytes, MPEG-PS pack header, Ogg, FLV); without bytes the extension, then the MIME type.
//! Media Foundation reads MP4 / MOV / M4V / 3GP, ASF (WMV), AVI, MPEG-TS, MKV / WebM and, with the Web Media Extensions, Ogg;
//! FLV and MPEG-PS are refused up front. The codecs inside depend on the installed decoders, so a playable container can still
//! fail to open; the card then shows an error. On Linux GStreamer tries every container.

/// A video container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container { Mp4, Mov, ThreeGp, Matroska, WebM, Avi, Asf, MpegTs, MpegPs, Ogg, Flv }

/// The container by its signature. `head`: the file's first bytes (64 are enough for everything but a WebM DocType that sits
/// late in a padded EBML header; 4 KiB is plenty).
pub fn sniff(head: &[u8]) -> Option<Container> {
    let at = |i: usize, s: &[u8]| head.len() >= i + s.len() && &head[i..i + s.len()] == s;
    if at(4, b"ftyp") {
        let brand = head.get(8..12).unwrap_or(b"    ");
        return Some(match brand {
            b"qt  " => Container::Mov,
            b if b.starts_with(b"3g") => Container::ThreeGp,
            _ => Container::Mp4, // isom, iso2..6, mp41, mp42, avc1, M4V , M4A , dash, MSNV, ...
        });
    }
    // QuickTime files without ftyp start with a moov / mdat / wide / free atom
    if [b"moov", b"mdat", b"wide", b"free", b"skip", b"pnot"].iter().any(|t| at(4, *t)) && head.len() >= 8 { return Some(Container::Mov); }
    if at(0, &[0x1A, 0x45, 0xDF, 0xA3]) {
        // EBML: DocType element (0x4282) holds "webm" or "matroska"
        let n = head.len().min(4096);
        let doc = head[..n].windows(2).position(|w| w == [0x42, 0x82]).map(|i| &head[i + 2..n]);
        return Some(match doc { Some(d) if d.len() >= 5 && &d[1..5] == b"webm" => Container::WebM, _ => Container::Matroska });
    }
    if at(0, b"RIFF") && at(8, b"AVI ") { return Some(Container::Avi); }
    // the ASF Header Object's GUID as it is stored (little-endian fields)
    if at(0, &[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C]) { return Some(Container::Asf); }
    if at(0, b"OggS") { return Some(Container::Ogg); }
    if at(0, b"FLV") && head.get(3) == Some(&1) { return Some(Container::Flv); }
    if at(0, &[0x00, 0x00, 0x01, 0xBA]) { return Some(Container::MpegPs); }
    // MPEG-TS: 0x47 every 188 bytes (BDAV / M2TS: every 192, 4 bytes in)
    let ts = |off: usize, step: usize| head.len() > off + 2 * step && (0..3).all(|k| head[off + k * step] == 0x47);
    if ts(0, 188) || ts(4, 192) { return Some(Container::MpegTs); }
    None
}

/// The container by file name.
pub fn by_name(name: &str) -> Option<Container> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "mp4" | "m4v" | "f4v" => Container::Mp4,
        "mov" | "qt" => Container::Mov,
        "3gp" | "3g2" => Container::ThreeGp,
        "mkv" => Container::Matroska,
        "webm" => Container::WebM,
        "avi" => Container::Avi,
        "wmv" | "asf" => Container::Asf,
        "ts" | "mts" | "m2ts" => Container::MpegTs,
        "mpg" | "mpeg" | "vob" => Container::MpegPs,
        "ogv" => Container::Ogg,
        "flv" => Container::Flv,
        _ => return None,
    })
}

/// The container by MIME type.
pub fn by_mime(mime: &str) -> Option<Container> {
    Some(match mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "video/mp4" | "video/x-m4v" => Container::Mp4,
        "video/quicktime" => Container::Mov,
        "video/3gpp" | "video/3gpp2" => Container::ThreeGp,
        "video/x-matroska" | "video/matroska" => Container::Matroska,
        "video/webm" => Container::WebM,
        "video/x-msvideo" | "video/avi" => Container::Avi,
        "video/x-ms-wmv" | "video/x-ms-asf" => Container::Asf,
        "video/mp2t" => Container::MpegTs,
        "video/mpeg" => Container::MpegPs,
        "video/ogg" => Container::Ogg,
        "video/x-flv" => Container::Flv,
        _ => return None,
    })
}

/// Bytes first (a renamed file is still what it is), then the name, then the MIME type.
pub fn detect(head: &[u8], name: &str, mime: &str) -> Option<Container> { sniff(head).or_else(|| by_name(name)).or_else(|| by_mime(mime)) }

/// An attachment that the message list shows as a video (a video MIME type, or a video file name). Audio-only types are not videos.
pub fn is_video(mime: &str, name: &str) -> bool {
    let m = mime.trim().to_ascii_lowercase();
    m.starts_with("video/") || (!m.starts_with("image/") && !m.starts_with("audio/") && by_name(name).is_some())
}

/// Whether this platform can read the container (on Linux the installed plugins decide).
pub fn playable(c: Container) -> bool {
    if cfg!(windows) { !matches!(c, Container::Flv | Container::MpegPs) } else { true }
}

/// Time as "m:ss": whole minutes (no hours), seconds floored with a leading zero.
pub fn time_text(secs: f64) -> String {
    let s = if secs.is_finite() && secs > 0.0 { secs.floor() as u64 } else { 0 };
    format!("{}:{:02}", s / 60, s % 60)
}

/// "position / duration", or "0:00 / 0:00" while either is unknown or the position is past the end.
pub fn position_text(pos: f64, dur: f64) -> String {
    if !(pos >= 0.0) || !(dur > 0.0) || pos > dur { return "0:00 / 0:00".into(); }
    format!("{} / {}", time_text(pos), time_text(dur))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ftyp(brand: &[u8; 4]) -> Vec<u8> { let mut v = vec![0, 0, 0, 0x18]; v.extend_from_slice(b"ftyp"); v.extend_from_slice(brand); v.extend_from_slice(&[0; 12]); v }

    #[test]
    fn containers_by_signature() {
        assert_eq!(sniff(&ftyp(b"isom")), Some(Container::Mp4));
        assert_eq!(sniff(&ftyp(b"M4V ")), Some(Container::Mp4));
        assert_eq!(sniff(&ftyp(b"qt  ")), Some(Container::Mov));
        assert_eq!(sniff(&ftyp(b"3gp5")), Some(Container::ThreeGp));
        assert_eq!(sniff(b"\0\0\0\x08wide\0\0\0\0mdat"), Some(Container::Mov));
        // EBML header with DocType "webm" / "matroska"
        let ebml = |doc: &[u8]| { let mut v = vec![0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x86, 0x81, 0x01, 0x42, 0x82, 0x80 | doc.len() as u8]; v.extend_from_slice(doc); v };
        assert_eq!(sniff(&ebml(b"webm")), Some(Container::WebM));
        assert_eq!(sniff(&ebml(b"matroska")), Some(Container::Matroska));
        assert_eq!(sniff(b"RIFF\x10\0\0\0AVI LIST"), Some(Container::Avi));
        assert_eq!(sniff(b"RIFF\x10\0\0\0WAVEfmt "), None); // a WAV is no video
        assert_eq!(sniff(&[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C, 0]), Some(Container::Asf));
        assert_eq!(sniff(b"OggS\0\x02"), Some(Container::Ogg));
        assert_eq!(sniff(b"FLV\x01\x05"), Some(Container::Flv));
        assert_eq!(sniff(&[0, 0, 1, 0xBA, 0x44]), Some(Container::MpegPs));
        let mut ts = vec![0u8; 188 * 3 + 1];
        for k in 0..3 { ts[k * 188] = 0x47; }
        assert_eq!(sniff(&ts), Some(Container::MpegTs));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn containers_by_name_and_mime() {
        assert_eq!(by_name("Urlaub.MP4"), Some(Container::Mp4));
        assert_eq!(by_name("clip.webm"), Some(Container::WebM));
        assert_eq!(by_name("a.b.mkv"), Some(Container::Matroska));
        assert_eq!(by_name("film.wmv"), Some(Container::Asf));
        assert_eq!(by_name("noext"), None);
        assert_eq!(by_name("bild.png"), None);
        assert_eq!(by_mime("video/quicktime"), Some(Container::Mov));
        assert_eq!(by_mime("video/webm; codecs=vp9"), Some(Container::WebM));
        assert_eq!(by_mime("application/octet-stream"), None);
        // the bytes win over a wrong name
        assert_eq!(detect(&ftyp(b"isom"), "clip.webm", ""), Some(Container::Mp4));
        assert_eq!(detect(b"", "clip.webm", "video/mp4"), Some(Container::WebM));
        assert_eq!(detect(b"", "clip", "video/mp4"), Some(Container::Mp4));
    }

    #[test]
    fn what_counts_as_a_video() {
        assert!(is_video("video/mp4", "x"));
        assert!(is_video("", "Aufnahme.mov"));
        assert!(is_video("application/octet-stream", "a.mkv"));
        assert!(!is_video("image/gif", "a.gif"));
        assert!(!is_video("audio/mp4", "a.m4a"));
        assert!(!is_video("audio/ogg", "a.ogv")); // the type says audio
        assert!(!is_video("application/pdf", "a.pdf"));
    }

    #[test]
    fn platform_support() {
        assert!(playable(Container::Mp4) && playable(Container::WebM) && playable(Container::Asf));
        assert!(playable(Container::Ogg));
        assert_eq!(playable(Container::Flv), !cfg!(windows));
    }

    #[test]
    fn time_texts() {
        assert_eq!(time_text(0.0), "0:00");
        assert_eq!(time_text(5.9), "0:05"); // floored
        assert_eq!(time_text(65.0), "1:05");
        assert_eq!(time_text(600.0), "10:00");
        assert_eq!(time_text(3_725.0), "62:05"); // no hours: minutes keep counting
        assert_eq!(time_text(f64::NAN), "0:00");
        assert_eq!(time_text(-3.0), "0:00");
        assert_eq!(position_text(12.4, 90.0), "0:12 / 1:30");
        assert_eq!(position_text(0.0, 0.0), "0:00 / 0:00"); // duration unknown
        assert_eq!(position_text(91.0, 90.0), "0:00 / 0:00"); // past the end
        assert_eq!(position_text(-1.0, 90.0), "0:00 / 0:00");
    }
}
