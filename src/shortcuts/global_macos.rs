//! Global hotkeys on macOS: Carbon's `RegisterEventHotKey` (still the supported way; needs no accessibility permission). The
//! hotkeys are registered on the main thread, where eframe runs the UI, and the press arrives there through the application
//! event target; the handler queues the command and wakes the window. Opt-in like on Linux (Settings > Tastenkombinationen),
//! since the keys are then taken from every other app. A binding's Ctrl is Cmd on macOS (as everywhere in the app).
use super::Combo;
use eframe::egui::{self, Key};
use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct Handle { active: Vec<*mut c_void>, installed: bool }

/// Pressed hotkeys (command indices), the commands the system accepted, and the window to wake.
static FIRED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static REGISTERED: Mutex<Option<HashSet<usize>>> = Mutex::new(None);
static CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

#[repr(C)]
struct EventTypeSpec { class: u32, kind: u32 }
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct EventHotKeyId { signature: u32, id: u32 }

type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;
#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(target: *mut c_void, handler: Handler, count: u32, list: *const EventTypeSpec, user: *mut c_void, out: *mut *mut c_void) -> i32;
    fn RegisterEventHotKey(code: u32, modifiers: u32, id: EventHotKeyId, target: *mut c_void, options: u32, out: *mut *mut c_void) -> i32;
    fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
    fn GetEventParameter(event: *mut c_void, name: u32, ty: u32, out_ty: *mut u32, size: usize, out_size: *mut usize, data: *mut c_void) -> i32;
}

const fn four(s: &[u8; 4]) -> u32 { u32::from_be_bytes(*s) }
const K_EVENT_CLASS_KEYBOARD: u32 = four(b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = four(b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = four(b"hkid");
const SIGNATURE: u32 = four(b"WBOX");
// Carbon modifier bits.
const CMD_KEY: u32 = 0x0100;
const SHIFT_KEY: u32 = 0x0200;
const OPTION_KEY: u32 = 0x0800;

/// The virtual key code (`kVK_*`, US layout positions) of a key; None: no global hotkey for it.
pub(crate) fn key_code(k: Key) -> Option<u32> {
    use Key::*;
    Some(match k {
        A => 0x00, S => 0x01, D => 0x02, F => 0x03, H => 0x04, G => 0x05, Z => 0x06, X => 0x07, C => 0x08, V => 0x09, B => 0x0B, Q => 0x0C,
        W => 0x0D, E => 0x0E, R => 0x0F, Y => 0x10, T => 0x11, O => 0x1F, U => 0x20, I => 0x22, P => 0x23, L => 0x25, J => 0x26, K => 0x28,
        N => 0x2D, M => 0x2E,
        Num1 => 0x12, Num2 => 0x13, Num3 => 0x14, Num4 => 0x15, Num6 => 0x16, Num5 => 0x17, Num9 => 0x19, Num7 => 0x1A, Num8 => 0x1C, Num0 => 0x1D,
        Equals | Plus => 0x18, Minus => 0x1B, CloseBracket => 0x1E, OpenBracket => 0x21, Quote => 0x27, Semicolon => 0x29, Backslash => 0x2A,
        Comma => 0x2B, Slash => 0x2C, Period => 0x2F, Backtick => 0x32,
        Enter => 0x24, Tab => 0x30, Space => 0x31, Backspace => 0x33, Escape => 0x35, Delete => 0x75, Home => 0x73, End => 0x77,
        PageUp => 0x74, PageDown => 0x79, ArrowLeft => 0x7B, ArrowRight => 0x7C, ArrowDown => 0x7D, ArrowUp => 0x7E,
        F1 => 0x7A, F2 => 0x78, F3 => 0x63, F4 => 0x76, F5 => 0x60, F6 => 0x61, F7 => 0x62, F8 => 0x64, F9 => 0x65, F10 => 0x6D, F11 => 0x67, F12 => 0x6F,
        _ => return None,
    })
}

/// Carbon modifiers of a binding: its Ctrl is Cmd.
pub(crate) fn modifiers(c: &Combo) -> u32 {
    (if c.ctrl { CMD_KEY } else { 0 }) | (if c.alt { OPTION_KEY } else { 0 }) | (if c.shift { SHIFT_KEY } else { 0 })
}

extern "C" fn on_hotkey(_call: *mut c_void, event: *mut c_void, _user: *mut c_void) -> i32 {
    let mut id = EventHotKeyId::default();
    let ok = unsafe { GetEventParameter(event, K_EVENT_PARAM_DIRECT_OBJECT, TYPE_EVENT_HOT_KEY_ID, std::ptr::null_mut(),
        std::mem::size_of::<EventHotKeyId>(), std::ptr::null_mut(), &mut id as *mut _ as *mut c_void) } == 0;
    if ok && id.signature == SIGNATURE && id.id > 0 {
        FIRED.lock().unwrap().push(id.id as usize - 1);
        if let Some(c) = CTX.lock().unwrap().as_ref() { c.request_repaint(); }
    }
    0
}

impl Handle {
    /// Replaces the set of global hotkeys. Called on the UI (main) thread.
    pub(crate) fn set(&mut self, ctx: &egui::Context, want: Vec<(usize, Combo)>) {
        *CTX.lock().unwrap() = Some(ctx.clone());
        for h in self.active.drain(..) { unsafe { UnregisterEventHotKey(h); } }
        let mut ok = HashSet::new();
        if !want.is_empty() && !self.installed {
            let spec = EventTypeSpec { class: K_EVENT_CLASS_KEYBOARD, kind: K_EVENT_HOT_KEY_PRESSED };
            self.installed = unsafe { InstallEventHandler(GetApplicationEventTarget(), on_hotkey, 1, &spec, std::ptr::null_mut(), std::ptr::null_mut()) } == 0;
        }
        if self.installed {
            for (idx, c) in want {
                let Some(code) = key_code(c.key) else { continue };
                let mut h = std::ptr::null_mut();
                let id = EventHotKeyId { signature: SIGNATURE, id: idx as u32 + 1 };
                // Fails when another app holds the same combination: that command stays in-app.
                if unsafe { RegisterEventHotKey(code, modifiers(&c), id, GetApplicationEventTarget(), 0, &mut h) } == 0 && !h.is_null() {
                    self.active.push(h);
                    ok.insert(idx);
                }
            }
        }
        *REGISTERED.lock().unwrap() = Some(ok);
    }
    pub(crate) fn drain(&mut self) -> Vec<usize> { std::mem::take(&mut *FIRED.lock().unwrap()) }
    pub(crate) fn is_registered(&self, idx: usize) -> bool { REGISTERED.lock().unwrap().as_ref().is_some_and(|r| r.contains(&idx)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_map_to_us_key_codes_and_cmd() {
        let c = Combo::parse("Ctrl+Shift+M").unwrap();
        assert_eq!((key_code(c.key), modifiers(&c)), (Some(0x2E), CMD_KEY | SHIFT_KEY));
        let c = Combo::parse("Ctrl+Alt+1").unwrap();
        assert_eq!((key_code(c.key), modifiers(&c)), (Some(0x12), CMD_KEY | OPTION_KEY));
        assert_eq!(key_code(Key::F5), Some(0x60));
        assert_eq!(key_code(Key::A), Some(0)); // kVK_ANSI_A is 0: a valid code
        // every letter and digit has its own code
        let all: HashSet<u32> = super::super::LETTERS.iter().chain(super::super::DIGITS.iter()).filter_map(|k| key_code(*k)).collect();
        assert_eq!(all.len(), 36);
        assert_eq!(SIGNATURE, 0x57424F58);
    }
}
