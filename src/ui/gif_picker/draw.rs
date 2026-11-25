//! Drawing the standard GIF picker (or handing the frame to the GIF-Extras plugin's popup while it is on).
use super::*;

impl App {
    /// Draws the popup above `anchor` while it is open. A picked GIF is staged for `space`.
    pub(crate) fn gif_picker(&mut self, ui: &mut egui::Ui, t: &Th, anchor: Rect, space: &str) {
        let ctx = ui.ctx().clone();
        let mut st = take();
        let now = ctx.input(|i| i.time);
        // Screenshot runs only: WEBOX_SHOT_GIF=1 (trending) | search | stage | empty | error (the plugin reads its own modes).
        if !st.shot_done && self.shot.is_some() && self.client.is_none() && self.frames >= 45 { // after WEBOX_SHOT_OPEN (frame 30) switched the space
            if let Ok(mode) = std::env::var("WEBOX_SHOT_GIF") {
                st.shot_done = true;
                put(st);
                self.gif_picker_toggle(&ctx);
                st = take();
                if !st.ext {
                    match mode.as_str() {
                        "stage" => { let g = st.sample_all[2].clone(); st.staged.insert(space.to_string(), g); st.open = false; }
                        "empty" => { self.gif_show(&mut st, "xyzzy"); st.focus = false; }
                        "error" => { st.query = "katze".into(); st.gifs.clear(); st.error = Some(FetchError::Network); st.focus = false; }
                        "search" => { self.gif_show(&mut st, "wow"); st.focus = false; }
                        _ => { st.focus = false; }
                    }
                }
            }
        }
        if !st.open { put(st); return }

        // The plugin's popup: the switch is read every frame, so turning the plugin off (or on) closes the open popup.
        let ext_on = self.gif_extras_on();
        if st.ext != ext_on { st.open = false; st.ext = false; put(st); return }
        if st.ext {
            put(st);
            let out = self.gif_ext_popup(ui, t, popup_rect(ctx.screen_rect(), anchor), anchor, space);
            let mut st = take();
            match out {
                None => { st.open = false; st.ext = false; }
                Some(o) => {
                    if let Some((g, bytes)) = o.pick {
                        if let Some(b) = bytes { st.thumbs.insert(&g.id, b); }
                        st.staged.insert(space.to_string(), g);
                    }
                    if let Some(e) = o.emoji { st.pending_emoji = Some(e); ctx.request_repaint(); }
                    if o.close { st.open = false; st.ext = false; }
                }
            }
            let picked = st.staged.contains_key(space);
            let refocus = !st.open && picked;
            put(st);
            if picked && self.settings["gif_send_instantly"].as_bool().unwrap_or(false) { self.gif_send_staged(space, now); }
            else if refocus { ctx.memory_mut(|m| m.request_focus(egui::Id::new("composer-edit"))); }
            return;
        }

        let dark = t.text1 == hex(0xF7F7F7);
        let pal = pal(dark);

        // Answers of the worker threads.
        let mut events = vec![];
        if let Some(rx) = &st.rx { while let Ok(e) = rx.try_recv() { events.push(e) } }
        for e in events {
            match e {
                Event::Page { ticket, res } if ticket == st.ticket => {
                    st.loading = false;
                    match res {
                        Ok(p) => {
                            if p.offset == 0 { st.gifs.clear(); }
                            for g in p.gifs { if !st.gifs.iter().any(|x| x.id == g.id) { st.gifs.push(g) } }
                            st.total = p.total;
                            st.next_offset = if p.count == 0 { p.total } else { p.offset + p.count };
                            if st.kind == Some(Kind::Trending) { st.trending = Some((st.gifs.clone(), st.next_offset, st.total)); }
                        }
                        Err(e) => { if st.gifs.is_empty() { st.error = Some(e) } else { st.next_offset = st.total } }
                    }
                }
                Event::Thumb { id, bytes } => st.thumbs.arrived(id, bytes),
                _ => {}
            }
        }
        st.thumbs.budget();

        // Search-as-you-type (300 ms): any text = search results, an emptied field = back to trending.
        if let Some(q) = st.debounce.due(now) { let q = q.trim().to_string(); self.gif_show(&mut st, &q); }
        if let Some(w) = st.debounce.wait(now) { ctx.request_repaint_after(std::time::Duration::from_secs_f32(w as f32 + 0.01)); }
        if st.loading || st.thumbs.busy() { ctx.request_repaint_after(std::time::Duration::from_millis(60)); }

        // Popup placement (centred over the GIF button, caret down at it) and the content inside its 1 px border.
        let pl = place(ctx.screen_rect(), anchor);
        let rect = pl.content;
        let lay = layout(rect);
        let colw = grid_col_width(lay.grid.width());
        let (tiles, total_h) = masonry(&st.gifs, colw, COLS);
        let n = st.gifs.len();
        let mut pick: Option<usize> = None;
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        if n > 0 {
            for key in [Key::ArrowDown, Key::ArrowUp] {
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) { st.sel = grid_step(&tiles, st.sel, key); st.scroll_to_sel = true; }
            }
            // Left / right move between GIFs once one is selected (before that they belong to the search field's caret).
            if st.sel.is_some() {
                for key in [Key::ArrowLeft, Key::ArrowRight] {
                    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) { st.sel = grid_step(&tiles, st.sel, key); st.scroll_to_sel = true; }
                }
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) { pick = Some(st.sel.unwrap_or(0).min(n - 1)); }
        }
        let mut open_terms = false;
        let mut load_more = false;
        let mut clear_query = false;
        let tx = st.tx.clone();

        egui::Area::new(egui::Id::new("gif-popup")).fixed_pos(pl.window.min).order(egui::Order::Foreground).interactable(true).show(&ctx, |ui| {
            ui.set_clip_rect(pl.window);
            let (_, _) = ui.allocate_exact_size(pl.window.size(), Sense::hover());
            let _ = ui.interact(pl.body, egui::Id::new("gif-popup-body"), Sense::click()); // clicks on the balloon stay in it
            let p = ui.painter().clone();
            // Drop shadow (blur 4, offset 0) in the 4 px margin, body + 1 px border, the caret pointing at the button.
            p.add(egui::Shadow { offset: [0, 0], blur: SHADOW_M as u8, spread: 0, color: pal.shadow }.as_shape(pl.body, BODY_R));
            p.rect_filled(pl.body, BODY_R, pal.body);
            p.rect_stroke(pl.body, BODY_R, Stroke::new(1.0_f32, pal.border), egui::StrokeKind::Inside);
            let [c0, tip, c1] = pl.caret();
            // The fill reaches 1 px into the body to cover the border under the caret's base.
            p.add(egui::Shape::convex_polygon(vec![pos2(c0.x, c0.y - 1.0), pos2(c1.x, c1.y - 1.0), tip], pal.body, Stroke::NONE));
            for c in [c0, c1] { p.line_segment([pos2(c.x, c.y - 0.5), tip], Stroke::new(1.0_f32, pal.border)); }
            // Search row: wx-headingColor-background with a 1 px bottom border of the same colour.
            let r = (BODY_R - 1.0) as u8;
            p.rect_filled(lay.search, egui::CornerRadius { nw: r, ne: r, sw: 0, se: 0 }, pal.head);

            // Search icon button (ghost): clicking it focuses the field.
            let search_id = egui::Id::new("gif-search");
            let iresp = ui.interact(lay.icon, egui::Id::new("gif-search-icon"), Sense::click());
            if iresp.hovered() { p.circle_filled(lay.icon.center(), BTN / 2.0, alpha(pal.text, 0.10)); }
            self.icons.paint(ui, "search-regular", Rect::from_center_size(lay.icon.center(), Vec2::splat(16.0)), pal.text2);
            if iresp.clicked() { st.focus = true; }
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(lay.field.shrink2(Vec2::new(6.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let r = c.add(egui::TextEdit::singleline(&mut st.query).id(search_id).hint_text(RichText::new("Nach GIFs suchen").color(pal.text2).size(14.0)).text_color(pal.text)
                .frame(false).margin(Vec2::ZERO).font(FontId::proportional(14.0)).desired_width(lay.field.width() - 12.0));
            if st.focus { r.request_focus(); st.focus = false; }
            if r.changed() { let q = st.query.clone(); st.debounce.set(&q, now); }
            // Cancel-search button: shown while there is text; a ring in wx-giphyIntegrationWidgetCancelSearchButton-background.
            if !st.query.is_empty() {
                let xresp = ui.interact(lay.cancel, egui::Id::new("gif-clear"), Sense::click());
                p.circle_stroke(lay.cancel.center(), 8.5, Stroke::new(1.0_f32, pal.cancel));
                self.icons.paint(ui, "cancel-bold", Rect::from_center_size(lay.cancel.center(), Vec2::splat(8.0)), pal.cancel);
                if xresp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                if xresp.on_hover_text("Eintrag löschen").clicked() { clear_query = true; }
            }

            // The grid, or the loading / error / empty page.
            let grid = lay.grid;
            let centre = grid.center();
            if n == 0 && st.loading {
                ui.put(Rect::from_center_size(centre, Vec2::splat(28.0)), egui::Spinner::new().size(28.0).color(pal.text2));
            } else if n == 0 && st.error.is_some() {
                // Connection error: 15 px title, 13 px inputText-secondary text.
                let g1 = p.layout_no_wrap("Suchen nach GIFs nicht möglich".into(), FontId::proportional(15.0), pal.text);
                p.galley(pos2(centre.x - g1.size().x / 2.0, centre.y - 24.0), g1, pal.text);
                let g2 = p.layout("GIFs funktionieren zurzeit nicht, bitte versuchen Sie es später noch einmal.".into(), FontId::proportional(13.0), pal.text2, grid.width() - 80.0);
                p.galley(pos2(centre.x - g2.size().x / 2.0, centre.y + 2.0), g2, pal.text2);
            } else if n == 0 && !st.loading {
                // No results: search_gif.png, "Keine Ergebnisse" (15 px), hint (13 px wx-subheading-text).
                if let Some((id, sz)) = giphy_art("search_gif.png").and_then(|b| self.icons.png(ui, "giphy-empty", b)) {
                    p.image(id, Rect::from_center_size(pos2(centre.x, centre.y - 52.0), Vec2::new(sz[0] as f32, sz[1] as f32) / 2.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                let g1 = p.layout_no_wrap("Keine Ergebnisse".into(), FontId::proportional(15.0), pal.text);
                p.galley(pos2(centre.x - g1.size().x / 2.0, centre.y + 4.0), g1, pal.text);
                let g2 = p.layout("Wir konnten hierfür kein GIF finden. Versuchen Sie es mit allgemeineren Schlüsselwörtern erneut.".into(), FontId::proportional(13.0), pal.text2, grid.width() - 80.0);
                p.galley(pos2(centre.x - g2.size().x / 2.0, centre.y + 28.0), g2, pal.text2);
            } else {
                // The scroll area: 12 px from the left, 8 px under the search row and above the footer, the arrow scroll bar at its right edge.
                let inner = Rect::from_min_max(pos2(grid.left() + PAD_L, grid.top() + PAD_T), pos2(grid.right(), grid.bottom() - PAD_B));
                let mut gui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                gui.set_clip_rect(inner);
                let mut sa = crate::ui::widgets::vscroll().id_salt(("gif-scroll", st.query.is_empty())).auto_shrink([false; 2]).arrow_bar(dark);
                if st.sel.is_none() && st.scroll_to_sel { sa = sa.vertical_scroll_offset(0.0); st.scroll_to_sel = false; }
                let out = sa.show_viewport(&mut gui, |ui, vp| {
                    let extra = if st.loading { 44.0 } else { 0.0 };
                    // masonry() counts a trailing gap under the tallest column: the last row ends flush with the viewport.
                    let (area, _) = ui.allocate_exact_size(Vec2::new(colw * COLS as f32 + GAP * (COLS as f32 - 1.0), (total_h - GAP).max(0.0) + extra), Sense::hover());
                    for (i, rel) in tiles.iter().enumerate() {
                        let r = rel.translate(area.min.to_vec2());
                        if st.sel == Some(i) && st.scroll_to_sel { ui.scroll_to_rect(r.expand2(Vec2::new(0.0, 6.0)), None); st.scroll_to_sel = false; }
                        if rel.bottom() < vp.top() - 120.0 || rel.top() > vp.bottom() + 120.0 { continue }
                        let g = st.gifs[i].clone();
                        st.thumbs.want(tx.as_ref(), &g.id, &g.thumb_url);
                        if rel.bottom() < vp.top() || rel.top() > vp.bottom() { continue }
                        let resp = ui.interact(r, egui::Id::new(("gif-tile", &g.id)), Sense::click());
                        // Tile: wx-giphy-background, the GIF inside a 2 px border (gray-80 / gray-10, blue-30 hovered or focused).
                        ui.painter().rect_filled(r, 0.0, pal.bg);
                        if let Some((tid, _)) = st.thumbs.tex(&ctx, &g.id, now) {
                            let ir = r.shrink(TILE_BORDER);
                            egui::Image::from_texture((tid, ir.size())).paint_at(ui, ir);
                        }
                        if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                        let on = resp.hovered() || st.sel == Some(i);
                        ui.painter().rect_stroke(r, 0.0, Stroke::new(TILE_BORDER, if on { pal.focus } else { pal.tile_edge }), egui::StrokeKind::Inside);
                        if resp.on_hover_text(&g.title).clicked() { pick = Some(i); }
                    }
                    if st.loading { ui.put(Rect::from_center_size(pos2(area.center().x, area.bottom() - 22.0), Vec2::splat(24.0)), egui::Spinner::new().size(24.0).color(pal.text2)); }
                });
                if !st.loading && st.next_offset < st.total && st.kind.is_some() && out.state.offset.y + out.inner_rect.height() > out.content_size.y - 160.0 { load_more = true; }
            }

            // Footer: 16 px margins, 75 px spacer, logo centred between two expanding spacers, link at the right; no top line.
            let foot = lay.foot;
            let link_font = FontId::proportional(16.0); // 16 px regular, underlined
            let lg = p.layout_no_wrap("Bedingungen anzeigen".into(), link_font, pal.link);
            let lr = Rect::from_min_size(pos2(foot.right() - FOOT_MARGIN - lg.size().x, foot.center().y - lg.size().y / 2.0), lg.size());
            let (l0, _) = lay.logo_x;
            let logo_cx = (l0 + lr.left()) / 2.0;
            if let Some((id, sz)) = giphy_art(if dark { "giphy_logo_white@2x.png" } else { "giphy_logo@2x.png" })
                .and_then(|b| self.icons.png(ui, if dark { "giphy-logo-w" } else { "giphy-logo" }, b)) {
                let s = (LOGO_MAX.x / sz[0] as f32).min(LOGO_MAX.y / sz[1] as f32).min(0.5); // @2x asset, at most 110 x 12
                p.image(id, Rect::from_center_size(pos2(logo_cx, foot.center().y), Vec2::new(sz[0] as f32, sz[1] as f32) * s), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                p.text(pos2(logo_cx, foot.center().y), egui::Align2::CENTER_CENTER, "POWERED BY GIPHY", FontId::proportional(10.0), pal.text2);
            }
            let lresp = ui.interact(lr.expand(3.0), egui::Id::new("gif-terms"), Sense::click());
            if lresp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            p.hline(lr.x_range(), lr.bottom() - 1.0, Stroke::new(1.0_f32, pal.link));
            p.galley(lr.min, lg, pal.link);
            if lresp.on_hover_text("Giphy-Geschäftsbedingungen anzeigen").clicked() { open_terms = true; }
        });

        if open_terms { system::open(giphy::TERMS_URL); }
        if clear_query { st.debounce.reset(); self.gif_show(&mut st, ""); st.focus = true; }
        if load_more {
            if let Some(kind) = st.kind.clone() { let o = st.next_offset; fetch(&mut st, kind, o); st.loading = true; }
        }
        // Click outside (but not on the opening button) closes the popup.
        let pressed = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
        if let Some(pos) = pressed { if !pl.contains(pos) && !anchor.contains(pos) { close = true; } }
        if let Some(i) = pick {
            if let Some(g) = st.gifs.get(i).cloned() { st.staged.insert(space.to_string(), g); }
            close = true;
        }
        if close { st.open = false; st.debounce.reset(); }
        let picked_gif = close && pick.is_some();
        put(st);
        // Setting "GIFs beim Anklicken sofort senden": the picked GIF goes out at once instead of waiting in the composer.
        if picked_gif && self.settings["gif_send_instantly"].as_bool().unwrap_or(false) { self.gif_send_staged(space, now); }
        else if picked_gif { ctx.memory_mut(|m| m.request_focus(egui::Id::new("composer-edit"))); }
    }
}
