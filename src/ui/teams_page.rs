//! "Teams" (list of teams, team detail with Bereiche / Mitglieder tabs, "Ein Team erstellen") and "Whiteboards" (header with
//! "Neues Whiteboard", search "Whiteboards durchsuchen", "Sortieren nach:", grid / list; empty state
//! `whiteboards-192-cobalt-lime`). German strings from the pack.
//! Teams (requests in cloud/spaces_ext/): create (480 wide centred column, title, name, description, people search with 32 pt
//! avatar chips, Erstellen enabled once a name is typed / Abbrechen), the team's spaces incl. the ones I have not joined
//! ("Beitreten"), "Bereich für dieses Team erstellen", members of the general space, "Teammitglieder hinzufügen ...",
//! "Team archivieren" (with its confirmation), "Team verlassen" ("Team verlassen?"), and the "Archivierte Teams" section.
//! Not implemented yet: the whiteboard list and whiteboard creation (Whiteboards only show a notice).
#![allow(clippy::too_many_arguments)]
use crate::*;
use super::menu::{self, Entry, Item};
use super::page_kit::*;
use super::spaces_ext::{self as sx, Dlg};

/// The create-team form's people (kept beside PageState.new_team, which only holds name + description).
fn new_team_people() -> egui::Id { egui::Id::new("new-team-people") }

impl App {
    pub(crate) fn teams_list(&mut self, ui: &mut egui::Ui, t: &Th, _now: f64) {
        let ctx = ui.ctx().clone();
        let mut st = self.pst(&ctx);
        let (plus, _) = self.list_header(ui, t, tr("Teams"), Some("Team erstellen"), None);
        if plus { st.new_team = Some((String::new(), String::new())); st.team_open = None; ctx.data_mut(|d| d.remove::<(Vec<Person>, String)>(new_team_people())); }
        ui.add_space(8.0);
        ui.spacing_mut().scroll = egui::style::ScrollStyle { bar_width: 10.0, bar_inner_margin: 2.0, bar_outer_margin: 0.0, ..egui::style::ScrollStyle::solid() };
        let mut menu_req: Option<(String, Rect)> = None;
        let (archived, active): (Vec<Team>, Vec<Team>) = self.teams.clone().into_iter().partition(|tm| self.team_archived(&tm.id));
        crate::ui::widgets::vscroll().auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            for (head, list) in [(None, &active), (Some("Archivierte Teams"), &archived)] { // "Archived Teams"
                if list.is_empty() { continue; }
                if let Some(h) = head { ui.add_space(10.0); ui.horizontal(|ui| { ui.add_space(16.0); ui.label(RichText::new(h).size(13.0).color(t.text2)); }); ui.add_space(4.0); }
                for tm in list.iter() {
                    let n = self.spaces.iter().filter(|s| s.team == tm.id).count();
                    let sub = format!("{n} Bereiche");
                    let mut spec = RowSpec::new(format!("team-{}", tm.id), &tm.name);
                    spec.sub = &sub; spec.space = true; spec.av_bg = Some(team_tones(t, tm.color, &tm.id).0); spec.selected = st.team_open.as_deref() == Some(tm.id.as_str());
                    let out = self.page_row(ui, t, &spec);
                    if out.clicked { st.team_open = Some(tm.id.clone()); st.new_team = None; }
                    if let Some(r) = out.menu { st.team_open = Some(tm.id.clone()); menu_req = Some((tm.id.clone(), r)); }
                }
            }
            if active.is_empty() && archived.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| { ui.label(RichText::new("Keine Teams").font(semibold(15.0)).color(t.text1)); }); // Webox's own wording
            }
        });
        self.set_pst(&ctx, st);
        if let Some((k, r)) = menu_req { self.open_page_menu(&ctx, "team-row", &k, r); }
    }

    /// "Ein Team erstellen". Returns true while the form is shown.
    fn create_team_view(&mut self, ui: &mut egui::Ui, t: &Th, st: &mut PageState) -> bool {
        let Some((mut name, mut desc)) = st.new_team.clone() else { return false };
        let ctx = ui.ctx().clone();
        let (mut people, mut q): (Vec<Person>, String) = ctx.data(|d| d.get_temp(new_team_people())).unwrap_or_default();
        let w = 480.0_f32.min(ui.max_rect().width() - 48.0);
        let area = Rect::from_min_size(pos2(ui.max_rect().center().x - w / 2.0, ui.max_rect().top() + 48.0), Vec2::new(w, ui.max_rect().height() - 72.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        c.spacing_mut().item_spacing.y = 8.0;
        c.vertical_centered(|ui| ui.label(RichText::new("Ein Team erstellen").font(semibold(22.0)).color(t.text1))); // "Create a team"
        c.add_space(20.0);
        c.label(RichText::new("Teamname").size(14.0).color(t.text1)); // "Team Name"
        c.add(egui::TextEdit::singleline(&mut name).hint_text("Team benennen (erforderlich)").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
        c.add_space(8.0);
        c.label(RichText::new("Teambeschreibung").size(14.0).color(t.text1)); // "Team Description"
        c.add(egui::TextEdit::singleline(&mut desc).hint_text("Team beschreiben").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
        c.add_space(8.0);
        c.label(RichText::new("Mitglieder zu diesem Team hinzufügen").size(14.0).color(t.text1)); // "Add members to this team"
        c.add(egui::TextEdit::singleline(&mut q).hint_text("Nach einem Namen suchen").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
        let ql = q.trim().to_lowercase();
        if !ql.is_empty() {
            let hits: Vec<Person> = self.people.iter().filter(|p| !people.iter().any(|x| x.id == p.id) && (p.name.to_lowercase().contains(&ql) || p.email.to_lowercase().contains(&ql))).take(5).cloned().collect();
            for p in hits {
                let (r, resp) = c.allocate_exact_size(Vec2::new(w, 30.0), Sense::click());
                if resp.hovered() { c.painter().rect_filled(r, 15.0, t.hover); c.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                c.painter().text(pos2(r.left() + 12.0, r.center().y), egui::Align2::LEFT_CENTER, format!("{}   {}", p.name, p.email), FontId::proportional(13.5), t.text1);
                if resp.clicked() { people.push(p); q.clear(); }
            }
        }
        // Avatar grid: cells 74 x 84 (50 + 24 spacing / 60 + 24), avatar 32, name under it, dismiss on hover.
        if !people.is_empty() {
            let mut remove = None;
            let cols = ((w / 74.0) as usize).max(1);
            let rows = people.len().div_ceil(cols);
            let (grid, _) = c.allocate_exact_size(Vec2::new(w, 84.0 * rows.min(2) as f32), Sense::hover());
            for (i, p) in people.iter().enumerate().take(cols * 2) {
                let cell = Rect::from_min_size(grid.min + Vec2::new(74.0 * (i % cols) as f32, 84.0 * (i / cols) as f32), Vec2::new(74.0, 84.0));
                let av = Rect::from_center_size(pos2(cell.center().x, cell.top() + 22.0), Vec2::splat(32.0));
                let ph = self.photo_for(&ctx, &p.id, &p.email);
                let mut ac = c.new_child(egui::UiBuilder::new().max_rect(av));
                avatar_photo(&mut ac, &mut self.icons, t, &p.name, 32.0, false, None, None, ph);
                let resp = c.interact(av, egui::Id::new(("new-team-chip", &p.id)), Sense::click());
                if resp.hovered() { c.painter().circle_filled(av.center(), 16.0, alpha(Color32::BLACK, 0.55)); self.icons.paint(&c, "cancel-regular", Rect::from_center_size(av.center(), Vec2::splat(12.0)), Color32::WHITE); }
                if resp.clicked() { remove = Some(i); }
                super::page_kit::fit_label(&c, pos2(cell.center().x, av.bottom() + 12.0), &p.name, FontId::proportional(12.5), t.text1, 70.0);
            }
            if let Some(i) = remove { people.remove(i); }
        }
        c.add_space(20.0);
        let (mut create, mut cancel) = (false, false);
        let ready = !name.trim().is_empty();
        c.vertical_centered(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let bw = 92.0 * 2.0 + 8.0;
                ui.add_space(((w - bw) / 2.0).max(0.0));
                create = ui.add_enabled_ui(ready, |ui| self.pill(ui, t, "Erstellen", Pill::Primary, None)).inner.clicked() && ready;
                cancel = self.pill(ui, t, "Abbrechen", Pill::Secondary, None).clicked();
            });
        });
        if create {
            let id = self.create_team(name.trim(), desc.trim(), &people);
            st.new_team = None;
            st.team_open = id;
            ctx.data_mut(|d| d.remove::<(Vec<Person>, String)>(new_team_people()));
            return true;
        }
        if cancel { st.new_team = None; ctx.data_mut(|d| d.remove::<(Vec<Person>, String)>(new_team_people())); return true; }
        st.new_team = Some((name, desc));
        ctx.data_mut(|d| d.insert_temp(new_team_people(), (people, q)));
        true
    }

    pub(crate) fn teams_detail(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let mut st = self.pst(&ctx);
        if self.create_team_view(ui, t, &mut st) { self.set_pst(&ctx, st); return; }
        let Some(tm) = st.team_open.as_ref().and_then(|id| self.teams.iter().find(|x| &x.id == id).cloned()) else {
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
            let clicked = self.empty_state(&mut c, t, "people-chat-320-cobalt-lime", 200.0, "Teams", "Wählen Sie ein Team aus der Liste aus oder erstellen Sie ein neues.", Some("Team erstellen")); // Webox's own wording
            if clicked { st.new_team = Some((String::new(), String::new())); self.set_pst(&ctx, st); }
            return;
        };
        let archived = self.team_archived(&tm.id);
        let spaces = self.team_spaces(&tm.id);
        let area = ui.max_rect().shrink2(Vec2::new(24.0, 28.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        let mut new_space = false;
        let mut add_members = false;
        c.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            avatar_photo(ui, &mut self.icons, t, &tm.name, 56.0, true, Some(team_tones(t, tm.color, &tm.id).0), None, None);
            ui.vertical(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(&tm.name).font(semibold(22.0)).color(t.text1));
                let sub = if archived { "Archiviert".to_string() } else { format!("{} Bereiche", spaces.iter().filter(|s| s.joined).count()) };
                ui.label(RichText::new(sub).size(14.0).color(t.text2));
            });
            if !archived {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if st.team_tab == 0 { new_space = self.pill(ui, t, "Bereich für dieses Team erstellen", Pill::Primary, Some("plus-regular")).clicked(); } // "Create a space for this team"
                    else { add_members = self.pill(ui, t, "Teammitglieder hinzufügen", Pill::Primary, Some("people-add-regular")).clicked(); } // "Add team members"
                });
            }
        });
        c.add_space(14.0);
        c.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (i, n) in ["Bereiche", "Mitglieder"].iter().enumerate() {
                if text_tab(ui, n, st.team_tab as usize == i, t, t.tab_on).clicked() { st.team_tab = i as u8; }
            }
        });
        c.add_space(8.0);
        let mut open: Option<String> = None;
        let mut join: Option<cloud::spaces_ext::TeamSpace> = None;
        let mut card: Option<Person> = None;
        crate::ui::widgets::vscroll().auto_shrink([false; 2]).show(&mut c, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            if st.team_tab == 0 {
                for ts in &spaces {
                    let sp = self.spaces.iter().find(|s| s.id == ts.id).cloned();
                    let mut spec = RowSpec::new(format!("tsp-{}", ts.id), &ts.title);
                    spec.space = true;
                    if let Some(s) = &sp { spec.bold = s.unread; spec.dot = s.unread; spec.right = &s.last; }
                    else { spec.sub = "Diesem Bereich beitreten"; spec.action = Some(("enter-room-regular", "Beitreten")); } // "Join this space" / "Join"
                    let out = self.page_row(ui, t, &spec);
                    if ts.joined || sp.is_some() { if out.clicked || out.double { open = Some(ts.id.clone()); } }
                    else if out.action || out.double { join = Some(ts.clone()); }
                }
                if spaces.is_empty() { ui.add_space(24.0); ui.vertical_centered(|ui| ui.label(RichText::new("In diesem Team gibt es noch keine Bereiche.").color(t.text2))); } // Webox's own wording
            } else {
                // Members: the participants of the team's general space (members live there); else everyone seen in its spaces.
                let general = self.team_info(&tm.id).map(|i| i.general).unwrap_or_default();
                let mut members: Vec<Person> = self.spaces.iter().find(|s| s.id == general).map(|s| s.participants.clone()).unwrap_or_default();
                if members.is_empty() {
                    let mut seen = HashSet::new();
                    for s in self.spaces.iter().filter(|s| s.team == tm.id) { for p in &s.participants { if seen.insert(p.email.clone() + &p.id) { members.push(p.clone()); } } }
                }
                members.sort_by_key(|p| p.name.to_lowercase());
                for p in &members {
                    let pp = self.live_person(p);
                    let photo = self.photo_for(ui.ctx(), &pp.id, &pp.email);
                    let sub = self.person_line(&pp, super::chrome::now_ms());
                    let mut spec = RowSpec::new(format!("tm-{}{}", pp.id, pp.email), &pp.name);
                    spec.sub = &sub; spec.status = Some(&pp.status); spec.photo = photo;
                    let out = self.page_row(ui, t, &spec);
                    if out.clicked || out.double { card = Some(pp.clone()); }
                }
                if members.is_empty() { ui.add_space(24.0); ui.vertical_centered(|ui| ui.label(RichText::new("Keine Mitglieder bekannt.").color(t.text2))); } // Webox's own wording
            }
        });
        self.set_pst(&ctx, st);
        let _ = now;
        if let Some(p) = card { Self::contact_card_open(&ctx, &p); }
        if let Some(ts) = join { self.join_team_space(&tm.id, &ts); }
        if add_members { sx::open_dialog(Dlg::TeamMembers(tm.id.clone(), String::new())); }
        if new_space { self.create_space_in_team(&ctx, &tm.id); }
        if let Some(id) = open { self.team_sel = Some(tm.id.clone()); self.open_space(id); }
    }

    pub(crate) fn teams_menu(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let target = self.pst(ctx).menu_target.map(|m| m.0).unwrap_or_default();
        let archived = self.team_archived(&target);
        let items = vec![
            Item::new("add", "Teammitglieder hinzufügen ...").icon("people-add-regular").enabled(!archived).entry(), Entry::Sep,
            Item::new("archive", "Team archivieren").icon("webex-teams-remove-regular").enabled(!archived).entry(), Item::new("leave", "Team verlassen").icon("exit-room-regular").danger().entry(),
        ];
        if let Some((team, pick)) = self.page_menu(ctx, t, "team-row", &items) {
            match pick.as_str() {
                "add" => sx::open_dialog(Dlg::TeamMembers(team, String::new())),
                "archive" => sx::open_dialog(Dlg::ArchiveTeam(team)),
                "leave" => sx::open_dialog(Dlg::TeamLeave(team)),
                _ => {}
            }
        }
        let _ = menu::light(t);
        self.spaces_ext_dialogs(ctx, t, now);
    }

    // Whiteboards.
    /// Drawn in a single card (see `pages_frame`): header row, then a grid with the "Neues Whiteboard" tile.
    pub(crate) fn whiteboards_page(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let mut st = self.pst(&ctx);
        let mut new_wb = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.label(RichText::new(tr("Whiteboards")).font(semibold(21.0)).color(t.text1));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                new_wb = self.pill(ui, t, "Neues Whiteboard", Pill::Primary, Some("plus-regular")).on_hover_text("Neues Whiteboard erstellen").clicked();
            });
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (r, _) = ui.allocate_exact_size(Vec2::new(280.0_f32.min(ui.available_width() - 200.0).max(160.0), 32.0), Sense::hover());
            ui.painter().rect_filled(r, 16.0, t.ink_a(0.07));
            self.icons.paint(ui, "search-regular", Rect::from_center_size(r.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(16.0)), t.text2);
            let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(r.min + Vec2::new(34.0, 0.0), r.max - Vec2::new(10.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            inner.add(egui::TextEdit::singleline(&mut st.wb_q).hint_text(RichText::new("Whiteboards durchsuchen").color(t.text2).size(14.5)).font(FontId::proportional(14.5)).frame(false).desired_width(r.width() - 48.0));
            ui.label(RichText::new("Sortieren nach:").size(13.5).color(t.text2));
            if text_tab(ui, if st.wb_sort == 0 { "Name" } else { "Zuletzt geändert" }, false, t, t.tab_on).clicked() { st.wb_sort = 1 - st.wb_sort; } // "Zuletzt geändert": Webox's own wording
        });
        ui.add_space(16.0);
        // Grid: the first tile is always "Neues Whiteboard". No whiteboards come from the data layer yet.
        let tile = Vec2::new(200.0, 140.0);
        let (r, _) = ui.allocate_exact_size(tile, Sense::hover());
        let resp = ui.interact(r, ui.id().with("wb-new"), Sense::click());
        ui.painter().rect_filled(r, 12.0, if resp.hovered() { t.ink_a(0.12) } else { t.ink_a(0.07) });
        ui.painter().rect_stroke(r, 12.0, Stroke::new(1.0, t.ink_a(0.2)), egui::StrokeKind::Inside);
        if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        self.icons.paint(ui, "plus-circle-regular", Rect::from_center_size(r.center() - Vec2::new(0.0, 12.0), Vec2::splat(32.0)), t.text1);
        ui.painter().text(r.center() + Vec2::new(0.0, 28.0), egui::Align2::CENTER_CENTER, "Neues Whiteboard", semibold(14.0), t.text1);
        if resp.clicked() { new_wb = true; }
        let rest = Rect::from_min_max(pos2(r.right() + 20.0, r.top()), pos2(ui.max_rect().right(), ui.max_rect().bottom()));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rest));
        self.empty_state(&mut c, t, "whiteboards-192-cobalt-lime", 150.0, "Noch keine Whiteboards",
            "Hier finden Sie Ihre gespeicherten Whiteboards. Über ein Whiteboard können Sie mit anderen Ideen entwerfen und skizzieren.", None); // title: Webox's own wording
        if new_wb { self.toast("Whiteboards sind in dieser Version nicht verfügbar", now); } // Webox's own wording
        self.set_pst(&ctx, st);
    }
}
