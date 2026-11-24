//! The app side of notifications: incoming messages, meeting reminders, the tray menu and showing toasts.
use super::*;

impl App {
    pub(super) fn nb(&self, key: &str, default: bool) -> bool { self.settings[key].as_bool().unwrap_or(default) }
    pub(crate) fn dnd(&self) -> bool { self.me.status == "DoNotDisturb" }
    pub(crate) fn window_active(&self) -> bool { self.notify.focused && !self.notify.minimized } // pub(crate): read receipts (actions.rs) too

    /// Brings the main window back from the tray / the taskbar and focuses it.
    pub(crate) fn show_main(&mut self, ctx: &egui::Context) {
        tray::HIDDEN.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.hwnd { tray::restore_from_tray(h); }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        // Wayland ignores all three: only an xdg-activation token brings the window to the front (the panel's, the notification's,
        // the second start's; else one of its own). Without a handed-in token the taskbar entry asks for attention as well.
        #[cfg(target_os = "linux")]
        if crate::platform::linux::session() == crate::platform::linux::Session::Wayland && !crate::platform::linux::activation::activate(self.notify.raise_token.take()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
            self.notify.flashing = true;
        }
    }

    /// Hides the main window into the tray (off-screen without a taskbar button on Windows, so the app keeps updating and can raise toasts).
    pub(super) fn hide_main(&mut self, ctx: &egui::Context) {
        tray::HIDDEN.store(true, std::sync::atomic::Ordering::Relaxed);
        match self.hwnd { Some(h) if cfg!(windows) => tray::hide_to_tray(h), _ => ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false)) }
    }

    /// A message arrived in `space`. Returns true when the main window is not in front, so the caller must not show the in-window toast:
    /// the OS-level toast (when the settings allow it) and the taskbar flash take over. With the window in front nothing changes.
    pub(crate) fn notify_incoming(&mut self, space: &str, m: &Message) -> bool {
        if self.quiet_now() { return true; } // Ruhezeiten: no toast at all (ui/settings_effects.rs)
        if self.window_active() { return false; }
        if self.dnd() { return true; }
        let (title, direct) = self.spaces.iter().find(|s| s.id == space).map(|s| (s.title.clone(), s.direct)).unwrap_or_default();
        let mention = mentions_me(&m.text, &m.html, &self.me.name);
        if self.nb("notif_mentions_only", false) && !direct && !mention { return true; }
        if self.nb("notif_taskbar_flash", true) { self.notify.want_flash = true; }
        if !self.nb("notif_desktop", true) { return true; }
        let preview = self.nb("notif_preview", true);
        let text = if preview { m.text.clone() } else { "Neue Nachricht".into() }; // "Neue Nachricht": Webox's own wording, the pack has no string for the hidden preview
        let file = m.text.starts_with('\u{1F4CE}'); // the data layer renders non-image files as "📎 name"
        let mut t = if mention {
            Toast::new(Kind::Mention, if direct { m.author.clone() } else { title }, if preview { text } else { String::new() })
                .lead(if direct { "hat Sie erwähnt".to_string() } else { format!("{} hat Sie erwähnt", m.author) }, Tone::Mention) // pack: "{{0}} hat Sie erwähnt • {{1}}"
        } else if file {
            Toast::new(Kind::Message, if direct { m.author.clone() } else { title }, m.text.trim_start_matches('\u{1F4CE}').trim().to_string())
                .lead(if direct { "Hat eine Datei freigegeben".to_string() } else { format!("{}: Hat eine Datei freigegeben", m.author) }, Tone::File)
        } else if !m.parent.is_empty() {
            Toast::new(Kind::Message, if direct { m.author.clone() } else { title }, text).lead(if direct { "Antwort".to_string() } else { format!("{}, Antwort", m.author) }, Tone::Reply)
        } else if direct { Toast::new(Kind::Message, m.author.clone(), text) }
        // A space message: space name, then the sender line, then the text.
        else { Toast::new(Kind::Message, title, text).lead(m.author.clone(), Tone::Reply) };
        t = t.avatar(&m.author, &m.email).space(space);
        self.notify.push(t);
        true
    }

    /// Meeting reminder toasts from the meeting list ("Meeting-Erinnerung"; start times are the list's HH:MM, UTC like the list shows them).
    pub(super) fn meeting_reminders(&mut self) {
        if self.client.is_none() || self.dnd() || !self.nb("meeting_toast_enabled", true) { return; }
        let ahead = self.settings["meeting_toast_minutes"].as_u64().unwrap_or(5) as i64;
        let (today, mins) = (system::today(), (epoch() / 60 % 1440) as i64);
        let snooze = epoch();
        for m in self.meetings.clone() {
            let Some((h, mi)) = m.start.split_once(':').and_then(|(h, m)| Some((h.trim().parse::<i64>().ok()?, m.trim().parse::<i64>().ok()?))) else { continue };
            let key = format!("{today}{}{}", m.start, m.title);
            let due = h * 60 + mi - mins;
            if self.notify.snoozed.get(&key).is_some_and(|&until| until > snooze) { continue; }
            if self.notify.fired.contains(&key) || due > ahead || due < -5 { continue; }
            self.notify.fired.insert(key.clone());
            self.notify.snoozed.remove(&key);
            let body = match due { d if d <= 0 => "Startet jetzt".to_string(), 1 => "Startet in 1 Minute".into(), d => format!("Startet in {d} Minuten") }; // the pack only has "%1, startet jetzt." / "will start %2"
            let mut t = Toast::new(Kind::MeetingReminder, m.title.clone(), body).kicker("Meeting-Erinnerung").button("join", "Beitreten", true).button("snooze", "Schlummern", false);
            t.key = key; t.link = Some(m.link.clone());
            self.notify.push(t);
        }
    }

    pub(super) fn sample_toast(&mut self, what: &str) {
        let big = "Hast du kurz Zeit für das Review? \u{1F600} Ich schicke dir gleich die Unterlagen und den Link zum Board, damit wir vor dem Meeting alles durchgehen können.";
        for w in what.split(',') {
            let t = match w.trim() {
                "message" => Toast::new(Kind::Message, "Jonas Weber", big).avatar("Jonas Weber", "jonas@example.com").space("s2"),
                "group" => Toast::new(Kind::Message, "Development", "Der Build ist wieder grün \u{1F44D}").lead("Timo Steiner", Tone::Reply).avatar("Timo Steiner", "timo@example.com").space("s6"),
                "file" => Toast::new(Kind::Message, "Lina Roth", "Quartalsbericht.pdf").lead("Hat eine Datei freigegeben", Tone::File).avatar("Lina Roth", "lina@example.com").space("s10"),
                "reply" => Toast::new(Kind::Message, "Projekt Alpha", "Ja, das passt so.").lead("Mara Lind, Antwort", Tone::Reply).avatar("Mara Lind", "mara@example.com").space("s11"),
                "mention" => Toast::new(Kind::Mention, "Development", "@Alex bitte schau dir den Pull Request an, wenn du Zeit hast.").lead("Timo Steiner hat Sie erwähnt", Tone::Mention).avatar("Timo Steiner", "timo@example.com").space("s6"),
                "reminder" => Toast::new(Kind::MeetingReminder, "Weekly Sync", "Startet in 5 Minuten").kicker("Meeting-Erinnerung").button("join", "Beitreten", true).button("snooze", "Schlummern", false),
                "msgreminder" => Toast::new(Kind::MessageReminder, "Jonas Weber", "Bitte noch die Rechnung prüfen.").kicker("Erinnerungsbenachrichtigung").space("s2"),
                "summary" => Toast::new(Kind::CallSummary, "Ihre Anrufübersicht ist fertig", "Jonas Weber \u{2022} 16.11.2026, 15:12").kicker("Webex-Anrufübersicht").button("view", "Ansicht", true),
                "recording" => Toast::new(Kind::Recording, "Ihre Aufzeichnung ist bereit.", ""),
                "whiteboard" => Toast::new(Kind::Whiteboard, "Webex-Whiteboard", "Sprint-Planung").button("open", "Öffnen", true),
                "info" => Toast::new(Kind::Info, "Webox", "Das Anruf-Dock ist nicht verfügbar."),
                _ => continue,
            };
            self.notify.push(t);
        }
    }

    /// Tray menu entries (the strings sit next to the tray strings in the pack).
    pub(super) fn tray_entries(&self) -> Vec<ui::menu::Entry> {
        use ui::menu::{Entry, Item};
        vec![
            Item::new("dock", "Anruf-Dock einblenden").entry(), Item::new("health", "Zustandsüberprüfung").entry(), Item::new("feedback", "Feedback senden").entry(),
            Item::new("about", "Info").entry(), Entry::Sep, Item::new("signout", "Abmelden").enabled(!self.need_login && self.client.is_some()).entry(), Item::new("quit", "Verlassen").entry(),
        ]
    }

    pub(super) fn pick_tray(&mut self, id: &str, ctx: &egui::Context, now: f64) {
        match id {
            // Webox has no separate call dock window: its calling features live on the "Anrufe" page.
            "dock" => { self.show_main(ctx); self.set_tab(Tab::Calling); }
            "health" => { self.show_main(ctx); ui::menu::menu_open(ctx, ui::app_dialogs::HEALTH); }
            "feedback" => { self.show_main(ctx); ui::menu::menu_open(ctx, ui::app_dialogs::FEEDBACK); }
            "about" => { self.show_main(ctx); ui::menu::menu_open(ctx, "about"); }
            "signout" => { self.show_main(ctx); self.sign_out(); }
            "quit" => { self.notify.quitting = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            _ => {}
        }
        let _ = now;
    }

    /// Everything of the notification system, once per frame: state, tray, toasts, taskbar. Call after the UI so `t` is current.
    pub(crate) fn notify_update(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let (focused, minimized) = ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.viewport().minimized.unwrap_or(false)));
        self.notify.focused = focused; self.notify.minimized = minimized;
        let clock = self.notify.clock();
        let dt = (clock - self.notify.last).clamp(0.0, 0.5);
        self.notify.last = clock;
        let screenshot = self.shot.is_some();

        // First frame: tray, test hooks.
        if !self.notify.started {
            self.notify.started = true;
            #[cfg(not(target_os = "linux"))]
            if !screenshot && std::env::var_os("WEBOX_NO_TRAY").is_none() { self.notify.tray = tray::Tray::new(ctx); }
            // Linux: registered off the UI thread, and only once the panel's watcher is up (autostart may beat the panel).
            #[cfg(target_os = "linux")]
            if !screenshot && std::env::var_os("WEBOX_NO_TRAY").is_none() { tray::Tray::start(ctx); }
            self.notify.tray_tried = true;
            if self.shot.is_some() { self.notify.quitting = true; }
            if let Ok(w) = std::env::var("WEBOX_SAMPLE_TOAST") {
                let delay = std::env::var("WEBOX_TOAST_DELAY").ok().and_then(|d| d.parse::<f64>().ok()).unwrap_or(0.5);
                self.notify.sample_at = Some((clock + delay, w));
            }
            if let Ok(h) = std::env::var("WEBOX_TOAST_HIDE") { self.notify.hide_at = Some(clock + h.parse::<f64>().unwrap_or(1.0)); }
        }
        if let Some(at) = self.notify.hide_at { if clock >= at { self.notify.hide_at = None; self.hide_main(ctx); } }
        if let Some((at, w)) = self.notify.sample_at.clone() {
            if clock >= at {
                self.notify.sample_at = None;
                if w == "traymenu" {
                    let (x, y) = tray::work_area().map(|a| (a.2 - 40, a.3 - 20)).unwrap_or((800, 600));
                    self.notify.menu = Some(TrayMenu { pos: (x, y), opened: clock, focused: false, hwnd: None, placed: None });
                } else { self.sample_toast(&w); }
            }
        }
        // Idle wake-up for clock-driven work only (tray events, cloud answers and file dialogs wake the window themselves):
        // meeting reminders, "last active" texts and the sign-in renewal are minute-based, so one frame just after each full
        // minute, plus when a snoozed reminder or a test hook is due.
        {
            let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
            let mut wait = 60.0 - unix.rem_euclid(60.0) + 0.05;
            for &until in self.notify.snoozed.values() { let w = until as f64 - unix + 0.05; if w > 0.0 { wait = wait.min(w); } }
            for at in [self.notify.hide_at, self.notify.sample_at.as_ref().map(|s| s.0)].into_iter().flatten() { wait = wait.min((at - clock).max(0.0) + 0.01); }
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }

        // Tray: events, unread icon, close-to-tray.
        let unread = self.spaces.iter().filter(|s| s.unread).count();
        #[cfg(target_os = "linux")] // the panel draws the menu itself (dbusmenu): hand it the same rows
        let rows: Vec<tray::MenuRow> = self.tray_entries().iter().map(|e| match e {
            ui::menu::Entry::Item(i) => tray::MenuRow { label: i.label.clone(), enabled: i.enabled, sep: false },
            ui::menu::Entry::Title(s) => tray::MenuRow { label: s.clone(), enabled: false, sep: false },
            ui::menu::Entry::Sep => tray::MenuRow { label: String::new(), enabled: false, sep: true },
            // The avatar menu's header card (never in the tray menu): its name as a disabled row.
            ui::menu::Entry::Profile(p) => tray::MenuRow { label: p.name.clone(), enabled: false, sep: false },
        }).collect();
        #[cfg(target_os = "linux")]
        if self.notify.tray.is_none() { self.notify.tray = tray::Tray::ready(); }
        if let Some(tr) = &mut self.notify.tray {
            tr.set_unread(unread);
            #[cfg(target_os = "linux")]
            tr.set_menu(rows);
            for e in tr.poll() {
                match e {
                    tray::TrayEvent::Left => { self.notify.menu = None; self.show_main(ctx); }
                    tray::TrayEvent::Menu(x, y) => { ui::menu::menu_open(ctx, "tray"); self.notify.menu = Some(TrayMenu { pos: (x, y), opened: clock, focused: false, hwnd: None, placed: None }); }
                    #[cfg(target_os = "linux")]
                    tray::TrayEvent::Item(i) => if let Some(ui::menu::Entry::Item(it)) = self.tray_entries().get(i as usize) {
                        if it.enabled { self.notify.out.push(Out::Menu(it.id.clone())); }
                    },
                }
            }
        }
        // Wayland can't hide a window (winit ignores Visible(false)): there the X minimizes instead (the tray / a notification
        // brings it back through xdg-activation), so a running call goes on. Strg+Q / "Verlassen" (quitting) still quit.
        // Without xdg-activation nothing could bring a minimized window back from the tray: the X quits there.
        #[cfg(target_os = "linux")]
        let (can_hide, can_restore) = (crate::platform::linux::session() != crate::platform::linux::Session::Wayland, crate::platform::linux::activation::available());
        #[cfg(not(target_os = "linux"))]
        let (can_hide, can_restore) = (true, true);
        if ctx.input(|i| i.viewport().close_requested()) && !self.notify.quitting && self.notify.tray.is_some() && self.nb("close_to_tray", true) && (can_hide || can_restore) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if can_hide { self.hide_main(ctx); } else { ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }
        }

        // Taskbar: flash while the window is behind, unread badge overlay.
        #[cfg(windows)]
        if let Some(h) = self.hwnd {
            if self.notify.want_flash && !focused { tray::flash(h, true); self.notify.flashing = true; }
            if focused && self.notify.flashing { tray::flash(h, false); self.notify.flashing = false; }
            let shown = if self.nb("notif_taskbar_flash", true) { unread } else { 0 };
            if self.notify.shown_unread != Some(shown) { self.notify.shown_unread = Some(shown); tray::overlay(h, shown); }
        }
        // Linux: another start handed over (launcher click while hidden in the tray, a link): show the window, open the space.
        #[cfg(target_os = "linux")]
        for uri in crate::platform::linux::instance::take(ctx) {
            self.notify.menu = None;
            self.show_main(ctx);
            if let Some(id) = system::space_from_uri(&uri) { if self.spaces.iter().any(|s| s.id == id) { self.set_tab(Tab::Messaging); self.open_space(id); } else { self.pending_space = Some(id); } }
        }
        // Linux: the urgency hint (X11) / xdg-activation request (Wayland) through winit, the count on the launcher entry.
        #[cfg(not(windows))]
        {
            use egui::{UserAttentionType, ViewportCommand};
            if self.notify.want_flash && !focused { ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(UserAttentionType::Informational)); self.notify.flashing = true; }
            if focused && self.notify.flashing { ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(UserAttentionType::Reset)); self.notify.flashing = false; }
            let shown = if self.nb("notif_taskbar_flash", true) { unread } else { 0 };
            if self.notify.shown_unread != Some(shown) && !screenshot { self.notify.shown_unread = Some(shown); tray::overlay(0, shown); }
        }
        self.notify.want_flash = false;

        self.meeting_reminders();

        // Toast lifecycle: hover pauses, the end fades out.
        for ts in &mut self.notify.toasts {
            if ts.hovered && !ts.dismissed { ts.deadline = (ts.deadline + dt).max(clock); }
            ts.hovered = false;
        }
        self.notify.toasts.retain(|ts| clock < ts.deadline + FADE);
        #[cfg(target_os = "linux")]
        { self.native_toasts(ctx); crate::ui::call_toast::call_note_frame(ctx); }
        #[cfg(target_os = "macos")]
        self.mac_toasts();

        if !self.notify.toasts.is_empty() && ctx.embed_viewports() {
            // No multi-viewport support: fall back to the in-window toast.
            let list: Vec<Toast> = self.notify.toasts.drain(..).collect();
            for ts in list { self.toast(format!("{}: {}", ts.title, ts.body), now); }
        }
        if !self.notify.toasts.is_empty() { self.show_toasts(ctx, t, clock); } else { self.notify.hwnd = None; self.notify.placed = None; self.notify.shaped = None; }
        if self.notify.menu.is_some() { self.show_tray_menu(ctx, t, clock, now); }

        // Reactions of the toasts.
        let outs = std::mem::take(&mut self.notify.out);
        for o in outs {
            match o {
                #[cfg(target_os = "linux")]
                Out::Token(t) => { self.notify.raise_token = Some(t); continue; }
                Out::Open(space) => { self.show_main(ctx); if let Some(id) = space { if self.spaces.iter().any(|s| s.id == id) { self.set_tab(Tab::Messaging); self.open_space(id); } } }
                Out::Button(ts, id) => match id {
                    "join" => { if let Some(l) = &ts.link { system::open(l); } }
                    "snooze" => { self.notify.fired.remove(&ts.key); self.notify.snoozed.insert(ts.key.clone(), epoch() + 60); } // snooze for 1 minute
                    "open" => { self.show_main(ctx); self.set_tab(Tab::Whiteboards); }
                    _ => self.show_main(ctx),
                },
                #[cfg(target_os = "linux")]
                Out::Menu(id) => self.pick_tray(&id, ctx, now),
            }
            // an action that didn't raise the window (snooze, join) leaves its token unused: it must not raise for a later one
            #[cfg(target_os = "linux")]
            { self.notify.raise_token = None; }
        }

        // Test hook: capture the toast window (and the sidecar facts), then quit.
        if let Ok(path) = std::env::var("WEBOX_TOAST_SHOT") {
            let ready = self.notify.toasts.iter().map(|ts| ts.born).fold(f64::MAX, f64::min);
            let menu_ready = self.notify.menu.as_ref().is_some_and(|m| clock - m.opened > 1.2 && m.hwnd.is_some());
            if (!self.notify.toasts.is_empty() && clock - ready > 1.3 && self.notify.hwnd.is_some()) || menu_ready {
                let h = if menu_ready { self.notify.menu.as_ref().and_then(|m| m.hwnd) } else { self.notify.hwnd };
                if let Some(h) = h {
                    let ok = tray::capture(h, std::path::Path::new(&path));
                    let info = format!("captured={ok} window={:?} work_area={:?} dpi_scale={} main_focused={}\n", tray::window_rect(h), tray::work_area(), tray::dpi_scale(h), focused);
                    let _ = std::fs::write(format!("{path}.txt"), info);
                    self.notify.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    pub(super) fn show_toasts(&mut self, ctx: &egui::Context, t: &Th, clock: f64) {
        // Photos need the app's texture cache; resolve before the viewport closure borrows the other fields.
        let photos: HashMap<u64, Option<egui::TextureId>> = self.notify.toasts.iter().map(|ts| (ts.id, ts.avatar.as_ref().and_then(|a| self.photos.texture(ctx, &a.1)))).collect();
        let App { notify: hub, icons, .. } = self;
        // The window is exactly the stack of cards: no shadow, so no padding around it; the gaps between the cards and the room
        // beside a narrower card are transparent (tray::see_through), or cut away without a compositor (tray::popup_shape).
        let lays: Vec<Lay> = hub.toasts.iter().map(|ts| layout(ctx, t, ts)).collect();
        let total: f32 = lays.iter().map(|l| l.h).sum::<f32>() + GAP * (lays.len() as f32 - 1.0).max(0.0);
        hub.measured = total;
        let ppp = hub.hwnd.map_or(ctx.pixels_per_point(), |h| tray::dpi_scale(h) * ctx.zoom_factor());
        let size = Vec2::new(lays.iter().map(|l| l.w).fold(0.0, f32::max).max(1.0), total.max(1.0));
        let pos = tray::work_area().map_or(pos2(200.0, 200.0), |a| pos2(a.2 as f32 / ppp - size.x - EDGE, a.3 as f32 / ppp - size.y - EDGE));
        let builder = egui::ViewportBuilder::default().with_title(TITLE).with_inner_size(size).with_position(pos).with_decorations(false)
            .with_transparent(true).with_always_on_top().with_taskbar(false).with_active(false).with_resizable(false);
        let t = *t;
        // Without a compositor (X11) the cards don't fade (that would darken them towards black).
        let comp = tray::composited();
        let mut cards: Vec<Rect> = vec![];
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("webox-toasts"), builder, |ctx, _class| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                let screen = ui.max_rect();
                let mut bottom = screen.bottom();
                for (i, ts) in hub.toasts.iter_mut().enumerate().rev() {
                    let lay = &lays[i];
                    let rect = Rect::from_min_max(pos2(screen.right() - lay.w, bottom - lay.h), pos2(screen.right(), bottom)); // own width, right edges aligned
                    bottom = rect.top() - GAP;
                    let fin = ((clock - ts.born) / FADE).clamp(0.0, 1.0);
                    let fout = if clock > ts.deadline { (1.0 - (clock - ts.deadline) / FADE).clamp(0.0, 1.0) } else { 1.0 };
                    // Fly-in from the screen edge (250 ms ease-out; the window edge clips it), and a short drift out while fading.
                    let sin = ((clock - ts.born) / 0.25).clamp(0.0, 1.0) as f32;
                    let slide = 80.0 * (1.0 - sin).powi(3) + 24.0 * (1.0 - fout as f32);
                    let rect = rect.translate(Vec2::new(slide, 0.0));
                    cards.push(rect.intersect(screen).translate(-screen.min.to_vec2()));
                    let o = draw_card(ui, icons, &t, ts, lay, rect, photos.get(&ts.id).copied().flatten(), if comp { fin.min(fout) as f32 } else { 1.0 });
                    ts.hovered = o.hovered;
                    if o.close { ts.dismissed = true; ts.deadline = clock; }
                    if let Some(b) = o.button { hub.out.push(Out::Button(ts.clone(), ts.buttons[b].0)); ts.dismissed = true; ts.deadline = clock; }
                    else if o.clicked { hub.out.push(Out::Open(ts.space.clone())); ts.dismissed = true; ts.deadline = clock; }
                }
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        });
        // Style + exact (physical-pixel) placement at the bottom right of the work area, above the taskbar.
        if hub.hwnd.is_none() { hub.hwnd = tray::find_window(TITLE); if let Some(h) = hub.hwnd { tray::make_toast_window(h); } }
        if let (Some(h), Some(a)) = (hub.hwnd, tray::work_area()) {
            let ppp = tray::dpi_scale(h) * ctx.zoom_factor();
            let (w, hh) = ((size.x * ppp).round() as i32, (size.y * ppp).round() as i32);
            let want = (a.2 - w - (EDGE * ppp).round() as i32, a.3 - hh - (EDGE * ppp).round() as i32, w, hh);
            if hub.placed != Some(want) { hub.placed = Some(want); tray::place(h, want.0, want.1, want.2, want.3); }
            if !comp {
                let px: Vec<(i32, i32, i32, i32)> = cards.iter().map(|r| ((r.left() * ppp).round() as i32, (r.top() * ppp).round() as i32, (r.width() * ppp).round() as i32, (r.height() * ppp).round() as i32)).collect();
                if hub.shaped.as_ref() != Some(&px) { tray::popup_shape(h, &px, 0); hub.shaped = Some(px); }
            }
        }
    }
    /// Wayland: a client can't place its own window at the bottom right, so the toasts go to the desktop's notification server
    /// (same title, text and buttons; a click opens the space). `WEBOX_NATIVE_TOASTS=1/0` forces it on / off (X11 default: off,
    /// the app's own toast windows as on Windows). The server is asked off the UI thread; a toast it fails (no server, an error, no
    /// answer within 3 s) comes back once as the app's own toast window and isn't sent again.
    #[cfg(target_os = "linux")]
    pub(super) fn native_toasts(&mut self, ctx: &egui::Context) {
        use crate::platform::linux::notifications as n;
        let on = match std::env::var("WEBOX_NATIVE_TOASTS").ok().as_deref() { Some("1") => true, Some("0") => false, _ => crate::platform::linux::session() == crate::platform::linux::Session::Wayland };
        let clock = self.notify.clock();
        let hub = &mut self.notify;
        if on && self.shot.is_none() {
            let mut keep = vec![];
            for ts in std::mem::take(&mut hub.toasts) {
                if ts.native_failed { keep.push(ts); continue; }
                if ts.dismissed { continue; }
                let lead = ts.lead.as_ref().map(|l| l.0.as_str()).unwrap_or("");
                let body = [ts.kicker.as_str(), lead, ts.body.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("\n");
                let mut actions: Vec<(&str, &str)> = vec![("default", "Öffnen")];
                actions.extend(ts.buttons.iter().map(|(id, l, _)| (*id, l.as_str())));
                hub.native_wait.push((n::send(ctx, &ts.title, &body, &actions, 1), ts)); // urgency normal; 2 (critical) is for incoming calls
            }
            hub.toasts = keep;
        }
        for (sent, mut ts) in std::mem::take(&mut hub.native_wait) {
            match sent.get() {
                None => hub.native_wait.push((sent, ts)), // still asking
                Some(Some(id)) => { hub.native.insert(*id, ts); }
                Some(None) => { ts.native_failed = true; ts.born = clock; ts.deadline = clock + DURATION; hub.toasts.push(ts); }
            }
        }
        let closed = n::closed(); // applied after the actions: a click and its close can land in the same frame
        for (id, key, token) in n::invoked() {
            if crate::ui::call_toast::call_note_invoked(id, &key, token.clone()) { continue; } // the incoming-call notification's Annehmen / Ablehnen
            let Some(ts) = hub.native.remove(&id) else { continue };
            n::close(id);
            if let Some(t) = token { hub.out.push(Out::Token(t)); } // raises the window for this very action (its show_main), not another
            match ts.buttons.iter().find(|b| b.0 == key).map(|b| b.0) {
                Some(b) => hub.out.push(Out::Button(ts, b)),
                None => hub.out.push(Out::Open(ts.space.clone())),
            }
        }
        for id in closed { hub.native.remove(&id); }
    }

    /// The tray's context menu: the black balloon of `ui/menu/` in a small borderless window at the cursor (closes on a pick, Esc, focus loss).
    pub(super) fn show_tray_menu(&mut self, ctx: &egui::Context, t: &Th, clock: f64, now: f64) {
        let entries = self.tray_entries();
        let body_h = 16.0 + entries.iter().map(|e| match e { ui::menu::Entry::Sep => 11.0, ui::menu::Entry::Title(_) => 32.0, _ => 40.0 }).sum::<f32>();
        // The window is exactly the balloon (no margin, no shadow; ui::menu draws "tray" at (0, 0)): its height from the entries,
        // its width as last drawn (284, wider for a long label).
        let size = Vec2::new(ui::menu::menu_body(ctx, "tray").map_or(280.0, |b| b.width()), body_h);
        let Some(m) = &mut self.notify.menu else { return };
        let ppp = ctx.pixels_per_point();
        // Right edge at the cursor, above it when the cursor is in the lower half of the work area (the tray sits there).
        let wa = tray::work_area().unwrap_or((0, 0, 1920, 1080));
        let (sw, sh) = ((size.x * ppp) as i32, (size.y * ppp) as i32);
        let x = (m.pos.0 - sw).clamp(wa.0, (wa.2 - sw).max(wa.0));
        let y = if m.pos.1 > (wa.1 + wa.3) / 2 { m.pos.1 - sh } else { m.pos.1 }.clamp(wa.1, (wa.3 - sh).max(wa.1));
        let builder = egui::ViewportBuilder::default().with_title(MENU_TITLE).with_inner_size(size).with_position(pos2(x as f32 / ppp, y as f32 / ppp))
            .with_decorations(false).with_transparent(true).with_always_on_top().with_taskbar(false).with_active(true).with_resizable(false);
        let (th, mut picked, mut done) = (*t, None, false);
        let App { icons, notify: hub, .. } = self;
        let menu = hub.menu.as_mut().unwrap();
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("webox-tray-menu"), builder, |ctx, _| {
            ui::menu::menu_open(ctx, "tray");
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |_| {});
            let anchor = Rect::from_min_size(pos2(0.0, 0.0), Vec2::ZERO);
            picked = ui::menu::webex_menu(ctx, icons, &th, "tray", anchor, ui::menu::Side::Below, ui::menu::Align::Left, false, &entries);
            let f = ctx.input(|i| i.viewport().focused);
            if f == Some(true) { menu.focused = true; }
            if (menu.focused && f == Some(false)) || !ui::menu::menu_is_open(ctx, "tray") || clock - menu.opened > 30.0 { done = true; }
            if !menu.focused && clock - menu.opened > 0.3 { ctx.send_viewport_cmd(egui::ViewportCommand::Focus); }
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        });
        if menu.hwnd.is_none() { menu.hwnd = tray::find_window(MENU_TITLE); if let Some(h) = menu.hwnd { tray::see_through(h); } }
        if let Some(h) = menu.hwnd.filter(|_| menu.placed != Some((x, y, sw, sh))) {
            menu.placed = Some((x, y, sw, sh)); tray::place(h, x, y, sw, sh);
            // The rounded corners (radius 10) need alpha; without a compositor (X11) the window is cut to the rounded balloon.
            tray::popup_shape(h, &[(0, 0, sw, sh)], (10.0 * ppp).round() as i32);
        }
        if done || picked.is_some() { hub.menu = None; ui::menu::menu_close(ctx, "tray"); }
        if let Some(id) = picked { self.pick_tray(&id, ctx, now); }
    }
}

/// Whether `low` (lower case) has `@word` as a whole word: not inside an address (x@allianz.com) or a longer name (@allison).
fn at_word(low: &str, word: &str) -> bool {
    let pat = format!("@{word}");
    !word.is_empty() && low.match_indices(&pat).any(|(i, _)| {
        !low[..i].chars().next_back().is_some_and(char::is_alphanumeric) && !low[i + pat.len()..].chars().next().is_some_and(char::is_alphanumeric)
    })
}

/// Whether a message mentions me (`name` = my display name) or everybody: an @all mention in its rich text
/// (`<spark-mention data-object-type="groupMention">`; a `<` in the sender's text always arrives escaped), else `@<name>`,
/// `@<first name>`, `@all` (`@alle`) as whole words in the plain text.
fn mentions_me(text: &str, html: &str, name: &str) -> bool {
    if name.is_empty() { return false; }
    let tags = html.to_lowercase();
    let all = tags.split("<spark-mention").skip(1).any(|t| { let tag = &t[..t.find('>').unwrap_or(t.len())]; tag.contains("groupmention") || tag.contains("data-group-type=\"all\"") });
    let (low, name) = (text.to_lowercase(), name.to_lowercase());
    let first = name.split_whitespace().next().unwrap_or("");
    all || [name.as_str(), first, "all", "alle"].iter().any(|w| at_word(&low, w))
}

#[cfg(test)]
mod tests {
    use super::mentions_me;

    #[test]
    fn mentions_are_whole_words() {
        let me = "Alex Muster";
        assert!(mentions_me("@all bitte lesen", "", me) && mentions_me("(@Alex) schau mal", "", me));
        assert!(mentions_me("danke @alex muster.", "", me) && mentions_me("@alle: Meeting", "", me));
        // An e-mail address or a longer word is no mention.
        assert!(!mentions_me("schreib an x@allianz.com", "", me));
        assert!(!mentions_me("max@all.example und @allison", "", me));
        assert!(!mentions_me("kontakt: info@alex.ch", "", me));
        assert!(!mentions_me("@all", "", ""), "no name known: nothing counts (as before)");
        // The rich text's @all mention counts, even when the plain text shows the name without "@".
        assert!(mentions_me("All bitte lesen", "<spark-mention data-object-type=\"groupMention\" data-group-type=\"all\">All</spark-mention> bitte lesen", me));
        assert!(!mentions_me("data-object-type=\"groupMention\"", "data-object-type=\"groupMention\"", me), "only inside a mention tag");
    }
}
