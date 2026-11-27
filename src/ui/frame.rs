use crate::*;

/// How long the text cursor keeps blinking after the last input.
const CURSOR_BLINK_SECS: f64 = 10.0;

impl eframe::App for App {
    // The main window's backdrop. The popups (toasts, tray menu, incoming-call toast) are immediate viewports, which eframe
    // always clears to transparent; their see-through padding comes from tray::see_through, not from this colour.
    // Opaque: on X11 the main window may have an ARGB visual (main.rs), and a compositor would show through any alpha < 1.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::from_rgb(12, 12, 12).to_normalized_gamma_f32()
    }

    // --screenshot only: scripted pointer / wheel events (ui/shot_input.rs).
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.shot.is_some() {
            // Only scripted input: the real pointer (on X11 one pointer for every window of a parallel screenshot run) and
            // keys would hover or press whatever lies under them, so the picture would depend on where the mouse rests.
            raw.events.retain(|e| matches!(e, egui::Event::Screenshot { .. } | egui::Event::WindowFocused(_)));
            raw.hovered_files.clear();
            raw.dropped_files.clear();
            super::shot_input::inject(self.frames + 1, raw);
        }
        super::autoscroll::begin_frame(ctx, raw); // middle-click autoscroll: ends on Esc / click, frame time
        #[cfg(target_os = "macos")]
        crate::platform::macos::appkit::inject(raw); // Edit menu picked with the mouse (copy, paste, undo, ...)
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // WEBOX_PROBE=1: every 5 s on stderr, the frames drawn, texture memory and which code asked for the repaints
        // (performance checks against the official Webex app).
        static PROBE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *PROBE.get_or_init(|| std::env::var_os("WEBOX_PROBE").is_some()) {
            static LAST: std::sync::Mutex<(f64, u32, Vec<String>)> = std::sync::Mutex::new((0.0, 0, Vec::new()));
            let t = ctx.input(|i| i.time);
            let mut l = LAST.lock().unwrap();
            l.1 += 1;
            for c in ctx.repaint_causes() { l.2.push(format!("{}:{} {}", c.file, c.line, c.reason)); }
            // WEBOX_PROBE=2: every frame, the focused widget and the visible layers (what changes between idle frames).
            if std::env::var_os("WEBOX_PROBE").is_some_and(|v| v == "2") {
                let mut layers: Vec<String> = ctx.memory(|m| m.areas().visible_layer_ids().into_iter().map(|l| format!("{:?}/{:?}", l.order, l.id)).collect());
                layers.sort();
                eprintln!("FRAME t={t:.2} focus={:?} layers={layers:?}", ctx.memory(|m| m.focused()));
            }
            if t - l.0 > 5.0 {
                let mut m: std::collections::BTreeMap<String, u32> = Default::default();
                for k in l.2.drain(..) { *m.entry(k).or_default() += 1; }
                let mut v: Vec<_> = m.into_iter().collect(); v.sort_by(|a, b| b.1.cmp(&a.1));
                let (n, bytes) = ctx.tex_manager().read().allocated().fold((0usize, 0usize), |(n, b), (_, m)| (n + 1, b + m.size[0] * m.size[1] * m.bytes_per_pixel));
                let heap = crate::platform::memprobe::heap().map(|(live, _)| format!(" heap_mb={:.1}", live as f64 / 1048576.0)).unwrap_or_default();
                let atlas = ctx.fonts(|f| f.font_image_size());
                let heap = format!("{heap} input_idle_s={:.0} font_atlas={}x{}", t - self.last_input, atlas[0], atlas[1]);
                let resps = crate::platform::memprobe::RESPS.swap(0, std::sync::atomic::Ordering::Relaxed);
                eprintln!("PROBE {:.0}s frames={} cloud_answers={resps} textures={} tex_mb={}{heap} rows={} {} causes={:?}", t - l.0, l.1, n, bytes >> 20,
                    super::messages::row_cache_len(), self.photos.mem_report(), &v[..v.len().min(12)]);
                l.0 = t; l.1 = 0;
            }
        }
        let _ = &frame;
        super::set_ui_ctx(ctx); // background results wake the window (ui::wake)
        crate::i18n::sync_from_settings(ctx, &self.settings); // live language switching (settings key `language`)
        // The app's native window handle, to parent the embedded WebView2 control (Windows only).
        #[cfg(windows)]
        if self.hwnd.is_none() {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = frame.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() { self.hwnd = Some(w.hwnd.get()); }
            }
        }
        // Linux: X11 window id (0 on Wayland), the session type and the scale factor for the toast / tray helpers (src/platform/linux/).
        #[cfg(target_os = "linux")]
        { let h = crate::platform::linux::note_window(frame, ctx); if self.hwnd.is_none() { self.hwnd = h; } }
        let now = ctx.input(|i| i.time);
        // The presence keepalive's input clock where the system has none (cloud/keepalive.rs; Windows uses GetLastInputInfo).
        // A pointer "move" to where the pointer already was is no input (Windows repeats WM_MOUSEMOVE for a resting cursor).
        let last_pos = self.last_pointer;
        if let Some(p) = ctx.input(|i| i.events.iter().rev().find_map(|e| if let egui::Event::PointerMoved(p) = e { Some(*p) } else { None })) { self.last_pointer = Some(p); }
        if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::PointerButton { .. } | egui::Event::MouseWheel { .. } | egui::Event::Text(_))
            || matches!(e, egui::Event::PointerMoved(p) if Some(*p) != last_pos))) {
            cloud::keepalive::note_input(crate::ui::chrome::now_ms());
            self.last_input = now;
        }
        // --signin [email]: once the window handle exists, start the native sign-in automatically
        // (same path as clicking Next on the login screen). The remembered idbroker session in our
        // own webview profile can carry it all the way to the code without a password.
        if let (true, Some(_)) = (self.autosignin.is_some(), self.hwnd) {
            if self.need_login && self.webview.is_none() && self.pending.is_none() {
                let arg = self.autosignin.take().unwrap_or_default();
                let email = if arg.contains('@') { arg } else { self.email_input.trim().to_string() };
                if email.contains('@') { self.kick_signin(&email); }
            }
        }
        // Wayland: a minimized window only comes back through xdg-activation (platform/linux/activation.rs); a compositor without
        // it gets the window shown instead.
        #[cfg(target_os = "linux")]
        let can_restore = crate::platform::linux::session() != crate::platform::linux::Session::Wayland || crate::platform::linux::activation::available();
        #[cfg(not(target_os = "linux"))]
        let can_restore = true;
        if self.minimize { self.minimize = false; if can_restore { ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); } }
        self.poll(now);
        for a in std::mem::take(&mut self.plugins.acts) { let _ = self.dispatch(a, crate::app::bus::Origin::Plugin, ctx); }
        // Hidden in the tray or minimised: the pictures' textures are freed (decoded again when the window is back).
        let away = crate::platform::tray::HIDDEN.load(std::sync::atomic::Ordering::Relaxed) || ctx.input(|i| i.viewport().minimized == Some(true));
        crate::assets::photos::AWAY.store(away, std::sync::atomic::Ordering::Relaxed);
        if away && !self.textures_released { self.photos.release_textures(ctx); }
        self.textures_released = away;
        super::sample_scale::tour(self, ctx); // WEBOX_SAMPLE_TOUR (sample runs only)
        self.typing_frame(ctx); // my typing → the service (throttled), typers' overlay animation (ui/typing.rs)
        self.poll_webview();
        if self.webview.is_some() { ctx.request_repaint(); }
        crate::assets::photos::GIFS_ONLY_FOCUSED.store(self.settings["gifs_pause_unfocused"].as_bool().unwrap_or(true), std::sync::atomic::Ordering::Relaxed);
        // Darstellungsdichte (ui/theme/density.rs): read every frame, so a change applies at once
        if self.frames == 1 && self.shot.is_some() { if let Ok(v) = std::env::var("WEBOX_SHOT_DENSITY") { self.settings["density"] = json!(v); }
            if let Ok(v) = std::env::var("WEBOX_SHOT_COMPACT_VIEW") { self.settings["compact_view"] = json!(v == "1"); } }
        crate::ui::theme::density::set_from(&self.settings);
        crate::ui::widgets::note_font_atlas(ctx);
        let t = self.apply_theme(ctx);
        // Every blink of the text cursor redraws the whole window (egui has no partial redraws): it blinks while you work and
        // stays on, without redraws, from 10 s after the last key press or mouse move, so a window left open is really idle.
        let blink = self.shot.is_none() && now - self.last_input < CURSOR_BLINK_SECS;
        if ctx.style().visuals.text_cursor.blink != blink { ctx.style_mut(|s| s.visuals.text_cursor.blink = blink); }
        // Screenshots: no text cursor. egui draws it only while the window has the OS focus, and parallel screenshot runs
        // (scripts/parity.ps1) take the focus from each other, so a picture must not depend on it.
        if self.shot.is_some() && ctx.style().visuals.text_cursor.stroke.width != 0.0 { ctx.style_mut(|s| s.visuals.text_cursor.stroke.width = 0.0); }
        if blink { ctx.request_repaint_after(std::time::Duration::from_secs_f64(CURSOR_BLINK_SECS - (now - self.last_input) + 0.01)); }
        self.shortcuts_frame(ctx); // registry dispatch, global hotkeys
        self.type_focus_frame(ctx); // typing with nothing focused goes to the composer (ui/type_focus.rs)
        self.settings_window(ctx, &t, now); // collects file-dialog results, screenshot hook (ui/settings/)
        self.appwide_frame(ctx); // settings that act app-wide (ui/settings_effects.rs)
        self.updates_frame(ctx, now); // daily update check, restart after an update (ui/update_dialog.rs)

        self.poll_login();
        // Renew the sign-in while running.
        if self.client.is_some() && self.pending.is_none() && self.renewal_due() { self.try_refresh(); }
        // While the saved sign-in is renewed (not need_login) the splash is shown instead of the login screen.
        let load = self.load_state(ctx, now);
        if self.need_login {
            if !self.login_sized { self.login_sized = true; ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(LOGIN_SIZE.into())); }
            egui::CentralPanel::default().frame(panel(t.win)).show(ctx, |ui| self.login_screen(ui, &t));
        } else {
        if self.login_sized { self.login_sized = false; ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(MAIN_SIZE.into())); }
        // The window's own background: theme-gradient-primary (dark #1A1A1A -> #0F0F0F, light #F0F0F0 -> #FFFFFF).
        vquad(&ctx.layer_painter(egui::LayerId::background()), ctx.screen_rect(), t.win, t.win_b);
        // a design plugin's wash over the window (behind rail and header; ui/theme/decor.rs)
        if let Some(w) = crate::ui::theme::decor::get().and_then(|d| d.window.clone()) {
            crate::ui::theme::decor::paint_wash(&ctx.layer_painter(egui::LayerId::background()), ctx.screen_rect(), 0.0, &w);
        }
        super::chrome::edge_strips(ctx); // the 6 pt margin (resize border) while not maximised, see chrome::EDGE_INSET
        self.top_bar(ctx, &t, now); // splash covers everything below the header
        self.connectivity_banner(ctx, &t, load.banner.as_deref());
        let splash_area = ctx.screen_rect(); // the splash covers the ENTIRE window, header and rail included (a Webox choice)
        self.rail(ctx, &t);
        self.rail_edge_button(ctx, &t);
        crate::ui::theme::decor::chrome(ctx); // a design plugin's flag band on the rail / stripe under the header
        match self.tab {
            Tab::Messaging => self.messaging(ctx, &t, now),
            // A team's spaces keep the messaging layout; every other page uses the page frame (ui/page_kit.rs).
            Tab::Teams if self.team_sel.is_some() => self.messaging(ctx, &t, now),
            Tab::Settings => self.settings_tab(ctx, &t, now), // in-window Settings page (ui/settings/)
            _ => self.pages_frame(ctx, &t, now),
        }
        self.splash_overlay(ctx, &t, splash_area, load.splash.as_deref());
        }

        // Global search popup + contact-card popup.
        if !self.need_login { self.nav_card_popup(ctx, &t, now); self.global_search_popup(ctx, &t, now); }
        self.call_overlay(ctx, &t, now);
        self.share_picker_frame(ctx, &t); // the share dialog while it is open (ui/share_picker/)
        self.notify_update(ctx, &t, now); // toast windows, tray, taskbar
        self.decor_effects(ctx); // design-plugin effects (burst, ripple, June sparkle), drawn last, no input (ui/theme/fx.rs)

        self.shortcut_search(ctx, &t); self.shortcut_dialogs(ctx, &t); // Ctrl+/ popup, sign-out confirm
        if !self.need_login { self.appwide_dialogs(ctx, &t, now); } // profile / help / device / update dialogs (ui/app_dialogs.rs)

        for (i, (text, _)) in self.toasts.iter().enumerate() {
            egui::Area::new(egui::Id::new(("toast", i))).anchor(egui::Align2::RIGHT_BOTTOM, [-20.0, -110.0 - 62.0 * i as f32]).show(ctx, |ui| {
                super::menu::toast_frame(&t).show(ui, |ui| {
                    ui.set_max_width(340.0);
                    ui.label(RichText::new(text).color(t.toast_text));
                });
            });
        }

        // --screenshot: draw ~120 frames, save the picture, quit.
        if self.shot.is_some() {
            self.frames += 1;
            // WEBOX_SHOT_CREATE=1 renders the "Create a space" screen (for layout comparison; nothing is created).
            if self.frames == 2 && std::env::var("WEBOX_SHOT_CREATE").is_ok() { self.new_space = Some(String::new()); }
            // WEBOX_SHOT_CALL=ringing|connected|incoming|menu renders the call UI with made-up data (display only, no call).
            if self.frames == 2 { if let Ok(k) = std::env::var("WEBOX_SHOT_CALL") { self.preview_call(&k, now); } }
            // WEBOX_SHOT_OPEN=<space id> opens that (sample) space, e.g. s2 = a direct chat.
            if self.frames == 30 { if let Ok(id) = std::env::var("WEBOX_SHOT_OPEN") { self.open_space(id); } }
            // WEBOX_SHOT_DRAFT=<space id> gives that (sample) space an unsent draft (the list row's draft icon; not saved).
            if self.frames == 2 { if let Ok(id) = std::env::var("WEBOX_SHOT_DRAFT") { self.drafts.insert(id, "Draft".into()); } }
            // WEBOX_SHOT_THEME=light|dark|<theme id, e.g. MomentumJadeLight> renders that theme (not saved).
            if self.frames == 1 { if let Ok(v) = std::env::var("WEBOX_SHOT_THEME") { self.settings["theme"] = json!(v); } }
            // WEBOX_SHOT_MENU=gear|status|filters|more|row opens that menu (row needs the first list row's dots; opens for s1).
            if self.frames == 40 {
                if let Ok(m) = std::env::var("WEBOX_SHOT_MENU") {
                    if m == "row" { ctx.data_mut(|d| d.insert_temp(egui::Id::new("row-menu-target"), ("s1".to_string(), Rect::from_center_size(pos2(540.0, 251.0), Vec2::splat(24.0))))); }
                    super::menu::menu_open(ctx, &m);
                    if m == "status-dnd" { super::menu::menu_open(ctx, "status"); }
                    if m == "dropdown" { super::menu::menu_open(ctx, "status-dialog"); super::menu::menu_open(ctx, "dropdown-status-clear-after"); }
                    if m == "status-dialog" { ctx.data_mut(|d| d.insert_temp(egui::Id::new("status-dialog-text"), String::new())); }
                }
            }
            // WEBOX_SHOT_SHORTCUTS=search|settings: the Ctrl+/ search popup, or Settings > Keyboard shortcuts. Add :edit (settings:edit) for key capture mode.
            if self.frames == 2 { if let Ok(m) = std::env::var("WEBOX_SHOT_SHORTCUTS") { match m.split(':').next().unwrap_or("") { "search" => self.show_shortcuts = true, _ => { self.tab = Tab::Settings; ctx.data_mut(|d| d.insert_temp(egui::Id::new("settings_cat"), 7usize)); } } } }
            // WEBOX_SHOT_EMOJI=1 opens the emoji picker (with sample recents, not saved) for layout comparison.
            if self.frames == 2 && std::env::var("WEBOX_SHOT_EMOJI").is_ok() { self.settings["emoji_recents"] = json!(["🤑", "😭", "🥹"]); self.emoji_picker_toggle(ctx); }
            // WEBOX_SHOT_TAB=calls|voicemail|meetings|contacts|teams|whiteboards opens that page; WEBOX_SHOT_SEL=<n> selects its n-th row
            // (teams: the team), WEBOX_SHOT_DIAL=1 shows the keypad, WEBOX_SHOT_NEWTEAM=1 the "Create a team" form, WEBOX_SHOT_CARD=1 the contact card.
            if self.frames == 2 {
                if let Ok(k) = std::env::var("WEBOX_SHOT_TAB") {
                    self.set_tab(match k.as_str() { "calls" => Tab::Calling, "voicemail" => Tab::Voicemail, "meetings" => Tab::Meetings, "contacts" => Tab::Contacts, "teams" => Tab::Teams, "whiteboards" => Tab::Whiteboards, _ => self.tab });
                }
            }
            if self.frames == 10 {
                let mut s = self.pst(ctx);
                let sel = std::env::var("WEBOX_SHOT_SEL").ok().and_then(|v| v.parse::<usize>().ok());
                if let Some(n) = sel {
                    s.call_sel = Some(n); s.vm_sel = Some(n); s.meet_sel = Some(n);
                    s.contact_sel = self.people.get(n).map(|p| p.id.clone()); s.team_open = self.teams.get(n).map(|x| x.id.clone());
                }
                if std::env::var("WEBOX_SHOT_DIAL").is_ok() { s.dial_open = true; }
                if std::env::var("WEBOX_SHOT_NEWTEAM").is_ok() { s.new_team = Some((String::new(), String::new())); }
                if std::env::var("WEBOX_SHOT_CARD").is_ok() { s.contact_card = Some(("jonas@example.com".into(), "Jonas Weber".into(), Rect::from_center_size(pos2(700.0, 200.0), Vec2::splat(4.0)))); }
                self.set_pst(ctx, s);
            }
            // WEBOX_SHOT_TABS=profile|people|content|links|whiteboards|meetings|apps|find|pins, WEBOX_SHOT_SEARCH=<query>|1, WEBOX_SHOT_CARD=1
            if self.frames == 40 {
                if let Ok(k) = std::env::var("WEBOX_SHOT_TABS") {
                    match k.as_str() {
                        "profile" => self.conv_tab = ConvTab::Profile, "people" => self.conv_tab = ConvTab::People, "meetings" => self.conv_tab = ConvTab::Meetings,
                        "content" | "links" | "whiteboards" => { self.conv_tab = ConvTab::Content; if let Some(id) = self.selected.clone() { super::space_tabs::set_content(ctx, &id, match k.as_str() { "links" => 1, "whiteboards" => 2, _ => 0 }); } }
                        "apps" => self.conv_tab = ConvTab::Apps,
                        "find" => { if let Some(id) = self.selected.clone() { super::space_tabs::find_toggle(ctx, &id); } }
                        "pins" => { if let Some(id) = self.selected.clone() { super::space_tabs::pins_toggle(ctx, &id); } }
                        _ => {}
                    }
                }
                if let Ok(q) = std::env::var("WEBOX_SHOT_SEARCH") { super::global_search::shot_open(ctx, if q == "1" { "" } else { &q }); }
                if std::env::var("WEBOX_SHOT_CARD").is_ok() { if let Some(p) = self.people.first().cloned() { Self::contact_card_open(ctx, &p); } }
            }
            self.spaces_shot(ctx, now); // WEBOX_SHOT_SPACES_* (ui/spaces_ext/)
            // WEBOX_SHOT_WIDE=1 renders the expanded (labelled) rail without touching the saved setting.
            if self.frames == 2 && std::env::var("WEBOX_SHOT_WIDE").is_ok() { self.rail_wide = true; }
            // WEBOX_SHOT_ZOOM=1.25 renders as on a 125 % display (on a 100 % one): device pixels = points × 1.25
            if self.frames == 1 { if let Some(z) = std::env::var("WEBOX_SHOT_ZOOM").ok().and_then(|v| v.parse::<f32>().ok()) { ctx.set_zoom_factor(z.clamp(0.5, 3.0)); } }
            ctx.request_repaint();
            // WEBOX_SHOT_FRAMES=<n>: take the picture at frame n instead of 120 (longer scroll / trace scripts).
            if self.frames == std::env::var("WEBOX_SHOT_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(120) { ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())); }
            let img = ctx.input(|i| i.raw.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
            if let (Some(img), Some(path)) = (img, self.shot.clone()) {
                let px: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
                if let Some(buf) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, px) { let _ = buf.save(&path); }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if let Some(end) = self.toasts.iter().map(|t| t.1).reduce(f64::min) {
            // In-window toasts: one frame when the first of them expires. Cloud answers wake the window themselves (cloud::start)
            // and the clock-driven work runs once a minute (notify_update).
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((end - now).max(0.0) + 0.02));
        }
        super::autoscroll::paint(ctx); // the autoscroll anchor and cursor, above everything
    }
}

