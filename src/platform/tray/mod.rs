//! System tray icon, taskbar flash / overlay badge and the Win32 plumbing of the toast windows - raw Windows API
//! (`windows` crate: Shell_NotifyIconW, a hidden message window, FlashWindowEx, ITaskbarList3), no tray crates.
//!
//! The tooltip carries the unread count ("ungelesene Benachrichtigung(en)"), an unread icon variant is rendered at run time
//! and the taskbar button flashes (setting "Benachrichtigungen in der Taskleiste").
//! The Win32 part is cfg(windows); Linux has native counterparts in `src/platform/linux/` (StatusNotifierItem tray, launcher badge,
//! X11 window helpers); other platforms get no-op stubs.
use std::path::Path;

/// What the tray icon reports back to the app. `Item(i)`: row i of the tray menu was picked in the panel's own menu (Linux dbusmenu).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrayEvent { Left, Menu(i32, i32), #[cfg(target_os = "linux")] Item(u32) }

// Icon pixels (platform independent).

/// The app icon as straight RGBA at `size` px.
pub fn app_icon_rgba(size: u32) -> Vec<u8> {
    // Use the icon's own frame of that size, resize only when `size` isn't one of them.
    let img = image::load_from_memory(crate::icons::app_icon_frame(size)).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(size, size));
    if img.width() == size && img.height() == size { img.into_raw() }
    else { image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3).into_raw() }
}

use resvg::tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

fn paint(r: u8, g: u8, b: u8) -> Paint<'static> { let mut p = Paint::default(); p.set_color(Color::from_rgba8(r, g, b, 255)); p.anti_alias = true; p }
const RED: (u8, u8, u8) = (0xD4, 0x35, 0x19); // Th.badge, the app's unread-badge red

fn pixmap_to_rgba(p: &Pixmap) -> Vec<u8> {
    p.pixels().iter().flat_map(|c| { let d = c.demultiply(); [d.red(), d.green(), d.blue(), d.alpha()] }).collect()
}

/// The app icon with the red unread badge in its bottom-right corner.
pub fn unread_icon_rgba(size: u32) -> Vec<u8> {
    let base = app_icon_rgba(size);
    let Some(mut pm) = Pixmap::new(size, size) else { return base };
    for (d, s) in pm.pixels_mut().iter_mut().zip(base.chunks(4)) {
        let a = s[3] as f32 / 255.0;
        let pr = |v: u8| (v as f32 * a).round() as u8;
        *d = resvg::tiny_skia::PremultipliedColorU8::from_rgba(pr(s[0]), pr(s[1]), pr(s[2]), s[3]).unwrap_or(*d);
    }
    let s = size as f32;
    let (cx, cy, r) = (s * 0.70, s * 0.70, s * 0.28);
    if let Some(ring) = PathBuilder::from_circle(cx, cy, r + s * 0.05) { pm.fill_path(&ring, &paint(255, 255, 255), FillRule::Winding, Transform::identity(), None); }
    if let Some(dot) = PathBuilder::from_circle(cx, cy, r) { pm.fill_path(&dot, &paint(RED.0, RED.1, RED.2), FillRule::Winding, Transform::identity(), None); }
    pixmap_to_rgba(&pm)
}

/// Draws `text` in white (Selawik Semibold, `px` high), centred on its ink at (cx, cy).
#[cfg(windows)]
fn draw_text(pm: &mut Pixmap, text: &str, px: f32, cx: f32, cy: f32) {
    use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
    let Ok(font) = FontRef::try_from_slice(include_bytes!("../../../assets/fonts/selawksb.ttf")) else { return };
    let sf = font.as_scaled(PxScale::from(px));
    let (mut x, mut prev, mut outlines) = (0.0, None, vec![]);
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev { x += sf.kern(p, id); }
        if let Some(o) = font.outline_glyph(id.with_scale_and_position(px, point(x, 0.0))) { outlines.push(o); }
        x += sf.h_advance(id);
        prev = Some(id);
    }
    let Some(first) = outlines.first() else { return };
    let mut b = first.px_bounds();
    for o in &outlines[1..] { let r = o.px_bounds(); b.min.x = b.min.x.min(r.min.x); b.min.y = b.min.y.min(r.min.y); b.max.x = b.max.x.max(r.max.x); b.max.y = b.max.y.max(r.max.y); }
    let (dx, dy) = ((cx - (b.min.x + b.max.x) / 2.0).round(), (cy - (b.min.y + b.max.y) / 2.0).round());
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let pixels = pm.pixels_mut();
    for o in &outlines {
        let r = o.px_bounds();
        o.draw(|gx, gy, c| {
            let (x, y) = ((r.min.x + dx) as i32 + gx as i32, (r.min.y + dy) as i32 + gy as i32);
            if x < 0 || y < 0 || x >= w || y >= h { return; }
            let p = &mut pixels[(y * w + x) as usize];
            let c = c.clamp(0.0, 1.0);
            let mix = |v: u8| (255.0 * c + v as f32 * (1.0 - c)).round() as u8;
            if let Some(n) = resvg::tiny_skia::PremultipliedColorU8::from_rgba(mix(p.red()), mix(p.green()), mix(p.blue()), mix(p.alpha())) { *p = n; }
        });
    }
}

/// The taskbar overlay badge: a red disc with the unread count in white (1-9, "9+" above).
#[cfg(windows)]
pub fn badge_rgba(n: usize, size: u32) -> Vec<u8> {
    let Some(mut pm) = Pixmap::new(size, size) else { return vec![0; (size * size * 4) as usize] };
    let s = size as f32;
    if let Some(disc) = PathBuilder::from_circle(s / 2.0, s / 2.0, s / 2.0 - 0.5) { pm.fill_path(&disc, &paint(RED.0, RED.1, RED.2), FillRule::Winding, Transform::identity(), None); }
    if n <= 9 { draw_text(&mut pm, &n.to_string(), s * 0.82, s / 2.0, s / 2.0); }
    else { draw_text(&mut pm, "9+", s * 0.62, s / 2.0, s / 2.0); }
    pixmap_to_rgba(&pm)
}

/// "N ungelesene Benachrichtigung(en)" as the German pack has it; the app name when nothing is unread.
pub fn tooltip(n: usize) -> String {
    match n { 0 => "Webox".into(), 1 => "1 ungelesene Benachrichtigung".into(), n => format!("{n} ungelesene Benachrichtigungen") }
}

pub fn save_png(rgba: Vec<u8>, w: u32, h: u32, path: &Path) {
    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    if let Some(img) = image::RgbaImage::from_raw(w, h, rgba) { let _ = img.save(path); }
}

// Windows.

#[cfg(windows)]
mod imp;

#[cfg(windows)]
pub use imp::*;

// Linux (X11 + Wayland): src/platform/linux/.
#[cfg(target_os = "linux")]
mod imp_linux {
    use super::*;
    use crate::platform::linux::{self, x11};
    pub use crate::platform::linux::sni::{MenuRow, Tray};

    /// Hidden to the tray = the window is unmapped (winit keeps running the app's frames on X11 and Wayland).
    pub fn hide_to_tray(_: isize) {}
    pub fn restore_from_tray(_: isize) -> bool { false }
    /// Unread count on the launcher / task manager entry (Unity LauncherEntry over D-Bus).
    pub fn overlay(_: isize, unread: usize) { linux::launcher::set_count(unread); }
    pub fn work_area() -> Option<(i32, i32, i32, i32)> { x11::work_area() }
    pub fn find_window(title: &str) -> Option<isize> { x11::find_window(title) }
    pub fn make_toast_window(h: isize) { x11::make_toast_window(h) }
    pub fn place(h: isize, x: i32, y: i32, w: i32, hh: i32) { x11::place(h, x, y, w, hh) }
    pub fn dpi_scale(_: isize) -> f32 { linux::scale() }
    pub fn window_rect(h: isize) -> Option<(i32, i32, i32, i32)> { x11::window_rect(h) }
    pub fn capture(h: isize, path: &Path) -> bool { x11::capture(h, path) }
    /// Per-pixel alpha needs no extra call here: the root viewport's ARGB visual (main.rs) keeps the popups' transparency.
    pub fn see_through(_: isize) {}
    /// Wayland always composites; X11 only with a compositing manager.
    pub fn composited() -> bool { !linux::is_x11() || x11::composited() }
    /// Without a compositor (X11): the popup's window is cut to its card / menu rectangles (physical px), see x11::shape.
    pub fn popup_shape(h: isize, rects: &[(i32, i32, i32, i32)], r: i32) { if linux::is_x11() && !x11::composited() { x11::shape(h, rects, r) } }
}
#[cfg(target_os = "linux")]
pub use imp_linux::*;

// Other platforms: no-ops.
#[cfg(not(any(windows, target_os = "linux")))]
mod stub {
    use super::*;
    pub struct Tray;
    impl Tray {
        pub fn new(_: &eframe::egui::Context) -> Option<Tray> { None }
        pub fn set_unread(&mut self, _: usize) {}
        pub fn poll(&mut self) -> Vec<TrayEvent> { vec![] }
    }
    pub fn hide_to_tray(_: isize) {}
    pub fn restore_from_tray(_: isize) -> bool { false }
    pub fn overlay(_: isize, _: usize) {}
    pub fn work_area() -> Option<(i32, i32, i32, i32)> { None }
    pub fn find_window(_: &str) -> Option<isize> { None }
    pub fn make_toast_window(_: isize) {}
    pub fn place(_: isize, _: i32, _: i32, _: i32, _: i32) {}
    pub fn dpi_scale(_: isize) -> f32 { 1.0 }
    pub fn window_rect(_: isize) -> Option<(i32, i32, i32, i32)> { None }
    pub fn capture(_: isize, _: &Path) -> bool { false }
    pub fn see_through(_: isize) {}
    pub fn composited() -> bool { true }
    pub fn popup_shape(_: isize, _: &[(i32, i32, i32, i32)], _: i32) {}
}
#[cfg(not(any(windows, target_os = "linux")))]
pub use stub::*;

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn badge_has_a_white_number_on_a_red_disc() {
        for (n, size) in [(1usize, 32u32), (3, 32), (12, 32), (1, 16), (7, 20)] {
            let px = badge_rgba(n, size);
            let at = |x: u32, y: u32| { let i = ((y * size + x) * 4) as usize; (px[i], px[i + 1], px[i + 2], px[i + 3]) };
            let white = px.chunks(4).filter(|p| p[0] > 230 && p[1] > 230 && p[2] > 230 && p[3] > 230).count();
            assert!(white > 0, "number drawn for {n} at {size}");
            let (r, g, b, a) = at(size / 2, 1);
            assert!(a > 0 && r > 150 && g < 90 && b < 60, "red rim for {n} at {size}");
            if let Ok(dir) = std::env::var("WEBOX_BADGE_PNG") { save_png(px, size, size, &Path::new(&dir).join(format!("badge-{n}-{size}.png"))); }
        }
    }
}