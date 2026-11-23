//! UI state and dialogs of the meetings / calling domain that are not tied to one page: the data that arrives
//! from `cloud/meetcall.rs`, the in-app scheduler ("Meeting ansetzen"), "Meeting starten" (personal room), the call window's
//! keypad (DTMF) and the sample voicemail player.
//!
//! Strings (German pack): "Meeting ansetzen", "Thema", "Startdatum", "Startzeit", "Endzeit", "Eingeladene Teilnehmer",
//! "Anhand von Namen oder E-Mail-Adresse hinzufügen", "Beschreibung", "Meeting-Tagesordnung hinzufügen", "Abbrechen",
//! "Ansetzen", "Meeting angesetzt", "Meeting konnte nicht angesetzt werden", "Tastatur", "Eingabe ausblenden" / "Eingabe
//! anzeigen", "Schließen", "Heute", "morgen".
#![allow(clippy::too_many_arguments)]
use crate::cloud::meetcall::{CalEvent, CallRecord, ScheduleReq};
use crate::i18n;
use crate::*;
use super::menu;

/// The scheduler form (all fields as typed).
#[derive(Clone, Default, Debug, PartialEq)]
pub(crate) struct SchedForm { pub title: String, pub date: String, pub start: String, pub end: String, pub invitees: String, pub agenda: String, pub err: String }

#[derive(Default)]
pub(crate) struct McState {
    /// janus call history; `None` until the first answer.
    pub records: Option<Vec<CallRecord>>,
    /// calendar events; `None` until the first answer.
    pub events: Option<Vec<CalEvent>>,
    /// Meetings scheduled in this session in `--sample` (nothing is sent there).
    pub sample_events: Vec<CalEvent>,
    pub sched: Option<SchedForm>, pub sched_busy: bool, pub pmr_busy: bool,
    /// Call window keypad: open, typed tones, input hidden.
    pub keypad: bool, pub keypad_text: String, pub keypad_hidden: bool,
    /// Sample voicemail player: (index, position in s when last paused / started, started at (app time) while playing).
    pub vm_pos: Option<(usize, f64, Option<f64>)>,
    pub requested_history: bool,
}

/// Local wall-clock parts of a UTC timestamp.
pub(crate) fn local(ms: i64) -> i18n::Parts { i18n::parts(ms.div_euclid(1000) + i18n::local_offset_secs(ms.div_euclid(1000)) as i64) }
/// UTC ms of a local date + time.
pub(crate) fn from_local(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> i64 {
    let secs = i18n::days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60;
    (secs - i18n::local_offset_secs(secs) as i64) * 1000
}
pub(crate) fn hhmm(ms: i64) -> String { let p = local(ms); format!("{:02}:{:02}", p.hour, p.minute) }
/// "16:00 - 17:00".
pub(crate) fn time_range(a: i64, b: i64) -> String { if b > a { format!("{} - {}", hhmm(a), hhmm(b)) } else { hhmm(a) } }
/// Day header of the meeting list: "Heute", "Morgen" or the date.
pub(crate) fn day_title(ms: i64, now_ms: i64) -> String {
    let day = |t: i64| { let p = local(t); i18n::days_from_civil(p.year, p.month, p.day) };
    match day(ms) - day(now_ms) {
        0 => tr("Today").to_string(),
        1 => { let m = i18n::tr("QObject", "Tomorrow").into_owned(); let mut c = m.chars(); c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or(m) }
        _ => { let p = local(ms); format!("{}, {:02}.{:02}.{}", ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"][p.weekday as usize], p.day, p.month, p.year) }
    }
}

/// Parses the scheduler's date / time fields ("TT.MM.JJJJ", "HH:MM") into UTC ms (start, end). End before start = next day.
pub(crate) fn parse_form(f: &SchedForm) -> Result<(i64, i64), &'static str> {
    let d: Vec<i64> = f.date.trim().split('.').filter(|s| !s.is_empty()).map(|s| s.trim().parse::<i64>().unwrap_or(-1)).collect();
    let [dd, mm, yy] = d[..] else { return Err("Datum: TT.MM.JJJJ") };
    if !(1..=31).contains(&dd) || !(1..=12).contains(&mm) || yy < 2000 { return Err("Datum: TT.MM.JJJJ") }
    let t = |s: &str| -> Option<(i64, i64)> { let (h, m) = s.trim().split_once(':')?; let (h, m) = (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?); ((0..24).contains(&h) && (0..60).contains(&m)).then_some((h, m)) };
    let (Some((h1, m1)), Some((h2, m2))) = (t(&f.start), t(&f.end)) else { return Err("Zeit: HH:MM") };
    let a = from_local(yy, mm, dd, h1, m1);
    let mut b = from_local(yy, mm, dd, h2, m2);
    if b <= a { b += 86_400_000; }
    Ok((a, b))
}

impl App {
    pub(crate) fn on_meetcall(&mut self, r: Resp, now: f64) {
        match r {
            Resp::CallHistory(v) => self.call.mc.records = Some(v),
            Resp::Calendar(v) => self.call.mc.events = Some(v),
            Resp::Scheduled(res) => {
                self.call.mc.sched_busy = false;
                match res {
                    Ok(_) => { self.call.mc.sched = None; self.toast(i18n::tr("QObject", "Meeting scheduled").into_owned(), now); if let Some(c) = &self.client { c.req(Req::Calendar); } }
                    Err(e) => {
                        let msg = i18n::tr("QObject", "Unable to schedule a meeting").into_owned();
                        if let Some(f) = &mut self.call.mc.sched { f.err = if e.is_empty() { msg.clone() } else { format!("{msg} ({e})") }; }
                        self.toast(msg, now);
                    }
                }
            }
            Resp::PersonalRoom(res) => {
                self.call.mc.pmr_busy = false;
                match res {
                    Ok(link) => { platform::nmhost::record(&self.cfg, &link); system::open(&link); }
                    // No start button without a personal room. Message: the pack's own row ("We had a problem getting your latest Personal Room
                    // information from your {{APP_NAME}} site."), {{APP_NAME}} = Webex (the meeting site's product, filled by `tr`, docs/naming.md).
                    Err(_) => { let m = crate::trq!("We had a problem getting your latest Personal Room information from your {{APP_NAME}} site.").into_owned(); self.toast(m, now) }
                }
            }
            _ => {}
        }
    }

    /// "Meeting starten": starts the personal-room meeting.
    pub(crate) fn start_personal_meeting(&mut self, now: f64) {
        match &self.client {
            Some(c) => { if !self.call.mc.pmr_busy { self.call.mc.pmr_busy = true; c.req(Req::PersonalRoom); } }
            None => self.toast("https://example.com/meet/sample (Beispiel, im Offline-Modus wird nichts geöffnet)", now),
        }
    }

    /// Opens the in-app scheduler. Defaults: today, next full half hour, 30 minutes.
    pub(crate) fn open_scheduler(&mut self, title: &str, invitees: &[String]) {
        let now = i18n::now_ms();
        let slot = 1_800_000;
        let start = (now / slot + 1) * slot;
        let p = local(start);
        self.call.mc.sched = Some(SchedForm { title: title.into(), date: format!("{:02}.{:02}.{}", p.day, p.month, p.year), start: hhmm(start), end: hhmm(start + slot),
            invitees: invitees.join(", "), agenda: String::new(), err: String::new() });
    }

    /// Names typed in "Eingeladene Teilnehmer" -> addresses (e-mail as typed, or a contact whose name matches).
    pub(crate) fn resolve_invitees(&self, s: &str) -> Vec<String> {
        s.split([',', ';', '\n']).map(str::trim).filter(|x| !x.is_empty()).filter_map(|x| {
            if x.contains('@') { return Some(x.to_string()); }
            let q = x.to_lowercase();
            self.people.iter().chain(self.spaces.iter().flat_map(|s| s.participants.iter())).find(|p| p.name.to_lowercase() == q || p.name.to_lowercase().contains(&q)).map(|p| p.email.clone()).filter(|e| !e.is_empty())
        }).collect()
    }

    /// Per-frame overlays of this domain (scheduler dialog); called from `call_overlay`.
    pub(crate) fn meetcall_overlays(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        // Screenshot presets (offline `--sample --screenshot` only): WEBOX_SHOT_MEETCALL=schedule | keypad (with WEBOX_SAMPLE_CALL=connected) | vmplay
        if self.shot.is_some() && self.client.is_none() && self.frames == 40 {
            match std::env::var("WEBOX_SHOT_MEETCALL").as_deref() {
                Ok("schedule") => { let inv = vec!["jonas@example.com".to_string()]; self.open_scheduler("Weekly Sync", &inv); }
                Ok("keypad") => { self.call.mc.keypad = true; self.call.mc.keypad_text = "12#".into(); }
                Ok("vmplay") => self.call.mc.vm_pos = Some((0, 14.0, None)),
                _ => {}
            }
        }
        let Some(mut f) = self.call.mc.sched.clone() else { return };
        let busy = self.call.mc.sched_busy;
        let people: Vec<(String, String)> = self.people.iter().map(|p| (p.name.clone(), p.email.clone())).collect();
        let mut body = |ui: &mut egui::Ui| {
            let lab = |ui: &mut egui::Ui, s: &str| { ui.label(RichText::new(s).size(13.0).color(t.text2)); };
            lab(ui, "Thema");
            ui.add(egui::TextEdit::singleline(&mut f.title).hint_text("Thema").desired_width(f32::INFINITY));
            ui.add_space(8.0);
            lab(ui, "Datum und Uhrzeit");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut f.date).hint_text("Startdatum").desired_width(110.0)).on_hover_text("Startdatum");
                ui.add(egui::TextEdit::singleline(&mut f.start).hint_text("Startzeit").desired_width(60.0)).on_hover_text("Startzeit");
                ui.label("-");
                ui.add(egui::TextEdit::singleline(&mut f.end).hint_text("Endzeit").desired_width(60.0)).on_hover_text("Endzeit");
            });
            ui.add_space(8.0);
            lab(ui, "Eingeladene Teilnehmer");
            ui.add(egui::TextEdit::singleline(&mut f.invitees).hint_text("Anhand von Namen oder E-Mail-Adresse hinzufügen").desired_width(f32::INFINITY));
            // Suggestions for the last typed name.
            let last = f.invitees.rsplit([',', ';']).next().unwrap_or("").trim().to_lowercase();
            if last.len() >= 2 && !last.contains('@') {
                for (name, email) in people.iter().filter(|(n, e)| !e.is_empty() && n.to_lowercase().contains(&last)).take(4) {
                    if ui.add(egui::Label::new(RichText::new(format!("{name}  <{email}>")).size(13.0).color(t.link)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        let keep = f.invitees.rfind([',', ';']).map(|i| f.invitees[..=i].to_string() + " ").unwrap_or_default();
                        f.invitees = format!("{keep}{email}, ");
                    }
                }
            }
            ui.add_space(8.0);
            lab(ui, "Beschreibung");
            ui.add(egui::TextEdit::multiline(&mut f.agenda).hint_text("Meeting-Tagesordnung hinzufügen").desired_rows(3).desired_width(f32::INFINITY));
            if !f.err.is_empty() { ui.add_space(6.0); ui.label(RichText::new(&f.err).size(13.0).color(t.badge)); }
            if busy { ui.add_space(6.0); ui.label(RichText::new("Wird geladen ...").size(13.0).color(t.text2)); } // pack string
        };
        let r = menu::webex_dialog(ctx, t, "meetcall-schedule", "Meeting ansetzen", "", Some(&mut body),
            &[menu::Button::secondary("cancel", "Abbrechen"), menu::Button::primary("schedule", "Ansetzen")]);
        match r.as_deref() {
            Some("schedule") if !busy => {
                match parse_form(&f) {
                    Err(e) => { f.err = e.into(); self.call.mc.sched = Some(f); }
                    Ok(_) if f.title.trim().is_empty() => { f.err = "Thema".into(); self.call.mc.sched = Some(f); }
                    Ok((a, b)) => {
                        let req = ScheduleReq { title: f.title.trim().into(), start_ms: a, end_ms: b, agenda: f.agenda.clone(), invitees: self.resolve_invitees(&f.invitees), timezone: String::new() };
                        f.err.clear();
                        match &self.client {
                            Some(c) => { c.req(Req::Schedule(req)); self.call.mc.sched_busy = true; self.call.mc.sched = Some(f); }
                            None => { // --sample: nothing is sent, the meeting appears in the local list
                                self.call.mc.sample_events.push(CalEvent { id: format!("sample-{a}"), title: req.title, start_ms: a, end_ms: b, organizer: self.me.name.clone(), attendees: req.invitees.len(), link: "https://example.com/meet/new".into(), ..Default::default() });
                                self.call.mc.sched = None;
                                self.toast(i18n::tr("QObject", "Meeting scheduled").into_owned(), now);
                            }
                        }
                    }
                }
            }
            Some("schedule") => self.call.mc.sched = Some(f),
            Some(_) => { if !busy { self.call.mc.sched = None; } else { self.call.mc.sched = Some(f); } }
            None => self.call.mc.sched = Some(f),
        }
    }

    /// The calendar's meetings as the page shows them (calendar events + the locus "starting soon" list), sorted by start.
    pub(crate) fn meeting_rows(&self) -> Vec<CalEvent> {
        let now = i18n::now_ms();
        let mut v: Vec<CalEvent> = match (&self.client, &self.call.mc.events) {
            (Some(_), Some(e)) => e.clone(),
            (Some(_), None) => vec![],
            (None, _) => sample_events(now),
        };
        v.extend(self.call.mc.sample_events.iter().cloned());
        // Locus `startingSoon` (Meeting{title,start "HH:MM" UTC,link}) that the calendar doesn't already have.
        let today = { let p = i18n::parts(now / 1000); i18n::days_from_civil(p.year, p.month, p.day) * 86_400_000 };
        for m in &self.meetings {
            if v.iter().any(|e| (!m.link.is_empty() && e.link == m.link) || e.title == m.title) { continue; } // already in the calendar
            let start = m.start.split_once(':').and_then(|(h, mi)| Some(today + h.parse::<i64>().ok()? * 3_600_000 + mi.parse::<i64>().ok()? * 60_000)).unwrap_or(0);
            v.push(CalEvent { id: format!("locus-{}", m.link), title: m.title.clone(), start_ms: start, end_ms: 0, link: m.link.clone(), ..Default::default() });
        }
        v.sort_by_key(|e| e.start_ms);
        v
    }

    /// Call window keypad: 12 keys, typed tones, show / hide input, close. Each key sends its tone (DTMF) at once.
    pub(crate) fn call_keypad_ui(&mut self, ui: &mut egui::Ui, t: &Th, area: Rect) {
        const KEYS: [(&str, &str); 12] = [("1", ""), ("2", "ABC"), ("3", "DEF"), ("4", "GHI"), ("5", "JKL"), ("6", "MNO"), ("7", "PQRS"), ("8", "TUV"), ("9", "WXYZ"), ("*", ""), ("0", "+"), ("#", "")];
        let (kw, kh, gap) = (64.0, 44.0, 8.0);
        let w = kw * 3.0 + gap * 2.0 + 32.0;
        let h = 40.0 + 30.0 + 4.0 * (kh + gap) + 12.0;
        let r = Rect::from_center_size(area.center(), Vec2::new(w, h));
        ui.painter().rect_filled(r, 12.0, if menu::light(t) { Color32::WHITE } else { hex(0x000000) });
        ui.painter().rect_stroke(r, 12.0, Stroke::new(1.0_f32, t.sep), egui::StrokeKind::Inside);
        ui.painter().text(r.left_top() + Vec2::new(16.0, 20.0), egui::Align2::LEFT_CENTER, "Tastatur", semibold(15.0), t.text1);
        let close = super::call_toast::ghost_at(ui, &mut self.icons, t, "kp-close", r.right_top() + Vec2::new(-22.0, 20.0), 28.0, "cancel-regular", t.text1).on_hover_text("Schließen");
        if close.clicked() { self.call.mc.keypad = false; }
        let eye = super::call_toast::ghost_at(ui, &mut self.icons, t, "kp-eye", r.right_top() + Vec2::new(-54.0, 20.0), 28.0, if self.call.mc.keypad_hidden { "show-regular" } else { "hide-regular" }, t.text1)
            .on_hover_text(if self.call.mc.keypad_hidden { "Eingabe anzeigen" } else { "Eingabe ausblenden" });
        if eye.clicked() { self.call.mc.keypad_hidden = !self.call.mc.keypad_hidden; }
        let shown = if self.call.mc.keypad_hidden { "•".repeat(self.call.mc.keypad_text.chars().count()) } else { self.call.mc.keypad_text.clone() };
        ui.painter().text(pos2(r.center().x, r.top() + 54.0), egui::Align2::CENTER_CENTER, shown, FontId::proportional(20.0), t.text1);
        let x0 = r.left() + 16.0;
        let y0 = r.top() + 74.0;
        let mut pressed: Option<&str> = None;
        for (i, (k, letters)) in KEYS.iter().enumerate() {
            let kr = Rect::from_min_size(pos2(x0 + (i % 3) as f32 * (kw + gap), y0 + (i / 3) as f32 * (kh + gap)), Vec2::new(kw, kh));
            let resp = ui.interact(kr, ui.id().with(("kp", i)), Sense::click());
            let fill = if resp.is_pointer_button_down_on() { t.pill } else if resp.hovered() { t.hover } else { t.selected };
            ui.painter().rect_filled(kr, 10.0, fill);
            ui.painter().text(kr.center() - Vec2::new(0.0, if letters.is_empty() { 0.0 } else { 5.0 }), egui::Align2::CENTER_CENTER, k, FontId::proportional(19.0), t.text1);
            if !letters.is_empty() { ui.painter().text(kr.center() + Vec2::new(0.0, 12.0), egui::Align2::CENTER_CENTER, letters, FontId::proportional(9.0), t.text2); }
            if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            if resp.clicked() { pressed = Some(k); }
        }
        // Typing digits works too while the keypad is open.
        let typed: String = ui.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Text(s) = e { Some(s.clone()) } else { None }).collect());
        let mut tones: String = pressed.map(String::from).unwrap_or_default();
        tones.extend(typed.chars().filter(|c| c.is_ascii_digit() || *c == '*' || *c == '#'));
        if !tones.is_empty() {
            self.call.mc.keypad_text.push_str(&tones);
            if !self.call.sample { if let Some(c) = &self.client { c.req(Req::Call(crate::cloud::locus::CallReq::Dtmf(tones))); } }
        }
        if ui.input(|i| i.key_pressed(Key::Escape)) { self.call.mc.keypad = false; }
    }

    /// Sample voicemail player position in seconds (and whether it plays); stops at the end.
    pub(crate) fn vm_position(&mut self, i: usize, secs: u32, now: f64) -> (f64, bool) {
        match self.call.mc.vm_pos {
            Some((j, pos, Some(since))) if j == i => {
                let p = pos + (now - since);
                if p >= secs as f64 { self.call.mc.vm_pos = Some((i, 0.0, None)); (0.0, false) } else { (p, true) }
            }
            Some((j, pos, None)) if j == i => (pos, false),
            _ => (0.0, false),
        }
    }
    pub(crate) fn vm_toggle(&mut self, i: usize, secs: u32, now: f64) {
        let (p, playing) = self.vm_position(i, secs, now);
        self.call.mc.vm_pos = Some((i, p, if playing { None } else { Some(now) }));
    }
}

/// `--sample` calendar: a few meetings over today and the next days (made up; offline only).
pub(crate) fn sample_events(now: i64) -> Vec<CalEvent> {
    let p = local(now);
    let at = |dd: i64, h: i64, m: i64| from_local(p.year, p.month, p.day, h, m) + dd * 86_400_000;
    let e = |id: &str, title: &str, a: i64, mins: i64, org: &str, loc: &str, rec: bool, n: usize| CalEvent {
        id: id.into(), title: title.into(), start_ms: a, end_ms: a + mins * 60_000, organizer: org.into(), location: loc.into(),
        link: format!("https://example.com/meet/{id}"), recurring: rec, attendees: n, ..Default::default() };
    vec![
        e("c1", "Weekly Sync", at(0, 11, 0), 30, "Timo Steiner", "", true, 6),
        e("c2", "Design-Review", at(0, 14, 30), 60, "Lina Roth", "Zürich, Raum 3", false, 4),
        e("c3", "Sprint-Planung", at(1, 9, 0), 90, "Mara Lind", "", true, 8),
        e("c4", "Kundentermin", at(3, 15, 0), 45, "Alex", "Bern", false, 3),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn personal_room_error_is_a_pack_string() {
        if !crate::assets::cisco_assets::embedded() { return; } // needs the translation packs (docs/cisco-assets.md)
        crate::i18n::set_language("de");
        let m = crate::trq!("We had a problem getting your latest Personal Room information from your {{APP_NAME}} site.");
        assert!(m.starts_with("Beim Abrufen der aktuellen Informationen zu Ihrem persönlichen Raum"), "{m}");
    }
    #[test]
    fn form_parsing() {
        let f = SchedForm { date: "17.11.2026".into(), start: "09:30".into(), end: "10:00".into(), ..Default::default() };
        let (a, b) = parse_form(&f).unwrap();
        assert_eq!(b - a, 1_800_000);
        assert_eq!(hhmm(a), "09:30");
        let over = SchedForm { end: "09:00".into(), ..f.clone() };
        let (a2, b2) = parse_form(&over).unwrap();
        assert_eq!(b2 - a2, 23 * 3_600_000 + 1_800_000); // ends the next day
        assert!(parse_form(&SchedForm { date: "2026-11-17".into(), ..f.clone() }).is_err());
        assert!(parse_form(&SchedForm { start: "25:00".into(), ..f }).is_err());
    }
    #[test]
    fn day_titles() {
        let now = from_local(2026, 11, 16, 12, 0);
        assert_eq!(day_title(now, now), tr("Today"));
        let tomorrow = day_title(now + 86_400_000, now); // "Morgen" with the German pack, "Tomorrow" without one (tests)
        assert!(!tomorrow.contains('.') && tomorrow.chars().next().is_some_and(char::is_uppercase));
        assert_eq!(day_title(from_local(2026, 11, 19, 9, 0), now), "Do, 19.11.2026");
    }
}
