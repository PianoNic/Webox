//! X11 without the ScreenCast portal: the screen and the top-level windows from the window manager's client list (EWMH),
//! Webox's own windows left out, thumbnails through GetImage. `ximagesrc` captures the chosen one.
use super::super::{convert, Kind, ShareSource};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, ImageFormat, Window};
use x11rb::rust_connection::RustConnection;

struct X { c: RustConnection, root: Window, size: (u16, u16) }

fn connect() -> Option<X> {
    let (c, n) = x11rb::connect(None).ok()?;
    let s = &c.setup().roots[n];
    let (root, size) = (s.root, (s.width_in_pixels, s.height_in_pixels));
    Some(X { c, root, size })
}

impl X {
    fn atom(&self, name: &str) -> u32 { self.c.intern_atom(false, name.as_bytes()).ok().and_then(|c| c.reply().ok()).map_or(0, |r| r.atom) }
    fn prop32(&self, w: Window, name: &str, ty: impl Into<u32>, len: u32) -> Vec<u32> {
        self.c.get_property(false, w, self.atom(name), ty, 0, len).ok().and_then(|c| c.reply().ok()).and_then(|r| r.value32().map(|v| v.collect())).unwrap_or_default()
    }
    fn text(&self, w: Window, prop: u32, ty: u32) -> Option<Vec<u8>> {
        let r = self.c.get_property(false, w, prop, ty, 0, 1024).ok()?.reply().ok()?;
        (!r.value.is_empty()).then_some(r.value)
    }
}

/// An X server is reachable (DISPLAY set, also under XWayland).
pub(super) fn present() -> bool { std::env::var_os("DISPLAY").is_some() && connect().is_some() }

pub(super) fn sources() -> Vec<ShareSource> {
    let Some(x) = connect() else { return vec![] };
    let mut v = vec![ShareSource { kind: Kind::Screen, handle: 0, index: 0, name: format!("{} 1", crate::trq!("Screen")), app: String::new(),
        size: (x.size.0 as u32, x.size.1 as u32), primary: true, minimized: false }];
    let me = std::process::id();
    let (utf8, net_name, hidden) = (x.atom("UTF8_STRING"), x.atom("_NET_WM_NAME"), x.atom("_NET_WM_STATE_HIDDEN"));
    let mut list = x.prop32(x.root, "_NET_CLIENT_LIST_STACKING", AtomEnum::WINDOW, 4096);
    list.reverse(); // topmost first
    for w in list {
        if x.prop32(w, "_NET_WM_PID", AtomEnum::CARDINAL, 1).first() == Some(&me) { continue; }
        let Some(name) = x.text(w, net_name, utf8).or_else(|| x.text(w, AtomEnum::WM_NAME.into(), AtomEnum::STRING.into())) else { continue };
        let app = x.text(w, AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into())
            .and_then(|c| c.split(|&b| b == 0).nth(1).map(|s| String::from_utf8_lossy(s).into_owned())).unwrap_or_default();
        let Some(g) = x.c.get_geometry(w).ok().and_then(|c| c.reply().ok()) else { continue };
        if g.width < 40 || g.height < 40 { continue; }
        let minimized = x.prop32(w, "_NET_WM_STATE", AtomEnum::ATOM, 32).contains(&hidden);
        v.push(ShareSource { kind: Kind::Window, handle: w as isize, index: 0, name: String::from_utf8_lossy(&name).into_owned(), app,
            size: (g.width as u32, g.height as u32), primary: false, minimized });
    }
    v
}

pub(super) fn thumbnail(s: &ShareSource, max: (u32, u32)) -> Option<(u32, u32, Vec<u8>)> {
    let x = connect()?;
    let w = if s.handle == 0 { x.root } else { s.handle as Window };
    let g = x.c.get_geometry(w).ok()?.reply().ok()?;
    let img = x.c.get_image(ImageFormat::Z_PIXMAP, w, 0, 0, g.width, g.height, !0).ok()?.reply().ok()?;
    let (pw, ph) = (g.width as usize, g.height as usize);
    if img.data.len() < pw * ph * 4 { return None; } // 24/32-bit TrueColor only
    Some(convert::thumb(convert::Picture { px: &img.data, w: pw as u32, h: ph as u32, stride: pw * 4, bgra: true }, max))
}
