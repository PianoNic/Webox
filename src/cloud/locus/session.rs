//! The session side of calls: dialling, answering, hanging up and following the locus.
use super::*;

impl Session {
    pub(crate) fn call_req(&mut self, r: CallReq) -> Result<(), String> {
        match r {
            CallReq::Dial(a, c) => self.dial(&a, &c),
            CallReq::Answer(c) => self.answer(&c),
            CallReq::HangUp => self.hang_up(),
            CallReq::Mute(m) => self.set_mute(m),
            CallReq::Devices(c) => {
                if self.calls.cur.is_some() { if let Some(e) = &self.calls.engine { c.apply_smart(&e.io); e.open(&c.input, &c.output)?; e.io.set_volume(c.volume); } }
                Ok(())
            }
            // Smart Audio changed: the capture callback reads the mode on every block, no device reopen
            CallReq::SmartMic(m) => { if let Some(e) = &self.calls.engine { e.io.set_smart(m); } Ok(()) }
            CallReq::Audit => self.audit(false),
            CallReq::Cleanup => self.audit(true),
            CallReq::Dtmf(t) => self.dtmf(&t),
            CallReq::Share(r) => self.share_req(r),
        }
    }

    /// Mute / unmute our microphone and tell Locus, so the other side sees it (`local_mute_body`, `PUT {self.url}/media`).
    /// Unmuting after someone else muted us first lifts that (`PATCH {self.url}/controls`), unless the locus forbids it.
    /// Before the call is answered here only the microphone changes (the join's ROAP OFFER carries `audioMuted`).
    pub(super) fn set_mute(&mut self, m: bool) -> Result<(), String> {
        self.mute_local(m);
        let Some(c) = self.calls.cur.as_ref().filter(|c| c.answered && !c.media_id.is_empty()) else { return Ok(()) };
        let (self_url, media_id, corr, me) = (c.self_url.clone(), c.media_id.clone(), c.correlation_id.clone(), c.view.as_ref().map(|v| v.mute).unwrap_or_default());
        if !webex_host(&self_url) { return Ok(()); }
        if !m && me.remote {
            if !me.unmute_allowed {
                self.mute_local(true);
                self.mute_sync(None);
                return Err("Du wurdest stummgeschaltet und kannst die Stummschaltung nicht selbst aufheben.".into());
            }
            let r = self.http("PATCH", &format!("{self_url}/controls"), Some(unmute_controls_body()));
            dbg_conn(&format!("call: unmute after being muted by others -> {}", if r.is_ok() { "ok" } else { "failed" }));
            if let Err(e) = r {
                self.mute_local(true);
                self.mute_sync(None);
                return Err(format!("Stummschaltung aufheben fehlgeschlagen: {}", scrub(&e)));
            }
        }
        let r = self.http("PUT", &format!("{self_url}/media"), Some(local_mute_body(self.device_body(), &corr, &media_id, m, true)));
        dbg_conn(&format!("call: {} sent to locus -> {}", if m { "mute" } else { "unmute" }, if r.is_ok() { "ok" } else { "failed" }));
        self.mute_sync(None);
        r.map(|_| ()).map_err(|e| format!("{} konnte nicht an Webex gemeldet werden: {}", if m { "Stummschalten" } else { "Stummschaltung aufheben" }, scrub(&e)))
    }

    /// The microphone itself: silence instead of it while muted.
    fn mute_local(&mut self, m: bool) {
        self.calls.muted = m;
        if let Some(a) = self.calls.cur.as_ref().and_then(|c| c.media.as_ref()) { a.muted.store(m, Ordering::Relaxed); }
    }

    /// Follows the mute state in the locus (`v`, or the last one we have): mutes our microphone when someone else muted
    /// us, and tells the UI what changed (`Resp::CallMute`).
    pub(super) fn mute_sync(&mut self, v: Option<&LocusView>) {
        let local = self.calls.muted;
        let Some(c) = self.calls.cur.as_ref() else { return };
        let Some(view) = v.or(c.view.as_ref()).cloned() else { return };
        let (info, newly_remote) = mute_update(c.mute_info.as_ref(), &view, local);
        if newly_remote {
            dbg_conn("call: muted by others, microphone muted");
            self.mute_local(true);
        }
        if let Some(c) = self.calls.cur.as_mut() {
            if c.mute_info.as_ref() != Some(&info) {
                if c.mute_info.is_none_or(|p| p.peer_muted != info.peer_muted) { dbg_conn(&format!("call: the other side {}", if info.peer_muted { "muted" } else { "unmuted" })); }
                if c.mute_info.is_none_or(|p| p.me.unmute_requested != info.me.unmute_requested) && info.me.unmute_requested { dbg_conn("call: asked to unmute"); }
                c.mute_info = Some(info);
                let _ = self.out.send(Resp::CallMute(info));
            }
        }
    }

    /// WRITE (only from the call window's keypad): sends tones into the connected call.
    pub(super) fn dtmf(&mut self, tones: &str) -> Result<(), String> {
        if !dtmf_ok(tones) { return Ok(()); }
        let Some(c) = self.calls.cur.as_ref().filter(|c| c.answered && c.last == Some(CallState::Connected)) else { return Ok(()) };
        let url = c.url.clone();
        if !webex_host(&url) { return Err("Ungültige Anruf-Adresse.".into()); }
        let r = self.http("POST", &format!("{url}/sendDtmf"), Some(dtmf_body(&self.device, tones, &jose::uuid())));
        dbg_conn(&format!("call: dtmf {} tone(s) -> {}", tones.len(), if r.is_ok() { "ok" } else { "failed" }));
        r.map(|_| ()).map_err(|e| format!("Tastenton fehlgeschlagen: {}", scrub(&e)))
    }

    pub(super) fn emit(&mut self, state: CallState) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        if c.last.as_ref() == Some(&state) { return; }
        c.last = Some(state.clone());
        dbg_conn(&format!("call: {:?}", state));
        let _ = self.out.send(Resp::Call(CallEvent { state, name: c.name.clone(), email: c.email.clone(), incoming: !c.outgoing }));
    }

    pub(super) fn device_body(&self) -> Value { json!({"url": self.device, "deviceType": self.calls.device_type}) }

    pub(super) fn start_ticker(&mut self) {
        let Some(tx) = self.calls.jobs.clone() else { return };
        self.calls.live.store(false, Ordering::SeqCst); // stop an older ticker
        let live = Arc::new(AtomicBool::new(true));
        self.calls.live = live.clone();
        std::thread::spawn(move || while live.load(Ordering::SeqCst) {
            std::thread::sleep(TICK);
            if live.load(Ordering::SeqCst) && tx.send(Job::Locus(json!({"tick": true}))).is_err() { break; }
        });
    }

    pub(super) fn end_call(&mut self, why: &str) {
        self.emit(CallState::Ended(why.into()));
        if let Some(mut c) = self.calls.cur.take() {
            if let Some(s) = c.share.take() { s.stop(); let _ = self.out.send(Resp::CallShare(false)); }
            if c.remote_share.take().is_some() { dbg_conn("call: share: received share ended with the call"); let _ = self.out.send(Resp::RemoteShare(None)); }
            crate::cloud::share_rx::clear();
            if let Some(m) = c.media { m.close(); }
        }
        self.release_audio();
        self.calls.live.store(false, Ordering::SeqCst);
    }

    /// Closes the call's audio devices (microphone included); every failed dial / answer ends here so the mic never stays open.
    pub(super) fn release_audio(&mut self) {
        if let Some(e) = &self.calls.engine { e.io.set_call(false); e.close(); }
    }

    /// Opens the audio devices and the WebRTC peer and returns it with the ROAP OFFER SDP (ICE gathering complete).
    /// On any failure the audio devices are closed again.
    pub(super) fn prepare_media(&mut self, c: &audio::Choice) -> Result<(Media, String), String> {
        let r = self.open_media(c);
        if r.is_err() { self.release_audio(); }
        r
    }

    fn open_media(&mut self, c: &audio::Choice) -> Result<(Media, String), String> {
        // ICE binds only where the system routes to Webex (the locus host stands in for the media servers)
        let host = self.url("locus", "").split("://").nth(1).and_then(|r| r.split(['/', ':']).next()).unwrap_or_default().to_string();
        let engine = self.calls.engine.get_or_insert_with(audio::Engine::new);
        let info = engine.open(&c.input, &c.output).map_err(|e| { dbg_conn(&format!("call: audio devices failed: {}", scrub(&e))); e })?;
        engine.io.set_volume(c.volume);
        c.apply_smart(&engine.io);
        engine.io.set_call(true); // echo cancellation and gain control on the call microphone
        dbg_conn(&format!("call: audio open ({} chars)", info.len()));
        let ip = route_ip(&host);
        dbg_conn(&format!("call: route interface {}", ip.map_or("unknown (all interfaces)", |i| if i.is_ipv4() { "found (IPv4)" } else { "found (IPv6)" })));
        let m = Media::new(engine.io.clone(), None, Net::Route(ip)).map_err(|e| { dbg_conn(&format!("call: media setup failed: {}", scrub(&e))); e })?;
        m.muted.store(self.calls.muted, Ordering::Relaxed);
        let offer = m.offer().map_err(|e| { dbg_conn(&format!("call: offer failed: {}", scrub(&e))); e })?;
        dbg_conn(&format!("call: offer ready ({} bytes) {}", offer.len(), crate::cloud::sdp::summary(&offer)));
        Ok((m, offer))
    }

    pub(super) fn dial(&mut self, addr: &str, choice: &audio::Choice) -> Result<(), String> {
        let addr = addr.trim().to_lowercase();
        call_allowed(&addr, std::env::var(GUARD_ENV).ok().as_deref())?; // before any network request
        if self.calls.cur.is_some() { return Err("Es läuft bereits ein Anruf.".into()); }
        if self.calls.last_dial.is_some_and(|t| t.elapsed() < REDIAL_COOLDOWN) { return Err("Bitte kurz warten, bevor erneut angerufen wird.".into()); }
        if self.calls.device_type.is_empty() || self.device.is_empty() { return Err("Dieses Gerät ist noch nicht registriert.".into()); }
        // The media offer comes first: a call is only ever created WITH an offer (a join without media leaves the callee "connecting").
        let (media, offer) = self.prepare_media(choice)?;
        self.calls.last_dial = Some(Instant::now());
        let corr = jose::uuid();
        let body = json!({"device": self.device_body(), "usingResource": null, "moveMediaToResource": false, "correlationId": corr,
            "respOnlySdp": true, "allowMultiDevice": true, "ensureConversation": false, "supportsNativeLobby": 1,
            "clientMediaPreferences": {"preferTranscoding": true}, "invitee": {"address": addr},
            "localMedias": [{"localSdp": roap_local_sdp("OFFER", Some(&offer), 1, self.calls.muted)}]});
        let url = self.url("locus", "/loci/call?alternateRedirect=true");
        dbg_conn("call: POST /loci/call (with ROAP offer)");
        let r = match self.http("POST", &url, Some(body)) { Ok(r) => r, Err(e) => { media.close(); self.release_audio(); return Err(format!("Anruf fehlgeschlagen: {}", scrub(&e))); } };
        self.adopt(&r, media, corr, true)
    }

    /// Answer an incoming call: join the locus with our OFFER (the same ROAP flow as dialling).
    pub(super) fn answer(&mut self, choice: &audio::Choice) -> Result<(), String> {
        let Some(c) = self.calls.cur.as_ref().filter(|c| !c.outgoing && !c.answered) else { return Ok(()) };
        let url = c.url.clone();
        if !webex_host(&url) { return Err("Ungültige Anruf-Adresse.".into()); }
        let (media, offer) = self.prepare_media(choice)?;
        let corr = jose::uuid();
        let body = json!({"device": self.device_body(), "usingResource": null, "moveMediaToResource": false, "correlationId": corr,
            "respOnlySdp": true, "allowMultiDevice": true, "ensureConversation": false, "supportsNativeLobby": 1,
            "clientMediaPreferences": {"preferTranscoding": true}, "localMedias": [{"localSdp": roap_local_sdp("OFFER", Some(&offer), 1, self.calls.muted)}]});
        dbg_conn("call: POST /participant (answer, with ROAP offer)");
        let r = match self.http("POST", &format!("{url}/participant"), Some(body)) {
            Ok(r) => r,
            Err(e) => { dbg_conn(&format!("call: answer failed: {}", scrub(&e))); media.close(); self.release_audio(); return Err(format!("Annehmen fehlgeschlagen: {}", scrub(&e))); }
        };
        self.adopt(&r, media, corr, false)
    }

    /// Takes the join answer (new call or answered call): remembers locus / self / media ids and any ready ROAP answer.
    pub(super) fn adopt(&mut self, r: &Value, media: Media, corr: String, new_call: bool) -> Result<(), String> {
        let l = locus_of(r);
        let v = parse_locus(l, &self.my_email);
        let lurl = if v.url.is_empty() { locus_url_of(r) } else { v.url.clone() };
        if lurl.is_empty() || !webex_host(&lurl) {
            media.close();
            self.release_audio();
            let keys = r.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>().join(",")).unwrap_or_default();
            return Err(format!("Anruf: unerwartete Antwort (keys: {keys})"));
        }
        let media_id = media_id_of(r, &self.device);
        dbg_conn(&format!("call: joined {} media_id={} (top level {})", describe(&v), !media_id.is_empty(), r["mediaConnections"][0]["mediaId"].is_string()));
        if new_call {
            let name = v.others.first().map(|p| p.name.clone()).unwrap_or_default();
            let email = v.others.first().map(|p| p.email.clone()).unwrap_or_default();
            self.calls.cur = Some(Active::new(lurl, v.self_id.clone(), v.self_url.clone(), true, name, email));
        }
        if let Some(c) = self.calls.cur.as_mut() {
            c.self_id = v.self_id.clone(); c.self_url = v.self_url.clone(); c.media_id = media_id; c.correlation_id = corr; c.answered = true; c.media = Some(media); c.roap_seq = 1; c.roap_done = false;
            c.media_warned = false; c.offer_at = Some(Instant::now()); c.joined_at = None; c.media_up_at = None;
            // the join answer carries the locus: it counts as read
            c.view = Some(v.clone()); c.seq = locus_seq(l); c.read_at = Some(Instant::now()); c.locus = l.clone();
        }
        self.start_ticker();
        self.mute_sync(Some(&v));
        if let Some(m) = remote_roap(r) { self.handle_roap(m); }
        self.evaluate(&v)
    }

    /// PUT {self.url}/media with a ROAP message (OK / ANSWER / a re-OFFER).
    pub(super) fn send_roap(&mut self, message_type: &str, sdp: Option<&str>, seq: u32) -> Result<(), String> {
        let (url, media_id, corr) = match &self.calls.cur { Some(c) => (c.self_url.clone(), c.media_id.clone(), c.correlation_id.clone()), None => return Ok(()) };
        if !webex_host(&url) { return Err("Keine gültige Medien-Adresse.".into()); }
        if media_id.is_empty() { return Err("no media id".into()); } // like the original client: no ROAP message without it
        let body = json!({"device": self.device_body(), "correlationId": corr, "clientMediaPreferences": {"preferTranscoding": true},
            "localMedias": [{"localSdp": roap_local_sdp(message_type, sdp, seq, self.calls.muted), "mediaId": media_id}]});
        self.http("PUT", &format!("{url}/media"), Some(body)).map(|_| ())
    }

    /// A ROAP message from the server (mercury event or inline in a response).
    pub(super) fn handle_roap(&mut self, (kind, seq, sdp): (String, u32, Option<String>)) {
        dbg_conn(&format!("call: roap {kind} seq={seq} sdp={}", sdp.is_some()));
        match kind.as_str() {
            "ANSWER" => {
                let Some(c) = self.calls.cur.as_ref() else { return };
                let (done, ours, share_seq) = (c.roap_done, c.roap_seq, c.lines_seq);
                if done && seq == ours {
                    // the server repeats its ANSWER when our OK got lost: OK again (the state machine waits for it)
                    let r = self.send_roap("OK", None, seq);
                    dbg_conn(&format!("call: repeated ANSWER seq={seq}, OK sent again -> {}", if r.is_ok() { "ok" } else { "failed" }));
                    return;
                }
                if done || seq != ours { dbg_conn(&format!("call: ANSWER seq={seq} does not match our OFFER seq={ours}, ignored")); return; }
                let (Some(sdp), Some(m)) = (sdp, c.media.as_ref()) else { dbg_conn("call: ANSWER without SDP or media"); return };
                let r = if share_seq == Some(seq) { m.apply_share_answer(sdp) } else { m.apply_answer(sdp) };
                match r {
                    Ok(()) => {
                        if let Some(c) = self.calls.cur.as_mut() { c.roap_done = true; c.offer_at = None; c.lines_seq = None; c.lines_at = None; }
                        let r = self.send_roap("OK", None, seq);
                        dbg_conn(&format!("call: roap OK sent -> {}", r.as_ref().map_or_else(|e| format!("failed: {}", scrub(e)), |_| "ok".into())));
                        if share_seq == Some(seq) { self.lines_ready(); }
                    }
                    Err(e) => {
                        dbg_conn(&format!("call: bad answer: {}", scrub(&e)));
                        if share_seq == Some(seq) {
                            if let Some(c) = self.calls.cur.as_mut() { c.roap_done = true; c.lines_seq = None; c.lines_at = None; }
                            self.lines_failed(&format!("Der Medienserver hat die Freigabe-Verbindung abgelehnt: {}", scrub(&e)));
                        }
                        else { self.media_trouble("Medienaushandlung fehlgeschlagen: die Antwort des Medienservers wurde nicht angenommen."); }
                    }
                }
            }
            "OFFER" => { // the server renegotiating: answer it
                let (Some(sdp), Some(m)) = (sdp, self.calls.cur.as_ref().and_then(|c| c.media.as_ref())) else { return };
                match m.answer(sdp) {
                    Ok(a) => {
                        // ROAP numbers both sides' offers in one sequence: our next OFFER must be newer than this one
                        if let Some(c) = self.calls.cur.as_mut() { c.roap_seq = c.roap_seq.max(seq); }
                        let r = self.send_roap("ANSWER", Some(&a), seq);
                        dbg_conn(&format!("call: roap ANSWER sent -> {}", if r.is_ok() { "ok" } else { "failed" }));
                    }
                    Err(e) => dbg_conn(&format!("call: answer failed: {}", scrub(&e))),
                }
            }
            // like the original client: an error report (glare, sequence) is logged, the call stays
            "ERROR" => {
                dbg_conn("call: roap ERROR, call kept");
                let share_offer = self.calls.cur.as_ref().is_some_and(|c| c.lines_seq == Some(seq) && !c.roap_done);
                if share_offer {
                    if let Some(c) = self.calls.cur.as_mut() { c.roap_done = true; c.offer_at = None; c.lines_seq = None; c.lines_at = None; }
                    self.lines_failed("Der Medienserver hat die Freigabe abgelehnt (ROAP-Fehler).");
                }
            }
            _ => {} // OK, TURN_DISCOVERY_RESPONSE, SHUTDOWN: nothing to do
        }
    }

    /// Media trouble keeps the call (ICE may still recover, the user hangs up): logged, one notice per call.
    pub(super) fn media_trouble(&mut self, why: &str) {
        let Some(c) = self.calls.cur.as_mut() else { return };
        if std::mem::replace(&mut c.media_warned, true) { return; }
        dbg_conn(&format!("call: media trouble, call kept: {why}"));
        let _ = self.out.send(Resp::Error(why.into()));
    }

    /// Cancel (outgoing) / leave (connected) / decline (incoming, not answered).
    pub(super) fn hang_up(&mut self) -> Result<(), String> {
        let Some(c) = self.calls.cur.as_ref() else { return Ok(()) };
        let (url, id, incoming_pending) = (c.url.clone(), c.self_id.clone(), !c.outgoing && !c.answered);
        if !webex_host(&url) { self.end_call("left"); return Err("Ungültige Anruf-Adresse.".into()); }
        // Local media stops first and whatever the server answers (stalled request, network error, back-off, expired
        // token): the other side must never keep hearing us. The request error is still shown.
        self.end_call(if incoming_pending { "declined" } else { "left" });
        let send = |s: &Self| if incoming_pending {
            s.http("PUT", &format!("{url}/participant/decline"), Some(json!({"device": s.device_body()})))
        } else {
            let leave = json!({"device": s.device_body(), "usingResource": null, "correlationId": jose::uuid()});
            s.http("PUT", &format!("{url}/participant/{id}/leave"), Some(leave))
        };
        let mut r = send(self);
        // A lost leave keeps us "in the call" on the other side's screen: one more try unless Locus said the
        // participant / locus is gone already (404 / 409 / 410).
        if let Err(e) = &r {
            dbg_conn(&format!("call: {} failed: {}", if incoming_pending { "decline" } else { "leave" }, scrub(e)));
            if !matches!(http_status(e), Some(404 | 409 | 410)) { std::thread::sleep(Duration::from_millis(500)); r = send(self); }
        }
        dbg_conn(&format!("call: {} -> {}", if incoming_pending { "decline" } else { "leave" }, if r.is_ok() { "ok" } else { "failed" }));
        r.map(|_| ()).map_err(|e| format!("Auflegen fehlgeschlagen: {}", scrub(&e)))
    }

    /// Maps the locus + our media state to what the UI shows; Connected only with real media.
    pub(super) fn evaluate(&mut self, v: &LocusView) -> Result<(), String> {
        let now = Instant::now();
        let Some(c) = self.calls.cur.as_mut() else { return Ok(()) };
        let (outgoing, answered) = (c.outgoing, c.answered);
        // Ringing here, but I am JOINED: answered on another of my devices (the official client's `joinedOnThisDevice`).
        // This device must stop ringing and never show the call as its own.
        if !outgoing && !answered && v.self_state == "JOINED" {
            dbg_conn("call: answered on another device, ringing stops here");
            self.end_call("elsewhere");
            return Ok(());
        }
        let media = c.media.as_ref().map(|m| m.state());
        let rx = c.media.as_ref().map_or(0, |m| m.rx_packets.load(Ordering::Relaxed));
        let Some(mut s) = call_state(v, outgoing || answered) else { return Ok(()) };
        if s == CallState::Connected { c.joined_at.get_or_insert(now); }
        if media == Some(MediaState::Connected) { c.media_up_at.get_or_insert(now); }
        let stall = if answered { media_stall(now, c.offer_at, c.joined_at, c.media_up_at, rx, media) } else { None };
        if let Some(why) = stall { self.media_trouble(why); }
        // "Verbunden" only with the peer connection up (ICE + DTLS); before that "Verbindung wird hergestellt…"
        if s == CallState::Connected && media != Some(MediaState::Connected) { s = CallState::Connecting; }
        if let CallState::Ended(w) = &s { self.end_call(w); } else { self.emit(s); }
        Ok(())
    }

    /// Mercury `locus.*` events and our own ticks while a call exists. An event's own full locus is applied as it is, a
    /// delta is merged into the locus we hold when its `baseSequence` is ours (`apply_event`); a GET happens for an
    /// event without enough (no locus, or a delta we can't apply: missed / out-of-order event), at most every 2 s, and as
    /// the slow fallback poll (`poll_due`). The official client caches an out-of-order event and syncs the locus when the
    /// missing one doesn't come in time; the next tick's GET (≤ 2 s) plays that part here.
    pub(crate) fn locus_event(&mut self, d: &Value) -> Result<(), String> {
        if let Some(m) = roap_in(d) {
            let ours = self.calls.cur.as_ref().is_some_and(|c| d["correlationId"].as_str().is_none_or(|x| x == c.correlation_id));
            if m.0 == "ERROR" { dbg_conn(&format!("call: roap ERROR ours={ours} {}", roap_error(&d["message"]))); }
            if ours { self.handle_roap(m); } else { dbg_conn(&format!("call: roap {} for another correlation id, ignored", m.0)); }
            return Ok(());
        }
        let (tick, now) = (d["tick"].as_bool() == Some(true), Instant::now());
        if tick { self.share_tick(); self.share_rx_tick(); }
        let (url, carried) = match self.calls.cur.as_mut() {
            Some(c) if tick => {
                if c.stats_at.is_none_or(|t| now.saturating_duration_since(t) >= STATS_EVERY) {
                    c.stats_at = Some(now);
                    if let Some(m) = c.media.as_ref() {
                        let share = c.share.as_ref().map_or(String::new(), |s| format!(" share_pictures={}", s.sent()));
                        dbg_conn(&format!("call: media {:?} rx={} tx={} rx_video={}{share}", m.state(), m.rx_packets.load(Ordering::Relaxed), m.tx_frames.load(Ordering::Relaxed), m.rx_video.load(Ordering::Relaxed)));
                    }
                }
                if !poll_due(c.read_at, now, c.last == Some(CallState::Connected), c.stale) {
                    let v = c.view.clone();
                    return v.map_or(Ok(()), |v| self.evaluate(&v)); // the known state again: media may have connected meanwhile
                }
                (c.url.clone(), None)
            }
            Some(c) => {
                let u = locus_url_of(d);
                if !(u.is_empty() || same_locus(&u, &c.url)) { return Ok(()) }
                let ev = d["eventType"].as_str().unwrap_or("?").chars().filter(|c| c.is_ascii_graphic()).take(48).collect::<String>();
                match apply_event(&c.locus, c.seq, d) {
                    Apply::Discard => { dbg_conn(&format!("call: {ev} seq={} not newer than held seq={}, discarded", locus_seq(locus_of(d)), c.seq)); return Ok(()) }
                    Apply::Use(l) => (c.url.clone(), Some(l)),
                    Apply::Read => {
                        let l = locus_of(d);
                        if is_delta(l) { dbg_conn(&format!("call: {ev} delta base={} held seq={}: out of sync, re-reading the locus", seq_of(&l["baseSequence"]), c.seq)); }
                        if c.read_at.is_some_and(|t| now.saturating_duration_since(t) < MIN_READ_GAP) { c.stale = true; return Ok(()) } // the next tick reads
                        (c.url.clone(), None)
                    }
                }
            }
            None => {
                let u = locus_url_of(d);
                if tick || u.is_empty() { return Ok(()) }
                // No call of ours: only a locus that can be an incoming 1:1 call is read (meetings and others are not).
                if !idle_event_needs_read(d, &self.my_email) { return Ok(()) }
                (u, if is_delta(locus_of(d)) { None } else { full_locus(d).cloned() }) // a delta alone is no whole locus
            }
        };
        if !webex_host(&url) { return Ok(()); }
        let (l, how) = match carried {
            Some(l) => (l, "event"),
            None => {
                let r = self.http("GET", &url, None)?;
                if let Some(c) = self.calls.cur.as_mut() { c.read_at = Some(now); c.stale = false; }
                (locus_of(&r).clone(), "read")
            }
        };
        let seq = locus_seq(&l);
        if let Some(c) = self.calls.cur.as_mut() {
            if seq != 0 && seq < c.seq { dbg_conn("call: older locus ignored"); return Ok(()); }
            c.seq = c.seq.max(seq);
            c.locus = l.clone();
        }
        let v = parse_locus(&l, &self.my_email);
        let v = LocusView { url: if v.url.is_empty() { url.clone() } else { v.url }, ..v };
        dbg_conn(&format!("call: {how} {} seq={seq} {}", d["eventType"].as_str().filter(|_| how == "event").unwrap_or(""), describe(&v)));
        if let Some(c) = self.calls.cur.as_mut() {
            c.view = Some(v.clone());
            self.share_locus(&l);
            self.mute_sync(Some(&v));
            return self.evaluate(&v);
        }
        // A call someone else started: only real 1:1 calls that are ringing for us (never meetings), and never while busy.
        if v.kind == "CALL" && call_state(&v, false) == Some(CallState::Incoming) {
            let p = v.others.iter().find(|p| p.state == "JOINED").cloned().unwrap_or_default();
            let mut a = Active::new(v.url.clone(), v.self_id.clone(), v.self_url.clone(), false, p.name, p.email);
            (a.view, a.seq, a.read_at, a.locus) = (Some(v.clone()), seq, (how == "read").then_some(now), l.clone());
            self.calls.cur = Some(a);
            self.start_ticker();
            self.emit(CallState::Incoming);
            // Tell Locus this device is ringing (the caller then sees "ringing").
            let alert = json!({"device": self.device_body(), "correlationId": jose::uuid()});
            let _ = self.http("PUT", &format!("{}/participant/alert", v.url), Some(alert));
        }
        Ok(())
    }

    /// Reads the user's loci and reports how many active calls this account is still joined to (status line, no names).
    /// With `leave`, leaves those that THIS device is joined to (our own leftovers); anyone else's calls are left alone.
    pub(super) fn audit(&mut self, leave: bool) -> Result<(), String> {
        let v = self.http("GET", &self.url("locus", "/loci"), None)?;
        let loci = v["loci"].as_array().cloned().unwrap_or_default();
        let (mut active, mut left) = (0, 0);
        for l in &loci {
            let p = parse_locus(l, &self.my_email);
            if !(p.active && p.kind == "CALL" && p.self_state == "JOINED") { continue; }
            active += 1;
            let ours = p.self_devices.iter().any(|(u, s)| *u == self.device && s == "JOINED");
            if leave && ours && webex_host(&p.url) {
                let body = json!({"device": self.device_body(), "usingResource": null, "correlationId": jose::uuid()});
                if self.http("PUT", &format!("{}/participant/{}/leave", p.url, p.self_id), Some(body)).is_ok() { left += 1; }
            }
        }
        self.say(&format!("calls: {} loci, {active} active call(s) joined, {left} left (this device)", loci.len()));
        Ok(())
    }
}
