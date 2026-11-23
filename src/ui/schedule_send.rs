//! "Später senden" (the clock next to the send button): schedule the composer's message, the list of scheduled
//! messages above the composer, send now / reschedule / discard.
//!
//! Strings (German pack): "Später senden", "Morgen um 9:00 Uhr", "Benutzerdefiniert", "Datum", "Zeit", "Ansetzen",
//! "Abbrechen", "Versuchen Sie es frühestens in 5 Minuten noch einmal.", "Sie haben den Versand einer Nachricht heute|morgen
//! um {{0}} geplant.", "Sie haben den Versand einer Nachricht {{0}} um {{1}} geplant.", "Sie haben geplant, dass {{0}}
//! Nachrichten gesendet werden.", "Geplant für morgen um {{0}}", "Alle geplanten Nachrichten anzeigen", "Geplante Nachrichten
//! erweitern|reduzieren", "Jetzt senden", "Nachricht neu ansetzen", "Geplante Nachricht verwerfen?", "Verwerfen".
//! Layout: radius 8, icon schedule-send-bold 16 at 8, spacer 12, primary title, accent link; message rows inset 40 / 12,
//! radius 8, secondary header "<sender>  <time>"; colours message-scheduled-*.
//!
//! Delivery: the message is kept locally (settings "scheduled_messages") and posted with the normal post request when it is
//! due while Webox runs; one that fell due while Webox was closed goes out right after the next sign-in. (Webex's own
//! server-side scheduling through the drafts service is not used.)
use super::menu;
use crate::*;
use egui::Id;

/// One scheduled message.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Scheduled { pub id: String, pub space: String, pub text: String, pub markdown: bool, pub mentions: Vec<crate::richtext::MentionRef>, pub due_ms: i64 }

/// Minimum distance into the future ("Choose a time more than 5 minutes in the future and try again.").
pub(crate) const MIN_AHEAD_MS: i64 = 5 * 60 * 1000;

pub(crate) fn now_ms() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }

// Calendar arithmetic (proleptic Gregorian, days since 1970-01-01).
pub(crate) fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
pub(crate) fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32, m, d)
}

/// Local wall clock (y, m, d, h, min) of a UTC instant (Windows: the system time zone incl. daylight saving; elsewhere UTC).
pub(crate) fn local_parts(ms: i64) -> (i32, u32, u32, u32, u32) {
    let secs = ms.div_euclid(1000);
    let (y, mo, d) = civil_from_days(secs.div_euclid(86_400));
    let (h, mi) = ((secs.rem_euclid(86_400) / 3600) as u32, (secs.rem_euclid(3600) / 60) as u32);
    to_local(y, mo, d, h, mi)
}
#[cfg(windows)]
fn st(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> windows::Win32::Foundation::SYSTEMTIME {
    windows::Win32::Foundation::SYSTEMTIME { wYear: y as u16, wMonth: mo as u16, wDayOfWeek: 0, wDay: d as u16, wHour: h as u16, wMinute: mi as u16, wSecond: 0, wMilliseconds: 0 }
}
#[cfg(windows)]
fn to_local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> (i32, u32, u32, u32, u32) {
    let mut l = windows::Win32::Foundation::SYSTEMTIME::default();
    if unsafe { windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime(None, &st(y, mo, d, h, mi), &mut l) }.is_ok() { (l.wYear as i32, l.wMonth as u32, l.wDay as u32, l.wHour as u32, l.wMinute as u32) } else { (y, mo, d, h, mi) }
}
#[cfg(not(windows))]
fn to_local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> (i32, u32, u32, u32, u32) { (y, mo, d, h, mi) }
/// The UTC instant of a local wall-clock time.
pub(crate) fn local_to_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    #[cfg(windows)]
    let (y, mo, d, h, mi) = {
        let mut u = windows::Win32::Foundation::SYSTEMTIME::default();
        if unsafe { windows::Win32::System::Time::TzSpecificLocalTimeToSystemTime(None, &st(y, mo, d, h, mi), &mut u) }.is_ok() { (u.wYear as i32, u.wMonth as u32, u.wDay as u32, u.wHour as u32, u.wMinute as u32) } else { (y, mo, d, h, mi) }
    };
    (days_from_civil(y, mo, d) * 86_400 + h as i64 * 3600 + mi as i64 * 60) * 1000
}

/// "Morgen um 9:00 Uhr": tomorrow 09:00 local time.
pub(crate) fn tomorrow_nine(now: i64) -> i64 {
    let (y, m, d, _, _) = local_parts(now);
    let (y2, m2, d2) = civil_from_days(days_from_civil(y, m, d) + 1);
    local_to_ms(y2, m2, d2, 9, 0)
}

/// The confirmation after scheduling (pack strings with "heute" / "morgen" / a date).
pub(crate) fn scheduled_text(now: i64, due: i64) -> String {
    let (y, m, d, _, _) = local_parts(now);
    let (yy, mm, dd, h, mi) = local_parts(due);
    let diff = days_from_civil(yy, mm, dd) - days_from_civil(y, m, d);
    let t = format!("{h:02}:{mi:02}");
    match diff {
        0 => format!("Sie haben den Versand einer Nachricht heute um {t} geplant."),
        1 => format!("Sie haben den Versand einer Nachricht morgen um {t} geplant."),
        _ => format!("Sie haben den Versand einer Nachricht {dd:02}.{mm:02}.{yy} um {t} geplant."),
    }
}
/// The time label of a scheduled message row.
fn row_time(now: i64, due: i64) -> String {
    let (y, m, d, _, _) = local_parts(now);
    let (yy, mm, dd, h, mi) = local_parts(due);
    if days_from_civil(yy, mm, dd) - days_from_civil(y, m, d) == 1 { format!("Geplant für morgen um {h:02}:{mi:02}") } else { format!("{dd:02}.{mm:02}.{yy}, {h:02}:{mi:02}") }
}

fn to_json(s: &Scheduled) -> serde_json::Value {
    json!({"id": s.id, "space": s.space, "text": s.text, "markdown": s.markdown, "due": s.due_ms,
        "mentions": s.mentions.iter().map(|m| json!({"id": m.id, "name": m.name, "all": m.all})).collect::<Vec<_>>()})
}
fn from_json(v: &serde_json::Value) -> Option<Scheduled> {
    let g = |k: &str| v[k].as_str().map(String::from);
    Some(Scheduled { id: g("id")?, space: g("space")?, text: g("text")?, markdown: v["markdown"].as_bool().unwrap_or(false), due_ms: v["due"].as_i64()?,
        mentions: v["mentions"].as_array().into_iter().flatten().filter_map(|m| Some(crate::richtext::MentionRef { id: m["id"].as_str()?.into(), name: m["name"].as_str()?.into(), all: m["all"].as_bool().unwrap_or(false) })).collect() })
}

/// Custom-time dialog state: (which scheduled message is rescheduled, None = the composer's), date, hour, minute.
#[derive(Clone)]
struct Custom { target: Option<String>, y: i32, m: u32, d: u32, h: u32, mi: u32 }
fn custom_id() -> Id { Id::new("sched-custom") }

impl App {
    pub(crate) fn msgx_load(&mut self) {
        self.msgx.sched = self.settings["scheduled_messages"].as_array().into_iter().flatten().filter_map(from_json).collect();
    }
    fn sched_save(&mut self) {
        if self.shot.is_some() { return; }
        self.settings["scheduled_messages"] = json!(self.msgx.sched.iter().map(to_json).collect::<Vec<_>>());
        system::save_settings(&self.cfg, &self.settings);
    }

    /// Keeps the composer's message for later.
    pub(crate) fn sched_add(&mut self, space: &str, text: String, mentions: Vec<crate::richtext::MentionRef>, due: i64, now: f64) {
        let markdown = self.composer_is_markdown();
        self.msgx.sched.push(Scheduled { id: crate::jose::uuid(), space: space.into(), text, markdown, mentions, due_ms: due });
        self.msgx.sched.sort_by_key(|s| s.due_ms);
        self.sched_save();
        self.toast(scheduled_text(now_ms(), due), now);
    }

    /// Posts a scheduled message now (normal post request; offline the local copy).
    fn sched_post(&mut self, s: Scheduled) {
        match &self.client {
            Some(c) => c.req(Req::Post { space: s.space, text: s.text, markdown: s.markdown, mentions: s.mentions, kind: cloud::PostKind::New }),
            None => {
                let list = self.messages.entry(s.space.clone()).or_default();
                let id = format!("{}-local{}", s.space, list.len());
                let html = if s.mentions.is_empty() && !s.markdown { String::new() } else { crate::richtext::render(&s.text, s.markdown, &s.mentions).html };
                list.push(Message { id, author: self.me.name.clone(), email: self.me.email.clone(), text: s.text, html, time: "now".into(), date: system::today(), mine: true, ..Default::default() });
            }
        }
    }

    /// Due messages go out (signed in: once the space list is there).
    pub(crate) fn sched_tick(&mut self, ctx: &egui::Context) {
        if self.msgx.sched.is_empty() || (self.client.is_some() && self.spaces.is_empty()) || self.shot.is_some() { return; }
        let now = now_ms();
        let (due, keep): (Vec<Scheduled>, Vec<Scheduled>) = std::mem::take(&mut self.msgx.sched).into_iter().partition(|s| s.due_ms <= now);
        self.msgx.sched = keep;
        if !due.is_empty() { for s in due { self.sched_post(s); } self.sched_save(); }
        if let Some(next) = self.msgx.sched.first() { ctx.request_repaint_after(std::time::Duration::from_millis((next.due_ms - now).clamp(500, 60_000) as u64)); }
    }

    /// The clock's menu ("Später senden") and the custom-time dialog. Returns the chosen time for the composer's message.
    pub(crate) fn sched_menu(&mut self, ctx: &egui::Context, t: &Th, anchor: Rect, clicked: bool) -> Option<i64> {
        if clicked { menu::menu_toggle(ctx, "sched-menu"); }
        let mut out = None;
        if menu::menu_is_open(ctx, "sched-menu") {
            let items = vec![menu::Entry::Title("Später senden".into()), menu::Item::new("tomorrow", "Morgen um 9:00 Uhr").entry(), menu::Item::new("custom", "Benutzerdefiniert").entry()];
            match menu::webex_menu(ctx, &mut self.icons, t, "sched-menu", anchor, menu::Side::Above, menu::Align::Right, true, &items).as_deref() {
                Some("tomorrow") => out = Some(tomorrow_nine(now_ms())),
                Some("custom") => self.sched_custom_open(ctx, None, now_ms() + 60 * 60 * 1000),
                _ => {}
            }
        }
        if let Some(due) = ctx.data_mut(|d| d.remove_temp::<i64>(Id::new("sched-composer-due"))) { out = Some(due); }
        out
    }

    /// The dialogs (custom time, discard), drawn once per frame by the message list. A time picked for the composer's message is
    /// handed to `sched_menu` through the context.
    pub(crate) fn sched_dialogs(&mut self, ctx: &egui::Context, t: &Th) {
        match self.sched_custom_dialog(ctx, t) {
            Some((Some(id), due)) => {
                if let Some(s) = self.msgx.sched.iter_mut().find(|s| s.id == id) { s.due_ms = due; }
                self.msgx.sched.sort_by_key(|s| s.due_ms);
                self.sched_save();
                let now = ctx.input(|i| i.time);
                self.toast(scheduled_text(now_ms(), due), now);
            }
            Some((None, due)) => ctx.data_mut(|d| d.insert_temp(Id::new("sched-composer-due"), due)),
            None => {}
        }
        self.sched_discard_dialog(ctx, t);
    }

    /// The custom-time dialog for the composer's message, starting an hour from now (screenshot hook).
    pub(crate) fn sched_custom_open_pub(&mut self, ctx: &egui::Context) { self.sched_custom_open(ctx, None, now_ms() + 3_600_000); }

    fn sched_custom_open(&mut self, ctx: &egui::Context, target: Option<String>, at: i64) {
        let (y, m, d, h, mi) = local_parts(at);
        let mi = (mi / 5) * 5;
        ctx.data_mut(|dd| dd.insert_temp(custom_id(), Custom { target, y, m, d, h, mi }));
    }

    /// "Benutzerdefiniert": date (day back / forward) and time; "Ansetzen" checks the 5-minute rule. Returns (target, time).
    fn sched_custom_dialog(&mut self, ctx: &egui::Context, t: &Th) -> Option<(Option<String>, i64)> {
        let mut c: Custom = ctx.data(|d| d.get_temp(custom_id()))?;
        let fg = t.text1;
        let mut content = |ui: &mut egui::Ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
            ui.horizontal(|ui| {
                ui.add_sized([60.0, 24.0], egui::Label::new(RichText::new("Datum").color(fg).size(14.0)));
                if ui.button("◀").on_hover_text("Vorheriger").clicked() { let (y, m, d) = civil_from_days(days_from_civil(c.y, c.m, c.d) - 1); (c.y, c.m, c.d) = (y, m, d); }
                ui.label(RichText::new(format!("{:02}.{:02}.{}", c.d, c.m, c.y)).color(fg).size(14.0));
                if ui.button("▶").on_hover_text("Weiter").clicked() { let (y, m, d) = civil_from_days(days_from_civil(c.y, c.m, c.d) + 1); (c.y, c.m, c.d) = (y, m, d); }
            });
            ui.horizontal(|ui| {
                ui.add_sized([60.0, 24.0], egui::Label::new(RichText::new("Zeit").color(fg).size(14.0)));
                ui.add(egui::DragValue::new(&mut c.h).range(0..=23).custom_formatter(|v, _| format!("{:02}", v as u32)));
                ui.label(RichText::new(":").color(fg));
                ui.add(egui::DragValue::new(&mut c.mi).range(0..=55).speed(0.3).custom_formatter(|v, _| format!("{:02}", v as u32)));
            });
        };
        let res = menu::webex_dialog(ctx, t, "sched-custom", "Später senden", "", Some(&mut content), &[menu::Button::secondary("cancel", "Abbrechen"), menu::Button::primary("set", "Ansetzen")]);
        ctx.data_mut(|d| d.insert_temp(custom_id(), c.clone()));
        match res.as_deref() {
            Some("set") => {
                let due = local_to_ms(c.y, c.m, c.d, c.h, c.mi);
                if due < now_ms() + MIN_AHEAD_MS {
                    let now = ctx.input(|i| i.time);
                    self.toast("Versuchen Sie es frühestens in 5 Minuten noch einmal.", now);
                    None
                } else { ctx.data_mut(|d| d.remove::<Custom>(custom_id())); Some((c.target, due)) }
            }
            Some(_) => { ctx.data_mut(|d| d.remove::<Custom>(custom_id())); None }
            None => None,
        }
    }

    /// The scheduled messages of `space` at the end of its message list: the header (+ the message rows when expanded).
    pub(crate) fn sched_list(&mut self, ui: &mut egui::Ui, t: &Th, space: &str) {
        let mine: Vec<Scheduled> = self.msgx.sched.iter().filter(|s| s.space == space).cloned().collect();
        let ctx = ui.ctx().clone();
        if mine.is_empty() { return; }
        let light = menu::light(t);
        let (bg, border, text, link) = if light { (hex(0xF7F7F7), alpha(Color32::BLACK, 0.2), alpha(Color32::BLACK, 0.95), hex(0x0353A8)) } else { (hex(0x1A1A1A), alpha(Color32::WHITE, 0.2), alpha(Color32::WHITE, 0.95), hex(0x64B4FA)) };
        let (row_bg, row_hover, head_c) = if light { (alpha(Color32::BLACK, 0.11), alpha(Color32::BLACK, 0.2), alpha(Color32::BLACK, 0.7)) } else { (alpha(Color32::WHITE, 0.11), alpha(Color32::WHITE, 0.2), alpha(Color32::WHITE, 0.7)) };
        let now = now_ms();
        let w = ui.available_width();
        ui.add_space(6.0);
        let title = if mine.len() == 1 { scheduled_text(now, mine[0].due_ms) } else { format!("Sie haben geplant, dass {} Nachrichten gesendet werden.", mine.len()) };
        let link_t = if self.msgx.sched_expanded { "" } else { "Alle geplanten Nachrichten anzeigen" };
        let head_h = 30.0;
        let rows_h: f32 = if self.msgx.sched_expanded { mine.len() as f32 * 64.0 + 8.0 } else { 0.0 };
        let (outer, _) = ui.allocate_exact_size(Vec2::new(w, head_h + rows_h), Sense::hover());
        let frame = Rect::from_min_max(pos2(outer.left() + 16.0, outer.top()), pos2(outer.right() - 21.0, outer.bottom()));
        ui.painter().rect_filled(frame, 8.0, bg);
        ui.painter().rect_stroke(frame, 8.0, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
        let cy = frame.top() + head_h / 2.0;
        self.icons.paint(ui, "schedule-send-bold", Rect::from_center_size(pos2(frame.left() + 8.0 + 8.0, cy), Vec2::splat(16.0)), text);
        let tg = ui.painter().layout_no_wrap(title, FontId::proportional(16.0), text);
        let tx = frame.left() + 8.0 + 16.0 + 12.0;
        let tw = tg.size().x.min(frame.width() - 36.0 - 260.0);
        ui.painter().with_clip_rect(Rect::from_min_size(pos2(tx, frame.top()), Vec2::new(tw, head_h))).galley(pos2(tx, cy - tg.size().y / 2.0), tg, text);
        let mut toggle = false;
        if !link_t.is_empty() {
            let lg = ui.painter().layout_no_wrap(link_t.into(), FontId::proportional(16.0), link);
            let lr = Rect::from_min_size(pos2(tx + tw + 6.0, cy - lg.size().y / 2.0), lg.size());
            let lresp = ui.interact(lr, Id::new(("sched-link", space)), Sense::click());
            if lresp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            ui.painter().galley(lr.min, lg, link);
            if lresp.clicked() { toggle = true; }
        }
        let ar = Rect::from_center_size(pos2(frame.right() - 8.0 - 12.0, cy), Vec2::splat(24.0));
        let aresp = ui.interact(ar, Id::new(("sched-arrow", space)), Sense::click());
        if aresp.hovered() { ui.painter().circle_filled(ar.center(), 12.0, t.hover); }
        self.icons.paint(ui, if self.msgx.sched_expanded { "arrow-up-regular" } else { "arrow-down-regular" }, Rect::from_center_size(ar.center(), Vec2::splat(14.0)), text);
        if aresp.on_hover_text(if self.msgx.sched_expanded { "Geplante Nachrichten reduzieren" } else { "Geplante Nachrichten erweitern" }).clicked() { toggle = true; }
        if toggle { self.msgx.sched_expanded = !self.msgx.sched_expanded; }
        if !self.msgx.sched_expanded { return; }
        let mut act: Option<(&'static str, String)> = None;
        for (i, s) in mine.iter().enumerate() {
            let r = Rect::from_min_size(pos2(frame.left() + 40.0, frame.top() + head_h + 4.0 + i as f32 * 64.0), Vec2::new(frame.width() - 52.0, 58.0));
            let resp = ui.interact(r, Id::new(("sched-row", &s.id)), Sense::hover());
            ui.painter().rect_filled(r, 8.0, if resp.hovered() { row_hover } else { row_bg });
            super::chrome::fit_text(ui.painter(), r.left() + 12.0, r.top() + 16.0, &format!("{}  {}", tr("You"), row_time(now, s.due_ms)), FontId::proportional(14.0), head_c, r.width() - 24.0);
            super::chrome::fit_text(ui.painter(), r.left() + 12.0, r.top() + 39.0, s.text.lines().next().unwrap_or(""), FontId::proportional(16.0), text, r.width() - 24.0);
            // Actions toolbar on hover: send now, reschedule, discard.
            let hot = resp.hovered() || ctx.data(|d| d.get_temp::<String>(Id::new("sched-hot"))).as_deref() == Some(s.id.as_str());
            if hot {
                let bar = Rect::from_min_size(pos2(r.right() - 24.0 - 3.0 * 32.0, r.top() - 12.0), Vec2::new(3.0 * 32.0, 24.0));
                ui.painter().rect_filled(bar, 12.0, if light { Color32::WHITE } else { hex(0x121212) });
                ui.painter().rect_stroke(bar, 12.0, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
                let mut any = false;
                for (k, (icon, tip, a)) in [("send-regular", "Jetzt senden", "now"), ("schedule-send-regular", "Nachricht neu ansetzen", "re"), ("delete-regular", "Verwerfen", "del")].into_iter().enumerate() {
                    let br = Rect::from_min_size(pos2(bar.left() + k as f32 * 32.0, bar.top()), Vec2::new(32.0, 24.0));
                    let b = ui.interact(br, Id::new(("sched-btn", &s.id, k)), Sense::click());
                    if b.hovered() { any = true; ui.painter().rect_filled(br.shrink(1.0), 10.0, t.hover); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                    self.icons.paint(ui, icon, Rect::from_center_size(br.center(), Vec2::splat(14.0)), text);
                    if b.on_hover_text(tip).clicked() { act = Some((a, s.id.clone())); }
                }
                if any || resp.hovered() { ctx.data_mut(|d| d.insert_temp(Id::new("sched-hot"), s.id.clone())); }
                else { ctx.data_mut(|d| d.remove::<String>(Id::new("sched-hot"))); }
            }
        }
        match act {
            Some(("now", id)) => {
                if let Some(i) = self.msgx.sched.iter().position(|s| s.id == id) { let s = self.msgx.sched.remove(i); self.sched_post(s); self.sched_save(); }
            }
            Some(("re", id)) => { let due = self.msgx.sched.iter().find(|s| s.id == id).map_or(now_ms() + 3_600_000, |s| s.due_ms); self.sched_custom_open(&ctx, Some(id), due); }
            Some((_, id)) => ctx.data_mut(|d| d.insert_temp(Id::new("sched-discard"), id)),
            None => {}
        }
    }

    /// "Geplante Nachricht verwerfen?"
    fn sched_discard_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        let Some(id) = ctx.data(|d| d.get_temp::<String>(Id::new("sched-discard"))) else { return };
        let res = menu::webex_dialog(ctx, t, "sched-discard", "Geplante Nachricht verwerfen?", "", None, &[menu::Button::secondary("cancel", "Abbrechen"), menu::Button::danger("discard", "Verwerfen")]);
        if let Some(r) = res {
            if r == "discard" { self.msgx.sched.retain(|s| s.id != id); self.sched_save(); }
            ctx.data_mut(|d| d.remove::<String>(Id::new("sched-discard")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_round_trips() {
        for z in [-1000i64, 0, 1, 59, 60, 365, 10_957, 20_000, 20_773] { let (y, m, d) = civil_from_days(z); assert_eq!(days_from_civil(y, m, d), z); }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2026, 11, 16), 20_773);
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 28) + 1), (2024, 2, 29));
    }

    #[test]
    fn tomorrow_nine_is_tomorrow_at_nine_local() {
        let now = now_ms();
        let t = tomorrow_nine(now);
        let (y, m, d, _, _) = local_parts(now);
        let (yy, mm, dd, h, mi) = local_parts(t);
        assert_eq!(days_from_civil(yy, mm, dd) - days_from_civil(y, m, d), 1);
        assert_eq!((h, mi), (9, 0));
        assert!(scheduled_text(now, t).starts_with("Sie haben den Versand einer Nachricht morgen um 09:00"));
    }

    #[test]
    fn scheduled_messages_persist() {
        let s = Scheduled { id: "x".into(), space: "s1".into(), text: "Hallo @Theo".into(), markdown: true, due_ms: 1_800_000_000_000,
            mentions: vec![crate::richtext::MentionRef { id: "t@example.com".into(), name: "Theo".into(), all: false }] };
        assert_eq!(from_json(&to_json(&s)), Some(s));
    }
}
