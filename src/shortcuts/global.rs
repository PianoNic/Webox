//! Global hotkeys (the "Global" checkbox): work while the app is minimised or in the background.
//! A dedicated thread owns the hotkeys (`RegisterHotKey` with a null window binds them to the calling thread's message queue)
//! and runs a message loop; it posts the command index to the app and wakes the UI (`request_repaint`).
//! The UI thread sends the wanted set with `set` (stored, then a WM_APP message wakes the loop, which re-registers).
use super::Combo;
use eframe::egui::{self, Key};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, PostThreadMessageW, MSG, WM_APP, WM_HOTKEY};

#[derive(Default)]
struct Shared {
    wanted: Vec<(usize, Combo)>,
    /// Command indices whose hotkey the system accepted (others stay in-app: another program owns them).
    registered: HashSet<usize>,
    fired: Vec<usize>,
    thread: u32,
    ctx: Option<egui::Context>,
}

#[derive(Default)]
pub(crate) struct Handle { shared: Arc<Mutex<Shared>>, started: bool }

/// Windows virtual-key code of a key (None: no global hotkey for it).
fn vk(k: Key) -> Option<u32> {
    use Key::*;
    Some(match k {
        A => 0x41, B => 0x42, C => 0x43, D => 0x44, E => 0x45, F => 0x46, G => 0x47, H => 0x48, I => 0x49, J => 0x4A, K => 0x4B, L => 0x4C, M => 0x4D,
        N => 0x4E, O => 0x4F, P => 0x50, Q => 0x51, R => 0x52, S => 0x53, T => 0x54, U => 0x55, V => 0x56, W => 0x57, X => 0x58, Y => 0x59, Z => 0x5A,
        Num0 => 0x30, Num1 => 0x31, Num2 => 0x32, Num3 => 0x33, Num4 => 0x34, Num5 => 0x35, Num6 => 0x36, Num7 => 0x37, Num8 => 0x38, Num9 => 0x39,
        F1 => 0x70, F2 => 0x71, F3 => 0x72, F4 => 0x73, F5 => 0x74, F6 => 0x75, F7 => 0x76, F8 => 0x77, F9 => 0x78, F10 => 0x79, F11 => 0x7A, F12 => 0x7B,
        Enter => 0x0D, Space => 0x20, Tab => 0x09, Backspace => 0x08, Delete => 0x2E, Insert => 0x2D, Home => 0x24, End => 0x23, PageUp => 0x21, PageDown => 0x22,
        ArrowLeft => 0x25, ArrowUp => 0x26, ArrowRight => 0x27, ArrowDown => 0x28,
        Comma => 0xBC, Period => 0xBE, Minus => 0xBD, Plus | Equals => 0xBB, Slash => 0xBF, Semicolon => 0xBA, Quote => 0xDE, Backslash => 0xDC,
        OpenBracket => 0xDB, CloseBracket => 0xDD, Backtick => 0xC0,
        _ => return None,
    })
}

impl Handle {
    /// Replaces the set of global hotkeys (starts the thread on first use).
    pub(crate) fn set(&mut self, ctx: &egui::Context, want: Vec<(usize, Combo)>) {
        if !self.started {
            if want.is_empty() { return; }
            self.started = true;
            let shared = self.shared.clone();
            shared.lock().unwrap().ctx = Some(ctx.clone());
            std::thread::Builder::new().name("webox-hotkeys".into()).spawn(move || run(shared)).ok();
        }
        let thread = { let mut s = self.shared.lock().unwrap(); s.wanted = want; s.thread };
        if thread != 0 { unsafe { let _ = PostThreadMessageW(thread, WM_APP, WPARAM(0), LPARAM(0)); } }
        // (If the thread has not published its id yet it reads `wanted` before its first GetMessageW.)
    }
    pub(crate) fn drain(&mut self) -> Vec<usize> { std::mem::take(&mut self.shared.lock().unwrap().fired) }
    pub(crate) fn is_registered(&self, idx: usize) -> bool { self.shared.lock().unwrap().registered.contains(&idx) }
}

fn run(shared: Arc<Mutex<Shared>>) {
    let tid = unsafe { GetCurrentThreadId() };
    shared.lock().unwrap().thread = tid;
    let mut active: Vec<i32> = Vec::new(); // registered hotkey ids (id = command index + 1)
    let sync = |shared: &Arc<Mutex<Shared>>, active: &mut Vec<i32>| {
        for id in active.drain(..) { unsafe { let _ = UnregisterHotKey(HWND(std::ptr::null_mut()), id); } }
        let wanted = shared.lock().unwrap().wanted.clone();
        let mut ok = HashSet::new();
        for (idx, c) in wanted {
            let Some(code) = vk(c.key) else { continue };
            let mut m = MOD_NOREPEAT;
            if c.ctrl { m |= MOD_CONTROL } if c.alt { m |= MOD_ALT } if c.shift { m |= MOD_SHIFT }
            let id = idx as i32 + 1;
            if unsafe { RegisterHotKey(HWND(std::ptr::null_mut()), id, HOT_KEY_MODIFIERS(m.0), code) }.is_ok() { active.push(id); ok.insert(idx); }
        }
        shared.lock().unwrap().registered = ok;
    };
    sync(&shared, &mut active);
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, HWND(std::ptr::null_mut()), 0, 0) }.0 > 0 {
        if msg.message == WM_HOTKEY {
            let idx = (msg.wParam.0 as i32 - 1).max(0) as usize;
            let ctx = { let mut s = shared.lock().unwrap(); s.fired.push(idx); s.ctx.clone() };
            if let Some(c) = ctx { c.request_repaint(); }
        } else if msg.message == WM_APP {
            sync(&shared, &mut active);
            if let Some(c) = shared.lock().unwrap().ctx.clone() { c.request_repaint(); }
        }
    }
}
