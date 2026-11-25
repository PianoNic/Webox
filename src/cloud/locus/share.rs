//! Screen share in a call, the sending side, in the official client's order (strings and functions of
//! `spark-windows-desktop-ui.dll`: "Connect on spark device before requesting floor", "<update_media>: roap flow",
//! "Share perform floor request", "Perform floor: Granted / Released for shareType = DesktopShare, shareUrl = …"):
//!
//! 1. only in a call that is connected on this device;
//! 2. the capture starts (`media::share`: Windows Graphics Capture / DXGI, the Wayland portal + PipeWire, X11,
//!    ScreenCaptureKit) and must deliver pictures;
//! 3. the share's media lines are negotiated once per call on the same ROAP session (a re-OFFER with `video/main`,
//!    `audio/slides`, `video/slides`; `Media::add_lines`, shared with the camera, `video.rs`), the server ANSWERs, we send OK;
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
    floor_url: String,
    phase: Phase,
    since: Instant,
    granted: bool,
    /// The share line's sent packets when this feed started, and whether the zero-packet check ran (`TX_CHECK`).
    tx_base: u64, tx_checked: bool,
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
        c.share = Some(Share { stream, feed: None, floor_url, phase: Phase::Capturing, since: Instant::now(), granted: false, tx_base: 0, tx_checked: false });
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
        let Some(c) = self.calls.cur.as_ref() else { return };
        let Some(s) = c.share.as_ref() else { return };
        let (phase, status, age, granted) = (s.phase, s.stream.lock().map(|x| x.status()).unwrap_or(Status::Ended), s.since.elapsed(), s.granted);
        // another ROAP round (an ICE restart) runs: the share's lines wait for the next tick
        let (busy, tx_due) = (!c.roap_done, phase == Phase::Sharing && !s.tx_checked && age > TX_CHECK);
        if tx_due {
            // `write_sample` succeeds even when nothing goes out: what the share line really sent since this feed began decides
            let sent = c.media.as_ref().map_or(0, |m| m.share_tx_packets()).saturating_sub(s.tx_base);
            if let Some(s) = self.calls.cur.as_mut().and_then(|c| c.share.as_mut()) { s.tx_checked = true; }
            if sent == 0 { return self.share_failed("Freigabe fehlgeschlagen: Es werden keine Bilddaten an den Medienserver gesendet."); }
        }
        match (phase, status) {
            (_, Status::Failed(e)) => self.share_failed(&format!("Bildschirmaufnahme fehlgeschlagen: {e}")),
            (Phase::Capturing, Status::Ended) => self.share_failed("Freigabe abgebrochen: Es wurde nichts zum Freigeben gewählt, oder die Quelle ist nicht mehr da."),
            (_, Status::Ended) => self.share_failed("Die Freigabe wurde beendet: Die freigegebene Quelle ist nicht mehr verfügbar."),
            (Phase::Capturing, Status::Running { .. }) if busy => {}
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

    /// The capture runs: the share's lines (negotiated once per call, for sending and receiving), then the floor.
    fn share_negotiate(&mut self) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let ready = c.media.as_ref().is_some_and(|m| m.lines_negotiated()) && c.lines_seq.is_none();
        if let Some(s) = c.share.as_mut() { s.phase = Phase::Negotiating; s.since = Instant::now(); }
        if ready { return self.share_answered(); }
        if c.lines_seq.is_some() { return; } // a negotiation for the far end's share / the camera runs: `lines_ready` goes on
        if let Err(e) = self.negotiate_lines(false) { self.share_failed(&format!("Freigabe fehlgeschlagen: {e}")); }
    }

    /// One ROAP round that adds the video lines (`video/main`, `audio/slides`, `video/slides`, all for the camera and the
    /// share, `Media::add_lines`) or, with `again`, offers them once more (to declare the SSRC a received share or camera
    /// really comes from).
    pub(super) fn negotiate_lines(&mut self, again: bool) -> Result<(), String> {
        let Some(c) = self.calls.cur.as_mut() else { return Err("Kein Anruf.".into()) };
        let Some(m) = c.media.as_ref() else { return Err("Keine Medienverbindung.".into()) };
        // lines that exist but whose round failed are offered again, not added twice
        let sdp = if again || m.has_lines() { m.reoffer() } else { m.add_lines() }
            .map_err(|e| format!("Die Medienleitung ließ sich nicht anlegen ({}).", scrub(&e)))?;
        let seq = c.roap_seq + 1;
        (c.roap_seq, c.roap_done, c.lines_seq, c.lines_at) = (seq, false, Some(seq), Some(Instant::now()));
        dbg_conn(&format!("call: lines: roap OFFER seq={seq} ({}) {}", if again { "again" } else { "video lines" }, crate::cloud::sdp::summary(&sdp)));
        if let Err(e) = self.send_roap("OFFER", Some(&sdp), seq) {
            if let Some(c) = self.calls.cur.as_mut() { c.roap_done = true; c.lines_seq = None; c.lines_at = None; }
            if let Some(m) = self.calls.cur.as_ref().and_then(|c| c.media.as_ref()) { m.lines_unnegotiated(); }
            return Err(format!("Webex hat die Medienanfrage abgelehnt ({}).", scrub(&e)));
        }
        Ok(())
    }

    /// The video lines are negotiated: our own share goes on to the floor request, our camera starts.
    pub(super) fn lines_ready(&mut self) {
        dbg_conn("call: lines negotiated");
        if self.calls.cur.as_ref().is_some_and(|c| c.share.as_ref().is_some_and(|s| s.phase == Phase::Negotiating)) { self.share_answered(); }
        self.video_tick();
    }

    /// Negotiating the video lines failed: our share ends with the reason, so does our camera; a received share can't be
    /// shown.
    pub(super) fn lines_failed(&mut self, why: &str) {
        dbg_conn(&format!("call: lines refused: {why}"));
        self.video_lines_failed(why);
        if self.calls.cur.as_ref().is_some_and(|c| c.share.is_some()) { return self.share_failed(&format!("Freigabe fehlgeschlagen: {why}")); }
        let Some(c) = self.calls.cur.as_ref() else { return };
        if let Some(n) = &c.remote_share {
            let _ = self.out.send(Resp::Error(format!("Die Freigabe von {} kann nicht angezeigt werden: {why}", if n.is_empty() { "deinem Gegenüber" } else { n })));
        }
    }

    /// The share's media lines are negotiated (or were already): request the floor, then send pictures.
    pub(super) fn share_answered(&mut self) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let self_url = c.self_url.clone();
        let Some(s) = c.share.as_mut() else { return };
        s.phase = Phase::Requesting;
        let url = s.floor_url.clone();
        dbg_conn("call: share: floor request (GRANTED)");
        if let Err(e) = self.http("PUT", &url, Some(floor_body("GRANTED", &self_url, &self.device, &self.calls.device_type))) {
            dbg_conn(&format!("call: share: floor request failed: {}", scrub(&e)));
            return self.share_failed(&floor_error(&e));
        }
        let Some(c) = self.calls.cur.as_mut() else { return };
        let (Some(m), Some(s)) = (c.media.as_ref(), c.share.as_mut()) else { return };
        let base = m.share_tx_packets(); // before the feed sends anything
        match m.feed_share(s.stream.clone()) {
            Ok(f) => { (s.feed, s.phase, s.since, s.tx_base, s.tx_checked) = (Some(f), Phase::Sharing, Instant::now(), base, false); }
            Err(e) => return self.share_failed(&format!("Freigabe fehlgeschlagen: {e}")),
        }
        dbg_conn("call: share: sharing");
        let _ = self.out.send(Resp::CallShare(true));
    }

    /// The locus changed: our floor granted, or taken / released elsewhere; the far end's share starts or ends.
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
        }
        self.remote_share_from(l);
    }

    /// The far end's share, from the locus: on when the `content` floor is GRANTED to a participant still in the call
    /// (not us), off when it is released or that participant left. Starting it makes sure the share's lines exist, so the
    /// server can send it to us (the official client offers them from the start; Webox adds them when they are needed).
    fn remote_share_from(&mut self, l: &Value) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let now = crate::cloud::share_rx::remote_sharer(l, &c.self_url);
        match (now, c.remote_share.is_some()) {
            (Some(name), false) => {
                let shown = [name, c.name.clone()].into_iter().find(|n| !n.is_empty()).unwrap_or_default();
                dbg_conn("call: share: the far end shares, receiving");
                crate::cloud::share_rx::clear();
                (c.remote_share, c.remote_since, c.rx_reoffered, c.rx_warned, c.rx_lines_tried) = (Some(shown.clone()), Some(Instant::now()), false, false, false);
                let _ = self.out.send(Resp::RemoteShare(Some(shown)));
                if let Some(m) = c.media.as_ref() {
                    m.expect.fetch_or(crate::cloud::media::EXPECT_SHARE, Ordering::Relaxed);
                    // counted per share: a second share from a new SSRC is declared like the first (`undeclared_video`)
                    m.rx_video.store(0, Ordering::Relaxed);
                }
                // not while another ROAP round runs (the call's first one included): `share_rx_tick` catches up
                let need = c.media.as_ref().is_some_and(|m| !m.lines_negotiated()) && c.lines_seq.is_none() && c.roap_done;
                if need { c.rx_lines_tried = true; if let Err(e) = self.negotiate_lines(false) { self.lines_failed(&e); } }
            }
            (None, true) => {
                dbg_conn("call: share: the far end's share released");
                c.remote_share = None;
                if let Some(m) = c.media.as_ref() { m.expect.fetch_and(!crate::cloud::media::EXPECT_SHARE, Ordering::Relaxed); }
                crate::cloud::share_rx::clear();
                let _ = self.out.send(Resp::RemoteShare(None));
            }
            _ => {}
        }
    }

    /// Every tick: the lines' ANSWER that never comes; while the far end shares, its share arriving from an SSRC webrtc-rs
    /// wasn't told (seen on the ICE socket, nothing taken): one more ROAP round declares it.
    pub(super) fn share_rx_tick(&mut self) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        if c.lines_at.is_some_and(|t| t.elapsed() > ROAP_TIMEOUT) {
            (c.lines_seq, c.lines_at, c.roap_done) = (None, None, true);
            if let Some(m) = c.media.as_ref() { m.lines_unnegotiated(); }
            return self.lines_failed("Der Medienserver hat auf die Videoverbindung nicht geantwortet.");
        }
        let Some(since) = c.remote_since.filter(|_| c.remote_share.is_some()) else { return };
        let Some(m) = c.media.as_ref() else { return };
        let (seen, rx, idle) = (m.undeclared_video().0.is_some(), m.rx_video.load(Ordering::Relaxed), c.lines_seq.is_none() && c.roap_done);
        // the far end shares but the lines never got negotiated (a ROAP round ran when it started): once now
        if idle && !c.rx_lines_tried && !m.lines_negotiated() && c.share.is_none() {
            c.rx_lines_tried = true;
            dbg_conn("call: share: negotiating the lines for the far end's share");
            if let Err(e) = self.negotiate_lines(false) { self.lines_failed(&e); }
            return;
        }
        let ready = idle && m.lines_negotiated();
        if ready && !c.rx_reoffered && seen && rx == 0 && since.elapsed() > RX_SSRC_WAIT {
            c.rx_reoffered = true;
            dbg_conn("call: share: the far end's share arrives from an undeclared SSRC, negotiating again");
            if let Err(e) = self.negotiate_lines(true) { self.lines_failed(&e); }
            return;
        }
        if !c.rx_warned && crate::cloud::share_rx::latest().is_none() && since.elapsed() > RX_NOTHING {
            c.rx_warned = true;
            dbg_conn(&format!("call: share: nothing decoded after {} s (video packets {rx}, seen on the wire {})", RX_NOTHING.as_secs(), seen));
            let who = c.remote_share.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| "Dein Gegenüber".into());
            let _ = self.out.send(Resp::Error(format!("{who} gibt den Bildschirm frei, aber es kommt kein Bild an.")));
        }
    }
}

/// A running share whose sender has put out no packet after this long failed.
const TX_CHECK: Duration = Duration::from_secs(6);
/// How long the far end's share / camera may arrive undecoded from an SSRC we didn't declare before the lines are offered
/// again.
pub(super) const RX_SSRC_WAIT: Duration = Duration::from_secs(3);
/// The user is told once when a received share shows no picture after this long.
const RX_NOTHING: Duration = Duration::from_secs(15);

