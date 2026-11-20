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
pub enum TrayEvent { Left, Menu(i32, i32), Item(u32) }

// Icon pixels (platform independent).

/// The app icon as straight RGBA at `size` px.
pub fn app_icon_rgba(size: u32) -> Vec<u8> {
    // Use the icon's own frame of that size, resize only when `size` isn't one of them.
    let img = image::load_from_memory(crate::icons::app_icon_frame(size)).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(size, size));
    if img.width() == size && img.height() == size { img.into_raw() }
    else { image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3).into_raw() }
}

use resvg::tiny_skia::{Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

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

/// Seven-segment strokes of a digit inside a w x h box at (x, y): (x0, y0, x1, y1) lines.
fn digit_lines(d: u8, x: f32, y: f32, w: f32, h: f32) -> Vec<(f32, f32, f32, f32)> {
    let (l, r, t, m, b) = (x, x + w, y, y + h / 2.0, y + h);
    let seg = |c: char| match c { 'a' => (l, t, r, t), 'b' => (r, t, r, m), 'c' => (r, m, r, b), 'd' => (l, b, r, b), 'e' => (l, m, l, b), 'f' => (l, t, l, m), _ => (l, m, r, m) };
    let on = ["abcdef", "bc", "abdeg", "abcdg", "bcfg", "acdfg", "acdefg", "abc", "abcdefg", "abcdfg"];
    on[(d % 10) as usize].chars().map(seg).collect()
}

/// The taskbar overlay badge: a red disc with the unread count in white (1-9, "9+" above).
pub fn badge_rgba(n: usize, size: u32) -> Vec<u8> {
    let Some(mut pm) = Pixmap::new(size, size) else { return vec![0; (size * size * 4) as usize] };
    let s = size as f32;
    if let Some(disc) = PathBuilder::from_circle(s / 2.0, s / 2.0, s / 2.0 - 0.5) { pm.fill_path(&disc, &paint(RED.0, RED.1, RED.2), FillRule::Winding, Transform::identity(), None); }
    let mut lines = vec![];
    if n <= 9 { lines = digit_lines(n as u8, s * 0.34, s * 0.22, s * 0.32, s * 0.56); }
    else {
        lines.extend(digit_lines(9, s * 0.18, s * 0.28, s * 0.24, s * 0.44));
        let (px, py, pr) = (s * 0.70, s * 0.50, s * 0.11);
        lines.extend([(px - pr, py, px + pr, py), (px, py - pr, px, py + pr)]);
    }
    let white = paint(255, 255, 255);
    let st = Stroke { width: s * 0.085, line_cap: LineCap::Round, ..Default::default() };
    for (x0, y0, x1, y1) in lines {
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0); pb.line_to(x1, y1);
        if let Some(p) = pb.finish() { pm.stroke_path(&p, &white, &st, Transform::identity(), None); }
    }
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

    pub fn inject(_: TrayEvent) {}
    /// Hidden to the tray = the window is unmapped (winit keeps running the app's frames on X11 and Wayland).
    pub fn hide_to_tray(_: isize) {}
    pub fn restore_from_tray(_: isize) -> bool { false }
    /// The urgency hint / xdg-activation request goes through winit (`notify.rs`); nothing to do with the raw window here.
    pub fn flash(_: isize, _: bool) {}
    /// Unread count on the launcher / task manager entry (Unity LauncherEntry over D-Bus).
    pub fn overlay(_: isize, unread: usize) { linux::launcher::set_count(unread); }
    pub fn work_area() -> Option<(i32, i32, i32, i32)> { x11::work_area() }
    pub fn find_window(title: &str) -> Option<isize> { x11::find_window(title) }
    pub fn make_toast_window(h: isize) { x11::make_toast_window(h) }
    pub fn place(h: isize, x: i32, y: i32, w: i32, hh: i32) { x11::place(h, x, y, w, hh) }
    pub fn dpi_scale(_: isize) -> f32 { linux::scale() }
    pub fn cursor() -> (i32, i32) { x11::cursor().unwrap_or((0, 0)) }
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
    pub fn inject(_: TrayEvent) {}
    pub fn hide_to_tray(_: isize) {}
    pub fn restore_from_tray(_: isize) -> bool { false }
    pub fn flash(_: isize, _: bool) {}
    pub fn overlay(_: isize, _: usize) {}
    pub fn work_area() -> Option<(i32, i32, i32, i32)> { None }
    pub fn find_window(_: &str) -> Option<isize> { None }
    pub fn make_toast_window(_: isize) {}
    pub fn place(_: isize, _: i32, _: i32, _: i32, _: i32) {}
    pub fn dpi_scale(_: isize) -> f32 { 1.0 }
    pub fn cursor() -> (i32, i32) { (0, 0) }
    pub fn window_rect(_: isize) -> Option<(i32, i32, i32, i32)> { None }
    pub fn capture(_: isize, _: &Path) -> bool { false }
    pub fn see_through(_: isize) {}
    pub fn composited() -> bool { true }
    pub fn popup_shape(_: isize, _: &[(i32, i32, i32, i32)], _: i32) {}
}
#[cfg(not(any(windows, target_os = "linux")))]
pub use stub::*;
