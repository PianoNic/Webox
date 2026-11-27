//! Screen share in a call, the sending side, in the official client's order (strings and functions of
//! `spark-windows-desktop-ui.dll`: "Connect on spark device before requesting floor", "<update_media>: roap flow",
//! "Share perform floor request", "Perform floor: Granted / Released for shareType = DesktopShare, shareUrl = …"):
//!
//! 1. only in a call that is connected on this device;
//! 2. the capture starts (`media::share`: Windows Graphics Capture / DXGI, the Wayland portal + PipeWire, X11,
//!    ScreenCaptureKit) and must deliver pictures;
//! 3. the share's media line (`video/slides`, sendrecv) is part of the call's first offer, so nothing is renegotiated;
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
enum Phase { Capturing, Requesting, Sharing }

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
    /// The media connection is being rebuilt (`Session::media_rebuild`): the feed into the old one's track stops; the
    /// capture and the floor stay, and `share_tick` feeds the new connection once its lines are answered.
    pub fn detach(&mut self) {
        if let Some(f) = self.feed.take() { f.stop(); }
        self.tx_checked = true;
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

/// The reason for a failed floor request.
pub fn floor_error(e: &str) -> String {
    match http_status(e) {
        Some(409) => crate::trq!("Can't share: someone else is sharing right now.").into_owned(),
        Some(403) => crate::trq!("Sharing not allowed: your organization's policy doesn't allow sharing in this call.").into_owned(),
        Some(404) => crate::trq!("Can't share: Webex doesn't know this call's sharing channel (anymore).").into_owned(),
        _ => crate::trq!("The share request to Webex failed: {{0}}", scrub(e)),
    }
}

/// Why a share can't start in this call (English source, translated where shown), or None.
pub fn share_blocked(state: Option<&CallState>, answered: bool, has_media: bool, roap_busy: bool, content_url: Option<&str>) -> Option<&'static str> {
    if !answered || !has_media || state != Some(&CallState::Connected) { return Some("Screen sharing is only possible once the call is connected."); }
    if content_url.is_none() { return Some("Can't share: this call offers no sharing channel (no 'content' in Locus)."); }
    if roap_busy { return Some("Can't share: the media connection is still being negotiated. Please try again in a moment."); }
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
        let Some(c) = self.calls.cur.as_mut() else { return Err(crate::trq!("Screen sharing is only possible in a call.").into_owned()) };
        if c.share.is_some() { return Ok(()); }
        let url = content_share_url(&c.locus);
        if let Some(why) = share_blocked(c.last.as_ref(), c.answered, c.media.is_some(), !c.roap_done, url.as_deref()) {
            dbg_conn(&format!("call: share refused: {why}"));
            return Err(crate::trq!(why).into_owned());
        }
        let floor_url = url.unwrap_or_default();
        if !webex_host(&floor_url) { return Err(crate::trq!("Can't share: invalid sharing address.").into_owned()); }
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
            Err(e) => { dbg_conn(&format!("call: share: floor release failed: {}", scrub(&e))); let _ = self.out.send(Resp::Error(crate::trq!("Stopping the share failed: {{0}}", scrub(&e)))); }
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
        // another ROAP round (a media reconnect) runs: the share's lines wait for the next tick
        let (busy, tx_due) = (!c.roap_done, phase == Phase::Sharing && !s.tx_checked && age > TX_CHECK);
        // a media reconnect replaced the connection: the running share goes on in the new one (same floor, no request)
        if phase == Phase::Sharing && s.feed.is_none() && !busy && c.media.as_ref().is_some_and(|m| m.state() == MediaState::Connected && m.lines_negotiated()) {
            return self.share_refeed();
        }
        if tx_due {
            // `write_sample` succeeds even when nothing goes out: what the share line really sent since this feed began decides
            let sent = c.media.as_ref().map_or(0, |m| m.share_tx_packets()).saturating_sub(s.tx_base);
            if let Some(s) = self.calls.cur.as_mut().and_then(|c| c.share.as_mut()) { s.tx_checked = true; }
            if sent == 0 { return self.share_failed(&crate::trq!("Sharing failed: no picture data is being sent to the media server.").into_owned()); }
        }
        match (phase, status) {
            (_, Status::Failed(e)) => self.share_failed(&crate::trq!("Screen capture failed: {{0}}", e)),
            (Phase::Capturing, Status::Ended) => self.share_failed(&crate::trq!("Sharing cancelled: nothing was picked to share, or the source is gone.").into_owned()),
            (_, Status::Ended) => self.share_failed(&crate::trq!("Sharing stopped: the shared source is no longer available.").into_owned()),
            (Phase::Capturing, Status::Running { .. }) if busy => {}
            (Phase::Capturing, Status::Running { width, height, encoder }) => {
                dbg_conn(&format!("call: share: capture running {width}x{height} ({})", encoder.chars().take(60).collect::<String>()));
                self.share_negotiate();
            }
            (Phase::Capturing, Status::Starting) if age > CAPTURE_START => self.share_failed(&crate::trq!("Screen capture doesn't start (timeout).").into_owned()),
            (Phase::Sharing, _) if !granted && age > FLOOR_GRANT => {
                dbg_conn("call: share: floor GRANTED not seen in the locus, sharing on (like the official client)");
                if let Some(s) = self.calls.cur.as_mut().and_then(|c| c.share.as_mut()) { s.granted = true; }
            }
            _ => {}
        }
    }

    /// The capture runs: the share's line was negotiated with the call's first offer (`video/slides`, sendrecv), so the
    /// floor request comes next; nothing is renegotiated.
    fn share_negotiate(&mut self) {
        let Some(c) = self.calls.cur.as_ref() else { return };
        if !c.media.as_ref().is_some_and(|m| m.lines_negotiated()) {
            return self.share_failed(&crate::trq!("Sharing failed: the media server didn't accept this call's share line.").into_owned());
        }
        self.share_answered();
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
            Err(e) => return self.share_failed(&crate::trq!("Sharing failed: {{0}}", e)),
        }
        dbg_conn("call: share: sharing");
        let _ = self.out.send(Resp::CallShare(true));
    }

    /// The locus changed: our floor granted, or taken / released elsewhere; the far end's share starts or ends.
    /// The share's pictures into the new media connection after a reconnect (`Share::detach`).
    fn share_refeed(&mut self) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let (Some(m), Some(s)) = (c.media.as_ref(), c.share.as_mut()) else { return };
        let base = m.share_tx_packets();
        match m.feed_share(s.stream.clone()) {
            Ok(f) => {
                (s.feed, s.since, s.tx_base, s.tx_checked) = (Some(f), Instant::now(), base, false);
                dbg_conn("call: share: sharing again on the reconnected media");
            }
            Err(e) => self.share_failed(&crate::trq!("Sharing failed: {{0}}", e)),
        }
    }

    pub(super) fn share_locus(&mut self, l: &Value) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let (disp, ben) = content_floor(l);
        let ours = !ben.is_empty() && ben == c.self_url;
        if let Some(s) = c.share.as_mut().filter(|s| matches!(s.phase, Phase::Requesting | Phase::Sharing)) {
            if disp == "GRANTED" && ours {
                if !s.granted { dbg_conn("call: share: floor GRANTED to this device"); }
                s.granted = true;
            } else if s.granted && (disp == "RELEASED" || (disp == "GRANTED" && !ours)) {
                let why = if disp == "GRANTED" { crate::trq!("Your share stopped: someone else is sharing now.").into_owned() } else { crate::trq!("Webex stopped your share.").into_owned() };
                dbg_conn(&format!("call: share: floor {disp} elsewhere, stopping"));
                s.granted = false; // nothing left to release
                self.share_stop();
                let _ = self.out.send(Resp::Error(why));
            }
        }
        self.remote_share_from(l);
    }

    /// The far end's share, from the locus: on when the `content` floor is GRANTED to a participant still in the call
    /// (not us), off when it is released or that participant left. Its pictures arrive on `video/slides`, offered from the
    /// call's start; an SSRC the answer didn't name is declared on our side (`Session::media_tick`).
    fn remote_share_from(&mut self, l: &Value) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let now = crate::cloud::share_rx::remote_sharer(l, &c.self_url);
        match (now, c.remote_share.is_some()) {
            (Some(name), false) => {
                let shown = [name, c.name.clone()].into_iter().find(|n| !n.is_empty()).unwrap_or_default();
                dbg_conn("call: share: the far end shares, receiving");
                crate::cloud::share_rx::clear();
                (c.remote_share, c.remote_since, c.rx_warned) = (Some(shown.clone()), Some(Instant::now()), false);
                let _ = self.out.send(Resp::RemoteShare(Some(shown)));
                if let Some(m) = c.media.as_ref() {
                    m.expect.fetch_or(crate::cloud::media::EXPECT_SHARE, Ordering::Relaxed);
                    // counted per share: a second share from a new SSRC is declared like the first (`undeclared_video`)
                    m.rx_video.store(0, Ordering::Relaxed);
                }
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

    /// Every tick while the far end shares: the user is told once when no picture arrives.
    pub(super) fn share_rx_tick(&mut self) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        let Some(since) = c.remote_since.filter(|_| c.remote_share.is_some()) else { return };
        let Some(m) = c.media.as_ref() else { return };
        let (seen, rx) = (m.seen_video.any(), m.rx_video.load(Ordering::Relaxed));
        if !c.rx_warned && crate::cloud::share_rx::latest().is_none() && since.elapsed() > RX_NOTHING {
            c.rx_warned = true;
            dbg_conn(&format!("call: share: nothing decoded after {} s (video packets {rx}, seen on the wire {})", RX_NOTHING.as_secs(), seen));
            let msg = match c.remote_share.clone().filter(|n| !n.is_empty()) {
                Some(who) => crate::trq!("{{0}} is sharing the screen, but no picture arrives.", who),
                None => crate::trq!("The other side is sharing the screen, but no picture arrives.").into_owned(),
            };
            let _ = self.out.send(Resp::Error(msg));
        }
    }
}

/// A running share whose sender has put out no packet after this long failed.
const TX_CHECK: Duration = Duration::from_secs(6);
/// The user is told once when a received share shows no picture after this long.
const RX_NOTHING: Duration = Duration::from_secs(15);

