//! A screen share received in a call: the far end's H.264 on the `video/slides` line, decoded off the UI thread, the newest
//! picture kept for the call window.
//!
//! Like the official client (`wmeclient.dll`: `CWmeRemoteVideoTrack::OnKeyFrameLost` -> `CWmeMediaSession::IndicatePictureLoss`,
//! repeated on `OnKeyFrameLostTimeout`): a picture loss indication (RTCP PLI) goes out when the stream starts, when packets
//! were lost or a picture didn't decode, and again every `KEYFRAME_RETRY` until a picture decodes. The decoder is OpenH264,
//! which the official client ships as well (`openh264.dll`).
//!
//! Cost: one decoder thread while pictures come; the UI gets the newest picture only (older ones are dropped, never
//! queued) and is woken once per decoded picture, so it never redraws faster than the share sends; with no share nothing
//! here runs and nothing wakes the UI.
use super::dbg_conn;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Pictures larger than this are not kept (the share encoder of any client stays below it; guards the RGBA buffer).
const MAX_PIXELS: usize = 4096 * 2304;
/// A key frame is asked for again after this long without a decodable picture.
pub const KEYFRAME_RETRY: Duration = Duration::from_millis(1000);

/// One decoded picture, made into an image on the decoder thread: the UI hands the same `Arc` to the GPU, no copy there.
pub struct Picture { pub image: Arc<eframe::egui::ColorImage>, pub seq: u64 }

static LATEST: Mutex<Option<Arc<Picture>>> = Mutex::new(None);
static SEQ: AtomicU64 = AtomicU64::new(0);

/// The newest decoded picture of the received share (None before the first one, and after `clear`).
pub fn latest() -> Option<Arc<Picture>> { LATEST.lock().ok().and_then(|g| g.clone()) }

/// The received share ended: the picture goes (the call window returns to its normal layout).
pub fn clear() { if let Ok(mut g) = LATEST.lock() { *g = None; } }

fn publish(w: usize, h: usize, rgba: &[u8]) {
    let image = Arc::new(eframe::egui::ColorImage::from_rgba_unmultiplied([w, h], rgba));
    let seq = SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    if let Ok(mut g) = LATEST.lock() { *g = Some(Arc::new(Picture { image, seq })); }
    crate::ui::wake();
}

/// `WEBOX_SAMPLE_CALL=share` (offline sample): a made-up received picture (`media::share::convert::test_picture`).
pub fn show_sample(w: u32, h: u32) {
    let mut px = vec![0u8; (w * h * 4) as usize];
    crate::media::share::convert::test_picture(0, w, h, &mut px);
    publish(w as usize, h as usize, &px);
}

/// Who shares in this locus when it isn't us: the `content` media share's floor GRANTED to another participant that is
/// still in the call. Returns that participant's name ("" when the locus has none).
pub fn remote_sharer(l: &Value, self_url: &str) -> Option<String> {
    let st = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    let share = l["mediaShares"].as_array()?.iter().find(|m| st(m, "name") == "content")?;
    let floor = &share["floor"];
    if st(floor, "disposition") != "GRANTED" { return None; }
    let ben = &floor["beneficiary"];
    let (url, id) = (st(ben, "url"), st(ben, "id"));
    if url.is_empty() && id.is_empty() { return None; }
    if !url.is_empty() && url == self_url { return None; }
    let p = l["participants"].as_array().and_then(|ps| ps.iter().find(|p| (!url.is_empty() && st(p, "url") == url) || (!id.is_empty() && st(p, "id") == id)));
    if let Some(p) = p {
        let devices: Vec<String> = p["devices"].as_array().into_iter().flatten().map(|d| st(d, "state")).collect();
        let left = st(p, "state") == "LEFT" || st(p, "state") == "DECLINED" || (!devices.is_empty() && devices.iter().all(|d| d == "LEFT"));
        if left { return None; }
        return Some(st(&p["person"], "name"));
    }
    Some(String::new())
}

/// The decode side of a received share: access units in, pictures out (`latest`). `keyframe` is raised when a key frame
/// is needed (the RTP reader sends the PLI).
pub struct Decode { tx: SyncSender<Vec<u8>>, pub keyframe: Arc<AtomicBool>, pub decoded: Arc<AtomicU64> }

impl Decode {
    /// Starts the decoder thread (it ends when the `Decode` is dropped).
    pub fn start() -> Decode {
        let (tx, rx) = sync_channel::<Vec<u8>>(3);
        let keyframe = Arc::new(AtomicBool::new(true)); // a key frame first
        let decoded = Arc::new(AtomicU64::new(0));
        let (k, n) = (keyframe.clone(), decoded.clone());
        let _ = std::thread::Builder::new().name("webox-share-decode".into()).spawn(move || decode_loop(rx, k, n));
        Decode { tx, keyframe, decoded }
    }
    /// One access unit (Annex B). A full queue drops it and asks for a key frame: the UI shows the newest picture only.
    pub fn push(&self, au: Vec<u8>) {
        match self.tx.try_send(au) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => self.keyframe.store(true, Ordering::SeqCst),
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

fn decode_loop(rx: Receiver<Vec<u8>>, keyframe: Arc<AtomicBool>, decoded: Arc<AtomicU64>) {
    let mut dec = match openh264::decoder::Decoder::new() { Ok(d) => d, Err(e) => { dbg_conn(&format!("call: share decoder failed to start: {e}")); return } };
    let mut rgba: Vec<u8> = vec![];
    while let Ok(au) = rx.recv() {
        let mut got = None;
        let mut failed = false;
        for nal in openh264::nal_units(&au) {
            match dec.decode(nal) {
                Ok(Some(yuv)) => {
                    use openh264::formats::YUVSource;
                    let (w, h) = yuv.dimensions();
                    if w == 0 || h == 0 || w * h > MAX_PIXELS { failed = true; continue; }
                    rgba.resize(w * h * 4, 0);
                    yuv.write_rgba8(&mut rgba);
                    got = Some((w, h));
                }
                Ok(None) => {}
                Err(_) => failed = true,
            }
        }
        if failed { keyframe.store(true, Ordering::SeqCst); }
        if let Some((w, h)) = got {
            if decoded.fetch_add(1, Ordering::Relaxed) == 0 { dbg_conn(&format!("call: share: first picture decoded ({w}x{h})")); }
            publish(w, h, &rgba);
        }
    }
}

/// When to send the next PLI: at once when one is wanted and none went out for `KEYFRAME_RETRY`, and again every
/// `KEYFRAME_RETRY` while no picture has decoded yet. Pure, for the tests.
pub fn pli_due(wanted: bool, decoded_any: bool, last: Option<Instant>, now: Instant) -> bool {
    let quiet = last.is_none_or(|t| now.saturating_duration_since(t) >= KEYFRAME_RETRY);
    (wanted || !decoded_any) && quiet
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn locus(disp: &str, ben_url: &str, sharer_state: &str) -> Value {
        json!({"participants": [
                {"id": "self-1", "url": "https://locus/p/self", "state": "JOINED", "person": {"name": "Me"}},
                {"id": "peer-1", "url": "https://locus/p/peer", "state": sharer_state, "person": {"name": "Andrin Muster"}, "devices": [{"state": sharer_state}]}],
            "mediaShares": [{"name": "whiteboard", "floor": {"disposition": "GRANTED", "beneficiary": {"url": "https://locus/p/peer"}}},
                {"name": "content", "url": "https://locus/m/c", "floor": {"disposition": disp, "beneficiary": {"url": ben_url}}}]})
    }

    #[test]
    fn remote_floor_turns_the_stage_on_and_off() {
        let me = "https://locus/p/self";
        assert_eq!(remote_sharer(&locus("GRANTED", "https://locus/p/peer", "JOINED"), me).as_deref(), Some("Andrin Muster"), "remote GRANTED: stage on");
        assert_eq!(remote_sharer(&locus("RELEASED", "https://locus/p/peer", "JOINED"), me), None, "RELEASED: stage off");
        assert_eq!(remote_sharer(&locus("GRANTED", me, "JOINED"), me), None, "our own share is not shown to us");
        assert_eq!(remote_sharer(&locus("GRANTED", "https://locus/p/peer", "LEFT"), me), None, "the sharer left: stage off");
        assert_eq!(remote_sharer(&json!({"participants": []}), me), None, "no media shares");
        assert_eq!(remote_sharer(&locus("GRANTED", "https://locus/p/unknown", "JOINED"), me).as_deref(), Some(""), "a sharer not in the list still shows");
    }

    #[test]
    fn key_frames_are_asked_for_at_a_bounded_rate() {
        let t0 = Instant::now();
        assert!(pli_due(false, false, None, t0), "at the start");
        assert!(!pli_due(false, false, Some(t0), t0 + Duration::from_millis(300)), "not again at once");
        assert!(pli_due(false, false, Some(t0), t0 + Duration::from_millis(1100)), "again while nothing decodes");
        assert!(!pli_due(false, true, Some(t0), t0 + Duration::from_secs(5)), "decoding: only when wanted");
        assert!(pli_due(true, true, Some(t0), t0 + Duration::from_secs(5)), "loss: wanted");
    }
}
