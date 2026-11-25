//! App side of the space / people / team actions (requests: cloud/spaces_ext/) and their dialogs:
//! "Space notifications", "Add to a team", "Remove from team?", "Leave the team?",
//! "Add team members". Signed in, each action sends one request on the user's click (never by itself); with sample
//! data (`--sample`) it changes the local copy so every control shows its result offline.
//!
//! State that has no field in the shared `App` / `Space` structs (notification setting, moderators, full people lists,
//! team facts, open dialog) lives in a thread-local store here.
use crate::*;
use crate::cloud::spaces_ext::{Notify, SpaceEvent, SpaceOp, TeamInfo, TeamSpace};
use super::menu::{self, Button};
use std::cell::RefCell;

/// The space-link format ("webexteams://im?space=<id>"; `system::space_from_uri` reads it back).
pub(crate) fn space_link(id: &str) -> String { format!("webexteams://im?space={id}") }

#[derive(Clone)]
pub(crate) enum Dlg {
    /// (space, selected option 0 Global, 1 All, 2 @Mentions, 3 Off, 4 Off and hide)
    Notify(String, usize),
    TeamPick(String, Option<String>),
    TeamRemove(String, String),
    TeamLeave(String),
    TeamMembers(String, String),
    ArchiveSpace(String),
    ArchiveTeam(String),
}

#[derive(Default)]
pub(crate) struct Extras {
    pub notify: HashMap<String, Notify>,
    pub moderators: HashMap<String, HashSet<String>>,
    pub locked: HashSet<String>,
    pub teams: HashMap<String, TeamInfo>,
    pub team_spaces: HashMap<String, Vec<TeamSpace>>,
    pub team_spaces_asked: HashSet<String>,
    pub people: HashMap<String, Vec<Person>>,
    pub people_asked: HashSet<String>,
    pub dlg: Option<Dlg>,
    pub drawn_pass: u64,
    seeded: bool,
}

thread_local! { static X: RefCell<Extras> = RefCell::new(Extras::default()); }
pub(crate) fn with<R>(f: impl FnOnce(&mut Extras) -> R) -> R { X.with(|x| f(&mut x.borrow_mut())) }
pub(crate) fn open_dialog(d: Dlg) { with(|x| x.dlg = Some(d)); }

/// Same person: id, or e-mail (participants carry e-mail-style ids next to uuids).
fn same(a: &Person, b: &Person) -> bool {
    (!a.id.is_empty() && a.id == b.id) || (!a.email.is_empty() && a.email.eq_ignore_ascii_case(&b.email))
}

impl App {
    fn op(&self, op: SpaceOp) -> bool { match &self.client { Some(c) => { c.req(Req::SpaceOp(op)); true } None => false } }
    fn sample(&self) -> bool { self.client.is_none() }

    /// Offline sample: a moderator, a space with "@Mentions only", the team's general space (once).
    fn seed_sample(&self) {
        if !self.sample() { return; }
        let me = self.me.id.clone();
        with(|x| {
            if x.seeded { return; }
            x.seeded = true;
            x.moderators.entry("s1".into()).or_default().insert(me);
            x.notify.insert("s6".into(), Notify::Mentions);
            x.teams.insert("t3".into(), TeamInfo { id: "t3".into(), general: "s7".into(), url: String::new(), archived: false });
        });
    }

    pub(crate) fn on_space_event(&mut self, e: SpaceEvent) {
        match e {
            SpaceEvent::Info(infos, teams) => with(|x| {
                for i in infos {
                    x.notify.insert(i.id.clone(), i.notify);
                    if i.locked { x.locked.insert(i.id.clone()); } else { x.locked.remove(&i.id); }
                    x.moderators.insert(i.id, i.moderators.into_iter().collect());
                }
                x.teams = teams.into_iter().map(|t| (t.id.clone(), t)).collect();
            }),
            SpaceEvent::People(id, list, mods) => with(|x| { x.people.insert(id.clone(), list); x.moderators.insert(id, mods.into_iter().collect()); }),
            SpaceEvent::TeamSpaces(team, list) => with(|x| { x.team_spaces.insert(team, list); }),
        }
    }

    // Read state / notifications.
    pub(crate) fn mark_space_unread(&mut self, id: &str) {
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == id) { s.unread = true; }
        if self.selected.as_deref() == Some(id) { self.msgx.unread_hold = Some(id.to_string()); } // as for "mark unread": no receipt while it stays open
        self.op(SpaceOp::MarkUnread(id.to_string()));
    }
    pub(crate) fn mark_read(&mut self, id: &str) {
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == id) { s.unread = false; }
        self.op(SpaceOp::MarkRead(id.to_string()));
    }
    pub(crate) fn notify_of(&self, id: &str) -> Notify { self.seed_sample(); with(|x| x.notify.get(id).copied().unwrap_or_default()) }
    pub(crate) fn set_notify(&mut self, id: &str, n: Notify) {
        with(|x| { x.notify.insert(id.to_string(), n); });
        self.op(SpaceOp::Notify(id.to_string(), n, false));
    }
    /// "Mute notifications" / "Unmute notifications".
    pub(crate) fn toggle_mute(&mut self, id: &str, now: f64) {
        let muted = self.notify_of(id).muted();
        self.set_notify(id, if muted { Notify::Global } else { Notify::Off });
        if !muted { self.toast(tr("Notifications are muted for this space"), now); }
    }

    // People.
    /// Full participant list once per session (the space list carries 10); falls back to the list's participants.
    pub(crate) fn space_people(&mut self, space: &Space) -> Vec<Person> {
        let asked = with(|x| !x.people_asked.insert(space.id.clone()));
        if !asked && !space.direct { self.op(SpaceOp::People(space.id.clone())); }
        with(|x| x.people.get(&space.id).cloned()).filter(|l| !l.is_empty()).unwrap_or_else(|| space.participants.clone())
    }
    pub(crate) fn is_moderator(&self, space: &str, p: &Person) -> bool {
        self.seed_sample();
        with(|x| x.moderators.get(space).is_some_and(|m| m.contains(&p.id) || (!p.email.is_empty() && m.iter().any(|e| e.eq_ignore_ascii_case(&p.email)))))
    }
    pub(crate) fn i_moderate(&self, space: &str) -> bool { let me = self.me.clone(); self.is_moderator(space, &me) }
    /// Whether the space is moderated (any moderator known) or locked.
    pub(crate) fn moderated(&self, space: &str) -> bool { self.seed_sample(); with(|x| x.locked.contains(space) || x.moderators.get(space).is_some_and(|m| !m.is_empty())) }

    pub(crate) fn remove_person(&mut self, space: &str, p: &Person) {
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == space) { s.participants.retain(|x| !same(x, p)); }
        with(|x| if let Some(l) = x.people.get_mut(space) { l.retain(|x| !same(x, p)); });
        self.op(SpaceOp::RemovePerson(space.to_string(), p.id.clone()));
    }
    pub(crate) fn set_moderator(&mut self, space: &str, p: &Person, on: bool) {
        with(|x| {
            let m = x.moderators.entry(space.to_string()).or_default();
            if on { m.insert(p.id.clone()); } else { m.remove(&p.id); m.remove(&p.email); }
        });
        self.op(SpaceOp::Moderator(space.to_string(), p.id.clone(), on));
    }

    /// "Message" / "Chat" for a person: opens the existing 1:1. Without one, sample data creates a local chat; signed in,
    /// `Req::Direct` (cloud/header_plus.rs `direct_with`: conversation lookup, created when not found, no text sent) runs on this
    /// click and the 1:1 opens once the reloaded list has it (`finish_direct`, the same path as "New message").
    pub(crate) fn open_or_create_direct(&mut self, p: &Person, now: f64) -> bool {
        if self.nav_open_chat(p) { return true; }
        if self.sample() {
            let id = format!("local-dm-{}", if p.email.is_empty() { &p.id } else { &p.email });
            self.spaces.insert(0, Space { id: id.clone(), title: p.name.clone(), direct: true, participants: vec![self.me.clone(), p.clone()], ..Default::default() });
            self.messages.insert(id.clone(), vec![]);
            self.tab = Tab::Messaging; self.team_sel = None;
            self.open_space(id);
            return true;
        }
        let who = if p.id.is_empty() { p.email.clone() } else { p.id.clone() };
        let Some(c) = &self.client else { return false };
        if who.is_empty() { return false; }
        c.req(Req::Direct { who, text: String::new(), invite: false });
        self.expect_direct_person(p);
        self.tab = Tab::Messaging; self.team_sel = None;
        let _ = now;
        true
    }

    // Archive / link.
    pub(crate) fn archive_space(&mut self, id: &str) {
        self.op(SpaceOp::Archive(id.to_string()));
        self.spaces.retain(|s| s.id != id);
        if self.selected.as_deref() == Some(id) { self.selected = None; } // nothing selected -> the create-space pane (no other space is opened or read)
    }

    // Teams.
    pub(crate) fn team_info(&self, team: &str) -> Option<TeamInfo> { self.seed_sample(); with(|x| x.teams.get(team).cloned()) }
    pub(crate) fn team_archived(&self, team: &str) -> bool { self.team_info(team).is_some_and(|t| t.archived) }

    pub(crate) fn add_space_to_team(&mut self, space: &str, team: &str) {
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == space) { s.team = team.to_string(); }
        self.op(SpaceOp::TeamAddSpace { team: team.to_string(), space: space.to_string() });
    }
    pub(crate) fn remove_space_from_team(&mut self, space: &str, team: &str) {
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == space) { s.team.clear(); }
        self.op(SpaceOp::TeamRemoveSpace { team: team.to_string(), space: space.to_string() });
    }
    pub(crate) fn create_team(&mut self, name: &str, summary: &str, people: &[Person]) -> Option<String> {
        if self.op(SpaceOp::TeamCreate { name: name.into(), summary: summary.into(), people: people.iter().map(|p| p.id.clone()).collect() }) { return None; }
        // Sample: the team and its general space ("General" in the sample list is the general space of "Product Team").
        let id = format!("local-team-{}", self.teams.len());
        let general = format!("{id}-general");
        self.teams.push(Team { id: id.clone(), name: name.to_string(), color: None });
        let mut parts = vec![self.me.clone()]; parts.extend(people.iter().cloned());
        self.spaces.insert(0, Space { id: general.clone(), title: name.to_string(), team: id.clone(), participants: parts, last: "jetzt".into(), ..Default::default() });
        self.messages.insert(general.clone(), vec![]);
        let me = self.me.id.clone();
        with(|x| { x.teams.insert(id.clone(), TeamInfo { id: id.clone(), general: general.clone(), url: String::new(), archived: false }); x.moderators.entry(general).or_default().insert(me); });
        Some(id)
    }
    pub(crate) fn leave_team(&mut self, team: &str) {
        self.op(SpaceOp::TeamLeave(team.to_string()));
        // "Leaving the team removes you from all of its spaces." (QObject)
        self.spaces.retain(|s| s.team != team);
        self.teams.retain(|t| t.id != team);
        if self.selected.as_ref().is_some_and(|id| !self.spaces.iter().any(|s| &s.id == id)) { self.selected = None; } // nothing selected -> the create-space pane
        if self.team_sel.as_deref() == Some(team) { self.team_sel = None; }
    }
    pub(crate) fn archive_team(&mut self, team: &str) {
        self.op(SpaceOp::TeamArchive(team.to_string()));
        with(|x| { let t = x.teams.entry(team.to_string()).or_insert_with(|| TeamInfo { id: team.to_string(), ..Default::default() }); t.archived = true; });
    }
    pub(crate) fn add_team_member(&mut self, team: &str, p: &Person) {
        let general = self.team_info(team).map(|t| t.general).unwrap_or_default();
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == general) { if !s.participants.iter().any(|x| same(x, p)) { s.participants.push(p.clone()); } }
        self.op(SpaceOp::TeamAddMember { team: team.to_string(), person: p.id.clone() });
    }
    /// The team's spaces: joined ones from the list plus (signed in) the team's other spaces from the service, asked once.
    pub(crate) fn team_spaces(&mut self, team: &str) -> Vec<TeamSpace> {
        let asked = with(|x| !x.team_spaces_asked.insert(team.to_string()));
        if !asked { self.op(SpaceOp::TeamSpaces(team.to_string())); }
        let mut v: Vec<TeamSpace> = self.spaces.iter().filter(|s| s.team == team && !s.hidden).map(|s| TeamSpace { id: s.id.clone(), title: s.title.clone(), joined: true }).collect();
        let more = with(|x| x.team_spaces.get(team).cloned()).unwrap_or_else(|| if self.sample() && team == "t3" {
            vec![TeamSpace { id: "t3-brainstorm".into(), title: "Brainstorming".into(), joined: false }] // sample: a team space I am not in
        } else { vec![] });
        for s in more { if !v.iter().any(|x| x.id == s.id) { v.push(s); } }
        v
    }
    pub(crate) fn join_team_space(&mut self, team: &str, ts: &TeamSpace) {
        if self.op(SpaceOp::TeamJoin { team: team.to_string(), space: ts.id.clone() }) { return; }
        self.spaces.insert(0, Space { id: ts.id.clone(), title: ts.title.clone(), team: team.to_string(), participants: vec![self.me.clone()], last: "jetzt".into(), ..Default::default() });
        self.messages.entry(ts.id.clone()).or_default();
    }

    /// Create a group space with its people (and team) in one request; sample: a local space. Returns the local id (sample).
    pub(crate) fn create_space_with(&mut self, title: &str, people: &[Person], team: Option<&str>, public: bool) -> Option<String> {
        let ids: Vec<String> = people.iter().map(|p| p.id.clone()).collect();
        let sent = match team {
            Some(tm) => self.op(SpaceOp::TeamCreateSpace { team: tm.to_string(), title: title.into(), people: ids }),
            None => self.op(SpaceOp::CreateWith { title: title.into(), people: ids }),
        };
        if sent { return None; }
        let sid = format!("local{}", self.spaces.len());
        let mut parts = vec![self.me.clone()]; parts.extend(people.iter().cloned());
        self.spaces.insert(0, Space { id: sid.clone(), title: title.into(), public, team: team.unwrap_or("").into(), participants: parts, ..Default::default() });
        self.messages.insert(sid.clone(), vec![]);
        Some(sid)
    }

    // Dialogs.
    /// Draws the open dialog of this module (called from the messaging screen and the Teams page; once per frame).
    pub(crate) fn spaces_ext_dialogs(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let pass = ctx.cumulative_pass_nr();
        if with(|x| { let dup = x.drawn_pass == pass; x.drawn_pass = pass; dup }) { return; }
        let Some(d) = with(|x| x.dlg.clone()) else { return };
        let light = menu::light(t);
        let (text, text2) = if light { (alpha(Color32::BLACK, 0.95), alpha(Color32::BLACK, 0.7)) } else { (alpha(Color32::WHITE, 0.95), alpha(Color32::WHITE, 0.7)) };
        let mut keep: Option<Dlg> = None;
        match d {
            Dlg::Notify(space, mut sel) => {
                let title = self.spaces.iter().find(|s| s.id == space).map(|s| s.title.clone()).unwrap_or_default();
                let mut settings = false;
                let mut body = |ui: &mut egui::Ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let radio = |ui: &mut egui::Ui, i: usize, label: &str, sel: &mut usize| {
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
                        let c = pos2(r.left() + 12.0, r.center().y);
                        ui.painter().circle_stroke(c, 8.0, Stroke::new(1.0_f32, if *sel == i { text } else { text2 }));
                        if *sel == i { ui.painter().circle_filled(c, 4.5, text); }
                        ui.painter().text(pos2(r.left() + 28.0, r.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(14.5), text);
                        if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                        if resp.clicked() { *sel = i; }
                    };
                    let note = |ui: &mut egui::Ui, s: &str, left: f32| { ui.horizontal(|ui| { ui.add_space(left); ui.add(egui::Label::new(RichText::new(s).size(13.0).color(text2)).wrap()); }); };
                    // Notification dialog strings (pack sources).
                    radio(ui, 0, tr("Global"), &mut sel);
                    note(ui, tr("You can change your notification preferences for all your spaces in Settings."), 20.0);
                    ui.horizontal(|ui| { ui.add_space(32.0); if ui.add(egui::Button::new(RichText::new(tr("Settings")).size(13.0))).clicked() { settings = true; } });
                    ui.add_space(16.0);
                    ui.label(RichText::new(tr("Custom")).size(13.0).color(text));
                    ui.add_space(6.0);
                    note(ui, tr("You can choose custom notifications or turn off all notifications for this space."), 0.0);
                    radio(ui, 1, tr("All messages"), &mut sel);
                    radio(ui, 2, tr("@Mentions only"), &mut sel);
                    radio(ui, 3, tr("Off"), &mut sel);
                    radio(ui, 4, tr("Off and Hide Space"), &mut sel);
                    note(ui, tr("This space won't show in your spaces list. It will show in search results, in Teams, or when you filter by @Mentions or Flags."), 23.0);
                };
                // The title wraps; the shared dialog title is one line, so the name goes on its own line.
                let head = if title.is_empty() { tr("Space notifications").to_string() } else { crate::i18n::args(&tr("Space notifications for %1").replace(" %1", "\n%1"), &[&title as &dyn std::fmt::Display]) };
                let res = menu::webex_dialog(ctx, t, "space-notify", &head, "", Some(&mut body), &[Button::primary("save", tr("Save")), Button::secondary("cancel", tr("Cancel"))]);
                if settings { self.set_tab(Tab::Settings); }
                match res.as_deref() {
                    Some("save") => {
                        let n = [Notify::Global, Notify::All, Notify::Mentions, Notify::Off, Notify::Off][sel.min(4)];
                        self.set_notify(&space, n);
                        if sel == 4 { self.hide_space(&space); }
                    }
                    Some(_) => {}
                    None if settings => {}
                    None => keep = Some(Dlg::Notify(space, sel)),
                }
            }
            Dlg::TeamPick(space, mut pick) => {
                let teams: Vec<Team> = self.teams.iter().filter(|tm| !self.team_archived(&tm.id)).cloned().collect();
                let sp_title = self.spaces.iter().find(|s| s.id == space).map(|s| s.title.clone()).unwrap_or_default();
                let tm_name = pick.as_ref().and_then(|p| self.teams.iter().find(|x| &x.id == p)).map(|x| x.name.clone()).unwrap_or_default();
                let info = if tm_name.is_empty() { String::new() } else { crate::trq!("This will allow members of <b>{{0}}</b> to find and join <b>{{1}}</b>, but will not add space participants to the team.", tm_name, sp_title).replace("<b>", "").replace("</b>", "") };
                let mut body = |ui: &mut egui::Ui| {
                    if teams.is_empty() { ui.label(RichText::new(tr("No teams")).size(14.0).color(text2)); } // Webox's own wording (teams_page.rs)
                    for tm in &teams {
                        let on = pick.as_deref() == Some(tm.id.as_str());
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
                        if on || resp.hovered() { ui.painter().rect_filled(r, 8.0, t.ink_a(if on { 0.14 } else { 0.07 })); }
                        let c = pos2(r.left() + 20.0, r.center().y);
                        ui.painter().circle_filled(c, 12.0, team_tones(t, tm.color, &tm.id).0);
                        ui.painter().text(c, egui::Align2::CENTER_CENTER, tm.name.chars().next().unwrap_or('#').to_uppercase().to_string(), semibold(12.0), Color32::WHITE);
                        ui.painter().text(pos2(r.left() + 42.0, r.center().y), egui::Align2::LEFT_CENTER, &tm.name, FontId::proportional(14.5), text);
                        if resp.clicked() { pick = Some(tm.id.clone()); }
                    }
                };
                let res = menu::webex_dialog(ctx, t, "space-team-pick", tr("Add to a team"), &info, Some(&mut body), &[Button::secondary("cancel", tr("Cancel")), Button::primary("add", tr("Add"))]);
                match res.as_deref() {
                    Some("add") => match &pick { Some(tm) => self.add_space_to_team(&space, tm), None => keep = Some(Dlg::TeamPick(space, pick)) },
                    Some(_) => {}
                    None => keep = Some(Dlg::TeamPick(space, pick)),
                }
            }
            Dlg::TeamRemove(space, team) => {
                let res = menu::webex_dialog(ctx, t, "space-team-remove", tr("Remove from team?"), "", None, &[Button::secondary("cancel", tr("Cancel")), Button::danger("remove", tr("Remove"))]);
                match res.as_deref() { Some("remove") => self.remove_space_from_team(&space, &team), Some(_) => {}, None => keep = Some(Dlg::TeamRemove(space, team)) }
            }
            Dlg::TeamLeave(team) => {
                let res = menu::webex_dialog(ctx, t, "team-leave", tr("Leave the team?"), tr("Leaving the team removes you from all of its spaces."), None,
                    &[Button::secondary("cancel", tr("Cancel")), Button::danger("leave", tr("Leave"))]);
                match res.as_deref() { Some("leave") => self.leave_team(&team), Some(_) => {}, None => keep = Some(Dlg::TeamLeave(team)) }
            }
            Dlg::TeamMembers(team, mut q) => {
                let general = self.team_info(&team).map(|x| x.general).unwrap_or_default();
                let members: Vec<Person> = self.spaces.iter().find(|s| s.id == general).map(|s| s.participants.clone()).unwrap_or_default();
                let ql = q.to_lowercase();
                let people: Vec<Person> = self.people.iter().filter(|p| ql.is_empty() || p.name.to_lowercase().contains(&ql) || p.email.to_lowercase().contains(&ql)).cloned().collect();
                let mut picks: Vec<Person> = vec![];
                let mut body = |ui: &mut egui::Ui| {
                    ui.add(egui::TextEdit::singleline(&mut q).hint_text(tr("Search for new team members")).desired_width(f32::INFINITY));
                    ui.add_space(6.0);
                    crate::ui::widgets::vscroll().max_height(260.0).show(ui, |ui| {
                        for p in &people {
                            let already = members.iter().any(|m| same(m, p));
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{}  {}", p.name, p.email)).size(14.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if already { ui.label(RichText::new(tr("Added")).color(text2)); }
                                    else if ui.button(tr("Add")).clicked() { picks.push(p.clone()); }
                                });
                            });
                        }
                    });
                };
                let res = menu::webex_dialog(ctx, t, "team-add-members", tr("Add team members"), "", Some(&mut body), &[Button::primary("done", tr("Done"))]);
                for p in picks { self.add_team_member(&team, &p); }
                if res.is_none() { keep = Some(Dlg::TeamMembers(team, q)); }
            }
            Dlg::ArchiveSpace(space) => {
                let title = self.spaces.iter().find(|s| s.id == space).map(|s| s.title.clone()).unwrap_or_default();
                let body = crate::trq!("Archiving a space will hide it from all team members. Are you sure you want to archive '{{0}}'?", title);
                let res = menu::webex_dialog(ctx, t, "space-archive", tr("Archive space"), &body, None, &[Button::secondary("cancel", tr("Cancel")), Button::danger("archive", tr("Archive space"))]);
                match res.as_deref() { Some("archive") => self.archive_space(&space), Some(_) => {}, None => keep = Some(Dlg::ArchiveSpace(space)) }
            }
            Dlg::ArchiveTeam(team) => {
                let name = self.teams.iter().find(|x| x.id == team).map(|x| x.name.clone()).unwrap_or_default();
                let body = crate::trq!("Archiving a team will hide the team and all its spaces from all team members. Are you sure you want to archive '{{0}}'?", name);
                let res = menu::webex_dialog(ctx, t, "team-archive", tr("Archive team"), &body, None, &[Button::secondary("cancel", tr("Cancel")), Button::danger("archive", tr("Archive team"))]);
                match res.as_deref() { Some("archive") => self.archive_team(&team), Some(_) => {}, None => keep = Some(Dlg::ArchiveTeam(team)) }
            }
        }
        let _ = now;
        with(|x| x.dlg = keep);
    }
}

/// The dialog's radio index of a setting (0 Global, 1 All, 2 @Mentions, 3 Off).
pub(crate) fn self_notify_index(n: Notify) -> usize { match n { Notify::Global => 0, Notify::All => 1, Notify::Mentions => 2, Notify::Off => 3 } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn space_link_is_the_protocol_form_the_app_reads_back() {
        let l = space_link("abc-123");
        assert_eq!(l, "webexteams://im?space=abc-123");
        assert_eq!(crate::system::space_from_uri(&l).as_deref(), Some("abc-123"));
    }
}

mod shots;
#[allow(unused_imports)]
pub(crate) use shots::*;
