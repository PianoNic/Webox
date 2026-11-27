//! The control bar over a playing video and its keys.
//!
//! The bar is 52 pt along the bottom of the picture on a dark fade, shown while paused and while the pointer is over the video,
//! faded in / out over 300 ms. Left: play / pause, the seek slider and the time ("0:12 / 1:30"); right: download or show in
//! folder, full screen, and the speaker button that opens a volume pop-over (0..1, step 0.05).
//! Keys: Space play / pause, Left / Right 5 s back / ahead, Up / Down volume by 0.05, M mute, F full screen, Esc leaves it.
use super::*;

const GHOST_S: f32 = 28.0;
const FADE_S: f32 = 0.3;
const SEEK_STEP: f64 = 5.0;
const VOL_STEP: f32 = 0.05;
/// The volume chosen last (kept for the next video of this run).
static LAST_VOLUME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000); // 1.0

impl App {
    /// Draws the bar for the open video inside `r` (the picture area); returns the bar's rect.
    pub(super) fn video_bar(&mut self, ui: &mut egui::Ui, t: &Th, r: Rect, viewer: bool, now: f64) -> Rect {
        let ctx = ui.ctx().clone();
        let bar = Rect::from_min_max(pos2(r.left(), r.bottom() - BANNER_H), r.max);
        let hover = ui.rect_contains_pointer(r);
        let Some((playing, pos, dur, vol, muted, audible, vol_open, scrub, alpha, key, name)) = ACTIVE.with(|s| {
            let mut s = s.borrow_mut();
            let x = s.as_mut()?;
            let show = !x.player.playing() || hover || x.volume_open || x.scrub.is_some();
            let dt = ui.input(|i| i.stable_dt).min(0.1) / FADE_S;
            x.bar = if show { (x.bar + dt).min(1.0) } else { (x.bar - dt).max(0.0) };
            if x.bar > 0.0 && x.bar < 1.0 { ctx.request_repaint(); }
            Some((x.player.playing(), x.player.position(), x.player.info().duration, x.player.volume(), x.player.muted(), x.player.audible(), x.volume_open, x.scrub, x.bar, x.key.clone(), x.name.clone()))
        }) else { return bar };
        if alpha <= 0.0 { return bar; }
        let p = ui.painter().clone();
        // transparent at the top of the bar to black at the bottom
        let mut m = Mesh::default();
        let (top, bot) = (Color32::from_black_alpha(0), Color32::from_black_alpha((170.0 * alpha) as u8));
        for (pt, col) in [(bar.left_top(), top), (bar.right_top(), top), (bar.right_bottom(), bot), (bar.left_bottom(), bot)] { m.vertices.push(Vertex { pos: pt, uv: egui::epaint::WHITE_UV, color: col }); }
        m.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        p.add(Shape::mesh(m));
        let fg = wa(Color32::WHITE, 0.95 * alpha);
        let cy = bar.center().y + 6.0;
        let ghost = |s: &mut Self, ui: &mut egui::Ui, at: Rect, icon: &'static str, tip: &str, salt: &str| -> egui::Response {
            let rs = ui.interact(at, Id::new(("video-bar", salt, viewer)), Sense::click());
            if rs.hovered() { ui.painter().circle_filled(at.center(), GHOST_S / 2.0, wa(Color32::WHITE, 0.15 * alpha)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            s.icons.paint(ui, icon, Rect::from_center_size(at.center(), Vec2::splat(16.0)), fg);
            rs.on_hover_text(tip.to_string())
        };
        let cell = |x: f32| Rect::from_min_size(pos2(x, cy - GHOST_S / 2.0), Vec2::splat(GHOST_S));
        let play_r = cell(bar.left() + 8.0);
        let play_tip = if playing { crate::tr!("RecordingControls", "Pause") } else { crate::tr!("RecordingControls", "Play") };
        if ghost(self, ui, play_r, if playing { "pause-bold" } else { "play-bold" }, &play_tip, "play").clicked() {
            ACTIVE.with(|s| if let Some(x) = s.borrow().as_ref() { x.player.toggle(); });
        }
        // right side: [download / folder] [full screen] [speaker], speaker at the right margin 8
        let spk_r = cell(bar.right() - 8.0 - GHOST_S);
        let fs_r = cell(spk_r.left() - GHOST_S);
        let dl_r = cell(fs_r.left() - GHOST_S);
        let silent = !audible || muted || vol <= 0.0;
        if ghost(self, ui, spk_r, if silent { "speaker-muted-bold" } else { "speaker-bold" }, &crate::tr!("LobbyMediaPlayer_qt6", "volume"), "spk").clicked() && audible {
            ACTIVE.with(|s| if let Some(x) = s.borrow_mut().as_mut() { x.volume_open = !x.volume_open; });
        }
        let fs_tip = if viewer { crate::tr!("MeetingViewButtons", "Exit full screen") } else { crate::tr!("MeetingViewButtons", "Full screen") };
        if ghost(self, ui, fs_r, if viewer { "fullscreen-exit-regular" } else { "fullscreen-regular" }, &fs_tip, "fs").clicked() {
            ACTIVE.with(|s| if let Some(x) = s.borrow_mut().as_mut() { x.viewer = !viewer; });
        }
        let att = Attachment { key: key.clone(), name: name.clone(), ..Default::default() };
        let shown = shown_file(&att);
        if ghost(self, ui, dl_r, if shown.is_some() { "open-in-folder-regular" } else { "download-regular" }, if shown.is_some() { crate::ui::widgets::tr("Show in folder") } else { crate::ui::widgets::tr("Download") }, "dl").clicked() {
            match shown { Some(f) => fs_::show_in_folder(&f), None => { let now = ui.input(|i| i.time); self.video_download(&ctx, &att, now); } }
        }
        // time label and seek slider
        let shown_pos = scrub.unwrap_or(pos);
        let tg = p.layout_no_wrap(format::position_text(shown_pos, dur), FontId::proportional(SECOND), fg);
        let room = dl_r.left() - 6.0 - (play_r.right() + 6.0);
        let with_time = room - tg.size().x - 12.0 >= 60.0;
        let time_x = dl_r.left() - 6.0 - tg.size().x;
        if with_time { p.galley(pos2(time_x, cy - tg.size().y / 2.0), tg, fg); }
        let sl = Rect::from_min_max(pos2(play_r.right() + 6.0, cy - 8.0), pos2(if with_time { time_x - 12.0 } else { dl_r.left() - 6.0 }, cy + 8.0));
        if let Some(to) = slider(ui, t, sl, if dur > 0.0 { (shown_pos / dur) as f32 } else { 0.0 }, alpha, Id::new(("video-seek", viewer))) {
            let target = to as f64 * dur;
            ACTIVE.with(|s| if let Some(x) = s.borrow_mut().as_mut() { match to_release(ui) { true => { x.player.seek(target); x.scrub = None; } false => x.scrub = Some(target) } });
        } else if scrub.is_some() && !ui.input(|i| i.pointer.primary_down()) {
            ACTIVE.with(|s| if let Some(x) = s.borrow_mut().as_mut() { if let Some(t) = x.scrub.take() { x.player.seek(t); } });
        }
        if vol_open && audible { self.video_volume(ui, t, spk_r, vol, viewer); }
        ACTIVE.with(|s| if let Some(x) = s.borrow_mut().as_mut() { if hover { x.moved_at = now; } });
        let _ = now;
        bar
    }

    /// The volume pop-over above the speaker button: 160 x 32, a 140 wide slider centred, step 0.05. A click elsewhere closes it.
    fn video_volume(&mut self, ui: &mut egui::Ui, t: &Th, anchor: Rect, vol: f32, viewer: bool) {
        let ctx = ui.ctx().clone();
        let size = Vec2::new(160.0, 32.0);
        let at = pos2((anchor.center().x - size.x / 2.0).max(4.0), anchor.top() - 8.0 - size.y);
        let id = Id::new(("video-volume", viewer));
        let mut close = false;
        egui::Area::new(id).order(egui::Order::Tooltip).fixed_pos(at).show(&ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(size, Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, 8.0, t.tk("wx-menuList-background"));
            p.rect_stroke(r, 8.0, Stroke::new(1.0_f32, t.tk("wx-default-border")), StrokeKind::Inside);
            let sl = Rect::from_center_size(r.center(), Vec2::new(140.0, 16.0));
            if let Some(v) = slider(ui, t, sl, vol, 1.0, id.with("s")) {
                let v = ((v / VOL_STEP).round() * VOL_STEP).clamp(0.0, 1.0);
                LAST_VOLUME.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
                ACTIVE.with(|s| if let Some(x) = s.borrow().as_ref() { x.player.set_volume(v); x.player.set_muted(false); });
            }
            let tip = crate::tr!("LobbyMediaPlayer_qt6", "%1% volume %2", (vol * 100.0).round(), if vol == 0.0 { crate::tr!("LobbyMediaPlayer_qt6", " muted").into_owned() } else { String::new() });
            ui.interact(r, id.with("bg"), Sense::hover()).on_hover_text(tip);
            if ui.input(|i| i.pointer.any_pressed()) && !ui.rect_contains_pointer(r) && !ui.rect_contains_pointer(anchor) { close = true; }
        });
        if close { ACTIVE.with(|s| if let Some(x) = s.borrow_mut().as_mut() { x.volume_open = false; }); }
    }

    /// Keys for the open video (only when no text field has the keyboard, or in the viewer).
    pub(super) fn video_keys(&mut self, ctx: &egui::Context) {
        let viewer = ACTIVE.with(|s| s.borrow().as_ref().is_some_and(|x| x.viewer));
        if !viewer && ctx.wants_keyboard_input() { return; }
        let take = |k: Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
        let (space, left, right, up, down, m, f, esc) = (take(Key::Space), take(Key::ArrowLeft), take(Key::ArrowRight), take(Key::ArrowUp), take(Key::ArrowDown), take(Key::M), take(Key::F), viewer && take(Key::Escape));
        ACTIVE.with(|s| {
            let mut s = s.borrow_mut();
            let Some(x) = s.as_mut() else { return };
            let pl = &x.player;
            if space { if pl.ended() { pl.seek(0.0); } pl.toggle(); }
            if left { pl.seek(pl.position() - SEEK_STEP); }
            if right { pl.seek(pl.position() + SEEK_STEP); }
            if up || down {
                let v = (pl.volume() + if up { VOL_STEP } else { -VOL_STEP }).clamp(0.0, 1.0);
                pl.set_volume(v);
                LAST_VOLUME.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
            if m { pl.set_muted(!pl.muted()); }
            if f { x.viewer = !x.viewer; }
            if esc { x.viewer = false; }
            if space || left || right || up || down || m || f { x.moved_at = ctx.input(|i| i.time); }
        });
    }
}

pub(super) fn last_volume() -> f32 { f32::from_bits(LAST_VOLUME.load(std::sync::atomic::Ordering::Relaxed)) }

/// The seek / volume slider in `r` (16 high): the value 0..1 under the pointer while it is pressed or dragged.
fn slider(ui: &mut egui::Ui, t: &Th, r: Rect, value: f32, alpha: f32, id: Id) -> Option<f32> {
    let rs = ui.interact(r, id, Sense::click_and_drag());
    let p = ui.painter();
    let track = Rect::from_min_max(pos2(r.left(), r.center().y - 2.0), pos2(r.right(), r.center().y + 2.0));
    let a = |c: Color32| c.gamma_multiply(alpha);
    p.rect_filled(track, 2.0, a(t.tk("slider-track-empty-background")));
    let v = value.clamp(0.0, 1.0);
    let x = track.left() + track.width() * v;
    p.rect_filled(Rect::from_min_max(track.min, pos2(x, track.max.y)), 2.0, a(t.tk("slider-track-indicator-background")));
    let active = rs.hovered() || rs.dragged() || rs.has_focus();
    if active {
        let st = if rs.dragged() { ":pressed" } else if rs.hovered() { ":hovered" } else { "" };
        p.circle_filled(pos2(x, r.center().y), 6.0, a(t.tk(&format!("slider-handle-background{st}"))));
        p.circle_stroke(pos2(x, r.center().y), 6.0, Stroke::new(1.0_f32, a(t.tk("slider-handle-border"))));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if rs.dragged() || rs.clicked() || rs.drag_stopped() {
        let px = rs.interact_pointer_pos().map_or(x, |q| q.x);
        return Some(((px - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0));
    }
    None
}

/// The pointer button went up this pass (end of a drag / click on the slider): the seek happens then.
fn to_release(ui: &egui::Ui) -> bool { ui.input(|i| i.pointer.any_released()) }
