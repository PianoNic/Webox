//! Settings pages, part 1: Allgemein, Audio, Smart Audio, Video, Freigabe, Erscheinungsbild, Barrierefreiheit, Tastenkombinationen,
//! Privatfunktion, Messaging. Labels are the German pack's where it has them. Settings with no function in Webox are stored
//! (settings JSON) but inert.
use super::kit::{self, section};
use super::nav::Page;
use crate::*;

impl App {
    pub(super) fn sb(&self, k: &str, d: bool) -> bool { self.settings[k].as_bool().unwrap_or(d) }
    pub(super) fn ss(&self, k: &str, d: &str) -> String { self.settings[k].as_str().unwrap_or(d).to_string() }
    /// Stores a setting and writes the settings JSON (not in --screenshot runs, which must never touch the user's file).
    pub(super) fn sset(&mut self, k: &str, v: serde_json::Value) {
        self.settings[k] = v;
        if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); }
    }
    /// A persisted checkbox. True when it changed.
    pub(super) fn pcheck(&mut self, ui: &mut egui::Ui, t: &Th, key: &str, def: bool, label: &str) -> bool {
        let mut v = self.sb(key, def);
        let ch = kit::check(ui, &mut self.icons, t, label, &mut v, true, false);
        if ch { self.sset(key, json!(v)); }
        ui.add_space(4.0);
        ch
    }
    /// A persisted combo box over (value, label) options.
    pub(super) fn pcombo(&mut self, ui: &mut egui::Ui, t: &Th, key: &str, def: &str, opts: &[(&str, &str)], width: f32) -> Option<String> {
        let cur = self.ss(key, def);
        let labels: Vec<String> = opts.iter().map(|o| o.1.to_string()).collect();
        let i = opts.iter().position(|o| o.0 == cur).unwrap_or(0);
        let pick = kit::combo(ui, &mut self.icons, t, key, width, &labels, i);
        pick.map(|p| { self.sset(key, json!(opts[p].0)); opts[p].0.to_string() })
    }
    /// A persisted radio group.
    pub(super) fn pradio(&mut self, ui: &mut egui::Ui, t: &Th, key: &str, def: &str, opts: &[(&str, &str)]) -> Option<String> {
        let cur = self.ss(key, def);
        let mut out = None;
        for (v, l) in opts { if kit::radio(ui, t, l, cur == *v, "") { out = Some(v.to_string()); } }
        if let Some(v) = &out { self.sset(key, json!(v)); }
        out
    }

    // Allgemein.
    pub(super) fn page_general(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        // Startup section: "Anmelden" / "Webox zusammen mit meinem Computer starten" (autostart, system.rs).
        section(ui, t, "Anmelden", "", true, |ui| {
            let mut v = self.sb("autostart", false);
            if kit::check(ui, &mut self.icons, t, concat!(crate::app_name!(), " zusammen mit meinem Computer starten"), &mut v, true, false) {
                match system::set_autostart(v) { Ok(()) => self.sset("autostart", json!(v)), Err(e) => self.toast(e, now) }
            }
        });
        section(ui, t, "Verfügbarkeit und Status", "Sie können anderen Personen mitteilen, dass Sie in Webex aktiv sind, und Ihren benutzerdefinierten Status sichtbar machen, wenn Sie einen Status festgelegt haben. Sie können auch die Verfügbarkeit und den Status dieser Personen sehen.", false, |ui| {
            self.pcheck(ui, t, "show_presence", true, "Verfügbarkeit und Status anzeigen");
            self.pcheck(ui, t, "show_calendar_meeting", true, "Anzeigen, wenn ich in einem Kalender-Meeting bin");
        });
        section(ui, t, "Outlook-Einstellungen", "", false, |ui| { self.pcheck(ui, t, "outlook_presence", false, concat!("Verbindung von ", crate::app_name!(), " mit Microsoft Outlook herstellen")); });
        // Landing screen: values kept from the previous page ("home").
        section(ui, t, "Zielbildschirm", concat!("Wählen Sie die Option aus, die Sie jedes Mal als erstes anzeigen möchten, wenn ", crate::app_name!(), " geöffnet wird."), false, |ui| {
            self.pcombo(ui, t, "home", "Messaging", &[("Messaging", "Nachrichten"), ("Meetings", "Meetings"), ("Calling", "Anrufe"), ("Contacts", "Kontakte")], 280.0);
        });
        section(ui, t, "Letzte Sitzungen", "Zeigen Sie Geräte an, die aktuell angemeldet sind oder kürzlich aktiv waren.", false, |ui| {
            if kit::link(ui, t, "Details anzeigen").clicked() { crate::ui::menu::menu_open(ui.ctx(), crate::ui::app_dialogs::SESSIONS); }
        });
        // Default file location: real folder dialog; stored in `download_dir` (the OS download folder unless set).
        section(ui, t, "Standardmäßiger Dateispeicherort", "", false, |ui| {
            let dir = self.ss("download_dir", "");
            let shown = if dir.is_empty() { dirs_download() } else { dir };
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 100.0).clamp(80.0, 500.0);
                let g = ui.painter().layout_no_wrap(shown.clone(), FontId::proportional(kit::SECOND), t.text2);
                let (r, _) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::hover());
                crate::ui::chrome::fit_text(ui.painter(), r.left(), r.center().y, &shown, FontId::proportional(kit::SECOND), t.text2, w);
                let _ = g;
                if kit::pill(ui, t, "Ändern", true, true).clicked() { super::sound_ui::pick_async("download_dir", true); }
            });
        });
        section(ui, t, "Lokale Zeitzone", "Wir verwenden diese Zeitzone, um anderen Ihre lokale Zeit in Ihrem Profil anzuzeigen. Dies wird auch für Benachrichtigungseinstellungen wie Ruhezeiten verwendet.", false, |ui| {
            // Only the computer's time zone is offered.
            self.pcombo(ui, t, "time_zone", "system", &[("system", "Zeitzone des Computers verwenden")], 472.0);
        });
        section(ui, t, "Multitasking", "", false, |ui| { self.pcheck(ui, t, "multitasking", false, "Mein(en) Webex-Anruf oder -Meeting automatisch in einem unverankerten Minifenster anzeigen, wenn es von anderen Apps verdeckt wird."); });
        section(ui, t, "CO₂-bewusster Modus", "Nutzen Sie den Betrieb mit geringem CO₂-Ausstoß, um Kohlendioxidemissionen zu minimieren, wenn der Strom aus einer Energiequelle mit hohem CO₂-Ausstoß stammt.", false, |ui| {
            let mut v = self.sb("low_carbon", false);
            if kit::check(ui, &mut self.icons, t, "CO₂-bewusster Modus", &mut v, true, false) { self.sset("low_carbon", json!(v)); }
            let mut c = self.sb("low_carbon_video", false);
            if kit::check(ui, &mut self.icons, t, "Videoauflösung bei Eins-zu-eins-Anrufen und -Meetings reduzieren, wenn kein nachhaltiger Strom zur Verfügung steht", &mut c, v, true) { self.sset("low_carbon_video", json!(c)); }
        });
        // Webox-specific: language, close-to-tray, browser helper, account.
        section(ui, t, "Sprache", "Sprache der Oberfläche.", false, |ui| { // "Sprache" = pack word
            self.pcombo(ui, t, "language", "system", &[("system", "Systemstandard"), ("de", "Deutsch"), ("en", "English")], 280.0);
        });
        section(ui, t, "Fenster", "", false, |ui| { // key shared with notify.rs
            self.pcheck(ui, t, "close_to_tray", true, "Beim Schließen in den Infobereich minimieren");
        });
        section(ui, t, "Meetings aus dem Browser", "Ersetzt den Webex-Browser-Helper.", false, |ui| { // Webox feature
            let label = if cfg!(windows) { "Meeting-Links aus Chrome, Edge und Firefox hier öffnen" } else { "Meeting-Links aus Chrome, Chromium, Edge und Firefox hier öffnen" };
            let mut v = self.sb("browser_helper", false);
            if kit::check(ui, &mut self.icons, t, label, &mut v, true, false) {
                match platform::nmhost::register(&self.cfg, v) { Ok(()) => self.sset("browser_helper", json!(v)), Err(e) => self.toast(e, now) }
            }
        });
        section(ui, t, "Konto", "", false, |ui| {
            if self.client.is_some() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  ·  {}", self.me.name, self.me.email)).size(kit::BODY).color(t.text1));
                    if kit::pill(ui, t, "Abmelden", true, true).clicked() { ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("settings-signout"), true)); }
                });
            } else {
                ui.label(RichText::new("Sie sehen Beispieldaten.").size(kit::BODY).color(t.text2));
                if kit::pill(ui, t, "Anmelden", true, true).clicked() { self.need_login = true; self.login_error.clear(); }
            }
            ui.add_space(8.0);
            ui.label(RichText::new(format!("Version {}  ·  {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS)).size(kit::SECOND).color(t.text2));
        });
    }

    // Audio.
    pub(super) fn page_audio(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, "", "", true, |ui| { self.settings_audio_devices(ui, t, now); });
        section(ui, t, "Klingeltöne und Warnungen", "", false, |ui| { self.sound_section_ringer(ui, t, now); });
        section(ui, t, "Erweiterte Audioeinstellungen", "", false, |ui| {
            // "Bypass Windows signal processing" (Windows only). Stored; Webox's engine has its own chain.
            if cfg!(windows) { self.pcheck(ui, t, "bypass_windows_dsp", false, "Windows-Signalverarbeitung umgehen"); }
        });
    }
    /// Smart Audio. Mikrofonaudio is real: the call microphone (and the Audio page's level meter) runs through the noise
    /// removal for "Automatisch" / "Nur Stimme" (media/denoise.rs). Externe Audiokommunikation only concerns PSTN calls
    /// (receive-side noise removal); Webox has no PSTN calling, so it is stored only.
    pub(super) fn page_smart_audio(&mut self, ui: &mut egui::Ui, t: &Th) {
        text_note(ui, t, concat!(crate::app_name!(), " verwendet künstliche Intelligenz, um die Audioqualität zu verbessern."));
        ui.add_space(4.0);
        // Option labels of the Smart Audio page (not in the pack).
        let opts = [("auto", "Automatisch"), ("off", "Aus"), ("voice", "Nur Stimme"), ("all", "Alle Geräusche")];
        section(ui, t, "Mikrofonaudio", "Wählen Sie, wie Ihr Mikrofonaudio bei Anrufen und Meetings zu hören sein soll.", false, |ui| {
            let eco = self.sb("smart_mic_eco", false);
            if let Some(v) = self.pradio(ui, t, "smart_mic", "auto", &opts) { self.apply_smart_mic(crate::media::audio::SmartMic::from_key(&v), eco); }
            // Webox addition (own labels): the lighter engine, and which one runs (meter / call microphone).
            ui.add_space(4.0);
            let mut e = eco;
            if kit::check_more(ui, &mut self.icons, t, "Energiesparende Geräuschentfernung", &mut e,
                "Verwendet RNNoise statt DeepFilterNet: braucht deutlich weniger Rechenleistung, entfernt Geräusche aber weniger gründlich.") {
                self.sset("smart_mic_eco", json!(e));
                let m = crate::media::audio::SmartMic::from_key(&self.ss("smart_mic", "auto"));
                self.apply_smart_mic(m, e);
            }
            use crate::media::denoise::{engine, Engine};
            let s = match engine() {
                Engine::Idle => "", Engine::Loading => "DeepFilterNet wird geladen …",
                Engine::DeepFilter => "Aktiv: DeepFilterNet 3", Engine::RnnEco => "Aktiv: RNNoise (energiesparend)",
                Engine::RnnFallback => "Aktiv: RNNoise (DeepFilterNet ist auf diesem Gerät zu langsam)",
                Engine::LoadFailed => "Aktiv: RNNoise (DeepFilterNet konnte nicht geladen werden)",
            };
            if !s.is_empty() { ui.add_space(2.0); kit::check_desc(ui, t, s); }
        });
        section(ui, t, "Externe Audiokommunikation", "Wählen Sie, wie Sie die Stimme des externen Teilnehmers in Eins-zu-eins-Anrufen über ein öffentliches Telefonnetzwerk hören möchten.", false, |ui| {
            self.pradio(ui, t, "smart_ext", "auto", &[("auto", "Automatisch"), ("off", "Aus")]);
            text_note(ui, t, "Kann sich auf Warteschleifenmusik und Töne auswirken");
        });
    }

    // Video: video_page.rs.

    // Freigabe von Inhalten: stored, inert.
    pub(super) fn page_share(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, "Freigabeoptionen", "", true, |ui| {
            self.pradio(ui, t, "share_apps", "all", &[("all", "Bildschirm freigeben"), ("apps", "Anwendungen freigeben")]);
        });
        section(ui, t, "Auflösung und Bildrate", "", false, |ui| {
            self.pcombo(ui, t, "share_res", "auto", &[("auto", "Automatisch"), ("1080", "1080p"), ("4k", "4K")], 280.0);
        });
        section(ui, t, "Mehrere Bildschirme", "", false, |ui| { self.pcheck(ui, t, "share_multi", false, "Mehrere Bildschirme"); });
    }

    // Erscheinungsbild.
    pub(super) fn page_appearance(&mut self, ui: &mut egui::Ui, t: &Th) {
        // Themen: the ten themes in picker order in a scrollable two-column grid, each entry a preview tile (drawn from its colour
        // stops: 140 x 92, radius 8, background gradient, an inner card from x 32 / y 8 with a 4 pt top-left radius, 20 % outline)
        // with a radio button and the name under it. The choice is stored under `theme` (the theme id, e.g. "MomentumDark") and
        // applied at once. While Windows high contrast is on the "OS High Contrast" theme is used and the tiles are disabled.
        let hc = crate::ui::theme::hc_cached(ui.ctx());
        section(ui, t, crate::i18n::lookup_any("Themes").unwrap_or("Themen"), concat!("Legen Sie fest, wie ", crate::app_name!(), " aussehen soll."), true, |ui| {
            let cur = ThemeId::from_key(&self.ss("theme", "MomentumDark")).unwrap_or_default();
            let pick = theme_grid(ui, t, cur, !hc);
            if hc { ui.add_space(4.0); kit::text_block(ui, "Wenn Sie ein Thema auswählen möchten, deaktivieren Sie zuerst den hohen Kontrast in den Windows-Einstellungen.", kit::SECOND, t.text2, 0.0); }
            if let Some(id) = pick { self.sset("theme", json!(id.key())); self.applied = None; }
        });
        section(ui, t, "Meetings", "", false, |ui| {
            self.pcheck(ui, t, "always_dark_calls", false, "Immer dunkles Thema verwenden");
        });
        section(ui, t, "Hautton für Reaktionen", "", false, |ui| {
            let cur = self.settings["emoji_tone"].as_u64().unwrap_or(0) as usize;
            let mut pick = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (i, c) in [0xFFC83D, 0xF7D9B5, 0xE2B98E, 0xC18F62, 0x9A6B43, 0x5E4130].into_iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
                    if resp.hovered() { ui.painter().rect_filled(r, 20.0, kit::tok(t).item_hover); }
                    ui.painter().circle_filled(r.center(), 11.0, hex(c));
                    if cur == i { ui.painter().circle_stroke(r.center(), 17.0, Stroke::new(2.0, t.text1)); }
                    if resp.clicked() { pick = Some(i); }
                }
            });
            if let Some(i) = pick { self.sset("emoji_tone", json!(i)); }
        });
        section(ui, t, "", "", false, |ui| { self.pcheck(ui, t, "hand_gestures", false, "Handgesten erkennen"); });
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Passen Sie weitere Einstellungen unter „Barrierefreiheit“ an.").size(kit::SECOND).color(t.text2));
            if kit::link(ui, t, "Ändern").clicked() { ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("settings-goto"), Page::Accessibility)); }
        });
    }

    // Barrierefreiheit.
    pub(super) fn page_accessibility(&mut self, ui: &mut egui::Ui, t: &Th) {
        let names = ["Klein", "Regulär (Standard)", "Mittel", "Groß"];
        let keys = ["small", "regular", "medium", "large"];
        let cur = keys.iter().position(|k| *k == self.ss("msg_font_size", "regular")).unwrap_or(1);
        section(ui, t, "Schriftgröße", "", true, |ui| {
            // Preview box: radius 8, min height 72, centred text in the message font (scaled), on the hover colour.
            let scale = msg_font_scale_of(keys[cur]);
            let g = ui.painter().layout("Ziehen Sie den Schieberegler, um zu sehen, wie Ihre Nachrichten aussehen werden.".into(), FontId::proportional(16.0 * scale), t.text1, ui.available_width() - 30.0);
            let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), (g.size().y + 30.0).max(72.0)), Sense::hover());
            ui.painter().rect_filled(r, 8.0, kit::tok(t).item_hover);
            let gp = r.center() - g.size() / 2.0;
            ui.painter().galley(gp, g, t.text1);
            ui.add_space(8.0);
            let mut v = cur as f32;
            if kit::slider(ui, t, &mut v, 0.0, 3.0, 4, ui.available_width()) { self.sset("msg_font_size", json!(keys[v.round() as usize])); }
            ui.horizontal(|ui| { // tick labels under the 4 positions
                let w = ui.available_width();
                let (r, _) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::hover());
                for (i, n) in names.iter().enumerate() {
                    let x = r.left() + 8.0 + i as f32 / 3.0 * (w - 16.0);
                    let al = [egui::Align2::LEFT_CENTER, egui::Align2::CENTER_CENTER, egui::Align2::CENTER_CENTER, egui::Align2::RIGHT_CENTER][i];
                    ui.painter().text(pos2(x, r.center().y), al, *n, FontId::proportional(kit::SECOND), if i == cur { t.text1 } else { t.text2 });
                }
            });
        });
        // Footer: "Auf Standard zurücksetzen".
        ui.add_space(16.0);
        if kit::pill(ui, t, "Auf Standard zurücksetzen", false, true).clicked() { self.sset("msg_font_size", json!("regular")); self.toast("Die Zugriffseinstellungen werden auf den Standardwert zurückgesetzt.", 0.0 + ui.input(|i| i.time)); }
    }

    // Privatfunktion.
    pub(super) fn page_privacy(&mut self, ui: &mut egui::Ui, t: &Th, _now: f64) {
        section(ui, t, "Verfügbarkeit und Status", "Sie können anderen Personen mitteilen, dass Sie in Webex aktiv sind, und Ihren benutzerdefinierten Status sichtbar machen, wenn Sie einen Status festgelegt haben. Sie können auch die Verfügbarkeit und den Status dieser Personen sehen.", true, |ui| {
            self.pcheck(ui, t, "show_presence", true, "Verfügbarkeit und Status anzeigen");
            let on = self.sb("show_presence", true);
            let mut c = self.sb("show_calendar_meeting", true);
            if kit::check(ui, &mut self.icons, t, "Anzeigen, wenn ich in einem Kalender-Meeting bin", &mut c, on, true) { self.sset("show_calendar_meeting", json!(c)); }
            kit::check_desc(ui, t, "Lassen Sie es andere Benutzer wissen, wenn Sie sich in einem über Ihren E-Mail-Kalender organisierten Meeting befinden und in Webex aktiv sind.");
        });
        section(ui, t, "Meine Verfügbarkeit und meinen Status ausblenden", "Fügen Sie Personen hinzu, vor denen Sie Ihre Verfügbarkeit und Ihren Status ausblenden möchten.", false, |ui| {
            let n = self.settings["hidden_from"].as_array().map_or(0, |a| a.len());
            ui.horizontal(|ui| {
                if kit::pill(ui, t, "Hinzufügen", true, true).clicked() { crate::ui::menu::menu_open(ui.ctx(), crate::ui::app_dialogs::HIDE_FROM); }
                ui.label(RichText::new(format!("Ihre Verfügbarkeit und Ihr Status werden vor {n} Personen verborgen.")).size(kit::SECOND).color(t.text2));
            });
        });
    }

    // Messaging.
    pub(super) fn page_messaging(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, "Rechtschreibprüfung", "", true, |ui| {
            let on = self.pcheck_ret(ui, t, "spell_check", true, "Rechtschreibprüfung aktivieren");
            ui.add_space(2.0);
            ui.horizontal(|ui| { ui.add_space(32.0); ui.add_enabled_ui(on, |ui| {
                self.pcombo(ui, t, "spell_lang", "de", &[("de", "Deutsch"), ("en", "English (US)"), ("fr", "Français")], 260.0); }); }); // dictionary list
        });
        section(ui, t, "Threads", "Wählen Sie aus, wie die Threads in Ihren Bereichen und Direktnachrichten angezeigt werden sollen.", false, |ui| {
            self.pradio(ui, t, "threads", "expand", &[("expand", "Threads erweitern"), ("collapse", "Threads reduzieren")]);
        });
        section(ui, t, "Lesebestätigungen", "Durch eine Lesebestätigung werden Personen darüber informiert, von wem eine Nachricht angesehen wurde. Wenn diese Option deaktiviert ist, dann senden Sie nur eine Lesebestätigung, wenn Sie eine Nachricht senden.", false, |ui| {
            self.pcheck(ui, t, "read_receipts", true, "Lesebestätigungen verwenden");
        });
        section(ui, t, "Namen automatisch eintragen", "", false, |ui| { self.pcheck(ui, t, "name_autocomplete", true, "Namen automatisch eintragen"); });
        section(ui, t, "Emojis", "", false, |ui| { // real effect: composer autocomplete
            self.pcheck(ui, t, "emoji_autocomplete", true, "Emojis automatisch vorschlagen");
            // Webox addition: a message of only 1-3 emoji goes out as a heading, like typing '# 😆'.
            self.pcheck(ui, t, "big_emoji_send", true, "Einzelne Emojis groß senden");
        });
    }
    fn pcheck_ret(&mut self, ui: &mut egui::Ui, t: &Th, key: &str, def: bool, label: &str) -> bool { self.pcheck(ui, t, key, def, label); self.sb(key, def) }

}

pub(super) fn text_note(ui: &mut egui::Ui, t: &Th, s: &str) { kit::text_block(ui, s, kit::SECOND, t.text2, 0.0); ui.add_space(4.0); }

/// Theme picker tiles (see `page_appearance`): preview 140 x 92, radio + name below, two columns 16 apart, rows 12 apart; the grid
/// scrolls inside the page when it is taller than the space the page leaves it (at least two rows stay visible). Returns the click.
fn theme_grid(ui: &mut egui::Ui, t: &Th, cur: ThemeId, enabled: bool) -> Option<ThemeId> {
    const PW: f32 = 140.0;
    const PH: f32 = 92.0;
    const LABEL_H: f32 = 30.0;
    const GAP_X: f32 = 16.0;
    const GAP_Y: f32 = 12.0;
    let row_h = PH + LABEL_H + GAP_Y;
    let rows = ThemeId::ALL.len().div_ceil(2);
    let full_h = rows as f32 * row_h;
    // The visible part of the page body minus the sections below the grid (Meetings, Hautton, Handgesten, Barrierefreiheit line).
    let room = (ui.clip_rect().bottom() - ui.cursor().top() - 300.0).max(2.0 * row_h);
    let mut pick = None;
    crate::ui::widgets::vscroll().id_salt("theme-grid").max_height(room.min(full_h)).auto_shrink([false, true]).show(ui, |ui| {
        let (area, _) = ui.allocate_exact_size(Vec2::new(2.0 * PW + GAP_X + 8.0, full_h), Sense::hover());
        for (i, id) in ThemeId::ALL.into_iter().enumerate() {
            let x = area.left() + (i % 2) as f32 * (PW + GAP_X);
            let y = area.top() + (i / 2) as f32 * row_h;
            let tile = Rect::from_min_size(pos2(x, y), Vec2::new(PW, PH + LABEL_H));
            let resp = ui.interact(tile, egui::Id::new(("theme-tile", id.idx())), if enabled { Sense::click() } else { Sense::hover() });
            let p = ui.painter().clone();
            let a = if enabled { 1.0 } else { 0.45 }; // opacity 0.45 while disabled
            paint_preview(&p, Rect::from_min_size(pos2(x, y), Vec2::new(PW, PH)), id, a);
            // Selection: button-primary-border 2 pt around the preview; hover: button-secondary-border.
            let pv = Rect::from_min_size(pos2(x, y), Vec2::new(PW, PH)).expand(3.0);
            if cur == id { p.rect_stroke(pv, 10.0, Stroke::new(2.0, t.tk("button-primary-border")), egui::StrokeKind::Inside); }
            else if resp.hovered() && enabled { p.rect_stroke(pv, 10.0, Stroke::new(2.0, t.tk("button-secondary-border:hovered")), egui::StrokeKind::Inside); }
            // Radio (radio-* tokens) + label (label-primary-text, tertiary body font).
            let c = pos2(x + 10.0, y + PH + 16.0);
            let k = kit::ctl(t);
            if cur == id {
                p.circle_filled(c, 8.0, k.checked_bg.pick(&resp).gamma_multiply(a));
                p.circle_filled(c, 3.0, k.checked_icon.gamma_multiply(a));
            } else {
                p.circle_filled(c, 8.0, k.unchecked_bg.pick(&resp).gamma_multiply(a));
                p.circle_stroke(c, 7.5, Stroke::new(1.0, k.unchecked_border.gamma_multiply(a)));
            }
            crate::ui::chrome::fit_text(&p, x + 26.0, c.y, &id.label(), FontId::proportional(kit::BODY), t.tk("label-primary-text").gamma_multiply(a), PW - 26.0);
            if enabled && resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            if enabled && resp.clicked() && cur != id { pick = Some(id); }
        }
    });
    pick
}

/// The theme preview (140 x 92): rounded background with a vertical gradient, the inner card (x 32..140, y 8..88, top-left
/// radius 4, gradient), a 1 px outline at 20 % - white in dark themes, black in light ones.
pub(crate) fn paint_preview(p: &egui::Painter, r: Rect, id: ThemeId, a: f32) {
    let [b0, b1, c0, c1, line] = id.preview().map(|c| c.gamma_multiply(a));
    let s = r.width() / 140.0;
    vgrad(p, r, 8.0 * s, b0, b1);
    let card = Rect::from_min_max(pos2(r.left() + 32.0 * s, r.top() + 8.0 * s), pos2(r.right(), r.top() + 88.0 * s));
    // Only the top-left corner is rounded: round all four on a rect running past the right and bottom edges, clipped to the card.

    vgrad(&p.with_clip_rect(card), Rect::from_min_max(card.min, card.max + Vec2::splat(8.0 * s)), 4.0 * s, c0, c1);
    p.rect_stroke(r, 8.0 * s, Stroke::new(1.0, alpha(line, 0.2 * a)), egui::StrokeKind::Inside);
}

/// Message font scale of the Accessibility slider: Klein / Regulär / Mittel / Groß.
pub(crate) fn msg_font_scale_of(k: &str) -> f32 { match k { "small" => 0.875, "medium" => 1.125, "large" => 1.25, _ => 1.0 } }

/// The OS download folder (default of "Standardmäßiger Dateispeicherort").
pub(super) fn dirs_download() -> String {
    #[cfg(windows)]
    if let Ok(p) = std::env::var("USERPROFILE") { return format!("{p}\\Downloads"); }
    if let Ok(p) = std::env::var("HOME") { return format!("{p}/Downloads"); }
    "Downloads".into()
}
