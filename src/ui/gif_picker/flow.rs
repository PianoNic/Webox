//! Opening the picker, searches, the staged GIF chip and sending it.
use super::*;

impl App {
    pub(crate) fn gif_picker_toggle(&mut self, ctx: &egui::Context) {
        let mut st = take();
        st.open = !st.open;
        let ext = st.open && self.gif_extras_on();
        st.ext = ext;
        if st.open {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("emoji-open"), false)); // one popup at a time
            if !ext { self.gif_open(&mut st); }
        }
        put(st);
        // The plugin's popup resets itself (outside the borrowed core state).
        if ext && !self.gif_ext_open(ctx) { let mut st = take(); st.ext = false; self.gif_open(&mut st); put(st); }
    }

    /// An emoji picked in the plugin popup's "Emojis" tab (the composer inserts it at the caret).
    pub(crate) fn gif_take_emoji(&mut self) -> Option<String> { let mut st = take(); let e = st.pending_emoji.take(); put(st); e }

    // The org's device policy `ucf-giphy-rating` decides when present; Giphy wants it lower-case. Offline: the setting, default "g".
    pub(crate) fn gif_rating(&self) -> String {
        self.feature("ucf-giphy-rating").map(|v| v.as_str().to_ascii_lowercase()).filter(|r| !r.is_empty())
            .unwrap_or_else(|| self.settings["gif_rating"].as_str().unwrap_or("g").to_string())
    }

    /// Resets the standard popup: empty search field (focused), the trending GIFs (sample mode: generated GIFs, no network).
    pub(super) fn gif_open(&mut self, st: &mut State) {
        st.query.clear(); st.focus = true; st.sel = None; st.debounce.reset(); st.error = None; st.scroll_to_sel = true;
        st.sample = self.client.is_none();
        st.thumbs.offline = st.sample;
        ensure_channel(st);
        let keep: HashSet<String> = st.staged.values().map(|g| g.id.clone()).collect();
        st.thumbs.trim(&keep);
        if st.sample && st.sample_all.is_empty() {
            let all = giphy::sample_gifs();
            for (g, b) in &all { st.thumbs.insert(&g.id, Rc::new(b.clone())); }
            st.sample_all = all.iter().map(|(g, _)| g.clone()).collect();
            st.trending = Some((st.sample_all.clone(), st.sample_all.len() as u32, st.sample_all.len() as u32));
        }
        self.gif_show(st, "");
    }

    /// Shows the trending list (empty query) or the search results for `q`.
    pub(super) fn gif_show(&mut self, st: &mut State, q: &str) {
        st.sel = None; st.scroll_to_sel = true; st.error = None; st.gifs.clear(); st.total = 0; st.next_offset = 0; st.loading = false;
        let rating = self.gif_rating();
        if q.is_empty() {
            st.query.clear();
            // trending?api_key=...&rating=G&limit=30&offset=0 (the first page is kept for the session).
            if let Some((g, next, total)) = st.trending.clone() { st.ticket += 1; st.gifs = g; st.next_offset = next; st.total = total; st.kind = Some(Kind::Trending); }
            else { fetch(st, Kind::Trending, 0, &rating); }
        } else {
            st.query = q.to_string();
            if st.sample {
                st.gifs = st.sample_all.iter().filter(|g| g.title.to_lowercase().contains(&q.to_lowercase())).cloned().collect();
                st.total = st.gifs.len() as u32; st.next_offset = st.total; st.kind = None;
            } else { fetch(st, Kind::Search(q.to_string()), 0, &rating); }
        }
    }

    /// The staged GIF of `space` as the composer's attachment chip: 100 x 60 bricklet above the composer's top edge.
    pub(crate) fn gif_stage_chip(&mut self, ui: &mut egui::Ui, t: &Th, crect: Rect, space: &str) {
        let mut st = take();
        let Some(g) = st.staged.get(space).cloned() else { put(st); return };
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let dark = t.text1 == hex(0xF7F7F7);
        // Panel above the composer line.
        let bar = Rect::from_min_max(pos2(crect.left(), crect.top() - 72.0), pos2(crect.right(), crect.top()));
        let mut remove = false;
        egui::Area::new(egui::Id::new("gif-stage")).fixed_pos(bar.min).order(egui::Order::Middle).interactable(true).show(&ctx, |ui| {
            let (_, _) = ui.allocate_exact_size(bar.size(), Sense::click());
            let p = ui.painter().clone();
            p.rect_filled(bar, 0.0, t.composer);
            let item = Rect::from_min_size(pos2(bar.left() + 25.0, bar.top() + 6.0), Vec2::new(100.0, 60.0));
            // Card: wx-fileBrickletStaging-background (gray-90 / gray-05), image part wx-fileBrickletStagingImage-background (gray-70 / gray-10).
            let card = Rect::from_min_size(item.min + Vec2::new(0.0, 8.0), Vec2::new(92.0, 52.0));
            p.rect_filled(card, 5.0, if dark { hex(0x292929) } else { hex(0xF7F7F7) });
            let img = Rect::from_min_size(card.min, Vec2::new(80.0, 52.0));
            p.rect_filled(img, 4.0, if dark { hex(0x545454) } else { hex(0xEDEDED) });
            let fit = {
                let (w, h) = (g.thumb_w.max(1) as f32, g.thumb_h.max(1) as f32);
                let s = (66.0 / w).min(45.0 / h);
                Vec2::new(w * s, h * s)
            };
            if let Some((tid, _)) = st.thumbs.tex(&ctx, &g.id, now) {
                egui::Image::from_texture((tid, fit)).paint_at(ui, Rect::from_center_size(img.center(), fit));
            }
            // x: 16 x 16 at the top right (cancel-bold 12 px, label-primary-text).
            let xr = Rect::from_min_size(pos2(item.right() - 16.0 - 2.0, item.top() + 2.0), Vec2::splat(16.0));
            let xresp = ui.interact(xr, egui::Id::new("gif-stage-x"), Sense::click());
            self.icons.paint(ui, "cancel-bold", Rect::from_center_size(xr.center(), Vec2::splat(12.0)), if xresp.hovered() { t.text1 } else { t.text2 });
            if xresp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            if xresp.on_hover_text("Gestaffelte Datei entfernen").clicked() { remove = true; } // pack: "Remove staged file"
        });
        if remove { st.staged.remove(space); }
        put(st);
    }

    /// Whether a GIF is staged for `space` (so the composer may send with an empty text).
    pub(crate) fn gif_staged(&self, space: &str) -> bool {
        let st = take();
        let r = st.staged.contains_key(space);
        put(st);
        r
    }

    /// Called when the composer sends: hands the staged GIF to the data layer (`Req::SendGif`) and clears it.
    /// Returns whether a GIF was staged.
    pub(crate) fn gif_send_staged(&mut self, space: &str, now: f64) -> bool {
        let mut st = take();
        let g = st.staged.remove(space);
        put(st);
        let Some(g) = g else { return false };
        match &self.client {
            Some(c) => c.req(cloud::Req::SendGif { space: space.to_string(), url: g.media_url, title: g.title, width: g.media_w, height: g.media_h }),
            None => self.toast("GIF senden ist noch nicht verfügbar", now),
        }
        true
    }
}
