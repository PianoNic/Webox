//! The scrolling message list: paging older messages, spinner, scrollbar, screenshot script.
use super::*;

impl App {
    pub(crate) fn message_list(&mut self, ui: &mut egui::Ui, t: &Th, id: &str, msgs: &[Message]) {
        let c = mc(t);
        ui.ctx().data_mut(|d| d.insert_temp(Id::new("msg-seq"), 0u32));
        let real = self.client.is_some();
        let (today, yesterday) = day_ref(real);
        // Signed in, date / time are already local: converted once on arrival (App::poll → prepare_arrived; `Message.utc` keeps the
        // received UTC values).
        let utc_msgs: &[Message] = msgs;
        // Thread structure in one pass.
        let ids: std::collections::HashSet<&str> = msgs.iter().map(|m| m.id.as_str()).collect();
        let roots: Vec<&Message> = msgs.iter().filter(|m| m.parent.is_empty() || !ids.contains(m.parent.as_str())).collect();
        let mut replies_of: std::collections::HashMap<&str, Vec<&Message>> = std::collections::HashMap::new();
        for r in msgs.iter().filter(|r| !r.parent.is_empty()) { replies_of.entry(r.parent.as_str()).or_default().push(r); }
        let force_hover: Option<usize> = if self.shot.is_some() { std::env::var("WEBOX_SHOT_HOVER_MSG").ok().and_then(|v| v.parse().ok()) } else { None };
        let mut counter = 0usize;
        let seen = self.seen_by(id, msgs);
        // People typing join the "Gesehen von" row with the typing overlay (ui/typing.rs).
        let seen = {
            let mut list = seen.unwrap_or_default();
            for p in crate::ui::typing::typers(id) {
                if !list.iter().any(|q| (!p.id.is_empty() && q.id == p.id) || (!p.email.is_empty() && q.email.eq_ignore_ascii_case(&p.email))) { list.push(p); }
            }
            (!list.is_empty()).then_some(list)
        };
        // Nothing of the list may paint outside its own area (tall images, the toolbar's shadow, ...).
        let list = ui.max_rect();
        ui.set_clip_rect(list.intersect(ui.clip_rect()));
        // Scrolling: a chat opens at its newest message and follows new messages and late image heights while you are at the
        // bottom; once you scroll up it stays where you are. Sending a message jumps to the bottom.
        let ctx = ui.ctx().clone();
        self.msgx_tick(&ctx); // flags, unread hold, scheduled sending
        self.unfurl_tick(&ctx, id); // link previews of this space's messages (ui/unfurl.rs)
        self.sample_older_tick(&ctx, id);
        let (loading, reached) = self.history(&ctx, id);
        let space_title = self.spaces.iter().find(|s| s.id == id).map(|s| s.title.clone()).unwrap_or_default();
        let (k_pin, k_space, k_last) = (Id::new(("msg-pinned", id)), Id::new("msg-last-space"), Id::new(("msg-last-id", id)));
        let mut pinned = ctx.data(|d| d.get_temp::<bool>(k_pin)).unwrap_or(true);
        let k_nm = Id::new(("msg-newmark", id));
        let switched = ctx.data(|d| d.get_temp::<String>(k_space)).as_deref() != Some(id);
        if switched {
            // (decided, was unread when opened, last seen time, id of the first newer message): the line stays while the chat is open.
            let (unread, ls) = self.spaces.iter().find(|s| s.id == id).map(|s| (s.unread, s.last_seen.clone())).unwrap_or_default();
            ctx.data_mut(|d| d.insert_temp(k_nm, (false, unread || !real, ls, String::new())));
            pinned = true;
            msel_put(&ctx, id, MultiSel::default()); // a new chat: nothing selected
            new_banners::set_targets(&ctx, id, new_banners::NewTargets::default()); // new-message / new-reply buttons (new_banners.rs)
            new_banners::forget_arrivals(&ctx, id);
            ctx.data_mut(|d| d.insert_temp(k_space, id.to_string()));
        }
        let mut nm = ctx.data(|d| d.get_temp::<(bool, bool, String, String)>(k_nm)).unwrap_or((true, false, String::new(), String::new()));
        if !nm.0 && !utc_msgs.is_empty() {
            nm.0 = true;
            if nm.1 && nm.2.len() >= 16 && nm.2.is_char_boundary(16) {
                let cut = nm.2[..16].to_string(); // "YYYY-MM-DDTHH:MM" (UTC), like the messages' date + time before localisation
                if let Some(f) = utc_msgs.iter().find(|m| m.parent.is_empty() && !m.mine && m.notice.is_empty() && { let (d, t) = utc_date_time(m); format!("{d}T{t}") > cut }) { nm.3 = f.id.clone(); }
                // The same cut gives the targets of "Neue Nachrichten" / "Neue Antworten" (new_banners.rs).
                let replies: Vec<(String, bool)> = utc_msgs.iter().filter(|m| !m.parent.is_empty() && !m.mine && m.notice.is_empty() && { let (d, t) = utc_date_time(m); format!("{d}T{t}") > cut }).map(|m| (m.id.clone(), false)).collect();
                new_banners::set_targets(&ctx, id, new_banners::NewTargets { msg: (!nm.3.is_empty()).then(|| (nm.3.clone(), false)), replies });
            }
            ctx.data_mut(|d| d.insert_temp(k_nm, nm.clone()));
        }
        let new_first = nm.3.clone();
        // Frames this chat has shown content (0 = none yet). The first frame with content is a "settle" frame: the scroll offset isn't
        // known yet, so it is laid out invisibly (otherwise the top of the list would flash for one frame and, with the offset at 0,
        // ask for older messages at once).
        let k_seen = Id::new(("msg-seen-frames", id));
        let seen_frames: u32 = if switched { 0 } else { ctx.data(|d| d.get_temp::<u32>(k_seen)).unwrap_or(0) };
        let settle = !msgs.is_empty() && seen_frames == 0;
        ctx.data_mut(|d| d.insert_temp(k_seen, if msgs.is_empty() { 0 } else { seen_frames.saturating_add(1) }));
        let last = msgs.last().map(|m| m.id.clone()).unwrap_or_default();
        if ctx.data(|d| d.get_temp::<String>(k_last)).as_deref() != Some(last.as_str()) {
            if msgs.last().is_some_and(|m| m.mine) { pinned = true; }
            ctx.data_mut(|d| d.insert_temp(k_last, last));
        }
        // Messages from others that arrive while the list is scrolled up become new-message / new-reply targets.
        let fresh = new_banners::arrived(&ctx, id, msgs);
        if !fresh.is_empty() && !pinned {
            let mut tg = new_banners::targets(&ctx, id);
            for m in msgs.iter().filter(|m| fresh.contains(&m.id) && !m.mine && m.notice.is_empty()) {
                if !m.parent.is_empty() { tg.replies.push((m.id.clone(), false)); }
                else if tg.msg.as_ref().is_none_or(|t| t.1) { tg.msg = Some((m.id.clone(), false)); }
            }
            new_banners::set_targets(&ctx, id, tg);
        }
        if self.shot.is_some() && self.frames == 6 { // screenshot checks only: pre-select text / open the viewer
            if let Ok(v) = std::env::var("WEBOX_SHOT_SELECT") { ctx.data_mut(|d| d.insert_temp(Id::new("msg-sel"), (format!("{id}-{v}"), 4usize, 22usize))); }
            if let Ok(v) = std::env::var("WEBOX_SHOT_MENU") { menu::menu_open(&ctx, &format!("msg-more-{id}-{v}")); } // WEBOX_SHOT_HOVER_MSG must point at the same message
            if std::env::var("WEBOX_SHOT_DELETE").is_ok() { ctx.data_mut(|d| d.insert_temp(Id::new("msg-delete-confirm"), format!("{id}-7"))); }
            if let Ok(v) = std::env::var("WEBOX_SHOT_LIGHTBOX") { ctx.data_mut(|d| d.insert_temp(Id::new("lightbox"), v)); }
            if let Ok(v) = std::env::var("WEBOX_SHOT_GOTO") { ctx.data_mut(|d| d.insert_temp(Id::new("msg-goto"), (format!("{id}-{v}"), 0u8))); } // scroll to message <n>
        }
        let scripted_up = ctx.data_mut(|d| d.remove_temp::<bool>(Id::new("msg-script-up"))).unwrap_or(false);
        // "Scroll to the latest messages" (button / shortcut, jump_button.rs): pinned again = laid out at the newest end.
        if ctx.data_mut(|d| d.remove_temp::<bool>(Id::new(JUMP_KEY))).unwrap_or(false) { pinned = true; ctx.data_mut(|d| d.remove::<f32>(Id::new(("msg-set-offset", id)))); }
        let mut area = crate::ui::widgets::vscroll().id_salt(("msg-scroll", id)).auto_shrink([false; 2]);
        // egui paints with the offset given here, so it must be a real one: the previous frame's maximum.
        let k_max = Id::new(("msg-max", id));
        let k_set = Id::new(("msg-set-offset", id));
        let prev_max = ctx.data(|d| d.get_temp::<f32>(k_max));
        let k_off = Id::new(("msg-off", id));
        let user_set = ctx.data_mut(|d| d.remove_temp::<f32>(k_set)); // from the scrollbar below (drag / track / arrows)
        let explicit_set = user_set.is_some();
        // The wheel over a row overlay (hover toolbar, tooltip, join/leave popover) scrolls the list as if it were on the list.
        let wheel = crate::ui::scroll_route::wheel_passthrough(ui, list);
        let user_set = user_set.or_else(|| (wheel != 0.0).then(|| {
            let base = if pinned { prev_max.unwrap_or(0.0) } else { ctx.data(|d| d.get_temp::<f32>(k_off)).unwrap_or(0.0) };
            base - wheel
        }));
        let applied = if scripted_up { 0.0 } else if let Some(s) = user_set { s.clamp(0.0, prev_max.unwrap_or(0.0)) } else { prev_max.unwrap_or(1.0e5) };
        area = area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden); // our own bar is drawn below
        if scripted_up || user_set.is_some() || pinned { area = area.vertical_scroll_offset(applied); }
        let out = ui.scope(|ui| {
        if settle { ui.set_invisible(); ctx.request_repaint(); }
        area.show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            { let y = ui.cursor().top(); ctx.data_mut(|d| { d.insert_temp(Id::new("msg-top0"), y); d.insert_temp(Id::new("msg-rowpos"), std::collections::HashMap::<String, f32>::new()); d.insert_temp(Id::new("msg-rowspans"), Vec::<(String, f32, f32)>::new()); d.insert_temp(Id::new("msg-copyrows"), Vec::<CopyRow>::new()); d.remove::<String>(Id::new("msg-pointer-row")); d.remove::<String>(Id::new("msg-press-row")); }); }
            // Older messages loading: the loading item (loading_item.rs).
            if loading { self.loading_item(ui, false); } else if reached { self.start_marker(ui, &c, &space_title); }
            ui.add_space(dm().msg_list_top);
            let (mut prev, mut day) = (String::new(), String::new());
            let (mut prev_min, mut prev_share) = (None::<i32>, false);
            let mut prev_ms = 0i64;
            for m in roots.iter() {
                if !m.date.is_empty() && m.date != day {
                    // A day banner only between two rows of different days, never above the first loaded row.
                    let had_previous = !day.is_empty();
                    day = m.date.clone();
                    prev.clear();
                    if had_previous { self.day_line(ui, t, &day_label(&day, &today, &yesterday), c.day); }
                }
                if m.id == new_first { self.day_line(ui, t, "Neue Nachrichten", c.accent); prev.clear(); } // chat-new-message-newMessageBanner-text
                if !m.notice.is_empty() && !m.group.is_empty() { self.group_row(ui, &c, m); prev.clear(); prev_share = false; continue; }
                if !m.notice.is_empty() { self.notice_row(ui, &c, m); prev.clear(); prev_share = false; continue; }
                // Grouping: same sender, within a few minutes, and not right after a share.
                let min = minutes(&m.time);
                let close = match (prev_ms, m.published_ms) { // 60 s window when the times are known
                    (a, b) if a > 0 && b > 0 => grouped_in_time(a, b),
                    _ => matches!((prev_min, min), (Some(a), Some(b)) if (b - a).abs() <= GROUP_MINUTES),
                };
                prev_ms = m.published_ms;
                let first = prev != m.email || !close || prev_share || m.html.contains("<blockquote");
                prev = m.email.clone();
                (prev_min, prev_share) = (min, !m.images.is_empty() && m.text.trim().is_empty() || m.text.contains('📎'));
                let replies: Vec<&Message> = if m.id.is_empty() { msgs.iter().filter(|r| r.parent.is_empty()).collect() }
                    else { replies_of.get(m.id.as_str()).cloned().unwrap_or_default() };
                self.message_row(ui, t, &c, id, m, first, !replies.is_empty(), 0.0, force_hover == Some(counter));
                counter += 1;
                if replies.is_empty() { continue; }
                let open = self.open_threads.contains(&m.id);
                // Settings > Nachrichten "Threads reduzieren": every thread starts collapsed.
                if (replies.len() > THREAD_PREVIEW || self.settings["threads"] == "collapse") && !open {
                    self.collapsed_thread(ui, t, &c, m, &replies);
                    prev.clear();
                    continue;
                }
                let mut last = String::new();
                for r in &replies {
                    let head = last != r.email;
                    last = r.email.clone();
                    self.message_row(ui, t, &c, id, r, head, true, REPLY_LEFT + REPLY_BAR, force_hover == Some(counter));
                    counter += 1;
                }
                self.thread_end(ui, t, &c, m, replies.len() > THREAD_PREVIEW);
                prev.clear();
            }
            if let Some(p) = &seen { self.seen_banner(ui, t, &c, p); } // the "Gesehen von" row: always the last row of the list
            self.sched_list(ui, t, id); // scheduled messages of this space
            ui.add_space(list_bottom()); // "Gesehen von" centre -> composer line = 48 pt
        })
        }).inner;
        self.multi_select_input(&ctx, ui, id, list);
        // A press that landed on no message text clears the text selection.
        if ctx.data_mut(|d| d.remove_temp::<bool>(Id::new("msg-sel-consumed"))).is_none() && ctx.input(|i| i.pointer.primary_pressed()) { ctx.data_mut(|d| d.remove::<(String, usize, usize)>(Id::new("msg-sel"))); }
        // A click on "Zur Originalnachricht springen": find the quoted message above the reply, scroll to it and let it flash.
        if let Some((from, quote)) = ctx.data_mut(|d| d.remove_temp::<(String, String)>(Id::new("msg-jump"))) {
            if let Some(tg) = jump_target(msgs, &from, &quote) {
                let y = ctx.data(|d| d.get_temp::<std::collections::HashMap<String, f32>>(Id::new("msg-rowpos")).and_then(|h| h.get(&tg).copied()));
                if let Some(y) = y { let now = ctx.input(|i| i.time); ctx.data_mut(|d| { d.insert_temp(k_set, (y - 70.0).max(0.0)); d.insert_temp(Id::new("msg-flash"), (tg.clone(), now)); }); ctx.request_repaint(); }
            }
        }
        // "In Unterhaltung anzeigen" (Inhalt tab): scroll to that message and let it flash; the row positions exist once
        // the list has been drawn, so the request waits a few frames.
        if let Some((tg, tries)) = ctx.data(|d| d.get_temp::<(String, u8)>(Id::new("msg-goto"))) {
            let y = ctx.data(|d| d.get_temp::<std::collections::HashMap<String, f32>>(Id::new("msg-rowpos")).and_then(|h| h.get(&tg).copied()));
            match y {
                Some(y) if tries > 0 => { let now = ctx.input(|i| i.time); ctx.data_mut(|d| { d.remove::<(String, u8)>(Id::new("msg-goto")); d.insert_temp(k_set, (y - 70.0).max(0.0)); d.insert_temp(Id::new("msg-flash"), (tg.clone(), now)); }); }
                _ if tries < 30 => { ctx.data_mut(|d| d.insert_temp(Id::new("msg-goto"), (tg.clone(), tries + 1))); }
                _ => ctx.data_mut(|d| d.remove::<(String, u8)>(Id::new("msg-goto"))),
            }
            ctx.request_repaint();
        }
        // "jump to this message" (search results): scroll to the row once it is laid out and let it flash; a few
        // frames of patience for rows that appear only after a thread was opened.
        if let Some((sp, mid, tries)) = ctx.data(|d| d.get_temp::<(String, String, u32)>(Id::new("msg-goto"))) {
            if sp == id {
                let y = ctx.data(|d| d.get_temp::<std::collections::HashMap<String, f32>>(Id::new("msg-rowpos")).and_then(|h| h.get(&mid).copied()));
                match y {
                    Some(y) => { let now = ctx.input(|i| i.time); ctx.data_mut(|d| { d.remove::<(String, String, u32)>(Id::new("msg-goto")); d.insert_temp(k_set, (y - 70.0).max(0.0)); d.insert_temp(Id::new("msg-flash"), (mid.clone(), now)); }); }
                    None if tries < 10 => ctx.data_mut(|d| d.insert_temp(Id::new("msg-goto"), (sp, mid, tries + 1))),
                    None => ctx.data_mut(|d| d.remove::<(String, String, u32)>(Id::new("msg-goto"))),
                }
                ctx.request_repaint();
            }
        }
        let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
        let off = out.state.offset.y;
        // Pinned until the user scrolls away from the bottom (the offset ends up below what we applied); a scrolled-up list
        // re-pins when it reaches the bottom again.
        let next_pinned = if scripted_up { false } else if user_set.is_none() && pinned { off >= applied.min(max) - 0.5 } else { off >= max - 2.0 };
        if self.shot.is_some() && std::env::var("WEBOX_SHOT_TRACE").is_ok() {
            // The first visible row and its on-screen y: a steady scroll moves it smoothly, a jitter makes it jump back and forth.
            let spans = ctx.data(|d| d.get_temp::<Vec<(String, f32, f32)>>(Id::new("msg-rowspans"))).unwrap_or_default();
            let (vid, vy) = pick_anchor(&spans, off).map(|(i, t)| (i, t - off)).unwrap_or_default();
            // A pass that is thrown away (request_discard below) is marked "redo": it is never shown.
            let redo = ctx.data(|d| d.get_temp::<(String, f32)>(Id::new(("msg-anchor", id)))).is_some_and(|(aid, ay)| spans.iter().any(|(i, t, _)| *i == aid && (t - ay).abs() > 0.5)) && !next_pinned || next_pinned && (max - applied).abs() > 0.5 || settle;
            // The offset the content was actually painted with (egui clamps `off` only after painting).
            let painted = out.inner_rect.top() - ctx.data(|d| d.get_temp::<f32>(Id::new("msg-top0"))).unwrap_or(out.inner_rect.top());
            eprintln!("TRACE frame {:>3} off {:>8.1} painted {:>8.1} max {:>8.1} applied {:>8.1} pinned {} next_pinned {} content {:>8.1} view {:>6.1} rows {:>3} loading {} vis {}@{:.1}{}", self.frames, off, painted, max, applied, pinned, next_pinned, out.content_size.y, out.inner_rect.height(), spans.len(), loading, vid, vy, if redo { " (redo, not shown)" } else { "" });
        }
        ctx.data_mut(|d| { d.insert_temp(k_pin, next_pinned); d.insert_temp(k_max, max); d.insert_temp(k_off, off); });
        self.jump_button(ui, list, off, max);
        self.new_banners(ui, list, id, msgs, off, out.inner_rect.height(), max, k_set);
        if let Some(to) = self.scrollbar(ui, &c, list, off, max, out.inner_rect.height(), id) {
            ctx.data_mut(|d| d.insert_temp(k_set, to));
            ctx.request_repaint();
        }
        // Keep what you are looking at where it is (helpers.rs `scroll_fix`): pinned to the bottom while at the bottom, else the
        // first visible row keeps its on-screen y when rows above it change height. A frame laid out with an offset that no
        // longer fits is discarded and laid out again at once, so no frame ever shows the content shifted (the jitter).
        let k_anchor = Id::new(("msg-anchor", id));
        let fix = {
            let spans = ctx.data(|d| d.get_temp::<Vec<(String, f32, f32)>>(Id::new("msg-rowspans"))).unwrap_or_default();
            let prev = ctx.data(|d| d.get_temp::<(String, f32)>(k_anchor));
            let explicit = explicit_set || scripted_up || ctx.data(|d| d.get_temp::<f32>(k_set)).is_some();
            scroll_fix(&spans, prev.as_ref(), off, max, applied, next_pinned, explicit, settle)
        };
        if let Some(to) = fix.set { ctx.data_mut(|d| d.insert_temp(k_set, to)); }
        match &fix.anchor { Some(a) => ctx.data_mut(|d| d.insert_temp(k_anchor, a.clone())), None => ctx.data_mut(|d| d.remove::<(String, f32)>(k_anchor)) }
        if fix.redo { ctx.request_discard("message list re-anchored"); ctx.request_repaint(); }
        // An older page counts as settled after a few frames without a correction; until then the top trigger stays quiet.
        let k_gen = Id::new(("msg-hist-gen", id));
        let generation = self.history_generation(&ctx, id);
        let (seen_gen, quiet) = ctx.data(|d| d.get_temp::<(u32, u32)>(k_gen)).unwrap_or((generation, 3));
        let quiet = if seen_gen != generation || fix.redo { 0 } else { quiet.saturating_add(1) };
        ctx.data_mut(|d| d.insert_temp(k_gen, (generation, quiet)));
        // Within one screen of the top: ask for the page before the oldest loaded message (once, not while loading, not at the start).
        if should_request_older(seen_frames, next_pinned, off, out.inner_rect.height(), max, loading, reached, !msgs.is_empty(), quiet >= 3) { self.request_older(&ctx, id); }
        if next_pinned && prev_max.map_or(true, |p| (p - max).abs() > 0.5) { ctx.request_repaint(); } // late image heights
        self.lightbox(ui.ctx(), msgs);
        self.video_viewer(ui.ctx(), t);
        self.reaction_picker_draw(ui);
        self.reactors_popover(ui, t, &c, msgs);
        self.forward_dialog(ui.ctx(), t, id);
        self.delete_dialog(ui.ctx(), t);
        self.quote_replace_dialog(ui.ctx(), t);
        self.sched_dialogs(ui.ctx(), t);
    }

    /// The start of the conversation, once everything older has been loaded.
    pub(super) fn start_marker(&self, ui: &mut egui::Ui, c: &Mc, title: &str) {
        let w = ui.available_width();
        let g = ui.painter().layout(format!("Anfang der Unterhaltung{}", if title.is_empty() { String::new() } else { format!(" · {title}") }), FontId::proportional(SECOND), c.day, (w - 150.0).max(80.0));
        // 32 pt of vertical padding (not tied to DAY_PAD).
        let (r, _) = ui.allocate_exact_size(Vec2::new(w, g.size().y + 32.0), Sense::hover());
        ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, r.center().y - g.size().y / 2.0), g, c.day);
    }

    /// The message list's scrollbar: a 16 pt track (white@0.11, rounded at the bottom) with a chevron button at each end and a
    /// 7.6 pt pill thumb (white@0.5); shown only when the list scrolls.
    /// Returns a new scroll offset when the user drags the thumb, clicks the track or presses a chevron.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn scrollbar(&self, ui: &mut egui::Ui, c: &Mc, list: Rect, off: f32, max: f32, view_h: f32, id: &str) -> Option<f32> {
        // The shared bar (autoscroll::arrow_bar); here the track runs under the header, so only its bottom is rounded.
        let track = Rect::from_min_max(pos2(list.right() - SB_W, list.top()), list.max);
        // "White ink" when the body colour's red channel is above 128.
        crate::ui::autoscroll::arrow_bar(ui, Id::new(("msg-sb", id)), track, off, max, view_h, c.body.r() > 128, false)
    }

    /// Screenshot-mode test script (never active in a normal run): WEBOX_SHOT_SCRIPT="10:up,40:s2,70:s1" runs, at the given
    /// frames, "up" = scroll the open chat to its top, or a space id = open that space (for checking scroll-to-bottom on switch).
    pub(crate) fn shot_script(&mut self, ctx: &egui::Context) {
        if self.shot.is_none() { return; }
        let Ok(script) = std::env::var("WEBOX_SHOT_SCRIPT") else { return };
        for step in script.split(',') {
            let Some((f, what)) = step.split_once(':') else { continue };
            if f.trim().parse::<u32>().ok() != Some(self.frames as u32) { continue; }
            match what.trim() {
                "up" => ctx.data_mut(|d| d.insert_temp(Id::new("msg-script-up"), true)),
                // Repro of "the messages arrive after the chat is open": stash the open chat's messages, put them back later.
                "stash" => if let Some(sp) = self.selected.clone() { let v = self.messages.remove(&sp).unwrap_or_default(); ctx.data_mut(|d| d.insert_temp(Id::new("msg-stash"), v)); },
                "unstash" => if let (Some(sp), Some(v)) = (self.selected.clone(), ctx.data(|d| d.get_temp::<Vec<Message>>(Id::new("msg-stash")))) { self.messages.insert(sp, v); },
                "images" => self.photos.add_images(ui::sample_photos::sample_images()), // late decode of the sample pictures (repro of height changes)
                // A message and a thread reply from someone else arrive (repro of the new-message / new-reply buttons).
                "incoming" => if let Some(sp) = self.selected.clone() {
                    let list = self.messages.entry(sp.clone()).or_default();
                    let root = list.iter().find(|m| m.parent.is_empty() && m.notice.is_empty()).map(|m| m.id.clone()).unwrap_or_default();
                    let mk = |id: String, text: &str, parent: String| Message { id, author: "Mara Lind".into(), email: "mara@example.com".into(), text: text.into(), time: "10:05".into(), date: system::today(), parent, ..Default::default() };
                    let n = list.len();
                    list.push(mk(format!("{sp}-in{n}"), "Neue Nachricht während du oben warst", String::new()));
                    if !root.is_empty() { let at = list.iter().position(|m| m.id == root).map_or(list.len(), |i| i + 1); list.insert(at, mk(format!("{sp}-in{n}r"), "Eine Antwort im Thread", root)); }
                },
                s if s.starts_with("scroll") => { // "scroll600": scroll the open chat to that offset (screenshot checks)
                    if let (Some(sp), Ok(v)) = (self.selected.clone(), s[6..].parse::<f32>()) { ctx.data_mut(|d| d.insert_temp(Id::new(("msg-set-offset", sp.as_str())), v)); }
                }
                s => self.open_space(s.to_string()),
            }
        }
    }
}
