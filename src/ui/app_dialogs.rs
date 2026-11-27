//! App-wide dialogs opened from the profile menu, its "Help" submenu, the header, the tray and shortcuts.
//! Drawn every frame by `appwide_dialogs` (hook in frame.rs). Each is a `menu::webex_dialog` keyed by the consts below
//! (open with `menu::menu_open(ctx, KEY)`). Texts are English sources, translated by the language packs where they have them.
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

/// Opens the date/time picker for "dnd" (DND "Customize...") or "status" (custom status "Custom").
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
    pub(crate) fn appwide_dialogs(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        self.edit_profile_dialog(ctx, t);
        self.health_dialog(ctx, t);
        self.feedback_dialog(ctx, t);
        self.updates_dialog(ctx, t); // ui/update_dialog.rs
        self.device_dialog(ctx, t);
        self.time_picker(ctx, t, now);
        self.sessions_dialog(ctx, t);
        self.hide_from_dialog(ctx, t);
        self.plugin_confirm_dialog(ctx, t);
        self.plugin_dialog(ctx, t);
    }

    /// A plugin's read-only dialog (`ui.dialog`, extras/plugins/dialog.rs): its switch, a row of tabs (a disabled one greyed
    /// out with its hint as tooltip), the selected tab's text (diff spans in green / red); "Close", the X and Esc close it.
    fn plugin_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        let Some(d) = self.plugins.dialog_shown().cloned() else { self.plugins.dialog = None; return };
        let k = format!("plugin-dialog-{}", d.serial);
        let (ks, kt) = (egui::Id::new((&k, "switch")), egui::Id::new((&k, "tab")));
        let mut on = ctx.data(|x| x.get_temp::<bool>(ks)).unwrap_or(d.switch.as_ref().is_some_and(|s| s.1));
        // WEBOX_SHOT_DIALOG_TAB=<n> (screenshot checks only): tab <n> is shown first.
        let first = self.shot.as_ref().and_then(|_| std::env::var("WEBOX_SHOT_DIALOG_TAB").ok()?.parse().ok()).unwrap_or(d.selected);
        let mut tab = ctx.data(|x| x.get_temp::<usize>(kt)).unwrap_or(first).min(d.tabs.len() - 1);
        if !menu::menu_is_open(ctx, &k) { menu::menu_open(ctx, &k); }
        menu::set_dialog_width(ctx, &k, 600.0);
        let (added, removed) = if t.light { (hex(0x1A7F37), hex(0xD43519)) } else { (hex(0x85E89D), hex(0xF55868)) };
        let mut body = |ui: &mut egui::Ui| {
            if let Some((label, _)) = &d.switch {
                ui.horizontal(|ui| {
                    ui.add(crate::ui::widgets::toggle(&mut on, t));
                    ui.add_space(6.0);
                    ui.label(RichText::new(label).size(14.0).color(t.text1));
                });
                ui.add_space(10.0);
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                for (i, tb) in d.tabs.iter().enumerate() {
                    let label = if tb.label.is_empty() && tb.time_ms > 0 { local_stamp(tb.time_ms) } else { tb.label.clone() };
                    if tb.disabled {
                        let r = ui.add(egui::Label::new(RichText::new(format!("  {label}  ")).size(14.0).color(alpha(t.text2, 0.45))).sense(Sense::hover()));
                        if !tb.hint.is_empty() { r.on_hover_text(&tb.hint); }
                    } else if crate::ui::widgets::text_tab(ui, &label, i == tab, t, t.tab_on).clicked() { tab = i; }
                }
            });
            ui.add_space(12.0);
            let tb = &d.tabs[tab];
            let spans = if on && !tb.alt.is_empty() { &tb.alt } else { &tb.text };
            crate::ui::widgets::vscroll().max_height(360.0).show(ui, |ui| {
                let job = crate::ui::messages::spans_job(spans, FontId::proportional(15.0), t.text1, added, removed, ui.available_width());
                ui.add(egui::Label::new(job).selectable(true));
            });
        };
        let res = menu::webex_dialog(ctx, t, &k, &d.title, "", Some(&mut body), &[Button::primary("close", tr("Close"))]);
        ctx.data_mut(|x| { x.insert_temp(ks, on); x.insert_temp(kt, tab); });
        if res.is_some() { self.plugins.dialog = None; menu::menu_close(ctx, &k); }
    }

    /// A plugin asks before it acts (`ui.confirm`, extras/plugins/confirm.rs): its text with its two buttons; the X and Esc
    /// answer "no". The plugin's thread waits for the answer; questions come one at a time, oldest first.
    fn plugin_confirm_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        let Some((id, title, text, ok, cancel)) = self.plugins.confirm_shown() else { return };
        let k = format!("plugin-confirm-{id}");
        if !menu::menu_is_open(ctx, &k) { menu::menu_open(ctx, &k); }
        if let Some(b) = menu::webex_dialog(ctx, t, &k, &title, &text, None, &[Button::secondary("no", &cancel), Button::primary("yes", &ok)]) {
            self.plugins.confirm_answer(id, b == "yes");
            menu::menu_close(ctx, &k);
        }
    }

    /// Settings > General > Recent sessions > "View details": the sessions Webox knows = this one (the device it registers,
    /// cloud.rs `name: "WebOx on <os>"`).
    fn sessions_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, SESSIONS) { return; }
        let signed = self.client.is_some();
        let mut body = |ui: &mut egui::Ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("WebOx on {}", std::env::consts::OS)).font(semibold(14.5)).color(t.text1));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(if signed { tr("This session · active") } else { tr("Not signed in") }).size(13.5).color(t.text2));
                });
            });
        };
        if menu::webex_dialog(ctx, t, SESSIONS, tr("Recent sessions"), "", Some(&mut body), &[Button::primary("close", tr("Close"))]).is_some() { menu::menu_close(ctx, SESSIONS); }
    }

    /// Settings > Privacy > "Hide my availability and status" > "Add": pick people; the list is kept in
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
            ui.add(egui::TextEdit::singleline(&mut q).hint_text(tr("Add by name, number, or email")).desired_width(f32::INFINITY)); // the header "+" pane's hint
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { if ui.small_button(tr("Remove")).clicked() { del = Some(i); } });
                });
            }
            if let Some(i) = del { list.remove(i); changed = true; }
            ui.add_space(6.0);
            ui.label(RichText::new(tr("This list is only saved in Webox; the Webex service is not told about it.")).size(12.5).color(t.text2));
        };
        let res = menu::webex_dialog(ctx, t, HIDE_FROM, tr("Hide my availability and status"), "", Some(&mut body), &[Button::primary("close", tr("Done"))]);
        if changed { self.settings["hidden_from"] = json!(list); if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); } }
        ctx.data_mut(|d| d.insert_temp(qid, q));
        if res.is_some() { menu::menu_close(ctx, HIDE_FROM); }
    }

    /// "Edit profile" (picture + name + pronouns + cover). Shown read-only: editing the name and picture is not
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
                    ui.label(RichText::new(tr("Profile picture")).font(semibold(14.0)).color(t.text1));
                    ui.add_space(14.0);
                    ui.label(RichText::new(tr("Display name")).font(semibold(14.0)).color(t.text1));
                    ui.label(RichText::new(if name.is_empty() { "—" } else { &name }).size(15.0).color(t.text1));
                    if !email.is_empty() { ui.add_space(4.0); ui.label(RichText::new(&email).size(13.5).color(t.text2)); }
                });
            });
        };
        if menu::webex_dialog(ctx, t, EDIT_PROFILE, tr("Edit profile"), "", Some(&mut body), &[Button::primary("close", tr("Close"))]).is_some() { menu::menu_close(ctx, EDIT_PROFILE); }
    }

    /// "Health Checker": what Webox itself knows - signed in, device registration, the connection state.
    fn health_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, HEALTH) { return; }
        let connected = self.client.is_some() && !self.need_login;
        let registered = connected && self.settings["device"].as_str().is_some_and(|d| !d.is_empty());
        let rows: Vec<(&str, &str, bool)> = vec![
            (tr("Server connection"), if connected { tr("Operational") } else { tr("Not registered") }, connected),
            (tr("Client registered"), if registered { tr("Registered") } else { tr("Not registered") }, registered),
        ];
        let ok = rows.iter().all(|r| r.2);
        let mut body = |ui: &mut egui::Ui| {
                        ui.label(RichText::new(if ok { tr("Everything looks good here!") } else { tr("One or more services are inaccessible") }).font(semibold(15.0)).color(t.text1));
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
        match menu::webex_dialog(ctx, t, HEALTH, tr("Health Checker"), "", Some(&mut body), &[Button::secondary("refresh", tr("Refresh")), Button::primary("close", tr("Close"))]).as_deref() {
            // "Refresh" re-runs the read-only checks (the space list reload is the connection test).
            Some("refresh") => if let Some(c) = &self.client { c.req(Req::Spaces); },
            Some(_) => menu::menu_close(ctx, HEALTH),
            None => {}
        }
    }

    /// "Send Feedback": Webox uploads no logs; it offers its own log folder ("Save Logs").
    fn feedback_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, FEEDBACK) { return; }
        let body = tr("Webox sends no logs to Cisco. Its logs are in its settings folder.");
        match menu::webex_dialog(ctx, t, FEEDBACK, tr("Send Feedback"), body, None, &[Button::secondary("logs", tr("Save Logs")), Button::primary("close", tr("Close"))]).as_deref() {
            Some("logs") => { system::open_folder(&self.cfg); menu::menu_close(ctx, FEEDBACK); }
            Some(_) => menu::menu_close(ctx, FEEDBACK),
            None => {}
        }
    }

    /// "Connect to a device" (device list → device search → code pairing). Webox finds no devices (no ultrasound / device
    /// search service), so the list shows the no-device state; the code page takes the 9-character code, but pairing itself is
    /// not implemented and answers "Couldn't connect automatically".
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
                DevPage::List => { ui.label(RichText::new(tr("No devices found")).size(15.0).color(t.text2)); }
                DevPage::Search => {
                    ui.add(egui::TextEdit::singleline(&mut q).hint_text(tr("Search for a room or device")).desired_width(f32::INFINITY));
                    ui.add_space(8.0);
                    ui.label(RichText::new(tr("No devices found")).size(14.0).color(t.text2));
                    ui.add_space(8.0);
                    let l = ui.add(egui::Label::new(RichText::new(tr("Connect using 9-character code")).size(14.0).color(t.link)).sense(Sense::click()));
                    if l.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    if l.clicked() { go_code = true; }
                }
                DevPage::Code => {
                    ui.label(RichText::new(tr("Enter the 9-character code shown on the Webex device.")).size(14.0).color(t.text1));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| { for c in code.iter_mut() { ui.add(egui::TextEdit::singleline(c).char_limit(3).desired_width(60.0)); } });
                }
                DevPage::Failed => { ui.label(RichText::new(tr("Couldn't connect automatically")).size(15.0).color(t.text1)); }
            };
            let buttons: Vec<Button> = match page {
                DevPage::List => vec![Button::secondary("close", tr("Close")), Button::primary("search", tr("Search for devices"))],
                DevPage::Search => vec![Button::secondary("back", tr("Back")), Button::primary("close", tr("Close"))],
                DevPage::Code => vec![Button::secondary("back", tr("Back")), Button::primary("next", tr("Next"))],
                DevPage::Failed => vec![Button::secondary("search", tr("Search for devices")), Button::primary("close", tr("Close"))],
            };
            menu::webex_dialog(ctx, t, DEVICE, tr("Connect to a device"), "", Some(&mut body), &buttons)
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

    /// Date / time picker for DND "Customize..." and the custom status' "Custom" (range: 30 minutes .. 30 days).
    fn time_picker(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        if !menu::menu_is_open(ctx, PICKER) { return; }
        let what: String = ctx.data(|d| d.get_temp(egui::Id::new("picker-what"))).unwrap_or_default();
        let mut day: usize = ctx.data(|d| d.get_temp(egui::Id::new("picker-day"))).unwrap_or(0);
        let mut slot: usize = ctx.data(|d| d.get_temp(egui::Id::new("picker-slot"))).unwrap_or(0);
        let n = crate::i18n::now_ms();
        let off = crate::i18n::local_offset_secs(n / 1000);
        let days: Vec<String> = (0..=status_time::MAX_DAYS as usize).map(|i| match i {
            0 => tr("Today").to_string(),
            1 => tr("Tomorrow").to_string(),
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
            if !ok { ui.add_space(6.0); ui.label(RichText::new(tr("Choose a time between 30 minutes and 30 days from now.")).size(12.5).color(hex(0xF55868))); }
        };
        let title = if what == "dnd" { tr("Set Do not disturb for") } else { tr("Clear status after") };
        let res = menu::webex_dialog(ctx, t, PICKER, title, "", Some(&mut body), &[Button::secondary("cancel", tr("Cancel")), Button::primary("save", tr("Save"))]);
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

/// Unix ms as a local date and time: "25.11.2026, 14:05".
pub(crate) fn local_stamp(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let local = secs + crate::i18n::local_offset_secs(secs) as i64;
    let (y, m, d) = crate::i18n::civil_from_days(local.div_euclid(86_400));
    let s = local.rem_euclid(86_400);
    format!("{d:02}.{m:02}.{y}, {:02}:{:02}", s / 3600, s % 3600 / 60)
}

