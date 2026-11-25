//! The inline video card and the video playing in it.
use super::*;

/// The card's size: the thumbnail's (or poster's) box like a picture (`image_box`: 320 x 320 at most, aspect kept); 16:9 at the
/// picture width while neither is known (a share without thumbnail has no size before its first frame is decoded).
pub(in super::super) fn card_size(a: &Attachment, avail: f32, known: Option<[usize; 2]>) -> Vec2 {
    if known.is_some() || (a.width > 0 && a.height > 0) { return image_box(a, avail, known); }
    let w = IMG_MAX.min(avail);
    Vec2::new(w, (w * 9.0 / 16.0).round())
}

/// The hover banner's background: black at 60 %, with white labels.
fn banner_bg() -> Color32 { Color32::from_black_alpha(153) }

impl App {
    /// The picture size a video card has before it is drawn (for `measure`): the decoded thumbnail, else the poster of the file
    /// when it is on this computer.
    pub(in super::super) fn video_known_size(&mut self, ctx: &egui::Context, a: &Attachment) -> Option<[usize; 2]> {
        if let Some(s) = self.photos.image_size(ctx, &a.key) { return Some(s); }
        match source(ctx, a) { Src::File(p) => poster(ctx, &p).map(|x| x.1), _ => None }
    }

    /// One video attachment at `at`; returns its height. `thumb`: the share has a thumbnail (it came in `Message.images`).
    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn video_block(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, at: egui::Pos2, a: &Attachment, wrap: f32, salt: (&str, usize), thumb: bool) -> f32 {
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        let tex = if thumb { self.image_frame(&ctx, &a.key, now) } else { None };
        let src = source(&ctx, a);
        let local_poster = match &src { Src::File(p) => poster(&ctx, p), _ => None };
        let known = tex.map(|x| x.1).or(local_poster.map(|x| x.1)).or_else(|| self.photos.image_size(&ctx, &a.key));
        let sz = card_size(a, wrap, known);
        let r = Rect::from_min_size(at, sz);
        let id = Id::new(("msg-video", salt.0, salt.1, &a.key, cur_seq(&ctx)));
        let resp = ui.interact(r, id, Sense::click());
        // Right click: the message's menu with the file's actions (Open, Download, Save as) on top.
        if resp.secondary_clicked() { super::super::ctx_menu::note_hit(&ctx, salt.0, super::super::ctx_menu::Hit::File { key: a.key.clone(), name: a.name.clone() }); }
        let active = active_key().as_deref() == Some(a.key.as_str());
        // a fetch that has finished: play now
        if !active && wanted(&a.key) && matches!(src, Src::File(_)) { self.video_start(&ctx, a, now); }
        // screenshot runs (with --sample): WEBOX_SHOT_VIDEO=play | pause | viewer starts the sample clip once it is made; pause
        // stops it after a second, viewer opens the full-screen viewer
        if self.shot.is_some() && a.key == SAMPLE_KEY && matches!(src, Src::File(_)) {
            static SHOT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
            let mode = std::env::var("WEBOX_SHOT_VIDEO").unwrap_or_default();
            if !mode.is_empty() && SHOT.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                SHOT.store(1, std::sync::atomic::Ordering::Relaxed);
                self.video_start(&ctx, a, now);
                ACTIVE.with(|s| if let Some(x) = s.borrow_mut().as_mut() { x.viewer = mode == "viewer"; });
            }
            if mode == "pause" && SHOT.load(std::sync::atomic::Ordering::Relaxed) == 1 && ACTIVE.with(|s| s.borrow().as_ref().is_some_and(|x| x.player.position() > 1.0)) {
                SHOT.store(2, std::sync::atomic::Ordering::Relaxed);
                ACTIVE.with(|s| if let Some(x) = s.borrow().as_ref() { x.player.pause(); });
            }
        }
        let active = active || active_key().as_deref() == Some(a.key.as_str());
        if active && !ACTIVE.with(|s| s.borrow().as_ref().is_some_and(|x| x.viewer)) {
            self.video_playing(ui, t, c, r, &resp, a, now);
            return sz.y;
        }
        let p = ui.painter().clone();
        p.rect_filled(r, IMG_RADIUS, c.tile); // attachment-file-preview-thumbnail-sent-background
        let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        match (tex, local_poster) {
            (Some((tid, _)), _) => round_image(&p, r, IMG_RADIUS, full, tid, Color32::WHITE),
            (None, Some((tid, [w, h], _))) => { let fit = contain(r, w, h); round_image(&p, fit, if fit == r { IMG_RADIUS } else { 0.0 }, full, tid, Color32::WHITE); }
            _ => self.icons.paint(ui, "file-video-regular", Rect::from_center_size(r.center() - Vec2::new(0.0, 44.0), Vec2::splat(24.0)), wa(c.white, 0.5)),
        }
        let failed_poster = matches!(&src, Src::File(p) if poster_failed(p));
        let center = r.center(); // anchors.centerIn: parent
        let mut retry = false;
        match &src {
            Src::Fetching(pct) => self.video_spinner(ui, c, center, Some(*pct), now),
            Src::Preparing => self.video_spinner(ui, c, center, None, now),
            Src::Failed(_) => retry = self.video_play_error(ui, t, c, r, false),
            Src::File(_) if failed_poster && !thumb => retry = self.video_play_error(ui, t, c, r, false),
            _ => {
                let br = Rect::from_center_size(center, Vec2::splat(PLAY_XL));
                let hov = ui.rect_contains_pointer(br);
                let ink = |k: &str| t.tk(&format!("button-primary-{k}{}", if hov { ":hovered" } else { "" }));
                p.circle_filled(br.center(), PLAY_XL / 2.0, ink("background"));
                self.icons.paint(ui, "play-filled", Rect::from_center_size(br.center() + Vec2::new(1.5, 0.0), Vec2::splat(20.0)), t.at(tok!("button-primary-text")));
            }
        }
        // the overlay banner on hover: name, size (+ our duration), download / show-in-folder
        let mut act = None;
        if resp.hovered() || ui.rect_contains_pointer(r) {
            act = self.video_banner(ui, c, r, a, &src, local_poster.map_or(0.0, |x| x.2), salt);
        }
        if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
        let resp = resp.on_hover_text(crate::trq!("Play video"));
        match act {
            Some("download") => self.video_download(&ctx, a, now),
            Some("folder") => if let Some(f) = shown_file(a) { fs_::show_in_folder(&f) },
            _ if resp.clicked() || retry => {
                if retry { fs_::download_cancel(&a.key); }
                self.video_start(&ctx, a, now);
            }
            _ => {}
        }
        if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Space) || i.key_pressed(Key::Enter)) { self.video_start(&ctx, a, now); }
        sz.y
    }

    /// The 52 pt banner along the card's bottom, its bottom corners rounded with the card.
    fn video_banner(&mut self, ui: &mut egui::Ui, c: &Mc, r: Rect, a: &Attachment, src: &Src, dur: f64, salt: (&str, usize)) -> Option<&'static str> {
        if r.height() < BANNER_H + 8.0 { return None; }
        let b = Rect::from_min_max(pos2(r.left(), r.bottom() - BANNER_H), r.max);
        let p = ui.painter().clone();
        p.rect_filled(b, egui::CornerRadius { nw: 0, ne: 0, sw: IMG_RADIUS as u8, se: IMG_RADIUS as u8 }, banner_bg());
        let fg = |k: f32| wa(Color32::WHITE, k); // attachment-file-preview-thumbnail-sent-overlay-label-primary / -secondary
        let btn = Rect::from_min_size(pos2(b.right() - 8.0 - 28.0, b.center().y - 14.0), Vec2::splat(28.0));
        let tx = b.left() + 12.0;
        let tw = (btn.left() - 8.0 - tx).max(20.0);
        let name = elide_middle(ui, &a.name, semibold(SECOND), tw);
        p.text(pos2(tx, b.top() + 8.0), Align2::LEFT_TOP, name, semibold(SECOND), fg(0.95));
        let mut sub = human_size(a.size);
        if dur > 0.0 { if !sub.is_empty() { sub.push_str(" · "); } sub.push_str(&format::time_text(dur)); }
        p.text(pos2(tx, b.top() + 28.0), Align2::LEFT_TOP, sub, F::Caption.font(), fg(0.7));
        if matches!(src, Src::Fetching(_)) { return None; }
        // download, or show in folder once the file is in the download folder
        let shown = shown_file(a).is_some();
        let bs = ui.interact(btn, Id::new(("msg-video-btn", salt.0, salt.1, cur_seq(ui.ctx()))), Sense::click());
        let fill = if bs.is_pointer_button_down_on() { 0.2 } else if bs.hovered() { 0.07 } else { 0.0 };
        if fill > 0.0 { p.circle_filled(btn.center(), 14.0, fg(fill)); }
        p.circle_stroke(btn.center(), 13.5, Stroke::new(1.0_f32, fg(0.5)));
        self.icons.paint(ui, if shown { "open-in-folder_12" } else { "download_12" }, Rect::from_center_size(btn.center(), Vec2::splat(12.0)), fg(0.95));
        let _ = c;
        if bs.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        bs.on_hover_text(if shown { crate::ui::widgets::tr("Show in folder") } else { crate::ui::widgets::tr("Download") }).clicked().then_some(if shown { "folder" } else { "download" })
    }

    /// A 40 pt spinner: the download's percentage as an arc, or turning while it is not known.
    pub(super) fn video_spinner(&mut self, ui: &mut egui::Ui, _c: &Mc, at: egui::Pos2, pct: Option<u32>, now: f64) {
        let p = ui.painter();
        p.circle_filled(at, 24.0, Color32::from_black_alpha(120));
        p.circle_stroke(at, 18.0, Stroke::new(3.0_f32, hex(0x8F8F8F))); // spinner-progress-track
        let (a0, span) = match pct { Some(k) => (-std::f32::consts::FRAC_PI_2, std::f32::consts::TAU * k as f32 / 100.0), None => ((now * 5.0) as f32, 1.6) };
        let pts: Vec<egui::Pos2> = (0..=40).map(|i| { let a = a0 + span * i as f32 / 40.0; at + Vec2::new(a.cos(), a.sin()) * 18.0 }).collect();
        if span > 0.0 { p.add(Shape::line(pts, Stroke::new(3.0_f32, hex(0x64B4FA)))); } // spinner-progress-cursor
        if let Some(k) = pct { p.text(at, Align2::CENTER_CENTER, format!("{k}%"), FontId::proportional(11.0), Color32::WHITE); }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }

    /// The error over the card: warning icon, title, explanation and a "Reload" pill. Returns true when Reload was clicked.
    pub(super) fn video_play_error(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, r: Rect, inline_format: bool) -> bool {
        let p = ui.painter().clone();
        p.rect_filled(r, IMG_RADIUS, c.tile);
        let title = if inline_format { crate::trq!("Can't play video inline") } else { crate::trq!("Can't play video") };
        let body = if inline_format { String::new() } else { crate::trq!("We ran into an issue while trying to play the video. Try again in a bit.").into_owned() };
        let w = r.width() - 24.0;
        let tg = ui.painter().layout(title.into_owned(), semibold(SECOND), c.white, w);
        let bg = ui.painter().layout(body, F::Caption.font(), wa(c.white, 0.7), w);
        let reload = crate::tr!("LobbyMediaPlayer_qt6", "Reload").into_owned();
        let total = 32.0 + 6.0 + tg.size().y + 4.0 + bg.size().y + 10.0 + 28.0;
        let mut y = r.center().y - total / 2.0;
        self.icons.paint(ui, "warning-bold", Rect::from_center_size(pos2(r.center().x, y + 16.0), Vec2::splat(32.0)), t.at(tok!("label-secondary-text")));
        y += 38.0;
        p.galley(pos2(r.center().x - tg.size().x / 2.0, y), tg.clone(), c.white);
        y += tg.size().y + 4.0;
        p.galley(pos2(r.center().x - bg.size().x / 2.0, y), bg.clone(), c.white);
        y += bg.size().y + 10.0;
        let g = ui.painter().layout_no_wrap(reload, F::Secondary.semibold(), c.white);
        let pill = Rect::from_center_size(pos2(r.center().x, y + 14.0), Vec2::new(g.size().x + 28.0, 28.0));
        if pill.bottom() > r.bottom() { return false; }
        let rs = ui.interact(pill, ui.id().with(("video-reload", r.min.x as i32, r.min.y as i32)), Sense::click());
        if rs.hovered() { p.rect_filled(pill, 14.0, wa(c.white, 0.07)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        p.rect_stroke(pill, 14.0, Stroke::new(1.0_f32, wa(c.white, 0.5)), StrokeKind::Inside);
        p.galley(pill.center() - g.size() / 2.0, g, c.white);
        rs.clicked()
    }

    /// The card while its video is open: the picture (contain-fit), the control bar, the
    /// big play button while paused, the spinner until the first picture, the error when it cannot play.
    #[allow(clippy::too_many_arguments)]
    fn video_playing(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, r: Rect, resp: &egui::Response, a: &Attachment, now: f64) {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        p.rect_filled(r, IMG_RADIUS, Color32::BLACK);
        let st = ACTIVE.with(|s| { let mut s = s.borrow_mut(); let x = s.as_mut()?; x.drawn = ctx.cumulative_pass_nr(); x.drawn_at = now; pull_frame(&ctx, x); Some((x.tex.as_ref().map(|t| t.id()), x.size, x.player.info(), x.player.playing(), x.player.ended())) });
        let Some((tid, size, info, playing, ended)) = st else { return };
        if let Some(e) = &info.error {
            let _ = e;
            if self.video_play_error(ui, t, c, r, false) { stop_active(); self.video_start(&ctx, a, now); }
            return;
        }
        if let Some(tid) = tid { round_image(&p, contain(r, size[0], size[1]), 0.0, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tid, Color32::WHITE); }
        else { self.video_spinner(ui, c, r.center(), None, now); }
        let bar = self.video_bar(ui, t, r, false, now);
        // a click on the picture (not on the bar) plays / pauses; a double click opens the viewer
        if resp.double_clicked() { ACTIVE.with(|s| if let Some(x) = s.borrow_mut().as_mut() { x.viewer = true; }); }
        else if resp.clicked() && !resp.interact_pointer_pos().is_some_and(|q| bar.contains(q)) { ACTIVE.with(|s| if let Some(x) = s.borrow().as_ref() { if x.player.ended() { x.player.seek(0.0); } x.player.toggle(); }); }
        if (!playing || ended) && tid.is_some() {
            let br = Rect::from_center_size(r.center(), Vec2::splat(PLAY_XL));
            p.circle_filled(br.center(), PLAY_XL / 2.0, t.at(tok!("button-primary-background")));
            self.icons.paint(ui, "play-filled", Rect::from_center_size(br.center() + Vec2::new(1.5, 0.0), Vec2::splat(20.0)), t.at(tok!("button-primary-text")));
        }
        if resp.hovered() { self.video_keys(&ctx); }
    }
}

/// The largest rect of aspect `w : h` inside `r`, centred.
pub(super) fn contain(r: Rect, w: usize, h: usize) -> Rect {
    if w == 0 || h == 0 { return r; }
    let k = (r.width() / w as f32).min(r.height() / h as f32);
    let s = Vec2::new(w as f32 * k, h as f32 * k);
    if (s - r.size()).length() < 1.0 { r } else { Rect::from_center_size(r.center(), s) }
}

/// Takes the picture that is due into the texture and asks for the repaint of the next one.
pub(super) fn pull_frame(ctx: &egui::Context, x: &mut Active) {
    let (f, due) = x.player.frame();
    if let Some(f) = f {
        let img = egui::ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.rgba);
        match &mut x.tex { Some(t) => t.set(img, egui::TextureOptions::LINEAR), None => x.tex = Some(ctx.load_texture("video-frame", img, egui::TextureOptions::LINEAR)) }
        x.size = [f.w, f.h];
    }
    if let Some(d) = due { ctx.request_repaint_after(d); }
}
