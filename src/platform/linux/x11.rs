//! X11 window plumbing for the toast / tray-menu windows (the Linux side of `tray.rs`' Win32 helpers), over the pure-Rust x11rb
//! connection: EWMH work area, finding our own top-level window by title, keeping it out of the taskbar and above the others,
//! exact placement in physical pixels, and the screen capture used by the toast test hook. All calls return None / do nothing
//! on Wayland (the main window decides the session, see `linux::note_window`).
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt, EventMask, ImageFormat, PropMode, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub(crate) struct X { pub c: RustConnection, pub root: Window, pub screen: (u16, u16, u8), atoms: HashMap<&'static str, u32> }

impl X {
    pub(crate) fn atom(&mut self, name: &'static str) -> u32 {
        if let Some(a) = self.atoms.get(name) { return *a; }
        let a = self.c.intern_atom(false, name.as_bytes()).ok().and_then(|c| c.reply().ok()).map_or(0, |r| r.atom);
        self.atoms.insert(name, a);
        a
    }
    fn prop32(&mut self, w: Window, name: &'static str, ty: u32, len: u32) -> Vec<u32> {
        let a = self.atom(name);
        self.c.get_property(false, w, a, ty, 0, len).ok().and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect())).unwrap_or_default()
    }
    fn title(&mut self, w: Window) -> Option<String> {
        let (name, utf8) = (self.atom("_NET_WM_NAME"), self.atom("UTF8_STRING"));
        for (prop, ty) in [(name, utf8), (AtomEnum::WM_NAME.into(), AtomEnum::STRING.into())] {
            if let Some(r) = self.c.get_property(false, w, prop, ty, 0, 256).ok().and_then(|c| c.reply().ok()) {
                if !r.value.is_empty() { return Some(String::from_utf8_lossy(&r.value).into_owned()); }
            }
        }
        None
    }
}

/// The shared connection; only on an X11 session (None before the main window told us, and on Wayland).
pub(crate) fn with<T>(f: impl FnOnce(&mut X) -> Option<T>) -> Option<T> {
    static CONN: OnceLock<Option<Mutex<X>>> = OnceLock::new();
    if !super::is_x11() { return None; }
    let m = CONN.get_or_init(|| {
        let (c, n) = x11rb::connect(None).ok()?;
        let s = &c.setup().roots[n];
        let (root, screen) = (s.root, (s.width_in_pixels, s.height_in_pixels, s.root_depth));
        Some(Mutex::new(X { c, root, screen, atoms: HashMap::new() }))
    }).as_ref()?;
    let mut g = m.lock().ok()?;
    f(&mut g)
}

/// _NET_WORKAREA of the first desktop (screen minus panels) in physical pixels: (left, top, right, bottom).
pub(crate) fn work_area() -> Option<(i32, i32, i32, i32)> {
    with(|x| {
        let root = x.root;
        let v = x.prop32(root, "_NET_WORKAREA", AtomEnum::CARDINAL.into(), 4);
        if v.len() >= 4 && v[2] > 0 && v[3] > 0 { return Some((v[0] as i32, v[1] as i32, (v[0] + v[2]) as i32, (v[1] + v[3]) as i32)); }
        Some((0, 0, x.screen.0 as i32, x.screen.1 as i32))
    })
}

/// Our own top-level window with this title (EWMH client list, or the window tree when no window manager runs).
pub(crate) fn find_window(title: &str) -> Option<isize> {
    with(|x| {
        let root = x.root;
        let pid = std::process::id();
        let mut cands = x.prop32(root, "_NET_CLIENT_LIST", AtomEnum::WINDOW.into(), 4096);
        if cands.is_empty() {
            let top = x.c.query_tree(root).ok()?.reply().ok()?.children;
            for w in top { cands.push(w); if let Some(r) = x.c.query_tree(w).ok().and_then(|c| c.reply().ok()) { cands.extend(r.children); } }
        }
        for w in cands {
            if x.title(w).as_deref() != Some(title) { continue; }
            let p = x.prop32(w, "_NET_WM_PID", AtomEnum::CARDINAL.into(), 1);
            if p.first().is_none_or(|&p| p == pid) { return Some(w as isize); }
        }
        None
    })
}

fn wm_state(x: &mut X, w: Window, add: bool, a: &'static str, b: &'static str) {
    let (st, a, b) = (x.atom("_NET_WM_STATE"), x.atom(a), x.atom(b));
    let ev = ClientMessageEvent::new(32, w, st, [u32::from(add), a, b, 1, 0]);
    let _ = x.c.send_event(false, x.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev);
}

/// No taskbar / pager entry, above the other windows, notification window type (the Win32 WS_EX_TOOLWINDOW | WS_EX_TOPMOST).
pub(crate) fn make_toast_window(h: isize) {
    with(|x| {
        let w = h as Window;
        let (ty, notif) = (x.atom("_NET_WM_WINDOW_TYPE"), x.atom("_NET_WM_WINDOW_TYPE_NOTIFICATION"));
        let _ = x.c.change_property32(PropMode::REPLACE, w, ty, AtomEnum::ATOM, &[notif]);
        wm_state(x, w, true, "_NET_WM_STATE_SKIP_TASKBAR", "_NET_WM_STATE_SKIP_PAGER");
        wm_state(x, w, true, "_NET_WM_STATE_ABOVE", "_NET_WM_STATE_STAYS_ON_TOP");
        let _ = x.c.flush();
        Some(())
    });
}

/// Moves / sizes a window in physical pixels.
pub(crate) fn place(h: isize, px: i32, py: i32, w: i32, hh: i32) {
    with(|x| {
        let aux = ConfigureWindowAux::new().x(px).y(py).width(w.max(1) as u32).height(hh.max(1) as u32);
        let _ = x.c.configure_window(h as Window, &aux);
        let _ = x.c.flush();
        Some(())
    });
}

pub(crate) fn cursor() -> Option<(i32, i32)> {
    with(|x| { let r = x.c.query_pointer(x.root).ok()?.reply().ok()?; Some((r.root_x as i32, r.root_y as i32)) })
}

/// The window's rectangle on the screen (left, top, right, bottom), without the window manager's frame.
pub(crate) fn window_rect(h: isize) -> Option<(i32, i32, i32, i32)> {
    with(|x| {
        let g = x.c.get_geometry(h as Window).ok()?.reply().ok()?;
        let t = x.c.translate_coordinates(h as Window, x.root, 0, 0).ok()?.reply().ok()?;
        Some((t.dst_x as i32, t.dst_y as i32, t.dst_x as i32 + g.width as i32, t.dst_y as i32 + g.height as i32))
    })
}

/// Before the main window exists: an X11 session (winit takes Wayland when WAYLAND_DISPLAY is set) whose screen offers a 32-bit
/// TrueColor (ARGB) visual. Only then may the root viewport ask for a transparent GL config: eframe picks ONE config for every
/// window, glutin-winit's `finalize_window` drops `with_transparent(true)` of a popup whose config has a 24-bit visual (a window's
/// visual is fixed when it is created), and eframe panics when no config matches. `WEBOX_OPAQUE_POPUPS=1` opts out.
pub(crate) fn argb_visual() -> bool {
    use x11rb::protocol::xproto::VisualClass;
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) || std::env::var_os("WEBOX_OPAQUE_POPUPS").is_some() { return false; }
    let Ok((c, n)) = x11rb::connect(None) else { return false };
    let Some(s) = c.setup().roots.get(n) else { return false };
    s.allowed_depths.iter().any(|d| d.depth == 32 && d.visuals.iter().any(|v| v.class == VisualClass::TRUE_COLOR))
}

/// A compositing manager owns `_NET_WM_CM_S0` (EWMH): only then does a popup's alpha channel show what's behind it.
/// Re-checked at most every 2 s (a compositor may start or stop while the app runs).
pub(crate) fn composited() -> bool {
    static LAST: Mutex<Option<(std::time::Instant, bool)>> = Mutex::new(None);
    let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, v)) = *g { if t.elapsed().as_secs_f32() < 2.0 { return v; } }
    let v = with(|x| { let a = x.atom("_NET_WM_CM_S0"); Some(x.c.get_selection_owner(a).ok()?.reply().ok()?.owner != 0) }).unwrap_or(false);
    *g = Some((std::time::Instant::now(), v));
    v
}

/// Without a compositor: cuts the window to these rectangles (window-relative physical pixels: x, y, w, h), corners rounded
/// with radius `r` (XShape bounding region, which also limits the input), so no black padding, gap or corner shows.
pub(crate) fn shape(h: isize, rects: &[(i32, i32, i32, i32)], r: i32) {
    use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
    use x11rb::protocol::xproto::{ClipOrdering, Rectangle};
    let mut out: Vec<Rectangle> = vec![];
    for &(x, y, w, hh) in rects {
        if w <= 0 || hh <= 0 { continue; }
        let r = r.min(w / 2).min(hh / 2).max(0);
        let rect = |x: i32, y: i32, w: i32, h: i32| Rectangle { x: x as i16, y: y as i16, width: w.max(0) as u16, height: h.max(0) as u16 };
        for i in 0..r { // one row per pixel in the rounded bands at the top and the bottom
            let d = r as f32 - i as f32 - 0.5;
            let inset = (r as f32 - (r as f32 * r as f32 - d * d).max(0.0).sqrt()).round() as i32;
            out.push(rect(x + inset, y + i, w - 2 * inset, 1));
            out.push(rect(x + inset, y + hh - 1 - i, w - 2 * inset, 1));
        }
        out.push(rect(x, y + r, w, hh - 2 * r));
    }
    with(|x| {
        x.c.shape_rectangles(SO::SET, SK::BOUNDING, ClipOrdering::UNSORTED, h as Window, 0, 0, &out).ok()?;
        let _ = x.c.flush();
        Some(())
    });
}

/// Copies the window's area of the screen (what the user sees) into a PNG - the toast test hook.
pub(crate) fn capture(h: isize, path: &Path) -> bool {
    let Some((l, t, r, b)) = window_rect(h) else { return false };
    with(|x| {
        let (sw, sh) = (x.screen.0 as i32, x.screen.1 as i32);
        let (l, t, r, b) = (l.clamp(0, sw), t.clamp(0, sh), r.clamp(0, sw), b.clamp(0, sh));
        if r <= l || b <= t { return None; }
        let (w, hh) = ((r - l) as u16, (b - t) as u16);
        let img = x.c.get_image(ImageFormat::Z_PIXMAP, x.root, l as i16, t as i16, w, hh, !0).ok()?.reply().ok()?;
        if img.data.len() < w as usize * hh as usize * 4 { return None; } // 24/32-bit TrueColor only
        let px: Vec<u8> = img.data.chunks(4).take(w as usize * hh as usize).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
        crate::platform::tray::save_png(px, w as u32, hh as u32, path);
        Some(true)
    }).unwrap_or(false)
}
