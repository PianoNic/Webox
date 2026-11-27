//! The profile menu that opens from the header's avatar, and the custom-status dialog (opened by the header's
//! "Set a Status" button and by the menu's "Status" item).
//! Wording from the language packs: "Set a Status", presence names, Availability, Status, Set Do not disturb for,
//! Edit profile, Settings, Help, Sign out, Exit %1, durations (30 minutes, 1 hour, 2 hours, 4 hours,
//! Today, Until turned off, Custom…), the dialog ("Status", "What's your status?", "Clear status after",
//! "Clear status", n/75).
//!
//! What the items do:
//! * Availability submenu: Available / Away / Do not disturb (+ duration) / Clear Do not disturb:
//!   `Req::SetPresence` = apheleia `POST /events` (cloud/presence_set.rs). Sample mode: local only.
//! * Custom status: kept in the settings (`custom_status` = {text, until}) and shown in the header until it expires. Not sent
//!   to the service, so colleagues do not see it.
//! * Edit profile → the "Edit profile" dialog (ui/app_dialogs.rs),
//!   Help → submenu (Help Center, What's New, Keyboard shortcuts, Health Checker, Send Feedback, Check for updates, About),
//!   Sign out → the sign-out confirmation, Exit → quit.
use crate::*;
use super::menu::{self, Button, Entry, Item, Side, Align};
use super::status_time::{self, Dur, CLEAR_AFTER, CLEAR_DEFAULT, DND};

const MAX_CHARS: usize = 75; // maximum length of the custom status
/// Help Center.
pub(crate) const HELP_CENTER_URL: &str = "https://help.webex.com/";
/// The preset list ("Choose a status"), pack sources.
const PRESETS: [&str; 4] = ["Working from home \u{1f3e0}", "Traveling for business \u{1f680}", "Out for lunch \u{1f37d}\u{fe0f}", "Be right back \u{23f3}\u{fe0f}"];
/// The shared emoji picker is open for the status dialog's emoji button (the composer must not draw / consume it).
const STATUS_EMOJI: &str = "status-emoji-picker";

fn now_ms() -> i64 { crate::i18n::now_ms() }
fn offset(now: i64) -> i32 { crate::i18n::local_offset_secs(now / 1000) }

/// Opens the custom-status dialog with the current text. Used by the header's status button and by the avatar menu's
/// "Status" item.
pub(crate) fn open_status_dialog(ctx: &egui::Context, current: &str) {
    menu::menu_close(ctx, "status");
    menu::menu_open(ctx, "status-dialog");
    ctx.data_mut(|d| { d.insert_temp(egui::Id::new("status-dialog-text"), current.to_string()); d.insert_temp(egui::Id::new("status-clear-after"), CLEAR_DEFAULT); });
}

/// True while the emoji picker belongs to the status dialog.
pub(crate) fn status_emoji_active(ctx: &egui::Context) -> bool { ctx.data(|d| d.get_temp::<bool>(egui::Id::new(STATUS_EMOJI))).unwrap_or(false) }

impl App {
    /// The custom status text while it is set and not expired.
    pub(crate) fn custom_status(&self, _ctx: &egui::Context) -> String {
        let cs = &self.settings["custom_status"];
        let until = cs["until"].as_i64().unwrap_or(0);
        if until > 0 && until <= now_ms() { return String::new(); }
        cs["text"].as_str().unwrap_or("").to_string()
    }

    fn set_custom_status(&mut self, text: &str, until: i64) {
        self.settings["custom_status"] = if text.is_empty() { serde_json::Value::Null } else { json!({"text": text, "until": until}) };
        if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); }
    }

    /// The text of the header button: "Set a Status", or "<presence> • <custom status>" once one is set.
    pub(crate) fn status_label(&self, ctx: &egui::Context) -> String {
        let cs = self.custom_status(ctx);
        if cs.is_empty() { return tr("Set a Status").to_string(); }
        let p = presence_text(&self.me.status);
        if p.is_empty() { cs } else { format!("{p} • {cs}") }
    }

    /// Sets my presence: the real request when signed in (the answer updates `me.status`), local in --sample.
    pub(crate) fn set_my_presence(&mut self, status: &str, ttl: i64) {
        let until = (ttl > 0 && status != "active").then(|| now_ms() + ttl * 1000);
        match &self.client {
            Some(c) => c.req(Req::SetPresence { status: status.to_string(), ttl }),
            None => {
                self.me.status = status.to_string();
                for k in [self.me.id.clone(), self.me.email.clone()] {
                    if k.is_empty() { continue; }
                    match until { Some(u) => { self.photos.presence_info.insert(k, (Some(u), String::new())); } None => { self.photos.presence_info.remove(&k); } }
                }
            }
        }
    }

    fn set_dnd(&mut self, d: Dur, ctx: &egui::Context) {
        let n = now_ms();
        let ttl = match d {
            Dur::Minutes(m) => m * 60,
            Dur::Today => status_time::ttl_secs(n, status_time::end_of_day_ms(n, offset(n))),
            Dur::NoEnd => cloud::NO_END_TTL,
            Dur::Custom => { super::app_dialogs::open_time_picker(ctx, "dnd"); return; }
        };
        self.set_my_presence("DoNotDisturb", ttl);
    }

    /// Draws the menus / dialog (every frame). `avatar` (the header avatar's rect) anchors the menu: only the avatar opens it
    /// (caret under the avatar). The status button opens the dialog directly (`open_status_dialog`).
    pub(crate) fn profile_menu(&mut self, ctx: &egui::Context, t: &Th, avatar: Rect, now: f64) {
        let dnd_on = self.me.status == "DoNotDisturb";
        let cs = self.custom_status(ctx);
        let photo = [self.me.email.clone(), self.me.id.clone()].iter().find_map(|k| self.photo(ctx, k));
        // The avatar menu: header card (cover, big avatar, name, e-mail), "Availability" / <presence> with chevron, "Status"
        // (opens the status dialog), "Edit profile", separator, "Settings", "Help" with chevron, separator, "Sign out",
        // "Exit %1" (%1 = this app's name). No icons. Menu width 318. Availability holds the presence entries, Help the
        // app-wide help entries.
        if self.cover.is_none() {
            if let Some(img) = self.cover_bytes.take().and_then(|b| crate::assets::photos::decode_net(&b).ok()) {
                let img = img.to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                self.cover = Some(ctx.load_texture("cover", ci, egui::TextureOptions::LINEAR));
            }
        }
        let card = menu::ProfileCard { name: self.me.name.clone(), email: self.me.email.clone(), photo, cover: self.cover.as_ref().map(|h| (h.id(), h.size())) };
        let presence = presence_text(&self.me.status);
        let avail = Item::new("avail", crate::tr!("ApplicationMenuController", "Availability").into_owned()).submenu();
        let items: Vec<Entry> = vec![
            Entry::Profile(card),
            if presence.is_empty() { avail } else { avail.sub(presence) }.entry(),
            Item::new("custom", crate::tr!("ApplicationMenuController", "Status").into_owned()).entry(),
            Item::new("edit", crate::tr!("ApplicationMenuController", "Edit profile").into_owned()).entry(),
            Entry::Sep,
            Item::new("settings", tr("Settings")).entry(),
            Item::new("help", tr("Help")).submenu().entry(),
            Entry::Sep,
            // Signed in: "Sign out". In the sample data there is no account: "Sign in" like on the settings page.
            if self.client.is_some() { Item::new("signout", tr("Sign out")).entry() } else { Item::new("signin", tr("Sign in")).entry() },
            Item::new("exit", crate::tr!("ApplicationMenuController", "Exit %1").replace("%1", crate::i18n::APP_NAME)).entry(), // %1 = the app
        ];
        if let Some(a) = menu::menu_impl(ctx, &mut self.icons, t, "status", avatar, Side::Below, Align::Left, true, &items, Some(318.0)) {
            if a != "avail" { menu::menu_close(ctx, "status-avail"); menu::menu_close(ctx, "status-dnd"); }
            if a != "help" { menu::menu_close(ctx, "status-help"); }
            match a.as_str() {
                "custom" => open_status_dialog(ctx, &cs),
                // The submenus keep the main menu open.
                "avail" => { menu::menu_open(ctx, "status"); menu::menu_close(ctx, "status-help"); menu::menu_open(ctx, "status-avail"); }
                "help" => { menu::menu_open(ctx, "status"); menu::menu_close(ctx, "status-avail"); menu::menu_close(ctx, "status-dnd"); menu::menu_open(ctx, "status-help"); }
                "edit" => menu::menu_open(ctx, super::app_dialogs::EDIT_PROFILE),
                "settings" => self.set_tab(Tab::Settings),
                "signout" => self.sc.sign_out_ask = true, // the sign-out confirmation (the shortcut module's dialog)
                "signin" => { self.need_login = true; self.login_error.clear(); }
                "exit" => self.act(Act::Quit, ctx),
                _ => {}
            }
        }
        // "Availability" submenu, to the right of its item.
        if menu::menu_is_open(ctx, "status-avail") {
            let anchor = menu::menu_item_rect(ctx, "status", "avail").unwrap_or(avatar);
            let mut sub: Vec<Entry> = vec![
                Item::new("active", tr("Available")).dot(hex(0x27A17A)).checked(self.me.status == "active").entry(),
                Item::new("inactive", tr("Away")).dot(hex(0x949494)).checked(self.me.status == "inactive").entry(),
                Item::new("dnd", tr("Do not disturb")).dot(hex(0xF55868)).checked(dnd_on).submenu().entry(),
            ];
            if dnd_on { sub.push(Item::new("dnd-clear", tr("Clear Do not disturb")).icon("cancel-regular").entry()); }
            if let Some(a) = menu::webex_menu(ctx, &mut self.icons, t, "status-avail", anchor, Side::Right, Align::Left, false, &sub) {
                if a != "dnd" { menu::menu_close(ctx, "status-dnd"); }
                match a.as_str() {
                    "dnd" => { menu::menu_open(ctx, "status"); menu::menu_open(ctx, "status-avail"); menu::menu_open(ctx, "status-dnd"); }
                    "active" => { self.set_my_presence("active", cloud::ACTIVE_TTL); menu::menu_close(ctx, "status"); }
                    "inactive" => { self.set_my_presence("inactive", cloud::NO_END_TTL); menu::menu_close(ctx, "status"); }
                                        "dnd-clear" => { self.set_my_presence("active", cloud::ACTIVE_TTL); self.toast(tr("Do not disturb status has been cleared."), now); menu::menu_close(ctx, "status"); }
                    _ => {}
                }
            }
        }
        // "Set Do not disturb for" submenu, to the right of its item.
        if menu::menu_is_open(ctx, "status-dnd") {
            let anchor = menu::menu_item_rect(ctx, "status-avail", "dnd").unwrap_or(avatar);
            let mut sub: Vec<Entry> = vec![Entry::Title(tr("Set Do not disturb for").into())];
            sub.extend(DND.iter().map(|(id, label, _)| Item::new(id, tr(label)).entry()));
            if let Some(a) = menu::webex_menu(ctx, &mut self.icons, t, "status-dnd", anchor, Side::Right, Align::Left, false, &sub) {
                if let Some((_, _, d)) = DND.iter().find(|x| x.0 == a) { self.set_dnd(*d, ctx); }
                menu::menu_close(ctx, "status");
            }
        }
        // "Help" submenu.
        if menu::menu_is_open(ctx, "status-help") {
            let anchor = menu::menu_item_rect(ctx, "status", "help").unwrap_or(avatar);
            let sub: Vec<Entry> = vec![
                Item::new("center", tr("Help Center")).entry(),
                Item::new("whatsnew", tr("What's New")).entry(),
                Item::new("shortcuts", tr("Keyboard shortcuts")).entry(),
                Item::new("health", tr("Health Checker")).entry(),
                Item::new("feedback", tr("Send Feedback")).entry(),
                Entry::Sep,
                Item::new("updates", tr("Check for updates")).entry(),
                Item::new("about", tr("About")).entry(),
            ];
            if let Some(a) = menu::webex_menu(ctx, &mut self.icons, t, "status-help", anchor, Side::Right, Align::Left, false, &sub) {
                menu::menu_close(ctx, "status");
                self.help_action(&a, ctx, now);
            }
        }
        // The status dialog.
        if menu::menu_is_open(ctx, "status-dialog") {
            let tid = egui::Id::new("status-dialog-text");
            let mut text: String = ctx.data(|d| d.get_temp(tid)).unwrap_or_default();
            let had = !cs.is_empty();
            let mut clear_idx: usize = ctx.data(|d| d.get_temp(egui::Id::new("status-clear-after"))).unwrap_or(CLEAR_DEFAULT);
            // Layout: "Status" label, the field (placeholder "What's your status?", emoji button bottom-left, counter "n/75"
            // bottom-right), "Clear status after" + combo (150 wide; disabled while the text is empty, the tooltip "First, write
            // your status ..." on hover), "Choose a status" + separator + the preset list (46 high rows, click = put the text in the field,
            // hovered row shows an x that removes the preset), buttons Save / Cancel ("Clear status" replaces Save while
            // the field still holds the current status).
            let presets_id = egui::Id::new("status-presets");
            let mut presets: Vec<String> = ctx.data(|d| d.get_temp(presets_id)).unwrap_or_else(|| PRESETS.iter().map(|s| crate::trq!(*s).into_owned()).collect());
            let show_clear = had && text == cs; // as of this frame's start
            let mut emoji_btn: Option<(Rect, bool)> = None;
            let icons = &mut self.icons;
            let mut body = |ui: &mut egui::Ui| {

                ui.label(RichText::new(crate::tr!("SetStatusDialog", "Status").into_owned()).size(16.0).color(t.text1));
                ui.add_space(6.0);
                let field = egui::Frame::default().stroke(Stroke::new(1.0_f32, alpha(t.text1, 0.9))).corner_radius(8.0).inner_margin(egui::Margin { left: 12, right: 12, top: 8, bottom: 6 });
                field.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut text).hint_text(crate::trq!("What's your status?").into_owned()).frame(false).font(FontId::proportional(16.0)).desired_width(ui.available_width() - 24.0).char_limit(MAX_CHARS));
                        // The field's clear "x", only while it holds text (clearing disables the combo again).
                        if !text.is_empty() {
                            let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                            icons.paint(ui, "cancel_16", Rect::from_center_size(r.center(), Vec2::splat(14.0)), t.text1);
                            if resp.clicked() { text.clear(); }
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        // Emoji button: opens the shared emoji picker (emoji_picker.rs), the pick is appended.
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                        icons.paint(ui, "emoticons_16_em", r, t.text1);
                        if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                        emoji_btn = Some((r, resp.clicked()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("{}/{}", text.chars().count(), MAX_CHARS)).size(16.0).color(t.text1));
                        });
                    });
                });
                ui.add_space(12.0);
                ui.label(RichText::new(crate::trq!("Clear status after").into_owned()).size(16.0).color(t.text1));
                ui.add_space(4.0);
                let opts: Vec<String> = CLEAR_AFTER.iter().map(|x| tr(x.0).to_string()).collect();
                // The duration combo is enabled only while there is text; while disabled, hovering shows its tooltip.
                let combo = ui.add_enabled_ui(!text.is_empty(), |ui| menu::dropdown(ui, icons, t, "status-clear-after", &mut clear_idx, &opts, 150.0)).inner;
                if text.is_empty() { combo.on_hover_text(crate::trq!("First, write your status or choose one").into_owned()); }
                ui.add_space(19.0);
                ui.label(RichText::new(crate::trq!("Choose a status").into_owned()).size(14.0).color(t.text2));
                ui.add_space(6.0);
                let sep = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover()).0;
                ui.painter().rect_filled(sep, 0.0, alpha(t.text1, 0.2));
                ui.add_space(10.0);
                let mut remove = None;
                for (i, p) in presets.iter().enumerate() {
                    let (row, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width() - 12.0, 46.0), Sense::click());
                    if resp.hovered() { ui.painter().rect_filled(row, 8.0, alpha(t.text1, 0.07)); }
                    super::welcome::emoji_line(ui, icons, pos2(row.left() + 12.0, row.center().y), p, FontId::proportional(16.0), t.text1);
                    let x = Rect::from_center_size(pos2(row.right() - 12.0 - 7.0, row.center().y), Vec2::splat(14.0));
                    if resp.hovered() || ui.rect_contains_pointer(x) {
                        icons.paint(ui, "cancel_16", x, t.text1);
                        if ui.interact(x, ui.id().with(("preset-x", i)), Sense::click()).clicked() { remove = Some(i); continue; }
                    }
                    if resp.clicked() { text = p.chars().take(MAX_CHARS).collect(); }
                }
                if let Some(i) = remove { presets.remove(i); }
            };
            let mut buttons = vec![];
            if show_clear { buttons.push(Button::secondary("clear", tr("Clear status"))); } else { buttons.push(Button::primary("save", tr("Save"))); }
            buttons.push(Button::secondary("cancel", tr("Cancel")));
            // The status dialog uses background-primary, not modal-primary (see menu::background_primary).
            menu::set_dialog_width(ctx, "status-dlg", 568.0); // 568 wide
            let picker_open = status_emoji_active(ctx) && ctx.data(|d| d.get_temp::<bool>(egui::Id::new("emoji-open"))).unwrap_or(false);
            let mut res = menu::webex_dialog_bg(ctx, t, "status-dlg", crate::i18n::APP_NAME, "", Some(&mut body), &buttons, Some(menu::background_primary(t))); // the dialog is a window titled with the app name
            // Esc while the picker is open closes only the picker.
            if picker_open && res.as_deref() == Some("cancel") && ctx.input(|i| i.key_pressed(Key::Escape)) { res = None; }
            // The emoji picker over the dialog.
            if let Some((r, clicked)) = emoji_btn {
                let open = ctx.data(|d| d.get_temp::<bool>(egui::Id::new("emoji-open"))).unwrap_or(false);
                if clicked {
                    if !(open && status_emoji_active(ctx)) { if !open { self.emoji_picker_toggle(ctx); } ctx.data_mut(|d| d.insert_temp(egui::Id::new(STATUS_EMOJI), true)); }
                    else { self.emoji_picker_toggle(ctx); ctx.data_mut(|d| d.insert_temp(egui::Id::new(STATUS_EMOJI), false)); }
                }
                if status_emoji_active(ctx) {
                    if !ctx.data(|d| d.get_temp::<bool>(egui::Id::new("emoji-open"))).unwrap_or(false) { ctx.data_mut(|d| d.insert_temp(egui::Id::new(STATUS_EMOJI), false)); }
                    else {
                        let mut picked: Option<String> = None;
                        egui::Area::new(egui::Id::new("status-emoji-host")).order(egui::Order::Debug).fixed_pos(r.min).interactable(false).show(ctx, |ui| {
                            self.emoji_picker_over(ui, r, egui::Order::Debug, &mut |e| picked = Some(e.to_string()));
                        });
                        if let Some(e) = picked {
                            if text.chars().count() + e.chars().count() <= MAX_CHARS { text.push_str(&e); }
                            self.emoji_picker_toggle(ctx);
                            ctx.data_mut(|d| d.insert_temp(egui::Id::new(STATUS_EMOJI), false));
                        }
                    }
                }
            }
            ctx.data_mut(|d| { d.insert_temp(tid, text.clone()); d.insert_temp(egui::Id::new("status-clear-after"), clear_idx); d.insert_temp(presets_id, presets.clone()); });
            if res.is_some() && status_emoji_active(ctx) {
                if ctx.data(|d| d.get_temp::<bool>(egui::Id::new("emoji-open"))).unwrap_or(false) { self.emoji_picker_toggle(ctx); }
                ctx.data_mut(|d| d.insert_temp(egui::Id::new(STATUS_EMOJI), false));
            }
            match res.as_deref() {
                Some("save") => {
                    let txt = text.trim().to_string();
                    let n = now_ms();
                    match CLEAR_AFTER.get(clear_idx).and_then(|o| o.1) {
                        _ if txt.is_empty() => self.set_custom_status("", 0),
                        Some(mins) => self.set_custom_status(&txt, n + mins * 60_000),
                        // "Custom": pick the end first; the text waits in the picker's state.
                        None => { ctx.data_mut(|d| d.insert_temp(egui::Id::new("status-pending-text"), txt)); super::app_dialogs::open_time_picker(ctx, "status"); }
                    }
                    menu::menu_close(ctx, "status-dialog");
                }
                Some("clear") => { self.set_custom_status("", 0); menu::menu_close(ctx, "status-dialog"); }
                Some(_) => menu::menu_close(ctx, "status-dialog"),
                None => {}
            }
        }
    }

    /// The end picked in the date/time picker ("dnd" or "status").
    pub(crate) fn time_picked(&mut self, what: &str, until_ms: i64, ctx: &egui::Context) {
        let n = now_ms();
        match what {
            "dnd" => self.set_my_presence("DoNotDisturb", status_time::ttl_secs(n, until_ms)),
            _ => {
                let txt: String = ctx.data_mut(|d| d.remove_temp(egui::Id::new("status-pending-text"))).unwrap_or_default();
                self.set_custom_status(&txt, until_ms);
            }
        }
    }

    /// One entry of the "Help" submenu (also used by the shortcut commands and the tray).
    pub(crate) fn help_action(&mut self, id: &str, ctx: &egui::Context, now: f64) {
        use super::app_dialogs as d;
        match id {
            "center" => system::open(HELP_CENTER_URL),
            // Webox has no release notes service.
            "whatsnew" => self.toast(tr("What's new is currently not available."), now),
            "shortcuts" => self.show_shortcuts = true,
            "health" => menu::menu_open(ctx, d::HEALTH),
            "feedback" => menu::menu_open(ctx, d::FEEDBACK),
            "updates" => menu::menu_open(ctx, d::UPDATES),
            "about" => menu::menu_open(ctx, "about"),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status button opens the dialog itself (not the profile menu), prefilled with the current status.
    #[test]
    fn status_button_opens_dialog_not_menu() {
        let ctx = egui::Context::default();
        menu::menu_open(&ctx, "status"); // e.g. the avatar menu was open
        open_status_dialog(&ctx, "Mittagspause");
        assert!(menu::menu_is_open(&ctx, "status-dialog"));
        assert!(!menu::menu_is_open(&ctx, "status"));
        assert_eq!(ctx.data(|d| d.get_temp::<String>(egui::Id::new("status-dialog-text"))).as_deref(), Some("Mittagspause"));
    }
}
