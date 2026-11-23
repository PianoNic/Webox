#![allow(clippy::too_many_arguments)]
use crate::*;
use crate::cloud::auth;

impl App {
    /// The first screen (pt): navy gradient with a red glow top-left and a blue glow top-centre, the mark with the
    /// app-configuration menu, three 259×32 pills 48 apart (the first white with a 2-pt #64B4FA focus ring), the terms line and
    /// the round "…" button.
    pub(crate) fn login_screen(&mut self, ui: &mut egui::Ui, _t: &Th) {
        let full = ui.max_rect();
        let p = ui.painter().clone();
        // The login background (2048×1188), drawn cropped-to-fill from the top-left (red glow in the corner, blue glow across the top).
        if let Some((id, [w, h])) = icons::login_bg_dark().and_then(|b| self.icons.png(ui, "login_bg_dark", b)) {
            let (ia, ra) = (w as f32 / h.max(1) as f32, full.width() / full.height());
            let uv = if ia > ra { Rect::from_min_max(pos2(0.0, 0.0), pos2(ra / ia, 1.0)) }
                     else { Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, ia / ra)) };
            p.image(id, full, uv, Color32::WHITE);
        } else {
            p.rect_filled(full, 0.0, hex(0x07111F));
        }
        let cx = full.center().x;
        let white = hex(0xF2F3F4);
        let ring = hex(0x64B4FA);

        // Title strip (custom decorations): mark + name left, minimize / close right.
        let tb = Rect::from_min_size(full.min, Vec2::new(full.width(), 32.0));
        let tresp = ui.interact(tb, ui.id().with("login-titlebar"), Sense::click_and_drag());
        if tresp.drag_started() { crate::ui::chrome::note_os_grab(ui.ctx()); ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag); }
        if tresp.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
        self.icons.logo(ui, Rect::from_center_size(pos2(full.left() + 16.0, full.top() + 16.0), Vec2::splat(15.0)));
        p.text(pos2(full.left() + 27.0, full.top() + 16.0), egui::Align2::LEFT_CENTER, "Webox", FontId::proportional(12.5), white);
        let ctl = |ui: &mut egui::Ui, id: &str, rect: Rect, hover: Color32, glyph: &dyn Fn(&egui::Painter, Rect)| {
            let resp = ui.interact(rect, egui::Id::new(id), Sense::click());
            if resp.hovered() { ui.painter().rect_filled(rect, 0.0, hover); }
            glyph(&ui.painter(), rect);
            resp.clicked()
        };
        let cr = Rect::from_min_size(pos2(full.right() - 46.0, full.top()), Vec2::new(46.0, 32.0));
        let mr = Rect::from_min_size(pos2(full.right() - 92.0, full.top()), Vec2::new(46.0, 32.0));
        if ctl(ui, "login-close", cr, hex(0xA12512), &|p, r| {
            let c = r.center(); let d = 5.0;
            p.line_segment([c - Vec2::new(d, d), c + Vec2::new(d, d)], Stroke::new(1.0_f32, white));
            p.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], Stroke::new(1.0_f32, white));
        }) { ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close); }
        if ctl(ui, "login-min", mr, alpha(Color32::WHITE, 0.09), &|p, r| {
            p.line_segment([r.center() - Vec2::new(5.0, 0.0), r.center() + Vec2::new(5.0, 0.0)], Stroke::new(1.0_f32, white));
        }) { ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }

        // Mark + name + configuration chevron (a neutral mark: the Webex logo and wordmark are not used).
        let mark = Rect::from_center_size(pos2(cx - 60.0, full.top() + 70.0), Vec2::splat(34.0));
        self.icons.logo(ui, mark);
        let name = p.text(pos2(mark.right() + 8.0, mark.center().y), egui::Align2::LEFT_CENTER, "webox", FontId::proportional(25.0), white);
        let chev = Rect::from_center_size(pos2(name.right() + 22.0, full.top() + 70.0), Vec2::splat(22.0));
        let chev_resp = ui.interact(chev.expand(4.0), ui.id().with("cfg"), Sense::click());
        self.icons.paint(ui, if self.config_open { "arrow-up-regular" } else { "arrow-down-regular" }, chev.shrink(4.0), white);
        if chev_resp.clicked() { self.config_open = !self.config_open }

        let pill = |ui: &mut egui::Ui, y: f32, label: &str, filled: bool, id: &str| -> egui::Response {
            let r = Rect::from_min_size(pos2(cx - 129.5, full.top() + y), Vec2::new(259.0, 32.0));
            let resp = ui.interact(r, ui.id().with(id), Sense::click());
            let p = ui.painter();
            if filled {
                p.rect_filled(r, 16.0, if resp.hovered() { hex(0xDCDDDF) } else { hex(0xF3F3F4) });
                p.rect_stroke(r.expand(2.5), 18.5, Stroke::new(2.0_f32, ring), egui::StrokeKind::Middle);
            } else {
                if resp.hovered() { p.rect_filled(r, 16.0, alpha(Color32::WHITE, 0.08)); }
                p.rect_stroke(r, 16.0, Stroke::new(1.0_f32, hex(0x848A91)), egui::StrokeKind::Inside);
            }
            let col = if filled { hex(0x0C0C0C) } else { white };
            for dx in [0.0, 0.45] { p.text(r.center() + Vec2::new(dx, 0.0), egui::Align2::CENTER_CENTER, label, FontId::proportional(14.5), col); } // drawn twice for a semibold look
            resp
        };

        match self.login_page {
            LoginPage::Start => {
                if pill(ui, 143.0, "Join a meeting", true, "join").clicked() { self.login_page = LoginPage::Join }
                if pill(ui, 191.0, "Sign in", false, "signin").clicked() { self.login_page = LoginPage::Email }
                if pill(ui, 239.0, "Sign up", false, "signup").clicked() { system::open(SIGN_UP_URL) }
            }
            LoginPage::Code => {
                // Native OOB step: the browser showed a one-time code; paste it to finish.
                let waiting = self.pending.is_some();
                let area = Rect::from_min_size(pos2(cx - 129.5, full.top() + 108.0), Vec2::new(259.0, 220.0));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Center)));
                c.label(RichText::new("Finish signing in").size(15.0).color(white));
                c.add_space(6.0);
                c.label(RichText::new("Sign in in your browser, then paste the code Webex shows you here.").size(12.0).color(hex(0xB8B8B8)));
                if waiting {
                    c.add_space(12.0);
                    c.spinner();
                } else {
                    c.add_space(12.0);
                    let r = c.add(egui::TextEdit::singleline(&mut self.code_input).hint_text("Authorization code").desired_width(259.0).margin(egui::Margin::symmetric(14, 8)));
                    let enter = r.lost_focus() && c.input(|i| i.key_pressed(Key::Enter));
                    c.add_space(12.0);
                    let go = c.add(egui::Button::new(RichText::new("Sign in").size(14.5).color(hex(0x0C0C0C))).fill(hex(0xF3F3F4)).corner_radius(16.0).min_size(Vec2::new(259.0, 32.0))).clicked() || enter;
                    if go && !self.code_input.trim().is_empty() {
                        self.login_error.clear();
                        let code = std::mem::take(&mut self.code_input).trim().to_string();
                        let (base, dev) = (self.idbroker_base(), self.oauth_device_id());
                        self.pending = Some(auth::exchange_async(code, self.pkce_verifier.clone(), base, dev));
                    } else if go {
                        self.login_error = "Paste the code from your browser.".into();
                    }
                    c.add_space(8.0);
                    if c.link(RichText::new("Open the sign-in page again").color(ring).size(12.0)).clicked() {
                        let (v, ch) = auth::pkce();
                        self.pkce_verifier = v;
                        system::open(&auth::authorize_url(self.email_input.trim(), &jose::uuid(), &ch, &self.idbroker_base()));
                    }
                    c.add_space(4.0);
                    if c.link(RichText::new("Back").color(ring)).clicked() { self.login_page = LoginPage::Email; self.login_error.clear(); }
                }
            }
            LoginPage::Web => {
                // Embedded sign-in. On Windows the WebView2 control is a child of our own window and sits over the region below this
                // header (same window). On Linux it's a separate WebKitGTK window we own (eframe is winit, not GTK), so here we just show
                // status. The captured code is handled in `poll_webview`, so this arm only draws the frame around it and the escape hatches.
                let back = |app: &mut App| { app.webview = None; app.login_page = LoginPage::Email; app.login_error.clear(); };
                if platform::webview::EMBEDDED {
                    let ppp = ui.ctx().pixels_per_point();
                    let region = Rect::from_min_max(pos2(full.left(), full.top() + 120.0), full.max);
                    if let Some(wv) = &self.webview {
                        wv.set_bounds((region.left() * ppp) as i32, (region.top() * ppp) as i32,
                                      (region.width() * ppp) as i32, (region.height() * ppp) as i32);
                    }
                    // Header strip above the control: Cancel on the left, "open in browser" on the right.
                    if ui.put(Rect::from_min_size(pos2(cx - 200.0, full.top() + 96.0), Vec2::new(90.0, 20.0)),
                              egui::Button::new(RichText::new("← Back").color(ring).size(13.0)).frame(false)).clicked() { back(self); }
                    if ui.put(Rect::from_min_size(pos2(cx + 90.0, full.top() + 96.0), Vec2::new(150.0, 20.0)),
                              egui::Button::new(RichText::new("Open in browser instead").color(ring).size(13.0)).frame(false)).clicked() {
                        self.webview = None;
                        let (v, ch) = auth::pkce();
                        self.pkce_verifier = v;
                        system::open(&auth::authorize_url(self.email_input.trim(), &jose::uuid(), &ch, &self.idbroker_base()));
                        self.login_page = LoginPage::Code;
                    }
                } else {
                    let area = Rect::from_min_size(pos2(cx - 150.0, full.top() + 150.0), Vec2::new(300.0, 180.0));
                    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Center)));
                    c.label(RichText::new("Finishing sign-in").size(15.0).color(white));
                    c.add_space(6.0);
                    c.label(RichText::new("Complete sign-in in the Webex window. It closes on its own when you're done.").size(12.0).color(hex(0xB8B8B8)));
                    c.add_space(14.0);
                    c.spinner();
                    c.add_space(14.0);
                    if c.link(RichText::new("Cancel").color(ring)).clicked() { back(self); }
                }
            }
            LoginPage::Email | LoginPage::Join => {
                let join = self.login_page == LoginPage::Join;
                let area = Rect::from_min_size(pos2(cx - 129.5, full.top() + 118.0), Vec2::new(259.0, 200.0));
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Center)));
                c.label(RichText::new(if join { "Enter the meeting link" } else { "Enter your email address" }).size(15.0).color(white));
                c.add_space(12.0);
                let field = if join { &mut self.meeting_input } else { &mut self.email_input };
                let r = c.add(egui::TextEdit::singleline(field).hint_text(if join { "Meeting link" } else { "Email address" }).desired_width(259.0).margin(egui::Margin::symmetric(14, 8)));
                let enter = r.lost_focus() && c.input(|i| i.key_pressed(Key::Enter));
                c.add_space(14.0);
                let go = c.add(egui::Button::new(RichText::new(if join { "Join" } else { "Next" }).size(14.5).color(hex(0x0C0C0C))).fill(hex(0xF3F3F4)).corner_radius(16.0).min_size(Vec2::new(259.0, 32.0))).clicked() || enter;
                if go && join {
                    let l = self.meeting_input.trim().to_string();
                    if l.starts_with("https://") { platform::nmhost::record(&self.cfg, &l); system::open(&l); self.login_error.clear(); } else { self.login_error = "Paste the whole meeting link (it starts with https://).".into(); }
                } else if go && self.email_input.contains('@') {
                    // Next: the shared native sign-in start (also used by --signin): cluster discovery, PKCE and the embedded webview
                    // (browser + paste fallback).
                    let email = self.email_input.trim().to_string();
                    self.kick_signin(&email);
                } else if go && !join {
                    self.login_error = "Enter your email address (it needs an @).".into();
                }
                c.add_space(8.0);
                if c.link(RichText::new("Back").color(ring)).clicked() { self.login_page = LoginPage::Start; self.login_error.clear(); }
            }
        }
        if !self.login_error.is_empty() {
            let err_rect = Rect::from_center_size(pos2(cx, full.top() + 340.0), Vec2::new(full.width() - 80.0, 54.0));
            ui.put(err_rect, egui::Label::new(RichText::new(&self.login_error).size(13.0).color(hex(0xF55868))).wrap());
        }

        // Terms line (two lines, 46 and 27 points above the bottom) with underlined links.
        let line = |y: f32, parts: &[(&str, Option<&str>)], ui: &mut egui::Ui| {
            let f = FontId::proportional(13.0);
            let w: f32 = parts.iter().map(|(s, _)| ui.fonts(|fo| fo.layout_no_wrap(s.to_string(), f.clone(), white).size().x)).sum();
            let mut x = cx - w / 2.0;
            for (s, link) in parts {
                let g = ui.fonts(|fo| fo.layout_no_wrap(s.to_string(), f.clone(), white));
                let r = Rect::from_min_size(pos2(x, y - g.size().y / 2.0), g.size());
                let col = if link.is_some() { ring } else { white };
                ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, *s, f.clone(), col);
                if let Some(u) = link {
                    ui.painter().hline(r.x_range(), r.bottom() - 1.0, Stroke::new(1.0_f32, ring));
                    if ui.interact(r, ui.id().with(*s), Sense::click()).clicked() { system::open(u) }
                }
                x += g.size().x;
            }
        };
        line(full.bottom() - 46.0, &[("By using WebOx you agree to the ", None), ("Terms of Service", Some(TERMS_URL)), (" and the", None)], ui);
        line(full.bottom() - 27.0, &[("Privacy Statement", Some(PRIVACY_URL)), (".", None)], ui);

        // Round "…" button, bottom right.
        let dots = Rect::from_center_size(pos2(full.right() - 33.0, full.bottom() - 31.0), Vec2::splat(32.0));
        let dresp = ui.interact(dots, ui.id().with("dots"), Sense::click());
        p.circle_stroke(dots.center(), 15.5, Stroke::new(1.0_f32, hex(0x848A91)));
        if dresp.hovered() { p.circle_filled(dots.center(), 15.0, alpha(Color32::WHITE, 0.08)); }
        self.icons.paint(ui, "more-regular", dots.shrink(8.0), white);
        let dresp = dresp.on_hover_text("Look around with sample data");
        if dresp.clicked() {
            self.settings["sample"] = json!(true);
            system::save_settings(&self.cfg, &self.settings);
            self.need_login = false;
        }

        // App-configuration menu (black, #303135 border, selected row #121212, check in blue).
        if self.config_open {
            let menu = Rect::from_min_size(pos2(cx - 176.0, full.top() + 94.0), Vec2::new(318.0, 197.0));
            egui::Area::new(egui::Id::new("cfgmenu")).fixed_pos(menu.min).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
                egui::Frame::default().fill(hex(0x000000)).stroke(Stroke::new(1.0_f32, hex(0x303135))).corner_radius(10.0).inner_margin(egui::Margin { left: 10, right: 10, top: 16, bottom: 14 }).show(ui, |ui| {
                    ui.set_width(298.0);
                    ui.label(RichText::new("  App configuration").size(14.5).strong().color(white));
                    ui.add_space(8.0);
                    let rows = [(false, "Webex", "For individual and enterprise users"), (true, "Webex for Government", "For FedRAMP users")];
                    for (gov, title, sub) in rows {
                        let on = self.settings["gov"].as_bool().unwrap_or(false) == gov;
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(298.0, 48.0), Sense::click());
                        if on || resp.hovered() { ui.painter().rect_filled(r, 8.0, hex(0x121212)); }
                        ui.painter().text(r.left_top() + Vec2::new(11.0, 7.0), egui::Align2::LEFT_TOP, title, FontId::proportional(14.5), white);
                        ui.painter().text(r.left_top() + Vec2::new(11.0, 27.0), egui::Align2::LEFT_TOP, sub, FontId::proportional(12.5), hex(0xB8B8B8));
                        if on { self.icons.paint(ui, "check-regular", Rect::from_center_size(r.right_center() - Vec2::new(22.0, 0.0), Vec2::splat(14.0)), hex(0x418ED3)); }
                        if resp.clicked() {
                            if gov { self.login_error = "Webex for Government isn't in this version.".into(); }
                            self.config_open = false;
                        }
                    }
                    ui.add_space(8.0);
                    if ui.link(RichText::new("  Learn more").size(14.5).color(ring).underline()).clicked() { system::open(LEARN_MORE_URL) }
                });
            });
        }
    }
}
