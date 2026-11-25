//! Settings pages, part 1: General, Audio, Smart Audio, Video, Sharing, Appearance, Accessibility, Keyboard shortcuts,
//! Privacy, Messaging. Labels are the pack's English sources where it has them. Settings with no function in Webox are stored
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
        ui.add_space(crate::ui::theme::density::m().settings_row_gap); // Standard 4
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

    // General.
    pub(super) fn page_general(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        // Startup section: "Sign In" / "Start Webox when my computer starts" (autostart, system.rs).
        section(ui, t, tr("Sign In"), "", true, |ui| {
            // Windows: the Run key itself, which the installer may have set.
            let mut v = system::autostart_registered().unwrap_or_else(|| self.sb("autostart", false));
            if kit::check(ui, &mut self.icons, t, &crate::trq!("Start {{APP_NAME}} when my computer starts"), &mut v, true, false) {
                match system::set_autostart(v) { Ok(()) => self.sset("autostart", json!(v)), Err(e) => self.toast(e, now) }
            }
        });
        section(ui, t, tr("Availability and Status"), &crate::trq!("You can let people know when you are active on {{APP_NAME}} and let them see your custom status when you set one. You can also see their availability and statuses."), false, |ui| {
            self.pcheck(ui, t, "show_presence", true, tr("Show availability and status"));
            self.pcheck(ui, t, "show_calendar_meeting", true, tr("Show when in a calendar meeting"));
        });
        section(ui, t, tr("Outlook Settings"), "", false, |ui| { self.pcheck(ui, t, "outlook_presence", false, &crate::trq!("Connect %1 to Microsoft Outlook", crate::app_name!())); });
        // Landing screen: values kept from the previous page ("home").
        section(ui, t, tr("Landing screen"), &crate::trq!("Choose what you'd like to see first every time {{APP_NAME}} opens."), false, |ui| {
            self.pcombo(ui, t, "home", "Messaging", &[("Messaging", tr("Messaging")), ("Meetings", tr("Meetings")), ("Calling", tr("Calling")), ("Contacts", tr("Contacts"))], 280.0);
        });
        section(ui, t, tr("Recent sessions"), tr("See devices that are currently signed in or have been active recently."), false, |ui| {
            if kit::link(ui, t, tr("Show Details")).clicked() { crate::ui::menu::menu_open(ui.ctx(), crate::ui::app_dialogs::SESSIONS); }
        });
        // Default file location: real folder dialog; stored in `download_dir` (the OS download folder unless set).
        section(ui, t, tr("Default file location"), "", false, |ui| {
            let dir = self.ss("download_dir", "");
            let shown = if dir.is_empty() { dirs_download() } else { dir };
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 100.0).clamp(80.0, 500.0);
                let g = ui.painter().layout_no_wrap(shown.clone(), FontId::proportional(kit::SECOND), t.text2);
                let (r, _) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::hover());
                crate::ui::chrome::fit_text(ui.painter(), r.left(), r.center().y, &shown, FontId::proportional(kit::SECOND), t.text2, w);
                let _ = g;
                if kit::pill(ui, t, tr("Change"), true, true).clicked() { super::sound_ui::pick_async("download_dir", true); }
            });
        });
        section(ui, t, tr("Local time zone"), &crate::trq!("We use this time zone to show others your local time in your profile.\nThis is also used for notification settings, such as Quiet hours.").replace('\n', " "), false, |ui| {
            // Only the computer's time zone is offered.
            self.pcombo(ui, t, "time_zone", "system", &[("system", tr("Use the computer's time zone"))], 472.0);
        });
        section(ui, t, tr("Multitasking"), "", false, |ui| { self.pcheck(ui, t, super::DOCK_AUTO_COVERED /* read by the dock */, false, &crate::trq!("Automatically show my {{APP_NAME}} call or meeting in a floating mini window when other apps are in front.")); });
        // Webox-specific: language, close-to-tray, browser helper, account.
        section(ui, t, tr("Language"), tr("The language of the interface."), false, |ui| { // "Language" = pack word
            self.pcombo(ui, t, "language", "system", &[("system", tr("System default")), ("de", "Deutsch"), ("en", "English")], 280.0);
        });
        section(ui, t, tr("Window"), "", false, |ui| { // key shared with notify.rs
            self.pcheck(ui, t, "close_to_tray", crate::platform::tray::close_to_tray_default(), tr("Minimize to the notification area when closing"));
        });
        // Webox's own updates (platform/updater): checked once a day; installing always asks first.
        section(ui, t, tr("Updates"), tr("Looks for a new version on GitHub once a day. Nothing is installed until you agree, and never during a call."), false, |ui| {
            self.pcheck(ui, t, "auto_update_check", true, tr("Check for updates automatically"));
            if kit::link(ui, t, tr("Check now")).clicked() { crate::ui::menu::menu_open(ui.ctx(), crate::ui::update_dialog::KEY); }
        });
        section(ui, t, tr("Meetings from the browser"), tr("Replaces the Webex browser helper."), false, |ui| { // Webox feature
            let label = if cfg!(windows) { tr("Open meeting links from Chrome, Edge and Firefox here") } else { tr("Open meeting links from Chrome, Chromium, Edge and Firefox here") };
            let mut v = self.sb("browser_helper", false);
            if kit::check(ui, &mut self.icons, t, label, &mut v, true, false) {
                match platform::nmhost::register(&self.cfg, v) { Ok(()) => self.sset("browser_helper", json!(v)), Err(e) => self.toast(e, now) }
            }
        });
        section(ui, t, tr("Account"), "", false, |ui| {
            if self.client.is_some() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  ·  {}", self.me.name, self.me.email)).size(kit::BODY).color(t.text1));
                    if kit::pill(ui, t, tr("Sign Out"), true, true).clicked() { ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("settings-signout"), true)); }
                });
            } else {
                ui.label(RichText::new(tr("You are looking at sample data.")).size(kit::BODY).color(t.text2));
                if kit::pill(ui, t, tr("Sign In"), true, true).clicked() { self.need_login = true; self.login_error.clear(); }
            }
            ui.add_space(8.0);
            ui.label(RichText::new(format!("Version {}  ·  {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS)).size(kit::SECOND).color(t.text2));
        });
    }

    // Audio.
    pub(super) fn page_audio(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, "", "", true, |ui| { self.settings_audio_devices(ui, t, now); });
        section(ui, t, tr("Ringtones and alerts"), "", false, |ui| { self.sound_section_ringer(ui, t, now); });
        section(ui, t, tr("Advanced audio settings"), "", false, |ui| {
            // "Bypass Windows signal processing" (Windows only). Stored; Webox's engine has its own chain.
            if cfg!(windows) { self.pcheck(ui, t, "bypass_windows_dsp", false, tr("Bypass Windows signal processing")); }
        });
    }
    /// Smart Audio. Microphone audio is real: the call microphone (and the Audio page's level meter) runs through the noise
    /// removal for "Auto" / "Voice only" (media/denoise.rs). External audio only concerns PSTN calls
    /// (receive-side noise removal); Webox has no PSTN calling, so it is stored only.
    pub(super) fn page_smart_audio(&mut self, ui: &mut egui::Ui, t: &Th) {
        text_note(ui, t, &crate::trq!("{{APP_NAME}} uses artificial intelligence to enhance audio."));
        ui.add_space(4.0);
        // Option labels of the Smart Audio page (not in the pack).
        let opts = [("auto", tr("Auto")), ("off", tr("Off")), ("voice", tr("Voice only")), ("all", tr("All noise"))];
        section(ui, t, tr("Microphone audio"), tr("Choose how you want your microphone audio to be heard during all calls and meetings."), false, |ui| {
            if let Some(v) = self.pradio(ui, t, "smart_mic", "auto", &opts) { self.apply_smart_mic(crate::media::audio::SmartMic::from_key(&v)); }
            // Webox addition (own labels): which engine runs (level meter / call microphone).
            use crate::media::denoise::{engine, Engine};
            let s = match engine() {
                Engine::Idle => "", Engine::Rnnoise => tr("Active: RNNoise"),
                Engine::Loading => tr("Loading DeepFilterNet …"), Engine::DeepFilter => tr("Active: DeepFilterNet 3"),
                Engine::RnnFallback => tr("Active: RNNoise (DeepFilterNet is too slow on this device)"),
                Engine::LoadFailed => tr("Active: RNNoise (DeepFilterNet could not be loaded)"),
            };
            if !s.is_empty() { ui.add_space(2.0); kit::check_desc(ui, t, s); }
        });
        section(ui, t, tr("External audio"), tr("Choose how you want to hear the external party's voice in one-on-one calls going over a public phone network."), false, |ui| {
            self.pradio(ui, t, "smart_ext", "auto", &[("auto", tr("Auto")), ("off", tr("Off"))]);
            text_note(ui, t, tr("May impact hold music and tones"));
        });
    }

    // Video: video_page.rs.

    // Sharing content: stored, inert.
    pub(super) fn page_share(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, tr("Share options"), "", true, |ui| {
            self.pradio(ui, t, "share_apps", "all", &[("all", tr("Share screen")), ("apps", tr("Share applications"))]);
        });
        section(ui, t, tr("Resolution and frame rate"), "", false, |ui| {
            self.pcombo(ui, t, "share_res", "auto", &[("auto", tr("Auto")), ("1080", "1080p"), ("4k", "4K")], 280.0);
        });
        section(ui, t, tr("Multi-Screen"), "", false, |ui| { self.pcheck(ui, t, "share_multi", false, tr("Multi-Screen")); });
    }

    // Appearance.
    pub(super) fn page_appearance(&mut self, ui: &mut egui::Ui, t: &Th) {
        // Themes: the ten themes in picker order as tiles in a four-column grid (theme_grid). The choice is stored under `theme`
        // (the theme id, e.g. "MomentumDark") and applied at once. While Windows high contrast is on the "OS High Contrast" theme
        // is used and the tiles are disabled.
        let hc = crate::ui::theme::hc_cached(ui.ctx());
        // Height of everything below the grid, measured last frame: the grid takes the rest of the viewport, so the sections
        // below sit at the bottom of the page like in Webex.
        let tail_id = egui::Id::new("appearance-tail");
        let tail = ui.ctx().data(|d| d.get_temp::<f32>(tail_id)).unwrap_or(300.0);
        let mut grid_bottom = 0.0;
        section(ui, t, tr("Themes"), &crate::trq!("Choose how you want {{APP_NAME}} to look."), true, |ui| {
            let cur = ThemeId::from_key(&self.ss("theme", "MomentumDark")).unwrap_or_default();
            let room = ui.clip_rect().bottom() - ui.cursor().top() - tail - 24.0; // 24: the page's bottom padding
            let pick = theme_grid(ui, t, cur, !hc, room);
            grid_bottom = ui.cursor().top();
            if hc { ui.add_space(4.0); kit::text_block(ui, tr("To choose this theme, first turn off high contrast in Windows settings."), kit::SECOND, t.text2, 0.0); }
            if let Some(id) = pick { self.sset("theme", json!(id.key())); self.applied = None; }
        });
        // Display density: applies at once (ui/theme/density.rs).
        section(ui, t, tr("Display density"), tr("Compact reduces the spacing in lists, messages, menus and settings. Font sizes stay the same."), false, |ui| {
            use crate::ui::theme::density::Density;
            let opts: Vec<(&str, &str)> = Density::ALL.iter().map(|d| (d.key(), tr(d.label()))).collect();
            self.pradio(ui, t, "density", Density::Standard.key(), &opts);
        });
        section(ui, t, tr("GIFs"), "", false, |ui| {
            self.pcheck(ui, t, "gifs_pause_unfocused", true, tr("Only play GIFs while Webox is the active window"));
        });
        section(ui, t, tr("Meetings"), "", false, |ui| {
            self.pcheck(ui, t, "always_dark_calls", false, tr("Always use dark theme"));
        });
        section(ui, t, tr("Reaction skin tone"), "", false, |ui| {
            let cur = self.settings["emoji_tone"].as_u64().unwrap_or(0) as usize;
            let mut pick = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (i, c) in [0xFFC83D, 0xF7D9B5, 0xE2B98E, 0xC18F62, 0x9A6B43, 0x5E4130].into_iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
                    if resp.hovered() { ui.painter().rect_filled(r, 20.0, kit::tok(t).item_hover); }
                    ui.painter().circle_filled(r.center(), 11.0, hex(c));
                    if cur == i { ui.painter().circle_stroke(r.center(), 17.0, Stroke::new(2.0_f32, t.text1)); }
                    if resp.clicked() { pick = Some(i); }
                }
            });
            if let Some(i) = pick { self.sset("emoji_tone", json!(i)); }
        });
        section(ui, t, "", "", false, |ui| { self.pcheck(ui, t, "hand_gestures", false, tr("Recognize hand gestures")); });
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Adjust more settings in Accessibility.")).size(kit::SECOND).color(t.text2));
            if kit::link(ui, t, tr("Change")).clicked() { ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("settings-goto"), Page::Accessibility)); }
        });
        let measured = ui.cursor().top() - grid_bottom;
        if (measured - tail).abs() > 0.5 { ui.ctx().data_mut(|d| d.insert_temp(tail_id, measured)); ui.ctx().request_repaint(); }
    }

    // Accessibility.
    pub(super) fn page_accessibility(&mut self, ui: &mut egui::Ui, t: &Th) {
        let names = [tr("Small"), tr("Regular (default)"), tr("Medium"), tr("Large")];
        let keys = ["small", "regular", "medium", "large"];
        let cur = keys.iter().position(|k| *k == self.ss("msg_font_size", "regular")).unwrap_or(1);
        section(ui, t, tr("Font size"), "", true, |ui| {
            // Preview box: radius 8, min height 72, centred text in the message font (scaled), on the hover colour.
            let scale = msg_font_scale_of(keys[cur]);
            let g = ui.painter().layout(tr("Drag the slider to see what your messages will look like.").into(), FontId::proportional(16.0 * scale), t.text1, ui.available_width() - 30.0);
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
        // Footer: "Reset to default".
        ui.add_space(16.0);
        if kit::pill(ui, t, tr("Reset to default"), false, true).clicked() { self.sset("msg_font_size", json!("regular")); self.toast(tr("Accessibility settings are reset to the default."), 0.0 + ui.input(|i| i.time)); }
    }

    // Privacy.
    pub(super) fn page_privacy(&mut self, ui: &mut egui::Ui, t: &Th, _now: f64) {
        section(ui, t, tr("Availability and Status"), &crate::trq!("You can let people know when you are active on {{APP_NAME}} and let them see your custom status when you set one. You can also see their availability and statuses."), true, |ui| {
            self.pcheck(ui, t, "show_presence", true, tr("Show availability and status"));
            let on = self.sb("show_presence", true);
            let mut c = self.sb("show_calendar_meeting", true);
            if kit::check(ui, &mut self.icons, t, tr("Show when in a calendar meeting"), &mut c, on, true) { self.sset("show_calendar_meeting", json!(c)); }
            kit::check_desc(ui, t, &crate::trq!("You can let people know when you are in a meeting organized through your email calendar and also active in {{APP_NAME}}."));
        });
        section(ui, t, tr("Hide my availability and status"), tr("Add a person to hide your availability and status from them."), false, |ui| {
            let n = self.settings["hidden_from"].as_array().map_or(0, |a| a.len());
            ui.horizontal(|ui| {
                if kit::pill(ui, t, tr("Add"), true, true).clicked() { crate::ui::menu::menu_open(ui.ctx(), crate::ui::app_dialogs::HIDE_FROM); }
                ui.label(RichText::new(crate::tr!("SettingsBlockedContactsDialog", "You're hiding your availability and status from %1 people", n)).size(kit::SECOND).color(t.text2));
            });
        });
    }

    // Messaging.
    pub(super) fn page_messaging(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, tr("Spell check"), "", true, |ui| {
            let on = self.pcheck_ret(ui, t, "spell_check", true, tr("Enable spell check"));
            ui.add_space(2.0);
            ui.horizontal(|ui| { ui.add_space(32.0); ui.add_enabled_ui(on, |ui| {
                self.pcombo(ui, t, "spell_lang", "de", &[("de", "Deutsch"), ("en", "English (US)"), ("fr", "Français")], 260.0); }); }); // dictionary list
        });
        // Typed emoticons such as :) or xD become emoji when a space follows and when sending (ui/composer, `emoticons_on`).
        section(ui, t, tr("Emoticons"), "", false, |ui| {
            self.pcheck(ui, t, "emoticons_to_emojis", false, tr("Convert emoticons such as :) or xD to emoji"));
        });
        // Space list: the compact view (ui/chrome/compact_list.rs).
        section(ui, t, &crate::trq!("Space list"), "", false, |ui| {
            self.pcheck(ui, t, crate::ui::chrome::compact_list::KEY, false, &crate::trq!("Show compact view"));
        });
        section(ui, t, tr("Threads"), tr("Choose how you want threads to show in your spaces and direct messages."), false, |ui| {
            self.pradio(ui, t, "threads", "expand", &[("expand", tr("Expand threads")), ("collapse", tr("Collapse threads"))]);
        });
        section(ui, t, tr("Read receipts"), tr("A read receipt lets people know who has seen a message. If this option is disabled, you will send a read receipt only when you send a message."), false, |ui| {
            self.pcheck(ui, t, "read_receipts", true, tr("Use Read Receipts"));
        });
        section(ui, t, tr("Autofill names"), "", false, |ui| { self.pcheck(ui, t, "name_autocomplete", true, tr("Autofill names")); });
        section(ui, t, tr("Emoji"), "", false, |ui| { // real effect: composer autocomplete
            self.pcheck(ui, t, "emoji_autocomplete", true, tr("Suggest emoji automatically"));
            // Webox addition: a message of only 1-3 emoji goes out as a heading, like typing '# 😆'.
            self.pcheck(ui, t, "big_emoji_send", true, tr("Send single emoji large"));
        });
        // Webox addition: typing with nothing focused writes into the open chat's message box (ui/type_focus.rs).
        section(ui, t, tr("Message box"), "", false, |ui| {
            self.pcheck(ui, t, crate::ui::type_focus::KEY, true, tr("Typing writes into the message box automatically"));
        });
    }
    fn pcheck_ret(&mut self, ui: &mut egui::Ui, t: &Th, key: &str, def: bool, label: &str) -> bool { self.pcheck(ui, t, key, def, label); self.sb(key, def) }

}

pub(super) fn text_note(ui: &mut egui::Ui, t: &Th, s: &str) { kit::text_block(ui, s, kit::SECOND, t.text2, 0.0); ui.add_space(4.0); }

/// Theme picker (see `page_appearance`): 120 x 84 tiles in four columns, 8 apart both ways, left-aligned. Each tile holds the
/// preview fitted into 120 x 48 (6 from the top) and the name centred under it; radius 12, a 2 pt ring when selected
/// (button-primary-border) or hovered (button-secondary-border), the button hover / pressed fill. The grid scrolls inside
/// `room` (the height the page leaves it, at least one row) and fills it, so the sections below stay at the page bottom.
fn theme_grid(ui: &mut egui::Ui, t: &Th, cur: ThemeId, enabled: bool, room: f32) -> Option<ThemeId> {
    const TW: f32 = 120.0;
    const TH: f32 = 84.0;
    const GAP: f32 = 8.0;
    const PREVIEW_H: f32 = 48.0;
    // Four columns; a body narrower than that wraps to fewer (Webox choice, Webex clips them).
    let cols = (((ui.available_width() + GAP) / (TW + GAP)).floor() as usize).clamp(1, 4);
    let rows = ThemeId::ALL.len().div_ceil(cols);
    let full_h = rows as f32 * (TH + GAP) - GAP;
    let h = room.max(TH);
    let k = kit::tok(t);
    let mut pick = None;
    crate::ui::widgets::vscroll().id_salt("theme-grid").max_height(h).auto_shrink([false, false]).show(ui, |ui| {
        let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), full_h), Sense::hover());
        for (i, id) in ThemeId::ALL.into_iter().enumerate() {
            let tile = Rect::from_min_size(area.min + Vec2::new((i % cols) as f32 * (TW + GAP), (i / cols) as f32 * (TH + GAP)), Vec2::new(TW, TH));
            let resp = ui.interact(tile, egui::Id::new(("theme-tile", id.idx())), if enabled { Sense::click() } else { Sense::hover() });
            let hover = enabled && resp.hovered();
            let p = ui.painter();
            let a = if enabled { 1.0 } else { 0.45 }; // opacity 0.45 while disabled
            if enabled && resp.is_pointer_button_down_on() { p.rect_filled(tile, 12.0, k.btn_press); }
            else if hover && !crate::ui::theme::hc_cached(ui.ctx()) { p.rect_filled(tile, 12.0, k.btn_hover); }
            // Preview: 140 x 92 fitted to the 48 px row, centred.
            let pw = PREVIEW_H * 140.0 / 92.0;
            paint_preview(p, Rect::from_min_size(pos2(tile.center().x - pw / 2.0, tile.top() + 6.0), Vec2::new(pw, PREVIEW_H)), id, a);
            // Name: label-primary-text, 12 px, centred in the 20 px left under the preview, elided.
            let col = t.tk("label-primary-text").gamma_multiply(a);
            let mut job = egui::text::LayoutJob::simple_singleline(tr(id.label()).to_string(), FontId::proportional(kit::SECOND), col);
            job.wrap = egui::text::TextWrapping::truncate_at_width(TW - 8.0);
            let g = p.layout_job(job);
            p.galley(pos2(tile.center().x, tile.bottom() - 16.0) - g.size() / 2.0, g, col);
            let ring = if cur == id { Some(t.tk("button-primary-border")) } else if hover { Some(t.tk("button-secondary-border:hovered")) } else { None };
            if let Some(c) = ring { p.rect_stroke(tile, 12.0, Stroke::new(2.0_f32, c.gamma_multiply(a)), egui::StrokeKind::Inside); }
            if hover { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            if enabled && resp.clicked() && cur != id { pick = Some(id); }
        }
    });
    pick
}

/// The theme preview (140 x 92 at scale 1): rounded background (radius 8) with a vertical gradient, the inner card
/// (x 32..140, y 8..88, gradient) and a 1 px outline at 20 % - white in dark themes, black in light ones. The card is rounded
/// 4 at the top left and bottom right so it stays inside the background's rounded corner.
pub(crate) fn paint_preview(p: &egui::Painter, r: Rect, id: ThemeId, a: f32) {
    let [b0, b1, c0, c1, line] = id.preview().map(|c| c.gamma_multiply(a));
    let s = r.width() / 140.0;
    vgrad(p, r, 8.0 * s, b0, b1);
    let card = Rect::from_min_max(pos2(r.left() + 32.0 * s, r.top() + 8.0 * s), pos2(r.right(), r.top() + 88.0 * s));
    crate::ui::widgets::vgrad_corners(p, card, [4.0 * s, 0.0, 4.0 * s, 0.0], c0, c1);
    p.rect_stroke(r, 8.0 * s, Stroke::new(1.0_f32, alpha(line, 0.2 * a)), egui::StrokeKind::Inside);
}

/// Message font scale of the Accessibility slider: Small / Regular / Medium / Large.
pub(crate) fn msg_font_scale_of(k: &str) -> f32 { match k { "small" => 0.875, "medium" => 1.125, "large" => 1.25, _ => 1.0 } }

/// The OS download folder (default of "Default file location").
pub(super) fn dirs_download() -> String { system::default_download_dir().to_string_lossy().into_owned() }
