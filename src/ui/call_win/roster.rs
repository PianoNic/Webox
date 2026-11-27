//! The participants panel of the call window ("Teilnehmerbereich"): everyone in the call with mute state, raised hand
//! and role, a field to invite people, and per person a menu to mute, lower the hand or remove them; the host can end
//! the call for everyone. Requests: `cloud/locus/controls.rs`.
//!
//! Official: `ParticipantList/ParticipantItemDelegate.qml` (row 48, avatar 32, control buttons 28, second line),
//! `PhoneWidget/ParticipantsPopup.qml` ("Remove from call", width 278), `ParticipantList/InvitePeopleAndDevicePage.qml`.
//! From the language pack: "Participants ({{0}})", "Participants panel", "Add People", "Add people by name or email
//! address", "Invite", "Invite sent", "Me", "Host", "Host, me", "Calling...",
//! "Muted", "Mute", "Unmute", "Raise hand", "Lower hand", "Raised hand", "Remove from call",
//! "Remove {{0}}?", "Remove", "Cancel".
//! Webox's own: the panel width (`PANEL_W`) and the end-for-everyone texts (assets/i18n/webox).
use super::*;
use crate::cloud::locus::controls::{Ctl, Member, Roster};
use crate::cloud::locus::CallReq;

/// Panel width (the official participants pane is resizable; Webox: fixed).
pub(crate) const PANEL_W: f32 = 320.0;
const HEAD_H: f32 = 48.0;
const ROW_H: f32 = 48.0; // ParticipantItemDelegate height
const AVATAR: f32 = 32.0;
const BTN: f32 = 28.0; // control button size
const MENU: &str = "roster-member";

#[derive(Default)]
pub(crate) struct RosterUi {
    pub open: bool,
    invite: String,
    /// The person whose menu is open.
    menu_for: Option<String>,
    confirm: Option<Confirm>,
}

#[derive(Clone, Debug, PartialEq)]
enum Confirm { Remove(String, String), EndAll }

/// Second line of a roster row.
pub(crate) fn second_line(m: &Member) -> &'static str {
    match (m.me, m.host) {
        _ if !m.joined() => tr("Calling..."),
        (true, true) => tr("Host, me"),
        (true, false) => tr("Me"),
        (false, true) => tr("Host"),
        _ => "",
    }
}

/// Menu entries for a person: (id, label, danger).
pub(crate) fn member_actions(r: &Roster, m: &Member) -> Vec<(&'static str, &'static str, bool)> {
    let mut v = Vec::new();
    if m.me {
        if r.group { v.push(if m.hand { ("hand-down", tr("Lower hand"), false) } else { ("hand-up", tr("Raise hand"), false) }); }
        return v;
    }
    if m.joined() { v.push(if m.muted { ("unmute", tr("Unmute"), false) } else { ("mute", tr("Mute"), false) }); }
    if m.hand && r.me_host { v.push(("hand-down", tr("Lower hand"), false)); }
    if r.may_remove() { v.push(("remove", tr("Remove from call"), true)); }
    v
}

/// The control for a picked menu entry (`remove` asks first, so it gives none).
pub(crate) fn action_ctl(id: &str, m: &Member) -> Option<Ctl> {
    match id {
        "mute" => Some(Ctl::Mute(m.id.clone(), true)),
        "unmute" => Some(Ctl::Mute(m.id.clone(), false)),
        "hand-up" => Some(Ctl::Hand(m.id.clone(), true)),
        "hand-down" => Some(Ctl::Hand(m.id.clone(), false)),
        _ => None,
    }
}

/// Up to four known people matching what is typed into the invite field (name or address), not already in the call.
pub(crate) fn invite_suggestions<'a>(people: &'a [cloud::Person], q: &str, r: &Roster) -> Vec<&'a cloud::Person> {
    let q = q.trim().to_lowercase();
    if q.chars().count() < 2 { return vec![]; }
    people.iter().filter(|p| !p.email.is_empty() && (p.name.to_lowercase().contains(&q) || p.email.to_lowercase().contains(&q)))
        .filter(|p| !r.members.iter().any(|m| m.email.eq_ignore_ascii_case(&p.email))).take(4).collect()
}

/// A sample roster for `WEBOX_SAMPLE_CALL=roster` (made-up people).
pub(crate) fn sample_roster() -> Roster {
    let m = |id: &str, name: &str, state: &str, muted: bool, hand: bool, host: bool, me: bool| Member {
        id: id.into(), name: name.into(), email: format!("{}@example.com", id), state: state.into(), muted, hand, host, me };
    let mut r = Roster { members: vec![
        m("p3", "Erika Beispiel", "JOINED", false, true, false, false),
        m("me", "Me Myself", "JOINED", false, false, true, true),
        m("p1", "Max Muster", "JOINED", true, false, false, false),
        m("p2", "Lea Probe", "JOINED", false, false, false, false),
        m("p4", "Jonas Test", "NOTIFIED", false, false, false, false),
    ], me_host: true, group: true };
    r.members.sort_by_key(|m| (!m.hand, !m.me, !m.joined(), m.name.to_lowercase()));
    r
}

/// The sample mode's local stand-in for Locus: applies a control to the made-up roster.
fn sample_apply(r: &mut Roster, c: &Ctl) {
    match c {
        Ctl::Mute(id, m) => { if let Some(x) = r.members.iter_mut().find(|x| &x.id == id) { x.muted = *m; } }
        Ctl::Hand(id, h) => { if let Some(x) = r.members.iter_mut().find(|x| &x.id == id) { x.hand = *h; } }
        Ctl::Remove(id) => r.members.retain(|x| &x.id != id),
        Ctl::Invite(a) => r.members.push(Member { id: a.clone(), name: a.clone(), email: a.clone(), state: "NOTIFIED".into(), ..Default::default() }),
        Ctl::EndAll => {}
    }
    r.members.sort_by_key(|m| (!m.hand, !m.me, !m.joined(), m.name.to_lowercase()));
}

/// A 28 px icon button (hover fill, focus ring).
fn icon_btn(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, id: impl std::hash::Hash, r: Rect, icon: &'static str, col: Color32, tip: &str) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    if resp.hovered() { ui.painter().rect_filled(r, 4.0, t.hover); }
    if resp.has_focus() { ui.painter().rect_stroke(r.shrink(1.0), 4.0, Stroke::new(2.0_f32, t.focus), egui::StrokeKind::Inside); }
    ic.paint(ui, icon, Rect::from_center_size(r.center(), Vec2::splat(16.0)), col);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

impl App {
    /// The roster button in the call window's control bar: shows / hides the panel.
    pub(super) fn roster_button(&mut self, ui: &mut egui::Ui, t: &Th, center: egui::Pos2) {
        let on = self.call.win.roster.open;
        if round_at(ui, &mut self.icons, "bar-roster", center, 40.0, "participant-list-regular", if on { t.accent } else { t.selected }, Color32::WHITE, t.focus)
            .on_hover_text(if on { tr("Hide participants panel") } else { tr("Participants panel") }).clicked() { self.call.win.roster.open = !on; }
    }

    /// "Raise hand" / "Lower hand" in the control bar (group calls only).
    pub(super) fn hand_button(&mut self, ui: &mut egui::Ui, t: &Th, center: egui::Pos2, now: f64) {
        let Some(me) = self.call.roster.me().cloned() else { return };
        if !self.call.roster.group { return; }
        if round_at(ui, &mut self.icons, "bar-hand", center, 40.0, if me.hand { "lower-hand-regular" } else { "raise-hand-regular" }, if me.hand { t.accent } else { t.selected }, Color32::WHITE, t.focus)
            .on_hover_text(if me.hand { tr("Lower hand") } else { tr("Raise hand") }).clicked() { self.send_control(Ctl::Hand(me.id, !me.hand), now); }
    }

    /// Sends a roster control (the offline sample applies it to its made-up roster).
    pub(crate) fn send_control(&mut self, c: Ctl, now: f64) {
        if self.call.sample { sample_apply(&mut self.call.roster, &c); if c == Ctl::EndAll { self.call.cur = None; } return; }
        let guard = std::env::var(cloud::locus::GUARD_ENV).ok();
        if let Err(e) = cloud::locus::controls::check(&self.call.roster, &c, guard.as_deref()) { self.toast(e, now); return; }
        let invite = matches!(c, Ctl::Invite(_));
        let Some(cl) = &self.client else { return };
        cl.req(Req::Call(CallReq::Control(c)));
        if invite { self.toast(tr("Invite sent"), now); }
    }

    /// The panel in `r` (right side of the call window body).
    pub(super) fn roster_panel(&mut self, ui: &mut egui::Ui, vctx: &egui::Context, t: &Th, r: Rect, now: f64) {
        let mut ctl: Option<Ctl> = None;
        ui.painter().rect_filled(r, 0.0, t.card);
        ui.painter().line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.0_f32, t.sep));
        let roster = self.call.roster.clone();
        // Header: "Participants (n)" and close.
        let head = Rect::from_min_size(r.min, Vec2::new(r.width(), HEAD_H));
        crate::ui::chrome::fit_text(ui.painter(), head.left() + 16.0, head.center().y, &crate::trq!("Participants ({{0}})", roster.joined()), semibold(14.0), t.text1, head.width() - 64.0);
        let close = Rect::from_center_size(pos2(head.right() - 12.0 - BTN / 2.0, head.center().y), Vec2::splat(BTN));
        if icon_btn(ui, &mut self.icons, t, "roster-close", close, "cancel-regular", t.text1, tr("Hide participants panel")).clicked() { self.call.win.roster.open = false; }
        // Invite: a field, known people matching it, "Invite".
        let inv = Rect::from_min_size(pos2(r.left() + 12.0, head.bottom()), Vec2::new(r.width() - 24.0, 32.0));
        let mut field = ui.new_child(egui::UiBuilder::new().max_rect(inv).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let te = field.add(egui::TextEdit::singleline(&mut self.call.win.roster.invite).hint_text(tr("Add People")).desired_width(inv.width() - BTN - 8.0))
            .on_hover_text(tr("Add people by name or email address"));
        let enter = te.lost_focus() && vctx.input(|i| i.key_pressed(egui::Key::Enter));
        let add = Rect::from_center_size(pos2(inv.right() - BTN / 2.0, inv.center().y), Vec2::splat(BTN));
        let add_clicked = icon_btn(ui, &mut self.icons, t, "roster-invite", add, "people-add-regular", t.text1, tr("Invite")).clicked();
        let typed = self.call.win.roster.invite.trim().to_string();
        let sugg: Vec<(String, String)> = invite_suggestions(&self.people, &typed, &roster).into_iter().map(|p| (p.name.clone(), p.email.clone())).collect();
        let mut y = inv.bottom() + 6.0;
        for (i, (name, email)) in sugg.iter().enumerate() {
            let row = Rect::from_min_size(pos2(r.left(), y), Vec2::new(r.width(), 32.0));
            let resp = ui.interact(row, ui.id().with(("roster-sugg", i)), Sense::click());
            if resp.hovered() { ui.painter().rect_filled(row, 0.0, t.hover); }
            crate::ui::chrome::fit_text(ui.painter(), row.left() + 16.0, row.center().y, &format!("{name} · {email}"), FontId::proportional(13.0), t.text1, row.width() - 32.0);
            if resp.on_hover_text(tr("Invite")).clicked() { ctl = Some(Ctl::Invite(email.clone())); }
            y += 32.0;
        }
        if (enter || add_clicked) && !typed.is_empty() {
            // a typed name picks the first match; otherwise the text is the address
            ctl = Some(Ctl::Invite(sugg.first().filter(|_| !typed.contains('@')).map_or(typed.clone(), |s| s.1.clone())));
        }
        if matches!(ctl, Some(Ctl::Invite(_))) { self.call.win.roster.invite.clear(); }
        // Footer: the host ends the call for everyone.
        let end_all = roster.me_host && roster.group;
        let foot_h = if end_all { 56.0 } else { 0.0 };
        let list = Rect::from_min_max(pos2(r.left(), y + 6.0), pos2(r.right(), r.bottom() - foot_h));
        // Rows.
        let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(list).layout(egui::Layout::top_down(egui::Align::Min)));
        lu.set_clip_rect(list);
        egui::ScrollArea::vertical().id_salt("roster-list").auto_shrink([false, false]).show(&mut lu, |ui| {
            for m in &roster.members {
                let (row, resp) = ui.allocate_exact_size(Vec2::new(list.width(), ROW_H), Sense::hover());
                if resp.hovered() { ui.painter().rect_filled(row, 0.0, t.hover); }
                let photo = self.photo_for(vctx, "", &m.email);
                let ar = Rect::from_min_size(pos2(row.left() + 16.0, row.center().y - AVATAR / 2.0), Vec2::splat(AVATAR));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(ar));
                avatar_photo(&mut c, &mut self.icons, t, &m.name, AVATAR, false, None, None, photo);
                let actions = member_actions(&roster, m);
                let mut x = row.right() - 12.0;
                let mut slot = || { x -= BTN; Rect::from_min_size(pos2(x, row.center().y - BTN / 2.0), Vec2::splat(BTN)) };
                if !actions.is_empty() {
                    let b = slot();
                    if icon_btn(ui, &mut self.icons, t, ("roster-more", &m.id), b, "more-regular", t.text1, &crate::trq!("More options for %1", m.name)).clicked() {
                        self.call.win.roster.menu_for = Some(m.id.clone());
                        menu_toggle(vctx, MENU);
                    }
                    if self.call.win.roster.menu_for.as_deref() == Some(m.id.as_str()) && super::super::menu::menu_is_open(vctx, MENU) {
                        let items: Vec<Entry> = actions.iter().map(|(id, label, danger)| { let i = Item::new(id, *label); (if *danger { i.danger() } else { i }).entry() }).collect();
                        if let Some(id) = webex_menu(vctx, &mut self.icons, t, MENU, b, Side::Below, Align::Right, true, &items) {
                            if id == "remove" { self.call.win.roster.confirm = Some(Confirm::Remove(m.id.clone(), m.name.clone())); }
                            else { ctl = action_ctl(&id, m); }
                        }
                    }
                }
                if m.joined() {
                    let b = slot();
                    let (icon, col, tip) = if m.muted { ("microphone-muted-regular", t.badge, tr("Muted")) } else { ("microphone-regular", t.text2, "") };
                    t_icon(ui, &mut self.icons, b, icon, col, tip, ("roster-mic", &m.id));
                }
                if m.hand {
                    let b = slot();
                    t_icon(ui, &mut self.icons, b, "raise-hand-filled", t.accent, tr("Raised hand"), ("roster-hand", &m.id));
                }
                let tx = ar.right() + 12.0;
                let w = (x - tx - 4.0).max(20.0);
                let second = second_line(m);
                let p = ui.painter();
                if second.is_empty() {
                    crate::ui::chrome::fit_text(p, tx, row.center().y, &m.name, semibold(13.0), t.text1, w);
                } else {
                    crate::ui::chrome::fit_text(p, tx, row.center().y - 8.0, &m.name, semibold(13.0), t.text1, w);
                    crate::ui::chrome::fit_text(p, tx, row.center().y + 9.0, second, FontId::proportional(12.0), t.text2, w);
                }
            }
        });
        if end_all {
            let b = Rect::from_center_size(pos2(r.center().x, r.bottom() - foot_h / 2.0), Vec2::new(r.width() - 32.0, 36.0));
            let resp = ui.interact(b, ui.id().with("roster-end-all"), Sense::click());
            ui.painter().rect_filled(b, 18.0, if resp.hovered() { t.badge.gamma_multiply(0.85) } else { t.badge });
            ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, tr("End call for everyone"), semibold(13.0), Color32::WHITE);
            if resp.has_focus() { ui.painter().rect_stroke(b.expand(1.5), 19.5, Stroke::new(2.0_f32, t.focus), egui::StrokeKind::Inside); }
            if resp.clicked() { self.call.win.roster.confirm = Some(Confirm::EndAll); }
        }
        // Confirmation (remove / end for all) over the panel's bottom.
        if let Some(cf) = self.call.win.roster.confirm.clone() {
            let (q, ok) = match &cf { Confirm::Remove(_, n) => (crate::trq!("Remove {{0}}?", n), tr("Remove")), Confirm::EndAll => (tr("End the call for everyone?").to_string(), tr("End for everyone")) };
            let bx = Rect::from_min_max(pos2(r.left() + 8.0, r.bottom() - 112.0), pos2(r.right() - 8.0, r.bottom() - 8.0));
            ui.painter().rect_filled(bx, 8.0, t.win_b);
            ui.painter().rect_stroke(bx, 8.0, Stroke::new(1.0_f32, t.sep), egui::StrokeKind::Inside);
            let _ = ui.interact(bx, ui.id().with("roster-confirm-bg"), Sense::click()); // swallow clicks on the list below
            crate::ui::chrome::fit_text(ui.painter(), bx.left() + 16.0, bx.top() + 28.0, &q, semibold(14.0), t.text1, bx.width() - 32.0);
            let bw = 120.0;
            let yes = Rect::from_min_size(pos2(bx.right() - 16.0 - bw, bx.bottom() - 16.0 - 32.0), Vec2::new(bw, 32.0));
            let no = yes.translate(Vec2::new(-bw - 8.0, 0.0));
            for (rect, label, danger) in [(no, tr("Cancel"), false), (yes, ok, true)] {
                let resp = ui.interact(rect, ui.id().with(("roster-confirm", label)), Sense::click());
                let fill = if danger { t.badge } else { t.selected };
                ui.painter().rect_filled(rect, 16.0, if resp.hovered() { fill.gamma_multiply(0.85) } else { fill });
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, semibold(13.0), if danger { Color32::WHITE } else { t.text1 });
                if resp.clicked() {
                    self.call.win.roster.confirm = None;
                    if danger { ctl = Some(match &cf { Confirm::Remove(id, _) => Ctl::Remove(id.clone()), Confirm::EndAll => Ctl::EndAll }); }
                }
            }
        }
        if let Some(c) = ctl { self.send_control(c, now); }
    }
}

/// A state icon (no click) with an optional tooltip.
fn t_icon(ui: &mut egui::Ui, ic: &mut icons::Icons, r: Rect, icon: &'static str, col: Color32, tip: &str, id: impl std::hash::Hash) {
    ic.paint(ui, icon, Rect::from_center_size(r.center(), Vec2::splat(16.0)), col);
    if !tip.is_empty() { ui.interact(r, ui.id().with(id), Sense::hover()).on_hover_text(tip); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(id: &str, me: bool, joined: bool, muted: bool, hand: bool, host: bool) -> Member {
        Member { id: id.into(), name: id.into(), email: format!("{id}@example.com"), state: if joined { "JOINED" } else { "NOTIFIED" }.into(), muted, hand, host, me }
    }

    #[test]
    fn roster_menu_offers_what_the_role_allows() {
        let r = Roster { members: vec![mem("me", true, true, false, false, false), mem("a", false, true, false, true, true)], me_host: false, group: true };
        let ids = |r: &Roster, m: &Member| member_actions(r, m).into_iter().map(|a| a.0).collect::<Vec<_>>();
        assert_eq!(ids(&r, &r.members[0]), vec!["hand-up"], "own row: raise hand only");
        assert_eq!(ids(&r, &r.members[1]), vec!["mute"], "not host and a host exists: no remove, no lowering their hand");
        let host = Roster { me_host: true, ..r.clone() };
        assert_eq!(ids(&host, &host.members[1]), vec!["mute", "hand-down", "remove"]);
        let one2one = Roster { group: false, members: vec![mem("me", true, true, false, false, false), mem("b", false, true, true, false, false)], me_host: false };
        assert_eq!(ids(&one2one, &one2one.members[0]), Vec::<&str>::new(), "no raise hand in a 1:1 call");
        assert_eq!(ids(&one2one, &one2one.members[1]), vec!["unmute", "remove"], "hostless: anyone may remove");
    }

    #[test]
    fn second_line_says_role_and_ringing() {
        assert_eq!(second_line(&mem("x", true, true, false, false, true)), tr("Host, me"));
        assert_eq!(second_line(&mem("x", true, true, false, false, false)), tr("Me"));
        assert_eq!(second_line(&mem("x", false, true, false, false, true)), tr("Host"));
        assert_eq!(second_line(&mem("x", false, false, false, false, false)), tr("Calling..."));
    }

    #[test]
    fn invite_suggestions_skip_people_already_in_the_call() {
        let people = vec![
            cloud::Person { id: "1".into(), name: "Max Muster".into(), email: "max@example.com".into(), status: String::new() },
            cloud::Person { id: "2".into(), name: "Maxi Probe".into(), email: "maxi@example.com".into(), status: String::new() },
        ];
        let r = Roster { members: vec![Member { email: "max@example.com".into(), ..Default::default() }], ..Default::default() };
        let s: Vec<_> = invite_suggestions(&people, "max", &r).into_iter().map(|p| p.email.as_str()).collect();
        assert_eq!(s, vec!["maxi@example.com"]);
        assert!(invite_suggestions(&people, "m", &r).is_empty(), "one letter suggests nothing");
    }

    #[test]
    fn sample_roster_controls_apply_locally() {
        let mut r = sample_roster();
        assert_eq!(r.members[0].name, "Erika Beispiel", "raised hand first");
        assert!(r.members[1].me);
        sample_apply(&mut r, &Ctl::Hand("p3".into(), false));
        assert!(r.members[0].me, "after lowering her hand we are first");
        sample_apply(&mut r, &Ctl::Remove("p2".into()));
        assert!(r.get("p2").is_none());
    }
}
