//! The separate call window: opening, actions, remembered position and its content.
use super::*;

/// The person strip next to a received share (the official film strip: `pipMinWidth: 160 + 8`).
const STRIP_W: f32 = 168.0;
/// The bar above a received share (`shareBarHeight: 32`).
const SHARE_BAR_H: f32 = 32.0;

/// `pic` scaled to fit into `room`, aspect kept (never cropped; scaled up too, like the official stage).
pub(crate) fn fit_size(pic: Vec2, room: Vec2) -> Vec2 {
    if pic.x <= 0.0 || pic.y <= 0.0 || room.x <= 0.0 || room.y <= 0.0 { return Vec2::ZERO; }
    let s = (room.x / pic.x).min(room.y / pic.y);
    pic * s
}

/// The part of `pic` that fills `room` (cropped to the room's aspect, centred) as texture coordinates.
pub(crate) fn fill_uv(pic: Vec2, room: Vec2) -> Rect {
    if pic.x <= 0.0 || pic.y <= 0.0 || room.x <= 0.0 || room.y <= 0.0 { return Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)); }
    let (ip, rp) = (pic.x / pic.y, room.x / room.y);
    if ip > rp { let u = rp / ip; Rect::from_min_max(pos2((1.0 - u) / 2.0, 0.0), pos2((1.0 + u) / 2.0, 1.0)) }
    else { let v = ip / rp; Rect::from_min_max(pos2(0.0, (1.0 - v) / 2.0), pos2(1.0, (1.0 + v) / 2.0)) }
}

/// The end button is the red "End call" pill from this control-bar width on, a round button below it.
const END_PILL_FROM: f32 = 600.0;

impl App {
    /// "Separate windows" is on (setting, default true) and the backend can create OS windows.
    pub(crate) fn separate_call_windows(&self, ctx: &egui::Context) -> bool {
        self.settings["call_separate_windows"].as_bool().unwrap_or(true) && !ctx.embed_viewports()
    }

    /// Nothing is written to the user's settings from offline sample / screenshot runs.
    pub(super) fn persist_ok(&self) -> bool { !self.call.sample && self.shot.is_none() }

    /// Ctrl+Shift+X ("ShowHideCallingDock", shortcuts/commands.rs): shows or hides the calling dock. Returns false when this
    /// window setup has no dock (separate windows off, or a backend without OS windows); the caller falls back to "Calling".
    pub(crate) fn toggle_calling_dock(&mut self, ctx: &egui::Context) -> bool {
        if !self.separate_call_windows(ctx) { return false; }
        self.call.win.toggle_dock();
        true
    }
    /// Tray "Show calling dock": shows the dock (never hides it); false like [`Self::toggle_calling_dock`].
    pub(crate) fn show_calling_dock(&mut self, ctx: &egui::Context) -> bool {
        if !self.separate_call_windows(ctx) { return false; }
        let w = &mut self.call.win;
        w.dock_open = true; w.dock_suppress = false;
        true
    }

    /// `WEBOX_SAMPLE_CALL=incoming|ringing|connected|dock|share|roster|video|video-menu|video-ringing|incoming-video` (offline
    /// `--sample` only: needs no signed-in client): shows the call windows with made-up data. No sound, no microphone, no
    /// camera, nothing is sent, nothing is saved.
    pub(crate) fn sample_call_hook(&mut self, ctx: &egui::Context, now: f64) {
        if self.call.win.sample_done || self.client.is_some() { return; }
        let Ok(k) = std::env::var("WEBOX_SAMPLE_CALL") else { return };
        self.call.win.sample_done = true;
        self.call.sample = true;
        match k.as_str() {
            "dock" => { self.preview_call("connected", now); self.call.win.dock_open = true; }
            // the other side shares its screen (a made-up picture): the stage layout
            "share" => { self.preview_call("connected", now); self.call.remote_share = Some("Max Muster".into()); cloud::share_rx::show_sample(1280, 720); }
            // the call statistics panel with made-up figures (stats.rs)
            "stats" => { self.preview_call("connected", now); self.sample_stats(); }
            "incoming" | "ringing" | "connected" => self.preview_call(&k, now),
            // a video call: the other side's picture, our self view (made-up pictures); with the video menu open; my
            // outgoing video call still ringing; a ringing call answered with video ("Answer calls with my video on")
            "video" | "video-menu" => {
                self.preview_call("connected", now); self.call.video.wanted = true; super::super::call::video::show_sample();
                if k == "video-menu" { super::super::menu::menu_open(ctx, "call-video"); }
            }
            "video-ringing" => { self.preview_call("ringing", now); self.call.video.wanted = true; super::super::call::video::show_sample(); }
            "incoming-video" => { self.preview_call("incoming", now); self.call.video_ui.sample_answer_video = true; }
            // a group call with the participants panel open (made-up people)
            "roster" => { self.preview_call("connected", now); self.call.roster = super::roster::sample_roster(); self.call.win.roster.open = true; }
            _ => {}
        }
    }

    /// Per-frame driver: toast window while ringing, call window while a call is on, dock when wanted.
    pub(crate) fn call_windows(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        // Call windows, the dock and the incoming-call toast use the call theme: the selected theme, or its dark variant while
        // Erscheinungsbild "Immer dunkles Thema verwenden" is ticked. Design plugins are not applied there.
        let _ = t;
        let ct = self.call_theme(ctx);
        let t = &ct;
        let v = self.call_view(ctx, now);
        let incoming = v.as_ref().is_some_and(|v| v.state == CallState::Incoming);
        let live = v.as_ref().is_some_and(|v| !v.ended);
        let email = v.as_ref().map(|v| v.email.clone()).unwrap_or_default();
        let mut act = CallActions::default();
        self.call_stats_frame(live && !incoming);
        {
            let w = &mut self.call.win;
            if !incoming { w.toast_closed = None; w.toast_pos = None; w.toast_hwnd = None; }
            if v.is_none() { w.call_b = None; w.dock_suppress = false; w.call_saved = None; w.roster = Default::default(); }
        }
        if v.is_none() { self.call.roster = Default::default(); }
        if let Some(v) = &v {
            if incoming {
                act = self.incoming_toast_window(ctx, t, now, v);
            } else {
                let a = self.call_window(ctx, t, now, v);
                if a.dismiss { self.call.cur = None; }
                act.hang |= a.hang; act.mute |= a.mute; act.answer |= a.answer; act.video |= a.video;
            }
        }
        // ---- dock: toggled by the user / Ctrl+Shift+X, or automatically during a call ----
        let root_min = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let root_focus = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let any_focus = root_focus || self.call.win.focus_call || self.call.win.focus_dock;
        let auto = live && !incoming && !self.call.win.dock_suppress && super::dock::dock_auto(&self.settings, root_min, any_focus);
        self.call.win.dock_shown = self.call.win.dock_open || auto;
        if self.call.win.dock_shown {
            let a = self.dock_window(ctx, t, now, v.as_ref());
            act.hang |= a.hang; act.mute |= a.mute; act.answer |= a.answer;
        } else { self.call.win.dock_b = None; self.call.win.dock_mode = DockMode::Normal; self.call.win.focus_dock = false; }
        self.finish_call_actions(ctx, act, &email);
    }

    pub(super) fn finish_call_actions(&mut self, ctx: &egui::Context, mut a: CallActions, email: &str) {
        if a.toggle_ring { self.call.ring_muted = !self.call.ring_muted; }
        if a.message { self.open_chat_with_email(ctx, email, false); a.hang = true; } // "Text back": decline, then open the chat (nothing is sent)
        if a.contact { self.open_chat_with_email(ctx, email, true); }
        if self.call.sample { // offline demo: the buttons act on the made-up call only
            if a.hang { self.call.cur = None; }
            if a.answer {
                if let Some(c) = &mut self.call.cur { c.state = CallState::Connected; }
                if self.answer_with_video() { self.call.video.wanted = true; super::super::call::video::show_sample(); }
            }
            if a.mute { self.call.muted = !self.call.muted; }
            if a.video { self.toggle_video(); }
            return;
        }
        self.apply_call_actions(a);
    }

    /// Opens the direct chat with `email` (or its profile tab = the "contact card") in the main window and raises it.
    pub(super) fn open_chat_with_email(&mut self, ctx: &egui::Context, email: &str, profile: bool) {
        let id = self.spaces.iter().find(|s| s.direct && !email.is_empty() && s.participants.iter().any(|p| p.email.eq_ignore_ascii_case(email))).map(|s| s.id.clone());
        if let Some(id) = id { self.open_space(id); if profile { self.conv_tab = ConvTab::Profile; } }
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
    }

    /// `--screenshot` for a secondary viewport: its own PNG next to the main one, `<name>-<viewport>.png`.
    pub(crate) fn viewport_shot(&self, vctx: &egui::Context, name: &str) {
        let Some(path) = &self.shot else { return };
        // eframe does not deliver this event for secondary viewports; scripts/capture_windows.ps1 captures them instead.
        if self.frames == 100 { vctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())); }
        let img = vctx.input(|i| i.raw.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
        if let Some(img) = img {
            let px: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let out = path.with_file_name(format!("{stem}-{name}.png"));
            if let Some(buf) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, px) { let _ = buf.save(out); }
        }
    }

    // Call window.
    pub(super) fn call_window(&mut self, ctx: &egui::Context, t: &Th, now: f64, v: &CallView) -> CallActions {
        let mut actions = CallActions::default();
        let wa = work_area(ctx);
        let size_default = Vec2::new(INTERSTITIAL.x, INTERSTITIAL.y + TITLE_BAR_H); // pre-join page 788x560 below the 40 px title bar
        if self.call.win.call_b.is_none() {
            let b = match rect_setting(&self.settings, "call_win_rect") {
                // Remembered position / size, if still on the screen.
                Some([x, y, w, h]) if x > wa.left() - w + 80.0 && x < wa.right() - 80.0 && y >= wa.top() - 4.0 && y < wa.bottom() - 80.0 => (pos2(x, y), Vec2::new(w, h)),
                _ => (wa.center() - size_default / 2.0, size_default),
            };
            self.call.win.call_b = Some(b);
        }
        let (pos, size) = self.call.win.call_b.unwrap_or((pos2(100.0, 100.0), size_default));
        let mut b = egui::ViewportBuilder::default().with_title(tr(CALL_TITLE)).with_decorations(false).with_inner_size(size).with_min_inner_size(CALL_MIN)
            .with_position(pos).with_taskbar(true);
        if self.shot.is_some() { b = b.with_active(false); }
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("webox-call-window"), b, |vctx, class| {
            if class != egui::ViewportClass::Immediate { return; }
            actions = self.call_window_content(vctx, t, now, v);
            self.remember_rect(vctx, true);
            self.viewport_shot(vctx, "call");
        });
        self.round_once(1, tr(CALL_TITLE));
        actions
    }

    /// DWM corner rounding for a freshly created borderless window (a few frames after creation: the window must exist).
    pub(super) fn round_once(&mut self, bit: u8, title: &str) {
        let w = &mut self.call.win;
        if w.rounded & bit == 0 { w.rounded |= bit; round_by_title(title); }
    }

    /// Remembers position / size of the call window or dock in the settings (not in sample / screenshot runs).
    pub(super) fn remember_rect(&mut self, vctx: &egui::Context, call: bool) {
        if !self.persist_ok() { return; }
        let (outer, inner, maxed, mini, time) = vctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect, i.viewport().maximized.unwrap_or(false), i.viewport().minimized.unwrap_or(false), i.time));
        let down = crate::ui::chrome::pointer_down(vctx);
        let full = vctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if maxed || mini || down || full { return; }
        let key = if call { "call_win_rect" } else { "dock_rect" };
        let r = match (outer, inner) {
            (Some(o), Some(inn)) => [o.min.x.round(), o.min.y.round(), inn.width().round(), inn.height().round()],
            // Wayland: no window position. Keep the previous one, remember only the size (screen_rect is in zoomed points).
            _ if crate::ui::chrome::wayland() => {
                let s = inner.map_or_else(|| vctx.screen_rect().size() * vctx.zoom_factor(), |r| r.size());
                let [x, y, ..] = rect_setting(&self.settings, key).unwrap_or([0.0, 0.0, 0.0, 0.0]);
                [x, y, s.x.round(), s.y.round()]
            }
            _ => return,
        };
        let slot = if call { &mut self.call.win.call_saved } else { &mut self.call.win.dock_saved };
        if *slot == Some(r) || (outer.is_none() && time - self.call.win.save_t < 0.5) { return; } // Wayland: no release event to wait for, so rate-limit instead
        let first = slot.is_none();
        *slot = Some(r);
        if first { return; } // the first reading is the window as created
        self.call.win.save_t = time;
        self.settings[key] = json!(r);
        system::save_settings(&self.cfg, &self.settings);
    }

    /// Windows-style caption buttons (40 px wide each, glyph 10 px; hover = gray-80, close hover = red-70 as in the main
    /// window's header). Returns (minimise, maximise / restore, close).
    pub(super) fn win_buttons(ui: &mut egui::Ui, t: &Th, right_top: egui::Pos2, h: f32, maxed: bool) -> (bool, bool, bool) {
        let mut out = [false; 3];
        for (i, id) in ["min", "max", "close"].into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(right_top.x - WIN_BTN_W * (3 - i) as f32, right_top.y), Vec2::new(WIN_BTN_W, h));
            let resp = ui.interact(r, ui.id().with(("win-btn", id)), Sense::click());
            if resp.hovered() { ui.painter().rect_filled(r, 0.0, if i == 2 { hex(0xA12512) } else { t.selected }); }
            if resp.has_focus() { ui.painter().rect_stroke(r.shrink(2.0), 2.0, Stroke::new(2.0_f32, t.focus), egui::StrokeKind::Inside); }
            let (p, c, s) = (ui.painter(), r.center(), Stroke::new(1.0_f32, t.text1));
            match i {
                0 => { p.line_segment([c - Vec2::new(5.0, 0.0), c + Vec2::new(5.0, 0.0)], s); }
                1 if maxed => {
                    p.rect_stroke(Rect::from_center_size(c + Vec2::new(-1.0, 1.0), Vec2::splat(8.0)), 0.0, s, egui::StrokeKind::Inside);
                    p.line_segment([c + Vec2::new(-1.0, -3.0), c + Vec2::new(4.0, -3.0)], s);
                    p.line_segment([c + Vec2::new(4.0, -3.0), c + Vec2::new(4.0, 2.0)], s);
                }
                1 => { p.rect_stroke(Rect::from_center_size(c, Vec2::splat(10.0)), 0.0, s, egui::StrokeKind::Inside); }
                _ => { let d = 5.0; p.line_segment([c - Vec2::splat(d), c + Vec2::splat(d)], s); p.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], s); }
            }
            out[i] = resp.on_hover_text([tr("Minimize"), if maxed { tr("Restore") } else { tr("Maximize") }, tr("Close window")][i]).clicked();
        }
        (out[0], out[1], out[2])
    }

    /// The received share as the stage of the call window: a 32 high bar "<Name> is sharing the screen" on top, the picture
    /// fitted into the rest (aspect kept, nothing cropped), a strip of `STRIP_W` on the right for the person. Returns the
    /// strip. Draws nothing per frame that isn't there: a new picture is uploaded once (`remote_share_picture`).
    fn share_stage(&mut self, ui: &mut egui::Ui, vctx: &egui::Context, t: &Th, body: Rect, v: &CallView) -> Rect {
        let strip = Rect::from_min_max(pos2(body.right() - STRIP_W, body.top()), body.right_bottom());
        let bar = Rect::from_min_size(body.left_top(), Vec2::new(body.width() - STRIP_W, SHARE_BAR_H));
        let stage = Rect::from_min_max(bar.left_bottom(), pos2(strip.left(), body.bottom())).shrink(4.0);
        let p = ui.painter();
        p.rect_filled(bar, 0.0, t.win_b);
        let who = self.call.remote_share.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| v.name.clone());
        crate::ui::chrome::fit_text(p, bar.left() + 12.0, bar.center().y, &crate::trq!("%1 is sharing the screen", who), semibold(13.0), t.text1, bar.width() - 24.0);
        p.rect_filled(stage, 4.0, Color32::BLACK);
        match self.remote_share_picture(vctx) {
            Some((id, size)) => {
                let r = Rect::from_center_size(stage.center(), fit_size(size, stage.size()));
                ui.painter().image(id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => { ui.painter().text(stage.center(), egui::Align2::CENTER_CENTER, tr("Loading share…"), FontId::proportional(14.0), t.text2); }
        }
        strip
    }

    pub(super) fn call_window_content(&mut self, vctx: &egui::Context, t: &Th, now: f64, v: &CallView) -> CallActions {
        let mut a = CallActions::default();
        let (maxed, focused) = vctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().focused.unwrap_or(false)));
        self.call.win.focus_call = focused;
        if vctx.input(|i| i.viewport().close_requested()) { vctx.send_viewport_cmd(egui::ViewportCommand::CancelClose); if v.ended { a.dismiss = true } else { a.hang = true } }
        if crate::ui::chrome::pointer_down(vctx) || vctx.input(|i| i.pointer.delta() != Vec2::ZERO || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. }))) { self.call.win.last_move = now; }
        let connected = v.state == CallState::Connected;
        let mut share = false;
        let autohide = self.settings["call_autohide_controls"].as_bool().unwrap_or(true);
        let fullscreen = self.call_full_screen(vctx);
        self.call_window_shortcuts(vctx);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(vctx, |ui| {
            let full = ui.max_rect();
            // theme-gradient-primary-0 (call window background / title bar gradient).
            let (g0, g1) = (t.win, t.win_b);
            vquad(ui.painter(), full, g0, g1);
            let top = if maxed { MAX_OVERFLOW } else { 0.0 };
            let right_extra = BAR_SIDE + if maxed { MAX_OVERFLOW } else { 0.0 }; // extra right margin
            // Title bar (40 px).
            let bar = Rect::from_min_size(full.min + Vec2::new(0.0, top), Vec2::new(full.width(), TITLE_BAR_H));
            let drag = ui.interact(bar, ui.id().with("titlebar"), Sense::click_and_drag());
            if drag.drag_started() { crate::ui::chrome::note_os_grab(vctx); vctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
            if drag.double_clicked() { vctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed)); }
            crate::ui::chrome::fit_text(ui.painter(), bar.left() + 8.0 + 8.0, bar.center().y, &v.name, semibold(14.0), t.text1, bar.width() - 5.0 * WIN_BTN_W - 40.0);
            let (mn, mx, cl) = Self::win_buttons(ui, t, bar.right_top(), TITLE_BAR_H, maxed);
            self.call_title_buttons(ui, t, bar, connected && !v.ended, fullscreen);
            if mn { vctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }
            if mx { vctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed)); }
            if cl { if v.ended { a.dismiss = true } else { a.hang = true } } // closing the call window ends the call
            // Content: interstitial (ringing screen for 1:1 calls) or the in-call view.
            let body = Rect::from_min_max(bar.left_bottom(), full.right_bottom() - Vec2::new(right_extra - BAR_SIDE, 0.0));
            // The participants panel takes the right side while open.
            let body = if connected && !v.ended && self.call.win.roster.open && body.width() > super::roster::PANEL_W + 200.0 {
                let panel = Rect::from_min_max(pos2(body.right() - super::roster::PANEL_W, body.top()), pos2(body.right(), full.bottom() - BAR_H - BAR_GAP));
                self.roster_panel(ui, vctx, t, panel, now);
                Rect::from_min_max(body.min, pos2(panel.left(), body.bottom()))
            } else { body };
            // The other side shares: their screen is the stage, the person moves to a strip on the right (the official
            // MeetingView: share on the left stage, film strip at least 160 + 8 wide, a 32 high share bar).
            let stage = connected && !v.ended && self.call.remote_share.is_some();
            // The self view sits on the stage, above the control bar.
            let full_screen = vctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            let pip_area = Rect::from_min_max(body.min, pos2(body.right(), full.bottom() - BAR_H - BAR_GAP));
            let body = if stage { self.share_stage(ui, vctx, t, body, v) } else { body };
            // The other side's video fills the stage (aspect kept, black bars), their name on a tag at the bottom left.
            let remote = if connected && !v.ended && !stage { self.remote_video_picture(vctx) } else { None };
            if let Some((id, size)) = remote {
                ui.painter().rect_filled(body, 0.0, Color32::BLACK);
                ui.painter().image(id, Rect::from_center_size(body.center(), fit_size(size, body.size())), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                let label = format!("{} · {}", v.name, state_text(&v.state, v.secs));
                let g = ui.painter().layout_no_wrap(label, semibold(13.0), Color32::WHITE);
                let tag = Rect::from_min_size(pos2(body.left() + 8.0, pip_area.bottom() - 8.0 - 28.0), Vec2::new((g.size().x + 20.0).min(body.width() - 16.0), 28.0));
                ui.painter().rect_filled(tag, 4.0, Color32::from_black_alpha(150)); // overlayFade-meetings-background
                ui.painter().with_clip_rect(tag).galley(pos2(tag.left() + 10.0, tag.center().y - g.size().y / 2.0), g, Color32::WHITE);
                if self.call.mute.peer_muted {
                    self.icons.paint(ui, "microphone-muted-regular", Rect::from_center_size(pos2(tag.right() + 16.0, tag.center().y), Vec2::splat(18.0)), t.badge);
                }
            }
            let cx = body.center().x;
            let cy = body.center().y - if stage { body.height() / 2.0 - 90.0 } else if connected { 36.0 } else { 20.0 };
            let av = if stage { 72.0 } else if connected { 128.0 } else { 112.0 };
            if remote.is_none() {
                let ar = Rect::from_center_size(pos2(cx, cy - av / 2.0 - 4.0), Vec2::splat(av));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(ar));
                avatar_photo(&mut c, &mut self.icons, t, &v.name, av, false, None, None, v.photo);
                if connected && !v.ended { self.peer_muted_badge(ui, t, ar); }
            }
            let name_y = cy + av / 2.0 + 24.0;
            if remote.is_none() {
                if stage {
                    crate::ui::chrome::fit_text(ui.painter(), body.left() + 8.0, name_y, &v.name, semibold(14.0), t.text1, body.width() - 16.0);
                } else {
                    ui.painter().text(pos2(cx, name_y), egui::Align2::CENTER_CENTER, &v.name, semibold(22.0), t.text1);
                }
                ui.painter().text(pos2(cx, name_y + 28.0), egui::Align2::CENTER_CENTER, state_text(&v.state, v.secs), FontId::proportional(14.0), t.text2);
            }
            if v.ended { return; }
            if !connected {
                // Ringing screen: mute | video | hang up (the pre-join page's controls for a 1:1 call).
                let y = name_y + 28.0 + 64.0;
                let muted = self.call.muted;
                if round_at(ui, &mut self.icons, "mute", pos2(cx - 60.0, y), 48.0, if muted { "microphone-muted-regular" } else { "microphone-regular" }, if muted { t.badge } else { t.selected }, Color32::WHITE, t.focus)
                    .on_hover_text(self.mute_tip()).clicked() { a.mute = true; }
                let von = self.call.video.wanted;
                if round_at(ui, &mut self.icons, "video", pos2(cx, y), 48.0, if von { "camera-regular" } else { "camera-muted-regular" }, if von { t.selected } else { t.badge }, Color32::WHITE, t.focus)
                    .on_hover_text(super::super::call::video::video_tip(von)).clicked() { a.video = true; }
                if round_at(ui, &mut self.icons, "hang", pos2(cx + 60.0, y), 48.0, "handset-down-slash-regular", t.badge, Color32::WHITE, t.focus).on_hover_text(tr("Hang up")).clicked() { a.hang = true; }
                self.self_view(ui, t, pip_area, full_screen);
                return;
            }
            self.self_view(ui, t, pip_area, full_screen);
            // Microphone level (capture-only engine, only while on screen; not over the other side's video).
            if remote.is_none() {
                let w = if stage { (body.width() - 24.0).max(40.0) } else { 220.0 };
                let lr = Rect::from_center_size(pos2(cx, name_y + 28.0 + 24.0), Vec2::new(w, 8.0));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(lr));
                self.level_bar(&mut c, t, now, w);
            }
            // Floating control bar (overlay: 56 high, 0.8 opacity, 4 side margin, 8 bottom gap, auto-hide).
            let hover_bar = ui.rect_contains_pointer(Rect::from_min_max(pos2(full.left(), full.bottom() - 120.0), full.right_bottom()));
            let menu_on = ["call-devices", "call-mic", "call-spk", "call-video"].iter().any(|k| super::super::menu::menu_is_open(vctx, k));
            let visible = !autohide || self.shot.is_some() || self.call.sample || hover_bar || menu_on || now - self.call.win.last_move < AUTOHIDE_SECS;
            if visible {
                let inset = if maxed { MAX_BAR_INSET } else { 0.0 };
                let br = Rect::from_min_size(pos2(full.left() + BAR_SIDE, full.bottom() - inset - BAR_GAP - BAR_H), Vec2::new((full.width() - BAR_SIDE - right_extra).max(0.0), BAR_H));
                let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(br));
                bu.set_opacity(BAR_OPACITY);
                bu.painter().rect_filled(br, 0.0, t.win_b); // theme-gradient-primary-1-background
                let y = br.center().y;
                let x0 = br.left() + 16.0; // mute, video (64 wide), then 48 apart: devices, keypad, share, participants, hand
                let muted = self.call.muted;
                if round_at(&mut bu, &mut self.icons, "bar-mute", pos2(x0 + 20.0, y), 40.0, if muted { "microphone-muted-regular" } else { "microphone-regular" }, if muted { t.badge } else { t.selected }, Color32::WHITE, t.focus)
                    .on_hover_text(self.mute_tip()).clicked() { a.mute = true; }
                // Video ("Start video" / "Stop video"), its arrow opens the camera and self-view menu ("Video options").
                let vopen = super::super::menu::menu_is_open(vctx, "call-video");
                let (vt, vm) = self.video_button(&mut bu, t, Rect::from_min_size(pos2(x0 + 48.0, y - 20.0), Vec2::new(64.0, 40.0)), vopen);
                if vt.clicked() { a.video = true; }
                if vm.clicked() { menu_toggle(vctx, "call-video"); }
                let x0 = x0 + 72.0;
                let open = ["call-devices", "call-mic", "call-spk"].iter().any(|k| super::super::menu::menu_is_open(vctx, k));
                let dev = round_at(&mut bu, &mut self.icons, "bar-dev", pos2(x0 + 20.0 + 48.0, y), 40.0, "speaker-regular", if open { t.accent } else { t.selected }, Color32::WHITE, t.focus).on_hover_text(tr("Audio devices"));
                if dev.clicked() { menu_toggle(vctx, "call-devices"); }
                // Keypad ("Keyboard" / "Dialpad"): tones go into the call as DTMF (locus sendDtmf).
                let kp_on = self.call.mc.keypad;
                if round_at(&mut bu, &mut self.icons, "bar-keypad", pos2(x0 + 20.0 + 96.0, y), 40.0, "dialpad-regular", if kp_on { t.accent } else { t.selected }, Color32::WHITE, t.focus)
                    .on_hover_text(tr("Dialpad")).clicked() { self.call.mc.keypad = !kp_on; }
                // Share ("Share"): the picker, then the share runs in the call (cloud/locus/share.rs); again = stop.
                let (sharing, pending) = (self.call.sharing, self.call.share_pending);
                let tip = if sharing { tr("Stop sharing") } else if pending { tr("Starting share… (click to cancel)") } else { tr("Share content") };
                if round_at(&mut bu, &mut self.icons, "bar-share", pos2(x0 + 20.0 + 144.0, y), 40.0, "share-screen-regular", if sharing { t.green } else if pending { t.accent } else { t.selected }, Color32::WHITE, t.focus)
                    .on_hover_text(tip).clicked() { share = true; }
                // Participants panel, raise hand (group calls).
                self.roster_button(&mut bu, t, pos2(x0 + 20.0 + 192.0, y));
                self.hand_button(&mut bu, t, pos2(x0 + 20.0 + 240.0, y), now);
                // Red pill on the right (left / right margin 16); a narrow bar has the round end button only.
                let wide = br.width() >= END_PILL_FROM;
                let pill = Rect::from_min_max(pos2(br.right() - 16.0 - if wide { 132.0 } else { 40.0 }, y - 20.0), pos2(br.right() - 16.0, y + 20.0));
                let pr = bu.interact(pill, bu.id().with("bar-end"), Sense::click());
                bu.painter().rect_filled(pill, 20.0, if pr.hovered() { t.badge.gamma_multiply(0.85) } else { t.badge });
                let ic = if wide { pill.left_center() + Vec2::new(24.0, 0.0) } else { pill.center() };
                self.icons.paint(&bu, "handset-down-slash-regular", Rect::from_center_size(ic, Vec2::splat(20.0)), Color32::WHITE);
                if wide { bu.painter().text(pill.left_center() + Vec2::new(42.0, 0.0), egui::Align2::LEFT_CENTER, tr("End call"), semibold(13.0), Color32::WHITE); }
                if pr.has_focus() { bu.painter().rect_stroke(pill.expand(1.5), 21.5, Stroke::new(2.0_f32, t.focus), egui::StrokeKind::Inside); }
                if pr.on_hover_text(tr("End call")).clicked() { a.hang = true; }
                self.device_menu(vctx, t, now, dev.rect);
                self.video_menu(vctx, t, now, vm.rect);
            }
            if self.call.mc.keypad {
                let area = Rect::from_min_max(bar.left_bottom(), pos2(full.right(), full.bottom() - BAR_H - BAR_GAP));
                self.call_keypad_ui(ui, t, area);
            }
            self.call_stats_panel(ui, t, Rect::from_min_max(bar.left_bottom(), full.right_bottom()));
        });
        crate::ui::chrome::edge_overlay(vctx); // Linux: no server-side border to resize by (no-op while maximised)
        vctx.request_repaint_after(std::time::Duration::from_millis(200));
        if share { if self.call.sample { self.call.sharing = !self.call.sharing; } else { self.share_button(now); } }
        a
    }
}
