//! "Meetings": header buttons (round buttons camera-regular / enter-room-regular / calendar-add-regular with bold labels,
//! "Meeting starten" / "Beitreten" / "Meeting ansetzen", collapsible to pills), calendar integration prompt, day list, meeting
//! details.
//!
//! Strings from the German pack ("Connect your calendar...", "No meetings found", "Keine Meetings angesetzt", "Heute",
//! "Kalender auswählen", "Meeting-Link kopieren", "Organisator", "Wiederkehrendes Meeting", ...). Illustration: `meetings-320`.
//! Data: the calendar service's events (cloud/meetcall.rs) plus the locus "starting soon" list; "Meeting starten" = personal
//! room (ui/meetcall.rs), "Meeting ansetzen" = the in-app scheduler dialog.
#![allow(clippy::too_many_arguments)]
use crate::*;
use crate::cloud::meetcall::CalEvent;
use super::meetcall::{day_title, time_range};
use super::menu::{self, Item};
use super::page_kit::*;

/// Where calendars are connected (Google / Microsoft): the Webex settings page.
const CONNECT_CALENDAR_URL: &str = "https://settings.webex.com/";

impl App {
    pub(crate) fn join_meeting_link(&mut self, link: &str, now: f64) {
        if link.is_empty() { self.toast("Für dieses Meeting gibt es keinen Link", now); return; } // Webox's own wording
        platform::nmhost::record(&self.cfg, link);
        system::open(link);
    }

    pub(crate) fn meetings_list(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let mut st = self.pst(&ctx);
        // Ask the data layer once per session (it also reads both lists at sign-in).
        if !ctx.data(|d| d.get_temp::<bool>(egui::Id::new("meetings-requested"))).unwrap_or(false) {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("meetings-requested"), true));
            if let Some(c) = &self.client { c.req(Req::Meetings); c.req(Req::Calendar); }
        }
        self.list_header(ui, t, tr("Meetings"), None, None);
        ui.add_space(6.0);
        let (mut start, mut join, mut sched) = (false, false, false);
        let collapsed = st.meet_view == 1;
        if collapsed {
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 34.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                start = self.pill(ui, t, "Meeting starten", Pill::Join, None).clicked();
                join = self.pill(ui, t, "Beitreten", Pill::Join, None).clicked();
                sched = self.pill(ui, t, "Meeting ansetzen", Pill::Secondary, None).clicked();
            });
        } else {
            // Three round buttons with bold labels, centred, 3 x (list width / 3) up to 480 wide.
            let w = ui.available_width().min(480.0) / 3.0;
            let total = ui.available_width();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(((total - w * 3.0) / 2.0).max(0.0));
                start = self.round_labeled(ui, t, "camera-regular", "Meeting starten", t.green, hex(0x121212), w).on_hover_text("Starten eines Meetings in einem persönlichen Raum").clicked();
                join = self.round_labeled(ui, t, "enter-room-regular", "Beitreten", t.green, hex(0x121212), w).on_hover_text("Einem Meeting beitreten").clicked();
                sched = self.round_labeled(ui, t, "calendar-add-regular", "Meeting ansetzen", t.white_btn, t.white_btn_text, w).clicked();
            });
        }
        // Expand / collapse (arrow-up-bold / arrow-down-bold).
        {
            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width() - 6.0, 22.0), Sense::click());
            let label = if collapsed { "Erweitern" } else { "Reduzieren" };
            let c = r.center();
            let tw = ui.fonts(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(12.5), t.text2).size().x);
            if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            ui.painter().text(pos2(c.x - 8.0, c.y), egui::Align2::CENTER_CENTER, label, FontId::proportional(12.5), if resp.hovered() { t.text1 } else { t.text2 });
            self.icons.paint(ui, if collapsed { "arrow-down-bold" } else { "arrow-up-bold" }, Rect::from_center_size(pos2(c.x + tw / 2.0 + 2.0, c.y), Vec2::splat(11.0)), t.text2);
            if resp.clicked() { st.meet_view = if collapsed { 0 } else { 1 }; }
        }
        if join { st.join_dialog = Some(String::new()); }
        ui.add_space(6.0);

        ui.spacing_mut().scroll = egui::style::ScrollStyle { bar_width: 10.0, bar_inner_margin: 2.0, bar_outer_margin: 0.0, ..egui::style::ScrollStyle::solid() };
        let rows = self.meeting_rows();
        let nowms = crate::i18n::now_ms();
        let mut menu_req: Option<(String, Rect)> = None;
        let mut join_link: Option<String> = None;
        let mut cal_dialog = false;
        let calendar_connected = rows.iter().any(|e| !e.id.starts_with("locus-"));
        crate::ui::widgets::vscroll().auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            // Calendar integration prompt ("Connect your calendar to see all your meetings right here.").
            if !st.cal_prompt_closed && !calendar_connected {
                let w = ui.available_width() - 10.0;
                let g = ui.fonts(|f| f.layout("Verbinden Sie Ihren Kalender, um alle Ihre Meetings hier anzuzeigen.".into(), FontId::proportional(13.5), t.text2, w - 32.0));
                let h = 16.0 + 22.0 + 4.0 + g.size().y + 12.0 + 32.0 + 16.0;
                let (r, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
                let r = r.translate(Vec2::new(8.0, 0.0));
                ui.painter().rect_filled(r, 12.0, t.ink_a(0.07));
                ui.painter().text(pos2(r.left() + 16.0, r.top() + 27.0), egui::Align2::LEFT_CENTER, "Kalender verbinden", semibold(15.0), t.text1);
                let xr = Rect::from_center_size(pos2(r.right() - 20.0, r.top() + 27.0), Vec2::splat(24.0));
                let xresp = ui.interact(xr, ui.id().with("cal-x"), Sense::click());
                if xresp.hovered() { ui.painter().circle_filled(xr.center(), 12.0, t.ink_a(0.12)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                let (c, d) = (xr.center(), 4.5);
                ui.painter().line_segment([c - Vec2::splat(d), c + Vec2::splat(d)], Stroke::new(1.3, t.text1));
                ui.painter().line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], Stroke::new(1.3, t.text1));
                if xresp.on_hover_text("Schließen").clicked() { st.cal_prompt_closed = true; }
                ui.painter().galley(pos2(r.left() + 16.0, r.top() + 42.0), g.clone(), t.text2);
                let br = Rect::from_min_size(pos2(r.left() + 16.0, r.top() + 42.0 + g.size().y + 12.0), Vec2::new(150.0, 32.0));
                let mut b = ui.new_child(egui::UiBuilder::new().max_rect(br));
                if self.pill(&mut b, t, "Kalender verbinden", Pill::Primary, Some("calendar-month-regular")).clicked() { cal_dialog = true; }
                ui.add_space(10.0);
            }
            if rows.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| { ui.label(RichText::new("Keine Meetings gefunden").font(semibold(15.0)).color(t.text1)); });
            } else {
                let mut last_day = String::new();
                for (i, m) in rows.iter().enumerate() {
                    let day = if m.start_ms > 0 { day_title(m.start_ms, nowms) } else { tr("Today").to_string() };
                    if day != last_day {
                        let (hr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
                        ui.painter().text(hr.left_center() + Vec2::new(24.0, 0.0), egui::Align2::LEFT_CENTER, &day, FontId::proportional(14.0), t.text2);
                        last_day = day;
                    }
                    let sub = if m.start_ms > 0 { time_range(m.start_ms, m.end_ms) } else { String::new() };
                    let live = m.start_ms <= nowms && (m.end_ms == 0 || nowms < m.end_ms);
                    let mut spec = RowSpec::new(format!("meet-{}", m.id), &m.title);
                    spec.sub = &sub; spec.sub_icon = Some(if m.recurring { "recurring-regular" } else { "calendar-today-regular" }); spec.selected = st.meet_sel == Some(i);
                    spec.icon = Some("meetings-filled"); spec.bold = live;
                    if !m.link.is_empty() { spec.action = Some(("enter-room-regular", "Beitreten")); }
                    let out = self.page_row(ui, t, &spec);
                    if out.clicked { st.meet_sel = Some(i); }
                    if out.double || out.action { join_link = Some(m.link.clone()); }
                    if let Some(r) = out.menu { st.meet_sel = Some(i); menu_req = Some((format!("{i}"), r)); }
                }
            }
            let hist = platform::nmhost::history(&self.cfg);
            if !hist.is_empty() {
                ui.add_space(10.0);
                let (hr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
                ui.painter().text(hr.left_center() + Vec2::new(24.0, 0.0), egui::Align2::LEFT_CENTER, "Zuletzt beigetreten", FontId::proportional(14.0), t.text2); // Webox's own wording
                for (site, url) in hist.iter().take(8) {
                    let mut spec = RowSpec::new(format!("hist-{url}"), site);
                    spec.sub = url; spec.icon = Some("history-regular"); spec.action = Some(("enter-room-regular", "Beitreten"));
                    let out = self.page_row(ui, t, &spec);
                    if out.clicked || out.action { join_link = Some(url.clone()); }
                }
            }
        });
        if cal_dialog { st.cal_dialog = true; }
        self.set_pst(&ctx, st);
        if start { self.start_personal_meeting(now); }
        if sched { self.open_scheduler("", &[]); }
        if let Some(l) = join_link { self.join_meeting_link(&l, now); }
        if let Some((k, r)) = menu_req { self.open_page_menu(&ctx, "meeting-row", &k, r); }
    }

    fn selected_meeting(&self, ctx: &egui::Context) -> Option<CalEvent> { self.pst(ctx).meet_sel.and_then(|i| self.meeting_rows().get(i).cloned()) }

    pub(crate) fn meetings_detail(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let Some(m) = self.selected_meeting(&ctx) else {
            let empty = self.meeting_rows().is_empty();
            let (title, body) = if empty { ("Keine Meetings angesetzt", "Verbinden Sie Ihren Kalender, um alle Ihre Meetings hier anzuzeigen.") }
                else { ("Wählen Sie ein Meeting aus", "") }; // second title: Webox's own wording
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
            // meetings-320-cobalt-lime is the colour art for every theme (the black line variant disappears on dark themes).
            self.empty_state(&mut c, t, "meetings-320-cobalt-lime", 200.0, title, body, None);
            return;
        };
        let nowms = crate::i18n::now_ms();
        let area = ui.max_rect().shrink2(Vec2::new(40.0, 36.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        c.label(RichText::new(&m.title).font(semibold(22.0)).color(t.text1));
        c.add_space(4.0);
        fn line(ui: &mut egui::Ui, icons: &mut icons::Icons, icon: &'static str, text: String, col: Color32) {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                icons.paint(ui, icon, r, col);
                ui.label(RichText::new(text).size(14.5).color(col));
            });
        }
        let col = t.text2;
        let line = |ui: &mut egui::Ui, icons: &mut icons::Icons, icon: &'static str, text: String| line(ui, icons, icon, text, col);
        if m.start_ms > 0 { line(&mut c, &mut self.icons, "calendar-today-regular", format!("{}, {}", day_title(m.start_ms, nowms), time_range(m.start_ms, m.end_ms))); }
        if m.recurring { line(&mut c, &mut self.icons, "recurring-regular", "Wiederkehrendes Meeting".into()); }
        if !m.organizer.is_empty() { line(&mut c, &mut self.icons, "user-regular", format!("Organisator: {}", m.organizer)); }
        if !m.location.is_empty() { line(&mut c, &mut self.icons, "location-regular", m.location.clone()); }
        if m.attendees > 0 { line(&mut c, &mut self.icons, "participant-list-regular", format!("Eingeladene Teilnehmer ({})", m.attendees)); }
        c.add_space(16.0);
        c.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if !m.link.is_empty() {
                if self.pill(ui, t, "Beitreten", Pill::Join, Some("enter-room-filled")).clicked() { self.join_meeting_link(&m.link, now); }
                if self.pill(ui, t, "Meeting-Link kopieren", Pill::Secondary, Some("copy-regular")).clicked() { ui.ctx().copy_text(m.link.clone()); self.toast("Kopiert", now); }
            }
        });
        if !m.link.is_empty() {
            c.add_space(18.0);
            c.label(RichText::new("Meeting-Link").font(semibold(14.0)).color(t.text1));
            if c.add(egui::Label::new(RichText::new(&m.link).size(14.0).color(t.link)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { self.join_meeting_link(&m.link, now); }
        }
    }

    pub(crate) fn meetings_menu(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let items = vec![Item::new("join", "Beitreten").icon("enter-room-regular").entry(), Item::new("copy", "Meeting-Link kopieren").icon("copy-regular").entry()];
        if let Some((key, pick)) = self.page_menu(ctx, t, "meeting-row", &items) {
            let Some(m) = key.parse::<usize>().ok().and_then(|i| self.meeting_rows().get(i).cloned()) else { return };
            match pick.as_str() {
                "join" => self.join_meeting_link(&m.link, now),
                _ if m.link.is_empty() => self.join_meeting_link("", now),
                _ => { ctx.copy_text(m.link); self.toast("Kopiert", now); }
            }
        }
        let mut st = self.pst(ctx);
        // "Einem Meeting beitreten".
        if let Some(mut input) = st.join_dialog.clone() {
            let mut body = |ui: &mut egui::Ui| { ui.add(egui::TextEdit::singleline(&mut input).hint_text("Meeting-Link, Nummer oder Videoadresse").desired_width(f32::INFINITY)); };
            let r = menu::webex_dialog(ctx, t, "join-meeting", "Einem Meeting beitreten", "Geben Sie eine Meeting-Kennnummer, eine Videoadresse oder einen Meeting-Link ein oder suchen Sie nach einem Namen, um dem persönlichen Raum dieser Person beizutreten.",
                Some(&mut body), &[menu::Button::secondary("cancel", "Abbrechen"), menu::Button::primary("join", "Beitreten")]);
            match r.as_deref() {
                Some("join") => {
                    st.join_dialog = None; self.set_pst(ctx, st);
                    let l = input.trim().to_string();
                    match join_target(&l) {
                        Some(url) => self.join_meeting_link(&url, now),
                        None if l.is_empty() => {}
                        None => self.toast("Bitte geben Sie einen Meeting-Link ein", now), // Webox's own wording
                    }
                    return;
                }
                Some(_) => { st.join_dialog = None; }
                None => { st.join_dialog = Some(input); }
            }
            self.set_pst(ctx, st.clone());
        }
        // "Kalender auswählen": Google / Microsoft; here the Webex settings page opens to connect a calendar.
        if st.cal_dialog {
            let mut body = |ui: &mut egui::Ui| {
                // Microsoft 365 first, then Google.
                for (icon, name) in [("calendar-month-regular", "Microsoft 365"), ("calendar-month-regular", "Google")] {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
                    if resp.hovered() { ui.painter().rect_filled(r, 8.0, t.ink_a(0.1)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    self.icons.paint(ui, icon, Rect::from_center_size(r.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(20.0)), t.text1);
                    ui.painter().text(r.left_center() + Vec2::new(40.0, 0.0), egui::Align2::LEFT_CENTER, name, FontId::proportional(15.0), t.text1);
                    if resp.clicked() { ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("cal-pick"), true)); }
                }
            };
            let r = menu::webex_dialog(ctx, t, "choose-cal", "Kalender auswählen", "Wählen Sie einen Kalender, der mit Webex verwendet werden soll.", Some(&mut body), &[menu::Button::secondary("cancel", "Abbrechen")]);
            let picked = ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("cal-pick"))).unwrap_or(false);
            if picked { system::open(CONNECT_CALENDAR_URL); }
            if r.is_some() || picked { let mut s = self.pst(ctx); s.cal_dialog = false; self.set_pst(ctx, s); }
        }
    }
}

/// What "Beitreten" opens for the typed text: a link as is, a Webex video address / personal room `name@site.webex.com` as
/// `https://site.webex.com/meet/name` (the personal room link form). Meeting numbers need the user's site: not resolvable (None).
pub(crate) fn join_target(s: &str) -> Option<String> {
    let s = s.trim();
    if s.starts_with("https://") || s.starts_with("http://") { return Some(s.to_string()); }
    let (name, host) = s.split_once('@')?;
    (!name.is_empty() && host.ends_with(".webex.com") && !name.contains(char::is_whitespace)).then(|| format!("https://{host}/meet/{name}"))
}

#[cfg(test)]
mod tests {
    use super::join_target;
    #[test]
    fn join_targets() {
        assert_eq!(join_target(" https://x.webex.com/meet/a "), Some("https://x.webex.com/meet/a".into()));
        assert_eq!(join_target("someone@go.webex.com"), Some("https://go.webex.com/meet/someone".into()));
        assert_eq!(join_target("someone@example.com"), None);
        assert_eq!(join_target("2734 123 4567"), None);
    }
}
