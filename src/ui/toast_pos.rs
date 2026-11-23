//! Tiny placement helper for the frameless always-on-top toast windows (notify.rs places its message toasts itself).
//! Everything is in egui points (= winit logical pixels).
use crate::*;

/// Gap between a toast window and the work-area edge, and between stacked toasts (pt).
pub(crate) const EDGE_GAP: f32 = 8.0;

/// The primary monitor's work area (screen minus taskbar) in points. Windows: `SPI_GETWORKAREA`; elsewhere, or when that
/// fails: the monitor size reported by the viewport minus an assumed 48 pt bar at the bottom.
pub(crate) fn work_area(ctx: &egui::Context) -> Rect {
    #[cfg(windows)]
    {
        use windows::Win32::{Foundation::RECT, UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS}};
        let ppp = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0).max(0.5);
        let mut r = RECT::default();
        let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut r as *mut RECT as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }.is_ok();
        if ok && r.right > r.left { return Rect::from_min_max(pos2(r.left as f32 / ppp, r.top as f32 / ppp), pos2(r.right as f32 / ppp, r.bottom as f32 / ppp)); }
    }
    let m = ctx.input(|i| i.viewport().monitor_size).unwrap_or(Vec2::new(1920.0, 1080.0));
    Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(m.x, (m.y - 48.0).max(100.0)))
}

/// Top-left position of toast number `index` (0 = lowest, stacking upwards) of window size `size` at the bottom-right
/// corner of the work area.
pub(crate) fn toast_anchor(ctx: &egui::Context, size: Vec2, index: usize) -> egui::Pos2 {
    let wa = work_area(ctx);
    pos2(wa.right() - EDGE_GAP - size.x, wa.bottom() - EDGE_GAP - (index as f32 + 1.0) * size.y - index as f32 * EDGE_GAP)
}

/// Windows 11 rounds the corners of framed windows only; for our borderless secondary windows ask DWM for the same
/// rounding the main window gets (see `round_corners` in main.rs). Finds the window by its exact title.
#[cfg(windows)]
pub(crate) fn round_by_title(title: &str) {
    use windows::{core::PCWSTR, Win32::{Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE}, UI::WindowsAndMessaging::FindWindowW}};
    let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if let Ok(h) = FindWindowW(PCWSTR::null(), PCWSTR::from_raw(wide.as_ptr())) {
            let pref: i32 = 2; // DWMWCP_ROUND
            let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(33), &pref as *const i32 as *const _, 4);
        }
    }
}
#[cfg(not(windows))]
pub(crate) fn round_by_title(_title: &str) {}
