//! "Anrufe" (call history + keypad) and "Voicemail".
//!
//! Strings come from the German pack ("Alle Anrufe", "Verpasste Anrufe", "Tastenfeldansicht anzeigen/ausblenden",
//! "Nummer suchen oder wählen", "Zur Hauptansicht zurückkehren", "Verpasst", "Ausgehender Anruf", "Dauer", the empty
//! states, the delete dialog, "Posteingang" / "Gesendet" / "Papierkorb"). Rows follow the messaging list rows (50 px,
//! avatar 32). Illustrations: `Callhistory_empty_space`, `empty_voicemail`.
//!
//! Data: the call history is the account's janus history (cloud/meetcall.rs). Voicemail: accounts without a voicemail
//! service show the "not set up" text; the Webex Calling voicemail API is not implemented.
#![allow(clippy::too_many_arguments)]
use crate::*;
use super::menu::{self, Entry, Item};
use super::page_kit::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CallKind { Incoming, Outgoing, Missed }

/// One row of the call history: the janus `userSessions` of the account (cloud/meetcall.rs), made-up rows in `--sample`.
/// `session_id` / `end_ms` identify the record for janus `markAsDeleted`.
#[derive(Clone, Debug)]
pub(crate) struct CallLogEntry { pub name: String, pub email: String, pub kind: CallKind, pub at_ms: i64, pub secs: u32, pub session_id: String, pub end_ms: i64, pub phone: String }
impl CallLogEntry { pub(crate) fn key(&self) -> String { if self.session_id.is_empty() { format!("{}|{}", self.name, self.at_ms) } else { self.session_id.clone() } } }

/// "Mehr erfahren" of the upgrade prompt: the Webex help root.
const LEARN_MORE: &str = "https://help.webex.com/";

/// One voicemail (`--sample` only, see the module comment).
#[derive(Clone, Debug)]
pub(crate) struct VoicemailEntry { pub name: String, pub email: String, pub number: String, pub at_ms: i64, pub secs: u32, pub unread: bool, pub urgent: bool, pub sent: bool, pub transcript: String }

pub(crate) fn kind_text(k: CallKind) -> &'static str { match k { CallKind::Incoming => "Eingehender Anruf", CallKind::Outgoing => "Ausgehender Anruf", CallKind::Missed => "Verpasst" } }
pub(crate) fn kind_icon(k: CallKind) -> &'static str { match k { CallKind::Incoming => "call-incoming-regular", CallKind::Outgoing => "call-outgoing-regular", CallKind::Missed => "call-missed-regular" } }
pub(crate) fn duration_text(s: u32) -> String { if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) } }
/// listitem-textmissed (#FC8B98 / #AB0A15).
pub(crate) fn missed_color(t: &Th) -> Color32 { t.tk("listitem-textmissed") }

const KEYS: [(&str, &str); 12] = [("1", ""), ("2", "ABC"), ("3", "DEF"), ("4", "GHI"), ("5", "JKL"), ("6", "MNO"), ("7", "PQRS"), ("8", "TUV"), ("9", "WXYZ"), ("*", ""), ("0", "+"), ("#", "")];

impl App {
    /// The call history shown in the page: made-up entries in `--sample`; with a real session the account's janus history
    /// (before it arrives: the calls of this session).
    pub(crate) fn call_log(&self) -> Vec<CallLogEntry> {
        if self.client.is_none() {
            let now = super::chrome::now_ms();
            let (m, h, d) = (60_000_i64, 3_600_000_i64, 86_400_000_i64);
            let e = |n: &str, em: &str, k: CallKind, ago: i64, secs: u32| CallLogEntry { name: n.into(), email: em.into(), kind: k, at_ms: now - ago, secs,
                session_id: String::new(), end_ms: now - ago + secs as i64 * 1000, phone: String::new() };
            return vec![
                e("Jonas Weber", "jonas@example.com", CallKind::Missed, 25 * m, 0),
                e("Timo Steiner", "timo@example.com", CallKind::Outgoing, 3 * h, 252),
                e("Mara Lind", "mara@example.com", CallKind::Incoming, d + h, 725),
                e("Lina Roth", "lina@example.com", CallKind::Missed, d + 4 * h, 0),
                e("Nina Huber", "nina@example.com", CallKind::Outgoing, 2 * d, 45),
                e("Jonas Weber", "jonas@example.com", CallKind::Incoming, 4 * d, 1867),
                e("Timo Steiner", "timo@example.com", CallKind::Missed, 9 * d, 0),
            ];
        }
        if let Some(recs) = &self.call.mc.records {
            return recs.iter().map(|r| {
                let known = self.people.iter().chain(self.spaces.iter().flat_map(|s| s.participants.iter()))
                    .find(|p| (!r.email.is_empty() && p.email.eq_ignore_ascii_case(&r.email)) || (!r.other_id.is_empty() && p.id == r.other_id));
                CallLogEntry {
                    name: known.map(|p| p.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| r.name.clone()),
                    email: known.map(|p| p.email.clone()).filter(|e| !e.is_empty()).unwrap_or_else(|| r.email.clone()),
                    kind: if r.missed() { CallKind::Missed } else if r.outgoing { CallKind::Outgoing } else { CallKind::Incoming },
                    at_ms: r.start_ms, secs: r.secs, session_id: r.session_id.clone(), end_ms: r.end_ms, phone: r.phone.clone(),
                }
            }).collect();
        }
        self.call.history.iter().map(|(name, incoming, result)| CallLogEntry {
            name: name.clone(), email: self.people.iter().find(|p| &p.name == name).map(|p| p.email.clone()).unwrap_or_default(),
            kind: if *result == "Verpasst" { CallKind::Missed } else if *incoming { CallKind::Incoming } else { CallKind::Outgoing }, at_ms: 0, secs: 0,
            session_id: String::new(), end_ms: 0, phone: String::new(),
        }).collect()
    }

    /// The number on the "Anrufe" rail entry: calls missed since the tab was last looked at (settings `calls_seen_ms`).
    pub(crate) fn missed_badge(&self) -> usize {
        let seen = self.settings[super::settings::CALLS_SEEN_MS].as_i64().unwrap_or(i64::MAX);
        if self.client.is_none() { // offline sample: the made-up history
            return self.call_log().iter().filter(|e| e.kind == CallKind::Missed && e.at_ms > seen).count();
        }
        unseen_missed(self.call.mc.records.as_deref(), &self.call.mc.missed_at, seen)
    }

    /// The "Anrufe" list is on screen in the window in front: its missed calls count as seen.
    fn mark_calls_seen(&mut self) {
        if !self.window_active() || self.missed_badge() == 0 { return; }
        self.settings[super::settings::CALLS_SEEN_MS] = json!(super::chrome::now_ms());
        if self.client.is_some() && self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); }
    }

    /// An incoming call ended unanswered on every device (locus "missed"): badge it and announce it.
    pub(crate) fn on_missed_call(&mut self, name: &str, email: &str, now: f64) {
        let m = &mut self.call.mc.missed_at;
        m.push(super::chrome::now_ms());
        if m.len() > 50 { m.remove(0); }
        self.notify_missed_call(name, email, now);
    }

    pub(crate) fn voicemails(&self) -> Vec<VoicemailEntry> {
        if self.client.is_some() { return vec![]; } // no voicemail service (see the module comment)
        let now = super::chrome::now_ms();
        let (m, h, d) = (60_000_i64, 3_600_000_i64, 86_400_000_i64);
        let v = |n: &str, em: &str, num: &str, ago: i64, secs: u32, unread: bool, urgent: bool, sent: bool, tr: &str| VoicemailEntry {
            name: n.into(), email: em.into(), number: num.into(), at_ms: now - ago, secs, unread, urgent, sent, transcript: tr.into() };
        vec![
            v("Lina Roth", "lina@example.com", "+1 202 555 0122", 50 * m, 38, true, true, false, "Hallo, hier ist Lina. Kannst du mich bitte kurz zurückrufen? Es geht um das Angebot für morgen."),
            v("Unbekannt", "", "+1 202 555 0187", 5 * h, 12, true, false, false, ""),
            v("Mara Lind", "mara@example.com", "+1 202 555 0131", d + 2 * h, 95, false, false, false, "Danke für die Unterlagen, ich melde mich nächste Woche."),
            v("Jonas Weber", "jonas@example.com", "", 2 * d, 21, false, false, true, ""),
        ]
    }

    pub(crate) fn person_for(&self, name: &str, email: &str) -> Option<Person> {
        self.people.iter().find(|p| (!email.is_empty() && p.email.eq_ignore_ascii_case(email)) || p.name == name).cloned()
    }

    /// Dials what was typed in the keypad: an address, or a name from the contact list. Goes through `start_call` only.
    fn dial_text(&mut self, now: f64) {
        let d = self.dial.trim().to_string();
        if d.is_empty() { return; }
        if d.contains('@') { self.start_call(&d, "", now); return; }
        let q = d.to_lowercase();
        if let Some(p) = self.people.iter().find(|p| p.name.to_lowercase().contains(&q)).cloned() { self.start_call(&p.email, &p.name, now); return; }
        self.no_phone_calling(now);
    }

    /// Calling a phone number needs Webex Calling; without it the upgrade prompt is shown, whose second line is this text with
    /// the "Mehr erfahren" link.
    pub(crate) fn no_phone_calling(&mut self, now: f64) {
        self.toast("Um eine beliebige Person mit einer Telefonnummer anzurufen, klicken Sie auf „Mehr erfahren“.", now);
    }

    /// Calls back the person of a history row: their Webex address (guarded call path), else the phone-number prompt.
    pub(crate) fn call_back(&mut self, e: &CallLogEntry, now: f64) {
        if !e.email.is_empty() { self.start_call(&e.email, &e.name, now); } else { self.no_phone_calling(now); }
    }

    pub(crate) fn open_chat_with(&mut self, name: &str, now: f64) {
        match self.spaces.iter().find(|s| s.direct && s.title == name).map(|s| s.id.clone()) {
            Some(id) => { self.tab = Tab::Messaging; self.team_sel = None; self.open_space(id); }
            None => self.toast(format!("Es gibt noch keinen Chat mit {name}"), now), // Webox's own wording
        }
    }

    pub(crate) fn calls_list(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        self.mark_calls_seen();
        let ctx = ui.ctx().clone();
        let mut st = self.pst(&ctx);
        let (plus, _) = self.list_header(ui, t, tr("Calling"), Some("Einen Anruf tätigen"), None);
        if plus { st.dial_open = true; }
        ui.add_space(8.0);
        let mut dial_toggle = false;
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if !st.dial_open {
                for (i, n) in ["Alle Anrufe", "Verpasste Anrufe"].iter().enumerate() {
                    if text_tab(ui, n, st.calls_tab as usize == i, t, t.tab_on).clicked() { st.calls_tab = i as u8; }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                let tip = if st.dial_open { "Tastenfeldansicht ausblenden" } else { "Tastenfeldansicht anzeigen" };
                if self.icons.button(ui, "dialpad-regular", 28.0, 18.0, if st.dial_open { t.link } else { t.text2 }, t.hover).on_hover_text(tip).clicked() { dial_toggle = true; }
            });
        });
        if dial_toggle { st.dial_open = !st.dial_open; }
        ui.add_space(6.0);
        if st.dial_open { self.set_pst(&ctx, st.clone()); self.dial_view(ui, t, now, &mut st); self.set_pst(&ctx, st); return; }

        // The janus history is read at sign-in and on every janus mercury event; ask once more if it never came.
        if !self.call.mc.requested_history && self.call.mc.records.is_none() {
            self.call.mc.requested_history = true;
            if let Some(c) = &self.client { c.req(Req::CallHistory); }
        }
        let log = self.call_log();
        let nowms = super::chrome::now_ms();
        ui.spacing_mut().scroll = egui::style::ScrollStyle { bar_width: 10.0, bar_inner_margin: 2.0, bar_outer_margin: 0.0, ..egui::style::ScrollStyle::solid() };
        let mut menu_req: Option<(String, Rect)> = None;
        let mut call_now: Option<CallLogEntry> = None;
        crate::ui::widgets::vscroll().auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let mut shown = 0;
            for (i, e) in log.iter().enumerate() {
                let key = e.key();
                if st.call_deleted.contains(&key) || (st.calls_tab == 1 && e.kind != CallKind::Missed) { continue; }
                shown += 1;
                let p = self.person_for(&e.name, &e.email);
                let photo = p.as_ref().and_then(|p| self.photo_for(ui.ctx(), &p.id, &p.email));
                let right = if e.at_ms > 0 { when_text(e.at_ms, nowms) } else { String::new() };
                let mut spec = RowSpec::new(format!("call{i}"), &e.name);
                spec.sub = kind_text(e.kind); spec.sub_icon = Some(kind_icon(e.kind)); spec.right = &right;
                if e.kind == CallKind::Missed { spec.title_col = Some(missed_color(t)); spec.sub_col = Some(missed_color(t)); }
                spec.selected = st.call_sel == Some(i); spec.status = p.as_ref().map(|p| p.status.as_str()); spec.photo = photo;
                spec.action = Some(("handset-regular", "Anrufen"));
                let out = self.page_row(ui, t, &spec);
                if out.clicked { st.call_sel = Some(i); }
                if out.double || out.action { call_now = Some(e.clone()); }
                if let Some(r) = out.menu { st.call_sel = Some(i); menu_req = Some((format!("{i}"), r)); }
            }
            if shown == 0 {
                ui.add_space(40.0);
                let (title, body) = if st.calls_tab == 1 { ("Keine verpassten Anrufe", "") } else { ("Keine Anrufe", "") }; // Webox's own wording
                ui.vertical_centered(|ui| { ui.label(RichText::new(title).font(semibold(15.0)).color(t.text1)); ui.label(RichText::new(body).color(t.text2)); });
            }
        });
        self.set_pst(&ctx, st);
        if let Some(e) = call_now { self.call_back(&e, now); }
        if let Some((k, r)) = menu_req { self.open_page_menu(&ctx, "call-row", &k, r); }
    }

    fn dial_view(&mut self, ui: &mut egui::Ui, t: &Th, now: f64, st: &mut PageState) {
        // Search / dial field.
        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width() - 6.0, 36.0), Sense::hover());
        ui.painter().rect_filled(r, 18.0, t.ink_a(0.07));
        self.icons.paint(ui, "search-regular", Rect::from_center_size(r.left_center() + Vec2::new(20.0, 0.0), Vec2::splat(16.0)), t.text2);
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(r.min + Vec2::new(38.0, 0.0), r.max - Vec2::new(40.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let resp = inner.add(egui::TextEdit::singleline(&mut self.dial).hint_text(RichText::new("Nummer suchen oder wählen").color(t.text2).size(15.0)).font(FontId::proportional(18.0)).frame(false).desired_width(r.width() - 80.0));
        if !self.dial.is_empty() {
            let br = Rect::from_center_size(r.right_center() - Vec2::new(20.0, 0.0), Vec2::splat(24.0));
            let bresp = ui.interact(br, ui.id().with("dial-back"), Sense::click());
            if bresp.hovered() { ui.painter().circle_filled(br.center(), 12.0, t.ink_a(0.12)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            self.icons.paint(ui, "backspace-regular", Rect::from_center_size(br.center(), Vec2::splat(16.0)), t.text1);
            if bresp.clicked() { self.dial.pop(); }
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) { self.dial_text(now); }
        ui.add_space(8.0);
        // Matches from the contact list.
        let q = self.dial.trim().to_lowercase();
        if !q.is_empty() {
            let hits: Vec<Person> = self.people.iter().filter(|p| p.name.to_lowercase().contains(&q) || p.email.to_lowercase().contains(&q)).take(3).cloned().collect();
            for p in hits {
                let photo = self.photo_for(ui.ctx(), &p.id, &p.email);
                let mut spec = RowSpec::new(format!("dial-{}", p.id), &p.name);
                spec.sub = &p.email; spec.status = Some(&p.status); spec.photo = photo; spec.action = Some(("handset-regular", "Anrufen"));
                let out = self.page_row(ui, t, &spec);
                if out.clicked || out.action { self.start_call(&p.email, &p.name, now); }
            }
            ui.add_space(4.0);
        }
        // 3 x 4 keypad.
        let avail = ui.available_width();
        let (kw, kh, gap) = (84.0_f32.min((avail - 24.0) / 3.0), 56.0, 10.0);
        let total_w = kw * 3.0 + gap * 2.0;
        let x0 = ui.min_rect().left() + (avail - total_w) / 2.0 - 2.0;
        let y0 = ui.cursor().top() + 6.0;
        for (i, (k, letters)) in KEYS.iter().enumerate() {
            let (col, row) = ((i % 3) as f32, (i / 3) as f32);
            let kr = Rect::from_min_size(pos2(x0 + col * (kw + gap), y0 + row * (kh + gap)), Vec2::new(kw, kh));
            let kresp = ui.interact(kr, ui.id().with(("key", i)), Sense::click());
            let fill = if kresp.is_pointer_button_down_on() { t.ink_a(0.2) } else if kresp.hovered() { t.ink_a(0.14) } else { t.ink_a(0.07) };
            if kresp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            ui.painter().rect_filled(kr, 14.0, fill);
            ui.painter().text(kr.center() - Vec2::new(0.0, if letters.is_empty() { 0.0 } else { 6.0 }), egui::Align2::CENTER_CENTER, k, FontId::proportional(22.0), t.text1);
            if !letters.is_empty() { ui.painter().text(kr.center() + Vec2::new(0.0, 14.0), egui::Align2::CENTER_CENTER, letters, FontId::proportional(10.0), t.text2); }
            if kresp.clicked() { self.dial.push_str(k); }
        }
        let by = y0 + 4.0 * (kh + gap) + 6.0;
        let ctr = pos2(x0 + total_w / 2.0, by + 28.0);
        let call_r = Rect::from_center_size(ctr, Vec2::splat(56.0));
        let cresp = ui.interact(call_r, ui.id().with("dial-call"), Sense::click());
        ui.painter().circle_filled(ctr, 28.0, if cresp.hovered() { t.green.gamma_multiply(0.85) } else { t.green });
        self.icons.paint(ui, "handset-filled", Rect::from_center_size(ctr, Vec2::splat(26.0)), Color32::WHITE);
        if cresp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        if cresp.on_hover_text("Anrufen").clicked() { self.dial_text(now); }
        ui.allocate_space(Vec2::new(1.0, (by + 70.0) - ui.cursor().top()));
        let rr = Rect::from_center_size(pos2(x0 + total_w / 2.0, by + 90.0), Vec2::new(220.0, 22.0));
        let rresp = ui.interact(rr, ui.id().with("dial-return"), Sense::click());
        if rresp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        ui.painter().text(rr.center(), egui::Align2::CENTER_CENTER, "Zur Hauptansicht zurückkehren", semibold(13.5), t.link);
        if rresp.clicked() { st.dial_open = false; }
    }

    pub(crate) fn calls_detail(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let st = self.pst(&ctx);
        let log = self.call_log();
        let sel = st.call_sel.and_then(|i| log.get(i).cloned());
        let Some(e) = sel else {
            let area = ui.max_rect();
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area));
            // The empty state's "Mehr erfahren" link opens the help center (ui/profile_menu.rs).
            if self.empty_state(&mut c, t, "Callhistory_empty_space", 160.0, "Einen Anruf tätigen oder einem Meeting beitreten",
                "Um eine andere Person anzurufen, die Webex verwendet, geben Sie den Namen oder die E-Mail-Adresse in die Suchleiste ein.\nUm einem Meeting beizutreten, geben Sie die Adresse in der Suchleiste ein oder kopieren Sie sie dorthin.", Some("Mehr erfahren")) { system::open(LEARN_MORE); }
            return;
        };
        let p = self.person_for(&e.name, &e.email);
        let area = ui.max_rect().shrink2(Vec2::new(40.0, 36.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        self.person_header(&mut c, t, &e.name, &e.email, p.as_ref(), 72.0);
        if !e.phone.is_empty() && e.phone != e.name { c.add_space(4.0); c.label(RichText::new(&e.phone).size(13.5).color(t.text2)); }
        c.add_space(18.0);
        c.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            if self.round_labeled(ui, t, "handset-filled", "Anrufen", t.green, Color32::WHITE, 72.0).clicked() { self.call_back(&e, now); }
            if self.round_labeled(ui, t, "chat-filled", "Chat", t.ink_a(0.14), t.text1, 72.0).clicked() { self.open_chat_with(&e.name, now); }
            if self.round_labeled(ui, t, "contact-card-filled", "Visitenkarte", t.ink_a(0.14), t.text1, 84.0).clicked() {
                let mut s = self.pst(ui.ctx()); s.contact_card = Some((e.email.clone(), e.name.clone(), Rect::from_center_size(ui.min_rect().center_top() + Vec2::new(0.0, 120.0), Vec2::splat(4.0)))); self.set_pst(ui.ctx(), s);
            }
        });
        c.add_space(22.0);
        c.label(RichText::new("Anrufdetails").font(semibold(15.0)).color(t.text1));
        c.add_space(6.0);
        let nowms = super::chrome::now_ms();
        for h in log.iter().filter(|x| x.name == e.name && !st.call_deleted.contains(&x.key())) {
            let (r, _) = c.allocate_exact_size(Vec2::new(c.available_width().min(520.0), 34.0), Sense::hover());
            let col = if h.kind == CallKind::Missed { missed_color(t) } else { t.text1 };
            self.icons.paint(&c, kind_icon(h.kind), Rect::from_center_size(r.left_center() + Vec2::new(10.0, 0.0), Vec2::splat(16.0)), col);
            c.painter().text(r.left_center() + Vec2::new(30.0, 0.0), egui::Align2::LEFT_CENTER, kind_text(h.kind), FontId::proportional(14.5), col);
            let when = if h.at_ms > 0 { when_text(h.at_ms, nowms) } else { String::new() };
            let right = if h.secs > 0 { format!("{when}   Dauer {}", duration_text(h.secs)) } else { when };
            c.painter().text(r.right_center(), egui::Align2::RIGHT_CENTER, right, FontId::proportional(13.5), t.text2);
            c.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0_f32, t.ink_a(0.08)));
        }
    }

    /// Big avatar + name + presence line + email (used in the detail panes).
    pub(crate) fn person_header(&mut self, ui: &mut egui::Ui, t: &Th, name: &str, email: &str, p: Option<&Person>, size: f32) {
        let photo = p.and_then(|p| self.photo_for(ui.ctx(), &p.id, &p.email));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            avatar_photo(ui, &mut self.icons, t, name, size, false, None, p.map(|p| p.status.as_str()), photo);
            ui.vertical(|ui| {
                ui.add_space(size / 2.0 - 24.0);
                ui.label(RichText::new(name).font(semibold(21.0)).color(t.text1));
                let line = p.map(|p| self.person_line(p, super::chrome::now_ms())).unwrap_or_default();
                if !line.is_empty() { ui.label(RichText::new(line).size(14.0).color(t.text2)); }
                if !email.is_empty() { ui.label(RichText::new(email).size(13.0).color(t.text2)); }
            });
        });
    }

    pub(crate) fn calls_menu(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let items = vec![
            Item::new("call", "Anrufen").icon("handset-regular").entry(), Item::new("chat", "Chat").icon("chat-regular").entry(),
            Item::new("card", "Visitenkarte").icon("contact-card-regular").entry(), Entry::Sep, Item::new("delete", "Löschen").icon("delete-regular").danger().entry(),
        ];
        if let Some((key, pick)) = self.page_menu(ctx, t, "call-row", &items) {
            let log = self.call_log();
            let Some(e) = key.parse::<usize>().ok().and_then(|i| log.get(i).cloned()) else { return };
            match pick.as_str() {
                "call" => self.call_back(&e, now),
                "chat" => self.open_chat_with(&e.name, now),
                "card" => { let mut s = self.pst(ctx); let at = s.menu_target.as_ref().map(|m| m.1).unwrap_or(Rect::NOTHING); s.contact_card = Some((e.email.clone(), e.name.clone(), at)); self.set_pst(ctx, s); }
                _ => { let mut s = self.pst(ctx); s.call_confirm_delete = Some(e.key()); self.set_pst(ctx, s); }
            }
        }
        let mut s = self.pst(ctx);
        if let Some(k) = s.call_confirm_delete.clone() {
            let r = menu::webex_dialog(ctx, t, "call-delete", "Ausgewählte löschen?", "Ausgewählte Elemente dauerhaft aus Ihrem Anrufprotokoll löschen?", None,
                &[menu::Button::secondary("cancel", "Abbrechen"), menu::Button::danger("delete", "Löschen")]);
            match r.as_deref() {
                Some("delete") => {
                    // A janus record is deleted on the server (the delete dialog -> markAsDeleted); hidden at once.
                    let rec = self.call_log().into_iter().find(|e| e.key() == k && !e.session_id.is_empty());
                    if let (Some(e), Some(c)) = (rec, &self.client) { c.req(Req::DeleteCalls(vec![(e.session_id, e.end_ms)])); }
                    s.call_deleted.insert(k); s.call_confirm_delete = None; s.call_sel = None; self.set_pst(ctx, s);
                }
                Some(_) => { s.call_confirm_delete = None; self.set_pst(ctx, s); }
                None => {}
            }
        }
    }

}

/// Calls missed after `seen` (Unix ms): the janus history's missed rows, or this session's own count (`local`, end times)
/// while janus has not listed them yet. Both describe the same calls, so the larger number is the answer, not the sum.
pub(crate) fn unseen_missed(records: Option<&[crate::cloud::meetcall::CallRecord]>, local: &[i64], seen: i64) -> usize {
    let janus = records.map_or(0, |r| r.iter().filter(|r| r.missed() && r.start_ms > seen).count());
    janus.max(local.iter().filter(|&&t| t > seen).count())
}

/// Text of the rail badge: the count, at most "99+".
pub(crate) fn badge_text(n: usize) -> String { if n > 99 { "99+".into() } else { n.to_string() } }

/// Hover text of the "Anrufe" rail entry with missed calls (pack: "{{0}} entgangener Anruf" / "{{0}} entgangene Anrufe").
pub(crate) fn missed_tip(n: usize) -> String { if n == 1 { "1 entgangener Anruf".into() } else { format!("{n} entgangene Anrufe") } }

mod voicemail;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::meetcall::CallRecord;

    fn rec(start_ms: i64, outgoing: bool, disposition: &str) -> CallRecord { CallRecord { start_ms, outgoing, disposition: disposition.into(), ..Default::default() } }

    #[test]
    fn only_incoming_unanswered_calls_after_the_last_visit_are_badged() {
        let r = [rec(50, false, "MISSED"), rec(150, false, "CANCELED"), rec(160, false, "Answered"), rec(170, true, "Initiated"), rec(180, false, "MISSED")];
        assert_eq!(unseen_missed(Some(&r), &[], 100), 2);
        assert_eq!(unseen_missed(Some(&r), &[], 200), 0, "opening the tab clears the badge");
    }

    #[test]
    fn a_call_missed_just_now_counts_before_janus_lists_it_and_is_not_counted_twice() {
        assert_eq!(unseen_missed(None, &[120], 100), 1);
        assert_eq!(unseen_missed(Some(&[]), &[120], 100), 1, "janus has not caught up");
        assert_eq!(unseen_missed(Some(&[rec(110, false, "MISSED")]), &[120], 100), 1, "the same call from both sides");
        assert_eq!(unseen_missed(None, &[90], 100), 0);
    }

    #[test]
    fn badge_and_tooltip_texts() {
        assert_eq!(badge_text(1), "1");
        assert_eq!(badge_text(120), "99+");
        assert_eq!(missed_tip(1), "1 entgangener Anruf");
        assert_eq!(missed_tip(3), "3 entgangene Anrufe");
    }
}
