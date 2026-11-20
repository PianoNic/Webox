#![allow(clippy::too_many_arguments)]
use crate::*;

/// The cover picture across the chat banner, drawn as the image itself: cropped to fill, rounded top corners (the card's
/// 10 pt), no overlay. Dark.png is used for both themes.
fn cover_banner(p: &egui::Painter, tex: egui::TextureId, r: Rect, uv: Rect, radius: f32, _card: Color32) {
    let mut pts: Vec<egui::Pos2> = Vec::new();
    let steps = 8;
    for i in 0..=steps { let a = std::f32::consts::PI + std::f32::consts::FRAC_PI_2 * i as f32 / steps as f32; pts.push(pos2(r.left() + radius + radius * a.cos(), r.top() + radius + radius * a.sin())); }
    for i in 0..=steps { let a = std::f32::consts::PI * 1.5 + std::f32::consts::FRAC_PI_2 * i as f32 / steps as f32; pts.push(pos2(r.right() - radius + radius * a.cos(), r.top() + radius + radius * a.sin())); }
    pts.push(r.right_bottom());
    pts.push(r.left_bottom());
    let map = |q: egui::Pos2| pos2(egui::lerp(uv.left()..=uv.right(), (q.x - r.left()) / r.width()), egui::lerp(uv.top()..=uv.bottom(), (q.y - r.top()) / r.height()));
    let mut m = egui::Mesh::with_texture(tex);
    let c = r.center();
    m.vertices.push(egui::epaint::Vertex { pos: c, uv: map(c), color: Color32::WHITE });
    for q in &pts { m.vertices.push(egui::epaint::Vertex { pos: *q, uv: map(*q), color: Color32::WHITE }); }
    let n = m.vertices.len() as u32;
    for i in 1..n { m.add_triangle(0, i, i % (n - 1) + 1); }
    p.add(egui::Shape::mesh(m));
}
impl App {
    pub(crate) fn conversation(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        self.shot_script(ui.ctx()); // screenshot-mode test steps only (messages.rs)
        let Some(id) = self.selected.clone() else {
            ui.centered_and_justified(|ui| ui.label(RichText::new(if self.client.is_some() { self.status.as_str() } else { "Pick a space" }).color(t.text2)));
            return;
        };
        let space = self.spaces.iter().find(|s| s.id == id).cloned().unwrap_or_default();
        let person = if space.direct { self.direct_person(&space) } else { None };

        // The content card: theme-gradient-primary (#1A1A1A→#0F0F0F) over the whole panel,
        // rounded top corners, with the banner, messages and composer painted on top.
        let full = ui.max_rect();
        // Only the top-left corner is rounded: the card runs flat into the window's right edge, so it is painted 10 pt wider and
        // its right-hand rounding falls outside the window.
        vgrad(ui.painter(), Rect::from_min_max(full.min, pos2(full.max.x + 10.0, full.max.y)), 10.0, t.conv, t.conv_b);
        let banner = Rect::from_min_size(full.min, Vec2::new(full.width(), 116.0));
        // The banner picture: in a direct chat the other person's own cover (their own, else the global default "@default", via
        // App::cover_for); in a group space only the space's own cover, never the user's; no cover = the flat header. Cropped to
        // fill; textures are cached by the app (photos.rs), nothing is decoded per frame.
        let cover_tex = if space.direct {
            match &person { Some(p) => self.cover_for(ui.ctx(), &p.id, &p.email), None => None }
        } else { self.space_cover_tex(ui.ctx(), &space.id) };
        if let Some((tid, [w, h])) = cover_tex {
            let (ia, ra) = (w as f32 / h.max(1) as f32, banner.width() / banner.height());
            let uv = if ia > ra { let u = ra / ia; Rect::from_min_max(pos2((1.0 - u) / 2.0, 0.0), pos2((1.0 + u) / 2.0, 1.0)) }
                     else { let v = ia / ra; Rect::from_min_max(pos2(0.0, (1.0 - v) / 2.0), pos2(1.0, (1.0 + v) / 2.0)) };
            cover_banner(ui.painter(), tid, banner, uv, 10.0, t.conv);
        }
        let has_cover = cover_tex.is_some(); // title (semibold 20) + favourite star at x=21 / y=28 inside the card
        let title_g = ui.painter().text(pos2(banner.left() + 21.0, banner.top() + 26.0), egui::Align2::LEFT_CENTER, &space.title, semibold(20.0), if has_cover { Color32::WHITE } else { t.text1 });
        let sr = Rect::from_center_size(pos2(title_g.right() + 21.0, banner.top() + 30.0), Vec2::splat(24.0));
        let sresp = ui.interact(sr, ui.id().with("fav-star"), Sense::click());
        self.icons.paint(ui, if space.favorite { "favorite-filled" } else { "favorite-regular" }, Rect::from_center_size(sr.center(), Vec2::splat(17.0)), if space.favorite { t.star } else { t.text2 });
        if sresp.on_hover_text("Favorit (Strg+Umschalt+F)").clicked() { self.set_favorite(&space.id, !space.favorite) }
        let line = person.as_ref().map(|p| self.person_line(p, super::chrome::now_ms()));
        let (sub, sub_col) = header_subline(t, line, &space, &self.teams, t.text2);
        if !sub.is_empty() { ui.painter().text(pos2(banner.left() + 21.0, banner.top() + 52.0), egui::Align2::LEFT_CENTER, sub, FontId::proportional(13.0), sub_col); }
        // Top-right actions: a group space shows the calendar button and the grey "Treffen" pill; a direct space shows the round
        // action buttons.
        let cy = banner.top() + 30.0;
        if space.direct {
            // Direct-chat row (28 pt circles on a 34 pt pitch), right to left: video + handset (emerald-60), share (cobalt-60),
            // schedule (disabled look, gray-70), start-chat/AI ring (gray-40). Hover / pressed use the next palette steps (-70 / -80).
            // Glyphs are outline icons.
            let mut x = banner.right() - 22.0;
            let closed = space.closed; // closed chat: all five disabled
            let mut put = |s: &mut Self, ui: &mut egui::Ui, which: u8| -> egui::Response {
                let r = Rect::from_center_size(pos2(x - 14.0, cy), Vec2::splat(28.0));
                x -= 34.0;
                let resp = ui.interact(r, ui.id().with(("banner-btn", which)), if closed { Sense::hover() } else { Sense::click() });
                let (icon, fill, ring, glyph): (&'static str, Option<[Color32; 3]>, Color32, Color32) = match which {
                    // Closed: ink 20 % / 40 %.
                    0 | 1 | 2 if closed => (["video-regular", "handset-regular", "share-screen-regular"][which as usize], Some([t.ink_a(0.2); 3]), Color32::TRANSPARENT, t.ink_a(0.4)),
                    3 if closed => ("calendar-add-regular", None, t.ink_a(0.2), t.ink_a(0.4)),
                    4 if closed => ("start-chat-regular", None, t.ink_a(0.2), t.ink_a(0.4)),
                    0 => ("video-regular", Some([hex(0x248052), hex(0x1C5E3D), hex(0x17422C)]), Color32::TRANSPARENT, Color32::WHITE),
                    1 => ("handset-regular", Some([hex(0x248052), hex(0x1C5E3D), hex(0x17422C)]), Color32::TRANSPARENT, Color32::WHITE),
                    2 => ("share-screen-regular", Some([hex(0x0A78CC), hex(0x08599C), hex(0x103C63)]), Color32::TRANSPARENT, Color32::WHITE),
                    3 => ("calendar-add-regular", None, t.tk("button-primary-background:disabled"), t.tk("button-primary-background:disabled")), // 20 %
                    _ => ("start-chat-regular", None, t.text2, t.text1),
                };
                let c = r.center();
                if let Some([n, h, p]) = fill {
                    ui.painter().circle_filled(c, 14.0, if resp.is_pointer_button_down_on() { p } else if resp.hovered() { h } else { n });
                } else {
                    if resp.hovered() && which == 4 && !closed { ui.painter().circle_filled(c, 14.0, t.ink_a(0.10)); }
                    ui.painter().circle_stroke(c, 13.5, Stroke::new(1.0, ring));
                }
                s.icons.paint(ui, icon, Rect::from_center_size(c, Vec2::splat(16.0)), glyph);
                if resp.hovered() && !closed { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                resp
            };
            // Tooltips: "Video Call", "Audio Call", "Screen Share", "Schedule a meeting. Share a link and meet later". All three call
            // buttons place the 1:1 call through the guarded call path (`start_call`); media is audio only.
            let peer = person.as_ref().filter(|p| !p.email.is_empty()).map(|p| (p.email.clone(), p.name.clone()))
                .or_else(|| space.participants.iter().find(|p| !p.email.is_empty() && !p.email.eq_ignore_ascii_case(&self.me.email)).map(|p| (p.email.clone(), p.name.clone())));
            let mut call_peer = false;
            if put(self, ui, 0).on_hover_text("Video-Anruf").clicked() { call_peer = true; }
            if put(self, ui, 1).on_hover_text("Audio-Anruf").clicked() { call_peer = true; }
            if put(self, ui, 2).on_hover_text("Bildschirmfreigabe").clicked() { call_peer = true; }
            if call_peer { match &peer { Some((e, n)) => self.start_call(e, n, now), None => self.toast(crate::cloud::locus::call_allowed("", None).err().unwrap_or_default(), now) } }
            if put(self, ui, 3).on_hover_text("Setzen Sie ein Meeting an. Link teilen und später treffen").clicked() {
                let inv: Vec<String> = peer.iter().map(|(e, _)| e.clone()).collect();
                self.open_scheduler(&space.title, &inv);
            }
            if put(self, ui, 4).on_hover_text("KI-Zusammenfassung").clicked() { self.toast("Die KI-Zusammenfassung ist in dieser Version nicht verfügbar", now) }        } else {
            let pill = Rect::from_center_size(pos2(banner.right() - 26.0 - 32.5, cy), Vec2::new(65.0, 28.0));
            // Disabled "Treffen": button-primary-background / -text disabled (20 % / 40 %).
            ui.painter().rect_filled(pill, 14.0, t.tk("button-primary-background:disabled"));
            ui.painter().text(pill.center(), egui::Align2::CENTER_CENTER, tr("Meet"), FontId::proportional(14.5), t.tk("button-primary-text:disabled"));
            // A 28 pt round S button, 6 pt before the pill.
            let cal = Rect::from_center_size(pos2(pill.left() - 6.0 - 14.0, cy), Vec2::splat(28.0));
            let cresp = ui.interact(cal, ui.id().with("cal-btn"), Sense::click());
            if cresp.hovered() { ui.painter().circle_filled(cal.center(), 14.0, t.hover); }
            ui.painter().circle_stroke(cal.center(), 13.5, Stroke::new(1.0, t.tk("button-secondary-border:disabled")));
            self.icons.paint(ui, "calendar-add-regular", Rect::from_center_size(cal.center(), Vec2::splat(15.0)), t.ink_a(0.8));
            // The in-app scheduler with the space's people invited ("Schedule a meeting. Share a link and meet later").
            if cresp.on_hover_text("Setzen Sie ein Meeting an. Link teilen und später treffen").clicked() {
                let inv: Vec<String> = space.participants.iter().filter(|p| !p.email.is_empty() && !p.email.eq_ignore_ascii_case(&self.me.email)).map(|p| p.email.clone()).collect();
                self.open_scheduler(&space.title, &inv);
            }
        }
        // Tabs: pills 28 px high centred 89 px below the card top; space actions at the right.
        let mut gear_rect = Rect::NOTHING;
        let mut pin_rect = Rect::NOTHING;
        let trow = Rect::from_min_size(pos2(banner.left() + 17.0, banner.top() + 69.0), Vec2::new(banner.width() - 34.0, 28.0));
        let mut h = ui.new_child(egui::UiBuilder::new().max_rect(trow).layout(egui::Layout::left_to_right(egui::Align::Center)));
        h.spacing_mut().item_spacing.x = 12.0;
        h.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            // Direct chat = Nachrichten / Profil / Inhalt / Meetings / +Apps; space = Nachrichten / Personen (N) / Inhalt / Meetings /
            // +Apps ("Personen (N)" = "People (%1)").
            let second = if space.direct { (ConvTab::Profile, tr("Profile").to_string()) } else if space.participants.is_empty() { (ConvTab::People, tr("People").to_string()) } else { (ConvTab::People, format!("{} ({})", tr("People"), space.participants.len())) };
            for (ct, name) in [(ConvTab::Messages, tr("Messages").to_string()), second, (ConvTab::Content, tr("Content").to_string()), (ConvTab::Meetings, "Meetings".to_string()), (ConvTab::Apps, tr("Apps").to_string())] {
                if space.closed && ct == ConvTab::Profile { continue; } // closed chat: no Profil tab
                if text_tab(ui, &name, self.conv_tab == ct, t, t.tab_on).clicked() { self.conv_tab = ct }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 9.0;
                ui.add_space(27.0 - 17.0);
                let gear = self.icons.button(ui, "settings-regular", 28.0, 17.0, t.text1, t.tab_on).on_hover_text("Bereichseinstellungen");
                if gear.clicked() { super::menu::menu_toggle(ui.ctx(), "gear"); }
                gear_rect = gear.rect;
                // search = in-conversation search (ui/space_tabs/), pin list = "Angeheftete Nachrichten" popover
                let sbtn = self.icons.button(ui, "search-regular", 28.0, 17.0, t.text1, t.tab_on).on_hover_text("Im Bereich suchen");
                if sbtn.clicked() { self.conv_tab = ConvTab::Messages; super::space_tabs::find_toggle(ui.ctx(), &space.id); }
                let pbtn = self.icons.button(ui, "pin-list-regular", 28.0, 17.0, t.text1, t.tab_on).on_hover_text("Angeheftete Nachrichten anzeigen");
                if pbtn.clicked() { super::space_tabs::pins_toggle(ui.ctx(), &space.id); }
                pin_rect = pbtn.rect;
            });
        });

        // The gear menu (German items), caret pointing up at the gear.
        let gear_items = self.gear_entries(&space);
        if let Some(a) = super::menu::webex_menu(ui.ctx(), &mut self.icons, t, "gear", gear_rect, super::menu::Side::Below, super::menu::Align::Right, true, &gear_items) { self.space_action(ui.ctx(), &a, &space, now); }

        // A closed chat has the read-only notice instead of the composer (ui/closed.rs).
        // Editing a message: the live input box moves into the edit overlay at the top; the own one keeps its idle height (ui/edit_overlay.rs).
        let editing = matches!(self.compose, Compose::Edit { .. }) && !space.closed && !space.announcement;
        let composer_h = if space.closed || space.announcement { super::closed::CLOSED_BAR_H } else if editing { self.composer_idle_height() } else { self.composer_height(ui.ctx()) }; // grows with text rows / chips
        if space.closed && self.conv_tab == ConvTab::Profile { self.conv_tab = ConvTab::Messages; }
        let body = Rect::from_min_max(pos2(full.left(), banner.bottom()), pos2(full.right(), full.bottom() - composer_h));
        let mut b = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        let msgs = self.messages.get(&id).cloned().unwrap_or_default();
        // the tabs' bodies live in ui/space_tabs/ + ui/profile.rs
        match self.conv_tab {
            ConvTab::Messages => { if !self.find_view(&mut b, t, &space, &msgs) {
                // An empty direct chat shows the welcome item (ui/welcome.rs); until the first page is here: the loading item (or Retry);
                // a chat that loaded empty gets the welcome item.
                match self.list_load_state(&id) {
                    st @ (super::messages::ListLoad::Loading | super::messages::ListLoad::Failed) => self.list_loading_view(&mut b, &id, st),
                    _ => match self.welcome_for(&space) { Some(w) => self.welcome_item(&mut b, t, w, now), None => self.message_list(&mut b, t, &id, &msgs) },
                }
            } }
            _ => self.space_tab_body(&mut b, t, &space, person.as_ref(), &msgs, now),
        }
        self.pins_popover(ui.ctx(), t, pin_rect, &id);

        // the message box lives in ui/composer/ (toolbar, emoji/mention popups, markdown, drafts).
        if space.closed { let r = Rect::from_min_max(pos2(full.left(), full.bottom() - composer_h), full.max); self.closed_notice(ui, t, r); }
        else if space.announcement { let r = Rect::from_min_max(pos2(full.left(), full.bottom() - composer_h), full.max); self.announcement_notice(ui, t, r); } // announcement space
        else if editing { self.edit_overlay(ui, t, &space, Rect::from_min_max(pos2(full.left(), banner.bottom()), full.max), composer_h, now); }
        else { self.composer(ui, t, &space, now); }
        self.space_dialogs(ui.ctx(), t, &space);
    }
}

impl App {
    /// A group space's own custom cover (the data side's `space_cover`); None = flat header. Never the user's own cover.
    pub(crate) fn space_cover_tex(&mut self, ctx: &egui::Context, id: &str) -> Option<(egui::TextureId, [usize; 2])> {
        // Same lookup as photos.rs `App::space_cover` (Resp::SpaceCovers files images as "spacecover:<id>").
        self.photos.image(ctx, &format!("spacecover:{id}"))
    }
}

/// The line under the header title: a direct chat's presence line (grey), or for a space of a team the team's name in the
/// team's text colour (26 pt below the title, the same place as the direct chat's line). Otherwise nothing.
pub(crate) fn header_subline(t: &Th, person_line: Option<String>, space: &Space, teams: &[Team], grey: Color32) -> (String, Color32) {
    if let Some(l) = person_line { return (l, grey) }
    match teams.iter().find(|x| !space.team.is_empty() && x.id == space.team).filter(|x| !x.name.trim().is_empty()) {
        Some(tm) => (tm.name.clone(), team_tones(t, tm.color, &tm.id).1),
        None => (String::new(), grey),
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;

    #[test]
    fn team_spaces_show_the_team_name_in_the_team_colour() {
        let grey = Color32::GRAY;
        let t = crate::dark();
        let teams = vec![Team { id: "t".into(), name: "Product Team".into(), color: Some((0x12, 0x34, 0x56)) }];
        let general = Space { id: "t".into(), team: "t".into(), title: "Allgemein".into(), ..Default::default() };
        assert_eq!(header_subline(&t, None, &general, &teams, grey), ("Product Team".to_string(), team_tones(&t, Some((0x12, 0x34, 0x56)), "t").1));
        let other = Space { id: "s".into(), team: "t".into(), ..Default::default() };
        assert_eq!(header_subline(&t, None, &other, &teams, grey).0, "Product Team");
        // No team, or a team whose name isn't known: no line.
        assert_eq!(header_subline(&t, None, &Space::default(), &teams, grey), (String::new(), grey));
        let unnamed = vec![Team { id: "t".into(), name: String::new(), color: None }];
        assert_eq!(header_subline(&t, None, &general, &unnamed, grey).0, "");
        // A direct chat keeps its grey presence line.
        assert_eq!(header_subline(&t, Some("Verfügbar".into()), &Space { direct: true, ..Default::default() }, &teams, grey), ("Verfügbar".to_string(), grey));
    }
}
