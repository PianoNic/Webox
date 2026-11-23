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

impl App {
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
    }

    pub(crate) fn on_call_event(&mut self, ev: CallEvent, now: f64) {
        let known = self.people.iter().chain(self.spaces.iter().flat_map(|s| s.participants.iter())).find(|p| !ev.email.is_empty() && p.email.eq_ignore_ascii_case(&ev.email)).map(|p| p.name.clone());
        let name = [known, Some(ev.name.clone()).filter(|n| !n.is_empty()), self.call.cur.as_ref().map(|c| c.name.clone()).filter(|n| !n.is_empty()), Some(ev.email.clone())]
            .into_iter().flatten().find(|n| !n.is_empty()).unwrap_or_else(|| "Unbekannt".into());
        let prev = self.call.cur.take();
        let (mut since, was_connected) = prev.as_ref().map_or((now, false), |p| (p.since, p.state == CallState::Connected));
        if ev.state == CallState::Connected && !was_connected { since = now; }
        let ended_at = if let CallState::Ended(w) = &ev.state {
            let r = match w.as_str() { "declined" => "Abgelehnt", "missed" => "Verpasst", "unanswered" => "Keine Antwort", _ => "Beendet" };
            self.call.history.insert(0, (name.clone(), ev.incoming, r));
            self.call.history.truncate(30);
            self.call.muted = false;
            self.call.mc.keypad = false; self.call.mc.keypad_text.clear();
            if let Some(c) = &self.client { c.req(Req::CallHistory); } // the session is in janus now
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
        let kind = if !p.call_sounds || self.call.sample || self.call.ring_muted { 0 } else { match state { Some(CallState::Incoming) => 1, Some(CallState::Dialing | CallState::Ringing) => 2, _ => 0 } };
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
        ui.set_width(280.0);
        ui.vertical_centered(|ui| {
            avatar_photo(ui, &mut self.icons, t, &v.name, 96.0, false, None, None, v.photo);
            ui.add_space(14.0);
            ui.label(RichText::new(&v.name).font(semibold(20.0)).color(t.text1));
            ui.add_space(4.0);
            ui.label(RichText::new(state_text(&v.state, v.secs)).size(14.0).color(t.text2));
            ui.add_space(22.0);
            if v.ended { return; }
            let row = Rect::from_center_size(ui.cursor().center_top() + Vec2::new(0.0, 26.0), Vec2::new(3.0 * 48.0 + 2.0 * 20.0, 52.0));
            let mut r = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)));
            r.spacing_mut().item_spacing.x = 20.0;
            let muted = self.call.muted;
            if round_fill(&mut r, &mut self.icons, if muted { "microphone-muted-regular" } else { "microphone-regular" }, if muted { t.badge } else { t.selected }, 48.0)
                .on_hover_text(if muted { "Stummschaltung aufheben" } else { "Stummschalten" }).clicked() { a.mute = true }
            if round_fill(&mut r, &mut self.icons, "handset-down-slash-regular", t.badge, 48.0).on_hover_text("Auflegen").clicked() { a.hang = true }
            let open = menu_is_open(&ctx, "call-devices");
            let dev = round_fill(&mut r, &mut self.icons, "speaker-regular", if open { t.accent } else { t.selected }, 48.0).on_hover_text("Audiogeräte");
            if dev.clicked() { menu_toggle(&ctx, "call-devices"); }
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
pub(crate) struct CallActions { pub hang: bool, pub answer: bool, pub mute: bool, pub message: bool, pub contact: bool, pub toggle_ring: bool, pub dismiss: bool }

mod devices;
