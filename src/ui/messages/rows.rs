//! One row of the list: seen-by, notices, grouping, day lines, measuring and drawing a message.
use super::*;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// A measured message body, reused while nothing it depends on changed (see `App::measure_cached`).
struct RowEntry { used: u64, content: Rc<Content> }
thread_local! {
    /// Measured rows by content key (UI thread only). Entries unused for a while are dropped once the map grows.
    static ROWS: RefCell<std::collections::HashMap<u64, RowEntry>> = RefCell::new(std::collections::HashMap::new());
}
/// Entries kept at most before unused ones are swept, and how many passes "unused" means.
const ROW_CACHE_SWEEP: usize = 4096;
const ROW_CACHE_IDLE_PASSES: u64 = 600;

thread_local! {
    /// Height of a message header line by (sender name, pixels per point): the height of the name's galley.
    static HDR_H: RefCell<std::collections::HashMap<(String, u32), f32>> = RefCell::new(std::collections::HashMap::new());
}
/// The height of a header line with this sender name (the name's galley height; laid out once per name and scale).
fn header_height(ui: &egui::Ui, who: &str) -> f32 {
    let k = (who.to_string(), ui.ctx().pixels_per_point().to_bits());
    if let Some(h) = HDR_H.with(|m| m.borrow().get(&k).copied()) { return h; }
    let h = ui.painter().layout_no_wrap(who.to_string(), FontId::proportional(SECOND), Color32::WHITE).size().y;
    HDR_H.with(|m| { let mut m = m.borrow_mut(); if m.len() > 2048 { m.clear(); } m.insert(k, h); });
    h
}

impl App {
    /// "Gesehen von": who has seen the conversation's newest message (whoever wrote it). The banner is always the list's last row,
    /// at the very end of the conversation. The readers come from `seen_keys`; the offline sample has no marks and shows the chat
    /// partner when my message is the newest one.
    pub(super) fn seen_by(&self, id: &str, msgs: &[Message]) -> Option<Vec<Person>> {
        // The read-receipt avatars are off when the account has `ucf-disable-shrunken-heads`.
        if self.feature("ucf-disable-shrunken-heads").and_then(|v| v.as_bool()) == Some(true) { return None; }
        let marks = self.read_marks(id);
        if self.client.is_none() && marks.is_empty() {
            if let Some(mine) = msgs.iter().rposition(|m| m.mine).filter(|&i| !msgs[i + 1..].iter().any(|m| !m.mine && !m.email.is_empty())) {
                if mine + 1 != msgs.len() || !msgs[mine].parent.is_empty() { return None; }
                let space = self.spaces.iter().find(|s| s.id == id)?;
                if !space.direct { return None; }
                return self.direct_person(space).map(|p| vec![p]);
            }
        }
        let readers: Vec<Person> = seen_keys(msgs, marks).into_iter().map(|(email, pid, name)| {
            let known = self.people.iter().find(|p| (!email.is_empty() && p.email.eq_ignore_ascii_case(&email)) || (!pid.is_empty() && p.id == pid));
            known.cloned().unwrap_or_else(|| Person { id: pid.clone(), name: if !name.is_empty() { name.clone() } else if email.is_empty() { pid.clone() } else { email.clone() }, email: email.clone(), status: String::new() })
        }).collect();
        (!readers.is_empty()).then_some(readers)
    }

    /// A system line (call log, joins / leaves, renames, ...): centred secondary text, no avatar, no box, the time at
    /// the end ("Jonas Weber war nicht erreichbar. 16:11"); height = text + the info-row padding.
    pub(super) fn notice_row(&self, ui: &mut egui::Ui, c: &Mc, m: &Message) {
        let w = ui.available_width();
        // The info row's time is stamped like a message header: "HH:MM" today / yesterday, else
        // "15.12.2025, 15:08" ("Sie haben Ihre Nachricht gelöscht. 15.12.2025, 15:08").
        let (today, yesterday) = day_ref(self.client.is_some());
        let when = header_time(&m.date, &m.time, &today, &yesterday);
        let mut job = egui::text::LayoutJob::simple(format!("{} {when}", m.notice).trim().to_string(), FontId::proportional(SECOND), c.day, (w - 20.0).max(40.0));
        job.halign = egui::Align::Center;
        let g = ui.fonts(|f| f.layout_job(job));
        let (r, _) = ui.allocate_exact_size(Vec2::new(w, g.size().y + day_pad()), Sense::hover());
        record_row(ui.ctx(), &m.id, r);
        self.info_row_select(ui, m, r);
        // With halign Center the galley is centred on its origin x.
        ui.painter().galley(pos2(r.center().x - g.rect.center().x, r.top() + day_pad() / 2.0), g, c.day);
    }

    /// A collapsed join/leave row: the info row in the link colour (`text-hyperlink`) with arrow-down_12 / arrow-up_12 12 pt from
    /// the right of the text; a click opens a popover (428 wide) listing the people; leaving the row closes it.
    pub(super) fn group_row(&mut self, ui: &mut egui::Ui, c: &Mc, m: &Message) {
        let w = ui.available_width();
        let (today, yesterday) = day_ref(self.client.is_some());
        let when = header_time(&m.date, &m.time, &today, &yesterday);
        let g = ui.painter().layout(format!("{} {when}", m.notice).trim().to_string(), FontId::proportional(SECOND), c.accent, (w - 60.0).max(40.0));
        let (r, resp) = ui.allocate_exact_size(Vec2::new(w, g.size().y + day_pad()), Sense::click());
        record_row(ui.ctx(), &m.id, r);
        self.info_row_select(ui, m, r);
        let key = Id::new(("group-row-open", &m.id));
        let mut open = ui.ctx().data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
        if resp.clicked() { open = !open; }
        if !resp.hovered() { open = false; } // leaving the row closes the popover
        ui.ctx().data_mut(|d| d.insert_temp(key, open));
        if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        let tw = g.size().x + 12.0 + 12.0; // text + 12 pt gap + 12 pt arrow
        let x0 = r.center().x - tw / 2.0;
        let ty = r.top() + day_pad() / 2.0;
        let gh = g.size().y;
        ui.painter().galley(pos2(x0, ty), g, c.accent);
        self.icons.paint(ui, if open { "arrow-up_12" } else { "arrow-down_12" }, Rect::from_center_size(pos2(x0 + tw - 6.0, ty + gh / 2.0), Vec2::splat(12.0)), c.accent);
        if open {
            crate::ui::scroll_route::scroll_passthrough(ui.ctx(), egui::LayerId::new(egui::Order::Foreground, key.with("pop")));
            egui::Area::new(key.with("pop")).order(egui::Order::Foreground).fixed_pos(pos2(r.center().x - 214.0, r.bottom()))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(428.0);
                        for p in &m.group {
                            let name = if p.name.is_empty() { p.email.clone() } else { p.name.clone() };
                            ui.label(egui::RichText::new(name).font(FontId::proportional(SECOND)).color(c.body));
                        }
                    });
                });
        }
    }

    /// An info row (notice, join/leave) as a row of the selection: it can be pressed / dragged over like a message and is copied
    /// with its text only (no name, not grouped).
    fn info_row_select(&self, ui: &egui::Ui, m: &Message, r: Rect) {
        if super::super::widgets::hover_ok(ui, r) { record_pointer_row(ui.ctx(), &m.id, ui.input(|i| i.pointer.primary_pressed())); }
        let esc = m.notice.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        record_copy(ui.ctx(), CopyRow { id: m.id.clone(), stamp: copy_stamp(&m.date, &m.time), body: esc, ..Default::default() });
    }

    /// Day separator: secondary label centred, 1 px lines of the same colour from x=75 to the label and from the label to the
    /// right edge minus 75; the label has 10 pt side padding.
    pub(super) fn day_line(&self, ui: &mut egui::Ui, _t: &Th, c: &Mc, label: &str, col: Color32) {
        let w = ui.available_width();
        let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(SECOND), col);
        let (r, _) = ui.allocate_exact_size(Vec2::new(w, g.size().y + day_pad()), Sense::hover());
        let (cx, cy) = (r.center().x, r.center().y);
        let (l, rr) = (cx - g.size().x / 2.0 - 10.0, cx + g.size().x / 2.0 + 10.0);
        ui.painter().galley(pos2(cx - g.size().x / 2.0, cy - g.size().y / 2.0), g, col);
        ui.painter().hline(r.left() + 75.0..=l, cy.round() + 0.5, Stroke::new(1.0, col));
        ui.painter().hline(rr..=r.right() - 75.0, cy.round() + 0.5, Stroke::new(1.0, col));
    }

    /// Measures the body of a message: text (links / mentions), file chips and image heights.
    pub(super) fn measure(&mut self, ui: &egui::Ui, c: &Mc, m: &Message, shown: &str, wrap: f32) -> Content {
        let mut files = Vec::new();
        let mut text = Vec::new();
        let same = shown == m.text; // a plugin that rewrote the text also disables the rich text
        let shown = normalize_breaks(shown);
        for line in shown.split('\n') {
            match line.trim_start().strip_prefix('📎') { Some(n) => files.push(n.trim().to_string()), None => text.push(line) }
        }
        let body = text.join("\n");
        let body = body.trim_matches('\n');
        let sys = emoji_text::ensure_font(ui.ctx());
        // The message's rich text (object.content) is drawn; fall back to the plain text when there is none / it fails.
        let rich = if same && !m.html.trim().is_empty() { html::html_job(&m.html, c, &self.me.id, wrap, sys) } else { None };
        let (galley, links, emojis, quotes, jump) = if let Some(p) = rich {
            (Some(ui.fonts(|f| f.layout_job(p.job))), p.links, p.runs, p.quotes, p.reply_quote)
        } else if body.trim().is_empty() { (None, Vec::new(), Vec::new(), Vec::new(), None) } else {
            let (job, links, emojis) = body_job(body, c, &self.me.name, wrap, sys);
            (Some(ui.fonts(|f| f.layout_job(job))), links, emojis, Vec::new(), None)
        };
        let mut h = 0.0;
        let mut first = true;
        let (files, file_atts, videos) = split_files(m, files);
        h += self.attachments_height(ui, c, m, &videos, files.len(), wrap, &mut first);
        let attach = !m.images.is_empty() || !videos.is_empty() || !files.is_empty() || !m.previews.is_empty();
        if attach { h += ATTACH_BOTTOM; } // bottom padding of the attachments column
        if let Some(g) = &galley { if attach { h += CAPTION_GAP; } h += g.size().y; }
        if !m.reactions.is_empty() { h += 2.0 + REACT_H; } // 2 pt spacer + reactions row
        Content { text: galley, links, emojis, quotes, jump, files, file_atts, videos, h }
    }

    /// `measure`, reused across frames: the key covers everything `measure` reads (message id, text, rich text, the plugin's
    /// rewrite, wrap width, theme, message font scale, pixels per point, emoji font, my id / name for mentions, image sizes as
    /// decoded, link-card texts, whether there are reactions), so a reused row is exactly what a fresh `measure` returns.
    /// Without it, re-parsing every message's HTML and laying out every row each frame would grow with the list's length.
    pub(super) fn measure_cached(&mut self, ui: &egui::Ui, c: &Mc, m: &Message, shown: &str, wrap: f32) -> Rc<Content> {
        let ctx = ui.ctx().clone();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // A message stamped on arrival (`Message.rev`, a hash of its content) and shown as it is (no plugin rewrite): its
        // revision stands for the text, rich text, link cards and reactions, so they needn't be hashed again every frame.
        if m.rev != 0 && std::ptr::eq(shown, m.text.as_str()) {
            (1u8, &m.id, m.rev).hash(&mut h);
        } else {
            (0u8, &m.id, shown, &m.text, &m.html).hash(&mut h);
            for p in &m.previews { (&p.title, &p.description, &p.site).hash(&mut h); }
            m.reactions.is_empty().hash(&mut h);
        }
        (wrap.to_bits(), c.body, c.white, c.accent, crate::ui::theme::density::get()).hash(&mut h); // density: the attachment gap
        (&self.me.id, &self.me.name, emoji_text::ensure_font(&ctx), super::super::widgets::msg_scale().to_bits(), ctx.pixels_per_point().to_bits()).hash(&mut h);
        for a in &m.images { (&a.key, a.width, a.height, self.photos.image_size(&ctx, &a.key)).hash(&mut h); }
        // a video card's height follows its poster once the first frame is decoded
        for a in m.files.iter().chain(&m.images).filter(|a| is_video(a)) { self.video_known_size(&ctx, a).hash(&mut h); }
        let key = h.finish();
        let pass = ctx.cumulative_pass_nr();
        let hit = ROWS.with(|r| r.borrow_mut().get_mut(&key).map(|e| { e.used = pass; e.content.clone() }));
        if let Some(c) = hit { return c; }
        let content = Rc::new(self.measure(ui, c, m, shown, wrap));
        ROWS.with(|r| {
            let mut r = r.borrow_mut();
            if r.len() >= ROW_CACHE_SWEEP { r.retain(|_, e| pass.saturating_sub(e.used) < ROW_CACHE_IDLE_PASSES); }
            r.insert(key, RowEntry { used: pass, content: content.clone() });
        });
        content
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn message_row(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, id: &str, m: &Message, head: bool, in_thread: bool, indent: f32, force: bool) {
        // Unique per row and frame-stable (the n-th row of the list): ids built from the message id alone clash when the same
        // message / reaction / file shows up twice, which egui reports as "First use of widget ID".
        let seq = next_seq(ui.ctx());
        // A plugin may rewrite what is shown; without one the message's own text is used as is (no copy per row and frame).
        let rewritten = self.plugins.rewrites_messages().then(|| self.plugins.on_message(&m.author, &m.text));
        let shown: &str = rewritten.as_deref().unwrap_or(m.text.as_str());
        let width = ui.available_width();
        let box_left = indent + BOX_LEFT;
        let box_w = (width - RIGHT_MARGIN - box_left + GRID_X).max(120.0);
        let wrap = box_w - 2.0 * RADIUS;
        let ct = self.measure_cached(ui, c, m, shown, wrap);
        let who = if m.mine { tr("You").to_string() } else { m.author.clone() };
        // The header line's height is that of the name's galley; the galleys themselves are laid out only for rows on screen.
        let hdr_h = if head { header_height(ui, &who) } else { 0.0 };
        let content_h = ct.h + if head { hdr_h + header_to_body() } else { 0.0 };
        let row_h = top_pad() + content_h + bottom_pad();
        let (row, _) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::hover());
        { let top0 = ui.ctx().data(|d| d.get_temp::<f32>(Id::new("msg-top0"))).unwrap_or(0.0); ui.ctx().data_mut(|d| { d.get_temp_mut_or_insert_with(Id::new("msg-rowpos"), std::collections::HashMap::<String, f32>::new).insert(m.id.clone(), row.top() - top0); }); }
        record_row(ui.ctx(), &m.id, row);
        let origin = row.min;
        let boxr = Rect::from_min_max(origin + Vec2::new(box_left, 0.0), pos2(origin.x + box_left + box_w, row.bottom())).shrink2(Vec2::new(0.0, 0.5));
        // Hover: the row, its toolbar, or an open menu of it.
        let bar_key = Id::new(("msg-bar", id, &m.id));
        let menu_open = menu::menu_is_open(ui.ctx(), &format!("msg-more-{}", m.id)) || menu::menu_is_open(ui.ctx(), &format!("msg-ctx-{}", m.id)) || react_picker_for(ui.ctx(), &m.id);
        let prev_bar: Option<Rect> = ui.ctx().data(|d| d.get_temp(bar_key));
        // Layer-aware: nothing on top of the row (image viewer, dialog, menu, ...) may be hovering it; the row's own toolbar (its own
        // layer, above the row) keeps it hovered.
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let bar_layer = egui::LayerId::new(egui::Order::Foreground, Id::new(("msg-toolbar", id, &m.id, seq)));
        let inside = super::super::widgets::hover_ok(ui, row);
        let on_bar = pointer.is_some_and(|p| prev_bar.is_some_and(|b| b.contains(p)) && ui.ctx().layer_id_at(p) == Some(bar_layer));
        let overlay = ui.ctx().data(|d| d.get_temp::<String>(Id::new("lightbox"))).is_some(); // the screenshot-only forced hover obeys overlays too
        let hovered = (force && !overlay) || inside || on_bar || menu_open;
        if !hovered { ui.ctx().data_mut(|d| d.remove::<Rect>(bar_key)); }
        // Multi-selection (multisel.rs): the row under the pointer / pressed, and the row as "copy selected messages" reads it.
        if inside { record_pointer_row(ui.ctx(), &m.id, ui.input(|i| i.pointer.primary_pressed())); }
        record_copy(ui.ctx(), CopyRow { id: m.id.clone(), name: if head { who.clone() } else { String::new() }, stamp: copy_stamp(&m.date, &m.time), grouped: !head, body: body_html(m) });
        let picked = msel(ui.ctx(), id).highlighted(&m.id);

        let vis = ui.is_rect_visible(row.expand(40.0));
        if vis {
            let (today_s, yest_s) = day_ref(self.client.is_some());
            let hdr = head.then(|| {
                let mk = |s: &str| ui.painter().layout_no_wrap(s.to_string(), FontId::proportional(SECOND), c.head);
                (mk(&who), mk(&header_time(&m.date, &m.time, &today_s, &yest_s)), m.edited.then(|| mk(tr("Edited"))))
            });
            let p = ui.painter().clone();
            if in_thread && indent > 0.0 || in_thread && head && indent > 0.0 {
                p.rect_filled(Rect::from_min_max(pos2(origin.x + GRID_X + REPLY_LEFT, row.top() + 4.0), pos2(origin.x + GRID_X + REPLY_LEFT + REPLY_BAR, row.bottom())), 0.0, c.bar);
            }
            // Multi-selection (checked) wins over hovered.
            if picked { p.rect_filled(boxr, RADIUS, c.checked); } else if hovered { p.rect_filled(boxr, RADIUS, c.hover_bg); }
            // "Zur Originalnachricht springen": the target flashes for a moment
            if let Some((fid, t0)) = ui.ctx().data(|d| d.get_temp::<(String, f64)>(Id::new("msg-flash"))) {
                if fid == m.id { let k = ((ui.input(|i| i.time) - t0) / 1.8) as f32; if k < 1.0 { p.rect_filled(boxr, RADIUS, wa(c.accent, 0.28 * (1.0 - k))); ui.ctx().request_repaint(); } }
            }
            // Avatar: own messages carry the blue chat bubble in a ring, others the profile picture.
            if head {
                let ar = Rect::from_min_size(origin + Vec2::new(indent + GRID_X + AVATAR_LEFT, avatar_top()), Vec2::splat(AVATAR));
                if m.mine {
                    let rc = pos2(ar.center().x.round(), ar.center().y.round()); // pixel-aligned so the 1 pt ring stays crisp
                    p.circle_stroke(rc, AVATAR / 2.0 - 0.5, Stroke::new(1.0, c.ring));
                    // "chat-filled" is the speech bubble (an icon-font glyph, drawable since the icon loader gives such glyphs their em box),
                    // drawn at 19.8 pt so the bubble itself is ~16 pt.
                    self.icons.paint(ui, "chat-filled", Rect::from_center_size(ar.center() + Vec2::new(0.0, 0.3), Vec2::splat(19.8)), c.bubble);
                } else {
                    let st = self.people.iter().find(|q| q.email == m.email).map(|q| q.status.clone());
                    let (pid, pem) = self.people.iter().find(|q| q.email == m.email || q.name == m.author).map(|q| (q.id.clone(), q.email.clone())).unwrap_or((String::new(), m.email.clone()));
                    let ph = self.photo_for(ui.ctx(), &pid, &pem);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(ar));
                    avatar_photo(&mut child, &mut self.icons, t, &m.author, AVATAR, false, None, st.as_deref(), ph);
                }
            }
            let tx = origin.x + box_left + RADIUS;
            let mut y = row.top() + top_pad();
            if let Some((name, time, edited)) = hdr {
                let mut x = tx;
                let hy = y;
                p.galley(pos2(x, hy), name.clone(), c.head);
                x += name.size().x + HEADER_GAP;
                p.galley(pos2(x, hy), time.clone(), c.head);
                x += time.size().x + HEADER_GAP;
                if let Some(e) = edited { p.galley(pos2(x, hy), e.clone(), c.head); x += e.size().x + HEADER_GAP; }
                if self.flags.contains(&(id.to_string(), m.id.clone())) { self.icons.paint(ui, "flag-active_12", Rect::from_min_size(pos2(x, hy + (hdr_h - 14.0) / 2.0), Vec2::splat(14.0)), c.accent); }
                y += hdr_h + header_to_body();
            }
            let n_attach = self.draw_attachments(ui, t, c, m, &ct, pos2(tx, y), wrap, &mut y);
            if let Some(g) = &ct.text {
                if n_attach > 0 { y += CAPTION_GAP; }
                let tr_ = Rect::from_min_size(pos2(tx, y), g.size());
                let origin = tr_.min + Vec2::new(0.0, TEXT_SHIFT);
                let ctx = ui.ctx().clone();
                // Selectable read-only text: drag selects inside one message, double click a word, triple click the paragraph, Ctrl+C
                // copies, right click offers copy / select all.
                let resp = ui.interact(tr_, Id::new(("msg-text", id, &m.id, seq)), Sense::click_and_drag());
                let k_sel = Id::new("msg-sel");
                let total = g.job.text.chars().count();
                let char_at = |pp: egui::Pos2| g.cursor_from_pos(pp - origin).ccursor.index.min(total);
                let link_here = resp.hover_pos().and_then(|pp| link_at(g, &ct.links, pp - origin));
                if resp.hovered() {
                    ctx.set_cursor_icon(if link_here.is_some() { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Text });
                    if ui.input(|i| i.pointer.primary_pressed()) { ctx.data_mut(|d| d.insert_temp(Id::new("msg-sel-consumed"), true)); }
                }
                let set_sel = |a: usize, b: usize| ctx.data_mut(|d| d.insert_temp(k_sel, (m.id.clone(), a, b)));
                if resp.drag_started() {
                    if let Some(pp) = ui.input(|i| i.pointer.press_origin()) { let a = char_at(pp); set_sel(a, a); }
                } else if resp.dragged() {
                    if let (Some(pp), Some((sid, a, _))) = (resp.interact_pointer_pos(), ctx.data(|d| d.get_temp::<(String, usize, usize)>(k_sel))) {
                        if sid == m.id { set_sel(a, char_at(pp)); }
                    }
                } else if resp.triple_clicked() {
                    if let Some(pp) = resp.interact_pointer_pos() { let (a, b) = paragraph_range(&g.job.text, char_at(pp)); set_sel(a, b); }
                } else if resp.double_clicked() {
                    if let Some(pp) = resp.interact_pointer_pos() { let (a, b) = word_range(&g.job.text, char_at(pp)); set_sel(a, b); }
                } else if resp.clicked() {
                    // Only web links are opened; a plain click just clears the selection.
                    if let Some(u) = resp.interact_pointer_pos().and_then(|pp| link_at(g, &ct.links, pp - origin)) {
                        if u == html::JUMP { if let Some(q) = &ct.jump { ctx.data_mut(|d| d.insert_temp(Id::new("msg-jump"), (m.id.clone(), q.clone()))); } }
                        else if let Some(r) = u.strip_prefix(html::MENTION) { self.mention_open(&ctx, r); }
                        else { open_web(&u); }
                    }
                    ctx.data_mut(|d| d.remove::<(String, usize, usize)>(k_sel));
                }
                let my_sel = ctx.data(|d| d.get_temp::<(String, usize, usize)>(k_sel)).filter(|s| s.0 == m.id).map(|s| (s.1.min(s.2), s.1.max(s.2)));
                if let Some((a, b)) = my_sel.filter(|(a, b)| a != b) {
                    for r in sel_rects(g, a, b) { p.rect_filled(r.translate(origin.to_vec2()), 0.0, c.select); } // text-highlight
                }
                p.galley(origin, g.clone(), c.body);
                // Block quotes: a bar at the left of their lines (separator-primary).
                for &(qa, qb) in &ct.quotes {
                    let rows = sel_rects(g, qa, qb);
                    if let (Some(f), Some(l)) = (rows.first(), rows.last()) {
                        p.rect_filled(Rect::from_min_max(pos2(origin.x, origin.y + f.top()), pos2(origin.x + 2.4, origin.y + l.bottom())), 1.2, wa(c.body, 0.6)); // light-grey quote bar
                    }
                }
                // Emoji: the colour pictures (or a box) on the placeholders the layout reserved (emoji_text.rs).
                let rects = emoji_text::placeholder_rects(g, &ct.emojis.iter().map(|r| r.char_idx).collect::<Vec<_>>());
                for (run, r) in ct.emojis.iter().zip(rects) {
                    let Some(r) = r else { continue };
                    let rect = Rect::from_center_size(origin + r.center().to_vec2() + Vec2::new(0.0, 0.5), Vec2::splat(run.size));
                    if !run.picture.as_ref().is_some_and(|name| self.icons.emoji(ui, name, rect)) {
                        p.rect_stroke(rect.shrink(2.5), 3.0, Stroke::new(1.0, c.head), StrokeKind::Inside); // never somebody else's glyph
                        if let Some(k) = run.keycap { p.text(rect.center(), Align2::CENTER_CENTER, k, FontId::proportional(BODY - 4.0), c.body); }
                    }
                }
                let selected = my_sel.filter(|(a, b)| a != b).map(|(a, b)| selected_text(&g.job.text, &ct.emojis, &ct.links, a, b));
                if let Some(t) = &selected {
                    // Ctrl+C (egui reports it as Event::Copy) copies the selection as plain text, emoji as characters.
                    if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) && ctx.memory(|mm| mm.focused().is_none()) { ctx.copy_text(t.clone()); }
                }
                let k_link = Id::new(("msg-ctx-link", &m.id));
                if resp.secondary_clicked() {
                    let l = resp.interact_pointer_pos().and_then(|pp| link_at(g, &ct.links, pp - origin)).filter(|l| l != html::JUMP && !l.starts_with(html::MENTION));
                    ctx.data_mut(|d| match l { Some(l) => d.insert_temp(k_link, l), None => d.remove::<String>(k_link) });
                }
                // Right-click menu (shared menu toolkit, ui/menu/): link actions, Kopieren (selection), Alles auswählen, then the
                // message actions.
                let k_menu = format!("msg-ctx-{}", m.id);
                if resp.secondary_clicked() {
                    if let Some(pp) = resp.interact_pointer_pos() { ctx.data_mut(|d| d.insert_temp(Id::new(("msg-ctx-at", &m.id)), pp)); }
                    menu::menu_open(&ctx, &k_menu);
                }
                if menu::menu_is_open(&ctx, &k_menu) {
                    let link = ctx.data(|d| d.get_temp::<String>(k_link));
                    let at = ctx.data(|d| d.get_temp::<egui::Pos2>(Id::new(("msg-ctx-at", &m.id)))).unwrap_or(tr_.min);
                    let mut items: Vec<menu::Entry> = Vec::new();
                    let multi = msel(&ctx, id).multi; // while multi-selecting, the whole menu is the multi-selection entry
                    if multi { items = self.message_entries(&ctx, id, m); } else {
                        if link.is_some() {
                            items.push(menu::Item::new("open-link", "Link öffnen").icon("browser-regular").entry());
                            items.push(menu::Item::new("copy-link", "Link kopieren").icon("link-regular").entry());
                            items.push(menu::Entry::Sep);
                        }
                        if selected.is_some() { items.push(menu::Item::new("copy", "Kopieren").icon("copy-regular").shortcut("Strg+C").entry()); }
                        items.push(menu::Item::new("select-all", "Alles auswählen").icon("select-all-regular").shortcut("Strg+A").entry());
                        items.push(menu::Entry::Sep);
                        items.extend(self.message_entries(&ctx, id, m));
                    }
                    if let Some(a) = menu::webex_menu(&ctx, &mut self.icons, t, &k_menu, Rect::from_min_size(at, Vec2::ZERO), menu::Side::Below, menu::Align::Left, false, &items) {
                        match a.as_str() {
                            "copy" => if let Some(t) = &selected { ctx.copy_text(t.clone()); },
                            "select-all" => ctx.data_mut(|d| d.insert_temp(k_sel, (m.id.clone(), 0usize, total))),
                            _ => self.message_do(&ctx, id, m, &a, link.as_deref()),
                        }
                    }
                }
                y += g.size().y;
            }
            if !m.reactions.is_empty() { self.reactions_row(ui, c, pos2(tx, y + 2.0), m); }
        }
        // A closed (read-only) chat has no reply / react / edit toolbar (ui/closed.rs).
        if hovered && !self.space_closed(id) { self.message_toolbar(ui, t, c, id, m, boxr, head, bar_key, ui.clip_rect().top()); }
    }
}
