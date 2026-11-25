//! Camera video in a call (calling parity C4 send, C5 receive; docs/calling-gap.md Batch 3), in the official client's
//! order (`spark-windows-desktop-ui.dll`: "Connection::startVideoTrack()", "Connection::muteVideo()",
//! "muteVideoWithTrackOpen", "Video track not ready when muteVideo", "<update_media>: roap flow" / "<update_media>: normal
//! flow"; `call-flow-spec.md` §6):
//!
//! 1. The camera needs the video lines. Once per call one ROAP re-OFFER on the same session adds them (`video/main`
//!    sendrecv, `audio/slides`, `video/slides`; `Media::add_lines` via `Session::negotiate_lines`, shared with the screen
//!    share); the server ANSWERs, we send OK. Only with the call connected on this device and no other ROAP round open.
//! 2. The camera starts on `video/main` (`Media::feed_camera`: capture, the plugins' frame filters, H.264) and Locus is told
//!    `videoMuted: false` (`PUT {self.url}/media` without ROAP, `local_mute_body`); every later ROAP message carries the same
//!    flag (`roap_local_sdp_av`).
//! 3. Off: the camera stops, the line stays ("muteVideoWithTrackOpen"), `videoMuted: true`. On again: no negotiation.
//! 4. Another camera mid-call: a new feed on the same line, no negotiation; the first picture is a key frame.
//!
//! The far end's camera: a participant other than us whose `status.videoStatus` is SENDRECV / SENDONLY (`peer_video`) makes
//! sure the lines exist, so the server can send it; its pictures arrive on `video/main` and are decoded into
//! `video_rx::MAIN`. When they arrive from an SSRC the answer didn't declare (seen on the ICE socket, nothing taken by
//! webrtc-rs), the lines are offered once more to declare it, like a received share.
//!
//! The UI's side (call window): `CallReq::Video(Some(PreviewKey))` / `CallReq::Video(None)`, `Resp::CallVideo(VideoInfo)`,
//! the pictures from `cloud::video_rx::{remote, self_view}`.
use super::*;
use crate::media::camera::send::{CamSend, SendState};
use crate::media::camera::{Fail, PreviewKey};

/// What the call window shows about video (sent when it changes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoInfo {
    /// The user wants the camera on (`CallReq::Video(Some)`), whether or not it sends yet.
    pub wanted: bool,
    /// Our camera's pictures go out.
    pub sending: bool,
    /// The camera sent (its name; "" while none) and what is sent: width, height, pictures per second.
    pub camera: String,
    pub size: (u32, u32),
    pub fps: u32,
    /// The far end's camera is on (Locus). Its pictures: `cloud::video_rx::remote()`.
    pub remote: bool,
}

/// The video side of the current call.
#[derive(Default)]
pub struct CallVideo {
    feed: Option<CamSend>,
    /// Locus knows our camera as on (`videoMuted: false` sent).
    unmuted: bool,
    /// The far end's camera is on, since when, and whether the lines were offered again for its SSRC.
    remote: bool,
    remote_since: Option<Instant>,
    rx_reoffered: bool,
    /// The info last sent to the UI.
    shown: Option<VideoInfo>,
}

impl CallVideo {
    /// `videoMuted` for Locus: true unless our camera is sent.
    pub fn muted(&self) -> bool { !self.unmuted }
    /// Camera pictures sent so far while the camera runs (the debug log's media line).
    pub fn sent(&self) -> Option<u64> { self.feed.as_ref().map(|f| f.sent()) }
}

/// Whether a participant other than us (`self_id`) sends camera video: in the call, `status.videoStatus` SENDRECV or
/// SENDONLY (the field next to `audioStatus` the official client and webex-js-sdk read).
pub fn peer_video(l: &Value, self_id: &str) -> bool {
    l["participants"].as_array().into_iter().flatten().any(|p| {
        let me = !self_id.is_empty() && st(p, "id") == self_id;
        let user = p["type"].as_str().is_none_or(|t| t == "USER");
        !me && user && st(p, "state") == "JOINED" && matches!(p["status"]["videoStatus"].as_str(), Some("SENDRECV" | "SENDONLY"))
    })
}

/// Why the camera can't be sent (German, a toast).
pub fn fail_text(f: &Fail) -> String {
    match f {
        Fail::NoCamera => "Video nicht möglich: Keine Kamera gefunden.".into(),
        Fail::InUse(n) => format!("Video nicht möglich: Die Kamera {n} wird von einer anderen Anwendung verwendet."),
        Fail::Denied => "Video nicht möglich: Kein Zugriff auf die Kamera. Bitte den Zugriff in den Systemeinstellungen erlauben.".into(),
        Fail::Unplugged(n) => format!("Video beendet: Die Kamera {n} ist nicht mehr angeschlossen."),
        Fail::NoPicture => "Video beendet: Die Kamera liefert kein Bild.".into(),
        Fail::Other(e) => format!("Video nicht möglich: Die Kamera lässt sich nicht starten ({}).", e.chars().take(120).collect::<String>()),
    }
}

/// The step to take for the camera now. Pure, for the tests.
#[derive(Debug, PartialEq)]
pub enum Step { Wait, Negotiate, Start, Reoffer }

/// `up`: the call is connected on this device with no ROAP round open; `lines`: the video lines exist; `want`: the user
/// wants the camera; `feeding`: it runs; `remote`: the far end's camera is on; `undeclared`: its pictures arrive from an
/// SSRC webrtc-rs doesn't take (for `since` this long, `reoffered` already once).
#[allow(clippy::too_many_arguments)]
pub fn next_step(up: bool, lines: bool, want: bool, feeding: bool, remote: bool, undeclared: bool, since: Option<Duration>, reoffered: bool) -> Step {
    if !up { return Step::Wait; }
    if !lines { return if want || remote { Step::Negotiate } else { Step::Wait }; }
    if want && !feeding { return Step::Start; }
    if remote && undeclared && !reoffered && since.is_some_and(|s| s > share::RX_SSRC_WAIT) { return Step::Reoffer; }
    Step::Wait
}

impl Session {
    /// `CallReq::Video`: the camera on (this one), switched to another, or off.
    pub(super) fn video_req(&mut self, r: Option<PreviewKey>) -> Result<(), String> {
        self.calls.video_want = r.clone();
        if let Some(c) = self.calls.cur.as_mut() {
            match (&r, c.video.feed.as_ref()) {
                (None, Some(_)) => {
                    dbg_conn("call: video: camera off");
                    c.video.feed = None;
                    crate::cloud::video_rx::clear_self();
                }
                (Some(k), Some(f)) if &f.key != k => {
                    dbg_conn("call: video: camera switch");
                    c.video.feed = None; // the old device is released first (one camera open at a time)
                }
                _ => {}
            }
        }
        if r.is_none() { self.video_mute(true); }
        self.video_tick();
        Ok(())
    }

    /// Every tick and after each change: negotiates the lines when needed, starts the camera, follows a failing camera,
    /// and declares the far end's camera SSRC once when it arrives undeclared.
    pub(super) fn video_tick(&mut self) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        if let Some(SendState::Failed(f)) = c.video.feed.as_ref().map(|f| f.state()) {
            let why = fail_text(&f);
            dbg_conn(&format!("call: video: camera failed: {f:?}"));
            c.video.feed = None;
            self.calls.video_want = None;
            self.video_mute(true);
            let _ = self.out.send(Resp::Error(why));
            return self.video_sync();
        }
        let Some(m) = c.media.as_ref() else { return self.video_sync() };
        let up = c.answered && c.roap_done && c.lines_seq.is_none() && m.state() == MediaState::Connected;
        let undeclared = m.undeclared_video().1.is_some() && m.rx_main.load(Ordering::Relaxed) == 0;
        let since = c.video.remote_since.map(|t| t.elapsed());
        let step = next_step(up, m.has_lines(), self.calls.video_want.is_some(), c.video.feed.is_some(), c.video.remote, undeclared, since, c.video.rx_reoffered);
        match step {
            Step::Wait => {}
            Step::Negotiate => {
                dbg_conn(&format!("call: video: adding the video lines ({})", if self.calls.video_want.is_some() { "our camera" } else { "the far end's camera" }));
                if let Err(e) = self.negotiate_lines(false) { self.video_lines_failed(&e); }
            }
            Step::Reoffer => {
                c.video.rx_reoffered = true;
                dbg_conn("call: video: the far end's camera arrives from an undeclared SSRC, negotiating again");
                if let Err(e) = self.negotiate_lines(true) { self.lines_failed(&e); }
            }
            Step::Start => self.video_start(),
        }
        self.video_sync();
    }

    /// The lines are there: the camera starts on `video/main`, and Locus learns our video is on.
    fn video_start(&mut self) {
        let Some(key) = self.calls.video_want.clone() else { return };
        let Some(c) = self.calls.cur.as_mut() else { return };
        let Some(m) = c.media.as_ref() else { return };
        match m.feed_camera(key) {
            Ok(f) => { dbg_conn("call: video: camera starting"); c.video.feed = Some(f); }
            Err(e) => { self.calls.video_want = None; let _ = self.out.send(Resp::Error(format!("Video nicht möglich: {e}"))); return; }
        }
        self.video_mute(false);
    }

    /// Negotiating the video lines failed: a camera waiting for them is off again (the user is told once, by `lines_failed`
    /// or here).
    pub(super) fn video_lines_failed(&mut self, why: &str) {
        let waiting = self.calls.cur.as_ref().is_some_and(|c| c.video.feed.is_none()) && self.calls.video_want.is_some();
        if !waiting { return; }
        self.calls.video_want = None;
        dbg_conn(&format!("call: video: lines refused: {why}"));
        let _ = self.out.send(Resp::Error(format!("Video nicht möglich: {why}")));
        self.video_sync();
    }

    /// Tells Locus whether our camera is sent (`videoMuted`), the "normal flow" without ROAP; only when it changes.
    fn video_mute(&mut self, video_muted: bool) {
        let audio_muted = self.calls.muted;
        let Some(c) = self.calls.cur.as_mut().filter(|c| c.answered && !c.media_id.is_empty()) else { return };
        if c.video.muted() == video_muted { return; }
        c.video.unmuted = !video_muted;
        let (self_url, media_id, corr) = (c.self_url.clone(), c.media_id.clone(), c.correlation_id.clone());
        if !webex_host(&self_url) { return; }
        let r = self.http("PUT", &format!("{self_url}/media"), Some(local_mute_body(self.device_body(), &corr, &media_id, audio_muted, video_muted)));
        dbg_conn(&format!("call: video {} sent to locus -> {}", if video_muted { "off" } else { "on" }, if r.is_ok() { "ok" } else { "failed" }));
        if let Err(e) = r { let _ = self.out.send(Resp::Error(format!("Video {} konnte nicht an Webex gemeldet werden: {}", if video_muted { "aus" } else { "an" }, scrub(&e)))); }
    }

    /// `videoMuted` for the ROAP messages and the mute requests of this call.
    pub(super) fn video_muted(&self) -> bool { self.calls.cur.as_ref().is_none_or(|c| c.video.muted()) }

    /// The locus changed: the far end's camera on or off.
    pub(super) fn video_locus(&mut self, l: &Value) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let on = peer_video(l, &c.self_id);
        if on == c.video.remote { return; }
        dbg_conn(&format!("call: video: the far end's camera {}", if on { "on" } else { "off" }));
        (c.video.remote, c.video.remote_since, c.video.rx_reoffered) = (on, on.then(Instant::now), false);
        if let Some(m) = c.media.as_ref() {
            if on { m.expect.fetch_or(crate::cloud::media::EXPECT_VIDEO, Ordering::Relaxed); } else { m.expect.fetch_and(!crate::cloud::media::EXPECT_VIDEO, Ordering::Relaxed); }
        }
        if !on { crate::cloud::video_rx::clear_remote(); }
        self.video_tick();
    }

    /// The call ends: camera off, pictures gone, the UI told.
    pub(super) fn video_end(&mut self) {
        if let Some(c) = self.calls.cur.as_mut() { c.video.feed = None; }
        self.calls.video_want = None;
        crate::cloud::video_rx::clear_remote();
        crate::cloud::video_rx::clear_self();
        let _ = self.out.send(Resp::CallVideo(VideoInfo::default()));
    }

    /// Sends `Resp::CallVideo` when what the UI shows changed.
    pub(super) fn video_sync(&mut self) {
        let wanted = self.calls.video_want.is_some();
        let Some(c) = self.calls.cur.as_mut() else { return };
        let mut info = VideoInfo { wanted, remote: c.video.remote, ..Default::default() };
        if let Some(SendState::Running { name, w, h, fps }) = c.video.feed.as_ref().map(|f| f.state()) {
            (info.sending, info.camera, info.size, info.fps) = (true, name, (w, h), fps);
        }
        if c.video.shown.as_ref() == Some(&info) { return; }
        c.video.shown = Some(info.clone());
        let _ = self.out.send(Resp::CallVideo(info));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_far_ends_camera_from_the_locus() {
        let l = |status: &str, state: &str| json!({"participants": [
            {"id": "me", "state": "JOINED", "status": {"audioStatus": "SENDRECV", "videoStatus": "SENDRECV"}},
            {"id": "peer", "type": "USER", "state": state, "status": {"audioStatus": "SENDRECV", "videoStatus": status}}]});
        assert!(peer_video(&l("SENDRECV", "JOINED"), "me"));
        assert!(peer_video(&l("SENDONLY", "JOINED"), "me"));
        assert!(!peer_video(&l("RECVONLY", "JOINED"), "me"), "camera off: receives only");
        assert!(!peer_video(&l("INACTIVE", "JOINED"), "me"));
        assert!(!peer_video(&l("SENDRECV", "LEFT"), "me"), "left the call");
        assert!(!peer_video(&json!({"participants": [{"id": "me", "state": "JOINED", "status": {"videoStatus": "SENDRECV"}}]}), "me"), "our own camera is not the far end's");
        assert!(!peer_video(&json!({}), "me"));
    }

    #[test]
    fn the_camera_waits_for_the_call_and_negotiates_once() {
        use Step::*;
        let s = Some(Duration::from_secs(5));
        assert_eq!(next_step(false, false, true, false, false, false, None, false), Wait, "not connected yet (or a ROAP round open)");
        assert_eq!(next_step(true, false, true, false, false, false, None, false), Negotiate, "camera on: lines first");
        assert_eq!(next_step(true, false, false, false, true, false, None, false), Negotiate, "far end's camera: lines to receive it");
        assert_eq!(next_step(true, false, false, false, false, false, None, false), Wait, "audio call: no video lines");
        assert_eq!(next_step(true, true, true, false, false, false, None, false), Start, "lines there: camera starts, no new offer");
        assert_eq!(next_step(true, true, true, true, false, false, None, false), Wait, "running");
        assert_eq!(next_step(true, true, false, false, true, true, s, false), Reoffer, "far end's camera undeclared: offer once more");
        assert_eq!(next_step(true, true, false, false, true, true, s, true), Wait, "only once");
        assert_eq!(next_step(true, true, false, false, true, true, Some(Duration::from_secs(1)), false), Wait, "not at once");
    }

    #[test]
    fn mute_bodies_carry_the_cameras_state() {
        let on: Value = serde_json::from_str(&roap_local_sdp_av("OFFER", Some("v=0\r\n"), 2, false, false)).unwrap();
        assert_eq!((on["audioMuted"].as_bool(), on["videoMuted"].as_bool()), (Some(false), Some(false)));
        let off: Value = serde_json::from_str(&roap_local_sdp("OK", None, 2, true)).unwrap();
        assert_eq!(off["videoMuted"].as_bool(), Some(true), "camera off by default");
        let b = local_mute_body(json!({"url": "d"}), "c", "m", false, false);
        let sdp: Value = serde_json::from_str(b["localMedias"][0]["localSdp"].as_str().unwrap()).unwrap();
        assert_eq!(sdp, json!({"audioMuted": false, "videoMuted": false}));
        assert!(fail_text(&Fail::NoCamera).contains("Keine Kamera"));
    }
}
