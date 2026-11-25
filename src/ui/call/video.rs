//! Call video in the UI: our camera on / off ("Start video" / "Stop video"), calls started or answered with the camera
//! on (the Video button, "Answer calls with my video on"), the camera menu behind the video button
//! (`devices.rs`), the self view and the other side's video as the call window's picture. The media is the cloud thread's
//! (`cloud/locus/video.rs`): `CallReq::Video(Some(camera))` / `CallReq::Video(None)` here, `Resp::CallVideo` back into
//! `CallUi::video`, the pictures from `cloud::video_rx`.
//!
//! Self view like the official `InnerFloatLocalContainer` / `MeetingView`: a 182 x 102 tile with a drop shadow, 8 from the
//! edges (full screen: 45 on the right and 3 on top), in a corner of the stage (bottom right at first). It can be dragged
//! and snaps to the nearest corner when let go; with the keyboard the arrow keys move it ("Use the arrow keys to move the
//! self view window"). It shows only while our video is on ("You can only show your self-view video when your video is
//! on.") and is mirrored like Settings > Video ("camera_mirror", default on).
use super::*;
use crate::cloud::video_rx;
use crate::media::camera::{self, CamDev, PreviewKey};

/// The self-view tile (pt).
pub(crate) const PIP: Vec2 = Vec2::new(182.0, 102.0);
const PIP_MARGIN: f32 = 8.0;

#[derive(Default)]
pub(crate) struct VideoUi {
    /// The camera is wanted on but the camera list isn't there yet to name it (sent from `video_frame` once it is).
    pending: bool,
    /// "Hide self view" (this call), the corner the self view sits in, where a drag of it started.
    pub(crate) hidden: bool, pub(crate) left: bool, pub(crate) top: bool, drag: Option<Vec2>,
    /// Textures of our camera and of the other side's video, and the picture each holds (uploaded once per picture).
    self_tex: Option<egui::TextureHandle>, self_seq: u64, remote_tex: Option<egui::TextureHandle>, remote_seq: u64,
    /// The cameras for the video menu (listed on a worker thread while the menu is open).
    pub(super) cams: Vec<CamDev>, pub(super) cam_job: Option<std::sync::mpsc::Receiver<Vec<CamDev>>>, pub(super) cams_at: f64,
    /// Offline sample: the toast answers with video whatever the setting says.
    pub(crate) sample_answer_video: bool,
}

/// The self view's rect in `area` (the stage below the title bar, above the control bar).
pub(crate) fn pip_rect(area: Rect, left: bool, top: bool, full_screen: bool) -> Rect {
    let (right_m, top_m) = if full_screen { (45.0, 3.0) } else { (PIP_MARGIN, PIP_MARGIN) };
    let x = if left { area.left() + PIP_MARGIN } else { (area.right() - PIP.x - right_m).max(area.left()) };
    let y = if top { area.top() + top_m } else { (area.bottom() - PIP.y - PIP_MARGIN).max(area.top()) };
    Rect::from_min_size(pos2(x, y), PIP)
}

/// The corner a self view let go at `center` snaps to: (left, top).
pub(crate) fn nearest_corner(area: Rect, center: egui::Pos2) -> (bool, bool) { (center.x < area.center().x, center.y < area.center().y) }

/// The arrow keys move the self view from corner to corner.
pub(crate) fn corner_after_key(key: Key, (left, top): (bool, bool)) -> (bool, bool) {
    match key { Key::ArrowLeft => (true, top), Key::ArrowRight => (false, top), Key::ArrowUp => (left, true), Key::ArrowDown => (left, false), _ => (left, top) }
}

/// The video button's tooltip (`DeviceControlBar`: "Start video" / "Stop video").
pub(crate) fn video_tip(on: bool) -> &'static str { if on { tr("Stop video") } else { tr("Start video") } }

/// The camera to use: the saved one if it is plugged in, else the first (as Settings > Video picks it).
pub(crate) fn pick_camera<'a>(devs: &'a [CamDev], want: &str) -> Option<&'a CamDev> { devs.iter().find(|d| d.name == want).or(devs.first()) }

/// Texture coordinates of the self view: `uv` mirrored left to right, or not.
pub(crate) fn mirror_uv(uv: Rect, mirror: bool) -> Rect {
    if mirror { Rect::from_min_max(pos2(uv.max.x, uv.min.y), pos2(uv.min.x, uv.max.y)) } else { uv }
}

/// Uploads the newest picture of a slot, only when a new one came.
fn upload(ctx: &egui::Context, name: &str, pic: Option<std::sync::Arc<cloud::share_rx::Picture>>, tex: &mut Option<egui::TextureHandle>, seq: &mut u64) -> Option<(egui::TextureId, Vec2)> {
    let Some(p) = pic else { *tex = None; *seq = 0; return None };
    if p.seq != *seq {
        let img = egui::ImageData::Color(p.image.clone());
        match tex { Some(t) => t.set(img, egui::TextureOptions::LINEAR), None => *tex = Some(ctx.load_texture(name, img, egui::TextureOptions::LINEAR)) }
        *seq = p.seq;
    }
    tex.as_ref().map(|t| (t.id(), t.size_vec2()))
}

impl App {
    /// "Answer calls with my video on" (Settings > Calling).
    pub(crate) fn answer_with_video(&self) -> bool {
        self.settings["answer_with_video"].as_bool().unwrap_or(false) || (self.call.sample && self.call.video_ui.sample_answer_video)
    }
    /// The mirror setting the self view follows (Settings > Video "Mirror my video preview").
    pub(crate) fn self_view_mirrored(&self) -> bool { self.settings["camera_mirror"].as_bool().unwrap_or(true) }

    /// Before a dial / answer / join: the camera the call starts with, and whether it is on (the cloud thread handles the
    /// requests in order, so the call's first OFFER already has them).
    pub(crate) fn prime_video(&mut self, on: bool) {
        self.end_video();
        self.call.video = Default::default();
        self.set_video(on); // off: also clears a camera left wanted by a dial that failed
    }

    /// The camera the call sends: the chosen device with Settings > Video's capture options.
    fn camera_key(&self, dev: CamDev) -> PreviewKey {
        let max_h = self.settings["camera_resolution"].as_str().unwrap_or("auto").parse().unwrap_or(1080);
        PreviewKey { dev: Some(dev), max_h, hw: self.settings["camera_hw_accel"].as_bool().unwrap_or(true), sample: self.call.sample, src: camera::hook::source() }
    }

    /// Our camera on (the saved camera, else the first) or off. Shown at once; the cloud thread's `Resp::CallVideo` follows.
    pub(crate) fn set_video(&mut self, on: bool) {
        self.call.video.wanted = on;
        self.call.video_ui.pending = false;
        if self.call.sample { return; }
        if !on { if let Some(c) = &self.client { c.req(Req::Call(CallReq::Video(None))); } return; }
        let want = self.settings["camera"].as_str().unwrap_or("").to_string();
        match pick_camera(&self.call.video_ui.cams, &want).cloned() {
            Some(d) => { let k = self.camera_key(d); if let Some(c) = &self.client { c.req(Req::Call(CallReq::Video(Some(k)))); } }
            None => self.call.video_ui.pending = true, // the list comes from a worker thread (`video_frame`)
        }
    }

    /// Every frame while a call shows: a camera wanted before the camera list was there goes out once the list is in.
    pub(crate) fn video_frame(&mut self, now: f64) {
        if !self.call.video_ui.pending { return; }
        self.refresh_cams(now);
        if self.call.video_ui.cam_job.is_some() { return; }
        self.call.video_ui.pending = false;
        if self.call.video_ui.cams.is_empty() {
            self.call.video.wanted = false;
            self.toast(tr("Video not possible: no camera found."), now);
        } else { self.set_video(true); }
    }

    /// The video button: our camera on / off in the running call (Locus gets `videoMuted` from the cloud thread).
    pub(crate) fn toggle_video(&mut self) { let on = !self.call.video.wanted; self.set_video(on); }

    /// A camera was picked in the video menu: saved like Settings > Video's choice, and a running camera switches to it
    /// (the same request with the other device; the media keeps the line, no new negotiation).
    pub(crate) fn switch_camera(&mut self, name: &str) {
        self.settings["camera"] = json!(name);
        if self.call.sample || self.shot.is_some() { return; }
        system::save_settings(&self.cfg, &self.settings);
        if self.call.video.wanted { self.set_video(true); }
    }

    /// The call ended: video state goes (the corner of the self view stays for the next call).
    pub(crate) fn end_video(&mut self) {
        let v = &mut self.call.video_ui;
        *v = VideoUi { left: v.left, top: v.top, cams: std::mem::take(&mut v.cams), cams_at: v.cams_at, ..Default::default() };
    }

    /// The other side's newest video picture and its size (None while they send none).
    pub(crate) fn remote_video_picture(&mut self, ctx: &egui::Context) -> Option<(egui::TextureId, Vec2)> {
        let v = &mut self.call.video_ui;
        upload(ctx, "call-remote-video", video_rx::remote(), &mut v.remote_tex, &mut v.remote_seq)
    }

    /// Our camera's newest picture (None while it is off or starting).
    fn self_view_picture(&mut self, ctx: &egui::Context) -> Option<(egui::TextureId, Vec2)> {
        let v = &mut self.call.video_ui;
        upload(ctx, "call-self-view", video_rx::self_view(), &mut v.self_tex, &mut v.self_seq)
    }

    /// The control bar's video button in `r` (64 x 40, like the official dual-arrow `UTCallControlButton`): the camera toggle on
    /// the left ("Start video" / "Stop video"; red while the video is off, like the muted microphone), the arrow to the
    /// video menu on the right ("Video options"). Returns (toggle, arrow).
    pub(crate) fn video_button(&mut self, ui: &mut egui::Ui, t: &Th, r: Rect, open: bool) -> (egui::Response, egui::Response) {
        let on = self.call.video.wanted;
        let split = r.left() + 40.0;
        let (lr, rr) = (Rect::from_min_max(r.min, pos2(split, r.bottom())), Rect::from_min_max(pos2(split, r.top()), r.max));
        let left = ui.interact(lr, ui.id().with("bar-video"), Sense::click());
        let right = ui.interact(rr, ui.id().with("bar-video-menu"), Sense::click());
        let base = if on { t.selected } else { t.badge };
        let shade = |resp: &egui::Response, c: Color32| if resp.is_pointer_button_down_on() { c.gamma_multiply(0.7) } else if resp.hovered() { c.gamma_multiply(0.85) } else { c };
        let p = ui.painter();
        p.rect_filled(lr, egui::CornerRadius { nw: 20, sw: 20, ne: 0, se: 0 }, shade(&left, base));
        p.rect_filled(rr, egui::CornerRadius { nw: 0, sw: 0, ne: 20, se: 20 }, shade(&right, if open { t.accent } else { base }));
        p.line_segment([pos2(split, r.top() + 10.0), pos2(split, r.bottom() - 10.0)], Stroke::new(1.0_f32, Color32::from_white_alpha(70)));
        self.icons.paint(ui, if on { "camera-regular" } else { "camera-muted-regular" }, Rect::from_center_size(lr.center() + Vec2::new(2.0, 0.0), Vec2::splat(20.0)), Color32::WHITE);
        self.icons.paint(ui, if open { "arrow-up-filled" } else { "arrow-down-filled" }, Rect::from_center_size(rr.center() - Vec2::new(2.0, 0.0), Vec2::splat(12.0)), Color32::WHITE);
        for resp in [&left, &right] {
            if resp.has_focus() { ui.painter().rect_stroke(resp.rect.expand(1.5), 4.0, Stroke::new(2.0_f32, t.focus), egui::StrokeKind::Inside); }
            if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        }
        (left.on_hover_text(video_tip(on)), right.on_hover_text(tr("Video options")))
    }

    /// The self view in `area` (the stage), while our video is on and it isn't hidden: draggable, snaps to the nearest
    /// corner, arrow keys move it while it has the focus.
    pub(crate) fn self_view(&mut self, ui: &mut egui::Ui, t: &Th, area: Rect, full_screen: bool) {
        let (on, v) = (self.call.video.wanted, &self.call.video_ui);
        if !on || v.hidden || area.width() < PIP.x + 2.0 * PIP_MARGIN || area.height() < PIP.y + 2.0 * PIP_MARGIN { return; }
        let home = pip_rect(area, v.left, v.top, full_screen);
        let r = match v.drag { Some(off) => {
            let p = ui.ctx().pointer_latest_pos().unwrap_or(home.min + off);
            let min = pos2((p.x - off.x).clamp(area.left(), area.right() - PIP.x), (p.y - off.y).clamp(area.top(), area.bottom() - PIP.y));
            Rect::from_min_size(min, PIP)
        } None => home };
        let resp = ui.interact(r, ui.id().with("self-view"), Sense::click_and_drag());
        if resp.drag_started() { if let Some(p) = resp.interact_pointer_pos() { self.call.video_ui.drag = Some(p - r.min); } }
        if resp.drag_stopped() {
            let (l, tp) = nearest_corner(area, r.center());
            let v = &mut self.call.video_ui;
            v.left = l; v.top = tp; v.drag = None;
        }
        if resp.has_focus() {
            let keys = [Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp, Key::ArrowDown];
            if let Some(k) = keys.into_iter().find(|k| ui.input(|i| i.key_pressed(*k))) {
                let v = &mut self.call.video_ui;
                (v.left, v.top) = corner_after_key(k, (v.left, v.top));
            }
        }
        // wx-ghDropShadow, then the picture filling the tile (cropped to it, like the official crop of the self view)
        ui.painter().add(egui::epaint::Shadow { offset: [0, 2], blur: 12, spread: 0, color: Color32::from_black_alpha(110) }.as_shape(r, 6.0));
        ui.painter().rect_filled(r, 6.0, Color32::BLACK);
        let ctx = ui.ctx().clone();
        match self.self_view_picture(&ctx) {
            Some((id, size)) => {
                let uv = crate::ui::call_win::fill_uv(size, r.size());
                let uv = mirror_uv(uv, self.self_view_mirrored());
                let mut mesh = egui::Mesh::with_texture(id);
                mesh.add_rect_with_uv(r, uv, Color32::WHITE);
                ui.painter().add(mesh);
            }
            None => {
                // camera starting: our own avatar, like a tile without video
                let (name, email) = (self.me.name.clone(), self.me.email.clone());
                let photo = self.photo_for(&ctx, "", &email);
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(r.center(), Vec2::splat(48.0))));
                avatar_photo(&mut c, &mut self.icons, t, &name, 48.0, false, None, None, photo);
            }
        }
        ui.painter().rect_stroke(r, 6.0, Stroke::new(1.0_f32, t.sep), egui::StrokeKind::Inside);
        if resp.has_focus() { ui.painter().rect_stroke(r.expand(2.0), 8.0, Stroke::new(2.0_f32, t.focus), egui::StrokeKind::Inside); }
        if resp.hovered() { ui.ctx().set_cursor_icon(if self.call.video_ui.drag.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab }); }
        resp.on_hover_text(tr("Use the arrow keys to move the self view window"));
    }
}

/// `WEBOX_SAMPLE_CALL=video` (offline sample): made-up pictures for both sides.
pub(crate) fn show_sample() {
    let pic = |i: u32, w: u32, h: u32| { let mut px = vec![0u8; (w * h * 4) as usize]; crate::media::share::convert::test_picture(i, w, h, &mut px); px };
    video_rx::MAIN.publish(1280, 720, &pic(0, 1280, 720));
    video_rx::SELF.publish(320, 180, &pic(3, 320, 180));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area() -> Rect { Rect::from_min_size(pos2(0.0, 40.0), Vec2::new(800.0, 456.0)) }

    #[test]
    fn self_view_sits_in_its_corner_8_from_the_edges() {
        let a = area();
        assert_eq!(pip_rect(a, false, false, false), Rect::from_min_size(pos2(800.0 - 182.0 - 8.0, 496.0 - 102.0 - 8.0), PIP), "bottom right at first");
        assert_eq!(pip_rect(a, true, true, false).min, pos2(8.0, 48.0));
        assert_eq!(pip_rect(a, false, true, true).min, pos2(800.0 - 182.0 - 45.0, 43.0), "full screen leaves room for its button");
    }

    #[test]
    fn a_dropped_self_view_snaps_to_the_nearest_corner() {
        let a = area();
        assert_eq!(nearest_corner(a, pos2(100.0, 100.0)), (true, true));
        assert_eq!(nearest_corner(a, pos2(700.0, 100.0)), (false, true));
        assert_eq!(nearest_corner(a, pos2(100.0, 450.0)), (true, false));
        assert_eq!(nearest_corner(a, pos2(700.0, 450.0)), (false, false));
    }

    #[test]
    fn arrow_keys_move_the_self_view() {
        assert_eq!(corner_after_key(Key::ArrowLeft, (false, false)), (true, false));
        assert_eq!(corner_after_key(Key::ArrowUp, (true, false)), (true, true));
        assert_eq!(corner_after_key(Key::ArrowRight, (true, true)), (false, true));
        assert_eq!(corner_after_key(Key::ArrowDown, (false, true)), (false, false));
        assert_eq!(corner_after_key(Key::A, (true, true)), (true, true));
    }

    #[test]
    fn mirror_flips_the_self_view_only() {
        let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        assert_eq!(mirror_uv(full, true), Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)));
        assert_eq!(mirror_uv(full, false), full);
        let crop = Rect::from_min_max(pos2(0.1, 0.0), pos2(0.9, 1.0));
        assert_eq!(mirror_uv(crop, true), Rect::from_min_max(pos2(0.9, 0.0), pos2(0.1, 1.0)), "the crop stays centred");
        assert_eq!((video_tip(true), video_tip(false)), (tr("Stop video"), tr("Start video")));
    }

    #[test]
    fn the_saved_camera_is_used_while_it_is_plugged_in() {
        let d = |n: &str| CamDev { id: n.into(), name: n.into() };
        let devs = vec![d("Integrated Camera"), d("Logitech BRIO")];
        assert_eq!(pick_camera(&devs, "Logitech BRIO").map(|c| c.name.as_str()), Some("Logitech BRIO"));
        assert_eq!(pick_camera(&devs, "unplugged").map(|c| c.name.as_str()), Some("Integrated Camera"));
        assert!(pick_camera(&[], "x").is_none());
    }

    #[test]
    fn the_self_view_is_cropped_to_fill_its_tile() {
        let uv = crate::ui::call_win::fill_uv(Vec2::new(640.0, 480.0), PIP); // 4:3 camera in a 16:9 tile: top and bottom go
        assert_eq!((uv.min.x, uv.max.x), (0.0, 1.0));
        assert!(uv.min.y > 0.1 && (uv.min.y + uv.max.y - 1.0).abs() < 1e-6, "centred");
        let wide = crate::ui::call_win::fill_uv(Vec2::new(1920.0, 800.0), PIP);
        assert!(wide.min.x > 0.0 && (wide.min.y, wide.max.y) == (0.0, 1.0));
    }
}
