//! Audio devices of calls: the saved choice, the device lists (enumerated on a worker thread), the level meter, the device
//! pickers of Settings > Audio and the device menu behind the call card's device button; the camera list and the video
//! menu behind the call window's video button.
use super::*;
use crate::media::camera;

impl App {
    pub(crate) fn choice(&mut self) -> Choice {
        if self.call.choice.is_none() { self.call.choice = Some(Choice::load(&self.settings)); }
        self.call.choice.clone().unwrap_or_default()
    }
    pub(super) fn save_choice(&mut self, c: Choice) {
        c.store(&mut self.settings);
        system::save_settings(&self.cfg, &self.settings);
        if self.call.cur.as_ref().is_some_and(|x| x.ended_at.is_none()) { if let Some(cl) = &self.client { cl.req(Req::Call(CallReq::Devices(c.clone()))); } }
        self.call.choice = Some(c);
        self.call.mic_test = None; // reopen on the new device
        self.call.mic_opening = None;
        self.call.mic_fail_at = None;
    }
    /// Smart Audio > Mikrofonaudio changed (already stored as `smart_mic`): applies it to the cached choice, the level
    /// meter's microphone and a running call at once (the capture callback reads the mode per block, nothing reopens).
    pub(crate) fn apply_smart_mic(&mut self, m: audio::SmartMic) {
        if let Some(c) = self.call.choice.as_mut() { c.smart = m; }
        if let Some(e) = &self.call.mic_test { e.io.set_smart(m); }
        if self.call.cur.as_ref().is_some_and(|x| x.ended_at.is_none()) { if let Some(cl) = &self.client { cl.req(Req::Call(CallReq::SmartMic(m))); } }
    }
    /// Device lists, refreshed at most every `LIST_EVERY` s while a picker is visible (hot-plug). The enumeration itself runs
    /// on a worker thread so the UI never waits for it; the previous list stays on screen until the new one arrives.
    fn refresh_devices(&mut self, now: f64) {
        // Linux: listing opens every ALSA PCM (cpal probes each one), so not every 2 s.
        const LIST_EVERY: f64 = if cfg!(target_os = "linux") { 10.0 } else { 2.0 };
        if let Some(rx) = &self.call.dev_job {
            match rx.try_recv() {
                Ok((ins, outs)) => { self.call.ins = ins; self.call.outs = outs; self.call.dev_job = None; }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.call.dev_job = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
            }
        }
        if now - self.call.listed_at > LIST_EVERY || self.call.listed_at == 0.0 {
            self.call.listed_at = now;
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || { let _ = tx.send(audio::devices()); crate::ui::wake(); });
            self.call.dev_job = Some(rx);
        }
    }
    /// Keeps a capture-only engine running while a picker is shown, for the level meter.
    fn meter_level(&mut self, now: f64) -> f32 {
        if self.call.sample { return 0.35; } // offline sample: no microphone is opened
        self.call.mic_seen = now;
        // An open in flight: check its answer without waiting; a failure drops the engine and backs off.
        if let Some(rx) = &self.call.mic_opening {
            match rx.try_recv() {
                Ok(Ok(_)) => { self.call.mic_opening = None; self.call.mic_fail_at = None; }
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.call.mic_opening = None; self.call.mic_test = None; self.call.mic_fail_at = Some(now); return 0.0;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.call.mic_test.is_none() {
            if self.call.mic_fail_at.is_some_and(|t| now - t < MIC_RETRY_SECS) { return 0.0; }
            let c = self.choice();
            let e = audio::Engine::new();
            c.apply_smart(&e.io); // the meter shows the microphone after Smart Audio's noise removal
            // The device opens on the engine's own thread; the level stays 0 until it is running.
            self.call.mic_opening = Some(e.open_input_async(&c.input));
            self.call.mic_test = Some(e);
        }
        self.call.mic_test.as_ref().map_or(0.0, |e| e.io.level())
    }

    /// A picker field in the toolkit's look: label, a rounded field showing the current choice, and the black balloon menu
    /// (`webex_menu`, 40 pt rows, caret off for form fields). Returns the picked entry id.
    fn picker_field(&mut self, ui: &mut egui::Ui, t: &Th, key: &str, label: &str, shown: &str, entries: Vec<Entry>) -> Option<String> {
        let ctx = ui.ctx().clone();
        ui.label(RichText::new(label).size(13.0).color(t.text2));
        let (r, resp) = ui.allocate_exact_size(Vec2::new(280.0, 36.0), Sense::click());
        ui.painter().rect_filled(r, 8.0, if resp.hovered() { t.hover } else { t.selected });
        ui.painter().rect_stroke(r, 8.0, Stroke::new(1.0_f32, t.sep), egui::StrokeKind::Inside);
        crate::ui::chrome::fit_text(ui.painter(), r.left() + 12.0, r.center().y, shown, FontId::proportional(14.0), t.text1, r.width() - 48.0);
        self.icons.paint(ui, "arrow-down-regular", Rect::from_center_size(pos2(r.right() - 18.0, r.center().y), Vec2::splat(14.0)), t.text2);
        if resp.clicked() { menu_toggle(&ctx, key); }
        webex_menu(&ctx, &mut self.icons, t, key, r, Side::Below, Align::Left, false, &entries)
    }

    /// Device list as menu entries: "Systemstandard", every device (checked = current), and the saved one if unplugged.
    fn dev_entries(list: &[Dev], cur: &str, prefix: &str) -> Vec<Entry> {
        let mut v = vec![Item::new(&format!("{prefix}:"), "Systemstandard").checked(cur.is_empty()).entry()];
        for d in list { v.push(Item::new(&format!("{prefix}:{}", d.name), if d.default { format!("{} (Standard)", d.name) } else { d.name.clone() }).checked(cur == d.name).entry()); }
        if !cur.is_empty() && !list.iter().any(|d| d.name == cur) { v.push(Item::new("none", format!("{cur} (nicht angeschlossen)")).disabled().checked(true).entry()); }
        v
    }
    fn dev_shown(cur: &str) -> String { if cur.is_empty() { "Systemstandard".into() } else { cur.into() } }

    /// The microphone level bar (capture-only engine while it is on screen).
    pub(crate) fn level_bar(&mut self, ui: &mut egui::Ui, t: &Th, now: f64, width: f32) {
        let lvl = self.meter_level(now);
        let (r, _) = ui.allocate_exact_size(Vec2::new(width, 8.0), Sense::hover());
        ui.painter().rect_filled(r, 4.0, t.selected);
        ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width() * lvl.sqrt().min(1.0), r.height())), 4.0, t.green);
    }

    /// Settings > Audio: device pickers (balloon menus), level meter, volume, test tone, ringer device.
    pub(crate) fn device_pickers(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        self.refresh_devices(now);
        let mut c = self.choice();
        let before = c.clone();
        let (ins, outs) = (self.call.ins.clone(), self.call.outs.clone());
        if let Some(id) = self.picker_field(ui, t, "pick-mic", "Mikrofon", &Self::dev_shown(&c.input), Self::dev_entries(&ins, &c.input, "m")) { if let Some(s) = id.strip_prefix("m:") { c.input = s.into(); } }
        ui.add_space(4.0);
        self.level_bar(ui, t, now, 280.0);
        ui.add_space(8.0);
        if let Some(id) = self.picker_field(ui, t, "pick-spk", "Lautsprecher", &Self::dev_shown(&c.output), Self::dev_entries(&outs, &c.output, "s")) { if let Some(s) = id.strip_prefix("s:") { c.output = s.into(); } }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Lautstärke").size(13.0).color(t.text2));
            ui.add(egui::Slider::new(&mut c.volume, 0.0..=1.0).show_value(false));
            if ui.button("Testton").on_hover_text("Spielt einen kurzen Ton auf dem gewählten Lautsprecher").clicked() {
                let (out, vol) = (c.output.clone(), c.volume.max(0.1));
                self.play_once("Tick", &out, false, vol);
            }
        });
        ui.add_space(8.0);
        if let Some(id) = self.picker_field(ui, t, "pick-ring", "Klingeltöne und Warnungen ausgeben über", &Self::dev_shown(&c.ringtone), Self::dev_entries(&outs, &c.ringtone, "g")) { if let Some(s) = id.strip_prefix("g:") { c.ringtone = s.into(); } }
        if c != before { self.save_choice(c); }
    }

    /// Camera list for the video menu: on a worker thread, every 2 s while the menu is open (hot-plug); a sample run shows
    /// the made-up camera of Settings > Video.
    pub(super) fn refresh_cams(&mut self, now: f64) {
        let sample = self.call.sample || self.shot.is_some();
        let v = &mut self.call.video_ui;
        if sample { if v.cams.is_empty() { v.cams = vec![camera::CamDev { id: "sample".into(), name: "Testkamera".into() }]; } return; }
        if let Some(rx) = &v.cam_job {
            match rx.try_recv() {
                Ok(l) => { v.cams = l; v.cam_job = None; }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => v.cam_job = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
            }
        }
        if v.cam_job.is_none() && (v.cams_at == 0.0 || now - v.cams_at > 2.0) {
            v.cams_at = now;
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = std::thread::Builder::new().name("camera-list".into()).spawn(move || { let _ = tx.send(camera::list()); crate::ui::wake(); });
            v.cam_job = Some(rx);
        }
    }

    /// The menu behind the video button's arrow (`CallControlBarView` VideoMenu): "Kamera" with every camera (checked = in
    /// use; "Kamera, {{0}}, ausgewählt"), then the self view: hide / show it and mirror it.
    pub(crate) fn video_menu(&mut self, ctx: &egui::Context, t: &Th, now: f64, anchor: Rect) {
        if !menu_is_open(ctx, "call-video") { return; }
        self.refresh_cams(now);
        let want = self.settings["camera"].as_str().unwrap_or("").to_string();
        let cur = video::pick_camera(&self.call.video_ui.cams, &want).map(|c| c.name.clone()).unwrap_or_default();
        let mut items = vec![Entry::Title("Kamera".into())];
        if self.call.video_ui.cams.is_empty() { items.push(Item::new("none", "Keine Kamera gefunden").disabled().entry()); }
        for c in &self.call.video_ui.cams { items.push(Item::new(&format!("cam:{}", c.name), c.name.clone()).checked(c.name == cur).entry()); }
        items.push(Entry::Sep);
        let (on, hidden, mirror) = (self.call.video.wanted, self.call.video_ui.hidden, self.self_view_mirrored());
        // "Sie können Ihr Selbstansichtsvideo nur einblenden, wenn Ihr Video aktiviert ist."
        let sv = if hidden { Item::new("selfview", "Selbstansichtsvideo einblenden") } else { Item::new("selfview", "Selbstansicht ausblenden") }.icon("selfview_16");
        items.push(if on { sv.entry() } else { sv.disabled().entry() });
        items.push(Item::new("mirror", if mirror { "Eigene Videoansicht nicht spiegeln" } else { "Eigene Videoansicht spiegeln" }).icon("mirror-regular").entry());
        let Some(id) = webex_menu(ctx, &mut self.icons, t, "call-video", anchor, Side::Above, Align::Center, true, &items) else { return };
        match id.as_str() {
            "selfview" => self.call.video_ui.hidden = !hidden,
            "mirror" => {
                self.settings["camera_mirror"] = json!(!mirror);
                if !self.call.sample && self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); }
            }
            _ => if let Some(name) = id.strip_prefix("cam:") { if name != cur { self.switch_camera(name); } },
        }
    }

    /// The balloon menu behind the device button:"Mikrofon" and "Lautsprecher" lists (checked = in use) and a test tone.
    pub(crate) fn device_menu(&mut self, ctx: &egui::Context, t: &Th, now: f64, anchor: Rect) {
        // Two levels so nothing outgrows the window: the short menu, then the device list of the chosen kind.
        let any = ["call-devices", "call-mic", "call-spk"].iter().any(|k| menu_is_open(ctx, k));
        if !any { return; }
        self.refresh_devices(now);
        let c = self.choice();
        let mut c2 = c.clone();
        let short = vec![
            Entry::Title("Audiogeräte".into()),
            Item::new("mic", "Mikrofon wählen").icon("microphone-regular").entry(),
            Item::new("spk", "Lautsprecher wählen").icon("speaker-regular").entry(),
            Entry::Sep,
            Item::new("test", "Testton abspielen").icon("speaker-on-regular").entry(),
        ];
        if let Some(id) = webex_menu(ctx, &mut self.icons, t, "call-devices", anchor, Side::Below, Align::Center, true, &short) {
            match id.as_str() {
                "mic" => menu_open(ctx, "call-mic"),
                "spk" => menu_open(ctx, "call-spk"),
                _ => { let (out, vol) = (c.output.clone(), c.volume.max(0.1)); self.play_once("Tick", &out, false, vol); }
            }
        }
        let mics = Self::dev_entries(&self.call.ins, &c.input, "m");
        if let Some(id) = webex_menu(ctx, &mut self.icons, t, "call-mic", anchor, Side::Below, Align::Center, true, &mics) { if let Some(s) = id.strip_prefix("m:") { c2.input = s.into(); } }
        let spks = Self::dev_entries(&self.call.outs, &c.output, "s");
        if let Some(id) = webex_menu(ctx, &mut self.icons, t, "call-spk", anchor, Side::Below, Align::Center, true, &spks) { if let Some(s) = id.strip_prefix("s:") { c2.output = s.into(); } }
        if c2 != c { self.save_choice(c2); }
    }
}
