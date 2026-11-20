//! The session side of calls: dialling, answering, hanging up and following the locus.
use super::*;

impl Session {
    pub(crate) fn call_req(&mut self, r: CallReq) -> Result<(), String> {
        match r {
            CallReq::Dial(a, c) => self.dial(&a, &c),
            CallReq::Answer(c) => self.answer(&c),
            CallReq::HangUp => self.hang_up(),
            CallReq::Mute(m) => { self.calls.muted = m; if let Some(a) = self.calls.cur.as_ref().and_then(|c| c.media.as_ref()) { a.muted.store(m, Ordering::Relaxed); } Ok(()) }
            CallReq::Devices(c) => {
                if self.calls.cur.is_some() { if let Some(e) = &self.calls.engine { c.apply_smart(&e.io); e.open(&c.input, &c.output)?; e.io.set_volume(c.volume); } }
                Ok(())
            }
            // Smart Audio changed: the capture callback reads the mode on every block, no device reopen
            CallReq::SmartMic(m) => { if let Some(e) = &self.calls.engine { e.io.set_smart(m); } Ok(()) }
            CallReq::Audit => self.audit(false),
            CallReq::Cleanup => self.audit(true),
            CallReq::Dtmf(t) => self.dtmf(&t),
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
        if let Some(c) = self.calls.cur.take() { if let Some(m) = c.media { m.close(); } }
        if let Some(e) = &self.calls.engine { e.io.set_call(false); e.close(); }
        self.calls.live.store(false, Ordering::SeqCst);
    }

    /// Opens the audio devices and the WebRTC peer and returns it with the ROAP OFFER SDP (ICE gathering complete).
    pub(super) fn prepare_media(&mut self, c: &audio::Choice) -> Result<(Media, String), String> {
        let engine = self.calls.engine.get_or_insert_with(audio::Engine::new);
        let info = engine.open(&c.input, &c.output)?;
        engine.io.set_volume(c.volume);
        c.apply_smart(&engine.io);
        engine.io.set_call(true); // echo cancellation and gain control on the call microphone
        dbg_conn(&format!("call: audio open ({} chars)", info.len()));
        let m = Media::new(engine.io.clone(), None)?;
        m.muted.store(self.calls.muted, Ordering::Relaxed);
        let offer = m.offer().map_err(|e| { dbg_conn(&format!("call: offer failed: {}", scrub(&e))); e })?;
        dbg_conn(&format!("call: offer ready ({} bytes, {} candidates)", offer.len(), offer.matches("a=candidate").count()));
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
        let r = match self.http("POST", &url, Some(body)) { Ok(r) => r, Err(e) => { media.close(); if let Some(en) = &self.calls.engine { en.close(); } return Err(format!("Anruf fehlgeschlagen: {}", scrub(&e))); } };
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
        let r = match self.http("POST", &format!("{url}/participant"), Some(body)) { Ok(r) => r, Err(e) => { media.close(); return Err(format!("Annehmen fehlgeschlagen: {}", scrub(&e))); } };
        self.adopt(&r, media, corr, false)
    }

    /// Takes the join answer (new call or answered call): remembers locus / self / media ids and any ready ROAP answer.
    pub(super) fn adopt(&mut self, r: &Value, media: Media, corr: String, new_call: bool) -> Result<(), String> {
        let l = locus_of(r);
        let v = parse_locus(l, &self.my_email);
        let lurl = if v.url.is_empty() { locus_url_of(r) } else { v.url.clone() };
        if lurl.is_empty() || !webex_host(&lurl) {
            media.close();
            let keys = r.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>().join(",")).unwrap_or_default();
            return Err(format!("Anruf: unerwartete Antwort (keys: {keys})"));
        }
        dbg_conn(&format!("call: joined {} media_id={}", describe(&v), r["mediaConnections"][0]["mediaId"].is_string()));
        let media_id = st(&r["mediaConnections"][0], "mediaId");
        if new_call {
            let name = v.others.first().map(|p| p.name.clone()).unwrap_or_default();
            let email = v.others.first().map(|p| p.email.clone()).unwrap_or_default();
            self.calls.cur = Some(Active { url: lurl, self_id: v.self_id.clone(), self_url: v.self_url.clone(), media_id, correlation_id: corr, outgoing: true, name, email,
                last: None, answered: true, media: Some(media), roap_seq: 1, roap_done: false });
        } else if let Some(c) = self.calls.cur.as_mut() {
            c.self_id = v.self_id.clone(); c.self_url = v.self_url.clone(); c.media_id = media_id; c.correlation_id = corr; c.answered = true; c.media = Some(media); c.roap_seq = 1; c.roap_done = false;
        }
        self.start_ticker();
        if let Some(m) = remote_roap(r) { self.handle_roap(m); }
        self.evaluate(&v)
    }

    /// PUT {self.url}/media with a ROAP message (OK / ANSWER).
    pub(super) fn send_roap(&mut self, message_type: &str, sdp: Option<&str>, seq: u32) -> Result<(), String> {
        let (url, media_id, corr) = match &self.calls.cur { Some(c) => (c.self_url.clone(), c.media_id.clone(), c.correlation_id.clone()), None => return Ok(()) };
        if !webex_host(&url) { return Err("Keine gültige Medien-Adresse.".into()); }
        let body = json!({"device": self.device_body(), "correlationId": corr, "clientMediaPreferences": {"preferTranscoding": true},
            "localMedias": [{"localSdp": roap_local_sdp(message_type, sdp, seq, self.calls.muted), "mediaId": media_id}]});
        self.http("PUT", &format!("{url}/media"), Some(body)).map(|_| ())
    }

    /// A ROAP message from the server (mercury event or inline in a response).
    pub(super) fn handle_roap(&mut self, (kind, seq, sdp): (String, u32, Option<String>)) {
        dbg_conn(&format!("call: roap {kind} seq={seq} sdp={}", sdp.is_some()));
        match kind.as_str() {
            "ANSWER" => {
                let (Some(sdp), Some(m)) = (sdp, self.calls.cur.as_ref().and_then(|c| c.media.as_ref())) else { return };
                if self.calls.cur.as_ref().is_some_and(|c| c.roap_done) { return; }
                match m.apply_answer(sdp) {
                    Ok(()) => {
                        if let Some(c) = self.calls.cur.as_mut() { c.roap_done = true; }
                        let r = self.send_roap("OK", None, seq);
                        dbg_conn(&format!("call: roap OK sent -> {}", if r.is_ok() { "ok" } else { "failed" }));
                    }
                    Err(e) => { dbg_conn(&format!("call: bad answer: {}", scrub(&e))); self.fail_media("Medienaushandlung fehlgeschlagen"); }
                }
            }
            "OFFER" => { // the server renegotiating: answer it
                let (Some(sdp), Some(m)) = (sdp, self.calls.cur.as_ref().and_then(|c| c.media.as_ref())) else { return };
                match m.answer(sdp) { Ok(a) => { let _ = self.send_roap("ANSWER", Some(&a), seq); } Err(e) => dbg_conn(&format!("call: answer failed: {}", scrub(&e))) }
            }
            "ERROR" => self.fail_media("Medienfehler"),
            _ => {} // OK, TURN_DISCOVERY_RESPONSE, SHUTDOWN: nothing to do
        }
    }

    pub(super) fn fail_media(&mut self, why: &str) {
        let _ = self.out.send(Resp::Error(why.into()));
        let _ = self.hang_up();
    }

    /// Cancel (outgoing) / leave (connected) / decline (incoming, not answered).
    pub(super) fn hang_up(&mut self) -> Result<(), String> {
        let Some(c) = self.calls.cur.as_ref() else { return Ok(()) };
        let (url, id, incoming_pending) = (c.url.clone(), c.self_id.clone(), !c.outgoing && !c.answered);
        if !webex_host(&url) { self.calls.cur = None; return Err("Ungültige Anruf-Adresse.".into()); }
        let r = if incoming_pending {
            self.http("PUT", &format!("{url}/participant/decline"), Some(json!({"device": self.device_body()})))
        } else {
            let leave = json!({"device": self.device_body(), "usingResource": null, "correlationId": jose::uuid()});
            self.http("PUT", &format!("{url}/participant/{id}/leave"), Some(leave))
        };
        dbg_conn(&format!("call: {} -> {}", if incoming_pending { "decline" } else { "leave" }, if r.is_ok() { "ok" } else { "failed" }));
        r.map_err(|e| format!("Auflegen fehlgeschlagen: {}", scrub(&e)))?;
        self.end_call(if incoming_pending { "declined" } else { "left" });
        Ok(())
    }

    /// Maps the locus + our media state to what the UI shows; Connected only with real media.
    pub(super) fn evaluate(&mut self, v: &LocusView) -> Result<(), String> {
        let Some(c) = self.calls.cur.as_ref() else { return Ok(()) };
        let (outgoing, answered) = (c.outgoing, c.answered);
        let media = c.media.as_ref().map(|m| m.state());
        let Some(mut s) = call_state(v, outgoing || answered) else { return Ok(()) };
        if s == CallState::Connected {
            match media { Some(MediaState::Connected) => {}
                Some(MediaState::Failed) => { self.fail_media("Verbindung fehlgeschlagen"); return Ok(()) }
                _ => s = CallState::Connecting }
        }
        if let CallState::Ended(w) = &s { self.end_call(w); } else { self.emit(s); }
        Ok(())
    }

    /// Mercury `locus.*` events and our own ticks while a call exists. Re-reads the locus (a full GET, no delta merging).
    pub(crate) fn locus_event(&mut self, d: &Value) -> Result<(), String> {
        if let Some(m) = roap_in(d) {
            if self.calls.cur.as_ref().is_some_and(|c| d["correlationId"].as_str().is_none_or(|x| x == c.correlation_id)) { self.handle_roap(m); }
            return Ok(());
        }
        let url = match &self.calls.cur {
            Some(c) if d["tick"].as_bool() == Some(true) => c.url.clone(),
            Some(c) => { let u = locus_url_of(d); if u.is_empty() || u == c.url { c.url.clone() } else { return Ok(()) } }
            None => {
                let u = locus_url_of(d);
                if d["tick"].as_bool() == Some(true) || u.is_empty() { return Ok(()) }
                // No call of ours: only a locus that can be an incoming 1:1 call is read (meetings and others are not).
                if !idle_event_needs_read(d, &self.my_email) { return Ok(()) }
                u
            }
        };
        if !webex_host(&url) { return Ok(()); }
        let r = self.http("GET", &url, None)?;
        let v = parse_locus(locus_of(&r), &self.my_email);
        let v = LocusView { url: if v.url.is_empty() { url.clone() } else { v.url }, ..v };
        let mdbg = self.calls.cur.as_ref().and_then(|c| c.media.as_ref()).map(|m| format!("{:?} rx={} tx={}", m.state(), m.rx_packets.load(Ordering::Relaxed), m.tx_frames.load(Ordering::Relaxed)));
        dbg_conn(&format!("call: read {} media={:?}", describe(&v), mdbg));
        if self.calls.cur.is_some() { return self.evaluate(&v); }
        // A call someone else started: only real 1:1 calls that are ringing for us (never meetings), and never while busy.
        if v.kind == "CALL" && call_state(&v, false) == Some(CallState::Incoming) {
            let p = v.others.iter().find(|p| p.state == "JOINED").cloned().unwrap_or_default();
            self.calls.cur = Some(Active { url: v.url.clone(), self_id: v.self_id.clone(), self_url: v.self_url.clone(), media_id: String::new(), correlation_id: String::new(),
                outgoing: false, name: p.name, email: p.email, last: None, answered: false, media: None, roap_seq: 0, roap_done: false });
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
