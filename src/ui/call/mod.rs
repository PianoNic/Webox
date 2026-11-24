//! Call UI: the in-call card (avatar, name, status / timer, mute, device menu, hang-up), the incoming-call toast
//! (320 pt wide: avatar, caller, round answer / decline buttons), ringing sounds, the audio device pickers (in the call card
//! and in Settings > Audio) and the session's call history.
//! Signalling and media live in `cloud/locus/` / `cloud/media.rs`.
//! German strings: "Verbindung wird hergestellt …", "Annehmen", "Ablehnen", "Verbunden", "Anruf abgebrochen", "Klingeln...",
//! "Mikrofon" / "Lautsprecher" from the language pack; the rest is Webox's own wording.
use crate::media::audio::{self, Choice, Dev};
use crate::cloud::locus::{CallEvent, CallReq, CallState};
use crate::media::sounds::{self, Prefs};
use super::menu::{menu_is_open, menu_close, menu_open, menu_toggle, toast_frame, webex_menu, Align, Entry, Item, Side};
use crate::*;

/// Minimum pause before the level meter tries to open a microphone again after a failed open.
const MIC_RETRY_SECS: f64 = 5.0;

pub(crate) struct Shown { pub state: CallState, pub name: String, pub email: String, pub since: f64, pub confirmed: bool, pub ended_at: Option<f64> }

#[derive(Default)]
pub(crate) struct CallUi {
    pub cur: Option<Shown>, pub history: Vec<(String, bool, &'static str)>,
    choice: Option<Choice>, pub(crate) muted: bool, pub(crate) open_menu: bool,
    /// Our screen share runs (the cloud thread says so), or a pick went to it and the answer is pending.
    pub(crate) sharing: bool, pub(crate) share_pending: bool,
    /// The other side shares (their name): the call window shows it as the stage. Its texture and the picture number
    /// it holds (a new picture is uploaded once, never per frame).
    pub(crate) remote_share: Option<String>, share_tex: Option<egui::TextureHandle>, share_seq: u64,
    // Separate-window state, offline sample mode (no sound, no microphone), toast's ring switch.
    pub(crate) win: super::call_win::WinState, pub(crate) sample: bool, pub(crate) ring_muted: bool,
    /// Plays ringtone / ringback / test tones (its own streams, on the ringtone or speaker device).
    /// Each engine with its device open until it answered; `ring_devs`: the worker listing the devices for "ring on all".
    ring: Vec<(audio::Engine, Option<std::sync::mpsc::Receiver<Result<String, String>>>)>, ring_kind: u8, prefs: Option<Prefs>,
    ring_devs: Option<std::sync::mpsc::Receiver<Vec<String>>>,
    /// One-shot sounds (message sounds, call-end tones, tests): the engine and, until it answered, its device open.
    once: Vec<(audio::Engine, Option<std::sync::mpsc::Receiver<Result<String, String>>>)>,
    /// Capture-only engine for the level meter while a device picker is on screen.
    mic_test: Option<audio::Engine>, mic_seen: f64,
    /// When opening the meter's microphone last failed: retried at most every `MIC_RETRY_SECS`, never per frame
    /// (opening a device blocks, for a Bluetooth headset for a long time).
    mic_fail_at: Option<f64>,
    /// Answer of the meter microphone's asynchronous open (the page never waits for the device).
    mic_opening: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// When starting the ringtone / ringback last failed (no output device opened, or the sound file can't be decoded):
    /// retried at most every `MIC_RETRY_SECS` while the call rings, never per frame.
    ring_fail_at: Option<std::time::Instant>,
    ins: Vec<Dev>, outs: Vec<Dev>, listed_at: f64,
    /// Device enumeration runs on a worker thread (it blocks for a noticeable time on Windows); the answer lands here.
    dev_job: Option<std::sync::mpsc::Receiver<(Vec<Dev>, Vec<Dev>)>>,
    // Call history, calendar, scheduler, keypad (ui/meetcall.rs).
    pub(crate) mc: super::meetcall::McState,
    /// Settings > Video: camera list and the live preview (released when the page is left).
    pub(crate) cam: super::settings::video_page::CamUi,
    /// The ringing call was ignored ("Anruf ignorieren"): toast gone, no local ring, the call is not declined (my other
    /// devices keep ringing until the caller gives up). Per call, like `ring_muted` and `reply_open`.
    pub(crate) ignored: bool,
    /// The toast shows the quick replies ("Anruf ablehnen und mit Textnachricht antworten") instead of its buttons.
    pub(crate) reply_open: bool,
    /// The caller whose ringing call was declined because of the block list (once per call; cleared when it ends).
    blocked: Option<String>,
}

/// Quick replies of "Anruf ablehnen und mit Textnachricht antworten". The official desktop app has the action (keyboard
/// shortcut "Decline call and reply with a text") but ships no reply texts, so these are Webox's own; the toast's last
/// entry opens the chat for a custom message ("Schreiben Sie eine benutzerdefinierte Nachricht.").
pub(crate) const QUICK_REPLIES: [&str; 3] = ["Kann gerade nicht sprechen.", "Ich rufe gleich zurück.", "Bin gerade in einem Meeting."];

/// Which sound plays: 0 none, 1 ringtone (incoming), 2 ringback (my outgoing call rings). An incoming call stays silent when
/// call sounds are off, the toast's bell muted it, it was ignored, or "Bitte nicht stören" / Ruhezeiten are on (`silenced`,
/// which mutes notifications in the official app); my own outgoing ringback is not a notification and still plays.
pub(crate) fn ring_kind(call_sounds: bool, sample: bool, muted: bool, ignored: bool, silenced: bool, state: Option<&CallState>) -> u8 {
    if !call_sounds || sample { return 0; }
    match state {
        Some(CallState::Incoming) if !(muted || ignored || silenced) => 1,
        Some(CallState::Dialing | CallState::Ringing) => 2,
        _ => 0,
    }
}

/// The caller is on the local block list (Einstellungen > Anrufe > Sperrliste): an entry with an @ matches the e-mail
/// (case-insensitive); any other entry is a phone number and matches a caller address whose user part is that number
/// (digits compared, so "+41 44 123 45 67" matches "+41441234567@example.com").
pub(crate) fn is_blocked(list: &[String], email: &str) -> bool {
    let email = email.trim();
    if email.is_empty() { return false; }
    let user = email.split('@').next().unwrap_or("");
    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
    let phone_like = !user.is_empty() && user.chars().all(|c| c.is_ascii_digit() || " +-()./".contains(c));
    list.iter().map(|e| e.trim()).filter(|e| !e.is_empty()).any(|e| {
        if e.contains('@') { e.eq_ignore_ascii_case(email) } else { let d = digits(e); phone_like && d.len() >= 3 && d == digits(user) }
    })
}

/// German status text of the card / toast.
pub(crate) fn state_text(s: &CallState, secs: u64) -> String {
    match s {
        CallState::Dialing | CallState::Connecting => "Verbindung wird hergestellt…".into(),
        CallState::Ringing => "Klingeln...".into(), // pack: QObject "Ringing..."
        CallState::Incoming => "Eingehender Anruf".into(),
        CallState::Connected => format!("Verbunden · {:02}:{:02}", secs / 60, secs % 60),
        CallState::Ended(w) => match w.as_str() { "declined" => "Anruf abgelehnt", "unanswered" => "Keine Antwort", "missed" => "Verpasster Anruf", _ => "Anruf beendet" }.into(),
    }
}

/// An ended call that only rang here (incoming, never answered on this device) closes without a call window, end
/// screen or tone: I declined it, the caller cancelled, or another of my devices took it.
pub(crate) fn ends_quietly(incoming: bool, shown: Option<&CallState>) -> bool {
    incoming && shown.is_none_or(|s| *s == CallState::Incoming)
}

/// The caller's existing 1:1 space (where a quick reply goes), if there is one.
pub(crate) fn direct_space_with(spaces: &[Space], email: &str) -> Option<String> {
    if email.is_empty() { return None; }
    spaces.iter().find(|s| s.direct && s.participants.iter().any(|p| p.email.eq_ignore_ascii_case(email))).map(|s| s.id.clone())
}

impl App {
    /// "Anruf ablehnen und mit Textnachricht antworten": declines the ringing call, then sends `text` to the caller's 1:1
    /// space (created when there is none yet). Offline sample: the made-up call just goes, nothing is sent.
    pub(crate) fn decline_with_reply(&mut self, email: &str, text: &str, now: f64) {
        if self.call.sample { self.call.cur = None; return; }
        self.hang_up();
        let Some(c) = &self.client else { return };
        if email.is_empty() { return; }
        match direct_space_with(&self.spaces, email) {
            Some(id) => c.req(Req::Send(id, text.to_string())),
            None => c.req(Req::Direct { who: email.to_string(), text: text.to_string(), invite: false }),
        }
        self.toast("Nachricht gesendet", now);
    }

    /// Places a call to `email` (checked again, and before any request, in the cloud thread).
    pub(crate) fn start_call(&mut self, email: &str, name: &str, now: f64) {
        if self.call.cur.as_ref().is_some_and(|c| c.ended_at.is_none()) { self.toast("Es läuft bereits ein Anruf.", now); return; }
        if let Err(e) = cloud::locus::call_allowed(email, std::env::var(cloud::locus::GUARD_ENV).ok().as_deref()) { self.toast(e, now); return; }
        let choice = self.choice();
        let Some(c) = &self.client else { self.toast("Nicht verbunden", now); return };
        c.req(Req::Call(CallReq::Dial(email.trim().to_string(), choice)));
        self.call.muted = false;
        self.call.cur = Some(Shown { state: CallState::Dialing, name: name.to_string(), email: email.trim().to_string(), since: now, confirmed: false, ended_at: None });
    }

    pub(crate) fn hang_up(&mut self) {
        // Queued behind a dial that is still in flight; the card closes when the cloud thread reports the end
        // (or the dial's error arrives, see `call_failed`).
        if let Some(c) = &self.client { c.req(Req::Call(CallReq::HangUp)); }
    }
    fn answer_call(&mut self) {
        let choice = self.choice();
        if let Some(c) = &self.client { c.req(Req::Call(CallReq::Answer(choice))); }
    }

    /// An error came from the cloud thread: if our dial never got confirmed, it failed, so close the card.
    pub(crate) fn call_failed(&mut self) {
        if self.call.cur.as_ref().is_some_and(|c| !c.confirmed) { self.call.cur = None; }
        self.call.share_pending = false; // a share start that failed says so with its own error
    }

    /// The other side's share started (their name) or ended.
    pub(crate) fn on_remote_share(&mut self, who: Option<String>) {
        if who.is_none() { self.call.share_tex = None; self.call.share_seq = 0; }
        self.call.remote_share = who;
    }

    /// The received share's newest picture and its size, uploaded to the GPU only when a new one arrived (the decoder
    /// thread made the image; the same `Arc` goes up, nothing is copied here). None while no picture has come yet.
    pub(crate) fn remote_share_picture(&mut self, ctx: &egui::Context) -> Option<(egui::TextureId, Vec2)> {
        self.call.remote_share.as_ref()?;
        if let Some(p) = cloud::share_rx::latest().filter(|p| p.seq != self.call.share_seq) {
            let img = egui::ImageData::Color(p.image.clone());
            match &mut self.call.share_tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.call.share_tex = Some(ctx.load_texture("call-remote-share", img, egui::TextureOptions::LINEAR)),
            }
            self.call.share_seq = p.seq;
        }
        self.call.share_tex.as_ref().map(|t| (t.id(), t.size_vec2()))
    }

    /// The call window's share button: opens the screen / window picker, or stops our share.
    pub(crate) fn share_button(&mut self, now: f64) {
        if self.call.sharing { if let Some(c) = &self.client { c.req(Req::Call(CallReq::Share(None))); } self.call.sharing = false; return; }
        if self.call.share_pending { return; }
        if let Some(why) = crate::media::share::unavailable() { self.toast(why, now); return; }
        super::share_picker::open();
    }

    /// The picker's choice goes to the call (each frame; nothing when the dialog was closed without a pick).
    fn take_share_choice(&mut self, now: f64) {
        let Some(ch) = super::share_picker::take_choice() else { return };
        let live = self.call.cur.as_ref().is_some_and(|c| c.ended_at.is_none() && c.state == CallState::Connected);
        if !live { self.toast("Bildschirmfreigabe ist erst möglich, wenn der Anruf verbunden ist.", now); return; }
        let Some(c) = &self.client else { self.toast("Nicht verbunden", now); return };
        c.req(Req::Call(CallReq::Share(Some(cloud::locus::share::ShareReq { source: ch.source, opts: ch.opts }))));
        self.call.share_pending = true;
    }

    /// "Bitte nicht stören" or Ruhezeiten: an incoming call does not ring (the toast still shows, silent).
    pub(crate) fn calls_silenced(&self) -> bool { self.me.status == "DoNotDisturb" || self.quiet_now() }

    pub(crate) fn on_call_event(&mut self, ev: CallEvent, now: f64) {
        let prev_incoming = self.call.cur.as_ref().is_some_and(|c| c.state == CallState::Incoming && c.ended_at.is_none());
        if ev.incoming && ev.state == CallState::Incoming {
            // Block list: declined before anything rings or shows (once per call; the locus repeats the ringing state).
            let list: Vec<String> = self.settings["block_list"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            if is_blocked(&list, &ev.email) {
                if self.call.blocked.as_deref() != Some(ev.email.as_str()) { self.call.blocked = Some(ev.email.clone()); self.hang_up(); }
                self.call.cur = None;
                return;
            }
            // A new ringing call: the toast's per-call switches start fresh.
            if !prev_incoming { self.call.ignored = false; self.call.reply_open = false; self.call.ring_muted = false; }
        }
        let known = self.people.iter().chain(self.spaces.iter().flat_map(|s| s.participants.iter())).find(|p| !ev.email.is_empty() && p.email.eq_ignore_ascii_case(&ev.email)).map(|p| p.name.clone());
        let name = [known, Some(ev.name.clone()).filter(|n| !n.is_empty()), self.call.cur.as_ref().map(|c| c.name.clone()).filter(|n| !n.is_empty()), Some(ev.email.clone())]
            .into_iter().flatten().find(|n| !n.is_empty()).unwrap_or_else(|| "Unbekannt".into());
        let prev = self.call.cur.take();
        let (mut since, was_connected) = prev.as_ref().map_or((now, false), |p| (p.since, p.state == CallState::Connected));
        if ev.state == CallState::Connected && !was_connected { since = now; }
        let ended_at = if let CallState::Ended(w) = &ev.state {
            let blocked = self.call.blocked.take().is_some_and(|b| b.eq_ignore_ascii_case(&ev.email));
            let r = match w.as_str() { _ if blocked => "Blockiert", "declined" => "Abgelehnt", "missed" => "Verpasst", "unanswered" => "Keine Antwort", "elsewhere" => "Anderswo angenommen", _ => "Beendet" };
            self.call.history.insert(0, (name.clone(), ev.incoming, r));
            self.call.history.truncate(30);
            self.call.muted = false;
            self.call.sharing = false; self.call.share_pending = false;
            self.on_remote_share(None);
            super::share_picker::close();
            self.call.mc.keypad = false; self.call.mc.keypad_text.clear();
            if let Some(c) = &self.client { c.req(Req::CallHistory); } // the session is in janus now
            if w == "missed" && ev.incoming && !blocked { self.on_missed_call(&name, &ev.email, now); } // rail badge + toast (calls_page)
            // An incoming call that ends while it still rings here (I declined it, the caller cancelled, another of my
            // devices took it) just stops ringing: no call window, no "Anruf abgelehnt" screen, no tone. Like the official
            // client, which only removes its incoming-call notification.
            if ends_quietly(ev.incoming, prev.as_ref().map(|p| &p.state)) { self.call.cur = None; return; }
            // Call-end tone (see sounds.rs).
            let tone = match w.as_str() { "declined" => Some("BusyTone"), "unanswered" => Some("FastBusy"), "left" if was_connected => Some("FastBusy"), _ => None };
            let p = self.prefs();
            if let (Some(id), true) = (tone, p.call_sounds) { let out = self.choice().output; self.play_once(id, &out, false, p.ringer_volume); }
            Some(now)
        } else { None };
        self.call.cur = Some(Shown { state: ev.state, name, email: ev.email, since, confirmed: true, ended_at });
    }

    /// Ringtone (incoming) / ringback (outgoing, until the callee answers) on the right output device.
    fn ring_sound(&mut self, state: Option<&CallState>) {
        use std::sync::mpsc::TryRecvError;
        let p = self.prefs();
        let kind = ring_kind(p.call_sounds, self.call.sample, self.call.ring_muted, self.call.ignored, self.calls_silenced(), state);
        if kind != self.call.ring_kind { self.call.ring.clear(); self.call.ring_devs = None; self.call.ring_kind = kind; } // stops on answer / decline / hang-up / end
        if kind == 0 { self.call.ring_fail_at = None; return; }
        // A failed start (sound file unreadable, no output device opened) is retried at most every MIC_RETRY_SECS: retrying it
        // every frame would list and open the audio devices (and re-decode the file) again and again while the call rings.
        if self.call.ring.is_empty() && self.call.ring_devs.is_none() && self.call.ring_fail_at.is_some_and(|t| t.elapsed().as_secs_f64() < MIC_RETRY_SECS) { return; }
        let id = if kind == 1 { p.ringtone.clone() } else { "Ringback".to_string() };
        let Some(pcm) = sounds::pcm(&id) else { self.call.ring_fail_at = Some(std::time::Instant::now()); return };
        if self.call.ring.is_empty() {
            // Nothing here waits for a device: "ring on all devices" lists them on a worker thread, and every device opens on
            // its engine's own thread (the ringtone queued meanwhile plays once it is open).
            let devs = match &self.call.ring_devs {
                Some(rx) => match rx.try_recv() { Ok(v) => { self.call.ring_devs = None; v } Err(TryRecvError::Empty) => return, Err(TryRecvError::Disconnected) => { self.call.ring_devs = None; vec![] } },
                None => {
                    let c = self.choice();
                    let dev = if kind == 1 && !c.ringtone.is_empty() { c.ringtone } else { c.output };
                    if kind == 1 && p.ring_all_devices {
                        let (tx, rx) = std::sync::mpsc::channel();
                        std::thread::spawn(move || { let _ = tx.send(Self::out_devices(&dev, true)); crate::ui::wake(); });
                        self.call.ring_devs = Some(rx);
                        return;
                    }
                    vec![dev]
                }
            };
            for d in devs { let e = audio::Engine::new(); let opened = e.open_output_async(&d); e.io.set_volume(p.ringer_volume); self.call.ring.push((e, Some(opened))); }
        }
        // Engines whose device didn't open go; the call view repaints every 200 ms while it rings, so the answers are seen.
        self.call.ring.retain_mut(|(_, opened)| match opened.as_ref().map(|rx| rx.try_recv()) {
            None | Some(Err(TryRecvError::Empty)) => true,
            Some(Ok(Ok(_))) => { *opened = None; true }
            Some(Ok(Err(_)) | Err(TryRecvError::Disconnected)) => false,
        });
        if self.call.ring.is_empty() { self.call.ring_fail_at = Some(std::time::Instant::now()); return; }
        self.call.ring_fail_at = None;
        // Loop: refill whenever less than half a second is queued (ringtone / ringback files include their own pauses).
        for (e, _) in &self.call.ring { e.io.set_volume(p.ringer_volume); if e.io.play_pending() < audio::RATE as usize / 2 { e.io.play_long(&pcm); } }
    }

    /// Output device names to play on: all of them (each sink once), or the chosen one ("" = system default). Listing them
    /// blocks: `all` only off the UI thread.
    fn out_devices(dev: &str, all: bool) -> Vec<String> {
        if all {
            let mut seen = HashSet::new();
            let v: Vec<String> = audio::outputs().into_iter().map(|d| d.name).filter(|n| seen.insert(n.clone())).collect();
            if !v.is_empty() { return v; }
        }
        vec![dev.to_string()]
    }

    pub(crate) fn prefs(&mut self) -> Prefs {
        if self.call.prefs.is_none() { self.call.prefs = Some(Prefs::load(&self.settings)); }
        self.call.prefs.clone().unwrap_or_default()
    }
    pub(crate) fn save_prefs(&mut self, p: Prefs) {
        p.store(&mut self.settings);
        system::save_settings(&self.cfg, &self.settings);
        self.call.prefs = Some(p);
    }

    /// The Settings pages change snd_* themselves; drop the cached prefs.
    pub(crate) fn reset_prefs_cache(&mut self) { self.call.prefs = None; }

    pub(crate) fn play_once(&mut self, id: &str, dev: &str, all: bool, vol: f32) {
        let Some(pcm) = sounds::pcm(id) else { return };
        for d in Self::out_devices(dev, all) {
            // The device opens on the engine's own thread, so an incoming message never makes the UI wait for it (tens of ms, up to
            // 10 s for a stuck device). A failed open drops the engine (see `prune_once`). Speaker only: opening the default
            // microphone too failed the whole open when it was busy or blocked, and switched Bluetooth earbuds to hands-free
            // mode, which mutes the stereo output the sound was sent to.
            let e = audio::Engine::new();
            let opened = e.open_output_async(&d);
            e.io.set_volume(vol); e.io.play_long(&pcm);
            self.call.once.push((e, Some(opened)));
        }
        crate::ui::wake(); // so `prune_once` runs and schedules the frame that releases the device
    }

    /// Drops one-shot sounds whose device didn't open or that have played out (which closes the device), and asks for the
    /// frame that will do it: the window has no periodic redraw, so a finished sound must not wait for the next input to
    /// release its output stream.
    fn prune_once(&mut self, ctx: &egui::Context) {
        use std::sync::mpsc::TryRecvError;
        self.call.once.retain_mut(|(e, opened)| {
            if let Some(rx) = opened {
                match rx.try_recv() {
                    Ok(Ok(_)) => *opened = None,
                    Ok(Err(_)) | Err(TryRecvError::Disconnected) => return false, // no device: nothing will play
                    Err(TryRecvError::Empty) => return true,
                }
            }
            e.io.play_pending() > 0
        });
        if self.call.once.is_empty() { return; }
        let opening = self.call.once.iter().any(|(_, o)| o.is_some());
        let left = self.call.once.iter().map(|(e, _)| e.io.play_pending()).max().unwrap_or(0) as f64 / audio::RATE as f64;
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(if opening { 0.05 } else { left + 0.05 }));
    }

    /// The new-message sound for a chat that isn't open: per category, off in Nicht stören or when switched off.
    pub(crate) fn message_sound(&mut self, space_id: &str) {
        let p = self.prefs();
        if !p.msg_sounds || self.me.status == "DoNotDisturb" || self.quiet_now() { return; } // + Ruhezeiten
        let (direct, fav) = self.spaces.iter().find(|s| s.id == space_id).map_or((false, false), |s| (s.direct, s.favorite));
        let id = if fav { p.msg_favorite } else if direct { p.msg_dm } else { p.msg_space };
        if id.is_empty() { return; }
        let out = self.choice().output;
        self.play_once(&id, &out, false, p.msg_volume);
    }

    /// `--screenshot` helper: shows the call UI with made-up data so the layout can be compared (nothing is sent anywhere).
    pub(crate) fn preview_call(&mut self, kind: &str, now: f64) {
        let state = match kind { "incoming" => CallState::Incoming, "ringing" => CallState::Ringing, _ => CallState::Connected };
        self.call.open_menu = kind == "menu";
        self.call.cur = Some(Shown { state, name: "Max Muster".into(), email: String::new(), since: now - 83.0, confirmed: true, ended_at: None });
    }

    // The call surface. `call_view` / `incoming_toast_ui` / `call_card_ui` / `apply_call_actions` draw into any `egui::Ui`
    // (they never touch the central panel), so the surface can be hosted by an Area in the main window (`call_overlay`) or by
    // a separate egui viewport's CentralPanel; the device menu uses only the Context, so it follows its host viewport.

    /// What the call surface needs for one frame; `None` when there is nothing to show.
    pub(crate) fn call_view(&mut self, ctx: &egui::Context, now: f64) -> Option<CallView> {
        if std::mem::take(&mut self.call.open_menu) { menu_open(ctx, "call-devices"); } // --screenshot preview only
        if self.call.cur.as_ref().is_some_and(|c| c.ended_at.is_some_and(|e| now - e > 3.0)) { self.call.cur = None; menu_close(ctx, "call-devices"); }
        if self.call.mic_test.is_some() && now - self.call.mic_seen > 1.0 { self.call.mic_test = None; self.call.mic_opening = None; } // meter not on screen any more
        self.call.cam.idle(now); // camera preview not on screen any more: stop the camera
        self.prune_once(ctx);
        let live = self.call.cur.as_ref().filter(|c| c.ended_at.is_none()).map(|c| c.state.clone());
        self.ring_sound(live.as_ref());
        self.take_share_choice(now);
        let c = self.call.cur.as_ref()?;
        let (state, name, email, since, ended) = (c.state.clone(), c.name.clone(), c.email.clone(), c.since, c.ended_at.is_some());
        let secs = if state == CallState::Connected { (now - since).max(0.0) as u64 } else { 0 };
        let photo = self.photo_for(ctx, "", &email);
        ctx.request_repaint_after(std::time::Duration::from_millis(if menu_is_open(ctx, "call-devices") { 60 } else { 200 }));
        Some(CallView { state, name, email, secs, ended, photo })
    }

    /// The incoming-call toast content (288 wide), in the toolkit's toast frame.
    pub(crate) fn incoming_toast_ui(&mut self, ui: &mut egui::Ui, t: &Th, v: &CallView) -> CallActions {
        let mut a = CallActions::default();
        toast_frame(t).inner_margin(16).show(ui, |ui| {
            ui.set_width(288.0);
            ui.horizontal(|ui| {
                avatar_photo(ui, &mut self.icons, t, &v.name, 48.0, false, None, None, v.photo);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&v.name).font(semibold(15.0)).color(t.text1));
                    ui.label(RichText::new(state_text(&v.state, 0)).size(13.0).color(t.text2));
                });
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                if round_fill(ui, &mut self.icons, "handset-regular", t.green, 40.0).on_hover_text("Annehmen").clicked() { a.answer = true }
                if round_fill(ui, &mut self.icons, "handset-down-slash-regular", t.badge, 40.0).on_hover_text("Ablehnen").clicked() { a.hang = true }
            });
        });
        a
    }

    /// The call card content (280 wide): avatar, name, status / timer, mute | hang up | devices, level meter. The device
    /// button opens the toolkit balloon menu (microphones, speakers, test tone).
    pub(crate) fn call_card_ui(&mut self, ui: &mut egui::Ui, t: &Th, now: f64, v: &CallView) -> CallActions {
        let mut a = CallActions::default();
        let ctx = ui.ctx().clone();
        let share = v.state == CallState::Connected && !v.ended && self.call.remote_share.is_some();
        ui.set_width(if share { 480.0 } else { 280.0 });
        if share {
            // the other side's screen on top of the card (the in-window fallback of the call window's stage)
            let who = self.call.remote_share.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| v.name.clone());
            ui.label(RichText::new(format!("{who} teilt den Bildschirm")).font(semibold(13.0)).color(t.text1));
            let (r, _) = ui.allocate_exact_size(Vec2::new(480.0, 270.0), Sense::hover());
            ui.painter().rect_filled(r, 4.0, Color32::BLACK);
            match self.remote_share_picture(&ctx) {
                Some((id, size)) => { ui.painter().image(id, Rect::from_center_size(r.center(), crate::ui::call_win::fit_size(size, r.size())), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
                None => { ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Freigabe wird geladen…", FontId::proportional(14.0), t.text2); }
            }
            ui.add_space(14.0);
        }
        ui.vertical_centered(|ui| {
            avatar_photo(ui, &mut self.icons, t, &v.name, 96.0, false, None, None, v.photo);
            ui.add_space(14.0);
            ui.label(RichText::new(&v.name).font(semibold(20.0)).color(t.text1));
            ui.add_space(4.0);
            ui.label(RichText::new(state_text(&v.state, v.secs)).size(14.0).color(t.text2));
            ui.add_space(22.0);
            if v.ended { return; }
            let row = Rect::from_center_size(ui.cursor().center_top() + Vec2::new(0.0, 26.0), Vec2::new(4.0 * 48.0 + 3.0 * 20.0, 52.0));
            let mut r = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)));
            r.spacing_mut().item_spacing.x = 20.0;
            let muted = self.call.muted;
            if round_fill(&mut r, &mut self.icons, if muted { "microphone-muted-regular" } else { "microphone-regular" }, if muted { t.badge } else { t.selected }, 48.0)
                .on_hover_text(if muted { "Stummschaltung aufheben" } else { "Stummschalten" }).clicked() { a.mute = true }
            if round_fill(&mut r, &mut self.icons, "handset-down-slash-regular", t.badge, 48.0).on_hover_text("Auflegen").clicked() { a.hang = true }
            let open = menu_is_open(&ctx, "call-devices");
            let dev = round_fill(&mut r, &mut self.icons, "speaker-regular", if open { t.accent } else { t.selected }, 48.0).on_hover_text("Audiogeräte");
            if dev.clicked() { menu_toggle(&ctx, "call-devices"); }
            if v.state == CallState::Connected {
                let sharing = self.call.sharing;
                if round_fill(&mut r, &mut self.icons, "share-screen-regular", if sharing { t.green } else { t.selected }, 48.0)
                    .on_hover_text(if sharing { "Freigabe beenden" } else { "Inhalte freigeben" }).clicked() { self.share_button(now); }
            }
            ui.allocate_rect(row, Sense::hover());
            ui.add_space(16.0);
            self.level_bar(ui, t, now, 220.0);
            // Volume of what we hear.
            let mut c = self.choice();
            let before = c.volume;
            ui.add_space(6.0);
            ui.add(egui::Slider::new(&mut c.volume, 0.0..=1.0).show_value(false));
            if c.volume != before { self.save_choice(c); }
            self.device_menu(&ctx, t, now, dev.rect);
        });
        a
    }

    /// Applies what the user pressed on the call surface.
    pub(crate) fn apply_call_actions(&mut self, a: CallActions) {
        if a.mute {
            self.call.muted = !self.call.muted;
            if let Some(c) = &self.client { c.req(Req::Call(CallReq::Mute(self.call.muted))); }
        }
        if a.answer { self.answer_call(); }
        if a.hang { self.hang_up(); }
    }

    /// Hosts the call surface in the main window (Areas over everything); call once per frame. A separate viewport would
    /// call `call_view` / `incoming_toast_ui` / `call_card_ui` / `apply_call_actions` from its own CentralPanel instead.
    pub(crate) fn call_overlay(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        // Separate OS windows (call window, calling dock, incoming toast); the in-window overlay below stays as the fallback
        // (setting "call_separate_windows" = false, or a backend without multi-viewport support).
        self.sample_call_hook(ctx, now);
        self.meetcall_overlays(ctx, t, now); // scheduler dialog (opened from any page)
        if self.separate_call_windows(ctx) { self.call_windows(ctx, t, now); return; }
        let Some(v) = self.call_view(ctx, now) else { return };
        let mut actions = CallActions::default();
        if v.state == CallState::Incoming {
            actions = self.call_note(ctx, &v); // Wayland: the desktop notification too (deduped per ringing)
            egui::Area::new(egui::Id::new("incoming-call")).anchor(egui::Align2::RIGHT_TOP, [-20.0, 64.0]).order(egui::Order::Foreground)
                .show(ctx, |ui| { let a = self.incoming_toast_ui(ui, t, &v); actions.answer |= a.answer; actions.hang |= a.hang; actions.dismiss |= a.dismiss; });
        } else {
            egui::Area::new(egui::Id::new("call-card")).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).order(egui::Order::Foreground).show(ctx, |ui| {
                egui::Frame::default().fill(t.card).stroke(Stroke::new(1.0_f32, t.sep)).corner_radius(12.0).inner_margin(28)
                    .shadow(egui::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(140) })
                    .show(ui, |ui| actions = self.call_card_ui(ui, t, now, &v));
            });
        }
        self.apply_call_actions(actions);
    }
}

/// One frame of call data for the surface (host-independent).
pub(crate) struct CallView { pub state: CallState, pub name: String, pub email: String, pub secs: u64, pub ended: bool, pub photo: Option<egui::TextureId> }
/// What the user pressed.
#[derive(Default)]
pub(crate) struct CallActions { pub hang: bool, pub answer: bool, pub mute: bool, pub message: bool, pub contact: bool, pub toggle_ring: bool, pub dismiss: bool,
    /// A quick reply was picked: decline, then send this text to the caller's 1:1 space.
    pub reply: Option<String> }

mod devices;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejecting_or_a_cancelled_ring_opens_no_window() {
        assert!(ends_quietly(true, Some(&CallState::Incoming)), "I declined / the caller cancelled / taken elsewhere");
        assert!(ends_quietly(true, None));
        assert!(!ends_quietly(true, Some(&CallState::Connected)), "an answered call shows its end");
        assert!(!ends_quietly(true, Some(&CallState::Connecting)));
        assert!(!ends_quietly(false, Some(&CallState::Ringing)), "my outgoing call the other side declined still says so");
    }

    #[test]
    fn dnd_quiet_hours_and_ignore_silence_only_the_incoming_ring() {
        let inc = Some(&CallState::Incoming);
        assert_eq!(ring_kind(true, false, false, false, false, inc), 1, "a normal incoming call rings");
        assert_eq!(ring_kind(true, false, false, false, true, inc), 0, "Bitte nicht stören / Ruhezeiten: no ringtone");
        assert_eq!(ring_kind(true, false, false, true, false, inc), 0, "Anruf ignorieren stops the local ring");
        assert_eq!(ring_kind(true, false, true, false, false, inc), 0, "the toast's bell");
        assert_eq!(ring_kind(true, false, false, false, true, Some(&CallState::Ringing)), 2, "my own outgoing ringback still plays under DND");
        assert_eq!(ring_kind(false, false, false, false, false, inc), 0);
        assert_eq!(ring_kind(true, true, false, false, false, inc), 0, "--sample makes no sound");
        assert_eq!(ring_kind(true, false, false, false, false, Some(&CallState::Connected)), 0);
    }

    #[test]
    fn block_list_matches_email_or_phone_number() {
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_blocked(&l(&["Spam@Example.com"]), "spam@example.com"));
        assert!(!is_blocked(&l(&["spam@example.com"]), "spammer@example.com"));
        assert!(is_blocked(&l(&["+41 44 123 45 67"]), "+41441234567@example.com"));
        assert!(!is_blocked(&l(&["+41 44 123 45 67"]), "max.muster@example.com"), "a number never matches a name address");
        assert!(!is_blocked(&l(&["12"]), "12@example.com"), "too short to be a number");
        assert!(!is_blocked(&l(&["", "  "]), "a@example.com"));
        assert!(!is_blocked(&l(&["a@example.com"]), ""), "an unknown caller is not blocked");
    }

    #[test]
    fn quick_reply_goes_to_the_existing_direct_space() {
        let p = |e: &str| Person { email: e.into(), ..Default::default() };
        let spaces = vec![
            Space { id: "group".into(), direct: false, participants: vec![p("me@example.com"), p("max@example.com"), p("eva@example.com")], ..Default::default() },
            Space { id: "dm".into(), direct: true, participants: vec![p("me@example.com"), p("Max@Example.com")], ..Default::default() },
        ];
        assert_eq!(direct_space_with(&spaces, "max@example.com").as_deref(), Some("dm"));
        assert_eq!(direct_space_with(&spaces, "eva@example.com"), None, "no 1:1 yet: a new one is created");
        assert_eq!(direct_space_with(&spaces, ""), None);
    }
}
