//! Global hotkeys on Linux, same interface as the Windows `global.rs` (`set` / `drain` / `is_registered`):
//!
//! * **X11**: `XGrabKey` on the root window (pure-Rust x11rb connection on a thread of its own), grabbed four times so Caps Lock
//!   and Num Lock don't matter. A grab another program holds fails with BadAccess, so that command stays in-app, like a
//!   RegisterHotKey failure on Windows.
//! * **Wayland**: clients can't grab keys; the xdg-desktop-portal **GlobalShortcuts** interface (KDE Plasma, GNOME 48+, Hyprland)
//!   binds them instead. The desktop may ask the user to confirm (or change) the keys once; the ones it accepts count as registered.
//!   Without the portal nothing is registered and the shortcuts work while the window has the focus.
use super::Combo;
use eframe::egui::{self, Key};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Shared {
    wanted: Vec<(usize, Combo)>,
    registered: HashSet<usize>,
    fired: Vec<usize>,
    dirty: bool,
    ctx: Option<egui::Context>,
}

#[derive(Default)]
pub(crate) struct Handle { shared: Arc<Mutex<Shared>>, started: bool, wake: Arc<tokio::sync::Notify> }

impl Handle {
    pub(crate) fn set(&mut self, ctx: &egui::Context, want: Vec<(usize, Combo)>) {
        { let mut s = self.shared.lock().unwrap(); s.wanted = want.clone(); s.dirty = true; s.ctx = Some(ctx.clone()); }
        self.wake.notify_one();
        if self.started || want.is_empty() { return; }
        let shared = self.shared.clone();
        match crate::platform::linux::session() {
            crate::platform::linux::Session::X11 => { std::thread::Builder::new().name("webox-hotkeys".into()).spawn(move || run_x11(shared)).ok(); }
            crate::platform::linux::Session::Wayland => { crate::platform::linux::rt().spawn(run_portal(shared, self.wake.clone())); }
            crate::platform::linux::Session::Unknown => return,
        }
        self.started = true;
    }
    pub(crate) fn drain(&mut self) -> Vec<usize> { std::mem::take(&mut self.shared.lock().unwrap().fired) }
    pub(crate) fn is_registered(&self, idx: usize) -> bool { self.shared.lock().unwrap().registered.contains(&idx) }
}

/// `WEBOX_HOTKEY_LOG=1`: report grabs and presses on stderr (the headless Linux test checks them).
fn trace(s: String) { if std::env::var_os("WEBOX_HOTKEY_LOG").is_some() { eprintln!("hotkeys: {s}"); } }

fn fire(shared: &Arc<Mutex<Shared>>, idx: usize) {
    trace(format!("fired {}", super::registry().cmds[idx].id));
    let ctx = { let mut s = shared.lock().unwrap(); s.fired.push(idx); s.ctx.clone() };
    if let Some(c) = ctx { c.request_repaint(); }
}

/// X keysym of a key (as written in /usr/include/X11/keysymdef.h); None: no global hotkey for it.
fn keysym(k: Key) -> Option<u32> {
    use Key::*;
    let letters = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z];
    if let Some(i) = letters.iter().position(|&l| l == k) { return Some(0x61 + i as u32); }
    let digits = [Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9];
    if let Some(i) = digits.iter().position(|&d| d == k) { return Some(0x30 + i as u32); }
    let fkeys = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    if let Some(i) = fkeys.iter().position(|&f| f == k) { return Some(0xffbe + i as u32); }
    Some(match k {
        Enter => 0xff0d, Space => 0x20, Tab => 0xff09, Backspace => 0xff08, Delete => 0xffff, Insert => 0xff63, Home => 0xff50, End => 0xff57,
        PageUp => 0xff55, PageDown => 0xff56, ArrowLeft => 0xff51, ArrowUp => 0xff52, ArrowRight => 0xff53, ArrowDown => 0xff54,
        Comma => 0x2c, Period => 0x2e, Minus => 0x2d, Plus | Equals => 0x3d, Slash => 0x2f, Semicolon => 0x3b, Quote => 0x27,
        Backslash => 0x5c, OpenBracket => 0x5b, CloseBracket => 0x5d, Backtick => 0x60,
        _ => return None,
    })
}

fn run_x11(shared: Arc<Mutex<Shared>>) {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
    use x11rb::protocol::Event;
    let Ok((c, n)) = x11rb::connect(None) else { return };
    let root = c.setup().roots[n].root;
    let (min, max) = (c.setup().min_keycode, c.setup().max_keycode);
    let Some(map) = c.get_keyboard_mapping(min, max - min + 1).ok().and_then(|r| r.reply().ok()) else { return };
    let per = map.keysyms_per_keycode as usize;
    let code_of = |sym: u32| -> Option<u8> {
        (0..=(max - min) as usize).find(|&i| map.keysyms.get(i * per..i * per + per).is_some_and(|ks| ks.contains(&sym))).map(|i| min + i as u8)
    };
    let base = ModMask::SHIFT | ModMask::CONTROL | ModMask::M1; // the modifiers that count (M1 = Alt)
    let extra = [ModMask::from(0u16), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2]; // Caps Lock / Num Lock on or off
    let mut active: Vec<(u8, u16, usize)> = vec![];
    loop {
        let dirty = std::mem::take(&mut shared.lock().unwrap().dirty);
        if dirty {
            for &(code, m, _) in &active { for e in extra { let _ = c.ungrab_key(code, root, ModMask::from(m) | e); } }
            active.clear();
            let wanted = shared.lock().unwrap().wanted.clone();
            let mut ok = HashSet::new();
            for (idx, combo) in wanted {
                let Some(code) = keysym(combo.key).and_then(code_of) else { continue };
                let mut m = ModMask::from(0u16);
                if combo.ctrl { m |= ModMask::CONTROL } if combo.alt { m |= ModMask::M1 } if combo.shift { m |= ModMask::SHIFT }
                let all = extra.iter().all(|&e| c.grab_key(false, root, m | e, code, GrabMode::ASYNC, GrabMode::ASYNC).map(|ck| ck.check().is_ok()).unwrap_or(false));
                if all { active.push((code, u16::from(m), idx)); ok.insert(idx); }
                else { for e in extra { let _ = c.ungrab_key(code, root, m | e); } }
            }
            let _ = c.flush();
            trace(format!("x11 grabbed {:?}", ok.iter().map(|&i| super::registry().cmds[i].id).collect::<Vec<_>>()));
            shared.lock().unwrap().registered = ok;
        }
        let mut got = false;
        while let Ok(Some(ev)) = c.poll_for_event() {
            got = true;
            if let Event::KeyPress(k) = ev {
                let m = u16::from(k.state) & u16::from(base);
                if let Some(&(_, _, idx)) = active.iter().find(|a| a.0 == k.detail && a.1 == m) { fire(&shared, idx); }
            }
        }
        if !got { std::thread::sleep(std::time::Duration::from_millis(30)); }
    }
}

/// Trigger in the shortcuts spec's form ("CTRL+SHIFT+a"), the default the portal offers the user.
fn trigger(c: &Combo) -> Option<String> {
    let sym = keysym(c.key)?;
    let name = match sym {
        0x20..=0x7e => match sym { 0x20 => "space".into(), 0x2c => "comma".into(), 0x2e => "period".into(), 0x2d => "minus".into(), 0x3d => "equal".into(),
            0x2f => "slash".into(), 0x3b => "semicolon".into(), 0x27 => "apostrophe".into(), 0x5c => "backslash".into(), 0x5b => "bracketleft".into(),
            0x5d => "bracketright".into(), 0x60 => "grave".into(), s => char::from_u32(s)?.to_string() },
        0xffbe..=0xffc9 => format!("F{}", sym - 0xffbe + 1),
        0xff0d => "Return".into(), 0xff09 => "Tab".into(), 0xff08 => "BackSpace".into(), 0xffff => "Delete".into(), 0xff63 => "Insert".into(),
        0xff50 => "Home".into(), 0xff57 => "End".into(), 0xff55 => "Page_Up".into(), 0xff56 => "Page_Down".into(),
        0xff51 => "Left".into(), 0xff52 => "Up".into(), 0xff53 => "Right".into(), 0xff54 => "Down".into(),
        _ => return None,
    };
    let mut s = String::new();
    if c.ctrl { s += "CTRL+" } if c.alt { s += "ALT+" } if c.shift { s += "SHIFT+" }
    Some(s + &name)
}

async fn run_portal(shared: Arc<Mutex<Shared>>, wake: Arc<tokio::sync::Notify>) {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    let Ok(gs) = GlobalShortcuts::new().await else { return };
    let Ok(mut activated) = gs.receive_activated().await else { return };
    let reg = super::registry();
    let mut session = None;
    loop {
        let wanted = { let mut s = shared.lock().unwrap(); s.dirty = false; s.wanted.clone() };
        if let Some(s) = session.take() { let s: ashpd::desktop::Session<'_, GlobalShortcuts> = s; let _ = s.close().await; }
        let mut ok = HashSet::new();
        if !wanted.is_empty() {
            if let Ok(s) = gs.create_session().await {
                let list: Vec<NewShortcut> = wanted.iter().map(|(i, c)| {
                    let cmd = &reg.cmds[*i];
                    let t = trigger(c);
                    NewShortcut::new(cmd.id, if cmd.label.is_empty() { cmd.id } else { cmd.label }).preferred_trigger(t.as_deref())
                }).collect();
                if let Ok(req) = gs.bind_shortcuts(&s, &list, None).await {
                    if let Ok(resp) = req.response() { for sc in resp.shortcuts() { if let Some(i) = reg.index(sc.id()) { ok.insert(i); } } }
                }
                session = Some(s);
            }
        }
        trace(format!("portal bound {:?}", ok.iter().map(|&i| reg.cmds[i].id).collect::<Vec<_>>()));
        shared.lock().unwrap().registered = ok;
        loop {
            tokio::select! {
                _ = wake.notified() => if shared.lock().unwrap().dirty { break },
                a = activated.next() => match a {
                    Some(a) => if let Some(i) = reg.index(a.shortcut_id()) { fire(&shared, i) },
                    None => return,
                },
            }
        }
    }
}
