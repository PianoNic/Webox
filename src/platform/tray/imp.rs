use super::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const CALLBACK_MSG: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
static EVENTS: Mutex<Vec<TrayEvent>> = Mutex::new(Vec::new());
static CTX: OnceLock<eframe::egui::Context> = OnceLock::new();
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static ICON: AtomicIsize = AtomicIsize::new(0); // the HICON currently shown

fn hwnd(h: isize) -> HWND { HWND(h as *mut c_void) }

/// A 32-bit premultiplied icon from straight RGBA.
fn hicon(rgba: &[u8], size: u32) -> Option<HICON> {
    unsafe {
        let bmi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: size as i32, biHeight: -(size as i32),
            biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let dc = GetDC(None);
        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok();
        ReleaseDC(None, dc);
        let color = color?;
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
        for (d, s) in dst.chunks_mut(4).zip(rgba.chunks(4)) {
            let a = s[3] as u32;
            d[0] = (s[2] as u32 * a / 255) as u8; d[1] = (s[1] as u32 * a / 255) as u8; d[2] = (s[0] as u32 * a / 255) as u8; d[3] = s[3];
        }
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);
        let info = ICONINFO { fIcon: BOOL(1), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(mask); let _ = DeleteObject(color);
        icon
    }
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let push = |e: TrayEvent| { EVENTS.lock().unwrap().push(e); if let Some(c) = CTX.get() { c.request_repaint(); } };
    if msg == CALLBACK_MSG {
        match (lp.0 & 0xFFFF) as u32 {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => push(TrayEvent::Left),
            WM_RBUTTONUP | WM_CONTEXTMENU => {
                let mut p = POINT::default();
                let _ = GetCursorPos(&mut p);
                let _ = SetForegroundWindow(h); // as MSDN asks for tray menus, so the popup may take the focus
                push(TrayEvent::Menu(p.x, p.y));
            }
            _ => {}
        }
        return LRESULT(0);
    }
    if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) { // explorer restarted: put the icon back
        let (icon, tip) = (ICON.load(Ordering::Relaxed), crate::platform::tray::tooltip(0));
        let _ = notify(NIM_ADD, h.0 as isize, icon, &tip);
        return LRESULT(0);
    }
    DefWindowProcW(h, msg, wp, lp)
}

fn notify(op: NOTIFY_ICON_MESSAGE, h: isize, icon: isize, tip: &str) -> bool {
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd(h);
        nid.uID = TRAY_ID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = CALLBACK_MSG;
        nid.hIcon = HICON(icon as *mut c_void);
        for (d, c) in nid.szTip.iter_mut().zip(tip.encode_utf16().take(127)) { *d = c; }
        Shell_NotifyIconW(op, &nid).as_bool()
    }
}

pub struct Tray { hwnd: isize, shown: Option<usize> }

impl Tray {
    /// Creates the hidden message window on its own thread (it needs a message loop) and adds the icon.
    pub fn new(ctx: &eframe::egui::Context) -> Option<Tray> {
        let _ = CTX.set(ctx.clone());
        let (tx, rx) = std::sync::mpsc::channel::<isize>();
        std::thread::spawn(move || unsafe {
            let Ok(module) = GetModuleHandleW(None) else { let _ = tx.send(0); return };
            let inst = HINSTANCE(module.0);
            let class = w!("WeboxTrayWindow");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst, lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
            let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("Webox tray"), WS_OVERLAPPED, 0, 0, 0, 0, None, None, inst, None) else { let _ = tx.send(0); return };
            TRAY_HWND.store(h.0 as isize, Ordering::Relaxed);
            let _ = tx.send(h.0 as isize);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() { let _ = TranslateMessage(&msg); DispatchMessageW(&msg); }
        });
        let h = rx.recv().ok().filter(|h| *h != 0)?;
        let mut t = Tray { hwnd: h, shown: None };
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
        let icon = hicon(&app_icon_rgba(size), size)?;
        ICON.store(icon.0 as isize, Ordering::Relaxed);
        if !notify(NIM_ADD, h, icon.0 as isize, &tooltip(0)) { return None; }
        t.shown = Some(0);
        Some(t)
    }

    /// Shows the icon for `unread` notifications (red badge variant when > 0) and the matching tooltip; cheap when unchanged.
    pub fn set_unread(&mut self, unread: usize) {
        if self.shown == Some(unread) { return; }
        self.shown = Some(unread);
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
        let rgba = if unread > 0 { unread_icon_rgba(size) } else { app_icon_rgba(size) };
        let Some(icon) = hicon(&rgba, size) else { return };
        let old = ICON.swap(icon.0 as isize, Ordering::Relaxed);
        let _ = notify(NIM_MODIFY, self.hwnd, icon.0 as isize, &tooltip(unread));
        if old != 0 { unsafe { let _ = DestroyIcon(HICON(old as *mut c_void)); } }
    }

    pub fn poll(&mut self) -> Vec<TrayEvent> { std::mem::take(&mut *EVENTS.lock().unwrap()) }
}

impl Drop for Tray {
    fn drop(&mut self) { let _ = notify(NIM_DELETE, self.hwnd, 0, ""); }
}

/// Test hook: pretends the tray icon was right-clicked at (x, y) / left-clicked.
pub fn inject(e: TrayEvent) { EVENTS.lock().unwrap().push(e); if let Some(c) = CTX.get() { c.request_repaint(); } }

// Taskbar button of the main window.

pub fn flash(main: isize, on: bool) {
    unsafe {
        let fi = FLASHWINFO { cbSize: std::mem::size_of::<FLASHWINFO>() as u32, hwnd: hwnd(main),
            dwFlags: if on { FLASHW_TRAY | FLASHW_TIMERNOFG } else { FLASHW_STOP }, uCount: 0, dwTimeout: 0 };
        let _ = FlashWindowEx(&fi);
    }
}

/// Unread badge on the taskbar button (ITaskbarList3::SetOverlayIcon); 0 clears it. Must be called on the UI thread.
pub fn overlay(main: isize, unread: usize) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(list) = CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER) else { return };
        if list.HrInit().is_err() { return; }
        if unread == 0 { let _ = list.SetOverlayIcon(hwnd(main), HICON::default(), PCWSTR::null()); return; }
        let size = 32;
        let Some(icon) = hicon(&badge_rgba(unread, size), size) else { return };
        let desc: Vec<u16> = tooltip(unread).encode_utf16().chain([0]).collect();
        let _ = list.SetOverlayIcon(hwnd(main), icon, PCWSTR(desc.as_ptr()));
        let _ = DestroyIcon(icon);
    }
}

/// "Hide to the tray": a hidden (SW_HIDE) window is never repainted on Windows, so the app would stop polling messages and
/// could not raise toasts. Instead the window leaves the taskbar (WS_EX_TOOLWINDOW) and moves off-screen, still "visible".
static SAVED: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);
pub fn hide_to_tray(main: isize) {
    unsafe {
        let w = hwnd(main);
        let Some((l, t, r, b)) = window_rect(main) else { return };
        if SAVED.lock().unwrap().is_some() { return; }
        *SAVED.lock().unwrap() = Some((l, t, r - l, b - t));
        let _ = ShowWindow(w, SW_HIDE);
        let ex = GetWindowLongPtrW(w, GWL_EXSTYLE);
        SetWindowLongPtrW(w, GWL_EXSTYLE, (ex | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize));
        let _ = SetWindowPos(w, None, -32000, -32000, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER);
        let _ = ShowWindow(w, SW_SHOWNOACTIVATE);
    }
}
/// Undoes `hide_to_tray`; false when the window wasn't hidden that way.
pub fn restore_from_tray(main: isize) -> bool {
    unsafe {
        let Some((x, y, w, h)) = SAVED.lock().unwrap().take() else { return false };
        let hw = hwnd(main);
        let _ = ShowWindow(hw, SW_HIDE);
        let ex = GetWindowLongPtrW(hw, GWL_EXSTYLE);
        SetWindowLongPtrW(hw, GWL_EXSTYLE, (ex & !(WS_EX_TOOLWINDOW.0 as isize)) | WS_EX_APPWINDOW.0 as isize);
        let _ = SetWindowPos(hw, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        let _ = ShowWindow(hw, SW_SHOW);
        let _ = SetForegroundWindow(hw);
        true
    }
}

// Toast windows.

/// The work area (screen minus taskbar) of the primary monitor in physical pixels: (left, top, right, bottom).
pub fn work_area() -> Option<(i32, i32, i32, i32)> {
    let mut r = RECT::default();
    unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut r as *mut RECT as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).ok()? };
    Some((r.left, r.top, r.right, r.bottom))
}

struct Find { title: Vec<u16>, pid: u32, found: isize }
unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
    let f = &mut *(lp.0 as *mut Find);
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, Some(&mut pid));
    if pid != f.pid { return BOOL(1); }
    let mut buf = [0u16; 64];
    let n = GetWindowTextW(h, &mut buf) as usize;
    if buf[..n] == f.title[..] { f.found = h.0 as isize; return BOOL(0); }
    BOOL(1)
}

/// Our own top-level window with this title.
pub fn find_window(title: &str) -> Option<isize> {
    let mut f = Find { title: title.encode_utf16().collect(), pid: unsafe { GetCurrentProcessId() }, found: 0 };
    unsafe { let _ = EnumWindows(Some(each), LPARAM(&mut f as *mut Find as isize)); }
    (f.found != 0).then_some(f.found)
}

/// WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST: a toast never takes the focus and has no taskbar button.
pub fn make_toast_window(h: isize) {
    unsafe {
        let w = hwnd(h);
        let ex = GetWindowLongPtrW(w, GWL_EXSTYLE);
        SetWindowLongPtrW(w, GWL_EXSTYLE, ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) as isize);
        let _ = SetWindowPos(w, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
    }
    see_through(h);
}

/// Per-pixel alpha for a transparent popup (toast stack, tray menu, incoming-call toast): the area around its rounded shape
/// shows what's behind instead of black.
/// `with_transparent(true)` alone doesn't do it: glutin-winit's `finalize_window` drops the flag when the shared GL config
/// (picked once for the opaque main window, without WGL_TRANSPARENT_ARB) reports no transparency support, so winit never calls
/// DwmEnableBlurBehindWindow and DWM shows the window opaque, ignoring its alpha channel (the pixel format has 8 alpha bits and
/// PFD_SUPPORT_COMPOSITION). This makes the same call winit would (blur-behind with an empty region: alpha only, no blur).
/// It also stops Windows 11 from rounding the window's square corners or drawing its 1 px border around the popup's own
/// shape and shadow (DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_DONOTROUND, DWMWA_BORDER_COLOR = DWMWA_COLOR_NONE).
pub fn see_through(h: isize) {
    use windows::Win32::Graphics::Dwm::{DwmEnableBlurBehindWindow, DwmSetWindowAttribute, DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE};
    let (corner, none): (i32, u32) = (1, 0xFFFF_FFFE);
    unsafe {
        let region = CreateRectRgn(0, 0, -1, -1);
        let bb = DWM_BLURBEHIND { dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION, fEnable: true.into(), hRgnBlur: region, fTransitionOnMaximized: false.into() };
        let _ = DwmEnableBlurBehindWindow(hwnd(h), &bb);
        let _ = DeleteObject(region);
        let _ = DwmSetWindowAttribute(hwnd(h), DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const i32 as *const c_void, 4);
        let _ = DwmSetWindowAttribute(hwnd(h), DWMWA_BORDER_COLOR, &none as *const u32 as *const c_void, 4);
    }
}

/// DWM composes every window (Windows 8+); popups never need the opaque fallback.
pub fn composited() -> bool { true }
/// The no-compositor fallback (Linux X11) only.
pub fn popup_shape(_: isize, _: &[(i32, i32, i32, i32)], _: i32) {}

/// Moves / sizes the window in physical pixels without activating it.
pub fn place(h: isize, x: i32, y: i32, w: i32, hgt: i32) {
    unsafe { let _ = SetWindowPos(hwnd(h), HWND_TOPMOST, x, y, w, hgt, SWP_NOACTIVATE); }
}

pub fn dpi_scale(h: isize) -> f32 { (unsafe { GetDpiForWindow(hwnd(h)) }.max(96)) as f32 / 96.0 }

pub fn cursor() -> (i32, i32) { let mut p = POINT::default(); unsafe { let _ = GetCursorPos(&mut p); } (p.x, p.y) }

pub fn window_rect(h: isize) -> Option<(i32, i32, i32, i32)> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd(h), &mut r).ok()? };
    Some((r.left, r.top, r.right, r.bottom))
}

/// Debug helper: copies the window's screen rectangle (what the user sees: transparent corners show the desktop) into a PNG.
pub fn capture(h: isize, path: &Path) -> bool {
    unsafe {
        let Some((l, t, r, b)) = window_rect(h) else { return false };
        let (w, hgt) = (r - l, b - t);
        if w <= 0 || hgt <= 0 { return false; }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, hgt);
        let old = SelectObject(mem, bmp);
        let ok = BitBlt(mem, 0, 0, w, hgt, screen, l, t, SRCCOPY | CAPTUREBLT).is_ok();
        let mut bmi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -hgt,
            biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut px = vec![0u8; (w * hgt * 4) as usize];
        GetDIBits(mem, bmp, 0, hgt as u32, Some(px.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp); let _ = DeleteDC(mem); ReleaseDC(None, screen);
        for p in px.chunks_mut(4) { p.swap(0, 2); p[3] = 255; }
        if ok { save_png(px, w as u32, hgt as u32, path); }
        ok
    }
}
