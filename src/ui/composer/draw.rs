//! The composer itself: drawing the input and its popups and handling keys. One long function, see docs/refactor-plan.md;
//! its first-frame restore (restore.rs), toolbar row and send pill (bar.rs) are methods of their own.
use super::*;

impl App {
    pub(crate) fn composer(&mut self, ui: &mut egui::Ui, t: &Th, space: &Space, now: f64) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let id = space.id.clone();
        let cid = egui::Id::new(CID);
        let light = super::super::menu::light(t);
        let mut st = st_get(&ctx);
        super::super::type_focus::composer_drawn(&ctx, &space.id);

        // First frame / space switch: restore drafts, markdown mode; screenshot presets (WEBOX_SHOT_COMPOSER).
        let shot = if self.shot.is_some() { std::env::var("WEBOX_SHOT_COMPOSER").ok() } else { None };
        self.composer_restore(&ctx, &mut st, &id, cid, &shot, now);
        let edit = matches!(self.compose, Compose::Edit { .. });
        // Finished dialogs / captures, the file shortcuts (Ctrl+O, screen capture), download announcements.
        self.files_frame(&ctx, now);
        self.screen_capture_frame(&ctx, now);
        let picked = crate::app::files::take_picks(&id);
        if !picked.is_empty() { self.stage_paths(&mut st.chips, picked, edit, now); }
        if let Some(png) = super::super::screen_capture::take_capture(&id) { self.stage_bytes(&mut st.chips, &crate::app::files::capture_name(), &png, edit, now); }
        if let Some(png) = super::super::attach::take_pasted(&id) { self.stage_bytes(&mut st.chips, &crate::app::files::pasted_image_name(), &png, edit, now); }
        let file_cmds = |e: &&str| matches!(*e, "AttachFile" | "ScreenCapture" | "ScreenCaptureNoMini");
        let cmds: Vec<&'static str> = self.sc.composer_events.iter().copied().filter(file_cmds).collect();
        self.sc.composer_events.retain(|e| !file_cmds(e));
        for c in cmds { match c { "AttachFile" => self.attach_pick(&id, now), "ScreenCapture" => self.screen_capture_start(&id, true, now), _ => self.screen_capture_start(&id, false, now) } }
        let rich = st.rich;
        st.chips_w = full.width() - 2.0 * super::super::attach::AREA_SIDE; // the staging area's inner width
        let chips_h = chips_height(&st);
        let (top, text_h, total) = metrics(edit, rich, chips_h, st.rows);
        let crect = Rect::from_min_max(pos2(full.left(), full.bottom() - total), full.max);
        ui.painter().rect_filled(crect, 0.0, t.composer);

        // While editing, the title "Nachricht bearbeiten" sits above the composer in the edit overlay (ui/edit_overlay.rs).
        let off = if edit { EDIT_HDR } else { 0.0 };

        // Toolbar (action bar) and, while editing, Cancel / Update (bar.rs).
        let (emoji_anchor, gif_anchor, update_hint) = self.composer_toolbar(ui, t, crect, off, &mut st, &id, &ctx, light, edit, now);

        // Reply banner.
        let draft_now = self.drafts.get(&id).cloned().unwrap_or_default();
        let hints: Vec<String> = if draft_now.starts_with('/') {
            let typed = draft_now.trim_start_matches('/').split(' ').next().unwrap_or("").to_string();
            self.plugins.commands().into_iter().filter(|(c, _)| c.starts_with(&typed)).map(|(c, p)| format!("/{c} · {p}")).collect()
        } else { vec![] };
        if let Compose::Reply { to, .. } = &self.compose {
            let br = Rect::from_min_size(pos2(crect.left() + 19.0, crect.top() + off + crate::ui::theme::density::m().composer_reply_y), Vec2::new(crect.width() - 38.0, 18.0)); // Standard 40
            let mut bc = ui.new_child(egui::UiBuilder::new().max_rect(br).layout(egui::Layout::left_to_right(egui::Align::Center)));
            bc.label(RichText::new(format!("Antwort im Thread an {to}")).size(12.5).color(t.link));
            if bc.small_button("Abbrechen (Esc)").clicked() { self.compose = Compose::New; }
        }

        // Rich-text toolbar row: [B I U S] [H1 H2 H3] link more | markdown.
        let mut y = crect.top() + off + base_top();
        if rich {
            let row = Rect::from_min_size(pos2(crect.left() + 25.0, y), Vec2::new(crect.width() - 50.0, 28.0));
            self.rich_toolbar(ui, t, row, &mut st, &id);
            y += 30.0;
        }
        // Staged files (side margins 16, 68 .. 92 high, the bricklets flow and scroll inside).
        if !st.chips.is_empty() {
            let chips = st.chips.clone();
            let side = super::super::attach::AREA_SIDE;
            let area = Rect::from_min_max(pos2(crect.left() + side, y), pos2(crect.right() - side, y + chips_h));
            if let Some(i) = self.draw_chips(ui, t, area, &chips) { st.chips.remove(i); }
            y += chips_h;
        }

        // Text box + send pill.
        let (pr_c, mr_top) = (pos2(crect.right() - 19.0 - 30.0, y + 12.0), y);
        let pr = Rect::from_center_size(pr_c, Vec2::new(60.0, 26.0));
        let mr = Rect::from_min_max(pos2(crect.left() + 20.5, mr_top), pos2(pr.left() - 12.0, mr_top + text_h));
        debug_assert!(top >= 0.0);

        // Keys that must be consumed BEFORE the TextEdit sees them (it would type a newline for Enter, move the caret for the arrows).
        let focused_prev = ctx.memory(|m| m.has_focus(cid));
        let emoji_items: Vec<&'static eac::Item> = match (&st.emoji_q, self.emoji_autocomplete_on()) { (Some((_, q)), true) => eac::search(q), _ => vec![] };
        let cands: Vec<mp::Cand> = match &st.ment_q { Some((_, q)) => self.mention_cands(space, q), None => vec![] };
        let (em_open, mn_open) = (!emoji_items.is_empty(), em_none(&emoji_items) && !cands.is_empty());
        let (mut accept_emoji, mut accept_ment, mut want_send, mut want_newline, mut want_esc, mut act): (bool, bool, bool, bool, bool, Option<KeyAct>) = (false, false, false, false, false, None);
        let mut space_typed = false;
        let mut cluster_key: Option<(Key, i32)> = None;
        let mut paste_too_long = false;
        if focused_prev {
            let n_e = emoji_items.len(); let n_m = cands.len();
            let (mut es, mut ms) = (st.emoji_sel, st.ment_sel);
            ctx.input_mut(|i| {
                if em_open {
                    if i.consume_key(Modifiers::NONE, Key::ArrowDown) { es = (es + 1).min(n_e - 1); }
                    if i.consume_key(Modifiers::NONE, Key::ArrowUp) { es = es.saturating_sub(1); }
                    // Tab accepts like Enter (as the mention list; see the lock_focus note at the TextEdit).
                    if i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab) { accept_emoji = true; }
                } else if mn_open {
                    if i.consume_key(Modifiers::NONE, Key::ArrowDown) { ms = (ms + 1).min(n_m - 1); }
                    if i.consume_key(Modifiers::NONE, Key::ArrowUp) { ms = ms.saturating_sub(1); }
                    if i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab) { accept_ment = true; }
                } else {
                    // Shift+Enter first: consume_key ignores an extra Shift, so a plain-Enter check ahead of it took Shift+Enter
                    // too and sent the message instead of breaking the line.
                    if i.consume_key(Modifiers::SHIFT, Key::Enter) { want_newline = true; }
                    else if i.consume_key(Modifiers::NONE, Key::Enter) { want_send = true; }
                }
                if i.consume_key(Modifiers::NONE, Key::Escape) { want_esc = true; }
                // Emoji clusters (ZWJ sequences, flags, skin tones, variation selectors) are one unit for the caret keys.
                for (k, dir) in [(Key::Backspace, -1i32), (Key::Delete, 1), (Key::ArrowLeft, -2), (Key::ArrowRight, 2)] {
                    if i.key_pressed(k) && i.modifiers.is_none() { cluster_key = Some((k, dir)); }
                }
                act = local_format_keys(i);
                space_typed = i.events.iter().any(|e| matches!(e, egui::Event::Text(s) if s == " "));
                // The clipboard text is plain text; more than the message limit is refused with the pack's message.
                let too_long = |e: &egui::Event| matches!(e, egui::Event::Paste(s) if s.chars().count() > ct::MAX_CHARS);
                if i.events.iter().any(too_long) { i.events.retain(|e| !too_long(e)); paste_too_long = true; }
            });
            st.emoji_sel = es; st.ment_sel = ms;
        }
        if let (true, Some((k, dir))) = (focused_prev, cluster_key) {
            let d = self.drafts.entry(id.clone()).or_default();
            let (sa, sp) = get_sel(&ctx, d.chars().count());
            if sa == sp {
                let cl = super::super::emoji_text::clusters(d);
                let rng: Vec<(usize, usize)> = cl.iter().map(|(a, b)| (ct::char_of(d, *a), ct::char_of(d, *b))).collect();
                let hit = if dir < 0 { rng.iter().find(|(a, b)| sp > *a && sp <= *b && *b - *a > 1) } else { rng.iter().find(|(a, b)| sp >= *a && sp < *b && *b - *a > 1) };
                if let Some(&(a, b)) = hit {
                    ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
                    match dir {
                        -1 => { let (x, y) = (ct::byte_of(d, a), ct::byte_of(d, b)); d.replace_range(x..y, ""); set_sel(&ctx, a, a); }
                        1 => { let (x, y) = (ct::byte_of(d, a), ct::byte_of(d, b)); d.replace_range(x..y, ""); set_sel(&ctx, a, a); }
                        -2 => set_sel(&ctx, a, a),
                        _ => set_sel(&ctx, b, b),
                    }
                    ctx.request_repaint();
                }
            }
        }
        // Ctrl+V with an image (and no text) on the clipboard stages it as a PNG named "<guid>.PNG" (see files::pasted_image_name).
        if focused_prev && super::super::attach::paste_key_edge(&ctx) && !ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))) {
            super::super::attach::clipboard_png_async(&ctx, &id); // staged when it arrives (above)
        }
        if paste_too_long { self.toast("Der Inhalt der Zwischenablage überschreitet die maximale Anzahl an Zeichen für eine einzelne Nachricht.", now); }
        // Esc priority: emoji / mention popups first, then the GIF picker (it closes itself), then edit / reply.
        let gif_open = super::super::gif_picker::gif_picker_open();
        if want_esc && !self.escape_for_popup(&mut st, em_open, mn_open) && !gif_open { self.composer_escape(&mut st, &id); }
        if want_esc && st.legend { st.legend = false; }

        // Newline (Shift+Enter): emoticon of the last word first, list continuation in markdown mode.
        if want_newline {
            let em = self.emoticons_on();
            let d = self.drafts.entry(id.clone()).or_default();
            let (_, cur) = get_sel(&ctx, d.chars().count());
            let mut cur2 = cur;
            if em { if let Some((nt, ne)) = ct::replace_emoticon_word_ending_at(d, cur) { *d = nt; cur2 = ne; } }
            let (nt, nc) = ct::newline(d, cur2, st.md);
            *d = nt;
            set_sel(&ctx, nc, nc);
        }
        match act {
            Some(KeyAct::Fmt(f)) => self.apply_format(&ctx, &id, f),
            Some(KeyAct::ToggleMarkdown) => self.toggle_markdown(&mut st, now),
            Some(KeyAct::Gif) => self.gif_picker_toggle(&ctx), // Ctrl+G
            None => {}
        }

        // The edit box.
        let ph = if space.id == super::super::new_message::DRAFT_ID { crate::trq!("Write a message").into_owned() }
            else if edit { crate::trq!("Edit message to {{0}}", &space.title) } // QObject "Edit message to {{0}}"
            else { format!("Nachricht an {} schreiben", space.title) };
        let labels: Vec<String> = ct::live_mentions(&draft_now, &st.mentions).iter().map(|m| m.label()).collect();
        let mcol = mention_colour(t);
        let txt_col = if light { hex(0x121212) } else { t.text1 };
        // Emoji in the box: the characters stay in the text (cursor / selection / undo / copy keep working on chars), but a cluster with a
        // colour picture is laid out in the system emoji font with a transparent colour (so it reserves the right width) and the picture is
        // painted over it after the TextEdit (see `cluster_rects`). Clusters without a picture are drawn by the system emoji font in the text colour.
        let sys = super::super::emoji_text::ensure_font(&ctx);
        let mut layouter = |ui: &egui::Ui, text: &str, wrap: f32| {
            let mut job = egui::text::LayoutJob::default();
            job.wrap.max_width = wrap;
            let font = FontId::proportional(16.0);
            let efont = if sys { FontId::new(16.0, egui::FontFamily::Name(super::super::emoji_text::FAMILY.into())) } else { font.clone() };
            // (start byte, end byte, kind) kind 0 = mention, 1 = emoji with picture, 2 = emoji drawn by the font
            let mut spans: Vec<(usize, usize, u8)> = Vec::new();
            for l in &labels { if let Some(p) = text.find(l.as_str()) { spans.push((p, p + l.len(), 0)); } }
            for (a, b) in super::super::emoji_text::clusters(text) { spans.push((a, b, if super::super::emoji_text::resolve(&text[a..b]).is_some() { 1 } else { 2 })); }
            spans.sort();
            let mut at = 0;
            let mut gap = 0.0_f32; // room still missing behind the last emoji picture (the font's own advance can be narrower than the 20 px picture)
            for (a, b, k) in spans {
                if a < at { continue; }
                job.append(&text[at..a], gap, egui::TextFormat::simple(font.clone(), txt_col));
                gap = 0.0;
                let fmt = match k { 0 => egui::TextFormat::simple(font.clone(), mcol), 1 => egui::TextFormat::simple(efont.clone(), Color32::TRANSPARENT), _ => egui::TextFormat::simple(efont.clone(), txt_col) };
                if k == 1 {
                    // egui does not shape: every code point of a sequence (tone, ZWJ parts, flag letters) would add its own advance. The first one keeps the
                    // font size (it reserves the picture's room), the others are laid out at 0.5 px so the sequence is as wide as one emoji.
                    let first = text[a..b].chars().next().map_or(0, |c| c.len_utf8());
                    job.append(&text[a..a + first], 0.0, fmt.clone());
                    if a + first < b { job.append(&text[a + first..b], 0.0, egui::TextFormat::simple(FontId::new(0.5, efont.family.clone()), Color32::TRANSPARENT)); }
                    let w = ui.fonts(|f| f.layout_no_wrap(text[a..a + first].to_string(), efont.clone(), Color32::TRANSPARENT).size().x);
                    gap = (PIC_W - w).max(0.0);
                } else {
                    job.append(&text[a..b], 0.0, fmt);
                }
                at = b;
            }
            job.append(&text[at..], gap, egui::TextFormat::simple(font, txt_col));
            ui.fonts(|f| f.layout_job(job))
        };
        let draft = self.drafts.entry(id.clone()).or_default();
        let mut e = ui.new_child(egui::UiBuilder::new().max_rect(mr).layout(egui::Layout::left_to_right(egui::Align::Min)));
        // While the mention list is open the field keeps focus on Tab, so Tab picks the person like Enter; otherwise Tab moves focus
        // as usual. The same for the emoji suggestions: Tab picks the current emoji like Enter.
        // (lock_focus = the Tab focus filter; the Tab key itself is consumed above, so no tab character is typed.)
        let out = egui::TextEdit::multiline(draft).id(cid).hint_text(RichText::new(ph).color(t.text2).size(16.0)).layouter(&mut layouter)
            .lock_focus(mn_open || em_open).desired_rows(1).desired_width(mr.width()).frame(false).margin(Vec2::ZERO).font(FontId::proportional(16.0)).show(&mut e);
        let mut focused = out.response.has_focus();
        // A press on one of the composer's own popups (emoji suggestions, mention list: rows or their scroll bar) must not take the
        // focus from the field: egui surrenders a widget's focus on any press outside it, and without focus the queries below are
        // cleared before the popup is drawn, so the row never sees its release and the click is lost.
        if focused_prev && !focused && (em_open || mn_open) && popup_pressed(&ctx) {
            ctx.memory_mut(|m| m.request_focus(cid));
            focused = true;
        }
        {
            // Colour pictures over the transparent placeholders.
            let d = draft.clone();
            let cl: Vec<(usize, usize)> = super::super::emoji_text::clusters(&d).into_iter().filter(|(a, b)| super::super::emoji_text::resolve(&d[*a..*b]).is_some()).collect();
            let ranges: Vec<(usize, usize)> = cl.iter().map(|(a, b)| (ct::char_of(&d, *a), ct::char_of(&d, *b))).collect();
            let rects = cluster_rects(&out.galley, &ranges);
            let clip = out.text_clip_rect;
            for ((a, b), r) in cl.iter().zip(rects) {
                let Some(r) = r else { continue };
                let Some(name) = super::super::emoji_text::resolve(&d[*a..*b]) else { continue };
                let side = 20.0;
                let c = pos2(out.galley_pos.x + r.left() + side / 2.0 + 1.0, out.galley_pos.y + r.center().y);
                let pr = Rect::from_center_size(c, Vec2::splat(side));
                if clip.intersects(pr) { self.icons.emoji(&mut ui.new_child(egui::UiBuilder::new().max_rect(clip)), &name, pr); }
            }
        }
        st.rows = out.galley.rows.len();
        let caret = out.state.cursor.char_range().map(|r| r.primary);
        let caret_rect = caret.map(|cc| { let r = out.galley.pos_from_ccursor(cc); Rect::from_min_size(out.galley_pos + r.min.to_vec2(), r.size()) }).unwrap_or(mr);
        let len = draft.chars().count();
        let cur = caret.map_or(len, |c| c.index.min(len));
        // Space released: the emoticon in front of it becomes an emoji.
        if space_typed && self.emoticons_on() {
            let d = self.drafts.entry(id.clone()).or_default();
            if let Some((nt, nc)) = ct::replace_emoticon_before_space(d, cur) { *d = nt; set_sel(&ctx, nc, nc); ctx.request_repaint(); }
        }
        if focused_prev && !focused && st.was_focused { self.save_drafts(); }
        st.was_focused = focused;

        // Accepting keys / actions.
        let draft_len = self.drafts.get(&id).map_or(0, |d| d.chars().count());
        let cur = cur.min(draft_len);
        let mut accept_emoji_at: Option<usize> = accept_emoji.then_some(st.emoji_sel);
        let mut accept_ment_at: Option<usize> = accept_ment.then_some(st.ment_sel);

        // Triggers (after this frame's typing); screenshot presets keep their popup open without focus, a scripted click runs the
        // real focus path.
        let shot_keep = shot.is_some() && std::env::var("WEBOX_SHOT_CLICK").is_err() && std::env::var("WEBOX_SHOT_KEY").is_err();
        if focused || shot_keep {
            let d = self.drafts.get(&id).cloned().unwrap_or_default();
            let eq = if self.emoji_autocomplete_on() { ct::emoji_query(&d, cur, eac::MIN_CHARS) } else { None };
            // A cancelled trigger (Esc, or a mention just accepted) stays closed while its query exists; the cancel is dropped
            // only once there is no query at all (a filtered-out query must not drop it, or the popup reopens a frame later).
            if eq.is_none() { st.emoji_cancel = None; }
            let eq = eq.filter(|(a, _)| st.emoji_cancel != Some(*a));
            if eq.as_ref().map(|q| &q.1) != st.emoji_q.as_ref().map(|q| &q.1) { st.emoji_sel = 0; }
            st.emoji_q = eq;
            let mq = ct::mention_query(&d, cur);
            if mq.is_none() { st.ment_cancel = None; }
            let mq = mq.filter(|(a, _)| st.ment_cancel != Some(*a));
            if mq.as_ref().map(|q| &q.1) != st.ment_q.as_ref().map(|q| &q.1) { st.ment_sel = 0; }
            if mq.is_none() { st.ment_dir = None; }
            st.ment_q = mq;
        } else if !ctx.memory(|m| m.focused().is_some()) || !em_open && !mn_open {
            st.emoji_q = None; st.ment_q = None; st.ment_dir = None;
        }

        // Emoji popup.
        if let Some((at, q)) = st.emoji_q.clone() {
            let items = eac::search(&q);
            if !items.is_empty() && self.emoji_autocomplete_on() {
                st.emoji_sel = st.emoji_sel.min(items.len() - 1);
                let pos = eac::popup_pos(caret_rect.left_top(), ctx.screen_rect());
                let moved = ctx.input(|i| i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowUp));
                if let Some(i) = eac::popup(&ctx, &mut self.icons, t, pos, &items, st.emoji_sel, moved) { accept_emoji_at = Some(i); }
                if let Some(i) = accept_emoji_at.filter(|i| *i < items.len()) {
                    let ch = items[i].ch.clone();
                    let d = self.drafts.entry(id.clone()).or_default();
                    let (a, b) = (ct::byte_of(d, at), ct::byte_of(d, cur));
                    d.replace_range(a..b, &ch);
                    let nc = at + ch.chars().count();
                    set_sel(&ctx, nc, nc);
                    ctx.memory_mut(|m| m.request_focus(cid));
                    self.record_recent_emoji(&ch);
                    st.emoji_q = None; ctx.request_repaint();
                }
            }
        }
        // Mention popup.
        if let Some((at, q)) = st.ment_q.clone() {
            let cands = self.mention_cands(space, &q);
            if !cands.is_empty() {
                st.ment_sel = st.ment_sel.min(cands.len() - 1);
                let (np, ns) = (cands.iter().filter(|c| c.section == 0).count(), cands.iter().filter(|c| c.section == 1).count());
                let pl = mp::place(mr, caret_rect, ctx.screen_rect().height(), np, ns, false, st.ment_dir);
                if st.ment_dir.is_none() { st.ment_dir = Some(pl.opens_below); } // lock the direction
                let moved = ctx.input(|i| i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowUp));
                if let Some(i) = mp::popup(self, &ctx, t, pl, &cands, st.ment_sel, moved) { accept_ment_at = Some(i); }
                if let Some(i) = accept_ment_at.filter(|i| *i < cands.len()) {
                    let cd = &cands[i];
                    let m = Mention { id: cd.id.clone(), name: cd.name.clone(), all: cd.all };
                    let d = self.drafts.entry(id.clone()).or_default();
                    let (nt, nc) = ct::insert_mention(d, at, cur, &m);
                    *d = nt;
                    st.mentions.retain(|x| x.name != m.name);
                    st.mentions.push(m);
                    set_sel(&ctx, nc, nc);
                    ctx.memory_mut(|m| m.request_focus(cid));
                    // The inserted "@Name" would match its own query again and reopen the list: this '@' is done.
                    st.ment_cancel = Some(at);
                    st.ment_q = None; st.ment_dir = None; ctx.request_repaint();
                }
            }
        }

        // Emoji picker (toolbar button): a pick is inserted at the caret.
        let mut picked: Option<String> = None;
        if !super::super::messages::react_picker_active(&ctx) && !super::super::profile_menu::status_emoji_active(&ctx) { self.emoji_picker(ui, emoji_anchor, &mut |em| picked = Some(em.to_string())); }
        if picked.is_none() { picked = self.gif_take_emoji(); } // an emoji chosen in the GIF popup's Emojis tab
        if let Some(em) = picked {
            let d = self.drafts.entry(id.clone()).or_default();
            let (_, p) = get_sel(&ctx, d.chars().count());
            let b = ct::byte_of(d, p);
            d.insert_str(b, &em);
            let np = p + em.chars().count();
            set_sel(&ctx, np, np);
            ctx.memory_mut(|m| m.request_focus(cid));
        }

        // GIF picker above the GIF button + the staged-GIF chip above the composer line (floats, does not grow the box).
        self.gif_picker(ui, t, gif_anchor, &id);
        self.gif_stage_chip(ui, t, crect, &id);
        self.upload_strip(ui, t, crect, &id, now); // uploads of this space: progress, cancel, retry
        self.quote_banner(ui, t, crect, &id); // "Nachricht zitieren" waiting to be sent

        // Send pill (bar.rs).
        let now_send = self.composer_send_buttons(ui, t, pr, light, &mut st, &id, &ctx, edit, now);
        if !hints.is_empty() {
            egui::Area::new(egui::Id::new("hints")).fixed_pos(crect.left_top() + Vec2::new(18.0, -30.0)).show(&ctx, |ui| {
                egui::Frame::default().fill(t.card).corner_radius(6.0).inner_margin(6).show(ui, |ui| ui.label(RichText::new(hints.join("     ")).size(12.5).color(t.text2)));
            });
        }
        // Composer line #07C1F5; an accent-theme plugin may make it a gradient (ui/theme/accents.rs).
        crate::ui::theme::accents::hline(ui.painter(), crect.x_range(), crect.top() + 0.5, 1.0, t.accent);

        // Dropped files become chips; while files hover the composer is outlined.
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering { ui.painter().rect_stroke(full.shrink(2.0), 10.0, Stroke::new(2.0_f32, t.accent), egui::StrokeKind::Inside); }
        // Dropped files are staged (in edit mode they are refused with the pack's message).
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        let mut paths = vec![];
        for f in dropped {
            match (&f.path, &f.bytes) {
                (Some(p), _) => paths.push(p.clone()),
                (None, Some(b)) if !f.name.is_empty() => self.stage_bytes(&mut st.chips, &f.name, b, edit, now),
                _ => {}
            }
        }
        self.stage_paths(&mut st.chips, paths, edit, now);

        // Markdown legend popup + the @all confirmation.
        if st.legend { self.markdown_legend(&ctx, t, &mut st, now); }
        if st.confirm_all {
            let n = space.participants.len().max(1);
            let body = format!("Wenn Sie eine Nachricht an @all senden, werden {n} Personen im Bereich @erwähnt."); // pack string
            match super::super::menu::webex_dialog(&ctx, t, "composer-all", "Alle erwähnen", &body, None, &[super::super::menu::Button::secondary("cancel", "Abbrechen"), super::super::menu::Button::primary("send", "Senden")]).as_deref() {
                Some("send") => { st.confirm_all = false; self.composer_send(&mut st, space, true, now); }
                Some(_) => st.confirm_all = false,
                None => {}
            }
        }

        // Sending (never in the frame that typing moved the focus here, ui/type_focus.rs).
        if want_send && super::super::type_focus::redirected_now(&ctx) { want_send = false; }
        if (want_send || now_send || update_hint) && !st.confirm_all { self.composer_send(&mut st, space, false, now); }
        // Shift+Up in an empty box edits the last own message (EditLastMessage).
        if focused && self.drafts.get(&id).is_none_or(|d| d.is_empty()) && ctx.input(|i| i.modifiers.shift && i.key_pressed(Key::ArrowUp)) {
            if let Some(m) = self.messages.get(&id).and_then(|l| l.iter().rev().find(|m| m.mine && m.notice.is_empty())).cloned() { self.start_edit(&m); }
        }
        let _ = (accept_emoji_at, accept_ment_at);
        // Esc / Tab with a popup open must stay in the field: the filter egui checks at the start of the NEXT frame, so it is set
        // from the popup state that frame will see (after this frame's typing / accepting / cancelling).
        let em_next = self.emoji_autocomplete_on() && st.emoji_q.as_ref().is_some_and(|(_, q)| !eac::search(q).is_empty());
        let mn_next = st.ment_q.as_ref().is_some_and(|(_, q)| !self.mention_cands(space, q).is_empty());
        ctx.memory_mut(|m| m.set_focus_lock_filter(cid, focus_filter(em_next || mn_next)));
        st_put(&ctx, st);
    }
}
