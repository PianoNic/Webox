//! The Webex sounds (`assets/cisco/sounds`, git-ignored, listed in `assets/sounds/manifest.json`), a small pure-Rust WAV
//! decoder (all of them are plain 16-bit PCM) and the user's sound preferences (persisted in the settings JSON, keys `snd_*`).
//!
//! The call ringtone is a policy (`incoming-call-alert-sound_1`, default "Delight"), selectable among Delight / Sunrise /
//! Beep; ringer volume 50 %; the ringer can play on all output devices or on one chosen "Klingeltöne und Warnungen" device;
//! message notification sounds come from the "momentum_ringtones" group (Open, Tick), selectable separately for direct
//! messages, spaces and favourite spaces. A declined call ends with BusyTone, a failed one with FastBusy.
use crate::media::audio::{Lin, RATE};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind { CallRingtone, MessageSound, Ringback, CallEnd, Dtmf, Alert, Other }

#[allow(dead_code)] // kind / looped document the manifest and are checked by tests
pub struct Sound { pub id: &'static str, pub kind: Kind, pub looped: bool, pub file: &'static str }

impl Sound {
    /// The WAV from the Webex client files (`assets/cisco/sounds`, else an installed Webex app); empty when neither has it.
    pub fn bytes(&self) -> &'static [u8] { crate::assets::cisco_assets::get(&format!("sounds/{}", self.file)).unwrap_or(&[]) }
}

macro_rules! snd { ($id:literal, $kind:expr, $loop:expr, $file:literal) => {
    Sound { id: $id, kind: $kind, looped: $loop, file: $file } } }

pub const SOUNDS: [Sound; 14] = [
    snd!("Delight", Kind::CallRingtone, true, "Delight.wav"), snd!("Sunrise", Kind::CallRingtone, true, "Sunrise.wav"), snd!("Beep", Kind::CallRingtone, true, "Beep.wav"),
    snd!("Open", Kind::MessageSound, false, "Open.wav"), snd!("Tick", Kind::MessageSound, false, "Tick.wav"),
    snd!("Ringback", Kind::Ringback, true, "Ringback.wav"),
    snd!("BusyTone", Kind::CallEnd, false, "BusyTone.wav"), snd!("FastBusy", Kind::CallEnd, false, "FastBusy.wav"),
    snd!("DTMF_Tone", Kind::Dtmf, false, "DTMF_Tone.wav"), snd!("VoicemailBeep", Kind::Other, false, "VoicemailBeep.wav"),
    snd!("GroupCallInvite", Kind::Alert, false, "GroupCallInvite.wav"), snd!("Reconnect", Kind::Alert, false, "Reconnect.wav"),
    snd!("ClosedCaptionActiveDisclaimer", Kind::Alert, false, "ClosedCaptionActiveDisclaimer.wav"), snd!("RecordingStartDisclaimer", Kind::Alert, false, "RecordingStartDisclaimer.wav"),
];
pub const CALL_RINGTONES: [&str; 3] = ["Delight", "Sunrise", "Beep"];
pub const MESSAGE_SOUNDS: [&str; 2] = ["Open", "Tick"];

pub fn get(id: &str) -> Option<&'static Sound> { SOUNDS.iter().find(|s| s.id == id) }

#[derive(Debug)]
pub struct Pcm { pub rate: u32, pub channels: u16, pub data: Vec<i16> }

/// Minimal RIFF/WAVE reader: 16-bit PCM only (what every bundled sound is).
pub fn decode_wav(b: &[u8]) -> Result<Pcm, String> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" { return Err("not a WAV file".into()); }
    let (mut pos, mut fmt, mut data) = (12usize, None, None);
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let len = u32::from_le_bytes([b[pos + 4], b[pos + 5], b[pos + 6], b[pos + 7]]) as usize;
        let body = &b[pos + 8..(pos + 8 + len).min(b.len())];
        match id { b"fmt " if body.len() >= 16 => fmt = Some((u16::from_le_bytes([body[0], body[1]]), u16::from_le_bytes([body[2], body[3]]), u32::from_le_bytes([body[4], body[5], body[6], body[7]]), u16::from_le_bytes([body[14], body[15]]))), b"data" => data = Some(body), _ => {} }
        pos += 8 + len + (len & 1);
    }
    let ((tag, ch, rate, bits), data) = (fmt.ok_or("no fmt chunk")?, data.ok_or("no data chunk")?);
    if tag != 1 || bits != 16 || ch == 0 { return Err(format!("unsupported WAV (format {tag}, {bits} bit)")); }
    Ok(Pcm { rate, channels: ch, data: data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect() })
}

/// 48 kHz mono, which is what the audio engine plays.
pub fn to_mono48(p: &Pcm) -> Vec<i16> {
    let ch = p.channels as usize;
    let mono: Vec<f32> = p.data.chunks_exact(ch).map(|f| f.iter().map(|&s| s as f32).sum::<f32>() / ch as f32 / i16::MAX as f32).collect();
    let mut out = vec![];
    Lin::new(p.rate, RATE).push(&mono, &mut out);
    out.iter().map(|v| (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect()
}

/// Prefix of a sound id that is a user-chosen file (`file:<absolute path>`), see `decode_file`.
pub const FILE_PREFIX: &str = "file:";
/// Custom sounds: at most 10 MB; ringtones at most 60 s (the settings page shows the German error).
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_RINGTONE_SECS: f32 = 60.0;

pub fn file_id(path: &std::path::Path) -> String { format!("{FILE_PREFIX}{}", path.display()) }
pub fn file_path(id: &str) -> Option<&str> { id.strip_prefix(FILE_PREFIX) }

/// Any wav / mp3 / ogg / flac from memory. WAV 16-bit PCM takes the small built-in fast path; everything else goes through
/// symphonia (pure Rust). `ext` is only a probing hint.
pub fn decode_any(bytes: Vec<u8>, ext: Option<&str>) -> Result<Pcm, String> {
    if let Ok(p) = decode_wav(&bytes) { return Ok(p); }
    use symphonia::core::{audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};
    let bad = || "Diese Datei kann nicht abgespielt werden (erlaubt: WAV, MP3, OGG, FLAC).".to_string();
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = ext { hint.with_extension(e); }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).map_err(|_| bad())?;
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL).ok_or_else(bad)?;
    let (id, params) = (track.id, track.codec_params.clone());
    let mut dec = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).map_err(|_| bad())?;
    let (mut data, mut rate, mut ch) = (Vec::<i16>::new(), params.sample_rate.unwrap_or(0), params.channels.map_or(0, |c| c.count()));
    let mut sb: Option<SampleBuffer<i16>> = None;
    loop {
        let pk = match format.next_packet() { Ok(p) => p, Err(Error::IoError(_)) | Err(Error::ResetRequired) => break, Err(_) => return Err(bad()) };
        if pk.track_id() != id { continue; }
        match dec.decode(&pk) {
            Ok(buf) => {
                let spec = *buf.spec();
                rate = spec.rate; ch = spec.channels.count();
                if sb.as_ref().map_or(true, |s| s.capacity() < buf.capacity() * ch) { sb = Some(SampleBuffer::<i16>::new(buf.capacity() as u64, spec)); }
                if let Some(s) = sb.as_mut() { s.copy_interleaved_ref(buf); data.extend_from_slice(s.samples()); }
            }
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }
    if data.is_empty() || rate == 0 || ch == 0 { return Err(bad()); }
    Ok(Pcm { rate, channels: ch as u16, data })
}

pub fn duration_secs(p: &Pcm) -> f32 { p.data.len() as f32 / p.channels.max(1) as f32 / p.rate.max(1) as f32 }

/// Reads and checks a user's file: size <= 10 MB, and for ringtones (`max_secs`) at most 60 s. German error texts.
pub fn decode_file(path: &std::path::Path, max_secs: Option<f32>) -> Result<Pcm, String> {
    let meta = std::fs::metadata(path).map_err(|_| "Die Datei wurde nicht gefunden.".to_string())?;
    if meta.len() > MAX_FILE_BYTES { return Err("Die Datei ist zu groß (höchstens 10 MB).".into()); }
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase());
    if !matches!(ext.as_deref(), Some("wav" | "mp3" | "ogg" | "flac")) { return Err("Nicht unterstütztes Format (erlaubt: WAV, MP3, OGG, FLAC).".into()); }
    let bytes = std::fs::read(path).map_err(|_| "Die Datei kann nicht gelesen werden.".to_string())?;
    let p = decode_any(bytes, ext.as_deref())?;
    if let Some(m) = max_secs { if duration_secs(&p) > m { return Err(format!("Der Klingelton darf höchstens {} Sekunden lang sein.", m as u32)); } }
    Ok(p)
}

/// Decoded (cached) 48 kHz mono samples of a built-in sound or of a custom file (`file:<path>`). A custom file that
/// is gone or unreadable gives `None` (callers then use the default sound, see `Prefs::load`).
pub fn pcm(id: &str) -> Option<std::sync::Arc<Vec<i16>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, std::sync::Arc<Vec<i16>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(p) = cache.lock().ok()?.get(id) { return Some(p.clone()); }
    let pcm = match file_path(id) { Some(path) => decode_file(std::path::Path::new(path), None).ok()?, None => decode_wav(get(id)?.bytes()).ok()? };
    let v = std::sync::Arc::new(to_mono48(&pcm));
    cache.lock().ok()?.insert(id.to_string(), v.clone());
    Some(v)
}

/// A sound id that still exists: built-in ids and present files stay, a vanished custom file becomes `default`.
pub fn valid_or(id: &str, default: &str) -> String {
    match file_path(id) { Some(p) if !std::path::Path::new(p).is_file() => default.to_string(), _ => id.to_string() }
}
/// The user's sound settings. Empty sound id = "Kein Ton".
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub ringtone: String, pub ringer_volume: f32, pub ring_all_devices: bool, pub call_sounds: bool,
    pub msg_dm: String, pub msg_space: String, pub msg_favorite: String, pub msg_volume: f32, pub msg_sounds: bool,
}
impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { ringtone: "Delight".into(), ringer_volume: 0.5, ring_all_devices: false, call_sounds: true,
            msg_dm: "Open".into(), msg_space: "Open".into(), msg_favorite: "Open".into(), msg_volume: 0.5, msg_sounds: true }
    }
}
impl Prefs {
    pub fn load(s: &serde_json::Value) -> Prefs {
        let d = Prefs::default();
        let t = |k: &str, dv: &str| s[k].as_str().map_or(dv.to_string(), String::from);
        let f = |k: &str, dv: f32| s[k].as_f64().map_or(dv, |v| v.clamp(0.0, 1.0) as f32);
        let b = |k: &str, dv: bool| s[k].as_bool().unwrap_or(dv);
        Prefs { ringtone: valid_or(&t("snd_ringtone", &d.ringtone), "Delight"), ringer_volume: f("snd_ringer_volume", d.ringer_volume), ring_all_devices: b("snd_ring_all", d.ring_all_devices),
            call_sounds: b("snd_call_sounds", true), msg_dm: valid_or(&t("snd_msg_dm", &d.msg_dm), "Open"), msg_space: valid_or(&t("snd_msg_space", &d.msg_space), "Open"),
            msg_favorite: valid_or(&t("snd_msg_favorite", &d.msg_favorite), "Open"), msg_volume: f("snd_msg_volume", d.msg_volume), msg_sounds: b("snd_msg_sounds", true) }
    }
    pub fn store(&self, s: &mut serde_json::Value) {
        s["snd_ringtone"] = self.ringtone.clone().into(); s["snd_ringer_volume"] = (self.ringer_volume as f64).into();
        s["snd_ring_all"] = self.ring_all_devices.into(); s["snd_call_sounds"] = self.call_sounds.into();
        s["snd_msg_dm"] = self.msg_dm.clone().into(); s["snd_msg_space"] = self.msg_space.clone().into(); s["snd_msg_favorite"] = self.msg_favorite.clone().into();
        s["snd_msg_volume"] = (self.msg_volume as f64).into(); s["snd_msg_sounds"] = self.msg_sounds.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The WAVs are Webex client files (docs/cisco-assets.md); tests that play them skip when neither the build nor
    /// an installed Webex app has them.
    fn have_sounds() -> bool { SOUNDS.iter().all(|s| !s.bytes().is_empty()) }

    #[test]
    fn every_sound_decodes_and_has_energy() {
        if !have_sounds() { return; }
        for s in &SOUNDS {
            let p = decode_wav(s.bytes()).unwrap_or_else(|e| panic!("{}: {e}", s.id));
            assert!(p.rate >= 8000 && (1..=2).contains(&p.channels), "{}", s.id);
            let m = pcm(s.id).unwrap();
            assert!(m.len() > 4000, "{} too short", s.id);
            assert!(crate::media::audio::rms(&m) > 0.01, "{} is silent", s.id);
        }
    }
    #[test]
    fn durations_match_the_originals() {
        if !have_sounds() { return; }
        let secs = |id| pcm(id).unwrap().len() as f32 / RATE as f32;
        assert!((secs("Delight") - 4.0).abs() < 0.1 && (secs("Sunrise") - 10.0).abs() < 0.1 && (secs("Ringback") - 4.0).abs() < 0.1);
        assert!((secs("BusyTone") - 0.7).abs() < 0.1);
    }
    #[test]
    fn manifest_lists_exactly_the_embedded_sounds() {
        let m: serde_json::Value = serde_json::from_str(include_str!("../../assets/sounds/manifest.json")).unwrap();
        let mut ids: Vec<&str> = m["sounds"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
        let mut have: Vec<&str> = SOUNDS.iter().map(|s| s.id).collect();
        ids.sort(); have.sort();
        assert_eq!(ids, have);
        for s in m["sounds"].as_array().unwrap() {
            let id = s["id"].as_str().unwrap();
            assert_eq!(s["loop"].as_bool().unwrap(), get(id).unwrap().looped, "{id}");
            assert_eq!(s["file"].as_str().unwrap(), get(id).unwrap().file, "{id}");
        }
        assert!(CALL_RINGTONES.iter().chain(MESSAGE_SOUNDS.iter()).all(|i| get(i).is_some()));
    }
    #[test]
    fn rejects_garbage_and_prefs_round_trip() {
        assert!(decode_wav(b"nope").is_err());
        let mut v = serde_json::json!({});
        let p = Prefs { ringtone: "Sunrise".into(), ringer_volume: 0.8, ring_all_devices: true, call_sounds: false, msg_dm: String::new(), ..Prefs::default() };
        p.store(&mut v);
        assert_eq!(Prefs::load(&v), p);
        assert_eq!(Prefs::load(&serde_json::json!({})).ringtone, "Delight");
    }
    fn fixture(n: &str) -> std::path::PathBuf { std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/test_sounds").join(n) }
    #[test]
    fn decodes_wav_mp3_ogg_flac_and_24bit_wav() {
        for n in ["tone.ogg", "tone.flac", "tone.mp3", "tone24.wav"] {
            let p = decode_file(&fixture(n), Some(60.0)).unwrap_or_else(|e| panic!("{n}: {e}"));
            let d = duration_secs(&p);
            assert!((0.25..0.45).contains(&d), "{n}: {d} s");
            assert!(p.rate == 44100 || p.rate == 48000, "{n}");
        }
        let p = decode_file(&fixture("tone.flac"), None).unwrap();
        assert!(!to_mono48(&p).is_empty());
    }
    #[test]
    fn original_mp3_is_the_same_audio_as_the_wav() {
        let orig = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/cisco/test_sounds/dtmf_orig.mp3");
        if !have_sounds() || !orig.exists() { return; }
        let wav = duration_secs(&decode_wav(get("DTMF_Tone").unwrap().bytes()).unwrap());
        let mp3 = duration_secs(&decode_file(&orig, None).unwrap());
        assert!((wav - mp3).abs() < 0.15, "wav {wav} s, mp3 {mp3} s");
    }
    #[test]
    fn custom_file_validation_and_fallback() {
        assert!(decode_file(&fixture("nope.wav"), None).unwrap_err().contains("nicht gefunden"));
        assert!(decode_file(&fixture("tone.ogg"), Some(0.1)).unwrap_err().contains("Sekunden"));
        let dir = std::env::temp_dir().join("webox_snd_test"); let _ = std::fs::create_dir_all(&dir);
        let big = dir.join("big.wav"); std::fs::write(&big, vec![0u8; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        assert!(decode_file(&big, None).unwrap_err().contains("zu groß"));
        let txt = dir.join("a.txt"); std::fs::write(&txt, b"hi").unwrap();
        assert!(decode_file(&txt, None).unwrap_err().contains("Format"));
        let _ = std::fs::remove_dir_all(&dir);
        let v = serde_json::json!({"snd_ringtone": "file:C:/does/not/exist.mp3", "snd_msg_dm": "file:C:/nope.ogg"});
        let p = Prefs::load(&v);
        assert_eq!((p.ringtone.as_str(), p.msg_dm.as_str()), ("Delight", "Open"));
        let ok = serde_json::json!({"snd_ringtone": file_id(&fixture("tone.ogg"))});
        assert!(Prefs::load(&ok).ringtone.starts_with(FILE_PREFIX));
        assert!(pcm(&file_id(&fixture("tone.mp3"))).is_some());
    }
    /// Plays through the real engine code path into the playback buffer without a device: queue + gain logic only.
    #[test]
    fn loopback_queue_holds_a_whole_ringtone() {
        let io = crate::media::audio::Io::default();
        // A ringtone-length tone (5 s at 48 kHz); the bundled ringtones are not in every checkout.
        let r: Vec<i16> = (0..48_000 * 5).map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16).collect();
        io.play_long(&r);
        assert_eq!(io.play_pending(), r.len()); // not truncated to the 1 s call buffer
    }
}
