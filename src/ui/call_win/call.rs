//! The separate call window: opening, actions, remembered position and its content.
use super::*;

impl App {
    /// "Separate windows" is on (setting, default true) and the backend can create OS windows.
    pub(crate) fn separate_call_windows(&self, ctx: &egui::Context) -> bool {
        self.settings["call_separate_windows"].as_bool().unwrap_or(true) && !ctx.embed_viewports()
    }

    /// Nothing is written to the user's settings from offline sample / screenshot runs.
    pub(super) fn persist_ok(&self) -> bool { !self.call.sample && self.shot.is_none() }

    /// Ctrl+Shift+X (bound in the shortcut registry): shows or hides the calling dock.
    #[allow(dead_code)] // called by the shortcut registry (Ctrl+Shift+X)
    pub(crate) fn toggle_calling_dock(&mut self) {
        let w = &mut self.call.win;
        if w.dock_shown { w.dock_open = false; w.dock_suppress = true; } else { w.dock_open = true; w.dock_suppress = false; }
    }

    /// `WEBOX_SAMPLE_CALL=incoming|ringing|connected|dock` (offline `--sample` only: needs no signed-in client): shows the
    /// call windows with made-up data. No sound, no microphone, nothing is sent, nothing is saved.
    pub(crate) fn sample_call_hook(&mut self, _ctx: &egui::Context, now: f64) {
        if self.call.win.sample_done || self.client.is_some() { return; }
        let Ok(k) = std::env::var("WEBOX_SAMPLE_CALL") else { return };
        self.call.win.sample_done = true;
        self.call.sample = true;
        match k.as_str() {
            "dock" => { self.preview_call("connected", now); self.call.win.dock_open = true; }
            "incoming" | "ringing" | "connected" => self.preview_call(&k, now),
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
        {
            let w = &mut self.call.win;
            if !incoming { w.toast_closed = None; w.toast_pos = None; w.toast_hwnd = None; }
            if v.is_none() { w.call_b = None; w.dock_suppress = false; w.call_saved = None; }
        }
        if let Some(v) = &v {
            if incoming {
                act = self.incoming_toast_window(ctx, t, now, v);
            } else {
                let a = self.call_window(ctx, t, now, v);
                if a.dismiss { self.call.cur = None; }
                act.hang |= a.hang; act.mute |= a.mute; act.answer |= a.answer;
            }
        }
        // ---- dock: toggled by the user / Ctrl+Shift+X, or automatically during a call ----
        let root_min = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let root_focus = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let any_focus = root_focus || self.call.win.focus_call || self.call.win.focus_dock;
        let covered = self.settings["dock_auto_covered"].as_bool().unwrap_or(false);
        let auto = live && !incoming && !self.call.win.dock_suppress && (root_min || (covered && !any_focus));
        self.call.win.dock_shown = self.call.win.dock_open || auto;
        if self.call.win.dock_shown {
            let a = self.dock_window(ctx, t, now, v.as_ref());
            act.hang |= a.hang; act.mute |= a.mute; act.answer |= a.answer;
        } else { self.call.win.dock_b = None; self.call.win.dock_mode = DockMode::Normal; self.call.win.focus_dock = false; }
        self.finish_call_actions(ctx, act, &email);
    }

    pub(super) fn finish_call_actions(&mut self, ctx: &egui::Context, mut a: CallActions, email: &str) {
        if a.toggle_ring { self.call.ring_muted = !self.call.ring_muted; }
        if a.message { self.open_chat_with_email(ctx, email, false); a.hang = true; } // "Zurückschreiben": decline, then open the chat (nothing is sent)
        if a.contact { self.open_chat_with_email(ctx, email, true); }
        if self.call.sample { // offline demo: the buttons act on the made-up call only
            if a.hang { self.call.cur = None; }
            if a.answer { if let Some(c) = &mut self.call.cur { c.state = CallState::Connected; } }
            if a.mute { self.call.muted = !self.call.muted; }
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
        let mut b = egui::ViewportBuilder::default().with_title(CALL_TITLE).with_decorations(false).with_inner_size(size).with_min_inner_size(CALL_MIN)
            .with_position(pos).with_taskbar(true);
        if self.shot.is_some() { b = b.with_active(false); }
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("webox-call-window"), b, |vctx, class| {
            if class != egui::ViewportClass::Immediate { return; }
            actions = self.call_window_content(vctx, t, now, v);
            self.remember_rect(vctx, true);
            self.viewport_shot(vctx, "call");
        });
        self.round_once(1, CALL_TITLE);
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
        let (outer, inner, maxed, mini, down) = vctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect, i.viewport().maximized.unwrap_or(false), i.viewport().minimized.unwrap_or(false), i.pointer.any_down()));
        let (Some(o), Some(inn)) = (outer, inner) else { return };
        if maxed || mini || down { return; }
        let r = [o.min.x.round(), o.min.y.round(), inn.width().round(), inn.height().round()];
        let slot = if call { &mut self.call.win.call_saved } else { &mut self.call.win.dock_saved };
        if *slot == Some(r) { return; }
        let first = slot.is_none();
        *slot = Some(r);
        if first { return; } // the first reading is the window as created
        self.settings[if call { "call_win_rect" } else { "dock_rect" }] = json!(r);
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
            if resp.has_focus() { ui.painter().rect_stroke(r.shrink(2.0), 2.0, Stroke::new(2.0, t.focus), egui::StrokeKind::Inside); }
            let (p, c, s) = (ui.painter(), r.center(), Stroke::new(1.0, t.text1));
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
            out[i] = resp.on_hover_text(["Minimieren", if maxed { "Wiederherstellen" } else { "Maximieren" }, "Fenster schließen"][i]).clicked();
        }
        (out[0], out[1], out[2])
    }

    pub(super) fn call_window_content(&mut self, vctx: &egui::Context, t: &Th, now: f64, v: &CallView) -> CallActions {
        let mut a = CallActions::default();
        let (maxed, focused) = vctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().focused.unwrap_or(false)));
        self.call.win.focus_call = focused;
        if vctx.input(|i| i.viewport().close_requested()) { vctx.send_viewport_cmd(egui::ViewportCommand::CancelClose); if v.ended { a.dismiss = true } else { a.hang = true } }
        if vctx.input(|i| i.pointer.delta() != Vec2::ZERO || i.pointer.any_down() || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. }))) { self.call.win.last_move = now; }
        let connected = v.state == CallState::Connected;
        let autohide = self.settings["call_autohide_controls"].as_bool().unwrap_or(true);
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
            if drag.drag_started() { vctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
            if drag.double_clicked() { vctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed)); }
            crate::ui::chrome::fit_text(ui.painter(), bar.left() + 8.0 + 8.0, bar.center().y, &v.name, semibold(14.0), t.text1, bar.width() - 3.0 * WIN_BTN_W - 40.0);
            let (mn, mx, cl) = Self::win_buttons(ui, t, bar.right_top(), TITLE_BAR_H, maxed);
            if mn { vctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }
            if mx { vctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed)); }
            if cl { if v.ended { a.dismiss = true } else { a.hang = true } } // closing the call window ends the call
            // Content: interstitial (ringing screen for 1:1 calls) or the in-call view.
            let body = Rect::from_min_max(bar.left_bottom(), full.right_bottom() - Vec2::new(right_extra - BAR_SIDE, 0.0));
            let cx = body.center().x;
            let cy = body.center().y - if connected { 36.0 } else { 20.0 };
            let av = if connected { 128.0 } else { 112.0 };
            {
                let ar = Rect::from_center_size(pos2(cx, cy - av / 2.0 - 4.0), Vec2::splat(av));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(ar));
                avatar_photo(&mut c, &mut self.icons, t, &v.name, av, false, None, None, v.photo);
            }
            let name_y = cy + av / 2.0 + 24.0;
            ui.painter().text(pos2(cx, name_y), egui::Align2::CENTER_CENTER, &v.name, semibold(22.0), t.text1);
            ui.painter().text(pos2(cx, name_y + 28.0), egui::Align2::CENTER_CENTER, state_text(&v.state, v.secs), FontId::proportional(14.0), t.text2);
            if v.ended { return; }
            if !connected {
                // Ringing screen: mute | hang up (the pre-join page's controls for a 1:1 call).
                let y = name_y + 28.0 + 64.0;
                let muted = self.call.muted;
                if round_at(ui, &mut self.icons, "mute", pos2(cx - 36.0, y), 48.0, if muted { "microphone-muted-regular" } else { "microphone-regular" }, if muted { t.badge } else { t.selected }, Color32::WHITE, t.focus)
                    .on_hover_text(if muted { "Stummschaltung aufheben" } else { "Stummschalten" }).clicked() { a.mute = true; }
                if round_at(ui, &mut self.icons, "hang", pos2(cx + 36.0, y), 48.0, "handset-down-slash-regular", t.badge, Color32::WHITE, t.focus).on_hover_text("Auflegen").clicked() { a.hang = true; }
                return;
            }
            // Microphone level (capture-only engine, only while on screen).
            {
                let lr = Rect::from_center_size(pos2(cx, name_y + 28.0 + 24.0), Vec2::new(220.0, 8.0));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(lr));
                self.level_bar(&mut c, t, now, 220.0);
            }
            // Floating control bar (overlay: 56 high, 0.8 opacity, 4 side margin, 8 bottom gap, auto-hide).
            let hover_bar = ui.rect_contains_pointer(Rect::from_min_max(pos2(full.left(), full.bottom() - 120.0), full.right_bottom()));
            let menu_on = ["call-devices", "call-mic", "call-spk"].iter().any(|k| super::super::menu::menu_is_open(vctx, k));
            let visible = !autohide || self.shot.is_some() || self.call.sample || hover_bar || menu_on || now - self.call.win.last_move < AUTOHIDE_SECS;
            if visible {
                let inset = if maxed { MAX_BAR_INSET } else { 0.0 };
                let br = Rect::from_min_size(pos2(full.left() + BAR_SIDE, full.bottom() - inset - BAR_GAP - BAR_H), Vec2::new((full.width() - BAR_SIDE - right_extra).max(0.0), BAR_H));
                let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(br));
                bu.set_opacity(BAR_OPACITY);
                bu.painter().rect_filled(br, 0.0, t.win_b); // theme-gradient-primary-1-background
                let y = br.center().y;
                let muted = self.call.muted;
                if round_at(&mut bu, &mut self.icons, "bar-mute", pos2(br.left() + 16.0 + 20.0, y), 40.0, if muted { "microphone-muted-regular" } else { "microphone-regular" }, if muted { t.badge } else { t.selected }, Color32::WHITE, t.focus)
                    .on_hover_text(if muted { "Stummschaltung aufheben" } else { "Stummschalten" }).clicked() { a.mute = true; }
                let open = ["call-devices", "call-mic", "call-spk"].iter().any(|k| super::super::menu::menu_is_open(vctx, k));
                let dev = round_at(&mut bu, &mut self.icons, "bar-dev", pos2(br.left() + 16.0 + 20.0 + 48.0, y), 40.0, "speaker-regular", if open { t.accent } else { t.selected }, Color32::WHITE, t.focus).on_hover_text("Audiogeräte");
                if dev.clicked() { menu_toggle(vctx, "call-devices"); }
                // Keypad ("Tastatur" / "Tastenfeld"): tones go into the call as DTMF (locus sendDtmf).
                let kp_on = self.call.mc.keypad;
                if round_at(&mut bu, &mut self.icons, "bar-keypad", pos2(br.left() + 16.0 + 20.0 + 96.0, y), 40.0, "dialpad-regular", if kp_on { t.accent } else { t.selected }, Color32::WHITE, t.focus)
                    .on_hover_text("Tastenfeld").clicked() { self.call.mc.keypad = !kp_on; }
                // Red pill on the right (left / right margin 16).
                let pill = Rect::from_min_max(pos2(br.right() - 16.0 - 132.0, y - 20.0), pos2(br.right() - 16.0, y + 20.0));
                let pr = bu.interact(pill, bu.id().with("bar-end"), Sense::click());
                bu.painter().rect_filled(pill, 20.0, if pr.hovered() { t.badge.gamma_multiply(0.85) } else { t.badge });
                self.icons.paint(&bu, "handset-down-slash-regular", Rect::from_center_size(pill.left_center() + Vec2::new(24.0, 0.0), Vec2::splat(20.0)), Color32::WHITE);
                bu.painter().text(pill.left_center() + Vec2::new(42.0, 0.0), egui::Align2::LEFT_CENTER, "Anruf beenden", semibold(13.0), Color32::WHITE);
                if pr.has_focus() { bu.painter().rect_stroke(pill.expand(1.5), 21.5, Stroke::new(2.0, t.focus), egui::StrokeKind::Inside); }
                if pr.on_hover_text("Anruf beenden").clicked() { a.hang = true; }
                self.device_menu(vctx, t, now, dev.rect);
            }
            if self.call.mc.keypad {
                let area = Rect::from_min_max(bar.left_bottom(), pos2(full.right(), full.bottom() - BAR_H - BAR_GAP));
                self.call_keypad_ui(ui, t, area);
            }
        });
        vctx.request_repaint_after(std::time::Duration::from_millis(200));
        a
    }
}
