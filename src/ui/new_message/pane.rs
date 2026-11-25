//! The New message pane: recipients, suggestions, drafts and opening the chat.
use super::*;

impl App {
    /// Opens the pane (from the + menu): remembers the conversation to return to on "Cancel".
    pub(crate) fn new_message_open(&mut self, ctx: &egui::Context) {
        super::super::invite::close_state(ctx);
        let prev = self.selected.take();
        self.new_space = None;
        self.set_tab(Tab::Messaging);
        set_state(ctx, NewMsg { open: true, prev, focus: true, ..Default::default() });
    }

    pub(super) fn new_message_close(&mut self, ctx: &egui::Context) {
        let s = state(ctx);
        set_state(ctx, NewMsg::default());
        self.drafts.remove(DRAFT_ID);
        if let Some(id) = s.prev { if self.spaces.iter().any(|x| x.id == id) { self.open_space(id); } }
    }

    /// The temporary list row (top of "Other").
    pub(crate) fn new_message_row(&mut self, ui: &mut egui::Ui, t: &Th) {
        let g = crate::ui::theme::density::ListRow::current(self.compact_view()); // the list row's geometry (chrome::row)
        let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), g.pitch), Sense::hover());
        let r = Rect::from_min_max(full.min + Vec2::new(12.0, 0.0), pos2(full.max.x - 4.0, full.min.y + g.h));
        let resp = ui.interact(r, egui::Id::new("newmsg-row"), Sense::click());
        if resp.clicked() { self.selected = None; super::super::invite::close_state(ui.ctx()); }
        if self.selected.is_none() { ui.painter().rect_filled(r, g.h / 2.0, t.selected); } else if resp.hovered() { ui.painter().rect_filled(r, g.h / 2.0, t.ink_a(0.05)); }
        let c = pos2(r.left() + (g.avatar_left + g.avatar / 2.0), r.center().y);
        ui.painter().circle_stroke(c, g.avatar / 2.0 - 0.5, Stroke::new(1.0_f32, t.text2));
        self.icons.paint(ui, "start-chat-regular", Rect::from_center_size(c, Vec2::splat(g.avatar * 18.0 / 32.0)), t.text1);
        super::super::chrome::fit_text(ui.painter(), r.left() + g.text_x, r.center().y, tr("New message"), FontId::proportional(g.title_pt), t.text1, r.width() - (g.text_x + 34.0));
        self.icons.paint(ui, "more-regular", Rect::from_center_size(r.right_center() - Vec2::new(26.0, 0.0), Vec2::splat(16.0)), t.text2);
    }

    /// The content pane.
    pub(crate) fn new_message_screen(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let mut s = state(&ctx);
        let full = ui.max_rect();
        vgrad(ui.painter(), Rect::from_min_max(full.min, pos2(full.max.x + 10.0, full.max.y)), 10.0, t.conv, t.conv_b); // square top-right at the window edge, as the conversation card
        let at = |x: f32, y: f32| pos2(full.left() + x, full.top() + y);

        // Header: title, pencil ("Edit space name") and the hidden name field (placeholder / tooltip "Space name").
        // The pencil hides label + pencil and shows the focused field; Enter / Return sets the title (trimmed text, empty =
        // default "New message"; the name is used when the space is created), Esc leaves the edit mode.
        if s.editing {
            let fr = Rect::from_min_size(at(TITLE_X - 2.0, HEAD_Y - 14.0), Vec2::new((full.width() - TITLE_X - RIGHT - PILL.0 - 24.0).clamp(160.0, 420.0), 28.0));
            let nid = egui::Id::new("newmsg-name");
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(fr).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let r = c.add(egui::TextEdit::singleline(&mut s.name_buf).id(nid).hint_text(RichText::new(crate::tr!("NewMessageWidget", "Space name")).color(t.text2).size(15.8))
                .font(FontId::proportional(15.8)).desired_width(fr.width()).margin(Vec2::new(2.0, 4.0)))
                .on_hover_text(crate::tr!("NewMessageWidget", "Space name").into_owned());
            if std::mem::take(&mut s.focus) { r.request_focus(); }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) { s.name = s.name_buf.trim().to_string(); s.editing = false; }
            else if ui.input(|i| i.key_pressed(Key::Escape)) { s.editing = false; s.name_buf = s.name.clone(); }
        } else {
            let title = if s.name.is_empty() { tr("New message").to_string() } else { s.name.clone() };
            let max_w = (full.width() - TITLE_X - RIGHT - PILL.0 - 80.0).max(80.0);
            let tg = super::super::chrome::fit_text(ui.painter(), full.left() + TITLE_X, full.top() + HEAD_Y - 0.8, &title, FontId::proportional(15.8), t.text1, max_w);
            // The pencil sits 36.8 pt right of the title's end.
            let pen = Rect::from_center_size(pos2(tg.right() + 36.8, full.top() + HEAD_Y + 0.8), Vec2::splat(20.0));
            let presp = ui.interact(pen, egui::Id::new("newmsg-pen"), Sense::click());
            if presp.hovered() { ui.painter().circle_filled(pen.center(), 12.0, t.hover); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            self.icons.paint(ui, "edit-regular", Rect::from_center_size(pen.center(), Vec2::splat(16.0)), t.text1);
            if presp.on_hover_text(crate::tr!("NewMessageWidget", "Edit space name").into_owned()).clicked() { s.editing = true; s.name_buf = s.name.clone(); s.focus = true; }
        }
        let pill = Rect::from_min_size(pos2(full.right() - RIGHT - PILL.0, full.top() + HEAD_Y - PILL.1 / 2.0), Vec2::new(PILL.0, PILL.1));
        let cresp = ui.interact(pill, egui::Id::new("newmsg-cancel"), Sense::click());
        if cresp.hovered() { ui.painter().rect_filled(pill, PILL.1 / 2.0, t.hover); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        ui.painter().rect_stroke(pill, PILL.1 / 2.0, Stroke::new(1.0_f32, t.tk("button-secondary-border")), egui::StrokeKind::Inside);
        ui.painter().text(pill.center(), egui::Align2::CENTER_CENTER, crate::tr!("NewMessageWidget", "Cancel"), semibold(15.0), t.text1);
        if cresp.clicked() { self.new_message_close(&ctx); return; }

        // "An:" + field (recipients as chips at the start of the field).
        ui.painter().text(at(32.0, FIELD_Y + FIELD_H / 2.0), egui::Align2::LEFT_CENTER, crate::tr!("NewMessageWidget", "To:"), semibold(14.0), t.text1);
        let field = Rect::from_min_max(at(FIELD_X, FIELD_Y), pos2(full.right() - RIGHT, full.top() + FIELD_Y + FIELD_H));
        let tid = egui::Id::new("newmsg-field");
        let focused = ctx.memory(|m| m.has_focus(tid));
        // inputBackground; border #A6A6A6 / #E6E6E6 focused = 60 % / 90 % ink.
        ui.painter().rect_filled(field, FIELD_H / 2.0, t.tk("inputBackground"));
        ui.painter().rect_stroke(field, FIELD_H / 2.0, Stroke::new(1.0_f32, if focused || !s.query.is_empty() { t.ink_a(0.9) } else { t.ink_a(0.6) }), egui::StrokeKind::Inside);
        self.icons.paint(ui, "search-regular", Rect::from_center_size(pos2(field.left() + 11.2, field.center().y + 1.0), Vec2::splat(16.0)), t.text1);
        // Recipients as chips: height 24, 16 px / 400, colours button-secondary-background / -border / -text (dark: transparent,
        // hover white 7 %, pressed 20 %; border white 50 %; text white 95 %), a cancel-bold icon at the right (10 pt icon, 12 pt
        // text padding, 26 pt icon area).
        let light = super::super::menu::light(t);
        let (cb, ct) = if light { (Color32::BLACK, Color32::BLACK) } else { (Color32::WHITE, Color32::WHITE) };
        let mut x = field.left() + 30.4;
        let mut remove = None;
        let chip_clip = Rect::from_min_max(field.min, pos2(field.right() - 120.0, field.bottom()));
        for (i, p) in s.to.iter().enumerate() {
            let label = if p.name.is_empty() { p.email.clone() } else { p.name.clone() };
            let w = ui.fonts(|f| f.layout_no_wrap(label.clone(), FontId::proportional(16.0), t.text1).size().x) + 12.0 + 26.0;
            let chip = Rect::from_min_size(pos2(x, field.center().y - 12.0), Vec2::new(w, 24.0));
            x = chip.right() + 6.0;
            if chip.right() > chip_clip.right() { continue; }
            let cresp = ui.interact(chip, egui::Id::new(("newmsg-chip", i)), Sense::click());
            let fill = if cresp.is_pointer_button_down_on() { alpha(cb, 0.2) } else if cresp.hovered() { alpha(cb, 0.07) } else { Color32::TRANSPARENT };
            ui.painter().rect_filled(chip, 12.0, fill);
            ui.painter().rect_stroke(chip, 12.0, Stroke::new(1.0_f32, alpha(cb, 0.5)), egui::StrokeKind::Inside);
            ui.painter().text(pos2(chip.left() + 12.0, chip.center().y), egui::Align2::LEFT_CENTER, &label, FontId::proportional(16.0), alpha(ct, 0.95));
            self.icons.paint(ui, "cancel-bold", Rect::from_center_size(pos2(chip.right() - 13.0 + 4.0 - 4.0, chip.center().y + 1.0), Vec2::splat(10.0)), alpha(ct, 0.95));
            // A click on the chip's icon or the chip removes it ("%1, To remove press spacebar").
            if cresp.on_hover_text(crate::tr!("NewMessageWidget", "%1, To remove press spacebar", label)).clicked() { remove = Some(i); }
        }
        if let Some(i) = remove { s.to.remove(i); }
        let x = x.min(chip_clip.right());
        let clear_w = if s.query.is_empty() { 0.0 } else { 30.0 };
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(x, field.top()), pos2(field.right() - 12.0 - clear_w, field.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        // The placeholder switches to "Add people by name or email" once someone is added.
        let hint = if s.to.is_empty() { crate::tr!("QObject", "Add by name, number, or email").into_owned() } else { crate::tr!("NewMessageWidget", "Add people by name or email").into_owned() };
        let before = s.query.clone();
        let te = c.add(egui::TextEdit::singleline(&mut s.query).id(tid).hint_text(RichText::new(hint).color(t.text2).size(16.0))
            .frame(false).margin(Vec2::ZERO).font(FontId::proportional(16.0)).desired_width(f32::INFINITY));
        if std::mem::take(&mut s.focus) { te.request_focus(); }
        if s.query != before { s.sel = None; }
        if !s.query.is_empty() {
            let xr = Rect::from_center_size(pos2(field.right() - 17.6, field.center().y), Vec2::splat(20.0));
            let xresp = ui.interact(xr, egui::Id::new("newmsg-clear"), Sense::click());
            self.icons.paint(ui, "cancel-regular", Rect::from_center_size(xr.center(), Vec2::splat(10.0)), t.text1);
            if xresp.clicked() { s.query.clear(); s.sel = None; te.request_focus(); }
        }
        // Backspace in the empty field removes the last chip.
        if !s.to.is_empty() && before.is_empty() && s.query.is_empty() && ui.input(|i| i.key_pressed(Key::Backspace)) && te.has_focus() { s.to.pop(); }
        // The directory search runs for every change of the text (signed in only; read-only).
        let q = s.query.trim().to_string();
        if !q.is_empty() && q != s.asked {
            if let Some(c) = &self.client { c.req(Req::DirectorySearch(q.clone())); }
            s.asked = q;
        }

        // The composer (DRAFT_ID makes it a new-message composer, see composer.rs).
        let draft = Space { id: DRAFT_ID.into(), ..Default::default() };
        set_state(&ctx, s.clone());
        self.composer(ui, t, &draft, now);
        if let Some(text) = self.drafts.remove(SEND_KEY) { self.new_message_send(&ctx, text, now); }
        let mut s = state(&ctx);
        if !s.open { return; } // closed by a send

        // The conversation with exactly the chosen people: its messages fill the body, the send goes there.
        let me = self.me.clone();
        let matched = { let dp = |sp: &Space| self.direct_person(sp); matched_space(&self.spaces, dp, &s.to, &me) };
        if let Some(id) = &matched {
            if s.loaded != *id {
                if let Some(c) = &self.client { if !self.messages.contains_key(id) { c.req(Req::Messages(id.clone())); } }
                s.loaded = id.clone();
            }
            let body = Rect::from_min_max(pos2(full.left(), field.bottom() + 12.0), pos2(full.right(), full.bottom() - self.composer_height(&ctx)));
            let msgs = self.messages.get(id).cloned().unwrap_or_default();
            let mut b = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
            self.message_list(&mut b, t, id, &msgs);
        }

        // Dropdown over the body.
        if !s.query.trim().is_empty() {
            let me = self.me.clone();
            // Known people plus the directory's answer for this query, without the ones already added.
            let pool: Vec<Person> = merged_people(&self.people, &directory_for(s.query.trim())).into_iter().filter(|p| !s.to.iter().any(|x| same_person(x, p))).collect();
            let ls = lines(&s.query, &pool, &self.spaces, &me);
            let ring = ring_index(&ls, s.sel);
            // Keyboard: Up / Down over the pickable lines, Enter picks.
            let pick_ix: Vec<usize> = ls.iter().enumerate().filter(|(_, l)| l.pickable()).map(|(i, _)| i).collect();
            let mut picked: Option<Line> = None;
            if te.has_focus() || s.sel.is_some() {
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) { if let Some(r) = ring { s.sel = pick_ix.iter().copied().find(|&i| i > r).or(Some(r)).filter(|&i| ls[i].pickable()); } }
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) { if let Some(r) = ring { s.sel = pick_ix.iter().copied().rev().find(|&i| i < r).or(Some(r)).filter(|&i| ls[i].pickable()); } }
                if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) { if let Some(r) = ring.filter(|&r| ls[r].pickable()) { picked = Some(ls[r].clone()); } }
            }
            let ring = ring_index(&ls, s.sel);
            let total: f32 = ls.iter().map(Line::h).sum();
            let top = field.bottom() + DROP_GAP;
            let drop = Rect::from_min_max(pos2(field.left() - 0.8, top), pos2(field.right() + 0.8, (top + total).min(full.bottom() - self.composer_height(&ctx) - 8.0)));
            egui::Area::new(egui::Id::new("newmsg-drop")).fixed_pos(drop.min).order(egui::Order::Foreground).show(&ctx, |ui| {
                ui.set_clip_rect(drop);
                ui.allocate_exact_size(drop.size(), Sense::hover());
                let p = ui.painter().clone();
                p.rect_filled(drop, 0.0, t.tk("background-primary"));
                p.rect_stroke(drop, 0.0, Stroke::new(1.0_f32, t.tk("listitem-background:pressed")), egui::StrokeKind::Inside); // listitem-background:pressed (11 %)
                let mut y = drop.top();
                for (i, l) in ls.iter().enumerate() {
                    let r = Rect::from_min_size(pos2(drop.left(), y), Vec2::new(drop.width(), l.h()));
                    y += l.h();
                    if r.top() > drop.bottom() { break; }
                    let inner = Rect::from_min_max(r.min + Vec2::new(5.6, 5.6), r.max - Vec2::new(5.6, 5.6));
                    let resp = l.pickable().then(|| ui.interact(r, egui::Id::new(("newmsg-line", i)), Sense::click()));
                    if ring == Some(i) || resp.as_ref().is_some_and(|r| r.hovered()) { p.rect_filled(inner, 8.0, t.tk("listitem-background:pressed")); }
                    if ring == Some(i) {
                        p.rect_stroke(r.shrink(0.8), 10.0, Stroke::new(2.0_f32, FOCUS), egui::StrokeKind::Inside);
                        p.rect_stroke(r.shrink(2.8), 9.0, Stroke::new(2.0_f32, t.tk("global-focus-innerline")), egui::StrokeKind::Inside);
                    }
                    match l {
                        Line::Header(h) => { p.text(pos2(r.left() + 14.4, r.center().y + 1.2), egui::Align2::LEFT_CENTER, tr(h), FontId::proportional(16.0), t.text2); }
                        Line::Empty(msg) => {
                            self.icons.art(ui, "desert-open-results-192-cobalt-lime", Rect::from_center_size(pos2(r.left() + 48.4, r.center().y), Vec2::splat(81.0))); // drawing 78 x 70 pt
                            p.text(pos2(r.left() + 114.4, r.center().y - 0.6), egui::Align2::LEFT_CENTER, tr(msg), FontId::proportional(16.0), t.text1);
                        }
                        Line::Person(pi) => {
                            let pp = pool[*pi].clone();
                            let av = Rect::from_center_size(pos2(r.left() + 24.4, r.center().y), Vec2::splat(32.0));
                            let ph = self.photo_for(&ctx, &pp.id, &pp.email);
                            let mut ac = ui.new_child(egui::UiBuilder::new().max_rect(av));
                            let name = if pp.name.is_empty() { pp.email.clone() } else { pp.name.clone() };
                            avatar_photo(&mut ac, &mut self.icons, t, &name, 32.0, false, None, None, ph);
                            let sub = self.person_line(&pp, super::super::chrome::now_ms());
                            let ny = if sub.is_empty() { r.center().y } else { r.top() + 14.2 };
                            let g = ui.fonts(|f| f.layout_job(hl_job(&name, &s.query, 16.0, t.text1)));
                            p.galley(pos2(r.left() + 52.8, ny - g.size().y / 2.0), g, t.text1);
                            if !sub.is_empty() { p.text(pos2(r.left() + 52.8, r.top() + 36.2), egui::Align2::LEFT_CENTER, sub, FontId::proportional(14.0), t.text2); }
                        }
                        Line::Space(si) => {
                            let sp = self.spaces[*si].clone();
                            let av = Rect::from_center_size(pos2(r.left() + 24.4, r.center().y), Vec2::splat(32.0));
                            let mut ac = ui.new_child(egui::UiBuilder::new().max_rect(av));
                            avatar_photo(&mut ac, &mut self.icons, t, &sp.title, 32.0, true, None, None, None);
                            let g = ui.fonts(|f| f.layout_job(hl_job(&sp.title, &s.query, 16.0, t.text1)));
                            p.galley(pos2(r.left() + 52.8, r.center().y - g.size().y / 2.0), g, t.text1);
                        }
                    }
                    if resp.is_some_and(|r| r.clicked()) { picked = Some(l.clone()); }
                }
            });
            match picked {
                Some(Line::Person(pi)) => {
                    // The person becomes a chip; the conversation with exactly the chosen people (if any) is shown below and the send goes there.
                    s.to.push(pool[pi].clone());
                    s.query.clear(); s.sel = None; s.focus = true;
                }
                Some(Line::Space(si)) => {
                    let id = self.spaces[si].id.clone();
                    set_state(&ctx, NewMsg::default());
                    self.open_space(id);
                    return;
                }
                _ => {}
            }
        }
        set_state(&ctx, s);
    }

    /// The composer's send in the pane (composer.rs hands it over): the user's own action.
    pub(crate) fn new_message_send(&mut self, ctx: &egui::Context, text: String, now: f64) {
        let s = state(ctx);
        let Some(p) = s.to.first().cloned() else { return };
        if text.trim().is_empty() { return; }
        let me = self.me.clone();
        let matched = { let dp = |sp: &Space| self.direct_person(sp); matched_space(&self.spaces, dp, &s.to, &me) };
        let who_of = |p: &Person| if p.id.is_empty() { p.email.clone() } else { p.id.clone() };
        let title = (!s.name.trim().is_empty()).then(|| s.name.trim().to_string());
        match (&self.client, matched) {
            // An existing conversation with exactly these people: the message goes there.
            (Some(c), Some(id)) => { c.req(Req::Send(id.clone(), text)); self.open_space(id); }
            (Some(c), None) if s.to.len() == 1 => {
                c.req(Req::Direct { who: who_of(&p), text, invite: false });
                ctx.data_mut(|d| d.insert_temp(egui::Id::new("newmsg-pending"), PendingDirect { id: p.id.clone(), email: p.email.clone(), ..Default::default() }));
            }
            (Some(c), None) => {
                c.req(Req::Group { title, who: s.to.iter().map(who_of).collect(), text });
                let known = self.spaces.iter().map(|x| x.id.clone()).collect();
                ctx.data_mut(|d| d.insert_temp(egui::Id::new("newmsg-pending"), PendingDirect { group: s.to.clone(), known, ..Default::default() }));
            }
            (None, matched) => {
                // --sample: local only, nothing leaves the process.
                let sid = matched.unwrap_or_else(|| {
                    let sid = format!("local-new{}", self.spaces.len());
                    let direct = s.to.len() == 1;
                    let title = title.clone().unwrap_or_else(|| s.to.iter().map(|x| x.name.clone()).collect::<Vec<_>>().join(", "));
                    self.spaces.insert(0, Space { id: sid.clone(), title, direct, participants: s.to.clone(), ..Default::default() });
                    sid
                });
                let list = self.messages.entry(sid.clone()).or_default();
                list.push(Message { id: format!("{sid}-local{}", list.len()), author: me.name.clone(), email: me.email.clone(), text, time: "now".into(), date: system::today(), mine: true, ..Default::default() });
                self.open_space(sid);
            }
        }
        set_state(ctx, NewMsg::default());
        self.drafts.remove(DRAFT_ID);
        let _ = now;
    }

    /// Opens the direct space a send / invite asked for, once the reloaded list has it (call every frame).
    pub(crate) fn finish_direct(&mut self, ctx: &egui::Context) {
        let k = egui::Id::new("newmsg-pending");
        if let Some((id, email)) = PENDING_PERSON.lock().unwrap().take() { ctx.data_mut(|d| d.insert_temp(k, PendingDirect { id, email, ..Default::default() })); }
        let Some(p) = ctx.data(|d| d.get_temp::<PendingDirect>(k)) else { return };
        let person = Person { id: p.id.clone(), email: p.email.clone(), ..Default::default() };
        let me = self.me.clone();
        let dp = |sp: &Space| self.direct_person(sp);
        let found = if p.group.is_empty() { existing_direct(&self.spaces, dp, &person).map(|x| x.id.clone()) } else {
            // The new group: a space that was not in the list before and has exactly the chosen people.
            let fresh: Vec<Space> = self.spaces.iter().filter(|x| !p.known.contains(&x.id)).cloned().collect();
            matched_space(&fresh, dp, &p.group, &me)
        };
        if let Some(id) = found {
            ctx.data_mut(|d| d.remove::<PendingDirect>(k));
            self.open_space(id);
        }
    }

    /// Remembers that the direct space with person `p` should open when it appears ("Message" / "Chat" on contact
    /// cards, `App::open_or_create_direct`, which has no egui context: handed over through `finish_direct`).
    pub(crate) fn expect_direct_person(&self, p: &Person) {
        *PENDING_PERSON.lock().unwrap() = Some((p.id.clone(), p.email.clone()));
    }

    /// Remembers that the direct space with `email` should open when it appears (the invite pane).
    pub(crate) fn expect_direct(&self, ctx: &egui::Context, email: &str) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("newmsg-pending"), PendingDirect { email: email.to_string(), ..Default::default() }));
    }
}
