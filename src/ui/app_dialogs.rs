//! App-wide dialogs opened from the profile menu, its "Hilfe" submenu, the header, the tray and shortcuts.
//! Drawn every frame by `appwide_dialogs` (hook in frame.rs). Each is a `menu::webex_dialog` keyed by the consts below
//! (open with `menu::menu_open(ctx, KEY)`). Texts from the German pack where it has them, else Webox's own wording.
use crate::*;
use super::menu::{self, Button};
use super::status_time;

pub(crate) const EDIT_PROFILE: &str = "appwide-edit-profile";
pub(crate) const HEALTH: &str = "appwide-health";
pub(crate) const FEEDBACK: &str = "appwide-feedback";
pub(crate) const UPDATES: &str = "appwide-updates";
pub(crate) const DEVICE: &str = "appwide-device";
const PICKER: &str = "appwide-time-picker";
pub(crate) const SESSIONS: &str = "appwide-sessions";
pub(crate) const HIDE_FROM: &str = "appwide-hide-from";

/// Opens the date/time picker for "dnd" (DND "Anpassen...") or "status" (custom status "Benutzerdefiniert").
pub(crate) fn open_time_picker(ctx: &egui::Context, what: &str) {
    let n = crate::i18n::now_ms();
    let (day, slot) = status_time::first_slot(n, crate::i18n::local_offset_secs(n / 1000));
    ctx.data_mut(|d| { d.insert_temp(egui::Id::new("picker-what"), what.to_string()); d.insert_temp(egui::Id::new("picker-day"), day); d.insert_temp(egui::Id::new("picker-slot"), slot); });
    menu::menu_open(ctx, PICKER);
}

/// The device popup's page: list (no devices) → search → 9-character code → result.
#[derive(Clone, Copy, PartialEq, Default)]
enum DevPage { #[default] List, Search, Code, Failed }

impl App {
    fn call_active(&self) -> bool { self.call.cur.as_ref().is_some_and(|c| c.ended_at.is_none()) }

    pub(crate) fn appwide_dialogs(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        self.edit_profile_dialog(ctx, t);
        self.health_dialog(ctx, t);
        self.feedback_dialog(ctx, t);
        self.updates_dialog(ctx, t); // ui/update_dialog.rs
        self.device_dialog(ctx, t);
        self.time_picker(ctx, t, now);
        self.sessions_dialog(ctx, t);
        self.hide_from_dialog(ctx, t);
    }

    /// Settings > Allgemein > Letzte Sitzungen > "Details anzeigen": the sessions Webox knows = this one (the device it registers,
    /// cloud.rs `name: "WebOx on <os>"`).
    fn sessions_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, SESSIONS) { return; }
        let signed = self.client.is_some();
        let mut body = |ui: &mut egui::Ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("WebOx on {}", std::env::consts::OS)).font(semibold(14.5)).color(t.text1));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(if signed { "Diese Sitzung · aktiv" } else { "Nicht angemeldet" }).size(13.5).color(t.text2));
                });
            });
        };
        if menu::webex_dialog(ctx, t, SESSIONS, "Letzte Sitzungen", "", Some(&mut body), &[Button::primary("close", "Schließen")]).is_some() { menu::menu_close(ctx, SESSIONS); }
    }

    /// Settings > Privatsphäre > "Meine Verfügbarkeit und meinen Status ausblenden" > "Hinzufügen": pick people; the list is kept in
    /// the settings (`hidden_from`, person ids). Not sent to the service, so these people still see the presence (the dialog
    /// says so).
    fn hide_from_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, HIDE_FROM) { return; }
        let qid = egui::Id::new("hide-from-q");
        let mut q: String = ctx.data(|d| d.get_temp(qid)).unwrap_or_default();
        let mut list: Vec<String> = self.settings["hidden_from"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        let people = self.people.clone();
        let mut changed = false;
        let mut body = |ui: &mut egui::Ui| {
            ui.add(egui::TextEdit::singleline(&mut q).hint_text("Anhand von Name, Nummer oder E-Mail-Adresse hinzufügen").desired_width(f32::INFINITY)); // the header "+" pane's hint
            ui.add_space(6.0);
            let ql = q.to_lowercase();
            let hits: Vec<&Person> = people.iter().filter(|p| !ql.is_empty() && (p.name.to_lowercase().contains(&ql) || p.email.to_lowercase().contains(&ql))).take(6).collect();
            for p in hits {
                if list.contains(&p.id) { continue; }
                let r = ui.add(egui::Label::new(RichText::new(format!("+ {}", p.name)).size(14.0).color(t.link)).sense(Sense::click()));
                if r.clicked() { list.push(p.id.clone()); changed = true; }
            }
            ui.add_space(8.0);
            let mut del = None;
            for (i, id) in list.iter().enumerate() {
                let name = people.iter().find(|p| &p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| id.clone());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).size(14.0).color(t.text1));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { if ui.small_button("Entfernen").clicked() { del = Some(i); } }); // pack "Remove"
                });
            }
            if let Some(i) = del { list.remove(i); changed = true; }
            ui.add_space(6.0);
            ui.label(RichText::new("Diese Liste wird nur in Webox gespeichert; der Webex-Dienst wird nicht informiert.").size(12.5).color(t.text2));
        };
        let res = menu::webex_dialog(ctx, t, HIDE_FROM, "Meine Verfügbarkeit und meinen Status ausblenden", "", Some(&mut body), &[Button::primary("close", "Fertig")]);
        if changed { self.settings["hidden_from"] = json!(list); if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); } }
        ctx.data_mut(|d| d.insert_temp(qid, q));
        if res.is_some() { menu::menu_close(ctx, HIDE_FROM); }
    }

    /// "Profil bearbeiten" (picture + name + pronouns + cover). Shown read-only: editing the name and picture is not
    /// implemented, so there is no Save button.
    fn edit_profile_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, EDIT_PROFILE) { return; }
        let photo = [self.me.email.clone(), self.me.id.clone()].iter().find_map(|k| self.photo(ctx, k));
        let (name, email) = (self.me.name.clone(), self.me.email.clone());
        let icons = &mut self.icons;
        let mut body = |ui: &mut egui::Ui| {
            ui.horizontal_top(|ui| {
                avatar_photo(ui, icons, t, &name, 96.0, false, None, None, photo);
                ui.add_space(20.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Profilbild").font(semibold(14.0)).color(t.text1)); // pack "Profile picture"
                    ui.add_space(14.0);
                    ui.label(RichText::new("Anzeigename").font(semibold(14.0)).color(t.text1)); // pack "Display name"
                    ui.label(RichText::new(if name.is_empty() { "—" } else { &name }).size(15.0).color(t.text1));
                    if !email.is_empty() { ui.add_space(4.0); ui.label(RichText::new(&email).size(13.5).color(t.text2)); }
                });
            });
        };
        if menu::webex_dialog(ctx, t, EDIT_PROFILE, "Profil bearbeiten", "", Some(&mut body), &[Button::primary("close", "Schließen")]).is_some() { menu::menu_close(ctx, EDIT_PROFILE); }
    }

    /// "Zustandsüberprüfung": what Webox itself knows - signed in, device registration, the connection state.
    fn health_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, HEALTH) { return; }
        let connected = self.client.is_some() && !self.need_login;
        let registered = connected && self.settings["device"].as_str().is_some_and(|d| !d.is_empty());
        let rows: Vec<(&str, &str, bool)> = vec![
            ("Serververbindung", if connected { "Betriebsbereit" } else { "Nicht registriert" }, connected),
            ("Client registriert", if registered { "Registriert" } else { "Nicht registriert" }, registered),
        ];
        let ok = rows.iter().all(|r| r.2);
        let mut body = |ui: &mut egui::Ui| {
            // "Everything looks good here!" / "One or more services are inaccessible".
            ui.label(RichText::new(if ok { "Alles sieht gut aus!" } else { "Mindestens ein Dienst ist nicht verfügbar" }).font(semibold(15.0)).color(t.text1));
            ui.add_space(10.0);
            for (k, v, good) in &rows {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(*k).size(14.0).color(t.text1));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(*v).size(14.0).color(if *good { hex(0x27A17A) } else { hex(0xF55868) }));
                    });
                });
                ui.add_space(4.0);
            }
            ui.add_space(4.0);
            ui.label(RichText::new(&self.status).size(12.5).color(t.text2));
        };
        match menu::webex_dialog(ctx, t, HEALTH, "Zustandsüberprüfung", "", Some(&mut body), &[Button::secondary("refresh", "Aktualisieren"), Button::primary("close", "Schließen")]).as_deref() {
            // "Aktualisieren" re-runs the read-only checks (the space list reload is the connection test).
            Some("refresh") => if let Some(c) = &self.client { c.req(Req::Spaces); },
            Some(_) => menu::menu_close(ctx, HEALTH),
            None => {}
        }
    }

    /// "Feedback senden": Webox uploads no logs; it offers its own log folder ("Protokolle speichern").
    fn feedback_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, FEEDBACK) { return; }
        let body = "Webox sendet keine Protokolle an Cisco. Die Protokolle von Webox liegen in ihrem Einstellungsordner.";
        match menu::webex_dialog(ctx, t, FEEDBACK, "Feedback senden", body, None, &[Button::secondary("logs", "Protokolle speichern"), Button::primary("close", "Schließen")]).as_deref() {
            Some("logs") => { system::open_folder(&self.cfg); menu::menu_close(ctx, FEEDBACK); }
            Some(_) => menu::menu_close(ctx, FEEDBACK),
            None => {}
        }
    }

    /// "Mit einem Gerät verbinden" (device list → device search → code pairing). Webox finds no devices (no ultrasound / device
    /// search service), so the list shows the no-device state; the code page takes the 9-character code, but pairing itself is
    /// not implemented and answers "Verbindung konnte nicht automatisch hergestellt werden".
    fn device_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, DEVICE) { return; }
        let pid = egui::Id::new("device-page");
        let page: DevPage = ctx.data(|d| d.get_temp(pid)).unwrap_or_default();
        let qid = egui::Id::new("device-query");
        let cid = egui::Id::new("device-code");
        let mut q: String = ctx.data(|d| d.get_temp(qid)).unwrap_or_default();
        let mut code: [String; 3] = ctx.data(|d| d.get_temp(cid)).unwrap_or_default();
        let mut go_code = false;
        let res = {
            let mut body = |ui: &mut egui::Ui| match page {
                DevPage::List => { ui.label(RichText::new("Keine Geräte gefunden").size(15.0).color(t.text2)); }
                DevPage::Search => {
                    ui.add(egui::TextEdit::singleline(&mut q).hint_text("Nach einem Raum oder Gerät suchen").desired_width(f32::INFINITY));
                    ui.add_space(8.0);
                    ui.label(RichText::new("Keine Geräte gefunden").size(14.0).color(t.text2));
                    ui.add_space(8.0);
                    let l = ui.add(egui::Label::new(RichText::new("Verbindung über neunstelligen Code herstellen").size(14.0).color(t.link)).sense(Sense::click()));
                    if l.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    if l.clicked() { go_code = true; }
                }
                DevPage::Code => {
                    ui.label(RichText::new("Geben Sie den auf dem Webex-Gerät angezeigten neunstelligen Code ein.").size(14.0).color(t.text1));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| { for c in code.iter_mut() { ui.add(egui::TextEdit::singleline(c).char_limit(3).desired_width(60.0)); } });
                }
                DevPage::Failed => { ui.label(RichText::new("Verbindung konnte nicht automatisch hergestellt werden").size(15.0).color(t.text1)); }
            };
            let buttons: Vec<Button> = match page {
                DevPage::List => vec![Button::secondary("close", "Schließen"), Button::primary("search", "Nach Geräten suchen")],
                DevPage::Search => vec![Button::secondary("back", "Zurück"), Button::primary("close", "Schließen")],
                DevPage::Code => vec![Button::secondary("back", "Zurück"), Button::primary("next", "Weiter")],
                DevPage::Failed => vec![Button::secondary("search", "Nach Geräten suchen"), Button::primary("close", "Schließen")],
            };
            menu::webex_dialog(ctx, t, DEVICE, "Mit einem Gerät verbinden", "", Some(&mut body), &buttons)
        };
        let mut next = if go_code { DevPage::Code } else { page };
        match res.as_deref() {
            Some("search") => next = DevPage::Search,
            Some("back") => next = if page == DevPage::Code { DevPage::Search } else { DevPage::List },
            Some("next") if code.iter().map(|c| c.trim().chars().count()).sum::<usize>() == 9 => next = DevPage::Failed,
            Some("next") => {}
            Some(_) => { menu::menu_close(ctx, DEVICE); next = DevPage::List; q.clear(); code = Default::default(); }
            None => {}
        }
        ctx.data_mut(|d| { d.insert_temp(pid, next); d.insert_temp(qid, q); d.insert_temp(cid, code); });
    }

    /// Date / time picker for DND "Anpassen..." and the custom status' "Benutzerdefiniert" (range: 30 minutes .. 30 days).
    fn time_picker(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        if !menu::menu_is_open(ctx, PICKER) { return; }
        let what: String = ctx.data(|d| d.get_temp(egui::Id::new("picker-what"))).unwrap_or_default();
        let mut day: usize = ctx.data(|d| d.get_temp(egui::Id::new("picker-day"))).unwrap_or(0);
        let mut slot: usize = ctx.data(|d| d.get_temp(egui::Id::new("picker-slot"))).unwrap_or(0);
        let n = crate::i18n::now_ms();
        let off = crate::i18n::local_offset_secs(n / 1000);
        let days: Vec<String> = (0..=status_time::MAX_DAYS as usize).map(|i| match i {
            0 => "Heute".to_string(), // pack "Today"
            1 => "Morgen".to_string(), // pack "Tomorrow"
            _ => crate::i18n::format_date_time(status_time::picked_ms(n, off, i, 0) + 12 * 3_600_000, crate::i18n::DateStyle::Date),
        }).collect();
        let slots: Vec<String> = (0..48).map(|i| format!("{:02}:{:02}", i / 2, (i % 2) * 30)).collect();
        let until = status_time::picked_ms(n, off, day, slot);
        let ok = status_time::picked_ok(n, until);
        let icons = &mut self.icons;
        let mut body = |ui: &mut egui::Ui| {
            ui.horizontal(|ui| {
                menu::dropdown(ui, icons, t, "picker-day", &mut day, &days, 180.0);
                ui.add_space(8.0);
                menu::dropdown(ui, icons, t, "picker-slot", &mut slot, &slots, 110.0);
            });
            if !ok { ui.add_space(6.0); ui.label(RichText::new("Wählen Sie einen Zeitpunkt zwischen 30 Minuten und 30 Tagen ab jetzt.").size(12.5).color(hex(0xF55868))); }
        };
        let title = if what == "dnd" { "„Bitte nicht stören“ einrichten für" } else { "Status löschen nach" };
        let res = menu::webex_dialog(ctx, t, PICKER, title, "", Some(&mut body), &[Button::secondary("cancel", "Abbrechen"), Button::primary("save", "Speichern")]);
        ctx.data_mut(|d| { d.insert_temp(egui::Id::new("picker-day"), day); d.insert_temp(egui::Id::new("picker-slot"), slot); });
        match res.as_deref() {
            Some("save") if ok => { menu::menu_close(ctx, PICKER); self.time_picked(&what, until, ctx); }
            Some("save") => {}
            Some(_) => menu::menu_close(ctx, PICKER),
            None => {}
        }
        let _ = now;
    }
}
