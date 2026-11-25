//! Camera video in a call (calling parity C4 send, C5 receive; docs/calling-gap.md Batch 3), in the official client's
//! order (`spark-windows-desktop-ui.dll`: "Connection::startVideoTrack()", "Connection::muteVideo()",
//! "muteVideoWithTrackOpen", "Video track not ready when muteVideo", "<update_media>: normal flow"; `call-flow-spec.md` §6):
//!
//! 1. `video/main` (sendrecv, H.264) is in the call's first offer, like the official client's four m-lines; nothing is
//!    added or renegotiated for the camera.
//! 2. On: once the call is connected on this device and its first ROAP round is done, the camera starts on `video/main`
//!    (`Media::feed_camera`: capture, the plugins' frame filters, H.264) and Locus is told `videoMuted: false`
//!    (`PUT {self.url}/media` without ROAP, `local_mute_body`); every later ROAP message carries the same flag
//!    (`roap_local_sdp_av`).
//! 3. Off: the camera stops, the line stays ("muteVideoWithTrackOpen"), `videoMuted: true`. On again: no negotiation.
//! 4. Another camera mid-call: a new feed on the same line, no negotiation; the first picture is a key frame.
//!
//! The far end's camera: a participant other than us whose `status.videoStatus` is SENDRECV / SENDONLY (`peer_video`).
//! Its pictures arrive on `video/main` and are decoded into `video_rx::MAIN`; when they come from an SSRC the answer
//! didn't declare, it is declared on our side (`Session::media_tick`, `Media::redeclare`).
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
    /// The far end's camera is on.
    remote: bool,
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

/// Why the camera can't be sent (a toast).
pub fn fail_text(f: &Fail) -> String {
    match f {
        Fail::NoCamera => crate::trq!("Video not possible: no camera found.").into_owned(),
        Fail::InUse(n) => crate::trq!("Video not possible: the camera {{0}} is used by another application.", n),
        Fail::Denied => crate::trq!("Video not possible: no access to the camera. Please allow access in the system settings.").into_owned(),
        Fail::Unplugged(n) => crate::trq!("Video stopped: the camera {{0}} is no longer connected.", n),
        Fail::NoPicture => crate::trq!("Video stopped: the camera delivers no picture.").into_owned(),
        Fail::Other(e) => crate::trq!("Video not possible: the camera doesn't start ({{0}}).", e.chars().take(120).collect::<String>()),
    }
}

/// The step to take for the camera now. Pure, for the tests.
#[derive(Debug, PartialEq)]
pub enum Step { Wait, Start }

/// `up`: the call is connected on this device with no ROAP round open; `lines`: the server answered the call's lines
/// (`video/main` among them); `want`: the user wants the camera; `feeding`: it runs. Never a negotiation.
pub fn next_step(up: bool, lines: bool, want: bool, feeding: bool) -> Step {
    if up && lines && want && !feeding { Step::Start } else { Step::Wait }
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

    /// Every tick and after each change: starts the camera once the call's media is up, follows a failing camera.
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
        let up = c.answered && c.roap_done && m.state() == MediaState::Connected;
        let want = self.calls.video_want.is_some();
        if up && want && c.video.feed.is_none() && !m.lines_negotiated() {
            // the server never answered the call's video lines: say so once, the camera stays off
            dbg_conn("call: video: the media server did not accept the video line, camera off");
            self.calls.video_want = None;
            let _ = self.out.send(Resp::Error(crate::trq!("Video not possible: the media server didn't accept this call's video line.").into_owned()));
            return self.video_sync();
        }
        if next_step(up, m.lines_negotiated(), want, c.video.feed.is_some()) == Step::Start { self.video_start(); }
        self.video_sync();
    }

    /// The lines are there: the camera starts on `video/main`, and Locus learns our video is on.
    fn video_start(&mut self) {
        let Some(key) = self.calls.video_want.clone() else { return };
        let Some(c) = self.calls.cur.as_mut() else { return };
        let Some(m) = c.media.as_ref() else { return };
        match m.feed_camera(key) {
            Ok(f) => { dbg_conn("call: video: camera starting"); c.video.feed = Some(f); }
            Err(e) => { self.calls.video_want = None; let _ = self.out.send(Resp::Error(crate::trq!("Video not possible: {{0}}", e))); return; }
        }
        self.video_mute(false);
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
        if let Err(e) = r { let _ = self.out.send(Resp::Error(if video_muted { crate::trq!("Couldn't report video off to Webex: {{0}}", scrub(&e)) } else { crate::trq!("Couldn't report video on to Webex: {{0}}", scrub(&e)) })); }
    }

    /// `videoMuted` for the ROAP messages and the mute requests of this call.
    pub(super) fn video_muted(&self) -> bool { self.calls.cur.as_ref().is_none_or(|c| c.video.muted()) }

    /// The locus changed: the far end's camera on or off.
    pub(super) fn video_locus(&mut self, l: &Value) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let on = peer_video(l, &c.self_id);
        if on == c.video.remote { return; }
        dbg_conn(&format!("call: video: the far end's camera {}", if on { "on" } else { "off" }));
        c.video.remote = on;
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
    fn the_camera_waits_for_the_call_and_never_negotiates() {
        use Step::*;
        assert_eq!(next_step(false, true, true, false), Wait, "not connected yet (or a ROAP round open)");
        assert_eq!(next_step(true, false, true, false), Wait, "the line was not answered: nothing to start on");
        assert_eq!(next_step(true, true, true, false), Start, "the first offer's video line: the camera starts, no new offer");
        assert_eq!(next_step(true, true, true, true), Wait, "running");
        assert_eq!(next_step(true, true, false, false), Wait, "camera off");
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
        assert_eq!(fail_text(&Fail::NoCamera), crate::trq!("Video not possible: no camera found."));
    }
}
