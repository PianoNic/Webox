//! The composer's "Screenshot" button (commands `ScreenCapture` / `ScreenCaptureNoMini` = "Screenshot" / "Screenshot ohne
//! miniaturisieren"): the app gets out of the way (minimised, unless "ohne miniaturisieren"), the monitor it was on is
//! captured, and a full-screen selection window lets the user drag the area to keep ("Wählen Sie einen Bereich des
//! Bildschirms aus, der erfasst werden soll,"; "Eingabetaste drücken, um gesamten Bildschirm zu erfassen"; "Abbrechen",
//! "Screenshot" under the selection; Esc cancels). The picture is staged in the composer as a PNG named "yyyy-MM-dd_HH-mm-ss"
//! (see `files::capture_name`).
//!
//! Capture per platform (`grab`, on a worker thread):
//! - Windows: GDI BitBlt of the monitor's rectangle (the same approach as `tray::capture`). A hidden or minimised window is never
//!   repainted on Windows (see tray.rs), so the minimise / capture / restore happens on the worker thread and the selection runs
//!   in an egui viewport once the main window is back. The overlay: dim 45 %, accent frame, pill buttons.
//! - Linux: the xdg-desktop-portal Screenshot interface (`interactive`), whose own dialog picks the area (the only way on
//!   Wayland, and what KDE / GNOME offer on X11 too). On X11 without a portal: GetImage of the root window and the overlay.
//! - macOS: `screencapture -i`, the system's own area picker.
//!
//! Every failure (no portal, access denied, cancelled in the desktop's dialog) ends in a short toast.
use crate::*;
use std::sync::Mutex;

#[derive(Default)]
struct Cap { space: String, img: Option<image::RgbaImage>, tex: Option<egui::TextureHandle>, drag: Option<(egui::Pos2, egui::Pos2)>, sel: Option<Rect>, busy: bool }
static CAP: Mutex<Option<Cap>> = Mutex::new(None);
/// Finished captures waiting for their composer: (space id, PNG bytes).
static DONE: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());
/// A failure of the worker thread, toasted by the next frame.
static NOTE: Mutex<Option<&'static str>> = Mutex::new(None);

/// What the platform capture gave: a screen for the selection overlay, an area the desktop already picked (PNG), or nothing.
#[allow(dead_code)] // every platform builds only some of them
enum Grab { Screen(image::RgbaImage), Png(Vec<u8>), Cancelled, Failed(&'static str) }
const CANCELLED: &str = "Screenshot abgebrochen.";
const FAILED: &str = "Screenshot fehlgeschlagen.";
#[cfg(target_os = "linux")]
const DENIED: &str = "Screenshot nicht möglich: Zugriff verweigert.";
#[cfg(target_os = "linux")]
const NO_PORTAL: &str = "Screenshot nicht möglich: Der Desktop bietet keinen Screenshot-Dienst (xdg-desktop-portal).";

/// The "Abbrechen" / "Screenshot" pills under (or, at the bottom edge, inside) the finished selection `s`, kept on screen.
pub(crate) fn pill_rects(s: Rect, view: Rect) -> (Rect, Rect) {
    let size = Vec2::new(110.0, 34.0);
    let y = if s.bottom() + 50.0 < view.bottom() { s.bottom() + 10.0 } else { (s.bottom() - 46.0).max(view.top()) };
    let x = (s.right() - size.x).clamp(view.left() + 4.0 + size.x + 8.0, (view.right() - 4.0 - size.x).max(view.left() + 122.0));
    let ok = Rect::from_min_size(pos2(x, y), size);
    (Rect::from_min_size(pos2(x - size.x - 8.0, y), size), ok)
}

/// Whether a press at `p` starts a new selection. Not on the pills of the finished one: the drag sensing area lies under them,
/// and egui hands a drag-only widget the drag on the press itself, which used to clear the selection (and with it the pills)
/// before the click could land.
pub(crate) fn starts_selection(p: egui::Pos2, sel: Option<Rect>, view: Rect) -> bool {
    sel.is_none_or(|s| { let (a, b) = pill_rects(s, view); !a.contains(p) && !b.contains(p) })
}

/// The captured PNG for `space`, once.
pub(crate) fn take_capture(space: &str) -> Option<Vec<u8>> {
    let mut d = DONE.lock().unwrap_or_else(|e| e.into_inner());
    let i = d.iter().position(|(s, _)| s == space)?;
    Some(d.remove(i).1)
}

/// Selection (in window points) -> pixel rectangle of the captured image (`img` px for a window of `view` points), clamped.
pub(crate) fn crop_px(sel: Rect, view: Rect, img: (u32, u32)) -> (u32, u32, u32, u32) {
    let (sx, sy) = (img.0 as f32 / view.width().max(1.0), img.1 as f32 / view.height().max(1.0));
    let x0 = ((sel.min.x - view.min.x) * sx).round().clamp(0.0, img.0 as f32) as u32;
    let y0 = ((sel.min.y - view.min.y) * sy).round().clamp(0.0, img.1 as f32) as u32;
    let x1 = ((sel.max.x - view.min.x) * sx).round().clamp(0.0, img.0 as f32) as u32;
    let y1 = ((sel.max.y - view.min.y) * sy).round().clamp(0.0, img.1 as f32) as u32;
    (x0, y0, x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1))
}

impl App {
    /// Toolbar button / shortcut. `minimize` false = "Screenshot ohne miniaturisieren".
    pub(crate) fn screen_capture_start(&mut self, space: &str, minimize: bool, now: f64) {
        if matches!(self.compose, Compose::Edit { .. }) { self.toast("Sie können keine Screenshots zu dieser Nachricht hinzufügen.", now); return; } // pack string
        if self.shot.is_some() { return; }
        {
            let mut g = CAP.lock().unwrap_or_else(|e| e.into_inner());
            if g.as_ref().is_some_and(|c| c.busy || c.img.is_some()) { return; }
            *g = Some(Cap { space: space.into(), busy: true, ..Default::default() });
        }
        let h = self.hwnd;
        std::thread::spawn(move || {
            let got = grab(h, minimize);
            let mut g = CAP.lock().unwrap_or_else(|e| e.into_inner());
            let note = match got {
                Grab::Screen(i) => { if let Some(c) = g.as_mut() { c.img = Some(i); c.busy = false; } None }
                Grab::Png(png) => { if let Some(c) = g.take() { DONE.lock().unwrap_or_else(|e| e.into_inner()).push((c.space, png)); } None }
                Grab::Cancelled => { *g = None; Some(CANCELLED) }
                Grab::Failed(m) => { *g = None; Some(m) }
            };
            drop(g);
            if note.is_some() { *NOTE.lock().unwrap_or_else(|e| e.into_inner()) = note; }
            crate::app::files::repaint();
        });
    }

    /// Every frame: toasts a failed capture, shows the selection window while a capture waits for its area.
    pub(crate) fn screen_capture_frame(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(m) = NOTE.lock().unwrap_or_else(|e| e.into_inner()).take() { self.toast(m, now); }
        let ready = CAP.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|c| c.img.is_some());
        if !ready { return; }
        let vp = egui::ViewportBuilder::default().with_title("Screenshot").with_decorations(false).with_always_on_top().with_taskbar(false)
            .with_fullscreen(true).with_active(true);
        let mut finish: Option<Option<Vec<u8>>> = None; // Some(Some(png)) captured, Some(None) cancelled
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("webox-screen-capture"), vp, |ctx, _| {
            let mut g = CAP.lock().unwrap_or_else(|e| e.into_inner());
            let Some(c) = g.as_mut() else { return };
            let Some(img) = c.img.as_ref() else { return };
            if c.tex.is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus); // keys (Enter / Esc) go to the overlay, not the main window
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                c.tex = Some(ctx.load_texture("screen-capture", ci, egui::TextureOptions::LINEAR));
            }
            let (iw, ih) = (img.width(), img.height());
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                let view = ui.max_rect();
                let resp = ui.interact(view, egui::Id::new("cap-area"), Sense::drag());
                let p = ui.painter();
                if let Some(t) = &c.tex { p.image(t.id(), view, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
                if resp.drag_started() { if let Some(q) = resp.interact_pointer_pos().filter(|&q| starts_selection(q, c.sel, view)) { c.drag = Some((q, q)); c.sel = None; } }
                if resp.dragged() { if let (Some((a, _)), Some(q)) = (c.drag, resp.interact_pointer_pos()) { c.drag = Some((a, q)); } }
                if resp.drag_stopped() { if let Some((a, b)) = c.drag.take() { let r = Rect::from_two_pos(a, b); if r.width() > 4.0 && r.height() > 4.0 { c.sel = Some(r); } } }
                let sel = c.drag.map(|(a, b)| Rect::from_two_pos(a, b)).or(c.sel);
                // Dim everything but the selection.
                let dim = Color32::from_black_alpha(115);
                match sel {
                    Some(s) => {
                        for r in [Rect::from_min_max(view.min, pos2(view.max.x, s.min.y)), Rect::from_min_max(pos2(view.min.x, s.max.y), view.max),
                            Rect::from_min_max(pos2(view.min.x, s.min.y), pos2(s.min.x, s.max.y)), Rect::from_min_max(pos2(s.max.x, s.min.y), pos2(view.max.x, s.max.y))] { p.rect_filled(r, 0.0, dim); }
                        p.rect_stroke(s, 0.0, Stroke::new(2.0_f32, hex(0x07C1F5)), egui::StrokeKind::Outside);
                        let (_, _, pw, ph) = crop_px(s, view, (iw, ih));
                        p.text(s.left_top() - Vec2::new(0.0, 6.0), egui::Align2::LEFT_BOTTOM, format!("{pw} × {ph}"), FontId::proportional(13.0), Color32::WHITE);
                    }
                    None => { p.rect_filled(view, 0.0, dim); }
                }
                // Hint at the top.
                let hint = "Wählen Sie einen Bereich des Bildschirms aus, der erfasst werden soll,\nEingabetaste drücken, um gesamten Bildschirm zu erfassen";
                let g2 = ui.fonts(|f| f.layout(hint.into(), FontId::proportional(15.0), Color32::WHITE, 700.0));
                let hr = Rect::from_center_size(pos2(view.center().x, view.top() + 40.0 + g2.size().y / 2.0), g2.size() + Vec2::new(32.0, 20.0));
                if sel.is_none_or(|s| !s.intersects(hr)) {
                    ui.painter().rect_filled(hr, 10.0, Color32::from_black_alpha(200));
                    ui.painter().galley(hr.min + Vec2::new(16.0, 10.0), g2, Color32::WHITE);
                }
                // Abbrechen / Screenshot under a finished selection.
                if let (Some(s), None) = (c.sel, c.drag) {
                    let (can_r, cap_r) = pill_rects(s, view);
                    let (a, b) = (ui.interact(can_r, egui::Id::new("cap-cancel"), Sense::click()), ui.interact(cap_r, egui::Id::new("cap-ok"), Sense::click()));
                    ui.painter().rect_filled(can_r, 17.0, if a.hovered() { hex(0x3B3B3B) } else { hex(0x262626) });
                    ui.painter().rect_stroke(can_r, 17.0, Stroke::new(1.0_f32, hex(0x7F7F7F)), egui::StrokeKind::Inside);
                    ui.painter().text(can_r.center(), egui::Align2::CENTER_CENTER, "Abbrechen", semibold(14.0), Color32::WHITE);
                    ui.painter().rect_filled(cap_r, 17.0, if b.hovered() { hex(0xDEDEDE) } else { hex(0xF2F3F1) });
                    ui.painter().text(cap_r.center(), egui::Align2::CENTER_CENTER, "Screenshot", semibold(14.0), hex(0x121212));
                    if a.clicked() { finish = Some(None); }
                    if b.clicked() { finish = Some(Some(crop_png(img, crop_px(s, view, (iw, ih))))); }
                }
                ctx.input(|i| {
                    if i.key_pressed(Key::Escape) { finish = Some(None); }
                    if i.key_pressed(Key::Enter) {
                        let s = c.sel.unwrap_or(view);
                        finish = Some(Some(crop_png(img, crop_px(s, view, (iw, ih)))));
                    }
                });
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            });
            if ctx.input(|i| i.viewport().close_requested()) { finish = Some(None); }
        });
        if let Some(r) = finish {
            let space = CAP.lock().unwrap_or_else(|e| e.into_inner()).take().map(|c| c.space).unwrap_or_default();
            if let Some(png) = r { DONE.lock().unwrap_or_else(|e| e.into_inner()).push((space, png)); }
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
        }
    }
}

fn crop_png(img: &image::RgbaImage, (x, y, w, h): (u32, u32, u32, u32)) -> Vec<u8> {
    let w = w.min(img.width().saturating_sub(x)).max(1);
    let h = h.min(img.height().saturating_sub(y)).max(1);
    let sub = image::imageops::crop_imm(img, x, y, w, h).to_image();
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = image::DynamicImage::ImageRgba8(sub).write_to(&mut out, image::ImageFormat::Png);
    out.into_inner()
}

/// Minimises the main window (unless `minimize` is false), copies its monitor, restores it.
#[cfg(windows)]
fn grab_gdi(main: Option<isize>, minimize: bool) -> Option<image::RgbaImage> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SetForegroundWindow, SW_MINIMIZE, SW_RESTORE};
    unsafe {
        let hw = main.map(|h| HWND(h as *mut c_void));
        let mon = MonitorFromWindow(hw.unwrap_or_default(), MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() { return None; }
        let r = mi.rcMonitor;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || h <= 0 { return None; }
        if minimize { if let Some(hw) = hw { let _ = ShowWindow(hw, SW_MINIMIZE); std::thread::sleep(std::time::Duration::from_millis(450)); } }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp);
        let ok = BitBlt(mem, 0, 0, w, h, screen, r.left, r.top, SRCCOPY | CAPTUREBLT).is_ok();
        let mut bmi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h,
            biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut px = vec![0u8; (w * h * 4) as usize];
        GetDIBits(mem, bmp, 0, h as u32, Some(px.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp); let _ = DeleteDC(mem); ReleaseDC(None, screen);
        if minimize { if let Some(hw) = hw { let _ = ShowWindow(hw, SW_RESTORE); let _ = SetForegroundWindow(hw); } }
        if !ok { return None; }
        for p in px.chunks_mut(4) { p.swap(0, 2); p[3] = 255; }
        image::RgbaImage::from_raw(w as u32, h as u32, px)
    }
}

#[cfg(windows)]
fn grab(main: Option<isize>, minimize: bool) -> Grab { grab_gdi(main, minimize).map_or(Grab::Failed(FAILED), Grab::Screen) }

/// Linux: the desktop's Screenshot portal (Wayland, and the X11 desktops that offer it); X11 without one grabs the root window.
#[cfg(target_os = "linux")]
fn grab(main: Option<isize>, minimize: bool) -> Grab {
    let r = crate::platform::linux::rt().block_on(async {
        ashpd::desktop::screenshot::Screenshot::request().interactive(true).modal(true).send().await?.response()
    });
    match portal_outcome(r.map(|s| s.uri().to_file_path().ok().and_then(|p| std::fs::read(p).ok()))) {
        Some(g) => g,
        None if crate::platform::linux::is_x11() => x11_grab(main.filter(|&w| w != 0), minimize).map_or(Grab::Failed(FAILED), Grab::Screen),
        None => Grab::Failed(NO_PORTAL),
    }
}

/// The portal's answer (the picture file's bytes) as a capture; None when there is no portal to ask.
#[cfg(target_os = "linux")]
fn portal_outcome(r: Result<Option<Vec<u8>>, ashpd::Error>) -> Option<Grab> {
    use ashpd::{desktop::ResponseError, Error};
    Some(match r {
        Ok(b) => b.and_then(as_png).map_or(Grab::Failed(FAILED), Grab::Png),
        Err(Error::Response(ResponseError::Cancelled)) => Grab::Cancelled,
        Err(Error::Response(ResponseError::Other)) => Grab::Failed(DENIED),
        Err(Error::PortalNotFound(_) | Error::Zbus(_)) => return None,
        Err(_) => Grab::Failed(FAILED),
    })
}

/// The portal's file as PNG (it usually is one already).
#[cfg(target_os = "linux")]
fn as_png(b: Vec<u8>) -> Option<Vec<u8>> {
    if b.starts_with(b"\x89PNG") { return Some(b); }
    let mut out = std::io::Cursor::new(Vec::new());
    image::load_from_memory(&b).ok()?.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// X11 without a portal: iconifies the main window (unless `minimize` is false), copies the root window, restores it.
#[cfg(target_os = "linux")]
fn x11_grab(main: Option<isize>, minimize: bool) -> Option<image::RgbaImage> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask, ImageFormat};
    let (c, n) = x11rb::connect(None).ok()?;
    let s = &c.setup().roots[n];
    let (root, w, h) = (s.root, s.width_in_pixels, s.height_in_pixels);
    let atom = |name: &str| c.intern_atom(false, name.as_bytes()).ok().and_then(|r| r.reply().ok()).map_or(0, |r| r.atom);
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    let win = main.filter(|_| minimize).map(|w| w as u32);
    if let Some(win) = win {
        let _ = c.send_event(false, root, mask, ClientMessageEvent::new(32, win, atom("WM_CHANGE_STATE"), [3u32, 0, 0, 0, 0])); // IconicState
        let _ = c.flush();
        std::thread::sleep(std::time::Duration::from_millis(450));
    }
    let img = c.get_image(ImageFormat::Z_PIXMAP, root, 0, 0, w, h, !0).ok().and_then(|r| r.reply().ok());
    if let Some(win) = win {
        let _ = c.map_window(win);
        let _ = c.send_event(false, root, mask, ClientMessageEvent::new(32, win, atom("_NET_ACTIVE_WINDOW"), [1u32, 0, 0, 0, 0]));
        let _ = c.flush();
    }
    let (mut px, n) = (img?.data, w as usize * h as usize * 4);
    if px.len() < n { return None; } // 24/32-bit TrueColor only
    px.truncate(n);
    for p in px.chunks_mut(4) { p.swap(0, 2); p[3] = 255; }
    image::RgbaImage::from_raw(w.into(), h.into(), px)
}

/// macOS: the system's own area picker (`screencapture -i`; Space switches to a window, Esc cancels), no sound.
#[cfg(target_os = "macos")]
fn grab(_: Option<isize>, _: bool) -> Grab {
    let path = std::env::temp_dir().join(format!("webox-screenshot-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let ran = std::process::Command::new("/usr/sbin/screencapture").arg("-i").arg("-x").arg(&path).status().is_ok();
    let png = std::fs::read(&path).ok().filter(|b| !b.is_empty());
    let _ = std::fs::remove_file(&path);
    match png { Some(b) => Grab::Png(b), None if ran => Grab::Cancelled, None => Grab::Failed(FAILED) }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn grab(_: Option<isize>, _: bool) -> Grab { Grab::Failed(FAILED) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_maps_to_pixels() {
        // A 1000 x 500 point window showing a 2000 x 1000 px capture (200 % scaling).
        let view = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1000.0, 500.0));
        assert_eq!(crop_px(Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 70.0)), view, (2000, 1000)), (20, 40, 200, 100));
        assert_eq!(crop_px(Rect::from_min_max(pos2(-50.0, -50.0), pos2(2000.0, 2000.0)), view, (2000, 1000)), (0, 0, 2000, 1000));
        let png = crop_png(&image::RgbaImage::new(50, 40), (10, 10, 100, 100));
        assert_eq!(image::load_from_memory(&png).unwrap().width(), 40);
    }

    #[test]
    fn pressing_a_pill_keeps_the_selection() {
        let view = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1920.0, 1080.0));
        for sel in [Rect::from_min_max(pos2(400.0, 300.0), pos2(900.0, 600.0)), // pills under the selection
            Rect::from_min_max(pos2(0.0, 500.0), pos2(30.0, 1080.0)), // narrow, at the left and bottom edges
            Rect::from_min_max(pos2(1800.0, 0.0), pos2(1920.0, 1080.0))] {
            let (cancel, ok) = pill_rects(sel, view);
            for r in [cancel, ok] {
                assert!(view.contains_rect(r), "{r:?} off screen for {sel:?}");
                assert!(!starts_selection(r.center(), Some(sel), view), "a press on a pill must not start a new selection");
            }
            assert!(!cancel.intersects(ok));
        }
        let sel = Rect::from_min_max(pos2(400.0, 300.0), pos2(900.0, 600.0));
        assert!(starts_selection(pos2(100.0, 100.0), Some(sel), view));
        assert!(starts_selection(pos2(500.0, 400.0), Some(sel), view)); // inside the old selection: a new one
        assert!(starts_selection(pill_rects(sel, view).1.center(), None, view));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portal_answers_become_toasts() {
        use ashpd::{desktop::ResponseError, Error};
        assert!(matches!(portal_outcome(Err(Error::Response(ResponseError::Cancelled))), Some(Grab::Cancelled)));
        assert!(matches!(portal_outcome(Err(Error::Response(ResponseError::Other))), Some(Grab::Failed(DENIED))));
        assert!(matches!(portal_outcome(Err(Error::Zbus(zbus::Error::InvalidReply))), None)); // no portal: X11 fallback or toast
        assert!(matches!(portal_outcome(Ok(None)), Some(Grab::Failed(FAILED))));
        let mut jpg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(8, 4).write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
        let Some(Grab::Png(png)) = portal_outcome(Ok(Some(jpg.into_inner()))) else { panic!("portal picture not staged") };
        assert_eq!(image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().width(), 8);
    }
}
