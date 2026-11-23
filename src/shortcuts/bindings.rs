//! The app side of shortcuts: loading, matching keys, running commands and global hotkeys.
use super::*;

/// Settings key (Linux): register the commands marked "Global" with the desktop (X11 grab / GlobalShortcuts portal). Default off.
pub(crate) const GLOBAL_KEY: &str = "linux_global_shortcuts";

impl App {
    pub(crate) fn sc_load(&mut self) { if !self.sc.loaded { let s = self.settings.clone(); self.sc.load(&s); } }
    pub(crate) fn sc_save(&mut self) {
        self.sc.refresh();
        let v = self.sc.to_json();
        if v.as_object().is_some_and(|m| m.is_empty()) { if let Some(o) = self.settings.as_object_mut() { o.remove("shortcuts"); } } else { self.settings["shortcuts"] = v; }
        system::save_settings(&self.cfg, &self.settings);
    }
    /// The user typed `combo` (or removed the key with `None`) for command `idx`; `other` is disabled / reset as decided by `find_conflict`.
    pub(crate) fn sc_assign(&mut self, idx: usize, combo: Option<Combo>, other: Option<Conflict>) {
        let cmds = &registry().cmds;
        if let Some(c) = other {
            let o = self.sc.ov.entry(c.other).or_default();
            o.key = Some(match c.fate { Fate::ResetTo(d) => Some(d), _ => None });
            normalize(&cmds[c.other], o);
            if *o == Ov::default() { self.sc.ov.remove(&c.other); }
        }
        let o = self.sc.ov.entry(idx).or_default();
        o.key = Some(combo);
        if combo.is_none() { o.global = None; }
        normalize(&cmds[idx], o);
        if *o == Ov::default() { self.sc.ov.remove(&idx); }
        self.sc_save();
    }
    pub(crate) fn sc_set_global(&mut self, idx: usize, on: bool) {
        let cmds = &registry().cmds;
        let o = self.sc.ov.entry(idx).or_default();
        o.global = Some(on);
        normalize(&cmds[idx], o);
        if *o == Ov::default() { self.sc.ov.remove(&idx); }
        self.sc_save();
    }
    /// Back to the default key and global flag (the caller resolved a conflict first, if any).
    pub(crate) fn sc_reset(&mut self, idx: usize, other: Option<Conflict>) {
        if let Some(c) = other {
            let o = self.sc.ov.entry(c.other).or_default();
            o.key = Some(match c.fate { Fate::ResetTo(d) => Some(d), _ => None });
            normalize(&registry().cmds[c.other], o);
            if *o == Ov::default() { self.sc.ov.remove(&c.other); }
        }
        self.sc.ov.remove(&idx);
        self.sc_save();
    }
    pub(crate) fn sc_reset_all(&mut self) { self.sc.ov.clear(); self.sc_save(); }

    /// Runs command `idx` of the registry (a key press, a global hotkey or a pick in the shortcut search).
    pub(crate) fn run_command(&mut self, idx: usize, ctx: &egui::Context) {
        let c = &registry().cmds[idx];
        let now = ctx.input(|i| i.time);
        match c.kind {
            Kind::Run(a) => self.act(a, ctx),
            Kind::Cmd => self.run_cmd(c.id, ctx),
            Kind::Composer => self.act(Act::Composer(c.id), ctx),
            Kind::Native => {}
            Kind::Stub => self.toast(format!("{}: nicht verfügbar", c.label), now),
        }
    }

    /// Message font size (Ctrl++ / Ctrl+-): steps of 10 %, kept in the settings (`zoom_factor`) and applied at start.
    pub(crate) fn sc_zoom(&mut self, ctx: &egui::Context, d: f32) {
        let z = (ctx.zoom_factor() + d).clamp(0.6, 2.0);
        ctx.set_zoom_factor(z);
        self.settings["zoom_factor"] = json!((z * 100.0).round() / 100.0);
        system::save_settings(&self.cfg, &self.settings);
    }

    /// Once per frame: loads the bindings, restores the zoom, delivers global hotkeys and dispatches key presses.
    pub(crate) fn shortcuts_frame(&mut self, ctx: &egui::Context) {
        self.sc_load();
        if !self.sc.zoom_restored {
            self.sc.zoom_restored = true;
            ctx.options_mut(|o| o.zoom_with_keyboard = false); // Ctrl++ / Ctrl+- are the registry's ZoomIn / ZoomOut now
            if let Some(z) = self.settings["zoom_factor"].as_f64() { ctx.set_zoom_factor((z as f32).clamp(0.6, 2.0)); }
        }
        let reg = registry();
        self.sc_sync_global(ctx);
        #[cfg(any(windows, target_os = "linux"))]
        for idx in self.sc.global.drain() { self.run_command(idx, ctx); }

        // The settings page is recording a key (it stamps the pass number every pass while in edit mode).
        if ctx.data(|d| d.get_temp::<u64>(egui::Id::new("sc-capture"))).is_some_and(|n| ctx.cumulative_pass_nr().saturating_sub(n) <= 1) { return; }
        // egui-winit turns Ctrl+C / X / V into Copy / Cut / Paste even with Shift and drops the key: with Shift they count as the key
        // (for commands Webox implements, so Ctrl+Shift+V still pastes while ToggleVideoMute is a stub).
        let events: Vec<(Key, Option<Key>, Modifiers, bool, bool)> = ctx.input(|i| i.events.iter().filter_map(|e| match e {
            egui::Event::Key { key, physical_key, pressed: true, repeat, modifiers } => Some((*key, *physical_key, *modifiers, *repeat, false)),
            egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) if i.modifiers.shift && i.modifiers.command =>
                Some((match e { egui::Event::Copy => Key::C, egui::Event::Cut => Key::X, _ => Key::V }, None, i.modifiers, false, true)),
            _ => None }).collect());
        let text_focus = ctx.wants_keyboard_input();
        for (key, phys, mods, repeat, clip) in events {
            let pred = |i: usize, loose: bool| {
                let (c, r) = (&reg.cmds[i], &self.sc.res[i]);
                let Some(k) = r.key else { return false };
                if !k.matches_at(key, phys, &mods, loose) || c.kind_native() { return false; }
                if clip && !matches!(c.kind, Kind::Run(_) | Kind::Cmd | Kind::Composer) { return false; }
                #[cfg(any(windows, target_os = "linux"))]
                if self.sc.global.is_registered(i) { return false; } // delivered by the hotkey thread
                if repeat && !matches!(c.id, "ZoomIn" | "ZoomOut" | "PreviousSpace" | "NextSpace") { return false; }
                // Without Ctrl/Alt a key belongs to the focused text box / the message list (Shift+Up, PgUp, ...); stubs never take those.
                if k.plain_modifier_free() && !matches!(key, Key::F1 | Key::F2 | Key::F3 | Key::F4 | Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::F11 | Key::F12) { return false; }
                if text_focus && k.plain_modifier_free() { return false; }
                true
            };
            // Exact Shift first: Ctrl+Shift+= (Superscript) before Ctrl+= (Subscript, a symbol that also matches with Shift).
            let hit = (0..reg.cmds.len()).find(|&i| pred(i, false)).or_else(|| (0..reg.cmds.len()).find(|&i| pred(i, true)));
            if let Some(i) = hit {
                ctx.input_mut(|inp| {
                    let pos = inp.events.iter().position(|e| if clip { matches!((key, e), (Key::V, egui::Event::Paste(_)) | (Key::C, egui::Event::Copy) | (Key::X, egui::Event::Cut)) } else {
                        matches!(e, egui::Event::Key { key: k2, physical_key: p2, pressed: true, modifiers: m2, .. } if *k2 == key && *p2 == phys && *m2 == mods) });
                    if let Some(p) = pos { take_key(&mut inp.events, p); } // a matched Ctrl+Shift+V does not paste
                });
                self.run_command(i, ctx);
            }
        }
    }

    /// Keeps the OS-level hotkeys in line with the settings (and with a call being on screen for call commands).
    pub(super) fn sc_sync_global(&mut self, _ctx: &egui::Context) {
        let reg = registry();
        // Linux: opt-in (Ctrl+Shift+X / C, Ctrl+D are everyday keys in other apps), and the call commands stay bound for the whole
        // run, because every change rebuilds the portal session and the desktop may ask again.
        let linux = cfg!(target_os = "linux");
        let call_on = linux || self.call.cur.as_ref().is_some_and(|c| c.ended_at.is_none());
        let mut want: Vec<(usize, Combo)> = Vec::new();
        for (i, c) in reg.cmds.iter().enumerate() {
            if linux && !self.settings[GLOBAL_KEY].as_bool().unwrap_or(false) { break; }
            let r = &self.sc.res[i];
            let (Some(k), true) = (r.key, r.global) else { continue };
            // Only commands Webox implements are worth taking a key away from the whole system for; call commands only while a call is on screen.
            if !matches!(c.kind, Kind::Run(_) | Kind::Cmd) || (c.needs_call() && !call_on) { continue; }
            want.push((i, k));
        }
        if want != self.sc.global_sig {
            #[cfg(windows)]
            self.sc.global.set(_ctx, want.clone());
            #[cfg(target_os = "linux")]
            if !self.sc.global.set(_ctx, want.clone()) { return; } // no session type known yet: try again next frame
            self.sc.global_sig = want;
        }
    }
}
