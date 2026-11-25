//! Images, the lightbox, file chips and link preview cards.
use super::*;

impl App {
    /// One image attachment: max 320 wide / 320 high, aspect kept, 8 pt corners.
    /// Returns its height. Click opens the lightbox.
    /// `deleted`: the message was deleted and a plugin keeps it: the picture is grey, in colour while the pointer is on it.
    pub(super) fn image_block(&mut self, ui: &mut egui::Ui, c: &Mc, at: egui::Pos2, a: &Attachment, wrap: f32, salt: (&str, usize), deleted: bool) -> f32 {
        // Only called for visible rows: image_frame animates (loops, honours delays, schedules its own repaint).
        let now = ui.input(|i| i.time);
        let tex = self.image_frame(ui.ctx(), &a.key, now);
        // The decoded size also while it is being decoded again (dropped for the memory budget): the row keeps its height.
        let sz = image_box(a, wrap, tex.map(|t| t.1).or_else(|| self.photos.image_size(ui.ctx(), &a.key)));
        let r = Rect::from_min_size(at, sz);
        // Ids carry the message id and the attachment's index (the same file can be shared twice, or in two messages).
        let wid = Id::new(("msg-img", salt.0, salt.1, &a.key, cur_seq(ui.ctx())));
        // A kept deleted message: grey, in colour while the pointer is on the picture.
        let tex = match tex { Some(t) if deleted && !ui.rect_contains_pointer(r) => Some(self.image_gray(ui.ctx(), &a.key).map_or(t, |g| (g.0, t.1))), t => t };
        match tex {
            Some((tid, _)) => round_image(ui.painter(), r, IMG_RADIUS, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tid, Color32::WHITE),
            None => {
                // A loading preview while the bytes are on their way; the "failed preview" (warning-regular 32 + "Failed to load preview",
                // text-primary BodySecondary on the thumbnail background) when they never arrive.
                let now = ui.input(|i| i.time);
                let first = ui.ctx().data_mut(|d| *d.get_temp_mut_or_insert_with(Id::new(("img-first-seen", &a.key)), || now));
                let limit = if self.shot.is_some() { std::env::var("WEBOX_SHOT_IMG_FAIL_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(IMG_FAIL_SECS) } else { IMG_FAIL_SECS };
                ui.painter().rect_filled(r, IMG_RADIUS, c.tile); // attachment-file-preview-thumbnail-sent-background
                if now - first > limit {
                    let g = ui.painter().layout_no_wrap("Vorschau konnte nicht geladen werden".to_string(), FontId::proportional(SECOND), c.body);
                    let top = r.center().y - (32.0 + 8.0 + g.size().y) / 2.0;
                    self.icons.paint(ui, "warning-regular", Rect::from_center_size(pos2(r.center().x, top + 16.0), Vec2::splat(32.0)), c.body);
                    ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, top + 40.0), g, c.body);
                } else {
                    let k = (((now * 1.6).sin() * 0.5 + 0.5) as f32) * 0.5;
                    ui.painter().rect_filled(r, IMG_RADIUS, wa(c.body, 0.05 + k * 0.10));
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                }
            }
        }
        let resp = ui.interact(r, wid, Sense::click());
        if tex.is_some() && resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        if resp.clicked() && tex.is_some() { ui.ctx().data_mut(|d| d.insert_temp(Id::new("lightbox"), a.key.clone())); }
        if resp.secondary_clicked() { ctx_menu::note_hit(ui.ctx(), salt.0, ctx_menu::Hit::Image(a.clone())); } // the row's menu adds the picture's actions
        // Plugin hook (GIF-Extras, off by default): e.g. its favourite star on GIFs. Nothing is drawn here while the plugin is off.
        self.gif_ext_image_overlay(ui, r, a, salt, tex.is_some());
        sz.y
    }

    /// "Bild kopieren" / the viewer's "Kopieren": the picture onto the clipboard (the original file when it has arrived, else the
    /// thumbnail).
    pub(super) fn image_copy(&mut self, ctx: &egui::Context, key: &str) {
        let bytes = self.photos.image_bytes(&format!("full:{key}")).or_else(|| self.photos.image_bytes(key)).map(|b| b.to_vec());
        if let Some(img) = bytes.and_then(|b| image::load_from_memory(&b).ok()) {
            let rgba = img.to_rgba8();
            ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()));
        }
    }

    /// "Speichern unter" for a picture: the original file, named as it was shared ("Bild.<ext>" when it has no name).
    pub(super) fn image_save(&mut self, key: &str, a: Option<&Attachment>) {
        let mut name = a.map(|a| a.name.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Bild".into());
        if !name.contains('.') { name.push_str(match a.map_or("", |a| a.mime.as_str()) { "image/gif" => ".gif", "image/jpeg" => ".jpg", "image/webp" => ".webp", _ => ".png" }); }
        self.file_save_as(key, &name);
    }

    // Full-resolution hooks: App::request_full_image / image_full_frame / release_full_image (photos.rs) fetch the full file on demand.
    pub(super) fn viewer_frame(&mut self, ctx: &egui::Context, key: &str, now: f64) -> Option<(TextureId, [usize; 2], bool)> {
        // The full-resolution file once it has arrived, the 512 px thumbnail until then (scaled to fit either way).
        if let Some((t, s)) = self.image_full_frame(ctx, key, now) { return Some((t, s, true)); }
        self.image_frame(ctx, key, now).map(|(t, s)| (t, s, false))
    }
    pub(super) fn viewer_request(&mut self, key: &str) { self.request_full_image(key); }
    pub(super) fn viewer_release(&mut self, key: &str) { self.release_full_image(key); }

    /// The image viewer: contain-fit in the window (48 pt margins), mouse wheel / ctrl+wheel / pinch zoom around the cursor
    /// (fit ..= 8x), drag to pan, double click toggles fit <-> 100 %, + / - / 0 / arrows / Esc, buttons at the bottom, previous /
    /// next image of the chat.
    pub(super) fn lightbox(&mut self, ctx: &egui::Context, msgs: &[Message]) {
        let k_key = Id::new("lightbox");
        let Some(key) = ctx.data(|d| d.get_temp::<String>(k_key)) else { return };
        let (k_view, k_open) = (Id::new("lb-view"), Id::new("lb-opened"));
        let close = |s: &mut Self, ctx: &egui::Context, key: &str| {
            s.viewer_release(key);
            ctx.data_mut(|d| { d.remove::<String>(k_key); d.remove::<String>(k_open); d.remove::<(f32, Vec2)>(k_view); });
        };
        let now = ctx.input(|i| i.time);
        let Some((tid, [w, h], full)) = self.viewer_frame(ctx, &key, now) else { close(self, ctx, &key); return };
        if ctx.data(|d| d.get_temp::<String>(k_open)).as_deref() != Some(key.as_str()) {
            self.viewer_request(&key);
            ctx.data_mut(|d| { d.insert_temp(k_open, key.clone()); d.insert_temp(k_view, (1.0f32, Vec2::ZERO)); });
        }
        let keys: Vec<String> = msgs.iter().flat_map(|m| m.images.iter().map(|a| a.key.clone())).collect();
        let att = msgs.iter().flat_map(|m| m.images.iter()).find(|a| a.key == key).cloned();
        let at = keys.iter().position(|k| *k == key);
        let screen = ctx.screen_rect();
        let ppp = ctx.pixels_per_point();
        let (mut zoom, mut pan) = ctx.data(|d| d.get_temp::<(f32, Vec2)>(k_view)).unwrap_or((1.0, Vec2::ZERO));
        let (iw, ih) = (w as f32 / ppp, h as f32 / ppp); // 100 % in points (aspect = the picture's own)
        let avail = (screen.size() - Vec2::new(2.0 * LB_MARGIN, 2.0 * LB_MARGIN + 56.0)).max(Vec2::splat(50.0)); // room for the toolbar below
        let centre = screen.center() - Vec2::new(0.0, 28.0);
        let contain = (avail.x / iw).min(avail.y / ih);
        let fit = if full { contain.min(1.0) } else { contain }; // a thumbnail is scaled up to fit; the full picture never beyond 100 %
        let max_zoom = (8.0 / fit).max(1.0);
        let mut nav: Option<usize> = None;
        let mut want_close = false;
        let (mut zoom_by, mut about) = (1.0f32, screen.center());
        ctx.input(|i| {
            about = i.pointer.hover_pos().unwrap_or(screen.center());
            zoom_by = i.zoom_delta() * (i.smooth_scroll_delta.y * 0.0015).exp();
            if i.key_pressed(Key::Escape) { want_close = true; }
            if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) { zoom_by *= 1.25; about = screen.center(); }
            if i.key_pressed(Key::Minus) { zoom_by /= 1.25; about = screen.center(); }
            if i.key_pressed(Key::Num0) { zoom = 1.0; pan = Vec2::ZERO; }
            if let Some(a) = at {
                if i.key_pressed(Key::ArrowLeft) && a > 0 { nav = Some(a - 1); }
                if i.key_pressed(Key::ArrowRight) && a + 1 < keys.len() { nav = Some(a + 1); }
            }
        });
        let zoom_to = |z: f32, about: egui::Pos2, zoom: &mut f32, pan: &mut Vec2| {
            let z = z.clamp(1.0, max_zoom);
            let rel = about - centre;
            *pan = rel - (rel - *pan) * (z / *zoom);
            *zoom = z;
        };
        if (zoom_by - 1.0).abs() > 1e-4 { zoom_to(zoom * zoom_by, about, &mut zoom, &mut pan); }
        let mut lb_act: Option<&'static str> = None;
        egui::Area::new(Id::new("lightbox-area")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(215));
            if resp.dragged() { pan += resp.drag_delta(); }
            let s = fit * zoom;
            let size = Vec2::new(iw * s, ih * s);
            // Keep the picture overlapping the window: centred when it fits, otherwise no further than its edges.
            for (p, sz, av) in [(&mut pan.x, size.x, avail.x), (&mut pan.y, size.y, avail.y)] { let lim = ((sz - av) / 2.0).max(0.0); *p = if sz <= av { 0.0 } else { p.clamp(-lim, lim) }; }
            let ir = Rect::from_center_size(centre + pan, size);
            round_image(ui.painter(), ir, 0.0, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tid, Color32::WHITE);
            if resp.double_clicked() {
                let hundred = 1.0 / fit;
                let to = if zoom > 1.01 { 1.0 } else if (hundred - 1.0).abs() > 0.05 { hundred } else { 2.0 };
                zoom_to(to, resp.interact_pointer_pos().unwrap_or(screen.center()), &mut zoom, &mut pan);
            } else if resp.clicked() && resp.interact_pointer_pos().is_some_and(|p| !ir.contains(p)) { want_close = true; }
            if resp.dragged() { ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing); }
            // Toolbar (bottom centre) and close (top right).
            let tb = Rect::from_center_size(pos2(r.center().x, r.bottom() - 34.0), Vec2::new(4.0 * 40.0 + 70.0, 40.0));
            ui.painter().rect_filled(tb, 20.0, Color32::from_black_alpha(190));
            let btn = |s: &mut Self, ui: &mut egui::Ui, rect: Rect, icon: &'static str, tip: &str| -> bool {
                let rs = ui.interact(rect, Id::new(("lb-btn", icon, rect.min.x as i32)), Sense::click());
                if rs.hovered() { ui.painter().circle_filled(rect.center(), 16.0, Color32::from_white_alpha(28)); }
                s.icons.paint(ui, icon, Rect::from_center_size(rect.center(), Vec2::splat(18.0)), Color32::WHITE);
                rs.on_hover_text(tip).clicked()
            };
            let cell = |k: f32| Rect::from_min_size(pos2(tb.left() + 6.0 + k, tb.top() + 4.0), Vec2::splat(32.0));
            if btn(self, ui, cell(0.0), "zoom-out-regular", "Verkleinern (-)") { zoom_to(zoom / 1.25, screen.center(), &mut zoom, &mut pan); }
            ui.painter().text(pos2(tb.left() + 6.0 + 32.0 + 35.0, tb.center().y), Align2::CENTER_CENTER, format!("{:.0} %", s * 100.0), FontId::proportional(SECOND), Color32::WHITE);
            if btn(self, ui, cell(32.0 + 70.0), "zoom-in-regular", "Vergrößern (+)") { zoom_to(zoom * 1.25, screen.center(), &mut zoom, &mut pan); }
            let at_fit = zoom <= 1.01;
            if btn(self, ui, cell(32.0 + 70.0 + 36.0), if at_fit { "fit-to-window-expand-regular" } else { "fit-to-window-shrink-regular" }, if at_fit { "Originalgröße (100 %)" } else { "An Fenster anpassen (0)" }) {
                zoom_to(if at_fit { (1.0 / fit).max(1.0) } else { 1.0 }, screen.center(), &mut zoom, &mut pan);
            }
            if btn(self, ui, Rect::from_min_size(pos2(r.right() - 52.0, r.top() + 14.0), Vec2::splat(36.0)), "cancel-regular", "Schließen (Esc)") { want_close = true; }
            // Plugin hook (GIF-Extras, off by default): one extra button left of "Schließen".
            let extra = self.gif_ext_viewer_button(ui, Rect::from_min_size(pos2(r.right() - 96.0, r.top() + 14.0), Vec2::splat(36.0)), att.as_ref());
            // Speichern unter / Kopieren.
            let bx = r.right() - 96.0 - if extra { 44.0 } else { 0.0 };
            if btn(self, ui, Rect::from_min_size(pos2(bx, r.top() + 14.0), Vec2::splat(36.0)), "save-regular", "Speichern unter") { lb_act = Some("save"); }
            if btn(self, ui, Rect::from_min_size(pos2(bx - 44.0, r.top() + 14.0), Vec2::splat(36.0)), "copy-regular", "Kopieren") { lb_act = Some("copy"); }
            if let Some(a) = at {
                if a > 0 && btn(self, ui, Rect::from_center_size(pos2(r.left() + 36.0, r.center().y), Vec2::splat(36.0)), "arrow-left-regular", "Vorheriges Bild (←)") { nav = Some(a - 1); }
                if a + 1 < keys.len() && btn(self, ui, Rect::from_center_size(pos2(r.right() - 36.0, r.center().y), Vec2::splat(36.0)), "arrow-right-regular", "Nächstes Bild (→)") { nav = Some(a + 1); }
            }
        });
        ctx.data_mut(|d| d.insert_temp(k_view, (zoom, pan)));
        match lb_act {
            Some("copy") => self.image_copy(ctx, &key),
            Some(_) => { let a = msgs.iter().flat_map(|m| m.images.iter()).find(|a| a.key == key).cloned(); self.image_save(&key, a.as_ref()); }
            None => {}
        }
        if let Some(n) = nav {
            self.viewer_release(&key);
            ctx.data_mut(|d| { d.insert_temp(k_key, keys[n].clone()); d.remove::<String>(k_open); });
        } else if want_close { close(self, ctx, &key); }
    }

    /// File card: 320 x 52 tile (radius 8, border 2), blue 64 pt icon box, bold name, size below.
    /// `key`: the attachment for downloads (None: a file the data layer has no descriptor for; in sample mode a stand-in
    /// key is used so the actions work offline). Click = "Öffnen" (download, then open), the round button on hover = "Herunterladen"
    /// (once downloaded "In Ordner anzeigen"), right click = the file menu. While the file goes up (pending
    /// upload) or comes down, a progress bar runs along the card's bottom and the size line shows the percentage.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn file_chip(&mut self, ui: &mut egui::Ui, _t: &Th, c: &Mc, r: Rect, name: &str, size: u64, salt: (&str, usize), key: Option<&str>) {
        let sample_key = format!("sample-file:{}:{name}", salt.0);
        let key = key.map(String::from).or_else(|| self.client.is_none().then(|| sample_key.clone()));
        use crate::app::files::{self as af, Phase};
        let resp = ui.interact(r, Id::new(("msg-file", salt.0, salt.1, cur_seq(ui.ctx()))), Sense::click());
        let p = ui.painter().clone();
        // The theme's label colour at an opacity (white in dark, black in light; c.white is already 95 %).
        let fg = |a: f32| wa(if c.white.r() > 100 { Color32::WHITE } else { Color32::BLACK }, a);
        // attachment-file-default-sent-background (white 11 %, hovered 20 %), 2 pt border -sent-border (white 20 %), radius 8;
        // the fill stays inside the border.
        p.rect_filled(r.shrink(2.0), RADIUS - 2.0, if resp.hovered() { c.tile_hover } else { c.tile });
        p.rect_stroke(r, RADIUS, Stroke::new(2.0_f32, c.tile_border), StrokeKind::Inside);
        // Icon box: 64 wide, full height, radius 8, -previewBox-background #1170CF; glyph 21 pt high, -previewBox-icon (white 95 %).
        let icon = Rect::from_min_size(r.min, Vec2::new(64.0, r.height()));
        p.rect_filled(icon, RADIUS, c.file_icon_bg);
        self.icons.paint(ui, file_icon(name), Rect::from_center_size(icon.center(), Vec2::splat(21.0)), fg(0.95));
        // Progress: a pending upload of this message (4 pt bar along the top, button-message-fill-background), or a download.
        let now = ui.input(|i| i.time);
        let up = af::uploads().into_iter().find(|u| u.id == salt.0 && !matches!(u.phase, Phase::Failed(_)));
        let dl = key.as_deref().and_then(af::download);
        let down = dl.as_ref().filter(|d| matches!(d.phase, Phase::Preparing | Phase::Uploading));
        let done = dl.as_ref().is_some_and(|d| d.phase == Phase::Done && d.path.exists());
        let failed = dl.as_ref().is_some_and(|d| matches!(d.phase, Phase::Failed(_)));
        if let Some(u) = &up {
            let f = if self.client.is_none() { super::super::attach::sample_progress(u.started, now) } else { super::super::attach::percent(u.sent, u.total) as f32 / 100.0 };
            p.rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width() * f, 4.0)), 0.0, hex(0x1170CF));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
        }
        // Action button: anchored right, right margin 8, vertically centred, 28 high; shown when the file can be fetched.
        let has_btn = key.is_some() && up.is_none();
        let btn = Rect::from_min_size(pos2(r.right() - 8.0 - 28.0, r.center().y - 14.0), Vec2::splat(28.0));
        // Text column: left = icon box right + 4, right = action button left - 4, top margin 5; bold title, middle-elided;
        // the title starts 80 pt right of the card's left edge, size line 14 pt.
        let tx = r.left() + 80.0;
        let right = if has_btn { btn.left() - 4.0 } else { r.right() - 8.0 };
        let nm = elide_middle(ui, name, semibold(BODY), (right - tx).max(10.0));
        let ng = ui.painter().layout_no_wrap(nm, semibold(BODY), fg(0.95));
        let ty = r.top() + 5.0;
        p.galley(pos2(tx, ty), ng.clone(), c.white);
        // Subtitle: the file size (label-secondary 70 %), only when the text column is wider than 160.
        let sub = human_size(size);
        if !sub.is_empty() && right - (r.left() + 68.0) > 160.0 { p.text(pos2(tx, ty + ng.size().y), Align2::LEFT_TOP, sub, FontId::proportional(SECOND), fg(0.7)); }
        let Some(key) = key else { resp.on_hover_text(name); return };
        if up.is_some() { resp.on_hover_text(name); return; } // still going up: nothing to fetch yet
        let now_s = now;
        let mut act: Option<&'static str> = None;
        if let Some(d) = down {
            // Download progress: "NN%" (label-primary, right padding 6) + a 28 pt loading spinner.
            let pct = super::super::attach::percent(d.got, d.total);
            let sp = btn.center();
            p.circle_stroke(sp, 12.0, Stroke::new(2.0_f32, hex(0x8F8F8F))); // spinner-progress-track
            let pts: Vec<egui::Pos2> = (0..=48).map(|i| { let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * (pct as f32 / 100.0) * i as f32 / 48.0; sp + Vec2::new(a.cos(), a.sin()) * 12.0 }).collect();
            if pct > 0 { p.add(egui::Shape::line(pts, Stroke::new(2.0_f32, hex(0x64B4FA)))); } // spinner-progress-cursor
            p.text(pos2(btn.left() - 6.0, btn.center().y), Align2::RIGHT_CENTER, format!("{pct}%"), FontId::proportional(12.0), fg(0.95));
            let bs = ui.interact(btn, Id::new(("msg-file-btn", salt.0, salt.1, cur_seq(ui.ctx()))), Sense::click());
            if bs.on_hover_text("Abbrechen").clicked() { act = Some("file-cancel"); }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
        } else {
            // Action button: round secondary S, 28 pt: border button-secondary-border (white 50 %), background transparent /
            // hovered 7 % / pressed 20 %, 12 pt icon download_12 -> open-in-folder_12 after the download, warning_12 (download_12
            // on hover) after a failure; tooltips "Download" / "Show in folder" / "Failed".
            let bs = ui.interact(btn, Id::new(("msg-file-btn", salt.0, salt.1, cur_seq(ui.ctx()))), Sense::click());
            let fill = if bs.is_pointer_button_down_on() { 0.2 } else if bs.hovered() { 0.07 } else { 0.0 };
            if fill > 0.0 { p.circle_filled(btn.center(), 14.0, fg(fill)); }
            p.circle_stroke(btn.center(), 13.5, Stroke::new(1.0_f32, fg(0.5)));
            let (ic, tip, a) = if done { ("open-in-folder_12", "In Ordner anzeigen", "file-folder") }
                else if failed { (if bs.hovered() { "download_12" } else { "warning_12" }, "Fehlgeschlagen", "file-download") }
                else { ("download_12", "Herunterladen", "file-download") };
            self.icons.paint(ui, ic, Rect::from_center_size(btn.center(), Vec2::splat(12.0)), fg(0.95));
            if bs.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            if bs.on_hover_text(tip).clicked() { act = Some(a); }
        }
        if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        // Right click: the message's menu with the file's actions on top (ctx_menu.rs).
        if resp.secondary_clicked() { ctx_menu::note_hit(ui.ctx(), salt.0, ctx_menu::Hit::File { key: key.clone(), name: name.to_string() }); }
        if act.is_none() && resp.clicked() { act = Some("file-open"); }
        let _ = resp.on_hover_text(name);
        match act {
            Some("file-cancel") => crate::app::files::download_cancel(&key),
            Some(a) => { self.file_menu_do(a, &key, name, now_s); }
            None => {}
        }
    }

    /// The text parts of a link-preview card: bold title (1 line), description (2 lines), site name (1 line), and the card's
    /// layout for the available width (`avail` = min(available, 620)).
    pub(super) fn card_parts(ui: &egui::Ui, c: &Mc, p: &LinkPreview, avail: f32) -> (Arc<Galley>, Arc<Galley>, Arc<Galley>, CardLayout) {
        let tw = CardLayout::text_width(avail);
        let one = |s: &str, f: FontId, col: Color32| { let mut j = egui::text::LayoutJob::simple(s.to_string(), f, col, tw); j.wrap.max_rows = 1; j.wrap.break_anywhere = true; j.wrap.overflow_character = Some('…'); ui.fonts(|x| x.layout_job(j)) };
        let title = one(&p.title, semibold(BODY), c.white); // attachment-unfurl-site-title
        let mut dj = egui::text::LayoutJob::simple(p.description.clone(), FontId::proportional(SECOND), c.white, tw); // ...-site-description
        dj.wrap.max_rows = 2;
        dj.wrap.overflow_character = Some('…');
        let desc = ui.fonts(|x| x.layout_job(dj));
        let site = one(&p.site, FontId::proportional(BODY), wa(c.white, 0.7 / 0.95)); // attachment-unfurl-site-url
        let l = CardLayout::new(avail, [title.size().y, desc.size().y, site.size().y]);
        (title, desc, site, l)
    }

    /// Draws a link-preview card at `at` (attachment-unfurl-background, 8 pt corners): the 200 x 105 picture box
    /// (attachment-unfurl-border, also when there is no picture), then title / description / site. The picture covers
    /// the box, centred and cropped. A click opens the page (http/https only). Returns the card's height, which never
    /// depends on the picture.
    pub(super) fn link_card(&mut self, ui: &mut egui::Ui, c: &Mc, at: egui::Pos2, avail: f32, p: &LinkPreview, salt: (&str, usize)) -> f32 {
        let (title, desc, site, l) = Self::card_parts(ui, c, p, avail.min(CARD_MAX_W));
        let r = Rect::from_min_size(at, Vec2::new(l.w, l.h));
        let resp = ui.interact(r, Id::new(("unfurl", salt.0, salt.1, cur_seq(ui.ctx()))), Sense::click());
        if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        ui.painter().rect_filled(r, RADIUS, if resp.hovered() { c.tile_hover } else { c.tile });
        // Wide: the picture box spans the card's full height (a text column taller than 105 makes the card taller, and a
        // fixed-height box left a strip of the card and its rounded corner showing below it). Narrow: 200 x 105 on top.
        let bh = if l.wide { l.h } else { CARD_THUMB_H };
        let bx = Rect::from_min_size(r.min, Vec2::new(CARD_THUMB_W, bh));
        // The box shares the card's left (wide) / top (narrow) corners.
        let rr = RADIUS as u8;
        let fill = if l.wide { egui::CornerRadius { nw: rr, sw: rr, ne: 0, se: 0 } } else { egui::CornerRadius { nw: rr, ne: rr, sw: 0, se: 0 } };
        ui.painter().rect_filled(bx, fill, c.tile_border);
        let tex = if p.image_key.is_empty() { None } else { let now = ui.input(|i| i.time); self.image_frame(ui.ctx(), &p.image_key, now) };
        if let Some((tid, [iw, ih])) = tex {
            // The picture always covers the whole box (no letterbox strips, as in the Webex desktop card): the visible part
            // is centred and cropped along whichever axis is too long.
            let (u, v) = cover_fraction(iw as f32, ih as f32, CARD_THUMB_W, bh);
            let (u0, v0) = ((1.0 - u) / 2.0, (1.0 - v) / 2.0);
            // drawn into a rounded rect that reaches past the box on the side without rounded corners, clipped to the box
            let (big, uv) = if l.wide {
                let big = Rect::from_min_size(bx.min, Vec2::new(CARD_THUMB_W + 2.0 * RADIUS, bh));
                (big, Rect::from_min_max(pos2(u0, v0), pos2(u0 + u * big.width() / CARD_THUMB_W, v0 + v)))
            } else {
                let big = Rect::from_min_size(bx.min, Vec2::new(CARD_THUMB_W, bh + 2.0 * RADIUS));
                (big, Rect::from_min_max(pos2(u0, v0), pos2(u0 + u, v0 + v * big.height() / bh)))
            };
            round_image(&ui.painter().with_clip_rect(bx), big, RADIUS, uv, tid, Color32::WHITE);
        }
        let clip = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
        for (g, y) in [&title, &desc, &site].into_iter().zip(l.text_y) { clip.galley(pos2(r.left() + l.text_x, r.top() + y), g.clone(), c.white); }
        if resp.clicked() { open_web(&p.url); }
        if resp.secondary_clicked() { ctx_menu::note_hit(ui.ctx(), salt.0, ctx_menu::Hit::Link(p.url.clone())); }
        resp.on_hover_text(&p.url);
        l.h
    }
}

/// The visible part (width and height as fractions 0..=1 of the picture) when a `iw` x `ih` picture covers a
/// `bw` x `bh` box: the longer axis is cropped, the other one is shown completely.
pub(super) fn cover_fraction(iw: f32, ih: f32, bw: f32, bh: f32) -> (f32, f32) {
    if iw <= 0.0 || ih <= 0.0 || bw <= 0.0 || bh <= 0.0 { return (1.0, 1.0); }
    let (img, bx) = (iw / ih, bw / bh);
    if img > bx { (bx / img, 1.0) } else { (1.0, img / bx) }
}

#[cfg(test)]
mod cover_tests {
    use super::cover_fraction;
    #[test]
    fn picture_always_covers_the_box() {
        // wider than the box: full height, the sides are cropped
        let (u, v) = cover_fraction(2000.0, 1000.0, 200.0, 105.0);
        assert!((v - 1.0).abs() < 1e-6 && (u - (200.0 / 105.0) / 2.0).abs() < 1e-6);
        // taller than the box: full width, top and bottom cropped
        let (u, v) = cover_fraction(1000.0, 1000.0, 200.0, 105.0);
        assert!((u - 1.0).abs() < 1e-6 && (v - 105.0 / 200.0).abs() < 1e-6);
        // exactly the box's shape: nothing cropped
        assert_eq!(cover_fraction(400.0, 210.0, 200.0, 105.0), (1.0, 1.0));
        // a broken size never crops
        assert_eq!(cover_fraction(0.0, 10.0, 200.0, 105.0), (1.0, 1.0));
    }
}

/// Geometry of a link-preview card relative to its top-left (see the CARD_* constants).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CardLayout { pub wide: bool, pub w: f32, pub h: f32, pub text_x: f32, pub text_y: [f32; 3] }

impl CardLayout {
    fn is_wide(avail: f32) -> bool { avail >= CARD_MAX_W }

    /// Width of the text column: up to 420 next to the picture, the card's width below it.
    pub fn text_width(avail: f32) -> f32 {
        let w = avail.min(CARD_MAX_W);
        if Self::is_wide(avail) { (w - CARD_THUMB_W - CARD_SPACING).min(CARD_TEXT_MAX_W) } else { w }.max(40.0)
    }

    /// `heights`: title, description, site name. Wide: the row is max(105, column) high; a column shorter than that is
    /// stretched and its three cells grow equally with each text centred in its cell. Narrow: picture, 5 spacing, then the column.
    pub fn new(avail: f32, heights: [f32; 3]) -> Self {
        let wide = Self::is_wide(avail);
        let w = avail.min(CARD_MAX_W);
        let col = 2.0 * CARD_MARGIN + heights.iter().sum::<f32>() + 2.0 * CARD_SPACING;
        let (h, top, extra, text_x) = if wide { (col.max(CARD_THUMB_H), 0.0, (CARD_THUMB_H - col).max(0.0), CARD_THUMB_W + CARD_SPACING) }
                                      else { (CARD_THUMB_H + CARD_SPACING + col, CARD_THUMB_H + CARD_SPACING, 0.0, 0.0) };
        let mut y = top;
        let mut text_y = [0.0; 3];
        for (i, hh) in heights.iter().enumerate() {
            let (mt, mb) = (if i == 0 { CARD_MARGIN } else { 0.0 }, if i == 2 { CARD_MARGIN } else { 0.0 });
            text_y[i] = y + mt + extra / 6.0;
            y += mt + hh + mb + extra / 3.0 + CARD_SPACING;
        }
        CardLayout { wide, w, h, text_x, text_y }
    }
}

#[cfg(test)]
mod card_tests {
    use super::*;

    #[test]
    fn wide_card_is_105_high_with_the_texts_spread() {
        let l = CardLayout::new(800.0, [20.0, 19.0, 20.0]); // column 20+19+20 + 2*10 + 2*5 = 89 < 105
        assert!(l.wide);
        assert_eq!((l.w, l.h, l.text_x), (620.0, 105.0, 205.0));
        let e = 16.0 / 3.0;
        assert!((l.text_y[0] - (10.0 + e / 2.0)).abs() < 1e-3);
        assert!((l.text_y[1] - (10.0 + 20.0 + e + 5.0 + e / 2.0)).abs() < 1e-3);
        assert!((l.text_y[2] - (10.0 + 20.0 + 5.0 + 19.0 + 5.0 + 2.0 * e + e / 2.0)).abs() < 1e-3);
        assert_eq!(CardLayout::text_width(800.0), 415.0);
    }

    #[test]
    fn taller_texts_make_the_card_taller() {
        let l = CardLayout::new(620.0, [20.0, 38.0, 20.0]); // 78 + 2*10 + 2*5 = 108 > 105
        assert_eq!(l.h, 108.0);
        assert_eq!(l.text_y, [10.0, 35.0, 78.0]);
    }

    #[test]
    fn narrow_card_puts_the_picture_above() {
        let l = CardLayout::new(500.0, [20.0, 19.0, 20.0]);
        assert!(!l.wide);
        assert_eq!((l.w, l.h, l.text_x), (500.0, 105.0 + 5.0 + 89.0, 0.0));
        assert_eq!(l.text_y[0], 120.0);
        assert_eq!(CardLayout::text_width(500.0), 500.0);
    }
}
