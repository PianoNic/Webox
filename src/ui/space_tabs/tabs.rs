//! The People, Content, Files and Meetings tabs of a space.
use super::*;

impl App {
    /// Body of every tab but "Nachrichten".
    pub(crate) fn space_tab_body(&mut self, b: &mut egui::Ui, t: &Th, space: &Space, person: Option<&Person>, msgs: &[Message], now: f64) {
        let ctx = b.ctx().clone();
        let mut s = st(&ctx);
        if s.space != space.id { s = St { space: space.id.clone(), ..Default::default() }; put(&ctx, s.clone()); }
        match self.conv_tab {
            ConvTab::Profile => self.profile_tab(b, t, person, now),
            ConvTab::People => self.people_tab(b, t, space, person, s, now),
            ConvTab::Content => self.content_tab(b, t, msgs, s, now),
            ConvTab::Meetings => self.meetings_tab(b, t, space, person, now),
            ConvTab::Apps => {
                // "The organization that owns this space hasn't enabled apps yet" (the Apps hub itself is not part of Webox).
                egui::Frame::default().inner_margin(24).show(b, |ui| {
                    ui.set_min_size(Vec2::new(ui.available_width(), 200.0));
                    ui.add_space(60.0);
                    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
                    self.icons.paint(ui, "apps-regular", Rect::from_center_size(r.center(), Vec2::splat(48.0)), t.text2);
                    ui.add_space(8.0);
                    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Die Organisation, die Eigentümer dieses Bereichs ist, hat noch keine Apps aktiviert", FontId::proportional(15.0), t.text2);
                });
            }
            ConvTab::Messages => {}
        }
    }

    // Personen.
    pub(super) fn people_tab(&mut self, b: &mut egui::Ui, t: &Th, space: &Space, person: Option<&Person>, mut s: St, now: f64) {
        let ctx = b.ctx().clone();
        let mut list: Vec<Person> = self.space_people(space); // full list (loaded once), else the list's 10
        if list.is_empty() { list = person.cloned().into_iter().collect(); }
        let me_in = list.iter().any(|p| p.id == self.me.id || (!p.email.is_empty() && p.email.eq_ignore_ascii_case(&self.me.email)));
        if !me_in && space.direct { list.insert(0, self.me.clone()); }
        let q = s.people_q.to_lowercase();
        list.retain(|p| q.is_empty() || p.name.to_lowercase().contains(&q) || p.email.to_lowercase().contains(&q));
        let area = b.max_rect();
        // Header row: search pill 300 x 32 at the left, "Personen hinzufügen" pill at the right.
        let head = Rect::from_min_size(area.min + Vec2::new(24.0, 14.0), Vec2::new(area.width() - 48.0, 32.0));
        let sr = Rect::from_min_size(head.min, Vec2::new(300.0_f32.min(head.width() - 190.0).max(160.0), 32.0));
        b.painter().rect_filled(sr, 16.0, t.search);
        self.icons.paint(b, "search-regular", Rect::from_center_size(sr.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(16.0)), hex(0xEDEDED));
        let mut c = b.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(sr.min + Vec2::new(34.0, 0.0), sr.max - Vec2::new(12.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        c.add(egui::TextEdit::singleline(&mut s.people_q).hint_text(RichText::new("Suchen").color(t.text2).size(14.5)).font(FontId::proportional(14.5)).frame(false).desired_width(sr.width() - 50.0));
        put(&ctx, s.clone());
        if !space.direct {
            let label = "Personen hinzufügen";
            let tw = b.fonts(|f| f.layout_no_wrap(label.into(), semibold(14.0), t.white_btn_text).size().x);
            let br = Rect::from_min_size(pos2(head.right() - tw - 52.0, head.top()), Vec2::new(tw + 52.0, 32.0));
            let resp = b.interact(br, egui::Id::new("people-add-btn"), Sense::click());
            let fill = if resp.is_pointer_button_down_on() { alpha(t.white_btn, 0.75) } else if resp.hovered() { alpha(t.white_btn, 0.88) } else { t.white_btn };
            if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            b.painter().rect_filled(br, 16.0, fill);
            self.icons.paint(b, "participant-add-regular", Rect::from_center_size(pos2(br.left() + 21.0, br.center().y), Vec2::splat(16.0)), t.white_btn_text);
            b.painter().text(pos2(br.left() + 36.0, br.center().y), egui::Align2::LEFT_CENTER, label, semibold(14.0), t.white_btn_text);
            if resp.clicked() { self.add_people = true; }
        }
        let body = Rect::from_min_max(pos2(area.left(), head.bottom() + 12.0), area.max);
        let mut lst = b.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        lst.set_clip_rect(body.intersect(b.clip_rect()));
        let mut ctx_target: Option<(Person, Rect)> = None;
        crate::ui::widgets::vscroll().id_salt(("people-scroll", &space.id)).auto_shrink([false; 2]).show(&mut lst, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            if list.is_empty() { ui.add_space(30.0); ui.label(RichText::new("Keine Ergebnisse").size(15.0).color(t.text2)); }
            for p in &list {
                let mine = p.id == self.me.id || (!p.email.is_empty() && p.email.eq_ignore_ascii_case(&self.me.email));
                let pp = self.live_person(p);
                let ph = self.photo_for(&ctx, &pp.id, &pp.email);
                let sub = if mine { presence_text(&self.me.status).to_string() } else { self.person_line(&pp, super::super::chrome::now_ms()) };
                let title = if mine { format!("{} (Sie)", pp.name) } else { pp.name.clone() }; // "%1 (You)"
                let resp = self.row(ui, t, &title, &sub, t.text2, false, false, false, None, Some(&pp.status), ph);
                // "Moderator" tag at the right of a moderator's row.
                if !space.direct && self.is_moderator(&space.id, &pp) && !ui.rect_contains_pointer(resp.rect) {
                    ui.painter().text(resp.rect.right_center() - Vec2::new(24.0, 0.0), egui::Align2::RIGHT_CENTER, "Moderator", FontId::proportional(13.0), t.text2);
                }
                if resp.clicked() { Self::contact_card_open(&ctx, &pp); }
                if resp.secondary_clicked() { ctx_target = Some((pp.clone(), Rect::from_min_size(ctx.input(|i| i.pointer.interact_pos()).unwrap_or(resp.rect.center()), Vec2::splat(1.0)))); }
            }
        });
        // Context menu of a person (contact card, message, "Assign as moderator", "Remove from space").
        if let Some((p, anchor)) = ctx_target {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("people-ctx-target"), (p.id, p.name, p.email, p.status, anchor)));
            menu::menu_open(&ctx, "people-ctx");
        }
        if let Some((id, name, email, status, anchor)) = ctx.data(|d| d.get_temp::<(String, String, String, String, Rect)>(egui::Id::new("people-ctx-target"))) {
            let p = Person { id, name, email, status };
            let mine = p.id == self.me.id || (!p.email.is_empty() && p.email.eq_ignore_ascii_case(&self.me.email));
            let mut items = vec![Item::new("card", "Kontaktkarte anzeigen").icon("participant-regular").entry()];
            if !mine { items.push(Item::new("chat", "Nachricht").icon("chat-regular").entry()); }
            // Moderation ("Assign as moderator" / "Remove as moderator" / "Remove from space"): offered in an unmoderated space and to
            // moderators of a moderated one ("This is a moderated space. Only a moderator can …").
            let may = !space.direct && (!self.moderated(&space.id) || self.i_moderate(&space.id));
            if may {
                items.push(Entry::Sep);
                if self.is_moderator(&space.id, &p) { items.push(Item::new("unmod", "Als Moderator entfernen").icon("participant-remove-regular").entry()); }
                else { items.push(Item::new("mod", "Als Moderator zuweisen").icon("participant-add-regular").entry()); }
                if !mine { items.push(Item::new("remove", "Aus Bereich entfernen").icon("participant-remove-regular").danger().entry()); }
            }
            if let Some(a) = menu::webex_menu(&ctx, &mut self.icons, t, "people-ctx", anchor, Side::Below, Align::Left, false, &items) {
                match a.as_str() {
                    "card" => Self::contact_card_open(&ctx, &p),
                    "chat" => { self.open_or_create_direct(&p, now); }
                    "mod" => self.set_moderator(&space.id, &p, true),
                    "unmod" => self.set_moderator(&space.id, &p, false),
                    "remove" => self.remove_person(&space.id, &p),
                    _ => {}
                }
            }
        }
    }

    // Inhalt.
    pub(super) fn content_tab(&mut self, b: &mut egui::Ui, t: &Th, msgs: &[Message], mut s: St, now: f64) {
        let _ = now;
        let ctx = b.ctx().clone();
        let area = b.max_rect();
        // Sub-tabs "Dateien" / "Links" / "Whiteboards".
        let trow = Rect::from_min_size(area.min + Vec2::new(24.0, 12.0), Vec2::new(area.width() - 48.0, 28.0));
        let mut h = b.new_child(egui::UiBuilder::new().max_rect(trow).layout(egui::Layout::left_to_right(egui::Align::Center)));
        h.spacing_mut().item_spacing.x = 8.0;
        for (i, n) in ["Dateien", "Links", "Whiteboards"].iter().enumerate() {
            if text_tab(&mut h, n, s.content == i as u8, t, t.tab_on).clicked() { s.content = i as u8; put(&ctx, s.clone()); }
        }
        let body = Rect::from_min_max(pos2(area.left(), trow.bottom() + 8.0), area.max);
        let mut lst = b.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        lst.set_clip_rect(body.intersect(b.clip_rect()));
        let mut to_msgs = false;
        crate::ui::widgets::vscroll().id_salt("content-scroll").auto_shrink([false; 2]).show(&mut lst, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            match s.content {
                0 => {
                    let files = files_of(msgs, self.client.is_none());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| { ui.add_space(24.0); ui.label(RichText::new("In den Nachrichten freigegebene Dateien").size(13.0).color(t.text2)); });
                    ui.add_space(6.0);
                    if files.is_empty() { empty_state(ui, &mut self.icons, t, Some("files-192-cobalt-lime"), "Keine Dateien"); } // "Files Tab, No files" (QObject)
                    for (i, f) in files.iter().enumerate() {
                        let sub = [f.who.clone(), f.when.clone(), size_text(f.size)].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
                        // File row: Download / In Ordner anzeigen / In Unterhaltung anzeigen, click = open.
                        match self.file_row(ui, t, f, &sub, i, now) {
                            Some("chat") => { to_msgs = true; ctx.data_mut(|d| d.insert_temp(egui::Id::new("msg-goto"), (f.msg.clone(), 0u8))); }
                            Some("view") => { to_msgs = true; ctx.data_mut(|d| d.insert_temp(egui::Id::new("lightbox"), f.key.clone())); }
                            _ => {}
                        }
                    }
                }
                1 => {
                    let links = links_of(msgs);
                    ui.add_space(4.0);
                    if links.is_empty() { empty_state(ui, &mut self.icons, t, Some("files-192-cobalt-lime"), "Keine Links"); } // "Links Tab, No links" (QObject)
                    for l in &links {
                        let sub = [l.who.clone(), l.when.clone()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
                        let (open, show) = self.content_row(ui, t, "link-regular", &l.url, t.link, &sub);
                        if open { system::open(&if l.url.starts_with("www.") { format!("https://{}", l.url) } else { l.url.clone() }); }
                        if show { to_msgs = true; }
                    }
                }
                _ => { ui.add_space(4.0); empty_state(ui, &mut self.icons, t, Some("whiteboards-192-cobalt-lime"), "Keine Whiteboards"); } // Webox's own wording
            }
        });
        if to_msgs { self.conv_tab = ConvTab::Messages; }
    }

    /// One file / link row (56 pt, radius 12): icon disc, title, "Author · when · size", and on hover "In Unterhaltung anzeigen".
    /// Returns (row clicked, "show in conversation" clicked).
    pub(super) fn content_row(&mut self, ui: &mut egui::Ui, t: &Th, icon: &'static str, title: &str, title_col: Color32, sub: &str) -> (bool, bool) {
        let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
        let r = Rect::from_min_max(full.min + Vec2::new(16.0, 0.0), pos2(full.max.x - 16.0, full.max.y - 2.0));
        let resp = ui.interact(r, ui.id().with(("content-row", title, sub)), Sense::click());
        let lr = Rect::from_min_max(pos2(r.right() - 190.0, r.top() + 10.0), pos2(r.right() - 8.0, r.bottom() - 10.0));
        let lresp = ui.interact(lr, ui.id().with(("content-row-show", title, sub)), Sense::click());
        let hot = resp.hovered() || lresp.hovered();
        if hot { ui.painter().rect_filled(r, 12.0, t.ink_a(0.05)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        let disc = Rect::from_center_size(pos2(r.left() + 28.0, r.center().y), Vec2::splat(32.0));
        ui.painter().circle_filled(disc.center(), 16.0, t.avatar);
        self.icons.paint(ui, icon, Rect::from_center_size(disc.center(), Vec2::splat(16.0)), t.avatar_text);
        let right_w = if hot { 190.0 } else { 0.0 };
        super::super::chrome::fit_text(ui.painter(), r.left() + 56.0, r.center().y - 9.0, title, FontId::proportional(15.0), title_col, r.width() - 56.0 - right_w - 12.0);
        super::super::chrome::fit_text(ui.painter(), r.left() + 56.0, r.center().y + 11.0, sub, FontId::proportional(13.0), t.text2, r.width() - 56.0 - right_w - 12.0);
        if hot { ui.painter().text(lr.right_center(), egui::Align2::RIGHT_CENTER, "In Unterhaltung anzeigen", FontId::proportional(13.5), t.link); }
        (resp.clicked() && !lresp.hovered(), lresp.clicked())
    }
    /// One file row of "Dateien": icon disc, name, "Author · when · size"; on hover the actions "Download", "In Ordner anzeigen"
    /// once downloaded and "In Unterhaltung anzeigen"; a click opens the file (pictures open in the viewer); right click = the
    /// file menu. While downloading the size line shows "Wird heruntergeladen… n %". Returns "chat" / "view" for the caller.
    /// Geometry as `content_row`.
    pub(super) fn file_row(&mut self, ui: &mut egui::Ui, t: &Th, f: &FileRow, sub: &str, i: usize, now: f64) -> Option<&'static str> {
        use crate::app::files::Phase;
        self.files_frame(&ui.ctx().clone(), now);
        let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
        let r = Rect::from_min_max(full.min + Vec2::new(16.0, 0.0), pos2(full.max.x - 16.0, full.max.y - 2.0));
        let resp = ui.interact(r, ui.id().with(("file-row", i, &f.key)), Sense::click());
        let dl = (!f.key.is_empty()).then(|| crate::app::files::download(&f.key)).flatten();
        let done = dl.as_ref().is_some_and(|d| d.phase == Phase::Done && d.path.exists());
        let busy = dl.as_ref().filter(|d| matches!(d.phase, Phase::Preparing | Phase::Uploading));
        // The actions at the right (links), from right to left.
        let mut acts: Vec<(&'static str, &'static str)> = vec![("chat", "In Unterhaltung anzeigen")];
        if !f.key.is_empty() { if done { acts.push(("folder", "In Ordner anzeigen")); } else if busy.is_none() { acts.push(("download", "Download")); } }
        let hot_area = resp.hovered() || super::super::widgets::hover_ok(ui, r);
        let mut x = r.right() - 8.0;
        let mut clicked: Option<&'static str> = None;
        let mut used = 0.0;
        if hot_area {
            ui.painter().rect_filled(r, 12.0, t.ink_a(0.05));
            for (id, label) in &acts {
                let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(13.5), t.link);
                let lr = Rect::from_min_max(pos2(x - g.size().x, r.top() + 10.0), pos2(x, r.bottom() - 10.0));
                let lresp = ui.interact(lr, ui.id().with(("file-row-act", i, *id)), Sense::click());
                if lresp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); ui.painter().hline(lr.x_range(), r.center().y + 9.0, Stroke::new(1.0_f32, t.link)); }
                ui.painter().galley(pos2(lr.left(), r.center().y - g.size().y / 2.0), g, t.link);
                if lresp.clicked() { clicked = Some(id); }
                x = lr.left() - 18.0;
                used = r.right() - x;
            }
            if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        }
        let disc = Rect::from_center_size(pos2(r.left() + 28.0, r.center().y), Vec2::splat(32.0));
        ui.painter().circle_filled(disc.center(), 16.0, t.avatar);
        self.icons.paint(ui, if f.image { "file-image-regular" } else { file_icon(&f.name) }, Rect::from_center_size(disc.center(), Vec2::splat(16.0)), t.avatar_text);
        let sub = match busy { Some(d) => format!("Wird heruntergeladen… {} %", super::super::attach::percent(d.got, d.total)), None => sub.to_string() };
        super::super::chrome::fit_text(ui.painter(), r.left() + 56.0, r.center().y - 9.0, &f.name, FontId::proportional(15.0), t.text1, r.width() - 56.0 - used - 12.0);
        super::super::chrome::fit_text(ui.painter(), r.left() + 56.0, r.center().y + 11.0, &sub, FontId::proportional(13.0), t.text2, r.width() - 56.0 - used - 12.0);
        let k_menu = format!("file-row-menu-{i}-{}", f.key);
        if resp.secondary_clicked() && !f.key.is_empty() {
            if let Some(pp) = resp.interact_pointer_pos() { ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("file-row-at", &k_menu)), pp)); }
            menu::menu_open(ui.ctx(), &k_menu);
        }
        if menu::menu_is_open(ui.ctx(), &k_menu) {
            let at = ui.ctx().data(|d| d.get_temp::<egui::Pos2>(egui::Id::new(("file-row-at", &k_menu)))).unwrap_or(r.left_bottom());
            let items = self.file_menu_entries(&f.key, true);
            if let Some(a) = menu::webex_menu(ui.ctx(), &mut self.icons, t, &k_menu, Rect::from_min_size(at, Vec2::ZERO), Side::Below, Align::Left, false, &items) {
                if a == "file-chat" { return Some("chat"); }
                self.file_menu_do(&a, &f.key, &f.name, now);
            }
        }
        match clicked {
            Some("chat") => return Some("chat"),
            Some("download") => self.file_fetch(&f.key, &f.name, crate::app::files::Then::Nothing, now),
            Some("folder") => self.file_fetch(&f.key, &f.name, crate::app::files::Then::ShowInFolder, now),
            _ if resp.clicked() && f.image => return Some("view"),
            _ if resp.clicked() && !f.key.is_empty() => self.file_fetch(&f.key, &f.name, crate::app::files::Then::Open, now),
            _ => {}
        }
        None
    }

    // Meetings.
    pub(super) fn meetings_tab(&mut self, b: &mut egui::Ui, t: &Th, space: &Space, person: Option<&Person>, now: f64) {
        let ctx = b.ctx().clone();
        // The meetings of this space: `App.meetings` are the account's upcoming meetings and carry no space id, so they are matched by title.
        let (st_, pn) = (space.title.to_lowercase(), person.map(|p| p.name.to_lowercase()).unwrap_or_default());
        let list: Vec<Meeting> = self.meetings.iter().filter(|m| { let mt = m.title.to_lowercase(); (!st_.is_empty() && mt.contains(&st_)) || (!pn.is_empty() && mt.contains(&pn)) }).cloned().collect();
        let area = b.max_rect();
        // "Meeting starten" top right: a placeholder toast.
        let label = "Meeting starten";
        let tw = b.fonts(|f| f.layout_no_wrap(label.into(), semibold(14.0), Color32::WHITE).size().x);
        let br = Rect::from_min_size(pos2(area.right() - 24.0 - tw - 52.0, area.top() + 14.0), Vec2::new(tw + 52.0, 32.0));
        let resp = b.interact(br, egui::Id::new("meeting-start-btn"), Sense::click());
        if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
        b.painter().rect_filled(br, 16.0, if resp.is_pointer_button_down_on() { hex(0x17422C) } else if resp.hovered() { hex(0x1C5E3D) } else { hex(0x248052) });
        self.icons.paint(b, "video-regular", Rect::from_center_size(pos2(br.left() + 21.0, br.center().y), Vec2::splat(16.0)), Color32::WHITE);
        b.painter().text(pos2(br.left() + 36.0, br.center().y), egui::Align2::LEFT_CENTER, label, semibold(14.0), Color32::WHITE);
        if resp.clicked() { self.toast("Das Starten von Meetings ist in dieser Version nicht verfügbar", now); }
        let body = Rect::from_min_max(pos2(area.left(), br.bottom() + 10.0), area.max);
        let mut lst = b.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        lst.set_clip_rect(body.intersect(b.clip_rect()));
        crate::ui::widgets::vscroll().id_salt("meetings-scroll").auto_shrink([false; 2]).show(&mut lst, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            if list.is_empty() { empty_state(ui, &mut self.icons, t, Some("desert-open-results-192-cobalt-lime"), "Keine Meetings angesetzt"); } // pack: CustomCalendar
            for m in &list {
                let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::hover());
                let r = Rect::from_min_max(full.min + Vec2::new(16.0, 0.0), pos2(full.max.x - 16.0, full.min.y + 50.0));
                let resp = ui.interact(r, ui.id().with(("meeting-row", &m.title, &m.start)), Sense::hover());
                if resp.hovered() { ui.painter().rect_filled(r, 25.0, t.ink_a(0.05)); }
                let disc = Rect::from_center_size(pos2(r.left() + 27.0, r.center().y), Vec2::splat(32.0)); // 32 pt disc, left margin 11
                ui.painter().circle_filled(disc.center(), 16.0, t.avatar);
                self.icons.paint(ui, "calendar-month-regular", Rect::from_center_size(disc.center(), Vec2::splat(16.0)), t.avatar_text);
                let jw = 92.0;
                super::super::chrome::fit_text(ui.painter(), r.left() + 54.0, r.center().y - 9.0, &m.title, FontId::proportional(15.0), t.text1, r.width() - 54.0 - jw - 24.0);
                super::super::chrome::fit_text(ui.painter(), r.left() + 54.0, r.center().y + 10.0, &m.start, FontId::proportional(13.0), t.text2, r.width() - 54.0 - jw - 24.0);
                let jr = Rect::from_center_size(pos2(r.right() - 11.0 - jw / 2.0, r.center().y), Vec2::new(jw, 28.0));
                let jresp = ui.interact(jr, ui.id().with(("meeting-join", &m.title, &m.start)), if m.link.is_empty() { Sense::hover() } else { Sense::click() });
                let fill = if m.link.is_empty() { t.ink_a(0.12) } else if jresp.is_pointer_button_down_on() { hex(0x17422C) } else if jresp.hovered() { hex(0x1C5E3D) } else { hex(0x248052) };
                if jresp.hovered() && !m.link.is_empty() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                ui.painter().rect_filled(jr, 14.0, fill);
                ui.painter().text(jr.center(), egui::Align2::CENTER_CENTER, "Beitreten", semibold(14.0), if m.link.is_empty() { t.text2 } else { Color32::WHITE });
                if jresp.clicked() { system::open(&m.link); }
            }
        });
    }
}
