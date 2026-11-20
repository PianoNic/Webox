//! The calling dock: snapping to a screen edge and its content.
use super::*;

impl App {
    // Calling dock.
    pub(super) fn dock_window(&mut self, ctx: &egui::Context, t: &Th, now: f64, v: Option<&CallView>) -> CallActions {
        let mut actions = CallActions::default();
        let wa = work_area(ctx);
        if self.call.win.dock_b.is_none() {
            let b = match rect_setting(&self.settings, "dock_rect") {
                Some([x, y, w, h]) if x > wa.left() - w + 80.0 && x < wa.right() - 80.0 && y >= wa.top() - 4.0 && y < wa.bottom() - 80.0 =>
                    (pos2(x, y), Vec2::new(w.clamp(DOCK_MIN.x, DOCK_MAX.x), h.clamp(DOCK_MIN.y, DOCK_MAX.y))),
                _ => (pos2(wa.right() - DOCK_DEFAULT.x - 24.0, wa.top() + 48.0), DOCK_DEFAULT), // default spot: top right
            };
            self.call.win.dock_b = Some(b);
            self.call.win.dock_mode = DockMode::Normal;
        }
        let (pos, size) = self.call.win.dock_b.unwrap_or((pos2(100.0, 100.0), DOCK_DEFAULT));
        let thumb = matches!(self.call.win.dock_mode, DockMode::Docked(_));
        let pin = self.settings["dock_keep_in_front"].as_bool().unwrap_or(false);
        let (min, max) = if thumb { (Vec2::splat(DOCK_THUMB), Vec2::splat(DOCK_THUMB)) } else { (DOCK_MIN, DOCK_MAX) };
        let mut b = egui::ViewportBuilder::default().with_title(DOCK_TITLE).with_decorations(false).with_inner_size(size).with_min_inner_size(min).with_max_inner_size(max)
            .with_position(pos).with_taskbar(false).with_window_level(if pin { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal });
        if self.shot.is_some() { b = b.with_active(false); }
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("webox-calling-dock"), b, |vctx, class| {
            if class != egui::ViewportClass::Immediate { return; }
            actions = self.dock_content(vctx, t, now, v, wa);
            self.remember_rect(vctx, false);
            self.viewport_shot(vctx, "dock");
        });
        self.round_once(2, DOCK_TITLE);
        actions
    }

    /// Screen-edge docking: within 20 px of a work-area edge, then a 60x60 thumbnail.
    pub(super) fn dock_snap(wa: Rect, o: Rect) -> Option<DockSide> {
        if o.left() - wa.left() <= DOCK_SNAP { Some(DockSide::Left) }
        else if wa.right() - o.right() <= DOCK_SNAP { Some(DockSide::Right) }
        else if o.top() - wa.top() <= DOCK_SNAP { Some(DockSide::Top) }
        else { None }
    }
    /// Top-left of a window of `size` stuck to `side`, keeping the position along the edge of `o`.
    pub(super) fn dock_place(wa: Rect, o: Rect, side: DockSide, size: Vec2) -> egui::Pos2 {
        let y = o.top().clamp(wa.top(), (wa.bottom() - size.y).max(wa.top()));
        let x = o.left().clamp(wa.left(), (wa.right() - size.x).max(wa.left()));
        match side { DockSide::Left => pos2(wa.left(), y), DockSide::Right => pos2(wa.right() - size.x, y), DockSide::Top => pos2(x, wa.top()) }
    }

    pub(super) fn dock_content(&mut self, vctx: &egui::Context, t: &Th, now: f64, v: Option<&CallView>, wa: Rect) -> CallActions {
        let mut a = CallActions::default();
        let (outer, inner, focused, hovered) = vctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect, i.viewport().focused.unwrap_or(false), i.pointer.hover_pos().is_some()));
        self.call.win.focus_dock = focused;
        if let (Some(o), Some(inn)) = (outer, inner) { self.call.win.dock_actual = Some((o.min, inn.size())); }
        let mut mode = self.call.win.dock_mode;
        // Docking state machine.
        if let Some(o) = outer {
            // Settle detection after an OS move (StartDrag swallows the button release): position unchanged for 0.3 s.
            if self.call.win.dock_dragging {
                let last = self.call.win.dock_last_pos();
                if last.is_none_or(|p| (p - o.min).length() > 0.5) { self.call.win.dock_last_move = now; self.call.win.set_dock_last_pos(o.min); }
                else if now - self.call.win.dock_last_move > 0.3 {
                    self.call.win.dock_dragging = false;
                    let size = inner.map_or(DOCK_DEFAULT, |r| r.size());
                    match (mode, Self::dock_snap(wa, o)) {
                        (DockMode::Normal | DockMode::Hovered(_), Some(side)) => {
                            let p = Self::dock_place(wa, o, side, Vec2::splat(DOCK_THUMB));
                            self.call.win.dock_restore = Some(size);
                            self.call.win.dock_b = Some((p, Vec2::splat(DOCK_THUMB)));
                            mode = DockMode::Docked(side);
                        }
                        (DockMode::Hovered(_), None) => { self.call.win.dock_b = Some((o.min, size)); mode = DockMode::Normal; }
                        _ => {}
                    }
                }
            }
        }
        match mode {
            DockMode::Docked(side) if hovered => {
                let size = self.call.win.dock_restore.unwrap_or(DOCK_DEFAULT);
                let o = outer.unwrap_or(Rect::from_min_size(pos2(wa.left(), wa.top()), Vec2::splat(DOCK_THUMB)));
                let p = Self::dock_place(wa, o, side, size);
                self.call.win.dock_b = Some((p, size));
                self.call.win.dock_hover_out = None;
                mode = DockMode::Hovered(side);
            }
            DockMode::Hovered(side) if !hovered && !self.call.win.dock_dragging && !self.call.win.dock_confirm => {
                let since = *self.call.win.dock_hover_out.get_or_insert(now);
                if now - since > 0.4 {
                    let o = outer.unwrap_or(Rect::from_min_size(pos2(wa.left(), wa.top()), DOCK_DEFAULT));
                    self.call.win.dock_restore = Some(o.size());
                    let p = Self::dock_place(wa, o, side, Vec2::splat(DOCK_THUMB));
                    self.call.win.dock_b = Some((p, Vec2::splat(DOCK_THUMB)));
                    self.call.win.dock_hover_out = None;
                    mode = DockMode::Docked(side);
                }
            }
            DockMode::Hovered(_) if hovered => { self.call.win.dock_hover_out = None; }
            _ => {}
        }
        self.call.win.dock_mode = mode;

        let bg = t.tk("modal-secondary-background"); // #1A1A1A dark / #F7F7F7 light
        let border = t.sep; // modal-primary-border
        let pin = self.settings["dock_keep_in_front"].as_bool().unwrap_or(false);
        if vctx.input(|i| i.viewport().close_requested()) { vctx.send_viewport_cmd(egui::ViewportCommand::CancelClose); self.dock_close_request(); }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(vctx, |ui| {
            let full = ui.max_rect();
            ui.painter().rect_filled(full, 0.0, bg);
            ui.painter().rect_stroke(full, 0.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
            if let DockMode::Docked(_) = mode {
                // 60x60 thumbnail: the caller's avatar (or the handset when idle).
                let ar = Rect::from_center_size(full.center(), Vec2::splat(44.0));
                if let Some(v) = v {
                    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(ar));
                    avatar_photo(&mut c, &mut self.icons, t, &v.name, 44.0, false, None, None, v.photo);
                } else { self.icons.paint(ui, "handset-regular", Rect::from_center_size(full.center(), Vec2::splat(28.0)), t.text1); }
                return;
            }
            // Header: pin | settings | minimise / dock, round S buttons.
            let hh = 44.0;
            let head = Rect::from_min_size(full.min, Vec2::new(full.width(), hh));
            let drag = ui.interact(head, ui.id().with("dock-head"), Sense::click_and_drag());
            if drag.drag_started() { vctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); self.call.win.dock_dragging = true; self.call.win.dock_last_move = now; self.call.win.set_dock_last_pos(outer.map_or(pos2(0.0, 0.0), |o| o.min)); }
            let y = head.center().y;
            let mut x = head.right() - 12.0 - 16.0;
            let side = match mode { DockMode::Hovered(s) => Some(s), _ => None };
            let (min_icon, min_tip) = match side { Some(DockSide::Left) => ("arrow-circle-left-bold", "Andocken"), Some(DockSide::Right) => ("arrow-circle-right-bold", "Andocken"), Some(DockSide::Top) => ("arrow-circle-up-bold", "Andocken"), None => ("minus-bold", "Minimieren") };
            if ghost_at(ui, &mut self.icons, t, "dock-min", pos2(x, y), 32.0, min_icon, t.text1).on_hover_text(min_tip).clicked() {
                if let Some(s) = side {
                    let o = outer.unwrap_or(full);
                    self.call.win.dock_restore = Some(o.size());
                    self.call.win.dock_b = Some((Self::dock_place(wa, o, s, Vec2::splat(DOCK_THUMB)), Vec2::splat(DOCK_THUMB)));
                    self.call.win.dock_mode = DockMode::Docked(s);
                } else { vctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }
            }
            x -= 36.0;
            let sett = ghost_at(ui, &mut self.icons, t, "dock-settings", pos2(x, y), 32.0, "settings-bold", t.text1).on_hover_text("Einstellungen");
            if sett.clicked() { menu_toggle(vctx, "dock-settings"); }
            x -= 36.0;
            if ghost_at(ui, &mut self.icons, t, "dock-pin", pos2(x, y), 32.0, if pin { "pin-muted-bold" } else { "pin-bold" }, t.text1).on_hover_text(if pin { "Lösen" } else { "Anheften" }).clicked() {
                self.settings["dock_keep_in_front"] = json!(!pin);
                if self.persist_ok() { system::save_settings(&self.cfg, &self.settings); }
            }
            let entries = vec![
                Item::new("calls", "Anrufeinstellungen").entry(), Item::new("dock", "Anruf-Dock-Einstellungen").entry(), Entry::Sep,
                Item::new("close", "Anruf-Dock schließen (Strg + Umschalt + X)").entry(),
            ];
            if let Some(id) = webex_menu(vctx, &mut self.icons, t, "dock-settings", sett.rect, Side::Below, Align::Right, true, &entries) {
                match id.as_str() {
                    "close" => self.dock_close_request(),
                    _ => { self.set_tab(Tab::Settings); vctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Minimized(false)); vctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus); }
                }
            }
            ui.painter().line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, border));
            // Body.
            let body = Rect::from_min_max(head.left_bottom() + Vec2::new(0.0, 1.0), full.right_bottom()).shrink(16.0);
            let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Center)));
            match v {
                Some(v) => {
                    bu.add_space(24.0);
                    a = self.call_card_ui(&mut bu, t, now, v);
                }
                _ => {
                    bu.add_space(40.0);
                    bu.label(RichText::new("Keine aktiven Anrufe").font(semibold(16.0)).color(t.text1));
                    bu.add_space(16.0);
                    let hist = self.call.history.clone();
                    for (name, incoming, res) in hist.iter().take(8) {
                        bu.label(RichText::new(format!("{}  {}  {}", if *incoming { "←" } else { "→" }, name, res)).size(13.0).color(t.text2));
                    }
                }
            }
            // "Anruf-Dock schließen?" (dialog 402 wide, border modal-primary-border).
            if self.call.win.dock_confirm {
                ui.painter().rect_filled(full, 0.0, Color32::from_black_alpha(120));
                let w = 402.0_f32.min(full.width() - 24.0);
                egui::Area::new(egui::Id::new("dock-confirm")).order(egui::Order::Foreground).fixed_pos(full.center() - Vec2::new(w / 2.0, 80.0)).show(vctx, |ui| {
                    egui::Frame::default().fill(t.tk("modal-primary-background")).stroke(Stroke::new(1.0, border)).inner_margin(20).show(ui, |ui| {
                        ui.set_width(w - 42.0);
                        ui.label(RichText::new("Anruf-Dock schließen?").font(semibold(16.0)).color(t.text1));
                        ui.add_space(8.0);
                        ui.label(RichText::new("Mit der Tastenkombination Strg+Umschalt+X können Sie es sofort wieder öffnen.").size(14.0).color(t.text2));
                        ui.add_space(12.0);
                        let mut chk = self.call.win.dock_chk;
                        ui.checkbox(&mut chk, "Nicht erneut nachfragen");
                        self.call.win.dock_chk = chk;
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            if ui.button("Schließen").clicked() {
                                if self.call.win.dock_chk { self.settings["dock_dont_ask"] = json!(true); if self.persist_ok() { system::save_settings(&self.cfg, &self.settings); } }
                                self.dock_close_now();
                            }
                            if ui.button("Abbrechen").clicked() { self.call.win.dock_confirm = false; }
                        });
                    });
                });
            }
        });
        vctx.request_repaint_after(std::time::Duration::from_millis(200));
        a
    }

    /// Closing the dock: asks first unless "Nicht erneut nachfragen" was ticked.
    pub(super) fn dock_close_request(&mut self) {
        if self.settings["dock_dont_ask"].as_bool().unwrap_or(false) { self.dock_close_now(); } else { self.call.win.dock_confirm = true; }
    }
    pub(super) fn dock_close_now(&mut self) {
        let w = &mut self.call.win;
        w.dock_confirm = false; w.dock_open = false; w.dock_suppress = true; w.dock_shown = false; w.dock_chk = false;
    }
}
