//! Sound pickers of the Settings pages (ringtone, message sounds, meeting sounds) with the "custom sound" option:
//! a native open-file dialog (rfd: Windows IFileOpenDialog / Linux portal; opened on its own thread so the UI keeps running),
//! wav / mp3 / ogg / flac decoded by `sounds::decode_file` (size <= 10 MB, ringtones <= 60 s, German error toast), the absolute path
//! stored in the settings (`ringtone_custom_path`, `msg_sound_custom_path`, per category `msg_dm_custom_path`, ...), shown in the
//! picker by its file name with Test / Entfernen. A file that disappears falls back to the default (`sounds::valid_or`).
//! "Benutzerdefinierter Ton…" builds on the pack word "Benutzerdefiniert".
use super::kit;
use crate::media::sounds::{self, Prefs};
use crate::*;
use std::sync::Mutex;

static PICKS: Mutex<Vec<(String, Option<PathBuf>)>> = Mutex::new(Vec::new());

/// Opens the native dialog on a worker thread; the result is collected by `App::poll_picks`.
pub(super) fn pick_async(key: &str, folder: bool) {
    let key = key.to_string();
    std::thread::spawn(move || {
        let r = if folder { rfd::FileDialog::new().pick_folder() } else { rfd::FileDialog::new().add_filter("Audiodateien", &["wav", "mp3", "ogg", "flac"]).pick_file() };
        if let Ok(mut v) = PICKS.lock() { v.push((key, r)); }
        crate::ui::wake();
    });
}

enum Opt { No, Built(&'static str), Custom(String), Pick }

/// Settings field -> (setting key of the id, default id, built-in choices, ringtone?).
fn field_info(f: &str) -> (&'static [&'static str], &'static str, bool) {
    if f == "ringtone" { (&sounds::CALL_RINGTONES, "Delight", true) } else { (&sounds::MESSAGE_SOUNDS, "Open", false) }
}
fn custom_key(f: &str) -> String { format!("{f}_custom_path") }

impl App {
    fn sound_id(&mut self, f: &str) -> String {
        let p = Prefs::load(&self.settings);
        match f { "ringtone" => p.ringtone, "msg_dm" => p.msg_dm, "msg_space" => p.msg_space, "msg_favorite" => p.msg_favorite,
            _ => sounds::valid_or(&self.ss(&format!("snd_{f}"), field_default(f)), field_default(f)) }
    }
    fn set_sound_id(&mut self, f: &str, id: String) {
        let mut p = Prefs::load(&self.settings);
        match f {
            "ringtone" => p.ringtone = id, "msg_dm" => p.msg_dm = id, "msg_space" => p.msg_space = id, "msg_favorite" => p.msg_favorite = id,
            _ => { self.sset(&format!("snd_{f}"), json!(id)); return; }
        }
        self.save_prefs(p);
    }
    fn remember_custom(&mut self, f: &str, path: &std::path::Path) {
        let s = path.display().to_string();
        self.settings[custom_key(f)] = json!(s);
        self.settings[if f == "ringtone" { "ringtone_custom_path" } else { "msg_sound_custom_path" }] = json!(s);
    }

    /// Collects finished file dialogs: validates (German toast on error) and applies.
    pub(super) fn poll_picks(&mut self, now: f64) {
        let got: Vec<_> = PICKS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
        for (key, path) in got {
            let Some(path) = path else { continue };
            if key == "download_dir" { self.sset("download_dir", json!(path.display().to_string())); continue; }
            let Some(f) = key.strip_prefix("snd:") else { continue };
            let (_, _, ring) = field_info(f);
            match sounds::decode_file(&path, ring.then_some(sounds::MAX_RINGTONE_SECS)) {
                Ok(_) => {
                    self.remember_custom(f, &path);
                    self.set_sound_id(f, sounds::file_id(&path));
                    self.sset("snd_custom_touch", json!(now as u64));
                }
                Err(e) => self.toast(e, now),
            }
        }
    }

    /// Label + combo (built-ins, the chosen custom file, "Benutzerdefinierter Ton…") + Test / Entfernen.
    pub(super) fn sound_picker(&mut self, ui: &mut egui::Ui, t: &Th, now: f64, f: &str, label: &str) {
        let (builtins, default, ring) = field_info(f);
        let cur = self.sound_id(f);
        let stored = self.ss(&custom_key(f), "");
        let mut opts: Vec<(Opt, String)> = vec![];
        if !ring { opts.push((Opt::No, "Kein Ton".into())); }
        opts.extend(builtins.iter().map(|b| (Opt::Built(b), b.to_string())));
        let file_name = |p: &str| std::path::Path::new(p).file_name().map_or(p.to_string(), |n| n.to_string_lossy().to_string());
        let cur_file = sounds::file_path(&cur).map(String::from);
        let shown_custom = cur_file.clone().or_else(|| (!stored.is_empty() && std::path::Path::new(&stored).is_file()).then(|| stored.clone()));
        if let Some(p) = &shown_custom { opts.push((Opt::Custom(p.clone()), format!("{}  (Benutzerdefiniert)", file_name(p)))); }
        opts.push((Opt::Pick, "Benutzerdefinierter Ton…".into()));
        let idx = opts.iter().position(|(o, _)| match o { Opt::No => cur.is_empty(), Opt::Built(b) => cur == *b, Opt::Custom(p) => cur_file.as_deref() == Some(p.as_str()), Opt::Pick => false }).unwrap_or(0);
        ui.label(RichText::new(label).size(kit::BODY).color(t.text1));
        ui.add_space(4.0);
        let labels: Vec<String> = opts.iter().map(|o| o.1.clone()).collect();
        let mut test = false; let mut remove = false;
        ui.horizontal(|ui| {
            if let Some(i) = kit::combo(ui, &mut self.icons, t, &format!("snd-{f}"), 280.0, &labels, idx) {
                match &opts[i].0 {
                    Opt::No => self.set_sound_id(f, String::new()),
                    Opt::Built(b) => self.set_sound_id(f, b.to_string()),
                    Opt::Custom(p) => self.set_sound_id(f, sounds::file_id(std::path::Path::new(p))),
                    Opt::Pick => pick_async(&format!("snd:{f}"), false),
                }
            }
            test = kit::pill(ui, t, "Test", true, !cur.is_empty()).on_hover_text("Ton anhören").clicked();
            remove = cur_file.is_some() && kit::pill(ui, t, "Entfernen", true, true).on_hover_text("Eigenen Ton entfernen, Standard verwenden").clicked();
        });
        if remove {
            self.settings[custom_key(f)] = json!("");
            self.settings[if ring { "ringtone_custom_path" } else { "msg_sound_custom_path" }] = json!("");
            self.set_sound_id(f, default.to_string());
        }
        if !stored.is_empty() && !std::path::Path::new(&stored).is_file() && cur_file.is_none() {
            kit::text_block(ui, "Die gewählte Datei wurde nicht gefunden. Der Standardton wird verwendet.", kit::SECOND, t.text2, 0.0);
        }
        if test { self.preview_sound(&cur, now); }
        ui.add_space(10.0);
    }

    fn preview_sound(&mut self, id: &str, now: f64) {
        if id.is_empty() { return; }
        if sounds::pcm(id).is_none() { self.toast("Der Ton kann nicht abgespielt werden.", now); return; }
        let c = self.choice();
        let vol = Prefs::load(&self.settings).msg_volume.max(0.2);
        let dev = if c.ringtone.is_empty() { c.output } else { c.ringtone };
        self.play_once(id, &dev, false, vol);
    }

    /// "Klingeltöne und Warnungen": call sounds, ringtone (custom allowed), volume, all output devices.
    pub(super) fn sound_section_ringer(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let mut p = Prefs::load(&self.settings);
        let before = p.clone();
        kit::check(ui, &mut self.icons, t, "Töne bei Anrufen (Klingelton, Freizeichen, Anrufende)", &mut p.call_sounds, true, false);
        ui.add_space(6.0);
        if p != before { self.save_prefs(p.clone()); }
        self.sound_picker(ui, t, now, "ringtone", "Klingelton für direkte Anrufe");
        let mut p = Prefs::load(&self.settings);
        let before = p.clone();
        ui.label(RichText::new("Lautstärke").size(kit::BODY).color(t.text1));
        kit::slider(ui, t, &mut p.ringer_volume, 0.0, 1.0, 0, 280.0);
        ui.add_space(6.0);
        kit::check(ui, &mut self.icons, t, "Klingelton auf allen Ausgabegeräten abspielen", &mut p.ring_all_devices, true, false);
        if p != before { self.save_prefs(p); }
    }

    /// Message sounds block of the message notification page: on/off, three categories, volume.
    pub(super) fn sound_section_messages(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let mut p = Prefs::load(&self.settings);
        let before = p.clone();
        kit::check(ui, &mut self.icons, t, "Töne für Nachrichten", &mut p.msg_sounds, true, false);
        ui.add_space(6.0);
        if p != before { self.save_prefs(p); }
        self.sound_picker(ui, t, now, "msg_dm", "Töne für Benachrichtigungen über Direktnachrichten");
        self.sound_picker(ui, t, now, "msg_space", "Töne für Bereichsbenachrichtigungen");
        self.sound_picker(ui, t, now, "msg_favorite", "Töne für Benachrichtigungen über Favoriten-Bereiche");
        let mut p = Prefs::load(&self.settings);
        let before = p.clone();
        ui.label(RichText::new("Lautstärke").size(kit::BODY).color(t.text1));
        kit::slider(ui, t, &mut p.msg_volume, 0.0, 1.0, 0, 280.0);
        if p != before { self.save_prefs(p); }
        ui.add_space(4.0);
        kit::text_block(ui, "Kein Ton im Status „Nicht stören“. Gespielt wird nur für Chats, die gerade nicht geöffnet sind.", kit::SECOND, t.text2, 0.0);
    }

    /// Plays one of the event sounds (dial-pad key tone, reconnect, group-call invite, disclaimers, voicemail beep) on the
    /// chosen output when call sounds are on. For other modules: `self.play_event_sound("DTMF_Tone")`.
    #[allow(dead_code)]
    pub(crate) fn play_event_sound(&mut self, id: &str) {
        let p = Prefs::load(&self.settings);
        if !p.call_sounds || sounds::get(id).is_none() { return; }
        let out = self.choice().output;
        self.play_once(id, &out, false, p.ringer_volume);
    }
}

fn field_default(f: &str) -> &'static str { match f { "call_pickup" => "Delight", _ => "Open" } }
