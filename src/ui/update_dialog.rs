//! App updates in the UI (platform/updater): the daily background check, the green update button in the rail above the
//! settings gear while a newer version is known, and the "Check for updates" dialog (from the rail button, the profile menu's
//! "Help", the tray and Settings). Nothing happens during a call: no check, no install, no restart, no rail button.
use crate::platform::updater::{self, Phase};
use crate::*;
use super::menu::{self, Button};

pub(crate) const KEY: &str = super::app_dialogs::UPDATES;
const STARTED: &str = "updates-started";
/// The rail button's colour (green; the theme has no token of its own for it).
const GREEN: Color32 = hex(0x27A17A);

impl App {
    fn in_call(&self) -> bool { self.call.cur.as_ref().is_some_and(|c| c.ended_at.is_none()) }

    /// Once per frame: the call flag for the installer, the daily check, the restart once the new version is in place.
    pub(crate) fn updates_frame(&mut self, ctx: &egui::Context, now: f64) {
        let in_call = self.in_call();
        updater::IN_CALL.store(in_call, std::sync::atomic::Ordering::Relaxed);
        // WEBOX_SHOT_UPDATE=available|modal|downloading: the rail button / dialog with a made-up release.
        if self.shot.is_some() && self.frames == 1 {
            if let Ok(v) = std::env::var("WEBOX_SHOT_UPDATE") {
                updater::sample(v == "downloading");
                if v != "available" { menu::menu_open(ctx, KEY); }
            }
        }
        if updater::NO_AUTO_CHECK.load(std::sync::atomic::Ordering::Relaxed) || std::env::var_os("WEBOX_NO_UPDATE_CHECK").is_some() || updater::from_nix_store() { return; }
        let n = crate::ui::chrome::now_ms();
        let last = self.settings["update_checked"].as_i64().unwrap_or(0);
        if self.settings["auto_update_check"].as_bool().unwrap_or(true) && !in_call && (n - last >= updater::AUTO_INTERVAL_MS || n < last) {
            self.settings["update_checked"] = json!(n);
            system::save_settings(&self.cfg, &self.settings);
            updater::check(true);
        }
        if !in_call && updater::status().phase == Phase::Ready {
            match updater::restart() {
                Ok(()) => { self.notify.quitting = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                Err(e) => self.toast(e, now),
            }
        }
    }

    /// The rail's update button (24 pt `update_24` in green), shown while a newer version is known and no call runs.
    /// Its tooltip is the pack's "Click to upgrade, which will require a restart"; Linux has no restart, so it names the version.
    pub(crate) fn rail_update_button(&mut self, ui: &mut egui::Ui, t: &Th, h: f32) {
        let s = updater::status();
        if !s.offered() || self.in_call() { return; }
        let version = s.release.map(|r| r.version.to_string()).unwrap_or_default();
        let tip = if updater::self_update() { tr("Click to upgrade, which will require a restart").to_string() } else { crate::trq!("Version %1 is available", version) };
        if self.rail_item_tinted(ui, t, "update_24", &tip, Some(GREEN), h).clicked() { menu::menu_open(ui.ctx(), KEY); }
    }

    pub(crate) fn updates_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        let sid = egui::Id::new(STARTED);
        if !menu::menu_is_open(ctx, KEY) { ctx.data_mut(|d| d.remove::<bool>(sid)); return; }
        let in_call = self.in_call();
        let s = updater::status();
        // Opening the dialog checks again unless a newer version is already known.
        if !in_call && !ctx.data(|d| d.get_temp::<bool>(sid)).unwrap_or(false) {
            ctx.data_mut(|d| d.insert_temp(sid, true));
            if matches!(s.phase, Phase::Idle | Phase::UpToDate | Phase::Failed(_)) && self.shot.is_none() { updater::check(false); }
        }
        let version = s.release.as_ref().map(|r| r.version.to_string()).unwrap_or_default();
        let notes = s.release.as_ref().map(|r| updater::release::excerpt(&r.notes, 8, 420)).unwrap_or_default();
        let page = s.release.as_ref().map(|r| r.page.clone()).unwrap_or_else(|| updater::release::RELEASES_PAGE.into());
        let can_install = updater::self_update() && s.release.as_ref().is_some_and(|r|
            updater::release::platform_asset(&r.assets, std::env::consts::OS, std::env::consts::ARCH).is_some());
        #[cfg(target_os = "macos")]
        let can_install = can_install && std::env::current_exe().ok().and_then(|e| updater::macos::bundle_of(&e)).is_some();
        let news = tr("Great news! There's an update available.");
        let with_notes = |head: &str| if notes.is_empty() { head.to_string() } else { format!("{head}\n\n{notes}") };
        let close = Button::primary("close", tr("Close"));
        let (head, info, buttons): (String, String, Vec<Button>) = match &s.phase {
            _ if in_call && s.phase != Phase::Ready => (tr("Check for updates").into(),
                tr("Check for updates is unavailable while you're on a call").into(), vec![close]),
            Phase::Idle | Phase::Checking => (tr("Check for updates").into(), tr("Checking for updates...").into(), vec![close]),
            Phase::UpToDate => (tr("You're up to date").into(),
                crate::trq!("You're on the latest version ({{0}}).", updater::VERSION), vec![close]),
            Phase::Available if can_install => (crate::trq!("Version %1 is available", version), with_notes(news),
                vec![Button::secondary("later", tr("Later")), Button::primary("install", tr("Update now"))]),
            // macOS outside Webox.app (`cargo run`): nothing to replace, the download page instead.
            Phase::Available if cfg!(target_os = "macos") => (crate::trq!("Version %1 is available", version),
                format!("{}\n\n{}", with_notes(news), tr("Download Webox.app from the release page on GitHub and replace the old app.")),
                vec![Button::secondary("page", tr("Download")), close]),
            Phase::Available => (crate::trq!("Version %1 is available", version),
                format!("{}\n\n{}", with_notes(news), tr("Packages from Nix or your distribution are updated by the system update.")),
                vec![Button::secondary("page", tr("Release notes")), close]),
            Phase::Downloading | Phase::Installing => (crate::trq!("Installing version %1", version), String::new(), vec![Button::secondary("later", tr("Hide"))]),
            Phase::Ready => (crate::trq!("Version %1 is installed", version),
                crate::trq!("%1 restarts after the call.", crate::i18n::APP_NAME), vec![close]),
            Phase::Failed(e) if s.release.is_none() => (tr("Unable to check for updates").into(), e.clone(),
                vec![Button::secondary("retry", tr("Try again")), close]),
            Phase::Failed(e) => (tr("Update failed").into(), format!("{e}\n{}", tr("Your current version stays as it is.")),
                vec![Button::secondary("retry", tr("Try again")), close]),
        };
        let busy = matches!(s.phase, Phase::Downloading | Phase::Installing);
        let installing = s.phase == Phase::Installing;
        let mut progress = |ui: &mut egui::Ui| {
            let (done, total) = updater::progress();
            let f = if installing { 1.0 } else if total > 0 { (done as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
            // A 4 pt track with the filled part in the link blue.
            let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
            ui.painter().rect_filled(r, 2.0, t.ink_a(0.2));
            ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width() * f, r.height())), 2.0, t.link);
            ui.add_space(8.0);
            let text = if installing { tr("Checking and installing the new version …").to_string() }
                else { crate::trq!("%1 of %2 MB", format!("{:.1}", done as f64 / 1e6), format!("{:.1}", total as f64 / 1e6)) };
            ui.label(RichText::new(text).size(13.0).color(t.text2));
            if self.shot.is_none() { ui.ctx().request_repaint_after(std::time::Duration::from_millis(250)); }
        };
        let res = menu::webex_dialog(ctx, t, KEY, &head, &info, if busy { Some(&mut progress) } else { None }, &buttons);
        match res.as_deref() {
            Some("install") if !in_call && self.shot.is_none() => updater::install(),
            Some("install") => {}
            Some("page") => system::open(&page),
            Some("retry") => updater::check(false),
            // "Later" closes the dialog; the rail button stays.
            Some(_) => menu::menu_close(ctx, KEY),
            None => {}
        }
    }
}
