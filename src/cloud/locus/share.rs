//! Screen share in a call, the sending side, in the official client's order (strings and functions of
//! `spark-windows-desktop-ui.dll`: "Connect on spark device before requesting floor", "<update_media>: roap flow",
//! "Share perform floor request", "Perform floor: Granted / Released for shareType = DesktopShare, shareUrl = …"):
//!
//! 1. only in a call that is connected on this device;
//! 2. the capture starts (`media::share`: Windows Graphics Capture / DXGI, the Wayland portal + PipeWire, X11,
//!    ScreenCaptureKit) and must deliver pictures;
//! 3. the share's media lines are negotiated once per call on the same ROAP session (a re-OFFER with `video/main`,
//!    `audio/slides`, `video/slides`; `Media::add_share`), the server ANSWERs, we send OK;
//! 4. the floor of the locus' `content` media share is requested: `PUT {mediaShares[content].url}` with
//!    `floor {disposition: GRANTED, requester, beneficiary {url, devices}}`;
//! 5. pictures go out on `video/slides`; the locus then shows the floor GRANTED to us. The official client treats a
//!    missing GRANTED as granted after a timeout ("Send floor granted notification event due to not receive floor granted
//!    from mercury dto"), so does Webox. When the floor goes to someone else or is released, the share stops
//!    ("Floor released while sharing. Remove share source.").
//!
//! Stopping releases the floor (`disposition: RELEASED`); a 404 / 409 answer to that is ignored like the official client
//! does ("Skip 409 conflict failure caused by floor release request", "Skip 404 not found failure …").
//! Every failure ends the share and tells the user why (German toast); nothing is swallowed.
use super::*;
use crate::cloud::media::ShareFeed;
use crate::media::share::{self as capture, Opts, ShareSource, ShareStream, Status};

/// What the user picked in the share dialog.
#[derive(Clone, Debug)]
pub struct ShareReq { pub source: ShareSource, pub opts: Opts }

/// The capture must deliver its first picture within this time (the Wayland portal dialog waits for the user).
const CAPTURE_START: Duration = Duration::from_secs(90);
/// The floor GRANTED should show in the locus by then; after it the share counts as granted anyway.
const FLOOR_GRANT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase { Capturing, Negotiating, Requesting, Sharing }

pub struct Share {
    stream: Arc<std::sync::Mutex<ShareStream>>,
    feed: Option<ShareFeed>,
    /// The ROAP seq of our share re-OFFER while its ANSWER is awaited.
    pub(super) offer_seq: Option<u32>,
    floor_url: String,
    phase: Phase,
    since: Instant,
    granted: bool,
}

impl Share {
    /// Pictures sent so far (the debug log's media line).
    pub fn sent(&self) -> u64 { self.feed.as_ref().map_or(0, |f| f.sent.load(std::sync::atomic::Ordering::Relaxed)) }
    /// Ends the capture and the feed (no request; the floor is released by `Session::share_stop`).
    pub fn stop(self) {
        if let Ok(s) = self.stream.lock() { s.cancel(); }
        if let Some(f) = self.feed { f.stop(); }
    }
}

/// The url of the locus' `content` media share (the floor a screen share asks for).
pub fn content_share_url(l: &Value) -> Option<String> {
    l["mediaShares"].as_array()?.iter().find(|m| st(m, "name") == "content").map(|m| st(m, "url")).filter(|u| !u.is_empty())
}

/// The `content` floor: (disposition, beneficiary url).
pub fn content_floor(l: &Value) -> (String, String) {
    let m = l["mediaShares"].as_array().and_then(|a| a.iter().find(|m| st(m, "name") == "content")).cloned().unwrap_or(Value::Null);
    (st(&m["floor"], "disposition"), st(&m["floor"]["beneficiary"], "url"))
}

/// `PUT {mediaShare.url}` body: request (`GRANTED`) or release (`RELEASED`) the floor for this device.
pub fn floor_body(disposition: &str, self_url: &str, device_url: &str, device_type: &str) -> Value {
    json!({"floor": {"disposition": disposition, "requester": {"url": self_url},
        "beneficiary": {"url": self_url, "devices": [{"deviceType": device_type, "url": device_url}]}}})
}

/// The German reason for a failed floor request.
pub fn floor_error(e: &str) -> String {
    match http_status(e) {
        Some(409) => "Freigabe nicht möglich: Gerade gibt jemand anderes Inhalte frei.".into(),
        Some(403) => "Freigabe nicht erlaubt: Die Richtlinie deiner Organisation verbietet das Freigeben in diesem Anruf.".into(),
        Some(404) => "Freigabe nicht möglich: Webex kennt den Freigabe-Kanal dieses Anrufs nicht (mehr).".into(),
        _ => format!("Freigabe-Anfrage an Webex fehlgeschlagen: {}", scrub(e)),
    }
}

/// Why a share can't start in this call, or None.
pub fn share_blocked(state: Option<&CallState>, answered: bool, has_media: bool, roap_busy: bool, content_url: Option<&str>) -> Option<&'static str> {
    if !answered || !has_media || state != Some(&CallState::Connected) { return Some("Bildschirmfreigabe ist erst möglich, wenn der Anruf verbunden ist."); }
    if content_url.is_none() { return Some("Freigabe nicht möglich: Dieser Anruf bietet keinen Freigabe-Kanal an (kein „content“ in Locus)."); }
    if roap_busy { return Some("Freigabe nicht möglich: Die Medienverbindung wird gerade noch ausgehandelt. Bitte gleich nochmals versuchen."); }
    None
}

impl Session {
    /// `Some`: start sharing what the user picked; `None`: stop sharing.
    pub(super) fn share_req(&mut self, r: Option<ShareReq>) -> Result<(), String> {
        match r {
            None => { self.share_stop(); Ok(()) }
            Some(r) => self.share_start(r),
        }
    }

    fn share_start(&mut self, r: ShareReq) -> Result<(), String> {
        let Some(c) = self.calls.cur.as_mut() else { return Err("Bildschirmfreigabe ist nur in einem Anruf möglich.".into()) };
        if c.share.is_some() { return Ok(()); }
        let url = content_share_url(&c.locus);
        if let Some(why) = share_blocked(c.last.as_ref(), c.answered, c.media.is_some(), !c.roap_done, url.as_deref()) {
            dbg_conn(&format!("call: share refused: {why}"));
            return Err(why.into());
        }
        let floor_url = url.unwrap_or_default();
        if !webex_host(&floor_url) { return Err("Freigabe nicht möglich: ungültige Freigabe-Adresse.".into()); }
        dbg_conn(&format!("call: share: capture starting ({:?}, {:?}, {} fps)", r.source.kind, r.opts.max_size, r.opts.fps));
        let stream = Arc::new(std::sync::Mutex::new(capture::start(&r.source, r.opts)));
        c.share = Some(Share { stream, feed: None, offer_seq: None, floor_url, phase: Phase::Capturing, since: Instant::now(), granted: false });
        self.share_tick();
        Ok(())
    }

    /// Stops our share (the user, the call's end or a lost floor): capture off, floor released.
    pub(super) fn share_stop(&mut self) {
        let Some(s) = self.calls.cur.as_mut().and_then(|c| c.share.take()) else { return };
        let (url, requested) = (s.floor_url.clone(), matches!(s.phase, Phase::Requesting | Phase::Sharing));
        s.stop();
        let _ = self.out.send(Resp::CallShare(false));
        if !requested { dbg_conn("call: share stopped before the floor was requested"); return; }
        let self_url = self.calls.cur.as_ref().map(|c| c.self_url.clone()).unwrap_or_default();
        let r = self.http("PUT", &url, Some(floor_body("RELEASED", &self_url, &self.device, &self.calls.device_type)));
        match r {
            Ok(_) => dbg_conn("call: share: floor RELEASED -> ok"),
            // like the official client: a release that raced the server (409) or found the share gone (404) is fine
            Err(e) if matches!(http_status(&e), Some(404 | 409)) => dbg_conn(&format!("call: share: floor release {:?}, ignored", http_status(&e))),
            Err(e) => { dbg_conn(&format!("call: share: floor release failed: {}", scrub(&e))); let _ = self.out.send(Resp::Error(format!("Freigabe beenden fehlgeschlagen: {}", scrub(&e)))); }
        }
    }

    /// A step of the share failed: it ends, and the user is told why.
    pub(super) fn share_failed(&mut self, why: &str) {
        dbg_conn(&format!("call: share failed: {why}"));
        self.share_stop();
        let _ = self.out.send(Resp::Error(why.to_string()));
    }

    /// Every tick: follows the capture until it delivers, then negotiates; watches a running share.
    pub(super) fn share_tick(&mut self) {
        let Some(s) = self.calls.cur.as_ref().and_then(|c| c.share.as_ref()) else { return };
        let (phase, status, age, granted) = (s.phase, s.stream.lock().map(|x| x.status()).unwrap_or(Status::Ended), s.since.elapsed(), s.granted);
        match (phase, status) {
            (_, Status::Failed(e)) => self.share_failed(&format!("Bildschirmaufnahme fehlgeschlagen: {e}")),
            (Phase::Capturing, Status::Ended) => self.share_failed("Freigabe abgebrochen: Es wurde nichts zum Freigeben gewählt, oder die Quelle ist nicht mehr da."),
            (_, Status::Ended) => self.share_failed("Die Freigabe wurde beendet: Die freigegebene Quelle ist nicht mehr verfügbar."),
            (Phase::Capturing, Status::Running { width, height, encoder }) => {
                dbg_conn(&format!("call: share: capture running {width}x{height} ({})", encoder.chars().take(60).collect::<String>()));
                self.share_negotiate();
            }
            (Phase::Capturing, Status::Starting) if age > CAPTURE_START => self.share_failed("Bildschirmaufnahme startet nicht (Zeitüberschreitung)."),
            (Phase::Negotiating, _) if age > ROAP_TIMEOUT => self.share_failed("Freigabe fehlgeschlagen: Der Medienserver hat auf die Freigabe-Verbindung nicht geantwortet."),
            (Phase::Sharing, _) if !granted && age > FLOOR_GRANT => {
                dbg_conn("call: share: floor GRANTED not seen in the locus, sharing on (like the official client)");
                if let Some(s) = self.calls.cur.as_mut().and_then(|c| c.share.as_mut()) { s.granted = true; }
            }
            _ => {}
        }
    }

    /// The capture runs: negotiate the share's media lines once per call, then ask for the floor.
    fn share_negotiate(&mut self) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let Some(m) = c.media.as_ref() else { return };
        if m.has_share() { if let Some(s) = c.share.as_mut() { s.phase = Phase::Requesting; } return self.share_answered(); }
        let sdp = match m.add_share() { Ok(s) => s, Err(e) => return self.share_failed(&format!("Freigabe fehlgeschlagen: Die Medienleitung ließ sich nicht anlegen ({}).", scrub(&e))) };
        let seq = c.roap_seq + 1;
        c.roap_seq = seq;
        c.roap_done = false;
        if let Some(s) = c.share.as_mut() { s.offer_seq = Some(seq); s.phase = Phase::Negotiating; s.since = Instant::now(); }
        dbg_conn(&format!("call: share: roap OFFER seq={seq} {}", crate::cloud::sdp::summary(&sdp)));
        if let Err(e) = self.send_roap("OFFER", Some(&sdp), seq) {
            if let Some(c) = self.calls.cur.as_mut() { c.roap_done = true; }
            self.share_failed(&format!("Freigabe fehlgeschlagen: Webex hat die Medienanfrage abgelehnt ({}).", scrub(&e)));
        }
    }

    /// The share's media lines are negotiated (or were already): request the floor, then send pictures.
    pub(super) fn share_answered(&mut self) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let self_url = c.self_url.clone();
        let Some(s) = c.share.as_mut() else { return };
        s.phase = Phase::Requesting;
        s.offer_seq = None;
        let url = s.floor_url.clone();
        dbg_conn("call: share: floor request (GRANTED)");
        if let Err(e) = self.http("PUT", &url, Some(floor_body("GRANTED", &self_url, &self.device, &self.calls.device_type))) {
            dbg_conn(&format!("call: share: floor request failed: {}", scrub(&e)));
            return self.share_failed(&floor_error(&e));
        }
        let Some(c) = self.calls.cur.as_mut() else { return };
        let (Some(m), Some(s)) = (c.media.as_ref(), c.share.as_mut()) else { return };
        match m.feed_share(s.stream.clone()) {
            Ok(f) => { s.feed = Some(f); s.phase = Phase::Sharing; s.since = Instant::now(); }
            Err(e) => return self.share_failed(&format!("Freigabe fehlgeschlagen: {e}")),
        }
        dbg_conn("call: share: sharing");
        let _ = self.out.send(Resp::CallShare(true));
    }

    /// The locus changed: our floor granted, or taken / released elsewhere; the other side's share announced.
    pub(super) fn share_locus(&mut self, l: &Value) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let (disp, ben) = content_floor(l);
        let ours = !ben.is_empty() && ben == c.self_url;
        if let Some(s) = c.share.as_mut().filter(|s| matches!(s.phase, Phase::Requesting | Phase::Sharing)) {
            if disp == "GRANTED" && ours {
                if !s.granted { dbg_conn("call: share: floor GRANTED to this device"); }
                s.granted = true;
            } else if s.granted && (disp == "RELEASED" || (disp == "GRANTED" && !ours)) {
                let why = if disp == "GRANTED" { "Deine Freigabe wurde beendet: Jetzt gibt jemand anderes frei." } else { "Deine Freigabe wurde von Webex beendet." };
                dbg_conn(&format!("call: share: floor {disp} elsewhere, stopping"));
                s.granted = false; // nothing left to release
                self.share_stop();
                let _ = self.out.send(Resp::Error(why.into()));
            }
            return;
        }
        let remote = disp == "GRANTED" && !ours && !ben.is_empty();
        if remote && !c.remote_share {
            dbg_conn("call: share: the other side shares (not shown: Webox can't display received shares yet)");
            let _ = self.out.send(Resp::Error(format!("{} gibt Inhalte frei. Webox kann empfangene Freigaben noch nicht anzeigen.", if c.name.is_empty() { "Dein Gegenüber" } else { c.name.as_str() })));
        }
        c.remote_share = remote;
    }
}
