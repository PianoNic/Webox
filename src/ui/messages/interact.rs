//! Reactions, threads, the seen banner, the hover toolbar and the delete and forward dialogs.
use super::*;
use crate::ui::widgets::tr;

impl App {
    /// Reaction chips (emoji | count, then who reacted when a plugin asks for it) with 4 pt spacing, then the add-reaction pill.
    pub(super) fn reactions_row(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, at: egui::Pos2, m: &Message) {
        let mut x = at.x;
        let avatars = self.plugins.reaction_avatars(); // None (no plugin): the chips look as they always did
        for (ri, r) in m.reactions.clone().into_iter().enumerate() {
            let sq = cur_seq(ui.ctx());
            let Some(glyph) = chip_emoji(&r.name) else { continue }; // a name that isn't an emoji is never shown as text
            let count = ui.painter().layout_no_wrap(r.count.to_string(), FontId::proportional(13.0), if r.mine.is_some() { c.accent } else { c.body });
            let who = self.chip_layout(ui, c, &r, avatars); // who_reacted.rs
            let (cw, aw) = (count.size().x, who.as_ref().map_or(0.0, |w| w.width));
            let (lw, rw) = (8.0 + 16.0 + 6.0, 6.0 + cw + aw + 8.0); // emoji padding 6 / 8; count padding 6 / 8
            let rect = Rect::from_min_size(pos2(x, at.y), Vec2::new(lw + 1.0 + rw, REACT_H));
            let (left, right) = (Rect::from_min_size(rect.min, Vec2::new(lw, REACT_H)), Rect::from_min_max(pos2(rect.left() + lw + 1.0, rect.top()), rect.max));
            // With avatars the count part ends just after the count: the avatars name their people themselves.
            let hit = if who.is_some() { Rect::from_min_max(right.min, pos2(right.left() + 6.0 + cw + 2.0, right.bottom())) } else { right };
            let (l, rr) = (ui.interact(left, Id::new(("react-l", &m.id, &r.name, ri, sq)), Sense::click()), ui.interact(hit, Id::new(("react-r", &m.id, &r.name, ri, sq)), Sense::click()));
            let half = (REACT_H / 2.0) as u8;
            if l.hovered() { ui.painter().rect_filled(left, egui::CornerRadius { nw: half, sw: half, ne: 0, se: 0 }, c.chip_hover); }
            if rr.hovered() { ui.painter().rect_filled(right, egui::CornerRadius { nw: 0, sw: 0, ne: half, se: half }, c.chip_hover); }
            ui.painter().rect_stroke(rect.shrink(0.5), REACT_H / 2.0, Stroke::new(1.0_f32, c.chip_border), StrokeKind::Inside);
            ui.painter().vline(rect.left() + lw + 0.5, (rect.top() + 4.0)..=(rect.bottom() - 4.0), Stroke::new(1.0_f32, c.chip_border));
            let er = Rect::from_center_size(pos2(rect.left() + 8.0 + 8.0, rect.center().y), Vec2::splat(16.0));
            if !self.icons.emoji(ui, &glyph, er) { ui.painter().text(er.center(), Align2::CENTER_CENTER, &glyph, FontId::proportional(14.0), c.body); }
            ui.painter().galley(pos2(right.left() + 6.0, rect.center().y - count.size().y / 2.0), count, c.body);
            if let Some(w) = &who { self.chip_avatars_draw(ui, t, c, m, &r, ri, w, right.left() + 6.0 + cw, rect.center().y); }
            // The emoji adds or takes back my reaction; the count shows who reacted (the reactors popover, no tooltip).
            let tip = crate::i18n::tr("ReactionsContentList", if r.mine.is_some() { "Remove reaction" } else { "Add reaction" });
            if l.on_hover_text(tip.as_ref()).clicked() {
                if r.mine.is_none() { crate::ui::theme::fx::reacted(rect); } // adding one: a design plugin's ripple
                self.toggle_reaction(&m.id, &r.name);
            }
            self.reactors_hover(&ui.ctx().clone(), m, &r, hit, &rr);
            x = rect.right() + 4.0;
        }
        // Add-reaction pill (reactions_12): height = row height, width = height + 8.
        let pill = Rect::from_min_size(pos2(x, at.y), Vec2::new(REACT_H + 8.0, REACT_H));
        let resp = ui.interact(pill, Id::new(("react-add", &m.id, cur_seq(ui.ctx()))), Sense::click());
        if resp.hovered() { ui.painter().rect_filled(pill, REACT_H / 2.0, c.chip_hover); }
        ui.painter().rect_stroke(pill.shrink(0.5), REACT_H / 2.0, Stroke::new(1.0_f32, c.chip_border), StrokeKind::Inside);
        self.icons.paint(ui, "reactions_12", Rect::from_center_size(pill.center(), Vec2::splat(12.0)), c.body);
        if resp.on_hover_text(tr("Add reaction")).clicked() { self.open_reaction_picker(&ui.ctx().clone(), &m.id, pill); }
    }

    /// "Add reaction" (toolbar ➕ and the chip row's pill): opens the shared emoji picker (emoji_picker.rs) above `anchor` and
    /// remembers which message the pick is for; `reaction_picker_draw` applies it. A second click closes it.
    pub(super) fn open_reaction_picker(&mut self, ctx: &egui::Context, msg: &str, anchor: Rect) {
        let k = Id::new("react-picker-target");
        let open = ctx.data(|d| d.get_temp::<bool>(Id::new("emoji-open"))).unwrap_or(false);
        if open && react_picker_for(ctx, msg) {
            self.emoji_picker_toggle(ctx);
            ctx.data_mut(|d| d.remove::<String>(k));
            return;
        }
        if !open { self.emoji_picker_toggle(ctx); }
        ctx.data_mut(|d| { d.insert_temp(k, msg.to_string()); d.insert_temp(Id::new("react-picker-anchor"), anchor); });
    }

    /// Draws the shared picker for a pending reaction and sends the pick through `toggle_reaction` (Req::React / the local
    /// chip in the offline sample). Called once per frame from the message list.
    pub(super) fn reaction_picker_draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let k = Id::new("react-picker-target");
        let Some(target) = ctx.data(|d| d.get_temp::<String>(k)) else { return };
        if !ctx.data(|d| d.get_temp::<bool>(Id::new("emoji-open"))).unwrap_or(false) { ctx.data_mut(|d| d.remove::<String>(k)); return; }
        let anchor = ctx.data(|d| d.get_temp::<Rect>(Id::new("react-picker-anchor"))).unwrap_or(Rect::NOTHING);
        let mut picked: Option<String> = None;
        self.emoji_picker(ui, anchor, &mut |e| picked = Some(e.to_string()));
        if let Some(e) = picked {
            self.toggle_reaction(&target, &reaction_name(&e));
            ctx.data_mut(|d| d.remove::<String>(k));
        }
    }

    /// Collapsed thread (wide mode, 48 pt): reply bar (28), the repliers' avatars (28), the ghost pill "N replies" with
    /// arrow-down_12, and the time of the last reply, spaced 3.
    pub(super) fn collapsed_thread(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, root: &Message, replies: &[&Message]) {
        let w = ui.available_width();
        let (row, resp) = ui.allocate_exact_size(Vec2::new(w, dm().thread_collapsed_h), Sense::click()); // Standard 48
        let hovered = resp.hovered();
        let x0 = row.left() + GRID_X + REPLY_LEFT;
        let cy = row.center().y;
        ui.painter().rect_filled(Rect::from_center_size(pos2(x0 + REPLY_BAR / 2.0, cy), Vec2::new(REPLY_BAR, 28.0)), 0.0, c.bar);
        let mut x = x0 + REPLY_BAR + 3.0;
        let mut seen: Vec<&str> = Vec::new();
        for r in replies { if !seen.contains(&r.email.as_str()) { seen.push(&r.email); } }
        for em in seen.iter().take(4) {
            let r = replies.iter().find(|r| r.email == *em).unwrap();
            let (pid, pem) = self.people.iter().find(|q| q.email == r.email || q.name == r.author).map(|q| (q.id.clone(), q.email.clone())).unwrap_or((String::new(), r.email.clone()));
            let ph = self.photo_for(ui.ctx(), &pid, &pem);
            let ar = Rect::from_center_size(pos2(x + 15.0, cy), Vec2::splat(28.0));
            if r.mine {
                ui.painter().circle_stroke(ar.center(), 13.5, Stroke::new(1.0_f32, c.ring));
                self.icons.paint(ui, "chat-filled", Rect::from_center_size(ar.center() + Vec2::new(0.0, 0.3), Vec2::splat(16.5)), c.bubble);
            } else {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(ar));
                avatar_photo(&mut child, &mut self.icons, t, &r.author, 28.0, false, None, None, ph);
            }
            x += 30.0 + 6.0;
        }
        x += 3.0 - 6.0;
        let label = if replies.len() == 1 { tr("1 reply").to_string() } else { crate::trq!("{{0}} replies", replies.len()) };
        let g = ui.painter().layout_no_wrap(label, semibold(SECOND), c.body); // ghost style (accent only for new replies)
        let pill = Rect::from_min_size(pos2(x, cy - 14.0), Vec2::new(12.0 + g.size().x + 6.0 + 12.0 + 12.0, 28.0));
        let pr = ui.interact(pill, Id::new(("thread-open", &root.id)), Sense::click());
        if pr.hovered() || hovered { ui.painter().rect_filled(pill, 14.0, c.chip_hover); }
        ui.painter().galley(pos2(pill.left() + 12.0, cy - g.size().y / 2.0), g.clone(), c.body);
        self.icons.paint(ui, "arrow-down_12", Rect::from_center_size(pos2(pill.left() + 12.0 + g.size().x + 6.0 + 6.0, cy), Vec2::splat(12.0)), c.body);
        let last = replies.last().map_or("", |r| r.time.as_str());
        ui.painter().text(pos2(pill.right() + 3.0, cy), Align2::LEFT_CENTER, last, FontId::proportional(SECOND), c.head);
        if pr.clicked() || resp.clicked() { self.open_threads.insert(root.id.clone()); }
    }

    /// Thread end buttons: secondary pills (top padding 5, spacing 5): "Reply in thread" (reply_12) and, when the thread
    /// can collapse, "Hide" (hide-bold).
    pub(super) fn thread_end(&mut self, ui: &mut egui::Ui, _t: &Th, c: &Mc, root: &Message, collapsible: bool) {
        let w = ui.available_width();
        let (row, _) = ui.allocate_exact_size(Vec2::new(w, 5.0 + 28.0 + dm().thread_end_bottom), Sense::hover()); // Standard 5 + 28 + 8
        let mut x = row.left() + GRID_X + REPLY_LEFT + 1.0;
        let y = row.top() + 5.0;
        let mut pill = |s: &mut Self, ui: &mut egui::Ui, icon: &'static str, label: &str, key: &str| -> bool {
            let g = ui.painter().layout_no_wrap(label.to_string(), semibold(SECOND), c.body);
            let r = Rect::from_min_size(pos2(x, y), Vec2::new(12.0 + 12.0 + 6.0 + g.size().x + 12.0, 28.0));
            let resp = ui.interact(r, Id::new((key, &root.id)), Sense::click());
            if resp.hovered() { ui.painter().rect_filled(r, 14.0, c.chip_hover); }
            ui.painter().rect_stroke(r.shrink(0.5), 14.0, Stroke::new(1.0_f32, c.chip_border), StrokeKind::Inside);
            s.icons.paint(ui, icon, Rect::from_center_size(pos2(r.left() + 12.0 + 6.0, r.center().y), Vec2::splat(12.0)), c.body);
            ui.painter().galley(pos2(r.left() + 12.0 + 12.0 + 6.0, r.center().y - g.size().y / 2.0), g, c.body);
            x = r.right() + 5.0;
            resp.clicked()
        };
        if pill(self, ui, "reply_12", tr("Reply to thread"), "thread-reply") { self.start_reply(root); }
        if collapsible && pill(self, ui, "hide-bold", tr("Hide"), "thread-hide") { self.open_threads.remove(&root.id); }
    }

    /// "Seen by": label and the readers' avatars, centred at the row centre - 20: label, avatars 24 pt on a 32 pt pitch (the first
    /// 13.6 pt after the label), then the overflow "+N".
    pub(super) fn seen_banner(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, people: &[Person]) {
        let w = ui.available_width();
        let (row, _) = ui.allocate_exact_size(Vec2::new(w, seen_h() + SEEN_TOP), Sense::hover());
        let cy = row.top() + SEEN_TOP + seen_h() / 2.0;
        let g = ui.painter().layout_no_wrap(tr("Seen by").to_string(), FontId::proportional(SECOND), c.body);
        let (shown, overflow) = seen_overflow(people.len());
        let more_txt = format!("+{}", people.len().saturating_sub(shown));
        let ow = if overflow > 0 { (ui.fonts(|f| f.layout_no_wrap(more_txt.clone(), FontId::proportional(SECOND), c.body).size().x) + 10.0).max(24.0) } else { 0.0 };
        let (left, slot0) = seen_layout(row.center().x, g.size().x, shown, ow);
        ui.painter().galley(pos2(left, cy - g.size().y / 2.0), g.clone(), c.body);
        let mut x = slot0 + 1.0; // the 24 pt avatar centred in its 26 pt slot
        for pp in people.iter().take(shown) {
            let ph = self.photo_for(ui.ctx(), &pp.id, &pp.email);
            let st = self.people.iter().find(|q| q.email == pp.email).map(|q| q.status.clone());
            let ar = Rect::from_min_size(pos2(x, cy - 12.0), Vec2::splat(24.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(ar));
            avatar_photo(&mut child, &mut self.icons, t, &pp.name, 24.0, false, None, st.as_deref(), ph);
            if let Some(space) = self.selected.as_deref() {
                if crate::ui::typing::is_typing(space, pp) { crate::ui::typing::draw_overlay(ui.painter(), ar.expand2(Vec2::new(1.0, 0.0)), crate::ui::typing::anim_time(ui.ctx())); } // the whole 26 pt slot
            }
            ui.interact(ar, Id::new(("seen-avatar", &pp.email, &pp.id)), Sense::hover()).on_hover_text(&pp.name);
            x += 32.0;
        }
        let x = x - 1.0; // back to the slot grid: the overflow pill follows the last slot
        // "+N" for the rest (round, wx-filesAppButton-background, secondary text, hover lists the names).
        if overflow > 0 {
            let more = &people[shown..];
            let r = Rect::from_min_size(pos2(x, cy - 12.0), Vec2::new(ow, 24.0));
            let resp = ui.interact(r, Id::new("seen-overflow"), Sense::hover());
            ui.painter().rect_filled(r, 12.0, if c.body.r() > 128 { hex(0x292929) } else { hex(0xEDEDED) });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, &more_txt, FontId::proportional(SECOND), c.body);
            resp.on_hover_text(more.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join("\n"));
        }
    }

    /// Hover toolbar above the top right of the hovered message: 24 pt high, 32 pt per button, background
    /// wx-menuList-background, border wx-default-border, a divider after every button; 👍 😀 … favourites, add reaction,
    /// edit (yours), reply in thread, forward, and "···" for the rest. Position: right = box right - 2*radius,
    /// top = box top - 9 (with a header) or - 20 (grouped).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn message_toolbar(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, id: &str, m: &Message, boxr: Rect, head: bool, bar_key: Id, list_top: f32) {
        let k_more = format!("msg-more-{}", m.id);
        let mut more_rect = Rect::NOTHING;
        // Settings > Customize the interface hides buttons; "···" (all actions) always stays.
        let g = &crate::ui::customize::MSG_BAR;
        let on = |id: &str| self.ui_shown(g, id);
        let (quick, react, edit, reply, fwd) = (on("quick"), on("react"), m.mine && on("edit"), on("reply"), on("forward"));
        let n = if quick { QUICK.len() } else { 0 } + usize::from(react) + usize::from(edit) + usize::from(reply) + usize::from(fwd) + 1;
        let w = TB_W * n as f32;
        let top = (boxr.top() + if head { -9.0 } else { -20.0 }).max(list_top + 1.0); // never above the message area
        let pos = pos2(boxr.right() - 2.0 * RADIUS - w, top);
        let ctx = ui.ctx().clone();
        ctx.data_mut(|d| d.insert_temp(bar_key, Rect::from_min_size(pos, Vec2::new(w, TB_H)).expand(2.0)));
        let bar_layer = egui::LayerId::new(egui::Order::Foreground, Id::new(("msg-toolbar", id, &m.id, cur_seq(ui.ctx()))));
        crate::ui::scroll_route::scroll_passthrough(&ctx, bar_layer); // the wheel over the bar scrolls the list
        crate::ui::type_focus::passive(&ctx, bar_layer); // not a popup: typing still goes to the composer
        egui::Area::new(bar_layer.id).fixed_pos(pos).order(egui::Order::Foreground).show(&ctx, |ui| {
            let (bar, _) = ui.allocate_exact_size(Vec2::new(w, TB_H), Sense::hover());
            let half = (TB_H / 2.0) as u8;
            ui.painter().rect_filled(bar, half, c.pill_bg);
            let mut i = 0usize;
            let mut button = |s: &mut Self, ui: &mut egui::Ui, icon: Option<&'static str>, emo: Option<&str>, tip: &str| -> egui::Response {
                let r = Rect::from_min_size(bar.min + Vec2::new(TB_W * i as f32, 0.0), Vec2::new(TB_W, TB_H));
                let resp = ui.interact(r, Id::new(("tb", id, &m.id, i, cur_seq(ui.ctx()))), Sense::click());
                if resp.hovered() || resp.is_pointer_button_down_on() {
                    let cr = egui::CornerRadius { nw: if i == 0 { half } else { 0 }, sw: if i == 0 { half } else { 0 }, ne: if i == n - 1 { half } else { 0 }, se: if i == n - 1 { half } else { 0 } };
                    ui.painter().rect_filled(r.shrink(1.0), cr, c.pill_hover);
                }
                let ir = Rect::from_center_size(r.center(), Vec2::splat(16.0));
                if let Some(e) = emo { if !s.icons.emoji(ui, e, ir) { ui.painter().text(ir.center(), Align2::CENTER_CENTER, e, FontId::proportional(14.0), c.white); } }
                if let Some(n) = icon { s.icons.paint(ui, n, ir, c.white); }
                if i + 1 < n { ui.painter().vline(r.right() - 0.5, (r.top() + 1.0)..=(r.bottom() - 1.0), Stroke::new(1.0_f32, c.pill_border)); }
                i += 1;
                resp.on_hover_text(tip)
            };
            if quick { for (ch, tip) in QUICK { if button(self, ui, None, Some(ch), tr(tip)).clicked() { self.toggle_reaction(&m.id, &reaction_name(ch)); } } }
            if react {
                let add = button(self, ui, Some("reactions_12"), None, tr("Add reaction"));
                let shot_open = self.shot.is_some() && self.frames == 6 && std::env::var("WEBOX_SHOT_REACT_PICKER").is_ok(); // screenshot check only
                if add.clicked() || shot_open { self.open_reaction_picker(&ctx, &m.id, add.rect); }
            }
            if edit && button(self, ui, Some("edit_12"), None, &format!("{} ({})", tr("Edit"), tr("Shift+Up arrow"))).clicked() { self.start_edit(m); }
            if reply && button(self, ui, Some("reply_12"), None, tr("Reply in thread")).clicked() { self.start_reply(m); }
            if fwd && button(self, ui, Some("forward-message_12"), None, tr("Forward")).clicked() { self.forward_text = Some(m.text.clone()); }
            let more = button(self, ui, Some("more_12"), None, tr("More actions"));
            if more.clicked() {
                menu::menu_toggle(&ctx, &k_more);
                menu::menu_close(&ctx, &super::ctx_menu::remind_key(&k_more));
                menu::menu_close(&ctx, &super::ctx_menu::react_key(&k_more));
                if menu::menu_is_open(&ctx, &k_more) { self.message_menu_opened(&ctx, id, m, &k_more); }
            }
            more_rect = more.rect;
            // Border: drawn last so the hover fills don't cover it.
            ui.painter().rect_stroke(bar.shrink(0.5), half, Stroke::new(1.0_f32, c.pill_border), StrokeKind::Inside);
        });
        // The "···" menu (shared menu toolkit): drawn every frame while open, anchored at the button.
        if [k_more.clone(), super::ctx_menu::remind_key(&k_more), super::ctx_menu::react_key(&k_more)].iter().any(|x| menu::menu_is_open(&ctx, x)) {
            let items = self.message_entries(&ctx, id, m);
            if let Some(a) = self.message_menu_show(&ctx, t, id, m, &k_more, more_rect, menu::Side::Below, menu::Align::Right, true, items, true) { self.message_do(&ctx, id, m, &a, None); }
        }
    }

    /// Entries shared by the "···" menu and the right-click menu (ui/msg_actions.rs).
    /// While several messages are selected the menu has only "Copy selected messages" (no icon).
    pub(super) fn message_entries(&self, ctx: &egui::Context, id: &str, m: &Message) -> Vec<menu::Entry> {
        if msel(ctx, id).multi { return vec![menu::Item::new("copy-selected", tr("Copy selected messages")).entry()]; }
        self.msg_menu_entries(id, m)
    }

    /// Runs a picked menu entry. Delete asks first (dialog in `message_list`). (ui/msg_actions.rs)
    pub(super) fn message_do(&mut self, ctx: &egui::Context, id: &str, m: &Message, action: &str, link: Option<&str>) {
        if action == "copy-selected" { copy_selected(ctx, id); return; }
        self.msg_menu_do(ctx, id, m, action, link)
    }

    /// Delete confirmation. Texts: "Delete message" (title), "Are you sure you want to delete this message?" (body),
    /// "Cancel" / "Delete" (buttons).
    pub(super) fn delete_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        let Some(mid) = ctx.data(|d| d.get_temp::<String>(Id::new("msg-delete-confirm"))) else { return };
        let res = menu::webex_dialog(ctx, t, "msg-delete", tr("Delete message"), tr("Are you sure you want to delete this message?"), None,
            &[menu::Button::secondary("cancel", tr("Cancel")), menu::Button::danger("delete", tr("Delete"))]);
        if let Some(r) = res {
            if r == "delete" { self.delete_message(&mid); }
            ctx.data_mut(|d| d.remove::<String>(Id::new("msg-delete-confirm")));
        }
    }

    /// Pick the space to forward a message to.
    pub(super) fn forward_dialog(&mut self, ctx: &egui::Context, t: &Th, from: &str) {
        let Some(text) = self.forward_text.clone() else { return };
        let spaces: Vec<(String, String)> = self.spaces.iter().filter(|s| s.id != from && !s.hidden).map(|s| (s.id.clone(), s.title.clone())).collect();
        let mut target: Option<String> = None;
        let fg = if menu::light(t) { Color32::BLACK } else { Color32::WHITE };
        let mut list = |ui: &mut egui::Ui| {
            ui.label(RichText::new(text.chars().take(160).collect::<String>()).color(wa(fg, 0.7)).size(13.0));
            ui.add_space(8.0);
            crate::ui::widgets::vscroll().max_height(260.0).show(ui, |ui| {
                for (sid, title) in &spaces {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
                    if resp.hovered() { ui.painter().rect_filled(r, 8.0, wa(fg, 0.15)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, title, FontId::proportional(15.0), wa(fg, 0.95));
                    if resp.clicked() { target = Some(sid.clone()); }
                }
            });
        };
        let res = menu::webex_dialog(ctx, t, "msg-forward", tr("Forward message"), "", Some(&mut list), &[menu::Button::secondary("cancel", tr("Cancel"))]);
        if let Some(to) = target { self.forward(&to, &text); self.forward_text = None; }
        else if res.is_some() { self.forward_text = None; }
    }
}
